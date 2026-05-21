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

use crate::cli::{
    TestCmd, TestListArgs, TestRecordArgs, TestResultArg, TestRunArgs, VerifyArgs, VerifyKindArg,
};
use crate::model::{EvidenceKind, Status, TestOutcome, TestRecord};
use crate::storage::{self, load_for_mutation};

pub fn run(cmd: TestCmd, file: &Option<PathBuf>) -> Result<()> {
    match cmd {
        TestCmd::Record(args) => record(args, file),
        TestCmd::Run(args) => run_suite(args, file),
        TestCmd::List(args) => list(args, file),
    }
}

/// REQ-0129: dedicated subcommand to inspect a requirement's test
/// record history without parsing `req show --json`. Mirrors the
/// TestRecord shape with one record per line.
fn list(mut args: TestListArgs, file: &Option<PathBuf>) -> Result<()> {
    use crate::storage::load_resolved;
    let (_, project) = load_resolved(file)?;
    args.id = super::resolve_id(&project, &args.id)?;
    let r = project
        .requirements
        .get(&args.id)
        .ok_or_else(|| anyhow!("no such requirement: {}", args.id))?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(&r.tests)?);
        return Ok(());
    }
    if r.tests.is_empty() {
        println!("(no test records)");
        return Ok(());
    }
    for t in &r.tests {
        println!(
            "{}  {}  {}  {}  {}",
            t.at.format("%Y-%m-%d %H:%M UTC"),
            short(&t.commit),
            t.outcome.as_str(),
            t.kind.as_str(),
            t.notes
        );
    }
    Ok(())
}

pub fn verify(mut args: VerifyArgs, file: &Option<PathBuf>) -> Result<()> {
    let (path, mut project, _lock) = load_for_mutation(file)?;
    args.id = super::resolve_id(&project, &args.id)?;
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
        content_hash: None,
        linked_files: None,
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
    if args.promote {
        // Promotion bypassed the lifecycle entirely before: Draft was
        // promoted straight to Verified. Now Implemented is the only
        // status that auto-promotes; everything else requires --force
        // so the user has to acknowledge the skip.
        let eligible = matches!(r.status, Status::Implemented);
        if eligible || args.force {
            if !matches!(r.status, Status::Verified | Status::Obsolete) {
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
        } else if !matches!(r.status, Status::Verified | Status::Obsolete) {
            return Err(anyhow!(
                "{} is at status '{}'; --promote only auto-promotes from \
                 'implemented'. Move it to implemented first (`req update \
                 {} --status implemented --reason ...`), or pass --force \
                 to skip the precondition.",
                args.id,
                r.status.as_str(),
                args.id
            ));
        }
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

fn record(mut args: TestRecordArgs, file: &Option<PathBuf>) -> Result<()> {
    let (path, mut project, _lock) = load_for_mutation(file)?;
    args.id = super::resolve_id(&project, &args.id)?;
    let commit = current_head_sha()
        .context("not in a git working tree — cannot record a test run without a commit SHA")?;
    let outcome = match args.result {
        TestResultArg::Pass => TestOutcome::Pass,
        TestResultArg::Fail => TestOutcome::Fail,
    };
    // REQ-0112: content-hash the linked source files at record time
    // so `req stale` can fire on actual content changes rather than
    // any HEAD movement. Auto-discovered via `// REQ-NNNN:` markers.
    let auto_linked = auto_linked_files(&args.id, std::path::Path::new("."));
    let content_hash = if auto_linked.is_empty() {
        None
    } else {
        Some(hash_files(&auto_linked))
    };
    let record = TestRecord {
        at: Utc::now(),
        actor: super::current_actor(),
        commit,
        outcome,
        notes: args.notes,
        kind: EvidenceKind::Automated,
        content_hash,
        linked_files: if auto_linked.is_empty() {
            None
        } else {
            Some(
                auto_linked
                    .iter()
                    .map(|p| p.to_string_lossy().to_string())
                    .collect(),
            )
        },
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
/// REQ-0112: auto-discover linked files for a test record. Wraps
/// `files_referencing` with a stable ordering so the content hash is
/// reproducible across runs.
pub fn auto_linked_files(req_id: &str, root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut files = files_referencing(req_id, root);
    files.sort();
    files
}

/// REQ-0112: hash a list of files (sha256 over each file's contents,
/// concatenated with a path separator). Missing files produce an empty
/// byte block; the path is always included so deletion vs same-content
/// renames are distinguishable.
pub fn hash_files(files: &[std::path::PathBuf]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    for p in files {
        hasher.update(p.to_string_lossy().as_bytes());
        hasher.update(b"\0");
        if let Ok(bytes) = std::fs::read(p) {
            hasher.update(&bytes);
        }
        hasher.update(b"\n");
    }
    format!("{:x}", hasher.finalize())
}

pub fn files_referencing(req_id: &str, root: &std::path::Path) -> Vec<std::path::PathBuf> {
    use once_cell::sync::Lazy;
    use regex::Regex;
    static REQ_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"REQ-\d{4}").unwrap());
    let exts: Vec<String> = [
        "rs", "py", "js", "ts", "tsx", "go", "java", "md", "toml", "c", "cpp", "h",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let mut hits = Vec::new();
    // REQ-0124: source_walk honours .gitignore so test-record linked-file
    // discovery doesn't pick up artefacts in tmp/, dist/, etc.
    crate::source_walk::walk_source_tree(root, &exts, |path| {
        if let Ok(text) = std::fs::read_to_string(path) {
            if REQ_RE.find_iter(&text).any(|m| m.as_str() == req_id) {
                hits.push(path.to_path_buf());
            }
        }
    });
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

/// REQ-0112: staleness check using a stored content_hash. Compares
/// the hash of the currently-linked files (auto-discovered, or the
/// explicit `linked_files` override) against `stored_hash`. STALE
/// when they differ; Fresh when they match. Always returns the count
/// of linked files for diagnostics.
pub fn staleness_by_content(
    stored_hash: &str,
    explicit_linked: Option<&Vec<String>>,
    req_id: &str,
    source_root: &std::path::Path,
) -> Staleness {
    let files: Vec<std::path::PathBuf> = match explicit_linked {
        Some(list) => list.iter().map(std::path::PathBuf::from).collect(),
        None => auto_linked_files(req_id, source_root),
    };
    let current_hash = hash_files(&files);
    if current_hash == stored_hash {
        Staleness::Fresh
    } else {
        // We don't have a per-file diff here — surface the linked set
        // so the user knows where to look. The `changed` vec carries
        // the linked files rather than just the deltas; this is more
        // useful in practice (you re-check all of them) and avoids a
        // second git invocation when content hashing already told us
        // something moved.
        let changed: Vec<String> = files
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect();
        if changed.is_empty() {
            Staleness::Drifted { linked: 0 }
        } else {
            Staleness::Stale {
                linked: changed.len(),
                changed,
            }
        }
    }
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

    // Either parse a pre-captured log file (--from-file) or run the test
    // command and parse its combined stdout+stderr. The file path bypasses
    // shell quoting entirely, which matters for tests on Windows where
    // splitting --cmd on whitespace drops cmd.exe's /C argument boundaries.
    let (combined, exec_success) = if let Some(p) = &args.from_file {
        let body = std::fs::read_to_string(p)
            .with_context(|| format!("read --from-file {}", p.display()))?;
        (body, true)
    } else {
        let parts: Vec<&str> = args.cmd.split_whitespace().collect();
        if parts.is_empty() {
            return Err(anyhow!("empty test command"));
        }
        let out = Command::new(parts[0])
            .args(&parts[1..])
            .output()
            .with_context(|| format!("invoke {}", args.cmd))?;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        (format!("{}\n{}", stdout, stderr), out.status.success())
    };

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

    // REQ-0128: load an external test-name → REQ-ID map and walk a
    // generic verdict regex over the same combined output. This is
    // the Node/Python/etc. path: tests don't follow `req_NNNN_*` so
    // the mapping is explicit. Format: `{ "<test name>": ["REQ-NNNN", ...] }`.
    if let Some(map_path) = &args.map_file {
        let body = std::fs::read_to_string(map_path)
            .with_context(|| format!("read --map {}", map_path.display()))?;
        let map: BTreeMap<String, Vec<String>> = serde_json::from_str(&body)
            .with_context(|| format!("parse --map {} as JSON", map_path.display()))?;
        // Generic verdict line: "<test name> ... ok|FAILED|ignored". We
        // anchor on the exact test name (as a substring of any line) and
        // look for one of the verdict tokens on the same line. This is a
        // forgiving match that works for mocha/pytest/jest in default
        // reporters.
        for (test_name, ids) in &map {
            for line in combined.lines() {
                if !line.contains(test_name) {
                    continue;
                }
                let verdict = if line.contains("FAILED") || line.contains("FAIL") {
                    Some("FAILED")
                } else if line.contains(" ok") || line.contains("PASS") || line.contains("pass") {
                    Some("ok")
                } else if line.contains("ignored") || line.contains("SKIP") || line.contains("skip")
                {
                    Some("ignored")
                } else {
                    None
                };
                if let Some(v) = verdict {
                    for id in ids {
                        let bucket = by_req.entry(id.clone()).or_default();
                        match v {
                            "ok" => bucket.passed.push(test_name.clone()),
                            "FAILED" => bucket.failed.push(test_name.clone()),
                            "ignored" => bucket.ignored.push(test_name.clone()),
                            _ => {}
                        }
                    }
                    break; // one verdict per test name
                }
            }
        }
    }

    if by_req.is_empty() {
        let msg = "no test names matched the `req_NNNN_*` convention";
        if args.json {
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &json!({ "ok": exec_success, "matched": 0, "message": msg })
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
        // REQ-0112: content-hash auto-discovered linked files.
        let auto_linked = auto_linked_files(req_id, std::path::Path::new("."));
        let content_hash = if auto_linked.is_empty() {
            None
        } else {
            Some(hash_files(&auto_linked))
        };
        let record = TestRecord {
            at: Utc::now(),
            actor: actor.clone(),
            commit: commit.clone().unwrap_or_else(|| "(no git)".into()),
            outcome,
            notes,
            kind: EvidenceKind::Automated,
            content_hash,
            linked_files: if auto_linked.is_empty() {
                None
            } else {
                Some(
                    auto_linked
                        .iter()
                        .map(|p| p.to_string_lossy().to_string())
                        .collect(),
                )
            },
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
                "ok": exec_success,
                "dry_run": args.dry_run,
                "source": args.from_file.as_ref().map(|p| p.display().to_string())
                    .unwrap_or_else(|| args.cmd.clone()),
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

    if !exec_success {
        std::process::exit(1);
    }
    Ok(())
}
