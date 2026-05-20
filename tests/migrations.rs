// REQ-0116: backwards-compatibility regression tests for project.req
// formats. Each historical _format gets a fixture file checked into
// tests/fixtures/. The tests confirm the migration path forward — the
// file is the source of truth that adopters commit to git, so it has
// to travel forward through tool upgrades without re-authoring.
mod common;
use common::{stderr, stdout, Sandbox};
use std::fs;

const V1_FIXTURE: &str = "tests/fixtures/v1_project.req";

#[test]
fn req_0116_v1_fixture_errors_with_migrate_hint() {
    // The binary speaks req-v2; opening a v1 file directly must error
    // with a clear pointer to `req migrate`, never silently mis-read.
    let out = common::req(&["--file", V1_FIXTURE, "validate"]);
    assert!(
        !out.status.success(),
        "v1 fixture should be rejected by a v2 binary (got success)"
    );
    let err = stderr(&out);
    assert!(
        err.contains("req migrate"),
        "error must hint at `req migrate`, got: {}",
        err
    );
}

#[test]
fn req_0116_v1_fixture_migrates_to_v2_with_ids_preserved() {
    // Copy the v1 fixture into a tempdir, migrate it, then confirm IDs
    // and titles round-tripped intact and the new format is v2.
    let s = Sandbox::new();
    let target = s.dir.path().join("project.req");
    fs::copy(V1_FIXTURE, &target).expect("copy v1 fixture");
    let target_s = target.to_str().unwrap().to_string();

    let out = common::req(&["--file", &target_s, "migrate"]);
    assert!(
        out.status.success(),
        "migrate v1 → v2 should succeed; stderr={}",
        stderr(&out)
    );
    assert!(
        stdout(&out).contains("req-v1 → req-v2"),
        "expected the v1 → v2 banner, got: {}",
        stdout(&out)
    );

    // Validate post-migration.
    let val = common::req(&["--file", &target_s, "validate"]);
    assert!(
        val.status.success(),
        "post-migrate validate failed: {}",
        stderr(&val)
    );

    // List should still show both anchor requirements with original titles.
    let list = common::req(&["--file", &target_s, "list", "--json"]);
    let body = stdout(&list);
    assert!(body.contains("REQ-0001"), "REQ-0001 missing: {}", body);
    assert!(body.contains("REQ-0002"), "REQ-0002 missing: {}", body);
    assert!(
        body.contains("Anchor requirement one"),
        "anchor title not preserved: {}",
        body
    );

    // The on-disk file should now be tagged v2.
    let migrated = fs::read_to_string(&target).unwrap();
    assert!(
        migrated.contains("\"_format\": \"req-v2\""),
        "_format should be req-v2 after migrate"
    );

    // A sibling backup of the v1 file should exist.
    let backup = target.with_extension("req.bak-req-v1");
    assert!(backup.exists(), "expected backup at {}", backup.display());
}

#[test]
fn req_0116_migrate_on_current_format_is_noop() {
    let s = Sandbox::new();
    s.init("p");
    let before = fs::read(s.path()).unwrap();
    let out = s.run(&["migrate"]);
    assert!(out.status.success(), "stderr={}", stderr(&out));
    assert!(
        stdout(&out).contains("no migration needed"),
        "expected no-op message, got: {}",
        stdout(&out)
    );
    let after = fs::read(s.path()).unwrap();
    assert_eq!(
        before, after,
        "migrate at current format must not modify the file"
    );
}

#[test]
fn req_0116_migrate_rejects_unknown_newer_format() {
    let s = Sandbox::new();
    s.init("p");
    // Synthesise a "future" format by editing the _format field. The
    // file's integrity hash will no longer match, but migrate must
    // refuse on the format check before looking at the hash so users
    // get the right hint ("upgrade your binary", not "run repair").
    let text = fs::read_to_string(s.path()).unwrap();
    let bumped = text.replace("\"_format\": \"req-v2\"", "\"_format\": \"req-v99\"");
    fs::write(s.path(), bumped).unwrap();
    let out = s.run(&["migrate"]);
    assert!(!out.status.success(), "newer format must error");
    let err = stderr(&out);
    assert!(
        err.contains("newer than this binary") || err.contains("Upgrade the binary"),
        "error should point to binary upgrade, got: {}",
        err
    );
}
