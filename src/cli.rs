// Implements REQ-0001 (single managed CLI binary): one source of truth for
// every subcommand the tool exposes.
// REQ-0094: every `value_enum` arg uses `ignore_case = true` so `Implemented`,
// `implemented`, and `IMPLEMENTED` all fold to the canonical lowercase form.
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
    disable_version_flag = true
)]
pub struct Cli {
    /// Print the version and exit (also `req version` or `req --version`).
    /// Both `-v` and the conventional `-V` are accepted.
    #[arg(short = 'v', short_alias = 'V', long = "version",
          action = clap::ArgAction::Version)]
    pub version: (),

    /// Path to the .req project file. Defaults to ./project.req or $REQ_FILE.
    /// Use `--file PATH` (no short; `-f` is reserved for per-subcommand use such
    /// as `req export -f markdown`).
    #[arg(long = "file", global = true, env = "REQ_FILE")]
    pub file: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Command,
}

impl Command {
    /// Whether the user asked for JSON output on this invocation. Drives the
    /// stderr error envelope in main.
    pub fn is_json(&self) -> bool {
        match self {
            Command::Add(a) => a.json,
            Command::Update(a) => a.json,
            Command::Delete(a) => a.json,
            Command::Link(a) => a.json,
            Command::Validate(a) => a.json,
            Command::Status(a) => a.json,
            Command::Test(TestCmd::Record(a)) => a.json,
            Command::Test(TestCmd::Run(a)) => a.json,
            Command::Test(TestCmd::List(a)) => a.json,
            Command::Verify(a) => a.json,
            Command::Stale(a) => a.json,
            Command::Batch(a) => a.json,
            Command::Import(a) => a.json,
            Command::Migrate(a) => a.json,
            Command::List(a) => a.json,
            Command::Show(a) => a.json,
            Command::Version(a) => a.json,
            Command::Next(a) => a.json,
            Command::Review(a) => a.json,
            Command::Split(a) => a.json,
            Command::Lint(a) => a.json,
            Command::Brief(a) => a.json, // REQ-0101
            Command::Check(a) => a.json,
            Command::Doctor(a) => a.json,
            Command::Diff(a) => a.json,
            Command::Help(a) => a.json,
            Command::Hazard(HazardCmd::Add(a)) => a.json,
            Command::Hazard(HazardCmd::List(a)) => a.json,
            Command::Hazard(HazardCmd::Show(a)) => a.json,
            Command::Hazard(HazardCmd::Assess(a)) => a.json,
            Command::Hazard(HazardCmd::Update(a)) => a.json,
            Command::Sf(SfCmd::Add(a)) => a.json,
            Command::Sf(SfCmd::List(a)) => a.json,
            Command::Sf(SfCmd::Show(a)) => a.json,
            Command::Sf(SfCmd::Update(a)) => a.json,
            Command::Sf(SfCmd::Mitigate(a)) => a.json,
            Command::Sreq(SreqCmd::Add(a)) => a.json,
            Command::Sreq(SreqCmd::List(a)) => a.json,
            Command::Sreq(SreqCmd::Show(a)) => a.json,
            Command::Sreq(SreqCmd::Update(a)) => a.json,
            Command::Sreq(SreqCmd::Realize(a)) => a.json,
            Command::Sreq(SreqCmd::Verify(a)) => a.json,
            Command::Trace(a) => a.json,
            Command::Safety(SafetyCmd::Status(a)) => a.json,
            Command::Safety(SafetyCmd::Calibrate(a)) => a.json,
            Command::Validation(ValidationCmd::Plan(a)) => a.json,
            Command::Validation(ValidationCmd::Analysis(a)) => a.json,
            Command::Validation(ValidationCmd::Test(a)) => a.json,
            Command::Validation(ValidationCmd::Conclude(a)) => a.json,
            Command::Validation(ValidationCmd::Show(a)) => a.json,
            Command::Validation(ValidationCmd::Backfill(a)) => a.json,
            _ => false,
        }
    }
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
    /// Soft-retire a requirement to Obsolete (links preserved). Pass --hard
    /// to actually remove it. Aliased as `retire`, which matches the default
    /// semantics; the historical name is `delete`.
    #[command(alias = "retire")]
    Delete(DeleteArgs),
    /// Create parent/child or trace links between requirements.
    Link(LinkArgs),
    /// Validate every requirement against best-practice rules.
    Validate(ValidateArgs),
    /// Show project-level implementation status with counts and percentages.
    Status(StatusArgs),
    /// Print the binary version (human or JSON).
    Version(VersionArgs),
    /// Suggest a single next requirement to work on (dependency-aware).
    Next(NextArgs),
    /// Validate requirements changed since a git ref + coverage for changed files.
    Check(CheckArgs),
    /// Report per-clone setup health (hooks, merge driver, signing, gitattributes).
    Doctor(DoctorArgs),
    /// Summarize per-requirement changes between two git revisions of project.req.
    Diff(DiffArgs),
    /// Attach a test record (commit SHA + outcome + notes) to a requirement.
    #[command(subcommand)]
    Test(TestCmd),
    /// Record a composition or inspection evidence record, optionally
    /// promoting the requirement to Verified.
    Verify(VerifyArgs),
    /// Report staleness of every requirement's latest test record relative
    /// to the files it links to (content drift, not just commit drift).
    Stale(StaleArgs),
    /// Apply many mutations atomically from a JSON document.
    Batch(BatchArgs),
    /// Import requirements from markdown or JSON; routed through the validator.
    Import(ImportArgs),
    /// Migrate project.req from an older _format to the current one (backs up first).
    Migrate(MigrateArgs),
    /// Print the JSON Schema for structured CLI inputs (req add --from-json, req batch).
    Schema(SchemaArgs),
    /// Export the project to another format.
    Export(ExportArgs),
    /// Launch the interactive terminal browser/editor.
    Tui,
    /// Run a local web server for humans to browse/edit.
    Serve(ServeArgs),
    /// Speak MCP (JSON-RPC over stdio) so an LLM agent can manage requirements.
    Mcp(McpArgs),
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
    /// Single markdown PR-review report: validate, coverage, stale,
    /// audit, and changed-requirement diff scoped to a git rev range.
    Review(ReviewArgs),
    /// Interactive split of a compound requirement into atomic ones.
    Split(SplitArgs),
    /// REQ-0101: project-wide quality audit beyond the validator: marker
    /// coverage, rationale length, acceptance count, test-record presence.
    Lint(LintArgs),
    /// REQ-0104: session-start brief. Where are we right now?
    Brief(BriefArgs),
    /// REQ-0105: one-shot project bootstrap (init + hooks + AGENTS.md).
    Setup(SetupArgs),
    /// REQ-0114: run the local equivalent of the CI gate suite.
    Precheck(PrecheckArgs),
    /// REQ-0111: set or print the project's purpose statement.
    Purpose(PurposeArgs),
    /// REQ-0109: retroactive backfill — advance requirements through
    /// the lifecycle to a target status in one invocation.
    Adopt(AdoptArgs),
    /// REQ-0134: manage hazards (HAZ-NNNN) — the functional-safety
    /// entry point. Risk-assess via the IEC 61508 risk graph.
    #[command(subcommand)]
    Hazard(HazardCmd),
    /// REQ-0134: manage safety functions (SF-NNNN) that mitigate hazards.
    #[command(subcommand)]
    Sf(SfCmd),
    /// REQ-0134: manage safety requirements (SR-NNNN) that realize
    /// safety functions.
    #[command(subcommand)]
    Sreq(SreqCmd),
    /// REQ-0136: print the end-to-end safety case for a HAZ/SF/SR id —
    /// hazard → safety function → safety requirements → verification.
    Trace(TraceArgs),
    /// REQ-0138: human-only functional-safety governance — accept the
    /// liability disclaimer (which activates the safety features) and
    /// manage the risk-graph calibration.
    #[command(subcommand)]
    Safety(SafetyCmd),
    /// REQ-0139: the staged validation dossier (plan → analysis → testing
    /// → statement → verdict) that gates promotion to Verified. Works on a
    /// REQ-NNNN or SR-NNNN id.
    #[command(subcommand)]
    Validation(ValidationCmd),
}

/// REQ-0139: subcommands of `req validation`. Each takes a REQ-/SR- id and
/// advances the dossier one stage; the stages must be filled in order.
#[derive(Subcommand, Debug)]
pub enum ValidationCmd {
    /// Stage 1 — open the dossier and record HOW the obligation will be
    /// validated (the analysis + testing approach).
    Plan(ValidationPlanArgs),
    /// Stage 2 — record validation by analysis (code review): findings and
    /// a pass/fail outcome.
    Analysis(ValidationActivityArgs),
    /// Stage 3 — record validation by testing: findings and a pass/fail
    /// outcome, citing recorded test evidence where it exists.
    Test(ValidationActivityArgs),
    /// Stage 4 — record the validation statement, derive the verdict, and
    /// optionally promote to Verified.
    Conclude(ValidationConcludeArgs),
    /// Show the dossier for a requirement or safety requirement.
    Show(ValidationShowArgs),
    /// Grandfather already-Verified items that pre-date the dossier by
    /// recording an audited exemption so a strict `req validate` passes.
    Backfill(ValidationBackfillArgs),
}

#[derive(Args, Debug)]
pub struct ValidationPlanArgs {
    /// REQ-NNNN or SR-NNNN id.
    pub id: String,
    /// How this obligation will be validated — the analysis (review) and
    /// testing approach.
    #[arg(long)]
    pub plan: String,
    /// Re-open a concluded dossier (clears the prior verdict/statement so
    /// the item can be re-validated). Requires --reason.
    #[arg(long, requires = "reason")]
    pub reopen: bool,
    /// Justification, required with --reopen. Recorded in history.
    #[arg(long)]
    pub reason: Option<String>,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct ValidationActivityArgs {
    /// REQ-NNNN or SR-NNNN id.
    pub id: String,
    /// Findings — what was reviewed/run and what was observed.
    #[arg(long)]
    pub findings: String,
    /// This dimension's outcome.
    #[arg(long, value_enum, ignore_case = true)]
    pub result: TestResultArg,
    /// Supporting references — files/commits reviewed (analysis) or test
    /// names / records cited (testing). Repeatable.
    #[arg(long = "ref")]
    pub references: Vec<String>,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct ValidationConcludeArgs {
    /// REQ-NNNN or SR-NNNN id.
    pub id: String,
    /// The validation statement supporting the verdict.
    #[arg(long)]
    pub statement: String,
    /// Promote to Verified after concluding (only when the verdict is
    /// Pass). Promotion is gated exactly like `req verify --promote`.
    #[arg(long)]
    pub promote: bool,
    /// Override the promotion preconditions (status ladder / SIL-rigour
    /// gate). Requires --reason; recorded as an audited exception.
    #[arg(long, requires = "reason")]
    pub force: bool,
    /// Justification, required with --force.
    #[arg(long)]
    pub reason: Option<String>,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct ValidationShowArgs {
    /// REQ-NNNN or SR-NNNN id.
    pub id: String,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct ValidationBackfillArgs {
    /// A single REQ-/SR- id to back-fill. Omit with --all to do every
    /// Verified item lacking a passing dossier.
    pub id: Option<String>,
    /// Back-fill every Verified requirement and safety requirement that
    /// has no passing dossier.
    #[arg(long)]
    pub all: bool,
    /// Justification recorded on each back-filled exemption.
    #[arg(long)]
    pub reason: String,
    #[arg(long)]
    pub json: bool,
}

#[derive(Subcommand, Debug)]
pub enum SafetyCmd {
    /// Accept the safety disclaimer, writing the acceptance file that
    /// activates hazards / safety functions / safety requirements.
    Accept(SafetyAcceptArgs),
    /// Show whether safety features are enabled and the calibration in use.
    Status(SafetyStatusArgs),
    /// View or edit the per-project risk-graph calibration (SIL bands).
    Calibrate(SafetyCalibrateArgs),
}

#[derive(Args, Debug)]
pub struct SafetyAcceptArgs {
    /// Who is accepting — recorded in the committed acceptance file.
    #[arg(long)]
    pub name: Option<String>,
}

#[derive(Args, Debug)]
pub struct SafetyStatusArgs {
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct SafetyCalibrateArgs {
    /// Set the human label for the calibration in use.
    #[arg(long)]
    pub label: Option<String>,
    /// Override one leaf, repeatable: --set "C_D/F_B/P_B=W3:4,W2:3,W1:2".
    /// Leaves not set keep the IEC 61508-5 Annex D default.
    #[arg(long = "set")]
    pub set: Vec<String>,
    /// Clear all overrides and the label, reverting to the Annex D default.
    #[arg(long)]
    pub reset: bool,
    /// Print the current calibration without changing it.
    #[arg(long)]
    pub show: bool,
    #[arg(long)]
    pub json: bool,
}

#[derive(Subcommand, Debug)]
pub enum HazardCmd {
    /// Log a hazard. Risk parameters are optional at this stage — a
    /// hazard starts `Identified` and is risk-assessed later.
    Add(HazardAddArgs),
    /// List hazards with optional SIL / status filters.
    List(HazardListArgs),
    /// Show one hazard in full, including its derived SIL.
    Show(HazardShowArgs),
    /// Set the C/F/P/W risk parameters; derives the required SIL and
    /// advances the hazard to `Assessed`.
    Assess(HazardAssessArgs),
    /// Update title/description/context/harm/status with a reason.
    Update(HazardUpdateArgs),
}

#[derive(Args, Debug)]
pub struct HazardAddArgs {
    #[arg(short, long)]
    pub title: String,
    #[arg(short, long, default_value = "")]
    pub description: String,
    /// The operational situation / mode in which the hazard arises.
    #[arg(long = "context", default_value = "")]
    pub context: String,
    /// Free-text narrative of the potential harm, in your own words —
    /// e.g. "an operator's hand could be severed".
    #[arg(long)]
    pub harm: String,
    /// Optional risk parameters. Supply all four to assess on creation;
    /// omit them to log the hazard as `Identified` and assess later.
    #[arg(short = 'C', long, value_enum, ignore_case = true)]
    pub consequence: Option<ConsequenceArg>,
    #[arg(short = 'F', long, value_enum, ignore_case = true)]
    pub frequency: Option<FrequencyArg>,
    #[arg(short = 'P', long, value_enum, ignore_case = true)]
    pub avoidance: Option<AvoidanceArg>,
    #[arg(short = 'W', long, value_enum, ignore_case = true)]
    pub probability: Option<ProbabilityArg>,
    #[arg(long)]
    pub tag: Vec<String>,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct HazardListArgs {
    /// Filter by derived SIL (e.g. SIL3). Hazards not yet assessed are
    /// excluded by any SIL filter.
    #[arg(long)]
    pub sil: Option<String>,
    /// Filter by status.
    #[arg(long, value_enum, ignore_case = true)]
    pub status: Option<HazardStatusArg>,
    /// Only hazards with no mitigating safety function.
    #[arg(long)]
    pub unmitigated: bool,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct HazardShowArgs {
    pub id: String,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct HazardAssessArgs {
    pub id: String,
    #[arg(short = 'C', long, value_enum, ignore_case = true)]
    pub consequence: ConsequenceArg,
    #[arg(short = 'F', long, value_enum, ignore_case = true)]
    pub frequency: FrequencyArg,
    #[arg(short = 'P', long, value_enum, ignore_case = true)]
    pub avoidance: AvoidanceArg,
    #[arg(short = 'W', long, value_enum, ignore_case = true)]
    pub probability: ProbabilityArg,
    #[arg(long)]
    pub reason: Option<String>,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct HazardUpdateArgs {
    pub id: String,
    #[arg(short, long)]
    pub title: Option<String>,
    #[arg(short, long)]
    pub description: Option<String>,
    #[arg(long = "context")]
    pub context: Option<String>,
    #[arg(long)]
    pub harm: Option<String>,
    #[arg(long, value_enum, ignore_case = true)]
    pub status: Option<HazardStatusArg>,
    #[arg(long)]
    pub add_tag: Vec<String>,
    #[arg(long)]
    pub remove_tag: Vec<String>,
    #[arg(long)]
    pub reason: Option<String>,
    /// Skip lifecycle guards (e.g. jump straight to Verified).
    #[arg(long)]
    pub force: bool,
    #[arg(long)]
    pub json: bool,
}

#[derive(Subcommand, Debug)]
pub enum SfCmd {
    /// Define a safety function.
    Add(SfAddArgs),
    /// List safety functions with their allocated SIL.
    List(SfListArgs),
    /// Show one safety function in full.
    Show(SfShowArgs),
    /// Update fields with a reason.
    Update(SfUpdateArgs),
    /// Record that this safety function mitigates a hazard (SF → HAZ).
    Mitigate(SfMitigateArgs),
}

#[derive(Args, Debug)]
pub struct SfAddArgs {
    #[arg(short, long)]
    pub title: String,
    #[arg(short, long, default_value = "")]
    pub description: String,
    /// The safe state this function achieves or maintains.
    #[arg(long = "safe-state", default_value = "")]
    pub safe_state: String,
    /// Hazard(s) this function mitigates (repeatable). Records the
    /// mitigates links immediately.
    #[arg(long = "mitigates")]
    pub mitigates: Vec<String>,
    #[arg(long)]
    pub tag: Vec<String>,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct SfListArgs {
    #[arg(long)]
    pub sil: Option<String>,
    #[arg(long, value_enum, ignore_case = true)]
    pub status: Option<SafetyFunctionStatusArg>,
    /// Only safety functions with no realizing safety requirement.
    #[arg(long)]
    pub unrealized: bool,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct SfShowArgs {
    pub id: String,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct SfUpdateArgs {
    pub id: String,
    #[arg(short, long)]
    pub title: Option<String>,
    #[arg(short, long)]
    pub description: Option<String>,
    #[arg(long = "safe-state")]
    pub safe_state: Option<String>,
    #[arg(long, value_enum, ignore_case = true)]
    pub status: Option<SafetyFunctionStatusArg>,
    #[arg(long)]
    pub add_tag: Vec<String>,
    #[arg(long)]
    pub remove_tag: Vec<String>,
    #[arg(long)]
    pub reason: Option<String>,
    #[arg(long)]
    pub force: bool,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct SfMitigateArgs {
    /// The safety function (SF-NNNN).
    pub sf: String,
    /// The hazard it mitigates (HAZ-NNNN).
    pub hazard: String,
    /// Remove the mitigates link instead of adding it.
    #[arg(long)]
    pub remove: bool,
    #[arg(long)]
    pub json: bool,
}

#[derive(Subcommand, Debug)]
pub enum SreqCmd {
    /// Add a safety requirement.
    Add(SreqAddArgs),
    /// List safety requirements with their inherited SIL.
    List(SreqListArgs),
    /// Show one safety requirement in full.
    Show(SreqShowArgs),
    /// Update fields / lifecycle status with a reason.
    Update(SreqUpdateArgs),
    /// Record that this safety requirement realizes a safety function
    /// (SR → SF).
    Realize(SreqRealizeArgs),
    /// Attach verification evidence, optionally promoting to Verified.
    /// The evidence rigour must meet the requirement's inherited SIL.
    Verify(SreqVerifyArgs),
}

#[derive(Args, Debug)]
pub struct SreqAddArgs {
    #[arg(short, long)]
    pub title: String,
    #[arg(short, long)]
    pub statement: String,
    #[arg(short, long)]
    pub rationale: String,
    #[arg(short = 'a', long = "accept")]
    pub acceptance: Vec<String>,
    /// Priority. Safety requirements default to `must`.
    #[arg(short, long, value_enum, ignore_case = true, default_value = "must")]
    pub priority: PriorityArg,
    /// Safety function(s) this requirement realizes (repeatable).
    #[arg(long = "realizes")]
    pub realizes: Vec<String>,
    #[arg(long)]
    pub tag: Vec<String>,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct SreqListArgs {
    #[arg(long)]
    pub sil: Option<String>,
    #[arg(long, value_enum, ignore_case = true)]
    pub status: Option<StatusArg>,
    /// Only safety requirements not yet Verified.
    #[arg(long)]
    pub unverified: bool,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct SreqShowArgs {
    pub id: String,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct SreqUpdateArgs {
    pub id: String,
    #[arg(short, long)]
    pub title: Option<String>,
    #[arg(short, long)]
    pub statement: Option<String>,
    #[arg(short, long)]
    pub rationale: Option<String>,
    #[arg(short = 'a', long = "accept")]
    pub acceptance: Option<Vec<String>>,
    #[arg(long = "add-acceptance")]
    pub add_acceptance: Vec<String>,
    #[arg(short, long, value_enum, ignore_case = true)]
    pub priority: Option<PriorityArg>,
    #[arg(long, value_enum, ignore_case = true)]
    pub status: Option<StatusArg>,
    #[arg(long)]
    pub add_tag: Vec<String>,
    #[arg(long)]
    pub remove_tag: Vec<String>,
    #[arg(long)]
    pub reason: Option<String>,
    #[arg(long)]
    pub force: bool,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct SreqRealizeArgs {
    /// The safety requirement (SR-NNNN).
    pub sreq: String,
    /// The safety function it realizes (SF-NNNN).
    pub sf: String,
    #[arg(long)]
    pub remove: bool,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct SreqVerifyArgs {
    pub id: String,
    /// Evidence kind: automated, composition, or inspection. The
    /// SIL-gate rejects inspection-only evidence for SIL 3/4.
    #[arg(long = "by", value_enum, ignore_case = true)]
    pub by: EvidenceArg,
    #[arg(long, default_value = "")]
    pub notes: String,
    #[arg(long = "cites")]
    pub cites: Vec<String>,
    /// Promote to Verified after recording. Promotion is gated: only
    /// from Implemented (like ordinary `req verify`), and a SIL 3/4
    /// requirement cannot be promoted on inspection-only evidence.
    #[arg(long)]
    pub promote: bool,
    /// Override the promotion guards (the status ladder and the
    /// SIL-rigour gate). Requires --reason; the override is recorded as
    /// a structured, audited exception on the evidence record.
    #[arg(long, requires = "reason")]
    pub force: bool,
    /// Justification, required with --force. Recorded on the evidence.
    #[arg(long)]
    pub reason: Option<String>,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct TraceArgs {
    /// A HAZ-NNNN, SF-NNNN, or SR-NNNN id. Tracing from a hazard shows
    /// the whole case; from an SF or SR shows the slice rooted there.
    pub id: String,
    #[arg(long)]
    pub json: bool,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum ConsequenceArg {
    #[value(name = "C_A")]
    Ca,
    #[value(name = "C_B")]
    Cb,
    #[value(name = "C_C")]
    Cc,
    #[value(name = "C_D")]
    Cd,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum FrequencyArg {
    #[value(name = "F_A")]
    Fa,
    #[value(name = "F_B")]
    Fb,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum AvoidanceArg {
    #[value(name = "P_A")]
    Pa,
    #[value(name = "P_B")]
    Pb,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum ProbabilityArg {
    W1,
    W2,
    W3,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum HazardStatusArg {
    Identified,
    Assessed,
    Mitigated,
    Verified,
    Obsolete,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum SafetyFunctionStatusArg {
    Proposed,
    Allocated,
    Implemented,
    Verified,
    Obsolete,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum EvidenceArg {
    Automated,
    Composition,
    Inspection,
}

#[derive(Args, Debug)]
pub struct AdoptArgs {
    /// Requirements to adopt. Provide IDs (REQ-0001 etc.) or use
    /// --all-drafts to scope to every requirement currently at Draft.
    pub ids: Vec<String>,
    /// Adopt every requirement currently at Draft.
    #[arg(long)]
    pub all_drafts: bool,
    /// Target lifecycle position. Default: verified.
    #[arg(long, value_enum, ignore_case = true, default_value = "verified")]
    pub to: AdoptTarget,
    /// Reason recorded on every history entry written by adopt.
    /// Defaults to "retroactive adoption from existing source state".
    #[arg(short, long)]
    pub reason: Option<String>,
    /// Print what would change without writing.
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(clap::ValueEnum, Clone, Debug)]
pub enum AdoptTarget {
    Proposed,
    Approved,
    Implemented,
    Verified,
}

#[derive(Args, Debug)]
pub struct PurposeArgs {
    /// New purpose statement. Omit to print the current value. Pass an
    /// empty string to clear. Max 500 characters.
    pub text: Option<String>,
    /// Recorded reason for the change (required when setting/changing).
    #[arg(short, long)]
    pub reason: Option<String>,
}

#[derive(Args, Debug)]
pub struct SetupArgs {
    /// Project name (used for `req init` when no project file exists).
    /// Defaults to the current directory name.
    #[arg(short, long)]
    pub name: Option<String>,
    /// Install the strict pre-commit hook (hunk-level marker check)
    /// instead of the default file-level one.
    #[arg(long)]
    pub strict: bool,
    /// Skip the pre-commit / post-commit hook install step.
    #[arg(long)]
    pub no_hooks: bool,
    /// Skip writing the AGENTS.md managed block.
    #[arg(long)]
    pub no_agents: bool,
    /// Overwrite an existing non-managed pre-commit hook.
    #[arg(long)]
    pub force: bool,
    /// REQ-0117: repo path to operate on. Defaults to the current
    /// working directory. Useful when running inside a worktree
    /// where the main repo's hooks/ live in a different tree.
    #[arg(long)]
    pub repo: Option<PathBuf>,
}

#[derive(Args, Debug)]
pub struct PrecheckArgs {
    /// Skip one or more steps (repeatable). Names: fmt, clippy, test,
    /// validate, coverage, review. Use this only for tight inner loops —
    /// the default is to run everything CI runs.
    #[arg(long = "skip", value_name = "STEP")]
    pub skip: Vec<String>,
    /// Continue running remaining steps after a failure. Default: stop
    /// on the first non-zero step so the failure is easy to read.
    #[arg(long)]
    pub keep_going: bool,
}

#[derive(Args, Debug)]
pub struct BriefArgs {
    /// Expand the brief: by-status counts, gate mode, recent spec activity.
    #[arg(long)]
    pub full: bool,
    /// Machine-readable JSON.
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct LintArgs {
    /// Root of the source tree to scan for `// REQ-NNNN:` markers.
    #[arg(long, default_value = ".")]
    pub path: PathBuf,
    /// Emit the audit as JSON instead of markdown.
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct ReviewArgs {
    /// Base git rev (default: origin/main, then main). Compared as
    /// `<base>..HEAD`. Used for both the changed-requirement diff and
    /// the changed-files coverage scope.
    #[arg(long, default_value = "origin/main")]
    pub base: String,
    /// Directory to scan for `// REQ-NNNN` markers when computing
    /// coverage. Defaults to the repo root.
    #[arg(long, default_value = ".")]
    pub path: PathBuf,
    /// File extensions to treat as source for the markerless check
    /// (repeat for multiple). Without this flag the gate uses an
    /// extensive default list that covers most common languages.
    /// Pass `--ext` once with no value to disable the extension
    /// filter entirely (every changed text file becomes source).
    #[arg(long = "ext")]
    pub ext: Vec<String>,
    /// Glob pattern (matched on the relative path with `/` separators)
    /// to exclude from the markerless check. Repeat for multiple.
    /// Defaults already cover tests/, build.rs, generated/, and the
    /// `.req` project file itself.
    #[arg(long = "ignore")]
    pub ignore: Vec<String>,
    /// Scope the report to STAGED changes (`git diff --cached`) rather
    /// than `<base>..HEAD`. Used by the pre-commit hook so an agent
    /// adding new code without a REQ marker is told at commit time,
    /// not after pushing. Implies `--base HEAD`.
    #[arg(long)]
    pub staged: bool,
    /// REQ-0086: --summary mode used by the post-commit hook.
    /// Print a one-line summary instead of the full report. Used by the
    /// pre-commit hook to confirm a passing gate with a calm reminder
    /// rather than silence. Format: `req: N source file(s) staged ·
    /// cites REQ-A, REQ-B · reminder: ...`. Returns no output (silent
    /// pass) when no source files are staged.
    #[arg(long)]
    pub summary: bool,
    /// Require a `// REQ-NNNN:` marker within N lines of each changed
    /// hunk, not merely somewhere in the file. Default (0) means
    /// file-level matching — any marker anywhere in a changed file
    /// satisfies the gate. Use a positive value (e.g. 50) for strict
    /// hunk-level enforcement on real PRs.
    #[arg(long = "marker-near-hunks", default_value_t = 0)]
    pub marker_near_hunks: u32,
    /// Exit non-zero when the report finds anything blocking: validate
    /// errors, coverage ghosts, source files changed in this range
    /// that carry zero REQ markers, OR — critically — a missing/
    /// invalid base ref (no silent fail-open on a CI YAML typo).
    /// Use in CI to gate PRs on spec hygiene.
    #[arg(long)]
    pub gate: bool,
    /// REQ-0126: when used with --gate, also fail if any Verified
    /// requirement carries a failing latest test record. The defect
    /// log lives next to the spec; this lets CI block merges that
    /// would ship known-broken behaviour.
    #[arg(long, requires = "gate")]
    pub no_defects: bool,
    /// REQ-0131: scope validator findings to requirements ADDED or
    /// CHANGED in this range, suppressing findings on requirements the
    /// commit did not touch. `--staged` implies this. The per-commit
    /// gate stays sharp instead of reprinting the whole project's
    /// backlog every commit; full-project error enforcement still lives
    /// in the dedicated `req validate` (staged-.req hook) and CI.
    #[arg(long, conflicts_with = "all")]
    pub new: bool,
    /// REQ-0131: force the full-project validator sweep even under
    /// `--staged`. This is the deliberate hygiene view — the name for
    /// the default, advisory `req review` behaviour, made explicit so
    /// it composes in scripts.
    #[arg(long)]
    pub all: bool,
    /// Emit the report as JSON instead of markdown.
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct SplitArgs {
    /// The compound requirement to split.
    pub id: String,
    /// New statement for one part (repeat for N parts). When supplied
    /// the command runs non-interactively. Each part inherits the
    /// original's kind, priority, and tags.
    #[arg(short = 's', long = "into")]
    pub into: Vec<String>,
    /// Reason for splitting — recorded on the original's history when
    /// it is soft-retired to Obsolete.
    #[arg(long)]
    pub reason: Option<String>,
    /// Don't soft-retire the original; keep it active and just create
    /// the new parts. Use when the split is *additive* rather than a
    /// replacement.
    #[arg(long)]
    pub keep_original: bool,
    /// JSON output.
    #[arg(long)]
    pub json: bool,
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
    /// Also write/update .claude/settings.json with a req-aware permissions
    /// allowlist and a Stop hook that runs req validate.
    #[arg(long)]
    pub claude_code: bool,
    /// Install the STRICT pre-commit hook. The strict body invokes
    /// `req review --staged --gate --marker-near-hunks 50`, so edits
    /// inside an already-marked file still need a marker near the
    /// changed hunk. Default (no flag) writes the file-level hook
    /// that catches markerless new files but lets in-file edits
    /// through. Re-run with or without the flag to swap modes.
    #[arg(long)]
    pub strict: bool,
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
    #[arg(long, conflicts_with_all = ["by_file", "by_req", "remap"])]
    pub unlinked_files: bool,
    /// Per-file report: for every file with at least one marker, list the
    /// REQ IDs it references. Closes the bidirectional view.
    #[arg(long, conflicts_with_all = ["unlinked_files", "by_req", "remap"])]
    pub by_file: bool,
    /// REQ-0127: inverse of --by-file. For every REQ-NNNN with at least
    /// one marker in source, list the files referencing it.
    #[arg(long, conflicts_with_all = ["unlinked_files", "by_file", "remap"])]
    pub by_req: bool,
    /// Rewrite REQ-NNNN markers in source files. Pass repeatedly:
    ///   --remap REQ-OLD=REQ-NEW --remap REQ-AAA=REQ-BBB
    /// Dry-run by default; pass --apply to write.
    #[arg(long, value_name = "OLD=NEW")]
    pub remap: Vec<String>,
    /// Actually rewrite files when --remap is used (otherwise dry-run).
    #[arg(long)]
    pub apply: bool,
    /// Exit non-zero if orphans, ghosts, or obsolete-in-code findings
    /// exist (default mode only). Makes coverage a pre-commit / CI gate.
    #[arg(long)]
    pub strict: bool,
    /// In strict mode, treat the listed REQ-IDs as expected orphans
    /// (no code site required). Use for verification-only or
    /// policy-only requirements. Repeatable.
    #[arg(long = "allow")]
    pub allow_orphans: Vec<String>,
    /// JSON output.
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct AuditArgs {
    /// Limit to N most recent commits.
    #[arg(short = 'n', long, default_value_t = 50)]
    pub limit: usize,
    /// Gate mode: exit non-zero if any commit in the range violates the
    /// configured signature policy. Combine with --require-signer and/or
    /// --require-good-signature.
    #[arg(long)]
    pub gate: bool,
    /// Require a "good" or "good-unknown" signature on every commit
    /// touching project.req in the range.
    #[arg(long)]
    pub require_good_signature: bool,
    /// Require the signer to be one of these identities (repeatable).
    /// Matched as a case-insensitive substring of the git %GS field.
    #[arg(long = "require-signer")]
    pub required_signers: Vec<String>,
    /// JSON output.
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct RepairArgs {
    /// Required acknowledgement that you reviewed the direct edits.
    #[arg(long)]
    pub confirm_direct_edit: bool,
    /// Re-sign the file even when validation errors remain. Use when a
    /// hand-edit broke both the hash AND introduced validation errors,
    /// and other commands refuse to read the file — without this flag
    /// you'd be stuck (repair refuses due to validation, every other
    /// command refuses due to the hash). Re-signing surfaces the
    /// validation errors via `req validate` instead of the integrity
    /// check, which is the working state you want.
    #[arg(long)]
    pub force: bool,
}

#[derive(Args, Debug)]
pub struct InitArgs {
    /// Project name.
    #[arg(short, long)]
    pub name: String,
    /// Output path for the .req file (or directory if --layout=directory).
    #[arg(short, long, default_value = "project.req")]
    pub output: PathBuf,
    /// Overwrite if the file exists.
    #[arg(long)]
    pub force: bool,
    /// Storage layout: `single` (default) keeps everything in one .req file;
    /// `directory` writes per-requirement files under output/requirements/
    /// plus an index file. Both preserve the integrity guarantee.
    #[arg(long, value_enum, ignore_case = true, default_value = "single")]
    pub layout: LayoutArg,
    /// REQ-0111: one-paragraph project purpose statement. Surfaced by
    /// `req brief` at session start. Max 500 characters.
    #[arg(long)]
    pub purpose: Option<String>,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum LayoutArg {
    Single,
    Directory,
}

#[derive(Args, Debug)]
pub struct AddArgs {
    /// One-line title (imperative, e.g. "User authenticates with email").
    /// Required in non-interactive mode; omit only with --interactive or --from-json.
    #[arg(short, long, required_unless_present_any = ["interactive", "from_json"])]
    pub title: Option<String>,
    /// Full normative statement. Should contain a modal verb (shall/must/should).
    /// Required in non-interactive mode; omit only with --interactive or --from-json.
    #[arg(short, long, required_unless_present_any = ["interactive", "from_json"])]
    pub statement: Option<String>,
    /// Rationale — why this requirement exists.
    /// Required in non-interactive mode; omit only with --interactive or --from-json.
    #[arg(short, long, required_unless_present_any = ["interactive", "from_json"])]
    pub rationale: Option<String>,
    /// Acceptance criteria. Repeat the flag for multiple.
    #[arg(short = 'a', long = "accept")]
    pub acceptance: Vec<String>,
    /// Requirement kind.
    #[arg(short = 'k', long, value_enum, ignore_case = true)]
    pub kind: Option<KindArg>,
    /// Priority.
    #[arg(short, long, value_enum, ignore_case = true)]
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
    /// Emit the created requirement as JSON on stdout; suppress human prose.
    #[arg(long)]
    pub json: bool,
    /// Read all fields from a JSON document (file path or `-` for stdin).
    /// Bypasses shell quoting for multi-line statements and rationale.
    #[arg(long = "from-json")]
    pub from_json: Option<String>,
}

#[derive(Args, Debug)]
pub struct ListArgs {
    /// Filter by status.
    #[arg(long, value_enum, ignore_case = true)]
    pub status: Option<StatusArg>,
    /// Include Obsolete requirements (hidden by default; --status obsolete
    /// always overrides this).
    #[arg(long)]
    pub include_obsolete: bool,
    /// Filter by kind.
    #[arg(long, value_enum, ignore_case = true)]
    pub kind: Option<KindArg>,
    /// Filter by priority.
    #[arg(long, value_enum, ignore_case = true)]
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
    /// Append an acceptance criterion (repeatable). Combines with --accept.
    #[arg(long = "add-acceptance")]
    pub add_acceptance: Vec<String>,
    /// Remove an acceptance criterion by 1-based index (repeatable).
    #[arg(long = "remove-acceptance")]
    pub remove_acceptance: Vec<usize>,
    #[arg(short = 'k', long, value_enum, ignore_case = true)]
    pub kind: Option<KindArg>,
    #[arg(short, long, value_enum, ignore_case = true)]
    pub priority: Option<PriorityArg>,
    #[arg(long, value_enum, ignore_case = true)]
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
    /// Skip status-machine guards (e.g. allow draft -> verified without
    /// passing through implemented). Use only when correcting a bad
    /// historical record.
    #[arg(long)]
    pub force: bool,
    /// Emit the updated requirement as JSON on stdout.
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct DeleteArgs {
    pub id: String,
    /// Hard-delete. Default is to set status=Obsolete (recommended).
    #[arg(long)]
    pub hard: bool,
    #[arg(long)]
    pub reason: Option<String>,
    /// Emit the deletion as JSON on stdout.
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct LinkArgs {
    /// Source requirement.
    pub from: String,
    /// Target requirement.
    pub to: String,
    /// Link kind.
    #[arg(short, long, value_enum, ignore_case = true, default_value = "parent")]
    pub kind: LinkKindArg,
    /// Remove the link instead of adding it.
    #[arg(long)]
    pub remove: bool,
    /// Emit the link result as JSON on stdout.
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct ExportArgs {
    /// Output format.
    #[arg(
        short,
        long,
        value_enum,
        ignore_case = true,
        default_value = "markdown"
    )]
    pub format: ExportFormat,
    /// Output path. `-` for stdout.
    #[arg(short, long, default_value = "-")]
    pub output: String,
}

#[derive(Args, Debug)]
pub struct VersionArgs {
    /// Emit a JSON object with name, version, mcp_protocol, file_format.
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct NextArgs {
    /// Restrict to one status (default: any non-Obsolete).
    #[arg(long, value_enum, ignore_case = true)]
    pub status: Option<StatusArg>,
    /// Restrict to one kind.
    #[arg(long, value_enum, ignore_case = true)]
    pub kind: Option<KindArg>,
    /// Restrict to one priority.
    #[arg(long, value_enum, ignore_case = true)]
    pub priority: Option<PriorityArg>,
    /// Restrict to a tag (repeatable, AND).
    #[arg(long)]
    pub tag: Vec<String>,
    /// Emit JSON instead of a one-line summary.
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct SchemaArgs {
    /// Which schema to emit.
    #[arg(value_enum, ignore_case = true, default_value = "add")]
    pub which: SchemaWhich,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum SchemaWhich {
    /// Schema for `req add --from-json`.
    Add,
    /// Schema for `req batch`.
    Batch,
    /// Schema for `req import --format json` (array form).
    Import,
    /// REQ-0128: schema for the `req test run --map` JSON file.
    TestMap,
}

#[derive(Args, Debug)]
pub struct MigrateArgs {
    /// JSON output describing the migration result.
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct BatchArgs {
    /// Path to the batch JSON document, or `-` for stdin.
    pub source: String,
    /// JSON output reporting the applied changes.
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct ImportArgs {
    /// Format of the source: markdown or json.
    #[arg(short, long, value_enum, ignore_case = true)]
    pub format: ImportFormat,
    /// Source path (`-` for stdin).
    pub source: String,
    /// Show what would be imported without writing.
    #[arg(long)]
    pub dry_run: bool,
    /// Reject the whole import if any item fails validation.
    #[arg(long)]
    pub strict: bool,
    /// JSON output.
    #[arg(long)]
    pub json: bool,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum ImportFormat {
    Markdown,
    Json,
}

#[derive(Args, Debug)]
pub struct DoctorArgs {
    /// JSON output for tooling / CI.
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct DiffArgs {
    /// Spec: BASE..HEAD git ref pair.
    pub spec: String,
    /// JSON output.
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct CheckArgs {
    /// Git ref to compare against (typically `origin/main`).
    pub base: String,
    /// JSON output.
    #[arg(long)]
    pub json: bool,
    /// Source-tree root for coverage scan on changed files.
    #[arg(long, default_value = ".")]
    pub path: PathBuf,
}

#[derive(Subcommand, Debug)]
pub enum TestCmd {
    /// Record a test run against a requirement; captures git HEAD SHA, outcome, notes.
    Record(TestRecordArgs),
    /// Run `cargo test` (or a custom command) and attach pass/fail records
    /// to each requirement whose test name follows the `req_NNNN_*` convention.
    Run(TestRunArgs),
    /// REQ-0129: list the test record history attached to one requirement.
    List(TestListArgs),
}

#[derive(Args, Debug)]
pub struct TestListArgs {
    /// Requirement to inspect.
    pub id: String,
    /// Machine-readable JSON instead of human-formatted lines.
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct StaleArgs {
    /// Source-tree root used to find files containing REQ-NNNN markers.
    #[arg(long, default_value = ".")]
    pub path: PathBuf,
    /// Only report requirements with at least one linked file changed
    /// since the latest record (the actually-stale ones).
    #[arg(long)]
    pub only_stale: bool,
    /// JSON output.
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct VerifyArgs {
    /// Requirement to verify.
    pub id: String,
    /// Evidence kind: composition or inspection. Use `req test record` for
    /// automated evidence (the default kind there).
    #[arg(long = "by", value_enum, ignore_case = true)]
    pub by: VerifyKindArg,
    /// Notes describing the verification. For composition this should name
    /// the cited tests or requirements; for inspection it should describe
    /// what was reviewed.
    #[arg(long)]
    pub notes: String,
    /// Cite a specific test name or REQ-ID (repeatable). Prepended to notes.
    #[arg(long = "cites")]
    pub cites: Vec<String>,
    /// Promote the requirement to Verified after recording. Only applies
    /// when the requirement is currently Implemented; pass --force to
    /// override (e.g. when correcting history).
    #[arg(long)]
    pub promote: bool,
    /// Skip the Implemented-status precondition on --promote.
    #[arg(long)]
    pub force: bool,
    /// REQ-0139: promote without a validation dossier, recording an
    /// audited exemption (ordinary requirements only). Requires --reason.
    #[arg(long = "no-dossier", requires = "reason")]
    pub no_dossier: bool,
    /// Justification, required with --no-dossier; recorded on the exemption.
    #[arg(long)]
    pub reason: Option<String>,
    /// JSON output.
    #[arg(long)]
    pub json: bool,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum VerifyKindArg {
    Composition,
    Inspection,
}

#[derive(Args, Debug)]
pub struct TestRunArgs {
    /// Custom test command. Defaults to `cargo test --release`.
    #[arg(
        long,
        default_value = "cargo test --release",
        conflicts_with = "from_file"
    )]
    pub cmd: String,
    /// Parse cargo-test-style output from this file instead of running a
    /// command. Useful for piping pre-captured logs into the recorder,
    /// or for tests of the recorder itself.
    #[arg(long = "from-file", conflicts_with = "cmd")]
    pub from_file: Option<PathBuf>,
    /// REQ-0128: ecosystems without the `req_NNNN_*` test-name
    /// convention (Node, Python) supply a JSON map of test name →
    /// REQ-ID(s). The recorder reads this in addition to (or instead
    /// of) the regex-based name match. Schema published by
    /// `req schema test-map`.
    #[arg(long = "map", value_name = "MAP_FILE")]
    pub map_file: Option<PathBuf>,
    /// Show what would be recorded without writing.
    #[arg(long)]
    pub dry_run: bool,
    /// After recording, auto-promote any requirement with a fresh passing
    /// record (any kind) against the current HEAD to status=Verified.
    #[arg(long)]
    pub promote: bool,
    /// Emit the full result map as JSON.
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct ValidateArgs {
    /// Emit findings as JSON; preserves the non-zero exit on errors.
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct StatusArgs {
    /// Scope the report to requirements carrying every listed tag
    /// (AND semantics). Useful for milestone-style rollups, e.g.
    /// `req status --tag auth` answers "what's left for auth".
    /// Repeat the flag for multiple tags.
    #[arg(long)]
    pub tag: Vec<String>,
    /// Emit the status counts and percentages as JSON.
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct TestRecordArgs {
    pub id: String,
    /// Test result: pass or fail.
    #[arg(long, value_enum, ignore_case = true)]
    pub result: TestResultArg,
    /// Free-text notes attached to the test record.
    #[arg(long, default_value = "")]
    pub notes: String,
    /// Emit the resulting requirement as JSON.
    #[arg(long)]
    pub json: bool,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum TestResultArg {
    Pass,
    Fail,
}

#[derive(Args, Debug)]
pub struct McpArgs {
    /// Write a .mcp.json bootstrap file (does NOT start the server).
    /// Pass --path to put it somewhere other than the repo root.
    #[arg(long)]
    pub init_config: bool,
    /// Target path for --init-config.
    #[arg(long, default_value = ".mcp.json")]
    pub config_path: PathBuf,
    /// Overwrite an existing config file.
    #[arg(long)]
    pub force: bool,
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
    /// Emit the section as JSON. For 'agents' this returns a structured
    /// triggers/commands/rules document.
    #[arg(long)]
    pub json: bool,
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
