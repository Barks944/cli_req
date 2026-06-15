// REQ-0138: functional-safety governance — the human-only controls that
// (1) gate the safety features behind an explicit, committed liability
// acknowledgement and (2) own the per-project risk-graph calibration.
//
// The acknowledgement lives in a SIBLING FILE next to project.req
// (`req-safety-acceptance.json`), not inside the integrity-hashed spec:
// it is a git-tracked artifact, so enabling the safety features shows up
// in a PR diff and a reviewer can see who signed on and when. The file's
// presence (with a current disclaimer version) is what activates the
// feature — delete it and safety mutations stop.
//
// This command is deliberately NOT on the agent/MCP surface: accepting
// liability and recalibrating risk are human governance decisions.
use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use std::path::{Path, PathBuf};

use crate::cli::{
    ImpactArgs, SafetyAcceptArgs, SafetyAckArgs, SafetyCalibrateArgs, SafetyCmd, SafetyStatusArgs,
    SafetyWalkthroughArgs,
};
use crate::model::{
    calibration_leaf, Avoidance, CalibrationRow, Consequence, DisclaimerAcceptance, Frequency,
    Link, LinkKind, Probability, Project, ProjectConfig, SafetyConfig, Sil, Status, TestOutcome,
    WalkthroughAck, SAFETY_DISCLAIMER_VERSION,
};
use crate::storage::{self, load_for_mutation, resolve_path};

/// REQ-0144: the agreement a user must accept before any functional-safety
/// feature is enabled disclaims all developer liability and states that req is
/// a research tool not intended for real safety applications. Keep this in sync
/// with `req help safety`; bump SAFETY_DISCLAIMER_VERSION when its substance
/// changes (a prior acceptance for an older version no longer satisfies the
/// gate, forcing the user to re-accept the updated terms).
pub const DISCLAIMER: &str = "\
FUNCTIONAL-SAFETY DISCLAIMER — read before enabling these features.

  • req is NOT a qualified safety tool. Under IEC 61508-3 §7.4.4 (and
    ISO 26262-8), a tool whose output you rely on without independent
    verification needs a tool-confidence/qualification argument. req
    provides none. Qualifying it, or independently verifying every SIL
    it computes, is YOUR responsibility.
  • The SIL req shows is a CANDIDATE derived from the qualitative risk
    parameters you enter, using the IEC 61508-5 Annex D worked-example
    calibration (or your own, if you set one). It is not objective and
    does not remove the need for competent review.
  • req tracks the REQUIRED integrity target and traceability — NOT
    achieved integrity (no PFD/PFH, diagnostic coverage, SFF, SIL
    decomposition). A \"complete\" trace means linked-and-verified, not
    safe.
  • req is a RESEARCH / EXPERIMENTAL tool. It is NOT intended for use
    in real safety applications and must not be relied upon for any
    safety-related decision.
  • This software is provided \"AS IS\", without warranty of any kind.
    The authors accept NO liability whatsoever. Nothing it produces is
    safety assurance or fitness for any safety-related purpose.

By accepting you confirm you understand the above — including that req
is a research tool, not for real safety use, and carries no developer
liability whatsoever — and take responsibility for the safety
determination.";

/// Path to the acceptance file that sits beside the project.
pub fn acceptance_path(project_path: &Path) -> PathBuf {
    let dir = if project_path.is_dir() {
        project_path.to_path_buf()
    } else {
        project_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."))
    };
    dir.join("req-safety-acceptance.json")
}

/// Read the acceptance record beside the project, if present and parseable.
pub fn read_acceptance(project_path: &Path) -> Option<DisclaimerAcceptance> {
    let p = acceptance_path(project_path);
    let body = std::fs::read_to_string(p).ok()?;
    serde_json::from_str(&body).ok()
}

/// Whether the safety features are activated for this project: a valid
/// acceptance file exists for the current disclaimer version.
pub fn is_enabled(project_path: &Path) -> bool {
    read_acceptance(project_path)
        .map(|a| a.disclaimer_version == SAFETY_DISCLAIMER_VERSION)
        .unwrap_or(false)
}

/// The gate every safety MUTATION calls. Reading (list/show/trace) is
/// always allowed so an existing safety case stays inspectable; only
/// creating or changing safety artifacts requires the acknowledgement.
pub fn ensure_enabled(file: &Option<PathBuf>) -> Result<()> {
    ensure_enabled_path(&resolve_path(file))
}

/// Path-keyed form of the gate, for callers (MCP) that already hold the
/// resolved project path.
pub fn ensure_enabled_path(path: &Path) -> Result<()> {
    if is_enabled(path) {
        return Ok(());
    }
    let existing = read_acceptance(path);
    let why = match existing {
        Some(_) => "the acceptance on file is for an older disclaimer version",
        None => "the functional-safety features are not enabled for this project",
    };
    Err(anyhow!(
        "{why}. A human must accept the safety disclaimer first:\n\n    \
         req safety accept-disclaimer --name \"Your Name <you@example.com>\"\n\n\
         This writes {} (commit it) which activates hazards / safety \
         functions / safety requirements. See `req help safety`.",
        acceptance_path(path).display()
    ))
}

pub fn run(cmd: SafetyCmd, file: &Option<PathBuf>) -> Result<()> {
    match cmd {
        SafetyCmd::Accept(a) => accept(a, file),
        SafetyCmd::Status(a) => status(a, file),
        SafetyCmd::Calibrate(a) => calibrate(a, file),
        // REQ-0169/0170: the guided walkthrough and per-requirement ack are
        // human governance acts, so they live on this (non-MCP) surface.
        // REQ-0172: they are gated on the feature having been ENGAGED (an
        // acceptance file of any version exists), NOT on the current disclaimer
        // version. Reviewing and acknowledging the existing case is exactly
        // what a human does as part of (re-)accepting after a disclaimer bump,
        // so requiring the current version here would deadlock `safety accept`
        // (which REQ-0172 blocks until the acks exist).
        SafetyCmd::Walkthrough(a) => {
            ensure_engaged(file)?;
            walkthrough(a, file)
        }
        SafetyCmd::Acknowledge(a) => {
            ensure_engaged(file)?;
            acknowledge(a, file)
        }
    }
}

/// REQ-0172: the safety feature has been ENGAGED for this project — an
/// acceptance file exists, of any disclaimer version. Unlike `ensure_enabled`
/// this does not require the *current* version, so a human can walk through
/// and acknowledge the existing safety case before re-accepting an updated
/// disclaimer (otherwise the REQ-0172 accept-gate would be unreachable).
fn ensure_engaged(file: &Option<PathBuf>) -> Result<()> {
    let path = resolve_path(file);
    if read_acceptance(&path).is_some() {
        return Ok(());
    }
    Err(anyhow!(
        "the functional-safety features have never been accepted for this project. \
         A human must run `req safety accept-disclaimer --name \"...\"` first."
    ))
}

// ---------------------------------------------------------------------------
// REQ-0169..0174: guided safety walkthrough + acknowledgement
// ---------------------------------------------------------------------------

/// REQ-0174: why a safety requirement's chain is not yet acknowledgeable, or
/// `None` when the chain is complete (assessed hazard → live SF → SR with
/// passing evidence).
fn chain_incompleteness(project: &Project, sr_id: &str) -> Option<String> {
    let sr = match project.safety_requirements.get(sr_id) {
        Some(sr) => sr,
        None => return Some("no such safety requirement".into()),
    };
    // A live realizing safety function that mitigates an assessed hazard is
    // exactly what gives the SR an inherited SIL.
    if project.inherited_sil(sr).is_none() {
        return Some(
            "chain has no assessed hazard reaching a live safety function (nothing to inherit a SIL from)"
                .into(),
        );
    }
    let has_pass = sr
        .verification
        .as_ref()
        .map(|v| v.passed())
        .unwrap_or(false)
        || sr
            .tests
            .iter()
            .any(|t| matches!(t.outcome, TestOutcome::Pass));
    if !has_pass {
        return Some("no passing verification evidence yet".into());
    }
    None
}

/// REQ-0172: a fresh acknowledgement is a non-objection ack made at the
/// current commit.
fn ack_is_fresh(ack: Option<&WalkthroughAck>, head: &str) -> bool {
    match ack {
        Some(a) => !a.objected && !a.commit.is_empty() && a.commit == head,
        None => false,
    }
}

/// In-scope safety requirements for the project-wide gate: every
/// non-Obsolete safety requirement.
fn in_scope_srs(project: &Project) -> Vec<String> {
    let mut ids: Vec<String> = project
        .safety_requirements
        .values()
        .filter(|sr| !matches!(sr.status, Status::Obsolete))
        .map(|sr| sr.id.clone())
        .collect();
    ids.sort();
    ids
}

fn head_sha() -> String {
    std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

fn walkthrough(args: SafetyWalkthroughArgs, file: &Option<PathBuf>) -> Result<()> {
    let path = resolve_path(file);
    let project = storage::load(&path)?;
    let head = head_sha();

    // Resolve scope: a single chain (HAZ/SF/SR) or every in-scope SR.
    let ids: Vec<String> = match &args.target {
        Some(t) => srs_for_target(&project, t)?,
        None => in_scope_srs(&project),
    };

    if args.gate {
        // REQ-0172: report the SRs that block acceptance (missing/stale/objected).
        let blocking: Vec<(String, String)> = ids
            .iter()
            .filter(|id| {
                !ack_is_fresh(project.safety_requirements[*id].walkthrough.as_ref(), &head)
            })
            .map(|id| {
                let reason = match project.safety_requirements[id].walkthrough.as_ref() {
                    None => "never acknowledged".to_string(),
                    Some(a) if a.objected => "objection on record".to_string(),
                    Some(_) => "acknowledgement stale (chain changed since review)".to_string(),
                };
                (id.clone(), reason)
            })
            .collect();
        if args.json {
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "in_scope": ids,
                    "blocking": blocking.iter().map(|(id, why)| serde_json::json!({"id": id, "why": why})).collect::<Vec<_>>(),
                    "ok": blocking.is_empty(),
                }))?
            );
        } else if blocking.is_empty() {
            println!(
                "All {} in-scope safety requirement(s) carry a fresh acknowledgement.",
                ids.len()
            );
        } else {
            println!(
                "{} safety requirement(s) are not acknowledged at the current commit:",
                blocking.len()
            );
            for (id, why) in &blocking {
                println!("  {} — {}", id, why);
            }
        }
        if !blocking.is_empty() {
            return Err(anyhow!(
                "walkthrough gate: unacknowledged safety requirements"
            ));
        }
        return Ok(());
    }

    // REQ-0169: render each chain for review.
    if ids.is_empty() {
        println!("No in-scope safety requirements to walk through.");
        return Ok(());
    }

    // REQ-0199: a human on a terminal gets the interactive arrow-key
    // navigator; agents, pipes, and CI fall back to the static rendering so
    // nothing scripted is left waiting on a keypress.
    use std::io::IsTerminal;
    let is_tty = atty_stdin() && std::io::stdout().is_terminal();
    let is_agent = matches!(super::current_actor_kind(), crate::model::ActorKind::Agent);
    if args.interactive && (!is_tty || is_agent) {
        eprintln!("(interactive mode needs a human terminal — showing the static walkthrough)");
    }
    // Source root for the staleness probe: project.req usually sits at the
    // repo root, beside the src/ tree the markers live in.
    let root = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    };
    if is_tty && !is_agent && !args.no_interactive {
        return run_interactive(file, ids, head, args.full, &root);
    }

    // Static rendering: each SR's chain printed top to bottom.
    for (n, id) in ids.iter().enumerate() {
        for line in render_sr(&project, id, &head, args.full, &root, n, ids.len()) {
            println!("{}", line);
        }
    }
    println!(
        "\nWalk each chain top to bottom, then acknowledge (or --object) each: \
         `req safety acknowledge SR-NNNN`.  (`-i` interactive · `--full` dossier detail)"
    );
    Ok(())
}

/// REQ-0169/0198: render one safety requirement's chain as a top-down story —
/// hazard → harm → mitigating safety function → requirement → verification
/// dossier → the human's call — into a line buffer the caller prints or
/// redraws. `full` expands the dossier with analysis/testing detail and refs.
fn render_sr(
    project: &Project,
    id: &str,
    head: &str,
    full: bool,
    root: &Path,
    idx: usize,
    total: usize,
) -> Vec<String> {
    let sr = &project.safety_requirements[id];
    let mut out: Vec<String> = Vec::new();
    out.push(String::new());
    out.push("━".repeat(74));
    out.push(format!("  [{}/{}]  {} — {}", idx + 1, total, sr.id, sr.title));
    out.push("━".repeat(74));
    out.push(String::new());

    // Hazard → mitigating safety function, grouped per realizing SF so the
    // "this hazard is mitigated by this function" link stays clear.
    for sf in project.safety_functions.values() {
        if !sr
            .links
            .iter()
            .any(|l| matches!(l.kind, crate::model::LinkKind::Realizes) && l.target == sf.id)
        {
            continue;
        }
        for l in &sf.links {
            if let Some(h) = project.hazards.get(&l.target) {
                let req = project.required_sil(h).map(|s| s.as_str()).unwrap_or("—");
                field(&mut out, "HAZARD", &format!("{}  ·  required {}  ·  {}", h.id, req, h.title));
                field(&mut out, "harm", &h.harm);
                // REQ-0204: the hazard's mitigation-adequacy standing (and, in
                // --full, the per-SF walk-through behind it).
                if let Some(a) = &h.adequacy {
                    let st = match (a.verdict, a.human_confirmation.is_some()) {
                        (Some(vd), true) => format!("{} · co-signed", vd.as_str()),
                        (Some(vd), false) => format!("{} · awaiting co-sign", vd.as_str()),
                        (None, _) => "in progress".to_string(),
                    };
                    field(&mut out, "adequacy", &st);
                    if full {
                        for c in &a.coverage {
                            field(&mut out, "", &format!("covers {}: {}", c.target, c.note));
                        }
                    }
                }
            }
        }
        let asil = project.allocated_sil(sf).map(|s| s.as_str()).unwrap_or("—");
        field(&mut out, "MITIGATED BY", &format!("{}  ·  {}  ·  {}", sf.id, asil, sf.title));
        // REQ-0204: the safety function's adequacy walk-through — why each
        // realizing SR implements it (expanded in --full).
        if full {
            if let Some(sfv) = &sf.verification {
                for c in &sfv.coverage {
                    field(&mut out, "", &format!("{} covers via {}: {}", sf.id, c.target, c.note));
                }
            }
        }
        out.push(String::new());
    }

    // The safety requirement itself.
    let inh = project.inherited_sil(sr).map(|s| s.as_str()).unwrap_or("—");
    field(&mut out, "REQUIREMENT", &format!("inherited {}  ·  status {}", inh, sr.status.as_str()));
    field(&mut out, "", &sr.statement);

    // REQ-0198: the verification dossier behind the requirement, not just its
    // conclusion. Default view = verdict + staleness + co-sign; --full adds the
    // analysis/testing summaries with their outcomes and source references.
    match sr.verification.as_ref() {
        None => field(&mut out, "EVIDENCE", "(no verification dossier yet)"),
        Some(v) => {
            let verdict = v
                .verdict
                .map(|o| o.as_str().to_uppercase())
                .unwrap_or_else(|| "pending".to_string());
            let by = if v.actor.is_empty() { "—".to_string() } else { v.actor.clone() };
            let at = v
                .concluded
                .map(|d| d.format("%Y-%m-%d").to_string())
                .unwrap_or_else(|| "—".to_string());
            let commit = v
                .concluded_commit
                .as_deref()
                .map(crate::commands::test_cmd::short)
                .unwrap_or_else(|| "—".to_string());
            field(
                &mut out,
                "EVIDENCE",
                &format!("verdict {}  ·  concluded by {} @ {}  ·  {}", verdict, by, commit, at),
            );
            if dossier_stale(v, id, root) {
                field(
                    &mut out,
                    "⚠ STALE",
                    "anchored source has changed since conclude — re-verify before relying on this",
                );
            } else if v.content_hash.is_some() {
                field(&mut out, "anchor", "fresh — evidence matches the current source");
            }
            match &v.human_confirmation {
                Some(hc) => field(
                    &mut out,
                    "co-sign",
                    &format!("confirmed by {} on {}", hc.actor, hc.at.format("%Y-%m-%d")),
                ),
                None => field(
                    &mut out,
                    "co-sign",
                    &format!("not yet co-signed — `req verification confirm {}`", sr.id),
                ),
            }
            if full {
                if let Some(a) = &v.analysis {
                    field(&mut out, "analysis", &format!("[{}] {}", a.outcome.as_str(), a.summary));
                    if !a.references.is_empty() {
                        field(&mut out, "", &format!("refs: {}", a.references.join(", ")));
                    }
                }
                if let Some(t) = &v.testing {
                    field(&mut out, "testing", &format!("[{}] {}", t.outcome.as_str(), t.summary));
                    if !t.references.is_empty() {
                        field(&mut out, "", &format!("refs: {}", t.references.join(", ")));
                    }
                }
            }
            if let Some(st) = &v.statement {
                field(&mut out, "conclusion", st);
            }
        }
    }

    // The human's call on this requirement.
    out.push(format!("  {}", "─".repeat(70)));
    match chain_incompleteness(project, id) {
        Some(why) => {
            out.push("  ⚠  chain incomplete — cannot be acknowledged yet".to_string());
            out.push(format!("     {}", why));
        }
        None => match sr.walkthrough.as_ref() {
            Some(a) if ack_is_fresh(Some(a), head) => out.push(format!(
                "  ✓  acknowledged by {} at {}",
                a.reviewer,
                a.at.format("%Y-%m-%d %H:%M UTC")
            )),
            Some(a) if a.objected => out.push(format!("  ✗  objection on record by {}", a.reviewer)),
            Some(_) => {
                out.push("  ⚠  prior acknowledgement is stale".to_string());
                out.push(format!(
                    "     re-acknowledge at this commit — `req safety acknowledge {}`",
                    sr.id
                ));
            }
            None => {
                out.push("  ▷  awaiting your acknowledgement".to_string());
                out.push(format!(
                    "     run:  req safety acknowledge {}   (or --object to decline)",
                    sr.id
                ));
            }
        },
    }
    out
}

/// REQ-0198: is this dossier's content-hash anchor stale against the current
/// source? Exempt dossiers have no genuine anchor, so they are never "stale".
fn dossier_stale(v: &crate::model::Verification, id: &str, root: &Path) -> bool {
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

/// REQ-0199: interactive arrow-key navigator over the in-scope safety
/// requirements. `←/→` move, `a`/`o` acknowledge/object, `f` toggles full
/// dossier detail, `q` quits. Acknowledgements are recorded in place under a
/// single held mutation lock, applying the same human-only and
/// chain-completeness rules as `req safety acknowledge`.
fn run_interactive(
    file: &Option<PathBuf>,
    ids: Vec<String>,
    head: String,
    full_start: bool,
    root: &Path,
) -> Result<()> {
    use console::{Key, Term};
    let (path, mut project, _lock) = load_for_mutation(file)?;
    let term = Term::stdout();
    let mut cur = 0usize;
    let mut full = full_start;
    let mut notice = String::new();
    loop {
        term.clear_screen().ok();
        for line in render_sr(&project, &ids[cur], &head, full, root, cur, ids.len()) {
            println!("{}", line);
        }
        println!("  {}", "─".repeat(70));
        if !notice.is_empty() {
            println!("  » {}", notice);
        }
        println!(
            "  ←/→ move  ·  a accept  ·  o object  ·  f {}  ·  q quit",
            if full { "brief" } else { "full" }
        );
        notice.clear();
        match term.read_key() {
            Ok(Key::ArrowLeft) | Ok(Key::Char('h')) => {
                if cur == 0 {
                    notice = "already at the first requirement".to_string();
                } else {
                    cur -= 1;
                }
            }
            Ok(Key::ArrowRight) | Ok(Key::Char('l')) | Ok(Key::Char(' ')) => {
                if cur + 1 >= ids.len() {
                    notice = "already at the last requirement".to_string();
                } else {
                    cur += 1;
                }
            }
            Ok(Key::Char('f')) | Ok(Key::Char('F')) => full = !full,
            Ok(Key::Char('a')) | Ok(Key::Char('A')) => {
                match record_ack(&mut project, &ids[cur], false, None, &head) {
                    Ok(msg) => {
                        storage::save(&path, &project)?;
                        notice = msg;
                    }
                    Err(e) => notice = e.to_string(),
                }
            }
            Ok(Key::Char('o')) | Ok(Key::Char('O')) => {
                match record_ack(&mut project, &ids[cur], true, None, &head) {
                    Ok(msg) => {
                        storage::save(&path, &project)?;
                        notice = msg;
                    }
                    Err(e) => notice = e.to_string(),
                }
            }
            Ok(Key::Char('q')) | Ok(Key::Char('Q')) | Ok(Key::Escape) => break,
            _ => {}
        }
    }
    term.clear_screen().ok();
    // REQ-0172: a parting summary of where the sign-off gate stands.
    let pending = ids
        .iter()
        .filter(|id| !ack_is_fresh(project.safety_requirements[*id].walkthrough.as_ref(), &head))
        .count();
    if pending == 0 {
        println!(
            "All {} in-scope safety requirement(s) carry a fresh acknowledgement.",
            ids.len()
        );
    } else {
        println!(
            "{} of {} in-scope safety requirement(s) still await a fresh acknowledgement \
             (`req safety walkthrough --gate`).",
            pending,
            ids.len()
        );
    }
    Ok(())
}

/// REQ-0169: word-wrap `text` to at most `width` columns on whitespace
/// boundaries. Returns at least one (possibly empty) line so callers can
/// rely on it for the guided-walkthrough layout.
fn wrap_text(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        if current.is_empty() {
            current.push_str(word);
        } else if current.chars().count() + 1 + word.chars().count() <= width {
            current.push(' ');
            current.push_str(word);
        } else {
            lines.push(std::mem::take(&mut current));
            current.push_str(word);
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

/// Push a labelled, word-wrapped field into `out`: the label sits in a
/// fixed-width gutter on the first line and continuation lines align under the
/// body column. An empty label continues the previous field's body.
fn field(out: &mut Vec<String>, label: &str, body: &str) {
    const LABEL_W: usize = 12; // fits "MITIGATED BY" / "REQUIREMENT"
    const BODY_W: usize = 58;
    let pad = 2 + LABEL_W + 1;
    for (i, line) in wrap_text(body, BODY_W).iter().enumerate() {
        if i == 0 {
            out.push(format!("  {:<lw$} {}", label, line, lw = LABEL_W));
        } else {
            out.push(format!("{:pad$}{}", "", line, pad = pad));
        }
    }
}

/// Resolve a HAZ/SF/SR target to the set of safety requirements in its chain.
fn srs_for_target(project: &Project, raw: &str) -> Result<Vec<String>> {
    let up = raw.trim().to_uppercase();
    let mut ids: Vec<String> = if up.starts_with("SR") {
        let (id, _) = crate::commands::verification::resolve(project, raw)?;
        vec![id]
    } else if up.starts_with("SF") {
        let sf = up;
        project
            .safety_requirements
            .values()
            .filter(|sr| {
                sr.links
                    .iter()
                    .any(|l| matches!(l.kind, crate::model::LinkKind::Realizes) && l.target == sf)
            })
            .map(|sr| sr.id.clone())
            .collect()
    } else if up.starts_with("HAZ") {
        // SRs whose realizing SF mitigates this hazard.
        let haz = up;
        let sfs: Vec<String> = project
            .safety_functions
            .values()
            .filter(|sf| {
                sf.links
                    .iter()
                    .any(|l| matches!(l.kind, crate::model::LinkKind::Mitigates) && l.target == haz)
            })
            .map(|sf| sf.id.clone())
            .collect();
        project
            .safety_requirements
            .values()
            .filter(|sr| {
                sr.links.iter().any(|l| {
                    matches!(l.kind, crate::model::LinkKind::Realizes) && sfs.contains(&l.target)
                })
            })
            .map(|sr| sr.id.clone())
            .collect()
    } else {
        return Err(anyhow!("walkthrough target must be a HAZ-/SF-/SR- id"));
    };
    ids.sort();
    ids.dedup();
    Ok(ids)
}

fn acknowledge(args: SafetyAckArgs, file: &Option<PathBuf>) -> Result<()> {
    // REQ-0173: acknowledgement is the human judgement the gate protects —
    // an agent may not record it.
    if matches!(super::current_actor_kind(), crate::model::ActorKind::Agent) {
        return Err(anyhow!(
            "a safety-walkthrough acknowledgement must be made by a human, but \
             REQ_ACTOR_KIND=agent. A person must run `req safety acknowledge`."
        ));
    }
    let (path, mut project, _lock) = load_for_mutation(file)?;
    let (id, fam) = crate::commands::verification::resolve(&project, &args.id)?;
    if !matches!(fam, crate::commands::verification::Family::Sr) {
        return Err(anyhow!("{} is not a safety requirement", args.id));
    }
    let head = head_sha();
    let msg = record_ack(&mut project, &id, args.object, args.note.clone(), &head)?;
    storage::save(&path, &project)?;
    println!("{}", msg);
    Ok(())
}

/// REQ-0170/0174: apply a walkthrough acknowledgement (or objection) to `id`
/// in memory, enforcing the chain-completeness rule, and return a status line.
/// Shared by `req safety acknowledge` and the interactive walkthrough so the
/// rules live in one place. Does not save — the caller persists the project.
fn record_ack(
    project: &mut Project,
    id: &str,
    object: bool,
    note: Option<String>,
    head: &str,
) -> Result<String> {
    // REQ-0174: refuse to acknowledge an incomplete chain (objections allowed).
    if !object {
        if let Some(why) = chain_incompleteness(project, id) {
            return Err(anyhow!(
                "{} cannot be acknowledged: {}. Complete the chain first, or object.",
                id,
                why
            ));
        }
    }
    let now = Utc::now();
    let reviewer = super::current_actor();
    let sr = project
        .safety_requirements
        .get_mut(id)
        .ok_or_else(|| anyhow!("no such safety requirement: {}", id))?;
    sr.walkthrough = Some(WalkthroughAck {
        reviewer: reviewer.clone(),
        at: now,
        commit: head.to_string(),
        objected: object,
        note: note.clone(),
    });
    sr.updated = now;
    sr.history.push(super::history(
        if object {
            "walkthrough objection"
        } else {
            "walkthrough acknowledged"
        },
        note,
    ));
    project.updated = now;
    Ok(if object {
        format!("Recorded objection on {} by {}.", id, reviewer)
    } else {
        format!(
            "Acknowledged {} by {} at {}.",
            id,
            reviewer,
            &head[..head.len().min(8)]
        )
    })
}

fn accept(args: SafetyAcceptArgs, file: &Option<PathBuf>) -> Result<()> {
    let path = resolve_path(file);
    // The project must exist (so the acceptance sits beside a real spec).
    storage::load(&path).context("open project before accepting (run `req init` first?)")?;

    // REQ-0193: accepting the disclaimer ACTIVATES the safety features and is
    // deliberately NOT gated on safety-requirement acknowledgement. Gating
    // activation on a case you can only build once activated was circular; the
    // acknowledgement gate lives on the sign-off check `req safety walkthrough
    // --gate` (REQ-0172), not here.

    // REQ-0138: acceptance must be a deliberate human act, as far as a
    // CLI can tell. We CANNOT cryptographically prove humanness — an
    // agent shares the shell and filesystem — so this is "raise the bar
    // + make it accountable", not "prevent". The bar:
    //   (1) `req safety` is absent from the MCP surface (an agent's
    //       normal channel can't reach it);
    //   (2) refuse a self-identified agent (REQ_ACTOR_KIND=agent);
    //   (3) require an interactive terminal to confirm — there is no
    //       `--yes` escape, because that was just a backdoor an agent
    //       could take.
    // The real control is that the result is a committed, attributed
    // file: a forged acceptance is visible in the diff under a name.
    if matches!(super::current_actor_kind(), crate::model::ActorKind::Agent) {
        return Err(anyhow!(
            "accepting the safety disclaimer must be done by a human, but \
             REQ_ACTOR_KIND=agent. A person must run `req safety accept-disclaimer`."
        ));
    }
    let name = match args.name {
        Some(n) if !n.trim().is_empty() => n,
        _ => return Err(anyhow!("--name is required (record who is accepting)")),
    };
    if !atty_stdin() {
        return Err(anyhow!(
            "`req safety accept-disclaimer` needs an interactive terminal — run it at a \
             real prompt. There is deliberately no non-interactive flag. For \
             unattended setup, a human can instead create {} by hand (it is a \
             small JSON file) and commit it; see `req help safety`.",
            acceptance_path(&path).display()
        ));
    }

    println!("{}\n", DISCLAIMER);
    use dialoguer::Confirm;
    let ok = Confirm::new()
        .with_prompt("Do you accept, on behalf of your project?")
        .default(false)
        .interact()?;
    if !ok {
        return Err(anyhow!("not accepted — safety features remain disabled"));
    }

    let record = DisclaimerAcceptance {
        notice: "By committing this file the named person accepts the req \
                 functional-safety disclaimer: req is an unqualified aid \
                 (IEC 61508-3 §7.4.4), the SIL is a candidate, the software \
                 is provided AS IS with no warranty or liability, and the \
                 safety determination remains the user's responsibility."
            .into(),
        accepted_by: name,
        at: Utc::now(),
        tool_version: env!("CARGO_PKG_VERSION").to_string(),
        disclaimer_version: SAFETY_DISCLAIMER_VERSION.to_string(),
    };
    let out = acceptance_path(&path);
    let body = serde_json::to_string_pretty(&record)?;
    std::fs::write(&out, body).with_context(|| format!("write {}", out.display()))?;
    println!(
        "\nSafety features ENABLED. Wrote {}.\nCommit this file — its presence \
         is what activates hazards / safety functions / safety requirements.",
        out.display()
    );
    Ok(())
}

fn status(args: SafetyStatusArgs, file: &Option<PathBuf>) -> Result<()> {
    let path = resolve_path(file);
    let project = storage::load(&path).ok();
    let acceptance = read_acceptance(&path);
    let enabled = is_enabled(&path);
    let cal_label = project
        .as_ref()
        .and_then(|p| p.config.as_ref())
        .and_then(|c| c.safety.as_ref())
        .and_then(|s| s.calibration_label.clone());
    let cal_overrides = project
        .as_ref()
        .and_then(|p| p.calibration())
        .map(|t| t.len())
        .unwrap_or(0);

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "enabled": enabled,
                "acceptance": acceptance,
                "disclaimer_version": SAFETY_DISCLAIMER_VERSION,
                "calibration_label": cal_label,
                "calibration_overrides": cal_overrides,
            }))?
        );
        return Ok(());
    }

    // REQ-0138: report the governance state — enablement, acceptance, calibration.
    println!(
        "Functional safety: {}",
        if enabled { "ENABLED" } else { "disabled" }
    );
    match &acceptance {
        Some(a) => println!(
            "  accepted by {} on {} (req {}, disclaimer v{})",
            a.accepted_by,
            a.at.format("%Y-%m-%d"),
            a.tool_version,
            a.disclaimer_version
        ),
        None => println!(
            "  no acceptance file — run `req safety accept-disclaimer --name \"...\"` to enable"
        ),
    }
    println!(
        "  calibration: {} ({} leaf override(s))",
        cal_label
            .as_deref()
            .unwrap_or("IEC 61508-5 Annex D (default)"),
        cal_overrides
    );
    Ok(())
}

fn calibrate(args: SafetyCalibrateArgs, file: &Option<PathBuf>) -> Result<()> {
    // Show-only path needs no lock.
    if args.show && args.label.is_none() && args.set.is_empty() && !args.reset {
        let (_p, project) = storage::load_resolved(file)?;
        print_calibration(&project);
        return Ok(());
    }

    let (path, mut project, _lock) = storage::load_for_mutation(file)?;
    let cfg = project
        .config
        .get_or_insert_with(ProjectConfig::default)
        .safety
        .get_or_insert_with(SafetyConfig::default);

    if args.reset {
        cfg.calibration = None;
        cfg.calibration_label = None;
    }
    if let Some(label) = args.label {
        cfg.calibration_label = Some(label);
    }
    for spec in &args.set {
        let (leaf, row) = parse_set(spec)?;
        cfg.calibration
            .get_or_insert_with(Default::default)
            .insert(leaf, row);
    }
    // Drop an emptied override map so the file stays clean.
    if cfg
        .calibration
        .as_ref()
        .map(|m| m.is_empty())
        .unwrap_or(false)
    {
        cfg.calibration = None;
    }

    storage::save(&path, &project)?;
    print_calibration(&project);
    Ok(())
}

fn print_calibration(project: &crate::model::Project) {
    let label = project
        .config
        .as_ref()
        .and_then(|c| c.safety.as_ref())
        .and_then(|s| s.calibration_label.as_deref())
        .unwrap_or("IEC 61508-5 Annex D (default)");
    println!("Risk-graph calibration: {}", label);
    match project.calibration() {
        None => println!("  (no leaf overrides — every leaf uses the Annex D default)"),
        Some(table) => {
            println!("  {} leaf override(s) [W3 / W2 / W1]:", table.len());
            for (leaf, row) in table {
                println!(
                    "    {:<14} {} / {} / {}",
                    leaf,
                    row.w3.as_str(),
                    row.w2.as_str(),
                    row.w1.as_str()
                );
            }
        }
    }
}

/// Parse `--set "C_D/F_B/P_B=W3:4,W2:3,W1:2"`. All three W values are
/// required; their SIL tokens accept `1..4`, `SIL1..SIL4`, `a`, `b`, or
/// `none`. The leaf must be a valid (C,F,P) combination.
// REQ-0138: parse a calibration override row from the governance CLI.
fn parse_set(spec: &str) -> Result<(String, CalibrationRow)> {
    let (leaf_raw, rhs) = spec.split_once('=').ok_or_else(|| {
        anyhow!(
            "--set expects LEAF=W3:x,W2:y,W1:z (missing '=' in '{}')",
            spec
        )
    })?;
    let leaf = validate_leaf(leaf_raw.trim())?;
    let (mut w1, mut w2, mut w3) = (None, None, None);
    for tok in rhs.split(',') {
        let (w, sv) = tok
            .split_once(':')
            .ok_or_else(|| anyhow!("expected Wn:SIL in '{}'", tok.trim()))?;
        let sil = Sil::parse(sv)
            .ok_or_else(|| anyhow!("'{}' is not a SIL (use 1..4, a, b, none)", sv.trim()))?;
        match w.trim().to_uppercase().as_str() {
            "W1" => w1 = Some(sil),
            "W2" => w2 = Some(sil),
            "W3" => w3 = Some(sil),
            other => return Err(anyhow!("unknown probability '{}' (want W1/W2/W3)", other)),
        }
    }
    match (w1, w2, w3) {
        (Some(w1), Some(w2), Some(w3)) => Ok((leaf, CalibrationRow { w1, w2, w3 })),
        _ => Err(anyhow!(
            "leaf {} needs all of W1, W2, W3 (e.g. W3:4,W2:3,W1:2)",
            leaf
        )),
    }
}

/// Confirm a leaf string names a real (C,F,P) combination and return its
/// canonical form.
fn validate_leaf(raw: &str) -> Result<String> {
    let parts: Vec<&str> = raw.split('/').collect();
    if parts.len() != 3 {
        return Err(anyhow!("leaf must be C_x/F_x/P_x, got '{}'", raw));
    }
    let c = parse_c(parts[0])?;
    let f = parse_f(parts[1])?;
    let p = parse_p(parts[2])?;
    Ok(calibration_leaf(c, f, p))
}

fn parse_c(s: &str) -> Result<Consequence> {
    Ok(match s.trim().to_uppercase().as_str() {
        "C_A" => Consequence::Ca,
        "C_B" => Consequence::Cb,
        "C_C" => Consequence::Cc,
        "C_D" => Consequence::Cd,
        o => return Err(anyhow!("bad consequence '{}' (C_A..C_D)", o)),
    })
}
fn parse_f(s: &str) -> Result<Frequency> {
    Ok(match s.trim().to_uppercase().as_str() {
        "F_A" => Frequency::Fa,
        "F_B" => Frequency::Fb,
        o => return Err(anyhow!("bad frequency '{}' (F_A/F_B)", o)),
    })
}
fn parse_p(s: &str) -> Result<Avoidance> {
    Ok(match s.trim().to_uppercase().as_str() {
        "P_A" => Avoidance::Pa,
        "P_B" => Avoidance::Pb,
        o => return Err(anyhow!("bad avoidance '{}' (P_A/P_B)", o)),
    })
}

fn sil_str(s: Option<Sil>) -> String {
    s.map(|s| s.as_str().to_string())
        .unwrap_or_else(|| "—".to_string())
}

fn parse_w(s: &str) -> Result<Probability> {
    Ok(match s.trim().to_uppercase().as_str() {
        "W1" => Probability::W1,
        "W2" => Probability::W2,
        "W3" => Probability::W3,
        o => return Err(anyhow!("bad probability '{}' (W1/W2/W3)", o)),
    })
}

// ---------------------------------------------------------------------------
// REQ-0156: read-only safety-graph impact analysis
// ---------------------------------------------------------------------------

/// Normalise a typed id of a given family to canonical `PREFIX-NNNN`.
fn norm_id(prefix: &str, raw: &str) -> String {
    let t = raw.trim();
    let up = t.to_uppercase();
    let digits = if let Some(rest) = up.strip_prefix(&format!("{}-", prefix)) {
        rest.to_string()
    } else if t.chars().all(|c| c.is_ascii_digit()) && !t.is_empty() {
        t.to_string()
    } else {
        return up;
    };
    match digits.parse::<u32>() {
        Ok(n) => format!("{}-{:04}", prefix, n),
        Err(_) => up,
    }
}

/// Split a `LHS=RHS` spec, erroring with the flag name on a missing `=`.
fn split_eq(spec: &str, flag: &str) -> Result<(String, String)> {
    spec.split_once('=')
        .map(|(a, b)| (a.trim().to_string(), b.trim().to_string()))
        .ok_or_else(|| anyhow!("{} expects LHS=RHS (missing '=' in '{}')", flag, spec))
}

/// SIL snapshot of every safety artifact: HAZ → required, SF → allocated,
/// SR → inherited.
fn sil_snapshot(p: &Project) -> std::collections::BTreeMap<String, Option<Sil>> {
    let mut m = std::collections::BTreeMap::new();
    for (id, h) in &p.hazards {
        m.insert(id.clone(), p.required_sil(h));
    }
    for (id, sf) in &p.safety_functions {
        m.insert(id.clone(), p.allocated_sil(sf));
    }
    for (id, sr) in &p.safety_requirements {
        m.insert(id.clone(), p.inherited_sil(sr));
    }
    m
}

/// REQ-0156: report which safety artifacts' derived SIL a proposed change
/// would move, without applying it. The edit is staged on a clone; the
/// on-disk project is never written.
pub fn impact(args: ImpactArgs, file: &Option<PathBuf>) -> Result<()> {
    let path = resolve_path(file);
    let project = storage::load(&path)?;
    let before = sil_snapshot(&project);
    let mut proposed = project.clone();
    let mut described: Vec<String> = Vec::new();

    if let Some(spec) = &args.calibrate {
        let (leaf, row) = parse_set(spec)?;
        let cfg = proposed.config.get_or_insert_with(ProjectConfig::default);
        let safety = cfg.safety.get_or_insert_with(SafetyConfig::default);
        let map = safety.calibration.get_or_insert_with(Default::default);
        map.insert(leaf.clone(), row);
        described.push(format!("calibrate {}", leaf));
    }
    if let Some(spec) = &args.mitigate {
        let (sf_raw, haz_raw) = split_eq(spec, "--mitigate")?;
        let sf = norm_id("SF", &sf_raw);
        let haz = norm_id("HAZ", &haz_raw);
        if !project.hazards.contains_key(&haz) {
            return Err(anyhow!("no such hazard: {}", haz_raw));
        }
        let sf_obj = proposed
            .safety_functions
            .get_mut(&sf)
            .ok_or_else(|| anyhow!("no such safety function: {}", sf_raw))?;
        sf_obj.links.push(Link {
            kind: LinkKind::Mitigates,
            target: haz.clone(),
        });
        described.push(format!("{} mitigates {}", sf, haz));
    }
    if let Some(spec) = &args.realize {
        let (sr_raw, sf_raw) = split_eq(spec, "--realize")?;
        let sr = norm_id("SR", &sr_raw);
        let sf = norm_id("SF", &sf_raw);
        if !project.safety_functions.contains_key(&sf) {
            return Err(anyhow!("no such safety function: {}", sf_raw));
        }
        let sr_obj = proposed
            .safety_requirements
            .get_mut(&sr)
            .ok_or_else(|| anyhow!("no such safety requirement: {}", sr_raw))?;
        sr_obj.links.push(Link {
            kind: LinkKind::Realizes,
            target: sf.clone(),
        });
        described.push(format!("{} realizes {}", sr, sf));
    }
    if let Some(spec) = &args.assess {
        let (haz_raw, params) = split_eq(spec, "--assess")?;
        let haz = norm_id("HAZ", &haz_raw);
        let parts: Vec<&str> = params.split('/').collect();
        if parts.len() != 4 {
            return Err(anyhow!(
                "--assess expects HAZ-NNNN=C_x/F_x/P_x/Wn (got '{}')",
                params
            ));
        }
        let c = parse_c(parts[0])?;
        let f = parse_f(parts[1])?;
        let p = parse_p(parts[2])?;
        let w = parse_w(parts[3])?;
        let h = proposed
            .hazards
            .get_mut(&haz)
            .ok_or_else(|| anyhow!("no such hazard: {}", haz_raw))?;
        h.consequence = Some(c);
        h.frequency = Some(f);
        h.avoidance = Some(p);
        h.probability = Some(w);
        described.push(format!("assess {}", haz));
    }

    if described.is_empty() {
        return Err(anyhow!(
            "nothing to analyse — pass at least one of --calibrate, --mitigate, --realize, --assess"
        ));
    }

    let after = sil_snapshot(&proposed);
    let mut changes: Vec<(String, Option<Sil>, Option<Sil>)> = Vec::new();
    for (id, b) in &before {
        let a = after.get(id).copied().unwrap_or(None);
        if *b != a {
            changes.push((id.clone(), *b, a));
        }
    }
    changes.sort_by(|x, y| x.0.cmp(&y.0));

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "proposed": described,
                "changes": changes.iter().map(|(id, b, a)| serde_json::json!({
                    "id": id,
                    "before": b.map(|s| s.as_str()),
                    "after": a.map(|s| s.as_str()),
                })).collect::<Vec<_>>(),
                "applied": false,
            }))?
        );
        return Ok(());
    }

    println!("Impact of proposed change ({}):", described.join(", "));
    if changes.is_empty() {
        println!("  no derived SIL changes.");
    } else {
        for (id, b, a) in &changes {
            println!("  {:<9}  {} → {}", id, sil_str(*b), sil_str(*a));
        }
    }
    println!("\n(no changes written — `req impact` is a read-only preview)");
    Ok(())
}

fn atty_stdin() -> bool {
    use std::io::IsTerminal;
    std::io::stdin().is_terminal()
}

#[cfg(test)]
mod tests {
    use super::*;

    // REQ-0144: the agreement a user must accept before any functional-safety
    // feature is enabled disclaims all developer liability AND states that req
    // is a research tool not intended for real safety applications.
    #[test]
    fn req_0144_disclaimer_states_no_liability_and_research_scope() {
        let d = DISCLAIMER.to_lowercase();
        assert!(
            d.contains("no liability whatsoever"),
            "agreement must disclaim all developer liability: {DISCLAIMER}"
        );
        assert!(
            d.contains("research"),
            "agreement must state req is a research tool: {DISCLAIMER}"
        );
        assert!(
            d.contains("real safety applications"),
            "agreement must state it is not for real safety applications: {DISCLAIMER}"
        );
    }

    // REQ-0144: the accepted disclaimer version is embedded so a substance
    // change (version bump) forces re-acceptance.
    #[test]
    fn req_0144_disclaimer_version_is_current() {
        assert_eq!(crate::model::SAFETY_DISCLAIMER_VERSION, "2");
    }
}
