pub mod add;
pub mod adopt;
pub mod audit;
pub mod batch;
pub mod brief;
pub mod check;
pub mod coverage;
pub mod delete;
pub mod diff;
pub mod doctor;
pub mod export;
pub mod help_cmd;
pub mod hooks;
pub mod import;
pub mod init;
pub mod link;
pub mod lint;
pub mod list;
pub mod migrate;
pub mod next;
pub mod precheck;
// REQ-0142: verification-provenance classifier, kept in its own small module
// so the safety requirement that depends on it anchors a stable file.
pub mod provenance;
pub mod purpose;
pub mod renumber;
pub mod repair;
pub mod review;
pub mod safety;
pub mod safety_gov;
pub mod schema;
pub mod setup;
pub mod show;
pub mod split;
pub mod stale;
pub mod status;
pub mod test_cmd;
pub mod update;
pub mod validate_cmd;
pub mod validation;
pub mod version;

use chrono::Utc;
use std::env;

use crate::model::{ActorKind, HistoryEntry, Project};

/// REQ-0090: case- and pad-insensitive ID resolution with did-you-mean.
/// Normalise a user-typed REQ-ID to the project's canonical form.
/// Accepts "req-0001", "REQ-1", "req-1", or just "1"; returns the
/// canonical "REQ-0001" if the form is unambiguous. Returns the input
/// unchanged when it doesn't look like a REQ-ID so the caller can
/// surface a normal "no such requirement" error.
pub fn normalize_id(raw: &str) -> String {
    let trimmed = raw.trim();
    let upper = trimmed.to_uppercase();
    let digits = if let Some(rest) = upper.strip_prefix("REQ-") {
        rest
    } else if trimmed.chars().all(|c| c.is_ascii_digit()) && !trimmed.is_empty() {
        trimmed
    } else {
        return trimmed.to_string();
    };
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return trimmed.to_string();
    }
    let n: u32 = digits.parse().unwrap_or(0);
    format!("REQ-{:04}", n)
}

/// Resolve an ID against the project, normalising case/padding. On
/// miss, suggest the lexically nearest existing ID if one is close
/// enough to be helpful.
pub fn resolve_id(project: &Project, raw: &str) -> anyhow::Result<String> {
    let canonical = normalize_id(raw);
    if project.requirements.contains_key(&canonical) {
        return Ok(canonical);
    }
    let suggestion = nearest_id(project, &canonical);
    let hint = match suggestion {
        Some(s) => format!(" — did you mean {}?", s),
        None => String::new(),
    };
    Err(anyhow::anyhow!("no such requirement: {}{}", raw, hint))
}

fn nearest_id(project: &Project, target: &str) -> Option<String> {
    let target_n = target
        .strip_prefix("REQ-")
        .and_then(|n| n.parse::<i64>().ok())?;
    project
        .requirements
        .keys()
        .filter_map(|k| {
            k.strip_prefix("REQ-")
                .and_then(|n| n.parse::<i64>().ok())
                .map(|n| (k.clone(), (n - target_n).abs()))
        })
        .min_by_key(|(_, d)| *d)
        .filter(|(_, d)| *d <= 5)
        .map(|(k, _)| k)
}

// Implements REQ-0022 (resolve actor from REQ_ACTOR / USER / USERNAME).
pub fn current_actor() -> String {
    env::var("REQ_ACTOR")
        .or_else(|_| env::var("USER"))
        .or_else(|_| env::var("USERNAME"))
        .unwrap_or_else(|_| "unknown".to_string())
}

pub fn current_actor_kind() -> ActorKind {
    match env::var("REQ_ACTOR_KIND").ok().as_deref() {
        Some(s) if s.eq_ignore_ascii_case("human") => ActorKind::Human,
        Some(s) if s.eq_ignore_ascii_case("agent") => ActorKind::Agent,
        _ => ActorKind::Unknown,
    }
}

/// REQ-0167: the human on whose behalf the acting agent is working, from
/// `REQ_ON_BEHALF_OF`. Empty/unset yields None. Trimmed so a stray blank
/// does not record an empty attribution.
pub fn current_on_behalf_of() -> Option<String> {
    env::var("REQ_ON_BEHALF_OF")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

pub fn history(action: impl Into<String>, reason: Option<String>) -> HistoryEntry {
    HistoryEntry {
        at: Utc::now(),
        actor: current_actor(),
        actor_kind: current_actor_kind(),
        action: action.into(),
        reason,
        on_behalf_of: current_on_behalf_of(),
    }
}

/// REQ-0161: a forced, irregular change must carry a substantive `--reason`.
/// An empty or near-empty reason makes a deliberate correction
/// indistinguishable from a careless override in the audit trail, so reject
/// anything shorter than `min_len` trimmed characters.
pub fn ensure_force_reason(reason: &Option<String>, min_len: usize) -> anyhow::Result<()> {
    let len = reason
        .as_deref()
        .map(|r| r.trim().chars().count())
        .unwrap_or(0);
    if len < min_len {
        return Err(anyhow::anyhow!(
            "a forced change needs a substantive --reason (≥{} characters; got {}). \
             Explain why this irregular change is correct so the audit trail records the why.",
            min_len,
            len
        ));
    }
    Ok(())
}
