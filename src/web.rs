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
    storage::load(&path).context("validate project before binding socket")?;

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
        rows.push_str(&format!(
            "<tr><td>{id}</td><td>{title}</td><td>{harm}</td><td>{req}</td><td>{alloc}</td><td>{v}/{t}</td><td>{verdict}</td></tr>",
            id = h(id),
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
    for l in &r.links {
        links.push_str(&format!(
            "<li><em>{}</em> &rarr; <a href=\"/r/{tgt}\">{tgt}</a></li>",
            l.kind.as_str(),
            tgt = h(&l.target)
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
