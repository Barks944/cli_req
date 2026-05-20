// REQ-0109: req adopt — retroactive backfill helper.
mod common;
use common::{stderr, stdout, Sandbox};

fn add_draft_functional(s: &Sandbox, title: &str) {
    let out = s.run(&[
        "add",
        "--title",
        title,
        "--statement",
        "The system shall provide the behaviour described by this fixture.",
        "--rationale",
        "Adopt fixture.",
        "--kind",
        "functional",
        "--priority",
        "should",
        "--accept",
        "fixture acceptance for adoption test",
    ]);
    assert!(out.status.success(), "add: {}", stderr(&out));
}

fn add_draft_constraint(s: &Sandbox, title: &str) {
    let out = s.run(&[
        "add",
        "--title",
        title,
        "--statement",
        "The system shall preserve this constraint.",
        "--rationale",
        "Adopt fixture.",
        "--kind",
        "constraint",
        "--priority",
        "could",
    ]);
    assert!(out.status.success(), "add: {}", stderr(&out));
}

#[test]
fn req_0109_adopt_walks_draft_to_verified_in_one_call() {
    let s = Sandbox::new();
    s.init("p");
    add_draft_constraint(&s, "First adopt target here");
    let out = s.run(&["adopt", "REQ-0001"]);
    assert!(out.status.success(), "adopt failed: {}", stderr(&out));

    let show = s.run(&["show", "REQ-0001", "--json"]);
    let body = stdout(&show);
    assert!(
        body.contains("\"Verified\""),
        "expected Verified status, got: {}",
        body
    );
}

#[test]
fn req_0109_adopt_records_one_history_entry_per_hop() {
    let s = Sandbox::new();
    s.init("p");
    add_draft_constraint(&s, "History trail target");
    let _ = s.run(&["adopt", "REQ-0001", "--to", "implemented"]);
    let body = stdout(&s.run(&["show", "REQ-0001", "--json"]));
    // Draft → Proposed → Approved → Implemented = 3 adopt entries.
    let hops = body.matches("\"adopt → ").count();
    assert_eq!(
        hops, 3,
        "expected 3 adopt hops in history, got {}; body: {}",
        hops, body
    );
}

#[test]
fn req_0109_adopt_auto_adds_placeholder_acceptance_for_functional() {
    let s = Sandbox::new();
    s.init("p");
    // Add a functional req WITHOUT acceptance by editing the file
    // after add. Easier path: use the add command, then remove
    // acceptance via direct edit followed by repair.
    let out = s.run(&[
        "add",
        "--title",
        "Functional without acceptance",
        "--statement",
        "The system shall provide this functional behaviour to the user.",
        "--rationale",
        "Used to test placeholder acceptance injection.",
        "--kind",
        "functional",
        "--priority",
        "could",
        "--accept",
        "this will be stripped to simulate an unfilled adoption case",
    ]);
    assert!(out.status.success(), "add: {}", stderr(&out));

    // Strip acceptance via JSON edit + repair so the validator sees it
    // as empty at adopt time.
    let mut json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(s.path()).unwrap()).unwrap();
    json["requirements"]["REQ-0001"]["acceptance"] = serde_json::json!([]);
    std::fs::write(s.path(), serde_json::to_string_pretty(&json).unwrap()).unwrap();
    let r = s.run(&["repair", "--confirm-direct-edit", "--force"]);
    assert!(r.status.success(), "repair: {}", stderr(&r));

    let adopt = s.run(&["adopt", "REQ-0001", "--to", "implemented"]);
    assert!(adopt.status.success(), "adopt: {}", stderr(&adopt));

    let show = stdout(&s.run(&["show", "REQ-0001", "--json"]));
    assert!(
        show.contains("implementation in source at adoption time"),
        "placeholder acceptance should appear: {}",
        show
    );
    assert!(
        show.contains("auto-added placeholder acceptance"),
        "history should mention the auto-add: {}",
        show
    );
}

#[test]
fn req_0109_adopt_all_drafts_scopes_to_drafts_only() {
    let s = Sandbox::new();
    s.init("p");
    add_draft_constraint(&s, "Will be adopted en masse one");
    add_draft_constraint(&s, "Will be adopted en masse two");
    // Walk REQ-0002 past Draft so it's NOT in the --all-drafts scope.
    let _ = s.run(&[
        "update",
        "REQ-0002",
        "--status",
        "approved",
        "--reason",
        "pre-adopt state",
        "--force",
    ]);
    let out = s.run(&["adopt", "--all-drafts", "--to", "implemented"]);
    assert!(out.status.success(), "adopt: {}", stderr(&out));

    let l = stdout(&s.run(&["list", "--json"]));
    // REQ-0001 should be implemented (adopted via --all-drafts).
    assert!(l.contains("\"REQ-0001\""));
    // REQ-0002 should not have moved further than approved.
    let r2 = stdout(&s.run(&["show", "REQ-0002", "--json"]));
    assert!(
        r2.contains("\"Approved\""),
        "REQ-0002 should remain at approved (not in --all-drafts scope): {}",
        r2
    );
}

#[test]
fn req_0109_adopt_dry_run_does_not_modify_file() {
    let s = Sandbox::new();
    s.init("p");
    add_draft_constraint(&s, "Dry run candidate here");
    let before = std::fs::read(s.path()).unwrap();
    let out = s.run(&["adopt", "REQ-0001", "--dry-run"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(
        stdout(&out).contains("dry-run"),
        "expected dry-run banner: {}",
        stdout(&out)
    );
    let after = std::fs::read(s.path()).unwrap();
    assert_eq!(before, after, "dry-run must not change the file");
}

#[test]
fn req_0109_adopt_skips_when_already_at_target() {
    let s = Sandbox::new();
    s.init("p");
    add_draft_functional(&s, "Already verified here");
    // Walk through to Verified manually.
    for status in ["proposed", "approved", "implemented", "verified"] {
        let _ = s.run(&[
            "update",
            "REQ-0001",
            "--status",
            status,
            "--reason",
            "pre-state",
            "--force",
        ]);
    }
    let out = s.run(&["adopt", "REQ-0001", "--to", "verified"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(
        stdout(&out).contains("already at or beyond target"),
        "expected skip banner: {}",
        stdout(&out)
    );
}

#[test]
fn req_0109_adopt_records_inspection_evidence_when_target_is_verified() {
    let s = Sandbox::new();
    s.init("p");
    add_draft_constraint(&s, "Inspection evidence target");
    let _ = s.run(&["adopt", "REQ-0001", "--to", "verified"]);
    let body = stdout(&s.run(&["show", "REQ-0001", "--json"]));
    assert!(
        body.contains("\"Inspection\""),
        "expected inspection evidence record: {}",
        body
    );
    assert!(
        body.contains("Verified by adoption"),
        "expected adoption note in test record: {}",
        body
    );
}
