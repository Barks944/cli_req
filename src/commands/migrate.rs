// Implements REQ-0068 and REQ-0116: req migrate. Detects the project's
// _format tag and walks the registered migration chain (src/migrations.rs)
// to bring it to the current FORMAT_TAG. The structure (back up before
// mutating, re-sign on the way out) is the contract; the chain itself
// is empty today because req-v1 is the only format in the wild.
// See `req help format-policy`.
use anyhow::{anyhow, Context, Result};
use serde_json::{json, Map, Value};
use std::path::PathBuf;

use crate::cli::MigrateArgs;
use crate::migrations;
use crate::storage::{self, resolve_path, FORMAT_TAG};

pub fn run(args: MigrateArgs, file: &Option<PathBuf>) -> Result<()> {
    let path = resolve_path(file);
    let _lock = storage::acquire_lock(&path)?;

    let raw = std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    let mut root: serde_json::Map<String, Value> = serde_json::from_str(&raw)
        .with_context(|| format!("{} is not valid JSON", path.display()))?;
    let detected: String = root
        .get("_format")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("not a .req file: missing _format"))?
        .to_string();

    if detected == FORMAT_TAG {
        let msg = format!(
            "{} already at format {}; no migration needed.",
            path.display(),
            detected
        );
        if args.json {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "ok": true, "migrated": false, "from": detected, "to": FORMAT_TAG, "message": msg
                }))?
            );
        } else {
            println!("{}", msg);
        }
        return Ok(());
    }

    // Detect direction.
    if detected.as_str() > FORMAT_TAG {
        return Err(anyhow!(
            "{} is at format {} which is newer than this binary ({}). Upgrade the binary first.",
            path.display(),
            detected,
            FORMAT_TAG,
        ));
    }

    // Back up the source file before any mutation. The backup carries
    // the detected format in its extension so successive migrations
    // (v1 → v2 → v3) don't clobber the earliest snapshot.
    let backup = path.with_extension(format!("req.bak-{}", detected));
    std::fs::copy(&path, &backup).with_context(|| format!("write backup {}", backup.display()))?;

    // Unwrap the integrity envelope, walk the chain, re-sign.
    let stored_hash = root
        .remove("_integrity")
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .ok_or_else(|| anyhow!("missing _integrity field"))?;
    root.remove("_warning");
    root.remove("_instructions");
    root.remove("_format");

    // Verify the source-side integrity before mutating it so a damaged
    // file gets a precise error pointing at the right remedy (repair,
    // not migrate).
    let payload_before = Value::Object(root.clone());
    let computed = storage::integrity_hash(&payload_before);
    if computed != stored_hash {
        return Err(anyhow!(
            "integrity check failed for {} before migration — run \
             `req repair --confirm-direct-edit` first, then re-run migrate.",
            path.display()
        ));
    }

    let (migrated, ended_at) =
        migrations::walk_chain(root, &detected, FORMAT_TAG).map_err(|e| {
            anyhow!(
                "{}\n\nRestore from {} and use an older binary, or wait \
                 for a version of req that registers this migration.",
                e,
                backup.display()
            )
        })?;

    let final_payload = Value::Object(migrated);
    let new_hash = storage::integrity_hash(&final_payload);
    let mut final_root: Map<String, Value> = match final_payload {
        Value::Object(m) => m,
        _ => unreachable!("walk_chain returns Object root"),
    };
    final_root.insert("_format".into(), Value::String(ended_at.clone()));
    final_root.insert("_integrity".into(), Value::String(new_hash));
    let serialised = serde_json::to_string_pretty(&Value::Object(final_root))?;
    std::fs::write(&path, serialised).with_context(|| format!("write {}", path.display()))?;

    let msg = format!(
        "migrated {}: {} → {} (backup at {})",
        path.display(),
        detected,
        ended_at,
        backup.display()
    );
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "ok": true,
                "migrated": true,
                "from": detected,
                "to": ended_at,
                "backup": backup.display().to_string(),
                "message": msg,
            }))?
        );
    } else {
        println!("{}", msg);
    }
    Ok(())
}
