pub mod add;
pub mod audit;
pub mod coverage;
pub mod delete;
pub mod export;
pub mod help_cmd;
pub mod hooks;
pub mod init;
pub mod link;
pub mod list;
pub mod renumber;
pub mod repair;
pub mod show;
pub mod status;
pub mod test_cmd;
pub mod update;
pub mod validate_cmd;

use chrono::Utc;
use std::env;

use crate::model::HistoryEntry;

// Implements REQ-0022 (resolve actor from REQ_ACTOR / USER / USERNAME).
pub fn current_actor() -> String {
    env::var("REQ_ACTOR")
        .or_else(|_| env::var("USER"))
        .or_else(|_| env::var("USERNAME"))
        .unwrap_or_else(|_| "unknown".to_string())
}

pub fn history(action: impl Into<String>, reason: Option<String>) -> HistoryEntry {
    HistoryEntry {
        at: Utc::now(),
        actor: current_actor(),
        action: action.into(),
        reason,
    }
}
