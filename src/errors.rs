// Implements REQ-0039 (structured JSON error envelope with stable codes).
// Every CLI subcommand that supports --json should route its error path
// through `emit_json_error` so agents pattern-match on stable codes rather
// than English prose.

use serde_json::json;

pub const E_INTEGRITY: &str = "REQ-E-INTEGRITY";
pub const E_NOT_FOUND: &str = "REQ-E-NOT-FOUND";
pub const E_VALIDATION: &str = "REQ-E-VALIDATION";
pub const E_CYCLE: &str = "REQ-E-CYCLE";
pub const E_DUPLICATE: &str = "REQ-E-DUPLICATE";
pub const E_INVALID_INPUT: &str = "REQ-E-INVALID-INPUT";
pub const E_IO: &str = "REQ-E-IO";
pub const E_INTEGRITY_HINT: &str =
    "Review changes with git diff; if intentional, run `req repair --confirm-direct-edit`.";

pub fn emit(code: &str, message: impl Into<String>, hint: Option<&str>) {
    let body = json!({
        "code": code,
        "message": message.into(),
        "hint": hint.unwrap_or(""),
    });
    // In --json mode the envelope is the program's structured output, so
    // it goes to stdout (the conventional channel for tool-readable data).
    // main.rs suppresses the human-facing anyhow chain in JSON mode so
    // the stream stays parseable.
    println!("{}", serde_json::to_string(&body).unwrap_or_default());
}

/// Classify an anyhow error into a stable REQ-E code by inspecting its
/// rendered message. We intentionally use string matching: the underlying
/// errors are scattered across modules and we don't want to refactor every
/// error site just to enable JSON. Codes are stable; messages are not.
pub fn classify(err: &anyhow::Error) -> &'static str {
    let s = err.to_string();
    let lower = s.to_lowercase();
    if lower.contains("integrity check failed") {
        E_INTEGRITY
    } else if lower.contains("no such requirement") || lower.contains("does not exist") {
        E_NOT_FOUND
    } else if lower.contains("validation error") || lower.contains("rejected:") {
        E_VALIDATION
    } else if lower.contains("cycle") {
        E_CYCLE
    } else if lower.contains("already exists") || lower.contains("duplicate") {
        E_DUPLICATE
    } else if lower.contains("os error") || lower.contains("read") || lower.contains("write") {
        E_IO
    } else {
        E_INVALID_INPUT
    }
}

pub fn hint_for(code: &str) -> Option<&'static str> {
    match code {
        E_INTEGRITY => Some(E_INTEGRITY_HINT),
        E_NOT_FOUND => Some("Run `req list` to see existing IDs."),
        E_VALIDATION => {
            Some("Run `req help best-practice` (or `req help errors`) for the rule catalog.")
        }
        E_CYCLE => {
            Some("Inspect the parent chain with `req show` and break the cycle before relinking.")
        }
        E_DUPLICATE => Some("The target already has this link or tag."),
        E_INVALID_INPUT => Some("Run `req <subcommand> --help` to check the expected arguments."),
        E_IO => Some("Check the file path and permissions."),
        _ => None,
    }
}
