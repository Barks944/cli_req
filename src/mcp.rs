// Implements REQ-0017 (MCP server for agents), REQ-0047 (.mcp.json bootstrap
// via --init-config), and REQ-0048 (first-class agent guidance baked into
// every tool description). Speaks JSON-RPC 2.0 over newline-delimited stdio.
// `repair` is deliberately not exposed — integrity recovery is humans-only.
use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

use crate::cli::McpArgs;
use crate::commands;
use crate::help_text;
use crate::model::{Kind, Link, LinkKind, Priority, Requirement, Status};
use crate::storage::{self, resolve_path};
use crate::validate;

const PROTOCOL_VERSION: &str = "2024-11-05";

#[derive(Deserialize)]
struct JsonRpcRequest {
    #[allow(dead_code)]
    jsonrpc: Option<String>,
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Serialize)]
struct JsonRpcResponse {
    jsonrpc: &'static str,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Serialize)]
struct JsonRpcError {
    code: i32,
    message: String,
}

pub fn run(args: McpArgs, file: &Option<PathBuf>) -> Result<()> {
    if args.init_config {
        return write_config(&args.config_path, args.force);
    }
    serve(file)
}

// ---------- stdio loop ----------

fn serve(file: &Option<PathBuf>) -> Result<()> {
    let path = resolve_path(file);
    let stdin = io::stdin();
    let mut stdin = stdin.lock();
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    let mut line = String::new();

    loop {
        line.clear();
        let n = stdin.read_line(&mut line).context("read from stdin")?;
        if n == 0 {
            break;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let req: JsonRpcRequest = match serde_json::from_str(trimmed) {
            Ok(r) => r,
            Err(e) => {
                // Cannot reply with an id we don't have. Emit a parse-error notification log.
                eprintln!("mcp: malformed JSON-RPC: {} ({})", e, trimmed);
                continue;
            }
        };

        let is_notification = req.id.is_none();
        let result = handle(&req.method, &req.params, &path);
        if is_notification {
            continue;
        }
        let id = req.id.unwrap_or(Value::Null);
        let resp = match result {
            Ok(value) => JsonRpcResponse {
                jsonrpc: "2.0",
                id,
                result: Some(value),
                error: None,
            },
            Err(e) => JsonRpcResponse {
                jsonrpc: "2.0",
                id,
                result: None,
                error: Some(JsonRpcError {
                    code: -32000,
                    message: e.to_string(),
                }),
            },
        };
        writeln!(stdout, "{}", serde_json::to_string(&resp)?)?;
        stdout.flush()?;
    }
    Ok(())
}

fn handle(method: &str, params: &Value, file: &Path) -> Result<Value> {
    match method {
        "initialize" => Ok(json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": { "tools": {} },
            "serverInfo": {
                "name": "req",
                "version": env!("CARGO_PKG_VERSION"),
            },
            "instructions": SERVER_GUIDANCE,
        })),
        "notifications/initialized" | "initialized" => Ok(Value::Null),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tool_definitions() })),
        "tools/call" => {
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("tools/call requires 'name'"))?;
            let args = params.get("arguments").cloned().unwrap_or(Value::Null);
            match call_tool(name, &args, file) {
                Ok(text) => Ok(json!({
                    "content": [{ "type": "text", "text": text }],
                    "isError": false,
                })),
                Err(e) => Ok(json!({
                    "content": [{ "type": "text", "text": e.to_string() }],
                    "isError": true,
                })),
            }
        }
        _ => Err(anyhow!("unknown method: {}", method)),
    }
}

const SERVER_GUIDANCE: &str = "\
This is the `req` MCP server for managed requirements. When the user describes \
new behaviour the system should have, call `req_add`. Before starting work on \
a feature call `req_list` and `req_show`. Before declaring work complete call \
`req_validate`. Never read project.req directly — its integrity hash will \
block the next CLI operation if you do. For full triggers and rules call \
`req_help` with `{\"section\":\"agents\"}` or `{\"section\":\"best-practice\"}`.";

// ---------- tool catalog ----------

struct ToolDef {
    name: &'static str,
    description: &'static str,
    schema: fn() -> Value,
}

fn tool_definitions() -> Vec<Value> {
    TOOLS
        .iter()
        .map(|t| {
            json!({
                "name": t.name,
                "description": t.description,
                "inputSchema": (t.schema)(),
            })
        })
        .collect()
}

const TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "req_list",
        description: "List managed requirements with optional filters. CALL THIS FIRST when starting work on any feature so you know what already exists. Returns each requirement's ID, title, kind, priority, status, and tags. Filters compose with AND semantics.",
        schema: list_schema,
    },
    ToolDef {
        name: "req_show",
        description: "Show full detail for one requirement: statement, rationale, acceptance criteria, links, and the full append-only history. Use this when you need the WHY behind a requirement, not just the title.",
        schema: show_schema,
    },
    ToolDef {
        name: "req_add",
        description: "Create a new requirement. Call this when the user describes new behaviour the system should have. The validator enforces best practice (modal verb shall/must/should/will, acceptance criteria for functional, no weasel words, no broken links). Bad input is REJECTED — rewrite the statement, don't try to bypass the rules. Returns the allocated REQ-NNNN ID.",
        schema: add_schema,
    },
    ToolDef {
        name: "req_update",
        description: "Modify an existing requirement. ALWAYS pass `reason` so the history records why. Prefer `add_acceptance` (append one) over `acceptance` (replace whole list). Status transitions Draft→Proposed→Approved→Implemented→Verified are not strictly enforced but should be respected.",
        schema: update_schema,
    },
    ToolDef {
        name: "req_delete",
        description: "Retire a requirement. Default soft-deletes (sets status to Obsolete, preserves links and history) — this is almost always what you want. Hard-delete (hard=true) refuses if inbound links exist. Always pass `reason`.",
        schema: delete_schema,
    },
    ToolDef {
        name: "req_link",
        description: "Create a typed link between two requirements. Kinds: parent (hierarchy), depends_on (sequencing), refines (atomic split of a coarser parent), conflicts, verifies. Parent links are cycle-checked. Use `refines` when splitting a compound requirement into atomic children.",
        schema: link_schema,
    },
    ToolDef {
        name: "req_validate",
        description: "Run the validator across every requirement. Returns errors and warnings. CALL THIS before declaring work complete. 0 errors is mandatory; warnings are advisory but should be addressed when easy.",
        schema: no_args_schema,
    },
    ToolDef {
        name: "req_coverage",
        description: "Map requirements to source code. Default mode reports orphans (requirements with no code marker) and ghosts (code mentioning unknown REQ IDs). Set unlinked_files=true to flip to file-side (code files without any REQ marker). Set by_file=true for per-file → REQ IDs. Use for refactor audits and trace reviews.",
        schema: coverage_schema,
    },
    ToolDef {
        name: "req_export",
        description: "Render the project to markdown, json, csv, or html. Use for reports to humans or downstream tooling. Returns the rendered text — does NOT write to disk.",
        schema: export_schema,
    },
    ToolDef {
        name: "req_help",
        description: "Fetch a structured documentation section. Sections: overview, concepts, best-practice, workflow, integration, version-control, audit, file-format, agents, mcp, export, tui, web. Call with {section: \"agents\"} for the trigger table; {section: \"best-practice\"} when uncertain about validator rules. List all section names with section=\"_index\".",
        schema: help_schema,
    },
];

// ---------- schemas ----------

fn no_args_schema() -> Value {
    json!({ "type": "object", "properties": {} })
}

fn list_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "status":   { "type": "string", "enum": ["draft","proposed","approved","implemented","verified","obsolete"] },
            "kind":     { "type": "string", "enum": ["functional","non-functional","constraint","interface","business"] },
            "priority": { "type": "string", "enum": ["must","should","could","wont"] },
            "tag":      { "type": "array", "items": { "type": "string" } },
            "query":    { "type": "string", "description": "Case-insensitive substring match against title + statement" }
        }
    })
}

fn show_schema() -> Value {
    json!({
        "type": "object",
        "required": ["id"],
        "properties": { "id": { "type": "string", "description": "REQ-NNNN" } }
    })
}

fn add_schema() -> Value {
    json!({
        "type": "object",
        "required": ["title","statement","rationale"],
        "properties": {
            "title":     { "type": "string", "description": "Short imperative title, 5-120 characters." },
            "statement": { "type": "string", "description": "Normative statement containing shall/must/should/will." },
            "rationale": { "type": "string", "description": "WHY this exists." },
            "kind":      { "type": "string", "enum": ["functional","non-functional","constraint","interface","business"], "default": "functional" },
            "priority":  { "type": "string", "enum": ["must","should","could","wont"], "default": "should" },
            "acceptance":{ "type": "array", "items": { "type": "string" }, "description": "Required for functional kind: testable criteria." },
            "tags":      { "type": "array", "items": { "type": "string" } },
            "parent":    { "type": "string", "description": "Optional parent REQ-NNNN to link as a parent." }
        }
    })
}

fn update_schema() -> Value {
    json!({
        "type": "object",
        "required": ["id","reason"],
        "properties": {
            "id":               { "type": "string" },
            "reason":           { "type": "string", "description": "Recorded on the new history entry. Mandatory." },
            "title":            { "type": "string" },
            "statement":        { "type": "string" },
            "rationale":        { "type": "string" },
            "acceptance":       { "type": "array", "items": { "type": "string" }, "description": "REPLACE the acceptance list wholesale." },
            "add_acceptance":   { "type": "array", "items": { "type": "string" }, "description": "Append these to the existing list (preferred)." },
            "remove_acceptance":{ "type": "array", "items": { "type": "integer" }, "description": "1-based indexes to remove." },
            "kind":             { "type": "string", "enum": ["functional","non-functional","constraint","interface","business"] },
            "priority":         { "type": "string", "enum": ["must","should","could","wont"] },
            "status":           { "type": "string", "enum": ["draft","proposed","approved","implemented","verified","obsolete"] },
            "add_tag":          { "type": "array", "items": { "type": "string" } },
            "remove_tag":       { "type": "array", "items": { "type": "string" } }
        }
    })
}

fn delete_schema() -> Value {
    json!({
        "type": "object",
        "required": ["id","reason"],
        "properties": {
            "id":     { "type": "string" },
            "reason": { "type": "string" },
            "hard":   { "type": "boolean", "default": false, "description": "Default false (soft, status→Obsolete). Hard refuses if inbound links exist." }
        }
    })
}

fn link_schema() -> Value {
    json!({
        "type": "object",
        "required": ["from","to"],
        "properties": {
            "from":   { "type": "string" },
            "to":     { "type": "string" },
            "kind":   { "type": "string", "enum": ["parent","depends_on","conflicts","refines","verifies"], "default": "parent" },
            "remove": { "type": "boolean", "default": false }
        }
    })
}

fn coverage_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "path":            { "type": "string", "default": "." },
            "extensions":      { "type": "array", "items": { "type": "string" }, "description": "File extensions to scan; defaults to common source types." },
            "unlinked_files":  { "type": "boolean", "default": false, "description": "List source files with no REQ markers." },
            "by_file":         { "type": "boolean", "default": false, "description": "Per-file: file → list of REQ IDs." }
        }
    })
}

fn export_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "format": { "type": "string", "enum": ["markdown","json","csv","html"], "default": "markdown" }
        }
    })
}

fn help_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "section": { "type": "string", "description": "Section name, or '_index' for the list of section names." }
        }
    })
}

// ---------- dispatcher ----------

fn call_tool(name: &str, args: &Value, file: &Path) -> Result<String> {
    match name {
        "req_list" => tool_list(args, file),
        "req_show" => tool_show(args, file),
        "req_add" => tool_add(args, file),
        "req_update" => tool_update(args, file),
        "req_delete" => tool_delete(args, file),
        "req_link" => tool_link(args, file),
        "req_validate" => tool_validate(file),
        "req_coverage" => tool_coverage(args, file),
        "req_export" => tool_export(args, file),
        "req_help" => tool_help(args),
        _ => Err(anyhow!("unknown tool: {}", name)),
    }
}

// ---------- tool implementations ----------

fn s(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(|s| s.to_string())
}
fn req_s(v: &Value, k: &str) -> Result<String> {
    s(v, k).ok_or_else(|| anyhow!("'{}' is required", k))
}
fn parse_kind(s: &str) -> Result<Kind> {
    Ok(match s {
        "functional" => Kind::Functional,
        "non-functional" | "nonfunctional" => Kind::NonFunctional,
        "constraint" => Kind::Constraint,
        "interface" => Kind::Interface,
        "business" => Kind::Business,
        _ => return Err(anyhow!("unknown kind: {}", s)),
    })
}
fn parse_priority(s: &str) -> Result<Priority> {
    Ok(match s {
        "must" => Priority::Must,
        "should" => Priority::Should,
        "could" => Priority::Could,
        "wont" => Priority::Wont,
        _ => return Err(anyhow!("unknown priority: {}", s)),
    })
}
fn parse_status(s: &str) -> Result<Status> {
    Ok(match s {
        "draft" => Status::Draft,
        "proposed" => Status::Proposed,
        "approved" => Status::Approved,
        "implemented" => Status::Implemented,
        "verified" => Status::Verified,
        "obsolete" => Status::Obsolete,
        _ => return Err(anyhow!("unknown status: {}", s)),
    })
}
fn parse_link_kind(s: &str) -> Result<LinkKind> {
    Ok(match s {
        "parent" => LinkKind::Parent,
        "depends_on" | "depends-on" => LinkKind::DependsOn,
        "conflicts" => LinkKind::Conflicts,
        "refines" => LinkKind::Refines,
        "verifies" => LinkKind::Verifies,
        _ => return Err(anyhow!("unknown link kind: {}", s)),
    })
}

fn tool_list(args: &Value, file: &Path) -> Result<String> {
    let project = storage::load(file)?;
    let status = s(args, "status").map(|s| parse_status(&s)).transpose()?;
    let kind = s(args, "kind").map(|s| parse_kind(&s)).transpose()?;
    let priority = s(args, "priority")
        .map(|s| parse_priority(&s))
        .transpose()?;
    let tags: Vec<String> = args
        .get("tag")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(|s| s.to_string())
                .collect()
        })
        .unwrap_or_default();
    let query = s(args, "query").map(|q| q.to_lowercase());

    let mut rows = Vec::new();
    for r in project.requirements.values() {
        if let Some(k) = kind {
            if r.kind != k {
                continue;
            }
        }
        if let Some(p) = priority {
            if r.priority != p {
                continue;
            }
        }
        if let Some(st) = status {
            if r.status != st {
                continue;
            }
        }
        if !tags.iter().all(|t| r.tags.iter().any(|rt| rt == t)) {
            continue;
        }
        if let Some(q) = &query {
            if !r.title.to_lowercase().contains(q) && !r.statement.to_lowercase().contains(q) {
                continue;
            }
        }
        rows.push(json!({
            "id": r.id,
            "title": r.title,
            "kind": r.kind.as_str(),
            "priority": r.priority.as_str(),
            "status": r.status.as_str(),
            "tags": r.tags,
        }));
    }
    Ok(serde_json::to_string_pretty(&json!({
        "project": project.name,
        "count": rows.len(),
        "requirements": rows
    }))?)
}

fn tool_show(args: &Value, file: &Path) -> Result<String> {
    let id = req_s(args, "id")?;
    let project = storage::load(file)?;
    let r = project
        .requirements
        .get(&id)
        .ok_or_else(|| anyhow!("no such requirement: {}", id))?;
    Ok(serde_json::to_string_pretty(r)?)
}

fn tool_add(args: &Value, file: &Path) -> Result<String> {
    let mut project = storage::load(file)?;
    let title = req_s(args, "title")?;
    let statement = req_s(args, "statement")?;
    let rationale = req_s(args, "rationale")?;
    let kind = s(args, "kind")
        .map(|s| parse_kind(&s))
        .transpose()?
        .unwrap_or(Kind::Functional);
    let priority = s(args, "priority")
        .map(|s| parse_priority(&s))
        .transpose()?
        .unwrap_or(Priority::Should);
    let acceptance: Vec<String> = args
        .get("acceptance")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(|s| s.to_string())
                .collect()
        })
        .unwrap_or_default();
    let tags: Vec<String> = args
        .get("tags")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(|s| s.to_string())
                .collect()
        })
        .unwrap_or_default();

    let now = Utc::now();
    let mut links = Vec::new();
    if let Some(parent) = s(args, "parent") {
        if !project.requirements.contains_key(&parent) {
            return Err(anyhow!("parent {} does not exist", parent));
        }
        links.push(Link {
            kind: LinkKind::Parent,
            target: parent,
        });
    }
    // Validate BEFORE allocating an ID, so a rejected add does not burn one.
    let mut req = Requirement {
        id: String::new(),
        title,
        statement,
        rationale,
        acceptance,
        kind,
        priority,
        status: Status::Draft,
        tags,
        links,
        created: now,
        updated: now,
        history: vec![commands::history("created via MCP", None)],
        tests: Vec::new(),
    };
    let findings = validate::validate_requirement(&req);
    let errs = validate::errors_only(&findings);
    if !errs.is_empty() {
        let msgs: Vec<String> = errs
            .iter()
            .map(|f| format!("[{}] {}", f.field, f.message))
            .collect();
        return Err(anyhow!("rejected: {}", msgs.join("; ")));
    }
    let id = project.allocate_id();
    req.id = id.clone();
    project.requirements.insert(id.clone(), req);
    project.updated = now;
    storage::save(file, &project)?;
    let warns: Vec<String> = findings
        .iter()
        .filter(|f| !f.error)
        .map(|f| format!("[{}] {}", f.field, f.message))
        .collect();
    Ok(json!({ "id": id, "warnings": warns }).to_string())
}

fn tool_update(args: &Value, file: &Path) -> Result<String> {
    let id = req_s(args, "id")?;
    let reason = req_s(args, "reason")?;
    let mut project = storage::load(file)?;
    let r = project
        .requirements
        .get_mut(&id)
        .ok_or_else(|| anyhow!("no such requirement: {}", id))?;
    let mut changes = Vec::new();
    if let Some(t) = s(args, "title") {
        if r.title != t {
            changes.push("title".into());
            r.title = t;
        }
    }
    if let Some(t) = s(args, "statement") {
        if r.statement != t {
            changes.push("statement".into());
            r.statement = t;
        }
    }
    if let Some(t) = s(args, "rationale") {
        if r.rationale != t {
            changes.push("rationale".into());
            r.rationale = t;
        }
    }
    if let Some(arr) = args.get("acceptance").and_then(Value::as_array) {
        r.acceptance = arr
            .iter()
            .filter_map(Value::as_str)
            .map(|s| s.to_string())
            .collect();
        changes.push("acceptance replaced".into());
    }
    if let Some(arr) = args.get("add_acceptance").and_then(Value::as_array) {
        for v in arr.iter().filter_map(Value::as_str) {
            r.acceptance.push(v.to_string());
            changes.push(format!("+acceptance: {:?}", v));
        }
    }
    if let Some(arr) = args.get("remove_acceptance").and_then(Value::as_array) {
        let mut idxs: Vec<usize> = arr
            .iter()
            .filter_map(Value::as_u64)
            .map(|u| u as usize)
            .collect();
        idxs.sort_unstable();
        idxs.dedup();
        idxs.reverse();
        for i in idxs {
            if i == 0 || i > r.acceptance.len() {
                return Err(anyhow!("remove_acceptance index {} out of range", i));
            }
            let g = r.acceptance.remove(i - 1);
            changes.push(format!("-acceptance #{}: {:?}", i, g));
        }
    }
    if let Some(k) = s(args, "kind").map(|s| parse_kind(&s)).transpose()? {
        if r.kind != k {
            changes.push(format!("kind→{}", k.as_str()));
            r.kind = k;
        }
    }
    if let Some(p) = s(args, "priority")
        .map(|s| parse_priority(&s))
        .transpose()?
    {
        if r.priority != p {
            changes.push(format!("priority→{}", p.as_str()));
            r.priority = p;
        }
    }
    if let Some(st) = s(args, "status").map(|s| parse_status(&s)).transpose()? {
        if r.status != st {
            changes.push(format!("status→{}", st.as_str()));
            r.status = st;
        }
    }
    if let Some(arr) = args.get("add_tag").and_then(Value::as_array) {
        for t in arr.iter().filter_map(Value::as_str) {
            if !r.tags.iter().any(|x| x == t) {
                r.tags.push(t.into());
                changes.push(format!("+tag {}", t));
            }
        }
    }
    if let Some(arr) = args.get("remove_tag").and_then(Value::as_array) {
        for t in arr.iter().filter_map(Value::as_str) {
            if let Some(p) = r.tags.iter().position(|x| x == t) {
                r.tags.remove(p);
                changes.push(format!("-tag {}", t));
            }
        }
    }
    if changes.is_empty() {
        return Ok(json!({ "id": id, "changes": [] }).to_string());
    }

    let findings = validate::validate_requirement(r);
    let errs = validate::errors_only(&findings);
    if !errs.is_empty() {
        let msgs: Vec<String> = errs
            .iter()
            .map(|f| format!("[{}] {}", f.field, f.message))
            .collect();
        return Err(anyhow!("rejected: {}", msgs.join("; ")));
    }
    r.updated = Utc::now();
    r.history
        .push(commands::history(changes.join("; "), Some(reason)));
    project.updated = Utc::now();
    storage::save(file, &project)?;
    Ok(json!({ "id": id, "changes": changes }).to_string())
}

fn tool_delete(args: &Value, file: &Path) -> Result<String> {
    let id = req_s(args, "id")?;
    let reason = req_s(args, "reason")?;
    let hard = args.get("hard").and_then(Value::as_bool).unwrap_or(false);
    let mut project = storage::load(file)?;
    if !project.requirements.contains_key(&id) {
        return Err(anyhow!("no such requirement: {}", id));
    }
    let inbound: Vec<String> = project
        .requirements
        .values()
        .filter(|r| r.links.iter().any(|l| l.target == id))
        .map(|r| r.id.clone())
        .collect();
    if hard {
        if !inbound.is_empty() {
            return Err(anyhow!(
                "{} is referenced by {} — soft-delete instead",
                id,
                inbound.join(", ")
            ));
        }
        project.requirements.remove(&id);
    } else {
        let r = project.requirements.get_mut(&id).unwrap();
        r.status = Status::Obsolete;
        r.updated = Utc::now();
        r.history
            .push(commands::history("marked obsolete via MCP", Some(reason)));
    }
    project.updated = Utc::now();
    storage::save(file, &project)?;
    Ok(json!({ "id": id, "deleted": if hard { "hard" } else { "soft" } }).to_string())
}

fn tool_link(args: &Value, file: &Path) -> Result<String> {
    let from = req_s(args, "from")?;
    let to = req_s(args, "to")?;
    if from == to {
        return Err(anyhow!("cannot link to self"));
    }
    let kind = s(args, "kind")
        .map(|s| parse_link_kind(&s))
        .transpose()?
        .unwrap_or(LinkKind::Parent);
    let remove = args.get("remove").and_then(Value::as_bool).unwrap_or(false);
    let mut project = storage::load(file)?;
    if !project.requirements.contains_key(&to) {
        return Err(anyhow!("target {} does not exist", to));
    }

    if matches!(kind, LinkKind::Parent) && !remove && would_cycle(&project, &from, &to) {
        return Err(anyhow!("parent {} -> {} would create a cycle", from, to));
    }
    let r = project
        .requirements
        .get_mut(&from)
        .ok_or_else(|| anyhow!("source {} does not exist", from))?;
    if remove {
        let before = r.links.len();
        r.links.retain(|l| !(l.kind == kind && l.target == to));
        if r.links.len() == before {
            return Err(anyhow!("no such link to remove"));
        }
        r.history.push(commands::history(
            format!("removed {} link to {}", kind.as_str(), to),
            None,
        ));
    } else {
        if r.links.iter().any(|l| l.kind == kind && l.target == to) {
            return Err(anyhow!("link already exists"));
        }
        r.links.push(Link {
            kind,
            target: to.clone(),
        });
        r.history.push(commands::history(
            format!("added {} link to {}", kind.as_str(), to),
            None,
        ));
    }
    r.updated = Utc::now();
    project.updated = Utc::now();
    storage::save(file, &project)?;
    Ok(json!({ "from": from, "to": to, "kind": kind.as_str(), "removed": remove }).to_string())
}

fn would_cycle(project: &crate::model::Project, from: &str, new_parent: &str) -> bool {
    let mut cur = new_parent.to_string();
    let mut seen = Vec::new();
    loop {
        if cur == from {
            return true;
        }
        if seen.contains(&cur) {
            return false;
        }
        seen.push(cur.clone());
        let next = project.requirements.get(&cur).and_then(|r| {
            r.links
                .iter()
                .find(|l| l.kind == LinkKind::Parent)
                .map(|l| l.target.clone())
        });
        match next {
            Some(n) => cur = n,
            None => return false,
        }
    }
}

fn tool_validate(file: &Path) -> Result<String> {
    let project = storage::load(file)?;
    let report = validate::validate_project(&project);
    let mut errors = 0usize;
    let mut warnings = 0usize;
    let mut findings = Vec::new();
    for (id, fs) in &report {
        for f in fs {
            if f.error {
                errors += 1
            } else {
                warnings += 1
            }
            findings.push(json!({ "id": id, "level": if f.error {"error"} else {"warning"}, "field": f.field, "message": f.message }));
        }
    }
    Ok(serde_json::to_string_pretty(&json!({
        "errors": errors, "warnings": warnings, "findings": findings
    }))?)
}

fn tool_coverage(args: &Value, file: &Path) -> Result<String> {
    use once_cell::sync::Lazy;
    use regex::Regex;
    static REQ_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"REQ-\d{4}").unwrap());
    const DEFAULTS: &[&str] = &[
        "rs", "py", "js", "ts", "tsx", "go", "java", "md", "toml", "c", "cpp", "h",
    ];
    const SKIP: &[&str] = &[
        ".git",
        "target",
        "node_modules",
        "dist",
        "build",
        ".venv",
        ".idea",
        ".vscode",
    ];

    let root = PathBuf::from(s(args, "path").unwrap_or_else(|| ".".into()));
    let exts: Vec<String> = args
        .get("extensions")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(|s| s.to_string())
                .collect()
        })
        .filter(|v: &Vec<String>| !v.is_empty())
        .unwrap_or_else(|| DEFAULTS.iter().map(|s| s.to_string()).collect());
    let unlinked = args
        .get("unlinked_files")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let by_file = args
        .get("by_file")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let mut per_file: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut scanned = 0usize;
    walk(&root, &exts, SKIP, &mut |p, content| {
        scanned += 1;
        let mut found = Vec::new();
        for m in REQ_RE.find_iter(content) {
            found.push(m.as_str().to_string());
        }
        if !found.is_empty() {
            per_file.insert(p.display().to_string(), found);
        }
    });

    if unlinked {
        let mut files = Vec::new();
        walk(&root, &exts, SKIP, &mut |p, content| {
            if !REQ_RE.is_match(content) {
                files.push(p.display().to_string());
            }
        });
        files.sort();
        files.dedup();
        return Ok(serde_json::to_string_pretty(&json!({
            "mode": "unlinked_files", "scanned": scanned, "unlinked": files
        }))?);
    }
    if by_file {
        let entries: Vec<Value> = per_file
            .into_iter()
            .map(|(f, mut ids)| {
                ids.sort();
                ids.dedup();
                json!({ "file": f, "req_ids": ids })
            })
            .collect();
        return Ok(serde_json::to_string_pretty(
            &json!({ "mode": "by_file", "files": entries }),
        )?);
    }
    let project = storage::load(file)?;
    let known: std::collections::BTreeSet<String> = project.requirements.keys().cloned().collect();
    let mut referenced = BTreeMap::new();
    let mut ghosts: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut obsolete_in_code: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut hits: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (file, ids) in &per_file {
        for id in ids {
            hits.entry(id.clone()).or_default().push(file.clone());
        }
    }
    for (id, refs) in &hits {
        match project.requirements.get(id) {
            Some(r) if matches!(r.status, Status::Obsolete) => {
                obsolete_in_code.insert(id.clone(), refs.clone());
            }
            Some(_) => {
                referenced.insert(id.clone(), refs.clone());
            }
            None => {
                ghosts.insert(id.clone(), refs.clone());
            }
        }
    }
    let orphans: Vec<String> = known
        .iter()
        .filter(|id| !hits.contains_key(*id))
        .filter(|id| !matches!(project.requirements[*id].status, Status::Obsolete))
        .cloned()
        .collect();
    Ok(serde_json::to_string_pretty(&json!({
        "mode": "default",
        "referenced": referenced, "orphans": orphans,
        "ghosts": ghosts, "obsolete_referenced": obsolete_in_code
    }))?)
}

fn walk(root: &Path, exts: &[String], skip: &[&str], visit: &mut impl FnMut(&Path, &str)) {
    let entries = match fs::read_dir(root) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name_s = name.to_string_lossy();
        if path.is_dir() {
            if skip.iter().any(|s| *s == name_s.as_ref()) {
                continue;
            }
            walk(&path, exts, skip, visit);
        } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            if !exts.iter().any(|x| x == ext) {
                continue;
            }
            if let Ok(text) = fs::read_to_string(&path) {
                visit(&path, &text);
            }
        }
    }
}

fn tool_export(args: &Value, file: &Path) -> Result<String> {
    let format = s(args, "format").unwrap_or_else(|| "markdown".into());
    let project = storage::load(file)?;
    match format.as_str() {
        "markdown" => Ok(crate::commands::export::to_markdown(&project)),
        "json" => Ok(serde_json::to_string_pretty(&project)?),
        "csv" | "html" => {
            // Reuse the command's renderers via a temp call — simplest is to inline-replicate for these.
            // For now defer with a guidance message.
            Err(anyhow!("format '{}' available via `req export -f {}` CLI; MCP returns markdown and json only for now", format, format))
        }
        _ => Err(anyhow!("unknown format: {}", format)),
    }
}

fn tool_help(args: &Value) -> Result<String> {
    let section = s(args, "section").unwrap_or_else(|| "_index".into());
    if section == "_index" {
        let list: Vec<Value> = help_text::sections()
            .iter()
            .map(|s| {
                json!({
                    "name": s.name, "summary": s.summary
                })
            })
            .collect();
        return Ok(serde_json::to_string_pretty(&json!({ "sections": list }))?);
    }
    match help_text::section(&section) {
        Some(s) => Ok(json!({ "name": s.name, "summary": s.summary, "body": s.body }).to_string()),
        None => Err(anyhow!("unknown section: {}", section)),
    }
}

// ---------- .mcp.json ----------

fn write_config(path: &Path, force: bool) -> Result<()> {
    if path.exists() && !force {
        return Err(anyhow!(
            "{} already exists — pass --force to overwrite",
            path.display()
        ));
    }
    let body = json!({
        "_readme": "MCP bootstrap for this project's `req` server. Agents reading this should treat req as the managed surface for editing project.req. See `req help mcp` and `req help agents` for triggers.",
        "mcpServers": {
            "req": {
                "command": "req",
                "args": ["mcp"],
                "description": "Managed requirements for this project. Tools: req_list, req_show, req_add, req_update, req_delete, req_link, req_validate, req_coverage, req_export, req_help. Call req_help with {section: 'agents'} on first contact for the trigger table."
            }
        }
    });
    fs::write(path, serde_json::to_string_pretty(&body)?)?;
    println!("Wrote {}", path.display());
    println!();
    println!("Agents connecting via MCP should call `tools/list` first, then");
    println!("`tools/call` with {{name: \"req_help\", arguments: {{section: \"agents\"}}}}");
    println!("for the trigger table that tells them when to use each tool.");
    Ok(())
}
