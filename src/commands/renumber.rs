// Implements REQ-0025 (renumber colliding IDs after merge, rewrite links,
// record history, support --dry-run).
use anyhow::{anyhow, Context, Result};
use std::path::PathBuf;
use std::process::Command;

use crate::cli::RenumberArgs;
use crate::model::{Project, Requirement};
use crate::storage::{self, load_with_options, resolve_path};

pub fn run(args: RenumberArgs, file: &Option<PathBuf>) -> Result<()> {
    let path = resolve_path(file);
    // After a merge resolution the integrity hash is typically wrong — force load.
    let mut current = load_with_options(&path, true)?;
    let base = load_from_git_ref(&args.base, path.file_name().unwrap().to_str().unwrap())?;

    let mut renames: Vec<(String, String)> = Vec::new();
    let mut next_id = current.next_id.max(base.next_id);

    // Collisions: same ID exists in both, but content differs from base's version.
    // These are entries that were added on our side but the ID was also taken on base.
    let candidate_ids: Vec<String> = current.requirements.keys().cloned().collect();
    for id in candidate_ids {
        let base_has = base.requirements.contains_key(&id);
        if !base_has {
            continue;
        }
        let differs = match (current.requirements.get(&id), base.requirements.get(&id)) {
            (Some(a), Some(b)) => a.created != b.created || a.title != b.title,
            _ => false,
        };
        if differs {
            let new_id = format!("REQ-{:04}", next_id);
            next_id += 1;
            renames.push((id, new_id));
        }
    }

    if renames.is_empty() {
        println!("No ID collisions against {}.", args.base);
        return Ok(());
    }

    println!("Planned renames:");
    for (old, new) in &renames {
        println!("  {} -> {}", old, new);
    }
    if args.dry_run {
        return Ok(());
    }

    apply_renames(&mut current, &renames);
    current.next_id = next_id;
    storage::save(&path, &current)?;
    println!("Renumbered {} requirement(s) and re-signed {}.", renames.len(), path.display());
    Ok(())
}

fn apply_renames(project: &mut Project, renames: &[(String, String)]) {
    let map: std::collections::HashMap<String, String> =
        renames.iter().cloned().collect();

    let mut taken: Vec<Requirement> = Vec::new();
    for (old, _) in renames {
        if let Some(mut r) = project.requirements.remove(old) {
            let new_id = map.get(old).unwrap().clone();
            r.id = new_id.clone();
            r.history.push(super::history(
                format!("renumbered from {} (merge with base)", old),
                None,
            ));
            taken.push(r);
        }
    }
    for r in taken {
        project.requirements.insert(r.id.clone(), r);
    }

    // Rewrite link targets.
    for r in project.requirements.values_mut() {
        for link in r.links.iter_mut() {
            if let Some(new) = map.get(&link.target) {
                link.target = new.clone();
            }
        }
    }
}

fn load_from_git_ref(reference: &str, filename: &str) -> Result<Project> {
    let spec = format!("{}:{}", reference, filename);
    let output = Command::new("git")
        .args(["show", &spec])
        .output()
        .with_context(|| format!("run git show {}", spec))?;
    if !output.status.success() {
        return Err(anyhow!(
            "git show {} failed: {}",
            spec,
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let tmp = std::env::temp_dir().join(format!("req-base-{}.req", std::process::id()));
    std::fs::write(&tmp, &output.stdout)?;
    let project = load_with_options(&tmp, true)?;
    std::fs::remove_file(&tmp).ok();
    Ok(project)
}
