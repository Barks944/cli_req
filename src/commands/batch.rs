// Implements REQ-0066: req batch — apply many mutations atomically from a
// JSON document. The whole batch is staged in memory and validated; any
// rejection rolls the entire transaction back (file is byte-identical to
// its pre-batch state). One file write per batch, one history entry per
// affected requirement.
use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use serde::Deserialize;
use serde_json::json;
use std::io::Read;
use std::path::PathBuf;

use crate::cli::BatchArgs;
use crate::model::{Kind, Link, LinkKind, Priority, Requirement, Status};
use crate::storage::{self, load_for_mutation};
use crate::validate;

#[derive(Deserialize)]
struct BatchDoc {
    #[serde(default)]
    reason: Option<String>,
    mutations: Vec<Mutation>,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum Mutation {
    Add {
        title: String,
        statement: String,
        rationale: String,
        #[serde(default)]
        req_kind: Option<String>,
        #[serde(default)]
        priority: Option<String>,
        #[serde(default)]
        acceptance: Vec<String>,
        #[serde(default)]
        tags: Vec<String>,
        #[serde(default)]
        parent: Option<String>,
        #[serde(default)]
        reason: Option<String>,
    },
    Update {
        id: String,
        #[serde(default)]
        title: Option<String>,
        #[serde(default)]
        statement: Option<String>,
        #[serde(default)]
        rationale: Option<String>,
        #[serde(default)]
        acceptance: Option<Vec<String>>,
        #[serde(default)]
        add_acceptance: Vec<String>,
        #[serde(default)]
        req_kind: Option<String>,
        #[serde(default)]
        priority: Option<String>,
        #[serde(default)]
        status: Option<String>,
        #[serde(default)]
        add_tag: Vec<String>,
        #[serde(default)]
        remove_tag: Vec<String>,
        #[serde(default)]
        reason: Option<String>,
        /// Skip the same lifecycle guard as `req update --force`.
        #[serde(default)]
        force: bool,
    },
    Delete {
        id: String,
        #[serde(default)]
        hard: bool,
        #[serde(default)]
        reason: Option<String>,
    },
    Link {
        from: String,
        to: String,
        #[serde(default = "default_link_kind")]
        link_kind: String,
        #[serde(default)]
        remove: bool,
        #[serde(default)]
        reason: Option<String>,
    },
    // REQ-0066: verify mutation mirrors `req verify`. Useful for the
    // adoption case (a backfill of 30+ reqs that all need an evidence
    // record + promotion); doing it through 30 shell invocations is
    // wasteful. Batch keeps atomic rollback so any one failure
    // unwinds the whole sequence.
    Verify {
        id: String,
        by: String, // "composition" | "inspection"
        notes: String,
        #[serde(default)]
        cites: Vec<String>,
        #[serde(default)]
        promote: bool,
        #[serde(default)]
        force: bool,
    },
}

fn default_link_kind() -> String {
    "parent".into()
}

pub fn run(args: BatchArgs, file: &Option<PathBuf>) -> Result<()> {
    let raw = if args.source == "-" {
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf)?;
        buf
    } else {
        std::fs::read_to_string(&args.source)
            .with_context(|| format!("read batch source {}", args.source))?
    };
    let doc: BatchDoc = serde_json::from_str(&raw).map_err(|e| {
        anyhow!(
            "batch document is not valid JSON ({}). See `req schema batch` for the expected shape.",
            e
        )
    })?;

    let (path, mut project, _lock) = load_for_mutation(file)?;
    let project_snapshot = serde_json::to_string(&project)?; // for rollback

    let now = Utc::now();
    let mut applied: Vec<serde_json::Value> = Vec::new();

    for (idx, m) in doc.mutations.iter().enumerate() {
        let result = apply_one(&mut project, m, &doc.reason, now);
        match result {
            Ok(summary) => applied.push(summary),
            Err(e) => {
                // Roll back the in-memory project; don't touch disk.
                // The reassignment is for clarity even though we return
                // immediately afterwards — keeps the snapshot semantics
                // explicit if a future change adds work after this point.
                let _rolled_back: crate::model::Project = serde_json::from_str(&project_snapshot)?;
                let envelope = json!({
                    "applied_before_failure": applied,
                    "failed_index": idx,
                    "error": e.to_string(),
                });
                if args.json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&json!({
                            "ok": false, "rolled_back": true, "details": envelope
                        }))?
                    );
                }
                return Err(anyhow!("batch rolled back at mutation #{}: {}", idx, e));
            }
        }
    }

    // Empty batches are a no-op — leave the file byte-identical.
    if !applied.is_empty() {
        project.updated = now;
        storage::save(&path, &project)?;
    }
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "ok": true,
                "applied": applied.len(),
                "results": applied,
            }))?
        );
    } else {
        println!("Applied {} mutation(s) in one batch.", applied.len());
        for a in &applied {
            println!("  {}", a);
        }
    }
    Ok(())
}

fn apply_one(
    project: &mut crate::model::Project,
    m: &Mutation,
    default_reason: &Option<String>,
    now: chrono::DateTime<Utc>,
) -> Result<serde_json::Value> {
    match m {
        Mutation::Add {
            title,
            statement,
            rationale,
            req_kind,
            priority,
            acceptance,
            tags,
            parent,
            reason,
        } => {
            let kind = parse_kind(req_kind.as_deref())?.unwrap_or(Kind::Functional);
            let prio = parse_priority(priority.as_deref())?.unwrap_or(Priority::Should);
            let mut links = Vec::new();
            if let Some(p) = parent {
                if !project.requirements.contains_key(p) {
                    return Err(anyhow!("parent {} does not exist", p));
                }
                links.push(Link {
                    kind: LinkKind::Parent,
                    target: p.clone(),
                });
            }
            let mut req = Requirement {
                id: String::new(),
                title: title.clone(),
                statement: statement.clone(),
                rationale: rationale.clone(),
                acceptance: acceptance.clone(),
                kind,
                priority: prio,
                status: Status::Draft,
                tags: tags.clone(),
                links,
                created: now,
                updated: now,
                history: vec![super::history(
                    "created via batch",
                    reason.clone().or_else(|| default_reason.clone()),
                )],
                tests: Vec::new(),
                // REQ-0139: new requirements start without a validation dossier.
                validation: None,
            };
            let findings = validate::validate_requirement(&req);
            let errs = validate::errors_only(&findings);
            if !errs.is_empty() {
                let msg: Vec<String> = errs
                    .iter()
                    .map(|f| format!("[{}] {}", f.field, f.message))
                    .collect();
                return Err(anyhow!("validation failed: {}", msg.join("; ")));
            }
            let id = project.allocate_id();
            req.id = id.clone();
            project.requirements.insert(id.clone(), req);
            Ok(json!({ "op": "add", "id": id }))
        }
        Mutation::Update {
            id,
            title,
            statement,
            rationale,
            acceptance,
            add_acceptance,
            req_kind,
            priority,
            status,
            add_tag,
            remove_tag,
            reason,
            force,
        } => {
            let r = project
                .requirements
                .get_mut(id)
                .ok_or_else(|| anyhow!("no such requirement: {}", id))?;
            let mut changes = Vec::new();
            if let Some(t) = title.clone() {
                if r.title != t {
                    changes.push("title".into());
                    r.title = t;
                }
            }
            if let Some(s) = statement.clone() {
                if r.statement != s {
                    changes.push("statement".into());
                    r.statement = s;
                }
            }
            if let Some(rt) = rationale.clone() {
                if r.rationale != rt {
                    changes.push("rationale".into());
                    r.rationale = rt;
                }
            }
            if let Some(ac) = acceptance.clone() {
                r.acceptance = ac;
                changes.push("acceptance replaced".into());
            }
            for ac in add_acceptance {
                r.acceptance.push(ac.clone());
                changes.push(format!("+acceptance: {:?}", ac));
            }
            if let Some(k) = parse_kind(req_kind.as_deref())? {
                if r.kind != k {
                    changes.push(format!("kind -> {}", k.as_str()));
                    r.kind = k;
                }
            }
            if let Some(p) = parse_priority(priority.as_deref())? {
                if r.priority != p {
                    changes.push(format!("priority -> {}", p.as_str()));
                    r.priority = p;
                }
            }
            if let Some(st) = parse_status(status.as_deref())? {
                if r.status != st {
                    // Same lifecycle policy as `req update`. Irregular
                    // moves need force=true on the mutation so batch
                    // can't be a back door around the state machine.
                    if !crate::model::is_natural_transition(r.status, st) && !*force {
                        return Err(anyhow!(
                            "{} -> {} is an irregular transition for {} via batch; \
                             pass \"force\": true on this mutation to override.",
                            r.status.as_str(),
                            st.as_str(),
                            id
                        ));
                    }
                    changes.push(format!("status -> {}", st.as_str()));
                    r.status = st;
                }
            }
            for t in add_tag {
                if !r.tags.iter().any(|x| x == t) {
                    r.tags.push(t.clone());
                    changes.push(format!("+tag {}", t));
                }
            }
            for t in remove_tag {
                if let Some(p) = r.tags.iter().position(|x| x == t) {
                    r.tags.remove(p);
                    changes.push(format!("-tag {}", t));
                }
            }
            let findings = validate::validate_requirement(r);
            let errs = validate::errors_only(&findings);
            if !errs.is_empty() {
                let msg: Vec<String> = errs
                    .iter()
                    .map(|f| format!("[{}] {}", f.field, f.message))
                    .collect();
                return Err(anyhow!("validation failed on {}: {}", id, msg.join("; ")));
            }
            r.updated = now;
            r.history.push(super::history(
                changes.join("; "),
                reason.clone().or_else(|| default_reason.clone()),
            ));
            Ok(json!({ "op": "update", "id": id, "changes": changes }))
        }
        Mutation::Delete { id, hard, reason } => {
            if !project.requirements.contains_key(id) {
                return Err(anyhow!("no such requirement: {}", id));
            }
            let inbound: Vec<String> = project
                .requirements
                .values()
                .filter(|r| r.links.iter().any(|l| l.target == *id))
                .map(|r| r.id.clone())
                .collect();
            if *hard {
                if !inbound.is_empty() {
                    return Err(anyhow!(
                        "hard-delete blocked: {} referenced by {}",
                        id,
                        inbound.join(", ")
                    ));
                }
                project.requirements.remove(id);
            } else {
                let r = project.requirements.get_mut(id).unwrap();
                r.status = Status::Obsolete;
                r.updated = now;
                r.history.push(super::history(
                    "marked obsolete via batch",
                    reason.clone().or_else(|| default_reason.clone()),
                ));
            }
            Ok(json!({ "op": "delete", "id": id, "mode": if *hard { "hard" } else { "soft" } }))
        }
        Mutation::Link {
            from,
            to,
            link_kind,
            remove,
            reason,
        } => {
            if from == to {
                return Err(anyhow!("cannot link {} to itself", from));
            }
            if !project.requirements.contains_key(to) {
                return Err(anyhow!("target {} does not exist", to));
            }
            let kind = parse_link_kind(link_kind)?;
            // Cycle check matches `req link` for every asymmetric kind.
            // Without this, batch could install cycles that the direct
            // CLI rejects.
            let cycle_checked = matches!(
                kind,
                LinkKind::Parent | LinkKind::DependsOn | LinkKind::Refines | LinkKind::Verifies
            );
            if cycle_checked && !*remove && creates_cycle(project, from, to, kind) {
                return Err(anyhow!(
                    "linking {} -> {} {} would create a cycle",
                    from,
                    kind.as_str(),
                    to
                ));
            }
            let r = project
                .requirements
                .get_mut(from)
                .ok_or_else(|| anyhow!("source {} does not exist", from))?;
            if *remove {
                let before = r.links.len();
                r.links.retain(|l| !(l.kind == kind && l.target == *to));
                if r.links.len() == before {
                    return Err(anyhow!("no such link {} -> {}", from, to));
                }
            } else {
                if r.links.iter().any(|l| l.kind == kind && l.target == *to) {
                    return Err(anyhow!("link already exists"));
                }
                r.links.push(Link {
                    kind,
                    target: to.clone(),
                });
            }
            r.updated = now;
            r.history.push(super::history(
                format!(
                    "{} {} link to {} via batch",
                    if *remove { "removed" } else { "added" },
                    kind.as_str(),
                    to
                ),
                reason.clone().or_else(|| default_reason.clone()),
            ));
            Ok(
                json!({ "op": "link", "from": from, "to": to, "kind": kind.as_str(), "removed": remove }),
            )
        }
        // REQ-0066: verify mutation. Mirrors `req verify` semantics:
        // record a TestRecord, optionally promote (with the same
        // status-floor guard `req verify --promote` enforces).
        Mutation::Verify {
            id,
            by,
            notes,
            cites,
            promote,
            force,
        } => {
            use crate::model::{EvidenceKind, Status, TestOutcome, TestRecord};
            let kind = match by.as_str() {
                "composition" => EvidenceKind::Composition,
                "inspection" => EvidenceKind::Inspection,
                other => {
                    return Err(anyhow!(
                        "verify mutation: unknown `by`: {} (use composition or inspection)",
                        other
                    ))
                }
            };
            // Capture HEAD SHA so the record pins to a commit, same
            // as `req verify` does.
            let commit = std::process::Command::new("git")
                .args(["rev-parse", "HEAD"])
                .output()
                .ok()
                .and_then(|o| {
                    if o.status.success() {
                        Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
                    } else {
                        None
                    }
                })
                .unwrap_or_else(|| "(no git)".into());
            let cites_prefix = if cites.is_empty() {
                String::new()
            } else {
                format!("cites: {} — ", cites.join(", "))
            };
            let record = TestRecord {
                at: now,
                actor: super::current_actor(),
                commit: commit.clone(),
                outcome: TestOutcome::Pass,
                notes: format!("{}{}", cites_prefix, notes),
                kind,
                content_hash: None,
                linked_files: None,
                sil_gate_exception: false,
            };
            let r = project
                .requirements
                .get_mut(id)
                .ok_or_else(|| anyhow!("no such requirement: {}", id))?;
            r.tests.push(record);
            r.history.push(super::history(
                format!(
                    "{} evidence recorded via batch against commit {}",
                    kind.as_str(),
                    crate::commands::test_cmd::short(&commit)
                ),
                Some(notes.clone()),
            ));
            r.updated = now;
            let mut promoted = false;
            if *promote {
                let eligible = matches!(r.status, Status::Implemented);
                if eligible || *force {
                    if !matches!(r.status, Status::Verified | Status::Obsolete) {
                        r.status = Status::Verified;
                        r.history.push(super::history(
                            format!(
                                "status promoted to verified ({} evidence via batch)",
                                kind.as_str()
                            ),
                            None,
                        ));
                        promoted = true;
                    }
                } else if !matches!(r.status, Status::Verified | Status::Obsolete) {
                    return Err(anyhow!(
                        "verify mutation: {} is at status '{}'; --promote only auto-promotes from \
                         'implemented'. Pass \"force\": true on this mutation, or transition to \
                         implemented first.",
                        id,
                        r.status.as_str()
                    ));
                }
            }
            Ok(json!({
                "op": "verify",
                "id": id,
                "kind": kind.as_str(),
                "commit": commit,
                "promoted": promoted,
            }))
        }
    }
}

/// Same walker as `commands::link::creates_cycle`: walk forward along
/// same-kind links from `target` and check whether the chain reaches
/// `from` (which would close a cycle once the new link is added).
fn creates_cycle(
    project: &crate::model::Project,
    from: &str,
    target: &str,
    kind: LinkKind,
) -> bool {
    let mut current = target.to_string();
    let mut visited = Vec::new();
    loop {
        if current == from {
            return true;
        }
        if visited.contains(&current) {
            return false;
        }
        visited.push(current.clone());
        let next = project.requirements.get(&current).and_then(|r| {
            r.links
                .iter()
                .find(|l| l.kind == kind)
                .map(|l| l.target.clone())
        });
        match next {
            Some(n) => current = n,
            None => return false,
        }
    }
}

fn parse_kind(s: Option<&str>) -> Result<Option<Kind>> {
    Ok(match s {
        Some("functional") => Some(Kind::Functional),
        Some("non-functional") | Some("nonfunctional") => Some(Kind::NonFunctional),
        Some("constraint") => Some(Kind::Constraint),
        Some("interface") => Some(Kind::Interface),
        Some("business") => Some(Kind::Business),
        Some(other) => return Err(anyhow!("unknown kind: {}", other)),
        None => None,
    })
}
fn parse_priority(s: Option<&str>) -> Result<Option<Priority>> {
    Ok(match s {
        Some("must") => Some(Priority::Must),
        Some("should") => Some(Priority::Should),
        Some("could") => Some(Priority::Could),
        Some("wont") => Some(Priority::Wont),
        Some(other) => return Err(anyhow!("unknown priority: {}", other)),
        None => None,
    })
}
fn parse_status(s: Option<&str>) -> Result<Option<Status>> {
    Ok(match s {
        Some("draft") => Some(Status::Draft),
        Some("proposed") => Some(Status::Proposed),
        Some("approved") => Some(Status::Approved),
        Some("implemented") => Some(Status::Implemented),
        Some("verified") => Some(Status::Verified),
        Some("obsolete") => Some(Status::Obsolete),
        Some(other) => return Err(anyhow!("unknown status: {}", other)),
        None => None,
    })
}
fn parse_link_kind(s: &str) -> Result<LinkKind> {
    Ok(match s {
        "parent" => LinkKind::Parent,
        "depends_on" | "depends-on" => LinkKind::DependsOn,
        "conflicts" => LinkKind::Conflicts,
        "refines" => LinkKind::Refines,
        "verifies" => LinkKind::Verifies,
        other => return Err(anyhow!("unknown link kind: {}", other)),
    })
}
