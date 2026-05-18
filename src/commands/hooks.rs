// Implements REQ-0023 (install pre-commit hook), REQ-0024 (register the
// `req-merge` merge driver via .gitattributes + printed git config
// commands), and REQ-0099 (pre-commit gate on markerless staged source).
use anyhow::{anyhow, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

use crate::cli::HooksArgs;

const HOOK_MARKER: &str = "# managed-by: req-hooks";
const HOOK_MODE_STRICT: &str = "# mode: strict";
const HOOK_MODE_DEFAULT: &str = "# mode: default";

// REQ-0099: pre-commit hook gates on markerless staged source.
// REQ-0100: --strict variant uses hunk-level matching via
//           `req review --staged --gate --marker-near-hunks 50` so
//           edits inside an already-marked file still need a marker
//           near the changed hunk. The two bodies share the same
//           outer structure; only the gate command differs.
//
// REQ_SKIP_GATE=1 bypasses the gate. Use for genuine WIP / merge /
// rebase commits; the env var leaves a trace in shell history.
fn precommit_body(strict: bool) -> String {
    let mode = if strict {
        HOOK_MODE_STRICT
    } else {
        HOOK_MODE_DEFAULT
    };
    let gate_cmd = if strict {
        "req review --staged --gate --marker-near-hunks 50"
    } else {
        "req review --staged --gate"
    };
    format!(
        r#"#!/bin/sh
{marker}
{mode_line}
# Validates staged .req files AND checks for code changes without REQ
# markers. Remove with `req hooks --uninstall` or by deleting this file.
set -e
if ! command -v req >/dev/null 2>&1; then
  echo "req: binary not found on PATH; skipping pre-commit checks" >&2
  exit 0
fi

if git diff --cached --name-only | grep -qE '\.req$'; then
  echo "req: validating staged requirements file(s)..."
  req validate
fi

if [ -z "$REQ_SKIP_GATE" ]; then
  if ! git diff --cached --name-only | grep -q '.'; then
    exit 0
  fi
  if ! {gate_cmd} >/dev/null 2>&1; then
    echo "" >&2
    echo "req: pre-commit gate blocked this commit." >&2
    echo "" >&2
    {gate_cmd} >&2 || true
    echo "" >&2
    echo "Either:" >&2
    echo "  - add a '// REQ-NNNN:' comment line near each flagged hunk" >&2
    echo "    citing the requirement this code implements, OR" >&2
    echo "  - run 'req add ...' to create a new requirement, then add" >&2
    echo "    its marker to the source." >&2
    echo "" >&2
    echo "If this is a genuine WIP / rebase / merge commit, bypass with:" >&2
    echo "  REQ_SKIP_GATE=1 git commit ..." >&2
    exit 1
  fi
fi
"#,
        marker = HOOK_MARKER,
        mode_line = mode,
        gate_cmd = gate_cmd,
    )
}

pub fn run(args: HooksArgs) -> Result<()> {
    let action = args.action.to_lowercase();
    let uninstall = match action.as_str() {
        "install" => false,
        "uninstall" => true,
        other => {
            return Err(anyhow!(
                "unknown action '{}': expected install or uninstall",
                other
            ))
        }
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
    let body = precommit_body(args.strict);
    fs::write(&hook, &body).context("write pre-commit hook")?;
    set_executable(&hook).ok();
    println!(
        "Installed {} ({})",
        hook.display(),
        if args.strict {
            "strict mode — hunk-level marker check"
        } else {
            "default mode — file-level marker check"
        }
    );

    let attrs = repo.join(".gitattributes");
    // REQ-0071: pin project.req to LF and disable text-mode normalization
    // so formatters and Windows autocrlf cannot silently invalidate the
    // integrity hash on checkout/commit. `-text` keeps git from doing
    // ANY conversion; `eol=lf` is the explicit storage form. All three
    // lines are added in a single write so the user sees one "Updated"
    // message, not three.
    ensure_gitattributes_lines(
        &attrs,
        &[
            "*.req merge=req-merge",
            "project.req -text eol=lf",
            "*.req -text eol=lf",
        ],
    )?;

    if args.claude_code {
        install_claude_code(&repo)?;
    }

    println!();
    println!("Next step (one-time, per clone): register the merge driver in this repo:");
    println!();
    println!("  git config merge.req-merge.name 'req merge driver'");
    println!("  git config merge.req-merge.driver 'req renumber --base %O || true'");
    println!();
    println!("After that, merges into project.req auto-renumber colliding IDs.");
    Ok(())
}

fn ensure_gitattributes_lines(path: &Path, lines: &[&str]) -> Result<()> {
    let existing = fs::read_to_string(path).unwrap_or_default();
    let mut new = existing.clone();
    let mut added: Vec<String> = Vec::new();
    for line in lines {
        if new.lines().any(|l| l.trim() == *line) {
            continue;
        }
        if !new.is_empty() && !new.ends_with('\n') {
            new.push('\n');
        }
        new.push_str(line);
        new.push('\n');
        added.push((*line).to_string());
    }
    if added.is_empty() {
        return Ok(());
    }
    fs::write(path, &new).with_context(|| format!("write {}", path.display()))?;
    println!(
        "Updated {} ({} line(s) added):",
        path.display(),
        added.len()
    );
    for l in &added {
        println!("  {}", l);
    }
    Ok(())
}

/// REQ-0044: write/update .claude/settings.json so a fresh Claude Code session
/// in this repo has the req binary on its permissions allowlist and a Stop
/// hook that runs `req validate`. Idempotent: merges with any pre-existing
/// settings rather than clobbering them.
fn install_claude_code(repo: &Path) -> Result<()> {
    use serde_json::{json, Value};
    let dir = repo.join(".claude");
    fs::create_dir_all(&dir).ok();
    let path = dir.join("settings.json");

    let mut root: Value = if path.exists() {
        let text = fs::read_to_string(&path)?;
        serde_json::from_str(&text).context("parse existing .claude/settings.json")?
    } else {
        json!({})
    };

    // Ensure permissions.allow contains our patterns.
    let want_allows = ["Bash(req:*)", "Bash(req --version)", "Bash(req --help)"];
    let allow = root
        .as_object_mut()
        .ok_or_else(|| anyhow!(".claude/settings.json is not a JSON object"))?
        .entry("permissions")
        .or_insert(json!({}))
        .as_object_mut()
        .ok_or_else(|| anyhow!("permissions is not an object"))?
        .entry("allow")
        .or_insert(json!([]));
    if let Some(arr) = allow.as_array_mut() {
        let existing: std::collections::BTreeSet<String> = arr
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();
        for a in want_allows {
            if !existing.contains(a) {
                arr.push(Value::String(a.into()));
            }
        }
    }

    // Ensure a Stop hook running `req validate` exists.
    let stop_hook = json!({
        "matcher": "*",
        "hooks": [{ "type": "command", "command": "req validate" }]
    });
    let hooks = root
        .as_object_mut()
        .unwrap()
        .entry("hooks")
        .or_insert(json!({}))
        .as_object_mut()
        .ok_or_else(|| anyhow!("hooks is not an object"))?
        .entry("Stop")
        .or_insert(json!([]));
    if let Some(arr) = hooks.as_array_mut() {
        let already = arr.iter().any(|v| {
            v.get("hooks")
                .and_then(|h| h.as_array())
                .map(|h| {
                    h.iter().any(|e| {
                        e.get("command")
                            .and_then(|c| c.as_str())
                            .map(|s| s.contains("req validate"))
                            .unwrap_or(false)
                    })
                })
                .unwrap_or(false)
        });
        if !already {
            arr.push(stop_hook);
        }
    }

    fs::write(&path, serde_json::to_string_pretty(&root)?)?;
    println!(
        "Updated {} (allowlist + Stop hook for req validate)",
        path.display()
    );
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
