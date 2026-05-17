// Implements REQ-0063 (content-drift staleness report): for each
// requirement's latest test record, compare against the current HEAD AND
// the set of files containing the requirement's REQ-NNNN marker. Reports
// Fresh / Drifted (no linked files changed) / STALE (linked files
// changed since the record).
use anyhow::Result;
use serde_json::json;
use std::path::PathBuf;

use crate::cli::StaleArgs;
use crate::commands::test_cmd::{self, Staleness};
use crate::storage::load_resolved;

pub fn run(args: StaleArgs, file: &Option<PathBuf>) -> Result<()> {
    let (_, project) = load_resolved(file)?;

    let mut rows = Vec::new();
    let mut counts = (0usize, 0usize, 0usize, 0usize, 0usize); // fresh, drifted, stale, no_records, unknown
    for r in project.requirements.values() {
        let latest = match r.tests.last() {
            None => {
                counts.3 += 1;
                if !args.only_stale {
                    rows.push((
                        r.id.clone(),
                        "no-records".to_string(),
                        "—".to_string(),
                        Vec::<String>::new(),
                    ));
                }
                continue;
            }
            Some(t) => t,
        };
        let s = test_cmd::staleness(&latest.commit, &r.id, &args.path);
        let label = match &s {
            Staleness::Fresh => {
                counts.0 += 1;
                "fresh"
            }
            Staleness::Drifted { .. } => {
                counts.1 += 1;
                "drifted"
            }
            Staleness::Stale { .. } => {
                counts.2 += 1;
                "STALE"
            }
            Staleness::Unknown => {
                counts.4 += 1;
                "unknown"
            }
        };
        if args.only_stale && !matches!(s, Staleness::Stale { .. }) {
            continue;
        }
        let changed: Vec<String> = match &s {
            Staleness::Stale { changed, .. } => changed.clone(),
            _ => Vec::new(),
        };
        rows.push((
            r.id.clone(),
            label.to_string(),
            test_cmd::short(&latest.commit),
            changed,
        ));
    }

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "summary": {
                    "fresh": counts.0,
                    "drifted": counts.1,
                    "stale": counts.2,
                    "no_records": counts.3,
                    "unknown": counts.4,
                },
                "rows": rows.iter().map(|(id, state, commit, changed)| json!({
                    "id": id, "state": state, "record_commit": commit, "changed_files": changed,
                })).collect::<Vec<_>>(),
            }))?
        );
        return Ok(());
    }

    println!("Staleness report (root: {})", args.path.display());
    println!("  fresh      : {}", counts.0);
    println!(
        "  drifted    : {}  (HEAD moved but linked files unchanged)",
        counts.1
    );
    println!(
        "  STALE      : {}  (linked files changed since record)",
        counts.2
    );
    println!("  no records : {}", counts.3);
    println!("  unknown    : {}  (no git context)", counts.4);
    if rows.is_empty() {
        if args.only_stale {
            println!("\nNothing stale.");
        }
        return Ok(());
    }
    println!();
    for (id, state, commit, changed) in &rows {
        println!("  {:<10} {:<10} record={}", id, state, commit);
        for c in changed {
            println!("                       changed: {}", c);
        }
    }
    if counts.2 > 0 {
        std::process::exit(1);
    }
    Ok(())
}
