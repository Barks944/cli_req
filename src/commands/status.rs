// Implements REQ-0054 (project-level implementation-status summary with
// count and percentage per status bucket and a single delivery_progress %).
use anyhow::Result;
use serde_json::json;
use std::path::PathBuf;

use crate::cli::StatusArgs;
use crate::model::{Status, TestOutcome};
use crate::storage::load_resolved;

/// REQ-0125: list IDs of Verified requirements whose latest TestRecord
/// outcome is Fail. Shared between status, brief, and lint so the
/// definition stays in one place.
pub fn verified_but_defective(project: &crate::model::Project) -> Vec<String> {
    let mut out: Vec<String> = project
        .requirements
        .iter()
        .filter(|(_, r)| matches!(r.status, Status::Verified))
        .filter(|(_, r)| {
            r.tests
                .last()
                .map(|t| matches!(t.outcome, TestOutcome::Fail))
                .unwrap_or(false)
        })
        .map(|(id, _)| id.clone())
        .collect();
    out.sort();
    out
}

pub fn run(args: StatusArgs, file: &Option<PathBuf>) -> Result<()> {
    let (_, project) = load_resolved(file)?;
    let mut counts = [0usize; 6];
    // REQ-0092: --tag filter scopes the status report to a milestone slice.
    // Filter once so both counts and total reflect the scoped view.
    let scope: Vec<_> = project
        .requirements
        .values()
        .filter(|r| args.tag.iter().all(|t| r.tags.iter().any(|rt| rt == t)))
        .collect();
    let total = scope.len();
    for r in &scope {
        let i = match r.status {
            Status::Draft => 0,
            Status::Proposed => 1,
            Status::Approved => 2,
            Status::Implemented => 3,
            Status::Verified => 4,
            Status::Obsolete => 5,
        };
        counts[i] += 1;
    }
    let pct = |n: usize| {
        if total == 0 {
            0.0
        } else {
            100.0 * n as f64 / total as f64
        }
    };
    let non_obsolete = total - counts[5];
    let done = counts[3] + counts[4];
    let delivery_pct = if non_obsolete == 0 {
        0.0
    } else {
        100.0 * done as f64 / non_obsolete as f64
    };
    // REQ-0142: of the verified bucket, how many rest on a GENUINE validation
    // dossier vs an audited exemption (backfill / no-dossier waiver) or no
    // dossier at all. `passed()` short-circuits on `exempt`, so without this
    // split the headline "verified" count is misleading. Staleness is not
    // probed here (no source root, and `req status` should stay cheap) — use
    // `req validation report` for the staleness-aware breakdown.
    let mut verified_genuine = 0usize;
    let mut verified_exempt = 0usize;
    for r in &scope {
        if !matches!(r.status, Status::Verified) {
            continue;
        }
        match crate::commands::validation::classify(r.validation.as_ref(), None, &r.id) {
            crate::commands::validation::Provenance::Genuine => verified_genuine += 1,
            _ => verified_exempt += 1,
        }
    }

    // REQ-0125: defects are Verified reqs whose latest test record is a Fail.
    // Filtered through the same tag scope as the rest of the report.
    let defective: Vec<String> = verified_but_defective(&project)
        .into_iter()
        .filter(|id| {
            args.tag
                .iter()
                .all(|t| project.requirements[id].tags.iter().any(|rt| rt == t))
        })
        .collect();

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "project": project.name,
                "filter": { "tags": args.tag },
                "total": total,
                "by_status": {
                    "draft":       counts[0],
                    "proposed":    counts[1],
                    "approved":    counts[2],
                    "implemented": counts[3],
                    "verified":    counts[4],
                    "obsolete":    counts[5],
                },
                // REQ-0142: genuine-vs-exempt split of the verified bucket.
                "verified_provenance": {
                    "genuine": verified_genuine,
                    "exempt": verified_exempt,
                },
                "delivery_progress_pct": (delivery_pct * 10.0).round() / 10.0,
                "non_obsolete": non_obsolete,
                "done": done,
                "verified_but_defective": defective,
            }))?
        );
        return Ok(());
    }

    println!("Project: {}", project.name);
    if !args.tag.is_empty() {
        println!("Scoped to tag(s): {}", args.tag.join(", "));
    }
    println!("Total:   {}", total);
    println!();
    println!(
        "  draft       : {:>4}  ({:>5.1}%)",
        counts[0],
        pct(counts[0])
    );
    println!(
        "  proposed    : {:>4}  ({:>5.1}%)",
        counts[1],
        pct(counts[1])
    );
    println!(
        "  approved    : {:>4}  ({:>5.1}%)",
        counts[2],
        pct(counts[2])
    );
    println!(
        "  implemented : {:>4}  ({:>5.1}%)",
        counts[3],
        pct(counts[3])
    );
    println!(
        "  verified    : {:>4}  ({:>5.1}%)",
        counts[4],
        pct(counts[4])
    );
    // REQ-0142: surface the genuine-vs-exempt split under the verified line.
    if counts[4] > 0 {
        println!(
            "    └─ genuine dossier: {}  ·  exempt/ungated: {}{}",
            verified_genuine,
            verified_exempt,
            if verified_exempt > 0 {
                "  (run `req validation report` for provenance)"
            } else {
                ""
            }
        );
    }
    println!(
        "  obsolete    : {:>4}  ({:>5.1}%)",
        counts[5],
        pct(counts[5])
    );
    println!();
    println!(
        "Delivery progress: {:.1}%  ({} of {} non-obsolete are implemented or verified)",
        delivery_pct, done, non_obsolete
    );
    if !defective.is_empty() {
        println!();
        println!(
            "verified-but-defective: {} (latest test record is a Fail — inspect with `req test list <id>`)",
            defective.len()
        );
        for id in &defective {
            println!("  - {}", id);
        }
    }
    Ok(())
}
