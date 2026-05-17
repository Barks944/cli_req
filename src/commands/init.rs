// Discharges REQ-0001 for the project-creation sub-surface.
// Discharges REQ-0001 (project-creation sub-surface) and REQ-0075 (directory
// layout selection at init time).
use anyhow::{anyhow, Result};

use crate::cli::{InitArgs, LayoutArg};
use crate::model::Project;
use crate::storage;

pub fn run(args: InitArgs) -> Result<()> {
    if args.output.exists() && !args.force {
        return Err(anyhow!(
            "{} already exists — pass --force to overwrite",
            args.output.display()
        ));
    }
    let project = Project::new(args.name);
    match args.layout {
        LayoutArg::Single => storage::save(&args.output, &project)?,
        LayoutArg::Directory => storage::save_directory(&args.output, &project)?,
    }
    println!(
        "Initialized empty .req project '{}' at {} ({} layout)",
        project.name,
        args.output.display(),
        match args.layout { LayoutArg::Single => "single-file", LayoutArg::Directory => "directory" },
    );
    Ok(())
}
