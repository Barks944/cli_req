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

    // REQ-0135: the staleness logic is id-agnostic, so run it over both
    // ordinary requirements and safety requirements. This is what makes a
    // SIL 3/4 SR's automated evidence go STALE when its linked code moves,
    // rather than standing as a claim forever.
    let mut process = |id: &str,
                       tests: &[crate::model::TestRecord],
                       validation: Option<&crate::model::Validation>| {
        // REQ-0139: when the item carries a concluded validation dossier
        // with a content-hash anchor, that anchor is the authoritative
        // staleness source — a code change since the validation was
        // concluded invalidates the verification. Prefer it over the test
        // record hash (which may pre-date the dossier).
        let dossier_anchor = validation.filter(|v| !v.exempt).and_then(|v| {
            v.content_hash
                .as_deref()
                .map(|h| (h, v.linked_files.as_ref(), &v.concluded_commit))
        });
        if let Some((stored_hash, linked, concluded_commit)) = dossier_anchor {
            let s = test_cmd::staleness_by_content(stored_hash, linked, id, &args.path);
            let commit = concluded_commit
                .as_deref()
                .map(test_cmd::short)
                .unwrap_or_else(|| "—".to_string());
            record_staleness(id, &commit, s, args.only_stale, &mut rows, &mut counts);
            return;
        }
        let latest = match tests.last() {
            None => {
                counts.3 += 1;
                if !args.only_stale {
                    rows.push((
                        id.to_string(),
                        "no-records".to_string(),
                        "—".to_string(),
                        Vec::<String>::new(),
                    ));
                }
                return;
            }
            Some(t) => t,
        };
        // REQ-0112: prefer content-hash comparison when the record
        // carries one. Falls back to SHA-based detection for older
        // records without a hash.
        let s = match latest.content_hash.as_deref() {
            Some(stored_hash) => test_cmd::staleness_by_content(
                stored_hash,
                latest.linked_files.as_ref(),
                id,
                &args.path,
            ),
            None => test_cmd::staleness(&latest.commit, id, &args.path),
        };
        record_staleness(
            id,
            &test_cmd::short(&latest.commit),
            s,
            args.only_stale,
            &mut rows,
            &mut counts,
        );
    };
    for r in project.requirements.values() {
        process(&r.id, &r.tests, r.validation.as_ref());
    }
    for sr in project.safety_requirements.values() {
        process(&sr.id, &sr.tests, sr.validation.as_ref());
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

/// Label a computed `Staleness`, bump the running counts, and push a row
/// (honouring `--only-stale`). Shared by the test-record and the REQ-0139
/// validation-dossier staleness paths.
#[allow(clippy::type_complexity)]
fn record_staleness(
    id: &str,
    commit: &str,
    s: Staleness,
    only_stale: bool,
    rows: &mut Vec<(String, String, String, Vec<String>)>,
    counts: &mut (usize, usize, usize, usize, usize),
) {
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
    if only_stale && !matches!(s, Staleness::Stale { .. }) {
        return;
    }
    let changed: Vec<String> = match &s {
        Staleness::Stale { changed, .. } => changed.clone(),
        _ => Vec::new(),
    };
    rows.push((
        id.to_string(),
        label.to_string(),
        commit.to_string(),
        changed,
    ));
}
