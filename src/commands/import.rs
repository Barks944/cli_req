// Implements REQ-0067: req import — ingest requirements from markdown or
// JSON; route every item through the validator so the integrity guarantee
// applies to imported content. IDs are re-allocated to avoid collisions
// with the destination project.
use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use serde_json::json;
use std::io::Read;
use std::path::PathBuf;

use crate::cli::{ImportArgs, ImportFormat};
use crate::model::{Kind, Priority, Requirement, Status};
use crate::storage::{self, load_for_mutation};
use crate::validate;

pub fn run(args: ImportArgs, file: &Option<PathBuf>) -> Result<()> {
    let raw = if args.source == "-" {
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf)?;
        buf
    } else {
        std::fs::read_to_string(&args.source)
            .with_context(|| format!("read import source {}", args.source))?
    };

    let candidates: Vec<Candidate> = match args.format {
        ImportFormat::Markdown => parse_markdown(&raw),
        ImportFormat::Json => parse_json(&raw)?,
    };

    if candidates.is_empty() {
        return Err(anyhow!("no requirement candidates found in source"));
    }

    let (path, mut project, _lock) = load_for_mutation(file)?;
    let now = Utc::now();

    let mut accepted: Vec<serde_json::Value> = Vec::new();
    let mut rejected: Vec<serde_json::Value> = Vec::new();

    for c in &candidates {
        // Build a candidate Requirement with a placeholder ID.
        let req = Requirement {
            id: String::new(),
            title: c.title.clone(),
            statement: c.statement.clone(),
            rationale: c.rationale.clone(),
            acceptance: c.acceptance.clone(),
            kind: c.kind,
            priority: c.priority,
            status: Status::Draft,
            tags: c.tags.clone(),
            links: Vec::new(),
            created: now,
            updated: now,
            history: vec![super::history(
                "imported",
                Some(format!("source: {}", args.source)),
            )],
            tests: Vec::new(),
            validation: None,
            extra: Default::default(),
        };
        let findings = validate::validate_requirement(&req);
        let errs: Vec<String> = findings
            .iter()
            .filter(|f| f.error)
            .map(|f| format!("[{}] {}", f.field, f.message))
            .collect();
        if !errs.is_empty() {
            rejected.push(json!({ "title": c.title, "errors": errs }));
            if args.strict {
                if args.json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&json!({
                            "ok": false, "rejected": rejected, "accepted": accepted
                        }))?
                    );
                }
                return Err(anyhow!("--strict: rejecting batch on first failure"));
            }
            continue;
        }
        // REQ-0118: dry-run must be a no-op for the file. Only allocate
        // and insert when the flag is absent; the dry-run branch produces
        // report-only output and never touches `project`.
        if !args.dry_run {
            let id = project.allocate_id();
            let mut r = req;
            r.id = id.clone();
            project.requirements.insert(id.clone(), r);
            accepted.push(json!({ "id": id, "title": c.title }));
        } else {
            accepted.push(json!({ "would_allocate": "next", "title": c.title }));
        }
    }

    if !args.dry_run && !accepted.is_empty() {
        project.updated = now;
        storage::save(&path, &project)?;
    }

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "ok": rejected.is_empty(),
                "dry_run": args.dry_run,
                "accepted": accepted,
                "rejected": rejected,
            }))?
        );
    } else {
        let mode = if args.dry_run { " (dry-run)" } else { "" };
        println!(
            "req import{}: {} accepted, {} rejected",
            mode,
            accepted.len(),
            rejected.len()
        );
        for a in &accepted {
            if let Some(id) = a["id"].as_str() {
                println!("  + {} {}", id, a["title"].as_str().unwrap_or(""));
            } else {
                println!("  + (would allocate) {}", a["title"].as_str().unwrap_or(""));
            }
        }
        for r in &rejected {
            println!("  - {}: {}", r["title"].as_str().unwrap_or(""), r["errors"]);
        }
    }
    Ok(())
}

struct Candidate {
    title: String,
    statement: String,
    rationale: String,
    acceptance: Vec<String>,
    kind: Kind,
    priority: Priority,
    tags: Vec<String>,
}

impl Default for Candidate {
    fn default() -> Self {
        Self {
            title: String::new(),
            statement: String::new(),
            rationale: String::new(),
            acceptance: Vec::new(),
            kind: Kind::Functional,
            priority: Priority::Should,
            tags: Vec::new(),
        }
    }
}

/// Parse markdown: each level-2 or level-3 heading starts a candidate.
/// The first non-blank prose line becomes the statement. Lines under a
/// "Rationale:" heading become rationale; "Acceptance:" bullet lines
/// become acceptance criteria. Tags are inferred from a trailing
/// `Tags: a, b, c` line.
fn parse_markdown(src: &str) -> Vec<Candidate> {
    let mut out = Vec::new();
    let mut cur: Option<Candidate> = None;
    let mut section: &str = "statement";
    for line in src.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("## ") || trimmed.starts_with("### ") {
            if let Some(c) = cur.take() {
                if !c.title.is_empty() {
                    out.push(c);
                }
            }
            let c = Candidate {
                title: trimmed.trim_start_matches('#').trim().to_string(),
                ..Candidate::default()
            };
            cur = Some(c);
            section = "statement";
            continue;
        }
        if let Some(c) = cur.as_mut() {
            let lower = trimmed.to_lowercase();
            if lower.starts_with("rationale:") || lower == "rationale" {
                section = "rationale";
                let rest = trimmed.split_once(':').map(|x| x.1).unwrap_or("").trim();
                if !rest.is_empty() {
                    c.rationale = rest.to_string();
                }
                continue;
            }
            if lower.starts_with("acceptance:") || lower == "acceptance" {
                section = "acceptance";
                continue;
            }
            if lower.starts_with("tags:") {
                let rest = trimmed.split_once(':').map(|x| x.1).unwrap_or("");
                c.tags = rest
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                continue;
            }
            if trimmed.is_empty() {
                continue;
            }
            match section {
                "statement" => {
                    if c.statement.is_empty() {
                        c.statement = trimmed.to_string();
                    }
                }
                "rationale" => {
                    if !c.rationale.is_empty() {
                        c.rationale.push(' ');
                    }
                    c.rationale.push_str(trimmed);
                }
                "acceptance" => {
                    let bullet = trimmed
                        .trim_start_matches('-')
                        .trim_start_matches('*')
                        .trim();
                    if !bullet.is_empty() {
                        c.acceptance.push(bullet.to_string());
                    }
                }
                _ => {}
            }
        }
    }
    if let Some(c) = cur {
        if !c.title.is_empty() {
            out.push(c);
        }
    }
    out
}

/// Parse JSON: either another project.req-shaped object (we pull the
/// requirements map and ingest them) or a flat array of candidate
/// objects: { title, statement, rationale, kind?, priority?, acceptance?, tags? }.
fn parse_json(src: &str) -> Result<Vec<Candidate>> {
    let v: serde_json::Value = serde_json::from_str(src).context("parse import JSON")?;
    if let Some(arr) = v.as_array() {
        return Ok(arr.iter().filter_map(value_to_candidate).collect());
    }
    if let Some(obj) = v.as_object() {
        if let Some(reqs) = obj.get("requirements").and_then(|r| r.as_object()) {
            return Ok(reqs.values().filter_map(value_to_candidate).collect());
        }
    }
    Err(anyhow!(
        "JSON source must be an array of candidates or a project.req-shaped object"
    ))
}

fn value_to_candidate(v: &serde_json::Value) -> Option<Candidate> {
    let title = v.get("title")?.as_str()?.to_string();
    let statement = v.get("statement")?.as_str()?.to_string();
    let rationale = v
        .get("rationale")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let acceptance: Vec<String> = v
        .get("acceptance")
        .and_then(|x| x.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    let tags: Vec<String> = v
        .get("tags")
        .and_then(|x| x.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    let kind = match v
        .get("kind")
        .and_then(|x| x.as_str())
        .unwrap_or("functional")
    {
        "functional" => Kind::Functional,
        "non-functional" | "nonfunctional" | "NonFunctional" => Kind::NonFunctional,
        "constraint" | "Constraint" => Kind::Constraint,
        "interface" | "Interface" => Kind::Interface,
        "business" | "Business" => Kind::Business,
        _ => Kind::Functional,
    };
    let priority = match v
        .get("priority")
        .and_then(|x| x.as_str())
        .unwrap_or("should")
    {
        "must" | "Must" => Priority::Must,
        "should" | "Should" => Priority::Should,
        "could" | "Could" => Priority::Could,
        "wont" | "Wont" => Priority::Wont,
        _ => Priority::Should,
    };
    Some(Candidate {
        title,
        statement,
        rationale,
        acceptance,
        kind,
        priority,
        tags,
    })
}
