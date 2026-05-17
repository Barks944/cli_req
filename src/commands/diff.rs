// Implements REQ-0069: summarize per-requirement changes between two git
// revisions of project.req in a review-friendly form.
use anyhow::{anyhow, Context, Result};
use serde_json::json;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::Command;

use crate::cli::DiffArgs;
use crate::model::{Project, Requirement};
use crate::storage::{self, resolve_path};

pub fn run(args: DiffArgs, file: &Option<PathBuf>) -> Result<()> {
    let path = resolve_path(file);
    let filename = path.file_name().and_then(|s| s.to_str())
        .ok_or_else(|| anyhow!("project file has no name component"))?;

    let (base_ref, head_ref) = match args.spec.split_once("..") {
        Some((b, h)) => (b.trim(), h.trim()),
        None => return Err(anyhow!("spec must be BASE..HEAD, got '{}'", args.spec)),
    };
    let head_spec = if head_ref.is_empty() { "HEAD".to_string() } else { head_ref.to_string() };

    let base = load_at_ref(base_ref, filename)?;
    let head = load_at_ref(&head_spec, filename)?;

    let mut added: Vec<String> = Vec::new();
    let mut removed: Vec<String> = Vec::new();
    let mut changed: Vec<serde_json::Value> = Vec::new();

    let base_ids: BTreeSet<&String> = base.requirements.keys().collect();
    let head_ids: BTreeSet<&String> = head.requirements.keys().collect();

    for id in head_ids.difference(&base_ids) {
        added.push((*id).clone());
    }
    for id in base_ids.difference(&head_ids) {
        removed.push((*id).clone());
    }

    for id in base_ids.intersection(&head_ids) {
        let b = &base.requirements[*id];
        let h = &head.requirements[*id];
        let transitions = compare(b, h);
        if !transitions.is_empty() {
            let reason = latest_history_reason(h);
            changed.push(json!({
                "id": id, "transitions": transitions, "reason": reason,
            }));
        }
    }

    if added.is_empty() && removed.is_empty() && changed.is_empty() {
        if args.json {
            println!("{}", json!({ "spec": args.spec, "empty": true }));
        } else {
            println!("req diff {}: no requirement-level changes.", args.spec);
        }
        return Ok(());
    }

    if args.json {
        println!("{}", serde_json::to_string_pretty(&json!({
            "spec": args.spec,
            "added": added,
            "removed": removed,
            "changed": changed,
        }))?);
        return Ok(());
    }

    println!("req diff {} → {}", base_ref, head_spec);
    if !added.is_empty() {
        println!("\nADDED ({})", added.len());
        for id in &added {
            let r = &head.requirements[id];
            println!("  + {} — {} [{} / {} / {}]",
                id, r.title, r.kind.as_str(), r.priority.as_str(), r.status.as_str());
        }
    }
    if !removed.is_empty() {
        println!("\nREMOVED ({})", removed.len());
        for id in &removed {
            let r = &base.requirements[id];
            println!("  - {} — {}", id, r.title);
        }
    }
    if !changed.is_empty() {
        println!("\nCHANGED ({})", changed.len());
        for entry in &changed {
            let id = entry["id"].as_str().unwrap();
            println!("  ~ {}", id);
            for t in entry["transitions"].as_array().unwrap() {
                println!("      {}", t.as_str().unwrap());
            }
            if let Some(reason) = entry["reason"].as_str() {
                if !reason.is_empty() {
                    println!("      reason: {}", reason);
                }
            }
        }
    }
    Ok(())
}

fn load_at_ref(reference: &str, filename: &str) -> Result<Project> {
    let spec = format!("{}:{}", reference, filename);
    let out = Command::new("git").args(["show", &spec]).output()
        .with_context(|| format!("git show {}", spec))?;
    if !out.status.success() {
        return Err(anyhow!("git show {} failed: {}", spec, String::from_utf8_lossy(&out.stderr)));
    }
    let tmp = std::env::temp_dir().join(format!("req-diff-{}-{}.req", reference.replace('/', "_"), std::process::id()));
    std::fs::write(&tmp, &out.stdout)?;
    let project = storage::load_with_options(&tmp, true)?;
    std::fs::remove_file(&tmp).ok();
    Ok(project)
}

fn compare(base: &Requirement, head: &Requirement) -> Vec<String> {
    let mut out = Vec::new();
    if base.title != head.title { out.push(format!("title: {:?} -> {:?}", base.title, head.title)); }
    if base.status != head.status { out.push(format!("status: {} -> {}", base.status.as_str(), head.status.as_str())); }
    if base.priority != head.priority { out.push(format!("priority: {} -> {}", base.priority.as_str(), head.priority.as_str())); }
    if base.kind != head.kind { out.push(format!("kind: {} -> {}", base.kind.as_str(), head.kind.as_str())); }
    if base.statement != head.statement { out.push("statement: changed".into()); }
    if base.rationale != head.rationale { out.push("rationale: changed".into()); }
    if base.acceptance != head.acceptance {
        out.push(format!("acceptance: {} -> {} items", base.acceptance.len(), head.acceptance.len()));
    }
    if base.tags != head.tags { out.push(format!("tags: {:?} -> {:?}", base.tags, head.tags)); }
    if base.links.len() != head.links.len() { out.push(format!("links: {} -> {}", base.links.len(), head.links.len())); }
    out
}

fn latest_history_reason(r: &Requirement) -> String {
    r.history.iter().rev()
        .find_map(|h| h.reason.clone())
        .unwrap_or_default()
}
