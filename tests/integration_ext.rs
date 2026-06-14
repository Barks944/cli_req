// REQ-0175..0184: external test-system integration (export / ingest / pull).
mod common;
use common::{stderr, stdout, Sandbox};
use std::process::Command;

fn git(dir: &std::path::Path, args: &[&str]) {
    let _ = Command::new("git").current_dir(dir).args(args).output();
}

/// A sandbox in a git repo with one Implemented requirement, returning the
/// sandbox and the HEAD commit sha.
fn implemented_in_git() -> (Sandbox, String) {
    let s = Sandbox::new();
    let dir = s.dir.path();
    git(dir, &["init", "-q", "-b", "main"]);
    git(dir, &["config", "user.email", "t@example.com"]);
    git(dir, &["config", "user.name", "Tester"]);
    git(dir, &["commit", "-q", "-m", "init", "--allow-empty"]);
    s.init("p");
    let _ = s.run(&[
        "add",
        "-t",
        "Stop on demand here",
        "-s",
        "The system shall stop on demand.",
        "-r",
        "operator safety",
        "-k",
        "functional",
        "-a",
        "process halts within one second",
    ]);
    for st in ["proposed", "approved", "implemented"] {
        let _ = s.run(&["update", "REQ-0001", "--status", st, "--reason", "step"]);
    }
    let out = Command::new("git")
        .current_dir(dir)
        .args(["rev-parse", "HEAD"])
        .output()
        .unwrap();
    let sha = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (s, sha)
}

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

#[test]
fn req_0175_export_requests_lists_due_requirements() {
    let (s, _sha) = implemented_in_git();
    let out = run_in(&s, &["test", "requests"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let v: serde_json::Value = serde_json::from_str(&stdout(&out)).expect("request json");
    assert_eq!(v["schema"], "req-test-request-v1");
    assert!(!v["commit"].as_str().unwrap().is_empty());
    let reqs = v["requirements"].as_array().unwrap();
    assert!(
        reqs.iter().any(|r| r["id"] == "REQ-0001"),
        "REQ-0001 due: {}",
        v
    );
}

#[test]
fn req_0177_180_183_ingest_attaches_records_and_is_idempotent() {
    let (s, sha) = implemented_in_git();
    let payload = format!(
        r#"{{"schema":"req-test-result-v1","system":"at_test","environment":"bench-944","commit":"{}","results":[{{"req_id":"REQ-0001","verdict":"pass","notes":"bench pass","decision":{{"plan":"bench plan","analysis":"reviewed","statement":"meets the obligation"}}}}]}}"#,
        sha
    );
    let p = s.dir.path().join("res.json");
    std::fs::write(&p, payload).unwrap();
    // First ingest attaches one record + dossier and (with --promote) verifies.
    let first = run_in(&s, &["test", "ingest", p.to_str().unwrap(), "--promote"]);
    assert!(first.status.success(), "{}", stderr(&first));
    assert!(stdout(&first).contains("Ingested 1"), "{}", stdout(&first));
    assert!(stdout(&first).contains("promoted to Verified: REQ-0001"));
    // REQ-0180: the record is bound to the payload commit.
    let show = run_in(&s, &["validation", "show", "REQ-0001", "--json"]);
    let dv: serde_json::Value = serde_json::from_str(&stdout(&show)).unwrap();
    assert_eq!(
        dv["validation"]["concluded_commit"], sha,
        "commit-bound dossier"
    );
    // REQ-0177: re-ingesting the same payload creates no duplicate.
    let second = run_in(&s, &["test", "ingest", p.to_str().unwrap(), "--promote"]);
    assert!(
        stdout(&second).contains("Ingested 0"),
        "idempotent: {}",
        stdout(&second)
    );
    assert!(stdout(&second).contains("1 duplicate"));
}

#[test]
fn req_0178_182_ingest_rejects_bad_payloads() {
    let (s, sha) = implemented_in_git();
    // Unsupported schema version.
    let bad_schema = s.dir.path().join("schema.json");
    std::fs::write(
        &bad_schema,
        r#"{"schema":"v999","system":"x","commit":"c","results":[]}"#,
    )
    .unwrap();
    let o1 = run_in(&s, &["test", "ingest", bad_schema.to_str().unwrap()]);
    assert!(!o1.status.success());
    assert!(stderr(&o1).contains("unsupported result schema"));
    // Unmapped verdict — must reject, never default to pass.
    let bad_verdict = s.dir.path().join("verdict.json");
    std::fs::write(&bad_verdict, format!(r#"{{"schema":"req-test-result-v1","system":"x","commit":"{}","results":[{{"req_id":"REQ-0001","verdict":"weird"}}]}}"#, sha)).unwrap();
    let o2 = run_in(&s, &["test", "ingest", bad_verdict.to_str().unwrap()]);
    assert!(!o2.status.success());
    assert!(stderr(&o2).contains("no mapping"), "{}", stderr(&o2));
    // Unknown requirement id.
    let bad_id = s.dir.path().join("id.json");
    std::fs::write(&bad_id, format!(r#"{{"schema":"req-test-result-v1","system":"x","commit":"{}","results":[{{"req_id":"REQ-9999","verdict":"pass"}}]}}"#, sha)).unwrap();
    let o3 = run_in(&s, &["test", "ingest", bad_id.to_str().unwrap()]);
    assert!(!o3.status.success());
    assert!(stderr(&o3).contains("unknown requirement"));
    // None of the failed ingests changed status.
    assert!(stdout(&run_in(&s, &["show", "REQ-0001"])).contains("implemented"));
}

#[test]
fn req_0184_safety_requirement_not_auto_promoted() {
    let (s, sha) = implemented_in_git();
    common::enable_safety(&s.path());
    let _ = s.run(&[
        "hazard", "add", "-t", "H", "--harm", "hurt", "-C", "C_C", "-F", "F_B", "-P", "P_B", "-W",
        "W3",
    ]);
    let _ = s.run(&["sf", "add", "-t", "F", "--mitigates", "HAZ-0001"]);
    let _ = s.run(&[
        "sreq",
        "add",
        "-t",
        "Halt",
        "-s",
        "The system shall halt the line.",
        "-r",
        "bounds exposure",
        "-a",
        "halts",
        "--realizes",
        "SF-0001",
    ]);
    let _ = s.run(&[
        "sreq",
        "update",
        "SR-0001",
        "--status",
        "implemented",
        "--force",
        "--reason",
        "set up for ingest test",
    ]);
    let payload = format!(
        r#"{{"schema":"req-test-result-v1","system":"at_test","commit":"{}","results":[{{"req_id":"SR-0001","verdict":"pass","notes":"bench"}}]}}"#,
        sha
    );
    let p = s.dir.path().join("sr.json");
    std::fs::write(&p, payload).unwrap();
    let out = run_in(&s, &["test", "ingest", p.to_str().unwrap(), "--promote"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(
        stdout(&out).contains("NOT promoted on external evidence"),
        "{}",
        stdout(&out)
    );
    // The SR has the evidence but stays Implemented.
    assert!(stdout(&run_in(&s, &["sreq", "show", "SR-0001"])).contains("implemented"));
}

#[test]
fn req_0176_schemas_are_exposed_and_versioned() {
    let s = Sandbox::new();
    s.init("p");
    let req = run_in(&s, &["schema", "test-request"]);
    assert!(stdout(&req).contains("req-test-request-v1"));
    let res = run_in(&s, &["schema", "test-result"]);
    assert!(stdout(&res).contains("req-test-result-v1"));
}

#[test]
fn req_0179_test_list_shows_provenance() {
    let (s, sha) = implemented_in_git();
    let payload = format!(
        r#"{{"schema":"req-test-result-v1","system":"at_test","environment":"bench-944","commit":"{}","results":[{{"req_id":"REQ-0001","verdict":"pass","notes":"bench"}}]}}"#,
        sha
    );
    let p = s.dir.path().join("res.json");
    std::fs::write(&p, payload).unwrap();
    let _ = run_in(&s, &["test", "ingest", p.to_str().unwrap()]);
    let list = run_in(&s, &["test", "list", "REQ-0001"]);
    assert!(
        stdout(&list).contains("ext:at_test/bench-944"),
        "provenance shown: {}",
        stdout(&list)
    );
}

#[test]
fn req_0181_pull_network_failure_leaves_spec_unchanged() {
    let (s, _sha) = implemented_in_git();
    // An unreachable endpoint must fail without mutating project.req.
    let before = std::fs::read_to_string(s.path()).unwrap();
    let out = run_in(
        &s,
        &["test", "pull", "http://127.0.0.1:1/results", "--token", "x"],
    );
    assert!(!out.status.success(), "unreachable pull should fail");
    assert!(stderr(&out).contains("failed"), "{}", stderr(&out));
    let after = std::fs::read_to_string(s.path()).unwrap();
    assert_eq!(
        before, after,
        "project.req must be unchanged after a failed pull"
    );
}
