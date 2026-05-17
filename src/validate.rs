// Implements REQ-0006 (modal verb), REQ-0007 (weasel words),
// REQ-0008 (acceptance for functional), REQ-0009 (status-transition guard),
// REQ-0029 (compound-statement heuristics), REQ-0030 (Unicode-char title
// length), REQ-0045 (stable REQ-V-NNNN rule codes on every Finding).
use once_cell::sync::Lazy;
use regex::Regex;

use crate::model::{Kind, Project, Requirement, Status};

/// A validation finding. `error = true` blocks the operation; otherwise it's a warning.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Finding {
    pub error: bool,
    pub field: &'static str,
    pub rule_code: &'static str,
    pub message: String,
}

impl Finding {
    fn err(code: &'static str, field: &'static str, message: impl Into<String>) -> Self {
        Self { error: true, field, rule_code: code, message: message.into() }
    }
    fn warn(code: &'static str, field: &'static str, message: impl Into<String>) -> Self {
        Self { error: false, field, rule_code: code, message: message.into() }
    }
}

/// Stable rule-code catalog. Keep in sync with `req help errors`.
/// Adding a code is backwards-compatible; renumbering existing codes is NOT.
/// Exported so future surfaces (req help errors --json, MCP req_help) can
/// emit the table without re-parsing prose.
#[allow(dead_code)]
pub const RULES: &[(&str, &str)] = &[
    ("REQ-V-0001", "title is required"),
    ("REQ-V-0002", "title is too short (min 5 characters)"),
    ("REQ-V-0003", "title is too long (max 120 characters)"),
    ("REQ-V-0004", "title ends with a period (warn)"),
    ("REQ-V-0005", "statement is required"),
    ("REQ-V-0006", "statement must be a complete sentence (>=5 words)"),
    ("REQ-V-0007", "statement is too long (>80 words, warn)"),
    ("REQ-V-0008", "statement must contain a normative modal verb"),
    ("REQ-V-0009", "statement contains a weasel word (warn)"),
    ("REQ-V-0010", "statement looks compound (warn)"),
    ("REQ-V-0011", "statement must not be a question"),
    ("REQ-V-0012", "rationale is required"),
    ("REQ-V-0013", "rationale is very short (warn)"),
    ("REQ-V-0014", "functional requirement is missing acceptance criteria"),
    ("REQ-V-0015", "acceptance criterion is too vague (warn)"),
    ("REQ-V-0016", "link target does not exist"),
    ("REQ-V-0017", "self-link not allowed"),
    ("REQ-V-0018", "status requires acceptance for functional requirement"),
];

static WEASEL_WORDS: &[&str] = &[
    "etc", "and/or", "user-friendly", "easy to use", "robust", "fast",
    "efficient", "flexible", "approximately", "as appropriate", "if possible",
    "tbd", "to be determined", "various", "some", "many", "few",
    "minimal", "maximal", "state-of-the-art", "seamless",
];

static MODAL_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\b(shall|must|should|will)\b").unwrap()
});

static URL_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\b[a-z][a-z0-9+.-]*://\S+").unwrap()
});

static BACKTICK_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"`[^`]*`").unwrap());

/// Strip URLs and inline `code` so the modal-verb check doesn't match a
/// `shall.example.com` host or a `should_run()` identifier.
fn strip_non_prose(s: &str) -> String {
    let no_urls = URL_RE.replace_all(s, " ");
    let no_code = BACKTICK_RE.replace_all(&no_urls, " ");
    no_code.into_owned()
}

pub fn validate_requirement(r: &Requirement) -> Vec<Finding> {
    let mut out = Vec::new();

    let title = r.title.trim();
    let title_chars = title.chars().count();
    if title.is_empty() {
        out.push(Finding::err("REQ-V-0001", "title", "title is required"));
    } else if title_chars < 5 {
        out.push(Finding::err("REQ-V-0002", "title", "title is too short (min 5 characters)"));
    } else if title_chars > 120 {
        out.push(Finding::err("REQ-V-0003", "title", "title is too long (max 120 characters)"));
    }
    if title.ends_with('.') {
        out.push(Finding::warn("REQ-V-0004", "title", "drop the trailing period — titles are not sentences"));
    }

    let stmt = r.statement.trim();
    if stmt.is_empty() {
        out.push(Finding::err("REQ-V-0005", "statement", "statement is required"));
    } else {
        let words = stmt.split_whitespace().count();
        if words < 5 {
            out.push(Finding::err("REQ-V-0006", "statement", "statement must be a complete sentence (>=5 words)"));
        }
        if words > 80 {
            out.push(Finding::warn(
                "REQ-V-0007", "statement",
                format!("statement is {} words long — split into atomic requirements", words),
            ));
        }
        let prose = strip_non_prose(stmt);
        if !MODAL_RE.is_match(&prose) {
            out.push(Finding::err(
                "REQ-V-0008", "statement",
                "statement must contain a normative modal verb (shall / must / should / will)",
            ));
        }
        let lower = stmt.to_lowercase();
        for w in WEASEL_WORDS {
            if lower.contains(w) {
                out.push(Finding::warn(
                    "REQ-V-0009", "statement",
                    format!("avoid the vague term '{}': prefer a measurable criterion", w),
                ));
            }
        }
        let modal_hits = MODAL_RE.find_iter(&prose).count();
        let csv_clauses = stmt
            .split(',')
            .filter(|s| !s.trim().is_empty())
            .count();
        let looks_compound = stmt.contains(';')
            || modal_hits > 1
            || (csv_clauses >= 3 && stmt.contains(" and "));
        if looks_compound {
            out.push(Finding::warn(
                "REQ-V-0010", "statement",
                "statement looks compound — split into atomic requirements",
            ));
        }
        if stmt.contains('?') {
            out.push(Finding::err("REQ-V-0011", "statement", "statement must not be a question"));
        }
    }

    if r.rationale.trim().is_empty() {
        out.push(Finding::err("REQ-V-0012", "rationale", "rationale is required — explain WHY"));
    } else if r.rationale.split_whitespace().count() < 3 {
        out.push(Finding::warn("REQ-V-0013", "rationale", "rationale is very short"));
    }

    if matches!(r.kind, Kind::Functional) && r.acceptance.is_empty() {
        out.push(Finding::err(
            "REQ-V-0014", "acceptance",
            "functional requirements need at least one acceptance criterion",
        ));
    }
    for (i, ac) in r.acceptance.iter().enumerate() {
        if ac.trim().split_whitespace().count() < 3 {
            out.push(Finding::warn(
                "REQ-V-0015", "acceptance",
                format!("acceptance #{} is too vague to verify", i + 1),
            ));
        }
    }

    out
}

pub fn validate_project(p: &Project) -> Vec<(String, Vec<Finding>)> {
    let mut out = Vec::new();
    for (id, r) in &p.requirements {
        let mut findings = validate_requirement(r);
        for link in &r.links {
            if !p.requirements.contains_key(&link.target) {
                findings.push(Finding::err(
                    "REQ-V-0016", "links",
                    format!("link target {} does not exist", link.target),
                ));
            } else if link.target == r.id {
                findings.push(Finding::err("REQ-V-0017", "links", "self-link is not allowed"));
            }
        }
        if matches!(r.status, Status::Approved | Status::Implemented | Status::Verified)
            && r.acceptance.is_empty()
            && matches!(r.kind, Kind::Functional)
        {
            findings.push(Finding::err(
                "REQ-V-0018", "status",
                "cannot be approved/implemented/verified without acceptance criteria",
            ));
        }
        if !findings.is_empty() {
            out.push((id.clone(), findings));
        }
    }
    out
}

pub fn errors_only(findings: &[Finding]) -> Vec<&Finding> {
    findings.iter().filter(|f| f.error).collect()
}
