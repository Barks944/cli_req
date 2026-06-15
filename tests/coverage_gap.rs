// Dedicated req_NNNN_* tests for behaviours that were implemented and genuine
// but lacked a named test, so `reverify --by-tests` could not re-anchor them.
// Writing real tests both fills the coverage gap and makes future refactors
// (e.g. the validate/verify terminology cleanup) cheaply re-anchorable.
mod common;
use common::{stderr, stdout, Sandbox};
use std::process::Command;

fn git(dir: &std::path::Path, args: &[&str]) {
    let _ = Command::new("git").current_dir(dir).args(args).output();
}

/// Run req with cwd = the sandbox (so git HEAD and "." resolve there).
fn run_in(s: &Sandbox, args: &[&str]) -> std::process::Output {
    let mut full: Vec<String> = vec!["--file".into(), s.path().to_str().unwrap().into()];
    full.extend(args.iter().map(|a| a.to_string()));
    Command::new(env!("CARGO_BIN_EXE_req"))
        .current_dir(s.dir.path())
        .args(&full)
        .env_remove("REQ_FILE")
        .output()
        .expect("invoke req")
}

/// A git-backed sandbox with one Implemented requirement; returns it + HEAD sha.
fn implemented_in_git() -> (Sandbox, String) {
    let s = Sandbox::new();
    let dir = s.dir.path();
    git(dir, &["init", "-q", "-b", "main"]);
    git(dir, &["config", "user.email", "t@example.com"]);
    git(dir, &["config", "user.name", "Tester"]);
    git(dir, &["commit", "-q", "-m", "init", "--allow-empty"]);
    s.init("p");
    let _ = run_in(&s, &[
        "add", "-t", "Halt the line cleanly", "-s", "The system shall halt the line.",
        "-r", "line safety constraint", "-k", "functional", "-a", "line halts within a second",
    ]);
    for st in ["proposed", "approved", "implemented"] {
        let _ = run_in(&s, &["update", "REQ-0001", "--status", st, "--reason", "advance"]);
    }
    let out = Command::new("git").current_dir(dir).args(["rev-parse", "HEAD"]).output().unwrap();
    let sha = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (s, sha)
}

fn add_one(s: &Sandbox) {
    let o = s.run(&[
        "add", "-t", "Stop on demand cleanly", "-s",
        "The system shall stop on operator demand.", "-r",
        "operator safety constraint", "-k", "functional", "-p", "must", "-a",
        "process halts within one second",
    ]);
    assert!(o.status.success(), "add: {}", stderr(&o));
}

/// REQ-0084: the status state machine rejects irregular transitions unless
/// --force is supplied, and the error names the alternative path.
#[test]
fn req_0084_status_machine_rejects_irregular_transitions() {
    let s = Sandbox::new();
    s.init("p");
    add_one(&s);
    // AC1: draft -> verified fails without --force; error names the path.
    let o = s.run(&["update", "REQ-0001", "--status", "verified", "--reason", "try to skip ahead"]);
    assert!(!o.status.success(), "draft->verified should fail");
    let e = stderr(&o).to_lowercase();
    assert!(
        e.contains("irregular") && e.contains("--force"),
        "error should name the override path: {}",
        stderr(&o)
    );
    // AC2: draft -> approved succeeds (carve-out).
    assert!(s
        .run(&["update", "REQ-0001", "--status", "approved", "--reason", "reviewed and approved"])
        .status
        .success());
    // AC3: active -> obsolete succeeds.
    assert!(s
        .run(&["update", "REQ-0001", "--status", "obsolete", "--reason", "superseded by another"])
        .status
        .success());
    // AC5: obsolete -> draft requires --force.
    assert!(!s
        .run(&["update", "REQ-0001", "--status", "draft", "--reason", "revive this please"])
        .status
        .success());
    assert!(s
        .run(&["update", "REQ-0001", "--status", "draft", "--reason", "revive with the force flag", "--force"])
        .status
        .success());
    // AC4: verified -> implemented requires --force (reach verified via force).
    assert!(s
        .run(&["update", "REQ-0001", "--status", "verified", "--reason", "force to verified for test", "--force"])
        .status
        .success());
    assert!(!s
        .run(&["update", "REQ-0001", "--status", "implemented", "--reason", "demote it back"])
        .status
        .success());
}

/// REQ-0090: REQ-ID lookups normalise across case and zero-pad forms, and a
/// near-miss id suggests the nearest existing one; a far miss does not.
#[test]
fn req_0090_id_resolution_normalises_and_suggests() {
    let s = Sandbox::new();
    s.init("p");
    add_one(&s);
    // AC1: normalised forms resolve.
    for form in ["req-1", "REQ-1", "req-0001", "REQ-0001"] {
        assert!(s.run(&["show", form]).status.success(), "{} should resolve", form);
    }
    // AC2: a near-miss id suggests the nearest existing id.
    let o = s.run(&["show", "REQ-0003"]);
    assert!(!o.status.success());
    assert!(
        stderr(&o).contains("did you mean REQ-0001"),
        "near-miss hint: {}",
        stderr(&o)
    );
    // AC3: a far miss returns a plain error with no suggestion.
    let o2 = s.run(&["show", "REQ-9999"]);
    assert!(!o2.status.success());
    assert!(
        stderr(&o2).contains("no such requirement") && !stderr(&o2).contains("did you mean"),
        "far-miss plain error: {}",
        stderr(&o2)
    );
}

/// REQ-0094: status values are accepted in any case and folded to canonical.
#[test]
fn req_0094_status_value_case_insensitive() {
    let s = Sandbox::new();
    s.init("p");
    add_one(&s);
    assert!(s
        .run(&["update", "REQ-0001", "--status", "Approved", "--reason", "case-insensitive approve"])
        .status
        .success());
    assert!(s
        .run(&["update", "REQ-0001", "--status", "IMPLEMENTED", "--reason", "case-insensitive implement"])
        .status
        .success());
    assert!(
        stdout(&s.run(&["show", "REQ-0001"])).to_lowercase().contains("implemented"),
        "status folds to canonical lowercase"
    );
}

/// REQ-0091: `repair --force` re-signs a hand-edited file even with validation
/// errors; without --force it refuses; afterwards conform surfaces the errors.
#[test]
fn req_0091_repair_force_resigns_despite_errors() {
    let s = Sandbox::new();
    s.init("p");
    add_one(&s);
    // Hand-edit: break integrity AND introduce a validation error.
    let path = s.path();
    let body = std::fs::read_to_string(&path)
        .unwrap()
        .replace("The system shall stop on operator demand.", "short");
    std::fs::write(&path, body).unwrap();
    // AC1: repair without --force refuses on validation errors.
    assert!(!s.run(&["repair", "--confirm-direct-edit"]).status.success());
    // AC2: repair --force re-signs and warns errors remain.
    let o = s.run(&["repair", "--confirm-direct-edit", "--force"]);
    assert!(o.status.success(), "repair --force: {}", stderr(&o));
    let out = format!("{}{}", stdout(&o), stderr(&o)).to_lowercase();
    assert!(out.contains("error") && out.contains("conform"), "warns + points to conform: {}", out);
    // AC3: the file now loads and conform reports the surviving error.
    let c = s.run(&["conform"]);
    assert!(stdout(&c).to_lowercase().contains("error") || !c.status.success());
}

/// REQ-0101: `req lint` audits quality dimensions in markdown and JSON.
#[test]
fn req_0101_lint_audits_quality_dimensions() {
    let s = Sandbox::new();
    s.init("p");
    add_one(&s);
    // lint quality findings count ACTIVE requirements; advance out of draft.
    s.run(&["update", "REQ-0001", "--status", "approved", "--reason", "make active for lint"]);
    // Scan the (markerless) sandbox dir so the source-marker finding populates,
    // rather than the surrounding repo whose source carries REQ markers.
    let dir = s.dir.path().to_str().unwrap().to_string();
    let md = stdout(&s.run(&["lint", "--path", &dir]));
    assert!(md.contains("Quality observations"), "md: {}", md);
    for needle in ["no source marker", "Rationales under", "acceptance criterion", "no test record"] {
        assert!(md.contains(needle), "lint markdown missing '{}':\n{}", needle, md);
    }
    let js = s.run(&["lint", "--path", &dir, "--json"]);
    assert!(js.status.success());
    let v: serde_json::Value = serde_json::from_str(&stdout(&js)).expect("lint json");
    assert!(v["quality"].is_object() && v["validator"].is_object(), "json shape: {}", stdout(&js));
}

/// REQ-0107: lint excludes inspection-only requirements from the
/// no-test-record finding (but other findings still apply).
#[test]
fn req_0107_lint_excludes_inspection_only_from_no_test() {
    let s = Sandbox::new();
    s.init("p");
    add_one(&s); // REQ-0001: ordinary, no test record.
    let o = s.run(&[
        "add", "-t", "Inspect only requirement here", "-s",
        "The system shall be inspected only.", "-r", "inspection rationale phrase",
        "-k", "functional", "-p", "could", "-a", "passes visual inspection",
        "--tag", "inspection-only",
    ]);
    assert!(o.status.success(), "add inspection-only: {}", stderr(&o));
    // The no-test-record finding counts ACTIVE requirements; advance both.
    s.run(&["update", "REQ-0001", "--status", "approved", "--reason", "make active for lint"]);
    s.run(&["update", "REQ-0002", "--status", "approved", "--reason", "make active for lint"]);
    let v: serde_json::Value =
        serde_json::from_str(&stdout(&s.run(&["lint", "--json"]))).unwrap();
    let ntr: Vec<String> = v["quality"]["no_test_record"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap().to_string())
        .collect();
    assert!(ntr.contains(&"REQ-0001".to_string()), "ordinary no-test req listed: {:?}", ntr);
    assert!(!ntr.contains(&"REQ-0002".to_string()), "inspection-only excluded: {:?}", ntr);
}

/// REQ-0095: re-adding a title similar to a recently-obsolete requirement warns
/// (naming the obsolete id) but does not block the add.
#[test]
fn req_0095_warns_on_readd_similar_to_recent_obsolete() {
    let s = Sandbox::new();
    s.init("p");
    assert!(s
        .run(&[
            "add", "-t", "Stop the motor on demand", "-s",
            "The system shall stop the motor on demand.", "-r",
            "operator safety constraint", "-k", "functional", "-p", "must", "-a",
            "motor halts within one second",
        ])
        .status
        .success());
    s.run(&["update", "REQ-0001", "--status", "obsolete", "--reason", "superseded by a newer approach"]);
    let o = s.run(&[
        "add", "-t", "Stop the motor on demand", "-s",
        "The system shall stop the motor immediately on demand.", "-r",
        "operator safety constraint here", "-k", "functional", "-p", "must", "-a",
        "motor halts within one second flat",
    ]);
    // AC2: the add is not blocked — a fresh Draft is created.
    assert!(o.status.success(), "add must not be blocked: {}", stderr(&o));
    let out = format!("{}{}", stdout(&o), stderr(&o));
    // AC1: the warning names the obsolete id.
    assert!(out.contains("similar") && out.contains("REQ-0001"), "warn names obsolete id:\n{}", out);
    assert!(out.contains("Added REQ-0002"), "draft still created:\n{}", out);
}

/// REQ-0118: a --dry-run invocation never writes project.req — two consecutive
/// dry-runs leave the file byte-identical and add no requirements.
#[test]
fn req_0118_dry_run_never_persists() {
    let s = Sandbox::new();
    s.init("p");
    let doc = s.dir.path().join("reqs.md");
    std::fs::write(&doc, "# Reqs\n\n## The system shall log every error to disk\n\nRationale: auditability.\n").unwrap();
    let before = std::fs::read(s.path()).unwrap();
    for _ in 0..2 {
        let _ = s.run(&["import", "--format", "markdown", "--dry-run", doc.to_str().unwrap()]);
    }
    let after = std::fs::read(s.path()).unwrap();
    assert_eq!(before, after, "dry-run must not modify project.req");
    assert!(
        !stdout(&s.run(&["list"])).to_lowercase().contains("log every error"),
        "dry-run must not persist a requirement"
    );
}

/// REQ-0180: an ingested test result is bound to the payload commit, and a
/// payload omitting the commit is rejected.
#[test]
fn req_0180_ingest_binds_commit_and_rejects_missing() {
    let (s, sha) = implemented_in_git();
    let p = s.dir.path().join("r.json");
    std::fs::write(
        &p,
        format!(
            r#"{{"schema":"req-test-result-v1","system":"at_test","commit":"{}","results":[{{"req_id":"REQ-0001","verdict":"pass","notes":"bench"}}]}}"#,
            sha
        ),
    )
    .unwrap();
    assert!(run_in(&s, &["test", "ingest", p.to_str().unwrap()]).status.success());
    let v: serde_json::Value =
        serde_json::from_str(&stdout(&run_in(&s, &["show", "REQ-0001", "--json"]))).unwrap();
    let last = v["tests"].as_array().unwrap().last().unwrap();
    assert_eq!(last["commit"].as_str().unwrap(), sha, "record bound to the payload commit");
    // AC3: a payload with no commit is rejected.
    let p2 = s.dir.path().join("nocommit.json");
    std::fs::write(
        &p2,
        r#"{"schema":"req-test-result-v1","system":"at_test","results":[{"req_id":"REQ-0001","verdict":"pass"}]}"#,
    )
    .unwrap();
    assert!(
        !run_in(&s, &["test", "ingest", p2.to_str().unwrap()]).status.success(),
        "a payload missing the commit must be rejected"
    );
}

/// REQ-0183: an external decision (plan/analysis/statement) populates the
/// dossier when its commit matches the anchor; a mismatched commit does not.
#[test]
fn req_0183_external_decision_populates_dossier() {
    let (s, sha) = implemented_in_git();
    let p = s.dir.path().join("d.json");
    std::fs::write(
        &p,
        format!(
            r#"{{"schema":"req-test-result-v1","system":"at_test","commit":"{}","results":[{{"req_id":"REQ-0001","verdict":"pass","decision":{{"plan":"bench plan here","analysis":"reviewed on bench","statement":"meets the obligation"}}}}]}}"#,
            sha
        ),
    )
    .unwrap();
    assert!(run_in(&s, &["test", "ingest", p.to_str().unwrap()]).status.success());
    let v: serde_json::Value =
        serde_json::from_str(&stdout(&run_in(&s, &["show", "REQ-0001", "--json"]))).unwrap();
    let ver = &v["verification"];
    assert_eq!(ver["plan"].as_str().unwrap(), "bench plan here");
    assert_eq!(ver["statement"].as_str().unwrap(), "meets the obligation");
    assert_eq!(ver["analysis"]["summary"].as_str().unwrap(), "reviewed on bench");
    // AC3: a decision whose commit does not match the anchor must not apply.
    let p2 = s.dir.path().join("d2.json");
    std::fs::write(
        &p2,
        r#"{"schema":"req-test-result-v1","system":"at_test","commit":"deadbeefdeadbeefdeadbeefdeadbeefdeadbeef","results":[{"req_id":"REQ-0001","verdict":"pass","decision":{"plan":"mismatched plan here"}}]}"#,
    )
    .unwrap();
    let _ = run_in(&s, &["test", "ingest", p2.to_str().unwrap()]);
    let v2: serde_json::Value =
        serde_json::from_str(&stdout(&run_in(&s, &["show", "REQ-0001", "--json"]))).unwrap();
    assert_ne!(
        v2["verification"]["plan"].as_str().unwrap(),
        "mismatched plan here",
        "a decision with a non-matching commit must not overwrite the dossier"
    );
}
