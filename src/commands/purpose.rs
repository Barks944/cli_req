// REQ-0111: req purpose — set/print/clear the project's purpose
// statement. The purpose is a reserved top-level key under the
// integrity hash (req-v2 schema) and is surfaced by `req brief` at
// session start. 500-char cap enforces concision — if the answer to
// "what is this project FOR" needs more than a paragraph, it belongs
// in README.md, not here.
use anyhow::{anyhow, Result};
use std::path::PathBuf;

use crate::cli::PurposeArgs;
use crate::model::PURPOSE_MAX_CHARS;
use crate::storage;

pub fn run(args: PurposeArgs, file: &Option<PathBuf>) -> Result<()> {
    let path = storage::resolve_path(file);

    if args.text.is_none() {
        // Print mode.
        let project = storage::load(&path)?;
        match project.purpose.as_deref() {
            Some(p) => println!("{}", p),
            None => println!("(no purpose set — `req purpose '...'` to add one)"),
        }
        return Ok(());
    }

    let reason = args
        .reason
        .clone()
        .ok_or_else(|| anyhow!("--reason is required when changing the purpose"))?;
    let new_text = args.text.unwrap_or_default();
    let trimmed = new_text.trim();
    if trimmed.chars().count() > PURPOSE_MAX_CHARS {
        return Err(anyhow!(
            "purpose is {} chars; max {}. Tighten it — `req brief` leads with this line.",
            trimmed.chars().count(),
            PURPOSE_MAX_CHARS
        ));
    }

    let _lock = storage::acquire_lock(&path)?;
    let mut project = storage::load(&path)?;
    let old = project.purpose.clone();
    let new = if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    };
    if old == new {
        println!("(purpose unchanged)");
        return Ok(());
    }
    project.purpose = new.clone();
    project.updated = chrono::Utc::now();
    storage::save(&path, &project)?;

    // History on the project itself isn't a thing today — Project
    // doesn't carry a history vec, only Requirements do. We surface
    // the change via stdout and the `_integrity` hash bumps so the git
    // diff captures the move. When project-level history lands,
    // attach an entry here.
    let _ = reason; // recorded via the user's commit message
    match (&old, &new) {
        (None, Some(_)) => println!("purpose set"),
        (Some(_), None) => println!("purpose cleared"),
        (Some(_), Some(_)) => println!("purpose updated"),
        (None, None) => {}
    }
    Ok(())
}
