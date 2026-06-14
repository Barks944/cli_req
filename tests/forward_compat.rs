// REQ-0140 / REQ-0141: forward-compatibility guarantees for the on-disk
// format. An older `req` binary must not silently destroy data written by a
// newer one: unknown fields round-trip unchanged (REQ-0140), and a file
// stamped with a newer schema revision is refused on save (REQ-0141).
mod common;
use common::{stderr, stdout, Sandbox};

/// Parse the project file, mutate it via `f`, and write it back verbatim.
/// Only touches reserved meta keys or injects fields the binary doesn't
/// model — never the integrity payload — so the file still loads.
fn edit_raw(s: &Sandbox, f: impl FnOnce(&mut serde_json::Value)) {
    let path = s.path();
    let body = std::fs::read_to_string(&path).expect("read project.req");
    let mut v: serde_json::Value = serde_json::from_str(&body).expect("parse project.req");
    f(&mut v);
    std::fs::write(&path, serde_json::to_string_pretty(&v).unwrap()).expect("write project.req");
}

fn add_req(s: &Sandbox, title: &str) {
    let out = s.run(&[
        "add",
        "--title",
        title,
        "--statement",
        "The system shall carry this requirement for the forward-compat fixture.",
        "--rationale",
        "Forward-compatibility test fixture.",
        "--kind",
        "constraint",
        "--priority",
        "could",
    ]);
    assert!(out.status.success(), "add failed: {}", stderr(&out));
}

// REQ-0140: a field the binary does not model, sitting on a requirement,
// survives a load→save cycle instead of being dropped. We simulate a
// "written by a newer req" file by injecting the unknown field and
// re-signing with `req repair`, then prove a normal mutation preserves it.
#[test]
fn req_0140_unknown_requirement_field_round_trips() {
    let s = Sandbox::new();
    s.init("p");
    add_req(&s, "Has an unknown field from a future version");

    // Inject a field the current model knows nothing about.
    edit_raw(&s, |v| {
        v["requirements"]["REQ-0001"]["future_field"] =
            serde_json::json!({"shape": "unknown", "n": 42});
    });
    // Re-sign after the direct edit (load→save through the extra-aware model).
    let repair = s.run(&["repair", "--confirm-direct-edit"]);
    assert!(
        repair.status.success(),
        "repair failed: {}",
        stderr(&repair)
    );

    // A subsequent ordinary mutation must not drop the unknown field.
    add_req(&s, "A second requirement triggering another save");

    let body = std::fs::read_to_string(s.path()).unwrap();
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        v["requirements"]["REQ-0001"]["future_field"]["n"], 42,
        "unknown field must round-trip; got:\n{}",
        body
    );
}

// REQ-0141: a project file stamped with a newer schema revision than the
// binary understands is refused on the next save, and nothing is written.
#[test]
fn req_0141_save_refused_when_file_schema_rev_is_newer() {
    let s = Sandbox::new();
    s.init("p");
    add_req(&s, "Baseline requirement before the rev bump");

    // Forge a newer schema revision. `_schema_rev` is a reserved key
    // excluded from the integrity hash, so the file still loads cleanly.
    edit_raw(&s, |v| {
        v["_schema_rev"] = serde_json::json!(9999);
    });

    let before = std::fs::read_to_string(s.path()).unwrap();
    let out = s.run(&[
        "add",
        "--title",
        "This add must be refused",
        "--statement",
        "The system shall never persist this requirement.",
        "--rationale",
        "Guard fixture.",
        "--kind",
        "constraint",
        "--priority",
        "could",
    ]);
    assert!(
        !out.status.success(),
        "add over a newer-schema file must fail; stdout={}",
        stdout(&out)
    );
    let msg = stderr(&out).to_lowercase();
    assert!(
        msg.contains("newer") && (msg.contains("schema rev") || msg.contains("upgrade")),
        "error should explain the schema-rev mismatch, got: {}",
        stderr(&out)
    );
    let after = std::fs::read_to_string(s.path()).unwrap();
    assert_eq!(before, after, "the file must be left untouched on refusal");
}

// REQ-0141: a file written before the guard existed (no `_schema_rev`) is
// treated as revision 0 and saves without complaint.
#[test]
fn req_0141_missing_schema_rev_is_treated_as_zero() {
    let s = Sandbox::new();
    s.init("p");
    add_req(&s, "First requirement");

    // Strip the stamp to mimic a pre-guard file.
    edit_raw(&s, |v| {
        if let Some(obj) = v.as_object_mut() {
            obj.remove("_schema_rev");
        }
    });
    let repair = s.run(&["repair", "--confirm-direct-edit"]);
    assert!(
        repair.status.success(),
        "repair failed: {}",
        stderr(&repair)
    );

    // A normal mutation succeeds and re-stamps the current revision.
    add_req(&s, "Second requirement after the strip");
    let body = std::fs::read_to_string(s.path()).unwrap();
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(
        v.get("_schema_rev")
            .and_then(serde_json::Value::as_u64)
            .is_some(),
        "save should re-stamp _schema_rev; got:\n{}",
        body
    );
}
