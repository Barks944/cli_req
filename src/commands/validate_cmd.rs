use anyhow::Result;
use std::path::PathBuf;

use crate::storage::load_resolved;
use crate::validate;

pub fn run(file: &Option<PathBuf>) -> Result<()> {
    let (_, project) = load_resolved(file)?;
    let report = validate::validate_project(&project);

    if report.is_empty() {
        println!("OK — {} requirement(s), no findings.", project.requirements.len());
        return Ok(());
    }

    let mut errs = 0usize;
    let mut warns = 0usize;
    for (id, findings) in &report {
        println!("{}", id);
        for f in findings {
            let tag = if f.error {
                errs += 1;
                "ERR "
            } else {
                warns += 1;
                "WARN"
            };
            println!("  {} [{}] {}", tag, f.field, f.message);
        }
    }
    println!();
    println!("{} error(s), {} warning(s)", errs, warns);
    if errs > 0 {
        std::process::exit(1);
    }
    Ok(())
}
