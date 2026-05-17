// Discharges REQ-0001 (show sub-surface).
use anyhow::{anyhow, Result};
use std::path::PathBuf;

use crate::cli::ShowArgs;
use crate::model::Requirement;
use crate::storage::load_resolved;

pub fn run(args: ShowArgs, file: &Option<PathBuf>) -> Result<()> {
    let (_, project) = load_resolved(file)?;
    let r = project
        .requirements
        .get(&args.id)
        .ok_or_else(|| anyhow!("no such requirement: {}", args.id))?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(r)?);
        return Ok(());
    }

    render(r);
    Ok(())
}

pub fn render(r: &Requirement) {
    println!("{} — {}", r.id, r.title);
    println!("{}", "-".repeat(60));
    println!("Kind     : {}", r.kind.as_str());
    println!("Priority : {}", r.priority.as_str());
    println!("Status   : {}", r.status.as_str());
    if !r.tags.is_empty() {
        println!("Tags     : {}", r.tags.join(", "));
    }
    println!("Created  : {}", r.created.format("%Y-%m-%d %H:%M UTC"));
    println!("Updated  : {}", r.updated.format("%Y-%m-%d %H:%M UTC"));
    println!();
    println!("Statement:");
    println!("  {}", r.statement);
    println!();
    println!("Rationale:");
    println!("  {}", r.rationale);
    if !r.acceptance.is_empty() {
        println!();
        println!("Acceptance:");
        for (i, ac) in r.acceptance.iter().enumerate() {
            println!("  {}. {}", i + 1, ac);
        }
    }
    if !r.links.is_empty() {
        println!();
        println!("Links:");
        for l in &r.links {
            println!("  {} -> {}", l.kind.as_str(), l.target);
        }
    }
    println!();
    println!("Test records:");
    if r.tests.is_empty() {
        println!("  (no test records)");
    } else {
        let head = super::test_cmd::current_head_sha_opt();
        for (i, t) in r.tests.iter().enumerate() {
            let is_latest = i + 1 == r.tests.len();
            let drift = if is_latest {
                match &head {
                    Some(h) if *h == t.commit => " [matches HEAD]".to_string(),
                    Some(h) => format!(" [drifted — HEAD now {}]", super::test_cmd::short(h)),
                    None => " [HEAD unknown — not in a git tree]".to_string(),
                }
            } else { String::new() };
            let notes = if t.notes.is_empty() { String::new() } else { format!(" — {}", t.notes) };
            println!(
                "  {} {} commit={} actor={}{}{}",
                t.at.format("%Y-%m-%d %H:%M"),
                t.outcome.as_str().to_uppercase(),
                super::test_cmd::short(&t.commit),
                t.actor,
                drift,
                notes,
            );
        }
    }

    if !r.history.is_empty() {
        println!();
        println!("History:");
        for h in &r.history {
            let r = h.reason.as_deref().unwrap_or("");
            let kind_tag = match h.actor_kind {
                crate::model::ActorKind::Unknown => String::new(),
                k => format!(" ({})", k.as_str()),
            };
            println!(
                "  {} {}{} {} {}",
                h.at.format("%Y-%m-%d %H:%M"),
                h.actor,
                kind_tag,
                h.action,
                if r.is_empty() { String::new() } else { format!("— {}", r) }
            );
        }
    }
}
