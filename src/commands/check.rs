// Implements REQ-0041 (incremental validation + scoped coverage since a git ref).
use anyhow::{anyhow, Context, Result};
use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::json;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::Command;

use crate::cli::CheckArgs;
use crate::model::Project;
use crate::storage::{self, resolve_path};
use crate::validate;

static REQ_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"REQ-\d{4}").unwrap());

pub fn run(args: CheckArgs, file: &Option<PathBuf>) -> Result<()> {
    let path = resolve_path(file);
    let current = storage::load(&path).context("load current project.req")?;
    let base = match load_base(&args.base, &path) {
        Ok(p) => Some(p),
        Err(e) => {
            // If the base ref doesn't exist, exit non-zero with a clear message.
            if args.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({
                        "ok": false,
                        "base": args.base,
                        "error": e.to_string(),
                    }))?
                );
            } else {
                eprintln!("req check: {}", e);
            }
            std::process::exit(2);
        }
    };

    // Which requirements changed?
    let changed_reqs: Vec<String> = changed_req_ids(&current, base.as_ref());

    // Validate just those requirements.
    let mut findings: Vec<serde_json::Value> = Vec::new();
    let mut errs = 0usize;
    let mut warns = 0usize;
    for id in &changed_reqs {
        if let Some(r) = current.requirements.get(id) {
            for f in validate::validate_requirement(r) {
                if f.error {
                    errs += 1
                } else {
                    warns += 1
                }
                findings.push(json!({
                    "req_id": id,
                    "rule_code": f.rule_code,
                    "field": f.field,
                    "severity": if f.error { "error" } else { "warning" },
                    "message": f.message,
                }));
            }
        }
    }

    // Which source files changed?
    let changed_files = git_changed_files(&args.base).unwrap_or_default();
    let mut coverage: Vec<serde_json::Value> = Vec::new();
    for f in &changed_files {
        let full = args.path.join(f);
        if let Ok(text) = std::fs::read_to_string(&full) {
            let ids: BTreeSet<String> = REQ_RE
                .find_iter(&text)
                .map(|m| m.as_str().to_string())
                .collect();
            let unknown: Vec<&String> = ids
                .iter()
                .filter(|id| !current.requirements.contains_key(*id))
                .collect();
            coverage.push(json!({
                "file": full.display().to_string(),
                "req_ids": ids,
                "unknown_ids": unknown,
                "has_markers": !ids.is_empty(),
            }));
        }
    }

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "ok": errs == 0,
                "base": args.base,
                "changed_requirements": changed_reqs,
                "errors": errs,
                "warnings": warns,
                "findings": findings,
                "changed_files": changed_files,
                "coverage": coverage,
            }))?
        );
    } else {
        println!("req check {}", args.base);
        println!("  changed requirements : {}", changed_reqs.len());
        println!("  changed files        : {}", changed_files.len());
        println!("  errors / warnings    : {} / {}", errs, warns);
        if !findings.is_empty() {
            println!("\nFindings:");
            for f in &findings {
                println!(
                    "  {} {} [{}] {}",
                    f["req_id"].as_str().unwrap_or("?"),
                    f["rule_code"].as_str().unwrap_or("?"),
                    f["severity"].as_str().unwrap_or("?"),
                    f["message"].as_str().unwrap_or("?")
                );
            }
        }
    }

    if errs > 0 {
        std::process::exit(1);
    }
    Ok(())
}

fn load_base(base: &str, current_path: &std::path::Path) -> Result<Project> {
    let filename = current_path
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| anyhow!("project file path has no name component"))?;
    let spec = format!("{}:{}", base, filename);
    let out = Command::new("git")
        .args(["show", &spec])
        .output()
        .with_context(|| format!("git show {}", spec))?;
    if !out.status.success() {
        return Err(anyhow!(
            "git show {} failed: {}",
            spec,
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let tmp = std::env::temp_dir().join(format!("req-check-base-{}.req", std::process::id()));
    std::fs::write(&tmp, &out.stdout)?;
    let project = storage::load_with_options(&tmp, true)?;
    std::fs::remove_file(&tmp).ok();
    Ok(project)
}

fn changed_req_ids(current: &Project, base: Option<&Project>) -> Vec<String> {
    match base {
        None => current.requirements.keys().cloned().collect(),
        Some(b) => current
            .requirements
            .iter()
            .filter(|(id, r)| match b.requirements.get(*id) {
                None => true,
                Some(prev) => {
                    prev.updated != r.updated
                        || prev.title != r.title
                        || prev.statement != r.statement
                        || prev.rationale != r.rationale
                        || prev.acceptance != r.acceptance
                        || prev.status != r.status
                        || prev.priority != r.priority
                        || prev.kind != r.kind
                        || prev.links.len() != r.links.len()
                }
            })
            .map(|(id, _)| id.clone())
            .collect(),
    }
}

fn git_changed_files(base: &str) -> Result<Vec<String>> {
    let out = Command::new("git")
        .args(["diff", "--name-only", &format!("{}...HEAD", base)])
        .output()?;
    if !out.status.success() {
        return Err(anyhow!(
            "git diff failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect())
}
