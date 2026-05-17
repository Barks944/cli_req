// Discharges REQ-0001 (add sub-surface) and REQ-0008 (acceptance gate at write).
use anyhow::{anyhow, Result};
use chrono::Utc;
use dialoguer::{theme::ColorfulTheme, Confirm, Input, MultiSelect, Select};
use std::path::PathBuf;

use crate::cli::AddArgs;
use crate::model::{Kind, Link, LinkKind, Priority, Requirement, Status};
use crate::storage::{self, load_resolved};
use crate::validate;

pub fn run(args: AddArgs, file: &Option<PathBuf>) -> Result<()> {
    let (path, mut project) = load_resolved(file)?;

    let interactive = args.interactive
        || (args.title.is_none() && args.statement.is_none() && atty_stdin());

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
