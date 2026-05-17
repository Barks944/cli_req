// Implements REQ-0012 (soft-delete by default; --hard refuses on inbound links).
use anyhow::{anyhow, Result};
use chrono::Utc;
use std::path::PathBuf;

use crate::cli::DeleteArgs;
use crate::model::Status;
use crate::storage::{self, load_resolved};

pub fn run(args: DeleteArgs, file: &Option<PathBuf>) -> Result<()> {
    let (path, mut project) = load_resolved(file)?;

    if !project.requirements.contains_key(&args.id) {
        return Err(anyhow!("no such requirement: {}", args.id));
    }

    let referenced_by: Vec<String> = project
        .requirements
        .values()
        .filter(|r| r.links.iter().any(|l| l.target == args.id))
        .map(|r| r.id.clone())
        .collect();

    if args.hard {
        if !referenced_by.is_empty() {
            return Err(anyhow!(
                "{} is referenced by {} — remove links before hard-delete",
                args.id,
                referenced_by.join(", ")
            ));
        }
        project.requirements.remove(&args.id);
        println!("Hard-deleted {}", args.id);
    } else {
        let r = project.requirements.get_mut(&args.id).unwrap();
        r.status = Status::Obsolete;
        r.updated = Utc::now();
        r.history.push(super::history("marked obsolete", args.reason));
        println!("Marked {} obsolete (links preserved)", args.id);
    }

    project.updated = Utc::now();
    storage::save(&path, &project)?;
    Ok(())
}
