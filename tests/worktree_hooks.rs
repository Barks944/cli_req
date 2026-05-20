// REQ-0117: hooks install must work inside a git worktree. In a
// worktree, `.git` is a file pointing at `.git/worktrees/<name>/`
// which has no `hooks/` subdir; hooks live in `<main>/.git/hooks`
// (the --git-common-dir path). This test confirms the install lands
// the script in the shared hooks directory, not the per-worktree one.
#[allow(dead_code)]
mod common;
use common::Sandbox;
use std::fs;
use std::process::Command;

fn run(cmd: &[&str], cwd: &std::path::Path) -> std::process::Output {
    Command::new(cmd[0])
        .args(&cmd[1..])
        .current_dir(cwd)
        .output()
        .expect("spawn")
}

#[test]
fn req_0117_hooks_install_lands_shared_dir_in_worktree() {
    // Skip if git is unavailable on the test host.
    if run(
        &["git", "--version"],
        std::env::current_dir().unwrap().as_path(),
    )
    .status
    .code()
        != Some(0)
    {
        eprintln!("git not on PATH, skipping");
        return;
    }

    let s = Sandbox::new();
    let main = s.dir.path().join("main");
    let wt = s.dir.path().join("wt");
    fs::create_dir_all(&main).unwrap();

    // Initialise main repo + a single commit so worktree add succeeds.
    let out = run(&["git", "init", "-q"], &main);
    assert!(out.status.success(), "git init");
    let _ = run(&["git", "config", "user.email", "t@e"], &main);
    let _ = run(&["git", "config", "user.name", "t"], &main);
    let _ = run(&["git", "config", "commit.gpgsign", "false"], &main);
    fs::write(main.join("README.md"), "seed\n").unwrap();
    let _ = run(&["git", "add", "-A"], &main);
    let out = run(&["git", "commit", "-q", "-m", "seed"], &main);
    assert!(out.status.success(), "seed commit");

    // Create a worktree at <tmp>/wt.
    let out = run(
        &["git", "worktree", "add", "-q", wt.to_str().unwrap()],
        &main,
    );
    assert!(
        out.status.success(),
        "worktree add: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Now run `req hooks install --repo <worktree>`. The fix is that
    // resolve_hooks_dir uses --git-common-dir, so this should write
    // to <main>/.git/hooks/pre-commit, not into the worktree's tiny
    // per-worktree .git dir.
    let req_out = Command::new(env!("CARGO_BIN_EXE_req"))
        .args([
            "hooks",
            "install",
            "--repo",
            wt.to_str().unwrap(),
            "--force",
        ])
        .output()
        .expect("invoke req");
    assert!(
        req_out.status.success(),
        "req hooks install in worktree: {}",
        String::from_utf8_lossy(&req_out.stderr)
    );

    // Pre-commit hook should be in the SHARED hooks dir.
    let shared_hook = main.join(".git/hooks/pre-commit");
    assert!(
        shared_hook.exists(),
        "shared hooks/pre-commit not written at {}",
        shared_hook.display()
    );

    // The per-worktree hooks dir should NOT carry a managed pre-commit
    // (there was no hooks/ to start with).
    let per_wt = s.dir.path().join("main/.git/worktrees/wt/hooks/pre-commit");
    assert!(
        !per_wt.exists(),
        "per-worktree pre-commit should not exist: {}",
        per_wt.display()
    );
}
