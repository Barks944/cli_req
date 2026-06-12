// Storage tests: integrity hash detects edits, repair recovers, atomic
// write semantics. Each test is named req_NNNN_description.
mod common;
use common::{stderr, stdout, Sandbox};
use std::fs;

#[test]
fn req_0002_init_writes_diffable_json() {
    let s = Sandbox::new();
    s.init("test-proj");
    let text = fs::read_to_string(s.path()).unwrap();
    assert!(
        text.starts_with('{'),
        "file should start with {{: {}",
        &text[..40]
    );
    assert!(text.contains("_format"), "should have _format field");
    assert!(text.contains("\"req-v3\""), "should declare format tag");
    assert!(text.contains("_integrity"), "should carry integrity hash");
}

#[test]
fn req_0003_integrity_blocks_load_after_semantic_tamper() {
    let s = Sandbox::new();
    s.init("p");
    s.run(&[
        "add",
        "--title",
        "Reasonable title",
        "--statement",
        "The system shall behave reasonably under all conditions tested.",
        "--rationale",
        "Baseline.",
        "--kind",
        "constraint",
        "--priority",
        "must",
    ]);
    // Tamper: change priority "Must" -> "Should"
    let text = fs::read_to_string(s.path()).unwrap();
    let tampered = text.replacen("\"Must\"", "\"Should\"", 1);
    assert_ne!(text, tampered, "tamper should change file content");
    fs::write(s.path(), tampered).unwrap();
    let out = s.run(&["list"]);
    assert!(!out.status.success(), "list should refuse after tamper");
    assert!(
        stderr(&out).contains("integrity"),
        "stderr: {}",
        stderr(&out)
    );
}

#[test]
fn req_0003_integrity_ignores_whitespace_only_change() {
    let s = Sandbox::new();
    s.init("p");
    let text = fs::read_to_string(s.path()).unwrap();
    // Insert a benign extra space inside a key (whitespace-only JSON edit).
    let with_space = text.replace("\"name\":", "\"name\" :");
    fs::write(s.path(), with_space).unwrap();
    let out = s.run(&["list"]);
    assert!(
        out.status.success(),
        "whitespace edit should not break integrity: {}",
        stderr(&out)
    );
}

#[test]
fn req_0005_repair_refuses_without_flag() {
    let s = Sandbox::new();
    s.init("p");
    let out = s.run(&["repair"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("confirm-direct-edit"));
}

#[test]
fn req_0005_repair_recovers_after_tamper() {
    let s = Sandbox::new();
    s.init("p");
    s.run(&[
        "add",
        "--title",
        "Some workable title",
        "--statement",
        "The system shall persist data across restarts of the host process.",
        "--rationale",
        "Durability.",
        "--kind",
        "constraint",
        "--priority",
        "must",
    ]);
    let text = fs::read_to_string(s.path()).unwrap();
    fs::write(s.path(), text.replacen("\"Must\"", "\"Should\"", 1)).unwrap();
    // Pre-condition: list refuses
    assert!(!s.run(&["list"]).status.success());
    // Repair with the consent flag
    let r = s.run(&["repair", "--confirm-direct-edit"]);
    assert!(r.status.success(), "repair stderr: {}", stderr(&r));
    // Post-condition: list works
    assert!(s.run(&["list"]).status.success());
}

#[test]
fn req_0019_save_is_atomic_no_partial_tmp_left_behind() {
    let s = Sandbox::new();
    s.init("p");
    s.run(&[
        "add",
        "--title",
        "Atomic write evidence",
        "--statement",
        "The save path shall produce no partial files visible to readers.",
        "--rationale",
        "Crash safety.",
        "--kind",
        "constraint",
        "--priority",
        "must",
    ]);
    let entries: Vec<_> = fs::read_dir(s.dir.path())
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().into_owned()))
        .collect();
    assert!(
        entries.iter().any(|n| n == "project.req"),
        "project.req present"
    );
    assert!(
        !entries.iter().any(|n| n.ends_with(".tmp")),
        "no .tmp lingering: {:?}",
        entries
    );
}

#[test]
fn req_0010_sequential_ids_no_reuse_after_delete() {
    let s = Sandbox::new();
    s.init("p");
    for i in 1..=3 {
        let title = format!("Requirement number {}", i);
        let out = s.run(&[
            "add",
            "--title",
            &title,
            "--statement",
            "The system shall do an interesting thing for this test.",
            "--rationale",
            "Sequence.",
            "--kind",
            "constraint",
            "--priority",
            "could",
        ]);
        assert!(out.status.success());
    }
    let _ = s.run(&["delete", "REQ-0002", "--hard", "--reason", "drop middle"]);
    let out = s.run(&[
        "add",
        "--title",
        "Followup after hard delete",
        "--statement",
        "The system shall continue assigning fresh identifiers without reuse.",
        "--rationale",
        "No reuse.",
        "--kind",
        "constraint",
        "--priority",
        "could",
    ]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let list = stdout(&s.run(&["list", "--json"]));
    assert!(
        list.contains("REQ-0004"),
        "next ID should be REQ-0004, got: {}",
        list
    );
}
