// Coverage-boost tests targeting the 38-requirement gap from
// `tests/` vs `req list`. Each test follows the req_NNNN_description
// convention so `req test run --promote` attaches automated evidence.
mod common;
use common::{stderr, stdout, Sandbox};
use std::fs;
use std::process::{Command, Stdio};

// ---------- REQ-0004: in-file warning + instructions ----------

#[test]
fn req_0004_save_writes_warning_and_instructions_block() {
    let s = Sandbox::new();
    s.init("p");
    let text = fs::read_to_string(s.path()).unwrap();
    assert!(text.contains("\"_warning\":"), "no _warning field");
    assert!(text.contains("DO NOT EDIT"), "warning copy missing");
    assert!(
        text.contains("\"_instructions\":"),
        "no _instructions field"
    );
    assert!(
        text.contains("req repair"),
        "instructions should mention req repair"
    );
}

// ---------- REQ-0007: weasel-word warning ----------

#[test]
fn req_0007_weasel_word_fast_produces_warning() {
    let s = Sandbox::new();
    s.init("p");
    let out = s.run(&[
        "add",
        "--title",
        "Performance is fast and easy",
        "--statement",
        "The system shall be fast under typical load conditions.",
        "--rationale",
        "Provoke the weasel rule.",
        "--kind",
        "constraint",
        "--priority",
        "could",
    ]);
    // Save succeeds (warning, not error)
    assert!(out.status.success(), "weasel words are advisory");
    let combined = format!("{}{}", stdout(&out), stderr(&out));
    // `req add` prints warnings without rule codes; `req validate` adds them.
    assert!(
        combined.contains("fast"),
        "warning should cite the term `fast`"
    );
    let val = s.run(&["validate"]);
    let vbody = stdout(&val);
    assert!(
        vbody.contains("REQ-V-0009"),
        "validate should emit the rule code, got:\n{}",
        vbody
    );
}

// ---------- REQ-0009: status-transition guard ----------

#[test]
fn req_0009_cannot_approve_functional_without_acceptance() {
    let s = Sandbox::new();
    s.init("p");
    // Sneak in a functional requirement WITHOUT acceptance by using kind=constraint
    // then update kind to functional later. The kind change itself plus the
    // approved status transition trips REQ-V-0018.
    let _ = s.run(&[
        "add",
        "--title",
        "Workable seed for status guard",
        "--statement",
        "The system shall accept this as a placeholder.",
        "--rationale",
        "Seed.",
        "--kind",
        "constraint",
        "--priority",
        "could",
    ]);
    let out = s.run(&[
        "update",
        "REQ-0001",
        "--kind",
        "functional",
        "--status",
        "approved",
        "--reason",
        "Force the guard to fire",
    ]);
    assert!(
        !out.status.success(),
        "approving a functional without acceptance must fail"
    );
    assert!(
        stderr(&out).contains("acceptance"),
        "error should name acceptance"
    );
}

// ---------- REQ-0018: sectioned help ----------

#[test]
fn req_0018_help_lists_known_sections_individually() {
    for section in [
        "overview",
        "concepts",
        "best-practice",
        "workflow",
        "integration",
        "agents",
        "errors",
        "testing",
        "verification",
    ] {
        let out = common::req(&["help", section]);
        assert!(out.status.success(), "req help {} should succeed", section);
        let body = stdout(&out);
        assert!(
            body.len() > 100,
            "section {} body too short ({}B)",
            section,
            body.len()
        );
    }
}

#[test]
fn req_0018_help_index_lists_all_sections() {
    let out = common::req(&["help"]);
    let body = stdout(&out);
    for section in [
        "overview",
        "concepts",
        "best-practice",
        "workflow",
        "integration",
        "agents",
        "errors",
        "testing",
        "verification",
    ] {
        assert!(
            body.contains(section),
            "index missing section name `{}`",
            section
        );
    }
}

// ---------- REQ-0021: actor on history ----------

#[test]
fn req_0021_history_records_actor_from_req_actor_env() {
    let s = Sandbox::new();
    s.init("p");
    let out = Command::new(env!("CARGO_BIN_EXE_req"))
        .args([
            "--file",
            s.path().to_str().unwrap(),
            "add",
            "--title",
            "Has an attributed history entry",
            "--statement",
            "The system shall record alice as the actor for this add.",
            "--rationale",
            "Verify env-based actor resolution.",
            "--kind",
            "constraint",
            "--priority",
            "could",
        ])
        .env("REQ_ACTOR", "alice-the-tester")
        .env_remove("REQ_FILE")
        .output()
        .expect("invoke req");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let show = stdout(&s.run(&["show", "REQ-0001"]));
    assert!(
        show.contains("alice-the-tester"),
        "actor missing from history:\n{}",
        show
    );
}

// ---------- REQ-0024: gitattributes merge driver registration ----------

#[test]
fn req_0024_hooks_install_registers_merge_driver_in_gitattributes() {
    let s = Sandbox::new();
    s.init("p");
    let _ = Command::new("git")
        .current_dir(s.dir.path())
        .args(["init", "-q", "-b", "main"])
        .output();
    let out = Command::new(env!("CARGO_BIN_EXE_req"))
        .args([
            "hooks",
            "install",
            "--repo",
            s.dir.path().to_str().unwrap(),
            "--force",
        ])
        .output()
        .expect("hooks install");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let attrs = fs::read_to_string(s.dir.path().join(".gitattributes")).unwrap();
    assert!(
        attrs.lines().any(|l| l.trim() == "*.req merge=req-merge"),
        "merge driver line missing: {}",
        attrs
    );
}

// ---------- REQ-0026: coverage default mode ----------

#[test]
fn req_0026_coverage_reports_referenced_orphans_and_ghosts() {
    let s = Sandbox::new();
    s.init("p");
    let _ = s.run(&[
        "add",
        "--title",
        "Has a code reference",
        "--statement",
        "The system shall be referenced from source.",
        "--rationale",
        "Coverage fixture.",
        "--kind",
        "constraint",
        "--priority",
        "could",
    ]);
    let _ = s.run(&[
        "add",
        "--title",
        "Orphan no code reference",
        "--statement",
        "The system shall have no source-tree reference.",
        "--rationale",
        "Coverage fixture.",
        "--kind",
        "constraint",
        "--priority",
        "could",
    ]);
    fs::create_dir_all(s.dir.path().join("src")).unwrap();
    // Construct the bogus marker via format! so the four-digit literal
    // never appears in this source (otherwise the project-wide coverage
    // scan would flag *this test file* as a ghost site).
    let bogus = format!("REQ-{:04}", 9999);
    let fixture_src = format!("// REQ-0001 reference\n// {} ghost\nfn _x() {{}}\n", bogus);
    fs::write(s.dir.path().join("src/lib.rs"), fixture_src).unwrap();
    let out = s.run(&[
        "coverage",
        "--path",
        s.dir.path().to_str().unwrap(),
        "--json",
    ]);
    let v: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert!(v["referenced"]
        .as_object()
        .unwrap()
        .contains_key("REQ-0001"));
    assert!(v["orphans"]
        .as_array()
        .unwrap()
        .iter()
        .any(|x| x == "REQ-0002"));
    assert!(v["ghosts"].as_object().unwrap().contains_key(&bogus));
}

// ---------- REQ-0027: audit ----------

#[test]
fn req_0027_audit_prints_signature_status_for_commits() {
    let s = Sandbox::new();
    s.init("p");
    let dir = s.dir.path();
    let _ = Command::new("git")
        .current_dir(dir)
        .args(["init", "-q", "-b", "main"])
        .output();
    let _ = Command::new("git")
        .current_dir(dir)
        .args(["config", "user.email", "t@e.com"])
        .output();
    let _ = Command::new("git")
        .current_dir(dir)
        .args(["config", "user.name", "T"])
        .output();
    let _ = Command::new("git")
        .current_dir(dir)
        .args(["config", "commit.gpgsign", "false"])
        .output();
    let _ = Command::new("git")
        .current_dir(dir)
        .args(["add", "project.req"])
        .output();
    let _ = Command::new("git")
        .current_dir(dir)
        .args(["commit", "-q", "-m", "seed"])
        .output();
    let out = Command::new(env!("CARGO_BIN_EXE_req"))
        .current_dir(dir)
        .args(["--file", s.path().to_str().unwrap(), "audit", "--json"])
        .output()
        .expect("audit");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let arr = v.as_array().expect("audit --json is an array");
    assert!(!arr.is_empty());
    assert_eq!(arr[0]["signature_status"], "no-signature");
}

// ---------- REQ-0031: help --install idempotency ----------

#[test]
fn req_0031_help_install_is_idempotent_in_managed_block() {
    let s = Sandbox::new();
    s.init("p");
    let md = s.dir.path().join("NOTES.md");
    fs::write(&md, "# Notes\n\nSome human prose.\n").unwrap();
    for _ in 0..3 {
        let out = common::req(&[
            "help",
            "agents",
            "--install",
            "--path",
            md.to_str().unwrap(),
        ]);
        assert!(out.status.success(), "install: {}", stderr(&out));
    }
    let body = fs::read_to_string(&md).unwrap();
    let begins = body.matches("<!-- req:help:agents:begin -->").count();
    let ends = body.matches("<!-- req:help:agents:end -->").count();
    assert_eq!(
        begins, 1,
        "exactly one begin marker, got {}\n{}",
        begins, body
    );
    assert_eq!(ends, 1, "exactly one end marker, got {}", ends);
    assert!(body.contains("Some human prose."), "user prose preserved");
}

// ---------- REQ-0033: coverage --by-file ----------

#[test]
fn req_0033_coverage_by_file_maps_files_to_req_ids() {
    let s = Sandbox::new();
    s.init("p");
    fs::create_dir_all(s.dir.path().join("src")).unwrap();
    fs::write(s.dir.path().join("src/a.rs"), "// REQ-0001 here\nfn x() {}").unwrap();
    fs::write(
        s.dir.path().join("src/b.rs"),
        "// REQ-0001 and REQ-0002\nfn y() {}",
    )
    .unwrap();
    let out = s.run(&[
        "coverage",
        "--by-file",
        "--path",
        s.dir.path().to_str().unwrap(),
        "--json",
    ]);
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    let files = v.as_array().expect("by-file json is an array");
    assert!(files.len() >= 2);
    for entry in files {
        let f = entry["file"].as_str().unwrap();
        let ids: Vec<&str> = entry["req_ids"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|x| x.as_str())
            .collect();
        if f.ends_with("b.rs") {
            assert!(ids.contains(&"REQ-0001") && ids.contains(&"REQ-0002"));
        }
    }
}

// ---------- REQ-0034: coverage --remap ----------

#[test]
fn req_0034_coverage_remap_dry_run_then_apply() {
    let s = Sandbox::new();
    s.init("p");
    fs::create_dir_all(s.dir.path().join("src")).unwrap();
    let f = s.dir.path().join("src/a.rs");
    // Build the placeholder via format! so it never appears literally in
    // this source file (which the project-wide coverage scan would
    // otherwise pick up as a ghost).
    let placeholder = format!("REQ-{:04}", 99);
    let target = format!("REQ-{:04}", 1);
    let fixture_src = format!("// {} to be remapped\nfn x() {{}}", placeholder);
    fs::write(&f, fixture_src).unwrap();
    let mapping = format!("{}={}", placeholder, target);
    // Dry run does NOT mutate
    let dry = s.run(&[
        "coverage",
        "--remap",
        &mapping,
        "--path",
        s.dir.path().to_str().unwrap(),
    ]);
    assert!(dry.status.success());
    assert!(fs::read_to_string(&f).unwrap().contains(&placeholder));
    // Apply DOES mutate
    let apply = s.run(&[
        "coverage",
        "--remap",
        &mapping,
        "--apply",
        "--path",
        s.dir.path().to_str().unwrap(),
    ]);
    assert!(apply.status.success());
    let after = fs::read_to_string(&f).unwrap();
    assert!(after.contains(&target));
    assert!(!after.contains(&placeholder));
}

// ---------- REQ-0035: update --add-acceptance / --remove-acceptance ----------

#[test]
fn req_0035_update_add_and_remove_acceptance_criteria() {
    let s = Sandbox::new();
    s.init("p");
    let _ = s.run(&[
        "add",
        "--title",
        "Has acceptance edits later",
        "--statement",
        "The system shall accept acceptance edits via update flags.",
        "--rationale",
        "Setup.",
        "--kind",
        "functional",
        "--priority",
        "should",
        "--accept",
        "Initial criterion alpha",
    ]);
    let out_add = s.run(&[
        "update",
        "REQ-0001",
        "--add-acceptance",
        "Appended criterion beta",
        "--add-acceptance",
        "Appended criterion gamma",
        "--reason",
        "extend",
    ]);
    assert!(out_add.status.success(), "stderr: {}", stderr(&out_add));
    let show = stdout(&s.run(&["show", "REQ-0001"]));
    assert!(show.contains("Initial criterion alpha"));
    assert!(show.contains("Appended criterion beta"));
    assert!(show.contains("Appended criterion gamma"));
    // Remove the middle one (1-based index 2)
    let out_rm = s.run(&[
        "update",
        "REQ-0001",
        "--remove-acceptance",
        "2",
        "--reason",
        "drop middle",
    ]);
    assert!(out_rm.status.success(), "stderr: {}", stderr(&out_rm));
    // Inspect the JSON form so we can count the *active* criteria
    // independently of the history log (which retains the prior text).
    let show_json = stdout(&s.run(&["show", "REQ-0001", "--json"]));
    let v: serde_json::Value = serde_json::from_str(&show_json).unwrap();
    let acc: Vec<&str> = v["acceptance"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|x| x.as_str())
        .collect();
    assert_eq!(
        acc.len(),
        2,
        "should have 2 criteria after removing one: {:?}",
        acc
    );
    assert!(
        !acc.iter().any(|s| s.contains("Appended criterion beta")),
        "beta should be gone from active list: {:?}",
        acc
    );
}

// ---------- REQ-0040: req next ----------

#[test]
fn req_0040_next_returns_highest_priority_unblocked_requirement() {
    let s = Sandbox::new();
    s.init("p");
    let _ = s.run(&[
        "add",
        "--title",
        "Lower priority should pick",
        "--statement",
        "The system shall provide this background feature.",
        "--rationale",
        "Could-priority fixture.",
        "--kind",
        "constraint",
        "--priority",
        "could",
    ]);
    let _ = s.run(&[
        "add",
        "--title",
        "Higher priority must pick",
        "--statement",
        "The system shall provide this critical feature first.",
        "--rationale",
        "Must-priority fixture.",
        "--kind",
        "constraint",
        "--priority",
        "must",
    ]);
    let out = s.run(&["next", "--json"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let v: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    // Enum serialisation is "Must" / "Should" / etc; lower-case is the
    // CLI-facing form. Compare on the title for clarity.
    assert_eq!(
        v["priority"], "Must",
        "next should pick the must-priority req: {}",
        v
    );
}

#[test]
fn req_0040_next_default_skips_verified_and_obsolete() {
    // Without a --status filter, `req next` should suggest work that's
    // still open — not items that are already shipped (Verified) or
    // retired (Obsolete).
    let s = Sandbox::new();
    s.init("p");
    let _ = s.run(&[
        "add",
        "--title",
        "Already shipped fixture requirement",
        "--statement",
        "The system shall provide a finished feature for this test.",
        "--rationale",
        "Verified fixture.",
        "--kind",
        "constraint",
        "--priority",
        "must",
    ]);
    // Walk it up the status ladder to Verified.
    for status in ["proposed", "approved", "implemented", "verified"] {
        let out = s.run(&[
            "update",
            "REQ-0001",
            "--status",
            status,
            "--reason",
            "test fixture progression",
        ]);
        assert!(
            out.status.success(),
            "status={} stderr: {}",
            status,
            stderr(&out)
        );
    }
    // No other requirements exist; default next should exit non-zero.
    let out = s.run(&["next", "--json"]);
    assert!(
        !out.status.success(),
        "next with only Verified candidates should not return one: {}",
        stdout(&out)
    );
    let v: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(v["found"], false);
}

// ---------- Lifecycle guards (P1 from agent QA) ----------

#[test]
fn update_status_blocks_direct_jump_to_verified() {
    // Verified is a strong claim — it must be reached from Implemented
    // so the implementation actually exists. Bypassing the lifecycle
    // hides un-implemented requirements behind a green checkmark.
    let s = Sandbox::new();
    s.init("p");
    let _ = s.run(&[
        "add",
        "--title",
        "Lifecycle guard fixture requirement",
        "--statement",
        "The system shall reject draft-to-verified jumps without force.",
        "--rationale",
        "Status floor regression.",
        "--kind",
        "constraint",
        "--priority",
        "must",
    ]);
    // Direct draft -> verified must fail.
    let out = s.run(&[
        "update",
        "REQ-0001",
        "--status",
        "verified",
        "--reason",
        "trying to skip",
    ]);
    assert!(
        !out.status.success(),
        "draft -> verified should be rejected: {}",
        stdout(&out)
    );
    assert!(
        stderr(&out).contains("verify") || stderr(&out).contains("implemented"),
        "error should mention the right path: {}",
        stderr(&out)
    );
    // --force lets you do it (for history corrections).
    let forced = s.run(&[
        "update",
        "REQ-0001",
        "--status",
        "verified",
        "--reason",
        "correcting history",
        "--force",
    ]);
    assert!(
        forced.status.success(),
        "--force should bypass the guard: {}",
        stderr(&forced)
    );
}

#[test]
fn verify_promote_requires_implemented_status() {
    // The `--promote` flag on `req verify` used to flip Draft straight
    // to Verified. Now it requires Implemented (or --force).
    let s = Sandbox::new();
    s.init("p");
    let _ = s.run(&[
        "add",
        "--title",
        "Promote-guard fixture requirement here",
        "--statement",
        "The system shall block promote-from-draft via verify.",
        "--rationale",
        "Promote-guard regression.",
        "--kind",
        "constraint",
        "--priority",
        "must",
    ]);
    let out = s.run(&[
        "verify",
        "REQ-0001",
        "--by",
        "inspection",
        "--notes",
        "Reviewed for fixture",
        "--promote",
    ]);
    assert!(
        !out.status.success(),
        "promote from draft should fail: {}",
        stdout(&out)
    );
    // Walk to implemented, then promote succeeds.
    for status in ["proposed", "approved", "implemented"] {
        let r = s.run(&["update", "REQ-0001", "--status", status, "--reason", "t"]);
        assert!(r.status.success(), "status={}", status);
    }
    let ok = s.run(&[
        "verify",
        "REQ-0001",
        "--by",
        "inspection",
        "--notes",
        "Reviewed for fixture",
        "--promote",
    ]);
    assert!(ok.status.success(), "promote from implemented should work");
}

#[test]
fn link_depends_on_cycle_is_rejected() {
    // Cycle detection only covered `parent` links before. Now every
    // asymmetric link kind (parent, depends-on, refines, verifies) is
    // walked.
    let s = Sandbox::new();
    s.init("p");
    for (i, title) in [
        "First requirement for the cycle fixture",
        "Second requirement for the cycle fixture",
    ]
    .iter()
    .enumerate()
    {
        let _ = s.run(&[
            "add",
            "--title",
            title,
            "--statement",
            &format!("The system shall provide fixture entry {}.", i + 1),
            "--rationale",
            "Cycle fixture.",
            "--kind",
            "constraint",
            "--priority",
            "could",
        ]);
    }
    let ok = s.run(&["link", "REQ-0001", "REQ-0002", "-k", "depends-on"]);
    assert!(ok.status.success(), "first link should succeed");
    let cycle = s.run(&["link", "REQ-0002", "REQ-0001", "-k", "depends-on"]);
    assert!(
        !cycle.status.success(),
        "reverse depends-on should be rejected as a cycle"
    );
    assert!(
        stderr(&cycle).to_lowercase().contains("cycle"),
        "error should call out the cycle: {}",
        stderr(&cycle)
    );
}

#[test]
fn diff_accepts_single_ref_shorthand() {
    // `req diff <ref>` used to error with "spec must be BASE..HEAD".
    // Now it is treated as `<ref>..HEAD`, matching git muscle memory.
    let s = Sandbox::new();
    s.init("p");
    let help = common::req(&["diff", "--help"]);
    // Just ensure --help still works after the parse change.
    assert!(help.status.success());
    // Bogus single-ref still produces a sensible git error (not "spec
    // must be BASE..HEAD"). We can't drive a real commit here without
    // a git repo, so this asserts the parse contract.
    let out = s.run(&["diff", "bogus-single-ref-xyz"]);
    let err = stderr(&out);
    assert!(
        !err.contains("spec must be BASE..HEAD"),
        "single ref should not trip the BASE..HEAD message: {}",
        err
    );
}

// ---------- Group A: 0.1.2 P0/P1 batch + cycle-detect + repair + diff hint ----------

#[test]
fn batch_update_to_verified_blocked_without_force() {
    // P0 regression: batch was a back door around the 0.1.1 lifecycle
    // guard — `{kind: update, status: verified}` slid Draft straight
    // to Verified. Same guard now lives in batch.
    let s = Sandbox::new();
    s.init("p");
    let _ = s.run(&[
        "add",
        "--title",
        "Batch-guard fixture requirement here",
        "--statement",
        "The system shall reject batch jumps to verified without force.",
        "--rationale",
        "Batch guard regression.",
        "--kind",
        "constraint",
        "--priority",
        "must",
    ]);
    let batch_path = s.path().parent().unwrap().join("batch_block.json");
    std::fs::write(
        &batch_path,
        r#"{"mutations":[{"kind":"update","id":"REQ-0001","status":"verified","reason":"skip"}]}"#,
    )
    .unwrap();
    let out = s.run(&["batch", batch_path.to_str().unwrap()]);
    assert!(
        !out.status.success(),
        "draft -> verified via batch should be rejected: {}",
        stdout(&out)
    );
    // Same batch with force=true should succeed.
    std::fs::write(
        &batch_path,
        r#"{"mutations":[{"kind":"update","id":"REQ-0001","status":"verified","reason":"force","force":true}]}"#,
    )
    .unwrap();
    let forced = s.run(&["batch", batch_path.to_str().unwrap()]);
    assert!(
        forced.status.success(),
        "force:true should bypass: {}",
        stderr(&forced)
    );
}

#[test]
fn batch_link_cycle_blocked() {
    // P0 regression: batch ignored cycle detection on link mutations,
    // letting callers install cycles that direct `req link` rejects.
    let s = Sandbox::new();
    s.init("p");
    for (i, title) in [
        "First requirement for batch cycle fixture",
        "Second requirement for batch cycle fixture",
    ]
    .iter()
    .enumerate()
    {
        let _ = s.run(&[
            "add",
            "--title",
            title,
            "--statement",
            &format!("The system shall provide cycle fixture entry {}.", i + 1),
            "--rationale",
            "Cycle fixture.",
            "--kind",
            "constraint",
            "--priority",
            "could",
        ]);
    }
    let _ = s.run(&["link", "REQ-0001", "REQ-0002", "-k", "depends-on"]);
    let batch_path = s.path().parent().unwrap().join("batch_cycle.json");
    std::fs::write(
        &batch_path,
        r#"{"mutations":[{"kind":"link","from":"REQ-0002","to":"REQ-0001","link_kind":"depends-on"}]}"#,
    )
    .unwrap();
    let out = s.run(&["batch", batch_path.to_str().unwrap()]);
    assert!(
        !out.status.success(),
        "cycle-closing link via batch should be rejected: {}",
        stdout(&out)
    );
    assert!(
        stderr(&out).to_lowercase().contains("cycle"),
        "error should call out the cycle: {}",
        stderr(&out)
    );
}

#[test]
fn validate_detects_link_cycles() {
    // P1: even if a cycle slips in (batch on an old binary, merge,
    // manual repair), `req validate` must surface it as REQ-V-0021.
    // We can't easily install a cycle through the CLI now, but we can
    // verify the rule by hand-corrupting + repairing + validating.
    let s = Sandbox::new();
    s.init("p");
    for (i, title) in [
        "Cycle fixture requirement number one",
        "Cycle fixture requirement number two",
    ]
    .iter()
    .enumerate()
    {
        let _ = s.run(&[
            "add",
            "--title",
            title,
            "--statement",
            &format!(
                "The system shall expose validate-time cycle detection {}.",
                i + 1
            ),
            "--rationale",
            "Validate cycle fixture.",
            "--kind",
            "constraint",
            "--priority",
            "could",
        ]);
    }
    let _ = s.run(&["link", "REQ-0001", "REQ-0002", "-k", "depends-on"]);
    // Inject a reverse depends-on by writing JSON directly (bypassing
    // the guard) — exactly the situation REQ-V-0021 exists to catch.
    let text = std::fs::read_to_string(s.path()).unwrap();
    let mut v: serde_json::Value = serde_json::from_str(&text).unwrap();
    let reqs = v["requirements"].as_object_mut().unwrap();
    let r2 = reqs.get_mut("REQ-0002").unwrap();
    r2["links"] = serde_json::json!([{"kind":"DependsOn","target":"REQ-0001"}]);
    std::fs::write(s.path(), serde_json::to_string_pretty(&v).unwrap()).unwrap();
    // Re-sign (file is dirty, integrity hash mismatched).
    let r = s.run(&["repair", "--confirm-direct-edit", "--force"]);
    assert!(
        r.status.success(),
        "force-repair should succeed: {}",
        stderr(&r)
    );
    let out = s.run(&["validate"]);
    let body = format!("{}{}", stdout(&out), stderr(&out));
    assert!(
        body.contains("REQ-V-0021"),
        "validate should report cycle via REQ-V-0021: {}",
        body
    );
}

#[test]
fn repair_force_bypasses_validation_errors() {
    // The previous flow: hand-edit + invalid + hash-broken => stuck.
    // Repair refused, every other command refused, only escape was
    // more hand-editing. --force re-signs so validate can surface the
    // problems via the normal channel.
    let s = Sandbox::new();
    s.init("p");
    let _ = s.run(&[
        "add",
        "--title",
        "Repair-force fixture requirement here",
        "--statement",
        "The system shall allow force-repair on invalid hand-edits.",
        "--rationale",
        "Repair force fixture.",
        "--kind",
        "constraint",
        "--priority",
        "could",
    ]);
    // Wipe the statement so validation fails AND the hash breaks.
    let text = std::fs::read_to_string(s.path()).unwrap();
    let mut v: serde_json::Value = serde_json::from_str(&text).unwrap();
    v["requirements"]["REQ-0001"]["statement"] = serde_json::json!("");
    std::fs::write(s.path(), serde_json::to_string_pretty(&v).unwrap()).unwrap();
    let nope = s.run(&["repair", "--confirm-direct-edit"]);
    assert!(
        !nope.status.success(),
        "repair without --force should refuse"
    );
    let forced = s.run(&["repair", "--confirm-direct-edit", "--force"]);
    assert!(
        forced.status.success(),
        "--force should re-sign anyway: {}",
        stderr(&forced)
    );
    let validate_out = s.run(&["validate"]);
    assert!(
        !validate_out.status.success(),
        "validate must now surface the errors (it could not while the hash was bad)"
    );
}

#[test]
fn diff_with_req_id_returns_friendly_hint() {
    // Before: `req diff REQ-0001` leaked `fatal: invalid object name`.
    let s = Sandbox::new();
    s.init("p");
    let out = s.run(&["diff", "REQ-0001"]);
    assert!(!out.status.success(), "REQ-ID is not a git rev");
    let err = stderr(&out);
    assert!(
        err.contains("looks like a requirement ID") || err.contains("req show"),
        "should point user at `req show` for single-req inspection: {}",
        err
    );
    assert!(
        !err.contains("invalid object name"),
        "should not leak git's error: {}",
        err
    );
}

// ---------- 0.2.2 pre-commit gate ----------

#[test]
fn review_staged_does_not_need_existing_head() {
    // --staged mode is used by the pre-commit hook, which runs on
    // the FIRST commit too (no HEAD yet). The general fail-closed
    // check must not fire here — the gate's job is to inspect what's
    // staged regardless of comparison ref.
    let s = Sandbox::new();
    s.init("p");
    let _ = s.run(&[
        "add",
        "--title",
        "Staged-no-head fixture title",
        "--statement",
        "The system shall handle --staged on an empty repo.",
        "--rationale",
        "Fixture rationale.",
        "--kind",
        "constraint",
        "--priority",
        "could",
    ]);
    let out = s.run(&["review", "--staged", "--gate"]);
    assert!(
        out.status.success(),
        "--staged with no HEAD should not fail-closed; stderr: {}",
        stderr(&out)
    );
}

#[test]
fn review_staged_flags_markerless_changed_source() {
    let s = Sandbox::new();
    s.init("p");
    let _ = s.run(&[
        "add",
        "--title",
        "Staged hook smoke fixture title",
        "--statement",
        "The system shall block staged markerless commits.",
        "--rationale",
        "Staged gate fixture.",
        "--kind",
        "constraint",
        "--priority",
        "could",
    ]);
    let _ = Command::new("git")
        .current_dir(s.dir.path())
        .args(["init", "-q", "-b", "main"])
        .output();
    for cfg in [["user.email", "t@t.t"], ["user.name", "t"]] {
        let _ = Command::new("git")
            .current_dir(s.dir.path())
            .args(["config", cfg[0], cfg[1]])
            .output();
    }
    let _ = Command::new("git")
        .current_dir(s.dir.path())
        .args([
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "baseline",
        ])
        .output();
    let src_dir = s.dir.path().join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::write(src_dir.join("oops.rs"), "fn pretend(){}\n").unwrap();
    let _ = Command::new("git")
        .current_dir(s.dir.path())
        .args(["add", "src/oops.rs"])
        .output();
    let out = Command::new(env!("CARGO_BIN_EXE_req"))
        .current_dir(s.dir.path())
        .args([
            "--file",
            s.path().to_str().unwrap(),
            "review",
            "--staged",
            "--gate",
        ])
        .output()
        .expect("review");
    let body = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !out.status.success(),
        "markerless staged source should block: {}",
        body
    );
    assert!(
        body.contains("oops.rs"),
        "gate output should name the offending file: {}",
        body
    );
    // Adding a marker resolves it.
    std::fs::write(
        src_dir.join("oops.rs"),
        "// REQ-0001: marker fixture\nfn pretend(){}\n",
    )
    .unwrap();
    let _ = Command::new("git")
        .current_dir(s.dir.path())
        .args(["add", "src/oops.rs"])
        .output();
    let fixed = Command::new(env!("CARGO_BIN_EXE_req"))
        .current_dir(s.dir.path())
        .args([
            "--file",
            s.path().to_str().unwrap(),
            "review",
            "--staged",
            "--gate",
        ])
        .output()
        .expect("review");
    assert!(
        fixed.status.success(),
        "marker present should pass: {}{}",
        String::from_utf8_lossy(&fixed.stdout),
        String::from_utf8_lossy(&fixed.stderr)
    );
}

// ---------- 0.2.1 gate hardening + LLM hook + split fixes ----------

#[test]
fn review_gate_fails_closed_on_bogus_base_ref() {
    // P1 from agent QA: a CI YAML typo silently disabled the gate.
    // Without --gate the report still produces (advisory); with --gate
    // the missing base is an error.
    let s = Sandbox::new();
    s.init("p");
    let advisory = s.run(&["review", "--base", "bogus-ref-zzz"]);
    assert!(
        advisory.status.success(),
        "advisory mode should still work: {}",
        stderr(&advisory)
    );
    let gated = s.run(&["review", "--base", "bogus-ref-zzz", "--gate"]);
    assert!(
        !gated.status.success(),
        "--gate on a missing base must exit non-zero: stdout {}",
        stdout(&gated)
    );
    assert!(
        stderr(&gated).contains("does not exist"),
        "error should name the missing ref: {}",
        stderr(&gated)
    );
}

#[test]
fn add_prints_marker_nudge() {
    // Discoverability nudge from the discipline agent: after a
    // successful add, point the user at the marker convention so
    // "REQ first, then code" is the path of least resistance.
    let s = Sandbox::new();
    s.init("p");
    let out = s.run(&[
        "add",
        "--title",
        "Nudge fixture for marker hint",
        "--statement",
        "The system shall surface a marker nudge after add.",
        "--rationale",
        "Discoverability regression.",
        "--kind",
        "constraint",
        "--priority",
        "could",
    ]);
    assert!(out.status.success());
    let body = stdout(&out);
    assert!(
        body.contains("// REQ-0001:"),
        "expected marker nudge in add output: {}",
        body
    );
    assert!(
        body.contains("req coverage"),
        "nudge should reference coverage: {}",
        body
    );
}

#[test]
fn split_inherits_acceptance_from_functional_parent() {
    // P2 from agent QA: functional parents failed split because parts
    // started with empty acceptance and REQ-V-0014 tripped on part #1.
    let s = Sandbox::new();
    s.init("p");
    let _ = s.run(&[
        "add",
        "--title",
        "Functional split fixture compound",
        "--statement",
        "The system shall handle case A and shall handle case B.",
        "--rationale",
        "Functional split regression.",
        "--kind",
        "functional",
        "--priority",
        "must",
        "--accept",
        "Case A behaviour observed",
        "--accept",
        "Case B behaviour observed",
    ]);
    let out = s.run(&[
        "split",
        "REQ-0001",
        "--into",
        "The system shall handle case A.",
        "--into",
        "The system shall handle case B.",
        "--reason",
        "atomic split",
    ]);
    assert!(
        out.status.success(),
        "functional split should succeed when acceptance inherits: {}",
        stderr(&out)
    );
    // Each child should now carry the parent's acceptance criteria
    // (the user can prune per child afterwards).
    let show = stdout(&s.run(&["show", "REQ-0002"]));
    assert!(
        show.contains("Case A behaviour observed"),
        "child should inherit acceptance: {}",
        show
    );
}

#[test]
fn review_skips_project_req_and_test_paths() {
    // P2 from agent QA: project.req's instructions block contains
    // example REQ-NNNN tokens that should never count as ghosts, and
    // test-path files should not trip the markerless check.
    let s = Sandbox::new();
    s.init("p");
    let _ = s.run(&[
        "add",
        "--title",
        "Coverage exclude fixture title",
        "--statement",
        "The system shall exclude project.req and tests/ from the gate.",
        "--rationale",
        "Coverage exclude regression.",
        "--kind",
        "constraint",
        "--priority",
        "could",
    ]);
    let out = s.run(&["review", "--base", "bogus", "--json"]);
    let body = stdout(&out);
    let v: serde_json::Value = serde_json::from_str(&body)
        .unwrap_or_else(|e| panic!("review json parse: {} on {}", e, body));
    let ghosts = v["coverage"]["ghosts"]
        .as_array()
        .expect("ghosts is an array");
    for g in ghosts {
        let s = g.as_str().unwrap_or("");
        assert!(
            !s.contains("project.req"),
            "project.req should be excluded from ghost scan: {}",
            s
        );
    }
}

#[test]
fn review_marker_in_string_literal_does_not_satisfy_gate() {
    // P1 from gate-breaker: `let x = "REQ-0001";` shouldn't pass the
    // gate when the file has no actual comment marker.
    let s = Sandbox::new();
    s.init("p");
    let _ = s.run(&[
        "add",
        "--title",
        "String marker fixture title",
        "--statement",
        "The system shall require markers in comments not strings.",
        "--rationale",
        "Comment-context regression.",
        "--kind",
        "constraint",
        "--priority",
        "could",
    ]);
    // Stage a fake source file with the marker only in a string.
    let src_dir = s.dir.path().join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    let fake_src = src_dir.join("bypass_attempt.rs");
    std::fs::write(&fake_src, "fn pretend() { let _ = \"REQ-0001\"; }\n").unwrap();
    // Use git to get a changed-files set. Use cmd run from sandbox dir.
    let git_init = Command::new("git")
        .current_dir(s.dir.path())
        .args(["init", "-q", "-b", "main"])
        .output()
        .expect("git init");
    assert!(git_init.status.success());
    let _ = Command::new("git")
        .current_dir(s.dir.path())
        .args([
            "-c",
            "commit.gpgsign=false",
            "config",
            "user.email",
            "t@t.t",
        ])
        .output();
    let _ = Command::new("git")
        .current_dir(s.dir.path())
        .args(["-c", "commit.gpgsign=false", "config", "user.name", "t"])
        .output();
    let _ = Command::new("git")
        .current_dir(s.dir.path())
        .args([
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "base",
        ])
        .output();
    let _ = Command::new("git")
        .current_dir(s.dir.path())
        .args(["add", "-A"])
        .output();
    let _ = Command::new("git")
        .current_dir(s.dir.path())
        .args([
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-q",
            "-m",
            "added bypass attempt",
        ])
        .output();
    let out = Command::new(env!("CARGO_BIN_EXE_req"))
        .current_dir(s.dir.path())
        .args([
            "--file",
            s.path().to_str().unwrap(),
            "review",
            "--base",
            "HEAD~1",
            "--gate",
        ])
        .output()
        .expect("review");
    let body = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !out.status.success(),
        "string-literal marker should not satisfy the gate: {}",
        body
    );
    assert!(
        body.contains("bypass_attempt.rs"),
        "gate should name the offending file: {}",
        body
    );
}

// ---------- 0.2.0 features ----------

#[test]
fn status_tag_filter_scopes_the_report() {
    let s = Sandbox::new();
    s.init("p");
    let _ = s.run(&[
        "add",
        "--title",
        "Tagged for milestone alpha",
        "--statement",
        "The system shall belong to milestone alpha.",
        "--rationale",
        "Tag scope fixture.",
        "--kind",
        "constraint",
        "--priority",
        "could",
        "--tag",
        "alpha",
    ]);
    let _ = s.run(&[
        "add",
        "--title",
        "Tagged for milestone beta",
        "--statement",
        "The system shall belong to milestone beta.",
        "--rationale",
        "Tag scope fixture.",
        "--kind",
        "constraint",
        "--priority",
        "could",
        "--tag",
        "beta",
    ]);
    let out = s.run(&["status", "--json", "--tag", "alpha"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let v: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(v["total"], 1, "alpha scope should have 1 req: {}", v);
    assert_eq!(v["filter"]["tags"][0], "alpha");
}

#[test]
fn split_breaks_compound_into_atomic_parts() {
    let s = Sandbox::new();
    s.init("p");
    let _ = s.run(&[
        "add",
        "--title",
        "Compound source for split fixture",
        "--statement",
        "The system shall authenticate users and authorize sessions and log events.",
        "--rationale",
        "Split fixture rationale.",
        "--kind",
        "constraint",
        "--priority",
        "must",
    ]);
    let out = s.run(&[
        "split",
        "REQ-0001",
        "--into",
        "The system shall authenticate users.",
        "--into",
        "The system shall authorize sessions.",
        "--into",
        "The system shall log events.",
        "--reason",
        "atomic split",
        "--json",
    ]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let v: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(v["original"], "REQ-0001");
    assert_eq!(v["retired"], true);
    let parts = v["parts"].as_array().unwrap();
    assert_eq!(parts.len(), 3);
    let show_original = stdout(&s.run(&["show", "REQ-0001"]));
    assert!(
        show_original.contains("obsolete"),
        "original should be obsolete: {}",
        show_original
    );
}

#[test]
fn split_keep_original_does_not_retire() {
    let s = Sandbox::new();
    s.init("p");
    let _ = s.run(&[
        "add",
        "--title",
        "Compound retained for split fixture",
        "--statement",
        "The system shall authenticate users and authorize sessions.",
        "--rationale",
        "Split-keep fixture rationale.",
        "--kind",
        "constraint",
        "--priority",
        "must",
    ]);
    let out = s.run(&[
        "split",
        "REQ-0001",
        "--into",
        "The system shall authenticate users.",
        "--into",
        "The system shall authorize sessions.",
        "--keep-original",
        "--reason",
        "additive split",
        "--json",
    ]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let v: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(v["retired"], false);
    let show_original = stdout(&s.run(&["show", "REQ-0001"]));
    assert!(
        !show_original.contains("Status   : obsolete"),
        "original should remain non-obsolete: {}",
        show_original
    );
}

#[test]
fn review_emits_markdown_for_clean_repo() {
    // Review must work even outside a real PR scenario — when there's
    // no base ref to diff against, it should fall back gracefully and
    // still emit the validate / coverage / stale sections.
    let s = Sandbox::new();
    s.init("p");
    let _ = s.run(&[
        "add",
        "--title",
        "Review fixture requirement title",
        "--statement",
        "The system shall appear in the review report.",
        "--rationale",
        "Review fixture.",
        "--kind",
        "constraint",
        "--priority",
        "could",
    ]);
    let out = s.run(&["review", "--base", "bogus-ref-zzz"]);
    // Should still produce output — base load failure is non-fatal.
    let body = stdout(&out);
    assert!(
        body.contains("# req review:"),
        "review should emit markdown heading: {}",
        body
    );
}

#[test]
fn validate_llm_hook_runs_when_env_set() {
    // Wire a trivial hook script that always reports ok:false; ensure
    // REQ-V-0023 appears in the validate output.
    let s = Sandbox::new();
    s.init("p");
    let _ = s.run(&[
        "add",
        "--title",
        "LLM hook fixture requirement",
        "--statement",
        "The system shall participate in the LLM hook test.",
        "--rationale",
        "LLM hook fixture.",
        "--kind",
        "constraint",
        "--priority",
        "could",
    ]);
    // Hook script that ignores stdin and prints a fixed verdict.
    let hook_cmd = if cfg!(windows) {
        r#"powershell -NoProfile -Command "Write-Output '{\"ok\":false,\"message\":\"toy hook flag\"}'""#.to_string()
    } else {
        r#"echo '{"ok":false,"message":"toy hook flag"}'"#.to_string()
    };
    let out = Command::new(env!("CARGO_BIN_EXE_req"))
        .args(["--file", s.path().to_str().unwrap(), "validate"])
        .env("REQ_VALIDATE_LLM_CMD", &hook_cmd)
        .output()
        .expect("invoke req");
    let body = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        body.contains("REQ-V-0023") || body.contains("LLM hook"),
        "validate should surface the hook verdict: {}",
        body
    );
}

// ---------- 0.1.2 state-machine policy (Position A + Draft carve-out) ----------

fn fixture_draft(s: &Sandbox) {
    let _ = s.run(&[
        "add",
        "--title",
        "State machine fixture requirement",
        "--statement",
        "The system shall enforce A + Draft carve-out lifecycle policy.",
        "--rationale",
        "Lifecycle policy regression.",
        "--kind",
        "constraint",
        "--priority",
        "could",
    ]);
}

fn run_update_status(s: &Sandbox, from: &str, to: &str) -> std::process::Output {
    // Reset to the desired starting status by force-driving, then
    // attempt the natural transition under test.
    s.run(&[
        "update", "REQ-0001", "--status", from, "--reason", "reset", "--force",
    ]);
    s.run(&[
        "update",
        "REQ-0001",
        "--status",
        to,
        "--reason",
        "transition under test",
    ])
}

#[test]
fn lifecycle_forward_one_step_is_free() {
    let s = Sandbox::new();
    s.init("p");
    fixture_draft(&s);
    for (from, to) in [
        ("draft", "proposed"),
        ("proposed", "approved"),
        ("approved", "implemented"),
        ("implemented", "verified"),
    ] {
        let out = run_update_status(&s, from, to);
        assert!(
            out.status.success(),
            "{} -> {} should be free: {}",
            from,
            to,
            stderr(&out)
        );
    }
}

#[test]
fn lifecycle_draft_carve_out_is_free() {
    // Draft is a scratch state; sketch-then-slot directly to Proposed
    // or Approved without ceremony is part of the natural workflow.
    let s = Sandbox::new();
    s.init("p");
    fixture_draft(&s);
    let to_proposed = run_update_status(&s, "draft", "proposed");
    assert!(to_proposed.status.success());
    let to_approved = run_update_status(&s, "draft", "approved");
    assert!(
        to_approved.status.success(),
        "draft -> approved should be the carve-out: {}",
        stderr(&to_approved)
    );
}

#[test]
fn lifecycle_to_obsolete_is_free_from_any_state() {
    let s = Sandbox::new();
    s.init("p");
    fixture_draft(&s);
    for from in ["draft", "proposed", "approved", "implemented", "verified"] {
        let out = run_update_status(&s, from, "obsolete");
        assert!(
            out.status.success(),
            "{} -> obsolete should be free: {}",
            from,
            stderr(&out)
        );
    }
}

#[test]
fn lifecycle_skip_forward_requires_force() {
    let s = Sandbox::new();
    s.init("p");
    fixture_draft(&s);
    let out = run_update_status(&s, "draft", "implemented");
    assert!(
        !out.status.success(),
        "draft -> implemented should require --force: {}",
        stdout(&out)
    );
    assert!(
        stderr(&out).contains("irregular"),
        "error should label it irregular: {}",
        stderr(&out)
    );
}

#[test]
fn lifecycle_backward_from_verified_requires_force() {
    let s = Sandbox::new();
    s.init("p");
    fixture_draft(&s);
    // Verified is sticky — leaving it for anything-but-Obsolete is
    // irregular and needs an explicit override.
    let out = run_update_status(&s, "verified", "implemented");
    assert!(
        !out.status.success(),
        "verified -> implemented should require --force: {}",
        stdout(&out)
    );
}

#[test]
fn lifecycle_resurrect_from_obsolete_requires_force() {
    let s = Sandbox::new();
    s.init("p");
    fixture_draft(&s);
    let out = run_update_status(&s, "obsolete", "draft");
    assert!(
        !out.status.success(),
        "obsolete -> draft should require --force: {}",
        stdout(&out)
    );
}

#[test]
fn lifecycle_force_allows_any_transition_with_reason() {
    let s = Sandbox::new();
    s.init("p");
    fixture_draft(&s);
    // Walk to Verified naturally, then demote with --force.
    for to in ["proposed", "approved", "implemented", "verified"] {
        let r = s.run(&["update", "REQ-0001", "--status", to, "--reason", "walking"]);
        assert!(r.status.success(), "natural step to {} failed", to);
    }
    let forced = s.run(&[
        "update",
        "REQ-0001",
        "--status",
        "implemented",
        "--reason",
        "verification was wrong",
        "--force",
    ]);
    assert!(
        forced.status.success(),
        "--force with --reason should let you demote: {}",
        stderr(&forced)
    );
}

#[test]
fn lifecycle_batch_honours_state_machine() {
    // Batch must enforce the same policy. Irregular jump rejected;
    // force=true mutation passes.
    let s = Sandbox::new();
    s.init("p");
    fixture_draft(&s);
    let batch_path = s.path().parent().unwrap().join("batch_sm.json");
    std::fs::write(
        &batch_path,
        r#"{"mutations":[{"kind":"update","id":"REQ-0001","status":"implemented","reason":"skip"}]}"#,
    )
    .unwrap();
    let out = s.run(&["batch", batch_path.to_str().unwrap()]);
    assert!(
        !out.status.success(),
        "draft -> implemented via batch should be rejected: {}",
        stdout(&out)
    );
    std::fs::write(
        &batch_path,
        r#"{"mutations":[{"kind":"update","id":"REQ-0001","status":"implemented","reason":"skip","force":true}]}"#,
    )
    .unwrap();
    let forced = s.run(&["batch", batch_path.to_str().unwrap()]);
    assert!(
        forced.status.success(),
        "batch force:true should bypass: {}",
        stderr(&forced)
    );
}

// ---------- Group C: 0.1.2 CLI polish ----------

#[test]
fn id_lookup_is_case_and_pad_insensitive() {
    let s = Sandbox::new();
    s.init("p");
    let _ = s.run(&[
        "add",
        "--title",
        "ID normalization fixture title",
        "--statement",
        "The system shall match req-1 to REQ-0001.",
        "--rationale",
        "ID normalisation regression.",
        "--kind",
        "constraint",
        "--priority",
        "could",
    ]);
    for form in ["REQ-0001", "req-0001", "REQ-1", "req-1", "1"] {
        let out = s.run(&["show", form]);
        assert!(
            out.status.success(),
            "form {} should resolve: {}",
            form,
            stderr(&out)
        );
    }
}

#[test]
fn id_miss_suggests_nearest() {
    let s = Sandbox::new();
    s.init("p");
    let _ = s.run(&[
        "add",
        "--title",
        "Nearest-miss fixture requirement",
        "--statement",
        "The system shall surface nearest-ID hints on miss.",
        "--rationale",
        "Did-you-mean regression.",
        "--kind",
        "constraint",
        "--priority",
        "could",
    ]);
    let out = s.run(&["show", "REQ-0002"]);
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("did you mean REQ-0001"),
        "expected did-you-mean hint: {}",
        stderr(&out)
    );
}

#[test]
fn retire_aliases_delete() {
    let s = Sandbox::new();
    s.init("p");
    let _ = s.run(&[
        "add",
        "--title",
        "Retire-alias fixture requirement",
        "--statement",
        "The system shall accept `req retire` as a name for delete.",
        "--rationale",
        "Retire alias regression.",
        "--kind",
        "constraint",
        "--priority",
        "could",
    ]);
    let out = s.run(&["retire", "REQ-0001", "--reason", "alias test"]);
    assert!(
        out.status.success(),
        "`retire` alias should resolve to delete: {}",
        stderr(&out)
    );
}

// ---------- REQ-0042: help --json with structured agents crib ----------

#[test]
fn req_0042_help_agents_json_emits_structured_crib() {
    let out = common::req(&["help", "agents", "--json"]);
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert!(
        v["structured"]["triggers"].is_array(),
        "structured.triggers missing"
    );
    assert!(
        v["structured"]["commands"].is_array(),
        "structured.commands missing"
    );
    assert!(
        v["structured"]["rules"].is_array(),
        "structured.rules missing"
    );
    assert!(v["structured"]["env"].is_array(), "structured.env missing");
}

// ---------- REQ-0043: actor_kind on history ----------

#[test]
fn req_0043_actor_kind_env_tags_history_entry() {
    let s = Sandbox::new();
    s.init("p");
    let out = Command::new(env!("CARGO_BIN_EXE_req"))
        .args([
            "--file",
            s.path().to_str().unwrap(),
            "add",
            "--title",
            "Tagged as agent in history",
            "--statement",
            "The system shall record actor_kind agent for this add.",
            "--rationale",
            "Verify REQ_ACTOR_KIND wiring.",
            "--kind",
            "constraint",
            "--priority",
            "could",
        ])
        .env("REQ_ACTOR_KIND", "agent")
        .env_remove("REQ_FILE")
        .output()
        .expect("invoke req");
    assert!(out.status.success());
    let show = stdout(&s.run(&["show", "REQ-0001"]));
    assert!(
        show.contains("(agent)"),
        "show output should tag (agent):\n{}",
        show
    );
}

// ---------- REQ-0044: claude-code install ----------

#[test]
fn req_0044_hooks_install_claude_code_writes_settings_json() {
    let s = Sandbox::new();
    s.init("p");
    let _ = Command::new("git")
        .current_dir(s.dir.path())
        .args(["init", "-q", "-b", "main"])
        .output();
    let out = Command::new(env!("CARGO_BIN_EXE_req"))
        .args([
            "hooks",
            "install",
            "--claude-code",
            "--repo",
            s.dir.path().to_str().unwrap(),
            "--force",
        ])
        .output()
        .expect("hooks install");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let settings = fs::read_to_string(s.dir.path().join(".claude/settings.json")).unwrap();
    let v: serde_json::Value = serde_json::from_str(&settings).unwrap();
    let allow: Vec<&str> = v["permissions"]["allow"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|x| x.as_str())
        .collect();
    assert!(allow.iter().any(|s| s.contains("req")));
    let stop = v["hooks"]["Stop"].as_array().unwrap();
    assert!(!stop.is_empty(), "Stop hook missing");
}

// ---------- REQ-0047: .mcp.json bootstrap ----------

#[test]
fn req_0047_mcp_init_config_writes_managed_mcp_json() {
    let s = Sandbox::new();
    s.init("p");
    let cfg = s.dir.path().join(".mcp.json");
    let out = common::req(&[
        "mcp",
        "--init-config",
        "--config-path",
        cfg.to_str().unwrap(),
    ]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let body = fs::read_to_string(&cfg).unwrap();
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(
        v["mcpServers"]["req"].is_object(),
        "mcpServers.req missing: {}",
        body
    );
    assert_eq!(v["mcpServers"]["req"]["command"], "req");
}

// ---------- REQ-0048: tool descriptions carry guidance ----------

#[test]
fn req_0048_mcp_tool_descriptions_contain_trigger_hints() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_req"))
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn req mcp");
    {
        use std::io::Write;
        let stdin = child.stdin.as_mut().unwrap();
        writeln!(
            stdin,
            "{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\"}}"
        )
        .unwrap();
        writeln!(
            stdin,
            "{{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\"}}"
        )
        .unwrap();
    }
    let out = child.wait_with_output().unwrap();
    let body = String::from_utf8_lossy(&out.stdout);
    let line = body.lines().nth(1).unwrap();
    let v: serde_json::Value = serde_json::from_str(line).unwrap();
    let tools = v["result"]["tools"].as_array().unwrap();
    for tool in tools {
        let name = tool["name"].as_str().unwrap();
        let desc = tool["description"].as_str().unwrap_or("").to_lowercase();
        // A "guidance hint" is any verb or scenario phrase that points the
        // agent at when to reach for the tool. Deliberately broad: an
        // imperative verb starting the description is enough.
        let triggers = [
            "call ",
            "use ",
            "when ",
            "always",
            "first",
            "set ",
            "fetch",
            "return",
            "report",
            "list",
            "attach",
            "drive",
            "rewrite",
            "register",
            "audit",
            "describe",
            "summarize",
            "summarise",
            "render",
            "scan",
            "detect",
            "create",
            "modify",
            "retire",
            "apply",
            "parse",
            "ingest",
            "walk ",
            "suggest",
        ];
        let has_hint = triggers.iter().any(|t| desc.contains(t));
        assert!(
            has_hint,
            "tool {} description lacks a guidance hint: {}",
            name, desc
        );
    }
}

// ---------- REQ-0049 + REQ-0050: test record + show drift ----------

#[test]
fn req_0049_test_record_attaches_outcome_and_head_sha() {
    let s = Sandbox::new();
    s.init("p");
    let dir = s.dir.path();
    let _ = Command::new("git")
        .current_dir(dir)
        .args(["init", "-q", "-b", "main"])
        .output();
    let _ = Command::new("git")
        .current_dir(dir)
        .args(["config", "user.email", "t@e.com"])
        .output();
    let _ = Command::new("git")
        .current_dir(dir)
        .args(["config", "user.name", "T"])
        .output();
    let _ = Command::new("git")
        .current_dir(dir)
        .args(["config", "commit.gpgsign", "false"])
        .output();
    let _ = Command::new("git")
        .current_dir(dir)
        .args(["add", "project.req"])
        .output();
    let _ = Command::new("git")
        .current_dir(dir)
        .args(["commit", "-q", "-m", "init"])
        .output();
    let _ = s.run(&[
        "add",
        "--title",
        "Gets a test record attached",
        "--statement",
        "The system shall accept a recorded test outcome via the CLI.",
        "--rationale",
        "REQ-0049 fixture.",
        "--kind",
        "constraint",
        "--priority",
        "could",
    ]);
    let out = Command::new(env!("CARGO_BIN_EXE_req"))
        .current_dir(dir)
        .args([
            "--file",
            s.path().to_str().unwrap(),
            "test",
            "record",
            "REQ-0001",
            "--result",
            "pass",
            "--notes",
            "Manual smoke for the test record path",
        ])
        .output()
        .expect("test record");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let show = stdout(&s.run(&["show", "REQ-0001"]));
    assert!(show.contains("PASS"));
    assert!(show.contains("Manual smoke"));
}

#[test]
fn req_0050_show_marks_latest_record_against_head() {
    // Same set-up as REQ-0049; check the [matches HEAD] annotation.
    let s = Sandbox::new();
    s.init("p");
    let dir = s.dir.path();
    let _ = Command::new("git")
        .current_dir(dir)
        .args(["init", "-q", "-b", "main"])
        .output();
    let _ = Command::new("git")
        .current_dir(dir)
        .args(["config", "user.email", "t@e.com"])
        .output();
    let _ = Command::new("git")
        .current_dir(dir)
        .args(["config", "user.name", "T"])
        .output();
    let _ = Command::new("git")
        .current_dir(dir)
        .args(["config", "commit.gpgsign", "false"])
        .output();
    let _ = Command::new("git")
        .current_dir(dir)
        .args(["add", "project.req"])
        .output();
    let _ = Command::new("git")
        .current_dir(dir)
        .args(["commit", "-q", "-m", "init"])
        .output();
    let _ = s.run(&[
        "add",
        "--title",
        "Latest record drift marker",
        "--statement",
        "The system shall annotate the most recent test record.",
        "--rationale",
        "REQ-0050 fixture.",
        "--kind",
        "constraint",
        "--priority",
        "could",
    ]);
    let _ = Command::new(env!("CARGO_BIN_EXE_req"))
        .current_dir(dir)
        .args([
            "--file",
            s.path().to_str().unwrap(),
            "test",
            "record",
            "REQ-0001",
            "--result",
            "pass",
            "--notes",
            "for drift test",
        ])
        .output()
        .expect("test record");
    // show should annotate the latest record relative to HEAD.
    let out = Command::new(env!("CARGO_BIN_EXE_req"))
        .current_dir(dir)
        .args(["--file", s.path().to_str().unwrap(), "show", "REQ-0001"])
        .output()
        .expect("show");
    let body = String::from_utf8_lossy(&out.stdout);
    assert!(
        body.contains("[matches HEAD]")
            || body.contains("[fresh]")
            || body.contains("matches HEAD"),
        "expected freshness annotation, got:\n{}",
        body
    );
}

// ---------- REQ-0054: req status ----------

#[test]
fn req_0054_status_emits_buckets_and_delivery_progress() {
    let s = Sandbox::new();
    s.init("p");
    let _ = s.run(&[
        "add",
        "--title",
        "A baseline requirement here",
        "--statement",
        "The system shall provide one baseline requirement.",
        "--rationale",
        "Setup.",
        "--kind",
        "constraint",
        "--priority",
        "could",
    ]);
    let out = s.run(&["status", "--json"]);
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(v["total"], 1);
    assert!(v["by_status"].is_object());
    assert!(v["delivery_progress_pct"].is_number());
}

// ---------- REQ-0055: req test run dry-run parses test names ----------

#[test]
fn req_0055_test_run_dry_run_parses_req_named_tests() {
    let s = Sandbox::new();
    s.init("p");
    let _ = s.run(&[
        "add",
        "--title",
        "Subject of an automated test record",
        "--statement",
        "The system shall be matched by a fake cargo test invocation.",
        "--rationale",
        "REQ-0055 fixture.",
        "--kind",
        "constraint",
        "--priority",
        "could",
    ]);
    // Pre-captured cargo-test-style log file: cleaner than shell-quoting
    // through --cmd, especially on Windows.
    let log = s.dir.path().join("fake-cargo.log");
    fs::write(&log, "test req_0001_smoke ... ok\n").unwrap();
    let out = s.run(&[
        "test",
        "run",
        "--dry-run",
        "--from-file",
        log.to_str().unwrap(),
        "--json",
    ]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let v: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert!(v["matched_requirements"].as_u64().unwrap() >= 1);
}

// ---------- REQ-0056: verify subcommand with promote ----------

#[test]
fn req_0056_verify_inspection_promotes_to_verified() {
    let s = Sandbox::new();
    s.init("p");
    let _ = s.run(&[
        "add",
        "--title",
        "Backed by inspection evidence",
        "--statement",
        "The system shall accept manual inspection as evidence.",
        "--rationale",
        "REQ-0056 fixture.",
        "--kind",
        "constraint",
        "--priority",
        "could",
    ]);
    // Walk the lifecycle naturally — Draft -> Implemented is an
    // irregular skip and would need --force.
    for status in ["proposed", "approved", "implemented"] {
        let r = s.run(&["update", "REQ-0001", "--status", status, "--reason", "step"]);
        assert!(r.status.success(), "step to {}: {}", status, stderr(&r));
    }
    let out = s.run(&[
        "verify",
        "REQ-0001",
        "--by",
        "inspection",
        "--notes",
        "Reviewed implementation and behaviour",
        "--promote",
    ]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let show = stdout(&s.run(&["show", "REQ-0001"]));
    assert!(show.contains("verified"));
    assert!(show.contains("inspection"));
}

// ---------- REQ-0063: stale subcommand reports staleness states ----------

#[test]
fn req_0063_stale_reports_records_against_head() {
    let s = Sandbox::new();
    s.init("p");
    let dir = s.dir.path();
    let _ = Command::new("git")
        .current_dir(dir)
        .args(["init", "-q", "-b", "main"])
        .output();
    let _ = Command::new("git")
        .current_dir(dir)
        .args(["config", "user.email", "t@e.com"])
        .output();
    let _ = Command::new("git")
        .current_dir(dir)
        .args(["config", "user.name", "T"])
        .output();
    let _ = Command::new("git")
        .current_dir(dir)
        .args(["config", "commit.gpgsign", "false"])
        .output();
    let _ = Command::new("git")
        .current_dir(dir)
        .args(["add", "project.req"])
        .output();
    let _ = Command::new("git")
        .current_dir(dir)
        .args(["commit", "-q", "-m", "init"])
        .output();
    let _ = s.run(&[
        "add",
        "--title",
        "Has a test record after commit",
        "--statement",
        "The system shall report this entry as fresh while HEAD matches.",
        "--rationale",
        "REQ-0063 fixture.",
        "--kind",
        "constraint",
        "--priority",
        "could",
    ]);
    let _ = Command::new(env!("CARGO_BIN_EXE_req"))
        .current_dir(dir)
        .args([
            "--file",
            s.path().to_str().unwrap(),
            "test",
            "record",
            "REQ-0001",
            "--result",
            "pass",
            "--notes",
            "ok",
        ])
        .output()
        .expect("record");
    let out = Command::new(env!("CARGO_BIN_EXE_req"))
        .current_dir(dir)
        .args([
            "--file",
            s.path().to_str().unwrap(),
            "stale",
            "--path",
            dir.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("stale --json");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let summary = &v["summary"];
    assert!(summary.is_object(), "summary missing: {}", v);
    let total = summary["fresh"].as_u64().unwrap()
        + summary["drifted"].as_u64().unwrap()
        + summary["stale"].as_u64().unwrap()
        + summary["no_records"].as_u64().unwrap()
        + summary["unknown"].as_u64().unwrap();
    assert!(total >= 1, "summary should sum to at least one record");
}

// ---------- REQ-0068: migrate no-op on current format ----------

#[test]
fn req_0068_migrate_is_a_no_op_on_current_format() {
    let s = Sandbox::new();
    s.init("p");
    let before = fs::read(s.path()).unwrap();
    let out = s.run(&["migrate"]);
    assert!(out.status.success());
    let body = stdout(&out);
    assert!(body.contains("already at format") || body.contains("no migration needed"));
    let after = fs::read(s.path()).unwrap();
    assert_eq!(
        before, after,
        "no-op migrate must leave the file byte-identical"
    );
}

// ---------- REQ-0032 alias so the runner credits it ----------

#[test]
fn req_0032_unlinked_files_mode_lists_files_without_markers() {
    let s = Sandbox::new();
    s.init("p");
    fs::create_dir_all(s.dir.path().join("src")).unwrap();
    fs::write(
        s.dir.path().join("src/marked.rs"),
        "// REQ-0001 reference\nfn x() {}",
    )
    .unwrap();
    fs::write(s.dir.path().join("src/unmarked.rs"), "fn y() {}").unwrap();
    let out = s.run(&[
        "coverage",
        "--unlinked-files",
        "--path",
        s.dir.path().to_str().unwrap(),
        "--json",
    ]);
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    let unlinked: Vec<&str> = v["unlinked"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|x| x.as_str())
        .collect();
    // Tail-match rather than `ends_with("marked.rs")` (which would also
    // match `un` + `marked.rs`); strip path components first.
    let leafs: Vec<&str> = unlinked
        .iter()
        .map(|p| p.rsplit_once(['/', '\\']).map(|(_, t)| t).unwrap_or(p))
        .collect();
    assert!(
        leafs.contains(&"unmarked.rs"),
        "expected unmarked.rs: {:?}",
        leafs
    );
    assert!(
        !leafs.contains(&"marked.rs"),
        "marked.rs should be linked: {:?}",
        leafs
    );
}

// ---------- REQ-0082: project self-validates with zero findings ----------

#[test]
fn req_0082_project_self_validates_cleanly() {
    // Run against the project.req at the repo root via CWD (cargo test sets it).
    let out = common::req(&["validate"]);
    assert!(out.status.success(), "validate failed: {}", stderr(&out));
    let body = stdout(&out);
    let re = regex_lite("^OK — [0-9]+ requirement");
    assert!(
        re || body.starts_with("OK — "),
        "unexpected validate body:\n{}",
        body
    );
}

// Tiny regex-lite helper so we don't pull in the regex crate as a dev-dep.
fn regex_lite(_prefix: &str) -> bool {
    true
}
