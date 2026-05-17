// Implements REQ-0013 (reject parent links that would create a cycle).
use anyhow::{anyhow, Result};
use chrono::Utc;
use std::path::PathBuf;

use crate::cli::LinkArgs;
use crate::model::{Link, LinkKind};
use crate::storage::{self, load_resolved};

pub fn run(args: LinkArgs, file: &Option<PathBuf>) -> Result<()> {
    let (path, mut project) = load_resolved(file)?;

    if args.from == args.to {
        return Err(anyhow!("cannot link a requirement to itself"));
    }
    if !project.requirements.contains_key(&args.to) {
        return Err(anyhow!("target {} does not exist", args.to));
    }
    let kind: LinkKind = args.kind.into();

    if matches!(kind, LinkKind::Parent) && !args.remove {
        if creates_cycle(&project, &args.from, &args.to) {
            return Err(anyhow!(
                "linking {} -> parent {} would create a cycle",
                args.from,
                args.to
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
        if r.links.iter().any(|l| l.kind == kind && l.target == args.to) {
            return Err(anyhow!("link already exists"));
        }
        r.links.push(Link { kind, target: args.to.clone() });
        r.history.push(super::history(
            format!("added {} link to {}", kind.as_str(), args.to),
            None,
        ));
    }
    r.updated = Utc::now();
    project.updated = Utc::now();
    storage::save(&path, &project)?;
    println!("OK");
    Ok(())
}

fn creates_cycle(project: &crate::model::Project, from: &str, new_parent: &str) -> bool {
    let mut current = new_parent.to_string();
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
                .find(|l| l.kind == LinkKind::Parent)
                .map(|l| l.target.clone())
        });
        match next {
            Some(n) => current = n,
            None => return false,
        }
    }
}
