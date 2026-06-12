// REQ-0110 + REQ-0111: req-v2 schema additions — `_purpose` and `_config`
// reserved top-level keys, surfaced in `req brief` and consumed by
// coverage/lint/review respectively.
mod common;
use common::{stderr, stdout, Sandbox};
use std::fs;

// ---------- REQ-0111: project purpose ----------

#[test]
fn req_0111_init_with_purpose_persists() {
    let s = Sandbox::new();
    let out = common::req(&[
        "init",
        "-n",
        "p",
        "-o",
        s.path().to_str().unwrap(),
        "--purpose",
        "Build a CLI for managed requirements with agent-friendly workflow.",
    ]);
    assert!(out.status.success(), "stderr={}", stderr(&out));
    let on_disk = fs::read_to_string(s.path()).unwrap();
    assert!(
        on_disk.contains("\"_purpose\""),
        "_purpose key should appear in file"
    );
    assert!(
        on_disk.contains("\"_format\": \"req-v3\""),
        "init must write the current format tag"
    );
}

#[test]
fn req_0111_init_purpose_exceeds_cap_rejected() {
    let s = Sandbox::new();
    let long: String = "x".repeat(501);
    let out = common::req(&[
        "init",
        "-n",
        "p",
        "-o",
        s.path().to_str().unwrap(),
        "--purpose",
        &long,
    ]);
    assert!(!out.status.success(), "501-char purpose should be rejected");
    assert!(
        stderr(&out).contains("max 500"),
        "error should name the cap, got: {}",
        stderr(&out)
    );
}

#[test]
fn req_0111_purpose_print_when_unset() {
    let s = Sandbox::new();
    s.init("p");
    let out = s.run(&["purpose"]);
    assert!(out.status.success(), "stderr={}", stderr(&out));
    assert!(
        stdout(&out).contains("no purpose set"),
        "expected unset hint, got: {}",
        stdout(&out)
    );
}

#[test]
fn req_0111_purpose_set_and_read_back() {
    let s = Sandbox::new();
    s.init("p");
    let out = s.run(&[
        "purpose",
        "A short statement of what this project is for.",
        "-r",
        "session-zero",
    ]);
    assert!(out.status.success(), "set failed: {}", stderr(&out));

    let read = s.run(&["purpose"]);
    assert!(
        stdout(&read).contains("A short statement of what this project is for."),
        "read-back should produce the set value: {}",
        stdout(&read)
    );
}

#[test]
fn req_0111_purpose_requires_reason_when_setting() {
    let s = Sandbox::new();
    s.init("p");
    let out = s.run(&["purpose", "some new value"]);
    assert!(!out.status.success(), "missing --reason should error");
    assert!(
        stderr(&out).contains("--reason"),
        "error must name the missing flag, got: {}",
        stderr(&out)
    );
}

#[test]
fn req_0111_brief_leads_with_purpose() {
    let s = Sandbox::new();
    let out = common::req(&[
        "init",
        "-n",
        "p",
        "-o",
        s.path().to_str().unwrap(),
        "--purpose",
        "Be the project's session-zero context line.",
    ]);
    assert!(out.status.success(), "init: {}", stderr(&out));
    let brief = s.run(&["brief"]);
    let body = stdout(&brief);
    // Purpose must appear before the headline.
    let purpose_pos = body.find("session-zero context line").unwrap_or(usize::MAX);
    let headline_pos = body.find("req brief:").unwrap_or(usize::MAX);
    assert!(
        purpose_pos < headline_pos,
        "purpose should lead brief, got:\n{}",
        body
    );
}

// ---------- REQ-0110: _config consumed by coverage ----------

#[test]
fn req_0110_config_coverage_extensions_used_when_no_cli_flag() {
    // Set _config.coverage.extensions = ["sql"], then add a .sql file
    // with no REQ marker. coverage --strict should treat it as a
    // markerless source file even though `sql` is not the only ext.
    let s = Sandbox::new();
    s.init("p");
    // Edit the file directly to inject _config then re-sign via repair.
    let mut json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(s.path()).unwrap()).unwrap();
    json.as_object_mut().unwrap().insert(
        "_config".into(),
        serde_json::json!({ "coverage": { "extensions": ["sql"] } }),
    );
    fs::write(s.path(), serde_json::to_string_pretty(&json).unwrap()).unwrap();
    let repair = s.run(&["repair", "--confirm-direct-edit"]);
    assert!(repair.status.success(), "repair: {}", stderr(&repair));

    // List should now show _config present indirectly by surviving a
    // round-trip. The behaviour check: a `.rs` file (NOT in the
    // narrowed ext list) should NOT be scanned for markers; coverage
    // should run cleanly without it being treated as an unlinked
    // source file.
    let body = stdout(&s.run(&["coverage", "--unlinked-files", "--json"]));
    // No requirements yet, so unlinked-files would list a stray .rs
    // file ONLY if .rs is still in the ext list. With ["sql"] override,
    // it shouldn't be.
    fs::create_dir_all(s.dir.path().join("src")).unwrap();
    fs::write(s.dir.path().join("src/lib.rs"), "fn nop() {}\n").unwrap();
    let body2 = stdout(&s.run(&[
        "coverage",
        "--unlinked-files",
        "--json",
        "--path",
        s.dir.path().to_str().unwrap(),
    ]));
    let _ = body; // first call retained for diagnostic on regression
    assert!(
        !body2.contains("src/lib.rs") && !body2.contains("src\\lib.rs"),
        "_config.coverage.extensions=[sql] should exclude .rs from the scan; got: {}",
        body2
    );
}
