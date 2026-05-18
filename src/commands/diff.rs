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
    let filename = path
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| anyhow!("project file has no name component"))?;

    // Accept three shapes: `BASE..HEAD` (canonical), `BASE..` (head =
    // working HEAD), and a single ref `BASE` as shorthand for
    // `BASE..HEAD`. The single-ref form matches `git diff <ref>` muscle
    // memory.
    let (base_ref, head_ref) = match args.spec.split_once("..") {
        Some((b, h)) => (b.trim(), h.trim()),
        None => (args.spec.trim(), ""),
    };
    if base_ref.is_empty() {
        return Err(anyhow!(
            "diff spec needs a base ref; pass `BASE..HEAD`, `BASE..`, or a single `BASE`"
        ));
    }
    // Common confusion: passing a requirement ID as the spec. Catch it
    // here so users see a useful hint instead of git's
    // `fatal: invalid object name`.
    if looks_like_req_id(base_ref) || looks_like_req_id(head_ref) {
        return Err(anyhow!(
            "`{}` looks like a requirement ID, not a git rev. \
             `req diff` takes a git ref or BASE..HEAD pair (e.g. `HEAD~1`, \
             `origin/main..HEAD`). Use `req show {}` to inspect a single \
             requirement.",
            args.spec,
            base_ref,
        ));
    }
    let head_spec = if head_ref.is_empty() {
        "HEAD".to_string()
    } else {
        head_ref.to_string()
    };

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
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "spec": args.spec,
                "added": added,
                "removed": removed,
                "changed": changed,
            }))?
        );
        return Ok(());
    }

    println!("req diff {} → {}", base_ref, head_spec);
    if !added.is_empty() {
        println!("\nADDED ({})", added.len());
        for id in &added {
            let r = &head.requirements[id];
            println!(
                "  + {} — {} [{} / {} / {}]",
                id,
                r.title,
                r.kind.as_str(),
                r.priority.as_str(),
                r.status.as_str()
            );
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

fn looks_like_req_id(s: &str) -> bool {
    let up = s.to_uppercase();
    up.starts_with("REQ-")
        && up
            .strip_prefix("REQ-")
            .map(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()))
            .unwrap_or(false)
}

fn load_at_ref(reference: &str, filename: &str) -> Result<Project> {
    let spec = format!("{}:{}", reference, filename);
    let out = Command::new("git")
        .args(["show", &spec])
        .output()
        .with_context(|| format!("git show {}", spec))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        // REQ-0069: friendly hint when git's error is opaque. On a
        // shallow checkout or a fresh repo, `req diff HEAD~1` produces
        // git's raw `fatal: invalid object name 'HEAD~1'` — useless
        // without context.
        if stderr.contains("invalid object name") || stderr.contains("unknown revision") {
            return Err(anyhow!(
                "git cannot resolve `{}` — does the ref exist in this clone? On a fresh repo with one commit, `HEAD~1` has no parent. (git said: {})",
                reference,
                stderr.trim()
            ));
        }
        return Err(anyhow!("git show {} failed: {}", spec, stderr));
    }
    let tmp = std::env::temp_dir().join(format!(
        "req-diff-{}-{}.req",
        reference.replace('/', "_"),
        std::process::id()
    ));
    std::fs::write(&tmp, &out.stdout)?;
    let project = storage::load_with_options(&tmp, true)?;
    std::fs::remove_file(&tmp).ok();
    Ok(project)
}

fn compare(base: &Requirement, head: &Requirement) -> Vec<String> {
    let mut out = Vec::new();
    if base.title != head.title {
        out.push(format!("title: {:?} -> {:?}", base.title, head.title));
    }
    if base.status != head.status {
        out.push(format!(
            "status: {} -> {}",
            base.status.as_str(),
            head.status.as_str()
        ));
    }
    if base.priority != head.priority {
        out.push(format!(
            "priority: {} -> {}",
            base.priority.as_str(),
            head.priority.as_str()
        ));
    }
    if base.kind != head.kind {
        out.push(format!(
            "kind: {} -> {}",
            base.kind.as_str(),
            head.kind.as_str()
        ));
    }
    if base.statement != head.statement {
        out.push("statement: changed".into());
    }
    if base.rationale != head.rationale {
        out.push("rationale: changed".into());
    }
    if base.acceptance != head.acceptance {
        out.push(format!(
            "acceptance: {} -> {} items",
            base.acceptance.len(),
            head.acceptance.len()
        ));
    }
    if base.tags != head.tags {
        out.push(format!("tags: {:?} -> {:?}", base.tags, head.tags));
    }
    if base.links.len() != head.links.len() {
        out.push(format!(
            "links: {} -> {}",
            base.links.len(),
            head.links.len()
        ));
    }
    out
}

fn latest_history_reason(r: &Requirement) -> String {
    r.history
        .iter()
        .rev()
        .find_map(|h| h.reason.clone())
        .unwrap_or_default()
}
