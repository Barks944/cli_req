// Discharges REQ-0001 (update sub-surface), contributes to REQ-0011 (history),
// and implements REQ-0035 (--add-acceptance / --remove-acceptance flags).
use anyhow::{anyhow, Result};
use chrono::Utc;
use std::path::PathBuf;

use crate::cli::UpdateArgs;
use crate::model::Status;
use crate::storage::{self, load_for_mutation};
use crate::validate;

pub fn run(mut args: UpdateArgs, file: &Option<PathBuf>) -> Result<()> {
    // Snapshot which field categories were touched so we can suppress
    // warnings whose inputs the user did not actually edit. Without
    // this every status nudge replays the same compound / weasel
    // warnings the author has already seen and accepted.
    let prose_changed = args.statement.is_some() || args.title.is_some();
    let rationale_changed = args.rationale.is_some();
    let acceptance_changed = args.acceptance.is_some()
        || !args.add_acceptance.is_empty()
        || !args.remove_acceptance.is_empty();

    let (path, mut project, _lock) = load_for_mutation(file)?;
    let canonical_id = super::resolve_id(&project, &args.id)?;
    args.id = canonical_id;
    let r = project
        .requirements
        .get_mut(&args.id)
        .ok_or_else(|| anyhow!("no such requirement: {}", args.id))?;

    let mut changes: Vec<String> = Vec::new();

    if let Some(t) = args.title {
        if r.title != t {
            changes.push(format!("title: {:?} -> {:?}", r.title, t));
            r.title = t;
        }
    }
    if let Some(s) = args.statement {
        if r.statement != s {
            changes.push("statement updated".into());
            r.statement = s;
        }
    }
    if let Some(rt) = args.rationale {
        if r.rationale != rt {
            changes.push("rationale updated".into());
            r.rationale = rt;
        }
    }
    if let Some(ac) = args.acceptance {
        changes.push(format!("acceptance replaced ({} items)", ac.len()));
        r.acceptance = ac;
    }
    for ac in &args.add_acceptance {
        r.acceptance.push(ac.clone());
        changes.push(format!("+acceptance #{}: {:?}", r.acceptance.len(), ac));
    }
    let mut to_remove = args.remove_acceptance.clone();
    to_remove.sort_unstable();
    to_remove.dedup();
    to_remove.reverse();
    for idx_1 in to_remove {
        if idx_1 == 0 || idx_1 > r.acceptance.len() {
            return Err(anyhow!(
                "--remove-acceptance index {} is out of range (1..={})",
                idx_1,
                r.acceptance.len()
            ));
        }
        let removed = r.acceptance.remove(idx_1 - 1);
        changes.push(format!("-acceptance #{}: {:?}", idx_1, removed));
    }
    if let Some(k) = args.kind {
        let k = k.into();
        if r.kind != k {
            changes.push(format!("kind {} -> {}", r.kind.as_str(), {
                let nk: crate::model::Kind = k;
                nk.as_str()
            }));
            r.kind = k;
        }
    }
    if let Some(p) = args.priority {
        let p = p.into();
        if r.priority != p {
            changes.push(format!("priority {} -> {}", r.priority.as_str(), {
                let np: crate::model::Priority = p;
                np.as_str()
            }));
            r.priority = p;
        }
    }
    if let Some(s) = args.status {
        let s: Status = s.into();
        if r.status != s {
            // Lifecycle policy lives in model.rs. Natural moves are
            // forward-one (with a Draft -> Proposed/Approved carve-out)
            // or any -> Obsolete. Everything else needs --force so
            // irregular moves are deliberate, with --reason on record.
            if !crate::model::is_natural_transition(r.status, s) && !args.force {
                // Special-case the Verified path with its longer hint
                // because it has a built-in alternative (`req verify`).
                let extra = if s == Status::Verified {
                    " — use `req verify --by <kind> --notes ... --promote` to attach evidence, or"
                } else {
                    ""
                };
                return Err(anyhow!(
                    "{} -> {} is an irregular transition for {}{}; \
                     pass --force --reason \"...\" to record an \
                     explicit override (e.g. correcting a bad record).",
                    r.status.as_str(),
                    s.as_str(),
                    args.id,
                    extra
                ));
            }
            changes.push(format!("status {} -> {}", r.status.as_str(), s.as_str()));
            r.status = s;
        }
    }
    for t in &args.add_tag {
        if !r.tags.iter().any(|x| x == t) {
            r.tags.push(t.clone());
            changes.push(format!("+tag {}", t));
        }
    }
    for t in &args.remove_tag {
        if let Some(pos) = r.tags.iter().position(|x| x == t) {
            r.tags.remove(pos);
            changes.push(format!("-tag {}", t));
        }
    }

    if changes.is_empty() {
        if args.json {
            println!(
                "{}",
                serde_json::to_string_pretty(&project.requirements[&args.id])?
            );
        } else {
            println!("No changes.");
        }
        return Ok(());
    }

    let findings = validate::validate_requirement(r);
    let errors = validate::errors_only(&findings);
    if !errors.is_empty() {
        eprintln!("Validation errors block save:");
        for f in &errors {
            eprintln!("  ERR [{}] {}", f.field, f.message);
        }
        return Err(anyhow!("update would violate requirements rules"));
    }
    for f in findings.iter().filter(|f| !f.error) {
        let surface = match f.field {
            "title" | "statement" => prose_changed,
            "rationale" => rationale_changed,
            "acceptance" => acceptance_changed,
            // Status- and link-related warnings are always relevant on
            // an update because the touched field may have unblocked
            // or triggered them.
            _ => true,
        };
        if surface {
            eprintln!("  WARN [{}] {}", f.field, f.message);
        }
    }

    r.updated = Utc::now();
    r.history
        .push(super::history(changes.join("; "), args.reason));
    project.updated = Utc::now();
    storage::save(&path, &project)?;
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&project.requirements[&args.id])?
        );
    } else {
        println!("Updated {}", args.id);
    }
    Ok(())
}
