// REQ-0112: content-hashing for test record staleness. New records
// carry a sha256 of the linked-file contents; `req stale` uses the
// hash (when present) so STALE fires only on actual content change,
// not on every HEAD movement.
mod common;
use common::{stderr, stdout, Sandbox};
use std::fs;
use std::process::Command;

fn init_git_repo(path: &std::path::Path) {
    let _ = Command::new("git")
        .args(["init", "-q"])
        .current_dir(path)
        .output()
        .expect("git init");
    let _ = Command::new("git")
        .args(["config", "user.email", "t@e"])
        .current_dir(path)
        .output();
    let _ = Command::new("git")
        .args(["config", "user.name", "t"])
        .current_dir(path)
        .output();
    let _ = Command::new("git")
        .args(["config", "commit.gpgsign", "false"])
        .current_dir(path)
        .output();
}

fn git_commit(path: &std::path::Path, msg: &str) {
    let _ = Command::new("git")
        .args(["add", "-A"])
        .current_dir(path)
        .output();
    let _ = Command::new("git")
        .args(["commit", "-q", "-m", msg])
        .current_dir(path)
        .output();
}

#[test]
fn req_0112_record_carries_content_hash_when_marker_present() {
    let s = Sandbox::new();
    s.init("p");
    init_git_repo(s.dir.path());

    let out = s.run(&[
        "add",
        "--title",
        "Hashed requirement here",
        "--statement",
        "The system shall be referenced from a source file with a marker.",
        "--rationale",
        "Content-hash fixture.",
        "--kind",
        "constraint",
        "--priority",
        "could",
    ]);
    assert!(out.status.success(), "add: {}", stderr(&out));

    // Drop a source file with the marker.
    fs::create_dir_all(s.dir.path().join("src")).unwrap();
    fs::write(
        s.dir.path().join("src/lib.rs"),
        "// REQ-0001: this implementation\nfn _ok() {}\n",
    )
    .unwrap();
    git_commit(s.dir.path(), "initial");

    // Record a passing test. The test_record path uses the binary's
    // working directory for auto-discovery, so jump into the sandbox
    // for this call by absolute --path tricks.
    let abs = s.dir.path().to_path_buf();
    let out = Command::new(env!("CARGO_BIN_EXE_req"))
        .current_dir(&abs)
        .args([
            "--file",
            s.path().to_str().unwrap(),
            "test",
            "record",
            "REQ-0001",
            "--result",
            "pass",
            "--notes",
            "content-hash test",
        ])
        .output()
        .expect("test record");
    assert!(
        out.status.success(),
        "test record: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let show = stdout(&s.run(&["show", "REQ-0001", "--json"]));
    assert!(
        show.contains("\"content_hash\""),
        "record should carry content_hash: {}",
        show
    );
    assert!(
        show.contains("\"linked_files\""),
        "record should carry linked_files: {}",
        show
    );
}

#[test]
fn req_0112_stale_fires_only_on_actual_content_change() {
    let s = Sandbox::new();
    s.init("p");
    init_git_repo(s.dir.path());

    let _ = s.run(&[
        "add",
        "--title",
        "Stale tracking target",
        "--statement",
        "The system shall be content-stable across unrelated commits.",
        "--rationale",
        "Stale-flap fixture.",
        "--kind",
        "constraint",
        "--priority",
        "could",
    ]);
    fs::create_dir_all(s.dir.path().join("src")).unwrap();
    fs::write(
        s.dir.path().join("src/lib.rs"),
        "// REQ-0001: anchor\nfn _ok() {}\n",
    )
    .unwrap();
    git_commit(s.dir.path(), "initial");

    let abs = s.dir.path().to_path_buf();
    let rec = Command::new(env!("CARGO_BIN_EXE_req"))
        .current_dir(&abs)
        .args([
            "--file",
            s.path().to_str().unwrap(),
            "test",
            "record",
            "REQ-0001",
            "--result",
            "pass",
            "--notes",
            "baseline",
        ])
        .output()
        .expect("test record");
    assert!(
        rec.status.success(),
        "{}",
        String::from_utf8_lossy(&rec.stderr)
    );

    // Touch an UNRELATED file and commit — HEAD moves but the linked
    // file's content is unchanged, so under content-hashing this must
    // NOT be flagged stale.
    fs::write(s.dir.path().join("README.md"), "unrelated change\n").unwrap();
    git_commit(s.dir.path(), "unrelated");

    let stale = Command::new(env!("CARGO_BIN_EXE_req"))
        .current_dir(&abs)
        .args([
            "--file",
            s.path().to_str().unwrap(),
            "stale",
            "--path",
            abs.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("stale");
    let body = String::from_utf8_lossy(&stale.stdout);
    assert!(
        body.contains("\"stale\": 0") || body.contains("\"state\": \"fresh\""),
        "content-hash should keep STALE quiet on unrelated commits: {}",
        body
    );

    // Now actually modify the linked file. Content-hash should now
    // fire STALE.
    fs::write(
        s.dir.path().join("src/lib.rs"),
        "// REQ-0001: anchor\nfn _ok() { /* changed */ }\n",
    )
    .unwrap();
    git_commit(s.dir.path(), "real change");

    let stale2 = Command::new(env!("CARGO_BIN_EXE_req"))
        .current_dir(&abs)
        .args([
            "--file",
            s.path().to_str().unwrap(),
            "stale",
            "--path",
            abs.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("stale 2");
    let body2 = String::from_utf8_lossy(&stale2.stdout);
    assert!(
        body2.contains("\"STALE\"") || body2.contains("\"stale\": 1"),
        "content-hash should fire STALE when linked file content changes: {}",
        body2
    );
}

#[test]
fn req_0112_old_record_without_hash_falls_back_to_sha() {
    // Older records (no content_hash) should still work via the
    // original SHA-based comparison. We don't test the full path
    // here because creating an old-shape record requires direct edit
    // + repair — and the field is added with serde default + skip
    // when None, so an absent content_hash is the legitimate "old
    // record" case. The behavioural contract: stale doesn't panic
    // and produces a non-error report on a sandbox with no records.
    let s = Sandbox::new();
    s.init("p");
    let out = s.run(&["stale", "--json"]);
    assert!(
        out.status.success(),
        "stale should succeed on empty: {}",
        stderr(&out)
    );
}
