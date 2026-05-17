// Validator dispatcher — drives REQ-0006, REQ-0007, REQ-0008, REQ-0009, REQ-0029,
// REQ-0030. (Rule bodies live in src/validate.rs.)
use anyhow::Result;
use serde_json::json;
use std::path::PathBuf;

use crate::cli::ValidateArgs;
use crate::storage::load_resolved;
use crate::validate;

pub fn run(args: ValidateArgs, file: &Option<PathBuf>) -> Result<()> {
    let (_, project) = load_resolved(file)?;
    let report = validate::validate_project(&project);

    let mut errs = 0usize;
    let mut warns = 0usize;
    for (_, findings) in &report {
        for f in findings {
            if f.error { errs += 1 } else { warns += 1 }
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
        println!("{}", serde_json::to_string_pretty(&json!({
            "errors": errs, "warnings": warns, "findings": findings
        }))?);
        if errs > 0 { std::process::exit(1); }
        return Ok(());
    }

    if report.is_empty() {
        println!("OK — {} requirement(s), no findings.", project.requirements.len());
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
    if errs > 0 {
        std::process::exit(1);
    }
    Ok(())
}
