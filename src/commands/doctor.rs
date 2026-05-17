// Implements REQ-0064: per-clone setup audit. Probes the hook, merge driver,
// gitattributes pin, and commit-signing configuration.
use anyhow::Result;
use serde_json::json;
use std::path::PathBuf;
use std::process::Command;

use crate::cli::DoctorArgs;

#[derive(serde::Serialize)]
struct Check {
    name: String,
    ok: bool,
    detail: String,
    /// Advisory checks are reported but do not flip the overall exit
    /// status. Use for nice-to-haves like signing that aren't
    /// load-bearing for req's correctness on a typical clone.
    #[serde(default)]
    advisory: bool,
}

pub fn run(args: DoctorArgs) -> Result<()> {
    let mut checks = Vec::new();

    // 1. pre-commit hook present + managed by req
    let pre_commit = PathBuf::from(".git/hooks/pre-commit");
    if pre_commit.exists() {
        let body = std::fs::read_to_string(&pre_commit).unwrap_or_default();
        let managed = body.contains("# managed-by: req-hooks");
        let runs_validate = body.contains("req validate");
        checks.push(Check {
            name: "pre-commit hook".into(),
            ok: managed && runs_validate,
            detail: if managed && runs_validate {
                format!("present at {}", pre_commit.display())
            } else if pre_commit.exists() {
                "present but not managed by req — run `req hooks install --force`".into()
            } else {
                "missing — run `req hooks install`".into()
            },
            advisory: false,
        });
    } else {
        checks.push(Check {
            name: "pre-commit hook".into(),
            ok: false,
            detail: "missing — run `req hooks install`".into(),
            advisory: false,
        });
    }

    // 2. .gitattributes pin
    let attrs = std::fs::read_to_string(".gitattributes").unwrap_or_default();
    let has_merge = attrs.lines().any(|l| l.trim() == "*.req merge=req-merge");
    let has_pin = attrs
        .lines()
        .any(|l| l.contains("project.req") && l.contains("-text") && l.contains("eol=lf"));
    checks.push(Check {
        name: "gitattributes merge driver".into(),
        ok: has_merge,
        detail: if has_merge {
            "registered".into()
        } else {
            "missing — run `req hooks install`".into()
        },
        advisory: false,
    });
    checks.push(Check {
        name: "gitattributes line-ending pin".into(),
        ok: has_pin,
        detail: if has_pin {
            "project.req pinned to LF -text".into()
        } else {
            "missing — run `req hooks install`".into()
        },
        advisory: false,
    });

    // 3. req-merge driver active in local git config
    let driver = git_config("merge.req-merge.driver");
    let driver_ok = driver.as_deref().map(|s| !s.is_empty()).unwrap_or(false);
    checks.push(Check {
        name: "git merge.req-merge driver activated".into(),
        ok: driver_ok,
        detail: match &driver {
            Some(s) if !s.is_empty() => format!("driver: {}", s),
            _ => "inactive — run `git config merge.req-merge.driver 'req renumber --base %O || true'`".into(),
        },
        advisory: false,
    });

    // 4. commit signing — check both the config flag AND the actual
    // signature on the latest commit. A green tick from "the flag is set"
    // would lie when keys aren't configured or signing fails silently.
    let gpg_flag = git_config("commit.gpgsign")
        .map(|s| s.to_lowercase() == "true")
        .unwrap_or(false);
    let ssh_sign = git_config("gpg.format")
        .map(|s| s == "ssh")
        .unwrap_or(false);
    let flag_on = gpg_flag || ssh_sign;
    // %G? returns the signature status of HEAD: G=good, B=bad,
    // U=good-unknown-trust, X=expired, Y=expired-key, R=revoked,
    // E=cannot-check, N=no-signature.
    let head_sig = std::process::Command::new("git")
        .args(["log", "-1", "--format=%G?"])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
            } else {
                None
            }
        })
        .unwrap_or_default();
    let signed_in_practice = matches!(head_sig.as_str(), "G" | "U");
    let signing_ok = flag_on && signed_in_practice;
    checks.push(Check {
        name: "commit signing".into(),
        ok: signing_ok,
        detail: match (flag_on, signed_in_practice, head_sig.as_str()) {
            (true, true, _) => "configured and HEAD is signed — req audit will report a signer".into(),
            (true, false, "N") | (true, false, "") =>
                "config flag is on but HEAD is unsigned — likely missing user.signingkey or a key not on the keychain".into(),
            (true, false, other) =>
                format!("config flag is on but HEAD signature is '{}' (not good) — fix key or expiration", other),
            (false, _, _) =>
                "disabled — `git config commit.gpgsign true` (or set gpg.format=ssh) and configure a key".into(),
        },
        // Signing is nice-to-have on a typical project, not a gate.
        // Many repos run `req` happily without signed commits; the
        // overall doctor exit code should not flip red just for that.
        advisory: true,
    });

    let failed_gating = checks.iter().filter(|c| !c.ok && !c.advisory).count();
    let advisory_failures = checks.iter().filter(|c| !c.ok && c.advisory).count();

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "ok": failed_gating == 0,
                "failed": failed_gating,
                "advisory_failed": advisory_failures,
                "checks": checks,
            }))?
        );
    } else {
        println!(
            "req doctor — {} check(s), {} failing, {} advisory",
            checks.len(),
            failed_gating,
            advisory_failures
        );
        for c in &checks {
            let mark = match (c.ok, c.advisory) {
                (true, _) => "OK  ",
                (false, true) => "WARN",
                (false, false) => "FAIL",
            };
            println!("  [{}] {}  {}", mark, c.name, c.detail);
        }
    }

    if failed_gating > 0 {
        std::process::exit(1);
    }
    Ok(())
}

fn git_config(key: &str) -> Option<String> {
    let out = Command::new("git")
        .args(["config", "--get", key])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}
