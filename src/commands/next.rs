// Implements REQ-0040 (suggest one next requirement to work on,
// respecting depends_on prerequisites and filters).
use anyhow::{anyhow, Result};
use serde_json::json;
use std::path::PathBuf;

use crate::cli::NextArgs;
use crate::model::{Kind, LinkKind, Priority, Project, Requirement, Status};
use crate::storage::load_resolved;

pub fn run(args: NextArgs, file: &Option<PathBuf>) -> Result<()> {
    let (_, project) = load_resolved(file)?;
    let status: Option<Status> = args.status.map(Into::into);
    let kind: Option<Kind> = args.kind.map(Into::into);
    let priority: Option<Priority> = args.priority.map(Into::into);
    let tags = args.tag.clone();

    let mut candidates: Vec<&Requirement> = project
        .requirements
        .values()
        .filter(|r| match status {
            Some(s) => r.status == s,
            // Default: only show work that is not already done.
            None => !matches!(r.status, Status::Obsolete | Status::Verified),
        })
        .filter(|r| kind.is_none_or(|k| r.kind == k))
        .filter(|r| priority.is_none_or(|p| r.priority == p))
        .filter(|r| tags.iter().all(|t| r.tags.iter().any(|rt| rt == t)))
        .filter(|r| dependencies_satisfied(r, &project))
        .collect();

    if candidates.is_empty() {
        let msg = "no requirement matches the filters with all dependencies satisfied";
        if args.json {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({ "found": false, "message": msg }))?
            );
        } else {
            eprintln!("{}", msg);
        }
        std::process::exit(1);
    }

    // Sort by priority (Must > Should > Could > Wont), then by status freshness
    // (Draft first), then by ID for determinism.
    candidates.sort_by_key(|r| {
        let p = match r.priority {
            Priority::Must => 0,
            Priority::Should => 1,
            Priority::Could => 2,
            Priority::Wont => 3,
        };
        let s = match r.status {
            Status::Draft => 0,
            Status::Proposed => 1,
            Status::Approved => 2,
            Status::Implemented => 3,
            Status::Verified => 4,
            Status::Obsolete => 5,
        };
        (p, s, r.id.clone())
    });
    let pick = candidates[0];

    if args.json {
        println!("{}", serde_json::to_string_pretty(pick)?);
    } else {
        println!(
            "{} — {} [{} / {} / {}]",
            pick.id,
            pick.title,
            pick.kind.as_str(),
            pick.priority.as_str(),
            pick.status.as_str()
        );
        if !pick.acceptance.is_empty() {
            println!("\nAcceptance:");
            for (i, ac) in pick.acceptance.iter().enumerate() {
                println!("  {}. {}", i + 1, ac);
            }
        }
    }
    Ok(())
}

fn dependencies_satisfied(r: &Requirement, project: &Project) -> bool {
    for link in &r.links {
        if matches!(link.kind, LinkKind::DependsOn) {
            match project.requirements.get(&link.target) {
                Some(dep) if matches!(dep.status, Status::Implemented | Status::Verified) => {}
                _ => return false,
            }
        }
    }
    true
}

#[allow(dead_code)]
fn _unused() -> Result<()> {
    Err(anyhow!("unused"))
}
