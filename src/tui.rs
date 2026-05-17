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
    "Browse / view (list + show)",
    "Status",
    "Next requirement to work on",
    "Add",
    "Update",
    "Link",
    "Delete (mark obsolete)",
    "Split a compound requirement",
    "Validate project",
    "Coverage report",
    "Stale report",
    "Review (PR-style spec report)",
    "Doctor (setup audit)",
    "Diff between git refs",
    "Audit (git signature trail)",
    "Export to markdown (stdout)",
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
        "Coverage report" => commands::coverage::run(default_coverage(), file),
        "Stale report" => commands::stale::run(default_stale(), file),
        "Review (PR-style spec report)" => commands::review::run(
            crate::cli::ReviewArgs {
                base: "origin/main".into(),
                path: PathBuf::from("."),
                gate: false,
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
        "Version" => commands::version::run(VersionArgs { json: false }),
        "Quit" => Err(anyhow::anyhow!("__quit__")),
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
