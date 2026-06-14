// REQ-0125: status / brief / lint surface Verified requirements whose
// latest test record is a Fail. "100% verified" with N failing latest
// records is the worst-kind-of-misleading signal — these tests prove
// the count is surfaced consistently across the three surfaces.
mod common;
use common::{stderr, stdout, Sandbox};
use std::process::Command;

fn init_with_one_failing_verified(s: &Sandbox) {
    s.init("p");
    Command::new("git")
        .args(["init", "-q"])
        .current_dir(s.dir.path())
        .output()
        .expect("git init");
    let _ = Command::new("git")
        .args(["config", "user.email", "t@e"])
        .current_dir(s.dir.path())
        .output();
    let _ = Command::new("git")
        .args(["config", "user.name", "t"])
        .current_dir(s.dir.path())
        .output();
    let _ = Command::new("git")
        .args(["config", "commit.gpgsign", "false"])
        .current_dir(s.dir.path())
        .output();
    std::fs::write(s.dir.path().join("seed"), "x").unwrap();
    let _ = Command::new("git")
        .args(["add", "-A"])
        .current_dir(s.dir.path())
        .output();
    let _ = Command::new("git")
        .args(["commit", "-q", "-m", "seed"])
        .current_dir(s.dir.path())
        .output();

    // One Verified req with a failing latest test record.
    s.run(&[
        "add",
        "--title",
        "Has a defective verified record",
        "--statement",
        "The system shall expose this defective baseline behaviour.",
        "--rationale",
        "Defect fixture.",
        "--kind",
        "constraint",
        "--priority",
        "could",
        // REQ-0139: this fixture is about a failing-test-record (REQ-V-0024),
        // not the validation dossier — exempt it from the dossier gate so
        // only the intended warning fires.
        "--tag",
        "validation-exempt",
    ]);
    for status in ["proposed", "approved", "implemented", "verified"] {
        let _ = s.run(&[
            "update", "REQ-0001", "--status", status, "--reason", "fixture", "--force",
        ]);
    }
    // Attach a failing test record at HEAD.
    let _ = Command::new(env!("CARGO_BIN_EXE_req"))
        .current_dir(s.dir.path())
        .args([
            "--file",
            s.path().to_str().unwrap(),
            "test",
            "record",
            "REQ-0001",
            "--result",
            "fail",
            "--notes",
            "regression in fixture",
        ])
        .output()
        .expect("record");
}

#[test]
fn req_0125_status_surfaces_defects() {
    let s = Sandbox::new();
    init_with_one_failing_verified(&s);
    let out = s.run(&["status", "--json"]);
    let v: serde_json::Value = serde_json::from_str(&stdout(&out)).expect("status JSON");
    let defective = v["verified_but_defective"]
        .as_array()
        .expect("verified_but_defective array");
    assert!(
        defective.iter().any(|x| x == "REQ-0001"),
        "REQ-0001 (Verified + failing latest record) should appear; got: {:?}",
        defective
    );
}

#[test]
fn req_0125_status_human_output_carries_the_count() {
    let s = Sandbox::new();
    init_with_one_failing_verified(&s);
    let out = s.run(&["status"]);
    assert!(
        out.status.success(),
        "status should succeed; stderr={}",
        stderr(&out)
    );
    let body = stdout(&out);
    assert!(
        body.contains("verified-but-defective: 1"),
        "human output should report the defect count, got:\n{}",
        body
    );
}

#[test]
fn req_0125_brief_surfaces_defects() {
    let s = Sandbox::new();
    init_with_one_failing_verified(&s);
    let out = s.run(&["brief", "--json"]);
    let v: serde_json::Value = serde_json::from_str(&stdout(&out)).expect("brief JSON");
    let defective = v["verified_but_defective"]
        .as_array()
        .expect("verified_but_defective array on brief");
    assert!(
        defective.iter().any(|x| x == "REQ-0001"),
        "brief should expose verified_but_defective; got: {:?}",
        defective
    );
}

// REQ-0126: review --gate --no-defects flips exit code on failing latest.
#[test]
fn req_0126_review_no_defects_gate_fails_when_defect_present() {
    let s = Sandbox::new();
    init_with_one_failing_verified(&s);
    let out = s.run(&[
        "review",
        "--base",
        "HEAD",
        "--gate",
        "--no-defects",
        "--json",
    ]);
    assert!(
        !out.status.success(),
        "--gate --no-defects must exit non-zero when a defect exists; stdout={}",
        stdout(&out)
    );
    let v: serde_json::Value = serde_json::from_str(&stdout(&out)).expect("review JSON");
    let defects = v["defects"].as_array().expect("defects array");
    assert!(
        defects.iter().any(|x| x == "REQ-0001"),
        "review JSON should expose defects; got: {:?}",
        defects
    );
}

#[test]
fn req_0126_review_without_no_defects_does_not_block_on_defects() {
    let s = Sandbox::new();
    init_with_one_failing_verified(&s);
    let out = s.run(&["review", "--base", "HEAD", "--gate", "--json"]);
    assert!(
        out.status.success(),
        "--gate without --no-defects must NOT block on defects; stderr={}",
        stderr(&out)
    );
}

// REQ-0129: req test list inspects test records on one req.
#[test]
fn req_0129_test_list_prints_records() {
    let s = Sandbox::new();
    init_with_one_failing_verified(&s);
    let out = s.run(&["test", "list", "REQ-0001", "--json"]);
    let body = stdout(&out);
    let v: serde_json::Value = serde_json::from_str(&body).expect("test list JSON");
    let arr = v.as_array().expect("records array");
    assert_eq!(arr.len(), 1, "should have one test record; got {:?}", arr);
    assert_eq!(arr[0]["outcome"], "Fail");
}

#[test]
fn req_0129_test_list_empty_records_clear_message() {
    let s = Sandbox::new();
    s.init("p");
    s.run(&[
        "add",
        "--title",
        "Has no test records anywhere",
        "--statement",
        "The system shall expose this baseline behaviour without ever being tested.",
        "--rationale",
        "Fixture for empty test list output.",
        "--kind",
        "constraint",
        "--priority",
        "could",
    ]);
    let out = s.run(&["test", "list", "REQ-0001"]);
    assert!(out.status.success(), "stderr={}", stderr(&out));
    assert!(
        stdout(&out).contains("(no test records)"),
        "expected (no test records) hint, got: {}",
        stdout(&out)
    );
}

// REQ-0130: `req validate` emits a REQ-V-0024 warning on Verified
// requirements whose latest test record is a Fail.
#[test]
fn req_0130_validate_warns_on_verified_with_failing_latest() {
    let s = Sandbox::new();
    init_with_one_failing_verified(&s);
    let out = s.run(&["validate", "--json"]);
    let body = stdout(&out);
    let v: serde_json::Value = serde_json::from_str(&body).expect("validate JSON");
    // findings is the per-req findings array
    let findings = v["findings"].as_array().expect("findings array");
    let any_0024 = findings
        .iter()
        .any(|f| f["rule_code"].as_str() == Some("REQ-V-0024"));
    assert!(
        any_0024,
        "expected REQ-V-0024 warning on the verified-but-failing fixture; findings={}",
        body
    );
}

#[test]
fn req_0130_validate_does_not_fail_on_warning_only() {
    let s = Sandbox::new();
    init_with_one_failing_verified(&s);
    let out = s.run(&["validate"]);
    assert!(
        out.status.success(),
        "REQ-V-0024 is a warning; exit code must remain zero. stderr={}",
        stderr(&out)
    );
}

#[test]
fn req_0130_verified_with_pass_latest_clean() {
    // Walk a req to Verified with a passing latest record; expect no warning.
    let s = Sandbox::new();
    s.init("p");
    s.run(&[
        "add",
        "--title",
        "Verified and passing baseline",
        "--statement",
        "The system shall expose this passing baseline behaviour reliably.",
        "--rationale",
        "Fixture.",
        "--kind",
        "constraint",
        "--priority",
        "could",
    ]);
    for status in ["proposed", "approved", "implemented", "verified"] {
        let _ = s.run(&[
            "update", "REQ-0001", "--status", status, "--reason", "fixture", "--force",
        ]);
    }
    // Init git so test record can attach a commit.
    let _ = Command::new("git")
        .args(["init", "-q"])
        .current_dir(s.dir.path())
        .output();
    let _ = Command::new("git")
        .args(["config", "user.email", "t@e"])
        .current_dir(s.dir.path())
        .output();
    let _ = Command::new("git")
        .args(["config", "user.name", "t"])
        .current_dir(s.dir.path())
        .output();
    let _ = Command::new("git")
        .args(["config", "commit.gpgsign", "false"])
        .current_dir(s.dir.path())
        .output();
    std::fs::write(s.dir.path().join("seed"), "x").unwrap();
    let _ = Command::new("git")
        .args(["add", "-A"])
        .current_dir(s.dir.path())
        .output();
    let _ = Command::new("git")
        .args(["commit", "-q", "-m", "seed"])
        .current_dir(s.dir.path())
        .output();
    let _ = Command::new(env!("CARGO_BIN_EXE_req"))
        .current_dir(s.dir.path())
        .args([
            "--file",
            s.path().to_str().unwrap(),
            "test",
            "record",
            "REQ-0001",
            "--result",
            "pass",
            "--notes",
            "ok",
        ])
        .output()
        .expect("record");

    let out = s.run(&["validate", "--json"]);
    let body = stdout(&out);
    assert!(
        !body.contains("REQ-V-0024"),
        "Verified + passing latest must not trip REQ-V-0024; got: {}",
        body
    );
}

#[test]
fn req_0125_lint_flags_defects_as_quality_finding() {
    let s = Sandbox::new();
    init_with_one_failing_verified(&s);
    let out = s.run(&["lint", "--json"]);
    let v: serde_json::Value = serde_json::from_str(&stdout(&out)).expect("lint JSON");
    let defective = v["quality"]["verified_but_defective"]
        .as_array()
        .expect("lint surface");
    assert!(
        defective.iter().any(|x| x == "REQ-0001"),
        "lint should report the defect; got: {:?}",
        defective
    );
}
