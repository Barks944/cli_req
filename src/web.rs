use anyhow::Result;
use std::path::PathBuf;

use crate::cli::ServeArgs;

pub fn run(_args: ServeArgs, _file: &Option<PathBuf>) -> Result<()> {
    eprintln!("web server not yet implemented — coming in a later iteration.");
    eprintln!("for now, use: req tui   |   req list   |   req export -f html");
    std::process::exit(2);
}
