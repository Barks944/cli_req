// Conformance checker dispatcher — drives REQ-0006, REQ-0007, REQ-0008, REQ-0009,
// REQ-0029, REQ-0030. (Rule bodies live in src/conform.rs.)
use anyhow::Result;
use serde_json::json;
use std::path::PathBuf;

use crate::cli::ConformArgs;
use crate::conform;
use crate::storage::load_resolved;

pub fn run(args: ConformArgs, file: &Option<PathBuf>) -> Result<()> {
    let (_, project) = load_resolved(file)?;
    let mut report = conform::conform_project(&project);

    // REQ-0148: a Verified safety requirement whose verified source has
    // drifted (content-hash staleness) is INVALID until it is re-verified and
    // re-confirmed by a human. This is a filesystem check (it hashes the linked
    // source), so it lives at the command layer, like `req stale` — but it is a
    // hard error (REQ-V-0035), so `req conform` and CI block until the safety
    // requirement is re-verified.
    let source_root = std::path::Path::new(".");
    for (id, sr) in &project.safety_requirements {
        if !matches!(sr.status, crate::model::Status::Verified) {
            continue;
        }
        let Some(v) = &sr.verification else { continue };
        let Some(hash) = &v.content_hash else {
            continue;
        };
        let stale = matches!(
            crate::commands::test_cmd::staleness_by_content(
                hash,
                v.linked_files.as_ref(),
                id,
                source_root,
            ),
            crate::commands::test_cmd::Staleness::Stale { .. }
        );
        if stale {
            report.push((
                id.clone(),
                vec![conform::Finding {
                    error: true,
                    field: "verification",
                    rule_code: "REQ-V-0035",
                    message: format!(
                        "{id} is Verified but its verified source has drifted (stale) — a stale safety requirement is invalid until re-verified and re-confirmed by a human: `req verification plan {id} --reopen --reason \"...\"` → analysis → test → conclude --promote, then a human runs `req verification confirm {id}`"
                    ),
                }],
            ));
        }
    }

    let mut errs = 0usize;
    let mut warns = 0usize;
    for (_, findings) in &report {
        for f in findings {
            if f.error {
                errs += 1
            } else {
                warns += 1
            }
        }
    }

    if args.json {
        let findings: Vec<_> = report
            .iter()
            .flat_map(|(id, fs)| {
                fs.iter().map(move |f| {
                    json!({
                        "req_id": id,
                        "rule_code": f.rule_code,
                        "field": f.field,
                        "severity": if f.error { "error" } else { "warning" },
                        "message": f.message,
                    })
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "errors": errs, "warnings": warns, "findings": findings,
                // SR-0006: machine consumers also see this is well-formedness, not V&V.
                "note": CONFORM_DISCLAIMER
            }))?
        );
        if errs > 0 {
            std::process::exit(1);
        }
        return Ok(());
    }

    if report.is_empty() {
        println!(
            "OK — {} requirement(s), no findings.",
            project.requirements.len()
        );
        // SR-0006 / REQ-0197: a well-formedness check must not read as V&V
        // status (terminology applied throughout output), and must point to the
        // command that reports the true verification standing.
        println!("{}", CONFORM_DISCLAIMER);
        return Ok(());
    }

    for (id, findings) in &report {
        println!("{}", id);
        for f in findings {
            let tag = if f.error { "ERR " } else { "WARN" };
            println!("  {} {} [{}] {}", tag, f.rule_code, f.field, f.message);
        }
    }
    println!();
    println!("{} error(s), {} warning(s)", errs, warns);
    // SR-0006: same disclaimer on the failure path.
    println!("{}", CONFORM_DISCLAIMER);
    if errs > 0 {
        std::process::exit(1);
    }
    Ok(())
}

/// SR-0006: `req conform` checks model well-formedness only — it says nothing
/// about whether requirements are verified or validated. Every conform output
/// states this and points the user at the true V&V standing.
const CONFORM_DISCLAIMER: &str = "This checks model well-formedness (req's rule set), not verification/validation status. \
For each requirement's V&V standing, run `req verification status`.";
