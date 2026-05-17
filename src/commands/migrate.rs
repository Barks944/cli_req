// Implements REQ-0068: req migrate. Detects the project's _format tag and
// applies registered migrations to bring it to the current version. The
// only currently-registered version is `req-v1`, so the command is a
// principled no-op today — but the structure (back up, mutate, re-sign)
// is in place so the first v1→v2 migration only needs to register a
// migration function. See `req help format-policy`.
use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};
use std::path::PathBuf;

use crate::cli::MigrateArgs;
use crate::storage::{self, resolve_path, FORMAT_TAG};

pub fn run(args: MigrateArgs, file: &Option<PathBuf>) -> Result<()> {
    let path = resolve_path(file);
    let _lock = storage::acquire_lock(&path)?;

    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("read {}", path.display()))?;
    let root: serde_json::Map<String, Value> = serde_json::from_str(&raw)
        .with_context(|| format!("{} is not valid JSON", path.display()))?;
    let detected = root.get("_format").and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("not a .req file: missing _format"))?;

    if detected == FORMAT_TAG {
        let msg = format!("{} already at format {}; no migration needed.", path.display(), detected);
        if args.json {
            println!("{}", serde_json::to_string_pretty(&json!({
                "ok": true, "migrated": false, "from": detected, "to": FORMAT_TAG, "message": msg
            }))?);
        } else {
            println!("{}", msg);
        }
        return Ok(());
    }

    // Detect direction.
    if detected > FORMAT_TAG {
        return Err(anyhow!(
            "{} is at format {} which is newer than this binary ({}). Upgrade the binary first.",
            path.display(), detected, FORMAT_TAG,
        ));
    }

    // Back up the source file before any mutation.
    let backup = path.with_extension(format!("req.bak-{}", detected));
    std::fs::copy(&path, &backup)
        .with_context(|| format!("write backup {}", backup.display()))?;

    // Apply migrations in sequence. No migrations registered yet — when v2
    // arrives this is where the v1→v2 transformer goes:
    //   if detected == "req-v1" { project = migrate_v1_to_v2(project)?; }
    //
    // For now, an older format that we don't have a migration for is an
    // error rather than a silent pass-through.
    return Err(anyhow!(
        "no migration registered for format {} → {}. Restore from {} and use an older binary.",
        detected, FORMAT_TAG, backup.display(),
    ));
}
