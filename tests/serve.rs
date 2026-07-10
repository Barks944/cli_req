// Tests for REQ-0016 (local read-only web server). Spawns `req serve`,
// hits each route over raw TCP (avoids adding an HTTP client dep), kills
// the child, asserts status codes + response shape.
mod common;
use common::Sandbox;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const HOST: &str = "127.0.0.1";

/// Pick a free port by binding ephemerally and immediately dropping. Race
/// window is tiny; tests serialise via --test-threads=1 anyway.
fn pick_free_port() -> u16 {
    let listener = std::net::TcpListener::bind(format!("{}:0", HOST)).expect("bind ephemeral");
    listener.local_addr().expect("local_addr").port()
}

fn spawn_server(s: &Sandbox, port: u16) -> Child {
    Command::new(env!("CARGO_BIN_EXE_req"))
        .args([
            "--file",
            s.path().to_str().unwrap(),
            "serve",
            "--host",
            HOST,
            "--port",
            &port.to_string(),
            "--read-only",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn req serve")
}

fn wait_for_bind(port: u16, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if TcpStream::connect_timeout(
            &format!("{}:{}", HOST, port).parse().unwrap(),
            Duration::from_millis(200),
        )
        .is_ok()
        {
            return true;
        }
        std::thread::sleep(Duration::from_millis(75));
    }
    false
}

/// Minimal HTTP/1.1 GET: returns (status_code, body).
fn http_get(port: u16, path: &str) -> (u16, String) {
    let mut stream = TcpStream::connect(format!("{}:{}", HOST, port)).expect("connect to server");
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
    write!(
        stream,
        "GET {} HTTP/1.1\r\nHost: {}:{}\r\nConnection: close\r\n\r\n",
        path, HOST, port
    )
    .expect("write request");
    let mut buf = String::new();
    stream.read_to_string(&mut buf).ok();
    // Parse the status line and split off the body.
    let mut lines = buf.splitn(2, "\r\n");
    let status_line = lines.next().unwrap_or("");
    let rest = lines.next().unwrap_or("");
    let body = rest
        .split_once("\r\n\r\n")
        .map(|x| x.1)
        .unwrap_or("")
        .to_string();
    // Status line shape: HTTP/1.1 200 OK
    let code = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(0);
    (code, body)
}

/// Helper that always kills the child even on panic.
struct GuardedChild(Option<Child>);
impl Drop for GuardedChild {
    fn drop(&mut self) {
        if let Some(mut c) = self.0.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

fn fixture() -> (Sandbox, GuardedChild, u16) {
    let s = Sandbox::new();
    s.init("p");
    // Stage one known requirement so route /r/REQ-0001 has something to return.
    let _ = s.run(&[
        "add",
        "--title",
        "Hosted on the local web server for inspection",
        "--statement",
        "The system shall render this requirement at GET /r/REQ-0001.",
        "--rationale",
        "Fixture for serve smoke tests.",
        "--kind",
        "constraint",
        "--priority",
        "could",
    ]);
    let port = pick_free_port();
    let child = spawn_server(&s, port);
    let bound = wait_for_bind(port, Duration::from_secs(10));
    assert!(
        bound,
        "req serve did not bind to {}:{} within 10s",
        HOST, port
    );
    (s, GuardedChild(Some(child)), port)
}

// ---------- REQ-0016 ----------

#[test]
fn req_0016_serve_root_returns_html_index() {
    let (_s, _child, port) = fixture();
    let (code, body) = http_get(port, "/");
    assert_eq!(code, 200, "index should return 200, got {}", code);
    assert!(
        body.contains("<html"),
        "body should be HTML: {}",
        &body[..body.len().min(200)]
    );
    assert!(body.contains("REQ-0001"), "index should list REQ-0001");
}

#[test]
fn req_0016_serve_show_route_returns_html_detail() {
    let (_s, _child, port) = fixture();
    let (code, body) = http_get(port, "/r/REQ-0001");
    assert_eq!(code, 200);
    assert!(body.contains("REQ-0001"));
    assert!(body.contains("Hosted on the local web server"));
    assert!(body.contains("Statement") || body.contains("statement"));
}

#[test]
fn req_0016_serve_api_list_returns_json_array() {
    let (_s, _child, port) = fixture();
    let (code, body) = http_get(port, "/api/list");
    assert_eq!(code, 200);
    let v: serde_json::Value = serde_json::from_str(&body)
        .unwrap_or_else(|_| panic!("/api/list should return JSON, got: {}", body));
    let arr = v.as_array().expect("array of requirements");
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["id"].as_str().unwrap(), "REQ-0001");
}

#[test]
fn req_0016_serve_api_show_returns_json_object() {
    let (_s, _child, port) = fixture();
    let (code, body) = http_get(port, "/api/r/REQ-0001");
    assert_eq!(code, 200);
    let v: serde_json::Value = serde_json::from_str(&body).expect("json object");
    assert_eq!(v["id"].as_str().unwrap(), "REQ-0001");
}

#[test]
fn req_0016_serve_unknown_id_returns_404() {
    let (_s, _child, port) = fixture();
    // Construct via format! so the four-digit literal never appears in
    // this source (project-wide coverage scan would otherwise pick it
    // up as a ghost marker).
    let bogus = format!("REQ-{:04}", 9999);
    let url = format!("/api/r/{}", bogus);
    let (code, _body) = http_get(port, &url);
    assert_eq!(code, 404);
}

#[test]
fn req_0016_serve_html_escapes_user_supplied_strings() {
    // Stage a requirement whose title contains characters the HTML
    // renderer must escape; assert they don't appear raw in the body.
    let s = Sandbox::new();
    s.init("p");
    let _ = s.run(&[
        "add",
        "--title",
        "Has <script>tag and \"quotes\" in title",
        "--statement",
        "The system shall escape these characters on render.",
        "--rationale",
        "Fixture for HTML-escape behaviour in serve.",
        "--kind",
        "constraint",
        "--priority",
        "could",
    ]);
    let port = pick_free_port();
    let child = spawn_server(&s, port);
    let _guard = GuardedChild(Some(child));
    assert!(wait_for_bind(port, Duration::from_secs(10)));
    let (code, body) = http_get(port, "/");
    assert_eq!(code, 200);
    assert!(
        !body.contains("<script>tag"),
        "raw < entity leaked through escape: {}",
        &body[..body.len().min(400)]
    );
    assert!(
        body.contains("&lt;script&gt;tag") || body.contains("&lt;script&gt;"),
        "expected &lt; entity in escaped output"
    );
}

// ---------- REQ-0134: functional-safety web view ----------

#[test]
fn req_0134_serve_safety_view_and_api() {
    let s = Sandbox::new();
    s.init("p");
    s.enable_safety();
    let _ = s.run(&[
        "hazard",
        "add",
        "-t",
        "Hazardous mode",
        "--harm",
        "operator could be hurt",
        "-C",
        "C_C",
        "-F",
        "F_B",
        "-P",
        "P_B",
        "-W",
        "W3",
    ]);
    let _ = s.run(&["sf", "add", "-t", "Interlock", "--mitigates", "HAZ-0001"]);
    let port = pick_free_port();
    let child = spawn_server(&s, port);
    assert!(
        wait_for_bind(port, Duration::from_secs(10)),
        "serve did not bind"
    );
    let _guard = GuardedChild(Some(child));

    // The index links to the safety view when hazards exist.
    let (code, body) = http_get(port, "/");
    assert_eq!(code, 200);
    assert!(body.contains("/safety"), "index should link to /safety");

    // The HARA view renders the hazard, its SIL, and the disclaimer.
    let (code, body) = http_get(port, "/safety");
    assert_eq!(code, 200, "/safety should return 200");
    assert!(body.contains("HAZ-0001"), "/safety should list the hazard");
    assert!(body.contains("SIL3"), "/safety should show the derived SIL");
    assert!(
        body.contains("not qualified per IEC 61508-3"),
        "/safety must carry the disclaimer"
    );

    // The JSON API returns the safety artifacts.
    let (code, body) = http_get(port, "/api/safety");
    assert_eq!(code, 200, "/api/safety should return 200");
    assert!(body.contains("\"hazards\""), "api should include hazards");
    assert!(body.contains("HAZ-0001"));
}

// ---------- REQ-0147: web relationship navigation across safety + verification ----------

#[test]
fn req_0147_web_navigates_safety_chain_and_verification() {
    let s = Sandbox::new();
    s.init("p");
    s.enable_safety();
    s.run(&[
        "hazard", "add", "-t", "H", "--harm", "hurt", "-C", "C_C", "-F", "F_B", "-P", "P_B", "-W",
        "W3",
    ]);
    s.run(&["sf", "add", "-t", "Stop fn", "--mitigates", "HAZ-0001"]);
    s.run(&[
        "sreq",
        "add",
        "-t",
        "Stop the blade",
        "-s",
        "The system shall stop the blade on demand.",
        "-r",
        "operator safety",
        "-a",
        "stops",
        "--realizes",
        "SF-0001",
    ]);
    s.run(&[
        "sreq", "update", "SR-0001", "--status", "approved", "--reason", "r",
    ]);
    s.run(&[
        "sreq",
        "update",
        "SR-0001",
        "--status",
        "implemented",
        "--reason",
        "r",
    ]);
    s.run(&[
        "sreq",
        "verify",
        "SR-0001",
        "--by",
        "automated",
        "--notes",
        "bench",
    ]);
    s.run(&["verification", "plan", "SR-0001", "--plan", "p"]);
    s.run(&[
        "verification",
        "analysis",
        "SR-0001",
        "--findings",
        "reviewed",
        "--result",
        "pass",
    ]);
    s.run(&[
        "verification",
        "test",
        "SR-0001",
        "--findings",
        "bench",
        "--result",
        "pass",
    ]);
    s.run(&[
        "verification",
        "conclude",
        "SR-0001",
        "--statement",
        "meets",
        "--promote",
    ]);
    s.run(&["verification", "confirm", "SR-0001"]); // human (REQ_ACTOR_KIND unset in tests)

    let port = pick_free_port();
    let _child = GuardedChild(Some(spawn_server(&s, port)));
    assert!(
        wait_for_bind(port, Duration::from_secs(10)),
        "req serve did not bind"
    );

    // /safety links each hazard into its detail page.
    let (c0, safety) = http_get(port, "/safety");
    assert_eq!(c0, 200);
    assert!(
        safety.contains("/s/HAZ-0001"),
        "safety page must link the hazard to its detail page:\n{safety}"
    );

    // The hazard page renders the full SF → SR chain (both navigable).
    let (c1, haz) = http_get(port, "/s/HAZ-0001");
    assert_eq!(c1, 200);
    assert!(
        haz.contains("/s/SF-0001") && haz.contains("/s/SR-0001"),
        "hazard page must render the SF→SR chain as links:\n{haz}"
    );

    // The safety-function page links back to the hazard and down to the SR.
    let (_c2, sf) = http_get(port, "/s/SF-0001");
    assert!(
        sf.contains("/s/HAZ-0001") && sf.contains("/s/SR-0001"),
        "SF page must link the hazard it mitigates and the SR that realizes it:\n{sf}"
    );

    // The safety-requirement page links to its function AND shows the
    // verification dossier with the human confirmation.
    let (c3, sr) = http_get(port, "/s/SR-0001");
    assert_eq!(c3, 200);
    assert!(
        sr.contains("/s/SF-0001"),
        "SR page must link the safety function it realizes:\n{sr}"
    );
    assert!(
        sr.contains("Verification dossier") && sr.contains("co-signed"),
        "SR page must show the verification dossier with the human co-sign:\n{sr}"
    );
}

// REQ-0205: the browser renders the adequacy walk-through (SF coverage notes +
// hazard adequacy dossier), breadcrumbs, and standing badges.
#[test]
fn req_0205_browser_renders_adequacy_walkthrough_and_badges() {
    let s = Sandbox::new();
    s.init("p");
    s.enable_safety();
    s.run(&[
        "hazard",
        "add",
        "-t",
        "Runaway",
        "--harm",
        "operator crushed",
        "-C",
        "C_A",
        "-F",
        "F_A",
        "-P",
        "P_A",
        "-W",
        "W1",
    ]);
    s.run(&[
        "sf",
        "add",
        "-t",
        "Estop",
        "--safe-state",
        "halted",
        "--mitigates",
        "HAZ-0001",
    ]);
    s.run(&[
        "sreq",
        "add",
        "-t",
        "Halt on demand",
        "-s",
        "The system shall halt all motion within 200 milliseconds of a demand.",
        "-r",
        "runaway injures the operator",
        "-a",
        "halts",
        "--realizes",
        "SF-0001",
    ]);
    // Record the SF→SR adequacy walk-through (cover works on an open dossier).
    s.run(&[
        "verification",
        "plan",
        "SF-0001",
        "--plan",
        "verify the function",
    ]);
    s.run(&[
        "verification",
        "cover",
        "SF-0001",
        "--child",
        "SR-0001",
        "--note",
        "SR-0001 IMPLEMENTS THE HALT",
    ]);
    // Open the hazard adequacy dossier and cover the mitigating SF.
    s.run(&[
        "hazard",
        "adequacy",
        "plan",
        "HAZ-0001",
        "--plan",
        "argue adequacy",
    ]);
    s.run(&[
        "hazard",
        "adequacy",
        "cover",
        "HAZ-0001",
        "--sf",
        "SF-0001",
        "--note",
        "ESTOP COVERS RUNAWAY",
    ]);

    let port = pick_free_port();
    let child = spawn_server(&s, port);
    let _guard = GuardedChild(Some(child));
    assert!(
        wait_for_bind(port, Duration::from_secs(10)),
        "serve did not bind"
    );

    // Landing: standing column + badges + co-sign roll-up scaffolding.
    let (_c, safety) = http_get(port, "/safety");
    assert!(
        safety.contains("Standing") && safety.contains("badge b-"),
        "landing badges:\n{safety}"
    );

    // Hazard page: the staged adequacy dossier with the per-SF coverage note + breadcrumb.
    let (_c, haz) = http_get(port, "/s/HAZ-0001");
    assert!(
        haz.contains("Mitigation adequacy")
            && haz.contains("ESTOP COVERS RUNAWAY")
            && haz.contains("Functional safety"),
        "hazard page must show the adequacy dossier + coverage + breadcrumb:\n{haz}"
    );

    // SF page: the verification dossier with the realizing-SR walk-through note.
    let (_c, sf) = http_get(port, "/s/SF-0001");
    assert!(
        sf.contains("Adequacy walk-through") && sf.contains("SR-0001 IMPLEMENTS THE HALT"),
        "SF page must show the realizing-SR adequacy walk-through:\n{sf}"
    );
}

// ---------- REQ-0209/0210/0211/0212: full review surface in the browser ----------

/// Build a requirement with a concluded dossier (with references) and a
/// second requirement left unverified, then serve.
fn dossier_fixture() -> (Sandbox, GuardedChild, u16) {
    let s = Sandbox::new();
    s.init("p");
    s.run(&[
        "add",
        "--title",
        "Render the dossier in the browser",
        "--statement",
        "The system shall render the verification dossier in the browser.",
        "--rationale",
        "review needs V&V results",
        "--kind",
        "functional",
        "--accept",
        "dossier visible",
    ]);
    for st in ["proposed", "approved", "implemented"] {
        s.run(&["update", "REQ-0001", "--status", st, "--reason", "step"]);
    }
    s.run(&[
        "verification",
        "plan",
        "REQ-0001",
        "--plan",
        "REVIEW THE RENDERER AND RUN THE SERVE TESTS",
    ]);
    s.run(&[
        "verification",
        "analysis",
        "REQ-0001",
        "--findings",
        "renderer matches the obligation",
        "--result",
        "pass",
        "--ref",
        "src/web.rs",
    ]);
    s.run(&[
        "verification",
        "test",
        "REQ-0001",
        "--findings",
        "serve suite green",
        "--result",
        "pass",
        "--ref",
        "tests/serve.rs",
    ]);
    s.run(&[
        "verification",
        "conclude",
        "REQ-0001",
        "--statement",
        "meets the obligation end to end",
        "--promote",
    ]);
    // A second, unverified requirement for the roll-up's unverified surface.
    s.run(&[
        "add",
        "--title",
        "Still unverified obligation",
        "--statement",
        "The system shall remain visible in the unverified surface.",
        "--rationale",
        "roll-up fixture requirement",
        "--kind",
        "functional",
        "--accept",
        "listed as unverified",
    ]);
    let port = pick_free_port();
    let child = spawn_server(&s, port);
    let bound = wait_for_bind(port, Duration::from_secs(10));
    assert!(bound, "req serve did not bind");
    (s, GuardedChild(Some(child)), port)
}

// REQ-0209: the requirement page carries the full verification dossier —
// plan, activities with references, statement, verdict — and the test-record
// list, plus the provenance standing.
#[test]
fn req_0209_requirement_page_renders_full_dossier() {
    let (_s, _child, port) = dossier_fixture();
    let (code, body) = http_get(port, "/r/REQ-0001");
    assert_eq!(code, 200);
    for needle in [
        "Verification dossier",
        "REVIEW THE RENDERER AND RUN THE SERVE TESTS", // plan
        "renderer matches the obligation",             // analysis findings
        "src/web.rs",                                  // analysis reference
        "serve suite green",                           // testing findings
        "meets the obligation end to end",             // statement
        "Test records",                                // record list
        "genuine",                                     // provenance standing
    ] {
        assert!(
            body.contains(needle),
            "/r/REQ-0001 must contain {needle:?}:\n{body}"
        );
    }
    // A requirement with no dossier says so explicitly.
    let (_c, body2) = http_get(port, "/r/REQ-0002");
    assert!(
        body2.contains("No verification dossier recorded"),
        "dossier-less requirement needs an explicit empty note:\n{body2}"
    );
}

// REQ-0210: the index IS the verification roll-up — a Verification column
// with the provenance standing per verified item and the dossier stage per
// unverified item, plus the provenance chips.
#[test]
fn req_0210_verification_rollup_on_index() {
    let (_s, _child, port) = dossier_fixture();
    let (code, body) = http_get(port, "/");
    assert_eq!(code, 200);
    for needle in [
        "<th>Verification</th>",
        "genuine", // REQ-0001's provenance standing
        "no-plan", // REQ-0002's dossier stage
        "data-standing=\"genuine\"",
        "data-standing=\"no-plan\"",
    ] {
        assert!(
            body.contains(needle),
            "index roll-up must contain {needle:?}:\n{body}"
        );
    }
}

// REQ-0211: with safety disabled and no artifacts, index and /safety carry an
// explicit disabled note instead of an empty section.
#[test]
fn req_0211_safety_disabled_note() {
    let (_s, _child, port) = dossier_fixture();
    let (_c, index) = http_get(port, "/");
    assert!(
        index.contains("functional safety: disabled"),
        "index must say safety is disabled:\n{index}"
    );
    let (code, safety) = http_get(port, "/safety");
    assert_eq!(code, 200);
    assert!(
        safety.contains("disabled"),
        "/safety must render the explicit disabled state:\n{safety}"
    );
}

// REQ-0211: with safety populated, /safety lists SFs and SRs with standing,
// shows each SR's walkthrough-acknowledgement state, and renders the active
// SIL calibration.
#[test]
fn req_0211_safety_page_calibration_and_walkthrough() {
    let s = Sandbox::new();
    s.init("p");
    s.enable_safety();
    s.run(&[
        "hazard", "add", "-t", "H", "--harm", "hurt", "-C", "C_C", "-F", "F_B", "-P", "P_B", "-W",
        "W3",
    ]);
    s.run(&["sf", "add", "-t", "Stop fn", "--mitigates", "HAZ-0001"]);
    s.run(&[
        "sreq",
        "add",
        "-t",
        "Stop the blade",
        "-s",
        "The system shall stop the blade on demand.",
        "-r",
        "operator safety",
        "-a",
        "stops",
        "--realizes",
        "SF-0001",
    ]);
    let port = pick_free_port();
    let _child = GuardedChild(Some(spawn_server(&s, port)));
    assert!(wait_for_bind(port, Duration::from_secs(10)), "bind");
    let (code, body) = http_get(port, "/safety");
    assert_eq!(code, 200);
    for needle in [
        "Safety functions",
        "Safety requirements",
        "SF-0001",
        "SR-0001",
        "never acknowledged", // walkthrough state
        "SIL calibration",
        "Annex D",
    ] {
        assert!(
            body.contains(needle),
            "/safety must contain {needle:?}:\n{body}"
        );
    }
}

// REQ-0212: every page carries the common nav; the index is filterable and
// badge-decorated.
#[test]
fn req_0212_nav_chrome_and_filterable_index() {
    let (_s, _child, port) = dossier_fixture();
    for path in ["/", "/r/REQ-0001", "/safety"] {
        let (code, body) = http_get(port, path);
        assert_eq!(code, 200, "{path}");
        assert!(
            body.contains("<nav>") && body.contains("href=\"/safety\""),
            "{path} must carry the common nav:\n{body}"
        );
    }
    // The filter bar: free text plus the structural dropdowns, badges on rows.
    let (_c, index) = http_get(port, "/");
    for needle in [
        "id=\"filter\"",
        "class=\"badge",
        "data-key=\"kind\"",
        "data-key=\"pri\"",
        "data-key=\"status\"",
        "data-key=\"standing\"",
        "data-key=\"tags\"",
    ] {
        assert!(
            index.contains(needle),
            "index filter bar must contain {needle:?}:\n{index}"
        );
    }
}
