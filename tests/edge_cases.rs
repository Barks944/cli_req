// Edge-case + malformed-input tests added during pre-release tightening.
// Covers the rough surfaces that previously had only happy-path tests:
// req batch rollback corners, req import malformed sources, MCP protocol
// shape, and req coverage --strict --allow.
mod common;
use common::{stderr, stdout, Sandbox};
use std::fs;
use std::io::Write;
use std::process::{Command, Stdio};

// ---------- REQ-0066: batch edge cases ----------

#[test]
fn req_0066_batch_empty_mutations_array_is_a_noop() {
    let s = Sandbox::new();
    s.init("p");
    let before = fs::read(s.path()).unwrap();
    let doc = serde_json::json!({ "mutations": [] });
    let path = s.dir.path().join("empty.json");
    fs::write(&path, doc.to_string()).unwrap();
    let out = s.run(&["batch", path.to_str().unwrap()]);
    assert!(out.status.success());
    let after = fs::read(s.path()).unwrap();
    assert_eq!(before, after, "empty batch must not touch the file");
}

#[test]
fn req_0066_batch_unknown_mutation_kind_is_rejected_atomically() {
    let s = Sandbox::new();
    s.init("p");
    let doc = serde_json::json!({
        "mutations": [
            { "kind": "summon-elder-god", "title": "x" }
        ]
    });
    let path = s.dir.path().join("bad-kind.json");
    fs::write(&path, doc.to_string()).unwrap();
    let out = s.run(&["batch", path.to_str().unwrap()]);
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("unknown variant") || stderr(&out).contains("parse batch document")
    );
}

#[test]
fn req_0066_batch_link_to_self_is_rejected_and_rolls_back() {
    let s = Sandbox::new();
    s.init("p");
    let before = fs::read(s.path()).unwrap();
    let doc = serde_json::json!({
        "mutations": [
            { "kind": "add",
              "title": "Solo requirement here",
              "statement": "The system shall accept this perfectly fine baseline.",
              "rationale": "Setup.",
              "req_kind": "constraint", "priority": "could" },
            { "kind": "link", "from": "REQ-0001", "to": "REQ-0001", "link_kind": "parent" }
        ]
    });
    let path = s.dir.path().join("self-link.json");
    fs::write(&path, doc.to_string()).unwrap();
    let out = s.run(&["batch", path.to_str().unwrap()]);
    assert!(!out.status.success());
    let after = fs::read(s.path()).unwrap();
    assert_eq!(before, after, "self-link must roll back the whole batch");
}

// ---------- REQ-0067: import malformed sources ----------

#[test]
fn req_0067_import_markdown_with_no_headings_emits_clear_error() {
    let s = Sandbox::new();
    s.init("p");
    let path = s.dir.path().join("noheadings.md");
    fs::write(&path, "Just prose. No headings, no requirements.\n").unwrap();
    let out = s.run(&["import", "-f", "markdown", path.to_str().unwrap()]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("no requirement candidates"));
}

#[test]
fn req_0067_import_json_with_invalid_shape_reports_clearly() {
    let s = Sandbox::new();
    s.init("p");
    let path = s.dir.path().join("scalar.json");
    fs::write(&path, "42").unwrap();
    let out = s.run(&["import", "-f", "json", path.to_str().unwrap()]);
    assert!(!out.status.success());
    assert!(
        stderr(&out).to_lowercase().contains("array")
            || stderr(&out).to_lowercase().contains("object")
    );
}

#[test]
fn req_0067_import_strict_aborts_on_first_invalid_item() {
    let s = Sandbox::new();
    s.init("p");
    let path = s.dir.path().join("mixed.md");
    fs::write(&path, "## A perfectly valid requirement here\n\nThe system shall implement this fine behaviour.\n\nRationale: ok.\n\n## Bad\n\nToo short.\n\nRationale: bad.\n").unwrap();
    let out = s.run(&[
        "import",
        "-f",
        "markdown",
        path.to_str().unwrap(),
        "--strict",
    ]);
    assert!(
        !out.status.success(),
        "strict mode should abort on the bad item"
    );
}

// ---------- REQ-0017: MCP protocol surface ----------

fn mcp_roundtrip(messages: &[&str]) -> String {
    let mut child = Command::new(env!("CARGO_BIN_EXE_req"))
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn req mcp");
    {
        let stdin = child.stdin.as_mut().expect("stdin");
        for m in messages {
            writeln!(stdin, "{}", m).expect("write");
        }
    }
    let out = child.wait_with_output().expect("wait");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn req_0017_mcp_initialize_returns_serverinfo() {
    let body = mcp_roundtrip(&[r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#]);
    let line = body.lines().next().expect("at least one response line");
    let v: serde_json::Value = serde_json::from_str(line).expect("response is JSON");
    assert_eq!(v["jsonrpc"], "2.0");
    assert_eq!(v["result"]["serverInfo"]["name"], "req");
}

#[test]
fn req_0017_mcp_tools_list_lists_ten_tools() {
    let body = mcp_roundtrip(&[
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
    ]);
    let line = body.lines().nth(1).expect("two response lines");
    let v: serde_json::Value = serde_json::from_str(line).expect("response is JSON");
    let tools = v["result"]["tools"].as_array().expect("tools array");
    assert!(
        tools.len() >= 10,
        "expected at least 10 tools, got {}",
        tools.len()
    );
    let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
    for required in &[
        "req_list",
        "req_show",
        "req_add",
        "req_update",
        "req_validate",
        "req_help",
    ] {
        assert!(names.contains(required), "missing tool {}", required);
    }
    assert!(
        !names.contains(&"req_repair"),
        "MCP must not expose req_repair"
    );
}

#[test]
fn req_0017_mcp_unknown_method_returns_error_envelope() {
    let body = mcp_roundtrip(&[r#"{"jsonrpc":"2.0","id":1,"method":"thanos.snap"}"#]);
    let line = body.lines().next().expect("response");
    let v: serde_json::Value = serde_json::from_str(line).expect("JSON");
    assert!(
        v["error"].is_object(),
        "unknown method should return error: {}",
        v
    );
}

// ---------- REQ-0065: coverage --strict --allow ----------

#[test]
fn req_0065_strict_allow_lets_known_orphans_pass() {
    let s = Sandbox::new();
    s.init("p");
    let _ = s.run(&[
        "add",
        "--title",
        "Verification-only requirement here",
        "--statement",
        "The system shall be verifiable through inspection only.",
        "--rationale",
        "No code site needed.",
        "--kind",
        "constraint",
        "--priority",
        "could",
    ]);
    // Without --allow: strict fails
    let blocked = s.run(&[
        "coverage",
        "--path",
        s.dir.path().to_str().unwrap(),
        "--strict",
    ]);
    assert!(!blocked.status.success(), "orphan should trip strict");
    // With --allow REQ-0001: strict passes
    let allowed = s.run(&[
        "coverage",
        "--path",
        s.dir.path().to_str().unwrap(),
        "--strict",
        "--allow",
        "REQ-0001",
    ]);
    assert!(
        allowed.status.success(),
        "explicit allow should clear strict: {}",
        stderr(&allowed)
    );
}
