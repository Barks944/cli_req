// Implements REQ-0054 (project-level implementation-status summary with
// count and percentage per status bucket and a single delivery_progress %).
use anyhow::Result;
use serde_json::json;
use std::path::PathBuf;

use crate::cli::StatusArgs;
use crate::model::Status;
use crate::storage::load_resolved;

pub fn run(args: StatusArgs, file: &Option<PathBuf>) -> Result<()> {
    let (_, project) = load_resolved(file)?;
    let total = project.requirements.len();
    let mut counts = [0usize; 6];
    for r in project.requirements.values() {
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

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "project": project.name,
                "total": total,
                "by_status": {
                    "draft":       counts[0],
                    "proposed":    counts[1],
                    "approved":    counts[2],
                    "implemented": counts[3],
                    "verified":    counts[4],
                    "obsolete":    counts[5],
                },
                "delivery_progress_pct": (delivery_pct * 10.0).round() / 10.0,
                "non_obsolete": non_obsolete,
                "done": done,
            }))?
        );
        return Ok(());
    }

    println!("Project: {}", project.name);
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
    Ok(())
}
