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

// REQ-0212: badge for a requirement's lifecycle status.
fn status_badge(s: Status) -> String {
    let kind = match s {
        Status::Verified => "ok",
        Status::Implemented => "info",
        Status::Obsolete => "mute",
        _ => "warn",
    };
    badge(kind, s.as_str())
}

// REQ-0211: the one-line functional-safety presence note for the index —
// an explicit "disabled" / "no artifacts" statement instead of a silently
// missing section when there is nothing to show.
fn safety_note(state: &AppState, project: &Project) -> String {
    let populated = !project.hazards.is_empty()
        || !project.safety_functions.is_empty()
        || !project.safety_requirements.is_empty();
    if populated {
        format!(
            " &middot; <a href=\"/safety\">functional safety ({} hazard(s), {} SF, {} SR)</a>",
            project.hazards.len(),
            project.safety_functions.len(),
            project.safety_requirements.len()
        )
    } else if crate::commands::safety_gov::is_enabled(&state.file) {
        " &middot; functional safety: enabled, no safety artifacts yet".to_string()
    } else {
        " &middot; functional safety: disabled".to_string()
    }
}

// REQ-0210/0212: the index IS the verification roll-up — one table carrying
// both the lifecycle status and the verification standing per requirement
// (provenance for verified items, dossier stage for unverified ones), with
// the provenance counts and the not-genuine warning in the header. A merged
// view: the reviewer should not need a second page to see V&V standing.
async fn index_html(
    State(state): State<Arc<AppState>>,
) -> Result<Html<String>, (StatusCode, String)> {
    use crate::commands::provenance::{classify, Provenance};
    let project = load_project(&state)?;
    let root = state
        .file
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    // Dossier stage per unverified item (requirements and SRs share ids space).
    let stages: std::collections::BTreeMap<String, &'static str> =
        crate::commands::verification::unverified_rows(&project)
            .into_iter()
            .map(|(id, _fam, stage)| (id, stage))
            .collect();
    let mut prov_counts: std::collections::BTreeMap<&'static str, usize> = Default::default();
    // REQ-0212: distinct values per facet, for the dropdown filters.
    let mut facet_standings: std::collections::BTreeSet<String> = Default::default();
    let mut facet_tags: std::collections::BTreeSet<String> = Default::default();
    let mut rows = String::new();
    for r in project.requirements.values() {
        // REQ-0210: the verification-standing cell — provenance behind a
        // Verified flag, otherwise how far the dossier pipeline progressed.
        let (standing_txt, standing) = if matches!(r.status, Status::Verified) {
            let p = classify(r.verification.as_ref(), Some(&root), &r.id);
            *prov_counts.entry(p.as_str()).or_default() += 1;
            let kind = match p {
                Provenance::Genuine => "ok",
                Provenance::Stale | Provenance::Ungated => "bad",
                _ => "warn",
            };
            (p.as_str().to_string(), badge(kind, p.as_str()))
        } else if let Some(stage) = stages.get(&r.id) {
            (stage.to_string(), badge("info", stage))
        } else {
            ("—".to_string(), "—".to_string())
        };
        facet_standings.insert(standing_txt.clone());
        for t in &r.tags {
            facet_tags.insert(t.clone());
        }
        // REQ-0212: each row carries its facet values as data attributes so
        // the dropdown filters match structurally, not by substring.
        rows.push_str(&format!(
            "<tr data-kind=\"{kind}\" data-pri=\"{pri}\" data-status=\"{status_txt}\" data-standing=\"{standing_txt}\" data-tags=\"|{tags_attr}|\">\
             <td><a href=\"/r/{id}\">{id}</a></td><td><a href=\"/r/{id}\">{title}</a></td><td>{kind}</td><td>{pri}</td><td>{status}</td><td>{standing}</td><td>{tags}</td></tr>",
            id = h(&r.id),
            title = h(&r.title),
            kind = r.kind.as_str(),
            pri = r.priority.as_str(),
            status = status_badge(r.status),
            status_txt = r.status.as_str(),
            standing = standing,
            standing_txt = h(&standing_txt),
            tags = h(&r.tags.join(", ")),
            tags_attr = h(&r.tags.join("|")),
        ));
    }
    // REQ-0212: the filter bar — free text plus structural dropdowns for
    // kind, priority, status, verification standing, and tag.
    let select = |key: &str, label: &str, values: &[String]| {
        let opts: String = values
            .iter()
            .map(|v| format!("<option value=\"{0}\">{0}</option>", h(v)))
            .collect();
        format!(
            "<select class=\"rowfilter\" data-key=\"{key}\"><option value=\"\">{label}: all</option>{opts}</select>",
        )
    };
    let distinct = |f: &dyn Fn(&Requirement) -> String| -> Vec<String> {
        let mut s: std::collections::BTreeSet<String> = Default::default();
        for r in project.requirements.values() {
            s.insert(f(r));
        }
        s.into_iter().collect()
    };
    let filter_bar = format!(
        "<div class=\"filters\">\
         <input id=\"filter\" type=\"search\" placeholder=\"filter (id, title, any text)…\">\
         {}{}{}{}{}\
         <span class=\"meta\" id=\"filter-count\"></span></div>",
        select("kind", "kind", &distinct(&|r| r.kind.as_str().to_string())),
        select(
            "pri",
            "priority",
            &distinct(&|r| r.priority.as_str().to_string())
        ),
        select(
            "status",
            "status",
            &distinct(&|r| r.status.as_str().to_string())
        ),
        select(
            "standing",
            "verification",
            &facet_standings.iter().cloned().collect::<Vec<_>>()
        ),
        select(
            "tags",
            "tag",
            &facet_tags.iter().cloned().collect::<Vec<_>>()
        ),
    );
    // REQ-0212: headline chips — the status breakdown, then the provenance
    // breakdown of the verified set.
    let mut by_status: std::collections::BTreeMap<&'static str, usize> = Default::default();
    for r in project.requirements.values() {
        *by_status.entry(r.status.as_str()).or_default() += 1;
    }
    let mut chips: String = by_status
        .iter()
        .map(|(s, n)| format!("<span class=\"chip\"><b>{}</b> {}</span>", n, h(s)))
        .collect();
    for (p, n) in &prov_counts {
        let kind = match *p {
            "genuine" => "ok",
            "stale" | "ungated" => "bad",
            _ => "warn",
        };
        chips.push_str(&format!(
            "<span class=\"chip\"><b>{}</b> {}</span>",
            n,
            badge(kind, p)
        ));
    }
    // REQ-0210: the honesty banner — how much of "verified" really rests on a
    // genuine dossier.
    let verified_total: usize = prov_counts.values().sum();
    let not_genuine = verified_total - prov_counts.get("genuine").copied().unwrap_or(0);
    let warn = if not_genuine > 0 {
        format!(
            "<p class=\"meta\" style=\"border-left:3px solid #c5221f;padding-left:.6rem;\">&#9888; \
             {} of {} verified requirement(s) do NOT rest on a genuine verification dossier.</p>",
            not_genuine, verified_total
        )
    } else {
        String::new()
    };
    Ok(Html(page(
        &format!("req — {}", h(&project.name)),
        &format!(
            "<h1>{name}</h1>\
             <p class=\"meta\">{count} requirement(s) &middot; served from <code>{path}</code> &middot; read-only{safety_note}</p>\
             <div class=\"chips\">{chips}</div>{warn}\
             {filter_bar}\
             <table class=\"filterable\"><thead><tr><th>ID</th><th>Title</th><th>Kind</th><th>Pri</th><th>Status</th><th>Verification</th><th>Tags</th></tr></thead><tbody>{rows}</tbody></table>",
            name = h(&project.name),
            count = project.requirements.len(),
            path = h(&state.file.display().to_string()),
            safety_note = safety_note(&state, &project),
            chips = chips,
            warn = warn,
            filter_bar = filter_bar,
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
    // REQ-0211: the explicit empty state — a project with safety disabled or
    // no safety artifacts says so, instead of rendering an empty section.
    if project.hazards.is_empty()
        && project.safety_functions.is_empty()
        && project.safety_requirements.is_empty()
    {
        let why = if crate::commands::safety_gov::is_enabled(&state.file) {
            "Functional safety is <strong>enabled</strong> but the project has no hazards, \
             safety functions, or safety requirements yet."
        } else {
            "Functional safety is <strong>disabled</strong> for this project — no safety \
             disclaimer has been accepted (<code>req safety accept</code>) and there are no \
             safety artifacts."
        };
        return Ok(Html(page(
            "req — functional safety",
            &format!("<h1>Functional safety</h1><p class=\"meta\">{}</p>", why),
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
        let (sk, st) = haz_standing(&project, hz);
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
    // REQ-0212: safety functions and safety requirements are browsable from
    // the safety index in their own right, not only via a hazard's chain.
    let mut sf_rows = String::new();
    let mut sfs: Vec<_> = project.safety_functions.values().collect();
    sfs.sort_by(|a, b| a.id.cmp(&b.id));
    for sf in &sfs {
        sf_rows.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            alink(&sf.id),
            h(&sf.title),
            sf_badge(&project, sf),
            sil_s(project.allocated_sil(sf)),
        ));
    }
    let mut sr_rows = String::new();
    let head = crate::commands::safety_gov::head_sha();
    let mut srs: Vec<_> = project.safety_requirements.values().collect();
    srs.sort_by(|a, b| a.id.cmp(&b.id));
    for sr in &srs {
        // REQ-0211: the walkthrough-acknowledgement state per SR — fresh,
        // stale, never acknowledged, or an objection on record.
        let walk = match sr.walkthrough.as_ref() {
            None => badge("warn", "never acknowledged"),
            Some(a) if a.objected => badge("bad", "objection"),
            Some(a) if crate::commands::safety_gov::ack_is_fresh(Some(a), &head) => {
                badge("ok", "acknowledged")
            }
            Some(_) => badge("warn", "acknowledgement stale"),
        };
        sr_rows.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            alink(&sr.id),
            h(&sr.title),
            sr_badge(sr),
            sil_s(project.inherited_sil(sr)),
            walk,
        ));
    }
    // REQ-0211: the active risk-graph calibration — the label and any leaf
    // overrides in force (`req safety calibrate`), so a reviewer sees which
    // scheme derived the SILs above.
    let cal_label = project
        .config
        .as_ref()
        .and_then(|c| c.safety.as_ref())
        .and_then(|s| s.calibration_label.as_deref())
        .unwrap_or("IEC 61508-5 Annex D (default)");
    let calibration = match project.calibration() {
        None => format!(
            "<div class=\"card\"><h2>SIL calibration</h2><p class=\"meta\">{} — no leaf \
             overrides; every leaf uses the Annex D worked-example default.</p></div>",
            h(cal_label)
        ),
        Some(table) => {
            let trows: String = table
                .iter()
                .map(|(leaf, row)| {
                    format!(
                        "<tr><td><code>{}</code></td><td>{}</td><td>{}</td><td>{}</td></tr>",
                        h(leaf),
                        row.w3.as_str(),
                        row.w2.as_str(),
                        row.w1.as_str()
                    )
                })
                .collect();
            format!(
                "<div class=\"card\"><h2>SIL calibration</h2><p class=\"meta\">{} — {} leaf override(s):</p>\
                 <table><thead><tr><th>Leaf (C/F/P)</th><th>W3</th><th>W2</th><th>W1</th></tr></thead><tbody>{}</tbody></table></div>",
                h(cal_label),
                table.len(),
                trows
            )
        }
    };
    Ok(Html(page(
        "req — functional safety",
        &format!(
            "<h1>Functional safety</h1>{disclaimer}{rollup}\
             <h2>Hazards</h2>\
             <table><thead><tr><th>Hazard</th><th>Title</th><th>Standing</th><th>Required SIL</th><th>Allocated SIL</th><th>SRs verified</th><th>Trace</th></tr></thead><tbody>{rows}</tbody></table>\
             {sf_table}{sr_table}{calibration}",
            disclaimer = disclaimer,
            rollup = rollup,
            rows = rows,
            sf_table = if sf_rows.is_empty() {
                String::new()
            } else {
                format!(
                    "<h2>Safety functions</h2>\
                     <table><thead><tr><th>ID</th><th>Title</th><th>Standing</th><th>Allocated SIL</th></tr></thead><tbody>{}</tbody></table>",
                    sf_rows
                )
            },
            sr_table = if sr_rows.is_empty() {
                String::new()
            } else {
                format!(
                    "<h2>Safety requirements</h2>\
                     <table><thead><tr><th>ID</th><th>Title</th><th>Standing</th><th>Inherited SIL</th><th>Walkthrough</th></tr></thead><tbody>{}</tbody></table>",
                    sr_rows
                )
            },
            calibration = calibration,
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
    for a in &r.acceptance {
        acc.push_str(&format!("<li>{}</li>", h(a)));
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
    for hist in r.history.iter().rev() {
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
    // REQ-0209: the requirement page carries the FULL verification dossier and
    // the test-record list — a browser review sees what `req verification
    // show` prints, plus the provenance standing behind the Verified flag.
    let dossier = dossier_html(r.verification.as_ref(), &project);
    let records = test_records_html(&r.tests);
    let prov = if matches!(r.status, Status::Verified) || r.verification.is_some() {
        format!(
            "<li><strong>Provenance:</strong> {}</li>",
            prov_badge(&state, r.verification.as_ref(), &r.id)
        )
    } else {
        String::new()
    };
    // REQ-0212: the page is panelled — a status-tinted header band, then one
    // card per section; the full history is collapsed by default.
    Ok(Html(page(
        &format!("{} — {}", r.id, h(&r.title)),
        &format!(
            "<div class=\"hdr st-{status_txt}\">\
               <h1>{id} <small>{title}</small></h1>\
               <ul class=\"meta\">\
                  <li><strong>Kind:</strong> {kind}</li>\
                  <li><strong>Priority:</strong> {pri}</li>\
                  <li><strong>Status:</strong> {status}</li>\
                  <li><strong>Tags:</strong> {tags}</li>\
                  {prov}\
               </ul>\
             </div>\
             <div class=\"card\"><h2>Statement</h2><p>{stmt}</p></div>\
             <div class=\"card\"><h2>Rationale</h2><p>{rat}</p></div>\
             {acc_block}\
             {dossier}\
             {records}\
             {links_block}\
             <details class=\"card\"><summary><h2>History ({nhist})</h2></summary><ul>{history}</ul></details>",
            id = h(&r.id),
            title = h(&r.title),
            kind = r.kind.as_str(),
            pri = r.priority.as_str(),
            status = status_badge(r.status),
            status_txt = r.status.as_str(),
            tags = h(&r.tags.join(", ")),
            prov = prov,
            stmt = h(&r.statement),
            rat = h(&r.rationale),
            acc_block = if acc.is_empty() {
                String::new()
            } else {
                format!(
                    "<div class=\"card\"><h2>Acceptance criteria</h2><ol>{}</ol></div>",
                    acc
                )
            },
            dossier = dossier,
            records = records,
            links_block = if links.is_empty() {
                String::new()
            } else {
                format!("<div class=\"card\"><h2>Links</h2><ul>{}</ul></div>", links)
            },
            nhist = r.history.len(),
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
fn sf_standing(project: &Project, sf: &SafetyFunction) -> (&'static str, &'static str) {
    use crate::model::SafetyFunctionStatus as S;
    // Chain-aware like haz_standing: an SF only stands "verified" while every
    // realizing SR is itself Verified.
    let srs = project.realizing_srs(&sf.id);
    let chain_ok = !srs.is_empty() && srs.iter().all(|sr| matches!(sr.status, Status::Verified));
    match sf.status {
        S::Verified if chain_ok => ("ok", "verified"),
        S::Verified => ("warn", "chain reopened"),
        S::Implemented => {
            if sf
                .verification
                .as_ref()
                .and_then(|v| v.human_confirmation.as_ref())
                .is_some()
            {
                if chain_ok {
                    ("ok", "verified")
                } else {
                    ("warn", "chain reopened")
                }
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
fn sf_badge(project: &Project, sf: &SafetyFunction) -> String {
    let (k, t) = sf_standing(project, sf);
    badge(k, t)
}

// REQ-0205: standing for a hazard — driven by its adequacy dossier.
fn haz_standing(project: &Project, hz: &Hazard) -> (&'static str, &'static str) {
    use crate::model::HazardStatus as S;
    use crate::model::SafetyFunctionStatus as FS;
    // A hazard's standing only counts as "verified" while its whole mitigation
    // chain still stands: every mitigating SF Verified. A co-signed adequacy
    // dossier over a reopened chain is a chain that needs re-arguing, and the
    // badge must say so rather than echo the stale dossier.
    let sfs = project.mitigating_sfs(&hz.id);
    let chain_ok = !sfs.is_empty() && sfs.iter().all(|sf| matches!(sf.status, FS::Verified));
    match hz.status {
        S::Verified if chain_ok => ("ok", "verified"),
        S::Verified => ("warn", "chain reopened"),
        S::Mitigated => match hz
            .adequacy
            .as_ref()
            .map(|a| (a.verdict.is_some(), a.human_confirmation.is_some()))
        {
            Some((true, true)) if chain_ok => ("ok", "verified"),
            Some((true, true)) => ("warn", "chain reopened"),
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
        None => "<div class=\"card\"><h2>Verification dossier</h2>\
                 <p class=\"meta\">No verification dossier recorded.</p></div>"
            .to_string(),
        Some(v) => {
            let vbadge = match v.verdict {
                Some(o) if o.as_str() == "pass" => badge("ok", "pass"),
                Some(_) => badge("bad", "fail"),
                None => badge("info", "open"),
            };
            // REQ-0209: each activity renders its outcome, findings, and its
            // references (files/commits reviewed, tests cited) — the same
            // content `req verification show` prints. REQ-0212: the reference
            // list is collapsed by default so a long test list doesn't
            // dominate the panel.
            let stage = |label: &str, a: &Option<crate::model::VerificationActivity>| match a
                .as_ref()
            {
                None => format!(
                    "<div class=\"sub\"><h3>{}</h3><p class=\"meta\">pending</p></div>",
                    h(label)
                ),
                Some(x) => {
                    let refs = if x.references.is_empty() {
                        String::new()
                    } else {
                        format!(
                            "<details><summary class=\"meta\">{} reference(s)</summary>{}</details>",
                            x.references.len(),
                            x.references
                                .iter()
                                .map(|r| format!("<div class=\"meta\">&middot; {}</div>", h(r)))
                                .collect::<String>()
                        )
                    };
                    // Long findings (e.g. an embedded audit dump) lead with a
                    // short excerpt; the full text sits behind a disclosure,
                    // preformatted so embedded tables keep their alignment.
                    const EXCERPT: usize = 350;
                    let first_line = x.summary.lines().next().unwrap_or("");
                    let summary = if x.summary.len() <= EXCERPT && x.summary.lines().count() <= 1 {
                        format!("<p>{}</p>", h(&x.summary))
                    } else {
                        let mut cut = first_line.chars().take(EXCERPT).collect::<String>();
                        if cut.len() < x.summary.len() {
                            cut.push('…');
                        }
                        format!(
                            "<p>{}</p>\
                             <details><summary class=\"meta\">full findings</summary>\
                             <pre class=\"findings\">{}</pre></details>",
                            h(&cut),
                            h(&x.summary)
                        )
                    };
                    format!(
                        "<div class=\"sub\"><h3>{label} {b}</h3>\
                         {summary}\
                         <p class=\"meta\">{actor} @ {at}</p>{refs}</div>",
                        label = h(label),
                        b = if matches!(x.outcome, crate::model::TestOutcome::Pass) {
                            badge("ok", "pass")
                        } else {
                            badge("bad", "fail")
                        },
                        summary = summary,
                        actor = h(&x.actor),
                        at = x.at.format("%Y-%m-%d"),
                        refs = refs,
                    )
                }
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
            let anchored = if let Some(hash) = &v.content_hash {
                format!(
                    "<p class=\"meta\">anchor <code>{}</code> over {} file(s){}</p>",
                    h(&hash[..hash.len().min(12)]),
                    v.linked_files.as_ref().map(|f| f.len()).unwrap_or(0),
                    v.concluded_commit
                        .as_deref()
                        .map(|c| format!(" @ <code>{}</code>", h(&c[..c.len().min(9)])))
                        .unwrap_or_default(),
                )
            } else {
                String::new()
            };
            // REQ-0209: an audited exemption is shown for what it is.
            let exempt = if v.exempt {
                badge(
                    "warn",
                    match v.exemption_kind {
                        Some(crate::model::ExemptionKind::NoDossier) => "no-dossier waiver",
                        Some(crate::model::ExemptionKind::Backfilled) => "backfilled",
                        None => "exempt (legacy)",
                    },
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
            // REQ-0212: the dossier as sub-panels — the result strip up top
            // (verdict, co-sign, exemption, anchor), then one sub-panel per
            // stage instead of a bulleted list.
            format!(
                "<div class=\"card\"><h2>Verification dossier</h2>\
                 <div class=\"sub result\"><h3>Result</h3>\
                   <p>verdict {vbadge} &middot; co-sign: {conf} {exempt}</p>\
                   {stmt}{anchored}</div>\
                 <div class=\"subs\">\
                   <div class=\"sub\"><h3>Plan</h3><p>{plan}</p></div>\
                   {an}\
                   {te}\
                 </div>{cov}</div>",
                plan = if v.plan.is_empty() {
                    "—".to_string()
                } else {
                    h(&v.plan)
                },
                an = stage("Analysis", &v.analysis),
                te = stage("Testing", &v.testing),
                stmt = v
                    .statement
                    .as_ref()
                    .map(|s| format!("<p><strong>Statement.</strong> {}</p>", h(s)))
                    .unwrap_or_default(),
            )
        }
    }
}

// REQ-0209: the true standing behind a Verified flag — genuine vs exempt vs
// stale vs unconfirmed — rendered as a badge, so a browser review can tell
// real verification from a carried-forward anchor at a glance.
fn prov_badge(state: &AppState, v: Option<&crate::model::Verification>, id: &str) -> String {
    use crate::commands::provenance::{classify, Provenance};
    let root = state
        .file
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    let p = classify(v, Some(&root), id);
    let kind = match p {
        Provenance::Genuine => "ok",
        Provenance::Stale | Provenance::Ungated => "bad",
        _ => "warn",
    };
    badge(kind, p.as_str())
}

// REQ-0209: the recorded test-record list — commit, actor, outcome, evidence
// kind, external source — mirroring what `req verification show` prints.
// REQ-0212: only the LATEST record is shown by default; the earlier ones are
// behind a collapsed disclosure so the page leads with the current evidence.
fn test_records_html(tests: &[crate::model::TestRecord]) -> String {
    if tests.is_empty() {
        return String::new();
    }
    let row = |t: &crate::model::TestRecord| {
        let ext = t
            .external
            .as_ref()
            .map(|e| {
                format!(
                    "{}{}{}",
                    h(&e.system),
                    e.environment
                        .as_deref()
                        .map(|v| format!(" / {}", h(v)))
                        .unwrap_or_default(),
                    e.raw_verdict
                        .as_deref()
                        .map(|v| format!(" ({})", h(v)))
                        .unwrap_or_default(),
                )
            })
            .unwrap_or_else(|| "—".into());
        format!(
            "<tr><td>{}</td><td>{}</td><td><code>{}</code></td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            t.at.format("%Y-%m-%d %H:%M"),
            h(&t.actor),
            h(&t.commit[..t.commit.len().min(9)]),
            if matches!(t.outcome, crate::model::TestOutcome::Pass) {
                badge("ok", "pass")
            } else {
                badge("bad", "fail")
            },
            h(t.kind.as_str()),
            ext,
            h(&t.notes),
        )
    };
    let thead =
        "<thead><tr><th>At</th><th>Actor</th><th>Commit</th><th>Outcome</th><th>Kind</th><th>External</th><th>Notes</th></tr></thead>";
    // REQ-0212: ONE table — the latest record visible, the earlier rows in
    // the same table (same header) hidden behind a reveal link.
    let latest = tests.last().map(row).unwrap_or_default();
    let earlier: Vec<String> = tests[..tests.len() - 1]
        .iter()
        .rev()
        .map(|t| row(t).replacen("<tr>", "<tr class=\"older\">", 1))
        .collect();
    let reveal = if earlier.is_empty() {
        String::new()
    } else {
        format!(
            "<p><a class=\"reveal-older\" data-show=\"show {n} earlier record(s)\" \
             data-hide=\"hide earlier record(s)\">show {n} earlier record(s)</a></p>",
            n = earlier.len()
        )
    };
    format!(
        "<div class=\"card\"><h2>Test records</h2>\
         <p class=\"meta\">latest of {}</p>\
         <table>{}<tbody>{}{}</tbody></table>{}</div>",
        tests.len(),
        thead,
        latest,
        earlier.join(""),
        reveal
    )
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
                        .map(|sf| sf_badge(project, sf))
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
            sf_badge(project, sf),
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
    let (sk, st) = haz_standing(project, hz);
    let adequacy = format!(
        "{}{}",
        adequacy_html(hz, project),
        signoff_card(crate::commands::safety::hazard_signoff_lines(project, hz))
    );
    // REQ-0212: the same panelled treatment as the requirement page — a
    // status-tinted header band, then one card per section.
    Some(format!(
        "{crumbs}\
         <div class=\"hdr st-{status_txt}\">\
           <h1>{id} <small>{title}</small></h1>\
           <ul class=\"meta\"><li>{standing}</li> \
           <li><strong>Status:</strong> {status}</li> \
           <li><strong>Required SIL:</strong> {sil}</li></ul>\
         </div>\
         <div class=\"card\"><h2>Harm</h2><p>{harm}</p>{risk}</div>\
         {adequacy}\
         <div class=\"card\"><h2>Mitigation chain</h2><ul>{chain}</ul></div>",
        crumbs = breadcrumbs(project, &id),
        id = h(&hz.id),
        title = h(&hz.title),
        standing = badge(sk, st),
        status = hz.status.as_str(),
        status_txt = hz.status.as_str(),
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
    let (sk, st) = sf_standing(project, sf);
    let dossier = format!(
        "{}{}",
        dossier_html(sf.verification.as_ref(), project),
        signoff_card(crate::commands::safety::sf_signoff_lines(project, sf))
    );
    // REQ-0212: same panelled treatment — status-tinted header, cards below.
    Some(format!(
        "{crumbs}\
         <div class=\"hdr st-{status_txt}\">\
           <h1>{id} <small>{title}</small></h1>\
           <ul class=\"meta\"><li>{standing}</li> \
           <li><strong>Status:</strong> {status}</li> \
           <li><strong>Allocated SIL:</strong> {sil}</li></ul>\
         </div>\
         {safe}\
         {dossier}\
         <div class=\"card\"><h2>Mitigates (hazards)</h2><ul>{haz}</ul></div>\
         <div class=\"card\"><h2>Realized by (safety requirements)</h2><ul>{srs}</ul></div>",
        crumbs = breadcrumbs(project, &id),
        id = h(&sf.id),
        title = h(&sf.title),
        standing = badge(sk, st),
        status = sf.status.as_str(),
        status_txt = sf.status.as_str(),
        sil = sil_s(project.allocated_sil(sf)),
        safe = if sf.safe_state.is_empty() {
            String::new()
        } else {
            format!(
                "<div class=\"card\"><h2>Safe state</h2><p>{}</p></div>",
                h(&sf.safe_state)
            )
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
    // REQ-0212: same panelled treatment — status-tinted header, cards below,
    // the test-record list leading with the latest record.
    Some(format!(
        "{crumbs}\
         <div class=\"hdr st-{status_txt}\">\
           <h1>{id} <small>{title}</small></h1>\
           <ul class=\"meta\"><li>{standing}</li> \
           <li><strong>Status:</strong> {status}</li> \
           <li><strong>Inherited SIL:</strong> {sil}</li></ul>\
           {walk}\
         </div>\
         <div class=\"card\"><h2>Statement</h2><p>{stmt}</p></div>\
         <div class=\"card\"><h2>Realizes (safety functions)</h2><ul>{sfs}</ul></div>\
         {dossier}\
         {records}",
        crumbs = breadcrumbs(project, &id),
        id = h(&sr.id),
        title = h(&sr.title),
        standing = sr_badge(sr),
        status = sr.status.as_str(),
        status_txt = sr.status.as_str(),
        sil = sil_s(project.inherited_sil(sr)),
        walk = walkthrough_html(sr),
        stmt = h(&sr.statement),
        sfs = if sfs.is_empty() {
            "<li class=\"meta\">none</li>".into()
        } else {
            sfs.join("")
        },
        dossier = dossier_html(sr.verification.as_ref(), project),
        records = test_records_html(&sr.tests),
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

// REQ-0212: common page chrome — every served page shares a persistent header
// with links to the requirements index, the verification roll-up, and the
// safety view, so a human reviewer can move between the review surfaces
// without knowing the URL scheme. A `#filter` input, when a page includes
// one, filters its `.filterable` table rows client-side.
fn page(title: &str, body: &str) -> String {
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>{title}</title><style>\
         body{{font-family:system-ui,sans-serif;max-width:72rem;margin:0 auto 2rem;padding:0 1rem;line-height:1.5;color:#222;}}\
         nav{{display:flex;gap:1.2rem;align-items:baseline;border-bottom:1px solid #e5e5e5;padding:.7rem 0;margin-bottom:1.2rem;position:sticky;top:0;background:#fff;z-index:2;}}\
         nav .brand{{font-weight:700;color:#222;text-decoration:none;}}\
         nav a{{color:#0366d6;text-decoration:none;}} nav a:hover{{text-decoration:underline;}}\
         h1{{margin-bottom:.2rem;}} h1 small{{font-weight:400;color:#666;}}\
         table{{width:100%;border-collapse:collapse;margin-top:1rem;}}\
         th,td{{padding:.4rem .6rem;border-bottom:1px solid #eee;text-align:left;vertical-align:top;}}\
         th{{background:#fafafa;position:sticky;top:2.9rem;}}\
         tbody tr:hover{{background:#f6f9ff;}}\
         .meta{{color:#666;font-size:.9rem;}} ul.meta{{list-style:none;padding:0;}} ul.meta li{{display:inline-block;margin-right:1.5rem;}}\
         code{{background:#f4f4f4;padding:.1rem .3rem;border-radius:3px;font-size:.9em;}}\
         a{{color:#0366d6;}}\
         .crumb{{color:#666;font-size:.9rem;margin:.1rem 0;}} .crumb a{{color:#0366d6;}}\
         .badge{{display:inline-block;padding:.05rem .45rem;border-radius:1rem;font-size:.78rem;font-weight:600;vertical-align:middle;white-space:nowrap;}}\
         .b-ok{{background:#e6f4ea;color:#137333;}} .b-warn{{background:#fef7e0;color:#a36a00;}} .b-bad{{background:#fce8e6;color:#c5221f;}} .b-info{{background:#eef;color:#3b4cca;}} .b-mute{{background:#f1f1f1;color:#666;}}\
         .card{{border:1px solid #eaeaea;border-radius:8px;padding:.6rem 1rem;margin:.6rem 0;background:#fcfcfc;}}\
         .card h2,.card h3{{margin-top:.4rem;}} .cover{{border-left:3px solid #ddd;padding-left:.7rem;margin:.4rem 0;}}\
         .blocked{{border-left:3px solid #c5221f;}} .done{{border-left:3px solid #137333;}}\
         .hdr{{border:1px solid #eaeaea;border-radius:8px;padding:.5rem 1rem .3rem;margin:.4rem 0 .9rem;}}\
         .st-verified{{background:#e6f4ea;border-color:#b7dfc3;}}\
         .st-implemented{{background:#e8f0fe;border-color:#c6dafc;}}\
         .st-approved{{background:#fef7e0;border-color:#f3e2ac;}}\
         .st-proposed{{background:#fdf3e7;border-color:#f0ddc0;}}\
         .st-draft{{background:#f6f6f6;border-color:#e2e2e2;}}\
         .st-obsolete{{background:#f1f1f1;border-color:#ddd;color:#666;}}\
         .st-mitigated{{background:#e8f0fe;border-color:#c6dafc;}}\
         .st-assessed{{background:#fef7e0;border-color:#f3e2ac;}}\
         .st-identified{{background:#fdf3e7;border-color:#f0ddc0;}}\
         .st-allocated{{background:#fdf3e7;border-color:#f0ddc0;}}\
         .subs{{display:grid;grid-template-columns:repeat(auto-fit,minmax(17rem,1fr));gap:.6rem;margin:.5rem 0;}}\
         .sub{{border:1px solid #ececec;border-radius:6px;padding:.45rem .7rem;background:#fff;overflow-wrap:anywhere;min-width:0;}}\
         .sub h3{{margin:0 0 .25rem;font-size:.72rem;text-transform:uppercase;letter-spacing:.05em;color:#888;}}\
         .sub p{{margin:.2rem 0;}}\
         pre.findings{{white-space:pre-wrap;font-size:.8rem;background:#f8f8f8;border-radius:4px;padding:.4rem .6rem;overflow-x:auto;}}\
         .sub.result{{margin:.5rem 0;background:#fff;}}\
         details summary{{cursor:pointer;list-style-position:inside;}}\
         tr.older{{display:none;}} tr.older.shown{{display:table-row;}}\
         a.reveal-older{{font-size:.85rem;cursor:pointer;}}\
         details summary h2{{display:inline-block;margin:.2rem 0;font-size:1.15rem;vertical-align:middle;}}\
         details.card summary{{margin:-.1rem 0;}}\
         .chips{{display:flex;gap:.5rem;flex-wrap:wrap;margin:.6rem 0;}}\
         .chip{{border:1px solid #e5e5e5;border-radius:8px;padding:.25rem .7rem;background:#fafafa;font-size:.85rem;}}\
         .chip b{{font-size:1rem;}}\
         input#filter{{flex:1 1 16rem;min-width:12rem;padding:.35rem .6rem;border:1px solid #ccc;border-radius:6px;font-size:.95rem;}}\
         .filters{{display:flex;gap:.5rem;flex-wrap:wrap;align-items:center;margin:.6rem 0 0;}}\
         .filters select{{padding:.3rem .4rem;border:1px solid #ccc;border-radius:6px;font-size:.9rem;background:#fff;}}\
         </style></head><body>\
         <nav><a class=\"brand\" href=\"/\">req</a>\
         <a href=\"/\">requirements</a>\
         <a href=\"/safety\">safety</a></nav>\
         {body}\
         <script>\
         (function(){{\
         var f=document.getElementById('filter');\
         var sels=[].slice.call(document.querySelectorAll('select.rowfilter'));\
         var count=document.getElementById('filter-count');\
         if(!f&&!sels.length)return;\
         function apply(){{\
           var q=f?f.value.toLowerCase():'';\
           var shown=0,total=0;\
           [].slice.call(document.querySelectorAll('table.filterable tbody tr')).forEach(function(tr){{\
             total++;\
             var ok=!q||tr.textContent.toLowerCase().indexOf(q)>=0;\
             sels.forEach(function(s){{\
               if(!s.value)return;\
               var key=s.getAttribute('data-key');\
               var v=tr.getAttribute('data-'+key)||'';\
               if(key==='tags'){{ok=ok&&v.indexOf('|'+s.value+'|')>=0;}}\
               else{{ok=ok&&v===s.value;}}\
             }});\
             tr.style.display=ok?'':'none';\
             if(ok)shown++;\
           }});\
           if(count)count.textContent=shown===total?'':shown+' of '+total+' shown';\
         }}\
         if(f)f.addEventListener('input',apply);\
         sels.forEach(function(s){{s.addEventListener('change',apply);}});\
         }})();\
         (function(){{\
         [].slice.call(document.querySelectorAll('a.reveal-older')).forEach(function(a){{\
           a.addEventListener('click',function(){{\
             var card=a.closest('.card');if(!card)return;\
             var rows=[].slice.call(card.querySelectorAll('tr.older'));\
             var showing=rows.length&&rows[0].classList.contains('shown');\
             rows.forEach(function(tr){{tr.classList.toggle('shown',!showing);}});\
             a.textContent=showing?a.getAttribute('data-show'):a.getAttribute('data-hide');\
           }});\
         }});\
         }})();\
         </script></body></html>",
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
