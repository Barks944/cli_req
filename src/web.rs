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
    LinkKind, Project, Requirement, SafetyFunction, SafetyRequirement, Sil, Status,
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
        // REQ-0147: the hazard id links to its detail page so a reader can walk
        // into the full mitigation + verification chain.
        rows.push_str(&format!(
            "<tr><td>{idlink}</td><td>{title}</td><td>{harm}</td><td>{req}</td><td>{alloc}</td><td>{v}/{t}</td><td>{verdict}</td></tr>",
            idlink = alink(id),
            title = h(&hz.title),
            harm = h(&hz.harm),
            req = sil_s(project.required_sil(hz)),
            alloc = sil_s(allocated),
            v = verified,
            t = total,
            verdict = if complete { "&#10003; complete" } else { "&#9888; incomplete" },
        ));
    }
    let disclaimer = "<p class=\"meta\" style=\"border-left:3px solid #e0a800;padding-left:.6rem;\">\
        &#9888; req computes a <em>candidate</em> SIL from your inputs and checks <em>traceability</em> only. \
        It is not qualified per IEC 61508-3 &sect;7.4.4 and does not assure risk reduction; the table uses the \
        Annex&nbsp;D worked-example calibration. The safety determination remains the engineer's responsibility.</p>";
    Ok(Html(page(
        "req — functional safety",
        &format!(
            "<p><a href=\"/\">&larr; index</a></p>\
             <h1>Functional safety</h1>{disclaimer}\
             <p class=\"meta\">{nh} hazard(s) &middot; {nf} safety function(s) &middot; {nr} safety requirement(s)</p>\
             <table><thead><tr><th>Hazard</th><th>Title</th><th>Harm</th><th>Required SIL</th><th>Allocated SIL</th><th>SRs verified</th><th>Trace</th></tr></thead><tbody>{rows}</tbody></table>",
            disclaimer = disclaimer,
            nh = project.hazards.len(),
            nf = project.safety_functions.len(),
            nr = project.safety_requirements.len(),
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

fn sf_mitigates(sf: &SafetyFunction, hid: &str) -> bool {
    sf.links
        .iter()
        .any(|l| l.kind == LinkKind::Mitigates && l.target == hid)
}
fn sr_realizes(sr: &SafetyRequirement, sfid: &str) -> bool {
    sr.links
        .iter()
        .any(|l| l.kind == LinkKind::Realizes && l.target == sfid)
}

// REQ-0147: render a safety requirement's verification dossier (the artifact the
// review surfaced as un-navigable) so it's reachable from the chain.
fn dossier_html(sr: &SafetyRequirement) -> String {
    match &sr.verification {
        None => "<p class=\"meta\">No verification dossier recorded.</p>".to_string(),
        Some(v) => {
            let verdict = v.verdict.map(|o| o.as_str()).unwrap_or("open");
            let stage = |a: &Option<crate::model::VerificationActivity>| {
                a.as_ref()
                    .map(|x| format!("{} — {}", x.outcome.as_str(), h(&x.summary)))
                    .unwrap_or_else(|| "—".to_string())
            };
            let conf = match &v.human_confirmation {
                Some(c) => format!(
                    "human-confirmed by {} @ {}",
                    h(&c.actor),
                    c.at.format("%Y-%m-%d %H:%M UTC")
                ),
                None => "&#9888; awaiting human confirmation (REQ-V-0034)".to_string(),
            };
            format!(
                "<h2>Verification dossier</h2>\
                 <ul>\
                   <li><strong>verdict:</strong> {verdict}</li>\
                   <li><strong>analysis:</strong> {an}</li>\
                   <li><strong>testing:</strong> {te}</li>\
                   <li><strong>confirmation:</strong> {conf}</li>\
                   {stmt}\
                 </ul>",
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

// REQ-0147: a hazard's detail page renders its full mitigation chain (each SF
// and SR a clickable link), so a reader can walk HAZ → SF → SR → verification.
fn render_hazard(project: &Project, raw: &str) -> Option<String> {
    let id = raw.to_uppercase();
    let hz = project.hazards.get(&id)?;
    let mut chain = String::new();
    for sf in project
        .safety_functions
        .values()
        .filter(|sf| sf_mitigates(sf, &id))
    {
        chain.push_str(&format!(
            "<li>{} — {} <span class=\"meta\">[{}, allocated {}]</span><ul>",
            alink(&sf.id),
            h(&sf.title),
            sf.status.as_str(),
            sil_s(project.allocated_sil(sf))
        ));
        for sr in project
            .safety_requirements
            .values()
            .filter(|sr| sr_realizes(sr, &sf.id))
        {
            let conf = match sr
                .verification
                .as_ref()
                .and_then(|v| v.human_confirmation.as_ref())
            {
                Some(c) => format!("human-confirmed by {}", h(&c.actor)),
                None => "&#9888; unconfirmed".to_string(),
            };
            chain.push_str(&format!(
                "<li>{} — {} <span class=\"meta\">[{}] · {}</span></li>",
                alink(&sr.id),
                h(&sr.title),
                sr.status.as_str(),
                conf
            ));
        }
        chain.push_str("</ul></li>");
    }
    if chain.is_empty() {
        chain.push_str("<li class=\"meta\">no mitigating safety function</li>");
    }
    Some(format!(
        "<p><a href=\"/safety\">&larr; functional safety</a></p>\
         <h1>{id} <small>{title}</small></h1>\
         <ul class=\"meta\"><li><strong>Status:</strong> {status}</li> \
         <li><strong>Required SIL:</strong> {sil}</li></ul>\
         <h2>Harm</h2><p>{harm}</p>\
         <h2>Mitigation chain</h2><ul>{chain}</ul>",
        id = h(&hz.id),
        title = h(&hz.title),
        status = hz.status.as_str(),
        sil = sil_s(project.required_sil(hz)),
        harm = h(&hz.harm),
        chain = chain,
    ))
}

// REQ-0147: a safety-function page links up to the hazards it mitigates and
// down to the safety requirements that realize it.
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
        .safety_requirements
        .values()
        .filter(|sr| sr_realizes(sr, &id))
        .map(|sr| {
            format!(
                "<li>{} — {} <span class=\"meta\">[{}]</span></li>",
                alink(&sr.id),
                h(&sr.title),
                sr.status.as_str()
            )
        })
        .collect();
    Some(format!(
        "<p><a href=\"/safety\">&larr; functional safety</a></p>\
         <h1>{id} <small>{title}</small></h1>\
         <ul class=\"meta\"><li><strong>Status:</strong> {status}</li> \
         <li><strong>Allocated SIL:</strong> {sil}</li></ul>\
         {safe}\
         <h2>Mitigates (hazards)</h2><ul>{haz}</ul>\
         <h2>Realized by (safety requirements)</h2><ul>{srs}</ul>",
        id = h(&sf.id),
        title = h(&sf.title),
        status = sf.status.as_str(),
        sil = sil_s(project.allocated_sil(sf)),
        safe = if sf.safe_state.is_empty() {
            String::new()
        } else {
            format!("<h2>Safe state</h2><p>{}</p>", h(&sf.safe_state))
        },
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

// REQ-0147: a safety-requirement page links to the function it realizes and
// inlines its verification dossier (the artifact that was previously unreachable).
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
        "<p><a href=\"/safety\">&larr; functional safety</a></p>\
         <h1>{id} <small>{title}</small></h1>\
         <ul class=\"meta\"><li><strong>Status:</strong> {status}</li> \
         <li><strong>Inherited SIL:</strong> {sil}</li></ul>\
         <h2>Statement</h2><p>{stmt}</p>\
         <h2>Realizes (safety functions)</h2><ul>{sfs}</ul>\
         {dossier}",
        id = h(&sr.id),
        title = h(&sr.title),
        status = sr.status.as_str(),
        sil = sil_s(project.inherited_sil(sr)),
        stmt = h(&sr.statement),
        sfs = if sfs.is_empty() {
            "<li class=\"meta\">none</li>".into()
        } else {
            sfs.join("")
        },
        dossier = dossier_html(sr),
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
