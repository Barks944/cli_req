// REQ-0139: the staged validation dossier.
//
// A requirement (REQ-NNNN) or safety requirement (SR-NNNN) reaches Verified
// only after walking an ordered validation: plan → analysis → testing →
// statement → verdict. The verdict is derived (Pass only when both the
// analysis and the testing stage pass), never free-typed, and a passing
// dossier is the precondition the promotion gate checks before any status
// flips to Verified. The dossier anchors a content hash of the linked source
// at conclude time, so a later code change drifts it STALE (via `req stale`)
// and the verification no longer stands.
//
// This module works on both id families by branching on the id prefix, the
// same shape as `req trace` — there is one `req validation` surface. The
// `op_*` functions are the IO-free core (mutate a &mut Project) shared by
// the CLI wrappers below and the MCP tools in src/mcp.rs.
use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use std::path::{Path, PathBuf};

use crate::cli::{
    TestResultArg, ValidationActivityArgs, ValidationBackfillArgs, ValidationCmd,
    ValidationConcludeArgs, ValidationPlanArgs, ValidationShowArgs,
};
use crate::commands::test_cmd::{auto_linked_files, current_head_sha_opt, hash_files, short};
use crate::model::{
    EvidenceKind, HistoryEntry, Project, Sil, Status, TestOutcome, TestRecord, Validation,
    ValidationActivity,
};
use crate::storage::{self, load_for_mutation, load_resolved};

// --------------------------------------------------------------------------
// shared types
// --------------------------------------------------------------------------

#[derive(Copy, Clone)]
pub enum Family {
    Req,
    Sr,
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
}

// --------------------------------------------------------------------------
// CLI dispatch
// --------------------------------------------------------------------------

pub fn run(cmd: ValidationCmd, file: &Option<PathBuf>) -> Result<()> {
    match cmd {
        ValidationCmd::Plan(a) => plan(a, file),
        ValidationCmd::Analysis(a) => activity(a, file, Stage::Analysis),
        ValidationCmd::Test(a) => activity(a, file, Stage::Testing),
        ValidationCmd::Conclude(a) => conclude(a, file),
        ValidationCmd::Show(a) => show(a, file),
        ValidationCmd::Backfill(a) => backfill(a, file),
    }
}

// --------------------------------------------------------------------------
// id resolution + field access
// --------------------------------------------------------------------------

fn normalize_sr(raw: &str) -> String {
    let trimmed = raw.trim();
    let upper = trimmed.to_uppercase();
    let digits = if let Some(rest) = upper.strip_prefix("SR-") {
        rest.to_string()
    } else if trimmed.chars().all(|c| c.is_ascii_digit()) && !trimmed.is_empty() {
        trimmed.to_string()
    } else {
        return upper;
    };
    match digits.parse::<u32>() {
        Ok(n) => format!("SR-{:04}", n),
        Err(_) => upper,
    }
}

/// Resolve a raw id to its canonical form and family. SR-prefixed ids route
/// to the safety-requirements map; everything else is an ordinary requirement.
pub fn resolve(project: &Project, raw: &str) -> Result<(String, Family)> {
    if raw.trim().to_uppercase().starts_with("SR") {
        let id = normalize_sr(raw);
        if project.safety_requirements.contains_key(&id) {
            Ok((id, Family::Sr))
        } else {
            Err(anyhow!("no such safety requirement: {}", raw))
        }
    } else {
        let id = super::resolve_id(project, raw)?;
        Ok((id, Family::Req))
    }
}

/// Disjoint mutable handles to the dossier-bearing fields shared by
/// `Requirement` and `SafetyRequirement`.
struct ItemMut<'a> {
    validation: &'a mut Option<Validation>,
    status: &'a mut Status,
    history: &'a mut Vec<HistoryEntry>,
    updated: &'a mut DateTime<Utc>,
    tests: &'a mut Vec<TestRecord>,
}

fn item_mut<'a>(project: &'a mut Project, id: &str, fam: Family) -> ItemMut<'a> {
    match fam {
        Family::Req => {
            let r = project.requirements.get_mut(id).unwrap();
            ItemMut {
                validation: &mut r.validation,
                status: &mut r.status,
                history: &mut r.history,
                updated: &mut r.updated,
                tests: &mut r.tests,
            }
        }
        Family::Sr => {
            let sr = project.safety_requirements.get_mut(id).unwrap();
            ItemMut {
                validation: &mut sr.validation,
                status: &mut sr.status,
                history: &mut sr.history,
                updated: &mut sr.updated,
                tests: &mut sr.tests,
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
    };
    tests.iter().any(|t| {
        matches!(t.outcome, TestOutcome::Pass)
            && matches!(t.kind, EvidenceKind::Automated | EvidenceKind::Composition)
    })
}

pub fn dossier<'a>(project: &'a Project, id: &str, fam: Family) -> Option<&'a Validation> {
    match fam {
        Family::Req => project.requirements[id].validation.as_ref(),
        Family::Sr => project.safety_requirements[id].validation.as_ref(),
    }
}

fn current_status(project: &Project, id: &str, fam: Family) -> Status {
    match fam {
        Family::Req => project.requirements[id].status,
        Family::Sr => project.safety_requirements[id].status,
    }
}

fn title_of(project: &Project, id: &str, fam: Family) -> String {
    match fam {
        Family::Req => project.requirements[id].title.clone(),
        Family::Sr => project.safety_requirements[id].title.clone(),
    }
}

fn test_summaries(project: &Project, id: &str, fam: Family) -> Vec<String> {
    let tests = match fam {
        Family::Req => &project.requirements[id].tests,
        Family::Sr => &project.safety_requirements[id].tests,
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
        if let Some(v) = it.validation.as_ref() {
            if v.is_concluded() && !reopen {
                return Err(anyhow!(
                    "{} already has a concluded validation dossier — pass --reopen --reason \"...\" \
                     to re-validate (this clears the prior verdict).",
                    id
                ));
            }
        }
        *it.validation = Some(Validation::opened(plan.to_string(), actor, commit, now));
        *it.updated = now;
        it.history.push(super::history(
            if reopen {
                "validation re-opened (plan recorded)"
            } else {
                "validation plan recorded"
            },
            reason.map(|s| s.to_string()),
        ));
    }
    project.updated = now;
    Ok(id)
}

/// Stages 2 & 3 — record validation by analysis / by testing.
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
    let entry = ValidationActivity {
        summary: findings.to_string(),
        outcome,
        references: refs,
        at: now,
        actor,
    };
    {
        let it = item_mut(project, &id, fam);
        let v = it.validation.as_mut().ok_or_else(|| {
            anyhow!(
                "{} has no validation dossier — run `req validation plan {} ...` first",
                id,
                id
            )
        })?;
        if v.is_concluded() {
            return Err(anyhow!(
                "{}'s dossier is already concluded — re-open it with `req validation plan {} --reopen --reason \"...\"` to revise.",
                id, id
            ));
        }
        match stage {
            Stage::Analysis => v.analysis = Some(entry),
            Stage::Testing => {
                if v.analysis.is_none() {
                    return Err(anyhow!(
                        "record validation by analysis before testing — run `req validation analysis {} ...` first",
                        id
                    ));
                }
                v.testing = Some(entry);
            }
        }
        *it.updated = now;
        it.history.push(super::history(
            format!(
                "validation {} recorded ({})",
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
                "{} has no validation dossier — run `req validation plan {} ...` first",
                id,
                id
            )
        })?;
        if v.analysis.is_none() || v.testing.is_none() {
            return Err(anyhow!(
                "{} cannot be concluded — record validation by analysis AND by testing first.",
                id
            ));
        }
        v.derive_verdict().unwrap_or(TestOutcome::Fail)
    };
    if promote {
        if matches!(verdict, TestOutcome::Fail) {
            return Err(anyhow!(
                "{}'s validation verdict is FAIL — cannot promote a failed validation to Verified. \
                 Fix the issue, then `req validation plan {} --reopen --reason \"...\"` and re-validate.",
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
                .map(|p| p.to_string_lossy().to_string())
                .collect(),
        )
    };

    // When promoting on a Pass, the conclusion IS the verification evidence —
    // record a TestRecord so the dossier and the evidence model agree (and
    // REQ-V-0030 sees a Verified safety requirement's passing evidence). The
    // evidence kind composes the dossier's existing strong evidence when
    // present, else it is an inspection-grade conclusion.
    let will_record = promote && matches!(verdict, TestOutcome::Pass);
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
    {
        let it = item_mut(project, &id, fam);
        {
            let v = it.validation.as_mut().unwrap();
            v.statement = Some(statement.to_string());
            v.verdict = Some(verdict);
            v.concluded = Some(now);
            v.concluded_commit = Some(commit.clone());
            v.content_hash = content_hash.clone();
            v.linked_files = linked_files.clone();
        }
        if will_record {
            *it.status = Status::Verified;
            promoted = true;
            it.tests.push(TestRecord {
                at: now,
                actor: super::current_actor(),
                commit: commit.clone(),
                outcome: TestOutcome::Pass,
                notes: format!("validation dossier concluded — {}", statement),
                kind: evidence_kind,
                content_hash,
                linked_files,
                sil_gate_exception,
            });
        }
        *it.updated = now;
        it.history.push(super::history(
            format!(
                "validation concluded ({}){}",
                verdict.as_str(),
                if promoted {
                    " — promoted to Verified"
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
    })
}

/// Read-only promotion checks: status ladder + (for SRs) the SIL-rigour
/// gate, mirroring `req verify` / `req sreq verify`.
fn promote_preflight(project: &Project, id: &str, fam: Family, force: bool) -> Result<()> {
    let status = current_status(project, id, fam);
    let ladder_ok = matches!(status, Status::Implemented | Status::Verified);
    if !ladder_ok && !force {
        return Err(anyhow!(
            "{} is {} — promoting straight to Verified is irregular. Advance it to Implemented \
             first, or pass --force --reason \"...\".",
            id,
            status.as_str()
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
        targets.push(resolve(project, raw)?);
    } else if all {
        for (id, r) in &project.requirements {
            if matches!(r.status, Status::Verified)
                && !r.validation.as_ref().map(|v| v.passed()).unwrap_or(false)
            {
                targets.push((id.clone(), Family::Req));
            }
        }
        for (id, sr) in &project.safety_requirements {
            if matches!(sr.status, Status::Verified)
                && !sr.validation.as_ref().map(|v| v.passed()).unwrap_or(false)
            {
                targets.push((id.clone(), Family::Sr));
            }
        }
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
        let mut v = Validation::opened(
            format!("[backfilled exemption] {}", reason),
            actor.clone(),
            commit.clone(),
            now,
        );
        v.exempt = true;
        v.statement = Some(format!("[backfilled: {}]", reason));
        v.verdict = Some(TestOutcome::Pass);
        v.concluded = Some(now);
        v.concluded_commit = Some(commit.clone());
        let it = item_mut(project, id, *fam);
        *it.validation = Some(v);
        *it.updated = now;
        it.history.push(super::history(
            "validation back-filled (audited exemption)",
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
pub fn exemption_dossier(reason: &str, actor: String, commit: String) -> Validation {
    let now = Utc::now();
    let mut v = Validation::opened(
        format!("[--no-dossier exemption] {}", reason),
        actor,
        commit,
        now,
    );
    v.exempt = true;
    v.statement = Some(format!("[no-dossier exemption: {}]", reason));
    v.verdict = Some(TestOutcome::Pass);
    v.concluded = Some(now);
    v
}

/// REQ-0139: the front-line gate for safety requirements. No tag exemption
/// — only a passing dossier (or an audited back-filled exemption).
pub fn gate_safety_requirement(sr: &crate::model::SafetyRequirement) -> Result<()> {
    if sr.validation.as_ref().map(|v| v.passed()).unwrap_or(false) {
        return Ok(());
    }
    Err(anyhow!(
        "{} (safety) cannot be promoted to Verified without a passing validation dossier. Run \
         `req validation plan {} ...` → analysis → test → conclude. Safety requirements cannot \
         be tag-exempted.",
        sr.id,
        sr.id
    ))
}

// --------------------------------------------------------------------------
// CLI wrappers
// --------------------------------------------------------------------------

fn plan(args: ValidationPlanArgs, file: &Option<PathBuf>) -> Result<()> {
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
        println!("Opened validation dossier for {}.", cid);
        println!(
            "Next: `req validation analysis {} --findings \"...\" --result pass|fail`",
            cid
        );
    }
    Ok(())
}

fn activity(args: ValidationActivityArgs, file: &Option<PathBuf>, stage: Stage) -> Result<()> {
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
            "Recorded validation by {} for {} — {}.",
            stage.label(),
            cid,
            outcome.as_str()
        );
        match stage {
            Stage::Analysis => println!(
                "Next: `req validation test {} --findings \"...\" --result pass|fail`",
                cid
            ),
            Stage::Testing => println!(
                "Next: `req validation conclude {} --statement \"...\" [--promote]`",
                cid
            ),
        }
    }
    Ok(())
}

fn conclude(args: ValidationConcludeArgs, file: &Option<PathBuf>) -> Result<()> {
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
            "Concluded validation for {} — verdict {}{}.",
            out.id,
            out.verdict.as_str().to_uppercase(),
            if out.promoted { " → Verified" } else { "" }
        );
    }
    Ok(())
}

fn backfill(args: ValidationBackfillArgs, file: &Option<PathBuf>) -> Result<()> {
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

fn show(args: ValidationShowArgs, file: &Option<PathBuf>) -> Result<()> {
    let (_path, project) = load_resolved(file)?;
    let (id, fam) = resolve(&project, &args.id)?;
    if args.json {
        return emit_json(&project, &id, fam);
    }
    println!("{}  {}", id, title_of(&project, &id, fam));
    match dossier(&project, &id, fam) {
        None => println!(
            "  (no validation dossier — run `req validation plan {} ...`)",
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

fn print_activity(label: &str, a: Option<&ValidationActivity>) {
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
            "validation": dossier(project, id, fam),
        }))?
    );
    Ok(())
}
