pub mod add;
pub mod audit;
pub mod check;
pub mod coverage;
pub mod delete;
pub mod export;
pub mod help_cmd;
pub mod hooks;
pub mod init;
pub mod link;
pub mod list;
pub mod next;
pub mod renumber;
pub mod repair;
pub mod version;
pub mod show;
pub mod stale;
pub mod status;
pub mod test_cmd;
pub mod update;
pub mod validate_cmd;

use chrono::Utc;
use std::env;

use crate::model::{ActorKind, HistoryEntry};

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

pub fn history(action: impl Into<String>, reason: Option<String>) -> HistoryEntry {
    HistoryEntry {
        at: Utc::now(),
        actor: current_actor(),
        actor_kind: current_actor_kind(),
        action: action.into(),
        reason,
    }
}
