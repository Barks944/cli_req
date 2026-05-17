// Implements REQ-0023 (install pre-commit hook) and REQ-0024 (register the
// `req-merge` merge driver via .gitattributes + printed git config commands).
use anyhow::{anyhow, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

use crate::cli::HooksArgs;

const HOOK_MARKER: &str = "# managed-by: req-hooks";
const PRECOMMIT_BODY: &str = r#"#!/bin/sh
# managed-by: req-hooks
# Runs `req validate` on staged .req files. Remove this hook with
# `req hooks --uninstall` or by deleting this file.
set -e
if git diff --cached --name-only | grep -qE '\.req$'; then
  if ! command -v req >/dev/null 2>&1; then
    echo "req: binary not found on PATH; skipping requirements validation" >&2
    exit 0
  fi
  echo "req: validating staged requirements file(s)..."
  req validate
fi
"#;

pub fn run(args: HooksArgs) -> Result<()> {
    let action = args.action.to_lowercase();
    let uninstall = match action.as_str() {
        "install" => false,
        "uninstall" => true,
        other => return Err(anyhow!("unknown action '{}': expected install or uninstall", other)),
    };
    let repo = args.repo.unwrap_or_else(|| PathBuf::from("."));
    let git_dir = repo.join(".git");
    if !git_dir.exists() {
        return Err(anyhow!(
            "{} is not a git repository (no .git directory)",
            repo.display()
        ));
    }
    let hooks_dir = git_dir.join("hooks");
    fs::create_dir_all(&hooks_dir).ok();
    let hook = hooks_dir.join("pre-commit");

    if uninstall {
        if hook.exists() {
            let body = fs::read_to_string(&hook).unwrap_or_default();
            if body.contains(HOOK_MARKER) {
                fs::remove_file(&hook).context("remove pre-commit hook")?;
                println!("Removed {}", hook.display());
            } else {
                println!("Skipped {} (not managed by req)", hook.display());
            }
        }
        return Ok(());
    }

    if hook.exists() && !args.force {
        let existing = fs::read_to_string(&hook).unwrap_or_default();
        if !existing.contains(HOOK_MARKER) {
            return Err(anyhow!(
                "{} already exists and was not installed by req — pass --force to overwrite",
                hook.display()
            ));
        }
    }
    fs::write(&hook, PRECOMMIT_BODY).context("write pre-commit hook")?;
    set_executable(&hook).ok();
    println!("Installed {}", hook.display());

    let attrs = repo.join(".gitattributes");
    ensure_gitattributes_line(&attrs, "*.req merge=req-merge")?;

    println!();
    println!("Next step (one-time, per clone): register the merge driver in this repo:");
    println!();
    println!("  git config merge.req-merge.name 'req merge driver'");
    println!("  git config merge.req-merge.driver 'req renumber --base %O || true'");
    println!();
    println!("After that, merges into project.req auto-renumber colliding IDs.");
    Ok(())
}

fn ensure_gitattributes_line(path: &Path, line: &str) -> Result<()> {
    let existing = fs::read_to_string(path).unwrap_or_default();
    if existing.lines().any(|l| l.trim() == line) {
        return Ok(());
    }
    let mut new = existing;
    if !new.is_empty() && !new.ends_with('\n') {
        new.push('\n');
    }
    new.push_str(line);
    new.push('\n');
    fs::write(path, new).with_context(|| format!("write {}", path.display()))?;
    println!("Updated {} (added: {})", path.display(), line);
    Ok(())
}

#[cfg(unix)]
fn set_executable(p: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perm = fs::metadata(p)?.permissions();
    perm.set_mode(0o755);
    fs::set_permissions(p, perm)
}
#[cfg(not(unix))]
fn set_executable(_p: &Path) -> std::io::Result<()> {
    Ok(())
}
