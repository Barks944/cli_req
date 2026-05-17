// Implements REQ-0049 (record test runs with git HEAD SHA + outcome + notes).
use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use serde_json::json;
use std::path::PathBuf;
use std::process::Command;

use crate::cli::{TestCmd, TestRecordArgs, TestResultArg};
use crate::model::{TestOutcome, TestRecord};
use crate::storage::{self, load_resolved};

pub fn run(cmd: TestCmd, file: &Option<PathBuf>) -> Result<()> {
    match cmd {
        TestCmd::Record(args) => record(args, file),
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
