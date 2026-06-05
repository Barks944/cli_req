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
    if std::env::var_os("REQ_ACTOR_KIND").is_none() {
        // Callers reach `serve` only via stdio JSON-RPC, so attribute history
        // to an agent unless the operator overrode it explicitly.
        // SAFETY: set before any thread is spawned by the stdio loop.
        unsafe { std::env::set_var("REQ_ACTOR_KIND", "agent") };
    }
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
        description: "Fetch a structured documentation section. Call with section=\"_index\" (the default) for the full, authoritative list of section names with one-line summaries — the section set evolves, so do not hardcode it. Useful starting points once you have the index: section=\"agents\" for the trigger table, section=\"best-practice\" when uncertain about validator rules, section=\"integration\" for hooks and CI wiring.",
        schema: help_schema,
    },
    // ---------- agent-facing tools added in v0.1 ----------
    ToolDef {
        name: "req_status",
        description: "Project-level dashboard. Returns total count, per-status bucket counts, and a single delivery_progress_pct. Call this early in a session to see how much is shipped vs draft.",
        schema: no_args_schema,
    },
    ToolDef {
        name: "req_next",
        description: "Suggest a single next requirement to work on, dependency-aware. Filters compose; picks the highest-priority candidate whose depends_on links are all Implemented or Verified. Returns null when nothing qualifies.",
        schema: next_schema,
    },
    ToolDef {
        name: "req_check",
        description: "Incremental validate + coverage scoped to changes since a git ref. Use when reviewing a branch: returns errors/warnings only for requirements that changed since `base`, plus coverage findings only for source files changed since `base`. Cheaper than a full validate on every iteration.",
        schema: check_schema,
    },
    ToolDef {
        name: "req_diff",
        description: "Summarize per-requirement changes between two git revisions. Use during code review when git diff is too noisy. spec is `BASE..HEAD` (e.g. `origin/main..HEAD`). Returns added, removed, and field-level transitions. Suited to code review.",
        schema: diff_schema,
    },
    ToolDef {
        name: "req_stale",
        description: "Report the staleness of every requirement's latest test record against current HEAD. Three states: fresh, drifted (HEAD moved but no linked file changed), STALE (linked files changed since the record). Set only_stale=true to filter.",
        schema: stale_schema,
    },
    ToolDef {
        name: "req_audit",
        description: "Walk git log on the .req file and report per-commit signer + signature status (good/bad/expired/no-signature). Set gate=true with require_good_signature or required_signers to enforce a policy and return violations.",
        schema: audit_schema,
    },
    ToolDef {
        name: "req_test_record",
        description: "Attach a test record (commit SHA + outcome + notes) to a requirement. Use when you've manually verified a behaviour outside cargo test. For automated cargo-test outcomes call req_test_run instead.",
        schema: test_record_schema,
    },
    ToolDef {
        name: "req_test_run",
        description: "Parse a pre-captured cargo-test log file and attach one TestRecord per requirement matched by `req_NNNN_*` test names. The MCP form takes `from_file` (a path) — for safety the server does NOT execute arbitrary commands. Set promote=true to flip Implemented to Verified for any requirement with a fresh passing record.",
        schema: test_run_schema,
    },
    ToolDef {
        name: "req_verify",
        description: "Record a composition or inspection evidence record on a requirement, optionally promoting to Verified. Composition cites another requirement's tests; inspection records a human review. Use composition when the behaviour is covered by another test you can name; use inspection sparingly.",
        schema: verify_schema,
    },
    ToolDef {
        name: "req_batch",
        description: "Apply many mutations atomically from a JSON document. Supported kinds: add, update, delete, link. Any single validation failure rolls back the WHOLE batch — project.req stays byte-identical to its pre-batch state. One file write per successful batch.",
        schema: batch_schema,
    },
    ToolDef {
        name: "req_import",
        description: "Ingest requirements from markdown (level-2/3 headings as titles) or JSON (an array of candidates, or another project.req's requirements map). Every imported item goes through the validator. Set dry_run=true to preview without writing.",
        schema: import_schema,
    },
    ToolDef {
        name: "req_schema",
        description: "Fetch the JSON Schema for one of req's structured-input surfaces. which=\"add\" describes req_add input; \"batch\" describes req_batch; \"import\" describes the import-array form. Each schema carries the project's _format tag so callers can pin to a version.",
        schema: schema_which_schema,
    },
    ToolDef {
        name: "req_doctor",
        description: "Audit per-clone setup health: pre-commit hook installed, gitattributes pinned, merge driver activated, commit signing enabled. Read-only. Use in CI to gate on missing setup.",
        schema: no_args_schema,
    },
    ToolDef {
        name: "req_version",
        description: "Return this binary's version, the .req file format tag, and the MCP protocol version. Use to pin tooling against a known-good combination.",
        schema: no_args_schema,
    },
    ToolDef {
        name: "req_migrate",
        description: "Detect the .req file's _format and (when supported) upgrade it to the current schema, writing a sibling backup first. On the current format this is a no-op confirming no migration is needed.",
        schema: no_args_schema,
    },
    ToolDef {
        name: "req_review",
        description: "Single-shot PR-review report: validate + coverage + stale + audit + changed-requirement diff, scoped to a git rev range, returned as markdown. Use this to attach a spec impact summary to a pull request or CI run.",
        schema: review_schema,
    },
    ToolDef {
        name: "req_split",
        description: "Split a compound requirement into N atomic ones. Pass `id` and `into` (an array of new statements). The original is soft-retired to Obsolete (unless `keep_original` is true). New parts inherit the original's kind, priority, and tags; titles get a `— part i of N` suffix. Use to remediate REQ-V-0010 compound findings without manual fan-out.",
        schema: split_schema,
    },
    // REQ-0104: req_brief MCP tool — session-start summary for agents.
    ToolDef {
        name: "req_brief",
        description: "Session-start summary: project name, delivery %, what's next to work on, what's loose (Implemented but not Verified, Drafts). Run this FIRST in any new session — it's the spec-state read that tells you where to pick up. Default is short; pass `full: true` for the dashboard view.",
        schema: brief_schema,
    },
    // REQ-0101: req_lint MCP tool.
    ToolDef {
        name: "req_lint",
        description: "Project-wide quality audit beyond the validator. Returns markdown (default) or JSON with sections for validator findings, requirements lacking source markers, short rationales, single-acceptance functionals, and active requirements with no test record. Read-only; lint observations never gate.",
        schema: lint_schema,
    },
];

// ---------- schemas ----------

fn no_args_schema() -> Value {
    json!({ "type": "object", "properties": {} })
}

fn review_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "base": { "type": "string", "description": "Base git rev (default: origin/main). Compared as `<base>..HEAD`." },
            "path": { "type": "string", "description": "Directory to scan for `// REQ-NNNN` markers (default: repo root)." },
            "gate": { "type": "boolean", "description": "Surface the markerless-source / ghost findings as a non-zero exit so callers can treat the report as a CI gate." },
            "staged": { "type": "boolean", "description": "Scope the report to staged changes (`git diff --cached`) instead of `<base>..HEAD`. Mirrors the pre-commit hook." },
            "new": { "type": "boolean", "description": "Scope validator findings to requirements added or changed in this range (implied by `staged`). Suppresses backlog warnings on untouched requirements." },
            "all": { "type": "boolean", "description": "Force the full-project validator sweep even under `staged` — the deliberate hygiene view." },
            "json": { "type": "boolean", "description": "Return JSON instead of markdown. Defaults to true on MCP." }
        }
    })
}

fn split_schema() -> Value {
    json!({
        "type": "object",
        "required": ["id", "into"],
        "properties": {
            "id":   { "type": "string", "description": "The compound requirement to split (any case/pad form accepted)." },
            "into": { "type": "array", "minItems": 2, "items": { "type": "string" }, "description": "Two or more atomic statements to create as siblings." },
            "reason": { "type": "string", "description": "Recorded on the original's history when soft-retired." },
            "keep_original": { "type": "boolean", "description": "Don't soft-retire the original; create new parts beside it." }
        }
    })
}

// REQ-0104: schema for the req_brief MCP tool.
fn brief_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "full": { "type": "boolean", "description": "Expand to a dashboard view with by-status counts, gate mode, recent activity. Default false (short)." },
            "json": { "type": "boolean", "description": "Return JSON instead of text. Defaults to true on MCP." }
        }
    })
}

// REQ-0101: schema for the req_lint MCP tool.
fn lint_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "path": { "type": "string", "description": "Source-tree root to scan for `// REQ-NNNN:` markers. Default `.`." },
            "json": { "type": "boolean", "description": "Return JSON instead of markdown. Defaults to true on MCP." }
        }
    })
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

fn next_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "status":   { "type": "string", "enum": ["draft","proposed","approved","implemented","verified","obsolete"] },
            "kind":     { "type": "string", "enum": ["functional","non-functional","constraint","interface","business"] },
            "priority": { "type": "string", "enum": ["must","should","could","wont"] },
            "tag":      { "type": "array", "items": { "type": "string" } }
        }
    })
}

fn check_schema() -> Value {
    json!({
        "type": "object",
        "required": ["base"],
        "properties": {
            "base": { "type": "string", "description": "Git ref to diff against (e.g. origin/main)." },
            "path": { "type": "string", "default": ".", "description": "Source-tree root for the coverage scan on changed files." }
        }
    })
}

fn diff_schema() -> Value {
    json!({
        "type": "object",
        "required": ["spec"],
        "properties": {
            "spec": { "type": "string", "description": "BASE..HEAD git ref pair, e.g. 'origin/main..HEAD'." }
        }
    })
}

fn stale_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "path":       { "type": "string", "default": "." },
            "only_stale": { "type": "boolean", "default": false }
        }
    })
}

fn audit_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "limit":                  { "type": "integer", "minimum": 1, "default": 50 },
            "gate":                   { "type": "boolean", "default": false, "description": "If true, returns violations and ok=false when a commit fails the policy." },
            "require_good_signature": { "type": "boolean", "default": false },
            "required_signers":       { "type": "array",   "items": { "type": "string" }, "description": "Case-insensitive substring match against git's %GS field." }
        }
    })
}

fn test_record_schema() -> Value {
    json!({
        "type": "object",
        "required": ["id", "result"],
        "properties": {
            "id":     { "type": "string", "pattern": "^REQ-\\d{4}$" },
            "result": { "type": "string", "enum": ["pass","fail"] },
            "notes":  { "type": "string", "default": "" }
        }
    })
}

fn test_run_schema() -> Value {
    json!({
        "type": "object",
        "required": ["from_file"],
        "properties": {
            "from_file": { "type": "string", "description": "Path to a pre-captured cargo-test log. The MCP form does NOT execute commands." },
            "dry_run":   { "type": "boolean", "default": false },
            "promote":   { "type": "boolean", "default": false }
        }
    })
}

fn verify_schema() -> Value {
    json!({
        "type": "object",
        "required": ["id", "by", "notes"],
        "properties": {
            "id":      { "type": "string", "pattern": "^REQ-\\d{4}$" },
            "by":      { "type": "string", "enum": ["composition","inspection"] },
            "notes":   { "type": "string" },
            "cites":   { "type": "array", "items": { "type": "string" }, "description": "Test names or REQ-IDs supporting the claim; prepended to notes." },
            "promote": { "type": "boolean", "default": false }
        }
    })
}

fn batch_schema() -> Value {
    json!({
        "type": "object",
        "required": ["mutations"],
        "properties": {
            "reason":    { "type": "string", "description": "Default reason applied to each mutation that omits its own." },
            "mutations": {
                "type": "array",
                "description": "List of mutations: add, update, delete, link. See `req schema batch` for full shape.",
                "items": { "type": "object" }
            }
        }
    })
}

fn import_schema() -> Value {
    json!({
        "type": "object",
        "required": ["format", "source"],
        "properties": {
            "format":  { "type": "string", "enum": ["markdown","json"] },
            "source":  { "type": "string", "description": "Path to the file to ingest." },
            "dry_run": { "type": "boolean", "default": false },
            "strict":  { "type": "boolean", "default": false, "description": "Abort the import on the first invalid item." }
        }
    })
}

fn schema_which_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "which": { "type": "string", "enum": ["add","batch","import"], "default": "add" }
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
        "req_status" => tool_status(file),
        "req_next" => tool_next(args, file),
        "req_check" => tool_check(args, file),
        "req_diff" => tool_diff(args, file),
        "req_stale" => tool_stale(args, file),
        "req_audit" => tool_audit(args, file),
        "req_test_record" => tool_test_record(args, file),
        "req_test_run" => tool_test_run(args, file),
        "req_verify" => tool_verify(args, file),
        "req_batch" => tool_batch(args, file),
        "req_import" => tool_import(args, file),
        "req_schema" => tool_schema(args),
        "req_doctor" => tool_doctor(),
        "req_version" => tool_version(),
        "req_migrate" => tool_migrate(file),
        "req_review" => tool_review(args, file),
        "req_split" => tool_split(args, file),
        "req_lint" => tool_lint(args, file),   // REQ-0101
        "req_brief" => tool_brief(args, file), // REQ-0104
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
            findings.push(json!({
                "id": id,
                "rule_code": f.rule_code,
                "level": if f.error { "error" } else { "warning" },
                "field": f.field,
                "message": f.message,
            }));
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
        "csv" => crate::commands::export::to_csv(&project),
        "html" => Ok(crate::commands::export::to_html(&project)),
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

// ---------- agent-facing tools (v0.1 additions) ----------

fn tool_status(file: &Path) -> Result<String> {
    let project = storage::load(file)?;
    let total = project.requirements.len();
    let mut counts = [0usize; 6];
    for r in project.requirements.values() {
        let i = match r.status {
            Status::Draft => 0,
            Status::Proposed => 1,
            Status::Approved => 2,
            Status::Implemented => 3,
            Status::Verified => 4,
            Status::Obsolete => 5,
        };
        counts[i] += 1;
    }
    let non_obsolete = total - counts[5];
    let done = counts[3] + counts[4];
    let pct = if non_obsolete == 0 {
        0.0
    } else {
        100.0 * done as f64 / non_obsolete as f64
    };
    Ok(serde_json::to_string_pretty(&json!({
        "project": project.name,
        "total": total,
        "by_status": {
            "draft": counts[0], "proposed": counts[1], "approved": counts[2],
            "implemented": counts[3], "verified": counts[4], "obsolete": counts[5],
        },
        "delivery_progress_pct": (pct * 10.0).round() / 10.0,
        "non_obsolete": non_obsolete,
        "done": done,
    }))?)
}

fn tool_next(args: &Value, file: &Path) -> Result<String> {
    let project = storage::load(file)?;
    let status: Option<Status> = s(args, "status").map(|s| parse_status(&s)).transpose()?;
    let kind: Option<Kind> = s(args, "kind").map(|s| parse_kind(&s)).transpose()?;
    let priority: Option<Priority> = s(args, "priority")
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

    let deps_satisfied = |r: &crate::model::Requirement| {
        r.links
            .iter()
            .filter(|l| matches!(l.kind, LinkKind::DependsOn))
            .all(|l| {
                project
                    .requirements
                    .get(&l.target)
                    .is_some_and(|d| matches!(d.status, Status::Implemented | Status::Verified))
            })
    };

    let mut candidates: Vec<&crate::model::Requirement> = project
        .requirements
        .values()
        .filter(|r| !matches!(r.status, Status::Obsolete))
        .filter(|r| status.is_none_or(|s| r.status == s))
        .filter(|r| kind.is_none_or(|k| r.kind == k))
        .filter(|r| priority.is_none_or(|p| r.priority == p))
        .filter(|r| tags.iter().all(|t| r.tags.iter().any(|rt| rt == t)))
        .filter(|r| deps_satisfied(r))
        .collect();
    candidates.sort_by_key(|r| {
        let p = match r.priority {
            Priority::Must => 0,
            Priority::Should => 1,
            Priority::Could => 2,
            Priority::Wont => 3,
        };
        let st = match r.status {
            Status::Draft => 0,
            Status::Proposed => 1,
            Status::Approved => 2,
            Status::Implemented => 3,
            Status::Verified => 4,
            Status::Obsolete => 5,
        };
        (p, st, r.id.clone())
    });
    Ok(match candidates.first() {
        Some(r) => serde_json::to_string_pretty(r)?,
        None => json!({ "found": false, "message": "no requirement matches the filters with all dependencies satisfied" }).to_string(),
    })
}

fn tool_check(args: &Value, file: &Path) -> Result<String> {
    let base = req_s(args, "base")?;
    let path = s(args, "path").unwrap_or_else(|| ".".into());
    let current = storage::load(file)?;
    let filename = file
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| anyhow!("project file has no name component"))?;
    let spec = format!("{}:{}", base, filename);
    let out = std::process::Command::new("git")
        .args(["show", &spec])
        .output()
        .with_context(|| format!("git show {}", spec))?;
    if !out.status.success() {
        return Err(anyhow!(
            "git show {} failed: {}",
            spec,
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let tmp = std::env::temp_dir().join(format!("req-mcp-check-{}.req", std::process::id()));
    std::fs::write(&tmp, &out.stdout)?;
    let base_proj = storage::load_with_options(&tmp, true)?;
    std::fs::remove_file(&tmp).ok();

    let changed_ids: Vec<String> = current
        .requirements
        .iter()
        .filter(|(id, r)| match base_proj.requirements.get(*id) {
            None => true,
            Some(prev) => {
                prev.updated != r.updated
                    || prev.title != r.title
                    || prev.statement != r.statement
                    || prev.rationale != r.rationale
                    || prev.acceptance != r.acceptance
                    || prev.status != r.status
                    || prev.priority != r.priority
                    || prev.kind != r.kind
                    || prev.links.len() != r.links.len()
            }
        })
        .map(|(id, _)| id.clone())
        .collect();
    let mut errors = 0usize;
    let mut warnings = 0usize;
    let mut findings: Vec<Value> = Vec::new();
    for id in &changed_ids {
        if let Some(r) = current.requirements.get(id) {
            for f in validate::validate_requirement(r) {
                if f.error {
                    errors += 1
                } else {
                    warnings += 1
                }
                findings.push(json!({
                    "req_id": id, "rule_code": f.rule_code, "field": f.field,
                    "severity": if f.error { "error" } else { "warning" },
                    "message": f.message,
                }));
            }
        }
    }
    let changed_files: Vec<String> = std::process::Command::new("git")
        .args(["diff", "--name-only", &format!("{}...HEAD", base)])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                Some(
                    String::from_utf8_lossy(&o.stdout)
                        .lines()
                        .map(|l| l.trim().to_string())
                        .filter(|l| !l.is_empty())
                        .collect(),
                )
            } else {
                None
            }
        })
        .unwrap_or_default();
    Ok(serde_json::to_string_pretty(&json!({
        "ok": errors == 0,
        "base": base,
        "path": path,
        "changed_requirements": changed_ids,
        "errors": errors,
        "warnings": warnings,
        "findings": findings,
        "changed_files": changed_files,
    }))?)
}

fn tool_diff(args: &Value, file: &Path) -> Result<String> {
    let spec = req_s(args, "spec")?;
    let (base_ref, head_ref) = match spec.split_once("..") {
        Some((b, h)) => (
            b.trim(),
            if h.trim().is_empty() {
                "HEAD"
            } else {
                h.trim()
            },
        ),
        // Single ref means BASE..HEAD, matching the CLI shorthand.
        None => (spec.trim(), "HEAD"),
    };
    if base_ref.is_empty() {
        return Err(anyhow!(
            "diff spec needs a base ref; pass `BASE..HEAD`, `BASE..`, or a single `BASE`"
        ));
    }
    let filename = file
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| anyhow!("project file has no name"))?;
    let load_ref = |r: &str| -> Result<crate::model::Project> {
        let s = format!("{}:{}", r, filename);
        let out = std::process::Command::new("git")
            .args(["show", &s])
            .output()?;
        if !out.status.success() {
            return Err(anyhow!(
                "git show {} failed: {}",
                s,
                String::from_utf8_lossy(&out.stderr)
            ));
        }
        let tmp = std::env::temp_dir().join(format!(
            "req-mcp-diff-{}-{}.req",
            r.replace('/', "_"),
            std::process::id()
        ));
        std::fs::write(&tmp, &out.stdout)?;
        let p = storage::load_with_options(&tmp, true)?;
        std::fs::remove_file(&tmp).ok();
        Ok(p)
    };
    let base = load_ref(base_ref)?;
    let head = load_ref(head_ref)?;

    let mut added: Vec<String> = Vec::new();
    let mut removed: Vec<String> = Vec::new();
    let mut changed: Vec<Value> = Vec::new();
    let base_ids: std::collections::BTreeSet<&String> = base.requirements.keys().collect();
    let head_ids: std::collections::BTreeSet<&String> = head.requirements.keys().collect();
    for id in head_ids.difference(&base_ids) {
        added.push((*id).clone());
    }
    for id in base_ids.difference(&head_ids) {
        removed.push((*id).clone());
    }
    for id in base_ids.intersection(&head_ids) {
        let b = &base.requirements[*id];
        let h = &head.requirements[*id];
        let mut t: Vec<String> = Vec::new();
        if b.title != h.title {
            t.push("title changed".to_string());
        }
        if b.status != h.status {
            t.push(format!(
                "status: {} -> {}",
                b.status.as_str(),
                h.status.as_str()
            ));
        }
        if b.priority != h.priority {
            t.push(format!(
                "priority: {} -> {}",
                b.priority.as_str(),
                h.priority.as_str()
            ));
        }
        if b.kind != h.kind {
            t.push(format!("kind: {} -> {}", b.kind.as_str(), h.kind.as_str()));
        }
        if b.statement != h.statement {
            t.push("statement changed".into());
        }
        if b.rationale != h.rationale {
            t.push("rationale changed".into());
        }
        if b.acceptance != h.acceptance {
            t.push(format!(
                "acceptance: {} -> {} items",
                b.acceptance.len(),
                h.acceptance.len()
            ));
        }
        if b.links.len() != h.links.len() {
            t.push(format!("links: {} -> {}", b.links.len(), h.links.len()));
        }
        if !t.is_empty() {
            let reason = h
                .history
                .iter()
                .rev()
                .find_map(|hist| hist.reason.clone())
                .unwrap_or_default();
            changed.push(json!({ "id": id, "transitions": t, "reason": reason }));
        }
    }
    Ok(serde_json::to_string_pretty(&json!({
        "spec": spec, "added": added, "removed": removed, "changed": changed,
        "empty": added.is_empty() && removed.is_empty() && changed.is_empty(),
    }))?)
}

fn tool_stale(args: &Value, file: &Path) -> Result<String> {
    use once_cell::sync::Lazy;
    use regex::Regex;
    static REQ_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"REQ-\d{4}").unwrap());

    let project = storage::load(file)?;
    let root = std::path::PathBuf::from(s(args, "path").unwrap_or_else(|| ".".into()));
    let only_stale = args
        .get("only_stale")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let head = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
            } else {
                None
            }
        });

    let mut rows: Vec<Value> = Vec::new();
    let mut summary = (0usize, 0usize, 0usize, 0usize, 0usize); // fresh, drifted, stale, no_records, unknown
    for r in project.requirements.values() {
        let Some(latest) = r.tests.last() else {
            summary.3 += 1;
            if !only_stale {
                rows.push(json!({ "id": r.id, "state": "no-records", "record_commit": "—", "changed_files": Vec::<String>::new() }));
            }
            continue;
        };
        // Find linked files
        let mut linked: Vec<String> = Vec::new();
        scan_for_marker(&root, &r.id, &REQ_RE, &mut linked);
        let head_str = match &head {
            Some(h) => h,
            None => {
                summary.4 += 1;
                if !only_stale {
                    rows.push(json!({ "id": r.id, "state": "unknown", "record_commit": short_sha(&latest.commit), "changed_files": Vec::<String>::new() }));
                }
                continue;
            }
        };
        if *head_str == latest.commit {
            summary.0 += 1;
            if !only_stale {
                rows.push(json!({ "id": r.id, "state": "fresh", "record_commit": short_sha(&latest.commit), "changed_files": Vec::<String>::new() }));
            }
            continue;
        }
        // git diff --name-only RECORD..HEAD limited to linked files
        let changed = git_diff_names(&latest.commit);
        let overlap: Vec<String> = linked
            .iter()
            .filter(|f| {
                changed
                    .iter()
                    .any(|c| c.replace('\\', "/").ends_with(f.as_str()) || f.ends_with(c))
            })
            .cloned()
            .collect();
        if overlap.is_empty() {
            summary.1 += 1;
            if !only_stale {
                rows.push(json!({ "id": r.id, "state": "drifted", "record_commit": short_sha(&latest.commit), "changed_files": Vec::<String>::new() }));
            }
        } else {
            summary.2 += 1;
            rows.push(json!({ "id": r.id, "state": "STALE", "record_commit": short_sha(&latest.commit), "changed_files": overlap }));
        }
    }
    Ok(serde_json::to_string_pretty(&json!({
        "summary": {
            "fresh": summary.0, "drifted": summary.1, "stale": summary.2,
            "no_records": summary.3, "unknown": summary.4,
        },
        "rows": rows,
    }))?)
}

fn short_sha(s: &str) -> String {
    s.chars().take(9).collect()
}

fn git_diff_names(from: &str) -> Vec<String> {
    std::process::Command::new("git")
        .args(["diff", "--name-only", &format!("{}..HEAD", from)])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                Some(
                    String::from_utf8_lossy(&o.stdout)
                        .lines()
                        .map(|l| l.trim().to_string())
                        .filter(|l| !l.is_empty())
                        .collect(),
                )
            } else {
                None
            }
        })
        .unwrap_or_default()
}

fn scan_for_marker(root: &Path, req_id: &str, re: &regex::Regex, hits: &mut Vec<String>) {
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
    const EXTS: &[&str] = &[
        "rs", "py", "js", "ts", "tsx", "go", "java", "md", "toml", "c", "cpp", "h",
    ];
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name_s = name.to_string_lossy();
        if path.is_dir() {
            if SKIP.iter().any(|s| *s == name_s.as_ref()) {
                continue;
            }
            scan_for_marker(&path, req_id, re, hits);
        } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            if !EXTS.contains(&ext) {
                continue;
            }
            if let Ok(text) = std::fs::read_to_string(&path) {
                if re.find_iter(&text).any(|m| m.as_str() == req_id) {
                    hits.push(path.to_string_lossy().replace('\\', "/"));
                }
            }
        }
    }
}

fn tool_audit(args: &Value, file: &Path) -> Result<String> {
    let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(50);
    let gate = args.get("gate").and_then(Value::as_bool).unwrap_or(false);
    let require_good = args
        .get("require_good_signature")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let signers: Vec<String> = args
        .get("required_signers")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(|s| s.to_string())
                .collect()
        })
        .unwrap_or_default();
    let fmt = "%H|||%aI|||%aN|||%G?|||%GS|||%s";
    let out = std::process::Command::new("git")
        .args([
            "log",
            "--follow",
            &format!("-n{}", limit),
            &format!("--format={}", fmt),
            "--",
        ])
        .arg(file)
        .output()
        .context("git log")?;
    if !out.status.success() {
        return Err(anyhow!(
            "git log failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut entries: Vec<Value> = Vec::new();
    let mut violations: Vec<Value> = Vec::new();
    for line in text.lines() {
        let p: Vec<&str> = line.splitn(6, "|||").collect();
        if p.len() != 6 {
            continue;
        }
        let status = match p[3] {
            "G" => "good",
            "B" => "bad",
            "U" => "good-unknown",
            "X" => "expired",
            "R" => "revoked",
            "E" => "cannot-check",
            "N" | "" => "no-signature",
            _ => "unknown",
        };
        let entry = json!({
            "commit": p[0], "date": p[1], "author": p[2],
            "signature_status": status, "signer": p[4], "subject": p[5],
        });
        if gate {
            let mut why: Vec<String> = Vec::new();
            if require_good && !matches!(status, "good" | "good-unknown") {
                why.push(format!("signature status '{}' is not 'good'", status));
            }
            if !signers.is_empty() {
                let signer_lc = p[4].to_lowercase();
                if !signers
                    .iter()
                    .any(|s| signer_lc.contains(&s.to_lowercase()))
                {
                    why.push(format!("signer '{}' not in required list", p[4]));
                }
            }
            if !why.is_empty() {
                violations.push(json!({ "commit": p[0], "signer": p[4], "signature_status": status, "subject": p[5], "why": why }));
            }
        }
        entries.push(entry);
    }
    Ok(serde_json::to_string_pretty(&json!({
        "ok": !gate || violations.is_empty(),
        "entries": entries,
        "violations": violations,
    }))?)
}

fn tool_test_record(args: &Value, file: &Path) -> Result<String> {
    let id = req_s(args, "id")?;
    let result_s = req_s(args, "result")?;
    let notes = s(args, "notes").unwrap_or_default();
    let outcome = match result_s.as_str() {
        "pass" => crate::model::TestOutcome::Pass,
        "fail" => crate::model::TestOutcome::Fail,
        other => return Err(anyhow!("unknown result: {}", other)),
    };
    let head = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .context("git rev-parse HEAD")?;
    if !head.status.success() {
        return Err(anyhow!(
            "not in a git working tree — cannot record a test run without a commit SHA"
        ));
    }
    let commit = String::from_utf8_lossy(&head.stdout).trim().to_string();
    let (path, mut project, _lock) = crate::storage::load_for_mutation(&Some(file.to_path_buf()))?;
    let r = project
        .requirements
        .get_mut(&id)
        .ok_or_else(|| anyhow!("no such requirement: {}", id))?;
    // REQ-0112: auto-discover linked files + content-hash them.
    let auto_linked = super::commands::test_cmd::auto_linked_files(&id, std::path::Path::new("."));
    let content_hash = if auto_linked.is_empty() {
        None
    } else {
        Some(super::commands::test_cmd::hash_files(&auto_linked))
    };
    let record = crate::model::TestRecord {
        at: Utc::now(),
        actor: super::commands::current_actor(),
        commit: commit.clone(),
        outcome,
        notes: notes.clone(),
        kind: crate::model::EvidenceKind::Automated,
        content_hash,
        linked_files: if auto_linked.is_empty() {
            None
        } else {
            Some(
                auto_linked
                    .iter()
                    .map(|p| p.to_string_lossy().to_string())
                    .collect(),
            )
        },
    };
    r.tests.push(record);
    r.history.push(crate::commands::history(
        format!(
            "test {} recorded against commit {} via MCP",
            outcome.as_str(),
            short_sha(&commit)
        ),
        Some(notes.clone()).filter(|s| !s.is_empty()),
    ));
    r.updated = Utc::now();
    project.updated = Utc::now();
    crate::storage::save(&path, &project)?;
    Ok(json!({ "id": id, "outcome": outcome.as_str(), "commit": commit }).to_string())
}

fn tool_test_run(args: &Value, file: &Path) -> Result<String> {
    use once_cell::sync::Lazy;
    use regex::Regex;
    static TEST_LINE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"(?m)^test\s+(?:[\w:]+::)?(req_(\d{4})\w*)\s+\.\.\.\s+(ok|FAILED|ignored)")
            .unwrap()
    });
    let from_file = req_s(args, "from_file")?;
    let dry_run = args
        .get("dry_run")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let promote = args
        .get("promote")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let body = std::fs::read_to_string(&from_file)
        .with_context(|| format!("read from_file {}", from_file))?;
    let head = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
            } else {
                None
            }
        });

    use std::collections::BTreeMap;
    type RunBuckets = (Vec<String>, Vec<String>, Vec<String>);
    let mut by_req: BTreeMap<String, RunBuckets> = BTreeMap::new();
    for cap in TEST_LINE.captures_iter(&body) {
        let test_name = cap[1].to_string();
        let id = format!("REQ-{}", &cap[2]);
        let bucket = by_req.entry(id).or_default();
        match &cap[3] {
            "ok" => bucket.0.push(test_name),
            "FAILED" => bucket.1.push(test_name),
            "ignored" => bucket.2.push(test_name),
            _ => {}
        }
    }
    if by_req.is_empty() {
        return Ok(
            json!({ "matched": 0, "message": "no test names matched the req_NNNN_* convention" })
                .to_string(),
        );
    }
    let mut summary: Vec<Value> = Vec::new();
    let mut promoted: Vec<String> = Vec::new();
    let (path, mut project, _lock) = crate::storage::load_for_mutation(&Some(file.to_path_buf()))?;
    let actor = crate::commands::current_actor();
    let commit = head.clone().unwrap_or_else(|| "(no git)".into());
    for (id, (passed, failed, ignored)) in &by_req {
        let exists = project.requirements.contains_key(id);
        let outcome = if !failed.is_empty() {
            crate::model::TestOutcome::Fail
        } else {
            crate::model::TestOutcome::Pass
        };
        let notes = format!(
            "cargo test: {} pass / {} fail / {} ignored",
            passed.len(),
            failed.len(),
            ignored.len()
        );
        summary.push(json!({
            "req_id": id, "exists": exists, "outcome": outcome.as_str(),
            "passed": passed.len(), "failed": failed.len(), "ignored": ignored.len(),
        }));
        if exists && !dry_run {
            let r = project.requirements.get_mut(id).unwrap();
            let auto_linked =
                super::commands::test_cmd::auto_linked_files(id, std::path::Path::new("."));
            let content_hash = if auto_linked.is_empty() {
                None
            } else {
                Some(super::commands::test_cmd::hash_files(&auto_linked))
            };
            r.tests.push(crate::model::TestRecord {
                at: Utc::now(),
                actor: actor.clone(),
                commit: commit.clone(),
                outcome,
                notes,
                kind: crate::model::EvidenceKind::Automated,
                content_hash,
                linked_files: if auto_linked.is_empty() {
                    None
                } else {
                    Some(
                        auto_linked
                            .iter()
                            .map(|p| p.to_string_lossy().to_string())
                            .collect(),
                    )
                },
            });
            r.history.push(crate::commands::history(
                format!("test {} recorded via MCP req_test_run", outcome.as_str()),
                None,
            ));
            r.updated = Utc::now();
            if promote
                && matches!(outcome, crate::model::TestOutcome::Pass)
                && !matches!(r.status, Status::Verified | Status::Obsolete)
            {
                r.status = Status::Verified;
                r.history.push(crate::commands::history(
                    "status promoted to verified (req_test_run promote)",
                    None,
                ));
                promoted.push(id.clone());
            }
        }
    }
    if !dry_run {
        project.updated = Utc::now();
        crate::storage::save(&path, &project)?;
    }
    Ok(serde_json::to_string_pretty(&json!({
        "ok": true, "dry_run": dry_run, "matched_requirements": summary.len(),
        "promoted": promoted, "results": summary,
    }))?)
}

fn tool_verify(args: &Value, file: &Path) -> Result<String> {
    let id = req_s(args, "id")?;
    let by = req_s(args, "by")?;
    let notes = req_s(args, "notes")?;
    let promote = args
        .get("promote")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let cites: Vec<String> = args
        .get("cites")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(|s| s.to_string())
                .collect()
        })
        .unwrap_or_default();
    let kind = match by.as_str() {
        "composition" => crate::model::EvidenceKind::Composition,
        "inspection" => crate::model::EvidenceKind::Inspection,
        other => return Err(anyhow!("unknown verification kind: {}", other)),
    };
    let commit = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
            } else {
                None
            }
        })
        .unwrap_or_else(|| "(no git)".into());
    let prefix = if cites.is_empty() {
        String::new()
    } else {
        format!("cites: {} — ", cites.join(", "))
    };
    let (path, mut project, _lock) = crate::storage::load_for_mutation(&Some(file.to_path_buf()))?;
    let r = project
        .requirements
        .get_mut(&id)
        .ok_or_else(|| anyhow!("no such requirement: {}", id))?;
    r.tests.push(crate::model::TestRecord {
        at: Utc::now(),
        actor: crate::commands::current_actor(),
        commit: commit.clone(),
        outcome: crate::model::TestOutcome::Pass,
        notes: format!("{}{}", prefix, notes),
        kind,
        content_hash: None,
        linked_files: None,
    });
    r.history.push(crate::commands::history(
        format!(
            "{} evidence recorded via MCP against commit {}",
            kind.as_str(),
            short_sha(&commit)
        ),
        Some(notes.clone()),
    ));
    r.updated = Utc::now();
    let force = args.get("force").and_then(Value::as_bool).unwrap_or(false);
    let mut promoted = false;
    if promote {
        let eligible = matches!(r.status, Status::Implemented);
        if eligible || force {
            if !matches!(r.status, Status::Verified | Status::Obsolete) {
                r.status = Status::Verified;
                r.history.push(crate::commands::history(
                    format!("status promoted to verified ({} evidence)", kind.as_str()),
                    None,
                ));
                promoted = true;
            }
        } else if !matches!(r.status, Status::Verified | Status::Obsolete) {
            return Err(anyhow!(
                "{} is at status '{}'; --promote only auto-promotes from \
                 'implemented'. Move it to implemented first, or pass force=true.",
                id,
                r.status.as_str()
            ));
        }
    }
    project.updated = Utc::now();
    crate::storage::save(&path, &project)?;
    Ok(
        json!({ "id": id, "kind": kind.as_str(), "commit": commit, "promoted": promoted })
            .to_string(),
    )
}

fn tool_batch(args: &Value, file: &Path) -> Result<String> {
    // Re-serialize the mutations array into a temp file so we can pipe it
    // through the existing batch parser without copying ~250 lines of
    // mutation-application logic. The CLI shape is identical.
    let _ = args
        .get("mutations")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("batch: 'mutations' array is required"))?;
    let tmp = std::env::temp_dir().join(format!("req-mcp-batch-{}.json", std::process::id()));
    std::fs::write(&tmp, serde_json::to_string(args)?)?;
    // Shell out to ourselves for the apply step. This guarantees that the
    // CLI and MCP paths apply mutations through identical code.
    let out = std::process::Command::new(std::env::current_exe()?)
        .arg("--file")
        .arg(file)
        .arg("batch")
        .arg(&tmp)
        .arg("--json")
        .output()
        .context("invoke self for batch apply")?;
    let _ = std::fs::remove_file(&tmp);
    if !out.status.success() {
        return Err(anyhow!("{}", first_envelope_line(&out.stdout, &out.stderr)));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// When a `req --json` subprocess fails it writes the single-line JSON
/// envelope (`{"code":...}`) to stdout; stderr is normally empty.
/// Either stream might also carry an anyhow chain in non-JSON paths,
/// so we scan both for the first envelope-shaped line and fall back to
/// the first non-empty line otherwise. Keeps MCP error text parseable
/// as JSON in one go.
fn first_envelope_line(stdout: &[u8], stderr: &[u8]) -> String {
    for stream in [stdout, stderr] {
        let text = String::from_utf8_lossy(stream);
        for line in text.lines() {
            let t = line.trim();
            if t.starts_with('{') && t.ends_with('}') {
                return t.to_string();
            }
        }
    }
    for stream in [stdout, stderr] {
        let text = String::from_utf8_lossy(stream);
        if let Some(line) = text.lines().map(str::trim).find(|l| !l.is_empty()) {
            return line.to_string();
        }
    }
    String::new()
}

fn tool_import(args: &Value, file: &Path) -> Result<String> {
    let format = req_s(args, "format")?;
    let source = req_s(args, "source")?;
    let dry_run = args
        .get("dry_run")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let strict = args.get("strict").and_then(Value::as_bool).unwrap_or(false);
    let mut argv: Vec<std::ffi::OsString> = vec![
        "--file".into(),
        file.as_os_str().into(),
        "import".into(),
        "-f".into(),
        format.into(),
        source.into(),
        "--json".into(),
    ];
    if dry_run {
        argv.push("--dry-run".into());
    }
    if strict {
        argv.push("--strict".into());
    }
    let out = std::process::Command::new(std::env::current_exe()?)
        .args(&argv)
        .output()
        .context("invoke self for import")?;
    if !out.status.success() {
        return Err(anyhow!("{}", first_envelope_line(&out.stdout, &out.stderr)));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn tool_schema(args: &Value) -> Result<String> {
    let which = s(args, "which").unwrap_or_else(|| "add".into());
    let argv = vec!["schema".to_string(), which];
    let out = std::process::Command::new(std::env::current_exe()?)
        .args(&argv)
        .output()
        .context("invoke self for schema")?;
    if !out.status.success() {
        return Err(anyhow!("{}", first_envelope_line(&out.stdout, &out.stderr)));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn tool_doctor() -> Result<String> {
    let out = std::process::Command::new(std::env::current_exe()?)
        .args(["doctor", "--json"])
        .output()
        .context("invoke self for doctor")?;
    // doctor exits non-zero on failed checks; we still want to return the
    // JSON body so the caller can act on it. Treat any output as success.
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn tool_version() -> Result<String> {
    Ok(serde_json::to_string_pretty(&json!({
        "name": "req",
        "package": env!("CARGO_PKG_NAME"),
        "version": env!("CARGO_PKG_VERSION"),
        "file_format": crate::storage::FORMAT_TAG,
        "mcp_protocol": PROTOCOL_VERSION,
    }))?)
}

fn tool_migrate(file: &Path) -> Result<String> {
    let out = std::process::Command::new(std::env::current_exe()?)
        .arg("--file")
        .arg(file)
        .args(["migrate", "--json"])
        .output()
        .context("invoke self for migrate")?;
    if !out.status.success() {
        return Err(anyhow!("{}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    // CLI prints text by default; we asked for --json so this is the structured form
    // (when implemented; current stub prints text). Return either way.
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn tool_review(args: &Value, file: &Path) -> Result<String> {
    let base = s(args, "base").unwrap_or_else(|| "origin/main".into());
    let path = s(args, "path").unwrap_or_else(|| ".".into());
    let json = args.get("json").and_then(Value::as_bool).unwrap_or(true);
    let gate = args.get("gate").and_then(Value::as_bool).unwrap_or(false);
    let staged = args.get("staged").and_then(Value::as_bool).unwrap_or(false);
    let new = args.get("new").and_then(Value::as_bool).unwrap_or(false);
    let all = args.get("all").and_then(Value::as_bool).unwrap_or(false);
    let mut argv: Vec<std::ffi::OsString> = vec![
        "--file".into(),
        file.as_os_str().into(),
        "review".into(),
        "--base".into(),
        base.into(),
        "--path".into(),
        path.into(),
    ];
    if json {
        argv.push("--json".into());
    }
    if gate {
        argv.push("--gate".into());
    }
    if staged {
        argv.push("--staged".into());
    }
    if new {
        argv.push("--new".into());
    }
    if all {
        argv.push("--all".into());
    }
    let out = std::process::Command::new(std::env::current_exe()?)
        .args(&argv)
        .output()
        .context("invoke self for review")?;
    if !out.status.success() {
        return Err(anyhow!("{}", first_envelope_line(&out.stdout, &out.stderr)));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

// REQ-0104: req_brief MCP tool implementation.
fn tool_brief(args: &Value, file: &Path) -> Result<String> {
    let full = args.get("full").and_then(Value::as_bool).unwrap_or(false);
    let json = args.get("json").and_then(Value::as_bool).unwrap_or(true);
    let mut argv: Vec<std::ffi::OsString> =
        vec!["--file".into(), file.as_os_str().into(), "brief".into()];
    if full {
        argv.push("--full".into());
    }
    if json {
        argv.push("--json".into());
    }
    let out = std::process::Command::new(std::env::current_exe()?)
        .args(&argv)
        .output()
        .context("invoke self for brief")?;
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

// REQ-0101: req_lint MCP tool implementation.
fn tool_lint(args: &Value, file: &Path) -> Result<String> {
    let path = s(args, "path").unwrap_or_else(|| ".".into());
    let json = args.get("json").and_then(Value::as_bool).unwrap_or(true);
    let mut argv: Vec<std::ffi::OsString> = vec![
        "--file".into(),
        file.as_os_str().into(),
        "lint".into(),
        "--path".into(),
        path.into(),
    ];
    if json {
        argv.push("--json".into());
    }
    let out = std::process::Command::new(std::env::current_exe()?)
        .args(&argv)
        .output()
        .context("invoke self for lint")?;
    // lint exits non-zero only on validator errors; treat any output as
    // the report and let the caller inspect it.
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn tool_split(args: &Value, file: &Path) -> Result<String> {
    let id = req_s(args, "id")?;
    let into = args
        .get("into")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("'into' must be an array of new statements (length >= 2)"))?;
    if into.len() < 2 {
        return Err(anyhow!("'into' must have at least 2 statements"));
    }
    let keep_original = args
        .get("keep_original")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let reason = s(args, "reason");
    let mut argv: Vec<std::ffi::OsString> = vec![
        "--file".into(),
        file.as_os_str().into(),
        "split".into(),
        id.into(),
        "--json".into(),
    ];
    for stmt in into {
        let s = stmt
            .as_str()
            .ok_or_else(|| anyhow!("'into' entries must be strings"))?;
        argv.push("--into".into());
        argv.push(s.into());
    }
    if keep_original {
        argv.push("--keep-original".into());
    }
    if let Some(r) = reason {
        argv.push("--reason".into());
        argv.push(r.into());
    }
    let out = std::process::Command::new(std::env::current_exe()?)
        .args(&argv)
        .output()
        .context("invoke self for split")?;
    if !out.status.success() {
        return Err(anyhow!("{}", first_envelope_line(&out.stdout, &out.stderr)));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
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
