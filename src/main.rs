// Top-level dispatch for REQ-0001 (single managed CLI binary).
// Top-level dispatch for REQ-0001 (single managed CLI binary).
mod cli;
mod commands;
mod errors;
mod help_text;
mod mcp;
mod migrations;
mod model;
mod source_walk;
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
                // The envelope is the parseable stdout output. Exit with
                // a non-zero code, but do NOT return Err — anyhow would
                // then write its full "Error: ... Caused by: ..." chain
                // to stderr, leaving callers with a non-JSON stream
                // mixed across stdout and stderr.
                let code = errors::classify(&e);
                errors::emit(code, e.to_string(), errors::hint_for(code));
                std::process::exit(1);
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
        Command::Verify(args) => commands::test_cmd::verify(args, &cli.file),
        Command::Stale(args) => commands::stale::run(args, &cli.file),
        Command::Batch(args) => commands::batch::run(args, &cli.file),
        Command::Import(args) => commands::import::run(args, &cli.file),
        Command::Migrate(args) => commands::migrate::run(args, &cli.file),
        Command::Schema(args) => commands::schema::run(args),
        Command::Version(args) => commands::version::run(args),
        Command::Next(args) => commands::next::run(args, &cli.file),
        Command::Check(args) => commands::check::run(args, &cli.file),
        Command::Doctor(args) => commands::doctor::run(args),
        Command::Diff(args) => commands::diff::run(args, &cli.file),
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
        Command::Review(args) => commands::review::run(args, &cli.file),
        Command::Split(args) => commands::split::run(args, &cli.file),
        // REQ-0101: project-wide quality audit beyond the validator.
        Command::Lint(args) => commands::lint::run(args, &cli.file),
        // REQ-0104: session-start brief.
        Command::Brief(args) => commands::brief::run(args, &cli.file),
        // REQ-0105: one-shot project bootstrap.
        Command::Setup(args) => commands::setup::run(args),
        // REQ-0114: local CI-equivalent gate suite.
        Command::Precheck(args) => commands::precheck::run(args, &cli.file),
        // REQ-0111: project purpose statement.
        Command::Purpose(args) => commands::purpose::run(args, &cli.file),
        // REQ-0109: retroactive backfill helper.
        Command::Adopt(args) => commands::adopt::run(args, &cli.file),
        // REQ-0134: functional-safety surface.
        Command::Hazard(cmd) => commands::safety::run_hazard(cmd, &cli.file),
        Command::Sf(cmd) => commands::safety::run_sf(cmd, &cli.file),
        Command::Sreq(cmd) => commands::safety::run_sreq(cmd, &cli.file),
        Command::Trace(args) => commands::safety::run_trace(args, &cli.file),
    }
}
