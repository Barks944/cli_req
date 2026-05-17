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
        });
    } else {
        checks.push(Check {
            name: "pre-commit hook".into(),
            ok: false,
            detail: "missing — run `req hooks install`".into(),
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
    });
    checks.push(Check {
        name: "gitattributes line-ending pin".into(),
        ok: has_pin,
        detail: if has_pin {
            "project.req pinned to LF -text".into()
        } else {
            "missing — run `req hooks install`".into()
        },
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
    });

    // 4. commit signing
    let gpg = git_config("commit.gpgsign")
        .map(|s| s.to_lowercase() == "true")
        .unwrap_or(false);
    let ssh_sign = git_config("gpg.format")
        .map(|s| s == "ssh")
        .unwrap_or(false);
    let signing_ok = gpg || ssh_sign;
    checks.push(Check {
        name: "commit signing".into(),
        ok: signing_ok,
        detail: if signing_ok {
            "enabled — req audit will report a signer per commit".into()
        } else {
            "disabled — `git config commit.gpgsign true` (or ssh signing) to populate `req audit`"
                .into()
        },
    });

    let failed = checks.iter().filter(|c| !c.ok).count();

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "ok": failed == 0,
                "failed": failed,
                "checks": checks,
            }))?
        );
    } else {
        println!("req doctor — {} check(s), {} failing", checks.len(), failed);
        for c in &checks {
            let mark = if c.ok { "OK " } else { "FAIL" };
            println!("  [{}] {}  {}", mark, c.name, c.detail);
        }
    }

    if failed > 0 {
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
