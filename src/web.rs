// Implements REQ-0016 (local web server for humans). Read-only by default
// in this minimum-viable form; mutation endpoints are deliberately absent
// until the locking story is designed.
use anyhow::{anyhow, Context, Result};
use axum::{
    extract::{Path as AxPath, State},
    http::StatusCode,
    response::{Html, Json},
    routing::get,
    Router,
};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use crate::cli::ServeArgs;
use crate::model::{
    Hazard, LinkKind, Project, Requirement, SafetyFunction, SafetyRequirement, Sil, Status,
};
use crate::storage::{self, resolve_path};

#[derive(Clone)]
struct AppState {
    file: PathBuf,
    read_only: bool,
}

pub fn run(args: ServeArgs, file: &Option<PathBuf>) -> Result<()> {
    let path = resolve_path(file);
    storage::load(&path).context("load project before binding socket")?;

    let state = AppState {
        file: path.clone(),
        read_only: args.read_only,
    };
    if !state.read_only {
        eprintln!(
            "note: write endpoints are not yet implemented; serving read-only regardless of --read-only"
        );
    }

    let app = Router::new()
        .route("/", get(index_html))
        .route("/r/:id", get(show_html))
        .route("/s/:id", get(safety_entity_html))
        .route("/safety", get(safety_html))
        .route("/api/list", get(api_list))
        .route("/api/r/:id", get(api_show))
        .route("/api/safety", get(api_safety))
        .with_state(Arc::new(state));

    let addr: SocketAddr = format!("{}:{}", args.host, args.port)
        .parse()
        .map_err(|e| anyhow!("invalid bind address: {}", e))?;

    println!("req serve: http://{} (Ctrl-C to stop)", addr);
    println!("  serving {}", path.display());
    println!("  read-only — every save goes through the CLI");

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("start tokio runtime")?;
    rt.block_on(async move {
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .with_context(|| format!("bind {}", addr))?;
        axum::serve(listener, app)
            .with_graceful_shutdown(shutdown())
            .await
            .context("serve")
    })
}

async fn shutdown() {
    let _ = tokio::signal::ctrl_c().await;
    eprintln!("\nreq serve: shutting down.");
}

fn load_project(state: &AppState) -> Result<Project, (StatusCode, String)> {
    storage::load(&state.file).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

async fn api_list(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<Requirement>>, (StatusCode, String)> {
    let project = load_project(&state)?;
    Ok(Json(project.requirements.into_values().collect()))
}

async fn api_show(
    State(state): State<Arc<AppState>>,
    AxPath(id): AxPath<String>,
) -> Result<Json<Requirement>, (StatusCode, String)> {
    let project = load_project(&state)?;
    match project.requirements.get(&id) {
        Some(r) => Ok(Json(r.clone())),
        None => Err((
            StatusCode::NOT_FOUND,
            format!("no such requirement: {}", id),
        )),
    }
}

async fn index_html(
    State(state): State<Arc<AppState>>,
) -> Result<Html<String>, (StatusCode, String)> {
    let project = load_project(&state)?;
    let mut rows = String::new();
    for r in project.requirements.values() {
        rows.push_str(&format!(
            "<tr><td><a href=\"/r/{id}\">{id}</a></td><td>{title}</td><td>{kind}</td><td>{pri}</td><td>{status}</td><td>{tags}</td></tr>",
            id = h(&r.id),
            title = h(&r.title),
            kind = r.kind.as_str(),
            pri = r.priority.as_str(),
            status = r.status.as_str(),
            tags = h(&r.tags.join(", ")),
        ));
    }
    let safety_link = if project.hazards.is_empty() {
        String::new()
    } else {
        format!(
            " &middot; <a href=\"/safety\">functional safety ({} hazard(s))</a>",
            project.hazards.len()
        )
    };
    Ok(Html(page(
        &format!("req — {}", h(&project.name)),
        &format!(
            "<h1>{name}</h1>\
             <p class=\"meta\">{count} requirement(s) &middot; served from <code>{path}</code> &middot; read-only{safety_link}</p>\
             <table><thead><tr><th>ID</th><th>Title</th><th>Kind</th><th>Pri</th><th>Status</th><th>Tags</th></tr></thead><tbody>{rows}</tbody></table>",
            name = h(&project.name),
            count = project.requirements.len(),
            path = h(&state.file.display().to_string()),
            safety_link = safety_link,
            rows = rows,
        ),
    )))
}

fn sil_s(s: Option<Sil>) -> String {
    s.map(|s| s.as_str().to_string())
        .unwrap_or_else(|| "—".into())
}

/// REQ-0134: read-only HARA-style web view of the functional-safety
/// artifacts, mirroring `req trace` / the markdown HARA export so a human
/// reviewer can read the whole safety case in a browser.
async fn safety_html(
    State(state): State<Arc<AppState>>,
) -> Result<Html<String>, (StatusCode, String)> {
    let project = load_project(&state)?;
    if project.hazards.is_empty() {
        return Ok(Html(page(
            "req — functional safety",
            "<p><a href=\"/\">&larr; index</a></p><h1>Functional safety</h1>\
             <p class=\"meta\">This project has no hazards.</p>",
        )));
    }
    let mitigates = |sf: &SafetyFunction, hid: &str| {
        sf.links
            .iter()
            .any(|l| l.kind == LinkKind::Mitigates && l.target == hid)
    };
    let realizes = |sr: &SafetyRequirement, sfid: &str| {
        sr.links
            .iter()
            .any(|l| l.kind == LinkKind::Realizes && l.target == sfid)
    };

    let mut rows = String::new();
    for (id, hz) in &project.hazards {
        let sfs: Vec<&SafetyFunction> = project
            .safety_functions
            .values()
            .filter(|sf| mitigates(sf, id))
            .collect();
        let allocated = sfs
            .iter()
            .filter_map(|sf| project.allocated_sil(sf))
            .max_by_key(|s| s.rank());
        let (mut verified, mut total) = (0usize, 0usize);
        for sf in &sfs {
            for sr in project
                .safety_requirements
                .values()
                .filter(|sr| realizes(sr, &sf.id))
            {
                total += 1;
                if matches!(sr.status, Status::Verified) {
                    verified += 1;
                }
            }
        }
        let adequate = match (project.required_sil(hz), allocated) {
            (Some(r), Some(a)) => a.rank() >= r.rank(),
            (Some(_), None) => false,
            (None, _) => true,
        };
        let complete = adequate && total > 0 && verified == total && !sfs.is_empty();
        let (sk, st) = haz_standing(hz);
        // REQ-0147: the hazard id links to its detail page so a reader can walk
        // into the full mitigation + verification chain; REQ-0205 adds the
        // adequacy standing badge and the verified-SR roll-up at a glance.
        rows.push_str(&format!(
            "<tr><td>{idlink}</td><td>{title}</td><td>{standing}</td><td>{req}</td><td>{alloc}</td><td>{v}/{t}</td><td>{verdict}</td></tr>",
            idlink = alink(id),
            title = h(&hz.title),
            standing = badge(sk, st),
            req = sil_s(project.required_sil(hz)),
            alloc = sil_s(allocated),
            v = verified,
            t = total,
            verdict = if complete { "&#10003; complete" } else { "&#9888; incomplete" },
        ));
    }
    // REQ-0205: a roll-up of how many safety requirements still await a co-sign.
    let awaiting = project
        .safety_requirements
        .values()
        .filter(|sr| crate::commands::provenance::sr_awaiting_cosign(sr))
        .count();
    let rollup = format!(
        "<p class=\"meta\">{nh} hazard(s) &middot; {nf} safety function(s) &middot; {nr} safety requirement(s){aw}</p>",
        nh = project.hazards.len(),
        nf = project.safety_functions.len(),
        nr = project.safety_requirements.len(),
        aw = if awaiting > 0 {
            format!(" &middot; <strong>{} awaiting human co-sign</strong>", awaiting)
        } else {
            String::new()
        },
    );
    let disclaimer = "<p class=\"meta\" style=\"border-left:3px solid #e0a800;padding-left:.6rem;\">\
        &#9888; req computes a <em>candidate</em> SIL from your inputs and checks <em>traceability</em> only. \
        It is not qualified per IEC 61508-3 &sect;7.4.4 and does not assure risk reduction; the table uses the \
        Annex&nbsp;D worked-example calibration. The safety determination remains the engineer's responsibility.</p>";
    Ok(Html(page(
        "req — functional safety",
        &format!(
            "<p><a href=\"/\">&larr; index</a></p>\
             <h1>Functional safety</h1>{disclaimer}{rollup}\
             <table><thead><tr><th>Hazard</th><th>Title</th><th>Standing</th><th>Required SIL</th><th>Allocated SIL</th><th>SRs verified</th><th>Trace</th></tr></thead><tbody>{rows}</tbody></table>",
            disclaimer = disclaimer,
            rollup = rollup,
            rows = rows,
        ),
    )))
}

#[derive(serde::Serialize)]
struct SafetyApi {
    hazards: Vec<crate::model::Hazard>,
    safety_functions: Vec<SafetyFunction>,
    safety_requirements: Vec<SafetyRequirement>,
}

async fn api_safety(
    State(state): State<Arc<AppState>>,
) -> Result<Json<SafetyApi>, (StatusCode, String)> {
    let project = load_project(&state)?;
    Ok(Json(SafetyApi {
        hazards: project.hazards.into_values().collect(),
        safety_functions: project.safety_functions.into_values().collect(),
        safety_requirements: project.safety_requirements.into_values().collect(),
    }))
}

async fn show_html(
    State(state): State<Arc<AppState>>,
    AxPath(id): AxPath<String>,
) -> Result<Html<String>, (StatusCode, String)> {
    let project = load_project(&state)?;
    let r = match project.requirements.get(&id) {
        Some(r) => r.clone(),
        None => {
            return Err((
                StatusCode::NOT_FOUND,
                format!("no such requirement: {}", id),
            ))
        }
    };
    let mut acc = String::new();
    for (i, a) in r.acceptance.iter().enumerate() {
        acc.push_str(&format!("<li>{}. {}</li>", i + 1, h(a)));
    }
    let mut links = String::new();
    // REQ-0147: a requirement's links resolve across families (a link target
    // may be a requirement or a safety entity), so navigation is unbroken.
    for l in &r.links {
        links.push_str(&format!(
            "<li><em>{}</em> &rarr; {}</li>",
            l.kind.as_str(),
            alink(&l.target)
        ));
    }
    let mut history = String::new();
    for hist in &r.history {
        history.push_str(&format!(
            "<li><code>{}</code> &middot; {} &middot; {}{}</li>",
            hist.at.format("%Y-%m-%d %H:%M"),
            h(&hist.actor),
            h(&hist.action),
            hist.reason
                .as_ref()
                .map(|reason| format!(" &mdash; <em>{}</em>", h(reason)))
                .unwrap_or_default(),
        ));
    }
    Ok(Html(page(
        &format!("{} — {}", r.id, h(&r.title)),
        &format!(
            "<p><a href=\"/\">&larr; index</a></p>\
             <h1>{id} <small>{title}</small></h1>\
             <ul class=\"meta\">\
                <li><strong>Kind:</strong> {kind}</li>\
                <li><strong>Priority:</strong> {pri}</li>\
                <li><strong>Status:</strong> {status}</li>\
                <li><strong>Tags:</strong> {tags}</li>\
             </ul>\
             <h2>Statement</h2><p>{stmt}</p>\
             <h2>Rationale</h2><p>{rat}</p>\
             {acc_block}\
             {links_block}\
             <h2>History</h2><ul>{history}</ul>",
            id = h(&r.id),
            title = h(&r.title),
            kind = r.kind.as_str(),
            pri = r.priority.as_str(),
            status = r.status.as_str(),
            tags = h(&r.tags.join(", ")),
            stmt = h(&r.statement),
            rat = h(&r.rationale),
            acc_block = if acc.is_empty() {
                String::new()
            } else {
                format!("<h2>Acceptance criteria</h2><ol>{}</ol>", acc)
            },
            links_block = if links.is_empty() {
                String::new()
            } else {
                format!("<h2>Links</h2><ul>{}</ul>", links)
            },
            history = history,
        ),
    )))
}

// REQ-0147: resolve any entity id to its web detail page so related entities
// are navigable by clicking — safety entities use /s/, requirements use /r/.
fn elink(id: &str) -> String {
    let up = id.to_uppercase();
    if up.starts_with("HAZ-") || up.starts_with("SF-") || up.starts_with("SR-") {
        format!("/s/{}", h(id))
    } else {
        format!("/r/{}", h(id))
    }
}

// A hyperlink to an entity's detail page, labelled with its id.
fn alink(id: &str) -> String {
    format!("<a href=\"{}\">{}</a>", elink(id), h(id))
}

// REQ-0205: a colored status pill.
fn badge(kind: &str, text: &str) -> String {
    format!("<span class=\"badge b-{}\">{}</span>", kind, h(text))
}

// REQ-0206: the derived sign-off basis card (reuses the same machine-checked
// chain summary the CLI `show` commands render).
fn signoff_card(lines: Vec<String>) -> String {
    if lines.is_empty() {
        return String::new();
    }
    let body: String = lines.iter().map(|l| h(l)).collect::<Vec<_>>().join("\n");
    format!(
        "<div class=\"card\"><h2>Sign-off basis</h2>\
         <pre style=\"white-space:pre-wrap;margin:0;font-family:inherit;\">{}</pre></div>",
        body
    )
}

// REQ-0205: one-word standing badge for a safety requirement (provenance-aware).
fn sr_badge(sr: &SafetyRequirement) -> String {
    let s = crate::commands::provenance::sr_standing(sr, None);
    let kind = match s {
        "verified" => "ok",
        "awaiting-cosign" => "warn",
        "stale" | "ungated" | "unconfirmed" => "bad",
        _ => "info",
    };
    badge(kind, s)
}

// REQ-0205: standing for a safety function — verified / awaiting-cosign /
// dossier-in-progress / allocated / proposed.
fn sf_standing(sf: &SafetyFunction) -> (&'static str, &'static str) {
    use crate::model::SafetyFunctionStatus as S;
    match sf.status {
        S::Verified => ("ok", "verified"),
        S::Implemented => {
            if sf
                .verification
                .as_ref()
                .and_then(|v| v.human_confirmation.as_ref())
                .is_some()
            {
                ("ok", "verified")
            } else {
                ("warn", "awaiting co-sign")
            }
        }
        S::Allocated => {
            let in_progress = sf
                .verification
                .as_ref()
                .map(|v| v.verdict.is_none() && (!v.coverage.is_empty() || v.analysis.is_some()))
                .unwrap_or(false);
            if in_progress {
                ("info", "dossier in progress")
            } else {
                ("info", "allocated")
            }
        }
        S::Proposed => ("info", "proposed"),
        S::Obsolete => ("info", "obsolete"),
    }
}
fn sf_badge(sf: &SafetyFunction) -> String {
    let (k, t) = sf_standing(sf);
    badge(k, t)
}

// REQ-0205: standing for a hazard — driven by its adequacy dossier.
fn haz_standing(hz: &Hazard) -> (&'static str, &'static str) {
    use crate::model::HazardStatus as S;
    match hz.status {
        S::Verified => ("ok", "verified"),
        S::Mitigated => match hz
            .adequacy
            .as_ref()
            .map(|a| (a.verdict.is_some(), a.human_confirmation.is_some()))
        {
            Some((true, true)) => ("ok", "verified"),
            Some((true, false)) => ("warn", "adequacy awaiting co-sign"),
            Some((false, _)) => ("info", "adequacy in progress"),
            None => ("info", "mitigated"),
        },
        S::Assessed => ("info", "assessed"),
        S::Identified => ("info", "identified"),
        S::Obsolete => ("info", "obsolete"),
    }
}

// REQ-0205: ancestry breadcrumbs (HAZ › SF › SR). A node can have several
// parents, so each distinct chain to the root is rendered on its own line.
fn breadcrumbs(project: &Project, id: &str) -> String {
    let up = id.to_uppercase();
    let root = "<a href=\"/safety\">Functional safety</a>";
    let mut lines: Vec<String> = Vec::new();
    let mitigated_by = |sfid: &str| -> Vec<String> {
        project
            .safety_functions
            .get(&sfid.to_uppercase())
            .map(|sf| {
                sf.links
                    .iter()
                    .filter(|l| l.kind == LinkKind::Mitigates)
                    .map(|l| l.target.clone())
                    .collect()
            })
            .unwrap_or_default()
    };
    if up.starts_with("HAZ") {
        lines.push(format!("{} &rsaquo; {}", root, h(&up)));
    } else if up.starts_with("SF") {
        let hazs = mitigated_by(&up);
        if hazs.is_empty() {
            lines.push(format!("{} &rsaquo; {}", root, h(&up)));
        }
        for hz in hazs {
            lines.push(format!(
                "{} &rsaquo; {} &rsaquo; {}",
                root,
                alink(&hz),
                h(&up)
            ));
        }
    } else if up.starts_with("SR") {
        let sfs: Vec<String> = project
            .safety_requirements
            .get(&up)
            .map(|sr| {
                sr.links
                    .iter()
                    .filter(|l| l.kind == LinkKind::Realizes)
                    .map(|l| l.target.clone())
                    .collect()
            })
            .unwrap_or_default();
        if sfs.is_empty() {
            lines.push(format!("{} &rsaquo; {}", root, h(&up)));
        }
        for sfid in sfs {
            let hazs = mitigated_by(&sfid);
            if hazs.is_empty() {
                lines.push(format!(
                    "{} &rsaquo; {} &rsaquo; {}",
                    root,
                    alink(&sfid),
                    h(&up)
                ));
            }
            for hz in hazs {
                lines.push(format!(
                    "{} &rsaquo; {} &rsaquo; {} &rsaquo; {}",
                    root,
                    alink(&hz),
                    alink(&sfid),
                    h(&up)
                ));
            }
        }
    }
    lines
        .iter()
        .map(|l| format!("<div class=\"crumb\">{}</div>", l))
        .collect()
}

// REQ-0147/REQ-0204: render a verification dossier (SR or SF). Shows the staged
// verdict/analysis/testing/statement, the human co-sign, the source anchor, and
// — for a safety function — the adequacy walk-through (a coverage note per
// realizing SR, each badged with that SR's standing).
fn dossier_html(v: Option<&crate::model::Verification>, project: &Project) -> String {
    match v {
        None => "<p class=\"meta\">No verification dossier recorded.</p>".to_string(),
        Some(v) => {
            let vbadge = match v.verdict {
                Some(o) if o.as_str() == "pass" => badge("ok", "pass"),
                Some(_) => badge("bad", "fail"),
                None => badge("info", "open"),
            };
            let stage = |a: &Option<crate::model::VerificationActivity>| {
                a.as_ref()
                    .map(|x| format!("{} — {}", x.outcome.as_str(), h(&x.summary)))
                    .unwrap_or_else(|| "—".to_string())
            };
            let conf = match &v.human_confirmation {
                Some(c) => format!(
                    "{} by {} @ {}",
                    badge("ok", "co-signed"),
                    h(&c.actor),
                    c.at.format("%Y-%m-%d %H:%M UTC")
                ),
                None => badge("warn", "awaiting human co-sign"),
            };
            let anchored = if v.content_hash.is_some() {
                format!(
                    "<li><strong>source anchor:</strong> {} file(s) hashed</li>",
                    v.linked_files.as_ref().map(|f| f.len()).unwrap_or(0)
                )
            } else {
                String::new()
            };
            let mut cov = String::new();
            if !v.coverage.is_empty() {
                cov.push_str("<h3>Adequacy walk-through (per realizing SR)</h3>");
                for c in &v.coverage {
                    let cb = project
                        .safety_requirements
                        .get(&c.target.to_uppercase())
                        .map(sr_badge)
                        .unwrap_or_default();
                    cov.push_str(&format!(
                        "<div class=\"cover\">{} {} — {}</div>",
                        alink(&c.target),
                        cb,
                        h(&c.note)
                    ));
                }
            }
            format!(
                "<div class=\"card\"><h2>Verification dossier</h2>\
                 <ul><li><strong>verdict:</strong> {vbadge}</li>\
                   <li><strong>analysis:</strong> {an}</li>\
                   <li><strong>testing:</strong> {te}</li>\
                   <li><strong>co-sign:</strong> {conf}</li>\
                   {stmt}{anchored}</ul>{cov}</div>",
                an = stage(&v.analysis),
                te = stage(&v.testing),
                stmt = v
                    .statement
                    .as_ref()
                    .map(|s| format!("<li><strong>statement:</strong> {}</li>", h(s)))
                    .unwrap_or_default(),
            )
        }
    }
}

// REQ-0204: render a hazard's staged mitigation-adequacy dossier — the plan, the
// per-mitigating-SF walk-through (each badged with that SF's standing), the
// residual-risk statement, the derived verdict, and the human co-sign.
fn adequacy_html(hz: &Hazard, project: &Project) -> String {
    match &hz.adequacy {
        None => "<div class=\"card\"><h2>Mitigation adequacy</h2>\
             <p class=\"meta\">No adequacy dossier yet — <code>req hazard adequacy plan</code> opens one.</p></div>"
            .to_string(),
        Some(a) => {
            let verdict = match a.verdict {
                Some(v) => badge(
                    if matches!(v, crate::model::AdequacyVerdict::Adequate) {
                        "ok"
                    } else {
                        "bad"
                    },
                    v.as_str(),
                ),
                None => badge("info", "in progress"),
            };
            let conf = match &a.human_confirmation {
                Some(c) => format!(
                    "{} by {} @ {}",
                    badge("ok", "co-signed"),
                    h(&c.actor),
                    c.at.format("%Y-%m-%d %H:%M UTC")
                ),
                None => badge("warn", "awaiting human co-sign"),
            };
            let mut cov = String::new();
            if !a.coverage.is_empty() {
                cov.push_str("<h3>Walk-through (per mitigating SF)</h3>");
                for c in &a.coverage {
                    let sb = project
                        .safety_functions
                        .get(&c.target.to_uppercase())
                        .map(sf_badge)
                        .unwrap_or_default();
                    cov.push_str(&format!(
                        "<div class=\"cover\">{} {} — {}</div>",
                        alink(&c.target),
                        sb,
                        h(&c.note)
                    ));
                }
            }
            format!(
                "<div class=\"card\"><h2>Mitigation adequacy</h2>\
                 <ul><li><strong>verdict:</strong> {verdict}</li>\
                   <li><strong>co-sign:</strong> {conf}</li>{plan}{resid}{ext}</ul>{cov}</div>",
                plan = if a.plan.is_empty() {
                    String::new()
                } else {
                    format!("<li><strong>plan:</strong> {}</li>", h(&a.plan))
                },
                resid = if a.statement.is_empty() {
                    String::new()
                } else {
                    format!("<li><strong>residual risk:</strong> {}</li>", h(&a.statement))
                },
                ext = a
                    .credited_external_measures
                    .as_ref()
                    .map(|e| format!("<li><strong>external credit:</strong> {}</li>", h(e)))
                    .unwrap_or_default(),
            )
        }
    }
}

// REQ-0171: a safety requirement's guided-walkthrough acknowledgement line.
fn walkthrough_html(sr: &SafetyRequirement) -> String {
    match &sr.walkthrough {
        None => String::new(),
        Some(w) => format!(
            "<p class=\"meta\">Walkthrough: {} by {} @ {}{}</p>",
            if w.objected {
                badge("bad", "objection")
            } else {
                badge("ok", "acknowledged")
            },
            h(&w.reviewer),
            w.at.format("%Y-%m-%d %H:%M UTC"),
            w.note
                .as_ref()
                .map(|n| format!(" — {}", h(n)))
                .unwrap_or_default(),
        ),
    }
}

// REQ-0147/REQ-0204: a hazard's detail page — its risk profile, the staged
// adequacy dossier, and the full mitigation chain (each SF and SR a clickable,
// badged link) so a reader can walk HAZ → SF → SR → verification.
fn render_hazard(project: &Project, raw: &str) -> Option<String> {
    let id = raw.to_uppercase();
    let hz = project.hazards.get(&id)?;
    let mut chain = String::new();
    for sf in project.mitigating_sfs(&id) {
        let meets = match (project.required_sil(hz), project.allocated_sil(sf)) {
            (Some(r), Some(a)) if a.rank() >= r.rank() => " &#10003;",
            (Some(_), _) => " &#9888;",
            _ => "",
        };
        chain.push_str(&format!(
            "<li>{} {} — {} <span class=\"meta\">allocated {}{}</span><ul>",
            alink(&sf.id),
            sf_badge(sf),
            h(&sf.title),
            sil_s(project.allocated_sil(sf)),
            meets,
        ));
        for sr in project.realizing_srs(&sf.id) {
            chain.push_str(&format!(
                "<li>{} {} — {}</li>",
                alink(&sr.id),
                sr_badge(sr),
                h(&sr.title),
            ));
        }
        chain.push_str("</ul></li>");
    }
    if chain.is_empty() {
        chain.push_str("<li class=\"meta\">no mitigating safety function</li>");
    }
    let (sk, st) = haz_standing(hz);
    let adequacy = format!(
        "{}{}",
        adequacy_html(hz, project),
        signoff_card(crate::commands::safety::hazard_signoff_lines(project, hz))
    );
    Some(format!(
        "{crumbs}\
         <h1>{id} <small>{title}</small></h1>\
         <ul class=\"meta\"><li>{standing}</li> \
         <li><strong>Status:</strong> {status}</li> \
         <li><strong>Required SIL:</strong> {sil}</li></ul>\
         <h2>Harm</h2><p>{harm}</p>\
         {risk}{adequacy}\
         <h2>Mitigation chain</h2><ul>{chain}</ul>",
        crumbs = breadcrumbs(project, &id),
        id = h(&hz.id),
        title = h(&hz.title),
        standing = badge(sk, st),
        status = hz.status.as_str(),
        sil = sil_s(project.required_sil(hz)),
        harm = h(&hz.harm),
        risk = match (hz.consequence, hz.frequency, hz.avoidance, hz.probability) {
            (Some(c), Some(f), Some(p), Some(w)) => format!(
                "<p class=\"meta\">risk graph: {} &middot; {} &middot; {} &middot; {}</p>",
                c.as_str(),
                f.as_str(),
                p.as_str(),
                w.as_str()
            ),
            _ => String::new(),
        },
        adequacy = adequacy,
        chain = chain,
    ))
}

// REQ-0147/REQ-0204: a safety-function page — its safe state, verification
// dossier (incl. the realizing-SR adequacy walk-through), and links up to the
// hazards it mitigates and down to the SRs that realize it.
fn render_sf(project: &Project, raw: &str) -> Option<String> {
    let id = raw.to_uppercase();
    let sf = project.safety_functions.get(&id)?;
    let haz: Vec<String> = sf
        .links
        .iter()
        .filter(|l| l.kind == LinkKind::Mitigates)
        .map(|l| format!("<li>{}</li>", alink(&l.target)))
        .collect();
    let srs: Vec<String> = project
        .realizing_srs(&id)
        .iter()
        .map(|sr| {
            format!(
                "<li>{} {} — {}</li>",
                alink(&sr.id),
                sr_badge(sr),
                h(&sr.title)
            )
        })
        .collect();
    let (sk, st) = sf_standing(sf);
    let dossier = format!(
        "{}{}",
        dossier_html(sf.verification.as_ref(), project),
        signoff_card(crate::commands::safety::sf_signoff_lines(project, sf))
    );
    Some(format!(
        "{crumbs}\
         <h1>{id} <small>{title}</small></h1>\
         <ul class=\"meta\"><li>{standing}</li> \
         <li><strong>Status:</strong> {status}</li> \
         <li><strong>Allocated SIL:</strong> {sil}</li></ul>\
         {safe}\
         {dossier}\
         <h2>Mitigates (hazards)</h2><ul>{haz}</ul>\
         <h2>Realized by (safety requirements)</h2><ul>{srs}</ul>",
        crumbs = breadcrumbs(project, &id),
        id = h(&sf.id),
        title = h(&sf.title),
        standing = badge(sk, st),
        status = sf.status.as_str(),
        sil = sil_s(project.allocated_sil(sf)),
        safe = if sf.safe_state.is_empty() {
            String::new()
        } else {
            format!("<h2>Safe state</h2><p>{}</p>", h(&sf.safe_state))
        },
        dossier = dossier,
        haz = if haz.is_empty() {
            "<li class=\"meta\">none</li>".into()
        } else {
            haz.join("")
        },
        srs = if srs.is_empty() {
            "<li class=\"meta\">none</li>".into()
        } else {
            srs.join("")
        },
    ))
}

// REQ-0147: a safety-requirement page — its statement, the function(s) it
// realizes, its verification dossier, and the guided-walkthrough acknowledgement.
fn render_sr(project: &Project, raw: &str) -> Option<String> {
    let id = raw.to_uppercase();
    let sr = project.safety_requirements.get(&id)?;
    let sfs: Vec<String> = sr
        .links
        .iter()
        .filter(|l| l.kind == LinkKind::Realizes)
        .map(|l| format!("<li>{}</li>", alink(&l.target)))
        .collect();
    Some(format!(
        "{crumbs}\
         <h1>{id} <small>{title}</small></h1>\
         <ul class=\"meta\"><li>{standing}</li> \
         <li><strong>Status:</strong> {status}</li> \
         <li><strong>Inherited SIL:</strong> {sil}</li></ul>\
         {walk}\
         <h2>Statement</h2><p>{stmt}</p>\
         <h2>Realizes (safety functions)</h2><ul>{sfs}</ul>\
         {dossier}",
        crumbs = breadcrumbs(project, &id),
        id = h(&sr.id),
        title = h(&sr.title),
        standing = sr_badge(sr),
        status = sr.status.as_str(),
        sil = sil_s(project.inherited_sil(sr)),
        walk = walkthrough_html(sr),
        stmt = h(&sr.statement),
        sfs = if sfs.is_empty() {
            "<li class=\"meta\">none</li>".into()
        } else {
            sfs.join("")
        },
        dossier = dossier_html(sr.verification.as_ref(), project),
    ))
}

// REQ-0147: detail page for a safety entity (HAZ/SF/SR) with hyperlinks to the
// entities it relates to, and — for a safety requirement — its verification
// dossier, so the whole chain is navigable in the browser.
async fn safety_entity_html(
    State(state): State<Arc<AppState>>,
    AxPath(id): AxPath<String>,
) -> Result<Html<String>, (StatusCode, String)> {
    let project = load_project(&state)?;
    let up = id.to_uppercase();
    let body = if up.starts_with("HAZ") {
        render_hazard(&project, &id)
    } else if up.starts_with("SF") {
        render_sf(&project, &id)
    } else if up.starts_with("SR") {
        render_sr(&project, &id)
    } else {
        None
    };
    match body {
        Some(b) => Ok(Html(page(&id, &b))),
        None => Err((
            StatusCode::NOT_FOUND,
            format!("no such safety entity: {}", id),
        )),
    }
}

fn page(title: &str, body: &str) -> String {
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>{title}</title><style>\
         body{{font-family:system-ui,sans-serif;max-width:64rem;margin:2rem auto;padding:0 1rem;line-height:1.5;color:#222;}}\
         h1{{margin-bottom:.2rem;}} h1 small{{font-weight:400;color:#666;}}\
         table{{width:100%;border-collapse:collapse;margin-top:1rem;}}\
         th,td{{padding:.4rem .6rem;border-bottom:1px solid #eee;text-align:left;vertical-align:top;}}\
         th{{background:#fafafa;}}\
         .meta{{color:#666;font-size:.9rem;}} ul.meta{{list-style:none;padding:0;}} ul.meta li{{display:inline-block;margin-right:1.5rem;}}\
         code{{background:#f4f4f4;padding:.1rem .3rem;border-radius:3px;font-size:.9em;}}\
         a{{color:#0366d6;}}\
         .crumb{{color:#666;font-size:.9rem;margin:.1rem 0;}} .crumb a{{color:#0366d6;}}\
         .badge{{display:inline-block;padding:.05rem .45rem;border-radius:1rem;font-size:.78rem;font-weight:600;vertical-align:middle;}}\
         .b-ok{{background:#e6f4ea;color:#137333;}} .b-warn{{background:#fef7e0;color:#a36a00;}} .b-bad{{background:#fce8e6;color:#c5221f;}} .b-info{{background:#eef;color:#3b4cca;}}\
         .card{{border:1px solid #eaeaea;border-radius:8px;padding:.6rem 1rem;margin:.6rem 0;background:#fcfcfc;}}\
         .card h2,.card h3{{margin-top:.4rem;}} .cover{{border-left:3px solid #ddd;padding-left:.7rem;margin:.4rem 0;}}\
         .blocked{{border-left:3px solid #c5221f;}} .done{{border-left:3px solid #137333;}}\
         </style></head><body>{body}</body></html>",
        title = h(title),
        body = body
    )
}

fn h(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
