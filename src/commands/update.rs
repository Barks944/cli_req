// Discharges REQ-0001 (update sub-surface) and contributes to REQ-0011 (history).
use anyhow::{anyhow, Result};
use chrono::Utc;
use std::path::PathBuf;

use crate::cli::UpdateArgs;
use crate::storage::{self, load_resolved};
use crate::validate;

pub fn run(args: UpdateArgs, file: &Option<PathBuf>) -> Result<()> {
    let (path, mut project) = load_resolved(file)?;
    let r = project
        .requirements
        .get_mut(&args.id)
        .ok_or_else(|| anyhow!("no such requirement: {}", args.id))?;

    let mut changes: Vec<String> = Vec::new();

    if let Some(t) = args.title {
        if r.title != t {
            changes.push(format!("title: {:?} -> {:?}", r.title, t));
            r.title = t;
        }
    }
    if let Some(s) = args.statement {
        if r.statement != s {
            changes.push("statement updated".into());
            r.statement = s;
        }
    }
    if let Some(rt) = args.rationale {
        if r.rationale != rt {
            changes.push("rationale updated".into());
            r.rationale = rt;
        }
    }
    if let Some(ac) = args.acceptance {
        changes.push(format!("acceptance replaced ({} items)", ac.len()));
        r.acceptance = ac;
    }
    for ac in &args.add_acceptance {
        r.acceptance.push(ac.clone());
        changes.push(format!("+acceptance #{}: {:?}", r.acceptance.len(), ac));
    }
    let mut to_remove = args.remove_acceptance.clone();
    to_remove.sort_unstable();
    to_remove.dedup();
    to_remove.reverse();
    for idx_1 in to_remove {
        if idx_1 == 0 || idx_1 > r.acceptance.len() {
            return Err(anyhow!(
                "--remove-acceptance index {} is out of range (1..={})",
                idx_1,
                r.acceptance.len()
            ));
        }
        let removed = r.acceptance.remove(idx_1 - 1);
        changes.push(format!("-acceptance #{}: {:?}", idx_1, removed));
    }
    if let Some(k) = args.kind {
        let k = k.into();
        if r.kind != k {
            changes.push(format!("kind {} -> {}", r.kind.as_str(), {
                let nk: crate::model::Kind = k;
                nk.as_str()
            }));
            r.kind = k;
        }
    }
    if let Some(p) = args.priority {
        let p = p.into();
        if r.priority != p {
            changes.push(format!("priority {} -> {}", r.priority.as_str(), {
                let np: crate::model::Priority = p;
                np.as_str()
            }));
            r.priority = p;
        }
    }
    if let Some(s) = args.status {
        let s = s.into();
        if r.status != s {
            changes.push(format!("status {} -> {}", r.status.as_str(), {
                let ns: crate::model::Status = s;
                ns.as_str()
            }));
            r.status = s;
        }
    }
    for t in &args.add_tag {
        if !r.tags.iter().any(|x| x == t) {
            r.tags.push(t.clone());
            changes.push(format!("+tag {}", t));
        }
    }
    for t in &args.remove_tag {
        if let Some(pos) = r.tags.iter().position(|x| x == t) {
            r.tags.remove(pos);
            changes.push(format!("-tag {}", t));
        }
    }

    if changes.is_empty() {
        println!("No changes.");
        return Ok(());
    }

    let findings = validate::validate_requirement(r);
    let errors = validate::errors_only(&findings);
    if !errors.is_empty() {
        eprintln!("Validation errors block save:");
        for f in &errors {
            eprintln!("  ERR [{}] {}", f.field, f.message);
        }
        return Err(anyhow!("update would violate requirements rules"));
    }
    for f in findings.iter().filter(|f| !f.error) {
        eprintln!("  WARN [{}] {}", f.field, f.message);
    }

    r.updated = Utc::now();
    r.history.push(super::history(changes.join("; "), args.reason));
    project.updated = Utc::now();
    storage::save(&path, &project)?;
    println!("Updated {}", args.id);
    Ok(())
}
