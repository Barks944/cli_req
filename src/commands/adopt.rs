// REQ-0109: req adopt — retroactive backfill helper. Walks each
// requirement in scope from Draft → ... → target in one invocation,
// recording an "adopt" history entry per hop so the trail is auditable.
//
// Decision (locked into this implementation): when adopting a
// functional requirement to Implemented or Verified, the validator
// rule REQ-V-0018 requires at least one acceptance criterion. Rather
// than refuse to run or require the user to pre-edit, we auto-generate
// a placeholder entry of the form `implementation in source at
// adoption time` and tag the history entry with that fact so a
// reviewer can see exactly which acceptance lines came from adoption
// rather than from a human's hand.
use anyhow::{anyhow, Result};
use chrono::Utc;
use std::path::PathBuf;

use crate::cli::{AdoptArgs, AdoptTarget};
use crate::commands::test_cmd::current_head_sha_opt;
use crate::commands::{history, resolve_id};
use crate::model::{EvidenceKind, Kind, Status, TestOutcome, TestRecord};
use crate::storage;

const ADOPT_PLACEHOLDER_ACCEPTANCE: &str = "implementation in source at adoption time";

pub fn run(args: AdoptArgs, file: &Option<PathBuf>) -> Result<()> {
    let path = storage::resolve_path(file);
    let _lock = storage::acquire_lock(&path)?;
    let mut project = storage::load(&path)?;

    // Resolve scope.
    let mut targets: Vec<String> = Vec::new();
    if args.all_drafts {
        for (id, r) in &project.requirements {
            if matches!(r.status, Status::Draft) {
                targets.push(id.clone());
            }
        }
    }
    for raw in &args.ids {
        targets.push(resolve_id(&project, raw)?);
    }
    targets.sort();
    targets.dedup();
    if targets.is_empty() {
        return Err(anyhow!(
            "no requirements in scope — pass at least one REQ-ID or --all-drafts"
        ));
    }

    let target_status = match args.to {
        AdoptTarget::Proposed => Status::Proposed,
        AdoptTarget::Approved => Status::Approved,
        AdoptTarget::Implemented => Status::Implemented,
        AdoptTarget::Verified => Status::Verified,
    };
    let reason = args
        .reason
        .clone()
        .unwrap_or_else(|| "retroactive adoption from existing source state".to_string());

    let chain: &[Status] = &[
        Status::Draft,
        Status::Proposed,
        Status::Approved,
        Status::Implemented,
        Status::Verified,
    ];

    let mut plan: Vec<(String, Vec<Status>, bool)> = Vec::new(); // (id, hops, needs_placeholder)
    for id in &targets {
        let r = project
            .requirements
            .get(id)
            .ok_or_else(|| anyhow!("unknown requirement {}", id))?;
        let start_idx = chain
            .iter()
            .position(|s| std::mem::discriminant(s) == std::mem::discriminant(&r.status));
        let target_idx = chain
            .iter()
            .position(|s| std::mem::discriminant(s) == std::mem::discriminant(&target_status));
        let (start_idx, target_idx) = match (start_idx, target_idx) {
            (Some(a), Some(b)) => (a, b),
            _ => {
                // Obsolete or unexpected — skip silently with a note.
                println!("[skip] {} not in active lifecycle ({:?})", id, r.status);
                continue;
            }
        };
        if start_idx >= target_idx {
            println!("[skip] {} already at or beyond target", id);
            continue;
        }
        let hops: Vec<Status> = chain[(start_idx + 1)..=target_idx].to_vec();
        let needs_placeholder =
            matches!(r.kind, Kind::Functional) && r.acceptance.is_empty() && (target_idx >= 3); // Implemented or Verified
        plan.push((id.clone(), hops, needs_placeholder));
    }

    if plan.is_empty() {
        println!("nothing to adopt.");
        return Ok(());
    }

    if args.dry_run {
        println!("dry-run — would adopt:");
        for (id, hops, needs) in &plan {
            let hop_names: Vec<&str> = hops.iter().map(|s| s.as_str()).collect();
            println!(
                "  {} → {}{}",
                id,
                hop_names.join(" → "),
                if *needs {
                    " (will auto-add placeholder acceptance)"
                } else {
                    ""
                }
            );
        }
        return Ok(());
    }

    for (id, hops, needs_placeholder) in plan {
        // Mutable access only inside the loop to keep the borrow narrow.
        let r = project.requirements.get_mut(&id).expect("validated above");
        if needs_placeholder {
            r.acceptance.push(ADOPT_PLACEHOLDER_ACCEPTANCE.to_string());
            r.history.push(history(
                "adopt: auto-added placeholder acceptance",
                Some(format!("REQ-V-0018: {}", ADOPT_PLACEHOLDER_ACCEPTANCE)),
            ));
        }
        for hop in &hops {
            r.status = *hop;
            r.updated = Utc::now();
            r.history.push(history(
                format!("adopt → {}", hop.as_str()),
                Some(reason.clone()),
            ));
        }
        // REQ-0109 acceptance: record inspection evidence when target is Verified.
        if matches!(target_status, Status::Verified) {
            r.tests.push(TestRecord {
                at: Utc::now(),
                actor: crate::commands::current_actor(),
                commit: current_head_sha_opt().unwrap_or_else(|| "(no git)".into()),
                outcome: TestOutcome::Pass,
                notes: format!("Verified by adoption. {}", reason),
                kind: EvidenceKind::Inspection,
                content_hash: None,
                linked_files: None,
                sil_gate_exception: false,
            });
        }
        println!("adopted {} → {}", id, target_status.as_str());
    }

    project.updated = Utc::now();
    storage::save(&path, &project)?;
    Ok(())
}
