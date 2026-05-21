// REQ-0128: req test run --map lets Node/Python/etc. attach pass/fail
// records to requirements without the `req_NNNN_*` naming convention.
mod common;
use common::{stderr, stdout, Sandbox};
use std::fs;

#[test]
fn req_0128_map_file_attaches_records_by_test_name() {
    let s = Sandbox::new();
    s.init("p");
    s.run(&[
        "add",
        "--title",
        "Mapped via external test name",
        "--statement",
        "The system shall accept evidence routed through a test-name map.",
        "--rationale",
        "REQ-0128 fixture.",
        "--kind",
        "constraint",
        "--priority",
        "could",
    ]);

    // Synthetic mocha-style log: one pass, one fail.
    let log = s.dir.path().join("mocha.log");
    fs::write(
        &log,
        "  expand vehicles endpoint pass\n  rejects negative pages FAILED\n",
    )
    .unwrap();

    let map = s.dir.path().join("map.json");
    fs::write(
        &map,
        r#"{
            "expand vehicles endpoint": ["REQ-0001"],
            "rejects negative pages": ["REQ-0001"]
        }"#,
    )
    .unwrap();

    let out = s.run(&[
        "test",
        "run",
        "--from-file",
        log.to_str().unwrap(),
        "--map",
        map.to_str().unwrap(),
        "--json",
    ]);
    assert!(out.status.success(), "stderr={}", stderr(&out));

    // After the run, REQ-0001 should carry at least one record from the map.
    let show = stdout(&s.run(&["show", "REQ-0001", "--json"]));
    assert!(
        show.contains("expand vehicles endpoint") || show.contains("rejects negative pages"),
        "test names from the map should appear in records; show: {}",
        show
    );
}

#[test]
fn req_0128_schema_test_map_published() {
    let out = common::req(&["schema", "test-map"]);
    assert!(out.status.success(), "stderr={}", stderr(&out));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("schema test-map JSON");
    assert!(
        v["$id"].as_str().unwrap().contains("test-map"),
        "schema $id should mention test-map"
    );
    // additionalProperties should constrain the value array's REQ-ID pattern.
    let pat = &v["additionalProperties"]["items"]["pattern"];
    assert_eq!(pat.as_str().unwrap(), "^REQ-\\d{4}$");
}
