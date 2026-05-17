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
    let (_, project) = load_resolved(file)?;
    let exts: Vec<String> = if args.extensions.is_empty() {
        DEFAULT_EXTS.iter().map(|s| s.to_string()).collect()
    } else {
        args.extensions.clone()
    };

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
