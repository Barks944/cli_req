// REQ-0086: req review — one-shot PR-style spec impact report.
// REQ-0106: --summary mode is status-aware; pre-commit no longer
//           duplicates the post-commit summary.
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
    // REQ-0126: defects are Verified requirements whose latest test
    // record outcome is Fail. With --gate --no-defects, any defect
    // flips the exit code.
    let defects = crate::commands::status::verified_but_defective(&current);
    // --staged forces the base ref to HEAD and switches the
    // changed-files source from `git diff <base>...HEAD --name-only`
    // to `git diff --cached --name-only`. Used by the pre-commit hook.
    let base_ref = if args.staged {
        "HEAD".to_string()
    } else {
        resolve_base(&args.base)
    };

    // Fail closed on missing base ref under --gate so a CI YAML typo
    // (`origin/master` vs `origin/main`) does not silently disable the
    // whole check. Without --gate this is still advisory: the rest of
    // the report is useful even without a comparison point.
    //
    // EXCEPTION: --staged mode uses `git diff --cached` directly and
    // does not need HEAD to exist (the first commit in a fresh repo
    // has no HEAD yet). The gate still works — it checks staged files
    // for markers regardless of comparison ref.
    let base_ref_exists = rev_exists(&base_ref);
    if args.gate && !args.staged && !base_ref_exists {
        return Err(anyhow!(
            "base ref `{}` does not exist (or this is not a git repository) — \
             refusing to gate on an empty diff. Pass an explicit --base, or \
             drop --gate if you want the advisory report.",
            base_ref
        ));
    }

    let base = if base_ref_exists {
        load_at_ref(&base_ref, &path).ok()
    } else {
        None
    };

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
    // Source extension set defaults to a broad list covering every
    // common language the gate has been asked about. Override via
    // --ext if your codebase needs a narrower or wider scope.
    // REQ-0086: schema-as-code (`sql`) is first-class implementation;
    // included so SQL migrations participate in the gate by default.
    let default_source_exts: &[&str] = &[
        "rs", "py", "js", "ts", "tsx", "jsx", "go", "java", "kt", "kts", "scala", "swift", "cs",
        "rb", "php", "lua", "hs", "ml", "ex", "exs", "erl", "clj", "cljs", "dart", "zig", "nim",
        "v", "cr", "fs", "fsx", "groovy", "pl", "pm", "sh", "bash", "ps1", "psm1", "c", "cc",
        "cpp", "cxx", "h", "hh", "hpp", "hxx", "m", "mm", "rsx", "sql",
    ];
    let source_exts: Vec<String> = if args.ext.is_empty() {
        default_source_exts.iter().map(|s| s.to_string()).collect()
    } else {
        args.ext.clone()
    };

    // Paths to skip from the markerless check (still counted for
    // ghost references). Defaults skip test trees, build helpers,
    // generated code, and the .req project file itself — which is
    // documentation, not code, and contains REQ-NNNN strings in its
    // instructions block that should never be treated as markers.
    let project_file_name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("project.req")
        .to_string();
    let default_ignore: Vec<String> = vec![
        format!("**/{}", project_file_name),
        project_file_name.clone(),
        "**/tests/**".into(),
        "**/test/**".into(),
        "**/__tests__/**".into(),
        "**/spec/**".into(),
        "**/specs/**".into(),
        "**/*_test.*".into(),
        "**/*.test.*".into(),
        "**/*.spec.*".into(),
        "build.rs".into(),
        "**/build.rs".into(),
        "**/generated/**".into(),
        "**/.generated/**".into(),
        // Project documentation that mentions REQ-IDs descriptively
        // rather than as code markers. Skipped from both scans so
        // CHANGELOG entries like "REQ-0042: …" don't read as ghosts.
        "CHANGELOG.md".into(),
        "**/CHANGELOG.md".into(),
        "README.md".into(),
        "**/README.md".into(),
        "AGENTS.md".into(),
        "**/AGENTS.md".into(),
    ];
    let mut ignore_patterns: Vec<String> = default_ignore;
    ignore_patterns.extend(args.ignore.iter().cloned());

    let changed_files = if args.staged {
        git_staged_files().unwrap_or_default()
    } else {
        git_changed_files(&base_ref).unwrap_or_default()
    };
    // Dedup ghosts: one finding per (id, file) pair, not per occurrence.
    let mut ghost_set: BTreeSet<(String, String)> = BTreeSet::new();
    let mut coverage_referenced: BTreeSet<String> = BTreeSet::new();
    let mut markerless_changed_source: Vec<String> = Vec::new();
    // Tightened to comment-context only: a `REQ-NNNN` token only
    // counts as a marker when it appears on a comment line. String
    // literals, doc attributes, and incidental matches in data files
    // do NOT satisfy the gate.
    let marker_line_re =
        regex::Regex::new(r#"(?m)^\s*(?:(?://|#|--|;|/\*|\*)|.*?(?://|#))\s*.*?(REQ-\d{4})"#)
            .unwrap();
    let any_req_re = regex::Regex::new(r"REQ-\d{4}").unwrap();
    for f in &changed_files {
        let rel_normalised = f.replace('\\', "/");
        let full = args.path.join(f);
        let ext_lower = full
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase());
        let is_source = ext_lower
            .as_ref()
            .map(|e| source_exts.iter().any(|x| x == e))
            .unwrap_or(false);
        let ignored = ignore_patterns
            .iter()
            .any(|p| glob_match(p, &rel_normalised));
        // Ignored files are skipped from BOTH scans. The .req project
        // file's `_instructions` block contains example REQ-NNNN tokens
        // that should never be treated as ghosts; CHANGELOG / README
        // mention IDs descriptively, not as ghost references.
        if ignored {
            continue;
        }
        match std::fs::read_to_string(&full) {
            Ok(text) => {
                let mut saw_marker_in_comment = false;
                // Ghost scan stays broad on non-ignored files: a ghost
                // in a string literal is still worth knowing about.
                for cap in any_req_re.find_iter(&text) {
                    let id = cap.as_str().to_string();
                    if !current.requirements.contains_key(&id) {
                        ghost_set.insert((id, rel_normalised.clone()));
                    } else {
                        coverage_referenced.insert(id);
                    }
                }
                // Comment-context scan decides "is this file marked?"
                // REQ-0098: when --marker-near-hunks N is set, require
                // a marker within N lines of each changed hunk, not
                // merely somewhere in the file. Default 0 keeps the
                // 0.2.x file-level behaviour.
                let marker_lines: Vec<usize> = if args.marker_near_hunks > 0 {
                    text.lines()
                        .enumerate()
                        .filter_map(|(i, line)| {
                            for cap in marker_line_re.captures_iter(line) {
                                let id = cap.get(1).unwrap().as_str().to_string();
                                if current.requirements.contains_key(&id) {
                                    return Some(i + 1);
                                }
                            }
                            None
                        })
                        .collect()
                } else {
                    Vec::new()
                };
                if args.marker_near_hunks > 0 {
                    let hunks = if args.staged {
                        git_hunks_for_staged(f)
                    } else {
                        git_hunks_for_file(&base_ref, f)
                    }
                    .unwrap_or_default();
                    if !hunks.is_empty() {
                        let window = args.marker_near_hunks as usize;
                        saw_marker_in_comment = hunks.iter().all(|(start, len)| {
                            let lo = start.saturating_sub(window);
                            let hi = start.saturating_add(*len).saturating_add(window);
                            marker_lines.iter().any(|m| *m >= lo && *m <= hi)
                        });
                    } else {
                        // Couldn't get hunks (rename-only, deleted, or
                        // not a git repo). Fall back to file-level so
                        // we don't false-positive a benign rename.
                        saw_marker_in_comment = !marker_lines.is_empty();
                    }
                } else {
                    for cap in marker_line_re.captures_iter(&text) {
                        let id = cap.get(1).unwrap().as_str().to_string();
                        if current.requirements.contains_key(&id) {
                            saw_marker_in_comment = true;
                            break;
                        }
                    }
                }
                if is_source && !saw_marker_in_comment {
                    markerless_changed_source.push(rel_normalised);
                }
            }
            Err(_) => {
                // Likely a deleted-in-HEAD file; not a coverage problem.
            }
        }
    }
    let coverage_ghosts: Vec<String> = ghost_set
        .into_iter()
        .map(|(id, file)| format!("{} (in {})", id, file))
        .collect();

    // REQ-0086 / REQ-0106: --summary mode prints a calm, status-aware
    // impact line per cited REQ. Used by the post-commit hook only —
    // the pre-commit pass path is silent now to avoid duplication.
    // For each cited REQ, look up its current status and propose the
    // legal next move (not a one-size-fits-all `--promote` that the
    // lifecycle would then reject for fresh Drafts).
    if args.summary {
        let source_count = changed_files
            .iter()
            .filter(|f| {
                std::path::Path::new(f)
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|e| source_exts.iter().any(|x| x == &e.to_lowercase()))
                    .unwrap_or(false)
            })
            .count();
        if source_count == 0 {
            return Ok(());
        }
        let cites: Vec<String> = coverage_referenced.iter().cloned().collect();
        if cites.is_empty() {
            println!(
                "req: {} source file(s) touched · no REQ markers cited",
                source_count
            );
            return Ok(());
        }
        // REQ-0106: collapse long lists. A 46-REQ adoption commit
        // shouldn't print a 46-line wall — show count + first 3 +
        // pointer to `req brief`. Threshold of 5 keeps regular
        // multi-REQ commits (1-5 cites) fully detailed.
        const SUMMARY_DETAIL_THRESHOLD: usize = 5;
        println!(
            "req: {} source file(s) touched · cited {} REQ(s):",
            source_count,
            cites.len()
        );
        let detailed: Vec<&String> = if cites.len() > SUMMARY_DETAIL_THRESHOLD {
            cites.iter().take(3).collect()
        } else {
            cites.iter().collect()
        };
        for id in &detailed {
            let r = match current.requirements.get(*id) {
                Some(r) => r,
                None => continue, // ghost — already surfaced separately
            };
            use crate::model::Status;
            let suggestion = match r.status {
                Status::Draft => format!(
                    "advance with `req update {} --status proposed --reason \"...\"`",
                    id
                ),
                Status::Proposed => format!(
                    "advance with `req update {} --status approved --reason \"...\"`",
                    id
                ),
                Status::Approved => format!(
                    "mark implemented with `req update {} --status implemented --reason \"...\"`",
                    id
                ),
                Status::Implemented => format!(
                    "verify with `req verify {} --by inspection --notes \"...\" --promote`",
                    id
                ),
                Status::Verified => "(already verified — no action)".to_string(),
                Status::Obsolete => "(retired — no action)".to_string(),
            };
            println!("  {} ({}) — {}", id, r.status.as_str(), suggestion);
        }
        if cites.len() > SUMMARY_DETAIL_THRESHOLD {
            println!(
                "  … and {} more — `req brief` for the full picture.",
                cites.len() - 3
            );
        }
        return Ok(());
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
                "defects": defects,
                "audit": audit_summary,
            }))?
        );
        let gate_fail = val_errors > 0
            || !coverage_ghosts.is_empty()
            || !markerless_changed_source.is_empty()
            || (args.no_defects && !defects.is_empty());
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

    // REQ-0126: surface defects in the markdown report when present.
    if !defects.is_empty() {
        let mut block = String::from("\n## Verified-but-defective\n\n");
        block.push_str(&format!(
            "{} verified req(s) carry a failing latest test record. Inspect with `req test list <id>`.\n\n",
            defects.len()
        ));
        for id in &defects {
            block.push_str(&format!("- **{}**\n", id));
        }
        // Insert before printing the final out.
        out.push_str(&block);
    }
    print!("{}", out);
    let gate_fail = val_errors > 0
        || !coverage_ghosts.is_empty()
        || !markerless_changed_source.is_empty()
        || (args.no_defects && !defects.is_empty());
    // Validate errors are always fatal. The wider gate (coverage
    // ghosts, markerless changed source, defects when --no-defects)
    // only flips the exit code in --gate mode, so the default
    // `req review` stays advisory.
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

// REQ-0098: parse `git diff -U0 <range> -- <file>` into a list of
// (start_line, length) tuples for the NEW side. Used by the
// hunk-level marker check.
fn parse_hunks(diff: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for line in diff.lines() {
        // Hunk header: @@ -a,b +c,d @@ ...
        if let Some(after) = line.strip_prefix("@@ ") {
            if let Some(plus_pos) = after.find('+') {
                let after_plus = &after[plus_pos + 1..];
                let end = after_plus.find(' ').unwrap_or(after_plus.len());
                let range = &after_plus[..end];
                let (start_s, len_s) = match range.split_once(',') {
                    Some((a, b)) => (a, b),
                    None => (range, "1"),
                };
                if let (Ok(start), Ok(len)) = (start_s.parse::<usize>(), len_s.parse::<usize>()) {
                    if len > 0 {
                        out.push((start, len));
                    }
                }
            }
        }
    }
    out
}

fn git_hunks_for_file(base_ref: &str, file: &str) -> Result<Vec<(usize, usize)>> {
    let range = format!("{}...HEAD", base_ref);
    let out = Command::new("git")
        .args(["diff", "-U0", &range, "--", file])
        .output()?;
    if !out.status.success() {
        return Ok(Vec::new());
    }
    Ok(parse_hunks(&String::from_utf8_lossy(&out.stdout)))
}

fn git_hunks_for_staged(file: &str) -> Result<Vec<(usize, usize)>> {
    let out = Command::new("git")
        .args(["diff", "-U0", "--cached", "--", file])
        .output()?;
    if !out.status.success() {
        return Ok(Vec::new());
    }
    Ok(parse_hunks(&String::from_utf8_lossy(&out.stdout)))
}

fn git_staged_files() -> Result<Vec<String>> {
    let out = Command::new("git")
        .args(["diff", "--cached", "--name-only"])
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

/// Minimal glob matcher: supports `**` (any subpath), `*` (any segment
/// without separator), and literal chars. Sufficient for the gate's
/// ignore patterns; no need for a full crate dependency.
fn glob_match(pattern: &str, path: &str) -> bool {
    fn matches(pat: &[u8], text: &[u8]) -> bool {
        let mut pi = 0;
        let mut ti = 0;
        let mut star_pi: Option<usize> = None;
        let mut star_ti = 0;
        while ti < text.len() {
            if pi < pat.len() && pat[pi] == b'*' {
                // Detect `**` for "match across separators".
                if pi + 1 < pat.len() && pat[pi + 1] == b'*' {
                    // `**/` — collapse trailing slash if present.
                    let next = if pi + 2 < pat.len() && pat[pi + 2] == b'/' {
                        pi + 3
                    } else {
                        pi + 2
                    };
                    // Try to match the rest of the pattern at every position.
                    if next >= pat.len() {
                        return true;
                    }
                    let remaining = &pat[next..];
                    for skip in 0..=text.len() - ti {
                        if matches(remaining, &text[ti + skip..]) {
                            return true;
                        }
                    }
                    return false;
                }
                star_pi = Some(pi);
                star_ti = ti;
                pi += 1;
                continue;
            }
            if pi < pat.len() && (pat[pi] == text[ti]) && text[ti] != b'/' {
                pi += 1;
                ti += 1;
                continue;
            }
            if pi < pat.len() && pat[pi] != b'*' && pat[pi] == text[ti] {
                pi += 1;
                ti += 1;
                continue;
            }
            if let Some(spi) = star_pi {
                pi = spi + 1;
                star_ti += 1;
                ti = star_ti;
                if ti > text.len() || text.get(ti - 1) == Some(&b'/') {
                    // Single-segment `*` doesn't cross slashes; bail.
                    return false;
                }
                continue;
            }
            return false;
        }
        while pi < pat.len() && pat[pi] == b'*' {
            pi += 1;
        }
        pi == pat.len()
    }
    matches(pattern.as_bytes(), path.as_bytes())
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
