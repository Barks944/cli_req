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
