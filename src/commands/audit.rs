// Implements REQ-0027 (per-commit signature status via git log --follow)
// and discharges REQ-0028 (lean on git, do not build our own PKI).
use anyhow::{anyhow, Context, Result};
use std::path::PathBuf;
use std::process::Command;

use crate::cli::AuditArgs;
use crate::storage::resolve_path;

#[derive(serde::Serialize)]
struct Entry {
    commit: String,
    date: String,
    author: String,
    signature_status: String,
    signer: String,
    subject: String,
}

pub fn run(args: AuditArgs, file: &Option<PathBuf>) -> Result<()> {
    let path = resolve_path(file);
    if !path.exists() {
        return Err(anyhow!(
            "{} does not exist — run `req init` first",
            path.display()
        ));
    }
    // %G? -> signature status (G good, B bad, U good-unknown, X expired, N no signature, ...)
    // %GS -> signer name
    // We use a sentinel separator unlikely to appear in commit subjects.
    let fmt = "%H|||%aI|||%aN|||%G?|||%GS|||%s";
    let output = Command::new("git")
        .args([
            "log",
            "--follow",
            &format!("-n{}", args.limit),
            &format!("--format={}", fmt),
            "--",
        ])
        .arg(&path)
        .output()
        .context("run git log")?;
    if !output.status.success() {
        return Err(anyhow!(
            "git log failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut entries: Vec<Entry> = Vec::new();
    for line in text.lines() {
        let parts: Vec<&str> = line.splitn(6, "|||").collect();
        if parts.len() != 6 {
            continue;
        }
        entries.push(Entry {
            commit: parts[0].into(),
            date: parts[1].into(),
            author: parts[2].into(),
            signature_status: explain_sig(parts[3]),
            signer: parts[4].into(),
            subject: parts[5].into(),
        });
    }

    if entries.is_empty() {
        println!("No git history found for {}.", path.display());
        return Ok(());
    }

    if args.json {
        println!("{}", serde_json::to_string_pretty(&entries)?);
        return Ok(());
    }

    let unsigned = entries
        .iter()
        .filter(|e| e.signature_status == "no-signature")
        .count();
    let bad = entries
        .iter()
        .filter(|e| matches!(e.signature_status.as_str(), "bad" | "expired" | "revoked"))
        .count();

    println!("Audit of {} ({} commit(s))", path.display(), entries.len());
    println!("  signed   : {}", entries.len() - unsigned - bad);
    println!("  unsigned : {}", unsigned);
    println!("  problem  : {}", bad);
    println!();
    println!("{:<10} {:<20} {:<18} {:<14} {}", "commit", "date", "author", "signature", "subject");
    for e in &entries {
        let short = &e.commit[..e.commit.len().min(9)];
        let signer = if e.signer.is_empty() {
            "-".into()
        } else {
            e.signer.clone()
        };
        println!(
            "{:<10} {:<20} {:<18} {:<14} {}",
            short,
            &e.date[..e.date.len().min(19)],
            truncate(&e.author, 18),
            format!("{} {}", e.signature_status, truncate(&signer, 8)),
            e.subject,
        );
    }
    Ok(())
}

fn explain_sig(code: &str) -> String {
    match code {
        "G" => "good",
        "B" => "bad",
        "U" => "good-unknown",
        "X" => "expired",
        "Y" => "expired-key",
        "R" => "revoked",
        "E" => "cannot-check",
        "N" | "" => "no-signature",
        _ => "unknown",
    }
    .to_string()
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(n - 1).collect();
        out.push('…');
        out
    }
}
