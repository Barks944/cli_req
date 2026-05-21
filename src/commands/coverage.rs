// Implements REQ-0026 (default coverage scan), REQ-0032 (--unlinked-files),
// REQ-0033 (--by-file) and REQ-0034 (--remap with --apply).
use anyhow::Result;
use once_cell::sync::Lazy;
use regex::Regex;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use crate::cli::CoverageArgs;
use crate::model::Status;
use crate::storage::load_resolved;

static REQ_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"REQ-\d{4}").unwrap());

// REQ-0033: default extension list for source scanning. Schema-as-code
// (SQL migrations, init scripts) is a first-class implementation surface
// for most backends, so `sql` is here too.
pub const DEFAULT_EXTS: &[&str] = &[
    "rs", "py", "js", "ts", "tsx", "go", "java", "md", "toml", "c", "cpp", "h", "sql",
];

/// REQ-0110: resolve extension list with CLI > _config > built-in defaults.
fn resolve_extensions(cli_exts: &[String], project: &crate::model::Project) -> Vec<String> {
    if !cli_exts.is_empty() {
        return cli_exts.to_vec();
    }
    if let Some(cfg) = project
        .config
        .as_ref()
        .and_then(|c| c.coverage.as_ref())
        .and_then(|c| c.extensions.as_ref())
    {
        if !cfg.is_empty() {
            return cfg.clone();
        }
    }
    DEFAULT_EXTS.iter().map(|s| s.to_string()).collect()
}
// REQ-0124: directory skip-list moved into source_walk via the
// `ignore` crate, which honours .gitignore + .ignore + global git
// excludes. Hard-coded SKIP_DIRS removed.

#[derive(serde::Serialize)]
struct Report {
    referenced: BTreeMap<String, Vec<String>>,
    /// REQ-0070: requirements referenced ONLY by test files — implementation
    /// markers absent. Distinct from `referenced` so a test-only marker
    /// does not falsely claim impl coverage.
    test_only: BTreeMap<String, Vec<String>>,
    /// Non-Draft, non-Obsolete requirements with no source marker. This
    /// is the strict-mode gating set: things the spec promises but the
    /// code doesn't cite.
    orphans: Vec<String>,
    /// REQ-0121: Draft requirements that also have no source marker.
    /// Reported separately so `req coverage` surfaces the full picture
    /// (and adopters running coverage on a fresh project see why the
    /// orphan count is zero) without breaking strict-mode gating.
    drafts_unmarked: Vec<String>,
    ghosts: BTreeMap<String, Vec<String>>,
    obsolete_referenced: BTreeMap<String, Vec<String>>,
}

pub fn run(args: CoverageArgs, file: &Option<PathBuf>) -> Result<()> {
    // REQ-0110: load the project up front so `_config.coverage.extensions`
    // contributes to extension resolution even on branches (unlinked_files,
    // by_file, remap) that didn't previously need the project.
    let (_, project) = load_resolved(file)?;
    let exts: Vec<String> = resolve_extensions(&args.extensions, &project);

    if args.unlinked_files {
        return run_unlinked_files(&args.path, &exts, args.json);
    }
    if args.by_file {
        return run_by_file(&args.path, &exts, args.json);
    }
    // REQ-0127: inverse view — REQ-NNNN → list of files referencing it.
    if args.by_req {
        return run_by_req(&args.path, &exts, args.json);
    }
    if !args.remap.is_empty() {
        return run_remap(&args.path, &exts, &args.remap, args.apply);
    }

    let mut hits: BTreeMap<String, Vec<String>> = BTreeMap::new();
    walk(&args.path, &exts, &mut |path, line_no, line| {
        for m in REQ_RE.find_iter(line) {
            let id = m.as_str().to_string();
            hits.entry(id)
                .or_default()
                .push(format!("{}:{}", path.display(), line_no));
        }
    });

    let known: BTreeSet<&String> = project.requirements.keys().collect();
    let mut report = Report {
        referenced: BTreeMap::new(),
        test_only: BTreeMap::new(),
        orphans: Vec::new(),
        drafts_unmarked: Vec::new(),
        ghosts: BTreeMap::new(),
        obsolete_referenced: BTreeMap::new(),
    };

    for (id, refs) in &hits {
        let has_impl = refs.iter().any(|r| !is_test_path(r));
        match project.requirements.get(id) {
            Some(r) if matches!(r.status, Status::Obsolete) => {
                report.obsolete_referenced.insert(id.clone(), refs.clone());
            }
            Some(_) if !has_impl => {
                // REQ-0070: only test files reference this requirement.
                report.test_only.insert(id.clone(), refs.clone());
            }
            Some(_) => {
                report.referenced.insert(id.clone(), refs.clone());
            }
            None => {
                report.ghosts.insert(id.clone(), refs.clone());
            }
        }
    }
    // Orphan = a requirement exists in the spec but no source marker
    // references it. Drafts haven't been implemented yet, so expecting
    // a marker is wrong by definition — exclude them. Obsolete reqs
    // are also excluded (already retired). This makes `req coverage
    // --strict` safe to use on projects that record near-term backlog
    // as Drafts without flooding CI with orphan findings.
    for id in known {
        if !hits.contains_key(id) {
            let r = &project.requirements[id];
            match r.status {
                Status::Obsolete => {}
                Status::Draft => report.drafts_unmarked.push(id.clone()),
                _ => report.orphans.push(id.clone()),
            }
        }
    }

    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        if args.strict {
            let allow: std::collections::HashSet<&String> = args.allow_orphans.iter().collect();
            let unexpected = report
                .orphans
                .iter()
                .filter(|id| !allow.contains(*id))
                .count();
            let findings = unexpected + report.ghosts.len() + report.obsolete_referenced.len();
            if findings > 0 {
                std::process::exit(1);
            }
        }
        return Ok(());
    }

    println!("Coverage report (root: {})", args.path.display());
    println!(
        "  referenced       : {}  (impl + maybe test markers)",
        report.referenced.len()
    );
    println!(
        "  test-only        : {}  (test marker but no impl marker)",
        report.test_only.len()
    );
    println!(
        "  orphans          : {}  (non-Draft non-Obsolete reqs with no marker — gated by --strict)",
        report.orphans.len()
    );
    println!(
        "  drafts unmarked  : {}  (Draft reqs with no marker — informational, not gated)",
        report.drafts_unmarked.len()
    );
    println!("  ghosts           : {}", report.ghosts.len());
    println!("  obsolete-in-code : {}", report.obsolete_referenced.len());
    if !report.orphans.is_empty() {
        println!("\nORPHANS (requirement exists but is not mentioned in code):");
        for id in &report.orphans {
            println!("  {}", id);
        }
    }
    if !report.drafts_unmarked.is_empty() {
        println!("\nDRAFTS UNMARKED (Draft reqs with no code marker yet — advance with `req update <id> --status implemented` once you add the marker):");
        for id in &report.drafts_unmarked {
            println!("  {}", id);
        }
    }
    if !report.ghosts.is_empty() {
        println!("\nGHOSTS (code mentions an unknown ID):");
        for (id, refs) in &report.ghosts {
            println!("  {}", id);
            for r in refs {
                println!("    {}", r);
            }
        }
    }
    if !report.test_only.is_empty() {
        println!("\nTEST-ONLY (referenced only by test files):");
        for (id, refs) in &report.test_only {
            println!("  {}", id);
            for r in refs {
                println!("    {}", r);
            }
        }
    }
    if !report.obsolete_referenced.is_empty() {
        println!("\nOBSOLETE-IN-CODE (code still references retired requirements):");
        for (id, refs) in &report.obsolete_referenced {
            println!("  {}", id);
            for r in refs {
                println!("    {}", r);
            }
        }
    }
    // REQ-0065: strict mode turns findings into a non-zero exit.
    if args.strict {
        let allow: std::collections::HashSet<&String> = args.allow_orphans.iter().collect();
        let unexpected_orphans: Vec<&String> = report
            .orphans
            .iter()
            .filter(|id| !allow.contains(*id))
            .collect();
        let findings =
            unexpected_orphans.len() + report.ghosts.len() + report.obsolete_referenced.len();
        if findings > 0 {
            eprintln!(
                "\ncoverage --strict: {} unallowed finding(s); exiting non-zero.",
                findings
            );
            std::process::exit(1);
        }
    }
    Ok(())
}

#[derive(serde::Serialize)]
struct UnlinkedReport {
    scanned: usize,
    linked: usize,
    unlinked: Vec<String>,
}

fn run_unlinked_files(root: &Path, exts: &[String], json: bool) -> Result<()> {
    let mut scanned = 0usize;
    let mut linked = 0usize;
    let mut unlinked: Vec<String> = Vec::new();
    walk_files(root, exts, &mut |path, has_marker| {
        scanned += 1;
        if has_marker {
            linked += 1;
        } else {
            unlinked.push(path.display().to_string());
        }
    });
    unlinked.sort();
    let report = UnlinkedReport {
        scanned,
        linked,
        unlinked,
    };

    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    let pct = if report.scanned == 0 {
        0.0
    } else {
        100.0 * (report.linked as f64) / (report.scanned as f64)
    };
    println!("Unlinked-files report (root: {})", root.display());
    println!("  scanned   : {}", report.scanned);
    println!("  linked    : {} ({:.0}%)", report.linked, pct);
    println!("  unlinked  : {}", report.unlinked.len());
    if !report.unlinked.is_empty() {
        println!("\nFiles with no REQ-NNNN markers:");
        for f in &report.unlinked {
            println!("  {}", f);
        }
    }
    Ok(())
}

// REQ-0124: gitignore-aware variant for the --unlinked-files / per-file
// reports. Yields each in-scope file with a bool indicating whether any
// REQ-marker is present.
fn walk_files(root: &Path, exts: &[String], visit: &mut impl FnMut(&Path, bool)) {
    crate::source_walk::walk_source_tree(root, exts, |path| {
        let has = fs::read_to_string(path)
            .map(|t| REQ_RE.is_match(&t))
            .unwrap_or(false);
        visit(path, has);
    });
}

#[derive(serde::Serialize)]
struct ByFileEntry {
    file: String,
    req_ids: Vec<String>,
}

/// REQ-0127: inverse of run_by_file — per-requirement list of files
/// referencing it.
fn run_by_req(root: &Path, exts: &[String], json: bool) -> Result<()> {
    let mut per_req: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    walk(root, exts, &mut |path, _line_no, line| {
        for m in REQ_RE.find_iter(line) {
            per_req
                .entry(m.as_str().to_string())
                .or_default()
                .insert(path.display().to_string());
        }
    });

    if json {
        // Flat REQ → [paths] map.
        let map: BTreeMap<&String, Vec<&String>> = per_req
            .iter()
            .map(|(id, files)| (id, files.iter().collect()))
            .collect();
        println!("{}", serde_json::to_string_pretty(&map)?);
        return Ok(());
    }

    if per_req.is_empty() {
        println!(
            "No files under {} contain REQ-NNNN markers.",
            root.display()
        );
        return Ok(());
    }
    println!("Per-requirement coverage (root: {}):\n", root.display());
    for (id, files) in &per_req {
        println!("  {}", id);
        for f in files {
            println!("    {}", f);
        }
    }
    Ok(())
}

fn run_by_file(root: &Path, exts: &[String], json: bool) -> Result<()> {
    let mut per_file: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    walk(root, exts, &mut |path, _line_no, line| {
        for m in REQ_RE.find_iter(line) {
            per_file
                .entry(path.display().to_string())
                .or_default()
                .insert(m.as_str().to_string());
        }
    });

    let entries: Vec<ByFileEntry> = per_file
        .into_iter()
        .map(|(file, ids)| ByFileEntry {
            file,
            req_ids: ids.into_iter().collect(),
        })
        .collect();

    if json {
        println!("{}", serde_json::to_string_pretty(&entries)?);
        return Ok(());
    }

    if entries.is_empty() {
        println!(
            "No files under {} contain REQ-NNNN markers.",
            root.display()
        );
        return Ok(());
    }
    println!("Per-file coverage (root: {}):\n", root.display());
    for e in &entries {
        println!("  {}", e.file);
        for id in &e.req_ids {
            println!("    {}", id);
        }
    }
    Ok(())
}

fn run_remap(root: &Path, exts: &[String], pairs: &[String], apply: bool) -> Result<()> {
    let mut map: BTreeMap<String, String> = BTreeMap::new();
    for raw in pairs {
        let (old, new) = raw
            .split_once('=')
            .ok_or_else(|| anyhow::anyhow!("--remap expects OLD=NEW, got '{}'", raw))?;
        if !REQ_RE.is_match(old) || !REQ_RE.is_match(new) {
            return Err(anyhow::anyhow!(
                "--remap values must look like REQ-NNNN: '{}={}' rejected",
                old,
                new
            ));
        }
        map.insert(old.to_string(), new.to_string());
    }

    let mut plan: Vec<(String, usize, String, String)> = Vec::new();
    walk(root, exts, &mut |path, line_no, line| {
        for (old, new) in &map {
            if line.contains(old) {
                plan.push((
                    path.display().to_string(),
                    line_no,
                    old.clone(),
                    new.clone(),
                ));
            }
        }
    });

    if plan.is_empty() {
        println!(
            "No occurrences of {} in {}.",
            pairs.join(", "),
            root.display()
        );
        return Ok(());
    }

    println!(
        "{} occurrence(s) of {} in {}:",
        plan.len(),
        pairs.join(", "),
        root.display()
    );
    for (file, line, old, new) in &plan {
        println!("  {}:{}  {} -> {}", file, line, old, new);
    }

    if !apply {
        println!("\nDry-run. Re-run with --apply to rewrite the files.");
        return Ok(());
    }

    let mut files: BTreeSet<String> = BTreeSet::new();
    for (file, _, _, _) in &plan {
        files.insert(file.clone());
    }
    for file in &files {
        let text = fs::read_to_string(file)?;
        let mut new_text = text.clone();
        for (old, new) in &map {
            new_text = new_text.replace(old.as_str(), new.as_str());
        }
        if new_text != text {
            fs::write(file, new_text)?;
        }
    }
    println!("\nRewrote {} file(s).", files.len());
    Ok(())
}

/// REQ-0070: classify a file:line marker as test-source or implementation.
/// Heuristic by path: anything under a `tests/` directory or matching
/// `*_test.<ext>` / `*_tests.<ext>` / `*.test.<ext>` counts as test.
pub fn is_test_path(file_ref: &str) -> bool {
    let normalised = file_ref.replace('\\', "/");
    let lower = normalised.to_lowercase();
    if lower.contains("/tests/") || lower.starts_with("tests/") || lower.starts_with("./tests/") {
        return true;
    }
    // strip `:lineno` suffix before suffix-matching
    let path_only = lower.split(':').next().unwrap_or(&lower);
    let suffixes = [
        "_test.rs",
        "_tests.rs",
        ".test.ts",
        ".test.tsx",
        ".test.js",
        "_test.py",
        "_test.go",
    ];
    suffixes.iter().any(|s| path_only.ends_with(s))
}

// REQ-0124: delegate the directory walk to source_walk, which honours
// .gitignore + .ignore + global excludes. The line-level marker scan
// stays per-file here.
fn walk(root: &Path, exts: &[String], visit: &mut impl FnMut(&Path, usize, &str)) {
    crate::source_walk::walk_source_tree(root, exts, |path| {
        if let Ok(text) = fs::read_to_string(path) {
            for (i, line) in text.lines().enumerate() {
                if REQ_RE.is_match(line) {
                    visit(path, i + 1, line);
                }
            }
        }
    });
}
