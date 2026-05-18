// REQ-0104: session-start brief.
// Designed to be the first thing an agent runs when picking up a
// project in a new conversation. Short by default so the agent's
// context isn't flooded; `--full` for the dashboard view; `--json`
// for tooling. Always read-only.
use anyhow::Result;
use serde_json::json;
use std::path::PathBuf;
use std::process::Command;

use crate::cli::BriefArgs;
use crate::model::{Project, Status};
use crate::storage::load_resolved;

pub fn run(args: BriefArgs, file: &Option<PathBuf>) -> Result<()> {
    let (_, project) = load_resolved(file)?;
    let snap = snapshot(&project);

    if args.json {
        println!("{}", serde_json::to_string_pretty(&snap.to_json())?);
        return Ok(());
    }

    if args.full {
        print!("{}", snap.to_full(&project));
    } else {
        print!("{}", snap.to_short(&project));
    }
    Ok(())
}

struct Snapshot {
    name: String,
    total: usize,
    by_status: [usize; 6],
    delivery_pct: f64,
    next_pick: Option<(String, String, String, String)>, // (id, title, status, priority)
    implemented_unverified: Vec<String>,
    drafts: Vec<String>,
    hook_mode: Option<String>,
    last_change: Option<String>,
}

fn snapshot(project: &Project) -> Snapshot {
    let total = project.requirements.len();
    let mut by_status = [0usize; 6];
    for r in project.requirements.values() {
        let i = match r.status {
            Status::Draft => 0,
            Status::Proposed => 1,
            Status::Approved => 2,
            Status::Implemented => 3,
            Status::Verified => 4,
            Status::Obsolete => 5,
        };
        by_status[i] += 1;
    }
    let non_obsolete = total - by_status[5];
    let done = by_status[3] + by_status[4];
    let delivery_pct = if non_obsolete == 0 {
        0.0
    } else {
        100.0 * done as f64 / non_obsolete as f64
    };

    // Next pick: same precedence as `req next` (highest priority,
    // earliest in lifecycle, satisfied dependencies). Simplified — we
    // sort and grab the first non-Verified, non-Obsolete.
    let mut candidates: Vec<&crate::model::Requirement> = project
        .requirements
        .values()
        .filter(|r| !matches!(r.status, Status::Verified | Status::Obsolete))
        .collect();
    candidates.sort_by_key(|r| {
        use crate::model::Priority;
        let p = match r.priority {
            Priority::Must => 0,
            Priority::Should => 1,
            Priority::Could => 2,
            Priority::Wont => 3,
        };
        let s = match r.status {
            Status::Draft => 0,
            Status::Proposed => 1,
            Status::Approved => 2,
            Status::Implemented => 3,
            _ => 9,
        };
        (p, s, r.id.clone())
    });
    let next_pick = candidates.first().map(|r| {
        (
            r.id.clone(),
            r.title.clone(),
            r.status.as_str().to_string(),
            r.priority.as_str().to_string(),
        )
    });

    // Loose ends: Implemented but never Verified (the natural next
    // step on these is `req verify ... --promote`).
    let mut implemented_unverified: Vec<String> = project
        .requirements
        .iter()
        .filter(|(_, r)| matches!(r.status, Status::Implemented))
        .map(|(id, _)| id.clone())
        .collect();
    implemented_unverified.sort();

    let mut drafts: Vec<String> = project
        .requirements
        .iter()
        .filter(|(_, r)| matches!(r.status, Status::Draft))
        .map(|(id, _)| id.clone())
        .collect();
    drafts.sort();

    let hook_mode = detect_hook_mode();
    let last_change = detect_last_spec_change();

    Snapshot {
        name: project.name.clone(),
        total,
        by_status,
        delivery_pct,
        next_pick,
        implemented_unverified,
        drafts,
        hook_mode,
        last_change,
    }
}

fn detect_hook_mode() -> Option<String> {
    let body = std::fs::read_to_string(".git/hooks/pre-commit").ok()?;
    if body.contains("# mode: strict") {
        Some("strict".into())
    } else if body.contains("# mode: default") {
        Some("default".into())
    } else if body.contains("# managed-by: req-hooks") {
        Some("managed (mode unknown)".into())
    } else {
        None
    }
}

fn detect_last_spec_change() -> Option<String> {
    let out = Command::new("git")
        .args(["log", "-1", "--format=%cr", "--", "project.req"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

impl Snapshot {
    fn to_short(&self, _project: &Project) -> String {
        let mut out = String::new();
        // Headline
        out.push_str(&format!(
            "req brief: {} — {} req(s), {:.0}% delivered",
            self.name, self.total, self.delivery_pct
        ));
        if self.by_status[0] > 0 {
            out.push_str(&format!(", {} draft", self.by_status[0]));
        }
        out.push_str(".\n");

        // Next pick
        match &self.next_pick {
            Some((id, title, status, priority)) => {
                let title_short: String = title.chars().take(60).collect();
                out.push_str(&format!(
                    "  next : {} [{} / {}] — {}\n",
                    id, priority, status, title_short
                ));
            }
            None => out.push_str("  next : nothing queued. Add one with `req add` or relax filters with `req next`.\n"),
        }

        // Loose ends
        if !self.implemented_unverified.is_empty() {
            let preview = self
                .implemented_unverified
                .iter()
                .take(3)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ");
            let more = if self.implemented_unverified.len() > 3 {
                format!(" +{} more", self.implemented_unverified.len() - 3)
            } else {
                String::new()
            };
            out.push_str(&format!(
                "  loose: {} implemented but not verified — {}{}\n",
                self.implemented_unverified.len(),
                preview,
                more
            ));
        }

        out.push_str("        `req brief --full` for the dashboard · `req next` to start work\n");
        out
    }

    fn to_full(&self, project: &Project) -> String {
        let mut out = String::new();
        out.push_str(&format!("# req brief — {}\n\n", self.name));
        out.push_str(&format!(
            "**{} requirements · {:.1}% delivered**\n\n",
            self.total, self.delivery_pct
        ));

        out.push_str("## Status breakdown\n\n");
        let labels = [
            "draft",
            "proposed",
            "approved",
            "implemented",
            "verified",
            "obsolete",
        ];
        for (i, lbl) in labels.iter().enumerate() {
            if self.by_status[i] > 0 {
                out.push_str(&format!("  - {:>12}: {}\n", lbl, self.by_status[i]));
            }
        }
        out.push('\n');

        out.push_str("## Suggested next\n\n");
        match &self.next_pick {
            Some((id, title, status, priority)) => out.push_str(&format!(
                "  **{}** — {}\n  status: {} · priority: {}\n",
                id, title, status, priority
            )),
            None => out.push_str("  nothing queued.\n"),
        }
        out.push('\n');

        if !self.drafts.is_empty() {
            out.push_str(&format!("## Drafts ({})\n\n", self.drafts.len()));
            for id in &self.drafts {
                if let Some(r) = project.requirements.get(id) {
                    out.push_str(&format!("  - {} — {}\n", id, r.title));
                }
            }
            out.push('\n');
        }

        if !self.implemented_unverified.is_empty() {
            out.push_str(&format!(
                "## Implemented but not Verified ({})\n\n  next step on each: `req verify <id> --by inspection --notes \"...\" --promote`\n\n",
                self.implemented_unverified.len()
            ));
            for id in &self.implemented_unverified {
                if let Some(r) = project.requirements.get(id) {
                    out.push_str(&format!("  - {} — {}\n", id, r.title));
                }
            }
            out.push('\n');
        }

        out.push_str("## Tooling\n\n");
        out.push_str(&format!(
            "  pre-commit hook: {}\n",
            self.hook_mode.as_deref().unwrap_or("not installed")
        ));
        out.push_str(&format!(
            "  last spec change: {}\n",
            self.last_change.as_deref().unwrap_or("(no git history)")
        ));
        out.push_str("\n  `req lint` for quality audit · `req review` for PR-style report\n");

        out
    }

    fn to_json(&self) -> serde_json::Value {
        json!({
            "project": self.name,
            "total": self.total,
            "by_status": {
                "draft":       self.by_status[0],
                "proposed":    self.by_status[1],
                "approved":    self.by_status[2],
                "implemented": self.by_status[3],
                "verified":    self.by_status[4],
                "obsolete":    self.by_status[5],
            },
            "delivery_pct": (self.delivery_pct * 10.0).round() / 10.0,
            "next": self.next_pick.as_ref().map(|(id, title, status, priority)| {
                json!({ "id": id, "title": title, "status": status, "priority": priority })
            }),
            "implemented_unverified": self.implemented_unverified,
            "drafts": self.drafts,
            "hook_mode": self.hook_mode,
            "last_spec_change": self.last_change,
        })
    }
}
