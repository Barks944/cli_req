// Top-level dispatch for REQ-0001 (single managed CLI binary).
// Top-level dispatch for REQ-0001 (single managed CLI binary).
mod cli;
mod commands;
mod errors;
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
    let json_mode = cli.command.is_json();

    match run(cli) {
        Ok(()) => Ok(()),
        Err(e) => {
            if json_mode {
                let code = errors::classify(&e);
                errors::emit(code, e.to_string(), errors::hint_for(code));
            }
            Err(e)
        }
    }
}

fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Init(args) => commands::init::run(args),
        Command::Add(args) => commands::add::run(args, &cli.file),
        Command::List(args) => commands::list::run(args, &cli.file),
        Command::Show(args) => commands::show::run(args, &cli.file),
        Command::Update(args) => commands::update::run(args, &cli.file),
        Command::Delete(args) => commands::delete::run(args, &cli.file),
        Command::Link(args) => commands::link::run(args, &cli.file),
        Command::Validate(args) => commands::validate_cmd::run(args, &cli.file),
        Command::Status(args) => commands::status::run(args, &cli.file),
        Command::Test(t) => commands::test_cmd::run(t, &cli.file),
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
