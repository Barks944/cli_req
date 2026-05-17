// Discharges REQ-0001 (add sub-surface), REQ-0008 (acceptance gate at write),
// REQ-0038 (--json output on creates).
use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use dialoguer::{theme::ColorfulTheme, Confirm, Input, MultiSelect, Select};
use std::path::PathBuf;

use crate::cli::AddArgs;
use crate::model::{Kind, Link, LinkKind, Priority, Requirement, Status};
use crate::storage::{self, load_resolved};
use crate::validate;

pub fn run(args: AddArgs, file: &Option<PathBuf>) -> Result<()> {
    // REQ-0072: --from-json bypasses shell quoting for multi-line content.
    let args = if args.from_json.is_some() {
        let src = args.from_json.clone().unwrap();
        merge_from_json(args, &src)?
    } else { args };

    let (path, mut project) = load_resolved(file)?;

    let interactive = args.interactive
        || (args.title.is_none() && args.statement.is_none() && !args.from_json.is_some() && atty_stdin());

    let theme = ColorfulTheme::default();

    let title = match args.title {
        Some(t) => t,
        None if interactive => Input::with_theme(&theme)
            .with_prompt("Title (imperative, ≤120 chars)")
            .interact_text()?,
        None => return Err(anyhow!("--title is required in non-interactive mode")),
    };

    let statement = match args.statement {
        Some(s) => s,
        None if interactive => Input::with_theme(&theme)
            .with_prompt("Statement (use shall/must/should/will)")
            .interact_text()?,
        None => return Err(anyhow!("--statement is required in non-interactive mode")),
    };

    let rationale = match args.rationale {
        Some(r) => r,
        None if interactive => Input::with_theme(&theme)
            .with_prompt("Rationale (why does this exist?)")
            .interact_text()?,
        None => return Err(anyhow!("--rationale is required in non-interactive mode")),
    };

    let kind: Kind = match args.kind {
        Some(k) => k.into(),
        None if interactive => {
            let opts = ["Functional", "NonFunctional", "Constraint", "Interface", "Business"];
            let idx = Select::with_theme(&theme)
                .with_prompt("Kind")
                .items(&opts)
                .default(0)
                .interact()?;
            match idx {
                0 => Kind::Functional,
                1 => Kind::NonFunctional,
                2 => Kind::Constraint,
                3 => Kind::Interface,
                _ => Kind::Business,
            }
        }
        None => Kind::Functional,
    };

    let priority: Priority = match args.priority {
        Some(p) => p.into(),
        None if interactive => {
            let opts = ["Must", "Should", "Could", "Wont"];
            let idx = Select::with_theme(&theme)
                .with_prompt("Priority (MoSCoW)")
                .items(&opts)
                .default(1)
                .interact()?;
            match idx {
                0 => Priority::Must,
                1 => Priority::Should,
                2 => Priority::Could,
                _ => Priority::Wont,
            }
        }
        None => Priority::Should,
    };

    let mut acceptance = args.acceptance;
    if interactive && matches!(kind, Kind::Functional) && acceptance.is_empty() {
        println!("Acceptance criteria (blank line to finish):");
        loop {
            let line: String = Input::with_theme(&theme)
                .with_prompt(format!("  AC #{}", acceptance.len() + 1))
                .allow_empty(true)
                .interact_text()?;
            if line.trim().is_empty() {
                break;
            }
            acceptance.push(line);
        }
    }

    let mut tags = args.tag;
    if interactive && tags.is_empty() {
        let raw: String = Input::with_theme(&theme)
            .with_prompt("Tags (comma-separated, blank to skip)")
            .allow_empty(true)
            .interact_text()?;
        tags = raw
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
    }

    let mut links = Vec::new();
    if let Some(parent) = &args.parent {
        if !project.requirements.contains_key(parent) {
            return Err(anyhow!("parent {} does not exist", parent));
        }
        links.push(Link { kind: LinkKind::Parent, target: parent.clone() });
    } else if interactive && !project.requirements.is_empty() {
        let ids: Vec<&String> = project.requirements.keys().collect();
        let display: Vec<String> = ids
            .iter()
            .map(|id| format!("{} — {}", id, project.requirements[*id].title))
            .collect();
        let picks = MultiSelect::with_theme(&theme)
            .with_prompt("Link to parents (space to toggle, enter to confirm)")
            .items(&display)
            .interact()?;
        for i in picks {
            links.push(Link { kind: LinkKind::Parent, target: ids[i].clone() });
        }
    }

    let now = Utc::now();
    // Build with placeholder id; validate BEFORE allocating so failed adds
    // do not consume IDs (REQ-0010: stable sequential allocation).
    let mut req = Requirement {
        id: String::new(),
        title,
        statement,
        rationale,
        acceptance,
        kind,
        priority,
        status: Status::Draft,
        tags,
        links,
        created: now,
        updated: now,
        history: vec![super::history("created", None)],
        tests: Vec::new(),
    };

    let findings = validate::validate_requirement(&req);
    let errors = validate::errors_only(&findings);
    if !findings.is_empty() {
        eprintln!("Validation:");
        for f in &findings {
            eprintln!("  {} [{}] {}", if f.error { "ERR " } else { "WARN" }, f.field, f.message);
        }
    }
    if !errors.is_empty() {
        if interactive {
            let proceed = Confirm::with_theme(&theme)
                .with_prompt("Errors above. Save anyway as Draft?")
                .default(false)
                .interact()?;
            if !proceed {
                return Err(anyhow!("aborted"));
            }
        } else {
            return Err(anyhow!("{} validation errors — fix and retry", errors.len()));
        }
    }

    let id = project.allocate_id();
    req.id = id.clone();
    project.requirements.insert(id.clone(), req.clone());
    project.updated = now;
    storage::save(&path, &project)?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&project.requirements[&id])?);
    } else {
        println!("Added {}", id);
    }
    Ok(())
}

fn atty_stdin() -> bool {
    use std::io::IsTerminal;
    std::io::stdin().is_terminal()
}

/// REQ-0072: load `--from-json` (file path or `-` for stdin) and overlay its
/// fields onto the existing AddArgs. CLI flags take precedence when both are
/// supplied (so `--from-json doc.json --priority must` lets you override).
fn merge_from_json(mut args: AddArgs, src: &str) -> Result<AddArgs> {
    use std::io::Read;
    let raw = if src == "-" {
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf)?;
        buf
    } else {
        std::fs::read_to_string(src)
            .with_context(|| format!("read --from-json source {}", src))?
    };
    #[derive(serde::Deserialize, Default)]
    struct AddDoc {
        title: Option<String>,
        statement: Option<String>,
        rationale: Option<String>,
        acceptance: Option<Vec<String>>,
        kind: Option<String>,
        priority: Option<String>,
        tags: Option<Vec<String>>,
        parent: Option<String>,
    }
    let doc: AddDoc = serde_json::from_str(&raw).context("parse --from-json document")?;
    if args.title.is_none() { args.title = doc.title; }
    if args.statement.is_none() { args.statement = doc.statement; }
    if args.rationale.is_none() { args.rationale = doc.rationale; }
    if args.acceptance.is_empty() {
        if let Some(a) = doc.acceptance { args.acceptance = a; }
    }
    if args.kind.is_none() {
        if let Some(k) = doc.kind {
            args.kind = Some(match k.as_str() {
                "functional" => crate::cli::KindArg::Functional,
                "non-functional" | "nonfunctional" => crate::cli::KindArg::NonFunctional,
                "constraint" => crate::cli::KindArg::Constraint,
                "interface" => crate::cli::KindArg::Interface,
                "business" => crate::cli::KindArg::Business,
                other => return Err(anyhow!("--from-json: unknown kind '{}'", other)),
            });
        }
    }
    if args.priority.is_none() {
        if let Some(p) = doc.priority {
            args.priority = Some(match p.as_str() {
                "must" => crate::cli::PriorityArg::Must,
                "should" => crate::cli::PriorityArg::Should,
                "could" => crate::cli::PriorityArg::Could,
                "wont" => crate::cli::PriorityArg::Wont,
                other => return Err(anyhow!("--from-json: unknown priority '{}'", other)),
            });
        }
    }
    if args.tag.is_empty() {
        if let Some(t) = doc.tags { args.tag = t; }
    }
    if args.parent.is_none() { args.parent = doc.parent; }
    Ok(args)
}
