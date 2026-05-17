// Implements REQ-0005 (req repair as the audited escape hatch for hand edits).
use anyhow::{anyhow, Result};
use std::path::PathBuf;

use crate::cli::RepairArgs;
use crate::storage;

pub fn run(args: RepairArgs, file: &Option<PathBuf>) -> Result<()> {
    if !args.confirm_direct_edit {
        return Err(anyhow!(
            "repair refuses to run without --confirm-direct-edit. \
             Review the file with `git diff` first."
        ));
    }
    let path = storage::resolve_path(file);
    let project = storage::load_with_options(&path, true)?;

    let findings = crate::validate::validate_project(&project);
    let errs: usize = findings
        .iter()
        .flat_map(|(_, fs)| fs.iter())
        .filter(|f| f.error)
        .count();
    if errs > 0 {
        eprintln!("Refusing to repair: file contains {} validation errors. Fix them first.", errs);
        for (id, fs) in &findings {
            for f in fs {
                if f.error {
                    eprintln!("  {}  ERR [{}] {}", id, f.field, f.message);
                }
            }
        }
        return Err(anyhow!("repair aborted"));
    }

    storage::save(&path, &project)?;
    println!("Re-signed {}. {} requirement(s).", path.display(), project.requirements.len());
    Ok(())
}
