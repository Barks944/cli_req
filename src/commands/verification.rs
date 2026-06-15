// REQ-0139: the staged verification dossier.
//
// A requirement (REQ-NNNN) or safety requirement (SR-NNNN) reaches Verified
// only after walking an ordered verification: plan → analysis → testing →
// statement → verdict. The verdict is derived (Pass only when both the
// analysis and the testing stage pass), never free-typed, and a passing
// dossier is the precondition the promotion gate checks before any status
// flips to Verified. The dossier anchors a content hash of the linked source
// at conclude time, so a later code change drifts it STALE (via `req stale`)
// and the verification no longer stands.
//
// This module works on both id families by branching on the id prefix, the
// same shape as `req trace` — there is one `req verification` surface. The
// `op_*` functions are the IO-free core (mutate a &mut Project) shared by
// the CLI wrappers below and the MCP tools in src/mcp.rs.
use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use std::path::{Path, PathBuf};

/// REQ-0165: the three legal routes to a Verified state when the dossier
/// gate blocks a promotion. Returned as discrete strings so the CLI and the
/// MCP surface present identical, enumerable guidance instead of drifting
/// prose.
pub fn promotion_routes(id: &str) -> Vec<String> {
    vec![
        format!(
            "dossier: run `req verification plan {id} ...` → analysis → test → conclude --promote"
        ),
        "waiver: pass --no-dossier --reason \"...\" to record an audited exemption".to_string(),
        format!(
            "exempt: tag {id} `{}` to exclude it from the dossier gate",
            crate::model::DEFAULT_VERIFICATION_EXEMPT_TAG
        ),
    ]
}

/// REQ-0165: the full human-readable rejection, built from the routes so the
/// message and the structured list can never disagree.
pub fn promotion_blocked_message(id: &str) -> String {
    format!(
        "{id} cannot be promoted to Verified without a passing verification dossier. \
         Choose one of:\n  - {}",
        promotion_routes(id).join("\n  - ")
    )
}

use crate::cli::{
    TestResultArg, VerificationActivityArgs, VerificationBackfillArgs, VerificationCmd,
    VerificationConcludeArgs, VerificationConfirmArgs, VerificationPlanArgs,
    VerificationRefreshArgs, VerificationReportArgs, VerificationReverifyArgs,
    VerificationShowArgs,
};
use crate::commands::test_cmd::{auto_linked_files, current_head_sha_opt, hash_files, short};
use crate::model::{
    EvidenceKind, HistoryEntry, Project, SafetyFunctionStatus, Sil, Status, TestOutcome,
    TestRecord, Verification, VerificationActivity,
};
use crate::storage::{self, load_for_mutation, load_resolved};

// --------------------------------------------------------------------------
// shared types
// --------------------------------------------------------------------------

#[derive(Copy, Clone, PartialEq, Eq)]
pub enum Family {
    Req,
    Sr,
    /// REQ-0201: a safety function (SF-NNNN) carries the same dossier as a
    /// safety requirement, so the one `req verification` surface verifies it
    /// too. Like a safety requirement it needs a human co-sign and cannot be
    /// tag-exempted or back-filled.
    Sf,
}

#[derive(Copy, Clone)]
pub enum Stage {
    Analysis,
    Testing,
}

impl Stage {
    fn label(self) -> &'static str {
        match self {
            Stage::Analysis => "analysis",
            Stage::Testing => "testing",
        }
    }
}

/// Outcome of `op_conclude`, for the caller to render.
pub struct ConcludeOutcome {
    pub id: String,
    pub verdict: TestOutcome,
    pub promoted: bool,
    /// REQ-0187: true when this was a safety requirement whose dossier and
    /// evidence were recorded but which now waits at Implemented for a human
    /// co-sign (`req verification confirm`) to reach Verified.
    pub awaiting_confirmation: bool,
}

// REQ-0142: the verification-provenance classifier now lives in its own module
// (src/commands/provenance.rs) so the safety requirement that depends on it
// anchors a small, stable file rather than this whole surface. Re-exported here
// so existing call sites (`verification::classify`, `verification::Provenance`, …)
// keep working.
pub use crate::commands::provenance::{classify, provenance_report, Provenance, ProvenanceRow};

// --------------------------------------------------------------------------
// CLI dispatch
// --------------------------------------------------------------------------

pub fn run(cmd: VerificationCmd, file: &Option<PathBuf>) -> Result<()> {
    match cmd {
        VerificationCmd::Plan(a) => plan(a, file),
        VerificationCmd::Analysis(a) => activity(a, file, Stage::Analysis),
        VerificationCmd::Test(a) => activity(a, file, Stage::Testing),
        VerificationCmd::Conclude(a) => conclude(a, file),
        VerificationCmd::Confirm(a) => confirm(a, file),
        VerificationCmd::Show(a) => show(a, file),
        VerificationCmd::Backfill(a) => backfill(a, file),
        VerificationCmd::Report(a) => report(a, file),
        // REQ-0191: `status` is an alias of `report` — the obvious name for
        // "what is the V&V standing of everything?".
        VerificationCmd::Status(a) => report(a, file),
        // REQ-0153: re-normalize staleness anchors that are provably unchanged.
        VerificationCmd::RefreshAnchors(a) => refresh_anchors(a, file),
        VerificationCmd::Reverify(a) => reverify(a, file),
    }
}

// --------------------------------------------------------------------------
// id resolution + field access
// --------------------------------------------------------------------------

fn normalize_sr(raw: &str) -> String {
    normalize_prefixed(raw, "SR")
}

// REQ-0201: safety functions (SF-NNNN) resolve through the same dossier surface.
fn normalize_sf(raw: &str) -> String {
    normalize_prefixed(raw, "SF")
}

/// Canonicalise `PREFIX-NNNN`, accepting bare digits and a missing prefix.
fn normalize_prefixed(raw: &str, prefix: &str) -> String {
    let trimmed = raw.trim();
    let upper = trimmed.to_uppercase();
    let dash = format!("{}-", prefix);
    let digits = if let Some(rest) = upper.strip_prefix(&dash) {
        rest.to_string()
    } else if trimmed.chars().all(|c| c.is_ascii_digit()) && !trimmed.is_empty() {
        trimmed.to_string()
    } else {
        return upper;
    };
    match digits.parse::<u32>() {
        Ok(n) => format!("{}-{:04}", prefix, n),
        Err(_) => upper,
    }
}

/// Resolve a raw id to its canonical form and family. SR-prefixed ids route to
/// the safety-requirements map, SF-prefixed ids to the safety-functions map;
/// everything else is an ordinary requirement.
pub fn resolve(project: &Project, raw: &str) -> Result<(String, Family)> {
    let upper = raw.trim().to_uppercase();
    if upper.starts_with("SR") {
        let id = normalize_sr(raw);
        if project.safety_requirements.contains_key(&id) {
            Ok((id, Family::Sr))
        } else {
            Err(anyhow!("no such safety requirement: {}", raw))
        }
    } else if upper.starts_with("SF") {
        let id = normalize_sf(raw);
        if project.safety_functions.contains_key(&id) {
            Ok((id, Family::Sf))
        } else {
            Err(anyhow!("no such safety function: {}", raw))
        }
    } else {
        let id = super::resolve_id(project, raw)?;
        Ok((id, Family::Req))
    }
}

/// REQ-0201: a mutable handle to whichever status field the item carries.
/// Ordinary requirements and safety requirements use `Status`; safety functions
/// use `SafetyFunctionStatus`. The dossier workflow only ever needs to advance to
/// Implemented (awaiting co-sign) or Verified, so the slot exposes just those
/// two transitions plus a label, hiding the type divergence from the ops.
enum StatusSlot<'a> {
    Req(&'a mut Status),
    Sf(&'a mut SafetyFunctionStatus),
}

impl StatusSlot<'_> {
    fn set_implemented(&mut self) {
        match self {
            StatusSlot::Req(s) => **s = Status::Implemented,
            StatusSlot::Sf(s) => **s = SafetyFunctionStatus::Implemented,
        }
    }
    fn set_verified(&mut self) {
        match self {
            StatusSlot::Req(s) => **s = Status::Verified,
            StatusSlot::Sf(s) => **s = SafetyFunctionStatus::Verified,
        }
    }
}

/// Disjoint mutable handles to the dossier-bearing fields shared by
/// `Requirement`, `SafetyRequirement`, and `SafetyFunction`. A safety function
/// has no `tests` vector (its dossier is its evidence), so `tests` is optional.
struct ItemMut<'a> {
    verification: &'a mut Option<Verification>,
    status: StatusSlot<'a>,
    history: &'a mut Vec<HistoryEntry>,
    updated: &'a mut DateTime<Utc>,
    tests: Option<&'a mut Vec<TestRecord>>,
}

fn item_mut<'a>(project: &'a mut Project, id: &str, fam: Family) -> ItemMut<'a> {
    match fam {
        Family::Req => {
            let r = project.requirements.get_mut(id).unwrap();
            ItemMut {
                verification: &mut r.verification,
                status: StatusSlot::Req(&mut r.status),
                history: &mut r.history,
                updated: &mut r.updated,
                tests: Some(&mut r.tests),
            }
        }
        Family::Sr => {
            let sr = project.safety_requirements.get_mut(id).unwrap();
            ItemMut {
                verification: &mut sr.verification,
                status: StatusSlot::Req(&mut sr.status),
                history: &mut sr.history,
                updated: &mut sr.updated,
                tests: Some(&mut sr.tests),
            }
        }
        Family::Sf => {
            let sf = project.safety_functions.get_mut(id).unwrap();
            ItemMut {
                verification: &mut sf.verification,
                status: StatusSlot::Sf(&mut sf.status),
                history: &mut sf.history,
                updated: &mut sf.updated,
                tests: None,
            }
        }
    }
}

/// Whether the item already carries a passing automated/composition test
/// record — the "strong" evidence the SIL-rigour gate wants.
fn has_strong_evidence(project: &Project, id: &str, fam: Family) -> bool {
    let tests = match fam {
        Family::Req => &project.requirements[id].tests,
        Family::Sr => &project.safety_requirements[id].tests,
        // REQ-0201: a safety function has no separate test-evidence model — its
        // dossier is the evidence, so there is never "strong" evidence to compose.
        Family::Sf => return false,
    };
    tests.iter().any(|t| {
        matches!(t.outcome, TestOutcome::Pass)
            && matches!(t.kind, EvidenceKind::Automated | EvidenceKind::Composition)
    })
}

pub fn dossier<'a>(project: &'a Project, id: &str, fam: Family) -> Option<&'a Verification> {
    match fam {
        Family::Req => project.requirements[id].verification.as_ref(),
        Family::Sr => project.safety_requirements[id].verification.as_ref(),
        Family::Sf => project.safety_functions[id].verification.as_ref(),
    }
}

fn title_of(project: &Project, id: &str, fam: Family) -> String {
    match fam {
        Family::Req => project.requirements[id].title.clone(),
        Family::Sr => project.safety_requirements[id].title.clone(),
        Family::Sf => project.safety_functions[id].title.clone(),
    }
}

fn test_summaries(project: &Project, id: &str, fam: Family) -> Vec<String> {
    let tests = match fam {
        Family::Req => &project.requirements[id].tests,
        Family::Sr => &project.safety_requirements[id].tests,
        Family::Sf => return Vec::new(),
    };
    tests.iter().map(summarise_record).collect()
}

fn summarise_record(t: &crate::model::TestRecord) -> String {
    format!(
        "record: {} @{} ({})",
        t.outcome.as_str(),
        if t.commit.is_empty() {
            "—".to_string()
        } else {
            short(&t.commit)
        },
        t.kind.as_str()
    )
}

// --------------------------------------------------------------------------
// core ops (IO-free; shared by CLI + MCP)
// --------------------------------------------------------------------------

/// Stage 1 — open (or re-open) the dossier with the plan.
pub fn op_plan(
    project: &mut Project,
    raw: &str,
    plan: &str,
    reopen: bool,
    reason: Option<&str>,
) -> Result<String> {
    let (id, fam) = resolve(project, raw)?;
    let now = Utc::now();
    let commit = current_head_sha_opt().unwrap_or_default();
    let actor = super::current_actor();
    {
        let it = item_mut(project, &id, fam);
        if let Some(v) = it.verification.as_ref() {
            if v.is_concluded() && !reopen {
                return Err(anyhow!(
                    "{} already has a concluded verification dossier — pass --reopen --reason \"...\" \
                     to re-verify (this clears the prior verdict).",
                    id
                ));
            }
        }
        *it.verification = Some(Verification::opened(plan.to_string(), actor, commit, now));
        *it.updated = now;
        it.history.push(super::history(
            if reopen {
                "verification re-opened (plan recorded)"
            } else {
                "verification plan recorded"
            },
            reason.map(|s| s.to_string()),
        ));
    }
    project.updated = now;
    Ok(id)
}

/// Stages 2 & 3 — record verification by analysis / by testing.
pub fn op_activity(
    project: &mut Project,
    raw: &str,
    stage: Stage,
    findings: &str,
    outcome: TestOutcome,
    references: &[String],
) -> Result<String> {
    let (id, fam) = resolve(project, raw)?;
    let now = Utc::now();
    let actor = super::current_actor();

    // Build references: for testing, fold in recorded test evidence on top
    // of the caller's explicit references (reference-if-present).
    let mut refs: Vec<String> = references.to_vec();
    if matches!(stage, Stage::Testing) {
        for s in test_summaries(project, &id, fam) {
            if !refs.contains(&s) {
                refs.push(s);
            }
        }
    }
    let entry = VerificationActivity {
        summary: findings.to_string(),
        outcome,
        references: refs,
        at: now,
        actor,
    };
    {
        let it = item_mut(project, &id, fam);
        let v = it.verification.as_mut().ok_or_else(|| {
            anyhow!(
                "{} has no verification dossier — run `req verification plan {} ...` first",
                id,
                id
            )
        })?;
        if v.is_concluded() {
            return Err(anyhow!(
                "{}'s dossier is already concluded — re-open it with `req verification plan {} --reopen --reason \"...\"` to revise.",
                id, id
            ));
        }
        match stage {
            Stage::Analysis => v.analysis = Some(entry),
            Stage::Testing => {
                if v.analysis.is_none() {
                    return Err(anyhow!(
                        "record verification by analysis before testing — run `req verification analysis {} ...` first",
                        id
                    ));
                }
                v.testing = Some(entry);
            }
        }
        *it.updated = now;
        it.history.push(super::history(
            format!(
                "verification {} recorded ({})",
                stage.label(),
                outcome.as_str()
            ),
            None,
        ));
    }
    project.updated = now;
    Ok(id)
}

/// Stage 4 — conclude: derive the verdict, record the statement, and
/// optionally promote (gated). `source_root` is where linked files are
/// hashed for the staleness anchor.
pub fn op_conclude(
    project: &mut Project,
    raw: &str,
    statement: &str,
    promote: bool,
    force: bool,
    reason: Option<&str>,
    source_root: &Path,
) -> Result<ConcludeOutcome> {
    let (id, fam) = resolve(project, raw)?;
    let now = Utc::now();
    let commit = current_head_sha_opt().unwrap_or_default();

    // Verdict + promotion preflight, all read-only, before any mutation.
    let verdict = {
        let v = dossier(project, &id, fam).ok_or_else(|| {
            anyhow!(
                "{} has no verification dossier — run `req verification plan {} ...` first",
                id,
                id
            )
        })?;
        if v.analysis.is_none() || v.testing.is_none() {
            return Err(anyhow!(
                "{} cannot be concluded — record verification by analysis AND by testing first.",
                id
            ));
        }
        v.derive_verdict().unwrap_or(TestOutcome::Fail)
    };
    if promote {
        if matches!(verdict, TestOutcome::Fail) {
            return Err(anyhow!(
                "{}'s verification verdict is FAIL — cannot promote a failed verification to Verified. \
                 Fix the issue, then `req verification plan {} --reopen --reason \"...\"` and re-verify.",
                id, id
            ));
        }
        promote_preflight(project, &id, fam, force)?;
    }

    // Compute the staleness anchor.
    let linked = auto_linked_files(&id, source_root);
    let content_hash = if linked.is_empty() {
        None
    } else {
        Some(hash_files(&linked))
    };
    let linked_files: Option<Vec<String>> = if linked.is_empty() {
        None
    } else {
        Some(
            linked
                .iter()
                // REQ-0152: store forward-slash paths so the dossier is portable
                // across platforms (and resolves on POSIX even when anchored on
                // Windows).
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .collect(),
        )
    };

    // When promoting on a Pass, the conclusion IS the verification evidence —
    // record a TestRecord so the dossier and the evidence model agree (and
    // REQ-V-0030 sees a Verified safety requirement's passing evidence). The
    // evidence kind composes the dossier's existing strong evidence when
    // present, else it is an inspection-grade conclusion.
    let do_promote = promote && matches!(verdict, TestOutcome::Pass);
    // A safety function has no test-evidence vector — its dossier IS the
    // evidence — so only ordinary / safety requirements record a TestRecord.
    let will_record = do_promote && !matches!(fam, Family::Sf);
    let strong = will_record && has_strong_evidence(project, &id, fam);
    let inherited = if matches!(fam, Family::Sr) {
        project.inherited_sil(&project.safety_requirements[&id])
    } else {
        None
    };
    let evidence_kind = if strong {
        EvidenceKind::Composition
    } else {
        EvidenceKind::Inspection
    };
    // A SIL 3/4 conclusion on inspection-grade evidence is only reachable
    // here under --force (the preflight blocks it otherwise); flag it as the
    // audited exception so REQ-V-0031 treats it as such, not a violation.
    let sil_gate_exception = will_record
        && force
        && matches!(evidence_kind, EvidenceKind::Inspection)
        && inherited
            .map(|s| s.rank() >= Sil::Sil3.rank())
            .unwrap_or(false);

    let mut promoted = false;
    let mut awaiting = false;
    {
        let mut it = item_mut(project, &id, fam);
        {
            let v = it.verification.as_mut().unwrap();
            v.statement = Some(statement.to_string());
            v.verdict = Some(verdict);
            v.concluded = Some(now);
            v.concluded_commit = Some(commit.clone());
            v.content_hash = content_hash.clone();
            v.linked_files = linked_files.clone();
        }
        if do_promote {
            // REQ-0187/REQ-0201: an ordinary requirement is promoted to Verified
            // here; a safety requirement OR a safety function records its genuine
            // dossier but stops at Implemented, awaiting the human co-sign that
            // promotes it. This removes the old unreachable
            // "Verified-but-unconfirmed" hard-error state an agent could not get
            // out of, and extends the co-sign gate up the chain to SFs.
            if matches!(fam, Family::Req) {
                it.status.set_verified();
                promoted = true;
            } else {
                it.status.set_implemented();
                awaiting = true;
            }
            // Only requirements carry a TestRecord vector; an SF's dossier is
            // its own evidence (will_record is false for SFs).
            if will_record {
                if let Some(tests) = it.tests.as_mut() {
                    tests.push(TestRecord {
                        at: now,
                        actor: super::current_actor(),
                        commit: commit.clone(),
                        outcome: TestOutcome::Pass,
                        notes: format!("verification dossier concluded — {}", statement),
                        kind: evidence_kind,
                        content_hash,
                        linked_files,
                        sil_gate_exception,
                        // REQ-0154: snapshot the inherited SIL (SR only) at conclude.
                        sil_at_verification: inherited,
                        external: None,
                    });
                }
            }
        }
        *it.updated = now;
        it.history.push(super::history(
            format!(
                "verification concluded ({}){}",
                verdict.as_str(),
                if promoted {
                    " — promoted to Verified"
                } else if awaiting {
                    " — awaiting human confirmation"
                } else {
                    ""
                }
            ),
            reason.map(|s| s.to_string()),
        ));
    }
    project.updated = now;
    Ok(ConcludeOutcome {
        id,
        verdict,
        promoted,
        awaiting_confirmation: awaiting,
    })
}

/// Read-only promotion checks: status ladder + (for SRs) the SIL-rigour
/// gate, mirroring `req verify` / `req sreq verify`.
fn promote_preflight(project: &Project, id: &str, fam: Family, force: bool) -> Result<()> {
    let (label, ladder_ok) = match fam {
        Family::Req => {
            let s = project.requirements[id].status;
            (s.as_str(), matches!(s, Status::Implemented | Status::Verified))
        }
        Family::Sr => {
            let s = project.safety_requirements[id].status;
            (s.as_str(), matches!(s, Status::Implemented | Status::Verified))
        }
        Family::Sf => {
            // REQ-0201: a safety function has no separate "implement" step: the
            // dossier conclude is what carries it Allocated → Implemented (awaiting
            // co-sign) → Verified. So allocated-or-beyond is the valid base —
            // only a Proposed SF (mitigating no hazard) cannot be verified.
            let s = project.safety_functions[id].status;
            (
                s.as_str(),
                matches!(
                    s,
                    SafetyFunctionStatus::Allocated
                        | SafetyFunctionStatus::Implemented
                        | SafetyFunctionStatus::Verified
                ),
            )
        }
    };
    if !ladder_ok && !force {
        return Err(anyhow!(
            "{} is {} — promoting straight to Verified is irregular. Advance it to Implemented \
             first, or pass --force --reason \"...\".",
            id,
            label
        ));
    }
    if matches!(fam, Family::Sr) {
        let sr = &project.safety_requirements[id];
        if let Some(sil) = project.inherited_sil(sr) {
            let has_strong_evidence = sr.tests.iter().any(|t| {
                matches!(t.outcome, TestOutcome::Pass)
                    && matches!(
                        t.kind,
                        crate::model::EvidenceKind::Automated
                            | crate::model::EvidenceKind::Composition
                    )
            });
            if sil.rank() >= Sil::Sil3.rank() && !has_strong_evidence && !force {
                return Err(anyhow!(
                    "SIL-rigour gate: {} inherits {} — Verified needs automated or composition \
                     test evidence (record it with `req sreq verify {} --by automated ...`), not \
                     analysis/inspection alone. Pass --force --reason \"...\" for an audited exception.",
                    id,
                    sil.as_str(),
                    id
                ));
            }
        }
    }
    Ok(())
}

/// Back-fill an audited exemption onto already-Verified items lacking a
/// passing dossier. Returns the ids touched.
pub fn op_backfill(
    project: &mut Project,
    raw_id: Option<&str>,
    all: bool,
    reason: &str,
) -> Result<Vec<String>> {
    let mut targets: Vec<(String, Family)> = Vec::new();
    if let Some(raw) = raw_id {
        let (id, fam) = resolve(project, raw)?;
        // REQ-0143/REQ-0201: safety requirements and safety functions have NO
        // exemption — they must be verified genuinely, never grandfathered.
        if matches!(fam, Family::Sr | Family::Sf) {
            let kind = if matches!(fam, Family::Sr) {
                "safety requirement"
            } else {
                "safety function"
            };
            return Err(anyhow!(
                "{} is a {} — safety artifacts cannot be exempted. Verify it genuinely with \
                 `req verification plan {} ...` → analysis → test → conclude --promote.",
                id, kind, id
            ));
        }
        targets.push((id, fam));
    } else if all {
        for (id, r) in &project.requirements {
            if matches!(r.status, Status::Verified)
                && !r.verification.as_ref().map(|v| v.passed()).unwrap_or(false)
            {
                targets.push((id.clone(), Family::Req));
            }
        }
        // REQ-0143: --all deliberately skips safety requirements — there is
        // no audited exemption for them.
    } else {
        return Err(anyhow!(
            "pass an id, or --all to back-fill every Verified item without a passing dossier"
        ));
    }

    let now = Utc::now();
    let actor = super::current_actor();
    let commit = current_head_sha_opt().unwrap_or_default();
    let mut done = Vec::new();
    for (id, fam) in &targets {
        let mut v = Verification::opened(
            format!("[backfilled exemption] {}", reason),
            actor.clone(),
            commit.clone(),
            now,
        );
        v.exempt = true;
        // REQ-0162: record the waiver kind structurally.
        v.exemption_kind = Some(crate::model::ExemptionKind::Backfilled);
        v.statement = Some(format!("[backfilled: {}]", reason));
        v.verdict = Some(TestOutcome::Pass);
        v.concluded = Some(now);
        v.concluded_commit = Some(commit.clone());
        let it = item_mut(project, id, *fam);
        *it.verification = Some(v);
        *it.updated = now;
        it.history.push(super::history(
            "verification back-filled (audited exemption)",
            Some(reason.to_string()),
        ));
        done.push(id.clone());
    }
    if !done.is_empty() {
        project.updated = now;
    }
    Ok(done)
}

/// REQ-0139: build the audited `--no-dossier` exemption dossier recorded by
/// `req verify --no-dossier --reason ...` (ordinary requirements only).
pub fn exemption_dossier(reason: &str, actor: String, commit: String) -> Verification {
    let now = Utc::now();
    let mut v = Verification::opened(
        format!("[--no-dossier exemption] {}", reason),
        actor,
        commit,
        now,
    );
    v.exempt = true;
    // REQ-0162: record the waiver kind structurally.
    v.exemption_kind = Some(crate::model::ExemptionKind::NoDossier);
    v.statement = Some(format!("[no-dossier exemption: {}]", reason));
    v.verdict = Some(TestOutcome::Pass);
    v.concluded = Some(now);
    v
}

/// REQ-0139 / REQ-0143: the front-line gate for safety requirements. No tag
/// exemption and no audited backfill — only a GENUINE concluded passing
/// dossier (analysis + testing + statement) lets a safety requirement reach
/// Verified. An `exempt` dossier is explicitly rejected here.
pub fn gate_safety_requirement(sr: &crate::model::SafetyRequirement) -> Result<()> {
    if classify(sr.verification.as_ref(), None, &sr.id).is_genuine() {
        return Ok(());
    }
    Err(anyhow!(
        "{} (safety) cannot be promoted to Verified without a GENUINE verification dossier. Run \
         `req verification plan {} ...` → analysis → test → conclude. Safety requirements cannot \
         be tag-exempted or back-filled.",
        sr.id,
        sr.id
    ))
}

// --------------------------------------------------------------------------
// CLI wrappers
// --------------------------------------------------------------------------

fn plan(args: VerificationPlanArgs, file: &Option<PathBuf>) -> Result<()> {
    let (path, mut project, _lock) = load_for_mutation(file)?;
    let id = op_plan(
        &mut project,
        &args.id,
        &args.plan,
        args.reopen,
        args.reason.as_deref(),
    )?;
    let (cid, fam) = resolve(&project, &id)?;
    storage::save(&path, &project)?;
    if args.json {
        emit_json(&project, &cid, fam)?;
    } else {
        println!("Opened verification dossier for {}.", cid);
        println!(
            "Next: `req verification analysis {} --findings \"...\" --result pass|fail`",
            cid
        );
    }
    Ok(())
}

fn activity(args: VerificationActivityArgs, file: &Option<PathBuf>, stage: Stage) -> Result<()> {
    let (path, mut project, _lock) = load_for_mutation(file)?;
    let outcome = match args.result {
        TestResultArg::Pass => TestOutcome::Pass,
        TestResultArg::Fail => TestOutcome::Fail,
    };
    let id = op_activity(
        &mut project,
        &args.id,
        stage,
        &args.findings,
        outcome,
        &args.references,
    )?;
    let (cid, fam) = resolve(&project, &id)?;
    storage::save(&path, &project)?;
    if args.json {
        emit_json(&project, &cid, fam)?;
    } else {
        println!(
            "Recorded verification by {} for {} — {}.",
            stage.label(),
            cid,
            outcome.as_str()
        );
        match stage {
            Stage::Analysis => println!(
                "Next: `req verification test {} --findings \"...\" --result pass|fail`",
                cid
            ),
            Stage::Testing => println!(
                "Next: `req verification conclude {} --statement \"...\" [--promote]`",
                cid
            ),
        }
    }
    Ok(())
}

fn conclude(args: VerificationConcludeArgs, file: &Option<PathBuf>) -> Result<()> {
    let (path, mut project, _lock) = load_for_mutation(file)?;
    let out = op_conclude(
        &mut project,
        &args.id,
        &args.statement,
        args.promote,
        args.force,
        args.reason.as_deref(),
        Path::new("."),
    )?;
    let (_cid, fam) = resolve(&project, &out.id)?;
    storage::save(&path, &project)?;
    if args.json {
        emit_json(&project, &out.id, fam)?;
    } else {
        println!(
            "Concluded verification for {} — verdict {}{}.",
            out.id,
            out.verdict.as_str().to_uppercase(),
            if out.promoted {
                " → Verified".to_string()
            } else if out.awaiting_confirmation {
                format!(
                    " → Implemented, awaiting human co-sign (`req verification confirm {}`)",
                    out.id
                )
            } else {
                String::new()
            }
        );
    }
    Ok(())
}

/// REQ-0145: a human co-signs the verification result. Refuses an agent actor so
/// an agent cannot confirm on a person's behalf; requires a concluded Pass
/// verdict; records the confirmation on the dossier. For a safety requirement
/// this human confirmation is REQUIRED (REQ-V-0034) before the verification
/// counts as passed — the agent's analysis + testing alone do not suffice.
pub fn op_confirm(project: &mut Project, raw: &str, note: &str) -> Result<String> {
    if matches!(super::current_actor_kind(), crate::model::ActorKind::Agent) {
        return Err(anyhow!(
            "confirming a verification result must be done by a human, but REQ_ACTOR_KIND=agent. \
             A person must run `req verification confirm`."
        ));
    }
    let (id, fam) = resolve(project, raw)?;
    // REQ-0187: read-only preconditions before the mutable borrow. The dossier
    // must have a concluded Pass verdict to confirm.
    {
        let v = dossier(project, &id, fam).ok_or_else(|| {
            anyhow!(
                "{} has no verification dossier to confirm — run `req verification plan {} ...` first.",
                id,
                id
            )
        })?;
        if !matches!(v.verdict, Some(TestOutcome::Pass)) {
            return Err(anyhow!(
                "{} has no concluded Pass verdict to confirm — record analysis, testing, and \
                 `req verification conclude` first.",
                id
            ));
        }
    }
    // REQ-0187: for a safety requirement, the co-sign is the act that promotes
    // to Verified, so re-apply the same status-ladder + SIL-rigour preflight
    // conclude used. Carry forward an audited SIL-gate exception so a forced
    // conclude can still be confirmed.
    if matches!(fam, Family::Sr) {
        let had_exception = project.safety_requirements[&id]
            .tests
            .iter()
            .any(|t| matches!(t.outcome, TestOutcome::Pass) && t.sil_gate_exception);
        promote_preflight(project, &id, fam, had_exception)?;
    } else if matches!(fam, Family::Sf) {
        // REQ-0201: a safety function's co-sign is also the act that promotes it
        // to Verified, so re-apply the status-ladder preflight (no SIL-evidence
        // gate — an SF has no test-evidence model).
        promote_preflight(project, &id, fam, false)?;
    }
    let now = Utc::now();
    let actor = super::current_actor();
    {
        let mut it = item_mut(project, &id, fam);
        let v = it.verification.as_mut().unwrap();
        v.human_confirmation = Some(VerificationActivity {
            summary: if note.is_empty() {
                "human confirmation of the verification result".to_string()
            } else {
                note.to_string()
            },
            outcome: TestOutcome::Pass,
            references: Vec::new(),
            at: now,
            actor: actor.clone(),
        });
        // REQ-0187/REQ-0201: the human co-sign promotes a safety requirement OR
        // a safety function to Verified. An ordinary requirement is already
        // Verified, so the co-sign only records the human confirmation.
        let promotes = matches!(fam, Family::Sr | Family::Sf);
        if promotes {
            it.status.set_verified();
        }
        *it.updated = now;
        it.history.push(super::history(
            if promotes {
                "verification result confirmed by human — promoted to Verified"
            } else {
                "verification result confirmed by human"
            },
            None,
        ));
    }
    project.updated = now;
    Ok(id)
}

fn confirm(args: VerificationConfirmArgs, file: &Option<PathBuf>) -> Result<()> {
    let (path, mut project, _lock) = load_for_mutation(file)?;
    let id = op_confirm(&mut project, &args.id, &args.note)?;
    let (_cid, fam) = resolve(&project, &id)?;
    storage::save(&path, &project)?;
    if args.json {
        emit_json(&project, &id, fam)?;
    } else {
        println!("Confirmed verification result for {id} — human co-sign recorded.");
    }
    Ok(())
}

fn backfill(args: VerificationBackfillArgs, file: &Option<PathBuf>) -> Result<()> {
    let (path, mut project, _lock) = load_for_mutation(file)?;
    let done = op_backfill(&mut project, args.id.as_deref(), args.all, &args.reason)?;
    if !done.is_empty() {
        storage::save(&path, &project)?;
    }
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({ "backfilled": done }))?
        );
    } else if done.is_empty() {
        println!("Nothing to back-fill — every Verified item already has a passing dossier.");
    } else {
        println!("Back-filled {} item(s): {}", done.len(), done.join(", "));
    }
    Ok(())
}

/// REQ-0153: outcome of `req verification refresh-anchors`.
pub struct RefreshReport {
    /// Ordinary requirements whose source was proven byte-identical to its
    /// anchor and whose content hash was re-normalized in place.
    pub refreshed: Vec<String>,
    /// Verified requirements whose source genuinely changed since the anchor —
    /// left stale; they need real re-verification.
    pub drifted: Vec<String>,
    /// Verified safety requirements that are stale under the new hash. Never
    /// auto-refreshed: REQ-0148/REQ-0145 require a human re-anchor + co-sign.
    pub safety_pending: Vec<String>,
}

/// REQ-0153: re-normalize the staleness anchors that the REQ-0152 hash change
/// invalidated, but ONLY where it is provably safe. For each Verified ordinary
/// requirement with a content hash:
///   - if the NEW hash already matches, it is fresh — skip;
///   - else if the LEGACY hash of the current source still equals the stored
///     hash, the source bytes are byte-identical to the anchor (the change was
///     purely the hash format), so re-store the new normalized hash — no
///     re-verification, the verification still stands;
///   - else the source genuinely drifted — leave it stale for re-verification.
///
/// Safety requirements are never touched here; they are reported as pending a
/// human re-anchor + co-sign.
pub fn op_refresh_anchors(project: &mut Project, root: &Path) -> RefreshReport {
    use crate::commands::test_cmd::{auto_linked_files, hash_files, hash_files_legacy};

    let mut safety_pending: Vec<String> = project
        .safety_requirements
        .iter()
        .filter(|(_, sr)| matches!(sr.status, Status::Verified))
        .filter_map(|(id, sr)| {
            let v = sr.verification.as_ref()?;
            let stored = v.content_hash.as_deref()?;
            let linked: Vec<PathBuf> = match &v.linked_files {
                Some(l) => l.iter().map(PathBuf::from).collect(),
                None => auto_linked_files(id, root),
            };
            // Only those actually stale under the new algorithm are pending.
            (hash_files(&linked) != stored).then(|| id.clone())
        })
        .collect();
    safety_pending.sort();

    let mut refreshed = Vec::new();
    let mut drifted = Vec::new();
    let ids: Vec<String> = project.requirements.keys().cloned().collect();
    for id in ids {
        let r = &project.requirements[&id];
        if !matches!(r.status, Status::Verified) {
            continue;
        }
        let Some(v) = &r.verification else { continue };
        let Some(stored) = v.content_hash.clone() else {
            continue;
        };
        let linked: Vec<PathBuf> = match &v.linked_files {
            Some(l) => l.iter().map(PathBuf::from).collect(),
            None => auto_linked_files(&id, root),
        };
        let new = hash_files(&linked);
        if new == stored {
            continue; // already fresh under the new algorithm
        }
        if hash_files_legacy(&linked) == stored {
            // Proven unchanged: re-store the normalized hash + forward-slash paths.
            let normalized: Vec<String> = linked
                .iter()
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .collect();
            let v = project
                .requirements
                .get_mut(&id)
                .unwrap()
                .verification
                .as_mut()
                .unwrap();
            v.content_hash = Some(new);
            v.linked_files = Some(normalized);
            refreshed.push(id);
        } else {
            drifted.push(id);
        }
    }
    refreshed.sort();
    drifted.sort();
    RefreshReport {
        refreshed,
        drifted,
        safety_pending,
    }
}

fn refresh_anchors(args: VerificationRefreshArgs, file: &Option<PathBuf>) -> Result<()> {
    let (path, mut project, _lock) = load_for_mutation(file)?;
    let report = op_refresh_anchors(&mut project, Path::new(&args.path));
    let changed = !report.refreshed.is_empty();
    if changed && !args.dry_run {
        project.updated = Utc::now();
        storage::save(&path, &project)?;
    }
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "refreshed": report.refreshed,
                "drifted": report.drifted,
                "safety_pending": report.safety_pending,
                "dry_run": args.dry_run,
            }))?
        );
        return Ok(());
    }
    println!(
        "{} anchor(s) {} (source proven unchanged since the REQ-0152 hash change).",
        report.refreshed.len(),
        if args.dry_run {
            "would be refreshed"
        } else {
            "refreshed"
        }
    );
    if !report.drifted.is_empty() {
        println!(
            "\n{} requirement(s) genuinely drifted — re-verify (plan --reopen → … → conclude --promote):",
            report.drifted.len()
        );
        for id in &report.drifted {
            println!("  {id}");
        }
    }
    if !report.safety_pending.is_empty() {
        println!(
            "\n{} safety requirement(s) need a human re-anchor + co-sign (never auto-refreshed):",
            report.safety_pending.len()
        );
        for id in &report.safety_pending {
            println!("  {id}");
        }
    }
    Ok(())
}

/// REQ-0200: is this dossier stale against the current source? (genuine,
/// anchored, and the anchored content changed). Exempt/anchorless dossiers
/// are not "stale" in this sense.
fn dossier_is_stale(v: Option<&Verification>, id: &str, root: &Path) -> bool {
    let Some(v) = v else { return false };
    if v.exempt {
        return false;
    }
    let Some(stored) = v.content_hash.as_deref() else {
        return false;
    };
    matches!(
        crate::commands::test_cmd::staleness_by_content(stored, v.linked_files.as_ref(), id, root),
        crate::commands::test_cmd::Staleness::Stale { .. }
    )
}

// REQ-0200: re-anchor stale ordinary requirements whose automated tests pass.
// A passing acceptance test at HEAD is objective evidence the requirement is
// still met, so it re-anchors the dossier cheaply and honestly without a model
// re-review. Safety requirements are never touched here (reported only); items
// with no matching test or any failing test are left stale and reported.
fn reverify(args: VerificationReverifyArgs, file: &Option<PathBuf>) -> Result<()> {
    if !args.by_tests {
        return Err(anyhow!(
            "reverify currently supports only --by-tests; pass --by-tests"
        ));
    }
    // Collect test results once (run the suite or ingest a captured log).
    let (results, _ok) = crate::commands::test_cmd::collect_results(
        &args.cmd,
        args.from_file.as_deref(),
        args.map_file.as_deref(),
    )?;

    let (path, mut project, _lock) = load_for_mutation(file)?;
    let root = args.path.clone();
    let commit = current_head_sha_opt().unwrap_or_default();

    // Candidate ordinary requirements: Verified + stale.
    let mut ordinary: Vec<String> = project
        .requirements
        .iter()
        .filter(|(_, r)| matches!(r.status, Status::Verified))
        .filter(|(id, r)| dossier_is_stale(r.verification.as_ref(), id, &root))
        .map(|(id, _)| id.clone())
        .collect();
    ordinary.sort();

    // Stale safety requirements are reported only (REQ-0200: never re-anchored
    // here — they need the SIL-adequate evidence + human co-sign path).
    let mut sr_skipped: Vec<String> = project
        .safety_requirements
        .iter()
        .filter(|(id, sr)| dossier_is_stale(sr.verification.as_ref(), id, &root))
        .map(|(id, _)| id.clone())
        .collect();
    sr_skipped.sort();

    let mut reanchored: Vec<String> = Vec::new();
    let mut no_tests: Vec<String> = Vec::new();
    let mut failing: Vec<String> = Vec::new();

    for id in &ordinary {
        match results.get(id) {
            None => no_tests.push(id.clone()),
            Some(r) if r.passed.is_empty() => no_tests.push(id.clone()),
            Some(r) if !r.failed.is_empty() => failing.push(id.clone()),
            Some(r) => {
                if args.dry_run {
                    reanchored.push(id.clone());
                    continue;
                }
                // Re-anchor: reopen → analysis(pass) → testing(pass) → conclude.
                let names = r.passed.join(", ");
                op_plan(
                    &mut project,
                    id,
                    "Re-anchor a behaviour-preserving drift: re-confirm the requirement against current source via its passing acceptance tests.",
                    true,
                    Some("REQ-0200 reverify --by-tests: anchored source drifted, behaviour unchanged"),
                )?;
                op_activity(
                    &mut project,
                    id,
                    Stage::Analysis,
                    "Behaviour unchanged since the prior genuine verification; re-confirmed by the requirement's passing acceptance tests at HEAD (evidence is the test run, not a fresh code review).",
                    TestOutcome::Pass,
                    &[],
                )?;
                op_activity(
                    &mut project,
                    id,
                    Stage::Testing,
                    &format!("cargo test: {} pass / 0 fail — {}", r.passed.len(), names),
                    TestOutcome::Pass,
                    &r.passed,
                )?;
                op_conclude(
                    &mut project,
                    id,
                    &format!(
                        "Re-anchored at {} on passing automated tests ({}); evidence is the test run at the current commit, not a fresh code review.",
                        short(&commit),
                        names
                    ),
                    true,
                    false,
                    None,
                    &root,
                )?;
                reanchored.push(id.clone());
            }
        }
    }

    if !reanchored.is_empty() && !args.dry_run {
        project.updated = Utc::now();
        storage::save(&path, &project)?;
    }

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "reanchored": reanchored,
                "no_tests": no_tests,
                "failing": failing,
                "safety_skipped": sr_skipped,
                "dry_run": args.dry_run,
            }))?
        );
        return Ok(());
    }

    println!(
        "{} stale ordinary requirement(s) {} from passing tests.",
        reanchored.len(),
        if args.dry_run {
            "would be re-anchored"
        } else {
            "re-anchored to genuine"
        }
    );
    if !no_tests.is_empty() {
        println!(
            "\n{} left stale — no matching passing test (need analysis path or a test):",
            no_tests.len()
        );
        for id in &no_tests {
            println!("  {id}");
        }
    }
    if !failing.is_empty() {
        println!(
            "\n{} left stale — a test is FAILING (investigate, do not accept):",
            failing.len()
        );
        for id in &failing {
            println!("  {id}");
        }
    }
    if !sr_skipped.is_empty() {
        println!(
            "\n{} stale safety requirement(s) NOT touched (need SIL evidence + human co-sign):",
            sr_skipped.len()
        );
        for id in &sr_skipped {
            println!("  {id}");
        }
    }
    Ok(())
}

// REQ-0142: the true-status report. Classifies every Verified item and
// rolls up the counts, so the headline "verified" number can be read with
// its provenance instead of taken at face value.
/// REQ-0185: classify a not-yet-passing item by how far its verification
/// dossier progressed, so the report shows the unverified surface instead
/// of only the verified one. Returns `None` for Verified/Obsolete items
/// (Verified ones are reported by provenance; Obsolete are out of scope).
fn unverified_stage(
    status: Status,
    v: Option<&crate::model::Verification>,
) -> Option<&'static str> {
    use crate::model::TestOutcome;
    if matches!(status, Status::Verified | Status::Obsolete) {
        return None;
    }
    Some(match v {
        None => "no-plan",
        Some(v) if v.plan.trim().is_empty() => "no-plan",
        Some(v) => match (&v.analysis, &v.testing) {
            (None, _) => "plan-only",
            (Some(a), _) if matches!(a.outcome, TestOutcome::Fail) => "analysis-failing",
            (Some(_), None) => "analysed-untested",
            (Some(_), Some(t)) if matches!(t.outcome, TestOutcome::Fail) => "tested-failing",
            (Some(_), Some(_)) => "ready-to-conclude",
        },
    })
}

/// REQ-0185: the ordered pipeline stages, worst-progressed first.
const UNVERIFIED_STAGES: &[&str] = &[
    "no-plan",
    "plan-only",
    "analysis-failing",
    "analysed-untested",
    "tested-failing",
    "ready-to-conclude",
];

/// REQ-0185: collect `(id, family, stage)` for every requirement and safety
/// requirement that has not reached a passing verification.
fn unverified_rows(project: &Project) -> Vec<(String, &'static str, &'static str)> {
    let mut rows = Vec::new();
    for (id, r) in &project.requirements {
        if let Some(stage) = unverified_stage(r.status, r.verification.as_ref()) {
            rows.push((id.clone(), "requirement", stage));
        }
    }
    for (id, sr) in &project.safety_requirements {
        if let Some(stage) = unverified_stage(sr.status, sr.verification.as_ref()) {
            rows.push((id.clone(), "safety-requirement", stage));
        }
    }
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    rows
}

fn report(args: VerificationReportArgs, file: &Option<PathBuf>) -> Result<()> {
    let (_path, project) = load_resolved(file)?;
    let rows = provenance_report(&project, Some(&args.path));
    // REQ-0185: the unverified surface, grouped by dossier-pipeline stage.
    let unverified = unverified_rows(&project);
    // REQ-0188: every safety requirement with its standing, none omitted.
    let mut sr_standings: Vec<(String, &'static str)> = project
        .safety_requirements
        .values()
        .map(|sr| {
            (
                sr.id.clone(),
                crate::commands::provenance::sr_standing(sr, Some(&args.path)),
            )
        })
        .collect();
    sr_standings.sort_by(|a, b| a.0.cmp(&b.0));

    let mut genuine = 0usize;
    let mut backfilled = 0usize;
    let mut no_dossier = 0usize;
    let mut stale = 0usize;
    // REQ-0150: count the unconfirmed (genuine dossier, no human co-sign) bucket.
    let mut unconfirmed = 0usize;
    let mut ungated = 0usize;
    for r in &rows {
        match r.provenance {
            Provenance::Genuine => genuine += 1,
            Provenance::ExemptBackfilled => backfilled += 1,
            Provenance::ExemptNoDossier => no_dossier += 1,
            Provenance::Stale => stale += 1,
            Provenance::Unconfirmed => unconfirmed += 1,
            Provenance::Ungated => ungated += 1,
        }
    }
    let total = rows.len();
    let shown: Vec<&ProvenanceRow> = rows
        .iter()
        .filter(|r| !args.not_genuine || !r.provenance.is_genuine())
        .collect();

    if args.json {
        let items: Vec<_> = shown
            .iter()
            .map(|r| {
                serde_json::json!({
                    "id": r.id,
                    "family": r.family,
                    "provenance": r.provenance.as_str(),
                    "genuine": r.provenance.is_genuine(),
                    "sil": r.sil,
                })
            })
            .collect();
        // REQ-0185: per-stage counts for the unverified surface.
        let mut unval_by_stage = serde_json::Map::new();
        for stage in UNVERIFIED_STAGES {
            let n = unverified.iter().filter(|(_, _, s)| s == stage).count();
            if n > 0 {
                unval_by_stage.insert((*stage).to_string(), serde_json::json!(n));
            }
        }
        let unval_items: Vec<_> = unverified
            .iter()
            .map(|(id, fam, stage)| serde_json::json!({ "id": id, "family": fam, "stage": stage }))
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "verified_total": total,
                "counts": {
                    "genuine": genuine,
                    "exempt_backfilled": backfilled,
                    "exempt_no_dossier": no_dossier,
                    "stale": stale,
                    // REQ-0150: expose the unconfirmed-SR count in JSON too.
                    "unconfirmed": unconfirmed,
                    "ungated": ungated,
                },
                "items": items,
                // REQ-0185: the unverified surface alongside the verified one.
                "unverified_total": unverified.len(),
                "unverified_by_stage": unval_by_stage,
                "unverified": unval_items,
                // REQ-0188: every safety requirement with its standing.
                "safety_requirements": sr_standings.iter().map(|(id, s)| {
                    serde_json::json!({ "id": id, "standing": s })
                }).collect::<Vec<_>>(),
            }))?
        );
        return Ok(());
    }

    println!("Verification provenance ({} verified item(s))", total);
    println!(
        "  genuine          : {:>4}   (concluded Pass dossier: analysis + testing + statement)",
        genuine
    );
    println!(
        "  exempt:backfilled: {:>4}   (grandfathered via `req verification backfill`)",
        backfilled
    );
    println!(
        "  exempt:no-dossier: {:>4}   (`req verify --no-dossier` waiver)",
        no_dossier
    );
    println!(
        "  stale            : {:>4}   (genuine dossier whose anchored source drifted)",
        stale
    );
    // REQ-0150: surface the unconfirmed safety-requirement bucket in the report.
    println!(
        "  unconfirmed      : {:>4}   (safety req: genuine dossier, no human co-sign — REQ-0145)",
        unconfirmed
    );
    println!(
        "  ungated          : {:>4}   (Verified with no passing dossier)",
        ungated
    );

    // REQ-0185: the unverified surface — everything that has NOT reached a
    // passing verification, grouped by how far its dossier progressed. Without
    // this the report shows only verified items and hides what work remains.
    println!();
    println!(
        "Unverified ({} item(s) with no passing dossier)",
        unverified.len()
    );
    for stage in UNVERIFIED_STAGES {
        let n = unverified.iter().filter(|(_, _, s)| s == stage).count();
        if n > 0 {
            println!("  {:<18}: {:>4}", stage, n);
        }
    }
    if !unverified.is_empty() {
        println!();
        for (id, fam, stage) in &unverified {
            println!("  {:<9}  {:<18}  {}", id, stage, fam);
        }
    }

    // REQ-0188: every safety requirement and its standing — never omitted by a
    // status filter, never silently counted as done. The awaiting-cosign state
    // is its own distinct category.
    if !sr_standings.is_empty() {
        println!();
        let awaiting = sr_standings
            .iter()
            .filter(|(_, s)| *s == "awaiting-cosign")
            .count();
        println!(
            "Safety requirements ({} total, {} awaiting human co-sign)",
            sr_standings.len(),
            awaiting
        );
        for (id, standing) in &sr_standings {
            println!("  {:<9}  {}", id, standing);
        }
    }

    let not_genuine = total - genuine;
    if not_genuine > 0 {
        println!();
        println!(
            "⚠ {} of {} verified item(s) do NOT rest on a genuine verification dossier.",
            not_genuine, total
        );
    }
    if shown.is_empty() {
        if args.not_genuine {
            println!("\nEvery verified item rests on a genuine dossier.");
        }
        return Ok(());
    }
    println!();
    for r in &shown {
        if r.provenance.is_genuine() && args.not_genuine {
            continue;
        }
        let sil = r
            .sil
            .as_deref()
            .map(|s| format!("  [{}]", s))
            .unwrap_or_default();
        println!(
            "  {:<9}  {:<18}  {}{}",
            r.id,
            r.provenance.as_str(),
            r.family,
            sil
        );
    }
    Ok(())
}

fn show(args: VerificationShowArgs, file: &Option<PathBuf>) -> Result<()> {
    let (_path, project) = load_resolved(file)?;
    let (id, fam) = resolve(&project, &args.id)?;
    if args.json {
        return emit_json(&project, &id, fam);
    }
    println!("{}  {}", id, title_of(&project, &id, fam));
    match dossier(&project, &id, fam) {
        None => println!(
            "  (no verification dossier — run `req verification plan {} ...`)",
            id
        ),
        Some(v) => {
            println!("  plan:       {}", v.plan);
            print_activity("analysis", v.analysis.as_ref());
            print_activity("testing", v.testing.as_ref());
            match &v.statement {
                Some(s) => println!("  statement:  {}", s),
                None => println!("  statement:  (pending)"),
            }
            match v.verdict {
                Some(o) => println!(
                    "  verdict:    {}{}",
                    o.as_str().to_uppercase(),
                    if v.exempt {
                        "  (audited exemption)"
                    } else {
                        ""
                    }
                ),
                None => println!("  verdict:    (not concluded)"),
            }
            if let Some(h) = &v.content_hash {
                println!(
                    "  anchored:   {} @ {}",
                    &h[..h.len().min(12)],
                    v.concluded_commit.as_deref().map(short).unwrap_or_default()
                );
            }
        }
    }
    Ok(())
}

fn print_activity(label: &str, a: Option<&VerificationActivity>) {
    match a {
        None => println!("  {:<9}: (pending)", label),
        Some(a) => {
            println!(
                "  {:<9}: {} — {}",
                label,
                a.outcome.as_str().to_uppercase(),
                a.summary
            );
            for r in &a.references {
                println!("              · {}", r);
            }
        }
    }
}

fn emit_json(project: &Project, id: &str, fam: Family) -> Result<()> {
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "id": id,
            "verification": dossier(project, id, fam),
        }))?
    );
    Ok(())
}
