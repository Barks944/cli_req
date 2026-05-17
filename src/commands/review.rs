// REQ-0086: req review — one-shot PR-style spec impact report.
// Single-shot "what should this PR have done with the spec?" report.
// Wraps validate, coverage, stale, audit, and the changed-requirement
// diff into one markdown (or JSON) document scoped to <base>..HEAD.
// Designed to be pasted into a PR description or piped into a CI
// comment.
use anyhow::{anyhow, Context, Result};
use serde_json::json;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::cli::ReviewArgs;
use crate::model::Project;
use crate::storage::{self, resolve_path};
use crate::validate;

pub fn run(args: ReviewArgs, file: &Option<PathBuf>) -> Result<()> {
    let path = resolve_path(file);
    let current = storage::load(&path).context("load project.req")?;
    let base_ref = resolve_base(&args.base);

    let base = load_at_ref(&base_ref, &path).ok();

    // --- changed requirements diff -----------------------------------
    let (added, removed, changed) = diff_buckets(base.as_ref(), &current);

    // --- validate (full project) -------------------------------------
    let val_findings = validate::validate_project(&current);
    let val_errors: usize = val_findings
        .iter()
        .flat_map(|(_, fs)| fs.iter())
        .filter(|f| f.error)
        .count();
    let val_warnings: usize = val_findings
        .iter()
        .flat_map(|(_, fs)| fs.iter())
        .filter(|f| !f.error)
        .count();

    // --- coverage on changed files -----------------------------------
    // Also surface "changed source file with zero REQ markers" — the
    // missing-spec-for-new-code signal. Without this the validator
    // can only check that recorded REQs are well-formed; new behaviour
    // can ship without any backing requirement and nothing catches it.
    // Source extensions match coverage's defaults so the two checks
    // see the same file universe.
    let source_exts: &[&str] = &["rs", "py", "js", "ts", "tsx", "go", "java", "c", "cpp", "h"];
    let changed_files = git_changed_files(&base_ref).unwrap_or_default();
    let mut coverage_ghosts: Vec<String> = Vec::new();
    let mut coverage_referenced: BTreeSet<String> = BTreeSet::new();
    let mut markerless_changed_source: Vec<String> = Vec::new();
    let req_re = regex::Regex::new(r"REQ-\d{4}").unwrap();
    for f in &changed_files {
        let full = args.path.join(f);
        let is_source = full
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| source_exts.contains(&e))
            .unwrap_or(false);
        match std::fs::read_to_string(&full) {
            Ok(text) => {
                let mut saw_marker = false;
                for cap in req_re.find_iter(&text) {
                    saw_marker = true;
                    let id = cap.as_str().to_string();
                    if !current.requirements.contains_key(&id) {
                        coverage_ghosts.push(format!("{} (in {})", id, full.display()));
                    } else {
                        coverage_referenced.insert(id);
                    }
                }
                if is_source && !saw_marker {
                    markerless_changed_source.push(full.display().to_string());
                }
            }
            Err(_) => {
                // Likely a deleted-in-HEAD file; not a coverage problem.
            }
        }
    }

    // --- stale records ------------------------------------------------
    // Reuse the staleness scanner but only summarise counts here; the
    // full table is what `req stale` is for.
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let mut stale_count = 0usize;
    let mut drifted_count = 0usize;
    for r in current.requirements.values() {
        if let Some(latest) = r.tests.last() {
            use crate::commands::test_cmd::Staleness;
            match crate::commands::test_cmd::staleness(&latest.commit, &r.id, &cwd) {
                Staleness::Stale { .. } => stale_count += 1,
                Staleness::Drifted { .. } => drifted_count += 1,
                _ => {}
            }
        }
    }

    // --- audit headline ----------------------------------------------
    let audit_summary = audit_summary_for_file(&path);

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "base": base_ref,
                "added": added,
                "removed": removed,
                "changed": changed,
                "validate": {
                    "errors": val_errors,
                    "warnings": val_warnings,
                    "findings": val_findings.iter().map(|(id, fs)| {
                        json!({
                            "id": id,
                            "findings": fs.iter().map(|f| json!({
                                "rule_code": f.rule_code,
                                "field": f.field,
                                "severity": if f.error { "error" } else { "warning" },
                                "message": f.message,
                            })).collect::<Vec<_>>(),
                        })
                    }).collect::<Vec<_>>(),
                },
                "coverage": {
                    "ghosts": coverage_ghosts,
                    "referenced": coverage_referenced,
                    "changed_files": changed_files,
                    "markerless_changed_source": markerless_changed_source,
                },
                "stale": {
                    "stale": stale_count,
                    "drifted": drifted_count,
                },
                "audit": audit_summary,
            }))?
        );
        let gate_fail =
            val_errors > 0 || !coverage_ghosts.is_empty() || !markerless_changed_source.is_empty();
        if val_errors > 0 || (args.gate && gate_fail) {
            std::process::exit(1);
        }
        return Ok(());
    }

    // ------------------ markdown report ------------------------------
    let mut out = String::new();
    out.push_str(&format!("# req review: {}..HEAD\n\n", base_ref));

    // Headline
    let gate_emoji = if val_errors > 0 {
        "FAIL"
    } else if val_warnings > 0
        || !coverage_ghosts.is_empty()
        || !markerless_changed_source.is_empty()
    {
        "WARN"
    } else {
        "OK"
    };
    out.push_str(&format!(
        "**Status:** {} — {} req(s) added / {} changed / {} removed; validate {} error(s), {} warning(s); coverage {} ghost(s), {} markerless source file(s); {} stale, {} drifted record(s).\n\n",
        gate_emoji,
        added.len(),
        changed.len(),
        removed.len(),
        val_errors,
        val_warnings,
        coverage_ghosts.len(),
        markerless_changed_source.len(),
        stale_count,
        drifted_count,
    ));

    if !added.is_empty() {
        out.push_str("## Added requirements\n\n");
        for id in &added {
            if let Some(r) = current.requirements.get(id) {
                out.push_str(&format!("- **{}** — {}\n", id, r.title));
            }
        }
        out.push('\n');
    }
    if !changed.is_empty() {
        out.push_str("## Changed requirements\n\n");
        for id in &changed {
            if let Some(r) = current.requirements.get(id) {
                out.push_str(&format!(
                    "- **{}** — {} ({})\n",
                    id,
                    r.title,
                    r.status.as_str()
                ));
            }
        }
        out.push('\n');
    }
    if !removed.is_empty() {
        out.push_str("## Removed requirements\n\n");
        for id in &removed {
            out.push_str(&format!("- ~~{}~~\n", id));
        }
        out.push('\n');
    }

    if !val_findings.is_empty() {
        out.push_str("## Validator findings\n\n");
        for (id, fs) in &val_findings {
            for f in fs {
                let sev = if f.error { "ERR " } else { "WARN" };
                out.push_str(&format!(
                    "- {} **{}** `{}` [{}] {}\n",
                    sev, id, f.rule_code, f.field, f.message
                ));
            }
        }
        out.push('\n');
    }

    if !coverage_ghosts.is_empty() {
        out.push_str("## Coverage ghosts in changed files\n\n");
        for g in &coverage_ghosts {
            out.push_str(&format!("- {}\n", g));
        }
        out.push('\n');
    }
    if !markerless_changed_source.is_empty() {
        out.push_str("## Changed source files without any REQ marker\n\n");
        out.push_str(
            "These files changed in this range but contain no `REQ-NNNN` reference. \
            Either add a `// REQ-NNNN:` line citing the requirement this code \
            implements, or add a new requirement (`req add ...`) and reference it. \
            New behaviour without a backing REQ is how spec drift starts.\n\n",
        );
        for f in &markerless_changed_source {
            out.push_str(&format!("- {}\n", f));
        }
        out.push('\n');
    }
    if !coverage_referenced.is_empty() {
        out.push_str("## Requirements referenced from changed files\n\n");
        for id in &coverage_referenced {
            out.push_str(&format!("- {}\n", id));
        }
        out.push('\n');
    }

    if stale_count > 0 || drifted_count > 0 {
        out.push_str("## Test-record freshness\n\n");
        out.push_str(&format!(
            "- {} stale (linked files moved since test record)\n- {} drifted (HEAD advanced but linked files unchanged)\n\nRun `req stale --only-stale` for the per-requirement table, or `req test run --from-file <log> --promote` to refresh after a passing test run.\n\n",
            stale_count, drifted_count
        ));
    }

    if let Some(a) = audit_summary.as_object() {
        out.push_str("## Audit headline\n\n");
        out.push_str(&format!(
            "- {} commit(s) on project.req in this range\n- last signer: {}\n- last signature status: {}\n\n",
            a.get("commits_in_range")
                .and_then(|v| v.as_u64())
                .unwrap_or(0),
            a.get("last_signer").and_then(|v| v.as_str()).unwrap_or("?"),
            a.get("last_status").and_then(|v| v.as_str()).unwrap_or("?")
        ));
    }

    print!("{}", out);
    let gate_fail =
        val_errors > 0 || !coverage_ghosts.is_empty() || !markerless_changed_source.is_empty();
    // Validate errors are always fatal. The wider gate (coverage
    // ghosts, markerless changed source) only flips the exit code in
    // --gate mode, so the default `req review` stays advisory.
    if val_errors > 0 || (args.gate && gate_fail) {
        std::process::exit(1);
    }
    Ok(())
}

fn resolve_base(requested: &str) -> String {
    if rev_exists(requested) {
        return requested.to_string();
    }
    if requested == "origin/main" && rev_exists("main") {
        return "main".to_string();
    }
    requested.to_string()
}

fn rev_exists(rev: &str) -> bool {
    Command::new("git")
        .args(["rev-parse", "--verify", rev])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn load_at_ref(base: &str, current_path: &Path) -> Result<Project> {
    let filename = current_path
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| anyhow!("project file has no name component"))?;
    let spec = format!("{}:{}", base, filename);
    let out = Command::new("git")
        .args(["show", &spec])
        .output()
        .with_context(|| format!("git show {}", spec))?;
    if !out.status.success() {
        return Err(anyhow!("git show {} failed", spec));
    }
    let tmp = std::env::temp_dir().join(format!("req-review-{}.req", std::process::id()));
    std::fs::write(&tmp, &out.stdout)?;
    let project = storage::load_with_options(&tmp, true)?;
    std::fs::remove_file(&tmp).ok();
    Ok(project)
}

fn diff_buckets(
    base: Option<&Project>,
    current: &Project,
) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut added = Vec::new();
    let mut removed = Vec::new();
    let mut changed = Vec::new();
    let base_ids: BTreeSet<&String> = base
        .map(|p| p.requirements.keys().collect())
        .unwrap_or_default();
    let cur_ids: BTreeSet<&String> = current.requirements.keys().collect();
    for id in cur_ids.difference(&base_ids) {
        added.push((*id).clone());
    }
    for id in base_ids.difference(&cur_ids) {
        removed.push((*id).clone());
    }
    if let Some(b) = base {
        for id in cur_ids.intersection(&base_ids) {
            let c = &current.requirements[*id];
            let p = &b.requirements[*id];
            if p.updated != c.updated
                || p.title != c.title
                || p.statement != c.statement
                || p.status != c.status
                || p.acceptance != c.acceptance
                || p.links.len() != c.links.len()
            {
                changed.push((*id).clone());
            }
        }
    }
    added.sort();
    removed.sort();
    changed.sort();
    (added, removed, changed)
}

fn git_changed_files(base: &str) -> Result<Vec<String>> {
    let out = Command::new("git")
        .args(["diff", "--name-only", &format!("{}...HEAD", base)])
        .output()?;
    if !out.status.success() {
        return Ok(Vec::new());
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect())
}

fn audit_summary_for_file(path: &Path) -> serde_json::Value {
    let filename = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("project.req");
    let out = Command::new("git")
        .args(["log", "--format=%H|%G?|%GS|%an|%s", "--", filename])
        .output();
    let body = out
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    let lines: Vec<&str> = body.lines().collect();
    let commits = lines.len();
    let last = lines.first().copied().unwrap_or("");
    let parts: Vec<&str> = last.splitn(5, '|').collect();
    json!({
        "commits_in_range": commits,
        "last_status": parts.get(1).copied().unwrap_or(""),
        "last_signer": parts.get(2).copied().unwrap_or(""),
        "last_author": parts.get(3).copied().unwrap_or(""),
    })
}
