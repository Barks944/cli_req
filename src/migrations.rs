// REQ-0116: format-migration registry. The contract this module owns
// is small but load-bearing: the project.req file is the source of
// truth that adopters commit to git, so it has to travel forward
// through tool upgrades without re-authoring. When the schema changes
// (REQ-0110 _config, REQ-0111 _purpose will be the first such bump),
// a migration step is registered here and `req migrate` walks the
// chain from the file's `_format` to the current FORMAT_TAG.
//
// A step is a function that takes the parsed JSON object root at one
// format and returns it transformed to the next format. Steps are
// pure: they MUST NOT touch the filesystem (that's `req migrate`'s
// job, including the sibling backup and re-signing).
use anyhow::Result;
use serde_json::{Map, Value};

/// Signature of a migration step's `apply` function: takes the
/// unwrapped object root at one format and returns it transformed.
pub type MigrationFn = fn(Map<String, Value>) -> Result<Map<String, Value>>;

/// One step in the migration chain. `from` is the `_format` value the
/// step accepts as input; `to` is the `_format` value it produces.
pub struct MigrationStep {
    pub from: &'static str,
    pub to: &'static str,
    /// Pure transformation on the unwrapped object root (no `_format`
    /// or `_integrity` keys present). The step returns the new root.
    pub apply: MigrationFn,
}

/// The ordered list of migration steps this binary knows. Append to
/// this when introducing a new `_format`. The current binary's
/// FORMAT_TAG (in storage.rs) must equal the `to` field of the last
/// entry, or the empty list's implicit terminus.
pub fn registered_steps() -> Vec<MigrationStep> {
    vec![MigrationStep {
        from: "req-v1",
        to: "req-v2",
        apply: v1_to_v2,
    }]
}

/// REQ-0110 + REQ-0111: v1 → v2 introduces two reserved top-level keys
/// (`_config` and `_purpose`). Both are optional, so the migration is
/// a pure pass-through — we don't synthesise either field. Existing
/// requirements, history entries, links, and test records are
/// preserved byte-for-byte.
fn v1_to_v2(root: Map<String, Value>) -> Result<Map<String, Value>> {
    Ok(root)
}

/// Walk the registry from `detected` toward `target`, applying each
/// step in turn. Returns the final root and the format tag it now
/// carries. Errors when no path exists; this is the signal the user
/// needs to upgrade the binary or restore from backup.
pub fn walk_chain(
    mut root: Map<String, Value>,
    detected: &str,
    target: &str,
) -> Result<(Map<String, Value>, String)> {
    if detected == target {
        return Ok((root, target.to_string()));
    }
    let steps = registered_steps();
    let mut current = detected.to_string();
    let mut applied: Vec<String> = Vec::new();
    loop {
        if current == target {
            return Ok((root, current));
        }
        let next = steps.iter().find(|s| s.from == current);
        match next {
            Some(step) => {
                root = (step.apply)(root)?;
                applied.push(format!("{} → {}", step.from, step.to));
                current = step.to.to_string();
            }
            None => {
                return Err(anyhow::anyhow!(
                    "no migration path from {} to {} (applied so far: {})",
                    current,
                    target,
                    if applied.is_empty() {
                        "none".to_string()
                    } else {
                        applied.join(", ")
                    }
                ));
            }
        }
    }
}
