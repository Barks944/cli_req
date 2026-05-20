// End-to-end smoke tests covering the main user journey: init, add, list,
// show, update, delete, link, export, validate. Each is named after the
// requirement it most directly exercises.
mod common;
use common::{stderr, stdout, Sandbox};

#[test]
fn req_0037_version_short_flags_print_same_string() {
    let v_lower = common::req(&["-v"]);
    let v_upper = common::req(&["-V"]);
    let v_long = common::req(&["--version"]);
    let v_sub = common::req(&["version"]);
    assert!(v_lower.status.success());
    let lower = stdout(&v_lower);
    let upper = stdout(&v_upper);
    let long = stdout(&v_long);
    let sub = stdout(&v_sub);
    assert_eq!(
        lower.trim(),
        upper.trim(),
        "-v and -V must print the same line"
    );
    assert_eq!(lower.trim(), long.trim(), "-v and --version must agree");
    assert_eq!(lower.trim(), sub.trim(), "-v and `req version` must agree");
    assert!(
        lower.starts_with("req "),
        "version line should start with the binary name"
    );
}

#[test]
fn req_0001_help_lists_every_subcommand() {
    let out = common::req(&["--help"]);
    let body = stdout(&out);
    for sub in &[
        "init", "add", "list", "show", "update", "delete", "link", "validate", "export", "tui",
        "serve", "mcp", "help", "repair", "status", "next", "check",
    ] {
        assert!(body.contains(sub), "--help missing subcommand `{}`", sub);
    }
}

// ---------- REQ-0114: req precheck ----------

#[test]
fn req_0114_precheck_listed_in_help() {
    let out = common::req(&["--help"]);
    assert!(
        stdout(&out).contains("precheck"),
        "precheck should appear in --help"
    );
}

#[test]
fn req_0114_precheck_unknown_skip_step_rejected() {
    let s = Sandbox::new();
    s.init("p");
    let out = s.run(&["precheck", "--skip", "polish-the-cat"]);
    assert!(
        !out.status.success(),
        "unknown skip step should be rejected"
    );
    assert!(
        stderr(&out).contains("unknown --skip step"),
        "error should name the unknown step, got: {}",
        stderr(&out)
    );
}

#[test]
fn req_0114_precheck_skip_all_steps_runs_clean() {
    // Skipping every step is a way to verify the wiring without
    // recursively launching cargo (which would infinite-loop from
    // inside `cargo test`).
    let s = Sandbox::new();
    s.init("p");
    let out = s.run(&[
        "precheck", "--skip", "fmt", "--skip", "clippy", "--skip", "test", "--skip", "validate",
        "--skip", "coverage", "--skip", "review",
    ]);
    assert!(
        out.status.success(),
        "all-skipped precheck should succeed; stderr={}",
        stderr(&out)
    );
    assert!(
        stdout(&out).contains("precheck OK"),
        "expected success banner, stdout: {}",
        stdout(&out)
    );
}

#[test]
fn req_0014_export_markdown_round_trip() {
    let s = Sandbox::new();
    s.init("export-test");
    s.run(&[
        "add",
        "--title",
        "Greet on launch",
        "--statement",
        "The system shall greet the user when the application starts.",
        "--rationale",
        "Welcome flow.",
        "--kind",
        "functional",
        "--priority",
        "should",
        "--accept",
        "Launch shows the greeting modal within 200ms",
    ]);
    let out = s.run(&["export", "-f", "markdown"]);
    let md = stdout(&out);
    assert!(md.contains("REQ-0001"));
    assert!(md.contains("Greet on launch"));
    assert!(md.contains("**Statement.**"));
    assert!(md.contains("**Acceptance criteria:**"));
}

#[test]
fn req_0013_parent_cycle_rejected() {
    let s = Sandbox::new();
    s.init("cycle-test");
    for i in 1..=2 {
        s.run(&[
            "add",
            "--title",
            &format!("Node number {}", i),
            "--statement",
            "The system shall implement this generic node behaviour.",
            "--rationale",
            "Hierarchy stub.",
            "--kind",
            "constraint",
            "--priority",
            "could",
        ]);
    }
    let a = s.run(&["link", "REQ-0001", "REQ-0002", "-k", "parent"]);
    assert!(a.status.success());
    let b = s.run(&["link", "REQ-0002", "REQ-0001", "-k", "parent"]);
    assert!(!b.status.success(), "cycle should be rejected");
}

#[test]
fn req_0012_soft_delete_preserves_inbound_links() {
    let s = Sandbox::new();
    s.init("delete-test");
    for i in 1..=2 {
        s.run(&[
            "add",
            "--title",
            &format!("Item number {}", i),
            "--statement",
            "The system shall behave as expected for this item.",
            "--rationale",
            "Stub.",
            "--kind",
            "constraint",
            "--priority",
            "could",
        ]);
    }
    s.run(&["link", "REQ-0001", "REQ-0002", "-k", "parent"]);
    // Hard delete should refuse because REQ-0001 references REQ-0002
    let hard = s.run(&["delete", "REQ-0002", "--hard", "--reason", "test"]);
    assert!(!hard.status.success());
    // Soft delete is allowed
    let soft = s.run(&["delete", "REQ-0002", "--reason", "test"]);
    assert!(soft.status.success());
    // REQ-0002 still present, marked obsolete
    let show = stdout(&s.run(&["show", "REQ-0002"]));
    assert!(show.contains("obsolete"));
}

#[test]
fn req_0038_add_json_emits_stdout_json() {
    let s = Sandbox::new();
    s.init("json-test");
    let out = s.run(&[
        "add",
        "--json",
        "--title",
        "JSON add smoke",
        "--statement",
        "The system shall return JSON on stdout when --json is set.",
        "--rationale",
        "Verify REQ-0038.",
        "--kind",
        "constraint",
        "--priority",
        "could",
    ]);
    let body = stdout(&out);
    assert!(
        body.trim_start().starts_with('{'),
        "stdout should be JSON: {}",
        body
    );
    let v: serde_json::Value = serde_json::from_str(&body).expect("valid JSON");
    assert_eq!(v["id"].as_str().unwrap(), "REQ-0001");
}

#[test]
fn req_0039_json_error_envelope_on_failure() {
    let s = Sandbox::new();
    s.init("err-test");
    // Build the bogus ID through formatting so the source file contains
    // no literal REQ-NNNN marker (avoids a coverage-scan ghost finding).
    let bogus = format!("REQ-{:04}", 9999);
    let out = s.run(&["show", &bogus, "--json"]);
    assert!(!out.status.success());
    // --json mode emits the structured envelope to stdout (the
    // parseable channel) so callers can JSON.parse() stdout directly.
    // Stderr stays quiet — no duplicate anyhow chain.
    let body = String::from_utf8_lossy(&out.stdout);
    let parsed: serde_json::Value = serde_json::from_str(body.trim())
        .unwrap_or_else(|e| panic!("stdout not JSON: {}\nstdout was: {}", e, body));
    assert_eq!(parsed["code"].as_str().unwrap(), "REQ-E-NOT-FOUND");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        !err.contains("Error:"),
        "stderr should not carry an anyhow chain in --json mode: {}",
        err
    );
}
