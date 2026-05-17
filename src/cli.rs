// Implements REQ-0001 (single managed CLI binary): one source of truth for
// every subcommand the tool exposes.
use clap::{Args, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

/// req — managed requirements CLI for LLM agents and humans.
///
/// Requirements live in a binary .req file. Agents cannot read or edit the
/// file directly; every change is mediated by this tool, which enforces
/// requirements best practice (atomic, testable, unambiguous statements).
#[derive(Parser, Debug)]
#[command(
    name = "req",
    version,
    about,
    long_about,
    propagate_version = true,
    disable_help_subcommand = true,
)]
pub struct Cli {
    /// Path to the .req project file. Defaults to ./project.req or $REQ_FILE.
    /// Use `--file PATH` (no short; `-f` is reserved for per-subcommand use such
    /// as `req export -f markdown`).
    #[arg(long = "file", global = true, env = "REQ_FILE")]
    pub file: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Create a new .req project file.
    Init(InitArgs),
    /// Add a new requirement (interactive unless flags supplied).
    Add(AddArgs),
    /// List requirements with optional filters.
    List(ListArgs),
    /// Show a single requirement in full.
    Show(ShowArgs),
    /// Update fields of an existing requirement.
    Update(UpdateArgs),
    /// Delete a requirement (or mark obsolete).
    Delete(DeleteArgs),
    /// Create parent/child or trace links between requirements.
    Link(LinkArgs),
    /// Validate every requirement against best-practice rules.
    Validate,
    /// Export the project to another format.
    Export(ExportArgs),
    /// Launch the interactive terminal browser/editor.
    Tui,
    /// Run a local web server for humans to browse/edit.
    Serve(ServeArgs),
    /// Speak MCP (JSON-RPC over stdio) so an LLM agent can manage requirements.
    Mcp,
    /// Show structured help. Use `req help <section>` to drill in.
    Help(HelpArgs),
    /// Recompute the integrity hash after an intentional direct edit.
    Repair(RepairArgs),
    /// Install git hooks (pre-commit validate, merge driver registration).
    Hooks(HooksArgs),
    /// Resolve requirement-ID collisions after merging from another branch.
    Renumber(RenumberArgs),
    /// Cross-reference REQ-IDs against the source tree; report orphans and ghosts.
    Coverage(CoverageArgs),
    /// Walk the git history of the .req file and report commit/signer per change.
    Audit(AuditArgs),
}

#[derive(Args, Debug)]
pub struct HooksArgs {
    /// `install` (default) or `uninstall`.
    #[arg(default_value = "install")]
    pub action: String,
    /// Path to the repository root. Defaults to the current working directory.
    #[arg(long)]
    pub repo: Option<PathBuf>,
    /// Overwrite an existing pre-commit hook.
    #[arg(long)]
    pub force: bool,
}

#[derive(Args, Debug)]
pub struct RenumberArgs {
    /// Git ref to compare against (typically `origin/main`).
    #[arg(long)]
    pub base: String,
    /// Show what would change without writing.
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Args, Debug)]
pub struct CoverageArgs {
    /// Root of the source tree to scan.
    #[arg(long, default_value = ".")]
    pub path: PathBuf,
    /// File extensions to scan (repeatable). Default: rs,py,js,ts,go,java,md,toml.
    #[arg(long = "ext")]
    pub extensions: Vec<String>,
    /// Flip the report: list source files that contain NO REQ-NNNN markers
    /// (i.e. code with no traceability link to any requirement).
    #[arg(long)]
    pub unlinked_files: bool,
    /// JSON output.
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct AuditArgs {
    /// Limit to N most recent commits.
    #[arg(short = 'n', long, default_value_t = 50)]
    pub limit: usize,
    /// JSON output.
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct RepairArgs {
    /// Required acknowledgement that you reviewed the direct edits.
    #[arg(long)]
    pub confirm_direct_edit: bool,
}

#[derive(Args, Debug)]
pub struct InitArgs {
    /// Project name.
    #[arg(short, long)]
    pub name: String,
    /// Output path for the .req file.
    #[arg(short, long, default_value = "project.req")]
    pub output: PathBuf,
    /// Overwrite if the file exists.
    #[arg(long)]
    pub force: bool,
}

#[derive(Args, Debug)]
pub struct AddArgs {
    /// One-line title (imperative, e.g. "User authenticates with email").
    #[arg(short, long)]
    pub title: Option<String>,
    /// Full normative statement. Should contain a modal verb (shall/must/should).
    #[arg(short, long)]
    pub statement: Option<String>,
    /// Rationale — why this requirement exists.
    #[arg(short, long)]
    pub rationale: Option<String>,
    /// Acceptance criteria. Repeat the flag for multiple.
    #[arg(short = 'a', long = "accept")]
    pub acceptance: Vec<String>,
    /// Requirement kind.
    #[arg(short = 'k', long, value_enum)]
    pub kind: Option<KindArg>,
    /// Priority.
    #[arg(short, long, value_enum)]
    pub priority: Option<PriorityArg>,
    /// Tags.
    #[arg(long)]
    pub tag: Vec<String>,
    /// Parent requirement ID (for hierarchy).
    #[arg(long)]
    pub parent: Option<String>,
    /// Force interactive mode even if flags are present.
    #[arg(short, long)]
    pub interactive: bool,
}

#[derive(Args, Debug)]
pub struct ListArgs {
    /// Filter by status.
    #[arg(long, value_enum)]
    pub status: Option<StatusArg>,
    /// Filter by kind.
    #[arg(long, value_enum)]
    pub kind: Option<KindArg>,
    /// Filter by priority.
    #[arg(long, value_enum)]
    pub priority: Option<PriorityArg>,
    /// Filter by tag (repeatable, AND semantics).
    #[arg(long)]
    pub tag: Vec<String>,
    /// Full-text search across title and statement.
    #[arg(short, long)]
    pub query: Option<String>,
    /// Render as JSON instead of a table.
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct ShowArgs {
    /// Requirement ID, e.g. REQ-0007.
    pub id: String,
    /// JSON output.
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct UpdateArgs {
    pub id: String,
    #[arg(short, long)]
    pub title: Option<String>,
    #[arg(short, long)]
    pub statement: Option<String>,
    #[arg(short, long)]
    pub rationale: Option<String>,
    /// Replace acceptance criteria wholesale (repeatable).
    #[arg(short = 'a', long = "accept")]
    pub acceptance: Option<Vec<String>>,
    #[arg(short = 'k', long, value_enum)]
    pub kind: Option<KindArg>,
    #[arg(short, long, value_enum)]
    pub priority: Option<PriorityArg>,
    #[arg(long, value_enum)]
    pub status: Option<StatusArg>,
    /// Add a tag (repeatable).
    #[arg(long)]
    pub add_tag: Vec<String>,
    /// Remove a tag (repeatable).
    #[arg(long)]
    pub remove_tag: Vec<String>,
    /// Reason for change — recorded in history.
    #[arg(long)]
    pub reason: Option<String>,
}

#[derive(Args, Debug)]
pub struct DeleteArgs {
    pub id: String,
    /// Hard-delete. Default is to set status=Obsolete (recommended).
    #[arg(long)]
    pub hard: bool,
    #[arg(long)]
    pub reason: Option<String>,
}

#[derive(Args, Debug)]
pub struct LinkArgs {
    /// Source requirement.
    pub from: String,
    /// Target requirement.
    pub to: String,
    /// Link kind.
    #[arg(short, long, value_enum, default_value = "parent")]
    pub kind: LinkKindArg,
    /// Remove the link instead of adding it.
    #[arg(long)]
    pub remove: bool,
}

#[derive(Args, Debug)]
pub struct ExportArgs {
    /// Output format.
    #[arg(short, long, value_enum, default_value = "markdown")]
    pub format: ExportFormat,
    /// Output path. `-` for stdout.
    #[arg(short, long, default_value = "-")]
    pub output: String,
}

#[derive(Args, Debug)]
pub struct ServeArgs {
    /// Bind address.
    #[arg(long, default_value = "127.0.0.1")]
    pub host: String,
    #[arg(short, long, default_value_t = 7878)]
    pub port: u16,
    /// Read-only — disable mutation endpoints.
    #[arg(long)]
    pub read_only: bool,
}

#[derive(Args, Debug)]
pub struct HelpArgs {
    /// Section to display. Omit to list all sections.
    pub section: Option<String>,
    /// List available sections.
    #[arg(short, long)]
    pub list: bool,
    /// Install the named section into a markdown file (default: AGENTS.md).
    /// Idempotent — uses sentinel markers so re-running updates in place.
    #[arg(long)]
    pub install: bool,
    /// Target file for --install.
    #[arg(long, default_value = "AGENTS.md")]
    pub path: PathBuf,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum KindArg {
    Functional,
    NonFunctional,
    Constraint,
    Interface,
    Business,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum PriorityArg {
    Must,
    Should,
    Could,
    Wont,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum StatusArg {
    Draft,
    Proposed,
    Approved,
    Implemented,
    Verified,
    Obsolete,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum LinkKindArg {
    Parent,
    DependsOn,
    Conflicts,
    Refines,
    Verifies,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum ExportFormat {
    Markdown,
    Json,
    Csv,
    Html,
}
