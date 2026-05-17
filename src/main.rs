// Top-level dispatch for REQ-0001 (single managed CLI binary).
mod cli;
mod commands;
mod help_text;
mod mcp;
mod model;
mod storage;
mod tui;
mod validate;
mod web;

use anyhow::Result;
use clap::Parser;
use cli::{Cli, Command};

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Init(args) => commands::init::run(args),
        Command::Add(args) => commands::add::run(args, &cli.file),
        Command::List(args) => commands::list::run(args, &cli.file),
        Command::Show(args) => commands::show::run(args, &cli.file),
        Command::Update(args) => commands::update::run(args, &cli.file),
        Command::Delete(args) => commands::delete::run(args, &cli.file),
        Command::Link(args) => commands::link::run(args, &cli.file),
        Command::Validate => commands::validate_cmd::run(&cli.file),
        Command::Export(args) => commands::export::run(args, &cli.file),
        Command::Tui => tui::run(&cli.file),
        Command::Serve(args) => web::run(args, &cli.file),
        Command::Mcp(args) => mcp::run(args, &cli.file),
        Command::Help(args) => commands::help_cmd::run(args),
        Command::Repair(args) => commands::repair::run(args, &cli.file),
        Command::Hooks(args) => commands::hooks::run(args),
        Command::Renumber(args) => commands::renumber::run(args, &cli.file),
        Command::Coverage(args) => commands::coverage::run(args, &cli.file),
        Command::Audit(args) => commands::audit::run(args, &cli.file),
    }
}
