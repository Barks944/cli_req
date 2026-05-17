// Tests for Slice B: REQ-0064 (doctor), REQ-0069 (diff), REQ-0070 (test-vs-impl).
mod common;
use common::Sandbox;
use std::fs;
use std::process::Command;

// ---------- REQ-0064: req doctor ----------

fn git(dir: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .expect("git")
}

fn fresh_git_repo() -> Sandbox {
    let s = Sandbox::new();
    s.init("doctor-test");
    let _ = git(s.dir.path(), &["init", "-q", "-b", "main"]);
    let _ = git(s.dir.path(), &["config", "user.email", "t@example.com"]);
    let _ = git(s.dir.path(), &["config", "user.name", "Tester"]);
    s
}

#[test]
fn req_0064_doctor_reports_missing_pre_commit() {
    let s = fresh_git_repo();
    let out = Command::new(env!("CARGO_BIN_EXE_req"))
        .current_dir(s.dir.path())
        .args(["--file", s.path().to_str().unwrap(), "doctor"])
        .output()
        .expect("invoke req");
    assert!(
        !out.status.success(),
        "doctor should fail when nothing is configured"
    );
    let body = String::from_utf8_lossy(&out.stdout);
    assert!(body.contains("pre-commit hook"));
    assert!(body.contains("FAIL"));
}

#[test]
fn req_0064_doctor_passes_after_hooks_install() {
    let s = fresh_git_repo();
    let install = Command::new(env!("CARGO_BIN_EXE_req"))
        .current_dir(s.dir.path())
        .args([
            "--file",
            s.path().to_str().unwrap(),
            "hooks",
            "install",
            "--force",
        ])
        .output()
        .expect("hooks install");
    assert!(
        install.status.success(),
        "hooks install: {}",
        String::from_utf8_lossy(&install.stderr)
    );
    // Activate the merge driver as documented
    let _ = git(
        s.dir.path(),
        &[
            "config",
            "merge.req-merge.driver",
            "req renumber --base %O || true",
        ],
    );
    // Then doctor — only commit-signing should remain failing (test env has none)
    let out = Command::new(env!("CARGO_BIN_EXE_req"))
        .current_dir(s.dir.path())
        .args(["--file", s.path().to_str().unwrap(), "doctor", "--json"])
        .output()
        .expect("invoke req");
    let body = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = serde_json::from_str(&body).expect("doctor --json shape");
    let checks = v["checks"].as_array().expect("checks array");
    let hook = checks
        .iter()
        .find(|c| c["name"] == "pre-commit hook")
        .unwrap();
    assert!(hook["ok"].as_bool().unwrap(), "pre-commit should be OK");
    let pin = checks
        .iter()
        .find(|c| c["name"] == "gitattributes line-ending pin")
        .unwrap();
    assert!(
        pin["ok"].as_bool().unwrap(),
        "gitattributes pin should be OK"
    );
}

// ---------- REQ-0069: req diff ----------

#[test]
fn req_0069_diff_reports_added_and_changed() {
    let s = fresh_git_repo();
    // baseline: add one req, commit
    let add1 = s.run(&[
        "add",
        "--title",
        "Baseline requirement here",
        "--statement",
        "The system shall start with this established baseline.",
        "--rationale",
        "Setup.",
        "--kind",
        "constraint",
        "--priority",
        "could",
    ]);
    assert!(add1.status.success());
    let _ = git(s.dir.path(), &["add", "project.req"]);
    let _ = git(s.dir.path(), &["commit", "-q", "-m", "baseline"]);

    // head: add a second and update the first
    let add2 = s.run(&[
        "add",
        "--title",
        "Second requirement appears now",
        "--statement",
        "The system shall now also do this additional thing.",
        "--rationale",
        "Added.",
        "--kind",
        "constraint",
        "--priority",
        "could",
    ]);
    assert!(add2.status.success());
    let upd = s.run(&[
        "update",
        "REQ-0001",
        "--status",
        "implemented",
        "--reason",
        "Done in this branch",
    ]);
    assert!(upd.status.success());
    let _ = git(s.dir.path(), &["add", "project.req"]);
    let _ = git(s.dir.path(), &["commit", "-q", "-m", "head"]);

    let out = Command::new(env!("CARGO_BIN_EXE_req"))
        .current_dir(s.dir.path())
        .args(["--file", s.path().to_str().unwrap(), "diff", "HEAD~1..HEAD"])
        .output()
        .expect("invoke req");
    let body = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        body.contains("ADDED"),
        "should report ADDED section: {}",
        body
    );
    assert!(body.contains("REQ-0002"));
    assert!(body.contains("CHANGED"));
    assert!(body.contains("REQ-0001"));
    assert!(body.contains("status:"));
}

#[test]
fn req_0069_diff_empty_when_no_changes() {
    let s = fresh_git_repo();
    s.run(&[
        "add",
        "--title",
        "Single requirement only",
        "--statement",
        "The system shall have just this requirement, nothing more.",
        "--rationale",
        "Setup.",
        "--kind",
        "constraint",
        "--priority",
        "could",
    ]);
    let _ = git(s.dir.path(), &["add", "project.req"]);
    let _ = git(s.dir.path(), &["commit", "-q", "-m", "only"]);
    let _ = git(
        s.dir.path(),
        &["commit", "-q", "--allow-empty", "-m", "empty"],
    );
    let out = Command::new(env!("CARGO_BIN_EXE_req"))
        .current_dir(s.dir.path())
        .args(["--file", s.path().to_str().unwrap(), "diff", "HEAD~1..HEAD"])
        .output()
        .expect("req diff");
    let body = String::from_utf8_lossy(&out.stdout);
    assert!(body.contains("no requirement-level changes"));
}

// ---------- REQ-0070: test-vs-impl classification ----------

#[test]
fn req_0070_test_only_marker_is_distinct_from_referenced() {
    use crate::common as common_alias;
    let _ = common_alias::req(&["--help"]);
    // Use the coverage helper directly via the production binary against a
    // sandbox tree that has one impl marker and one test-only marker.
    let s = Sandbox::new();
    s.init("p");
    // Allocate REQ-0001 and REQ-0002 (will be REQ-0001..REQ-0002).
    let _ = s.run(&[
        "add",
        "--title",
        "Impl-only requirement",
        "--statement",
        "The system shall be referenced from src only.",
        "--rationale",
        "Test fixture.",
        "--kind",
        "constraint",
        "--priority",
        "could",
    ]);
    let _ = s.run(&[
        "add",
        "--title",
        "Test-only requirement",
        "--statement",
        "The system shall be referenced from tests only.",
        "--rationale",
        "Test fixture.",
        "--kind",
        "constraint",
        "--priority",
        "could",
    ]);
    // Create the file tree
    fs::create_dir_all(s.dir.path().join("src")).unwrap();
    fs::create_dir_all(s.dir.path().join("tests")).unwrap();
    fs::write(
        s.dir.path().join("src/lib.rs"),
        "// REQ-0001 implementation site\nfn _foo() {}\n",
    )
    .unwrap();
    fs::write(
        s.dir.path().join("tests/coverage_test.rs"),
        "// REQ-0002 test-only reference\nfn _t() {}\n",
    )
    .unwrap();

    // Run coverage in --json mode and assert classification
    let out = Command::new(env!("CARGO_BIN_EXE_req"))
        .args([
            "--file",
            s.path().to_str().unwrap(),
            "coverage",
            "--path",
            s.dir.path().to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("coverage --json");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("coverage --json shape");
    assert!(
        v["referenced"]
            .as_object()
            .unwrap()
            .contains_key("REQ-0001"),
        "REQ-0001 should be referenced: {}",
        v
    );
    assert!(
        v["test_only"].as_object().unwrap().contains_key("REQ-0002"),
        "REQ-0002 should be test-only: {}",
        v
    );
    assert!(
        !v["referenced"]
            .as_object()
            .unwrap()
            .contains_key("REQ-0002"),
        "REQ-0002 must NOT count as fully-referenced"
    );
}
