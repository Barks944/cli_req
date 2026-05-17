// Implements REQ-0026 (scan source tree for REQ-XXXX markers; report
// referenced, orphans, ghosts, obsolete-in-code).
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

const DEFAULT_EXTS: &[&str] = &["rs", "py", "js", "ts", "tsx", "go", "java", "md", "toml", "c", "cpp", "h"];
const SKIP_DIRS: &[&str] = &[".git", "target", "node_modules", "dist", "build", ".venv", ".idea", ".vscode"];

#[derive(serde::Serialize)]
struct Report {
    referenced: BTreeMap<String, Vec<String>>,
    orphans: Vec<String>,
    ghosts: BTreeMap<String, Vec<String>>,
    obsolete_referenced: BTreeMap<String, Vec<String>>,
}

pub fn run(args: CoverageArgs, file: &Option<PathBuf>) -> Result<()> {
    let exts: Vec<String> = if args.extensions.is_empty() {
        DEFAULT_EXTS.iter().map(|s| s.to_string()).collect()
    } else {
        args.extensions.clone()
    };

    if args.unlinked_files {
        return run_unlinked_files(&args.path, &exts, args.json);
    }
    if args.by_file {
        return run_by_file(&args.path, &exts, args.json);
    }
    if !args.remap.is_empty() {
        return run_remap(&args.path, &exts, &args.remap, args.apply);
    }

    let (_, project) = load_resolved(file)?;
    let mut hits: BTreeMap<String, Vec<String>> = BTreeMap::new();
    walk(&args.path, &exts, &mut |path, line_no, line| {
        for m in REQ_RE.find_iter(line) {
            let id = m.as_str().to_string();
            hits.entry(id).or_default().push(format!("{}:{}", path.display(), line_no));
        }
    });

    let known: BTreeSet<&String> = project.requirements.keys().collect();
    let mut report = Report {
        referenced: BTreeMap::new(),
        orphans: Vec::new(),
        ghosts: BTreeMap::new(),
        obsolete_referenced: BTreeMap::new(),
    };

    for (id, refs) in &hits {
        match project.requirements.get(id) {
            Some(r) if matches!(r.status, Status::Obsolete) => {
                report.obsolete_referenced.insert(id.clone(), refs.clone());
            }
            Some(_) => {
                report.referenced.insert(id.clone(), refs.clone());
            }
            None => {
                report.ghosts.insert(id.clone(), refs.clone());
            }
        }
    }
    for id in known {
        if !hits.contains_key(id) {
            let r = &project.requirements[id];
            if !matches!(r.status, Status::Obsolete) {
                report.orphans.push(id.clone());
            }
        }
    }

    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    println!("Coverage report (root: {})", args.path.display());
    println!("  referenced       : {}", report.referenced.len());
    println!("  orphans          : {}", report.orphans.len());
    println!("  ghosts           : {}", report.ghosts.len());
    println!("  obsolete-in-code : {}", report.obsolete_referenced.len());
    if !report.orphans.is_empty() {
        println!("\nORPHANS (requirement exists but is not mentioned in code):");
        for id in &report.orphans {
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
    if !report.obsolete_referenced.is_empty() {
        println!("\nOBSOLETE-IN-CODE (code still references retired requirements):");
        for (id, refs) in &report.obsolete_referenced {
            println!("  {}", id);
            for r in refs {
                println!("    {}", r);
            }
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
    let report = UnlinkedReport { scanned, linked, unlinked };

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

fn walk_files(root: &Path, exts: &[String], visit: &mut impl FnMut(&Path, bool)) {
    let entries = match fs::read_dir(root) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name_s = name.to_string_lossy();
        if path.is_dir() {
            if SKIP_DIRS.iter().any(|s| *s == name_s.as_ref()) {
                continue;
            }
            walk_files(&path, exts, visit);
        } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            if !exts.iter().any(|x| x == ext) {
                continue;
            }
            let has = fs::read_to_string(&path)
                .map(|t| REQ_RE.is_match(&t))
                .unwrap_or(false);
            visit(&path, has);
        }
    }
}

#[derive(serde::Serialize)]
struct ByFileEntry {
    file: String,
    req_ids: Vec<String>,
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
        .map(|(file, ids)| ByFileEntry { file, req_ids: ids.into_iter().collect() })
        .collect();

    if json {
        println!("{}", serde_json::to_string_pretty(&entries)?);
        return Ok(());
    }

    if entries.is_empty() {
        println!("No files under {} contain REQ-NNNN markers.", root.display());
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
                plan.push((path.display().to_string(), line_no, old.clone(), new.clone()));
            }
        }
    });

    if plan.is_empty() {
        println!("No occurrences of {} in {}.", pairs.join(", "), root.display());
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

fn walk(root: &Path, exts: &[String], visit: &mut impl FnMut(&Path, usize, &str)) {
    let entries = match fs::read_dir(root) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name_s = name.to_string_lossy();
        if path.is_dir() {
            if SKIP_DIRS.iter().any(|s| *s == name_s.as_ref()) {
                continue;
            }
            walk(&path, exts, visit);
        } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            if !exts.iter().any(|x| x == ext) {
                continue;
            }
            if let Ok(text) = fs::read_to_string(&path) {
                for (i, line) in text.lines().enumerate() {
                    if REQ_RE.is_match(line) {
                        visit(&path, i + 1, line);
                    }
                }
            }
        }
    }
}
