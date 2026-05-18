// Tests for Slice A: REQ-0061 / 0065 / 0071 / 0072 / 0073 / 0074.
// Naming convention: req_NNNN_description (consumed by `req test run`).
mod common;
use common::{stderr, stdout, Sandbox};
use std::fs;

fn add_req(
    s: &Sandbox,
    title: &str,
    statement: &str,
    kind: &str,
    accepts: &[&str],
) -> std::process::Output {
    let mut args = vec![
        "add",
        "--title",
        title,
        "--statement",
        statement,
        "--rationale",
        "Set up for slice A tests.",
        "--kind",
        kind,
        "--priority",
        "could",
    ];
    for a in accepts {
        args.push("--accept");
        args.push(*a);
    }
    s.run(&args)
}

// ---------- REQ-0061: MIT license ----------

#[test]
fn req_0061_license_file_is_mit() {
    let text = fs::read_to_string("LICENSE").expect("LICENSE present at repo root");
    assert!(
        text.contains("MIT License"),
        "LICENSE should say MIT License"
    );
    assert!(
        text.contains("Permission is hereby granted"),
        "MIT body missing"
    );
}

#[test]
fn req_0061_cargo_declares_license() {
    let text = fs::read_to_string("Cargo.toml").expect("Cargo.toml present");
    assert!(
        text.contains("license = \"MIT\""),
        "Cargo.toml should declare license = \"MIT\""
    );
}

#[test]
fn req_0061_readme_links_license() {
    let text = fs::read_to_string("README.md").expect("README present");
    assert!(
        text.contains("](LICENSE)") || text.contains("[LICENSE](LICENSE)"),
        "README should link to LICENSE"
    );
}

// ---------- REQ-0074: format-version policy section ----------

#[test]
fn req_0074_help_has_format_policy_section() {
    let out = common::req(&["help", "format-policy"]);
    assert!(
        out.status.success(),
        "req help format-policy should succeed"
    );
    let body = stdout(&out);
    assert!(
        body.contains("req migrate"),
        "policy should mention req migrate"
    );
    assert!(
        body.contains("_format"),
        "policy should describe _format tag"
    );
}

#[test]
fn req_0074_help_index_lists_format_policy() {
    let out = common::req(&["help"]);
    let body = stdout(&out);
    assert!(
        body.contains("format-policy"),
        "help index should list format-policy section"
    );
}

// ---------- REQ-0073: hide obsolete from default list ----------

#[test]
fn req_0073_obsolete_hidden_from_default_list() {
    let s = Sandbox::new();
    s.init("p");
    add_req(
        &s,
        "Live and well thing",
        "The system shall keep this active forever.",
        "constraint",
        &[],
    );
    add_req(
        &s,
        "Retire this old one",
        "The system shall keep this one for a while.",
        "constraint",
        &[],
    );
    let _ = s.run(&["delete", "REQ-0002", "--reason", "no longer needed"]);
    let out = stdout(&s.run(&["list", "--json"]));
    assert!(out.contains("REQ-0001"), "live req should appear");
    assert!(
        !out.contains("REQ-0002"),
        "obsolete req should be hidden by default, got: {}",
        out
    );
}

#[test]
fn req_0073_include_obsolete_flag_brings_them_back() {
    let s = Sandbox::new();
    s.init("p");
    add_req(
        &s,
        "Live and well thing",
        "The system shall keep this active forever.",
        "constraint",
        &[],
    );
    add_req(
        &s,
        "Retire this old one",
        "The system shall keep this one for a while.",
        "constraint",
        &[],
    );
    let _ = s.run(&["delete", "REQ-0002", "--reason", "no longer needed"]);
    let out = stdout(&s.run(&["list", "--include-obsolete", "--json"]));
    assert!(
        out.contains("REQ-0002"),
        "--include-obsolete should bring obsolete back: {}",
        out
    );
}

#[test]
fn req_0073_status_obsolete_filter_still_works() {
    let s = Sandbox::new();
    s.init("p");
    add_req(
        &s,
        "Live and well thing",
        "The system shall keep this active forever.",
        "constraint",
        &[],
    );
    add_req(
        &s,
        "Retire this old one",
        "The system shall keep this one for a while.",
        "constraint",
        &[],
    );
    let _ = s.run(&["delete", "REQ-0002", "--reason", "no longer needed"]);
    let out = stdout(&s.run(&["list", "--status", "obsolete", "--json"]));
    assert!(
        out.contains("REQ-0002"),
        "explicit --status obsolete should show: {}",
        out
    );
    assert!(
        !out.contains("REQ-0001"),
        "explicit --status obsolete should NOT show live reqs: {}",
        out
    );
}

// ---------- REQ-0065: coverage --strict ----------

#[test]
fn req_0065_coverage_strict_exits_nonzero_with_orphans() {
    let s = Sandbox::new();
    s.init("p");
    add_req(
        &s,
        "Orphan candidate one",
        "The system shall have one untested requirement.",
        "constraint",
        &[],
    );
    // Walk past Draft: coverage excludes Drafts from the orphan
    // check by design (a Draft has no implementation yet). For the
    // strict-mode-trips-on-orphans assertion to apply we need a
    // requirement that should have a marker but doesn't.
    let _ = s.run(&[
        "update",
        "REQ-0001",
        "--status",
        "implemented",
        "--reason",
        "test fixture: orphan candidate at implemented",
        "--force",
    ]);
    let out = s.run(&[
        "coverage",
        "--path",
        s.dir.path().to_str().unwrap(),
        "--strict",
    ]);
    assert!(
        !out.status.success(),
        "orphan should make strict mode fail; stderr={}",
        stderr(&out)
    );
}

#[test]
fn req_0065_coverage_default_remains_informational() {
    let s = Sandbox::new();
    s.init("p");
    add_req(
        &s,
        "Orphan candidate one",
        "The system shall have one untested requirement.",
        "constraint",
        &[],
    );
    let out = s.run(&["coverage", "--path", s.dir.path().to_str().unwrap()]);
    assert!(
        out.status.success(),
        "default mode should be zero-exit even with orphans"
    );
}

#[test]
fn req_0065_coverage_strict_passes_when_clean() {
    let s = Sandbox::new();
    s.init("p");
    let out = s.run(&[
        "coverage",
        "--path",
        s.dir.path().to_str().unwrap(),
        "--strict",
    ]);
    assert!(
        out.status.success(),
        "empty project has no orphans, strict should pass"
    );
}

// ---------- REQ-0072: req add --from-json ----------

#[test]
fn req_0072_add_from_json_file() {
    let s = Sandbox::new();
    s.init("p");
    let doc = serde_json::json!({
        "title": "Added through a JSON document",
        "statement": "The system shall accept new requirements via --from-json.",
        "rationale": "Bypasses shell quoting for multi-line content.",
        "kind": "functional",
        "priority": "should",
        "acceptance": ["A JSON file produces a valid requirement"],
        "tags": ["json", "smoke"]
    });
    let json_path = s.dir.path().join("new-req.json");
    fs::write(&json_path, serde_json::to_string_pretty(&doc).unwrap()).unwrap();
    let out = s.run(&["add", "--from-json", json_path.to_str().unwrap(), "--json"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let body = stdout(&out);
    assert!(body.contains("Added through a JSON document"));
    assert!(body.contains("\"json\""), "tags should round-trip");
}

#[test]
fn req_0072_add_from_json_validator_still_rejects() {
    let s = Sandbox::new();
    s.init("p");
    let doc = serde_json::json!({
        "title": "Bad",
        "statement": "too short",
        "rationale": "Verify validator path is shared.",
        "kind": "constraint",
        "priority": "could"
    });
    let json_path = s.dir.path().join("bad.json");
    fs::write(&json_path, serde_json::to_string(&doc).unwrap()).unwrap();
    let out = s.run(&["add", "--from-json", json_path.to_str().unwrap()]);
    assert!(
        !out.status.success(),
        "validator should still reject bad input from JSON"
    );
    assert!(
        stderr(&out).contains("title is too short")
            || stderr(&out).contains("modal verb")
            || stderr(&out).contains("complete sentence")
    );
}

// ---------- REQ-0071: gitattributes pinning ----------

#[test]
fn req_0071_hooks_install_writes_gitattributes_pin() {
    let s = Sandbox::new();
    s.init("p");
    // Make the sandbox a git repo so hooks install can do its work.
    let _ = std::process::Command::new("git")
        .args(["init", "-q", s.dir.path().to_str().unwrap()])
        .output()
        .expect("git init");
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_req"))
        .args([
            "hooks",
            "install",
            "--repo",
            s.dir.path().to_str().unwrap(),
            "--force",
        ])
        .output()
        .expect("invoke req");
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let attrs =
        fs::read_to_string(s.dir.path().join(".gitattributes")).expect(".gitattributes written");
    assert!(
        attrs.contains("project.req -text eol=lf"),
        "should pin project.req: {}",
        attrs
    );
    assert!(
        attrs.contains("*.req merge=req-merge"),
        "should keep merge driver: {}",
        attrs
    );
}

#[test]
fn req_0071_hooks_install_is_idempotent() {
    let s = Sandbox::new();
    s.init("p");
    let _ = std::process::Command::new("git")
        .args(["init", "-q", s.dir.path().to_str().unwrap()])
        .output()
        .expect("git init");
    for _ in 0..2 {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_req"))
            .args([
                "hooks",
                "install",
                "--repo",
                s.dir.path().to_str().unwrap(),
                "--force",
            ])
            .output()
            .expect("invoke req");
        assert!(out.status.success());
    }
    let attrs =
        fs::read_to_string(s.dir.path().join(".gitattributes")).expect(".gitattributes written");
    let pin_count = attrs.matches("project.req -text eol=lf").count();
    assert_eq!(
        pin_count, 1,
        "pin should appear exactly once, got {}: {}",
        pin_count, attrs
    );
}
