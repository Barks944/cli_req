// Implements REQ-0049 (record test runs with git HEAD SHA + outcome + notes)
// and REQ-0055 (req test run — drive cargo test, parse output, attach one
// pass/fail record per REQ).
use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::json;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Command;

use crate::cli::{TestCmd, TestRecordArgs, TestResultArg, TestRunArgs};
use crate::model::{TestOutcome, TestRecord};
use crate::storage::{self, load_resolved};

pub fn run(cmd: TestCmd, file: &Option<PathBuf>) -> Result<()> {
    match cmd {
        TestCmd::Record(args) => record(args, file),
        TestCmd::Run(args) => run_suite(args, file),
    }
}

fn record(args: TestRecordArgs, file: &Option<PathBuf>) -> Result<()> {
    let (path, mut project) = load_resolved(file)?;
    if !project.requirements.contains_key(&args.id) {
        return Err(anyhow!("no such requirement: {}", args.id));
    }
    let commit = current_head_sha().context("not in a git working tree — cannot record a test run without a commit SHA")?;
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
    };
    let r = project.requirements.get_mut(&args.id).unwrap();
    r.tests.push(record.clone());
    r.history.push(super::history(
        format!("test {} recorded against commit {}", outcome.as_str(), short(&record.commit)),
        Some(record.notes.clone()).filter(|s| !s.is_empty()),
    ));
    r.updated = Utc::now();
    project.updated = Utc::now();
    storage::save(&path, &project)?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(&project.requirements[&args.id])?);
    } else {
        println!("Recorded {} test for {} against {}.", outcome.as_str(), args.id, short(&record.commit));
    }
    Ok(())
}

fn current_head_sha() -> Result<String> {
    let out = Command::new("git").args(["rev-parse", "HEAD"]).output()?;
    if !out.status.success() {
        return Err(anyhow!("git rev-parse HEAD failed: {}", String::from_utf8_lossy(&out.stderr)));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub fn current_head_sha_opt() -> Option<String> {
    current_head_sha().ok()
}

pub fn short(sha: &str) -> String {
    sha.chars().take(9).collect()
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
    let (path, mut project) = load_resolved(file)?;

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
            println!("{}", serde_json::to_string_pretty(&json!({ "ok": out.status.success(), "matched": 0, "message": msg }))?);
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
        let outcome = if !res.failed.is_empty() { TestOutcome::Fail } else { TestOutcome::Pass };
        let total = res.passed.len() + res.failed.len() + res.ignored.len();
        let notes = format!(
            "cargo test: {} pass / {} fail / {} ignored — {}",
            res.passed.len(),
            res.failed.len(),
            res.ignored.len(),
            if res.failed.is_empty() {
                res.passed.iter().chain(res.ignored.iter()).cloned().collect::<Vec<_>>().join(", ")
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
        };
        records_to_apply.push((req_id.clone(), record));
    }

    if !args.dry_run {
        for (req_id, record) in &records_to_apply {
            let r = project.requirements.get_mut(req_id).unwrap();
            r.tests.push(record.clone());
            r.history.push(super::history(
                format!("test {} recorded against commit {} via req test run",
                    record.outcome.as_str(), short(&record.commit)),
                None,
            ));
            r.updated = Utc::now();
        }
        if !records_to_apply.is_empty() {
            project.updated = Utc::now();
            storage::save(&path, &project)?;
        }
    }

    if args.json {
        println!("{}", serde_json::to_string_pretty(&json!({
            "ok": out.status.success(),
            "dry_run": args.dry_run,
            "command": args.cmd,
            "matched_requirements": summary.len(),
            "recorded": if args.dry_run { 0 } else { records_to_apply.len() },
            "results": summary,
        }))?);
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
            println!("  {} {:<4} {} pass / {} fail / {} ignored{}", id, outcome.to_uppercase(), p, f, i, tag);
        }
        if !args.dry_run {
            println!();
            println!("Recorded {} test record(s).", records_to_apply.len());
        }
    }

    if !out.status.success() {
        std::process::exit(1);
    }
    Ok(())
}
