// REQ-0139: the staged validation dossier (plan → analysis → testing →
// statement → verdict) and the promotion gate it drives. Each test maps to
// the requirement it covers.
mod common;
use common::{stderr, stdout, Sandbox};

/// Add one ordinary functional requirement and walk it to Implemented.
fn implemented_req(s: &Sandbox) {
    s.init("p");
    s.run(&[
        "add",
        "--title",
        "Stop on demand requirement",
        "--statement",
        "The system shall stop the process on operator demand.",
        "--rationale",
        "Operator safety.",
        "--kind",
        "functional",
        "--accept",
        "process halts within one second",
    ]);
    for st in ["proposed", "approved", "implemented"] {
        let r = s.run(&["update", "REQ-0001", "--status", st, "--reason", "step"]);
        assert!(r.status.success(), "step to {}: {}", st, stderr(&r));
    }
}

/// REQ-0139: the full dossier walks plan → analysis → testing → conclude
/// and `--promote` flips status to Verified; the project then validates
/// cleanly (no REQ-V-0032).
#[test]
fn req_0139_full_dossier_promotes_a_requirement() {
    let s = Sandbox::new();
    implemented_req(&s);
    assert!(s
        .run(&[
            "validation",
            "plan",
            "REQ-0001",
            "--plan",
            "review + run unit test"
        ])
        .status
        .success());
    assert!(s
        .run(&[
            "validation",
            "analysis",
            "REQ-0001",
            "--findings",
            "logic matches the obligation",
            "--result",
            "pass",
        ])
        .status
        .success());
    assert!(s
        .run(&[
            "validation",
            "test",
            "REQ-0001",
            "--findings",
            "suite green",
            "--result",
            "pass",
        ])
        .status
        .success());
    let done = s.run(&[
        "validation",
        "conclude",
        "REQ-0001",
        "--statement",
        "meets the obligation",
        "--promote",
    ]);
    assert!(done.status.success(), "conclude: {}", stderr(&done));
    assert!(stdout(&s.run(&["show", "REQ-0001"])).contains("verified"));
    assert!(
        s.run(&["validate"]).status.success(),
        "should validate clean"
    );
}

/// REQ-0139: a requirement cannot be promoted to Verified via `req verify
/// --promote` without a passing dossier.
#[test]
fn req_0139_verify_promote_blocked_without_dossier() {
    let s = Sandbox::new();
    implemented_req(&s);
    let out = s.run(&[
        "verify",
        "REQ-0001",
        "--by",
        "inspection",
        "--notes",
        "looked",
        "--promote",
    ]);
    assert!(!out.status.success(), "promote must be blocked");
    assert!(
        stderr(&out).contains("validation dossier"),
        "stderr: {}",
        stderr(&out)
    );
}

/// REQ-0139: the `validation-exempt` tag lets an ordinary requirement reach
/// Verified without a dossier, and the validator stays quiet.
#[test]
fn req_0139_exempt_tag_bypasses_the_gate() {
    let s = Sandbox::new();
    s.init("p");
    s.run(&[
        "add",
        "--title",
        "Trivial config default",
        "--statement",
        "The system shall default the timeout to thirty seconds.",
        "--rationale",
        "Sane default.",
        "--kind",
        "functional",
        "--accept",
        "default is thirty seconds",
        "--tag",
        "validation-exempt",
    ]);
    for st in ["proposed", "approved", "implemented"] {
        s.run(&["update", "REQ-0001", "--status", st, "--reason", "step"]);
    }
    let out = s.run(&[
        "verify",
        "REQ-0001",
        "--by",
        "inspection",
        "--notes",
        "ok",
        "--promote",
    ]);
    assert!(out.status.success(), "exempt promote: {}", stderr(&out));
    assert!(s.run(&["validate"]).status.success());
}

/// REQ-0139: `req verify --no-dossier --reason` records an audited
/// exemption and promotes; --no-dossier without a reason is rejected.
#[test]
fn req_0139_no_dossier_override_records_audited_exemption() {
    let s = Sandbox::new();
    implemented_req(&s);
    // Missing reason → clap rejects.
    let no_reason = s.run(&[
        "verify",
        "REQ-0001",
        "--by",
        "inspection",
        "--notes",
        "x",
        "--promote",
        "--no-dossier",
    ]);
    assert!(!no_reason.status.success(), "--no-dossier needs --reason");
    // With a reason → promotes and validates clean.
    let ok = s.run(&[
        "verify",
        "REQ-0001",
        "--by",
        "inspection",
        "--notes",
        "x",
        "--promote",
        "--no-dossier",
        "--reason",
        "trivial; covered by integration suite",
    ]);
    assert!(ok.status.success(), "override: {}", stderr(&ok));
    assert!(s.run(&["validate"]).status.success());
    assert!(stdout(&s.run(&["validation", "show", "REQ-0001"])).contains("exemption"));
}

/// REQ-0139: stage-order is enforced — analysis before plan, testing before
/// analysis, and conclude before testing all fail.
#[test]
fn req_0139_stage_order_is_enforced() {
    let s = Sandbox::new();
    implemented_req(&s);
    // analysis before plan
    assert!(!s
        .run(&[
            "validation",
            "analysis",
            "REQ-0001",
            "--findings",
            "x",
            "--result",
            "pass"
        ])
        .status
        .success());
    s.run(&["validation", "plan", "REQ-0001", "--plan", "p"]);
    // testing before analysis
    assert!(!s
        .run(&[
            "validation",
            "test",
            "REQ-0001",
            "--findings",
            "x",
            "--result",
            "pass"
        ])
        .status
        .success());
    // conclude before testing
    s.run(&[
        "validation",
        "analysis",
        "REQ-0001",
        "--findings",
        "a",
        "--result",
        "pass",
    ]);
    assert!(!s
        .run(&[
            "validation",
            "conclude",
            "REQ-0001",
            "--statement",
            "s",
            "--promote"
        ])
        .status
        .success());
}

/// REQ-0139: a failing stage yields a FAIL verdict, and a FAIL verdict
/// cannot be promoted to Verified.
#[test]
fn req_0139_fail_verdict_blocks_promotion() {
    let s = Sandbox::new();
    implemented_req(&s);
    s.run(&["validation", "plan", "REQ-0001", "--plan", "p"]);
    s.run(&[
        "validation",
        "analysis",
        "REQ-0001",
        "--findings",
        "ok",
        "--result",
        "pass",
    ]);
    s.run(&[
        "validation",
        "test",
        "REQ-0001",
        "--findings",
        "a test fails",
        "--result",
        "fail",
    ]);
    let out = s.run(&[
        "validation",
        "conclude",
        "REQ-0001",
        "--statement",
        "s",
        "--promote",
    ]);
    assert!(!out.status.success(), "FAIL verdict must not promote");
    assert!(stderr(&out).to_lowercase().contains("fail"));
    // Concluding WITHOUT promote is allowed and records the FAIL verdict.
    let recorded = s.run(&["validation", "conclude", "REQ-0001", "--statement", "s"]);
    assert!(recorded.status.success(), "{}", stderr(&recorded));
    assert!(stdout(&s.run(&["validation", "show", "REQ-0001"])).contains("FAIL"));
}

/// REQ-V-0032: a Verified requirement with no passing dossier is a hard
/// validation error.
#[test]
fn req_0139_validator_flags_verified_without_dossier() {
    let s = Sandbox::new();
    s.init("p");
    s.run(&[
        "add",
        "--title",
        "Some verified behaviour",
        "--statement",
        "The system shall expose the baseline behaviour reliably.",
        "--rationale",
        "Fixture.",
        "--kind",
        "constraint",
        "--priority",
        "could",
    ]);
    // Force straight to Verified, bypassing the verify gate at mutation time.
    for st in ["proposed", "approved", "implemented", "verified"] {
        s.run(&[
            "update", "REQ-0001", "--status", st, "--reason", "f", "--force",
        ]);
    }
    let out = s.run(&["validate", "--json"]);
    assert!(!out.status.success(), "validate should fail");
    assert!(
        stdout(&out).contains("REQ-V-0032"),
        "expected REQ-V-0032; got {}",
        stdout(&out)
    );
    // backfill clears it.
    let bf = s.run(&[
        "validation",
        "backfill",
        "--all",
        "--reason",
        "grandfathered",
    ]);
    assert!(bf.status.success(), "backfill: {}", stderr(&bf));
    assert!(
        s.run(&["validate"]).status.success(),
        "validate clean after backfill"
    );
}

/// REQ-0139: re-opening a concluded dossier clears the verdict so the item
/// can be re-validated (e.g. after code changed).
#[test]
fn req_0139_reopen_clears_the_verdict() {
    let s = Sandbox::new();
    implemented_req(&s);
    s.run(&["validation", "plan", "REQ-0001", "--plan", "p"]);
    s.run(&[
        "validation",
        "analysis",
        "REQ-0001",
        "--findings",
        "a",
        "--result",
        "pass",
    ]);
    s.run(&[
        "validation",
        "test",
        "REQ-0001",
        "--findings",
        "t",
        "--result",
        "pass",
    ]);
    s.run(&["validation", "conclude", "REQ-0001", "--statement", "s"]);
    // A second plan without --reopen is rejected.
    assert!(!s
        .run(&["validation", "plan", "REQ-0001", "--plan", "p2"])
        .status
        .success());
    // With --reopen + reason it succeeds and the verdict is cleared.
    let re = s.run(&[
        "validation",
        "plan",
        "REQ-0001",
        "--plan",
        "p2",
        "--reopen",
        "--reason",
        "code changed",
    ]);
    assert!(re.status.success(), "{}", stderr(&re));
    assert!(stdout(&s.run(&["validation", "show", "REQ-0001"])).contains("not concluded"));
}

// ---------------------------------------------------------------------------
// safety requirements
// ---------------------------------------------------------------------------

/// Build a SIL-bearing SR realized through a hazard/SF and walk it to
/// Implemented. Returns nothing; the SR is SR-0001.
fn implemented_sr(s: &Sandbox, sil_high: bool) {
    s.init("p");
    s.enable_safety();
    // C_C/F_B/P_B/W3 → SIL3 (high); C_B/F_A/P_A/W1 → no/low SIL.
    let (c, f, p, w) = if sil_high {
        ("C_C", "F_B", "P_B", "W3")
    } else {
        ("C_B", "F_B", "P_B", "W1")
    };
    s.run(&[
        "hazard", "add", "-t", "H", "--harm", "hurt", "-C", c, "-F", f, "-P", p, "-W", w,
    ]);
    s.run(&["sf", "add", "-t", "F", "--mitigates", "HAZ-0001"]);
    s.run(&[
        "sreq",
        "add",
        "-t",
        "Stop the blade",
        "-s",
        "The system shall stop the blade on demand.",
        "-r",
        "Operator safety.",
        "-a",
        "stops within 200ms",
        "--realizes",
        "SF-0001",
    ]);
    for st in ["approved", "implemented"] {
        s.run(&[
            "sreq", "update", "SR-0001", "--status", st, "--reason", "step",
        ]);
    }
}

/// REQ-0139: a safety requirement cannot be promoted via `sreq verify
/// --promote` without a passing dossier, and there is NO tag exemption.
#[test]
fn req_0139_safety_requirement_requires_dossier_no_exemption() {
    let s = Sandbox::new();
    implemented_sr(&s, false);
    let out = s.run(&[
        "sreq",
        "verify",
        "SR-0001",
        "--by",
        "automated",
        "--promote",
    ]);
    assert!(
        !out.status.success(),
        "SR promote must be blocked without dossier"
    );
    assert!(
        stderr(&out).contains("validation dossier"),
        "stderr: {}",
        stderr(&out)
    );
}

/// REQ-0139: a full dossier promotes a (low-SIL) safety requirement, and
/// the safety case validates with no REQ-V-0033.
#[test]
fn req_0139_full_dossier_promotes_a_safety_requirement() {
    let s = Sandbox::new();
    implemented_sr(&s, false);
    s.run(&["validation", "plan", "SR-0001", "--plan", "review + bench"]);
    s.run(&[
        "validation",
        "analysis",
        "SR-0001",
        "--findings",
        "logic ok",
        "--result",
        "pass",
    ]);
    s.run(&[
        "validation",
        "test",
        "SR-0001",
        "--findings",
        "bench ok",
        "--result",
        "pass",
    ]);
    let done = s.run(&[
        "validation",
        "conclude",
        "SR-0001",
        "--statement",
        "stop obligation met",
        "--promote",
    ]);
    assert!(done.status.success(), "conclude: {}", stderr(&done));
    assert!(stdout(&s.run(&["sreq", "show", "SR-0001"]))
        .to_lowercase()
        .contains("verified"));
    assert!(
        s.run(&["validate"]).status.success(),
        "safety case validates clean"
    );
}

/// REQ-0139: the SIL-rigour gate still bites under `validation conclude
/// --promote` — a SIL3 SR with only analysis/inspection evidence cannot be
/// promoted without --force.
#[test]
fn req_0139_conclude_promote_respects_sil_gate() {
    let s = Sandbox::new();
    implemented_sr(&s, true); // SIL3, no automated test evidence recorded
    s.run(&["validation", "plan", "SR-0001", "--plan", "review only"]);
    s.run(&[
        "validation",
        "analysis",
        "SR-0001",
        "--findings",
        "reviewed",
        "--result",
        "pass",
    ]);
    s.run(&[
        "validation",
        "test",
        "SR-0001",
        "--findings",
        "manual check",
        "--result",
        "pass",
    ]);
    let blocked = s.run(&[
        "validation",
        "conclude",
        "SR-0001",
        "--statement",
        "ok",
        "--promote",
    ]);
    assert!(
        !blocked.status.success(),
        "SIL3 promote without strong evidence must block"
    );
    assert!(
        stderr(&blocked).contains("SIL-rigour gate"),
        "stderr: {}",
        stderr(&blocked)
    );
    // Concluding without promote still records the dossier.
    assert!(s
        .run(&["validation", "conclude", "SR-0001", "--statement", "ok"])
        .status
        .success());
}

// ---------------------------------------------------------------------------
// REQ-0142: verification provenance report
// ---------------------------------------------------------------------------

/// REQ-0142: a genuinely-concluded dossier classifies as `genuine`, and the
/// report counts it as such; `--not-genuine` then hides it.
#[test]
fn req_0142_report_marks_genuine_dossier() {
    let s = Sandbox::new();
    implemented_req(&s);
    s.run(&["validation", "plan", "REQ-0001", "--plan", "review + test"]);
    s.run(&[
        "validation",
        "analysis",
        "REQ-0001",
        "--findings",
        "ok",
        "--result",
        "pass",
    ]);
    s.run(&[
        "validation",
        "test",
        "REQ-0001",
        "--findings",
        "green",
        "--result",
        "pass",
    ]);
    s.run(&[
        "validation",
        "conclude",
        "REQ-0001",
        "--statement",
        "met",
        "--promote",
    ]);
    let rep = s.run(&["validation", "report", "--json"]);
    assert!(rep.status.success(), "report: {}", stderr(&rep));
    let out = stdout(&rep);
    assert!(
        out.contains("\"genuine\": 1"),
        "expected genuine=1; got {}",
        out
    );
    assert!(out.contains("\"exempt_backfilled\": 0"), "got {}", out);
    // --not-genuine hides the genuine item.
    let only_bad = stdout(&s.run(&["validation", "report", "--not-genuine"]));
    assert!(
        !only_bad.contains("REQ-0001 "),
        "genuine item should be filtered out; got {}",
        only_bad
    );
}

/// REQ-0142: a backfilled exemption is reported as `exempt:backfilled`, is
/// surfaced under `--not-genuine`, and `req status` splits it out of the
/// genuine count.
#[test]
fn req_0142_report_marks_backfilled_exemption() {
    let s = Sandbox::new();
    s.init("p");
    s.run(&[
        "add",
        "--title",
        "Some verified behaviour",
        "--statement",
        "The system shall expose the baseline behaviour reliably.",
        "--rationale",
        "Fixture.",
        "--kind",
        "constraint",
        "--priority",
        "could",
    ]);
    for st in ["proposed", "approved", "implemented", "verified"] {
        s.run(&[
            "update", "REQ-0001", "--status", st, "--reason", "f", "--force",
        ]);
    }
    s.run(&[
        "validation",
        "backfill",
        "--all",
        "--reason",
        "grandfathered",
    ]);
    let rep = stdout(&s.run(&["validation", "report", "--json"]));
    assert!(rep.contains("\"genuine\": 0"), "got {}", rep);
    assert!(rep.contains("\"exempt_backfilled\": 1"), "got {}", rep);
    assert!(rep.contains("exempt:backfilled"), "got {}", rep);
    // --not-genuine still lists it.
    let only_bad = stdout(&s.run(&["validation", "report", "--not-genuine"]));
    assert!(only_bad.contains("REQ-0001"), "got {}", only_bad);
    // status splits the verified bucket.
    let st = stdout(&s.run(&["status", "--json"]));
    assert!(
        st.contains("\"verified_provenance\""),
        "status json missing provenance; got {}",
        st
    );
    assert!(st.contains("\"exempt\": 1"), "got {}", st);
}

/// REQ-0143: a safety requirement may NOT be exempted. `backfill <SR>` is
/// refused, `backfill --all` skips it, and a Verified SR without a genuine
/// dossier stays a hard REQ-V-0033 error.
#[test]
fn req_0143_safety_requirement_cannot_be_exempted() {
    let s = Sandbox::new();
    implemented_sr(&s, false);
    // Force the SR to Verified with no dossier (bypassing the verify gate).
    s.run(&[
        "sreq", "update", "SR-0001", "--status", "verified", "--reason", "force",
    ]);
    // It is now a hard validation error.
    let out = s.run(&["validate", "--json"]);
    assert!(!out.status.success(), "validate should fail");
    assert!(
        stdout(&out).contains("REQ-V-0033"),
        "expected REQ-V-0033; got {}",
        stdout(&out)
    );
    // backfill by id is refused for a safety requirement.
    let bf = s.run(&[
        "validation",
        "backfill",
        "SR-0001",
        "--reason",
        "grandfather",
    ]);
    assert!(!bf.status.success(), "backfill of an SR must be refused");
    assert!(
        stderr(&bf).to_lowercase().contains("safety requirement"),
        "stderr: {}",
        stderr(&bf)
    );
    // backfill --all skips it, so the error remains.
    let bfa = s.run(&["validation", "backfill", "--all", "--reason", "grandfather"]);
    assert!(bfa.status.success(), "backfill --all: {}", stderr(&bfa));
    assert!(
        !s.run(&["validate"]).status.success(),
        "SR error must persist — --all does not exempt safety requirements"
    );
}
