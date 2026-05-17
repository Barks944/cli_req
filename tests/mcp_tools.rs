// Exercises individual MCP tools end-to-end via the JSON-RPC stdio
// transport. Each test drives `req mcp` with a sequence of messages,
// asserts the response shape, and (where the tool mutates) verifies
// the mutation persisted via the CLI.
mod common;
use common::{stdout, Sandbox};
use std::io::Write;
use std::process::{Command, Stdio};

/// Send a sequence of newline-delimited JSON-RPC messages to a fresh
/// `req mcp` subprocess and collect the (parsed) responses in order.
/// Notifications without an `id` produce no response.
fn mcp_dialogue(s: &Sandbox, messages: &[serde_json::Value]) -> Vec<serde_json::Value> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_req"))
        .args(["--file", s.path().to_str().unwrap(), "mcp"])
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
    let body = String::from_utf8_lossy(&out.stdout);
    body.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            serde_json::from_str(l).unwrap_or_else(|_| panic!("non-JSON response line: {}", l))
        })
        .collect()
}

fn initialize() -> serde_json::Value {
    serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize"})
}

fn call_tool(id: i32, name: &str, args: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/call",
        "params": { "name": name, "arguments": args }
    })
}

fn text_of(response: &serde_json::Value) -> String {
    response["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or("")
        .to_string()
}

// ---------- REQ-0083: TUI parity guard ----------

/// CLI commands that don't need a TUI menu entry. Same exclusion set
/// as MCP plus a couple that are awkward in an interactive menu:
/// `check` and `renumber` need a git ref typed in (still doable, but
/// the menu would be a thin wrapper), and `migrate`/`schema`/`batch`/
/// `import` are file-driven operations users typically run from the
/// shell.
const HUMANS_ONLY_TUI: &[&str] = &[
    "init", "tui", "serve", "mcp", "hooks", "renumber", "repair", "migrate", "schema", "batch",
    "import", "test", "verify", "check", "help",
];

#[test]
fn req_0083_tui_menu_covers_every_agent_relevant_cli_command() {
    // Source-level introspection: parse the MENU const out of src/tui.rs.
    let src = std::fs::read_to_string("src/tui.rs").expect("read tui.rs");
    let start = src.find("pub const MENU").expect("MENU const present");
    // Find the `=` that opens the literal, then the next `[` after that.
    let eq = src[start..].find('=').expect("= after MENU") + start;
    let open_rel = src[eq..].find('[').expect("[ after =");
    let open = eq + open_rel;
    let close = src[open..].find(']').expect("] after [") + open;
    let body = &src[open + 1..close];
    let menu_labels: Vec<String> = body
        .split(',')
        .map(|s| s.trim().trim_matches('"').trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    // Re-derive the CLI command set.
    let help = common::req(&["--help"]);
    let help_body = stdout(&help);
    let mut cli: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut in_commands = false;
    for line in help_body.lines() {
        let trimmed = line.trim_start();
        if trimmed == "Commands:" {
            in_commands = true;
            continue;
        }
        if !in_commands {
            continue;
        }
        if trimmed.is_empty()
            || trimmed.starts_with("Options:")
            || trimmed.starts_with("Arguments:")
        {
            in_commands = false;
            continue;
        }
        let leading = line.chars().take_while(|c| *c == ' ').count();
        if leading != 2 {
            continue;
        }
        if let Some(first) = trimmed.split_whitespace().next() {
            if first.chars().all(|c| c.is_ascii_lowercase() || c == '-') {
                cli.insert(first.to_string());
            }
        }
    }
    for excl in HUMANS_ONLY_TUI {
        cli.remove(*excl);
    }

    // For each remaining CLI command, find a menu entry that mentions it
    // (parens-form like "Browse (list + show)" or substring match).
    let menu_lc: Vec<String> = menu_labels.iter().map(|s| s.to_lowercase()).collect();
    let missing: Vec<&String> = cli
        .iter()
        .filter(|cmd| {
            let needle = cmd.to_lowercase();
            !menu_lc.iter().any(|label| label.contains(&needle))
        })
        .collect();
    assert!(
        missing.is_empty(),
        "REQ-0083 — CLI commands without a TUI menu entry (and not in HUMANS_ONLY_TUI): {:?}\n\
         Menu items:\n  {:?}\n\
         Add a menu line to src/tui.rs::MENU or list the command in HUMANS_ONLY_TUI \
         in tests/mcp_tools.rs with a one-line justification.",
        missing,
        menu_labels
    );
}

// ---------- REQ-0083: MCP/CLI parity guard ----------

/// CLI subcommands that DELIBERATELY do not have an MCP tool. Adding a
/// command here is an opt-out; the test below ensures every other CLI
/// command appears as a req_* MCP tool. See REQ-0083.
const HUMANS_ONLY_CLI: &[&str] = &["init", "tui", "serve", "mcp", "hooks", "renumber", "repair"];

#[test]
fn req_0083_mcp_tool_surface_covers_every_agent_relevant_cli_command() {
    let help = common::req(&["--help"]);
    let body = stdout(&help);
    // Parse only the `Commands:` section of --help. clap formats it as a
    // header line "Commands:" followed by indented subcommand rows until
    // the next blank line or an "Options:" section.
    let mut in_commands = false;
    let mut cli: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for line in body.lines() {
        let trimmed = line.trim_start();
        if trimmed == "Commands:" {
            in_commands = true;
            continue;
        }
        if !in_commands {
            continue;
        }
        if trimmed.is_empty()
            || trimmed.starts_with("Options:")
            || trimmed.starts_with("Arguments:")
        {
            in_commands = false;
            continue;
        }
        // Only top-level subcommand lines have exactly 2 leading spaces;
        // continuation lines (wrapped descriptions) are indented further.
        let leading = line.chars().take_while(|c| *c == ' ').count();
        if leading != 2 {
            continue;
        }
        if let Some(first) = trimmed.split_whitespace().next() {
            if first.chars().all(|c| c.is_ascii_lowercase() || c == '-') {
                cli.insert(first.to_string());
            }
        }
    }
    for excl in HUMANS_ONLY_CLI {
        cli.remove(*excl);
    }
    let s = Sandbox::new();
    s.init("p");
    let responses = mcp_dialogue(
        &s,
        &[
            initialize(),
            serde_json::json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
        ],
    );
    let tools = responses[1]["result"]["tools"]
        .as_array()
        .expect("tools/list returns array");
    let mcp_names: std::collections::BTreeSet<String> = tools
        .iter()
        .filter_map(|t| {
            t["name"]
                .as_str()
                .map(|n| n.trim_start_matches("req_").to_string())
        })
        .collect();
    let mcp_root_names: std::collections::BTreeSet<String> = mcp_names
        .iter()
        .map(|n| n.split('_').next().unwrap_or(n).to_string())
        .collect();
    let missing: Vec<&String> = cli
        .iter()
        .filter(|c| !mcp_root_names.contains(*c) && !mcp_names.contains(*c))
        .collect();
    assert!(
        missing.is_empty(),
        "REQ-0083 — CLI subcommands without an MCP tool (and not in HUMANS_ONLY_CLI): {:?}\n\
         Either expose it as a req_* tool in src/mcp.rs or add it to HUMANS_ONLY_CLI.",
        missing
    );
}

// ---------- REQ-0017: tool surface ----------

#[test]
fn req_0017_mcp_req_list_returns_count() {
    let s = Sandbox::new();
    s.init("p");
    let _ = s.run(&[
        "add",
        "--title",
        "First listed via MCP",
        "--statement",
        "The system shall be returned by the req_list tool.",
        "--rationale",
        "MCP list fixture.",
        "--kind",
        "constraint",
        "--priority",
        "could",
    ]);
    let responses = mcp_dialogue(
        &s,
        &[
            initialize(),
            call_tool(2, "req_list", serde_json::json!({})),
        ],
    );
    let list_text = text_of(&responses[1]);
    let v: serde_json::Value = serde_json::from_str(&list_text).expect("inner json");
    assert_eq!(v["count"], 1);
    assert_eq!(v["requirements"][0]["id"].as_str().unwrap(), "REQ-0001");
}

#[test]
fn req_0017_mcp_req_show_returns_full_requirement() {
    let s = Sandbox::new();
    s.init("p");
    let _ = s.run(&[
        "add",
        "--title",
        "Shown by the req_show MCP tool",
        "--statement",
        "The system shall be returned in full by req_show.",
        "--rationale",
        "MCP show fixture.",
        "--kind",
        "constraint",
        "--priority",
        "could",
    ]);
    let responses = mcp_dialogue(
        &s,
        &[
            initialize(),
            call_tool(2, "req_show", serde_json::json!({"id": "REQ-0001"})),
        ],
    );
    let body = text_of(&responses[1]);
    let v: serde_json::Value = serde_json::from_str(&body).expect("requirement json");
    assert_eq!(v["id"].as_str().unwrap(), "REQ-0001");
    assert!(v["statement"].as_str().unwrap().contains("req_show"));
}

#[test]
fn req_0017_mcp_req_add_persists_through_storage() {
    let s = Sandbox::new();
    s.init("p");
    let responses = mcp_dialogue(
        &s,
        &[
            initialize(),
            call_tool(
                2,
                "req_add",
                serde_json::json!({
                    "title": "Added through the MCP req_add tool",
                    "statement": "The system shall persist an MCP-driven add via storage::save.",
                    "rationale": "Verify the MCP write path uses the same storage layer as the CLI.",
                    "kind": "constraint", "priority": "could"
                }),
            ),
        ],
    );
    let body = text_of(&responses[1]);
    let v: serde_json::Value = serde_json::from_str(&body).expect("add json");
    assert_eq!(v["id"].as_str().unwrap(), "REQ-0001");
    // Now the CLI must see it too — round-trip through storage.
    let list = stdout(&s.run(&["list", "--json"]));
    assert!(list.contains("REQ-0001"));
}

#[test]
fn req_0017_mcp_req_add_validation_failure_returns_iserror() {
    let s = Sandbox::new();
    s.init("p");
    let responses = mcp_dialogue(
        &s,
        &[
            initialize(),
            call_tool(
                2,
                "req_add",
                serde_json::json!({
                    "title": "Bad",  // too short
                    "statement": "too short",
                    "rationale": "x",
                    "kind": "constraint", "priority": "could"
                }),
            ),
        ],
    );
    let r = &responses[1]["result"];
    assert_eq!(
        r["isError"], true,
        "validator failure should set isError=true: {}",
        r
    );
    let msg = r["content"][0]["text"].as_str().unwrap();
    assert!(
        msg.contains("rejected"),
        "error message should name the rejection: {}",
        msg
    );
}

#[test]
fn req_0017_mcp_req_update_records_reason_in_history() {
    let s = Sandbox::new();
    s.init("p");
    let _ = s.run(&[
        "add",
        "--title",
        "Subject of an MCP update",
        "--statement",
        "The system shall accept an MCP-driven update with a reason.",
        "--rationale",
        "Fixture for req_update MCP tool.",
        "--kind",
        "constraint",
        "--priority",
        "could",
    ]);
    let _ = mcp_dialogue(
        &s,
        &[
            initialize(),
            call_tool(
                2,
                "req_update",
                serde_json::json!({
                    "id": "REQ-0001",
                    "reason": "Updated via the MCP tool for test purposes",
                    "add_tag": ["mcp-touched"]
                }),
            ),
        ],
    );
    let show = stdout(&s.run(&["show", "REQ-0001"]));
    assert!(show.contains("mcp-touched"));
    assert!(show.contains("Updated via the MCP tool"));
}

#[test]
fn req_0017_mcp_req_link_creates_typed_edge() {
    let s = Sandbox::new();
    s.init("p");
    for i in 1..=2 {
        let _ = s.run(&[
            "add",
            "--title",
            &format!("Node {} for MCP linking", i),
            "--statement",
            "The system shall be linkable through the MCP req_link tool.",
            "--rationale",
            "Fixture.",
            "--kind",
            "constraint",
            "--priority",
            "could",
        ]);
    }
    let _ = mcp_dialogue(
        &s,
        &[
            initialize(),
            call_tool(
                2,
                "req_link",
                serde_json::json!({
                    "from": "REQ-0001", "to": "REQ-0002", "link_kind": "parent"
                }),
            ),
        ],
    );
    let show = stdout(&s.run(&["show", "REQ-0001"]));
    assert!(
        show.contains("parent -> REQ-0002"),
        "link should be persisted, got:\n{}",
        show
    );
}

#[test]
fn req_0017_mcp_req_delete_soft_marks_obsolete() {
    let s = Sandbox::new();
    s.init("p");
    let _ = s.run(&[
        "add",
        "--title",
        "Retired via the MCP req_delete tool",
        "--statement",
        "The system shall be soft-deleted by the MCP tool.",
        "--rationale",
        "Fixture.",
        "--kind",
        "constraint",
        "--priority",
        "could",
    ]);
    let _ = mcp_dialogue(
        &s,
        &[
            initialize(),
            call_tool(
                2,
                "req_delete",
                serde_json::json!({
                    "id": "REQ-0001", "reason": "Retired via MCP tool"
                }),
            ),
        ],
    );
    let show = stdout(&s.run(&["show", "REQ-0001"]));
    assert!(show.to_lowercase().contains("obsolete"));
}

#[test]
fn req_0017_mcp_req_validate_emits_finding_counts() {
    let s = Sandbox::new();
    s.init("p");
    let _ = s.run(&[
        "add",
        "--title",
        "Valid baseline requirement here",
        "--statement",
        "The system shall validate cleanly under the MCP tool.",
        "--rationale",
        "Fixture.",
        "--kind",
        "constraint",
        "--priority",
        "could",
    ]);
    let responses = mcp_dialogue(
        &s,
        &[
            initialize(),
            call_tool(2, "req_validate", serde_json::json!({})),
        ],
    );
    let body = text_of(&responses[1]);
    let v: serde_json::Value = serde_json::from_str(&body).expect("validate json");
    assert_eq!(v["errors"], 0);
    assert!(v["warnings"].is_number());
}

#[test]
fn req_0017_mcp_req_help_index_lists_section_names() {
    let s = Sandbox::new();
    s.init("p");
    let responses = mcp_dialogue(
        &s,
        &[
            initialize(),
            call_tool(2, "req_help", serde_json::json!({"section": "_index"})),
        ],
    );
    let body = text_of(&responses[1]);
    let v: serde_json::Value = serde_json::from_str(&body).expect("help index json");
    let sections: Vec<&str> = v["sections"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|s| s["name"].as_str())
        .collect();
    for expected in ["agents", "best-practice", "errors", "verification"] {
        assert!(
            sections.contains(&expected),
            "{} missing from index",
            expected
        );
    }
}

#[test]
fn req_0017_mcp_req_help_named_section_returns_body() {
    let s = Sandbox::new();
    s.init("p");
    let responses = mcp_dialogue(
        &s,
        &[
            initialize(),
            call_tool(2, "req_help", serde_json::json!({"section": "agents"})),
        ],
    );
    let body = text_of(&responses[1]);
    let v: serde_json::Value = serde_json::from_str(&body).expect("section json");
    assert_eq!(v["name"], "agents");
    assert!(v["body"].as_str().unwrap().len() > 100);
}

#[test]
fn req_0017_mcp_req_export_renders_markdown() {
    let s = Sandbox::new();
    s.init("p");
    let _ = s.run(&[
        "add",
        "--title",
        "Exported via MCP req_export",
        "--statement",
        "The system shall be rendered to markdown by the MCP tool.",
        "--rationale",
        "Fixture for the export tool.",
        "--kind",
        "constraint",
        "--priority",
        "could",
    ]);
    let responses = mcp_dialogue(
        &s,
        &[
            initialize(),
            call_tool(2, "req_export", serde_json::json!({"format": "markdown"})),
        ],
    );
    let body = text_of(&responses[1]);
    assert!(body.contains("REQ-0001"));
    assert!(body.contains("Exported via MCP"));
    assert!(body.contains("##") || body.contains("**Statement"));
}

#[test]
fn req_0017_mcp_self_link_rejected_with_iserror() {
    let s = Sandbox::new();
    s.init("p");
    let _ = s.run(&[
        "add",
        "--title",
        "Cannot self-link via MCP",
        "--statement",
        "The system shall reject self-links from the MCP tool.",
        "--rationale",
        "Fixture.",
        "--kind",
        "constraint",
        "--priority",
        "could",
    ]);
    let responses = mcp_dialogue(
        &s,
        &[
            initialize(),
            call_tool(
                2,
                "req_link",
                serde_json::json!({
                    "from": "REQ-0001", "to": "REQ-0001", "link_kind": "parent"
                }),
            ),
        ],
    );
    let r = &responses[1]["result"];
    assert_eq!(r["isError"], true);
}

#[test]
fn req_0017_mcp_ping_returns_empty_object() {
    let s = Sandbox::new();
    s.init("p");
    let responses = mcp_dialogue(
        &s,
        &[
            initialize(),
            serde_json::json!({"jsonrpc":"2.0","id":2,"method":"ping"}),
        ],
    );
    assert_eq!(responses[1]["result"], serde_json::json!({}));
}

// Reference initialize_then's underscored argument so clippy stays quiet.
#[allow(dead_code)]
fn _silence_unused() {
    let _ = initialize;
}
