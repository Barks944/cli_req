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

// REQ-0139: the agent guidance steers verification through the dossier.
const SERVER_GUIDANCE: &str = "\
This is the `req` MCP server for managed requirements. When the user describes \
new behaviour the system should have, call `req_add`. Before starting work on \
a feature call `req_list` and `req_show`. Before declaring work complete call \
`req_validate`. To VALIDATE a requirement (REQ-NNNN or SR-NNNN) and move it to \
Verified, do NOT one-shot it: walk the validation dossier — `req_validation_plan` \
(how you'll validate), then `req_validation_analysis` (code review + result), then \
`req_validation_test` (testing + result), then `req_validation_conclude` (statement \
+ derived verdict, promote=true to flip to Verified). Promotion is BLOCKED without a \
passing dossier. Never read project.req directly — its integrity hash will \
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
        name: "req_test_list",
        description: "List the test-record history attached to one requirement: each record's timestamp, commit SHA, outcome (pass/fail), evidence kind (test/composition/inspection), and notes, oldest first. Read-only — the agent-facing counterpart to `req_test_record`/`req_test_run`. Use to see what verification a requirement already carries before re-testing or promoting it.",
        schema: test_list_schema,
    },
    ToolDef {
        name: "req_verify",
        description: "Record a composition or inspection evidence record on a requirement, optionally promoting to Verified. Composition cites another requirement's tests; inspection records a human review. NOTE: promote=true now REQUIRES a passing validation dossier (see req_validation_*) — for the full staged validation prefer req_validation_conclude. An ordinary requirement may instead carry a `validation-exempt` tag, or you may pass no_dossier=true with a reason to record an audited exemption.",
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
    // REQ-0134: functional-safety tools (IEC 61508). Hazards -> safety
    // functions -> safety requirements, with derived SILs.
    ToolDef {
        name: "req_hazard_add",
        description: "Create a hazard (HAZ-NNNN). Use this when a new hazard is identified. REQUIRED: title, harm (free-text narrative of the potential harm, e.g. \"an operator's hand could be severed\"). Optional: description, context (operational situation), and the four IEC 61508 risk-graph parameters consequence (C_A..C_D), frequency (F_A/F_B), avoidance (P_A/P_B), probability (W1..W3). Supply all four to risk-assess on creation; omit them to log as Identified and assess later. The SIL is DERIVED from C/F/P/W — never pass a SIL.",
        schema: hazard_add_schema,
    },
    ToolDef {
        name: "req_hazard_list",
        description: "List hazards with their derived required SIL and status. Optional filters: sil (e.g. SIL3), status, unmitigated=true (only hazards with no mitigating safety function). Call this to review the hazard log.",
        schema: hazard_list_schema,
    },
    ToolDef {
        name: "req_hazard_show",
        description: "Return one hazard in full, including its derived SIL and the safety functions that mitigate it. Pass id (HAZ-NNNN).",
        schema: id_schema,
    },
    ToolDef {
        name: "req_hazard_assess",
        description: "Set a hazard's C/F/P/W risk parameters; this derives the required SIL and advances the hazard to Assessed. REQUIRED: id, consequence (C_A..C_D), frequency (F_A/F_B), avoidance (P_A/P_B), probability (W1..W3). Pass reason for the history.",
        schema: hazard_assess_schema,
    },
    ToolDef {
        name: "req_hazard_update",
        description: "Update a hazard's title/description/context/harm/status. Pass id and reason. Use status to move it through Identified -> Assessed -> Mitigated -> Verified -> Obsolete.",
        schema: hazard_update_schema,
    },
    ToolDef {
        name: "req_sf_add",
        description: "Create a safety function (SF-NNNN) — the risk-reduction measure that brings a hazard to a safe state. REQUIRED: title. Optional: description, safe_state (the state it maintains), mitigates (array of HAZ-NNNN it covers). Its allocated SIL is DERIVED as the max required SIL of the hazards it mitigates.",
        schema: sf_add_schema,
    },
    ToolDef {
        name: "req_sf_list",
        description: "List safety functions with their derived allocated SIL and status. Optional filters: sil, status, unrealized=true (only those with no realizing safety requirement).",
        schema: sf_list_schema,
    },
    ToolDef {
        name: "req_sf_show",
        description: "Return one safety function in full: allocated SIL, the hazards it mitigates, and the safety requirements that realize it. Pass id (SF-NNNN).",
        schema: id_schema,
    },
    ToolDef {
        name: "req_sf_update",
        description: "Modify a safety function's title/description/safe_state/status. Set id and reason.",
        schema: sf_update_schema,
    },
    ToolDef {
        name: "req_sf_mitigate",
        description: "Record that a safety function mitigates a hazard (SF -> HAZ). REQUIRED: sf (SF-NNNN), hazard (HAZ-NNNN). Set remove=true to unlink. Adding the first mitigation advances the hazard to Mitigated.",
        schema: sf_mitigate_schema,
    },
    ToolDef {
        name: "req_sreq_add",
        description: "Add a safety requirement (SR-NNNN) that realizes a safety function. REQUIRED: title, statement (use shall/must), rationale. Optional: acceptance (array), priority (default must), realizes (array of SF-NNNN). It inherits its SIL from the safety function it realizes; that SIL governs how rigorously it must be verified.",
        schema: sreq_add_schema,
    },
    ToolDef {
        name: "req_sreq_list",
        description: "List safety requirements with their inherited SIL and status. Optional filters: sil, status, unverified=true.",
        schema: sreq_list_schema,
    },
    ToolDef {
        name: "req_sreq_show",
        description: "Return one safety requirement in full: inherited SIL, statement, acceptance, the safety functions it realizes, and its latest evidence. Pass id (SR-NNNN).",
        schema: id_schema,
    },
    ToolDef {
        name: "req_sreq_update",
        description: "Modify a safety requirement's fields/status. Set id and reason. add_acceptance appends; acceptance replaces.",
        schema: sreq_update_schema,
    },
    ToolDef {
        name: "req_sreq_realize",
        description: "Record that a safety requirement realizes a safety function (SR -> SF). REQUIRED: sreq (SR-NNNN), sf (SF-NNNN). Set remove=true to unlink.",
        schema: sreq_realize_schema,
    },
    ToolDef {
        name: "req_sreq_verify",
        description: "Attach verification evidence to a safety requirement, optionally promoting to Verified. REQUIRED: id, by (automated | composition | inspection). The SIL-rigour gate BLOCKS inspection-only evidence for a SIL 3/4 requirement — provide automated/composition evidence, or set force=true to record an audited exception. Pass notes and optional cites (array).",
        schema: sreq_verify_schema,
    },
    ToolDef {
        name: "req_trace",
        description: "Print the end-to-end safety case for a HAZ/SF/SR id: hazard -> required SIL -> safety function -> allocated SIL (adequate?) -> safety requirements -> verification evidence, with a roll-up verdict (complete / incomplete and what's blocking). The single best call to review whether a hazard is fully mitigated and verified.",
        schema: id_schema,
    },
    // REQ-0139: the staged validation dossier. This is the path to Verified
    // for both REQ-NNNN and SR-NNNN — promotion is BLOCKED without a passing
    // dossier. Walk the stages in order: plan -> analysis -> test -> conclude.
    ToolDef {
        name: "req_validation_plan",
        description: "STAGE 1 of validating a requirement. Open the validation dossier for a REQ-NNNN or SR-NNNN by recording the PLAN: how you will validate it — what you will review (analysis) and how you will test it. A passing dossier is REQUIRED before that requirement can be promoted to Verified. Pass id and plan. Use reopen=true with a reason to re-validate a concluded dossier (e.g. after code changed).",
        schema: validation_plan_schema,
    },
    ToolDef {
        name: "req_validation_analysis",
        description: "STAGE 2. Record validation BY ANALYSIS (code review): your findings from reading the implementation against the requirement, plus a pass/fail result. Pass id, findings, result (pass|fail), and optional references (files/commits reviewed). Requires the plan to exist first.",
        schema: validation_activity_schema,
    },
    ToolDef {
        name: "req_validation_test",
        description: "STAGE 3. Record validation BY TESTING: what you ran/observed and a pass/fail result. Recorded TestRecords on the requirement are auto-referenced; cite extra evidence via references. Pass id, findings, result (pass|fail). Requires the analysis stage first. Prefer real tests (req_test_record / req_test_run) over prose where they exist.",
        schema: validation_activity_schema,
    },
    ToolDef {
        name: "req_validation_conclude",
        description: "STAGE 4. Record the validation STATEMENT and derive the verdict (Pass only when BOTH analysis and testing passed). Set promote=true to flip the requirement to Verified — gated exactly like req_verify (and the SIL-rigour gate for safety requirements). A FAIL verdict cannot be promoted. Pass id, statement.",
        schema: validation_conclude_schema,
    },
    ToolDef {
        name: "req_validation_show",
        description: "Return the validation dossier (plan, analysis, testing, statement, verdict, staleness anchor) for a REQ-NNNN or SR-NNNN. Read-only.",
        schema: id_schema,
    },
    ToolDef {
        name: "req_validation_backfill",
        description: "Grandfather already-Verified items that pre-date the dossier requirement by recording an AUDITED exemption, so a strict req_validate passes. Pass id (one item) or all=true (every Verified item lacking a passing dossier), with a reason. Use sparingly — prefer a real dossier.",
        schema: validation_backfill_schema,
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

// REQ-0129: read-only history of a requirement's attached test records.
fn test_list_schema() -> Value {
    json!({
        "type": "object",
        "required": ["id"],
        "properties": {
            "id": { "type": "string", "description": "REQ-NNNN (any case/pad form accepted)." }
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
            "promote": { "type": "boolean", "default": false },
            "force":   { "type": "boolean", "default": false, "description": "Skip the Implemented-status precondition on promote." },
            "no_dossier": { "type": "boolean", "default": false, "description": "REQ-0139: promote without a validation dossier, recording an audited exemption. Requires reason. Ordinary requirements only." },
            "reason":  { "type": "string", "description": "Justification, required with no_dossier." }
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

// ---------- REQ-0134: functional-safety schemas ----------

fn id_schema() -> Value {
    json!({
        "type": "object",
        "properties": { "id": { "type": "string", "description": "HAZ-/SF-/SR-NNNN id" } },
        "required": ["id"]
    })
}

// REQ-0139: input schemas for the validation-dossier MCP tools.
fn validation_plan_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "id": { "type": "string", "description": "REQ-NNNN or SR-NNNN" },
            "plan": { "type": "string", "description": "How you will validate this — the analysis (review) and testing approach." },
            "reopen": { "type": "boolean", "description": "Re-open a concluded dossier to re-validate (clears the prior verdict). Requires reason." },
            "reason": { "type": "string" }
        },
        "required": ["id", "plan"]
    })
}

fn validation_activity_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "id": { "type": "string", "description": "REQ-NNNN or SR-NNNN" },
            "findings": { "type": "string", "description": "What was reviewed/run and what was observed." },
            "result": { "type": "string", "enum": ["pass", "fail"] },
            "references": { "type": "array", "items": { "type": "string" }, "description": "Files/commits reviewed (analysis) or test names/records cited (testing)." }
        },
        "required": ["id", "findings", "result"]
    })
}

fn validation_conclude_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "id": { "type": "string", "description": "REQ-NNNN or SR-NNNN" },
            "statement": { "type": "string", "description": "The validation statement supporting the verdict." },
            "promote": { "type": "boolean", "description": "Promote to Verified (only when the verdict is Pass; gated like req_verify)." },
            "force": { "type": "boolean", "description": "Override the promotion preconditions (status ladder / SIL-rigour gate). Requires reason." },
            "reason": { "type": "string" }
        },
        "required": ["id", "statement"]
    })
}

// REQ-0139: schema for the validation-dossier back-fill tool.
fn validation_backfill_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "id": { "type": "string", "description": "A single REQ-/SR- id; omit with all=true." },
            "all": { "type": "boolean", "description": "Back-fill every Verified item without a passing dossier." },
            "reason": { "type": "string", "description": "Justification recorded on each exemption." }
        },
        "required": ["reason"]
    })
}

const C_ENUM: [&str; 4] = ["C_A", "C_B", "C_C", "C_D"];
const F_ENUM: [&str; 2] = ["F_A", "F_B"];
const P_ENUM: [&str; 2] = ["P_A", "P_B"];
const W_ENUM: [&str; 3] = ["W1", "W2", "W3"];

fn hazard_add_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "title": { "type": "string" },
            "harm": { "type": "string", "description": "free-text potential-harm narrative" },
            "description": { "type": "string" },
            "context": { "type": "string", "description": "operational situation/mode" },
            "consequence": { "type": "string", "enum": C_ENUM },
            "frequency": { "type": "string", "enum": F_ENUM },
            "avoidance": { "type": "string", "enum": P_ENUM },
            "probability": { "type": "string", "enum": W_ENUM },
            "tags": { "type": "array", "items": { "type": "string" } }
        },
        "required": ["title", "harm"]
    })
}

fn hazard_list_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "sil": { "type": "string" },
            "status": { "type": "string" },
            "unmitigated": { "type": "boolean" }
        }
    })
}

fn hazard_assess_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "id": { "type": "string" },
            "consequence": { "type": "string", "enum": C_ENUM },
            "frequency": { "type": "string", "enum": F_ENUM },
            "avoidance": { "type": "string", "enum": P_ENUM },
            "probability": { "type": "string", "enum": W_ENUM },
            "reason": { "type": "string" }
        },
        "required": ["id", "consequence", "frequency", "avoidance", "probability"]
    })
}

fn hazard_update_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "id": { "type": "string" },
            "title": { "type": "string" },
            "description": { "type": "string" },
            "context": { "type": "string" },
            "harm": { "type": "string" },
            "status": { "type": "string" },
            "add_tag": { "type": "array", "items": { "type": "string" } },
            "remove_tag": { "type": "array", "items": { "type": "string" } },
            "reason": { "type": "string" }
        },
        "required": ["id"]
    })
}

fn sf_add_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "title": { "type": "string" },
            "description": { "type": "string" },
            "safe_state": { "type": "string" },
            "mitigates": { "type": "array", "items": { "type": "string" } },
            "tags": { "type": "array", "items": { "type": "string" } }
        },
        "required": ["title"]
    })
}

fn sf_list_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "sil": { "type": "string" },
            "status": { "type": "string" },
            "unrealized": { "type": "boolean" }
        }
    })
}

fn sf_update_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "id": { "type": "string" },
            "title": { "type": "string" },
            "description": { "type": "string" },
            "safe_state": { "type": "string" },
            "status": { "type": "string" },
            "add_tag": { "type": "array", "items": { "type": "string" } },
            "remove_tag": { "type": "array", "items": { "type": "string" } },
            "reason": { "type": "string" }
        },
        "required": ["id"]
    })
}

fn sf_mitigate_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "sf": { "type": "string" },
            "hazard": { "type": "string" },
            "remove": { "type": "boolean" }
        },
        "required": ["sf", "hazard"]
    })
}

fn sreq_add_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "title": { "type": "string" },
            "statement": { "type": "string" },
            "rationale": { "type": "string" },
            "acceptance": { "type": "array", "items": { "type": "string" } },
            "priority": { "type": "string", "enum": ["must","should","could","wont"] },
            "realizes": { "type": "array", "items": { "type": "string" } },
            "tags": { "type": "array", "items": { "type": "string" } }
        },
        "required": ["title", "statement", "rationale"]
    })
}

fn sreq_list_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "sil": { "type": "string" },
            "status": { "type": "string" },
            "unverified": { "type": "boolean" }
        }
    })
}

fn sreq_update_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "id": { "type": "string" },
            "title": { "type": "string" },
            "statement": { "type": "string" },
            "rationale": { "type": "string" },
            "acceptance": { "type": "array", "items": { "type": "string" } },
            "add_acceptance": { "type": "array", "items": { "type": "string" } },
            "priority": { "type": "string" },
            "status": { "type": "string" },
            "add_tag": { "type": "array", "items": { "type": "string" } },
            "remove_tag": { "type": "array", "items": { "type": "string" } },
            "reason": { "type": "string" }
        },
        "required": ["id"]
    })
}

fn sreq_realize_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "sreq": { "type": "string" },
            "sf": { "type": "string" },
            "remove": { "type": "boolean" }
        },
        "required": ["sreq", "sf"]
    })
}

fn sreq_verify_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "id": { "type": "string" },
            "by": { "type": "string", "enum": ["automated","composition","inspection"] },
            "notes": { "type": "string" },
            "cites": { "type": "array", "items": { "type": "string" } },
            "promote": { "type": "boolean" },
            "force": { "type": "boolean", "description": "override the promotion guards; requires reason" },
            "reason": { "type": "string", "description": "justification, required when force is true" }
        },
        "required": ["id", "by"]
    })
}

// ---------- dispatcher ----------

fn call_tool(name: &str, args: &Value, file: &Path) -> Result<String> {
    // REQ-0138: an agent (driving via MCP) cannot create or change safety
    // artifacts until a human has accepted the disclaimer via the CLI.
    // Reads (list/show/trace) are not gated.
    const SAFETY_MUTATIONS: &[&str] = &[
        "req_hazard_add",
        "req_hazard_assess",
        "req_hazard_update",
        "req_sf_add",
        "req_sf_update",
        "req_sf_mitigate",
        "req_sreq_add",
        "req_sreq_update",
        "req_sreq_realize",
        "req_sreq_verify",
    ];
    if SAFETY_MUTATIONS.contains(&name) {
        crate::commands::safety_gov::ensure_enabled_path(file)?;
    }
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
        "req_test_list" => tool_test_list(args, file),
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
        // REQ-0134: functional-safety tools.
        "req_hazard_add" => safety_mcp::hazard_add(args, file),
        "req_hazard_list" => safety_mcp::hazard_list(args, file),
        "req_hazard_show" => safety_mcp::hazard_show(args, file),
        "req_hazard_assess" => safety_mcp::hazard_assess(args, file),
        "req_hazard_update" => safety_mcp::hazard_update(args, file),
        "req_sf_add" => safety_mcp::sf_add(args, file),
        "req_sf_list" => safety_mcp::sf_list(args, file),
        "req_sf_show" => safety_mcp::sf_show(args, file),
        "req_sf_update" => safety_mcp::sf_update(args, file),
        "req_sf_mitigate" => safety_mcp::sf_mitigate(args, file),
        "req_sreq_add" => safety_mcp::sreq_add(args, file),
        "req_sreq_list" => safety_mcp::sreq_list(args, file),
        "req_sreq_show" => safety_mcp::sreq_show(args, file),
        "req_sreq_update" => safety_mcp::sreq_update(args, file),
        "req_sreq_realize" => safety_mcp::sreq_realize(args, file),
        "req_sreq_verify" => safety_mcp::sreq_verify(args, file),
        "req_trace" => safety_mcp::trace(args, file),
        // REQ-0139: the staged validation dossier.
        "req_validation_plan" => validation_mcp::plan(args, file),
        "req_validation_analysis" => validation_mcp::analysis(args, file),
        "req_validation_test" => validation_mcp::test(args, file),
        "req_validation_conclude" => validation_mcp::conclude(args, file),
        "req_validation_show" => validation_mcp::show(args, file),
        "req_validation_backfill" => validation_mcp::backfill(args, file),
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
        // REQ-0139: a new requirement starts without a validation dossier.
        validation: None,
        extra: Default::default(),
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
        sil_gate_exception: false,
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
    // REQ-0139: a bulk test run only auto-promotes items already cleared by a
    // passing dossier (or an ordinary-requirement tag exemption).
    let exempt_tags = project.validation_exempt_tags();
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
                sil_gate_exception: false,
            });
            r.history.push(crate::commands::history(
                format!("test {} recorded via MCP req_test_run", outcome.as_str()),
                None,
            ));
            r.updated = Utc::now();
            // REQ-0139: only auto-promote items cleared by a passing dossier.
            let dossier_ok = r.validation.as_ref().map(|v| v.passed()).unwrap_or(false)
                || r.tags.iter().any(|t| exempt_tags.contains(t));
            if promote
                && dossier_ok
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

// REQ-0129: agent-facing read of a requirement's test-record history,
// mirroring the CLI `req test list`. Returns the `tests` vector as JSON.
fn tool_test_list(args: &Value, file: &Path) -> Result<String> {
    let raw = req_s(args, "id")?;
    let project = storage::load(file)?;
    let id = crate::commands::resolve_id(&project, &raw)?;
    let r = project
        .requirements
        .get(&id)
        .ok_or_else(|| anyhow!("no such requirement: {}", id))?;
    Ok(serde_json::to_string_pretty(&r.tests)?)
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
    if !project.requirements.contains_key(&id) {
        return Err(anyhow!("no such requirement: {}", id));
    }
    // REQ-0139: evaluate the validation-dossier gate before the mutable borrow.
    let dossier_ok = project.requirements[&id]
        .validation
        .as_ref()
        .map(|v| v.passed())
        .unwrap_or(false);
    let exempt_by_tag = project.req_is_validation_exempt(&project.requirements[&id]);
    let no_dossier = args
        .get("no_dossier")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let reason = args.get("reason").and_then(Value::as_str).map(String::from);
    let r = project.requirements.get_mut(&id).unwrap();
    r.tests.push(crate::model::TestRecord {
        at: Utc::now(),
        actor: crate::commands::current_actor(),
        commit: commit.clone(),
        outcome: crate::model::TestOutcome::Pass,
        notes: format!("{}{}", prefix, notes),
        kind,
        content_hash: None,
        linked_files: None,
        sil_gate_exception: false,
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
                // REQ-0139: a passing dossier (or a tag/no_dossier exemption)
                // is the precondition for Verified.
                if !dossier_ok && !exempt_by_tag {
                    if no_dossier {
                        r.validation = Some(crate::commands::validation::exemption_dossier(
                            reason.as_deref().unwrap_or(""),
                            crate::commands::current_actor(),
                            commit.clone(),
                        ));
                    } else {
                        return Err(anyhow!(
                            "{} cannot be promoted to Verified without a passing validation \
                             dossier. Use the req_validation_* tools, tag it `{}`, or pass \
                             no_dossier=true with a reason.",
                            id,
                            crate::model::DEFAULT_VALIDATION_EXEMPT_TAG
                        ));
                    }
                }
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

// ---------- REQ-0134: functional-safety tool handlers ----------
//
// These mirror the `req hazard|sf|sreq|trace` CLI surface for agents
// driving the tool over MCP. Each loads, mutates, saves, and returns a
// JSON view. SILs are derived (never accepted as input), and the
// SIL-rigour gate is enforced identically to the CLI.
// REQ-0139: MCP surface for the validation dossier. Thin wrappers over the
// IO-free `commands::validation::op_*` core, so the CLI and the MCP server
// share the exact gate + verdict logic.
mod validation_mcp {
    use super::{storage, Value};
    use crate::commands::validation::{self, Stage};
    use crate::model::TestOutcome;
    use anyhow::{anyhow, Result};
    use std::path::Path;

    fn s(v: &Value, k: &str) -> Option<String> {
        v.get(k).and_then(Value::as_str).map(|s| s.to_string())
    }
    fn req_s(v: &Value, k: &str) -> Result<String> {
        s(v, k).ok_or_else(|| anyhow!("'{}' is required", k))
    }
    fn b(v: &Value, k: &str) -> bool {
        v.get(k).and_then(Value::as_bool).unwrap_or(false)
    }
    fn arr(v: &Value, k: &str) -> Vec<String> {
        v.get(k)
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(String::from)
                    .collect()
            })
            .unwrap_or_default()
    }
    fn outcome(a: &Value) -> Result<TestOutcome> {
        match req_s(a, "result")?.to_lowercase().as_str() {
            "pass" => Ok(TestOutcome::Pass),
            "fail" => Ok(TestOutcome::Fail),
            o => Err(anyhow!("bad result {} (pass|fail)", o)),
        }
    }
    fn dossier_json(p: &crate::model::Project, id: &str) -> Result<String> {
        let (cid, fam) = validation::resolve(p, id)?;
        Ok(serde_json::to_string_pretty(&serde_json::json!({
            "id": cid,
            "validation": validation::dossier(p, &cid, fam),
        }))?)
    }

    pub fn plan(a: &Value, file: &Path) -> Result<String> {
        let _g = storage::acquire_lock(file)?;
        let mut p = storage::load(file)?;
        let id = validation::op_plan(
            &mut p,
            &req_s(a, "id")?,
            &req_s(a, "plan")?,
            b(a, "reopen"),
            s(a, "reason").as_deref(),
        )?;
        storage::save(file, &p)?;
        dossier_json(&p, &id)
    }

    pub fn analysis(a: &Value, file: &Path) -> Result<String> {
        activity(a, file, Stage::Analysis)
    }
    pub fn test(a: &Value, file: &Path) -> Result<String> {
        activity(a, file, Stage::Testing)
    }
    // REQ-0139: record a validation-by-analysis / by-testing stage.
    fn activity(a: &Value, file: &Path, stage: Stage) -> Result<String> {
        let _g = storage::acquire_lock(file)?;
        let mut p = storage::load(file)?;
        let id = validation::op_activity(
            &mut p,
            &req_s(a, "id")?,
            stage,
            &req_s(a, "findings")?,
            outcome(a)?,
            &arr(a, "references"),
        )?;
        storage::save(file, &p)?;
        dossier_json(&p, &id)
    }

    pub fn conclude(a: &Value, file: &Path) -> Result<String> {
        let _g = storage::acquire_lock(file)?;
        let mut p = storage::load(file)?;
        let force = b(a, "force");
        let reason = s(a, "reason");
        if force
            && reason
                .as_deref()
                .map(|r| r.trim().is_empty())
                .unwrap_or(true)
        {
            return Err(anyhow!("force=true requires a non-empty reason"));
        }
        let out = validation::op_conclude(
            &mut p,
            &req_s(a, "id")?,
            &req_s(a, "statement")?,
            b(a, "promote"),
            force,
            reason.as_deref(),
            Path::new("."),
        )?;
        storage::save(file, &p)?;
        let (_cid, fam) = validation::resolve(&p, &out.id)?;
        Ok(serde_json::to_string_pretty(&serde_json::json!({
            "id": out.id,
            "verdict": out.verdict.as_str(),
            "promoted": out.promoted,
            "validation": validation::dossier(&p, &out.id, fam),
        }))?)
    }

    pub fn show(a: &Value, file: &Path) -> Result<String> {
        let p = storage::load(file)?;
        dossier_json(&p, &req_s(a, "id")?)
    }

    // REQ-0139: back-fill an audited exemption onto a Verified item.
    pub fn backfill(a: &Value, file: &Path) -> Result<String> {
        let _g = storage::acquire_lock(file)?;
        let mut p = storage::load(file)?;
        let done = validation::op_backfill(
            &mut p,
            s(a, "id").as_deref(),
            b(a, "all"),
            &req_s(a, "reason")?,
        )?;
        if !done.is_empty() {
            storage::save(file, &p)?;
        }
        Ok(serde_json::to_string_pretty(
            &serde_json::json!({ "backfilled": done }),
        )?)
    }
}

mod safety_mcp {
    use super::{commands, json, storage, Value};
    use crate::model::{
        Avoidance, Consequence, EvidenceKind, Frequency, Hazard, HazardStatus, Link, LinkKind,
        Probability, Project, SafetyFunction, SafetyFunctionStatus, SafetyRequirement, Sil, Status,
        TestOutcome, TestRecord,
    };
    use anyhow::{anyhow, Result};
    use chrono::Utc;
    use std::path::Path;

    fn s(v: &Value, k: &str) -> Option<String> {
        v.get(k).and_then(Value::as_str).map(|s| s.to_string())
    }
    fn req_s(v: &Value, k: &str) -> Result<String> {
        s(v, k).ok_or_else(|| anyhow!("'{}' is required", k))
    }
    fn b(v: &Value, k: &str) -> bool {
        v.get(k).and_then(Value::as_bool).unwrap_or(false)
    }
    fn arr(v: &Value, k: &str) -> Vec<String> {
        v.get(k)
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(|s| s.to_string())
                    .collect()
            })
            .unwrap_or_default()
    }
    /// Apply add_tag/remove_tag arrays from an MCP request to a tag list,
    /// matching the CLI `--add-tag`/`--remove-tag` semantics.
    fn apply_tags(tags: &mut Vec<String>, a: &Value) {
        for t in arr(a, "add_tag") {
            if !tags.contains(&t) {
                tags.push(t);
            }
        }
        let rm = arr(a, "remove_tag");
        tags.retain(|t| !rm.contains(t));
    }
    fn norm(prefix: &str, raw: &str) -> String {
        let up = raw.trim().to_uppercase();
        let want = format!("{}-", prefix);
        let digits = if let Some(r) = up.strip_prefix(&want) {
            r.to_string()
        } else if raw.trim().chars().all(|c| c.is_ascii_digit()) && !raw.trim().is_empty() {
            raw.trim().to_string()
        } else {
            return up;
        };
        match digits.parse::<u32>() {
            Ok(n) => format!("{}-{:04}", prefix, n),
            Err(_) => up,
        }
    }
    fn parse_c(s: &str) -> Result<Consequence> {
        Ok(match s.to_uppercase().as_str() {
            "C_A" => Consequence::Ca,
            "C_B" => Consequence::Cb,
            "C_C" => Consequence::Cc,
            "C_D" => Consequence::Cd,
            o => return Err(anyhow!("bad consequence {} (want C_A..C_D)", o)),
        })
    }
    fn parse_f(s: &str) -> Result<Frequency> {
        Ok(match s.to_uppercase().as_str() {
            "F_A" => Frequency::Fa,
            "F_B" => Frequency::Fb,
            o => return Err(anyhow!("bad frequency {} (want F_A/F_B)", o)),
        })
    }
    fn parse_p(s: &str) -> Result<Avoidance> {
        Ok(match s.to_uppercase().as_str() {
            "P_A" => Avoidance::Pa,
            "P_B" => Avoidance::Pb,
            o => return Err(anyhow!("bad avoidance {} (want P_A/P_B)", o)),
        })
    }
    fn parse_w(s: &str) -> Result<Probability> {
        Ok(match s.to_uppercase().as_str() {
            "W1" => Probability::W1,
            "W2" => Probability::W2,
            "W3" => Probability::W3,
            o => return Err(anyhow!("bad probability {} (want W1..W3)", o)),
        })
    }
    fn parse_status(s: &str) -> Result<Status> {
        Ok(match s.to_lowercase().as_str() {
            "draft" => Status::Draft,
            "proposed" => Status::Proposed,
            "approved" => Status::Approved,
            "implemented" => Status::Implemented,
            "verified" => Status::Verified,
            "obsolete" => Status::Obsolete,
            o => return Err(anyhow!("bad status {}", o)),
        })
    }
    fn parse_haz_status(s: &str) -> Result<HazardStatus> {
        Ok(match s.to_lowercase().as_str() {
            "identified" => HazardStatus::Identified,
            "assessed" => HazardStatus::Assessed,
            "mitigated" => HazardStatus::Mitigated,
            "verified" => HazardStatus::Verified,
            "obsolete" => HazardStatus::Obsolete,
            o => return Err(anyhow!("bad hazard status {}", o)),
        })
    }
    fn parse_sf_status(s: &str) -> Result<SafetyFunctionStatus> {
        Ok(match s.to_lowercase().as_str() {
            "proposed" => SafetyFunctionStatus::Proposed,
            "allocated" => SafetyFunctionStatus::Allocated,
            "implemented" => SafetyFunctionStatus::Implemented,
            "verified" => SafetyFunctionStatus::Verified,
            "obsolete" => SafetyFunctionStatus::Obsolete,
            o => return Err(anyhow!("bad safety-function status {}", o)),
        })
    }
    fn parse_priority(s: &str) -> Result<crate::model::Priority> {
        Ok(match s.to_lowercase().as_str() {
            "must" => crate::model::Priority::Must,
            "should" => crate::model::Priority::Should,
            "could" => crate::model::Priority::Could,
            "wont" => crate::model::Priority::Wont,
            o => return Err(anyhow!("bad priority {}", o)),
        })
    }
    fn sil_s(s: Option<Sil>) -> Value {
        match s {
            Some(s) => Value::String(s.as_str().to_string()),
            None => Value::Null,
        }
    }
    fn mitigates(sf: &SafetyFunction, hid: &str) -> bool {
        sf.links
            .iter()
            .any(|l| l.kind == LinkKind::Mitigates && l.target == hid)
    }
    fn realizes(sr: &SafetyRequirement, sfid: &str) -> bool {
        sr.links
            .iter()
            .any(|l| l.kind == LinkKind::Realizes && l.target == sfid)
    }

    // ----- hazards -----

    pub fn hazard_add(a: &Value, file: &Path) -> Result<String> {
        let _guard = storage::acquire_lock(file)?;
        let mut p = storage::load(file)?;
        let now = Utc::now();
        let consequence = s(a, "consequence").map(|x| parse_c(&x)).transpose()?;
        let frequency = s(a, "frequency").map(|x| parse_f(&x)).transpose()?;
        let avoidance = s(a, "avoidance").map(|x| parse_p(&x)).transpose()?;
        let probability = s(a, "probability").map(|x| parse_w(&x)).transpose()?;
        let assessed = consequence.is_some()
            && frequency.is_some()
            && avoidance.is_some()
            && probability.is_some();
        let id = p.allocate_haz_id();
        let h = Hazard {
            id: id.clone(),
            title: req_s(a, "title")?,
            description: s(a, "description").unwrap_or_default(),
            operating_context: s(a, "context").unwrap_or_default(),
            harm: req_s(a, "harm")?,
            consequence,
            frequency,
            avoidance,
            probability,
            status: if assessed {
                HazardStatus::Assessed
            } else {
                HazardStatus::Identified
            },
            tags: arr(a, "tags"),
            links: Vec::new(),
            created: now,
            updated: now,
            history: vec![commands::history("created", None)],
            extra: Default::default(),
        };
        let sil = p.required_sil(&h);
        p.hazards.insert(id.clone(), h);
        p.updated = now;
        storage::save(file, &p)?;
        Ok(serde_json::to_string_pretty(
            &json!({ "id": id, "required_sil": sil_s(sil), "status": p.hazards[&id].status.as_str() }),
        )?)
    }

    // REQ-0134: MCP twin — list hazards with derived SIL.
    pub fn hazard_list(a: &Value, file: &Path) -> Result<String> {
        let p = storage::load(file)?;
        let status = s(a, "status").map(|x| parse_haz_status(&x)).transpose()?;
        let sil = s(a, "sil").map(|x| x.to_uppercase());
        let unmit = b(a, "unmitigated");
        let mut rows = Vec::new();
        for h in p.hazards.values() {
            if let Some(st) = status {
                if h.status != st {
                    continue;
                }
            }
            if let Some(want) = &sil {
                if p.required_sil(h).map(|s| s.as_str().to_uppercase()) != Some(want.clone()) {
                    continue;
                }
            }
            if unmit && p.safety_functions.values().any(|sf| mitigates(sf, &h.id)) {
                continue;
            }
            rows.push(json!({
                "id": h.id, "title": h.title, "status": h.status.as_str(),
                "required_sil": sil_s(p.required_sil(h)),
            }));
        }
        rows.sort_by(|x, y| x["id"].as_str().cmp(&y["id"].as_str()));
        Ok(serde_json::to_string_pretty(
            &json!({ "count": rows.len(), "hazards": rows }),
        )?)
    }

    pub fn hazard_show(a: &Value, file: &Path) -> Result<String> {
        let p = storage::load(file)?;
        let id = norm("HAZ", &req_s(a, "id")?);
        let h = p
            .hazards
            .get(&id)
            .ok_or_else(|| anyhow!("no such hazard: {}", id))?;
        Ok(serde_json::to_string_pretty(&json!({
            "hazard": h,
            "required_sil": sil_s(p.required_sil(h)),
            "mitigated_by": p.safety_functions.values()
                .filter(|sf| mitigates(sf, &id))
                .map(|sf| sf.id.clone()).collect::<Vec<_>>(),
        }))?)
    }

    // REQ-0134: MCP twin — set C/F/P/W, derive the SIL.
    pub fn hazard_assess(a: &Value, file: &Path) -> Result<String> {
        let _guard = storage::acquire_lock(file)?;
        let mut p = storage::load(file)?;
        let id = norm("HAZ", &req_s(a, "id")?);
        if !p.hazards.contains_key(&id) {
            return Err(anyhow!("no such hazard: {}", id));
        }
        let c = parse_c(&req_s(a, "consequence")?)?;
        let f = parse_f(&req_s(a, "frequency")?)?;
        let pa = parse_p(&req_s(a, "avoidance")?)?;
        let w = parse_w(&req_s(a, "probability")?)?;
        let now = Utc::now();
        {
            let h = p.hazards.get_mut(&id).unwrap();
            h.consequence = Some(c);
            h.frequency = Some(f);
            h.avoidance = Some(pa);
            h.probability = Some(w);
            if matches!(h.status, HazardStatus::Identified) {
                h.status = HazardStatus::Assessed;
            }
            h.updated = now;
            h.history
                .push(commands::history("assessed", s(a, "reason")));
        }
        p.updated = now;
        let sil = p.required_sil(&p.hazards[&id]);
        storage::save(file, &p)?;
        Ok(serde_json::to_string_pretty(
            &json!({ "id": id, "required_sil": sil_s(sil) }),
        )?)
    }

    pub fn hazard_update(a: &Value, file: &Path) -> Result<String> {
        let _guard = storage::acquire_lock(file)?;
        let mut p = storage::load(file)?;
        let id = norm("HAZ", &req_s(a, "id")?);
        if !p.hazards.contains_key(&id) {
            return Err(anyhow!("no such hazard: {}", id));
        }
        let status = s(a, "status").map(|x| parse_haz_status(&x)).transpose()?;
        let now = Utc::now();
        {
            let h = p.hazards.get_mut(&id).unwrap();
            if let Some(t) = s(a, "title") {
                h.title = t;
            }
            if let Some(d) = s(a, "description") {
                h.description = d;
            }
            if let Some(c) = s(a, "context") {
                h.operating_context = c;
            }
            if let Some(harm) = s(a, "harm") {
                h.harm = harm;
            }
            if let Some(st) = status {
                h.status = st;
            }
            apply_tags(&mut h.tags, a);
            h.updated = now;
            h.history.push(commands::history("updated", s(a, "reason")));
        }
        p.updated = now;
        storage::save(file, &p)?;
        Ok(serde_json::to_string_pretty(&p.hazards[&id])?)
    }

    // ----- safety functions -----

    // REQ-0134: MCP twin — create a safety function.
    pub fn sf_add(a: &Value, file: &Path) -> Result<String> {
        let _guard = storage::acquire_lock(file)?;
        let mut p = storage::load(file)?;
        let now = Utc::now();
        let mut links = Vec::new();
        for raw in arr(a, "mitigates") {
            let hid = norm("HAZ", &raw);
            if !p.hazards.contains_key(&hid) {
                return Err(anyhow!("no such hazard: {}", hid));
            }
            links.push(Link {
                kind: LinkKind::Mitigates,
                target: hid,
            });
        }
        let status = if links.is_empty() {
            SafetyFunctionStatus::Proposed
        } else {
            SafetyFunctionStatus::Allocated
        };
        let id = p.allocate_sf_id();
        let sf = SafetyFunction {
            id: id.clone(),
            title: req_s(a, "title")?,
            description: s(a, "description").unwrap_or_default(),
            safe_state: s(a, "safe_state").unwrap_or_default(),
            status,
            tags: arr(a, "tags"),
            links: links.clone(),
            created: now,
            updated: now,
            history: vec![commands::history("created", None)],
            extra: Default::default(),
        };
        let alloc = p.allocated_sil(&sf);
        p.safety_functions.insert(id.clone(), sf);
        for l in &links {
            if let Some(h) = p.hazards.get_mut(&l.target) {
                if matches!(h.status, HazardStatus::Identified | HazardStatus::Assessed) {
                    h.status = HazardStatus::Mitigated;
                    h.updated = now;
                    h.history
                        .push(commands::history(format!("mitigated by {}", id), None));
                }
            }
        }
        p.updated = now;
        storage::save(file, &p)?;
        Ok(serde_json::to_string_pretty(
            &json!({ "id": id, "allocated_sil": sil_s(alloc) }),
        )?)
    }

    // REQ-0134: MCP twin — list safety functions with allocated SIL.
    pub fn sf_list(a: &Value, file: &Path) -> Result<String> {
        let p = storage::load(file)?;
        let status = s(a, "status").map(|x| parse_sf_status(&x)).transpose()?;
        let sil = s(a, "sil").map(|x| x.to_uppercase());
        let unreal = b(a, "unrealized");
        let mut rows = Vec::new();
        for sf in p.safety_functions.values() {
            if let Some(st) = status {
                if sf.status != st {
                    continue;
                }
            }
            if let Some(want) = &sil {
                if p.allocated_sil(sf).map(|s| s.as_str().to_uppercase()) != Some(want.clone()) {
                    continue;
                }
            }
            if unreal
                && p.safety_requirements
                    .values()
                    .any(|sr| realizes(sr, &sf.id))
            {
                continue;
            }
            rows.push(json!({
                "id": sf.id, "title": sf.title, "status": sf.status.as_str(),
                "allocated_sil": sil_s(p.allocated_sil(sf)),
            }));
        }
        rows.sort_by(|x, y| x["id"].as_str().cmp(&y["id"].as_str()));
        Ok(serde_json::to_string_pretty(
            &json!({ "count": rows.len(), "safety_functions": rows }),
        )?)
    }

    pub fn sf_show(a: &Value, file: &Path) -> Result<String> {
        let p = storage::load(file)?;
        let id = norm("SF", &req_s(a, "id")?);
        let sf = p
            .safety_functions
            .get(&id)
            .ok_or_else(|| anyhow!("no such safety function: {}", id))?;
        Ok(serde_json::to_string_pretty(&json!({
            "safety_function": sf,
            "allocated_sil": sil_s(p.allocated_sil(sf)),
            "realized_by": p.safety_requirements.values()
                .filter(|sr| realizes(sr, &id)).map(|sr| sr.id.clone()).collect::<Vec<_>>(),
        }))?)
    }

    // REQ-0134: MCP twin — edit a safety function.
    pub fn sf_update(a: &Value, file: &Path) -> Result<String> {
        let _guard = storage::acquire_lock(file)?;
        let mut p = storage::load(file)?;
        let id = norm("SF", &req_s(a, "id")?);
        if !p.safety_functions.contains_key(&id) {
            return Err(anyhow!("no such safety function: {}", id));
        }
        let status = s(a, "status").map(|x| parse_sf_status(&x)).transpose()?;
        let now = Utc::now();
        {
            let sf = p.safety_functions.get_mut(&id).unwrap();
            if let Some(t) = s(a, "title") {
                sf.title = t;
            }
            if let Some(d) = s(a, "description") {
                sf.description = d;
            }
            if let Some(ss) = s(a, "safe_state") {
                sf.safe_state = ss;
            }
            if let Some(st) = status {
                sf.status = st;
            }
            apply_tags(&mut sf.tags, a);
            sf.updated = now;
            sf.history
                .push(commands::history("updated", s(a, "reason")));
        }
        p.updated = now;
        storage::save(file, &p)?;
        Ok(serde_json::to_string_pretty(&p.safety_functions[&id])?)
    }

    pub fn sf_mitigate(a: &Value, file: &Path) -> Result<String> {
        let _guard = storage::acquire_lock(file)?;
        let mut p = storage::load(file)?;
        let sf_id = norm("SF", &req_s(a, "sf")?);
        let haz_id = norm("HAZ", &req_s(a, "hazard")?);
        if !p.safety_functions.contains_key(&sf_id) {
            return Err(anyhow!("no such safety function: {}", sf_id));
        }
        if !p.hazards.contains_key(&haz_id) {
            return Err(anyhow!("no such hazard: {}", haz_id));
        }
        let remove = b(a, "remove");
        let now = Utc::now();
        {
            let sf = p.safety_functions.get_mut(&sf_id).unwrap();
            if remove {
                sf.links
                    .retain(|l| !(l.kind == LinkKind::Mitigates && l.target == haz_id));
            } else if mitigates(sf, &haz_id) {
                return Err(anyhow!("{} already mitigates {}", sf_id, haz_id));
            } else {
                sf.links.push(Link {
                    kind: LinkKind::Mitigates,
                    target: haz_id.clone(),
                });
                if matches!(sf.status, SafetyFunctionStatus::Proposed) {
                    sf.status = SafetyFunctionStatus::Allocated;
                }
            }
            sf.updated = now;
            sf.history.push(commands::history(
                if remove {
                    format!("unlinked mitigates {}", haz_id)
                } else {
                    format!("mitigates {}", haz_id)
                },
                None,
            ));
        }
        if !remove {
            if let Some(h) = p.hazards.get_mut(&haz_id) {
                if matches!(h.status, HazardStatus::Identified | HazardStatus::Assessed) {
                    h.status = HazardStatus::Mitigated;
                    h.updated = now;
                    h.history
                        .push(commands::history(format!("mitigated by {}", sf_id), None));
                }
            }
        }
        p.updated = now;
        storage::save(file, &p)?;
        Ok(serde_json::to_string_pretty(
            &json!({ "sf": sf_id, "hazard": haz_id, "linked": !remove }),
        )?)
    }

    // ----- safety requirements -----

    // REQ-0134: MCP twin — create a safety requirement.
    pub fn sreq_add(a: &Value, file: &Path) -> Result<String> {
        let _guard = storage::acquire_lock(file)?;
        let mut p = storage::load(file)?;
        let now = Utc::now();
        let mut links = Vec::new();
        for raw in arr(a, "realizes") {
            let sfid = norm("SF", &raw);
            if !p.safety_functions.contains_key(&sfid) {
                return Err(anyhow!("no such safety function: {}", sfid));
            }
            links.push(Link {
                kind: LinkKind::Realizes,
                target: sfid,
            });
        }
        let priority = s(a, "priority")
            .map(|x| parse_priority(&x))
            .transpose()?
            .unwrap_or(crate::model::Priority::Must);
        let id = p.allocate_sr_id();
        let sr = SafetyRequirement {
            id: id.clone(),
            title: req_s(a, "title")?,
            statement: req_s(a, "statement")?,
            rationale: req_s(a, "rationale")?,
            acceptance: arr(a, "acceptance"),
            priority,
            status: Status::Draft,
            tags: arr(a, "tags"),
            links,
            created: now,
            updated: now,
            history: vec![commands::history("created", None)],
            tests: Vec::new(),
            validation: None,
            extra: Default::default(),
        };
        let sil = p.inherited_sil(&sr);
        p.safety_requirements.insert(id.clone(), sr);
        p.updated = now;
        storage::save(file, &p)?;
        Ok(serde_json::to_string_pretty(
            &json!({ "id": id, "inherited_sil": sil_s(sil) }),
        )?)
    }

    // REQ-0134: MCP twin — list safety requirements with inherited SIL.
    pub fn sreq_list(a: &Value, file: &Path) -> Result<String> {
        let p = storage::load(file)?;
        let status = s(a, "status").map(|x| parse_status(&x)).transpose()?;
        let sil = s(a, "sil").map(|x| x.to_uppercase());
        let unver = b(a, "unverified");
        let mut rows = Vec::new();
        for sr in p.safety_requirements.values() {
            if let Some(st) = status {
                if sr.status != st {
                    continue;
                }
            }
            if let Some(want) = &sil {
                if p.inherited_sil(sr).map(|s| s.as_str().to_uppercase()) != Some(want.clone()) {
                    continue;
                }
            }
            if unver && matches!(sr.status, Status::Verified) {
                continue;
            }
            rows.push(json!({
                "id": sr.id, "title": sr.title, "status": sr.status.as_str(),
                "inherited_sil": sil_s(p.inherited_sil(sr)),
            }));
        }
        rows.sort_by(|x, y| x["id"].as_str().cmp(&y["id"].as_str()));
        Ok(serde_json::to_string_pretty(
            &json!({ "count": rows.len(), "safety_requirements": rows }),
        )?)
    }

    pub fn sreq_show(a: &Value, file: &Path) -> Result<String> {
        let p = storage::load(file)?;
        let id = norm("SR", &req_s(a, "id")?);
        let sr = p
            .safety_requirements
            .get(&id)
            .ok_or_else(|| anyhow!("no such safety requirement: {}", id))?;
        Ok(serde_json::to_string_pretty(&json!({
            "safety_requirement": sr,
            "inherited_sil": sil_s(p.inherited_sil(sr)),
        }))?)
    }

    // REQ-0134: MCP twin — edit a safety requirement.
    pub fn sreq_update(a: &Value, file: &Path) -> Result<String> {
        let _guard = storage::acquire_lock(file)?;
        let mut p = storage::load(file)?;
        let id = norm("SR", &req_s(a, "id")?);
        if !p.safety_requirements.contains_key(&id) {
            return Err(anyhow!("no such safety requirement: {}", id));
        }
        let status = s(a, "status").map(|x| parse_status(&x)).transpose()?;
        let priority = s(a, "priority").map(|x| parse_priority(&x)).transpose()?;
        let now = Utc::now();
        {
            let sr = p.safety_requirements.get_mut(&id).unwrap();
            if let Some(t) = s(a, "title") {
                sr.title = t;
            }
            if let Some(st) = s(a, "statement") {
                sr.statement = st;
            }
            if let Some(r) = s(a, "rationale") {
                sr.rationale = r;
            }
            if a.get("acceptance").is_some() {
                sr.acceptance = arr(a, "acceptance");
            }
            for ac in arr(a, "add_acceptance") {
                sr.acceptance.push(ac);
            }
            if let Some(pr) = priority {
                sr.priority = pr;
            }
            if let Some(st) = status {
                sr.status = st;
            }
            apply_tags(&mut sr.tags, a);
            sr.updated = now;
            sr.history
                .push(commands::history("updated", s(a, "reason")));
        }
        p.updated = now;
        storage::save(file, &p)?;
        Ok(serde_json::to_string_pretty(&p.safety_requirements[&id])?)
    }

    // REQ-0134: MCP twin — link a safety requirement to a safety function.
    pub fn sreq_realize(a: &Value, file: &Path) -> Result<String> {
        let _guard = storage::acquire_lock(file)?;
        let mut p = storage::load(file)?;
        let sr_id = norm("SR", &req_s(a, "sreq")?);
        let sf_id = norm("SF", &req_s(a, "sf")?);
        if !p.safety_requirements.contains_key(&sr_id) {
            return Err(anyhow!("no such safety requirement: {}", sr_id));
        }
        if !p.safety_functions.contains_key(&sf_id) {
            return Err(anyhow!("no such safety function: {}", sf_id));
        }
        let remove = b(a, "remove");
        let now = Utc::now();
        {
            let sr = p.safety_requirements.get_mut(&sr_id).unwrap();
            if remove {
                sr.links
                    .retain(|l| !(l.kind == LinkKind::Realizes && l.target == sf_id));
            } else if realizes(sr, &sf_id) {
                return Err(anyhow!("{} already realizes {}", sr_id, sf_id));
            } else {
                sr.links.push(Link {
                    kind: LinkKind::Realizes,
                    target: sf_id.clone(),
                });
            }
            sr.updated = now;
            sr.history.push(commands::history(
                if remove {
                    format!("unlinked realizes {}", sf_id)
                } else {
                    format!("realizes {}", sf_id)
                },
                None,
            ));
        }
        p.updated = now;
        storage::save(file, &p)?;
        Ok(serde_json::to_string_pretty(
            &json!({ "sreq": sr_id, "sf": sf_id, "linked": !remove }),
        )?)
    }

    // REQ-0135: MCP twin — attach evidence under the SIL-rigour gate.
    pub fn sreq_verify(a: &Value, file: &Path) -> Result<String> {
        let _guard = storage::acquire_lock(file)?;
        let mut p = storage::load(file)?;
        let id = norm("SR", &req_s(a, "id")?);
        if !p.safety_requirements.contains_key(&id) {
            return Err(anyhow!("no such safety requirement: {}", id));
        }
        let kind = match req_s(a, "by")?.to_lowercase().as_str() {
            "automated" => EvidenceKind::Automated,
            "composition" => EvidenceKind::Composition,
            "inspection" => EvidenceKind::Inspection,
            o => {
                return Err(anyhow!(
                    "bad evidence kind {} (automated|composition|inspection)",
                    o
                ))
            }
        };
        let force = b(a, "force");
        let reason = s(a, "reason");
        if force
            && reason
                .as_deref()
                .map(|r| r.trim().is_empty())
                .unwrap_or(true)
        {
            return Err(anyhow!(
                "force=true requires a non-empty reason explaining the override"
            ));
        }
        let promote = b(a, "promote");
        let inherited = p.inherited_sil(&p.safety_requirements[&id]);
        let status = p.safety_requirements[&id].status;

        // REQ-0135: gates bite only on promotion; force (with a reason)
        // overrides them and records a structured audited exception.
        let mut gate_exception = false;
        if promote {
            // REQ-0139: a passing validation dossier is the precondition for
            // a safety requirement to reach Verified (no tag exemption).
            crate::commands::validation::gate_safety_requirement(&p.safety_requirements[&id])?;
            let ladder_ok = matches!(status, Status::Implemented | Status::Verified);
            if !ladder_ok && !force {
                return Err(anyhow!(
                    "{} is {} — promoting straight to Verified is irregular. Advance it to \
                     Implemented first, or pass force=true with a reason.",
                    id,
                    status.as_str()
                ));
            }
            if let Some(sil) = inherited {
                if sil.rank() >= Sil::Sil3.rank() && matches!(kind, EvidenceKind::Inspection) {
                    if force {
                        gate_exception = true;
                    } else {
                        return Err(anyhow!(
                            "SIL-rigour gate: {} inherits {} — it cannot be verified on \
                             inspection-only evidence. Provide automated or composition \
                             evidence, or pass force=true with a reason for an audited exception.",
                            id,
                            sil.as_str()
                        ));
                    }
                }
            }
        }
        let now = Utc::now();
        let mut notes = s(a, "notes").unwrap_or_default();
        let cites = arr(a, "cites");
        if !cites.is_empty() {
            notes = format!("cites {} — {}", cites.join(", "), notes);
        }
        if let (true, Some(r)) = (force, reason.as_deref()) {
            notes = format!("[override: {}] {}", r, notes);
        }
        {
            let sr = p.safety_requirements.get_mut(&id).unwrap();
            sr.tests.push(TestRecord {
                at: now,
                actor: commands::current_actor(),
                commit: git_head(),
                outcome: TestOutcome::Pass,
                notes,
                kind,
                content_hash: None,
                linked_files: None,
                sil_gate_exception: gate_exception,
            });
            if promote {
                sr.status = Status::Verified;
            }
            sr.updated = now;
            // REQ-0135: record the verification (and promotion) on the SR.
            sr.history.push(commands::history(
                if promote {
                    "verified (promoted)"
                } else {
                    "evidence recorded"
                },
                reason,
            ));
        }
        p.updated = now;
        storage::save(file, &p)?;
        Ok(serde_json::to_string_pretty(&p.safety_requirements[&id])?)
    }

    fn git_head() -> String {
        std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default()
    }

    // ----- trace (JSON safety case) -----

    pub fn trace(a: &Value, file: &Path) -> Result<String> {
        let p = storage::load(file)?;
        let raw = req_s(a, "id")?;
        let up = raw.trim().to_uppercase();
        let haz_ids: Vec<String> = if up.starts_with("HAZ") {
            vec![norm("HAZ", &raw)]
        } else if up.starts_with("SF") {
            let id = norm("SF", &raw);
            let sf = p
                .safety_functions
                .get(&id)
                .ok_or_else(|| anyhow!("no such safety function: {}", id))?;
            sf.links
                .iter()
                .filter(|l| l.kind == LinkKind::Mitigates)
                .map(|l| l.target.clone())
                .collect()
        } else if up.starts_with("SR") {
            let id = norm("SR", &raw);
            let sr = p
                .safety_requirements
                .get(&id)
                .ok_or_else(|| anyhow!("no such safety requirement: {}", id))?;
            let mut hs = Vec::new();
            for l in sr.links.iter().filter(|l| l.kind == LinkKind::Realizes) {
                if let Some(sf) = p.safety_functions.get(&l.target) {
                    for m in sf.links.iter().filter(|l| l.kind == LinkKind::Mitigates) {
                        hs.push(m.target.clone());
                    }
                }
            }
            hs
        } else {
            return Err(anyhow!("trace expects a HAZ-/SF-/SR- id; got {}", raw));
        };

        let cases: Vec<Value> = haz_ids
            .iter()
            .filter(|h| p.hazards.contains_key(*h))
            .map(|h| trace_case(&p, h))
            .collect();
        Ok(serde_json::to_string_pretty(&json!({ "cases": cases }))?)
    }

    fn trace_case(p: &Project, haz_id: &str) -> Value {
        let h = &p.hazards[haz_id];
        let required = p.required_sil(h);
        let sfs: Vec<&SafetyFunction> = p
            .safety_functions
            .values()
            .filter(|sf| mitigates(sf, haz_id))
            .collect();
        let allocated = sfs
            .iter()
            .filter_map(|sf| p.allocated_sil(sf))
            .max_by_key(|s| s.rank());
        let adequate = match (required, allocated) {
            (Some(r), Some(al)) => al.rank() >= r.rank(),
            (Some(_), None) => false,
            (None, _) => true,
        };
        let mut sr_total = 0;
        let mut sr_verified = 0;
        let mut blocking: Vec<String> = Vec::new();
        let sf_json: Vec<Value> = sfs
            .iter()
            .map(|sf| {
                let srs: Vec<Value> = p
                    .safety_requirements
                    .values()
                    .filter(|sr| realizes(sr, &sf.id))
                    .map(|sr| {
                        sr_total += 1;
                        let verified = matches!(sr.status, Status::Verified);
                        if verified {
                            sr_verified += 1;
                        } else {
                            blocking.push(format!("{} not verified", sr.id));
                        }
                        json!({
                            "id": sr.id, "title": sr.title, "status": sr.status.as_str(),
                            "inherited_sil": sil_s(p.inherited_sil(sr)),
                            "evidence": sr.tests.last().map(|t| t.kind.as_str()),
                        })
                    })
                    .collect();
                json!({
                    "id": sf.id, "title": sf.title, "status": sf.status.as_str(),
                    "allocated_sil": sil_s(p.allocated_sil(sf)),
                    "safety_requirements": srs,
                })
            })
            .collect();
        if sfs.is_empty() {
            blocking.push("no mitigating safety function".to_string());
        } else if sr_total == 0 {
            blocking.push("no realizing safety requirement".to_string());
        }
        let complete = adequate && blocking.is_empty();
        json!({
            "hazard": { "id": h.id, "title": h.title, "status": h.status.as_str(), "harm": h.harm },
            "required_sil": sil_s(required),
            "allocated_sil": sil_s(allocated),
            "adequate": adequate,
            "complete": complete,
            "safety_requirements": { "total": sr_total, "verified": sr_verified },
            "safety_functions": sf_json,
            "blocking": blocking,
        })
    }
}
