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

#[allow(dead_code)]
pub fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}
#[allow(dead_code)]
pub fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}
