// REQ-0175..0184: external test-system integration.
//
// Two halves of one versioned contract with an external test orchestrator
// (e.g. the AugTronic at_test bench):
//   • EXPORT a machine-readable list of the requirements due for verification
//     (REQ-0175/0176), so the bench knows what the spec expects of it.
//   • INGEST the results it produces back into project.req as test records
//     (REQ-0177..0180), mapping its verdict vocabulary onto the local model
//     (REQ-0182), populating a verification dossier from its per-requirement
//     decision (REQ-0183), and never auto-promoting a safety requirement on
//     external evidence alone (REQ-0184).
//
// `req test pull` (REQ-0181) is the thin transport: an authenticated HTTP GET
// of a result payload that feeds the same ingest core a local file would.
use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::cli::{TestIngestArgs, TestPullArgs, TestRequestsArgs};
use crate::model::{
    EvidenceKind, ExternalSource, Project, Status, TestOutcome, TestRecord, Verification,
    VerificationActivity,
};
use crate::storage::{self, load_for_mutation, load_resolved};

/// REQ-0176: the versioned schema tags. An ingested payload declaring an
/// unsupported version is rejected rather than mis-read.
pub const REQUEST_SCHEMA: &str = "req-test-request-v1";
pub const RESULT_SCHEMA: &str = "req-test-result-v1";

// --------------------------------------------------------------------------
// payloads (the published contract)
// --------------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
pub struct RequestPayload {
    pub schema: String,
    pub commit: String,
    pub requirements: Vec<RequestItem>,
}

#[derive(Serialize, Deserialize)]
pub struct RequestItem {
    pub id: String,
    pub statement: String,
    pub acceptance: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sil: Option<String>,
}

#[derive(Deserialize)]
pub struct ResultPayload {
    pub schema: String,
    pub system: String,
    #[serde(default)]
    pub environment: Option<String>,
    pub commit: String,
    pub results: Vec<ResultItem>,
}

#[derive(Deserialize)]
pub struct ResultItem {
    pub req_id: String,
    pub verdict: String,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub evidence_kind: Option<String>,
    #[serde(default)]
    pub decision: Option<Decision>,
}

#[derive(Deserialize)]
pub struct Decision {
    pub plan: String,
    #[serde(default)]
    pub analysis: Option<String>,
    #[serde(default)]
    pub statement: Option<String>,
}

// --------------------------------------------------------------------------
// REQ-0182: verdict mapping
// --------------------------------------------------------------------------

/// The built-in mapping from common external verdicts to the local model.
/// `bench_cap_suspected` and `error` map to Fail (not a clean pass);
/// `operator_override` maps to Pass but the raw verdict is preserved so the
/// nuance is never lost. A project may override any entry via config.
fn default_verdict(raw: &str) -> Option<TestOutcome> {
    match raw {
        "pass" => Some(TestOutcome::Pass),
        "fail" => Some(TestOutcome::Fail),
        "error" => Some(TestOutcome::Fail),
        "bench_cap_suspected" => Some(TestOutcome::Fail),
        "operator_override" => Some(TestOutcome::Pass),
        _ => None,
    }
}

/// Resolve an external verdict to a local outcome, honouring the project's
/// configured overrides first, then the built-in defaults. Returns an error
/// for an unmapped verdict (REQ-0182: never default to pass).
fn resolve_verdict(project: &Project, raw: &str) -> Result<TestOutcome> {
    if let Some(map) = project
        .config
        .as_ref()
        .and_then(|c| c.test_integration.as_ref())
        .and_then(|t| t.verdict_map.as_ref())
    {
        if let Some(v) = map.get(raw) {
            return match v.to_lowercase().as_str() {
                "pass" => Ok(TestOutcome::Pass),
                "fail" => Ok(TestOutcome::Fail),
                other => Err(anyhow!(
                    "verdict map entry '{}' => '{}' must be 'pass' or 'fail'",
                    raw,
                    other
                )),
            };
        }
    }
    default_verdict(raw).ok_or_else(|| {
        anyhow!(
            "external verdict '{}' has no mapping — add it to _config.test_integration.verdict_map (pass|fail); refusing to default to pass",
            raw
        )
    })
}

// --------------------------------------------------------------------------
// REQ-0175: export the requirements due for verification
// --------------------------------------------------------------------------

fn build_request(project: &Project) -> RequestPayload {
    let commit = current_head();
    let mut requirements = Vec::new();
    // "Due for verification" = an active requirement not yet Verified, past Draft.
    for (id, r) in &project.requirements {
        if matches!(r.status, Status::Approved | Status::Implemented) {
            requirements.push(RequestItem {
                id: id.clone(),
                statement: r.statement.clone(),
                acceptance: r.acceptance.clone(),
                sil: None,
            });
        }
    }
    for (id, sr) in &project.safety_requirements {
        if matches!(sr.status, Status::Approved | Status::Implemented) {
            requirements.push(RequestItem {
                id: id.clone(),
                statement: sr.statement.clone(),
                acceptance: sr.acceptance.clone(),
                sil: project.inherited_sil(sr).map(|s| s.as_str().to_string()),
            });
        }
    }
    requirements.sort_by(|a, b| a.id.cmp(&b.id));
    RequestPayload {
        schema: REQUEST_SCHEMA.to_string(),
        commit,
        requirements,
    }
}

pub fn requests(args: TestRequestsArgs, file: &Option<PathBuf>) -> Result<()> {
    let (_p, project) = load_resolved(file)?;
    let payload = build_request(&project);
    let body = serde_json::to_string_pretty(&payload)?;
    match &args.out {
        Some(path) => {
            std::fs::write(path, &body).with_context(|| format!("write {}", path))?;
            println!(
                "Wrote {} requirement(s) due for verification to {} (commit {}).",
                payload.requirements.len(),
                path,
                short(&payload.commit)
            );
        }
        None => println!("{}", body),
    }
    Ok(())
}

// --------------------------------------------------------------------------
// REQ-0177..0184: ingest results
// --------------------------------------------------------------------------

/// Outcome of an ingest, for the caller to render.
pub struct IngestReport {
    pub attached: usize,
    pub skipped_duplicate: usize,
    pub promoted: Vec<String>,
    pub withheld_safety: Vec<String>,
}

/// REQ-0178: validate the whole payload before any mutation, so a malformed
/// or partially-unknown payload leaves project.req byte-identical.
fn preflight(project: &Project, payload: &ResultPayload) -> Result<()> {
    if payload.schema != RESULT_SCHEMA {
        return Err(anyhow!(
            "unsupported result schema '{}' (this binary speaks '{}')",
            payload.schema,
            RESULT_SCHEMA
        ));
    }
    if payload.commit.trim().is_empty() {
        return Err(anyhow!(
            "payload is missing the commit it was produced against"
        ));
    }
    if payload.system.trim().is_empty() {
        return Err(anyhow!(
            "payload is missing the originating system identity"
        ));
    }
    for r in &payload.results {
        let (id, fam) = crate::commands::verification::resolve(project, &r.req_id)
            .map_err(|_| anyhow!("result references unknown requirement '{}'", r.req_id))?;
        resolve_verdict(project, &r.verdict)?;
        // REQ-0183: a decision may only attach to a dossier anchored at the
        // same commit (or one with no conclusion yet).
        if r.decision.is_some() {
            let existing = match fam {
                crate::commands::verification::Family::Req => {
                    project.requirements[&id].verification.as_ref()
                }
                crate::commands::verification::Family::Sr => {
                    project.safety_requirements[&id].verification.as_ref()
                }
            };
            if let Some(v) = existing {
                if let Some(cc) = &v.concluded_commit {
                    if cc != &payload.commit {
                        return Err(anyhow!(
                            "{}: external decision is for commit {} but the dossier is anchored at {}",
                            id,
                            short(&payload.commit),
                            short(cc)
                        ));
                    }
                }
            }
        }
    }
    Ok(())
}

pub fn ingest_payload(
    project: &mut Project,
    payload: &ResultPayload,
    promote: bool,
) -> Result<IngestReport> {
    preflight(project, payload)?;
    let now = Utc::now();
    let mut report = IngestReport {
        attached: 0,
        skipped_duplicate: 0,
        promoted: Vec::new(),
        withheld_safety: Vec::new(),
    };
    for r in &payload.results {
        let (id, fam) = crate::commands::verification::resolve(project, &r.req_id)?;
        let outcome = resolve_verdict(project, &r.verdict)?;
        let kind = match r.evidence_kind.as_deref() {
            Some("composition") => EvidenceKind::Composition,
            Some("inspection") => EvidenceKind::Inspection,
            _ => EvidenceKind::Automated,
        };
        let external = ExternalSource {
            system: payload.system.clone(),
            environment: payload.environment.clone(),
            raw_verdict: Some(r.verdict.clone()),
        };
        let record = TestRecord {
            at: now,
            actor: super::current_actor(),
            commit: payload.commit.clone(),
            outcome,
            notes: r.notes.clone().unwrap_or_default(),
            kind,
            content_hash: None,
            linked_files: None,
            sil_gate_exception: false,
            sil_at_verification: None,
            external: Some(external),
        };

        // REQ-0177: idempotent — skip an identical prior ingest.
        let is_sr = matches!(fam, crate::commands::verification::Family::Sr);
        let tests = if is_sr {
            &project.safety_requirements[&id].tests
        } else {
            &project.requirements[&id].tests
        };
        let dup = tests.iter().any(|t| {
            t.commit == record.commit
                && t.outcome == record.outcome
                && t.external.as_ref().map(|e| (&e.system, &e.raw_verdict))
                    == record
                        .external
                        .as_ref()
                        .map(|e| (&e.system, &e.raw_verdict))
        });
        if dup {
            report.skipped_duplicate += 1;
            continue;
        }

        // REQ-0183: populate the dossier from the external decision.
        let dossier = r.decision.as_ref().map(|d| {
            let mut v = Verification::opened(
                d.plan.clone(),
                payload.system.clone(),
                payload.commit.clone(),
                now,
            );
            if let Some(a) = &d.analysis {
                v.analysis = Some(VerificationActivity {
                    summary: a.clone(),
                    outcome,
                    references: Vec::new(),
                    at: now,
                    actor: payload.system.clone(),
                });
            }
            v.testing = Some(VerificationActivity {
                summary: r
                    .notes
                    .clone()
                    .unwrap_or_else(|| "external bench result".into()),
                outcome,
                references: Vec::new(),
                at: now,
                actor: payload.system.clone(),
            });
            v.statement = d.statement.clone();
            v.verdict = Some(outcome);
            v.concluded = Some(now);
            v.concluded_commit = Some(payload.commit.clone());
            v
        });

        if is_sr {
            let sr = project.safety_requirements.get_mut(&id).unwrap();
            sr.tests.push(record);
            if let Some(v) = dossier {
                sr.verification = Some(v);
            }
            sr.updated = now;
            sr.history.push(super::history(
                "external evidence ingested",
                r.notes.clone(),
            ));
            // REQ-0184: never auto-promote a safety requirement on external
            // evidence — that needs the human walkthrough + co-sign.
            if promote {
                report.withheld_safety.push(id.clone());
            }
        } else {
            let req = project.requirements.get_mut(&id).unwrap();
            req.tests.push(record);
            if let Some(v) = dossier {
                req.verification = Some(v);
            }
            req.updated = now;
            req.history.push(super::history(
                "external evidence ingested",
                r.notes.clone(),
            ));
            // REQ-0184: an ordinary requirement MAY be promoted when its
            // dossier is now complete and it is sitting at Implemented.
            if promote
                && matches!(outcome, TestOutcome::Pass)
                && matches!(req.status, Status::Implemented)
                && req
                    .verification
                    .as_ref()
                    .map(|v| v.passed())
                    .unwrap_or(false)
            {
                req.status = Status::Verified;
                req.history.push(super::history(
                    "promoted to verified (external dossier)",
                    None,
                ));
                report.promoted.push(id.clone());
            }
        }
        report.attached += 1;
    }
    Ok(report)
}

fn render_report(report: &IngestReport, system: &str) {
    println!(
        "Ingested {} result(s) from {} ({} duplicate(s) skipped).",
        report.attached, system, report.skipped_duplicate
    );
    if !report.promoted.is_empty() {
        println!("  promoted to Verified: {}", report.promoted.join(", "));
    }
    for id in &report.withheld_safety {
        println!(
            "  {}: safety requirement NOT promoted on external evidence — record a human walkthrough + co-sign (REQ-0184)",
            id
        );
    }
}

pub fn ingest(args: TestIngestArgs, file: &Option<PathBuf>) -> Result<()> {
    let raw = std::fs::read_to_string(&args.source)
        .with_context(|| format!("read result payload {}", args.source))?;
    let payload: ResultPayload = serde_json::from_str(&raw).map_err(|e| {
        anyhow!(
            "result payload is not valid JSON ({}). See `req schema test-result`.",
            e
        )
    })?;
    let (path, mut project, _lock) = load_for_mutation(file)?;
    let report = ingest_payload(&mut project, &payload, args.promote)?;
    if report.attached > 0 || !report.promoted.is_empty() {
        project.updated = Utc::now();
        storage::save(&path, &project)?;
    }
    render_report(&report, &payload.system);
    Ok(())
}

// --------------------------------------------------------------------------
// REQ-0181: authenticated HTTP pull
// --------------------------------------------------------------------------

pub fn pull(args: TestPullArgs, file: &Option<PathBuf>) -> Result<()> {
    // Fetch first; only touch project.req once we have a valid payload, so a
    // network or auth failure leaves the spec unchanged.
    let mut req = ureq::get(&args.from);
    if let Some(token) = &args.token {
        req = req.set("Authorization", &format!("Bearer {}", token));
    }
    let body = req
        .call()
        .map_err(|e| anyhow!("fetch {} failed: {}", args.from, e))?
        .into_string()
        .context("read response body")?;
    let payload: ResultPayload = serde_json::from_str(&body)
        .map_err(|e| anyhow!("response is not a valid result payload ({})", e))?;
    let (path, mut project, _lock) = load_for_mutation(file)?;
    let report = ingest_payload(&mut project, &payload, args.promote)?;
    if report.attached > 0 || !report.promoted.is_empty() {
        project.updated = Utc::now();
        storage::save(&path, &project)?;
    }
    render_report(&report, &payload.system);
    Ok(())
}

// --------------------------------------------------------------------------
// helpers
// --------------------------------------------------------------------------

fn current_head() -> String {
    std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

fn short(sha: &str) -> &str {
    &sha[..sha.len().min(8)]
}
