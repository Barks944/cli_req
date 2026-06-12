// REQ-0085: req split — assisted remediation for REQ-V-0010 compound findings.
// Interactive (or flag-driven) split of a compound requirement into
// atomic ones. The validator can flag REQ-V-0010; this command is
// the assisted fix. The original is soft-retired to Obsolete (with a
// reason that names the replacements) and the new parts inherit the
// original's kind, priority, tags. Inbound links are NOT auto-
// rewritten — the caller still has to decide which child takes them
// — but the original's history records the IDs of the children so
// you can find them with `req show <original>`.
use anyhow::{anyhow, Result};
use chrono::Utc;
use dialoguer::{theme::ColorfulTheme, Input};
use std::path::PathBuf;

use crate::cli::SplitArgs;
use crate::model::{Requirement, Status};
use crate::storage::{self, load_for_mutation};
use crate::validate;

pub fn run(mut args: SplitArgs, file: &Option<PathBuf>) -> Result<()> {
    let (path, mut project, _lock) = load_for_mutation(file)?;
    args.id = super::resolve_id(&project, &args.id)?;

    // Snapshot the original so the parts inherit the right metadata.
    let original = project
        .requirements
        .get(&args.id)
        .ok_or_else(|| anyhow!("no such requirement: {}", args.id))?
        .clone();

    if matches!(original.status, Status::Obsolete) {
        return Err(anyhow!(
            "{} is already obsolete; nothing to split.",
            args.id
        ));
    }

    let statements = if !args.into.is_empty() {
        args.into.clone()
    } else {
        prompt_for_parts(&original)?
    };

    if statements.len() < 2 {
        return Err(anyhow!(
            "split needs at least 2 parts (got {}); for a rewrite use `req update --statement`",
            statements.len()
        ));
    }

    // Validate each part *before* we mutate, so a failure leaves the
    // project untouched.
    let now = Utc::now();
    let mut staged: Vec<Requirement> = Vec::new();
    for (i, stmt) in statements.iter().enumerate() {
        // Inherit acceptance criteria from the parent. Functional
        // requirements require at least one acceptance entry, and
        // pre-0.2.1 split started every child with an empty list,
        // which tripped REQ-V-0014 on the first part and aborted the
        // split — meaning functional parents couldn't be split at
        // all. Inheriting matches the obvious intent ("each part
        // gets the parent's contract") and the user can edit per
        // child afterwards with `req update --accept` / `--remove-acceptance`.
        let part = Requirement {
            id: String::new(),
            title: synth_title(&original.title, i, statements.len()),
            statement: stmt.trim().to_string(),
            rationale: format!(
                "Split from {} ({} of {}). Original rationale: {}",
                args.id,
                i + 1,
                statements.len(),
                original.rationale
            ),
            acceptance: original.acceptance.clone(),
            kind: original.kind,
            priority: original.priority,
            status: Status::Draft,
            tags: original.tags.clone(),
            links: Vec::new(),
            created: now,
            updated: now,
            history: vec![super::history(
                format!("split from {}", args.id),
                args.reason.clone(),
            )],
            tests: Vec::new(),
            // REQ-0139: split children start without a validation dossier.
            validation: None,
        };
        let findings = validate::validate_requirement(&part);
        let errs = validate::errors_only(&findings);
        if !errs.is_empty() {
            let msg: Vec<String> = errs
                .iter()
                .map(|f| format!("[{}] {}", f.field, f.message))
                .collect();
            return Err(anyhow!(
                "part #{} failed validation; nothing mutated. {}",
                i + 1,
                msg.join("; ")
            ));
        }
        // Surface warnings for visibility without aborting.
        for f in findings.iter().filter(|f| !f.error) {
            eprintln!("  WARN part #{} [{}] {}", i + 1, f.field, f.message);
        }
        staged.push(part);
    }

    // Allocate IDs and insert.
    let mut child_ids: Vec<String> = Vec::new();
    for mut part in staged {
        let new_id = project.allocate_id();
        part.id = new_id.clone();
        project.requirements.insert(new_id.clone(), part);
        child_ids.push(new_id);
    }

    // Retire the original unless asked to keep it.
    if !args.keep_original {
        let o = project.requirements.get_mut(&args.id).unwrap();
        o.status = Status::Obsolete;
        o.updated = now;
        o.history.push(super::history(
            format!("split into {}", child_ids.join(", ")),
            args.reason.clone(),
        ));
    } else {
        let o = project.requirements.get_mut(&args.id).unwrap();
        o.history.push(super::history(
            format!(
                "split — created sibling parts {} (original kept active)",
                child_ids.join(", ")
            ),
            args.reason.clone(),
        ));
        o.updated = now;
    }

    project.updated = now;
    storage::save(&path, &project)?;

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "original": args.id,
                "retired": !args.keep_original,
                "parts": child_ids,
            }))?
        );
    } else if args.keep_original {
        println!(
            "Created {} part(s) from {} (original kept active): {}",
            child_ids.len(),
            args.id,
            child_ids.join(", ")
        );
    } else {
        println!(
            "Retired {} and created {} part(s): {}",
            args.id,
            child_ids.len(),
            child_ids.join(", ")
        );
    }
    Ok(())
}

fn prompt_for_parts(original: &Requirement) -> Result<Vec<String>> {
    let theme = ColorfulTheme::default();
    println!("Splitting {} — {}", original.id, original.title);
    println!();
    println!("Original statement:");
    println!("  {}", original.statement);
    println!();
    println!("Enter each new atomic statement on its own line. Empty line ends input.");
    let mut out: Vec<String> = Vec::new();
    loop {
        let label = format!("Part {} statement", out.len() + 1);
        let line: String = Input::with_theme(&theme)
            .with_prompt(label)
            .allow_empty(true)
            .interact_text()?;
        if line.trim().is_empty() {
            break;
        }
        out.push(line);
    }
    Ok(out)
}

fn synth_title(parent: &str, idx: usize, total: usize) -> String {
    let suffix = format!(" — part {} of {}", idx + 1, total);
    // Keep within title length budget (120 chars).
    let max_parent = 120usize.saturating_sub(suffix.chars().count());
    let mut t: String = parent.chars().take(max_parent).collect();
    t.push_str(&suffix);
    t
}
