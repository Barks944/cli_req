// REQ-0204: the recursive adequacy walk-through. A safety function concludes its
// dossier only when every realizing SR is covered by a note AND Verified; a
// hazard concludes its adequacy dossier only when every mitigating SF is covered
// AND Verified. The hard chain gate forces sign-off to proceed bottom-up.
mod common;
use common::{stderr, stdout, Sandbox};
use std::process::Command;

fn run_as(s: &Sandbox, kind: &str, args: &[&str]) -> std::process::Output {
    let path = s.path();
    let mut full: Vec<String> = vec!["--file".into(), path.to_str().unwrap().into()];
    full.extend(args.iter().map(|a| a.to_string()));
    Command::new(env!("CARGO_BIN_EXE_req"))
        .args(&full)
        .env_remove("REQ_FILE")
        .env("REQ_ACTOR", if kind == "agent" { "bot" } else { "alice" })
        .env("REQ_ACTOR_KIND", kind)
        .output()
        .expect("invoke req")
}

/// A hazard (low SIL, so no strong-evidence gate) ← SF ← SR chain.
fn seed(s: &Sandbox) {
    s.init("p");
    s.enable_safety();
    assert!(s
        .run(&[
            "hazard", "add", "-t", "Runaway motion hazard", "--harm", "an operator is crushed",
            "-C", "C_A", "-F", "F_A", "-P", "P_A", "-W", "W1",
        ])
        .status
        .success());
    assert!(s
        .run(&[
            "sf", "add", "-t", "Emergency stop function", "--safe-state", "motion halted",
            "--mitigates", "HAZ-0001",
        ])
        .status
        .success());
    assert!(s
        .run(&[
            "sreq", "add", "-t", "Stop on demand", "-s",
            "The system shall halt all motion within 200 milliseconds of a demand.", "-r",
            "runaway motion injures the operator", "-a", "halts within 200ms", "--realizes",
            "SF-0001",
        ])
        .status
        .success());
}

/// Drive SR-0001 all the way to Verified (agent dossier + human co-sign).
fn verify_sr(s: &Sandbox) {
    assert!(s
        .run(&["sreq", "update", "SR-0001", "--status", "implemented", "--force", "--reason", "implemented for the test"])
        .status
        .success());
    assert!(s.run(&["verification", "plan", "SR-0001", "--plan", "verify the stop"]).status.success());
    assert!(s.run(&["verification", "analysis", "SR-0001", "--findings", "code review ok", "--result", "pass"]).status.success());
    assert!(s.run(&["verification", "test", "SR-0001", "--findings", "tests pass", "--result", "pass"]).status.success());
    assert!(s.run(&["verification", "conclude", "SR-0001", "--statement", "stop verified", "--promote"]).status.success());
    assert!(run_as(s, "human", &["verification", "confirm", "SR-0001"]).status.success());
}

fn open_sf_dossier(s: &Sandbox) {
    assert!(s.run(&["verification", "plan", "SF-0001", "--plan", "verify the function achieves its safe state"]).status.success());
    assert!(s.run(&["verification", "analysis", "SF-0001", "--findings", "review ok", "--result", "pass"]).status.success());
    assert!(s.run(&["verification", "test", "SF-0001", "--findings", "tests ok", "--result", "pass"]).status.success());
}

// REQ-0204: an SF dossier cannot conclude while a realizing SR is not Verified.
#[test]
fn req_0204_sf_conclude_blocked_until_realizing_sr_verified() {
    let s = Sandbox::new();
    seed(&s);
    open_sf_dossier(&s);
    assert!(s
        .run(&["verification", "cover", "SF-0001", "--child", "SR-0001", "--note", "SR-0001 implements the stop"])
        .status
        .success());
    // SR-0001 is still Draft → conclude is hard-blocked.
    let blocked = s.run(&["verification", "conclude", "SF-0001", "--statement", "achieves safe state", "--promote"]);
    assert!(!blocked.status.success(), "SF conclude must be blocked while its SR is unverified");
    assert!(
        stderr(&blocked).contains("not Verified") && stderr(&blocked).contains("SR-0001"),
        "block message should name the unverified SR: {}",
        stderr(&blocked)
    );

    // Verify SR-0001, and the same conclude now succeeds.
    verify_sr(&s);
    let ok = s.run(&["verification", "conclude", "SF-0001", "--statement", "achieves safe state via verified SR-0001", "--promote"]);
    assert!(ok.status.success(), "SF conclude should pass once the SR is Verified: {}", stderr(&ok));
}

// REQ-0204: an SF dossier cannot conclude while a realizing SR is uncovered.
#[test]
fn req_0204_sf_conclude_blocked_until_sr_covered() {
    let s = Sandbox::new();
    seed(&s);
    verify_sr(&s);
    open_sf_dossier(&s);
    // No cover note recorded → blocked even though the SR is Verified.
    let blocked = s.run(&["verification", "conclude", "SF-0001", "--statement", "x", "--promote"]);
    assert!(!blocked.status.success());
    assert!(stderr(&blocked).contains("no walk-through note"), "{}", stderr(&blocked));
}

// REQ-0204: cover rejects a requirement that does not realize the safety function.
#[test]
fn req_0204_cover_rejects_non_realizing_sr() {
    let s = Sandbox::new();
    seed(&s);
    // A second SR that does NOT realize SF-0001.
    assert!(s
        .run(&["sreq", "add", "-t", "Unrelated requirement", "-s", "The system shall log an event for audit.", "-r", "auditability", "-a", "logged"])
        .status
        .success());
    open_sf_dossier(&s);
    let bad = s.run(&["verification", "cover", "SF-0001", "--child", "SR-0002", "--note", "nope"]);
    assert!(!bad.status.success());
    assert!(stderr(&bad).contains("does not live-realize"), "{}", stderr(&bad));
}

// REQ-0204: a hazard adequacy dossier cannot conclude until every mitigating SF
// is covered AND Verified; the full bottom-up chain then conforms with no
// adequacy-chain errors.
#[test]
fn req_0204_hazard_adequacy_walks_and_gates_the_chain() {
    let s = Sandbox::new();
    seed(&s);
    verify_sr(&s);
    open_sf_dossier(&s);
    assert!(s.run(&["verification", "cover", "SF-0001", "--child", "SR-0001", "--note", "implements stop"]).status.success());
    assert!(s.run(&["verification", "conclude", "SF-0001", "--statement", "achieves safe state", "--promote"]).status.success());

    // Open the hazard adequacy dossier; conclude before covering SF-0001 is blocked.
    assert!(s.run(&["hazard", "adequacy", "plan", "HAZ-0001", "--plan", "argue residual risk"]).status.success());
    let uncovered = s.run(&["hazard", "adequacy", "conclude", "HAZ-0001", "--statement", "residual ok"]);
    assert!(!uncovered.status.success());
    assert!(stderr(&uncovered).contains("no walk-through note"), "{}", stderr(&uncovered));

    // Cover SF-0001, but it is only Implemented (awaiting co-sign) → still blocked on "not Verified".
    assert!(s.run(&["hazard", "adequacy", "cover", "HAZ-0001", "--sf", "SF-0001", "--note", "estop covers runaway"]).status.success());
    let unverified = s.run(&["hazard", "adequacy", "conclude", "HAZ-0001", "--statement", "residual ok"]);
    assert!(!unverified.status.success());
    assert!(stderr(&unverified).contains("not Verified"), "{}", stderr(&unverified));

    // Co-sign SF-0001 → Verified; now the hazard adequacy concludes and co-signs.
    assert!(run_as(&s, "human", &["verification", "confirm", "SF-0001"]).status.success());
    assert!(s.run(&["hazard", "adequacy", "conclude", "HAZ-0001", "--statement", "residual risk acceptable"]).status.success());
    assert!(run_as(&s, "human", &["hazard", "confirm", "HAZ-0001"]).status.success());

    // The fully co-signed bottom-up chain has no adequacy-chain conformance errors.
    let conf = stdout(&s.run(&["conform"]));
    assert!(
        !conf.contains("REQ-V-0044") && !conf.contains("REQ-V-0045"),
        "a sound bottom-up chain must not trip the adequacy-chain rules:\n{}",
        conf
    );
}
