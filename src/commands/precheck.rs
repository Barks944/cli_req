// REQ-0114: req precheck — run the same gate suite CI runs, in CI's
// order, locally. The point isn't novel functionality; it's making the
// gate a single invocation so contributors can wire it into a save
// action or pre-push hook and catch environment-skew issues (rustfmt
// drift, global git config flipping fixture behaviour) in the same
// loop where the code lives. Three CI failures in a row during the
// 0.3.2 wrap-up motivated this — every one of them would have been a
// local failure first if `req precheck` had existed.
use anyhow::Result;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use crate::cli::PrecheckArgs;

struct Step {
    /// Short identifier the user passes to --skip.
    name: &'static str,
    /// One-line description shown in the progress prefix.
    label: &'static str,
    /// Program + args. `req` self-invocations use the current binary so
    /// precheck always exercises the in-tree version, not whatever
    /// `req` happens to be on PATH.
    program: ProgramSpec,
}

enum ProgramSpec {
    Cargo(&'static [&'static str]),
    SelfReq(&'static [&'static str]),
}

pub fn run(args: PrecheckArgs, project_file: &Option<PathBuf>) -> Result<()> {
    let steps: [Step; 6] = [
        Step {
            name: "fmt",
            label: "cargo fmt --check",
            program: ProgramSpec::Cargo(&["fmt", "--all", "--", "--check"]),
        },
        Step {
            name: "clippy",
            // Mirror CI exactly: --release --locked with warnings denied.
            // Running clippy on the release profile (rather than a separate
            // debug pass) also means precheck reuses the release artifacts
            // the test step builds, and `-- -D warnings` denies the same
            // rustc + clippy lints CI's warnings-as-errors build enforces.
            label: "cargo clippy",
            program: ProgramSpec::Cargo(&[
                "clippy",
                "--release",
                "--locked",
                "--",
                "-D",
                "warnings",
            ]),
        },
        Step {
            name: "test",
            // REQ-0114: mirror CI — the test job runs the RELEASE profile
            // (`--locked`) and serialises tests within each binary
            // (`--test-threads=1`)
            // to avoid the fixture-config flakiness CI guards against. The
            // debug profile would diverge from CI and can mask or invent
            // failures (e.g. a stale debug binary lacking a new CLI flag).
            label: "cargo test (release, serial)",
            program: ProgramSpec::Cargo(&[
                "test",
                "--release",
                "--locked",
                "--",
                "--test-threads=1",
            ]),
        },
        Step {
            name: "validate",
            label: "req validate",
            program: ProgramSpec::SelfReq(&["validate"]),
        },
        Step {
            name: "coverage",
            label: "req coverage --strict",
            program: ProgramSpec::SelfReq(&["coverage", "--strict"]),
        },
        Step {
            name: "review",
            label: "req review --gate",
            program: ProgramSpec::SelfReq(&["review", "--gate"]),
        },
    ];

    let skip: Vec<String> = args.skip.iter().map(|s| s.to_lowercase()).collect();
    for s in &skip {
        if !steps.iter().any(|step| step.name == s) {
            anyhow::bail!(
                "unknown --skip step `{}`; known: fmt, clippy, test, validate, coverage, review",
                s
            );
        }
    }

    let self_exe = std::env::current_exe()
        .map_err(|e| anyhow::anyhow!("could not locate the running req binary: {}", e))?;

    let mut first_failure: Option<&'static str> = None;
    let total = steps
        .iter()
        .filter(|s| !skip.contains(&s.name.into()))
        .count();
    let mut idx = 0;
    for step in &steps {
        if skip.contains(&step.name.to_string()) {
            println!("[skip] {} (--skip {})", step.label, step.name);
            continue;
        }
        idx += 1;
        println!("\n=== [{}/{}] {} ===", idx, total, step.label);

        let mut cmd: Command = match step.program {
            ProgramSpec::Cargo(args) => {
                let mut c = Command::new("cargo");
                c.args(args);
                c
            }
            ProgramSpec::SelfReq(extra) => {
                let mut c = Command::new(&self_exe);
                // Pipe the same --file the user invoked precheck with
                // so coverage/review/validate see the same project.
                if let Some(p) = project_file.as_ref() {
                    c.arg("--file").arg(p);
                }
                c.args(extra);
                c
            }
        };
        cmd.stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
        let status = cmd
            .status()
            .map_err(|e| anyhow::anyhow!("failed to spawn `{}`: {}", step.label, e))?;
        if !status.success() {
            eprintln!(
                "\n[fail] step `{}` exited with {}",
                step.name,
                status
                    .code()
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "signal".into())
            );
            if first_failure.is_none() {
                first_failure = Some(step.name);
            }
            if !args.keep_going {
                eprintln!(
                    "\nStopping at first failure (`{}`). Fix it and re-run, or pass --keep-going to see all failures.",
                    step.name
                );
                std::process::exit(1);
            }
        }
    }

    if let Some(name) = first_failure {
        eprintln!("\nprecheck FAILED (first failure: `{}`)", name);
        std::process::exit(1);
    }
    println!("\nprecheck OK — all steps passed.");
    Ok(())
}
