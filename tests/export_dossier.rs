// REQ-0209/0210/0211: the markdown/HTML export mirrors the verification
// dossier, the verification-status roll-up, and the functional-safety layer
// (or its explicit empty state) — GitHub issue #8.
mod common;
use common::{stdout, Sandbox};

fn add_implemented(s: &Sandbox) {
    s.run(&[
        "add",
        "--title",
        "Export the dossier",
        "--statement",
        "The system shall export the verification dossier.",
        "--rationale",
        "review evidence must survive export",
        "--kind",
        "functional",
        "--accept",
        "dossier in export",
    ]);
    for st in ["proposed", "approved", "implemented"] {
        s.run(&["update", "REQ-0001", "--status", st, "--reason", "step"]);
    }
}

fn conclude_dossier(s: &Sandbox) {
    s.run(&[
        "verification",
        "plan",
        "REQ-0001",
        "--plan",
        "REVIEW THE EXPORTER",
    ]);
    s.run(&[
        "verification",
        "analysis",
        "REQ-0001",
        "--findings",
        "exporter matches the obligation",
        "--result",
        "pass",
        "--ref",
        "src/commands/export.rs",
    ]);
    s.run(&[
        "verification",
        "test",
        "REQ-0001",
        "--findings",
        "export suite green",
        "--result",
        "pass",
    ]);
    s.run(&[
        "verification",
        "conclude",
        "REQ-0001",
        "--statement",
        "meets the obligation in both formats",
        "--promote",
    ]);
}

// REQ-0209: the exported markdown carries the full dossier per requirement.
#[test]
fn req_0209_markdown_export_carries_dossier() {
    let s = Sandbox::new();
    s.init("p");
    add_implemented(&s);
    conclude_dossier(&s);
    let out = s.run(&["export", "-f", "markdown"]);
    assert!(out.status.success());
    let md = stdout(&out);
    for needle in [
        "REVIEW THE EXPORTER",                  // plan
        "exporter matches the obligation",      // analysis findings
        "src/commands/export.rs",               // analysis reference
        "export suite green",                   // testing findings
        "meets the obligation in both formats", // statement
        "verdict **PASS**",                     // derived verdict
        "provenance `genuine`",                 // provenance
        "Test records",                         // record list
    ] {
        assert!(
            md.contains(needle),
            "markdown must contain {needle:?}:\n{md}"
        );
    }
    // The HTML export mirrors the markdown body.
    let out = s.run(&["export", "-f", "html"]);
    let html = stdout(&out);
    assert!(
        html.contains("REVIEW THE EXPORTER") && html.contains("Test records"),
        "html must mirror the dossier:\n{html}"
    );
}

// REQ-0210: the export includes the verification-status roll-up.
#[test]
fn req_0210_export_includes_verification_rollup() {
    let s = Sandbox::new();
    s.init("p");
    add_implemented(&s);
    conclude_dossier(&s);
    // One extra unverified requirement.
    s.run(&[
        "add",
        "--title",
        "Unverified leftover",
        "--statement",
        "The system shall appear in the unverified surface.",
        "--rationale",
        "roll-up fixture requirement",
        "--kind",
        "functional",
        "--accept",
        "listed as unverified",
    ]);
    let md = stdout(&s.run(&["export", "-f", "markdown"]));
    for needle in [
        "# Verification status",
        "| genuine | 1 |",
        "Unverified (by dossier stage)",
        "REQ-0002 — no-plan",
    ] {
        assert!(
            md.contains(needle),
            "roll-up must contain {needle:?}:\n{md}"
        );
    }
}

// REQ-0211: safety disabled + no artifacts → explicit note, not silence.
#[test]
fn req_0211_export_safety_disabled_note() {
    let s = Sandbox::new();
    s.init("p");
    add_implemented(&s);
    let md = stdout(&s.run(&["export", "-f", "markdown"]));
    assert!(
        md.contains("# Functional safety") && md.contains("Disabled"),
        "export must state that functional safety is disabled:\n{md}"
    );
}

// REQ-0211: with safety artifacts, the export carries the SF/SR dossiers,
// the walkthrough state, and the SIL calibration.
#[test]
fn req_0211_export_full_safety_layer() {
    let s = Sandbox::new();
    s.init("p");
    s.enable_safety();
    s.run(&[
        "hazard", "add", "-t", "H", "--harm", "hurt", "-C", "C_C", "-F", "F_B", "-P", "P_B", "-W",
        "W3",
    ]);
    s.run(&["sf", "add", "-t", "Stop fn", "--mitigates", "HAZ-0001"]);
    s.run(&[
        "sreq",
        "add",
        "-t",
        "Stop the blade",
        "-s",
        "The system shall stop the blade on demand.",
        "-r",
        "operator safety",
        "-a",
        "stops",
        "--realizes",
        "SF-0001",
    ]);
    let md = stdout(&s.run(&["export", "-f", "markdown"]));
    for needle in [
        "## Safety verification dossiers",
        "### SF-0001",
        "### SR-0001",
        "## Walkthrough acknowledgements",
        "never acknowledged",
        "## SIL calibration",
        "Annex D",
    ] {
        assert!(
            md.contains(needle),
            "safety export must contain {needle:?}:\n{md}"
        );
    }
}
