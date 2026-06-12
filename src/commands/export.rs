// Implements REQ-0014 (markdown / json / csv / html exports).
// REQ-0136: the markdown/HTML export grows a HARA section (hazard
// analysis and risk assessment) when a project carries safety artifacts.
use anyhow::Result;
use std::fs;
use std::path::PathBuf;

use crate::cli::{ExportArgs, ExportFormat};
use crate::model::{LinkKind, Project, Requirement, SafetyFunction, SafetyRequirement, Sil, Status};
use crate::storage::load_resolved;

pub fn run(args: ExportArgs, file: &Option<PathBuf>) -> Result<()> {
    let (_, project) = load_resolved(file)?;

    let output = match args.format {
        ExportFormat::Markdown => to_markdown(&project),
        ExportFormat::Json => serde_json::to_string_pretty(&project)?,
        ExportFormat::Csv => to_csv(&project)?,
        ExportFormat::Html => to_html(&project),
    };

    if args.output == "-" {
        print!("{}", output);
    } else {
        fs::write(&args.output, output)?;
        eprintln!("Wrote {}", args.output);
    }
    Ok(())
}

pub fn to_markdown(p: &Project) -> String {
    let mut s = String::new();
    s.push_str(&format!("# {}\n\n", p.name));
    s.push_str(&format!(
        "_{} requirement(s). Generated {}._\n\n",
        p.requirements.len(),
        chrono::Utc::now().format("%Y-%m-%d %H:%M UTC")
    ));
    for r in p.requirements.values() {
        s.push_str(&fmt_md(r));
        s.push_str("\n---\n\n");
    }
    s.push_str(&safety_markdown(p));
    s
}

/// REQ-0136: render the HARA (hazard analysis and risk assessment) when a
/// project carries safety artifacts. An overview table plus a per-hazard
/// safety case, so a human reviewer can sign off the whole chain from a
/// single document. Returns empty when there are no hazards.
fn safety_markdown(p: &Project) -> String {
    if p.hazards.is_empty() {
        return String::new();
    }
    let sil = |s: Option<Sil>| s.map(|s| s.as_str().to_string()).unwrap_or_else(|| "—".into());
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
    s.push_str("| Hazard | Harm | C/F/P/W | Required SIL | Allocated SIL | SRs verified | Case |\n");
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
        let adequate = match (h.required_sil(), allocated) {
            (Some(r), Some(a)) => a.rank() >= r.rank(),
            (Some(_), None) => false,
            (None, _) => true,
        };
        let complete = adequate && total > 0 && verified == total && !sfs.is_empty();
        let cfpw = match (h.consequence, h.frequency, h.avoidance, h.probability) {
            (Some(c), Some(f), Some(a), Some(w)) => {
                format!("{}·{}·{}·{}", c.as_str(), f.as_str(), a.as_str(), w.as_str())
            }
            _ => "—".into(),
        };
        s.push_str(&format!(
            "| {} {} | {} | {} | {} | {} | {}/{} | {} |\n",
            id,
            md_cell(&h.title),
            md_cell(&h.harm),
            cfpw,
            sil(h.required_sil()),
            sil(allocated),
            verified,
            total,
            if complete { "✓ complete" } else { "⚠ incomplete" },
        ));
    }
    s.push('\n');

    // Per-hazard safety case.
    s.push_str("## Safety cases\n\n");
    for (id, h) in &p.hazards {
        s.push_str(&format!("### {} — {}\n\n", id, h.title));
        s.push_str(&format!("- **Harm.** {}\n", h.harm));
        if !h.operating_context.is_empty() {
            s.push_str(&format!("- **Operating context.** {}\n", h.operating_context));
        }
        s.push_str(&format!(
            "- **Risk.** {} → required **{}**\n",
            match (h.consequence, h.frequency, h.avoidance, h.probability) {
                (Some(c), Some(f), Some(a), Some(w)) =>
                    format!("{} · {} · {} · {}", c.as_str(), f.as_str(), a.as_str(), w.as_str()),
                _ => "not yet assessed".into(),
            },
            sil(h.required_sil())
        ));
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
                if sf.safe_state.is_empty() { "—" } else { &sf.safe_state }
            ));
            for sr in p
                .safety_requirements
                .values()
                .filter(|sr| sr_realizes(sr, &sf.id))
            {
                let mark = if matches!(sr.status, Status::Verified) { "✓" } else { "⚠" };
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

fn fmt_md(r: &Requirement) -> String {
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
    if !r.links.is_empty() {
        s.push_str("**Links:**\n\n");
        for l in &r.links {
            s.push_str(&format!("- _{}_ → `{}`\n", l.kind.as_str(), l.target));
        }
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

pub fn to_html(p: &Project) -> String {
    let body = to_markdown(p);
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
