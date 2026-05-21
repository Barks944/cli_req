// REQ-0124: coverage / lint / review must respect .gitignore so
// ignored artefact paths (tmp/, dist/, build outputs) never appear as
// ghost references. Walker delegated to the `ignore` crate.
mod common;
use common::{stdout, Sandbox};
use std::fs;
use std::process::Command;

fn init_git(path: &std::path::Path) {
    let _ = Command::new("git")
        .args(["init", "-q"])
        .current_dir(path)
        .output();
    let _ = Command::new("git")
        .args(["config", "user.email", "t@e"])
        .current_dir(path)
        .output();
    let _ = Command::new("git")
        .args(["config", "user.name", "t"])
        .current_dir(path)
        .output();
}

#[test]
fn req_0124_coverage_skips_gitignored_paths() {
    let s = Sandbox::new();
    init_git(s.dir.path());
    s.init("p");

    // Ignored directory with a stray REQ-marker that would otherwise
    // become a ghost in the coverage report.
    fs::write(s.dir.path().join(".gitignore"), "tmp/\n").unwrap();
    fs::create_dir_all(s.dir.path().join("tmp")).unwrap();
    fs::write(
        s.dir.path().join("tmp/scratch.rs"),
        "// REQ-9999: scratch\nfn nope() {}\n",
    )
    .unwrap();

    // Tracked source with a legit marker.
    fs::create_dir_all(s.dir.path().join("src")).unwrap();
    fs::write(
        s.dir.path().join("src/lib.rs"),
        "// REQ-0001: real reference\nfn ok() {}\n",
    )
    .unwrap();

    let out = s.run(&[
        "coverage",
        "--path",
        s.dir.path().to_str().unwrap(),
        "--json",
    ]);
    let body = stdout(&out);
    let v: serde_json::Value = serde_json::from_str(&body).expect("JSON");
    let ghosts = v["ghosts"].as_object().expect("ghosts object");
    assert!(
        !ghosts.contains_key("REQ-9999"),
        ".gitignored REQ-9999 must not appear as a ghost; got: {}",
        body
    );
}

#[test]
fn req_0124_coverage_unlinked_files_honours_gitignore() {
    let s = Sandbox::new();
    init_git(s.dir.path());
    s.init("p");

    fs::write(s.dir.path().join(".gitignore"), "build/\n").unwrap();
    fs::create_dir_all(s.dir.path().join("build")).unwrap();
    fs::write(s.dir.path().join("build/output.rs"), "fn ignored() {}\n").unwrap();

    let out = s.run(&[
        "coverage",
        "--unlinked-files",
        "--path",
        s.dir.path().to_str().unwrap(),
        "--json",
    ]);
    let body = stdout(&out);
    assert!(
        !body.contains("build/output.rs") && !body.contains("build\\output.rs"),
        "ignored build/output.rs must not appear in --unlinked-files; got: {}",
        body
    );
}
