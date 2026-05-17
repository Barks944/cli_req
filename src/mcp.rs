use anyhow::Result;
use std::path::PathBuf;

pub fn run(_file: &Option<PathBuf>) -> Result<()> {
    eprintln!("MCP server not yet implemented — coming in a later iteration.");
    eprintln!("for now, agents should shell out to `req <subcommand>`.");
    std::process::exit(2);
}
