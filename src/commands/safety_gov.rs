// REQ-0138: functional-safety governance — the human-only controls that
// (1) gate the safety features behind an explicit, committed liability
// acknowledgement and (2) own the per-project risk-graph calibration.
//
// The acknowledgement lives in a SIBLING FILE next to project.req
// (`req-safety-acceptance.json`), not inside the integrity-hashed spec:
// it is a git-tracked artifact, so enabling the safety features shows up
// in a PR diff and a reviewer can see who signed on and when. The file's
// presence (with a current disclaimer version) is what activates the
// feature — delete it and safety mutations stop.
//
// This command is deliberately NOT on the agent/MCP surface: accepting
// liability and recalibrating risk are human governance decisions.
use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use std::path::{Path, PathBuf};

use crate::cli::{SafetyAcceptArgs, SafetyCalibrateArgs, SafetyCmd, SafetyStatusArgs};
use crate::model::{
    calibration_leaf, Avoidance, CalibrationRow, Consequence, DisclaimerAcceptance, Frequency,
    ProjectConfig, SafetyConfig, Sil, SAFETY_DISCLAIMER_VERSION,
};
use crate::storage::{self, resolve_path};

/// The acknowledgement text shown before sign-on and embedded in the
/// acceptance file. Keep in sync with `req help safety`; bump
/// SAFETY_DISCLAIMER_VERSION when its substance changes.
pub const DISCLAIMER: &str = "\
FUNCTIONAL-SAFETY DISCLAIMER — read before enabling these features.

  • req is NOT a qualified safety tool. Under IEC 61508-3 §7.4.4 (and
    ISO 26262-8), a tool whose output you rely on without independent
    verification needs a tool-confidence/qualification argument. req
    provides none. Qualifying it, or independently verifying every SIL
    it computes, is YOUR responsibility.
  • The SIL req shows is a CANDIDATE derived from the qualitative risk
    parameters you enter, using the IEC 61508-5 Annex D worked-example
    calibration (or your own, if you set one). It is not objective and
    does not remove the need for competent review.
  • req tracks the REQUIRED integrity target and traceability — NOT
    achieved integrity (no PFD/PFH, diagnostic coverage, SFF, SIL
    decomposition). A \"complete\" trace means linked-and-verified, not
    safe.
  • This software is provided \"AS IS\", without warranty of any kind.
    The authors accept NO liability. Nothing it produces is safety
    assurance or fitness for any safety-related purpose.

By accepting you confirm you understand the above and take
responsibility for the safety determination.";

/// Path to the acceptance file that sits beside the project.
pub fn acceptance_path(project_path: &Path) -> PathBuf {
    let dir = if project_path.is_dir() {
        project_path.to_path_buf()
    } else {
        project_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."))
    };
    dir.join("req-safety-acceptance.json")
}

/// Read the acceptance record beside the project, if present and parseable.
pub fn read_acceptance(project_path: &Path) -> Option<DisclaimerAcceptance> {
    let p = acceptance_path(project_path);
    let body = std::fs::read_to_string(p).ok()?;
    serde_json::from_str(&body).ok()
}

/// Whether the safety features are activated for this project: a valid
/// acceptance file exists for the current disclaimer version.
pub fn is_enabled(project_path: &Path) -> bool {
    read_acceptance(project_path)
        .map(|a| a.disclaimer_version == SAFETY_DISCLAIMER_VERSION)
        .unwrap_or(false)
}

/// The gate every safety MUTATION calls. Reading (list/show/trace) is
/// always allowed so an existing safety case stays inspectable; only
/// creating or changing safety artifacts requires the acknowledgement.
pub fn ensure_enabled(file: &Option<PathBuf>) -> Result<()> {
    ensure_enabled_path(&resolve_path(file))
}

/// Path-keyed form of the gate, for callers (MCP) that already hold the
/// resolved project path.
pub fn ensure_enabled_path(path: &Path) -> Result<()> {
    if is_enabled(path) {
        return Ok(());
    }
    let existing = read_acceptance(path);
    let why = match existing {
        Some(_) => "the acceptance on file is for an older disclaimer version",
        None => "the functional-safety features are not enabled for this project",
    };
    Err(anyhow!(
        "{why}. A human must accept the safety disclaimer first:\n\n    \
         req safety accept --name \"Your Name <you@example.com>\"\n\n\
         This writes {} (commit it) which activates hazards / safety \
         functions / safety requirements. See `req help safety`.",
        acceptance_path(path).display()
    ))
}

pub fn run(cmd: SafetyCmd, file: &Option<PathBuf>) -> Result<()> {
    match cmd {
        SafetyCmd::Accept(a) => accept(a, file),
        SafetyCmd::Status(a) => status(a, file),
        SafetyCmd::Calibrate(a) => calibrate(a, file),
    }
}

fn accept(args: SafetyAcceptArgs, file: &Option<PathBuf>) -> Result<()> {
    let path = resolve_path(file);
    // The project must exist (so the acceptance sits beside a real spec).
    storage::load(&path).context("open project before accepting (run `req init` first?)")?;

    // REQ-0138: acceptance must be a deliberate human act, as far as a
    // CLI can tell. We CANNOT cryptographically prove humanness — an
    // agent shares the shell and filesystem — so this is "raise the bar
    // + make it accountable", not "prevent". The bar:
    //   (1) `req safety` is absent from the MCP surface (an agent's
    //       normal channel can't reach it);
    //   (2) refuse a self-identified agent (REQ_ACTOR_KIND=agent);
    //   (3) require an interactive terminal to confirm — there is no
    //       `--yes` escape, because that was just a backdoor an agent
    //       could take.
    // The real control is that the result is a committed, attributed
    // file: a forged acceptance is visible in the diff under a name.
    if matches!(super::current_actor_kind(), crate::model::ActorKind::Agent) {
        return Err(anyhow!(
            "accepting the safety disclaimer must be done by a human, but \
             REQ_ACTOR_KIND=agent. A person must run `req safety accept`."
        ));
    }
    let name = match args.name {
        Some(n) if !n.trim().is_empty() => n,
        _ => return Err(anyhow!("--name is required (record who is accepting)")),
    };
    if !atty_stdin() {
        return Err(anyhow!(
            "`req safety accept` needs an interactive terminal — run it at a \
             real prompt. There is deliberately no non-interactive flag. For \
             unattended setup, a human can instead create {} by hand (it is a \
             small JSON file) and commit it; see `req help safety`.",
            acceptance_path(&path).display()
        ));
    }

    println!("{}\n", DISCLAIMER);
    use dialoguer::Confirm;
    let ok = Confirm::new()
        .with_prompt("Do you accept, on behalf of your project?")
        .default(false)
        .interact()?;
    if !ok {
        return Err(anyhow!("not accepted — safety features remain disabled"));
    }

    let record = DisclaimerAcceptance {
        notice: "By committing this file the named person accepts the req \
                 functional-safety disclaimer: req is an unqualified aid \
                 (IEC 61508-3 §7.4.4), the SIL is a candidate, the software \
                 is provided AS IS with no warranty or liability, and the \
                 safety determination remains the user's responsibility."
            .into(),
        accepted_by: name,
        at: Utc::now(),
        tool_version: env!("CARGO_PKG_VERSION").to_string(),
        disclaimer_version: SAFETY_DISCLAIMER_VERSION.to_string(),
    };
    let out = acceptance_path(&path);
    let body = serde_json::to_string_pretty(&record)?;
    std::fs::write(&out, body).with_context(|| format!("write {}", out.display()))?;
    println!(
        "\nSafety features ENABLED. Wrote {}.\nCommit this file — its presence \
         is what activates hazards / safety functions / safety requirements.",
        out.display()
    );
    Ok(())
}

fn status(args: SafetyStatusArgs, file: &Option<PathBuf>) -> Result<()> {
    let path = resolve_path(file);
    let project = storage::load(&path).ok();
    let acceptance = read_acceptance(&path);
    let enabled = is_enabled(&path);
    let cal_label = project
        .as_ref()
        .and_then(|p| p.config.as_ref())
        .and_then(|c| c.safety.as_ref())
        .and_then(|s| s.calibration_label.clone());
    let cal_overrides = project
        .as_ref()
        .and_then(|p| p.calibration())
        .map(|t| t.len())
        .unwrap_or(0);

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "enabled": enabled,
                "acceptance": acceptance,
                "disclaimer_version": SAFETY_DISCLAIMER_VERSION,
                "calibration_label": cal_label,
                "calibration_overrides": cal_overrides,
            }))?
        );
        return Ok(());
    }

    println!("Functional safety: {}", if enabled { "ENABLED" } else { "disabled" });
    match &acceptance {
        Some(a) => println!(
            "  accepted by {} on {} (req {}, disclaimer v{})",
            a.accepted_by,
            a.at.format("%Y-%m-%d"),
            a.tool_version,
            a.disclaimer_version
        ),
        None => println!(
            "  no acceptance file — run `req safety accept --name \"...\"` to enable"
        ),
    }
    println!(
        "  calibration: {} ({} leaf override(s))",
        cal_label.as_deref().unwrap_or("IEC 61508-5 Annex D (default)"),
        cal_overrides
    );
    Ok(())
}

fn calibrate(args: SafetyCalibrateArgs, file: &Option<PathBuf>) -> Result<()> {
    // Show-only path needs no lock.
    if args.show && args.label.is_none() && args.set.is_empty() && !args.reset {
        let (_p, project) = storage::load_resolved(file)?;
        print_calibration(&project);
        return Ok(());
    }

    let (path, mut project, _lock) = storage::load_for_mutation(file)?;
    let cfg = project
        .config
        .get_or_insert_with(ProjectConfig::default)
        .safety
        .get_or_insert_with(SafetyConfig::default);

    if args.reset {
        cfg.calibration = None;
        cfg.calibration_label = None;
    }
    if let Some(label) = args.label {
        cfg.calibration_label = Some(label);
    }
    for spec in &args.set {
        let (leaf, row) = parse_set(spec)?;
        cfg.calibration
            .get_or_insert_with(Default::default)
            .insert(leaf, row);
    }
    // Drop an emptied override map so the file stays clean.
    if cfg
        .calibration
        .as_ref()
        .map(|m| m.is_empty())
        .unwrap_or(false)
    {
        cfg.calibration = None;
    }

    storage::save(&path, &project)?;
    print_calibration(&project);
    Ok(())
}

fn print_calibration(project: &crate::model::Project) {
    let label = project
        .config
        .as_ref()
        .and_then(|c| c.safety.as_ref())
        .and_then(|s| s.calibration_label.as_deref())
        .unwrap_or("IEC 61508-5 Annex D (default)");
    println!("Risk-graph calibration: {}", label);
    match project.calibration() {
        None => println!("  (no leaf overrides — every leaf uses the Annex D default)"),
        Some(table) => {
            println!("  {} leaf override(s) [W3 / W2 / W1]:", table.len());
            for (leaf, row) in table {
                println!(
                    "    {:<14} {} / {} / {}",
                    leaf,
                    row.w3.as_str(),
                    row.w2.as_str(),
                    row.w1.as_str()
                );
            }
        }
    }
}

/// Parse `--set "C_D/F_B/P_B=W3:4,W2:3,W1:2"`. All three W values are
/// required; their SIL tokens accept `1..4`, `SIL1..SIL4`, `a`, `b`, or
/// `none`. The leaf must be a valid (C,F,P) combination.
fn parse_set(spec: &str) -> Result<(String, CalibrationRow)> {
    let (leaf_raw, rhs) = spec
        .split_once('=')
        .ok_or_else(|| anyhow!("--set expects LEAF=W3:x,W2:y,W1:z (missing '=' in '{}')", spec))?;
    let leaf = validate_leaf(leaf_raw.trim())?;
    let (mut w1, mut w2, mut w3) = (None, None, None);
    for tok in rhs.split(',') {
        let (w, sv) = tok
            .split_once(':')
            .ok_or_else(|| anyhow!("expected Wn:SIL in '{}'", tok.trim()))?;
        let sil = Sil::parse(sv)
            .ok_or_else(|| anyhow!("'{}' is not a SIL (use 1..4, a, b, none)", sv.trim()))?;
        match w.trim().to_uppercase().as_str() {
            "W1" => w1 = Some(sil),
            "W2" => w2 = Some(sil),
            "W3" => w3 = Some(sil),
            other => return Err(anyhow!("unknown probability '{}' (want W1/W2/W3)", other)),
        }
    }
    match (w1, w2, w3) {
        (Some(w1), Some(w2), Some(w3)) => Ok((leaf, CalibrationRow { w1, w2, w3 })),
        _ => Err(anyhow!(
            "leaf {} needs all of W1, W2, W3 (e.g. W3:4,W2:3,W1:2)",
            leaf
        )),
    }
}

/// Confirm a leaf string names a real (C,F,P) combination and return its
/// canonical form.
fn validate_leaf(raw: &str) -> Result<String> {
    let parts: Vec<&str> = raw.split('/').collect();
    if parts.len() != 3 {
        return Err(anyhow!("leaf must be C_x/F_x/P_x, got '{}'", raw));
    }
    let c = parse_c(parts[0])?;
    let f = parse_f(parts[1])?;
    let p = parse_p(parts[2])?;
    Ok(calibration_leaf(c, f, p))
}

fn parse_c(s: &str) -> Result<Consequence> {
    Ok(match s.trim().to_uppercase().as_str() {
        "C_A" => Consequence::Ca,
        "C_B" => Consequence::Cb,
        "C_C" => Consequence::Cc,
        "C_D" => Consequence::Cd,
        o => return Err(anyhow!("bad consequence '{}' (C_A..C_D)", o)),
    })
}
fn parse_f(s: &str) -> Result<Frequency> {
    Ok(match s.trim().to_uppercase().as_str() {
        "F_A" => Frequency::Fa,
        "F_B" => Frequency::Fb,
        o => return Err(anyhow!("bad frequency '{}' (F_A/F_B)", o)),
    })
}
fn parse_p(s: &str) -> Result<Avoidance> {
    Ok(match s.trim().to_uppercase().as_str() {
        "P_A" => Avoidance::Pa,
        "P_B" => Avoidance::Pb,
        o => return Err(anyhow!("bad avoidance '{}' (P_A/P_B)", o)),
    })
}

fn atty_stdin() -> bool {
    use std::io::IsTerminal;
    std::io::stdin().is_terminal()
}
