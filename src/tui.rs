// Implements REQ-0015 (interactive terminal menu for humans).
use anyhow::Result;
use dialoguer::{theme::ColorfulTheme, Select};
use std::path::PathBuf;

use crate::cli::{
    AddArgs, DeleteArgs, ExportArgs, ExportFormat, ListArgs, ShowArgs, UpdateArgs,
};
use crate::commands;
use crate::storage::load_resolved;

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

        let actions = &[
            "Browse / view",
            "Add",
            "Update",
            "Delete (mark obsolete)",
            "Validate project",
            "Export to markdown (stdout)",
            "Quit",
        ];
        let idx = Select::with_theme(&theme)
            .with_prompt("Action")
            .items(actions)
            .default(0)
            .interact()?;

        match idx {
            0 => browse(file, &project)?,
            1 => commands::add::run(default_add(), file)?,
            2 => {
                if let Some(id) = pick_id(&theme, &project)? {
                    commands::update::run(default_update(id), file)?;
                }
            }
            3 => {
                if let Some(id) = pick_id(&theme, &project)? {
                    commands::delete::run(DeleteArgs { id, hard: false, reason: None, json: false }, file)?;
                }
            }
            4 => commands::validate_cmd::run(crate::cli::ValidateArgs { json: false }, file)?,
            5 => commands::export::run(
                ExportArgs { format: ExportFormat::Markdown, output: "-".into() },
                file,
            )?,
            _ => return Ok(()),
        }
    }
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
        commands::show::run(ShowArgs { id: ids[i].clone(), json: false }, file)?;
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
    let pick = Select::with_theme(theme).items(&labels).default(0).interact_opt()?;
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
        json: false,
    }
}

// Unused for now; kept for completeness so list-style filtering compiles if reused.
#[allow(dead_code)]
fn default_list() -> ListArgs {
    ListArgs {
        status: None,
        kind: None,
        priority: None,
        tag: vec![],
        query: None,
        json: false,
    }
}
