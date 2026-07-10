// REQ-0208: `req test serve` — live HTTP serve mode for an external test system.
// Spawns the server, hits it over raw TCP (no HTTP-client dep), asserts auth,
// the served requirement views, and that ingested results land as STAGED
// evidence (dossier left open, status never promoted) and that a rejected
// result leaves project.req byte-identical without bricking the server.
mod common;
use common::{stdout, Sandbox};
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const HOST: &str = "127.0.0.1";

fn pick_free_port() -> u16 {
    let listener = std::net::TcpListener::bind(format!("{}:0", HOST)).expect("bind ephemeral");
    listener.local_addr().expect("local_addr").port()
}

fn spawn_serve(s: &Sandbox, port: u16, token: &str) -> Child {
    Command::new(env!("CARGO_BIN_EXE_req"))
        .args([
            "--file",
            s.path().to_str().unwrap(),
            "test",
            "serve",
            "--host",
            HOST,
            "--port",
            &port.to_string(),
            "--token",
            token,
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn req test serve")
}

fn wait_for_bind(port: u16, timeout: Duration) -> bool {
    let addr = format!("{}:{}", HOST, port)
        .to_socket_addrs()
        .unwrap()
        .next()
        .unwrap();
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_ok() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(75));
    }
    false
}

/// Minimal HTTP/1.1 request over raw TCP. Returns (status_code, body).
fn http(
    port: u16,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Option<&str>,
) -> (u16, String) {
    let mut stream =
        TcpStream::connect(format!("{}:{}", HOST, port)).expect("connect to test-serve");
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
    let body = body.unwrap_or("");
    let mut req =
        format!("{method} {path} HTTP/1.1\r\nHost: {HOST}:{port}\r\nConnection: close\r\n");
    if let Some(t) = token {
        req.push_str("Authorization: Bearer ");
        req.push_str(t);
        req.push_str("\r\n");
    }
    if method == "POST" {
        req.push_str("Content-Type: application/json\r\n");
        req.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    req.push_str("\r\n");
    req.push_str(body);
    stream.write_all(req.as_bytes()).expect("write request");
    let mut buf = String::new();
    stream.read_to_string(&mut buf).ok();
    let (status_line, rest) = buf.split_once("\r\n").unwrap_or((buf.as_str(), ""));
    let resp_body = rest
        .split_once("\r\n\r\n")
        .map(|x| x.1)
        .unwrap_or("")
        .to_string();
    let code = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(0);
    (code, resp_body)
}

/// Kills the child server even if a test panics.
struct GuardedChild(Option<Child>);
impl Drop for GuardedChild {
    fn drop(&mut self) {
        if let Some(mut c) = self.0.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

/// Sandbox holding one Implemented requirement (REQ-0001).
fn implemented_sandbox() -> Sandbox {
    let s = Sandbox::new();
    s.init("p");
    s.run(&[
        "add",
        "-t",
        "Stop on demand here",
        "-s",
        "The system shall stop on demand.",
        "-r",
        "operator safety",
        "-k",
        "functional",
        "-a",
        "process halts within one second",
    ]);
    for st in ["proposed", "approved", "implemented"] {
        s.run(&["update", "REQ-0001", "--status", st, "--reason", "step"]);
    }
    s
}

#[test]
fn req_0208_serve_requires_auth_and_serves_requirements() {
    let s = implemented_sandbox();
    let port = pick_free_port();
    let _child = GuardedChild(Some(spawn_serve(&s, port, "tok")));
    assert!(
        wait_for_bind(port, Duration::from_secs(10)),
        "server never bound"
    );

    // No token -> 401, before any project data is served.
    let (code, _) = http(port, "GET", "/test/requirements", None, None);
    assert_eq!(code, 401);

    // With token -> the full requirement set (req list --json shape).
    let (code, body) = http(port, "GET", "/test/requirements", Some("tok"), None);
    assert_eq!(code, 200);
    assert!(body.contains("REQ-0001"), "requirements body: {body}");

    // The due-for-verification hint lists the Implemented requirement.
    let (code, body) = http(port, "GET", "/test/requests", Some("tok"), None);
    assert_eq!(code, 200);
    assert!(
        body.contains("req-test-request-v1"),
        "requests body: {body}"
    );
    assert!(body.contains("REQ-0001"), "requests body: {body}");
}

#[test]
fn req_0208_serve_ingest_is_staged_not_concluded() {
    let s = implemented_sandbox();
    let port = pick_free_port();
    {
        let _child = GuardedChild(Some(spawn_serve(&s, port, "tok")));
        assert!(
            wait_for_bind(port, Duration::from_secs(10)),
            "server never bound"
        );
        let payload = r#"{"schema":"req-test-result-v1","system":"at_test","commit":"deadbeef","results":[{"req_id":"REQ-0001","verdict":"pass","notes":"bench pass","decision":{"plan":"sweep the range","analysis":"logic sound"}}]}"#;
        let (code, body) = http(port, "POST", "/test/results", Some("tok"), Some(payload));
        assert_eq!(code, 200, "body: {body}");
        assert!(body.contains("\"attached\":1"), "body: {body}");
    } // child dropped -> killed -> file lock released

    // Status stays Implemented — the bench never promotes.
    let show = stdout(&s.run(&["show", "REQ-0001"]));
    assert!(show.contains("implemented"), "status: {show}");

    // Dossier is OPEN: plan/analysis/testing recorded, verdict not concluded.
    let dossier = stdout(&s.run(&["verification", "show", "REQ-0001"]));
    assert!(dossier.contains("sweep the range"), "dossier: {dossier}");
    assert!(
        dossier.contains("not concluded") || dossier.contains("pending"),
        "dossier should be open (awaiting human closeout): {dossier}"
    );
}

#[test]
fn req_0208_serve_rejects_bad_result_and_stays_alive() {
    let s = implemented_sandbox();
    let port = pick_free_port();
    let _child = GuardedChild(Some(spawn_serve(&s, port, "tok")));
    assert!(
        wait_for_bind(port, Duration::from_secs(10)),
        "server never bound"
    );

    let before = std::fs::read(s.path()).expect("read before");

    // Unknown requirement -> 422, project.req untouched, and the server survives.
    // Constructed via format! so the four-digit literal never appears in this
    // source (the project-wide coverage scan would flag it as a ghost marker).
    let bad = format!(
        r#"{{"schema":"req-test-result-v1","system":"at_test","commit":"c","results":[{{"req_id":"REQ-{:04}","verdict":"pass"}}]}}"#,
        9999
    );
    let (code, _) = http(port, "POST", "/test/results", Some("tok"), Some(&bad));
    assert_eq!(code, 422);

    let after = std::fs::read(s.path()).expect("read after");
    assert_eq!(
        before, after,
        "project.req must be byte-identical after a rejected result"
    );

    // Still alive and functional after the rejected request (no lock poisoning).
    let (code, _) = http(port, "GET", "/test/health", Some("tok"), None);
    assert_eq!(code, 200);
    let good = r#"{"schema":"req-test-result-v1","system":"at_test","commit":"c","results":[{"req_id":"REQ-0001","verdict":"pass","decision":{"plan":"p","analysis":"a"}}]}"#;
    let (code, body) = http(port, "POST", "/test/results", Some("tok"), Some(good));
    assert_eq!(code, 200, "body: {body}");
}
