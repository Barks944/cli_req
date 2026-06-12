// Implements REQ-0015 (interactive terminal menu for humans) and the TUI
// half of REQ-0083 (cross-surface parity): the menu exposes every
// agent-relevant CLI operation so a human at the terminal can achieve
// the same things an agent reaches for via MCP or the flag CLI.
use anyhow::Result;
use dialoguer::{theme::ColorfulTheme, Input, Select};
use std::path::PathBuf;

use crate::cli::{
    AddArgs, CoverageArgs, DeleteArgs, DiffArgs, DoctorArgs, ExportArgs, ExportFormat, ListArgs,
    NextArgs, ShowArgs, StaleArgs, StatusArgs, UpdateArgs, ValidateArgs, VersionArgs,
};
use crate::commands;
use crate::storage::load_resolved;

/// The full TUI menu. Each entry maps to a top-level CLI command so the
/// surfaces stay one-to-one. See REQ-0083 for the parity contract.
pub const MENU: &[&str] = &[
    // REQ-0104: session brief is the natural first action.
    "Brief (where are we now?)",
    "Browse / view (list + show)",
    "Status",
    "Next requirement to work on",
    "Add",
    "Update",
    "Link",
    "Delete (mark obsolete)",
    "Split a compound requirement",
    "Validate project",
    // REQ-0101: lint menu entry.
    "Lint (quality audit)",
    "Coverage report",
    "Stale report",
    "Review (PR-style spec report)",
    "Doctor (setup audit)",
    "Diff between git refs",
    "Audit (git signature trail)",
    "Export to markdown (stdout)",
    // REQ-0134: functional-safety review surface for humans — browse
    // hazards / SF / SR and trace a safety case. (Label avoids commas so
    // the parity guard's comma-split keeps it as one entry.)
    "Safety: hazards / SF / SR (sreq) / trace",
    "Version",
    "Quit",
];

pub fn run(file: &Option<PathBuf>) -> Result<()> {
    let theme = ColorfulTheme::default();
    loop {
        let (_, project) = load_resolved(file)?;
        let header = format!(
            "req — {} ({} requirements)",
            project.name,
            project.requirements.len()
        );
        println!("\n{}", header);
        println!("{}", "=".repeat(header.len()));

        let idx = Select::with_theme(&theme)
            .with_prompt("Action")
            .items(MENU)
            .default(0)
            .interact()?;

        let action = match dispatch(idx, file, &project, &theme) {
            Ok(()) => continue,
            Err(e) if e.to_string() == "__quit__" => return Ok(()),
            Err(e) => Err(e),
        };
        action?;
    }
}

fn dispatch(
    idx: usize,
    file: &Option<PathBuf>,
    project: &crate::model::Project,
    theme: &ColorfulTheme,
) -> Result<()> {
    match MENU[idx] {
        // REQ-0104: brief from the TUI.
        "Brief (where are we now?)" => commands::brief::run(
            crate::cli::BriefArgs {
                full: false,
                json: false,
            },
            file,
        ),
        "Browse / view (list + show)" => browse(file, project),
        "Status" => commands::status::run(
            StatusArgs {
                tag: vec![],
                json: false,
            },
            file,
        ),
        "Next requirement to work on" => commands::next::run(default_next(), file),
        "Add" => commands::add::run(default_add(), file),
        "Update" => {
            if let Some(id) = pick_id(theme, project)? {
                commands::update::run(default_update(id), file)?;
            }
            Ok(())
        }
        "Link" => link_flow(file, project, theme),
        "Delete (mark obsolete)" => {
            if let Some(id) = pick_id(theme, project)? {
                commands::delete::run(
                    DeleteArgs {
                        id,
                        hard: false,
                        reason: None,
                        json: false,
                    },
                    file,
                )?;
            }
            Ok(())
        }
        "Split a compound requirement" => split_flow(file, theme),
        "Validate project" => commands::validate_cmd::run(ValidateArgs { json: false }, file),
        // REQ-0101: lint TUI dispatch.
        "Lint (quality audit)" => commands::lint::run(
            crate::cli::LintArgs {
                path: PathBuf::from("."),
                json: false,
            },
            file,
        ),
        "Coverage report" => commands::coverage::run(default_coverage(), file),
        "Stale report" => commands::stale::run(default_stale(), file),
        "Review (PR-style spec report)" => commands::review::run(
            crate::cli::ReviewArgs {
                base: "origin/main".into(),
                path: PathBuf::from("."),
                ext: vec![],
                ignore: vec![],
                staged: false,
                marker_near_hunks: 0,
                gate: false,
                no_defects: false,
                summary: false,
                new: false,
                all: false,
                json: false,
            },
            file,
        ),
        "Doctor (setup audit)" => commands::doctor::run(DoctorArgs { json: false }),
        "Diff between git refs" => diff_flow(file, theme),
        "Audit (git signature trail)" => audit_flow(file),
        "Export to markdown (stdout)" => commands::export::run(
            ExportArgs {
                format: ExportFormat::Markdown,
                output: "-".into(),
            },
            file,
        ),
        "Safety: hazards / SF / SR (sreq) / trace" => safety_flow(file, theme),
        "Version" => commands::version::run(VersionArgs { json: false }),
        "Quit" => Err(anyhow::anyhow!("__quit__")),
        _ => Ok(()),
    }
}

/// REQ-0134: read-oriented functional-safety browser. Authoring (add /
/// assess / verify) stays on the CLI/MCP surface where the multi-field
/// risk parameters belong; the TUI exists so a human reviewer can walk
/// the hazard log and trace a safety case end to end.
fn safety_flow(file: &Option<PathBuf>, theme: &ColorfulTheme) -> Result<()> {
    let opts = [
        "Hazards (list)",
        "Safety functions (list)",
        "Safety requirements (sreq, list)",
        "Trace a safety case",
        "Back",
    ];
    let idx = Select::with_theme(theme)
        .with_prompt("Safety")
        .items(&opts)
        .default(0)
        .interact()?;
    if idx == 3 {
        let id: String = dialoguer::Input::with_theme(theme)
            .with_prompt("Trace which id? (HAZ-/SF-/SR-NNNN)")
            .interact_text()?;
        if id.trim().is_empty() {
            return Ok(());
        }
        return commands::safety::run_trace(
            crate::cli::TraceArgs { id, json: false },
            file,
        );
    }
    // The three list views are non-interactive — factored out so they can
    // be smoke-tested without driving the terminal menu (REQ-0134).
    safety_review(idx, file)
}

/// REQ-0134: the read-only safety review actions reachable from the TUI
/// menu, separated from the interactive Select so they are unit-testable.
fn safety_review(action: usize, file: &Option<PathBuf>) -> Result<()> {
    use crate::cli::{HazardCmd, HazardListArgs, SfCmd, SfListArgs, SreqCmd, SreqListArgs};
    match action {
        0 => commands::safety::run_hazard(
            HazardCmd::List(HazardListArgs {
                sil: None,
                status: None,
                unmitigated: false,
                json: false,
            }),
            file,
        ),
        1 => commands::safety::run_sf(
            SfCmd::List(SfListArgs {
                sil: None,
                status: None,
                unrealized: false,
                json: false,
            }),
            file,
        ),
        2 => commands::safety::run_sreq(
            SreqCmd::List(SreqListArgs {
                sil: None,
                status: None,
                unverified: false,
                json: false,
            }),
            file,
        ),
        _ => Ok(()),
    }
}

fn split_flow(file: &Option<PathBuf>, _theme: &ColorfulTheme) -> Result<()> {
    use dialoguer::Input;
    let id: String = Input::with_theme(_theme)
        .with_prompt("Requirement to split (e.g. REQ-0042)")
        .interact_text()?;
    let reason: String = Input::with_theme(_theme)
        .with_prompt("Reason (recorded on history)")
        .allow_empty(true)
        .interact_text()?;
    // Delegate the interactive part-statement prompting to split::run
    // by passing an empty `into` (it prompts when empty).
    commands::split::run(
        crate::cli::SplitArgs {
            id,
            into: vec![],
            reason: if reason.trim().is_empty() {
                None
            } else {
                Some(reason)
            },
            keep_original: false,
            json: false,
        },
        file,
    )
}

fn link_flow(
    file: &Option<PathBuf>,
    project: &crate::model::Project,
    theme: &ColorfulTheme,
) -> Result<()> {
    let from = match pick_id(theme, project)? {
        Some(id) => id,
        None => return Ok(()),
    };
    let to = match pick_id(theme, project)? {
        Some(id) => id,
        None => return Ok(()),
    };
    let kinds = ["parent", "depends-on", "refines", "conflicts", "verifies"];
    let idx = Select::with_theme(theme)
        .with_prompt("Link kind")
        .items(&kinds)
        .default(0)
        .interact()?;
    let kind = match kinds[idx] {
        "parent" => crate::cli::LinkKindArg::Parent,
        "depends-on" => crate::cli::LinkKindArg::DependsOn,
        "refines" => crate::cli::LinkKindArg::Refines,
        "conflicts" => crate::cli::LinkKindArg::Conflicts,
        _ => crate::cli::LinkKindArg::Verifies,
    };
    commands::link::run(
        crate::cli::LinkArgs {
            from,
            to,
            kind,
            remove: false,
            json: false,
        },
        file,
    )
}

fn diff_flow(file: &Option<PathBuf>, theme: &ColorfulTheme) -> Result<()> {
    let spec: String = Input::with_theme(theme)
        .with_prompt("Git diff spec (e.g. origin/main..HEAD)")
        .default("HEAD~1..HEAD".to_string())
        .interact_text()?;
    commands::diff::run(DiffArgs { spec, json: false }, file)
}

fn audit_flow(file: &Option<PathBuf>) -> Result<()> {
    use crate::cli::AuditArgs;
    commands::audit::run(
        AuditArgs {
            limit: 20,
            gate: false,
            require_good_signature: false,
            required_signers: Vec::new(),
            json: false,
        },
        file,
    )
}

fn browse(file: &Option<PathBuf>, project: &crate::model::Project) -> Result<()> {
    if project.requirements.is_empty() {
        println!("(no requirements yet)");
        return Ok(());
    }
    let theme = ColorfulTheme::default();
    let ids: Vec<String> = project.requirements.keys().cloned().collect();
    let labels: Vec<String> = ids
        .iter()
        .map(|id| {
            let r = &project.requirements[id];
            format!("{:<10} [{}] {}", id, r.status.as_str(), r.title)
        })
        .collect();
    let pick = Select::with_theme(&theme)
        .with_prompt("Pick a requirement")
        .items(&labels)
        .default(0)
        .interact_opt()?;
    if let Some(i) = pick {
        commands::show::run(
            ShowArgs {
                id: ids[i].clone(),
                json: false,
            },
            file,
        )?;
    }
    Ok(())
}

fn pick_id(theme: &ColorfulTheme, project: &crate::model::Project) -> Result<Option<String>> {
    if project.requirements.is_empty() {
        println!("(no requirements yet)");
        return Ok(None);
    }
    let ids: Vec<String> = project.requirements.keys().cloned().collect();
    let labels: Vec<String> = ids
        .iter()
        .map(|id| format!("{} — {}", id, project.requirements[id].title))
        .collect();
    let pick = Select::with_theme(theme)
        .items(&labels)
        .default(0)
        .interact_opt()?;
    Ok(pick.map(|i| ids[i].clone()))
}

fn default_add() -> AddArgs {
    AddArgs {
        title: None,
        statement: None,
        rationale: None,
        acceptance: vec![],
        kind: None,
        priority: None,
        tag: vec![],
        parent: None,
        interactive: true,
        json: false,
        from_json: None,
    }
}

fn default_update(id: String) -> UpdateArgs {
    UpdateArgs {
        id,
        title: None,
        statement: None,
        rationale: None,
        acceptance: None,
        add_acceptance: vec![],
        remove_acceptance: vec![],
        kind: None,
        priority: None,
        status: None,
        add_tag: vec![],
        remove_tag: vec![],
        reason: None,
        force: false,
        json: false,
    }
}

// Unused for now; kept for completeness so list-style filtering compiles if reused.
#[allow(dead_code)]
fn default_list() -> ListArgs {
    ListArgs {
        status: None,
        include_obsolete: false,
        kind: None,
        priority: None,
        tag: vec![],
        query: None,
        json: false,
    }
}

fn default_next() -> NextArgs {
    NextArgs {
        status: None,
        kind: None,
        priority: None,
        tag: vec![],
        json: false,
    }
}

fn default_coverage() -> CoverageArgs {
    CoverageArgs {
        path: PathBuf::from("."),
        extensions: vec![],
        unlinked_files: false,
        by_file: false,
        by_req: false,
        remap: vec![],
        apply: false,
        strict: false,
        allow_orphans: vec![],
        json: false,
    }
}

fn default_stale() -> StaleArgs {
    StaleArgs {
        path: PathBuf::from("."),
        only_stale: false,
        json: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // REQ-0134: the TUI safety-review dispatch routes the three list
    // actions to the safety commands without panicking. Drives
    // safety_review directly (the interactive Select is bypassed) against
    // a real on-disk project, which is the part worth covering — the menu
    // wiring, not dialoguer's terminal I/O.
    #[test]
    fn safety_review_dispatch_lists_without_panicking() {
        let dir = tempfile::Builder::new()
            .prefix("req-tui-")
            .tempdir()
            .unwrap();
        let path = dir.path().join("project.req");
        let mut project = crate::model::Project::new("p".into());
        // A minimal safety chain so the list views have something to show.
        let now = chrono::Utc::now();
        let hid = project.allocate_haz_id();
        project.hazards.insert(
            hid.clone(),
            crate::model::Hazard {
                id: hid,
                title: "H".into(),
                description: String::new(),
                operating_context: String::new(),
                harm: "hurt".into(),
                consequence: Some(crate::model::Consequence::Cc),
                frequency: Some(crate::model::Frequency::Fb),
                avoidance: Some(crate::model::Avoidance::Pb),
                probability: Some(crate::model::Probability::W3),
                status: crate::model::HazardStatus::Assessed,
                tags: vec![],
                links: vec![],
                created: now,
                updated: now,
                history: vec![],
            },
        );
        crate::storage::save(&path, &project).unwrap();
        let file = Some(path);
        // actions 0,1,2 are the non-interactive list views; 99 is the
        // "Back"/no-op arm.
        for action in [0usize, 1, 2, 99] {
            assert!(
                safety_review(action, &file).is_ok(),
                "safety_review({}) should not error",
                action
            );
        }
    }
}
