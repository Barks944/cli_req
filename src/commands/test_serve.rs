// REQ-0208: live HTTP serve mode for an external test orchestrator (e.g. the
// AugTronic at_test bench). Complements the batch `req test requests`/`ingest`
// contract in `integration.rs` by giving it a live transport with `req` as the
// server:
//
//   GET  /test/requirements  full requirement set (same shape as `req list --json`)
//   GET  /test/requests      the due-for-verification hint (`req-test-request-v1`)
//   POST /test/results       ingest verdicts as STAGED evidence (`req-test-result-v1`)
//   GET  /test/health        liveness + served commit
//
// The server holds an exclusive lock on project.req for its lifetime (one serve
// per file), is the sole writer, and NEVER concludes or promotes a requirement
// — closeout stays a human step. Run it on a worktree checked out at the commit
// under test; every ingested record is anchored to the commit in the payload.
use anyhow::{anyhow, Context, Result};
use axum::{
    extract::State,
    http::{header::AUTHORIZATION, HeaderMap, StatusCode},
    response::Json,
    routing::{get, post},
    Router,
};
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::cli::TestServeArgs;
use crate::commands::integration::{self, ResultPayload};
use crate::model::{Project, Requirement};
use crate::storage::{self, LockGuard};

struct AppState {
    path: PathBuf,
    project: Mutex<Project>,
    token: String,
    // Held for the whole server lifetime: one `req test serve` per project.req.
    _lock: LockGuard,
}

type ApiError = (StatusCode, String);

pub fn run(args: TestServeArgs, file: &Option<PathBuf>) -> Result<()> {
    let path = storage::resolve_path(file);

    // One serve per project.req: take the exclusive advisory lock and hold it
    // for the whole server lifetime. Fails fast if another serve — or any other
    // req mutation — already holds it.
    let lock = storage::acquire_lock(&path).context(
        "could not lock project.req — is another `req test serve` (or a req mutation) running?",
    )?;
    let project = storage::load(&path).context("load project before binding socket")?;

    // Token: use the supplied one, else mint a fresh random token and print it
    // so an agent/operator can copy it into the test system.
    let token = match &args.token {
        Some(t) if !t.trim().is_empty() => t.clone(),
        Some(_) => return Err(anyhow!("--token must not be empty")),
        None => {
            let t = mint_token();
            println!("req test serve: generated bearer token (copy into your test system):");
            println!("    {t}");
            t
        }
    };

    let state = Arc::new(AppState {
        path: path.clone(),
        project: Mutex::new(project),
        token,
        _lock: lock,
    });

    let app = Router::new()
        .route("/test/health", get(health))
        .route("/test/requirements", get(requirements))
        .route("/test/requests", get(requests))
        .route("/test/results", post(results))
        .with_state(state);

    let addr: SocketAddr = format!("{}:{}", args.host, args.port)
        .parse()
        .map_err(|e| anyhow!("invalid bind address: {e}"))?;

    println!("req test serve: http://{addr} (Ctrl-C to stop)");
    println!("  serving {}", path.display());
    if let Some(env) = &args.environment {
        println!("  environment label: {env}");
    }
    println!("  records evidence only — never concludes or promotes (human closeout).");

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("start tokio runtime")?;
    rt.block_on(async move {
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .with_context(|| format!("bind {addr}"))?;
        axum::serve(listener, app)
            .with_graceful_shutdown(shutdown())
            .await
            .context("serve")
    })
}

async fn shutdown() {
    let _ = tokio::signal::ctrl_c().await;
    eprintln!("\nreq test serve: shutting down.");
}

fn mint_token() -> String {
    let mut buf = [0u8; 24];
    getrandom::getrandom(&mut buf).expect("OS RNG unavailable");
    hex::encode(buf)
}

fn check_auth(headers: &HeaderMap, token: &str) -> Result<(), ApiError> {
    let unauth = || {
        (
            StatusCode::UNAUTHORIZED,
            "missing or invalid bearer token".to_string(),
        )
    };
    let header = headers
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(unauth)?;
    // Scheme is case-insensitive per RFC 7235; the token compare is constant-time.
    let (scheme, presented) = header.split_once(' ').ok_or_else(unauth)?;
    if scheme.eq_ignore_ascii_case("bearer") && constant_time_eq(presented, token) {
        Ok(())
    } else {
        Err(unauth())
    }
}

/// Length-checked constant-time byte comparison — avoids leaking the token via
/// early-exit timing. (The length may leak; the token length is fixed.)
fn constant_time_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Recover from a poisoned lock: a handler panic must not brick the server. The
/// staged-ingest transaction (clone → save → swap) keeps the shared Project
/// consistent even when a mutation errors, so the inner value is safe to reuse.
fn lock_project(st: &AppState) -> std::sync::MutexGuard<'_, Project> {
    st.project.lock().unwrap_or_else(|e| e.into_inner())
}

async fn health(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    check_auth(&headers, &st.token)?;
    let project = lock_project(&st);
    Ok(Json(json!({
        "status": "ok",
        "project": project.name,
        "commit": integration::current_head(),
        "requirements": project.requirements.len(),
    })))
}

async fn requirements(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    check_auth(&headers, &st.token)?;
    let project = lock_project(&st);
    // Same shape as `req list --json --include-obsolete`: a bare array of the
    // full Requirement objects, so at_test's existing parser works unchanged.
    let all: Vec<&Requirement> = project.requirements.values().collect();
    let body = serde_json::to_value(&all)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("serialize: {e}")))?;
    Ok(Json(body))
}

async fn requests(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    check_auth(&headers, &st.token)?;
    let project = lock_project(&st);
    let payload = integration::build_request(&project);
    let body = serde_json::to_value(&payload)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("serialize: {e}")))?;
    Ok(Json(body))
}

async fn results(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    body: String,
) -> Result<Json<Value>, ApiError> {
    check_auth(&headers, &st.token)?;
    let payload: ResultPayload = serde_json::from_str(&body).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("invalid result payload ({e}). See `req schema test-result`."),
        )
    })?;
    // Transactional: ingest into a clone, persist, then swap the shared state in
    // only on full success. An error (or a panic) leaves the in-memory Project
    // untouched, so a bad request can't leak partial state into a later save.
    let mut guard = lock_project(&st);
    let mut candidate = guard.clone();
    let report = integration::ingest_payload_staged(&mut candidate, &payload)
        .map_err(|e| (StatusCode::UNPROCESSABLE_ENTITY, e.to_string()))?;
    storage::save(&st.path, &candidate)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("save: {e}")))?;
    *guard = candidate;
    Ok(Json(json!({
        "attached": report.attached,
        "skipped_duplicate": report.skipped_duplicate,
        "dossiers": report.dossiers,
        "preserved": report.preserved,
    })))
}
