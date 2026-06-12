// Shared helpers for integration tests. Each test gets its own tempdir +
// project.req so parallel tests cannot collide.
//
// Tests in this suite are named `req_NNNN_description` where NNNN is the
// 4-digit ID of the requirement they cover. `req test run` parses test
// names with that prefix to map outcomes back to requirements.

use std::path::PathBuf;
use std::process::{Command, Output};

pub struct Sandbox {
    pub dir: tempfile::TempDir,
}

impl Sandbox {
    pub fn new() -> Self {
        let dir = tempfile::Builder::new()
            .prefix("req-test-")
            .tempdir()
            .expect("create tempdir");
        Self { dir }
    }
    pub fn path(&self) -> PathBuf {
        self.dir.path().join("project.req")
    }
    pub fn init(&self, name: &str) {
        let out = req(&["init", "-n", name, "-o", self.path().to_str().unwrap()]);
        assert!(
            out.status.success(),
            "req init failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    pub fn run(&self, args: &[&str]) -> Output {
        let path = self.path();
        let path_s = path.to_str().unwrap().to_string();
        let mut full: Vec<&str> = vec!["--file", path_s.as_str()];
        full.extend(args.iter().copied());
        req(&full)
    }
}

pub fn req(args: &[&str]) -> Output {
    let bin = env!("CARGO_BIN_EXE_req");
    Command::new(bin)
        .args(args)
        .env_remove("REQ_FILE")
        .output()
        .expect("invoke req binary")
}

/// REQ-0138: write a safety-acceptance file beside `project_file` so a
/// test can exercise the gated safety features. This mirrors the
/// committed artifact a human's `req safety accept` produces — and since
/// the gate is "file present", writing it directly is a faithful way to
/// enable safety without an interactive terminal.
#[allow(dead_code)]
pub fn enable_safety(project_file: &std::path::Path) {
    let dir = if project_file.is_dir() {
        project_file.to_path_buf()
    } else {
        project_file
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| std::path::PathBuf::from("."))
    };
    let body = r#"{"accepted_by":"Test","at":"2026-01-01T00:00:00Z","tool_version":"test","disclaimer_version":"1"}"#;
    std::fs::write(dir.join("req-safety-acceptance.json"), body).expect("write acceptance file");
}

impl Sandbox {
    /// Enable the functional-safety features for this sandbox project.
    #[allow(dead_code)]
    pub fn enable_safety(&self) {
        enable_safety(&self.path());
    }
}

#[allow(dead_code)]
pub fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}
#[allow(dead_code)]
pub fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}
