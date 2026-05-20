// Discharges REQ-0001 for the project-creation sub-surface.
// Discharges REQ-0001 (project-creation sub-surface) and REQ-0075 (directory
// layout selection at init time).
use anyhow::{anyhow, Result};

use crate::cli::{InitArgs, LayoutArg};
use crate::model::{Project, PURPOSE_MAX_CHARS};
use crate::storage;

pub fn run(args: InitArgs) -> Result<()> {
    if args.output.exists() && !args.force {
        return Err(anyhow!(
            "{} already exists — pass --force to overwrite",
            args.output.display()
        ));
    }
    let mut project = Project::new(args.name);
    if let Some(p) = args.purpose.as_ref() {
        let trimmed = p.trim();
        if trimmed.chars().count() > PURPOSE_MAX_CHARS {
            return Err(anyhow!(
                "--purpose is {} chars; max {}. Tighten it — `req brief` leads with this line.",
                trimmed.chars().count(),
                PURPOSE_MAX_CHARS
            ));
        }
        if !trimmed.is_empty() {
            project.purpose = Some(trimmed.to_string());
        }
    }
    match args.layout {
        LayoutArg::Single => storage::save(&args.output, &project)?,
        LayoutArg::Directory => storage::save_directory(&args.output, &project)?,
    }
    println!(
        "Initialized empty .req project '{}' at {} ({} layout)",
        project.name,
        args.output.display(),
        match args.layout {
            LayoutArg::Single => "single-file",
            LayoutArg::Directory => "directory",
        },
    );
    println!();
    println!("Next steps:");
    println!("  req help agents --install      # write the agent trigger table into AGENTS.md");
    println!("  req hooks install              # pre-commit + merge driver (add --claude-code");
    println!("                                 #   to also write .claude/settings.json)");
    println!("  req mcp --init-config          # bootstrap .mcp.json for MCP-capable clients");
    println!("  req add --help                 # add your first requirement");
    Ok(())
}
