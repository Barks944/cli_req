// Implements REQ-0006 (modal verb), REQ-0007 (weasel words),
// REQ-0008 (acceptance for functional), REQ-0009 (status-transition guard),
// REQ-0029 (compound-statement heuristics), REQ-0030 (Unicode-char title
// length), REQ-0045 (stable REQ-V-NNNN rule codes on every Finding).
use once_cell::sync::Lazy;
use regex::Regex;

use crate::model::{
    EvidenceKind, HazardStatus, Kind, LinkKind, Project, Requirement, SafetyFunctionStatus, Sil,
    Status,
};

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
        Self {
            error: true,
            field,
            rule_code: code,
            message: message.into(),
        }
    }
    fn warn(code: &'static str, field: &'static str, message: impl Into<String>) -> Self {
        Self {
            error: false,
            field,
            rule_code: code,
            message: message.into(),
        }
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
    (
        "REQ-V-0006",
        "statement must be a complete sentence (>=5 words)",
    ),
    ("REQ-V-0007", "statement is too long (>80 words, warn)"),
    (
        "REQ-V-0008",
        "statement must contain a normative modal verb",
    ),
    ("REQ-V-0009", "statement contains a weasel word (warn)"),
    ("REQ-V-0010", "statement looks compound (warn)"),
    ("REQ-V-0011", "statement must not be a question"),
    ("REQ-V-0012", "rationale is required"),
    ("REQ-V-0013", "rationale is very short (warn)"),
    (
        "REQ-V-0014",
        "functional requirement is missing acceptance criteria",
    ),
    ("REQ-V-0015", "acceptance criterion is too vague (warn)"),
    ("REQ-V-0016", "link target does not exist"),
    ("REQ-V-0017", "self-link not allowed"),
    (
        "REQ-V-0018",
        "status requires acceptance for functional requirement",
    ),
    (
        "REQ-V-0019",
        "verifies-link source has no test record (verification claim without evidence)",
    ),
    (
        "REQ-V-0020",
        "duplicate-intent: another non-obsolete requirement is semantically very similar",
    ),
    (
        "REQ-V-0021",
        "link cycle detected (graph-level; one finding per cycle)",
    ),
    (
        "REQ-V-0022",
        "statement stacks uncertainty hedges (perhaps, probably, maybe, possibly, might) (warn)",
    ),
    (
        "REQ-V-0023",
        "external statement-quality hook flagged this requirement (opt-in via REQ_VALIDATE_LLM_CMD)",
    ),
    (
        "REQ-V-0024",
        "verified status but latest test record outcome is Fail — structural contradiction (warn)",
    ),
    ("REQ-V-0025", "hazard is missing its free-text harm narrative"),
    (
        "REQ-V-0026",
        "hazard is assessed (or beyond) but missing one or more C/F/P/W risk parameters",
    ),
    (
        "REQ-V-0027",
        "hazard is mitigated (or beyond) but no live safety function mitigates it",
    ),
    (
        "REQ-V-0028",
        "safety mitigates/realizes link points at a non-existent target",
    ),
    (
        "REQ-V-0029",
        "safety function is allocated (or beyond) but mitigates no hazard",
    ),
    (
        "REQ-V-0030",
        "safety requirement is Verified but carries no passing evidence",
    ),
    (
        "REQ-V-0031",
        "SIL 3/4 safety requirement verified on inspection-only evidence (error without an audited --force exception; warn with one)",
    ),
];

static HEDGE_WORDS: &[&str] = &[
    "perhaps",
    "probably",
    "maybe",
    "possibly",
    "might",
    "roughly",
    "potentially",
];

static WEASEL_WORDS: &[&str] = &[
    "etc",
    "and/or",
    "user-friendly",
    "easy to use",
    "robust",
    "fast",
    "efficient",
    "flexible",
    "approximately",
    "as appropriate",
    "if possible",
    "tbd",
    "to be determined",
    "various",
    "some",
    "many",
    "few",
    "minimal",
    "maximal",
    "state-of-the-art",
    "seamless",
];

static MODAL_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\b(shall|must|should|will)\b").unwrap());

static URL_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)\b[a-z][a-z0-9+.-]*://\S+").unwrap());

static BACKTICK_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"`[^`]*`").unwrap());

// REQ-0113: priority-label tokens (Must/Should/Could/Wont) are reserved
// terms in req's domain. When they appear as a noun-phrase reference to
// the priority field (`Must-priority requirements`) they are not modal
// verbs and must be stripped before the modal-verb count, or the
// validator's own ruleset bites our own spec. Narrowest fix: strip the
// token only when immediately followed by `-priority` or `-priorities`.
static PRIORITY_LABEL_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\b(must|should|could|wont)(-priorit(?:y|ies))\b").unwrap());

/// Strip URLs and inline `code` so the modal-verb check doesn't match a
/// `shall.example.com` host or a `should_run()` identifier. Also strip
/// priority-label tokens that sit next to priority-noun usage (REQ-0113).
fn strip_non_prose(s: &str) -> String {
    let no_urls = URL_RE.replace_all(s, " ");
    let no_code = BACKTICK_RE.replace_all(&no_urls, " ");
    let no_priority_labels = PRIORITY_LABEL_RE.replace_all(&no_code, " ");
    no_priority_labels.into_owned()
}

// REQ-0102: validator findings name the cause and a suggested fix so
// each warning is a teaching moment rather than a terse rule name.
pub fn validate_requirement(r: &Requirement) -> Vec<Finding> {
    let mut out = Vec::new();

    let title = r.title.trim();
    let title_chars = title.chars().count();
    if title.is_empty() {
        out.push(Finding::err(
            "REQ-V-0001",
            "title",
            "title is required — write a short imperative phrase naming the obligation",
        ));
    } else if title_chars < 5 {
        out.push(Finding::err(
            "REQ-V-0002",
            "title",
            format!(
                "title is too short ({} chars; min 5) — expand to a phrase a reviewer can grep for",
                title_chars
            ),
        ));
    } else if title_chars > 120 {
        out.push(Finding::err(
            "REQ-V-0003",
            "title",
            format!(
                "title is too long ({} chars; max 120) — move detail into the statement, keep the title scannable",
                title_chars
            ),
        ));
    }
    if title.ends_with('.') {
        out.push(Finding::warn(
            "REQ-V-0004",
            "title",
            "trailing period on title — titles are noun phrases, not sentences; drop it",
        ));
    }

    let stmt = r.statement.trim();
    if stmt.is_empty() {
        out.push(Finding::err(
            "REQ-V-0005",
            "statement",
            "statement is required — write the obligation as one sentence with a normative modal verb",
        ));
    } else {
        let words = stmt.split_whitespace().count();
        if words < 5 {
            out.push(Finding::err(
                "REQ-V-0006",
                "statement",
                format!(
                    "statement is {} words; needs ≥5 — a complete obligation usually reads `<actor> shall <verb> <object> <condition>`",
                    words
                ),
            ));
        }
        // REQ-0102: statement-length messages quote the actual word count.
        if words > 80 {
            out.push(Finding::warn(
                "REQ-V-0007",
                "statement",
                format!(
                    "statement is {} words — try `req split <id>` to break it into atomic obligations",
                    words
                ),
            ));
        }
        let prose = strip_non_prose(stmt);
        if !MODAL_RE.is_match(&prose) {
            out.push(Finding::err(
                "REQ-V-0008",
                "statement",
                "statement has no normative modal verb (shall / must / should / will) — see `req help best-practice` for which to use when",
            ));
        }
        // Weasel + compound checks both run against the stripped-prose form
        // so that backtick-wrapped cited terms and embedded enumerations do
        // not trip rules they exist only to describe.
        let prose_lower = prose.to_lowercase();
        for w in WEASEL_WORDS {
            if prose_lower.contains(w) {
                out.push(Finding::warn(
                    "REQ-V-0009",
                    "statement",
                    format!(
                        "vague term '{}' — replace with a measurable criterion (specific number, named protocol, exact behaviour)",
                        w
                    ),
                ));
            }
        }
        let modal_hits = MODAL_RE.find_iter(&prose).count();
        // Compound-statement heuristic, rebuilt for v0.1.2 after the agent
        // QA sweep:
        //   • Multiple modal verbs ("shall X and shall Y") — strong signal.
        //   • Semicolon — strong signal.
        //   • Multiple " and " joins ("A and B and C") — strong signal even
        //     with a single modal; this was the headline false-negative.
        //   • An Oxford-comma list ("A, B, and C") is one obligation acting
        //     on a list, not a compound statement, so suppress when there
        //     are 2+ commas and only one " and ". This was the headline
        //     false-positive that fired on intentional enumerations.
        let and_joins = prose.to_lowercase().matches(" and ").count();
        let comma_count = prose.matches(',').count();
        let looks_enumeration = and_joins == 1 && comma_count >= 2;
        let looks_compound =
            prose.contains(';') || modal_hits > 1 || (and_joins >= 2 && !looks_enumeration);
        if looks_compound {
            let reason = if prose.contains(';') {
                "semicolon detected"
            } else if modal_hits > 1 {
                "repeated modal verb (`shall X and shall Y`)"
            } else {
                "multiple `and` joins (`A and B and C`)"
            };
            out.push(Finding::warn(
                "REQ-V-0010",
                "statement",
                format!(
                    "compound statement — {} — try `req split <id>` to break it into atomic siblings",
                    reason
                ),
            ));
        }
        // REQ-0089 / REQ-V-0022: stacked uncertainty hedges. A single hedge in
        // prose is sloppy; two or more is a smell that the author
        // doesn't know what they want.
        // REQ-0102: hedge-stacking message names the offending pattern AND
        // quotes the matching words so the author sees exactly what to fix.
        let hedge_words_found: Vec<&str> = HEDGE_WORDS
            .iter()
            .copied()
            .filter(|w| prose_lower.contains(*w))
            .collect();
        if hedge_words_found.len() >= 2 {
            let quoted = hedge_words_found
                .iter()
                .map(|w| format!("`{}`", w))
                .collect::<Vec<_>>()
                .join(", ");
            out.push(Finding::warn(
                "REQ-V-0022",
                "statement",
                format!(
                    "uncertainty hedges stacked: {} — commit to a concrete behaviour and state what the system actually does",
                    quoted
                ),
            ));
        }
        if stmt.contains('?') {
            out.push(Finding::err(
                "REQ-V-0011",
                "statement",
                "statement contains `?` — a requirement is an obligation, not a question; rephrase as a positive `<actor> shall <verb>` clause",
            ));
        }
    }

    if r.rationale.trim().is_empty() {
        out.push(Finding::err(
            "REQ-V-0012",
            "rationale",
            "rationale is required — explain WHY this requirement exists (which need it serves, which past mistake it prevents, which stakeholder asked)",
        ));
    } else {
        let word_count = r.rationale.split_whitespace().count();
        if word_count < 3 {
            out.push(Finding::warn(
                "REQ-V-0013",
                "rationale",
                format!(
                    "rationale is only {} word(s) — a useful rationale names the cause or constraint, not just the consequence",
                    word_count
                ),
            ));
        }
    }

    // REQ-0102: acceptance-missing message points at the fix command.
    if matches!(r.kind, Kind::Functional) && r.acceptance.is_empty() {
        out.push(Finding::err(
            "REQ-V-0014",
            "acceptance",
            "functional requirement has no acceptance criteria — add at least one observable behaviour a reviewer can check off (`req update <id> --add-acceptance \"...\"`)",
        ));
    }
    for (i, ac) in r.acceptance.iter().enumerate() {
        if ac.split_whitespace().count() < 3 {
            out.push(Finding::warn(
                "REQ-V-0015",
                "acceptance",
                format!(
                    "acceptance #{} is only {} word(s) — name a concrete observable, e.g. `req validate exits 0 on a clean project`",
                    i + 1,
                    ac.split_whitespace().count()
                ),
            ));
        }
    }

    out
}

/// REQ-0076: similarity threshold for duplicate-intent detection. Jaccard
/// on lowercased token sets of (title + statement). 0.65 trips on near
/// rewordings without flagging coincidentally-overlapping vocabularies.
pub const DUP_INTENT_THRESHOLD: f64 = 0.65;

fn token_set(s: &str) -> std::collections::HashSet<String> {
    use once_cell::sync::Lazy;
    use regex::Regex;
    static STOP: Lazy<std::collections::HashSet<&'static str>> = Lazy::new(|| {
        [
            "the", "a", "an", "and", "or", "of", "to", "for", "on", "in", "is", "be", "by", "with",
            "as", "that", "this", "shall", "must", "should", "will", "system", "cli",
        ]
        .iter()
        .copied()
        .collect()
    });
    static WORD_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"[a-z0-9]+").unwrap());
    let lower = s.to_lowercase();
    WORD_RE
        .find_iter(&lower)
        .map(|m| m.as_str().to_string())
        .filter(|w| w.len() > 2 && !STOP.contains(w.as_str()))
        .collect()
}

fn jaccard(a: &std::collections::HashSet<String>, b: &std::collections::HashSet<String>) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 0.0;
    }
    let inter = a.intersection(b).count() as f64;
    let union = a.union(b).count() as f64;
    inter / union
}

pub fn validate_project(p: &Project) -> Vec<(String, Vec<Finding>)> {
    let mut out = Vec::new();
    // Precompute token sets for non-obsolete requirements once.
    let active: Vec<(&String, std::collections::HashSet<String>)> = p
        .requirements
        .iter()
        .filter(|(_, r)| !matches!(r.status, crate::model::Status::Obsolete))
        .map(|(id, r)| (id, token_set(&format!("{} {}", r.title, r.statement))))
        .collect();
    for (id, r) in &p.requirements {
        let mut findings = validate_requirement(r);
        // Advisory (warning-level) findings on retired requirements are
        // pure noise — they cannot be re-edited via the normal flow and
        // Obsolete is a terminal state. Drop warnings; keep errors.
        if matches!(r.status, crate::model::Status::Obsolete) {
            findings.retain(|f| f.error);
        }
        // REQ-0076: duplicate-intent detection across non-obsolete reqs.
        if !matches!(r.status, crate::model::Status::Obsolete) {
            let my_tokens: Option<&std::collections::HashSet<String>> = active
                .iter()
                .find_map(|(aid, ts)| if *aid == id { Some(ts) } else { None });
            if let Some(my) = my_tokens {
                for (other_id, other_tokens) in &active {
                    if *other_id == id {
                        continue;
                    }
                    // Only flag once per pair: the one with the smaller ID gets the warning.
                    if id.as_str() > other_id.as_str() {
                        continue;
                    }
                    let sim = jaccard(my, other_tokens);
                    if sim >= DUP_INTENT_THRESHOLD {
                        findings.push(Finding::warn(
                            "REQ-V-0020",
                            "statement",
                            format!(
                                "duplicate-intent: {} overlaps {} at {:.0}% similarity",
                                id,
                                other_id,
                                sim * 100.0
                            ),
                        ));
                    }
                }
            }
        }
        for link in &r.links {
            if !p.requirements.contains_key(&link.target) {
                findings.push(Finding::err(
                    "REQ-V-0016",
                    "links",
                    format!("link target {} does not exist", link.target),
                ));
            } else if link.target == r.id {
                findings.push(Finding::err(
                    "REQ-V-0017",
                    "links",
                    "self-link is not allowed",
                ));
            }
            // REQ-0077: a `verifies` link is a verification claim — if the
            // source has no test record at all, the claim has no evidence.
            // REQ-0093: gated on status >= Implemented. A Draft/Proposed/
            // Approved requirement is not yet expected to carry evidence,
            // and firing the warning that early trains authors to ignore
            // validator output.
            let evidence_expected = matches!(r.status, Status::Implemented | Status::Verified);
            if matches!(link.kind, crate::model::LinkKind::Verifies)
                && r.tests.is_empty()
                && evidence_expected
            {
                findings.push(Finding::warn(
                    "REQ-V-0019",
                    "links",
                    format!(
                        "verifies → {} but {} has no test records — attach evidence with `req test record {} --result pass --notes \"...\"` or `req verify {} --by inspection --notes \"...\"`",
                        link.target, r.id, r.id, r.id
                    ),
                ));
            }
        }
        if matches!(
            r.status,
            Status::Approved | Status::Implemented | Status::Verified
        ) && r.acceptance.is_empty()
            && matches!(r.kind, Kind::Functional)
        {
            findings.push(Finding::err(
                "REQ-V-0018",
                "status",
                "cannot be approved/implemented/verified without acceptance criteria",
            ));
        }
        // REQ-V-0024 (REQ-0130): Verified status with a failing latest
        // test record is a structural contradiction — the spec claims
        // the behaviour works while the most recent evidence says it
        // does not. Warning, not error: pre-commit and active dev are
        // both fine with a known-failing record in flight; CI hard
        // gates via `req review --gate --no-defects` (REQ-0126).
        if matches!(r.status, Status::Verified) {
            if let Some(latest) = r.tests.last() {
                if matches!(latest.outcome, crate::model::TestOutcome::Fail) {
                    findings.push(Finding::warn(
                        "REQ-V-0024",
                        "status",
                        "verified status but latest test record is Fail — re-record on a fix, or move status back with `req update <id> --status implemented --reason \"...\"`",
                    ));
                }
            }
        }
        if !findings.is_empty() {
            out.push((id.clone(), findings));
        }
    }
    // REQ-0087 / REQ-V-0023: opt-in external statement-quality hook.
    // REQ-0097: bounded parallelism via REQ_VALIDATE_LLM_CONCURRENCY. The CLI
    // stays deterministic by default; only when REQ_VALIDATE_LLM_CMD
    // is set do we shell out (per non-obsolete requirement) to ask
    // an external judge whether the statement is testable. The hook
    // is fed a small JSON stub on stdin and returns
    // `{ "ok": bool, "message": "..." }` on stdout. Failure of the
    // hook itself surfaces as a single REQ-V-0023 warning but does
    // not stop the rest of validation.
    if let Ok(cmd) = std::env::var("REQ_VALIDATE_LLM_CMD") {
        let trimmed = cmd.trim().to_string();
        if !trimmed.is_empty() {
            // REQ-0097: configurable concurrency cap. Default 1 keeps
            // behaviour identical to 0.2.x for callers that don't opt
            // in. Higher values fan out across non-obsolete reqs in
            // a thread pool; findings are sorted by id afterwards so
            // output stays stable regardless of completion order.
            let concurrency: usize = std::env::var("REQ_VALIDATE_LLM_CONCURRENCY")
                .ok()
                .and_then(|s| s.parse().ok())
                .filter(|n: &usize| *n >= 1)
                .unwrap_or(1);
            // Build the work list once.
            let work: Vec<(String, String)> = p
                .requirements
                .iter()
                .filter(|(_, r)| !matches!(r.status, crate::model::Status::Obsolete))
                .map(|(id, r)| {
                    let payload = serde_json::json!({
                        "id": id,
                        "title": r.title,
                        "statement": r.statement,
                        "rationale": r.rationale,
                    })
                    .to_string();
                    (id.clone(), payload)
                })
                .collect();
            // Run, batching in `concurrency`-sized chunks. Each chunk
            // spawns N OS threads; we join them all before the next
            // chunk. Simple and good enough for the IO-bound LLM
            // workload — no crate dependency.
            type LlmJob = std::thread::JoinHandle<(String, Result<(bool, String), String>)>;
            let mut llm_findings: Vec<(String, Finding)> = Vec::new();
            for chunk in work.chunks(concurrency.max(1)) {
                let mut handles: Vec<LlmJob> = Vec::new();
                for (id, payload) in chunk {
                    let id = id.clone();
                    let payload = payload.clone();
                    let cmd = trimmed.clone();
                    handles.push(std::thread::spawn(move || {
                        let outcome = run_llm_hook(&cmd, &payload);
                        (id, outcome)
                    }));
                }
                for h in handles {
                    if let Ok((id, outcome)) = h.join() {
                        match outcome {
                            Ok((true, _)) => continue,
                            Ok((false, message)) => llm_findings.push((
                                id,
                                Finding::warn(
                                    "REQ-V-0023",
                                    "statement",
                                    format!("LLM hook flagged: {}", message),
                                ),
                            )),
                            Err(e) => llm_findings.push((
                                id,
                                Finding::warn(
                                    "REQ-V-0023",
                                    "statement",
                                    format!("LLM hook unavailable: {}", e),
                                ),
                            )),
                        }
                    }
                }
            }
            // Sort by id so the final findings list is order-stable
            // regardless of which thread finished first.
            llm_findings.sort_by(|a, b| a.0.cmp(&b.0));
            for (id, finding) in llm_findings {
                if let Some((_, existing)) = out.iter_mut().find(|(rid, _)| *rid == id) {
                    existing.push(finding);
                } else {
                    out.push((id, vec![finding]));
                }
            }
        }
    }
    // REQ-0088 / REQ-V-0021: walk the link graph per asymmetric kind and surface
    // any cycles. Direct `req link` rejects new cycle-closing edges,
    // but cycles can still enter a project via batch (pre-0.1.2), via
    // hand-edit + repair, or via merge. This is the second-line
    // defence so users see the problem before it surfaces as a hang
    // somewhere downstream. Each cycle is reported once, attributed
    // to its smallest-ID member.
    for kind in [
        crate::model::LinkKind::Parent,
        crate::model::LinkKind::DependsOn,
        crate::model::LinkKind::Refines,
        crate::model::LinkKind::Verifies,
    ] {
        let cycles = find_cycles(p, kind);
        for cycle in cycles {
            let owner = cycle
                .iter()
                .min()
                .cloned()
                .unwrap_or_else(|| cycle[0].clone());
            let path = cycle.join(" -> ");
            let finding = Finding::err(
                "REQ-V-0021",
                "links",
                format!("{} cycle: {} -> {}", kind.as_str(), path, cycle[0]),
            );
            // If `owner` already has findings, attach; otherwise add a
            // new entry. Keeping output stable on (id, rule).
            if let Some((_, existing)) = out.iter_mut().find(|(rid, _)| *rid == owner) {
                existing.push(finding);
            } else {
                out.push((owner, vec![finding]));
            }
        }
    }
    // REQ-0137: functional-safety artifacts validate on the same pass so
    // the pre-commit hook and CI gate cover the whole spec.
    for (id, findings) in validate_safety(p) {
        if let Some((_, existing)) = out.iter_mut().find(|(rid, _)| *rid == id) {
            existing.extend(findings);
        } else {
            out.push((id, findings));
        }
    }
    // REQ-0102: deterministic finding order. HashMap iteration is
    // unordered; without an explicit sort consecutive `req validate`
    // runs can list reqs in different orders. Sort by id ascending so
    // tooling and diffs are stable.
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// REQ-0137: validate the functional-safety artifacts. The integrity of
/// the SIL derivation chain is enforced here — a SIL 3/4 safety
/// requirement cannot stay Verified on inspection-only evidence without
/// an audited exception, an assessed hazard must carry its full risk
/// profile, and a mitigated hazard must actually have a live mitigation.
pub fn validate_safety(p: &Project) -> Vec<(String, Vec<Finding>)> {
    let mut out: Vec<(String, Vec<Finding>)> = Vec::new();
    let mut push = |id: &str, f: Finding| {
        if let Some((_, v)) = out.iter_mut().find(|(rid, _)| rid == id) {
            v.push(f);
        } else {
            out.push((id.to_string(), vec![f]));
        }
    };

    // ---- hazards ----
    for (id, h) in &p.hazards {
        if matches!(h.status, HazardStatus::Obsolete) {
            continue;
        }
        if h.harm.trim().is_empty() {
            push(
                id,
                Finding::err(
                    "REQ-V-0025",
                    "harm",
                    "hazard has no harm narrative — describe the potential harm in plain words (e.g. \"an operator's hand could be severed\")",
                ),
            );
        }
        let assessed_or_beyond = !matches!(h.status, HazardStatus::Identified);
        if assessed_or_beyond && !h.is_assessed() {
            push(
                id,
                Finding::err(
                    "REQ-V-0026",
                    "risk",
                    format!(
                        "{} is {} but has not been fully risk-assessed — set all four C/F/P/W parameters via `req hazard assess {}`",
                        id,
                        h.status.as_str(),
                        id
                    ),
                ),
            );
        }
        let mitigated_or_beyond =
            matches!(h.status, HazardStatus::Mitigated | HazardStatus::Verified);
        if mitigated_or_beyond {
            let has_live_mitigation = p.safety_functions.values().any(|sf| {
                !matches!(sf.status, SafetyFunctionStatus::Obsolete)
                    && sf
                        .links
                        .iter()
                        .any(|l| l.kind == LinkKind::Mitigates && l.target == *id)
            });
            if !has_live_mitigation {
                push(
                    id,
                    Finding::err(
                        "REQ-V-0027",
                        "links",
                        format!(
                            "{} is {} but no live safety function mitigates it — add one with `req sf add --mitigates {}` or step the status back",
                            id,
                            h.status.as_str(),
                            id
                        ),
                    ),
                );
            }
        }
    }

    // ---- safety functions ----
    for (id, sf) in &p.safety_functions {
        if matches!(sf.status, SafetyFunctionStatus::Obsolete) {
            continue;
        }
        for l in &sf.links {
            if l.kind == LinkKind::Mitigates && !p.hazards.contains_key(&l.target) {
                push(
                    id,
                    Finding::err(
                        "REQ-V-0028",
                        "links",
                        format!("mitigates link target {} does not exist", l.target),
                    ),
                );
            }
        }
        let allocated_or_beyond = !matches!(sf.status, SafetyFunctionStatus::Proposed);
        let mitigates_any = sf.links.iter().any(|l| l.kind == LinkKind::Mitigates);
        if allocated_or_beyond && !mitigates_any {
            push(
                id,
                Finding::err(
                    "REQ-V-0029",
                    "links",
                    format!(
                        "{} is {} but mitigates no hazard — link it with `req sf mitigate {} HAZ-NNNN`",
                        id,
                        sf.status.as_str(),
                        id
                    ),
                ),
            );
        }
    }

    // ---- safety requirements ----
    for (id, sr) in &p.safety_requirements {
        if matches!(sr.status, Status::Obsolete) {
            continue;
        }
        // Reuse the requirement statement-quality rules via a shim so a
        // safety requirement is held to the same bar (modal verb, weasel
        // words, acceptance criteria) as an ordinary one.
        let shim = Requirement {
            id: sr.id.clone(),
            title: sr.title.clone(),
            statement: sr.statement.clone(),
            rationale: sr.rationale.clone(),
            acceptance: sr.acceptance.clone(),
            kind: Kind::Functional,
            priority: sr.priority,
            status: sr.status,
            tags: Vec::new(),
            links: Vec::new(),
            created: sr.created,
            updated: sr.updated,
            history: Vec::new(),
            tests: sr.tests.clone(),
        };
        for f in validate_requirement(&shim) {
            push(id, f);
        }
        for l in &sr.links {
            if l.kind == LinkKind::Realizes && !p.safety_functions.contains_key(&l.target) {
                push(
                    id,
                    Finding::err(
                        "REQ-V-0028",
                        "links",
                        format!("realizes link target {} does not exist", l.target),
                    ),
                );
            }
        }
        if matches!(sr.status, Status::Verified) {
            let last_pass = sr
                .tests
                .iter()
                .rev()
                .find(|t| matches!(t.outcome, crate::model::TestOutcome::Pass));
            match last_pass {
                None => push(
                    id,
                    Finding::err(
                        "REQ-V-0030",
                        "tests",
                        format!(
                            "{} is Verified but has no passing evidence — record it with `req sreq verify {} --by automated ...`",
                            id, id
                        ),
                    ),
                ),
                Some(t) => {
                    let sil = p.inherited_sil(sr);
                    let needs_strong = sil.map(|s| s.rank() >= Sil::Sil3.rank()).unwrap_or(false);
                    if needs_strong && matches!(t.kind, EvidenceKind::Inspection) {
                        let audited = t.notes.contains("[SIL-gate exception]");
                        if audited {
                            push(
                                id,
                                Finding::warn(
                                    "REQ-V-0031",
                                    "tests",
                                    format!(
                                        "{} inherits {} and is verified by an audited inspection exception — confirm the justification holds",
                                        id,
                                        sil.map(|s| s.as_str()).unwrap_or("SIL3+")
                                    ),
                                ),
                            );
                        } else {
                            push(
                                id,
                                Finding::err(
                                    "REQ-V-0031",
                                    "tests",
                                    format!(
                                        "{} inherits {} but its verification is inspection-only — SIL 3/4 needs automated or composition evidence (re-verify, or use --force to record an audited exception)",
                                        id,
                                        sil.map(|s| s.as_str()).unwrap_or("SIL3+")
                                    ),
                                ),
                            );
                        }
                    }
                }
            }
        }
    }

    out
}

/// Invoke the configured LLM hook and parse `{ok, message}` from
/// stdout. The command is run via the platform shell so users can
/// configure pipelines (`my-llm | jq .`) without us baking in a tool.
/// Returns Ok((ok, message)) on parse success, Err on transport or
/// parse failure.
fn run_llm_hook(cmd: &str, payload: &str) -> Result<(bool, String), String> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    // REQ-0096: honour an explicit shell override so Windows users
    // with Git Bash or WSL on PATH can run sh-script hooks without
    // cmd.exe's quirks. Default is the platform shell. Common values:
    //   sh   bash   pwsh   powershell   cmd
    let (shell, flag): (String, String) = match std::env::var("REQ_VALIDATE_LLM_SHELL") {
        Ok(s) if !s.trim().is_empty() => {
            let s = s.trim().to_string();
            let f = if s.eq_ignore_ascii_case("cmd") {
                "/C".to_string()
            } else if s.eq_ignore_ascii_case("powershell") || s.eq_ignore_ascii_case("pwsh") {
                "-Command".to_string()
            } else {
                "-c".to_string()
            };
            (s, f)
        }
        _ => {
            if cfg!(windows) {
                ("cmd".to_string(), "/C".to_string())
            } else {
                ("sh".to_string(), "-c".to_string())
            }
        }
    };
    let mut child = Command::new(&shell)
        .args([&flag, cmd])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            format!(
                "spawn `{}` (override with REQ_VALIDATE_LLM_SHELL): {}",
                shell, e
            )
        })?;
    {
        // Take ownership of stdin so it is dropped (and the pipe is
        // closed) at the end of this block. Without the close, a hook
        // that does `read_to_end` on stdin hangs until the 10s timeout
        // even though the payload arrived in the first millisecond.
        let mut stdin = child.stdin.take().ok_or("stdin unavailable")?;
        stdin
            .write_all(payload.as_bytes())
            .map_err(|e| format!("write: {}", e))?;
        // Explicit drop here makes the close intent unmissable.
        drop(stdin);
    }
    // Hard ten-second cap. A hung hook should never lock validate.
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    return Err("timed out after 10s".to_string());
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(format!("wait: {}", e)),
        }
    }
    let out = child
        .wait_with_output()
        .map_err(|e| format!("wait: {}", e))?;
    if !out.status.success() {
        return Err(format!(
            "hook exited non-zero: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let body = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value =
        serde_json::from_str(body.trim()).map_err(|e| format!("parse json: {}", e))?;
    let ok = v["ok"].as_bool().ok_or("missing 'ok' boolean")?;
    let message = v["message"].as_str().unwrap_or("").to_string();
    Ok((ok, message))
}

/// Find every distinct cycle in the same-kind link graph. Returns each
/// cycle as a list of IDs in walk order, with the smallest ID first to
/// canonicalize representations across multiple discovery paths.
fn find_cycles(p: &Project, kind: crate::model::LinkKind) -> Vec<Vec<String>> {
    use std::collections::BTreeSet;
    let mut seen: BTreeSet<Vec<String>> = BTreeSet::new();
    for start in p.requirements.keys() {
        let mut current = start.clone();
        let mut path: Vec<String> = Vec::new();
        loop {
            if let Some(pos) = path.iter().position(|x| x == &current) {
                let cycle = path[pos..].to_vec();
                let mut canonical = cycle.clone();
                // Rotate so the smallest ID leads, for stable dedup.
                if let Some(min_pos) = canonical
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, v)| (*v).clone())
                    .map(|(i, _)| i)
                {
                    canonical.rotate_left(min_pos);
                }
                seen.insert(canonical);
                break;
            }
            path.push(current.clone());
            let next = p.requirements.get(&current).and_then(|r| {
                r.links
                    .iter()
                    .find(|l| l.kind == kind)
                    .map(|l| l.target.clone())
            });
            match next {
                Some(n) if p.requirements.contains_key(&n) => current = n,
                _ => break,
            }
        }
    }
    seen.into_iter().collect()
}

pub fn errors_only(findings: &[Finding]) -> Vec<&Finding> {
    findings.iter().filter(|f| f.error).collect()
}
