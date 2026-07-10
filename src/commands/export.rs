// Implements REQ-0014 (markdown / json / csv / html exports).
// REQ-0136: the markdown/HTML export grows a HARA section (hazard
// analysis and risk assessment) when a project carries safety artifacts.
use anyhow::Result;
use std::fs;
use std::path::PathBuf;

use crate::cli::{ExportArgs, ExportFormat};
use crate::model::{
    LinkKind, Project, Requirement, SafetyFunction, SafetyRequirement, Sil, Status,
};
use crate::storage::load_resolved;

pub fn run(args: ExportArgs, file: &Option<PathBuf>) -> Result<()> {
    let (path, project) = load_resolved(file)?;

    let output = match args.format {
        ExportFormat::Markdown => to_markdown_ctx(&project, Some(&path)),
        ExportFormat::Json => serde_json::to_string_pretty(&project)?,
        ExportFormat::Csv => to_csv(&project)?,
        ExportFormat::Html => to_html_ctx(&project, Some(&path)),
    };

    if args.output == "-" {
        print!("{}", output);
    } else {
        fs::write(&args.output, output)?;
        eprintln!("Wrote {}", args.output);
    }
    Ok(())
}

/// REQ-0209/0210/0211: the context-aware renderer. `project_path` (the
/// project.req file) enables the provenance/staleness probe and the
/// functional-safety enablement check; `None` renders without them.
pub fn to_markdown_ctx(p: &Project, project_path: Option<&std::path::Path>) -> String {
    let root = source_root(project_path);
    let mut s = String::new();
    s.push_str(&format!("# {}\n\n", p.name));
    s.push_str(&format!(
        "_{} requirement(s). Generated {}._\n\n",
        p.requirements.len(),
        chrono::Utc::now().format("%Y-%m-%d %H:%M UTC")
    ));
    for r in p.requirements.values() {
        s.push_str(&fmt_md(r, root.as_deref()));
        s.push_str("\n---\n\n");
    }
    s.push_str(&verification_status_markdown(p, root.as_deref()));
    s.push_str(&safety_markdown(p, project_path));
    s
}

/// The directory linked source files are hashed against (the project file's
/// parent), for the provenance staleness probe.
fn source_root(project_path: Option<&std::path::Path>) -> Option<std::path::PathBuf> {
    project_path.map(|f| match f.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => std::path::PathBuf::from("."),
    })
}

/// REQ-0210: the project-level verification-status roll-up — the same view
/// `req verification report` prints: provenance counts over every Verified
/// item, the unverified surface by dossier stage, and each safety
/// requirement's standing.
fn verification_status_markdown(p: &Project, root: Option<&std::path::Path>) -> String {
    use crate::commands::provenance::{provenance_report, sr_standing, Provenance};
    let rows = provenance_report(p, root);
    let unverified = crate::commands::verification::unverified_rows(p);
    let count = |pr: Provenance| rows.iter().filter(|r| r.provenance == pr).count();

    let mut s = String::new();
    s.push_str("# Verification status\n\n");
    s.push_str(&format!(
        "_{} verified item(s), {} unverified._\n\n",
        rows.len(),
        unverified.len()
    ));
    s.push_str("| Provenance | Count |\n|---|---|\n");
    for (label, n) in [
        ("genuine", count(Provenance::Genuine)),
        ("stale", count(Provenance::Stale)),
        ("unconfirmed", count(Provenance::Unconfirmed)),
        ("exempt:backfilled", count(Provenance::ExemptBackfilled)),
        ("exempt:no-dossier", count(Provenance::ExemptNoDossier)),
        ("ungated", count(Provenance::Ungated)),
    ] {
        s.push_str(&format!("| {} | {} |\n", label, n));
    }
    s.push('\n');
    let not_genuine = rows.len() - count(Provenance::Genuine);
    if not_genuine > 0 {
        s.push_str(&format!(
            "> ⚠ {} of {} verified item(s) do NOT rest on a genuine verification dossier.\n\n",
            not_genuine,
            rows.len()
        ));
    }
    let flagged: Vec<_> = rows.iter().filter(|r| !r.provenance.is_genuine()).collect();
    if !flagged.is_empty() {
        s.push_str("**Not genuine:**\n\n");
        for r in flagged {
            s.push_str(&format!(
                "- {} — {} ({})\n",
                r.id,
                r.provenance.as_str(),
                r.family
            ));
        }
        s.push('\n');
    }
    if !unverified.is_empty() {
        s.push_str("**Unverified (by dossier stage):**\n\n");
        for (id, fam, stage) in &unverified {
            s.push_str(&format!("- {} — {} ({})\n", id, stage, fam));
        }
        s.push('\n');
    }
    if !p.safety_requirements.is_empty() {
        s.push_str("**Safety-requirement standings:**\n\n");
        let mut srs: Vec<_> = p.safety_requirements.values().collect();
        srs.sort_by(|a, b| a.id.cmp(&b.id));
        for sr in srs {
            s.push_str(&format!("- {} — {}\n", sr.id, sr_standing(sr, root)));
        }
        s.push('\n');
    }
    s
}

/// REQ-0136: render the HARA (hazard analysis and risk assessment) when a
/// project carries safety artifacts. An overview table plus a per-hazard
/// safety case, so a human reviewer can sign off the whole chain from a
/// single document. Returns empty when there are no hazards.
fn safety_markdown(p: &Project, project_path: Option<&std::path::Path>) -> String {
    // REQ-0211: explicit empty state — a project with functional safety
    // disabled or unpopulated says so instead of omitting the section.
    if p.hazards.is_empty() && p.safety_functions.is_empty() && p.safety_requirements.is_empty() {
        let enabled = project_path
            .map(crate::commands::safety_gov::is_enabled)
            .unwrap_or(false);
        return format!(
            "# Functional safety\n\n_{}_\n",
            if enabled {
                "Enabled, but the project has no hazards, safety functions, or safety requirements yet."
            } else {
                "Disabled — no safety disclaimer accepted and no safety artifacts."
            }
        );
    }
    let root = source_root(project_path);
    let sil = |s: Option<Sil>| {
        s.map(|s| s.as_str().to_string())
            .unwrap_or_else(|| "—".into())
    };
    let mut s = String::new();
    s.push_str("# Functional safety (IEC 61508)\n\n");
    s.push_str(&format!(
        "_{} hazard(s), {} safety function(s), {} safety requirement(s)._\n\n",
        p.hazards.len(),
        p.safety_functions.len(),
        p.safety_requirements.len()
    ));
    // REQ-0135: honesty disclaimer — this document traces a candidate
    // classification, it is not an assurance argument.
    s.push_str(
        "> **Disclaimer.** `req` computes a *candidate* SIL from the inputs below and \
checks *traceability* only. It is **not** a qualified safety tool (IEC 61508-3 §7.4.4), \
does not model achieved integrity (PFD/PFH, diagnostic coverage, SIL decomposition), and \
does not assure that residual risk is acceptable. \"Complete\" means the chain is linked \
and verified — not that the design is safe. The safety determination remains the \
engineer's responsibility.\n\n",
    );

    // HARA overview table.
    s.push_str("## Hazard analysis & risk assessment\n\n");
    s.push_str(
        "| Hazard | Harm | C/F/P/W | Required SIL | Allocated SIL | SRs verified | Case |\n",
    );
    s.push_str("|---|---|---|---|---|---|---|\n");
    for (id, h) in &p.hazards {
        let sfs: Vec<&SafetyFunction> = p
            .safety_functions
            .values()
            .filter(|sf| sf_mitigates(sf, id))
            .collect();
        let allocated = sfs
            .iter()
            .filter_map(|sf| p.allocated_sil(sf))
            .max_by_key(|s| s.rank());
        let (verified, total) = sr_tally(p, &sfs);
        let adequate = match (p.required_sil(h), allocated) {
            (Some(r), Some(a)) => a.rank() >= r.rank(),
            (Some(_), None) => false,
            (None, _) => true,
        };
        let complete = adequate && total > 0 && verified == total && !sfs.is_empty();
        let cfpw = match (h.consequence, h.frequency, h.avoidance, h.probability) {
            (Some(c), Some(f), Some(a), Some(w)) => {
                format!(
                    "{}·{}·{}·{}",
                    c.as_str(),
                    f.as_str(),
                    a.as_str(),
                    w.as_str()
                )
            }
            _ => "—".into(),
        };
        s.push_str(&format!(
            "| {} {} | {} | {} | {} | {} | {}/{} | {} |\n",
            id,
            md_cell(&h.title),
            md_cell(&h.harm),
            cfpw,
            sil(p.required_sil(h)),
            sil(allocated),
            verified,
            total,
            if complete {
                "✓ complete"
            } else {
                "⚠ incomplete"
            },
        ));
    }
    s.push('\n');

    // REQ-0136: per-hazard safety-case section of the HARA export.
    // Per-hazard safety case.
    s.push_str("## Safety cases\n\n");
    for (id, h) in &p.hazards {
        s.push_str(&format!("### {} — {}\n\n", id, h.title));
        s.push_str(&format!("- **Harm.** {}\n", h.harm));
        if !h.operating_context.is_empty() {
            s.push_str(&format!(
                "- **Operating context.** {}\n",
                h.operating_context
            ));
        }
        s.push_str(&format!(
            "- **Risk.** {} → required **{}**\n",
            match (h.consequence, h.frequency, h.avoidance, h.probability) {
                (Some(c), Some(f), Some(a), Some(w)) => format!(
                    "{} · {} · {} · {}",
                    c.as_str(),
                    f.as_str(),
                    a.as_str(),
                    w.as_str()
                ),
                _ => "not yet assessed".into(),
            },
            sil(p.required_sil(h))
        ));
        // REQ-0211: the hazard's mitigation-adequacy dossier — plan, per-SF
        // walk-through, residual-risk statement, verdict, and co-sign.
        match &h.adequacy {
            None => s.push_str("- **Adequacy.** _no adequacy dossier yet._\n"),
            Some(a) => {
                s.push_str(&format!(
                    "- **Adequacy.** {} — {}\n",
                    a.verdict
                        .map(|v| v.as_str().to_string())
                        .unwrap_or_else(|| "in progress".into()),
                    match &a.human_confirmation {
                        Some(c) =>
                            format!("co-signed by {} @ {}", c.actor, c.at.format("%Y-%m-%d")),
                        None => "awaiting human co-sign".into(),
                    }
                ));
                if !a.statement.is_empty() {
                    s.push_str(&format!("  - _Residual risk._ {}\n", a.statement));
                }
                for c in &a.coverage {
                    s.push_str(&format!("  - _{}_: {}\n", c.target, c.note));
                }
            }
        }
        let sfs: Vec<&SafetyFunction> = p
            .safety_functions
            .values()
            .filter(|sf| sf_mitigates(sf, id))
            .collect();
        if sfs.is_empty() {
            s.push_str("- **Mitigation.** _none_\n");
        }
        for sf in &sfs {
            s.push_str(&format!(
                "\n  **{} — {}** (allocated {}, {})  \n  _safe state:_ {}\n",
                sf.id,
                sf.title,
                sil(p.allocated_sil(sf)),
                sf.status.as_str(),
                if sf.safe_state.is_empty() {
                    "—"
                } else {
                    &sf.safe_state
                }
            ));
            // REQ-0136: render each realizing safety requirement under its SF.
            for sr in p
                .safety_requirements
                .values()
                .filter(|sr| sr_realizes(sr, &sf.id))
            {
                let mark = if matches!(sr.status, Status::Verified) {
                    "✓"
                } else {
                    "⚠"
                };
                s.push_str(&format!(
                    "  - {} {} {} — _{}_ (inherits {})\n",
                    mark,
                    sr.id,
                    md_cell(&sr.title),
                    sr.status.as_str(),
                    sil(p.inherited_sil(sr))
                ));
            }
        }
        s.push_str("\n---\n\n");
    }

    // REQ-0211: the full verification dossier per safety function and safety
    // requirement — including the human co-sign state (REQ-0145) — so the
    // exported document carries the same evidence the browser shows.
    s.push_str("## Safety verification dossiers\n\n");
    let mut sfs: Vec<_> = p.safety_functions.values().collect();
    sfs.sort_by(|a, b| a.id.cmp(&b.id));
    for sf in sfs {
        s.push_str(&format!(
            "### {} — {} ({})\n\n",
            sf.id,
            sf.title,
            sf.status.as_str()
        ));
        s.push_str(&dossier_md(
            sf.verification.as_ref(),
            &sf.id,
            root.as_deref(),
            &[],
        ));
    }
    let mut srs: Vec<_> = p.safety_requirements.values().collect();
    srs.sort_by(|a, b| a.id.cmp(&b.id));
    for sr in &srs {
        s.push_str(&format!(
            "### {} — {} ({})\n\n",
            sr.id,
            sr.title,
            sr.status.as_str()
        ));
        s.push_str(&dossier_md(
            sr.verification.as_ref(),
            &sr.id,
            root.as_deref(),
            &sr.tests,
        ));
    }

    // REQ-0211: walkthrough / acknowledgement state per safety requirement.
    if !srs.is_empty() {
        s.push_str("## Walkthrough acknowledgements\n\n");
        let head = crate::commands::safety_gov::head_sha();
        for sr in &srs {
            let state = match sr.walkthrough.as_ref() {
                None => "never acknowledged".to_string(),
                Some(a) if a.objected => format!(
                    "OBJECTION by {} @ {}{}",
                    a.reviewer,
                    a.at.format("%Y-%m-%d"),
                    a.note
                        .as_deref()
                        .map(|n| format!(" — {}", n))
                        .unwrap_or_default()
                ),
                Some(a) if crate::commands::safety_gov::ack_is_fresh(Some(a), &head) => format!(
                    "acknowledged by {} @ {}",
                    a.reviewer,
                    a.at.format("%Y-%m-%d")
                ),
                Some(a) => format!(
                    "acknowledgement STALE (by {} @ commit `{}`; chain changed since)",
                    a.reviewer,
                    &a.commit[..a.commit.len().min(9)]
                ),
            };
            s.push_str(&format!("- {} — {}\n", sr.id, state));
        }
        s.push('\n');
    }

    // REQ-0211: the risk-graph calibration in force and any leaf overrides.
    s.push_str("## SIL calibration\n\n");
    let label = p
        .config
        .as_ref()
        .and_then(|c| c.safety.as_ref())
        .and_then(|c| c.calibration_label.as_deref())
        .unwrap_or("IEC 61508-5 Annex D (default)");
    match p.calibration() {
        None => s.push_str(&format!(
            "_{} — no leaf overrides; every leaf uses the Annex D worked-example default._\n\n",
            label
        )),
        Some(table) => {
            s.push_str(&format!(
                "_{} — {} leaf override(s)._\n\n",
                label,
                table.len()
            ));
            s.push_str("| Leaf (C/F/P) | W3 | W2 | W1 |\n|---|---|---|---|\n");
            for (leaf, row) in table {
                s.push_str(&format!(
                    "| `{}` | {} | {} | {} |\n",
                    leaf,
                    row.w3.as_str(),
                    row.w2.as_str(),
                    row.w1.as_str()
                ));
            }
            s.push('\n');
        }
    }
    s
}

fn sf_mitigates(sf: &SafetyFunction, haz_id: &str) -> bool {
    sf.links
        .iter()
        .any(|l| l.kind == LinkKind::Mitigates && l.target == haz_id)
}

fn sr_realizes(sr: &SafetyRequirement, sf_id: &str) -> bool {
    sr.links
        .iter()
        .any(|l| l.kind == LinkKind::Realizes && l.target == sf_id)
}

/// (verified, total) safety requirements across the given functions.
fn sr_tally(p: &Project, sfs: &[&SafetyFunction]) -> (usize, usize) {
    let mut verified = 0;
    let mut total = 0;
    for sf in sfs {
        for sr in p
            .safety_requirements
            .values()
            .filter(|sr| sr_realizes(sr, &sf.id))
        {
            total += 1;
            if matches!(sr.status, Status::Verified) {
                verified += 1;
            }
        }
    }
    (verified, total)
}

/// Escape a string for a single markdown table cell (pipes break the row).
fn md_cell(s: &str) -> String {
    s.replace('|', "\\|").replace('\n', " ")
}

fn fmt_md(r: &Requirement, root: Option<&std::path::Path>) -> String {
    let mut s = String::new();
    s.push_str(&format!("## {} — {}\n\n", r.id, r.title));
    s.push_str(&format!(
        "- **Kind:** {}  \n- **Priority:** {}  \n- **Status:** {}\n",
        r.kind.as_str(),
        r.priority.as_str(),
        r.status.as_str()
    ));
    if !r.tags.is_empty() {
        s.push_str(&format!("- **Tags:** {}\n", r.tags.join(", ")));
    }
    s.push_str(&format!("\n**Statement.** {}\n\n", r.statement));
    s.push_str(&format!("**Rationale.** {}\n\n", r.rationale));
    if !r.acceptance.is_empty() {
        s.push_str("**Acceptance criteria:**\n\n");
        for ac in &r.acceptance {
            s.push_str(&format!("- {}\n", ac));
        }
        s.push('\n');
    }
    // REQ-0209: the full verification dossier and test records — the export
    // mirrors what `req verification show` prints, plus provenance.
    s.push_str(&dossier_md(r.verification.as_ref(), &r.id, root, &r.tests));
    if !r.links.is_empty() {
        s.push_str("**Links:**\n\n");
        for l in &r.links {
            s.push_str(&format!("- _{}_ → `{}`\n", l.kind.as_str(), l.target));
        }
    }
    s
}

/// REQ-0209: render a verification dossier (and any test records) as
/// markdown. Shared by ordinary requirements, safety requirements, and
/// safety functions.
fn dossier_md(
    v: Option<&crate::model::Verification>,
    id: &str,
    root: Option<&std::path::Path>,
    tests: &[crate::model::TestRecord],
) -> String {
    use crate::commands::provenance::classify;
    let mut s = String::new();
    match v {
        None => {
            s.push_str("**Verification.** _no dossier recorded._\n\n");
        }
        Some(v) => {
            s.push_str(&format!(
                "**Verification** — verdict **{}**, provenance `{}`\n\n",
                v.verdict
                    .map(|o| o.as_str().to_uppercase())
                    .unwrap_or_else(|| "(not concluded)".into()),
                classify(Some(v), root, id).as_str()
            ));
            let act = |label: &str, a: &Option<crate::model::VerificationActivity>| match a {
                None => format!("- **{}.** _(pending)_\n", label),
                Some(a) => {
                    let refs = if a.references.is_empty() {
                        String::new()
                    } else {
                        format!(" _(refs: {})_", a.references.join(", "))
                    };
                    format!(
                        "- **{}.** {} — {}{}\n",
                        label,
                        a.outcome.as_str().to_uppercase(),
                        a.summary,
                        refs
                    )
                }
            };
            if !v.plan.is_empty() {
                s.push_str(&format!("- **Plan.** {}\n", v.plan));
            }
            s.push_str(&act("Analysis", &v.analysis));
            s.push_str(&act("Testing", &v.testing));
            if let Some(st) = &v.statement {
                s.push_str(&format!("- **Statement.** {}\n", st));
            }
            match &v.human_confirmation {
                Some(c) => s.push_str(&format!(
                    "- **Co-sign.** {} @ {}\n",
                    c.actor,
                    c.at.format("%Y-%m-%d %H:%M UTC")
                )),
                None => s.push_str("- **Co-sign.** _awaiting human co-sign._\n"),
            }
            if v.exempt {
                s.push_str(&format!(
                    "- **Exemption.** {}\n",
                    match v.exemption_kind {
                        Some(crate::model::ExemptionKind::NoDossier) => "no-dossier waiver",
                        Some(crate::model::ExemptionKind::Backfilled) => "backfilled",
                        None => "exempt (legacy)",
                    }
                ));
            }
            if let Some(hash) = &v.content_hash {
                s.push_str(&format!(
                    "- **Anchor.** `{}` over {} file(s){}\n",
                    &hash[..hash.len().min(12)],
                    v.linked_files.as_ref().map(|f| f.len()).unwrap_or(0),
                    v.concluded_commit
                        .as_deref()
                        .map(|c| format!(" @ `{}`", &c[..c.len().min(9)]))
                        .unwrap_or_default()
                ));
            }
            s.push('\n');
        }
    }
    if !tests.is_empty() {
        s.push_str("**Test records:**\n\n");
        for t in tests {
            let ext = t
                .external
                .as_ref()
                .map(|e| {
                    format!(
                        " ext={}{}",
                        e.system,
                        e.environment
                            .as_deref()
                            .map(|v| format!("/{}", v))
                            .unwrap_or_default()
                    )
                })
                .unwrap_or_default();
            s.push_str(&format!(
                "- {} {} [{}] commit=`{}` actor={}{} — {}\n",
                t.at.format("%Y-%m-%d %H:%M"),
                t.outcome.as_str().to_uppercase(),
                t.kind.as_str(),
                &t.commit[..t.commit.len().min(9)],
                t.actor,
                ext,
                t.notes
            ));
        }
        s.push('\n');
    }
    s
}

pub fn to_csv(p: &Project) -> Result<String> {
    let mut out = String::from("id,title,kind,priority,status,tags,statement\n");
    for r in p.requirements.values() {
        out.push_str(&format!(
            "{},{},{},{},{},{},{}\n",
            csv_field(&r.id),
            csv_field(&r.title),
            r.kind.as_str(),
            r.priority.as_str(),
            r.status.as_str(),
            csv_field(&r.tags.join("|")),
            csv_field(&r.statement),
        ));
    }
    Ok(out)
}

fn csv_field(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

// REQ-0209/0211: the HTML export mirrors the context-aware markdown.
pub fn to_html_ctx(p: &Project, project_path: Option<&std::path::Path>) -> String {
    let body = to_markdown_ctx(p, project_path);
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>{}</title>\
        <style>body{{font-family:system-ui,sans-serif;max-width:48rem;margin:2rem auto;padding:0 1rem;line-height:1.5;color:#222}}\
        h1,h2{{border-bottom:1px solid #ddd;padding-bottom:.3rem}}code{{background:#f4f4f4;padding:.1rem .3rem;border-radius:3px}}</style>\
        </head><body><pre style=\"white-space:pre-wrap;font-family:inherit\">{}</pre></body></html>",
        html_escape(&p.name),
        html_escape(&body)
    )
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
