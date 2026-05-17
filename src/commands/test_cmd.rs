// Implements REQ-0049 (record test runs with git HEAD SHA + outcome + notes),
// REQ-0055 (req test run — drive cargo test, parse output, attach one
// pass/fail record per REQ), and REQ-0056 (verify-by-evidence policy:
// Verified status is backed by an automated test OR a written justification,
// recorded as a composition or inspection EvidenceKind on the same TestRecord
// shape with --promote auto-flipping status when a fresh passing record of
// any kind exists against current HEAD).
use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::json;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Command;

use crate::cli::{TestCmd, TestRecordArgs, TestResultArg, TestRunArgs, VerifyArgs, VerifyKindArg};
use crate::model::{EvidenceKind, Status, TestOutcome, TestRecord};
use crate::storage::{self, load_for_mutation};

pub fn run(cmd: TestCmd, file: &Option<PathBuf>) -> Result<()> {
    match cmd {
        TestCmd::Record(args) => record(args, file),
        TestCmd::Run(args) => run_suite(args, file),
    }
}

pub fn verify(args: VerifyArgs, file: &Option<PathBuf>) -> Result<()> {
    let (path, mut project, _lock) = load_for_mutation(file)?;
    if !project.requirements.contains_key(&args.id) {
        return Err(anyhow!("no such requirement: {}", args.id));
    }
    let kind = match args.by {
        VerifyKindArg::Composition => EvidenceKind::Composition,
        VerifyKindArg::Inspection => EvidenceKind::Inspection,
    };
    let commit = current_head_sha_opt().unwrap_or_else(|| "(no git)".into());
    let cites_prefix = if args.cites.is_empty() {
        String::new()
    } else {
        format!("cites: {} — ", args.cites.join(", "))
    };
    let record = TestRecord {
        at: Utc::now(),
        actor: super::current_actor(),
        commit: commit.clone(),
        outcome: TestOutcome::Pass,
        notes: format!("{}{}", cites_prefix, args.notes),
        kind,
    };
    let r = project.requirements.get_mut(&args.id).unwrap();
    r.tests.push(record.clone());
    r.history.push(super::history(
        format!(
            "{} evidence recorded against commit {}",
            kind.as_str(),
            short(&commit)
        ),
        Some(args.notes.clone()),
    ));
    r.updated = Utc::now();
    let mut promoted = false;
    if args.promote && !matches!(r.status, Status::Verified | Status::Obsolete) {
        r.status = Status::Verified;
        r.history.push(super::history(
            format!(
                "status promoted to verified ({} evidence on HEAD)",
                kind.as_str()
            ),
            None,
        ));
        promoted = true;
    }
    project.updated = Utc::now();
    storage::save(&path, &project)?;

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "id": args.id, "kind": kind.as_str(),
                "commit": commit, "promoted": promoted,
                "requirement": project.requirements[&args.id],
            }))?
        );
    } else {
        println!(
            "Recorded {} evidence on {} against commit {}.{}",
            kind.as_str(),
            args.id,
            short(&commit),
            if promoted {
                " Promoted to Verified."
            } else {
                ""
            }
        );
    }
    Ok(())
}

fn record(args: TestRecordArgs, file: &Option<PathBuf>) -> Result<()> {
    let (path, mut project, _lock) = load_for_mutation(file)?;
    if !project.requirements.contains_key(&args.id) {
        return Err(anyhow!("no such requirement: {}", args.id));
    }
    let commit = current_head_sha()
        .context("not in a git working tree — cannot record a test run without a commit SHA")?;
    let outcome = match args.result {
        TestResultArg::Pass => TestOutcome::Pass,
        TestResultArg::Fail => TestOutcome::Fail,
    };
    let record = TestRecord {
        at: Utc::now(),
        actor: super::current_actor(),
        commit,
        outcome,
        notes: args.notes,
        kind: EvidenceKind::Automated,
    };
    let r = project.requirements.get_mut(&args.id).unwrap();
    r.tests.push(record.clone());
    r.history.push(super::history(
        format!(
            "test {} recorded against commit {}",
            outcome.as_str(),
            short(&record.commit)
        ),
        Some(record.notes.clone()).filter(|s| !s.is_empty()),
    ));
    r.updated = Utc::now();
    project.updated = Utc::now();
    storage::save(&path, &project)?;

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&project.requirements[&args.id])?
        );
    } else {
        println!(
            "Recorded {} test for {} against {}.",
            outcome.as_str(),
            args.id,
            short(&record.commit)
        );
    }
    Ok(())
}

fn current_head_sha() -> Result<String> {
    let out = Command::new("git").args(["rev-parse", "HEAD"]).output()?;
    if !out.status.success() {
        return Err(anyhow!(
            "git rev-parse HEAD failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub fn current_head_sha_opt() -> Option<String> {
    current_head_sha().ok()
}

pub fn short(sha: &str) -> String {
    sha.chars().take(9).collect()
}

// ---------- staleness ----------

/// Three-state staleness signal for a TestRecord, computed by intersecting
/// "files referencing this requirement in code" with "files changed in git
/// between the record commit and HEAD". `Fresh` means the record commit is
/// HEAD. `Drifted` means HEAD moved but no linked file changed. `Stale`
/// means at least one linked file changed since the record commit.
pub enum Staleness {
    Fresh,
    /// HEAD moved but none of the requirement's linked files changed.
    /// The number is how many linked files exist.
    Drifted {
        linked: usize,
    },
    /// At least one linked file changed since the record commit.
    /// `linked` is kept on the variant so `req stale --json` callers can
    /// see how many files the requirement is linked to alongside the
    /// changed-files list. Read by the JSON renderer in commands/stale.rs.
    #[allow(dead_code)]
    Stale {
        changed: Vec<String>,
        linked: usize,
    },
    /// No git context — neither fresh nor stale can be computed.
    Unknown,
}

impl Staleness {
    pub fn tag(&self) -> String {
        match self {
            Staleness::Fresh => "[matches HEAD]".to_string(),
            Staleness::Drifted { linked: 0 } => "[drifted — no linked files]".to_string(),
            Staleness::Drifted { linked } => {
                format!("[drifted — no changes to {} linked file(s)]", linked)
            }
            Staleness::Stale { changed, .. } => {
                format!("[STALE — changed: {}]", changed.join(", "))
            }
            Staleness::Unknown => "[HEAD unknown]".to_string(),
        }
    }
}

/// Files under `root` that contain `REQ-NNNN` for the given id.
pub fn files_referencing(req_id: &str, root: &std::path::Path) -> Vec<std::path::PathBuf> {
    use once_cell::sync::Lazy;
    use regex::Regex;
    static REQ_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"REQ-\d{4}").unwrap());
    const DEFAULTS: &[&str] = &[
        "rs", "py", "js", "ts", "tsx", "go", "java", "md", "toml", "c", "cpp", "h",
    ];
    const SKIP: &[&str] = &[
        ".git",
        "target",
        "node_modules",
        "dist",
        "build",
        ".venv",
        ".idea",
        ".vscode",
    ];

    let mut hits = Vec::new();
    fn walk(
        root: &std::path::Path,
        exts: &[&str],
        skip: &[&str],
        req_id: &str,
        req_re: &regex::Regex,
        hits: &mut Vec<std::path::PathBuf>,
    ) {
        let entries = match std::fs::read_dir(root) {
            Ok(e) => e,
            Err(_) => return,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name_s = name.to_string_lossy();
            if path.is_dir() {
                if skip.iter().any(|s| *s == name_s.as_ref()) {
                    continue;
                }
                walk(&path, exts, skip, req_id, req_re, hits);
            } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                if !exts.contains(&ext) {
                    continue;
                }
                if let Ok(text) = std::fs::read_to_string(&path) {
                    if req_re.find_iter(&text).any(|m| m.as_str() == req_id) {
                        hits.push(path);
                    }
                }
            }
        }
    }
    walk(root, DEFAULTS, SKIP, req_id, &REQ_RE, &mut hits);
    hits
}

/// Files changed in git between `record_commit` and HEAD.
fn git_changed_since(record_commit: &str) -> Option<std::collections::BTreeSet<String>> {
    let out = std::process::Command::new("git")
        .args(["diff", "--name-only", &format!("{}..HEAD", record_commit)])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect(),
    )
}

pub fn staleness(record_commit: &str, req_id: &str, source_root: &std::path::Path) -> Staleness {
    let head = match current_head_sha_opt() {
        Some(h) => h,
        None => return Staleness::Unknown,
    };
    if head == record_commit {
        return Staleness::Fresh;
    }
    let linked = files_referencing(req_id, source_root);
    let linked_strs: std::collections::BTreeSet<String> = linked
        .iter()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .collect();
    let changed = match git_changed_since(record_commit) {
        Some(c) => c,
        None => return Staleness::Unknown,
    };
    let mut overlap: Vec<String> = linked_strs
        .iter()
        .filter(|f| {
            changed
                .iter()
                .any(|c| c.replace('\\', "/").ends_with(f.as_str()) || f.ends_with(c))
        })
        .cloned()
        .collect();
    overlap.sort();
    overlap.dedup();
    if overlap.is_empty() {
        Staleness::Drifted {
            linked: linked.len(),
        }
    } else {
        Staleness::Stale {
            changed: overlap,
            linked: linked.len(),
        }
    }
}

// ---------- req test run ----------

static TEST_LINE: Lazy<Regex> = Lazy::new(|| {
    // matches `test req_0006_some_name ... ok` or `... FAILED` or `... ignored`
    Regex::new(r"(?m)^test\s+(?:[\w:]+::)?(req_(\d{4})\w*)\s+\.\.\.\s+(ok|FAILED|ignored)").unwrap()
});

#[derive(Debug, Default)]
struct ReqResult {
    passed: Vec<String>,
    failed: Vec<String>,
    ignored: Vec<String>,
}

fn run_suite(args: TestRunArgs, file: &Option<PathBuf>) -> Result<()> {
    let (path, mut project, _lock) = load_for_mutation(file)?;

    let parts: Vec<&str> = args.cmd.split_whitespace().collect();
    if parts.is_empty() {
        return Err(anyhow!("empty test command"));
    }
    let out = Command::new(parts[0])
        .args(&parts[1..])
        .output()
        .with_context(|| format!("invoke {}", args.cmd))?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let combined = format!("{}\n{}", stdout, stderr);

    let mut by_req: BTreeMap<String, ReqResult> = BTreeMap::new();
    for cap in TEST_LINE.captures_iter(&combined) {
        let test_name = cap[1].to_string();
        let id = format!("REQ-{}", &cap[2]);
        let verdict = &cap[3];
        let bucket = by_req.entry(id).or_default();
        match verdict {
            "ok" => bucket.passed.push(test_name),
            "FAILED" => bucket.failed.push(test_name),
            "ignored" => bucket.ignored.push(test_name),
            _ => {}
        }
    }

    if by_req.is_empty() {
        let msg = "no test names matched the `req_NNNN_*` convention";
        if args.json {
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &json!({ "ok": out.status.success(), "matched": 0, "message": msg })
                )?
            );
        } else {
            eprintln!("{}", msg);
        }
        return Ok(());
    }

    let commit = current_head_sha_opt();
    let actor = super::current_actor();

    let mut records_to_apply: Vec<(String, TestRecord)> = Vec::new();
    let mut summary: Vec<serde_json::Value> = Vec::new();
    for (req_id, res) in &by_req {
        let exists = project.requirements.contains_key(req_id);
        let outcome = if !res.failed.is_empty() {
            TestOutcome::Fail
        } else {
            TestOutcome::Pass
        };
        let total = res.passed.len() + res.failed.len() + res.ignored.len();
        let notes = format!(
            "cargo test: {} pass / {} fail / {} ignored — {}",
            res.passed.len(),
            res.failed.len(),
            res.ignored.len(),
            if res.failed.is_empty() {
                res.passed
                    .iter()
                    .chain(res.ignored.iter())
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            } else {
                res.failed.join(", ")
            }
        );
        summary.push(json!({
            "req_id": req_id,
            "exists_in_project": exists,
            "outcome": outcome.as_str(),
            "tests": total,
            "passed": res.passed.len(),
            "failed": res.failed.len(),
            "ignored": res.ignored.len(),
            "test_names": {
                "passed": res.passed,
                "failed": res.failed,
                "ignored": res.ignored,
            },
        }));
        if !exists || args.dry_run {
            continue;
        }
        let record = TestRecord {
            at: Utc::now(),
            actor: actor.clone(),
            commit: commit.clone().unwrap_or_else(|| "(no git)".into()),
            outcome,
            notes,
            kind: EvidenceKind::Automated,
        };
        records_to_apply.push((req_id.clone(), record));
    }

    let mut promoted: Vec<String> = Vec::new();
    if !args.dry_run {
        for (req_id, record) in &records_to_apply {
            let r = project.requirements.get_mut(req_id).unwrap();
            r.tests.push(record.clone());
            r.history.push(super::history(
                format!(
                    "test {} recorded against commit {} via req test run",
                    record.outcome.as_str(),
                    short(&record.commit)
                ),
                None,
            ));
            r.updated = Utc::now();
        }
        // Auto-promote pass after writing records, so the latest record is
        // already on r.tests when we evaluate "is there fresh evidence?".
        if args.promote {
            let head = current_head_sha_opt();
            for (req_id, _) in &records_to_apply {
                let r = project.requirements.get_mut(req_id).unwrap();
                if matches!(r.status, Status::Verified | Status::Obsolete) {
                    continue;
                }
                let fresh = match &head {
                    Some(h) => r
                        .tests
                        .iter()
                        .any(|t| t.outcome == TestOutcome::Pass && &t.commit == h),
                    None => false,
                };
                if fresh {
                    r.status = Status::Verified;
                    r.history.push(super::history(
                        "status promoted to verified (req test run --promote, fresh passing record on HEAD)",
                        None,
                    ));
                    promoted.push(req_id.clone());
                }
            }
        }
        if !records_to_apply.is_empty() || !promoted.is_empty() {
            project.updated = Utc::now();
            storage::save(&path, &project)?;
        }
    }

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "ok": out.status.success(),
                "dry_run": args.dry_run,
                "command": args.cmd,
                "matched_requirements": summary.len(),
                "recorded": if args.dry_run { 0 } else { records_to_apply.len() },
                "results": summary,
            }))?
        );
    } else {
        let mode = if args.dry_run { " (dry-run)" } else { "" };
        println!("req test run{} — `{}`", mode, args.cmd);
        for entry in &summary {
            let id = entry["req_id"].as_str().unwrap();
            let exists = entry["exists_in_project"].as_bool().unwrap();
            let outcome = entry["outcome"].as_str().unwrap();
            let p = entry["passed"].as_u64().unwrap();
            let f = entry["failed"].as_u64().unwrap();
            let i = entry["ignored"].as_u64().unwrap();
            let tag = if !exists { " (unknown REQ)" } else { "" };
            println!(
                "  {} {:<4} {} pass / {} fail / {} ignored{}",
                id,
                outcome.to_uppercase(),
                p,
                f,
                i,
                tag
            );
        }
        if !args.dry_run {
            println!();
            println!("Recorded {} test record(s).", records_to_apply.len());
            if args.promote {
                println!("Promoted {} requirement(s) to Verified.", promoted.len());
            }
        }
    }

    if !out.status.success() {
        std::process::exit(1);
    }
    Ok(())
}
