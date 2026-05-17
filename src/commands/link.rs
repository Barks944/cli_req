// Implements REQ-0013 (reject parent links that would create a cycle).
use anyhow::{anyhow, Result};
use chrono::Utc;
use std::path::PathBuf;

use crate::cli::LinkArgs;
use crate::model::{Link, LinkKind};
use crate::storage::{self, load_for_mutation};

pub fn run(args: LinkArgs, file: &Option<PathBuf>) -> Result<()> {
    let (path, mut project, _lock) = load_for_mutation(file)?;

    if args.from == args.to {
        return Err(anyhow!("cannot link a requirement to itself"));
    }
    if !project.requirements.contains_key(&args.to) {
        return Err(anyhow!("target {} does not exist", args.to));
    }
    let kind: LinkKind = args.kind.into();

    // Cycle-check every asymmetric link kind. `conflicts` is symmetric
    // (A conflicts with B == B conflicts with A) so a "cycle" is just a
    // duplicate — caught by the duplicate-link check below.
    let cycle_checked = matches!(
        kind,
        LinkKind::Parent | LinkKind::DependsOn | LinkKind::Refines | LinkKind::Verifies
    );
    if cycle_checked && !args.remove && creates_cycle(&project, &args.from, &args.to, kind) {
        return Err(anyhow!(
            "linking {} -> {} {} would create a cycle",
            args.from,
            kind.as_str(),
            args.to
        ));
    }

    let r = project
        .requirements
        .get_mut(&args.from)
        .ok_or_else(|| anyhow!("source {} does not exist", args.from))?;

    if args.remove {
        let before = r.links.len();
        r.links.retain(|l| !(l.kind == kind && l.target == args.to));
        if r.links.len() == before {
            return Err(anyhow!("no such link to remove"));
        }
        r.history.push(super::history(
            format!("removed {} link to {}", kind.as_str(), args.to),
            None,
        ));
    } else {
        if r.links
            .iter()
            .any(|l| l.kind == kind && l.target == args.to)
        {
            return Err(anyhow!("link already exists"));
        }
        r.links.push(Link {
            kind,
            target: args.to.clone(),
        });
        r.history.push(super::history(
            format!("added {} link to {}", kind.as_str(), args.to),
            None,
        ));
    }
    r.updated = Utc::now();
    project.updated = Utc::now();
    storage::save(&path, &project)?;
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "from": args.from, "to": args.to,
                "kind": kind.as_str(), "removed": args.remove
            }))?
        );
    } else {
        println!("OK");
    }
    Ok(())
}

/// Walk forward along same-kind links from `target` and report whether the
/// chain reaches `from` (which would close a cycle once the new link is
/// added). Generalised from the original parent-only walker so every
/// asymmetric link kind gets the same protection.
fn creates_cycle(
    project: &crate::model::Project,
    from: &str,
    target: &str,
    kind: LinkKind,
) -> bool {
    let mut current = target.to_string();
    let mut visited = Vec::new();
    loop {
        if current == from {
            return true;
        }
        if visited.contains(&current) {
            return false;
        }
        visited.push(current.clone());
        let next = project.requirements.get(&current).and_then(|r| {
            r.links
                .iter()
                .find(|l| l.kind == kind)
                .map(|l| l.target.clone())
        });
        match next {
            Some(n) => current = n,
            None => return false,
        }
    }
}
