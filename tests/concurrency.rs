// Concurrency tests for REQ-0062 (advisory file lock around mutation
// cycles). Spawns multiple child `req` processes and asserts that no
// updates are lost: both writes land with distinct IDs.
mod common;
use common::Sandbox;
use std::process::{Command, Stdio};

fn spawn_add(path: &std::path::Path, title: &str, rationale: &str) -> std::process::Child {
    Command::new(env!("CARGO_BIN_EXE_req"))
        .args([
            "--file",
            path.to_str().unwrap(),
            "add",
            "--title",
            title,
            "--statement",
            "The system shall accept this concurrent requirement attempt for the lock test.",
            "--rationale",
            rationale,
            "--kind",
            "constraint",
            "--priority",
            "could",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn child")
}

#[test]
fn req_0062_two_concurrent_adds_both_land_with_distinct_ids() {
    let s = Sandbox::new();
    s.init("conc");
    let path = s.path();
    let mut a = spawn_add(&path, "Concurrent requirement A here", "Process A");
    let mut b = spawn_add(&path, "Concurrent requirement B here", "Process B");
    let oa = a.wait_with_output().expect("A finished");
    let ob = b.wait_with_output().expect("B finished");
    assert!(
        oa.status.success(),
        "A stderr: {}",
        String::from_utf8_lossy(&oa.stderr)
    );
    assert!(
        ob.status.success(),
        "B stderr: {}",
        String::from_utf8_lossy(&ob.stderr)
    );

    let list = common::stdout(&s.run(&["list", "--json"]));
    assert!(list.contains("REQ-0001"), "REQ-0001 missing from: {}", list);
    assert!(
        list.contains("REQ-0002"),
        "REQ-0002 missing — lost update! list: {}",
        list
    );
}

#[test]
fn req_0062_five_concurrent_adds_all_land_with_unique_ids() {
    let s = Sandbox::new();
    s.init("conc");
    let path = s.path();
    let children: Vec<_> = (0..5)
        .map(|i| {
            spawn_add(
                &path,
                &format!("Concurrent requirement number {}", i),
                &format!("P{}", i),
            )
        })
        .collect();
    let mut ok = 0;
    for (i, c) in children.into_iter().enumerate() {
        let out = c.wait_with_output().expect("child wait");
        if out.status.success() {
            ok += 1
        } else {
            eprintln!(
                "child {} stderr: {}",
                i,
                String::from_utf8_lossy(&out.stderr)
            );
        }
    }
    assert_eq!(ok, 5, "all five concurrent adds should succeed");

    let list = common::stdout(&s.run(&["list", "--json"]));
    for i in 1..=5 {
        let want = format!("REQ-{:04}", i);
        assert!(list.contains(&want), "{} missing — lost update", want);
    }
}

#[test]
fn req_0062_lock_sidecar_is_cleaned_up_after_release() {
    let s = Sandbox::new();
    s.init("conc");
    let _ = s.run(&[
        "add",
        "--title",
        "Single add releases lock",
        "--statement",
        "The system shall release the lock after a normal save.",
        "--rationale",
        "Verify cleanup.",
        "--kind",
        "constraint",
        "--priority",
        "could",
    ]);
    let dir = s.dir.path();
    let lock_present = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .any(|e| e.file_name().to_string_lossy().ends_with(".lock"));
    assert!(
        !lock_present,
        "lock sidecar should be removed after the mutation completed"
    );
}
