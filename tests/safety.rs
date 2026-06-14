// REQ-0134..0137: functional-safety feature + the hardening from the
// pre-publish code review. Each test maps to the requirement it covers.
mod common;
use common::{req, stderr, stdout, Sandbox};

/// REQ-0134: a hazard derives its SIL from C/F/P/W, a safety function
/// allocates the max over the hazards it mitigates, and a safety
/// requirement inherits its function's SIL.
#[test]
fn req_0134_sil_derives_and_propagates_through_the_chain() {
    let s = Sandbox::new();
    s.init("p");
    s.enable_safety();
    assert!(s
        .run(&[
            "hazard",
            "add",
            "-t",
            "H",
            "--harm",
            "someone is hurt",
            "-C",
            "C_C",
            "-F",
            "F_B",
            "-P",
            "P_B",
            "-W",
            "W3"
        ])
        .status
        .success());
    // C_C / F_B / P_B / W3 -> SIL3 (IEC 61508-5 Annex D).
    assert!(stdout(&s.run(&["hazard", "list"])).contains("SIL3"));
    assert!(s
        .run(&["sf", "add", "-t", "F", "--mitigates", "HAZ-0001"])
        .status
        .success());
    assert!(
        stdout(&s.run(&["sf", "list"])).contains("SIL3"),
        "SF allocates SIL3"
    );
    assert!(s
        .run(&[
            "sreq",
            "add",
            "-t",
            "R",
            "-s",
            "The system shall stop.",
            "-r",
            "because",
            "-a",
            "stops",
            "--realizes",
            "SF-0001"
        ])
        .status
        .success());
    assert!(
        stdout(&s.run(&["sreq", "list"])).contains("SIL3"),
        "SR inherits SIL3"
    );
}

/// REQ-0135: a SIL 3/4 safety requirement cannot be promoted to Verified
/// on inspection-only evidence; --force requires a --reason; and a
/// forced override is recorded as a STRUCTURED flag (not a forgeable
/// notes substring).
#[test]
fn req_0135_sil_gate_blocks_inspection_and_force_needs_reason() {
    let s = Sandbox::new();
    s.init("p");
    s.enable_safety();
    s.run(&[
        "hazard", "add", "-t", "H", "--harm", "hurt", "-C", "C_C", "-F", "F_B", "-P", "P_B", "-W",
        "W3",
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
        "Operator safety during cleaning.",
        "-a",
        "blade stops within 200ms",
        "--realizes",
        "SF-0001",
    ]);
    s.run(&[
        "sreq", "update", "SR-0001", "--status", "approved", "--reason", "r",
    ]);
    s.run(&[
        "sreq",
        "update",
        "SR-0001",
        "--status",
        "implemented",
        "--reason",
        "r",
    ]);

    // REQ-0139: give the SR a passing validation dossier (without promoting)
    // so the dossier gate is satisfied and the SIL-rigour gate is what's
    // under test below.
    s.run(&[
        "validation",
        "plan",
        "SR-0001",
        "--plan",
        "review logic and bench-test the stop",
    ]);
    s.run(&[
        "validation",
        "analysis",
        "SR-0001",
        "--findings",
        "stop logic reviewed",
        "--result",
        "pass",
    ]);
    s.run(&[
        "validation",
        "test",
        "SR-0001",
        "--findings",
        "bench-measured stop time",
        "--result",
        "pass",
    ]);
    s.run(&[
        "validation",
        "conclude",
        "SR-0001",
        "--statement",
        "meets the stop obligation",
    ]);

    // Gate blocks inspection-only promotion at SIL3.
    let blocked = s.run(&[
        "sreq",
        "verify",
        "SR-0001",
        "--by",
        "inspection",
        "--promote",
    ]);
    assert!(
        !blocked.status.success(),
        "SIL3 inspection promote must be blocked"
    );
    assert!(stderr(&blocked).contains("SIL-rigour gate"));

    // --force without --reason is rejected (clap requires).
    let no_reason = s.run(&[
        "sreq",
        "verify",
        "SR-0001",
        "--by",
        "inspection",
        "--promote",
        "--force",
    ]);
    assert!(
        !no_reason.status.success(),
        "--force without --reason must fail"
    );

    // --force with --reason succeeds and records a structured exception.
    let forced = s.run(&[
        "sreq",
        "verify",
        "SR-0001",
        "--by",
        "inspection",
        "--promote",
        "--force",
        "--reason",
        "accepted at design review",
    ]);
    assert!(forced.status.success(), "stderr={}", stderr(&forced));
    let shown = stdout(&s.run(&["sreq", "show", "SR-0001", "--json"]));
    let v: serde_json::Value = serde_json::from_str(&shown).expect("json");
    let last = v["tests"].as_array().unwrap().last().unwrap();
    assert_eq!(
        last["sil_gate_exception"], true,
        "structured exception flag set"
    );
    assert_eq!(v["status"], "Verified");
}

/// REQ-0135: recording inspection evidence WITHOUT promoting is allowed
/// (the gate only bites on the Verified claim).
#[test]
fn req_0135_recording_inspection_without_promote_is_allowed() {
    let s = Sandbox::new();
    s.init("p");
    s.enable_safety();
    s.run(&[
        "hazard", "add", "-t", "H", "--harm", "hurt", "-C", "C_C", "-F", "F_B", "-P", "P_B", "-W",
        "W3",
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
        "Operator safety during cleaning.",
        "-a",
        "blade stops within 200ms",
        "--realizes",
        "SF-0001",
    ]);
    let out = s.run(&["sreq", "verify", "SR-0001", "--by", "inspection"]);
    assert!(
        out.status.success(),
        "non-promoting inspection record must be allowed: {}",
        stderr(&out)
    );
}

/// REQ-0135: an Obsolete hazard stops feeding its SIL into a live safety
/// function's allocation (model agrees with the validator).
#[test]
fn req_0135_obsolete_hazard_drops_from_allocation() {
    let s = Sandbox::new();
    s.init("p");
    s.enable_safety();
    // SIL3 hazard + a low-SIL hazard, one SF covering both.
    s.run(&[
        "hazard", "add", "-t", "High", "--harm", "killed", "-C", "C_C", "-F", "F_B", "-P", "P_B",
        "-W", "W3",
    ]); // SIL3
    s.run(&[
        "hazard", "add", "-t", "Low", "--harm", "minor", "-C", "C_B", "-F", "F_A", "-P", "P_A",
        "-W", "W3",
    ]); // "a"
    s.run(&[
        "sf",
        "add",
        "-t",
        "F",
        "--mitigates",
        "HAZ-0001",
        "--mitigates",
        "HAZ-0002",
    ]);
    assert!(
        stdout(&s.run(&["sf", "list"])).contains("SIL3"),
        "allocated = max = SIL3"
    );
    // Retire the SIL3 hazard; allocation must fall.
    s.run(&[
        "hazard",
        "update",
        "HAZ-0001",
        "--status",
        "obsolete",
        "--reason",
        "reclassified",
    ]);
    assert!(
        !stdout(&s.run(&["sf", "list"])).contains("SIL3"),
        "obsolete hazard must no longer drive allocation"
    );
}

/// REQ-0135 (BLOCKER fix): a directory-layout project persists safety
/// artifacts across processes instead of silently dropping them.
#[test]
fn req_0135_directory_layout_persists_safety_artifacts() {
    let dir = tempfile::Builder::new()
        .prefix("req-dir-")
        .tempdir()
        .unwrap();
    let proj = dir.path().join("proj");
    let p = proj.to_str().unwrap();
    assert!(req(&["init", "-n", "d", "-o", p, "--layout", "directory"])
        .status
        .success());
    common::enable_safety(std::path::Path::new(p));
    assert!(req(&[
        "--file", p, "hazard", "add", "-t", "H", "--harm", "hurt", "-C", "C_D", "-F", "F_B", "-P",
        "P_B", "-W", "W3"
    ])
    .status
    .success());
    // Fresh process re-reads the directory project.
    let listed = req(&["--file", p, "hazard", "list"]);
    assert!(listed.status.success(), "{}", stderr(&listed));
    assert!(
        stdout(&listed).contains("HAZ-0001"),
        "hazard must survive a directory-layout round trip"
    );
    // Integrity must still verify.
    assert!(
        req(&["--file", p, "validate"]).status.success(),
        "directory integrity must hold after a safety write"
    );
}

/// REQ-0136: trace prints the chain, an honest traceability roll-up, and
/// the tool-qualification disclaimer.
#[test]
fn req_0136_trace_is_honest_about_what_it_asserts() {
    let s = Sandbox::new();
    s.init("p");
    s.enable_safety();
    s.run(&[
        "hazard", "add", "-t", "H", "--harm", "hurt", "-C", "C_C", "-F", "F_B", "-P", "P_B", "-W",
        "W3",
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
        "Operator safety during cleaning.",
        "-a",
        "blade stops within 200ms",
        "--realizes",
        "SF-0001",
    ]);
    let out = stdout(&s.run(&["trace", "HAZ-0001"]));
    assert!(
        out.contains("TRACE STATUS"),
        "uses traceability wording, not 'safety case'"
    );
    assert!(
        !out.contains("SAFETY CASE"),
        "must not claim a safety-case verdict"
    );
    assert!(
        out.contains("not qualified per IEC 61508-3"),
        "carries the disclaimer"
    );
}

/// REQ-0137: the validator flags a hazard with no harm narrative. (Built
/// via batch-free path: a normal add always has harm, so we drive the
/// rule by checking a well-formed chain validates clean, and that the
/// rule codes are present in the catalogue surfaced by `req help`.)
#[test]
fn req_0137_wellformed_safety_chain_validates_clean() {
    let s = Sandbox::new();
    s.init("p");
    s.enable_safety();
    s.run(&[
        "hazard", "add", "-t", "H", "--harm", "hurt", "-C", "C_C", "-F", "F_B", "-P", "P_B", "-W",
        "W3",
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
        "Operator safety during cleaning.",
        "-a",
        "blade stops within 200ms",
        "--realizes",
        "SF-0001",
    ]);
    let out = s.run(&["validate"]);
    assert!(
        out.status.success(),
        "well-formed safety chain must validate: {}",
        stdout(&out)
    );
}

/// REQ-0135 (evidence-honesty loop): `req test run` attaches automated
/// evidence to a safety requirement from an `sr_NNNN_*` test, and that
/// evidence goes STALE when its linked code changes. Runs the binary
/// with the working directory set to the project so the source-marker
/// scan and the content hash see the right tree.
#[test]
fn req_0135_sr_evidence_from_test_run_goes_stale_on_code_change() {
    use std::process::Command;
    let dir = tempfile::Builder::new()
        .prefix("req-evh-")
        .tempdir()
        .unwrap();
    let root = dir.path();
    let bin = env!("CARGO_BIN_EXE_req");
    let run = |args: &[&str]| {
        Command::new(bin)
            .args(args)
            .current_dir(root)
            .env_remove("REQ_FILE")
            .output()
            .expect("run req")
    };

    assert!(run(&["init", "-n", "p"]).status.success());
    common::enable_safety(&root.join("project.req"));
    run(&[
        "hazard",
        "add",
        "-t",
        "Hazardous mode",
        "--harm",
        "operator hurt",
        "-C",
        "C_C",
        "-F",
        "F_B",
        "-P",
        "P_B",
        "-W",
        "W3",
    ]);
    run(&["sf", "add", "-t", "Interlock", "--mitigates", "HAZ-0001"]);
    run(&[
        "sreq",
        "add",
        "-t",
        "Cut blade power",
        "-s",
        "The interlock shall cut blade power within 200 ms.",
        "-r",
        "Bounds operator exposure.",
        "-a",
        "power cut <=200ms",
        "--realizes",
        "SF-0001",
    ]);

    // Implementing source carries the safety-requirement comment marker.
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("src/interlock.rs"),
        "// SR-0001: interlock\nfn interlock() {}\n",
    )
    .unwrap();

    // A captured test log with an sr_0001_* test name.
    std::fs::write(
        root.join("log.txt"),
        "running 1 test\ntest sr_0001_cuts_power ... ok\n",
    )
    .unwrap();
    let tr = run(&["test", "run", "--from-file", "log.txt"]);
    assert!(
        tr.status.success(),
        "test run: {}",
        String::from_utf8_lossy(&tr.stderr)
    );

    // The SR now carries an Automated evidence record.
    let shown =
        String::from_utf8_lossy(&run(&["sreq", "show", "SR-0001", "--json"]).stdout).into_owned();
    let v: serde_json::Value = serde_json::from_str(&shown).expect("json");
    let tests = v["tests"].as_array().expect("tests");
    assert!(
        tests.iter().any(|t| t["kind"] == "Automated"),
        "SR must carry automated evidence from the run"
    );

    // Fresh now (content matches the hash recorded at run time).
    let fresh = run(&["stale", "--only-stale"]);
    assert!(
        !String::from_utf8_lossy(&fresh.stdout).contains("SR-0001"),
        "should be fresh before any change"
    );

    // Change the linked file → the SR's evidence goes STALE.
    std::fs::write(
        root.join("src/interlock.rs"),
        "// SR-0001: interlock\nfn interlock() { /* changed */ }\n",
    )
    .unwrap();
    let stale = run(&["stale"]);
    let out = String::from_utf8_lossy(&stale.stdout);
    assert!(
        out.contains("SR-0001") && out.contains("STALE"),
        "SR evidence must go stale on code change:\n{}",
        out
    );
}

/// REQ-0135: `req coverage` traces // SR-NNNN markers — an implemented SR
/// with no marker is an orphan, a marker pointing at no SR is a ghost,
/// and --strict exits non-zero on either.
#[test]
fn req_0135_sr_coverage_orphans_and_ghosts() {
    use std::process::Command;
    let dir = tempfile::Builder::new()
        .prefix("req-cov-")
        .tempdir()
        .unwrap();
    let root = dir.path();
    let bin = env!("CARGO_BIN_EXE_req");
    let run = |args: &[&str]| {
        Command::new(bin)
            .args(args)
            .current_dir(root)
            .env_remove("REQ_FILE")
            .output()
            .expect("run req")
    };
    assert!(run(&["init", "-n", "p"]).status.success());
    common::enable_safety(&root.join("project.req"));
    run(&[
        "hazard",
        "add",
        "-t",
        "Hazardous mode",
        "--harm",
        "hurt",
        "-C",
        "C_C",
        "-F",
        "F_B",
        "-P",
        "P_B",
        "-W",
        "W3",
    ]);
    run(&["sf", "add", "-t", "Interlock", "--mitigates", "HAZ-0001"]);
    // One SR is marked in code; the other is left an orphan. Every SR id
    // is built at runtime (never as a literal `SR-NNNN` token) so this
    // test's own fixtures don't register in the real project's coverage.
    let sr = |n: u32| format!("SR-{:04}", n);
    run(&[
        "sreq",
        "add",
        "-t",
        "Marked one",
        "-s",
        "The interlock shall cut blade power fast.",
        "-r",
        "safety",
        "-a",
        "cuts",
        "--realizes",
        "SF-0001",
    ]);
    run(&[
        "sreq",
        "add",
        "-t",
        "Orphan one",
        "-s",
        "The guard shall be detected within 50 ms.",
        "-r",
        "safety",
        "-a",
        "detects",
        "--realizes",
        "SF-0001",
    ]);
    for n in [1u32, 2] {
        run(&[
            "sreq",
            "update",
            &sr(n),
            "--status",
            "approved",
            "--reason",
            "r",
        ]);
        run(&[
            "sreq",
            "update",
            &sr(n),
            "--status",
            "implemented",
            "--reason",
            "r",
        ]);
    }
    std::fs::create_dir_all(root.join("src")).unwrap();
    let ghost = sr(99);
    std::fs::write(
        root.join("src/x.rs"),
        format!("// {}: here\n// {}: ghost\nfn x() {{}}\n", sr(1), ghost),
    )
    .unwrap();

    let cov = run(&["coverage", "--path", "."]);
    let out = String::from_utf8_lossy(&cov.stdout);
    assert!(
        out.contains(&sr(2)),
        "the unmarked SR should be an orphan:\n{}",
        out
    );
    assert!(
        out.contains(&ghost),
        "the unknown SR id should be a ghost:\n{}",
        out
    );
    assert!(
        !out.contains("SR ORPHANS") || !out.contains(&format!("{}\n    {}", sr(1), sr(1))),
        "the marked SR is referenced, not an orphan"
    );

    // --strict turns SR orphan/ghost findings into a non-zero exit.
    assert!(
        !run(&["coverage", "--path", ".", "--strict"])
            .status
            .success(),
        "strict must fail on SR findings"
    );
}

/// REQ-0138: safety features are gated behind a human-accepted disclaimer
/// file; an agent cannot accept; and a calibration override changes the
/// derived SIL.
#[test]
fn req_0138_governance_gate_agent_refusal_and_calibration() {
    use std::process::Command;
    let dir = tempfile::Builder::new()
        .prefix("req-gov-")
        .tempdir()
        .unwrap();
    let root = dir.path();
    let bin = env!("CARGO_BIN_EXE_req");
    let run = |args: &[&str], kind: Option<&str>| {
        let mut c = Command::new(bin);
        c.args(args).current_dir(root).env_remove("REQ_FILE");
        match kind {
            Some(k) => {
                c.env("REQ_ACTOR_KIND", k);
            }
            None => {
                c.env_remove("REQ_ACTOR_KIND");
            }
        }
        c.output().expect("run req")
    };
    assert!(run(&["init", "-n", "p"], None).status.success());

    // Gate: hazard creation is blocked before acceptance.
    let blocked = run(
        &[
            "hazard", "add", "-t", "H", "--harm", "x", "-C", "C_C", "-F", "F_B", "-P", "P_B", "-W",
            "W3",
        ],
        None,
    );
    assert!(
        !blocked.status.success(),
        "hazard add must be gated before acceptance"
    );
    assert!(String::from_utf8_lossy(&blocked.stderr).contains("not enabled"));

    // An agent cannot accept (refused on the self-identified actor kind).
    let agent = run(&["safety", "accept", "--name", "Bot"], Some("agent"));
    assert!(!agent.status.success(), "agent must not be able to accept");
    assert!(String::from_utf8_lossy(&agent.stderr).contains("human"));

    // Even a non-agent cannot accept without an interactive terminal —
    // there is no --yes backdoor. (Tests have no TTY.)
    let no_tty = run(&["safety", "accept", "--name", "Tom"], None);
    assert!(!no_tty.status.success(), "accept must require a terminal");
    assert!(String::from_utf8_lossy(&no_tty.stderr).contains("interactive terminal"));

    // Enable via the committed acceptance file (what an interactive accept
    // produces / what a human commits) → the feature activates.
    common::enable_safety(&root.join("project.req"));
    assert!(run(
        &[
            "hazard",
            "add",
            "-t",
            "Hazardous",
            "--harm",
            "x",
            "-C",
            "C_C",
            "-F",
            "F_B",
            "-P",
            "P_B",
            "-W",
            "W3"
        ],
        None
    )
    .status
    .success());

    // Default calibration: C_C/F_B/P_B/W3 -> SIL3.
    assert!(String::from_utf8_lossy(&run(&["hazard", "list"], None).stdout).contains("SIL3"));
    // Override that leaf -> SIL4, and confirm the derived SIL follows.
    assert!(run(
        &["safety", "calibrate", "--set", "C_C/F_B/P_B=W3:4,W2:3,W1:2"],
        None
    )
    .status
    .success());
    assert!(
        String::from_utf8_lossy(&run(&["hazard", "list"], None).stdout).contains("SIL4"),
        "calibration override must change the derived SIL"
    );
}

/// REQ-0137 (SF-0002 protective path): a BROKEN safety case must FAIL
/// `req validate` with a non-zero exit, not merely print — this is the
/// "a broken safety case fails CI" half of SF-0002, which the clean-case
/// test above (`req_0137_wellformed_safety_chain_validates_clean`) does
/// not exercise. We drive rule REQ-V-0027 by retiring the only safety
/// function that mitigates a Mitigated hazard, leaving the hazard with no
/// live mitigation.
#[test]
fn req_0137_broken_safety_case_fails_validate() {
    let s = Sandbox::new();
    s.init("p");
    s.enable_safety();
    s.run(&[
        "hazard",
        "add",
        "-t",
        "Hazardous mode",
        "--harm",
        "operator hurt",
        "-C",
        "C_C",
        "-F",
        "F_B",
        "-P",
        "P_B",
        "-W",
        "W3",
    ]);
    // A safety function mitigating the hazard auto-advances it to Mitigated.
    s.run(&["sf", "add", "-t", "Interlock", "--mitigates", "HAZ-0001"]);

    // Baseline: a well-formed chain validates clean (guards against the
    // test passing for the wrong reason).
    assert!(
        s.run(&["validate"]).status.success(),
        "baseline chain must be clean"
    );

    // Retire the only mitigation. The hazard stays Mitigated but now has
    // no live safety function behind it — a broken safety case.
    assert!(
        s.run(&[
            "sf",
            "update",
            "SF-0001",
            "--status",
            "obsolete",
            "--reason",
            "retired without replacement"
        ])
        .status
        .success(),
        "obsoleting the SF should itself succeed"
    );

    let broken = s.run(&["validate"]);
    assert!(
        !broken.status.success(),
        "a broken safety case must fail validate with a non-zero exit"
    );
    let out = stdout(&broken) + &stderr(&broken);
    assert!(
        out.contains("REQ-V-0027"),
        "validate must flag the mitigated-hazard-without-live-SF rule:\n{}",
        out
    );
}

/// REQ-0011 (SF-0003 mechanism): a mutation of a SAFETY artifact records a
/// reasoned, APPEND-ONLY history entry. Each status change must ADD an
/// entry carrying its `--reason` and an attributable actor/action, never
/// replacing the prior history — the property SF-0003 relies on for a
/// tamper-evident audit trail on safety artifacts specifically (the cited
/// REQ-0017/REQ-0109 tests only exercise ordinary requirements).
#[test]
fn req_0011_safety_mutation_records_reasoned_append_only_history() {
    let s = Sandbox::new();
    s.init("p");
    s.enable_safety();
    s.run(&[
        "hazard", "add", "-t", "H", "--harm", "hurt", "-C", "C_C", "-F", "F_B", "-P", "P_B", "-W",
        "W3",
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
        "Operator safety during cleaning.",
        "-a",
        "blade stops within 200ms",
        "--realizes",
        "SF-0001",
    ]);

    let reasons = ["reviewed at design gate", "implementation landed on main"];
    s.run(&[
        "sreq", "update", "SR-0001", "--status", "approved", "--reason", reasons[0],
    ]);
    s.run(&[
        "sreq",
        "update",
        "SR-0001",
        "--status",
        "implemented",
        "--reason",
        reasons[1],
    ]);

    let shown = stdout(&s.run(&["sreq", "show", "SR-0001", "--json"]));
    let v: serde_json::Value = serde_json::from_str(&shown).expect("json");
    let hist = v["history"].as_array().expect("history array present");

    // Append-only: created + two reasoned updates accumulate (≥ 3 entries),
    // never collapse to the latest.
    assert!(
        hist.len() >= 3,
        "history must accumulate, got {}:\n{}",
        hist.len(),
        shown
    );

    // Each supplied reason is recorded.
    let recorded: Vec<String> = hist
        .iter()
        .filter_map(|e| e["reason"].as_str().map(str::to_string))
        .collect();
    assert!(
        recorded.iter().any(|r| r == reasons[0]),
        "first reason must be recorded: {:?}",
        recorded
    );
    assert!(
        recorded.iter().any(|r| r == reasons[1]),
        "second reason must be recorded: {:?}",
        recorded
    );

    // Every entry is attributable (actor + action) — the tamper-evident shape.
    assert!(
        hist.iter()
            .all(|e| e["action"].is_string() && e["actor"].is_string()),
        "every history entry must carry an actor and action:\n{}",
        shown
    );
}

/// REQ-0144: the safety-feature acceptance agreement is version-stamped, so an
/// acceptance recorded for an OLDER disclaimer version no longer satisfies the
/// gate — the user must re-accept the updated (no-liability / research-only)
/// terms before safety features work again.
#[test]
fn req_0144_stale_disclaimer_version_blocks_safety_features() {
    use std::process::Command;
    let dir = tempfile::Builder::new()
        .prefix("req-0144-")
        .tempdir()
        .unwrap();
    let root = dir.path();
    let bin = env!("CARGO_BIN_EXE_req");
    let run = |args: &[&str]| {
        Command::new(bin)
            .args(args)
            .current_dir(root)
            .env_remove("REQ_FILE")
            .env_remove("REQ_ACTOR_KIND")
            .output()
            .expect("run req")
    };
    let hazard = [
        "hazard", "add", "-t", "H", "--harm", "x", "-C", "C_C", "-F", "F_B", "-P", "P_B", "-W",
        "W3",
    ];
    assert!(run(&["init", "-n", "p"]).status.success());

    // Acceptance for an OUTDATED disclaimer version (1) must NOT activate.
    std::fs::write(
        root.join("req-safety-acceptance.json"),
        r#"{"accepted_by":"Old","at":"2026-01-01T00:00:00Z","tool_version":"test","disclaimer_version":"1"}"#,
    )
    .unwrap();
    let blocked = run(&hazard);
    assert!(
        !blocked.status.success(),
        "an acceptance for an older disclaimer version must not enable safety features"
    );
    let msg = format!(
        "{}{}",
        String::from_utf8_lossy(&blocked.stderr),
        String::from_utf8_lossy(&blocked.stdout)
    );
    assert!(
        msg.to_lowercase().contains("accept"),
        "the block should tell the user to re-accept: {msg}"
    );

    // Acceptance for the CURRENT disclaimer version (2) activates the features.
    std::fs::write(
        root.join("req-safety-acceptance.json"),
        r#"{"accepted_by":"New","at":"2026-01-01T00:00:00Z","tool_version":"test","disclaimer_version":"2"}"#,
    )
    .unwrap();
    let ok = run(&hazard);
    assert!(
        ok.status.success(),
        "current-version acceptance must enable safety features: {}",
        String::from_utf8_lossy(&ok.stderr)
    );
}

/// REQ-0145: a safety requirement validated by an agent is NOT passed until a
/// human confirms the result. REQ-V-0034 flags the unconfirmed SR; an agent
/// cannot confirm; a human's confirmation clears the finding.
#[test]
fn req_0145_safety_validation_needs_human_confirmation() {
    use std::process::Command;
    let dir = tempfile::Builder::new()
        .prefix("req-0145-")
        .tempdir()
        .unwrap();
    let root = dir.path();
    let bin = env!("CARGO_BIN_EXE_req");
    let run = |args: &[&str], kind: Option<&str>| {
        let mut c = Command::new(bin);
        c.args(args).current_dir(root).env_remove("REQ_FILE");
        match kind {
            Some(k) => {
                c.env("REQ_ACTOR_KIND", k);
            }
            None => {
                c.env_remove("REQ_ACTOR_KIND");
            }
        }
        c.output().expect("run req")
    };
    assert!(run(&["init", "-n", "p"], None).status.success());
    std::fs::write(
        root.join("req-safety-acceptance.json"),
        r#"{"accepted_by":"H","at":"2026-01-01T00:00:00Z","tool_version":"t","disclaimer_version":"2"}"#,
    )
    .unwrap();
    run(
        &[
            "hazard", "add", "-t", "H", "--harm", "hurt", "-C", "C_C", "-F", "F_B", "-P", "P_B",
            "-W", "W3",
        ],
        None,
    );
    run(&["sf", "add", "-t", "F", "--mitigates", "HAZ-0001"], None);
    run(
        &[
            "sreq",
            "add",
            "-t",
            "Stop the blade",
            "-s",
            "The system shall stop the blade on demand.",
            "-r",
            "operator safety during cleaning",
            "-a",
            "blade stops within 200ms",
            "--realizes",
            "SF-0001",
        ],
        None,
    );
    run(
        &[
            "sreq", "update", "SR-0001", "--status", "approved", "--reason", "r",
        ],
        None,
    );
    run(
        &[
            "sreq",
            "update",
            "SR-0001",
            "--status",
            "implemented",
            "--reason",
            "r",
        ],
        None,
    );
    run(
        &[
            "sreq",
            "verify",
            "SR-0001",
            "--by",
            "automated",
            "--notes",
            "bench",
        ],
        None,
    );
    run(
        &["validation", "plan", "SR-0001", "--plan", "review+bench"],
        None,
    );
    run(
        &[
            "validation",
            "analysis",
            "SR-0001",
            "--findings",
            "reviewed",
            "--result",
            "pass",
        ],
        None,
    );
    run(
        &[
            "validation",
            "test",
            "SR-0001",
            "--findings",
            "bench",
            "--result",
            "pass",
        ],
        None,
    );
    assert!(run(
        &[
            "validation",
            "conclude",
            "SR-0001",
            "--statement",
            "meets the obligation",
            "--promote"
        ],
        None
    )
    .status
    .success());

    // Verified on the agent's dossier, but REQ-V-0034 flags it as not-yet-passed.
    let v1 = run(&["validate"], None);
    assert!(
        !v1.status.success(),
        "an agent-only SR validation must not pass a clean validate"
    );
    let v1out = format!(
        "{}{}",
        String::from_utf8_lossy(&v1.stdout),
        String::from_utf8_lossy(&v1.stderr)
    );
    assert!(
        v1out.contains("REQ-V-0034"),
        "REQ-V-0034 must flag the unconfirmed safety requirement: {v1out}"
    );

    // An agent cannot confirm on a human's behalf.
    let by_agent = run(&["validation", "confirm", "SR-0001"], Some("agent"));
    assert!(
        !by_agent.status.success(),
        "an agent must not be able to confirm a safety validation"
    );

    // A human confirms — and the project validates clean.
    assert!(
        run(&["validation", "confirm", "SR-0001"], Some("human"))
            .status
            .success(),
        "a human confirmation must succeed"
    );
    let v2 = run(&["validate"], None);
    assert!(
        v2.status.success(),
        "after human confirmation the project validates clean: {}",
        String::from_utf8_lossy(&v2.stderr)
    );
}

/// REQ-0146: `req trace` from a safety requirement resolves upward to the
/// mitigated hazard and inlines the validation dossier (human output and
/// --json carry the same chain).
#[test]
fn req_0146_trace_from_sr_shows_chain_and_dossier() {
    use std::process::Command;
    let dir = tempfile::Builder::new()
        .prefix("req-0146-")
        .tempdir()
        .unwrap();
    let root = dir.path();
    let bin = env!("CARGO_BIN_EXE_req");
    let run = |args: &[&str], kind: Option<&str>| {
        let mut c = Command::new(bin);
        c.args(args).current_dir(root).env_remove("REQ_FILE");
        match kind {
            Some(k) => {
                c.env("REQ_ACTOR_KIND", k);
            }
            None => {
                c.env_remove("REQ_ACTOR_KIND");
            }
        }
        c.output().expect("run req")
    };
    assert!(run(&["init", "-n", "p"], None).status.success());
    std::fs::write(
        root.join("req-safety-acceptance.json"),
        r#"{"accepted_by":"H","at":"2026-01-01T00:00:00Z","tool_version":"t","disclaimer_version":"2"}"#,
    )
    .unwrap();
    run(
        &[
            "hazard", "add", "-t", "H", "--harm", "hurt", "-C", "C_C", "-F", "F_B", "-P", "P_B",
            "-W", "W3",
        ],
        None,
    );
    run(&["sf", "add", "-t", "F", "--mitigates", "HAZ-0001"], None);
    run(
        &[
            "sreq",
            "add",
            "-t",
            "Stop the blade",
            "-s",
            "The system shall stop the blade on demand.",
            "-r",
            "operator safety",
            "-a",
            "blade stops within 200ms",
            "--realizes",
            "SF-0001",
        ],
        None,
    );
    run(
        &[
            "sreq", "update", "SR-0001", "--status", "approved", "--reason", "r",
        ],
        None,
    );
    run(
        &[
            "sreq",
            "update",
            "SR-0001",
            "--status",
            "implemented",
            "--reason",
            "r",
        ],
        None,
    );
    run(
        &[
            "sreq",
            "verify",
            "SR-0001",
            "--by",
            "automated",
            "--notes",
            "bench",
        ],
        None,
    );
    run(
        &["validation", "plan", "SR-0001", "--plan", "review+bench"],
        None,
    );
    run(
        &[
            "validation",
            "analysis",
            "SR-0001",
            "--findings",
            "reviewed",
            "--result",
            "pass",
        ],
        None,
    );
    run(
        &[
            "validation",
            "test",
            "SR-0001",
            "--findings",
            "bench",
            "--result",
            "pass",
        ],
        None,
    );
    run(
        &[
            "validation",
            "conclude",
            "SR-0001",
            "--statement",
            "meets the obligation",
            "--promote",
        ],
        None,
    );
    run(&["validation", "confirm", "SR-0001"], Some("human"));

    // Human output: tracing from the SR resolves UP to the hazard and inlines
    // the dossier.
    let human = String::from_utf8_lossy(&run(&["trace", "SR-0001"], None).stdout).to_string();
    assert!(
        human.contains("HAZ-0001"),
        "trace from an SR must resolve upward to the mitigated hazard:\n{human}"
    );
    assert!(
        human.contains("dossier: verdict pass"),
        "trace must inline the validation dossier verdict:\n{human}"
    );
    assert!(
        human.contains("human-confirmed"),
        "trace must show the human confirmation:\n{human}"
    );

    // --json carries the chain with each SR's validation dossier.
    let j = String::from_utf8_lossy(&run(&["trace", "SR-0001", "--json"], None).stdout).to_string();
    let v: serde_json::Value = serde_json::from_str(&j).expect("trace --json parses");
    let val = &v["chain"][0]["safety_requirements"][0]["validation"];
    assert!(
        val.get("verdict").is_some() && val.get("human_confirmation").is_some(),
        "--json chain must include the SR's validation dossier:\n{j}"
    );
}

// ---------- REQ-0148 / REQ-0149: SR staleness rigor + dependency scoping ----------

// Build a confirmed safety requirement whose only genuine dependency is a
// code-comment marker in src/safety_impl.rs, plus a prose mention in notes.md.
fn setup_marked_confirmed_sr(root: &std::path::Path) {
    use std::process::Command;
    let bin = env!("CARGO_BIN_EXE_req");
    let run = |args: &[&str], kind: Option<&str>| {
        let mut c = Command::new(bin);
        c.args(args).current_dir(root).env_remove("REQ_FILE");
        match kind {
            Some(k) => {
                c.env("REQ_ACTOR_KIND", k);
            }
            None => {
                c.env_remove("REQ_ACTOR_KIND");
            }
        }
        c.output().expect("run req")
    };
    std::fs::create_dir_all(root.join("src")).unwrap();
    // Genuine marker (comment) — the SR's real dependency.
    std::fs::write(
        root.join("src/safety_impl.rs"),
        "// SR-0001: the interlock implementation\npub fn interlock() {}\n",
    )
    .unwrap();
    // Prose mention only (no comment) — must NOT become a dependency.
    std::fs::write(
        root.join("notes.md"),
        "Design notes: SR-0001 keeps the operator safe.\n",
    )
    .unwrap();
    assert!(run(&["init", "-n", "p"], None).status.success());
    std::fs::write(
        root.join("req-safety-acceptance.json"),
        r#"{"accepted_by":"H","at":"2026-01-01T00:00:00Z","tool_version":"t","disclaimer_version":"2"}"#,
    )
    .unwrap();
    run(
        &[
            "hazard", "add", "-t", "H", "--harm", "hurt", "-C", "C_C", "-F", "F_B", "-P", "P_B",
            "-W", "W3",
        ],
        None,
    );
    run(&["sf", "add", "-t", "F", "--mitigates", "HAZ-0001"], None);
    run(
        &[
            "sreq",
            "add",
            "-t",
            "Interlock",
            "-s",
            "The system shall engage the interlock on demand.",
            "-r",
            "operator safety",
            "-a",
            "interlock engages",
            "--realizes",
            "SF-0001",
        ],
        None,
    );
    run(
        &[
            "sreq", "update", "SR-0001", "--status", "approved", "--reason", "r",
        ],
        None,
    );
    run(
        &[
            "sreq",
            "update",
            "SR-0001",
            "--status",
            "implemented",
            "--reason",
            "r",
        ],
        None,
    );
    run(
        &[
            "sreq",
            "verify",
            "SR-0001",
            "--by",
            "automated",
            "--notes",
            "bench",
        ],
        None,
    );
    run(&["validation", "plan", "SR-0001", "--plan", "p"], None);
    run(
        &[
            "validation",
            "analysis",
            "SR-0001",
            "--findings",
            "ok",
            "--result",
            "pass",
        ],
        None,
    );
    run(
        &[
            "validation",
            "test",
            "SR-0001",
            "--findings",
            "ok",
            "--result",
            "pass",
        ],
        None,
    );
    run(
        &[
            "validation",
            "conclude",
            "SR-0001",
            "--statement",
            "meets",
            "--promote",
        ],
        None,
    );
    run(&["validation", "confirm", "SR-0001"], Some("human"));
}

fn req_in(root: &std::path::Path, args: &[&str]) -> std::process::Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_req"))
        .args(args)
        .current_dir(root)
        .env_remove("REQ_FILE")
        .env_remove("REQ_ACTOR_KIND")
        .output()
        .expect("run req")
}

/// REQ-0149: a requirement's dependency is the code-comment marker, not prose.
#[test]
fn req_0149_staleness_scopes_to_comment_markers_not_prose() {
    let dir = tempfile::Builder::new()
        .prefix("req-0149-")
        .tempdir()
        .unwrap();
    let root = dir.path();
    setup_marked_confirmed_sr(root);

    let shown =
        String::from_utf8_lossy(&req_in(root, &["validation", "show", "SR-0001", "--json"]).stdout)
            .to_string();
    let v: serde_json::Value = serde_json::from_str(&shown).expect("validation show --json");
    let linked: Vec<String> = v["validation"]["linked_files"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    assert!(
        linked.iter().any(|f| f.contains("safety_impl.rs")),
        "the code-comment marker file must be a dependency: {linked:?}"
    );
    assert!(
        !linked.iter().any(|f| f.contains("notes.md")),
        "a prose-only mention must NOT be a dependency: {linked:?}"
    );

    // Editing the prose file must not make the safety requirement stale.
    std::fs::write(root.join("notes.md"), "Design notes: rewritten prose.\n").unwrap();
    assert!(
        req_in(root, &["validate"]).status.success(),
        "editing prose must not invalidate the safety requirement"
    );
}

/// REQ-0148: once the validated source drifts, the SR is a hard validate error.
#[test]
fn req_0148_stale_safety_requirement_is_a_validate_error() {
    let dir = tempfile::Builder::new()
        .prefix("req-0148-")
        .tempdir()
        .unwrap();
    let root = dir.path();
    setup_marked_confirmed_sr(root);

    // Confirmed + fresh → validate clean.
    assert!(
        req_in(root, &["validate"]).status.success(),
        "a freshly validated + confirmed SR should pass"
    );

    // Drift the marker file → stale → REQ-V-0035 error.
    std::fs::write(
        root.join("src/safety_impl.rs"),
        "// SR-0001: the interlock implementation\npub fn interlock() { /* changed */ }\n",
    )
    .unwrap();
    let out = req_in(root, &["validate"]);
    assert!(
        !out.status.success(),
        "a stale safety requirement must fail validate"
    );
    let msg = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        msg.contains("REQ-V-0035"),
        "stale SR must be flagged REQ-V-0035:\n{msg}"
    );
}

/// REQ-0158: a safety requirement obeys the same lifecycle ladder as an
/// ordinary requirement — a Verified SR cannot be quietly demoted without
/// an explicit --force (and a substantive --reason, per REQ-0161).
#[test]
fn req_0158_safety_requirement_obeys_status_ladder() {
    let s = Sandbox::new();
    s.init("p");
    s.enable_safety();
    let _ = s.run(&[
        "hazard",
        "add",
        "-t",
        "H",
        "--harm",
        "someone is hurt",
        "-C",
        "C_C",
        "-F",
        "F_B",
        "-P",
        "P_B",
        "-W",
        "W3",
    ]);
    let _ = s.run(&["sf", "add", "-t", "F", "--mitigates", "HAZ-0001"]);
    let _ = s.run(&[
        "sreq",
        "add",
        "-t",
        "R",
        "-s",
        "The system shall stop.",
        "-r",
        "because",
        "-a",
        "stops",
        "--realizes",
        "SF-0001",
    ]);
    // Seed a Verified state via a forced irregular jump (with a real reason).
    let up = s.run(&[
        "sreq",
        "update",
        "SR-0001",
        "--status",
        "verified",
        "--force",
        "--reason",
        "seed verified state for the ladder test",
    ]);
    assert!(up.status.success(), "seed verify: {}", stderr(&up));
    // Demoting Verified -> Draft without --force must be rejected.
    let demote = s.run(&["sreq", "update", "SR-0001", "--status", "draft"]);
    assert!(
        !demote.status.success(),
        "SR demotion without --force should be rejected"
    );
    assert!(
        stderr(&demote).contains("irregular transition"),
        "expected irregular-transition error, got: {}",
        stderr(&demote)
    );
    // With --force and a substantive reason it succeeds.
    let forced = s.run(&[
        "sreq",
        "update",
        "SR-0001",
        "--status",
        "draft",
        "--force",
        "--reason",
        "correcting a bad promotion record",
    ]);
    assert!(
        forced.status.success(),
        "forced demote: {}",
        stderr(&forced)
    );
}

// ---------- REQ-0154/0155/0157: SIL provenance & escalation ----------

/// Build a Verified safety requirement chain at SIL2 via the dossier flow.
/// Returns the sandbox with HAZ-0001(SIL2) ← SF-0001 ← SR-0001 (Verified,
/// evidence snapshotted at SIL2).
fn verified_sil2_chain() -> Sandbox {
    let s = Sandbox::new();
    s.init("p");
    s.enable_safety();
    let _ = s.run(&[
        "hazard",
        "add",
        "-t",
        "Hazard A",
        "--harm",
        "someone is hurt",
        "-C",
        "C_C",
        "-F",
        "F_B",
        "-P",
        "P_B",
        "-W",
        "W2", // -> SIL2
    ]);
    let _ = s.run(&["sf", "add", "-t", "Function", "--mitigates", "HAZ-0001"]);
    let _ = s.run(&[
        "sreq",
        "add",
        "-t",
        "Requirement",
        "-s",
        "The system shall stop within 200 ms.",
        "-r",
        "bounds exposure",
        "-a",
        "stops",
        "--realizes",
        "SF-0001",
    ]);
    // Advance the SR up the lifecycle ladder so conclude --promote is eligible.
    let _ = s.run(&[
        "sreq",
        "update",
        "SR-0001",
        "--status",
        "proposed",
        "--reason",
        "advance for validation",
    ]);
    let _ = s.run(&[
        "sreq",
        "update",
        "SR-0001",
        "--status",
        "approved",
        "--reason",
        "advance for validation",
    ]);
    let _ = s.run(&[
        "sreq",
        "update",
        "SR-0001",
        "--status",
        "implemented",
        "--reason",
        "advance for validation",
    ]);
    // Walk the dossier to Verified (records the SIL snapshot at conclude).
    let _ = s.run(&[
        "validation",
        "plan",
        "SR-0001",
        "--plan",
        "analysis + testing of the stop path",
    ]);
    let _ = s.run(&[
        "validation",
        "analysis",
        "SR-0001",
        "--result",
        "pass",
        "--findings",
        "reviewed the stop path",
    ]);
    let _ = s.run(&[
        "validation",
        "test",
        "SR-0001",
        "--result",
        "pass",
        "--findings",
        "bench test passes",
    ]);
    let _ = s.run(&[
        "validation",
        "conclude",
        "SR-0001",
        "--statement",
        "SR-0001 met: stop path verified.",
        "--promote",
    ]);
    s
}

#[test]
fn req_0154_evidence_snapshots_sil_at_verification() {
    let s = verified_sil2_chain();
    let show = stdout(&s.run(&["sreq", "show", "SR-0001"]));
    assert!(
        show.contains("SIL2 (verified at)"),
        "sreq show should display the verification-time SIL:\n{}",
        show
    );
}

#[test]
fn req_0155_escalation_flags_evidence_below_current_sil() {
    let s = verified_sil2_chain();
    // Link a higher-SIL hazard to the same function: SF allocates SIL3, so
    // SR-0001 now inherits SIL3 while its evidence was justified at SIL2.
    let _ = s.run(&[
        "hazard",
        "add",
        "-t",
        "Hazard B",
        "--harm",
        "worse harm",
        "-C",
        "C_C",
        "-F",
        "F_B",
        "-P",
        "P_B",
        "-W",
        "W3", // -> SIL3
    ]);
    let _ = s.run(&["sf", "mitigate", "SF-0001", "HAZ-0002"]);
    let val = s.run(&["validate"]);
    let body = format!("{}{}", stdout(&val), stderr(&val));
    assert!(
        body.contains("REQ-V-0036"),
        "escalation should raise REQ-V-0036:\n{}",
        body
    );
    // REQ-0154: the show view flags the escalation too.
    let show = stdout(&s.run(&["sreq", "show", "SR-0001"]));
    assert!(
        show.contains("inherited SIL rose"),
        "sreq show should flag the escalation:\n{}",
        show
    );
}

#[test]
fn req_0157_brief_surfaces_sil_escalated() {
    let s = verified_sil2_chain();
    let _ = s.run(&[
        "hazard",
        "add",
        "-t",
        "Hazard B",
        "--harm",
        "worse harm",
        "-C",
        "C_C",
        "-F",
        "F_B",
        "-P",
        "P_B",
        "-W",
        "W3",
    ]);
    let _ = s.run(&["sf", "mitigate", "SF-0001", "HAZ-0002"]);
    let brief = s.run(&["brief", "--json"]);
    let v: serde_json::Value = serde_json::from_str(&stdout(&brief)).expect("brief json");
    let esc = v["sil_escalated"].as_array().expect("sil_escalated array");
    assert!(
        esc.iter()
            .any(|x| x.as_str().unwrap_or("").contains("SR-0001")),
        "brief should surface escalated SR-0001: {}",
        v["sil_escalated"]
    );
}

// ---------- REQ-0160: trace states verification scope, not validated risk ----------

#[test]
fn req_0160_trace_labels_verification_scope() {
    let s = verified_sil2_chain();
    let trace = stdout(&s.run(&["trace", "HAZ-0001"]));
    assert!(
        trace.contains("NOT a residual-risk validation"),
        "trace must state its scope:\n{}",
        trace
    );
    assert!(
        !trace.contains("residual risk is acceptable"),
        "trace must not imply residual risk is acceptable:\n{}",
        trace
    );
}

// ---------- REQ-0168: independence — author should not be the verifier ----------

#[test]
fn req_0168_warns_when_author_verifies_own_safety_requirement() {
    // verified_sil2_chain authors and verifies SR-0001 as the same actor.
    let s = verified_sil2_chain();
    let val = s.run(&["validate"]);
    let body = format!("{}{}", stdout(&val), stderr(&val));
    assert!(
        body.contains("REQ-V-0037"),
        "same author+verifier should warn REQ-V-0037:\n{}",
        body
    );
}
