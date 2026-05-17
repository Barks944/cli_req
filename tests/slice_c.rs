// Tests for Slice C: REQ-0066 (batch) and REQ-0067 (import).
mod common;
use common::{stderr, stdout, Sandbox};
use std::fs;

// ---------- REQ-0066: req batch ----------

#[test]
fn req_0066_batch_applies_multiple_mutations_atomically() {
    let s = Sandbox::new();
    s.init("p");
    let doc = serde_json::json!({
        "reason": "First batch smoke",
        "mutations": [
            { "kind": "add",
              "title": "First batched req here",
              "statement": "The system shall accept this from a batch document.",
              "rationale": "Test the batch path.",
              "req_kind": "constraint", "priority": "could" },
            { "kind": "add",
              "title": "Second batched req here",
              "statement": "The system shall also accept this from a batch document.",
              "rationale": "Test the batch path.",
              "req_kind": "constraint", "priority": "could" },
            { "kind": "link", "from": "REQ-0002", "to": "REQ-0001", "link_kind": "parent" }
        ]
    });
    let path = s.dir.path().join("batch.json");
    fs::write(&path, serde_json::to_string_pretty(&doc).unwrap()).unwrap();
    let out = s.run(&["batch", path.to_str().unwrap()]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let list = stdout(&s.run(&["list", "--json"]));
    assert!(list.contains("REQ-0001"));
    assert!(list.contains("REQ-0002"));
}

#[test]
fn req_0066_batch_rolls_back_on_validation_failure() {
    let s = Sandbox::new();
    s.init("p");
    // Take a snapshot of the project file BEFORE the bad batch
    let before = fs::read(s.path()).unwrap();
    let doc = serde_json::json!({
        "mutations": [
            { "kind": "add",
              "title": "First good req here",
              "statement": "The system shall accept this perfectly fine requirement.",
              "rationale": "Good one.",
              "req_kind": "constraint", "priority": "could" },
            // This one fails (no modal verb)
            { "kind": "add",
              "title": "Second bad req here",
              "statement": "Does nothing useful at all here.",
              "rationale": "Bad — no modal verb.",
              "req_kind": "constraint", "priority": "could" }
        ]
    });
    let path = s.dir.path().join("bad.json");
    fs::write(&path, serde_json::to_string_pretty(&doc).unwrap()).unwrap();
    let out = s.run(&["batch", path.to_str().unwrap()]);
    assert!(!out.status.success());
    // File must be byte-identical to its pre-batch state
    let after = fs::read(s.path()).unwrap();
    assert_eq!(
        before, after,
        "project.req should be unchanged after rollback"
    );
}

// ---------- REQ-0067: req import ----------

#[test]
fn req_0067_import_markdown_accepts_well_formed_items() {
    let s = Sandbox::new();
    s.init("p");
    let md = r#"
## First imported requirement here

The system shall implement the first imported behaviour as described.

Rationale: This is an import smoke test.

Acceptance:
- The first behaviour is reproducible
- Coverage notices it

Tags: import, smoke

## Second imported requirement here

The system shall also implement a second behaviour from markdown import.

Rationale: Multiple items in one document.

Acceptance:
- Second behaviour is independently testable
"#;
    let path = s.dir.path().join("spec.md");
    fs::write(&path, md).unwrap();
    let out = s.run(&["import", "-f", "markdown", path.to_str().unwrap(), "--json"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let body = stdout(&out);
    let v: serde_json::Value = serde_json::from_str(&body).expect("import --json shape");
    assert_eq!(v["accepted"].as_array().unwrap().len(), 2);
    assert!(v["rejected"].as_array().unwrap().is_empty());
}

#[test]
fn req_0067_import_dry_run_does_not_write() {
    let s = Sandbox::new();
    s.init("p");
    let before = fs::read(s.path()).unwrap();
    let md = r#"
## Dry-run imported requirement

The system shall be visible in the report but not written when dry-run is set.

Rationale: Test the --dry-run flag.
"#;
    let path = s.dir.path().join("dry.md");
    fs::write(&path, md).unwrap();
    let out = s.run(&[
        "import",
        "-f",
        "markdown",
        path.to_str().unwrap(),
        "--dry-run",
    ]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let after = fs::read(s.path()).unwrap();
    assert_eq!(before, after, "project.req unchanged on dry-run");
}

#[test]
fn req_0067_import_json_array_schema() {
    let s = Sandbox::new();
    s.init("p");
    let doc = serde_json::json!([
        {
            "title": "From JSON array form",
            "statement": "The system shall ingest items from a flat JSON array.",
            "rationale": "Convenient bulk import path.",
            "kind": "constraint",
            "priority": "could"
        }
    ]);
    let path = s.dir.path().join("items.json");
    fs::write(&path, serde_json::to_string_pretty(&doc).unwrap()).unwrap();
    let out = s.run(&["import", "-f", "json", path.to_str().unwrap(), "--json"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let v: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(v["accepted"].as_array().unwrap().len(), 1);
}
