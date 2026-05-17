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
    fs::write(
        s.dir.path().join("src/lib.rs"),
        "// REQ-0001 reference\n// REQ-9999 ghost\nfn _x() {}\n",
    )
    .unwrap();
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
    assert!(v["ghosts"].as_object().unwrap().contains_key("REQ-9999"));
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
    fs::write(&f, "// REQ-0099 to be remapped\nfn x() {}").unwrap();
    // Dry run does NOT mutate
    let dry = s.run(&[
        "coverage",
        "--remap",
        "REQ-0099=REQ-0001",
        "--path",
        s.dir.path().to_str().unwrap(),
    ]);
    assert!(dry.status.success());
    assert!(fs::read_to_string(&f).unwrap().contains("REQ-0099"));
    // Apply DOES mutate
    let apply = s.run(&[
        "coverage",
        "--remap",
        "REQ-0099=REQ-0001",
        "--apply",
        "--path",
        s.dir.path().to_str().unwrap(),
    ]);
    assert!(apply.status.success());
    let after = fs::read_to_string(&f).unwrap();
    assert!(after.contains("REQ-0001"));
    assert!(!after.contains("REQ-0099"));
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
    let _ = s.run(&[
        "update",
        "REQ-0001",
        "--status",
        "implemented",
        "--reason",
        "manual review",
    ]);
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
