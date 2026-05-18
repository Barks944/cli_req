// REQ-0101: req lint — project quality audit beyond the validator.
// Surfaces soft signals (rationale length, acceptance count, test-record
// presence, marker coverage) without making them enforced rules.
// Output is markdown by default; --json for tooling. Exit code reflects
// validator errors only — lint observations never gate.
use anyhow::Result;
use serde_json::json;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::cli::LintArgs;
use crate::model::{Project, Status};
use crate::storage::load_resolved;
use crate::validate;

const SHORT_RATIONALE_WORDS: usize = 10;
const SINGLE_ACCEPTANCE: usize = 1;

pub fn run(args: LintArgs, file: &Option<PathBuf>) -> Result<()> {
    let (_, project) = load_resolved(file)?;
    let report = build_report(&project, &args.path);
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report.to_json())?);
    } else {
        print!("{}", report.to_markdown(&project));
    }
    if report.validator_errors > 0 {
        std::process::exit(1);
    }
    Ok(())
}

struct LintReport {
    project_name: String,
    total: usize,
    by_status: [usize; 6],
    validator_errors: usize,
    validator_warnings: usize,
    validator_findings: Vec<(String, Vec<validate::Finding>)>,
    markerless_active: Vec<String>,
    short_rationale: Vec<(String, usize)>,
    single_acceptance_functional: Vec<String>,
    no_test_record: Vec<String>,
    verification_kinds: [usize; 3], // automated, composition, inspection
}

fn build_report(project: &Project, src_path: &Path) -> LintReport {
    let total = project.requirements.len();
    let mut by_status = [0usize; 6];
    for r in project.requirements.values() {
        let i = match r.status {
            Status::Draft => 0,
            Status::Proposed => 1,
            Status::Approved => 2,
            Status::Implemented => 3,
            Status::Verified => 4,
            Status::Obsolete => 5,
        };
        by_status[i] += 1;
    }

    let validator_findings = validate::validate_project(project);
    let validator_errors: usize = validator_findings
        .iter()
        .flat_map(|(_, fs)| fs.iter())
        .filter(|f| f.error)
        .count();
    let validator_warnings: usize = validator_findings
        .iter()
        .flat_map(|(_, fs)| fs.iter())
        .filter(|f| !f.error)
        .count();

    // Marker coverage: scan src_path for // REQ-NNNN markers.
    // REQ-0101: "active" = not Draft and not Obsolete. Drafts are
    // sketches that haven't been implemented yet, so flagging them
    // as markerless is noise. The no-test-record check uses the same
    // scope, keeping the term consistent across lint sections.
    let referenced = scan_markers(src_path);
    let markerless_active: Vec<String> = project
        .requirements
        .iter()
        .filter(|(_, r)| !matches!(r.status, Status::Obsolete | Status::Draft))
        .filter(|(id, _)| !referenced.contains(*id))
        .map(|(id, _)| id.clone())
        .collect();

    let mut markerless_active = markerless_active;
    markerless_active.sort();

    // Soft observations.
    // REQ-0101: de-dupe with the validator. REQ-V-0013 already fires
    // on very-short rationales (<3 words). Suppress the lint entry
    // when the validator has already named the same requirement, so
    // a user doesn't see the same REQ flagged twice with different
    // thresholds.
    let validator_rationale_ids: std::collections::BTreeSet<String> = validator_findings
        .iter()
        .filter(|(_, fs)| fs.iter().any(|f| f.rule_code == "REQ-V-0013"))
        .map(|(id, _)| id.clone())
        .collect();
    let mut short_rationale: Vec<(String, usize)> = project
        .requirements
        .iter()
        .filter(|(_, r)| !matches!(r.status, Status::Obsolete))
        .filter_map(|(id, r)| {
            if validator_rationale_ids.contains(id) {
                return None;
            }
            let words = r.rationale.split_whitespace().count();
            if words < SHORT_RATIONALE_WORDS {
                Some((id.clone(), words))
            } else {
                None
            }
        })
        .collect();
    short_rationale.sort();

    let mut single_acceptance_functional: Vec<String> = project
        .requirements
        .iter()
        .filter(|(_, r)| {
            !matches!(r.status, Status::Obsolete)
                && matches!(r.kind, crate::model::Kind::Functional)
                && r.acceptance.len() <= SINGLE_ACCEPTANCE
        })
        .map(|(id, _)| id.clone())
        .collect();
    single_acceptance_functional.sort();

    let mut no_test_record: Vec<String> = project
        .requirements
        .iter()
        .filter(|(_, r)| {
            !matches!(r.status, Status::Obsolete | Status::Draft) && r.tests.is_empty()
        })
        .map(|(id, _)| id.clone())
        .collect();
    no_test_record.sort();

    let mut verification_kinds = [0usize; 3];
    for r in project.requirements.values() {
        if let Some(latest) = r.tests.last() {
            let i = match latest.kind {
                crate::model::EvidenceKind::Automated => 0,
                crate::model::EvidenceKind::Composition => 1,
                crate::model::EvidenceKind::Inspection => 2,
            };
            verification_kinds[i] += 1;
        }
    }

    LintReport {
        project_name: project.name.clone(),
        total,
        by_status,
        validator_errors,
        validator_warnings,
        validator_findings,
        markerless_active,
        short_rationale,
        single_acceptance_functional,
        no_test_record,
        verification_kinds,
    }
}

fn scan_markers(root: &Path) -> BTreeSet<String> {
    use regex::Regex;
    let re = Regex::new(r"REQ-\d{4}").unwrap();
    let mut found: BTreeSet<String> = BTreeSet::new();
    let skip_dirs = [
        ".git",
        "target",
        "node_modules",
        ".agent-sandbox",
        ".venv",
        "dist",
        "build",
    ];
    let exts = [
        "rs", "py", "js", "ts", "tsx", "go", "java", "kt", "scala", "swift", "cs", "rb", "php",
        "lua", "c", "cpp", "h", "hh", "hpp", "hxx", "m", "mm",
    ];
    fn walk(
        dir: &Path,
        skip: &[&str],
        exts: &[&str],
        re: &regex::Regex,
        out: &mut BTreeSet<String>,
    ) {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };
        for ent in entries.flatten() {
            let path = ent.path();
            let name = path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            if path.is_dir() {
                if skip.contains(&name.as_str()) {
                    continue;
                }
                walk(&path, skip, exts, re, out);
            } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                if exts.contains(&ext) {
                    if let Ok(text) = std::fs::read_to_string(&path) {
                        for cap in re.find_iter(&text) {
                            out.insert(cap.as_str().to_string());
                        }
                    }
                }
            }
        }
    }
    walk(root, &skip_dirs, &exts, &re, &mut found);
    found
}

impl LintReport {
    fn to_json(&self) -> serde_json::Value {
        json!({
            "project": self.project_name,
            "total": self.total,
            "by_status": {
                "draft":       self.by_status[0],
                "proposed":    self.by_status[1],
                "approved":    self.by_status[2],
                "implemented": self.by_status[3],
                "verified":    self.by_status[4],
                "obsolete":    self.by_status[5],
            },
            "validator": {
                "errors": self.validator_errors,
                "warnings": self.validator_warnings,
                "findings": self.validator_findings.iter().map(|(id, fs)| {
                    json!({
                        "id": id,
                        "findings": fs.iter().map(|f| json!({
                            "rule_code": f.rule_code,
                            "severity": if f.error { "error" } else { "warning" },
                            "field": f.field,
                            "message": f.message,
                        })).collect::<Vec<_>>(),
                    })
                }).collect::<Vec<_>>(),
            },
            "quality": {
                "markerless_active": self.markerless_active,
                // REQ-0101: short_rationale entries are objects so the
                // JSON shape matches the rest of the quality block (every
                // other field is an ID array; this one carries the word
                // count, but as a named field rather than a tuple).
                "short_rationale": self.short_rationale.iter().map(|(id, words)| {
                    json!({ "id": id, "words": words })
                }).collect::<Vec<_>>(),
                "single_acceptance_functional": self.single_acceptance_functional,
                "no_test_record": self.no_test_record,
                "verification_kinds": {
                    "automated":   self.verification_kinds[0],
                    "composition": self.verification_kinds[1],
                    "inspection":  self.verification_kinds[2],
                },
            },
        })
    }

    fn to_markdown(&self, project: &Project) -> String {
        let mut out = String::new();
        out.push_str(&format!("# req lint — {}\n\n", self.project_name));
        let headline_emoji = if self.validator_errors > 0 {
            "FAIL"
        } else if self.validator_warnings > 0 {
            "WARN"
        } else {
            "OK"
        };
        let quality_count = self.markerless_active.len()
            + self.short_rationale.len()
            + self.single_acceptance_functional.len()
            + self.no_test_record.len();
        out.push_str(&format!(
            "**Status:** {} — {} requirement(s); validate {} error(s), {} warning(s); {} quality observation(s).\n\n",
            headline_emoji, self.total, self.validator_errors, self.validator_warnings, quality_count
        ));

        out.push_str("## Status distribution\n\n");
        let labels = [
            "draft",
            "proposed",
            "approved",
            "implemented",
            "verified",
            "obsolete",
        ];
        for (i, lbl) in labels.iter().enumerate() {
            if self.by_status[i] > 0 {
                out.push_str(&format!("- **{}**: {}\n", lbl, self.by_status[i]));
            }
        }
        out.push('\n');

        if !self.validator_findings.is_empty() {
            out.push_str("## Validator findings\n\n");
            for (id, fs) in &self.validator_findings {
                for f in fs {
                    let sev = if f.error { "ERR " } else { "WARN" };
                    out.push_str(&format!(
                        "- {} **{}** `{}` [{}] {}\n",
                        sev, id, f.rule_code, f.field, f.message
                    ));
                }
            }
            out.push('\n');
        }

        out.push_str("## Quality observations\n\n");
        if quality_count == 0 {
            out.push_str("None. All active requirements have marker coverage, meaningful rationale, multiple acceptance criteria, and at least one test record.\n\n");
        } else {
            if !self.markerless_active.is_empty() {
                out.push_str(&format!(
                    "### Active requirements with no source marker ({})\n\nThese have not been linked from any `// REQ-NNNN:` comment. Add a marker in the file that implements the requirement, or document why no code marker is appropriate (verification-only, policy meta-req).\n\n",
                    self.markerless_active.len()
                ));
                for id in &self.markerless_active {
                    if let Some(r) = project.requirements.get(id) {
                        out.push_str(&format!(
                            "- **{}** — {} ({})\n",
                            id,
                            r.title,
                            r.status.as_str()
                        ));
                    }
                }
                out.push('\n');
            }
            if !self.short_rationale.is_empty() {
                out.push_str(&format!(
                    "### Rationales under {} words ({})\n\nA useful rationale names a cause or constraint, not just a consequence. Expand with `req update <id> -r \"...\" --reason \"...\"`.\n\n",
                    SHORT_RATIONALE_WORDS, self.short_rationale.len()
                ));
                for (id, w) in &self.short_rationale {
                    out.push_str(&format!("- **{}** — {} word(s)\n", id, w));
                }
                out.push('\n');
            }
            if !self.single_acceptance_functional.is_empty() {
                out.push_str(&format!(
                    "### Functional requirements with ≤1 acceptance criterion ({})\n\nOne acceptance criterion rarely covers a real obligation. Add more with `req update <id> --add-acceptance \"...\"`.\n\n",
                    self.single_acceptance_functional.len()
                ));
                for id in &self.single_acceptance_functional {
                    out.push_str(&format!("- **{}**\n", id));
                }
                out.push('\n');
            }
            if !self.no_test_record.is_empty() {
                out.push_str(&format!(
                    "### Active requirements with no test record ({})\n\nAdvanced-state requirements (Proposed onwards) without any evidence record. Attach one with `req test record`, `req verify --by inspection`, or `req test run --promote`.\n\n",
                    self.no_test_record.len()
                ));
                for id in &self.no_test_record {
                    out.push_str(&format!("- **{}**\n", id));
                }
                out.push('\n');
            }
        }

        out.push_str("## Verification kind distribution\n\n");
        out.push_str(&format!(
            "- **automated**: {}\n- **composition**: {}\n- **inspection**: {}\n\n",
            self.verification_kinds[0], self.verification_kinds[1], self.verification_kinds[2],
        ));

        out
    }
}
