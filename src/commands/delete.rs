// Implements REQ-0012 (soft-delete by default; --hard refuses on inbound links).
use anyhow::{anyhow, Result};
use chrono::Utc;
use std::path::PathBuf;

use crate::cli::DeleteArgs;
use crate::model::Status;
use crate::storage::{self, load_for_mutation};

pub fn run(args: DeleteArgs, file: &Option<PathBuf>) -> Result<()> {
    let (path, mut project, _lock) = load_for_mutation(file)?;

    if !project.requirements.contains_key(&args.id) {
        return Err(anyhow!("no such requirement: {}", args.id));
    }

    let referenced_by: Vec<String> = project
        .requirements
        .values()
        .filter(|r| r.links.iter().any(|l| l.target == args.id))
        .map(|r| r.id.clone())
        .collect();

    let mode;
    if args.hard {
        if !referenced_by.is_empty() {
            return Err(anyhow!(
                "{} is referenced by {} — remove links before hard-delete",
                args.id,
                referenced_by.join(", ")
            ));
        }
        project.requirements.remove(&args.id);
        mode = "hard";
        if !args.json { println!("Hard-deleted {}", args.id); }
    } else {
        let r = project.requirements.get_mut(&args.id).unwrap();
        r.status = Status::Obsolete;
        r.updated = Utc::now();
        r.history.push(super::history("marked obsolete", args.reason));
        mode = "soft";
        if !args.json { println!("Marked {} obsolete (links preserved)", args.id); }
    }

    project.updated = Utc::now();
    storage::save(&path, &project)?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&serde_json::json!({
            "id": args.id, "deleted": mode
        }))?);
    }
    Ok(())
}
