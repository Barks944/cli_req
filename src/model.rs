// Implements REQ-0010 (sequential IDs via allocate_id) and the data shape
// behind REQ-0011 (append-only history).
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::cli::{
    AvoidanceArg, ConsequenceArg, EvidenceArg, FrequencyArg, HazardStatusArg, KindArg, LinkKindArg,
    PriorityArg, ProbabilityArg, SafetyFunctionStatusArg, StatusArg,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub name: String,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
    pub next_id: u32,
    pub requirements: BTreeMap<String, Requirement>,
    /// REQ-0134: functional-safety artifacts (IEC 61508). Hazards
    /// (HAZ-NNNN), the safety functions that mitigate them (SF-NNNN),
    /// and the safety requirements that realize those functions
    /// (SR-NNNN) live in their own maps so they never blur into the
    /// ordinary requirements space. All three are serialised only when
    /// non-empty, so a project that uses no safety features keeps a
    /// byte-identical file (and integrity hash) to one written before
    /// the feature existed.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub hazards: BTreeMap<String, Hazard>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub safety_functions: BTreeMap<String, SafetyFunction>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub safety_requirements: BTreeMap<String, SafetyRequirement>,
    /// Separate ID counters per artifact family. Each is omitted from
    /// the file while it still holds its default (1), so files that
    /// never touch safety features are unchanged.
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub next_haz_id: u32,
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub next_sf_id: u32,
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub next_sr_id: u32,
    /// REQ-0111: optional one-paragraph statement of what this project
    /// is FOR. Serialised as `_purpose` (reserved key under the
    /// integrity hash, introduced in req-v2). 500-char cap enforced at
    /// edit time.
    #[serde(default, rename = "_purpose", skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
    /// REQ-0110: per-project configuration. Serialised as `_config`
    /// (reserved key under the integrity hash, introduced in req-v2).
    /// Precedence: CLI flag overrides _config overrides built-in
    /// defaults.
    #[serde(default, rename = "_config", skip_serializing_if = "Option::is_none")]
    pub config: Option<ProjectConfig>,
    /// REQ-0140: forward-compatibility catch-all. Any top-level field
    /// written by a newer `req` that this binary does not model is captured
    /// here and re-emitted verbatim on save, so an older binary round-trips
    /// a newer file instead of silently dropping it. Flattened: unknown keys
    /// sit inline alongside the modelled ones.
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

/// REQ-0110: the schema for the in-file `_config` map. Each section
/// holds optional overrides; `None` means "use the binary's default".
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProjectConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coverage: Option<CoverageConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate: Option<GateConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lint: Option<LintConfig>,
    /// REQ-0138: functional-safety governance — the risk-graph
    /// calibration in use and the one-time liability acknowledgement.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub safety: Option<SafetyConfig>,
    /// REQ-0139: validation-dossier policy (which tags exempt an ordinary
    /// requirement from the mandatory dossier gate).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation: Option<ValidationConfig>,
}

/// REQ-0139: per-project validation-dossier policy.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ValidationConfig {
    /// Tags that exempt an ordinary requirement (never a safety
    /// requirement) from the mandatory validation-dossier gate. Defaults
    /// to `["validation-exempt"]` when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exempt_tags: Option<Vec<String>>,
}

/// REQ-0139: the default tag that exempts an ordinary requirement from the
/// validation-dossier gate when no project override is configured.
pub const DEFAULT_VALIDATION_EXEMPT_TAG: &str = "validation-exempt";

impl Project {
    /// REQ-0139: the tags that exempt an ordinary requirement from the
    /// validation gate, honouring the project override.
    pub fn validation_exempt_tags(&self) -> Vec<String> {
        self.config
            .as_ref()
            .and_then(|c| c.validation.as_ref())
            .and_then(|v| v.exempt_tags.clone())
            .unwrap_or_else(|| vec![DEFAULT_VALIDATION_EXEMPT_TAG.to_string()])
    }

    /// REQ-0139: whether an ordinary requirement is exempt from the
    /// validation gate by virtue of carrying a configured exempt tag.
    pub fn req_is_validation_exempt(&self, r: &Requirement) -> bool {
        let tags = self.validation_exempt_tags();
        r.tags.iter().any(|t| tags.iter().any(|e| e == t))
    }
}

/// REQ-0138: per-project functional-safety governance. Both fields are
/// set deliberately by a human via `req safety` (never an agent).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SafetyConfig {
    /// A human label for the risk-graph calibration in use, e.g.
    /// "IEC 61508-5 Annex D (default)" or "AcmeRail scheme rev 3".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub calibration_label: Option<String>,
    /// Per-leaf SIL overrides. Leaf key is "C_x/F_x/P_x" (e.g.
    /// "C_D/F_B/P_B"); leaves absent here fall back to the Annex D
    /// worked-example default, so this is "override only what differs".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub calibration: Option<BTreeMap<String, CalibrationRow>>,
}

/// REQ-0138: the three SIL outcomes for one (C,F,P) leaf, indexed by the
/// W parameter. This is the unit a calibration overrides.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct CalibrationRow {
    pub w1: Sil,
    pub w2: Sil,
    pub w3: Sil,
}

/// REQ-0138: a recorded, dated acknowledgement of the safety disclaimer.
/// Serialised to a sibling acceptance FILE in the repo (not into the
/// integrity-hashed spec), so it shows up in PR diffs and a reviewer can
/// see who turned the safety features on. `disclaimer_version` lets a
/// future wording change re-require sign-on.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisclaimerAcceptance {
    /// Human-readable notice carried in the file so anyone reading the
    /// repo sees what was accepted, not just that something was.
    #[serde(rename = "_notice", default, skip_serializing_if = "String::is_empty")]
    pub notice: String,
    pub accepted_by: String,
    pub at: DateTime<Utc>,
    pub tool_version: String,
    pub disclaimer_version: String,
}

/// REQ-0138: bump when the substance of the safety disclaimer changes so
/// existing projects are prompted to re-acknowledge.
pub const SAFETY_DISCLAIMER_VERSION: &str = "1";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CoverageConfig {
    /// Source-file extensions to scan for `// REQ-NNNN:` markers in
    /// addition to (or instead of) the built-in defaults. When set,
    /// the values REPLACE the defaults so adopters can scope tightly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extensions: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GateConfig {
    /// Strict-mode marker proximity in lines (default 50). Overrides
    /// `req review --gate`'s `--marker-near-hunks` flag.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub marker_near_hunks: Option<u32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LintConfig {
    /// Word-count threshold below which a rationale is flagged as too
    /// short by `req lint`. Defaults to the binary's built-in value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub short_rationale_words: Option<u32>,
    /// Tags that exempt a requirement from `req lint`'s no-test-record
    /// finding (REQ-0107). Defaults to `["inspection-only"]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inspection_only_tags: Option<Vec<String>>,
}

/// REQ-0111: maximum length of the `_purpose` string. Enforced at edit
/// time by `req init --purpose` and `req purpose`. Caps the field at
/// one paragraph so `req brief` can lead with it without scrolling.
pub const PURPOSE_MAX_CHARS: usize = 500;

impl Project {
    pub fn new(name: String) -> Self {
        let now = Utc::now();
        Self {
            name,
            created: now,
            updated: now,
            next_id: 1,
            requirements: BTreeMap::new(),
            hazards: BTreeMap::new(),
            safety_functions: BTreeMap::new(),
            safety_requirements: BTreeMap::new(),
            next_haz_id: 1,
            next_sf_id: 1,
            next_sr_id: 1,
            purpose: None,
            config: None,
            extra: BTreeMap::new(),
        }
    }

    pub fn allocate_id(&mut self) -> String {
        let id = format!("REQ-{:04}", self.next_id);
        self.next_id += 1;
        id
    }

    /// REQ-0134: allocate the next HAZ / SF / SR identifier. Each family
    /// has an independent counter so the three id spaces never collide.
    pub fn allocate_haz_id(&mut self) -> String {
        let id = format!("HAZ-{:04}", self.next_haz_id);
        self.next_haz_id += 1;
        id
    }

    pub fn allocate_sf_id(&mut self) -> String {
        let id = format!("SF-{:04}", self.next_sf_id);
        self.next_sf_id += 1;
        id
    }

    pub fn allocate_sr_id(&mut self) -> String {
        let id = format!("SR-{:04}", self.next_sr_id);
        self.next_sr_id += 1;
        id
    }

    /// REQ-0134: the SIL allocated to a safety function is the most
    /// demanding required-SIL among every hazard it mitigates. This is
    /// the IEC 61508 allocation rule — a function protecting against two
    /// hazards inherits the worse of the two. Returns `None` when the SF
    /// mitigates no hazards, or none of them are assessed yet.
    /// REQ-0138: the project's calibration override table, if any.
    pub fn calibration(&self) -> Option<&BTreeMap<String, CalibrationRow>> {
        self.config
            .as_ref()
            .and_then(|c| c.safety.as_ref())
            .and_then(|s| s.calibration.as_ref())
    }

    /// REQ-0138: a hazard's required SIL under THIS project's calibration
    /// (falling back to Annex D per-leaf). Prefer this over
    /// `Hazard::required_sil`, which always uses the default calibration.
    pub fn required_sil(&self, h: &Hazard) -> Option<Sil> {
        match (h.consequence, h.frequency, h.avoidance, h.probability) {
            (Some(c), Some(f), Some(p), Some(w)) => {
                Some(determine_sil_calibrated(c, f, p, w, self.calibration()))
            }
            _ => None,
        }
    }

    pub fn allocated_sil(&self, sf: &SafetyFunction) -> Option<Sil> {
        sf.links
            .iter()
            .filter(|l| l.kind == LinkKind::Mitigates)
            .filter_map(|l| self.hazards.get(&l.target))
            // REQ-0135: a retired hazard must not keep feeding its SIL
            // into a live function's allocation — that would disagree
            // with the validator, which only counts live mitigations.
            .filter(|h| !matches!(h.status, HazardStatus::Obsolete))
            // REQ-0138: use the project's calibration, not the default.
            .filter_map(|h| self.required_sil(h))
            .max_by_key(|s| s.rank())
    }

    /// REQ-0134: the SIL a safety requirement inherits, taken as the
    /// most demanding allocated-SIL among the safety functions it
    /// realizes. Drives the verification-rigour gate.
    pub fn inherited_sil(&self, sr: &SafetyRequirement) -> Option<Sil> {
        sr.links
            .iter()
            .filter(|l| l.kind == LinkKind::Realizes)
            .filter_map(|l| self.safety_functions.get(&l.target))
            .filter(|sf| !matches!(sf.status, SafetyFunctionStatus::Obsolete))
            .filter_map(|sf| self.allocated_sil(sf))
            .max_by_key(|s| s.rank())
    }
}

/// Serde default + skip helper for the per-family id counters.
fn one() -> u32 {
    1
}
fn is_one(n: &u32) -> bool {
    *n == 1
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Requirement {
    pub id: String,
    pub title: String,
    pub statement: String,
    pub rationale: String,
    pub acceptance: Vec<String>,
    pub kind: Kind,
    pub priority: Priority,
    pub status: Status,
    pub tags: Vec<String>,
    pub links: Vec<Link>,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
    pub history: Vec<HistoryEntry>,
    /// Test records (REQ-0049 / REQ-0050). Defaults to empty so older files
    /// load forward-compatibly.
    #[serde(default)]
    pub tests: Vec<TestRecord>,
    /// REQ-0139: the structured validation dossier — plan → analysis →
    /// testing → statement → verdict. Absent until a validation is
    /// opened; serialised only when present so projects that never use
    /// it keep a byte-identical file (and integrity hash).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation: Option<Validation>,
    /// REQ-0140: forward-compatibility catch-all — see `Project::extra`.
    /// Preserves any per-requirement field a newer `req` writes (the
    /// silent-drop of `validation` by a stale binary is what motivated it).
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestRecord {
    pub at: DateTime<Utc>,
    pub actor: String,
    pub commit: String,
    pub outcome: TestOutcome,
    pub notes: String,
    /// Implements the policy that Verified status can be backed by an
    /// automated test OR a written justification (composition or
    /// inspection). Defaults to Automated for forward compat with
    /// older project.req files.
    #[serde(default = "EvidenceKind::automated")]
    pub kind: EvidenceKind,
    /// REQ-0112: sha256 of the linked-file contents at record time.
    /// When present, `req stale` compares this against a re-hash of
    /// the current files; STALE fires only when content actually
    /// changed, not on every HEAD move. Older records without this
    /// field continue to use the SHA-based comparison.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
    /// REQ-0112: optional explicit list of linked file paths. When
    /// set, overrides the default auto-discovery via `// REQ-NNNN:`
    /// markers. Use when the marker scan would be too blunt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub linked_files: Option<Vec<String>>,
    /// REQ-0135: set true ONLY by `req sreq verify --force` when it
    /// deliberately overrode the SIL-rigour gate (a SIL 3/4 safety
    /// requirement verified on inspection-only evidence). This is the
    /// structured, non-forgeable record of an audited exception — the
    /// validator keys REQ-V-0031 off this field, not off a substring in
    /// `notes`, so the exception cannot be faked by hand-writing notes.
    /// The justifying `--reason` is recorded in `notes`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub sil_gate_exception: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

#[derive(Debug, Copy, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum TestOutcome {
    Pass,
    Fail,
}

impl TestOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            TestOutcome::Pass => "pass",
            TestOutcome::Fail => "fail",
        }
    }
}

// ============================================================================
// REQ-0139: the validation dossier
//
// A requirement (or safety requirement) reaches Verified only after a
// staged validation an agent must fill IN ORDER:
//
//   1. plan      — how the obligation will be validated (analysis + testing).
//   2. analysis  — validation by analysis (code review): findings + pass/fail.
//   3. testing   — validation by testing: findings + pass/fail, referencing
//                  recorded TestRecords when they exist, else structured prose.
//   4. statement — the written validation statement and the final verdict.
//
// The verdict is DERIVED (Pass only when both activity outcomes pass), never
// free-typed, and a passing dossier is the precondition for promotion to
// Verified. The dossier anchors a content hash of the linked source at
// conclude time so a later code change drifts it STALE — the verification
// does not stand forever once the code it covers moves.
// ============================================================================

/// REQ-0139: one validation activity (the analysis stage or the testing
/// stage). Carries the findings, this dimension's pass/fail outcome, and
/// supporting references (files/commits reviewed, or test names / test
/// records cited).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationActivity {
    pub summary: String,
    pub outcome: TestOutcome,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<String>,
    pub at: DateTime<Utc>,
    pub actor: String,
}

/// REQ-0139: the staged validation dossier attached to a requirement or
/// safety requirement. Stages fill in order; `verdict` stays `None` until
/// `conclude` derives it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Validation {
    pub plan: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub analysis: Option<ValidationActivity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub testing: Option<ValidationActivity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub statement: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verdict: Option<TestOutcome>,
    /// True when this dossier is an audited `--no-dossier` exemption
    /// (ordinary requirements only); the justification is in `statement`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub exempt: bool,
    pub opened: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub opened_commit: String,
    pub actor: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub concluded: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub concluded_commit: Option<String>,
    /// Hash of the linked source files at conclude time — the staleness
    /// anchor that lets `req stale` invalidate the verification when the
    /// covered code later changes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub linked_files: Option<Vec<String>>,
}

impl Validation {
    /// A fresh dossier holding only the plan.
    pub fn opened(plan: String, actor: String, commit: String, at: DateTime<Utc>) -> Self {
        Self {
            plan,
            analysis: None,
            testing: None,
            statement: None,
            verdict: None,
            exempt: false,
            opened: at,
            opened_commit: commit,
            actor,
            concluded: None,
            concluded_commit: None,
            content_hash: None,
            linked_files: None,
        }
    }

    pub fn is_concluded(&self) -> bool {
        self.verdict.is_some()
    }

    /// Whether this dossier satisfies the promotion / validation gate: a
    /// concluded Pass verdict, or an audited exemption.
    pub fn passed(&self) -> bool {
        self.exempt || matches!(self.verdict, Some(TestOutcome::Pass))
    }

    /// The verdict the two activity outcomes imply: Pass only when both
    /// stages passed; Fail otherwise. `None` when either stage is missing.
    pub fn derive_verdict(&self) -> Option<TestOutcome> {
        match (&self.analysis, &self.testing) {
            (Some(a), Some(t)) => {
                if matches!(a.outcome, TestOutcome::Pass) && matches!(t.outcome, TestOutcome::Pass)
                {
                    Some(TestOutcome::Pass)
                } else {
                    Some(TestOutcome::Fail)
                }
            }
            _ => None,
        }
    }
}

#[derive(Debug, Copy, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum EvidenceKind {
    /// Captured by `req test run` from a cargo (or other) test suite.
    Automated,
    /// Verified by citing another requirement's passing tests; the notes
    /// should name the cited evidence.
    Composition,
    /// Verified by human review of the code at the recorded commit.
    Inspection,
}

impl EvidenceKind {
    pub fn automated() -> Self {
        EvidenceKind::Automated
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            EvidenceKind::Automated => "automated",
            EvidenceKind::Composition => "composition",
            EvidenceKind::Inspection => "inspection",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Link {
    pub kind: LinkKind,
    pub target: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub at: DateTime<Utc>,
    pub actor: String,
    /// Implements REQ-0043: human vs agent vs unknown. Defaults to Unknown so
    /// older files (where the field is absent) load forward-compatibly.
    #[serde(default = "ActorKind::unknown")]
    pub actor_kind: ActorKind,
    pub action: String,
    pub reason: Option<String>,
}

#[derive(Debug, Copy, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ActorKind {
    Human,
    Agent,
    Unknown,
}

impl ActorKind {
    pub fn unknown() -> Self {
        ActorKind::Unknown
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            ActorKind::Human => "human",
            ActorKind::Agent => "agent",
            ActorKind::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Copy, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Kind {
    Functional,
    NonFunctional,
    Constraint,
    Interface,
    Business,
}

#[derive(Debug, Copy, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Priority {
    Must,
    Should,
    Could,
    Wont,
}

#[derive(Debug, Copy, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Status {
    Draft,
    Proposed,
    Approved,
    Implemented,
    Verified,
    Obsolete,
}

/// REQ-0084: cross-surface lifecycle state machine.
/// Lifecycle policy: which transitions are free (the natural workflow)
/// versus which need an explicit `--force` to acknowledge the irregular
/// move. Returns true when `from -> to` is natural.
///
/// Natural transitions:
///   • Forward one step on the ladder.
///   • From Draft, jump directly to Proposed or Approved (the "sketch
///     and slot" carve-out — Draft is a scratch state).
///   • Any active status to Obsolete (retire).
///   • Same state (no-op handled by the caller).
///
/// Irregular (force-required):
///   • Skip-forward past Approved (e.g. Draft -> Implemented).
///   • Backward moves (e.g. Verified -> Approved). These are real,
///     legitimate operations — a bad test record, a wrong promotion —
///     but they should be deliberate and recorded.
///   • Resurrection (Obsolete -> anything).
///   • Leaving Verified for anything but Obsolete (sticky-Verified).
pub fn is_natural_transition(from: Status, to: Status) -> bool {
    use Status::*;
    if from == to {
        return true;
    }
    if to == Obsolete && from != Obsolete {
        return true;
    }
    matches!(
        (from, to),
        (Draft, Proposed)
            | (Draft, Approved) // carve-out
            | (Proposed, Approved)
            | (Approved, Implemented)
            | (Implemented, Verified)
    )
}

#[derive(Debug, Copy, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum LinkKind {
    Parent,
    DependsOn,
    Conflicts,
    Refines,
    Verifies,
    /// REQ-0134: a safety function mitigates a hazard (SF -> HAZ).
    Mitigates,
    /// REQ-0134: a safety requirement realizes a safety function
    /// (SR -> SF).
    Realizes,
}

impl From<KindArg> for Kind {
    fn from(k: KindArg) -> Self {
        match k {
            KindArg::Functional => Kind::Functional,
            KindArg::NonFunctional => Kind::NonFunctional,
            KindArg::Constraint => Kind::Constraint,
            KindArg::Interface => Kind::Interface,
            KindArg::Business => Kind::Business,
        }
    }
}

impl From<PriorityArg> for Priority {
    fn from(p: PriorityArg) -> Self {
        match p {
            PriorityArg::Must => Priority::Must,
            PriorityArg::Should => Priority::Should,
            PriorityArg::Could => Priority::Could,
            PriorityArg::Wont => Priority::Wont,
        }
    }
}

impl From<StatusArg> for Status {
    fn from(s: StatusArg) -> Self {
        match s {
            StatusArg::Draft => Status::Draft,
            StatusArg::Proposed => Status::Proposed,
            StatusArg::Approved => Status::Approved,
            StatusArg::Implemented => Status::Implemented,
            StatusArg::Verified => Status::Verified,
            StatusArg::Obsolete => Status::Obsolete,
        }
    }
}

impl From<LinkKindArg> for LinkKind {
    fn from(l: LinkKindArg) -> Self {
        match l {
            LinkKindArg::Parent => LinkKind::Parent,
            LinkKindArg::DependsOn => LinkKind::DependsOn,
            LinkKindArg::Conflicts => LinkKind::Conflicts,
            LinkKindArg::Refines => LinkKind::Refines,
            LinkKindArg::Verifies => LinkKind::Verifies,
        }
    }
}

impl From<ConsequenceArg> for Consequence {
    fn from(c: ConsequenceArg) -> Self {
        match c {
            ConsequenceArg::Ca => Consequence::Ca,
            ConsequenceArg::Cb => Consequence::Cb,
            ConsequenceArg::Cc => Consequence::Cc,
            ConsequenceArg::Cd => Consequence::Cd,
        }
    }
}

impl From<FrequencyArg> for Frequency {
    fn from(f: FrequencyArg) -> Self {
        match f {
            FrequencyArg::Fa => Frequency::Fa,
            FrequencyArg::Fb => Frequency::Fb,
        }
    }
}

impl From<AvoidanceArg> for Avoidance {
    fn from(a: AvoidanceArg) -> Self {
        match a {
            AvoidanceArg::Pa => Avoidance::Pa,
            AvoidanceArg::Pb => Avoidance::Pb,
        }
    }
}

impl From<ProbabilityArg> for Probability {
    fn from(p: ProbabilityArg) -> Self {
        match p {
            ProbabilityArg::W1 => Probability::W1,
            ProbabilityArg::W2 => Probability::W2,
            ProbabilityArg::W3 => Probability::W3,
        }
    }
}

impl From<HazardStatusArg> for HazardStatus {
    fn from(s: HazardStatusArg) -> Self {
        match s {
            HazardStatusArg::Identified => HazardStatus::Identified,
            HazardStatusArg::Assessed => HazardStatus::Assessed,
            HazardStatusArg::Mitigated => HazardStatus::Mitigated,
            HazardStatusArg::Verified => HazardStatus::Verified,
            HazardStatusArg::Obsolete => HazardStatus::Obsolete,
        }
    }
}

impl From<SafetyFunctionStatusArg> for SafetyFunctionStatus {
    fn from(s: SafetyFunctionStatusArg) -> Self {
        match s {
            SafetyFunctionStatusArg::Proposed => SafetyFunctionStatus::Proposed,
            SafetyFunctionStatusArg::Allocated => SafetyFunctionStatus::Allocated,
            SafetyFunctionStatusArg::Implemented => SafetyFunctionStatus::Implemented,
            SafetyFunctionStatusArg::Verified => SafetyFunctionStatus::Verified,
            SafetyFunctionStatusArg::Obsolete => SafetyFunctionStatus::Obsolete,
        }
    }
}

impl From<EvidenceArg> for EvidenceKind {
    fn from(e: EvidenceArg) -> Self {
        match e {
            EvidenceArg::Automated => EvidenceKind::Automated,
            EvidenceArg::Composition => EvidenceKind::Composition,
            EvidenceArg::Inspection => EvidenceKind::Inspection,
        }
    }
}

impl Kind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Kind::Functional => "functional",
            Kind::NonFunctional => "non-functional",
            Kind::Constraint => "constraint",
            Kind::Interface => "interface",
            Kind::Business => "business",
        }
    }
}

impl Priority {
    pub fn as_str(&self) -> &'static str {
        match self {
            Priority::Must => "must",
            Priority::Should => "should",
            Priority::Could => "could",
            Priority::Wont => "wont",
        }
    }
}

impl Status {
    pub fn as_str(&self) -> &'static str {
        match self {
            Status::Draft => "draft",
            Status::Proposed => "proposed",
            Status::Approved => "approved",
            Status::Implemented => "implemented",
            Status::Verified => "verified",
            Status::Obsolete => "obsolete",
        }
    }
}

impl LinkKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            LinkKind::Parent => "parent",
            LinkKind::DependsOn => "depends-on",
            LinkKind::Conflicts => "conflicts",
            LinkKind::Refines => "refines",
            LinkKind::Verifies => "verifies",
            LinkKind::Mitigates => "mitigates",
            LinkKind::Realizes => "realizes",
        }
    }
}

// ============================================================================
// REQ-0134: functional-safety model (IEC 61508)
//
// Four artifacts wire together a complete safety case:
//
//   HAZ-NNNN  Hazard            risk assessed via the C/F/P/W risk graph,
//             ───────►          which DERIVES a required SIL. There is no
//                               hand-set SIL field; the integrity comes
//                               from the inputs, not a typed-in level.
//   SF-NNNN   Safety Function   mitigates one or more hazards; its
//             ───────►          allocated SIL is the MAX of the required
//                               SIL of the hazards it covers.
//   SR-NNNN   Safety Requirement realizes one or more safety functions and
//             ───────►          inherits the function's SIL. Carries its own
//                               lifecycle, // SR-NNNN code markers, and
//                               verification evidence; the SIL drives how
//                               rigorous that verification must be.
//
// The derivation chain (C/F/P/W -> required -> allocated -> inherited)
// means an agent cannot quietly assign a convenient integrity level: the
// only inputs are the qualitative risk parameters, and the validator
// recomputes everything downstream.
// ============================================================================

/// IEC 61508-5 Annex D risk-graph parameter **C** — the consequence of
/// the hazardous event.
#[derive(Debug, Copy, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Consequence {
    /// C_A — minor injury.
    #[serde(rename = "C_A")]
    Ca,
    /// C_B — serious permanent injury to one or more persons; death to one.
    #[serde(rename = "C_B")]
    Cb,
    /// C_C — death to several persons.
    #[serde(rename = "C_C")]
    Cc,
    /// C_D — many people killed.
    #[serde(rename = "C_D")]
    Cd,
}

/// IEC 61508-5 Annex D risk-graph parameter **F** — frequency of, and
/// exposure time in, the hazardous zone.
#[derive(Debug, Copy, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Frequency {
    /// F_A — rare to more often exposure.
    #[serde(rename = "F_A")]
    Fa,
    /// F_B — frequent to permanent exposure.
    #[serde(rename = "F_B")]
    Fb,
}

/// IEC 61508-5 Annex D risk-graph parameter **P** — possibility of
/// avoiding the hazardous event.
#[derive(Debug, Copy, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Avoidance {
    /// P_A — possible under certain conditions.
    #[serde(rename = "P_A")]
    Pa,
    /// P_B — almost impossible.
    #[serde(rename = "P_B")]
    Pb,
}

/// IEC 61508-5 Annex D risk-graph parameter **W** — probability of the
/// unwanted occurrence (the demand rate, absent the safety function).
#[derive(Debug, Copy, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Probability {
    /// W1 — a very slight probability that the unwanted occurrences will
    /// come to pass, and only a few unwanted occurrences are likely.
    W1,
    /// W2 — a slight probability; few unwanted occurrences are likely.
    W2,
    /// W3 — a relatively high probability; frequent unwanted occurrences.
    W3,
}

/// The integrity level the risk graph allocates. The two qualitative
/// floors below SIL 1 (`—` and `a`) and the ceiling above SIL 4 (`b`)
/// are part of the standard's output and are represented explicitly so
/// the gate can tell "no safety requirement" apart from "SIL 1".
#[derive(Debug, Copy, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Sil {
    /// `—` — no safety requirement.
    #[serde(rename = "none")]
    NoneRequired,
    /// `a` — no special safety requirement.
    #[serde(rename = "a")]
    A,
    #[serde(rename = "SIL1")]
    Sil1,
    #[serde(rename = "SIL2")]
    Sil2,
    #[serde(rename = "SIL3")]
    Sil3,
    #[serde(rename = "SIL4")]
    Sil4,
    /// `b` — a single E/E/PE safety-related system is not sufficient.
    #[serde(rename = "b")]
    B,
}

impl Consequence {
    pub fn as_str(&self) -> &'static str {
        match self {
            Consequence::Ca => "C_A",
            Consequence::Cb => "C_B",
            Consequence::Cc => "C_C",
            Consequence::Cd => "C_D",
        }
    }
}

impl Frequency {
    pub fn as_str(&self) -> &'static str {
        match self {
            Frequency::Fa => "F_A",
            Frequency::Fb => "F_B",
        }
    }
}

impl Avoidance {
    pub fn as_str(&self) -> &'static str {
        match self {
            Avoidance::Pa => "P_A",
            Avoidance::Pb => "P_B",
        }
    }
}

impl Probability {
    pub fn as_str(&self) -> &'static str {
        match self {
            Probability::W1 => "W1",
            Probability::W2 => "W2",
            Probability::W3 => "W3",
        }
    }
}

impl Sil {
    /// Total order for "take the most demanding SIL" aggregation.
    pub fn rank(&self) -> u8 {
        match self {
            Sil::NoneRequired => 0,
            Sil::A => 1,
            Sil::Sil1 => 2,
            Sil::Sil2 => 3,
            Sil::Sil3 => 4,
            Sil::Sil4 => 5,
            Sil::B => 6,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Sil::NoneRequired => "—",
            Sil::A => "a",
            Sil::Sil1 => "SIL1",
            Sil::Sil2 => "SIL2",
            Sil::Sil3 => "SIL3",
            Sil::Sil4 => "SIL4",
            Sil::B => "b",
        }
    }

    /// REQ-0138: parse a SIL from a calibration token. Accepts the
    /// canonical forms plus shorthands: `1..4`, `SIL1..SIL4`, `a`, `b`,
    /// and `none`/`-`/`—` for "no safety requirement". Case-insensitive.
    pub fn parse(s: &str) -> Option<Sil> {
        match s.trim().to_lowercase().as_str() {
            "none" | "-" | "—" | "" => Some(Sil::NoneRequired),
            "a" => Some(Sil::A),
            "1" | "sil1" => Some(Sil::Sil1),
            "2" | "sil2" => Some(Sil::Sil2),
            "3" | "sil3" => Some(Sil::Sil3),
            "4" | "sil4" => Some(Sil::Sil4),
            "b" => Some(Sil::B),
            _ => None,
        }
    }
}

/// REQ-0134: the IEC 61508-5 Annex D risk graph — the standard's WORKED
/// EXAMPLE calibration. Pure function of the four qualitative parameters;
/// this is the single place SIL is ever decided. Every leaf is pinned by
/// a unit test. NOTE: Annex D requires a risk graph to be calibrated per
/// project/sector; this default calibration must be confirmed or replaced
/// for real safety-related use (see `req help safety`).
///
/// The table is read as: pick the (C, F, P) leaf to get a row of three
/// outcomes ordered `[W3, W2, W1]`, then index by W. C_A short-circuits
/// to "no safety requirement" before F/P/W are even consulted.
pub fn determine_sil(c: Consequence, f: Frequency, p: Avoidance, w: Probability) -> Sil {
    use Avoidance::*;
    use Consequence::*;
    use Frequency::*;
    use Probability::*;
    use Sil::{NoneRequired as N, Sil1, Sil2, Sil3, A, B};

    if let Ca = c {
        return N;
    }
    // [W3, W2, W1] for each consequence/frequency/avoidance leaf.
    let row: [Sil; 3] = match (c, f, p) {
        (Cb, Fa, Pa) => [A, N, N],
        (Cb, Fa, Pb) => [Sil1, A, N],
        (Cb, Fb, Pa) => [Sil1, A, N],
        (Cb, Fb, Pb) => [Sil2, Sil1, A],
        (Cc, Fa, Pa) => [Sil1, A, N],
        (Cc, Fa, Pb) => [Sil2, Sil1, A],
        (Cc, Fb, Pa) => [Sil2, Sil1, A],
        (Cc, Fb, Pb) => [Sil3, Sil2, Sil1],
        (Cd, Fa, Pa) => [Sil2, Sil1, A],
        (Cd, Fa, Pb) => [Sil3, Sil2, Sil1],
        (Cd, Fb, Pa) => [Sil3, Sil2, Sil1],
        (Cd, Fb, Pb) => [B, Sil3, Sil2],
        // C_A handled above; the compiler can't see that, so cover it.
        (Ca, _, _) => [N, N, N],
    };
    match w {
        W3 => row[0],
        W2 => row[1],
        W1 => row[2],
    }
}

/// REQ-0138: the canonical calibration-leaf key for a (C,F,P) triple,
/// e.g. "C_D/F_B/P_B". This is how a per-project calibration addresses
/// the cell it overrides.
pub fn calibration_leaf(c: Consequence, f: Frequency, p: Avoidance) -> String {
    format!("{}/{}/{}", c.as_str(), f.as_str(), p.as_str())
}

/// REQ-0138: resolve a SIL for the four risk parameters, honouring a
/// per-project calibration override for the matching leaf and falling
/// back to the Annex D worked-example default (`determine_sil`) for any
/// leaf the calibration does not override. The "sensible defaults" half
/// of full-overridable calibration.
pub fn determine_sil_calibrated(
    c: Consequence,
    f: Frequency,
    p: Avoidance,
    w: Probability,
    calibration: Option<&BTreeMap<String, CalibrationRow>>,
) -> Sil {
    if let Some(table) = calibration {
        if let Some(row) = table.get(&calibration_leaf(c, f, p)) {
            return match w {
                Probability::W1 => row.w1,
                Probability::W2 => row.w2,
                Probability::W3 => row.w3,
            };
        }
    }
    determine_sil(c, f, p, w)
}

/// Lifecycle of a hazard. Mirrors the requirement ladder's shape but
/// names the functional-safety states: a hazard is Identified, then
/// Assessed (C/F/P/W set, SIL derived), then Mitigated (a safety
/// function covers it), then Verified (the mitigation is shown
/// effective), or Obsolete (retired/reclassified).
#[derive(Debug, Copy, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum HazardStatus {
    Identified,
    Assessed,
    Mitigated,
    Verified,
    Obsolete,
}

impl HazardStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            HazardStatus::Identified => "identified",
            HazardStatus::Assessed => "assessed",
            HazardStatus::Mitigated => "mitigated",
            HazardStatus::Verified => "verified",
            HazardStatus::Obsolete => "obsolete",
        }
    }
}

/// REQ-0134: a hazardous event and its risk assessment. The four risk
/// parameters are optional so a hazard can be logged at `Identified`
/// before it is assessed; the validator requires them from `Assessed`
/// onward. The SIL is never stored — `required_sil()` derives it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hazard {
    pub id: String,
    pub title: String,
    pub description: String,
    /// The operational situation / mode in which the hazard arises.
    pub operating_context: String,
    /// REQ-0134: free-text narrative of the potential harm, in the
    /// reviewer's own words — "an operator's hand could be severed",
    /// "a pedestrian could be killed". This is deliberately distinct
    /// from the `consequence` bucket below: the C_A..C_D parameter is
    /// the formal severity class fed to the risk graph, while this
    /// sentence is what a human reviewer reads and sanity-checks that
    /// bucket against. Captured from `Identified` onward, before the
    /// hazard is formally assessed.
    pub harm: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub consequence: Option<Consequence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frequency: Option<Frequency>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avoidance: Option<Avoidance>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probability: Option<Probability>,
    pub status: HazardStatus,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub links: Vec<Link>,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
    pub history: Vec<HistoryEntry>,
    /// REQ-0140: forward-compatibility catch-all — see `Project::extra`.
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

impl Hazard {
    /// True once every risk parameter is set. (The required SIL itself is
    /// derived via `Project::required_sil`, which also applies the
    /// project's calibration — there is no hazard-local SIL getter so a
    /// caller can't accidentally bypass the calibration.)
    pub fn is_assessed(&self) -> bool {
        self.consequence.is_some()
            && self.frequency.is_some()
            && self.avoidance.is_some()
            && self.probability.is_some()
    }
}

/// Lifecycle of a safety function.
#[derive(Debug, Copy, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum SafetyFunctionStatus {
    Proposed,
    Allocated,
    Implemented,
    Verified,
    Obsolete,
}

impl SafetyFunctionStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            SafetyFunctionStatus::Proposed => "proposed",
            SafetyFunctionStatus::Allocated => "allocated",
            SafetyFunctionStatus::Implemented => "implemented",
            SafetyFunctionStatus::Verified => "verified",
            SafetyFunctionStatus::Obsolete => "obsolete",
        }
    }
}

/// REQ-0134: a safety function — the risk-reduction measure that brings
/// a hazardous situation to, or maintains it in, a safe state. Its SIL
/// is allocated (derived) from the hazards it mitigates, not stored.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SafetyFunction {
    pub id: String,
    pub title: String,
    pub description: String,
    /// The safe state this function achieves or maintains.
    pub safe_state: String,
    pub status: SafetyFunctionStatus,
    #[serde(default)]
    pub tags: Vec<String>,
    /// Links carry the `mitigates` edges to the hazards this function
    /// covers.
    #[serde(default)]
    pub links: Vec<Link>,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
    pub history: Vec<HistoryEntry>,
    /// REQ-0140: forward-compatibility catch-all — see `Project::extra`.
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

/// REQ-0134: a safety requirement — a normative obligation that realizes
/// a safety function. It carries the full requirement machinery
/// (acceptance criteria, lifecycle, // SR-NNNN code markers, test
/// records) but lives in its own id space, and the SIL it inherits from
/// its safety function drives the verification-rigour gate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SafetyRequirement {
    pub id: String,
    pub title: String,
    pub statement: String,
    pub rationale: String,
    #[serde(default)]
    pub acceptance: Vec<String>,
    pub priority: Priority,
    pub status: Status,
    #[serde(default)]
    pub tags: Vec<String>,
    /// Links carry the `realizes` edges to the safety functions this
    /// requirement implements.
    #[serde(default)]
    pub links: Vec<Link>,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
    pub history: Vec<HistoryEntry>,
    #[serde(default)]
    pub tests: Vec<TestRecord>,
    /// REQ-0139: the staged validation dossier. Mandatory (no tag
    /// exemption) before a safety requirement may reach Verified.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation: Option<Validation>,
    /// REQ-0140: forward-compatibility catch-all — see `Project::extra`.
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[cfg(test)]
mod sil_tests {
    use super::*;

    // Pin every leaf of the IEC 61508-5 Annex D calibrated risk graph.
    // Reference: each (C, F, P) maps to outcomes for [W3, W2, W1].
    #[test]
    #[allow(clippy::type_complexity)]
    fn risk_graph_every_leaf() {
        use Avoidance::*;
        use Consequence::*;
        use Frequency::*;
        use Probability::*;
        use Sil::{NoneRequired as N, Sil1, Sil2, Sil3, A, B};

        // (C, F, P) -> [W3, W2, W1]
        let table: &[((Consequence, Frequency, Avoidance), [Sil; 3])] = &[
            ((Cb, Fa, Pa), [A, N, N]),
            ((Cb, Fa, Pb), [Sil1, A, N]),
            ((Cb, Fb, Pa), [Sil1, A, N]),
            ((Cb, Fb, Pb), [Sil2, Sil1, A]),
            ((Cc, Fa, Pa), [Sil1, A, N]),
            ((Cc, Fa, Pb), [Sil2, Sil1, A]),
            ((Cc, Fb, Pa), [Sil2, Sil1, A]),
            ((Cc, Fb, Pb), [Sil3, Sil2, Sil1]),
            ((Cd, Fa, Pa), [Sil2, Sil1, A]),
            ((Cd, Fa, Pb), [Sil3, Sil2, Sil1]),
            ((Cd, Fb, Pa), [Sil3, Sil2, Sil1]),
            ((Cd, Fb, Pb), [B, Sil3, Sil2]),
        ];
        for ((c, f, p), [e3, e2, e1]) in table {
            assert_eq!(
                determine_sil(*c, *f, *p, W3),
                *e3,
                "{:?}/{:?}/{:?}/W3",
                c,
                f,
                p
            );
            assert_eq!(
                determine_sil(*c, *f, *p, W2),
                *e2,
                "{:?}/{:?}/{:?}/W2",
                c,
                f,
                p
            );
            assert_eq!(
                determine_sil(*c, *f, *p, W1),
                *e1,
                "{:?}/{:?}/{:?}/W1",
                c,
                f,
                p
            );
        }
    }

    #[test]
    fn consequence_ca_never_needs_safety() {
        use Avoidance::*;
        use Frequency::*;
        use Probability::*;
        for f in [Fa, Fb] {
            for p in [Pa, Pb] {
                for w in [W1, W2, W3] {
                    assert_eq!(determine_sil(Consequence::Ca, f, p, w), Sil::NoneRequired);
                }
            }
        }
    }

    #[test]
    fn sil_rank_is_monotonic() {
        assert!(Sil::B.rank() > Sil::Sil4.rank());
        assert!(Sil::Sil4.rank() > Sil::Sil1.rank());
        assert!(Sil::Sil1.rank() > Sil::A.rank());
        assert!(Sil::A.rank() > Sil::NoneRequired.rank());
    }
}
