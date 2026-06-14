// Implements REQ-0013 (reject parent links that would create a cycle).
use anyhow::{anyhow, Result};
use chrono::Utc;
use std::path::PathBuf;

use crate::cli::LinkArgs;
use crate::model::{Link, LinkKind};
use crate::storage::{self, load_for_mutation};

pub fn run(mut args: LinkArgs, file: &Option<PathBuf>) -> Result<()> {
    let (path, mut project, _lock) = load_for_mutation(file)?;
    args.from = super::resolve_id(&project, &args.from)?;
    args.to = super::resolve_id(&project, &args.to)?;

    if args.from == args.to {
        return Err(anyhow!("cannot link a requirement to itself"));
    }
    let kind: LinkKind = args.kind.into();

    // Cycle-check every asymmetric link kind. `conflicts` is symmetric
    // (A conflicts with B == B conflicts with A) so a "cycle" is just a
    // duplicate — caught by the duplicate-link check below.
    let cycle_checked = matches!(
        kind,
        LinkKind::Parent | LinkKind::DependsOn | LinkKind::Refines | LinkKind::Verifies
    );
    if cycle_checked && !args.remove {
        // REQ-0166: reject the closing edge at link time and name the cycle
        // path, instead of leaving it for the next full `req validate`.
        if let Some(path) = cycle_path(&project, &args.from, &args.to, kind) {
            let chain = std::iter::once(args.from.clone())
                .chain(path)
                .collect::<Vec<_>>()
                .join(" -> ");
            return Err(anyhow!(
                "linking {} -> {} {} would create a cycle: {}",
                args.from,
                kind.as_str(),
                args.to,
                chain
            ));
        }
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

/// REQ-0166: depth-first search forward along *every* same-kind link from
/// `target`; if the search reaches `from`, adding `from -> target` would
/// close a cycle, and we return the path `[target, …, from]` so the caller
/// can name it. The earlier walker followed only the first same-kind edge
/// per node, so it silently missed cycles that closed through any other
/// branch. A global visited set is sound here because reachability is
/// monotonic: a node that cannot reach `from` cannot do so via another path.
fn cycle_path(
    project: &crate::model::Project,
    from: &str,
    target: &str,
    kind: LinkKind,
) -> Option<Vec<String>> {
    use std::collections::HashSet;
    let mut stack: Vec<(String, Vec<String>)> = vec![(target.to_string(), vec![target.to_string()])];
    let mut visited: HashSet<String> = HashSet::new();
    while let Some((node, path)) = stack.pop() {
        if node == from {
            return Some(path);
        }
        if !visited.insert(node.clone()) {
            continue;
        }
        if let Some(r) = project.requirements.get(&node) {
            for l in r.links.iter().filter(|l| l.kind == kind) {
                let mut next_path = path.clone();
                next_path.push(l.target.clone());
                stack.push((l.target.clone(), next_path));
            }
        }
    }
    None
}
