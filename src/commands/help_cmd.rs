// Implements REQ-0018 (structured sectioned help), REQ-0031 (idempotent
// install of any section into AGENTS.md), and REQ-0042 (--json with a
// structured agents-crib payload).
use anyhow::{anyhow, Context, Result};
use once_cell::sync::Lazy;
use regex::Regex;
use std::fs;

use crate::cli::HelpArgs;
use crate::help_text::{self, Section};

// REQ-0120: regex matching the same four-digit REQ-ID pattern used by
// the coverage scanner. Anything that matches this in user-installed
// AGENTS.md text would be picked up as a code reference in the
// adopter's project — so the install path rewrites them.
static LITERAL_REQ_ID: Lazy<Regex> = Lazy::new(|| Regex::new(r"REQ-\d{4}").unwrap());

fn sanitize_req_ids_for_agents_md(body: &str) -> String {
    LITERAL_REQ_ID.replace_all(body, "REQ-NNNN").into_owned()
}

pub fn run(args: HelpArgs) -> Result<()> {
    if args.list || args.section.is_none() {
        if args.json {
            let sections: Vec<_> = help_text::sections()
                .iter()
                .map(|s| serde_json::json!({ "name": s.name, "summary": s.summary }))
                .collect();
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({ "sections": sections }))?
            );
            return Ok(());
        }
        println!("Help sections — `req help <name>`:\n");
        for s in help_text::sections() {
            println!("  {:<14} {}", s.name, s.summary);
        }
        println!(
            "\nTip: `req help all` to print everything.\n     \
             `req help <section> --install` to write the section into AGENTS.md.\n     \
             `req help <section> --json` for a structured form."
        );
        return Ok(());
    }
    let want = args.section.unwrap();
    if args.install {
        if want == "all" {
            return Err(anyhow!("--install requires a specific section, not 'all'"));
        }
        let s = help_text::section(&want)
            .ok_or_else(|| anyhow!("no such section: {}. Try `req help --list`.", want))?;
        return install_section(s, &args.path);
    }
    if want == "all" {
        if args.json {
            let sections: Vec<_> = help_text::sections()
                .iter()
                .map(
                    |s| serde_json::json!({ "name": s.name, "summary": s.summary, "body": help_text::render_body(s.body) }),
                )
                .collect();
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({ "sections": sections }))?
            );
            return Ok(());
        }
        for s in help_text::sections() {
            println!("## {}\n", s.name);
            println!("{}\n", help_text::render_body(s.body));
        }
        return Ok(());
    }
    let section = match help_text::section(&want) {
        Some(s) => s,
        None => {
            eprintln!("No such section: {}. Try `req help --list`.", want);
            std::process::exit(2);
        }
    };

    if args.json {
        let mut body = serde_json::json!({
            "name": section.name,
            "summary": section.summary,
            "body": help_text::render_body(section.body),
        });
        if section.name == "agents" {
            body["structured"] = agents_crib();
        }
        println!("{}", serde_json::to_string_pretty(&body)?);
        return Ok(());
    }

    println!("{}\n", section.name);
    println!("{}", help_text::render_body(section.body));
    Ok(())
}

/// Structured form of the agents crib for REQ-0042. Keep in sync with the
/// prose body of help_text::section("agents") — both surfaces should
/// describe the same triggers/commands/rules.
fn agents_crib() -> serde_json::Value {
    serde_json::json!({
        "triggers": [
            { "situation": "user describes new behaviour the system should have", "first_command": "req add" },
            { "situation": "starting work on a feature",                          "first_command": "req list" },
            { "situation": "about to commit",                                     "first_command": "req validate" },
            { "situation": "changed behaviour covered by a requirement",          "first_command": "req update <id> --reason ..." },
            { "situation": "refactor; unsure what's load-bearing",                "first_command": "req coverage --path src" },
            { "situation": "finding code with no requirement link",               "first_command": "req coverage --unlinked-files" },
            { "situation": "requirement is no longer relevant",                   "first_command": "req delete <id> --reason ..." },
            { "situation": "file won't load (integrity error)",                   "first_command": "req repair --confirm-direct-edit" },
            { "situation": "merge brought in colliding IDs",                      "first_command": "req renumber --base origin/main" },
            { "situation": "want at-a-glance progress",                           "first_command": "req status" },
            { "situation": "what should I work on next?",                         "first_command": "req next" },
        ],
        "commands": [
            { "name": "req list",     "purpose": "What exists" },
            { "name": "req show",     "purpose": "Full detail with history" },
            { "name": "req add",      "purpose": "Create; validator enforces best practice" },
            { "name": "req update",   "purpose": "Modify; --reason mandatory" },
            { "name": "req link",     "purpose": "Typed links: parent / depends-on / refines / conflicts / verifies" },
            { "name": "req delete",   "purpose": "Soft (Obsolete) by default" },
            { "name": "req validate", "purpose": "Run rules; 0 errors required to ship" },
            { "name": "req status",   "purpose": "Counts and percentages by status bucket" },
            { "name": "req next",     "purpose": "One requirement to work on, deps satisfied" },
            { "name": "req check",    "purpose": "Validate + coverage scoped to changes since <ref>" },
            { "name": "req coverage", "purpose": "Spec ↔ code drift; --unlinked-files, --by-file, --remap" },
            { "name": "req help",     "purpose": "Browse docs; --install writes a section into AGENTS.md; --json for tooling" },
        ],
        "rules": [
            "Statements need a normative modal verb (shall/must/should/will).",
            "Functional requirements need at least one acceptance criterion.",
            "Pass --reason on every update and delete; history records the why.",
            "Drop // REQ-NNNN markers in source where you implement a requirement.",
            "Never cat/read project.req — the integrity hash will block you on the next op.",
            "Set REQ_ACTOR_KIND=agent in your environment so history attributes you correctly.",
        ],
        "env": [
            { "name": "REQ_ACTOR",      "purpose": "Override the author name on history entries (default: $USER)." },
            { "name": "REQ_ACTOR_KIND", "purpose": "Set to 'human' or 'agent' for REQ-0043 provenance tagging." },
            { "name": "REQ_FILE",       "purpose": "Override the default .req file path." },
        ],
    })
}

fn install_section(section: &Section, path: &std::path::Path) -> Result<()> {
    let begin = format!("<!-- req:help:{}:begin -->", section.name);
    let end = format!("<!-- req:help:{}:end -->", section.name);
    // REQ-0120: replace literal REQ-NNNN identifiers with a placeholder
    // when writing into AGENTS.md. The help text cites cli_req's own
    // requirement IDs (REQ-0001, REQ-0117 etc.) as examples; left
    // literal, those become source-tree references in any adopter's
    // project and trip their coverage scanner with ghosts. The
    // coverage regex matches `REQ-\d{4}`, so a non-digit placeholder
    // (REQ-NNNN) is safe.
    let body = sanitize_req_ids_for_agents_md(&help_text::render_body(section.body));
    let block = format!(
        "{begin}\n\n\
         <!-- Managed by `req help {} --install`. Re-run to refresh; edit OUTSIDE the markers to add your own notes. -->\n\n\
         ## req — {}\n\n\
         _{}_\n\n\
         ```\n{}\n```\n\n\
         {end}",
        section.name, section.name, section.summary, body
    );

    let existing = fs::read_to_string(path).unwrap_or_default();
    let new_contents = if let (Some(b), Some(e)) = (existing.find(&begin), existing.find(&end)) {
        let after_end = e + end.len();
        let mut s = String::new();
        s.push_str(&existing[..b]);
        s.push_str(&block);
        s.push_str(&existing[after_end..]);
        s
    } else {
        let mut s = existing.clone();
        if !s.is_empty() && !s.ends_with('\n') {
            s.push('\n');
        }
        if !s.is_empty() {
            s.push('\n');
        }
        s.push_str(&block);
        s.push('\n');
        s
    };

    fs::write(path, new_contents).with_context(|| format!("write {}", path.display()))?;
    println!(
        "Installed `{}` section into {} (between {} and {}).",
        section.name,
        path.display(),
        begin,
        end
    );
    Ok(())
}
