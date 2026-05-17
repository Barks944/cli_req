// Tests for the final round: REQ-0075 (directory storage), REQ-0076
// (duplicate-intent detection), REQ-0077 (verifies-without-evidence),
// REQ-0078 (schema), REQ-0079 (audit gate), REQ-0080 (CHANGELOG).
mod common;
use common::{stderr, stdout, Sandbox};
use std::fs;
use std::process::Command;

// ---------- REQ-0075: directory-backed storage ----------

#[test]
fn req_0075_init_directory_layout_writes_index_and_requirements_dir() {
    let s = Sandbox::new();
    let dir = s.dir.path().join("proj");
    let out = Command::new(env!("CARGO_BIN_EXE_req"))
        .args([
            "init",
            "-n",
            "dir-proj",
            "-o",
            dir.to_str().unwrap(),
            "--layout",
            "directory",
        ])
        .output()
        .expect("init");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(dir.join("index.req").exists());
    assert!(dir.join("requirements").is_dir());
}

#[test]
fn req_0075_add_persists_one_file_per_requirement() {
    let s = Sandbox::new();
    let dir = s.dir.path().join("proj");
    Command::new(env!("CARGO_BIN_EXE_req"))
        .args([
            "init",
            "-n",
            "dir-proj",
            "-o",
            dir.to_str().unwrap(),
            "--layout",
            "directory",
        ])
        .output()
        .expect("init");
    let out = Command::new(env!("CARGO_BIN_EXE_req"))
        .args([
            "--file",
            dir.to_str().unwrap(),
            "add",
            "--title",
            "Persisted under the directory layout here",
            "--statement",
            "The system shall write this requirement to its own file under requirements/.",
            "--rationale",
            "Test fixture.",
            "--kind",
            "constraint",
            "--priority",
            "could",
        ])
        .output()
        .expect("add");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        dir.join("requirements/REQ-0001.req").exists(),
        "REQ-0001.req should exist under requirements/"
    );
}

#[test]
fn req_0075_integrity_detects_per_file_tamper() {
    let s = Sandbox::new();
    let dir = s.dir.path().join("proj");
    Command::new(env!("CARGO_BIN_EXE_req"))
        .args([
            "init",
            "-n",
            "dir-proj",
            "-o",
            dir.to_str().unwrap(),
            "--layout",
            "directory",
        ])
        .output()
        .expect("init");
    Command::new(env!("CARGO_BIN_EXE_req"))
        .args([
            "--file",
            dir.to_str().unwrap(),
            "add",
            "--title",
            "Will be tampered with in this test",
            "--statement",
            "The system shall persist this so we can mutate the file.",
            "--rationale",
            "Test.",
            "--kind",
            "constraint",
            "--priority",
            "could",
        ])
        .output()
        .expect("add");
    // Tamper the per-requirement file.
    let req_path = dir.join("requirements/REQ-0001.req");
    let text = fs::read_to_string(&req_path).unwrap();
    fs::write(&req_path, text.replace("\"Could\"", "\"Should\"")).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_req"))
        .args(["--file", dir.to_str().unwrap(), "list"])
        .output()
        .expect("list");
    assert!(
        !out.status.success(),
        "list should refuse after per-file tamper"
    );
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("integrity check failed"));
}

// ---------- REQ-0076: duplicate-intent detection ----------

#[test]
fn req_0076_near_clone_triggers_dup_intent_warning() {
    let s = Sandbox::new();
    s.init("p");
    let _ = s.run(&[
        "add",
        "--title",
        "Persist user sessions across restarts forever",
        "--statement",
        "The system shall persist user sessions across process restarts.",
        "--rationale",
        "Users lose work today.",
        "--kind",
        "functional",
        "--priority",
        "should",
        "--accept",
        "Session survives restart in fixture",
    ]);
    // Near-clone: same intent, very similar wording
    let _ = s.run(&[
        "add",
        "--title",
        "Persist user sessions across process restarts",
        "--statement",
        "The system shall persist user sessions across process restarts always.",
        "--rationale",
        "Same intent, different words.",
        "--kind",
        "functional",
        "--priority",
        "should",
        "--accept",
        "Session survives restart in fixture as well",
    ]);
    let out = s.run(&["validate"]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("REQ-V-0020"),
        "expected duplicate-intent warning, got:\n{}",
        text
    );
}

// ---------- REQ-0077: verifies link without evidence ----------

#[test]
fn req_0077_verifies_link_without_test_record_warns() {
    let s = Sandbox::new();
    s.init("p");
    // Two reqs, neither has any test record
    for i in 1..=2 {
        s.run(&[
            "add",
            "--title",
            &format!("Subject of the verification {}", i),
            "--statement",
            "The system shall have this perfectly fine baseline behaviour.",
            "--rationale",
            "Setup.",
            "--kind",
            "constraint",
            "--priority",
            "could",
        ]);
    }
    // REQ-0002 verifies REQ-0001 — but REQ-0002 has no test record
    s.run(&["link", "REQ-0002", "REQ-0001", "-k", "verifies"]);
    let out = s.run(&["validate"]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("REQ-V-0019"),
        "expected verifies-without-evidence warning, got:\n{}",
        text
    );
}

// ---------- REQ-0078: req schema ----------

#[test]
fn req_0078_schema_add_is_valid_json_with_format() {
    let out = common::req(&["schema", "add"]);
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("schema add is JSON");
    assert_eq!(
        v["$schema"].as_str().unwrap(),
        "https://json-schema.org/draft/2020-12/schema"
    );
    assert!(
        v["$id"]
            .as_str()
            .unwrap()
            .starts_with("urn:req-cli:schema:"),
        "schema $id should be a stable urn:, got: {}",
        v["$id"]
    );
    assert!(v["properties"]["title"].is_object());
    assert!(v["properties"]["statement"].is_object());
    assert_eq!(v["_format"].as_str().unwrap(), "req-v1");
}

#[test]
fn req_0078_schema_batch_describes_oneof_mutations() {
    let out = common::req(&["schema", "batch"]);
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("schema batch is JSON");
    let mutations = &v["properties"]["mutations"]["items"]["oneOf"];
    assert!(
        mutations.is_array(),
        "batch schema should describe mutation alternatives"
    );
    assert_eq!(mutations.as_array().unwrap().len(), 4);
}

// ---------- REQ-0079: audit gate ----------

#[test]
fn req_0079_audit_gate_exits_nonzero_without_signing() {
    // Build a temp git repo, commit something touching project.req
    let s = Sandbox::new();
    s.init("p");
    let dir = s.dir.path();
    let _ = std::process::Command::new("git")
        .current_dir(dir)
        .args(["init", "-q", "-b", "main"])
        .output();
    let _ = std::process::Command::new("git")
        .current_dir(dir)
        .args(["config", "user.email", "t@example.com"])
        .output();
    let _ = std::process::Command::new("git")
        .current_dir(dir)
        .args(["config", "user.name", "T"])
        .output();
    let _ = std::process::Command::new("git")
        .current_dir(dir)
        .args(["add", "project.req"])
        .output();
    let _ = std::process::Command::new("git")
        .current_dir(dir)
        .args(["commit", "-q", "-m", "init"])
        .output();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_req"))
        .current_dir(dir)
        .args([
            "--file",
            s.path().to_str().unwrap(),
            "audit",
            "--gate",
            "--require-good-signature",
        ])
        .output()
        .expect("audit gate");
    assert!(!out.status.success(), "unsigned commit should violate gate");
}

// ---------- REQ-0080: CHANGELOG.md ----------

#[test]
fn req_0080_changelog_exists_with_unreleased_section() {
    let text = fs::read_to_string("CHANGELOG.md").expect("CHANGELOG.md present");
    assert!(text.contains("# Changelog"));
    assert!(text.contains("[Unreleased]"));
    assert!(text.to_lowercase().contains("keep a changelog"));
}
