// Integration tests for `req merge` — the semantic three-way merge driver
// (REQ-0207) and its safety contract that no side is ever silently dropped
// (SR-0010).
//
// Test names are prefixed with the requirement ID they cover so `req test run`
// maps outcomes back to REQ-0207 / SR-0010.

mod common;

use common::req;
use std::path::Path;
use std::process::Output;

/// Build a base project with one functional requirement, then return paths for
/// base/ours/theirs/merged, with ours and theirs seeded as copies of base.
struct Trio {
    _dir: tempfile::TempDir,
    base: std::path::PathBuf,
    ours: std::path::PathBuf,
    theirs: std::path::PathBuf,
    merged: std::path::PathBuf,
}

fn p(path: &Path) -> &str {
    path.to_str().unwrap()
}

fn add_req(file: &Path, title: &str, statement: &str, accept: &str) {
    let out = req(&[
        "--file", p(file), "add", "-t", title, "-k", "functional", "-s", statement, "-r",
        "Rationale that is long enough to satisfy the linter.", "-a", accept,
    ]);
    assert!(out.status.success(), "add failed: {}", String::from_utf8_lossy(&out.stderr));
}

fn setup() -> Trio {
    let dir = tempfile::Builder::new().prefix("req-merge-").tempdir().unwrap();
    let base = dir.path().join("base.req");
    let ours = dir.path().join("ours.req");
    let theirs = dir.path().join("theirs.req");
    let merged = dir.path().join("merged.req");
    let out = req(&["init", "-n", "merge-test", "-o", p(&base)]);
    assert!(out.status.success(), "init failed: {}", String::from_utf8_lossy(&out.stderr));
    add_req(&base, "Shared base requirement", "The system shall do the base thing.", "base thing happens");
    std::fs::copy(&base, &ours).unwrap();
    std::fs::copy(&base, &theirs).unwrap();
    Trio { _dir: dir, base, ours, theirs, merged }
}

fn merge(t: &Trio) -> Output {
    req(&[
        "merge", "--base", p(&t.base), "--ours", p(&t.ours), "--theirs", p(&t.theirs),
        "--output", p(&t.merged),
    ])
}

fn titles(file: &Path) -> String {
    String::from_utf8_lossy(&req(&["--file", p(file), "list"]).stdout).into_owned()
}

// REQ-0207 AC1: two branches that each add a (distinct) requirement merge
// cleanly — both additions are preserved, the colliding auto-ID is renumbered,
// and the result is a valid, re-signed file (exit 0).
#[test]
fn req_0207_clean_merge_keeps_both_additions() {
    let t = setup();
    add_req(&t.ours, "Ours adds login", "The system shall allow login.", "login works");
    add_req(&t.theirs, "Theirs adds logout", "The system shall allow logout.", "logout works");

    let out = merge(&t);
    assert!(out.status.success(), "expected clean merge, got: {}", String::from_utf8_lossy(&out.stderr));

    let listing = titles(&t.merged);
    assert!(listing.contains("Shared base requirement"), "base lost:\n{listing}");
    assert!(listing.contains("Ours adds login"), "ours addition lost:\n{listing}");
    assert!(listing.contains("Theirs adds logout"), "theirs addition lost:\n{listing}");
    // The merged file must load with a valid integrity hash (a re-listing here
    // would fail if it didn't).
    assert!(req(&["--file", p(&t.merged), "list"]).status.success(), "merged file failed to load");
}

// REQ-0207 AC1 (field level): edits to DISTINCT fields of the same artifact are
// both applied — this is a clean merge, not a conflict.
#[test]
fn req_0207_field_level_merge_of_same_artifact() {
    let t = setup();
    assert!(req(&["--file", p(&t.ours), "update", "REQ-0001", "--title", "Ours edited the title"]).status.success());
    assert!(req(&["--file", p(&t.theirs), "update", "REQ-0001", "--priority", "must"]).status.success());

    let out = merge(&t);
    assert!(out.status.success(), "distinct-field edits should merge cleanly: {}", String::from_utf8_lossy(&out.stderr));

    let show = String::from_utf8_lossy(&req(&["--file", p(&t.merged), "show", "REQ-0001"]).stdout).into_owned();
    assert!(show.contains("Ours edited the title"), "ours title edit lost:\n{show}");
    assert!(show.to_lowercase().contains("must"), "theirs priority edit lost:\n{show}");
}

// SR-0010: when both sides edit the SAME field of the same artifact, the merge
// MUST NOT pick a side. It exits non-zero and preserves BOTH versions in the
// output for human resolution.
#[test]
fn sr_0010_same_field_conflict_exits_nonzero_and_preserves_both() {
    let t = setup();
    assert!(req(&["--file", p(&t.ours), "update", "REQ-0001", "--title", "OURS distinct title"]).status.success());
    assert!(req(&["--file", p(&t.theirs), "update", "REQ-0001", "--title", "THEIRS distinct title"]).status.success());

    let out = merge(&t);
    assert!(!out.status.success(), "a same-field divergence must fail loud, not silently merge");

    let merged = std::fs::read_to_string(&t.merged).unwrap();
    assert!(merged.contains("OURS distinct title"), "our side dropped:\n{merged}");
    assert!(merged.contains("THEIRS distinct title"), "their side dropped:\n{merged}");
    assert!(merged.contains("<<<<<<<") && merged.contains(">>>>>>>"), "no conflict markers written");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("CONFLICT"), "conflict not reported on stderr: {stderr}");
}

// SR-0010: an edit on one side versus a delete on the other is unresolvable —
// it must conflict (preserving the edited version), never silently honour the
// delete and drop the edit.
#[test]
fn sr_0010_edit_vs_delete_conflicts() {
    let t = setup();
    // ours deletes REQ-0001; theirs edits it.
    let del = req(&["--file", p(&t.ours), "delete", "REQ-0001", "--hard"]);
    assert!(del.status.success(), "delete failed: {}", String::from_utf8_lossy(&del.stderr));
    assert!(req(&["--file", p(&t.theirs), "update", "REQ-0001", "--title", "THEIRS kept and edited"]).status.success());

    let out = merge(&t);
    assert!(!out.status.success(), "edit-vs-delete must conflict");
    let merged = std::fs::read_to_string(&t.merged).unwrap();
    assert!(merged.contains("THEIRS kept and edited"), "edited side dropped on edit-vs-delete:\n{merged}");
}

// SR-0010 AC3: the merge driver registered by `req hooks` must not mask its
// exit status (no `|| true`) and must be the semantic `req merge` driver.
#[test]
fn sr_0010_hooks_registers_safe_driver() {
    let dir = tempfile::Builder::new().prefix("req-hooks-").tempdir().unwrap();
    // hooks install writes into .git/hooks, so make it a real repo first.
    let gi = std::process::Command::new("git").arg("init").arg(dir.path()).output().expect("git init");
    assert!(gi.status.success(), "git init failed");

    let out = req(&["hooks", "install", "--repo", dir.path().to_str().unwrap()]);
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(combined.contains("req merge --base"), "safe driver not registered:\n{combined}");
    assert!(!combined.contains("|| true"), "driver still masks its exit status:\n{combined}");
    assert!(!combined.contains("renumber --base %O"), "still registers the misconfigured renumber driver:\n{combined}");
}
