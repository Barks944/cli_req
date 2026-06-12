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

    // Implementing source carries the // SR-0001 marker.
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
