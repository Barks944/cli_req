// Discharges REQ-0001 for the project-creation sub-surface.
use anyhow::{anyhow, Result};

use crate::cli::InitArgs;
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
    storage::save(&args.output, &project)?;
    println!(
        "Initialized empty .req project '{}' at {}",
        project.name,
        args.output.display()
    );
    Ok(())
}
