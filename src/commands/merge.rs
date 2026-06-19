// REQ-0207: `req merge` — a semantic three-way merge driver for project.req.
//
// Wired into git via `.gitattributes` (`*.req merge=req-merge`) and a
// `merge.req-merge.driver` config of:
//
//     req merge --base %O --ours %A --theirs %B
//
// git substitutes %O/%A/%B with temp-file paths for the common ancestor,
// our version, and their version; the driver must leave the result in %A and
// exit 0 on a clean merge or non-zero to signal a conflict. This replaces the
// previous misconfigured driver (`req renumber --base %O || true`), which
// passed git's %O *temp-file path* where `renumber` expected a git *ref* and
// then masked the resulting failure with `|| true` — so git silently kept one
// side wholesale and could drop an entire branch's spec data on merge.
//
// SR-0010: the safety-critical contract is that this driver NEVER silently
// chooses a side. Non-conflicting changes from both sides are auto-merged; any
// unresolvable divergence preserves BOTH sides and exits non-zero so the merge
// surfaces as a git conflict for human resolution.

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::cli::MergeArgs;
use crate::model::{Hazard, Link, Project, Requirement, SafetyFunction, SafetyRequirement};
use crate::storage::{load_with_options, to_canonical_json};

/// Top-level keys merged by a rule other than plain three-way value identity.
/// The per-family ID counters only ever grow, so the safe reconciliation of
/// two divergent counters is their maximum (never a conflict). `updated` takes
/// the later timestamp; `created` the earlier.
const COUNTER_KEYS: &[&str] = &["next_id", "next_haz_id", "next_sf_id", "next_sr_id"];

pub fn run(args: MergeArgs) -> Result<()> {
    // The three inputs are git-managed blobs (or, in tests, real files). Their
    // integrity hashes are valid, but we force-load so a hand-resolved %A that
    // is mid-conflict still parses if it happens to be passed in.
    let base = load_with_options(&args.base, true)
        .with_context(|| format!("load merge base {}", args.base.display()))?;
    let ours = load_with_options(&args.ours, true)
        .with_context(|| format!("load our version {}", args.ours.display()))?;
    let mut theirs = load_with_options(&args.theirs, true)
        .with_context(|| format!("load their version {}", args.theirs.display()))?;

    // SR-0010: an ID present on both sides but absent at the fork point is two
    // DIFFERENT artifacts that collided on the same auto-assigned ID — not an
    // incompatible edit of one artifact. Reconcile by renumbering their side
    // (keeping BOTH; never dropping one) before the value merge, exactly as the
    // post-merge `req renumber` did, so common "both branches added a
    // requirement" merges resolve cleanly instead of conflicting.
    let renamed = deconflict_added_ids(&base, &ours, &mut theirs);

    let bv = serde_json::to_value(&base).context("serialize base")?;
    let ov = serde_json::to_value(&ours).context("serialize ours")?;
    let tv = serde_json::to_value(&theirs).context("serialize theirs")?;

    // Resolve every conflict toward OURS for one tree and toward THEIRS for
    // the other; the two trees are identical except at conflicting points.
    // Conflicts are recorded only on the first (ours) pass to avoid double
    // counting.
    let mut conflicts: Vec<String> = Vec::new();
    let merged_ours = merge3(
        Some(&bv),
        Some(&ov),
        Some(&tv),
        "",
        Side::Ours,
        &mut conflicts,
        true,
    );
    let mut sink: Vec<String> = Vec::new();
    let merged_theirs = merge3(
        Some(&bv),
        Some(&ov),
        Some(&tv),
        "",
        Side::Theirs,
        &mut sink,
        false,
    );

    let output = args.output.clone().unwrap_or_else(|| args.ours.clone());

    if conflicts.is_empty() {
        // REQ-0207 / SR-0010: clean merge — every artifact from either side is preserved
        // (additions/edits to distinct items, deletions relative to the base).
        let project: Project = serde_json::from_value(
            merged_ours.ok_or_else(|| anyhow!("merge produced no document"))?,
        )
        .context("deserialize merged project")?;
        crate::storage::save(&output, &project).context("write merged project")?;
        let renamed_note = if renamed > 0 {
            format!(
                " ({} colliding addition(s) on their side renumbered)",
                renamed
            )
        } else {
            String::new()
        };
        eprintln!(
            "req merge: clean 3-way merge — {} requirement(s), {} hazard(s), {} safety function(s), {} safety requirement(s){}.",
            project.requirements.len(),
            project.hazards.len(),
            project.safety_functions.len(),
            project.safety_requirements.len(),
            renamed_note
        );
        return Ok(());
    }

    // SR-0010: unresolvable divergence — do NOT pick a side. Write both
    // fully-auto-merged trees wrapped in standard conflict markers (they differ
    // only at the conflicting items, so a diff/merge tool highlights exactly
    // what needs a human) and exit non-zero so git records the conflict.
    let ours_doc = render_side(merged_ours, "ours")?;
    let theirs_doc = render_side(merged_theirs, "theirs")?;
    let body = format!(
        "<<<<<<< ours (auto-merged; {} conflict(s) below need manual resolution)\n\
         {}\n\
         =======\n\
         {}\n\
         >>>>>>> theirs\n",
        conflicts.len(),
        ours_doc.trim_end(),
        theirs_doc.trim_end(),
    );
    std::fs::write(&output, body).with_context(|| format!("write {}", output.display()))?;

    eprintln!(
        "req merge: {} unresolvable conflict(s) — left for human resolution (no side dropped):",
        conflicts.len()
    );
    for c in &conflicts {
        eprintln!("  CONFLICT: {}", c);
    }
    eprintln!(
        "Resolve {}: keep one block, hand-merge the conflicting item(s), delete the\n\
         conflict markers, then `git add` it. (Run `req repair --confirm-direct-edit`\n\
         if you hand-edit and the integrity hash no longer matches.)",
        output.display()
    );
    // Non-zero exit: git marks the path conflicted and keeps what we wrote.
    std::process::exit(1);
}

fn render_side(v: Option<Value>, which: &str) -> Result<String> {
    let v = v.ok_or_else(|| anyhow!("merge produced no {} document", which))?;
    let project: Project = serde_json::from_value(v)
        .with_context(|| format!("deserialize {} side of merge", which))?;
    to_canonical_json(&project)
}

#[derive(Clone, Copy, PartialEq)]
enum Side {
    Ours,
    Theirs,
}

/// Recursive three-way merge of one node. `None` means the node is absent on
/// that side (added or deleted relative to the base). Returns `None` when the
/// merged result is "deleted". Conflicts at this node and below are appended to
/// `conflicts` (as human-readable paths) only when `record` is true, and are
/// resolved toward `prefer`.
///
/// SR-0010: the resolution rules below never lose data without surfacing it —
/// the only cases that pick a side silently are ones where the other side made
/// no change relative to the base.
fn merge3(
    base: Option<&Value>,
    ours: Option<&Value>,
    theirs: Option<&Value>,
    path: &str,
    prefer: Side,
    conflicts: &mut Vec<String>,
    record: bool,
) -> Option<Value> {
    // Both sides agree (including "both deleted") — nothing to reconcile.
    if ours == theirs {
        return ours.cloned();
    }
    // One side is unchanged from the base — take the side that changed.
    if ours == base {
        return theirs.cloned();
    }
    if theirs == base {
        return ours.cloned();
    }

    // Both sides changed, and differently. If both are objects we can descend
    // and merge field-by-field (or, for the artifact maps, id-by-id). Anything
    // else — a leaf value, an array, a type mismatch, or an edit-vs-delete — is
    // an unresolvable conflict at this node.
    match (ours, theirs) {
        (Some(Value::Object(o)), Some(Value::Object(t))) => {
            let bo = base.and_then(|v| v.as_object());
            Some(merge_object(bo, o, t, path, prefer, conflicts, record))
        }
        _ => {
            if record {
                conflicts.push(describe(path, ours, theirs));
            }
            match prefer {
                Side::Ours => ours.cloned(),
                Side::Theirs => theirs.cloned(),
            }
        }
    }
}

fn merge_object(
    base: Option<&Map<String, Value>>,
    ours: &Map<String, Value>,
    theirs: &Map<String, Value>,
    path: &str,
    prefer: Side,
    conflicts: &mut Vec<String>,
    record: bool,
) -> Value {
    let mut keys: BTreeSet<&String> = BTreeSet::new();
    keys.extend(ours.keys());
    keys.extend(theirs.keys());
    if let Some(b) = base {
        keys.extend(b.keys());
    }

    // REQ-0207 / SR-0010: the keyed object merge — every artifact id / field present on
    // either side is carried through unless deleted relative to the base, so no
    // side is dropped.
    let mut out = Map::new();
    let at_root = path.is_empty();
    for k in keys {
        // Monotonic / order-by rules — counters and timestamps reconcile
        // arithmetically rather than conflicting. Counters live only at the
        // project root; `created`/`updated` timestamps appear at every level
        // (project and each artifact) and merge the same way anywhere.
        if at_root && COUNTER_KEYS.contains(&k.as_str()) {
            out.insert(k.clone(), max_counter(base, ours, theirs, k));
            continue;
        }
        if k == "updated" {
            out.insert(k.clone(), pick_extreme(ours, theirs, k, true));
            continue;
        }
        if k == "created" {
            out.insert(k.clone(), pick_extreme(ours, theirs, k, false));
            continue;
        }
        // SR-0003: history is an append-only audit trail. Two sides that each
        // appended to the same artifact's history have not made conflicting
        // edits — the correct merge is the union of entries (the shared base
        // prefix is common to both), not a conflict.
        if k == "history" {
            if let Some(v) = merge_history(base.and_then(|b| b.get(k)), ours.get(k), theirs.get(k))
            {
                out.insert(k.clone(), v);
                continue;
            }
        }

        let child = if path.is_empty() {
            k.clone()
        } else {
            format!("{}/{}", path, k)
        };
        let merged = merge3(
            base.and_then(|b| b.get(k)),
            ours.get(k),
            theirs.get(k),
            &child,
            prefer,
            conflicts,
            record,
        );
        if let Some(v) = merged {
            out.insert(k.clone(), v);
        }
        // `None` => the key is deleted in the merged result; omit it.
    }
    Value::Object(out)
}

/// Counters only grow, so the safe merge of two divergent counters is their
/// maximum. An absent counter defaults to 1 (its serialised default).
fn max_counter(
    base: Option<&Map<String, Value>>,
    ours: &Map<String, Value>,
    theirs: &Map<String, Value>,
    key: &str,
) -> Value {
    let get = |m: Option<&Map<String, Value>>| {
        m.and_then(|m| m.get(key))
            .and_then(|v| v.as_u64())
            .unwrap_or(1)
    };
    let n = get(Some(ours)).max(get(Some(theirs))).max(get(base));
    Value::Number(n.into())
}

/// `updated` takes the later timestamp; `created` the earlier. ISO-8601
/// timestamps sort lexically, so string comparison is order-correct.
fn pick_extreme(
    ours: &Map<String, Value>,
    theirs: &Map<String, Value>,
    key: &str,
    latest: bool,
) -> Value {
    let o = ours.get(key).cloned().unwrap_or(Value::Null);
    let t = theirs.get(key).cloned().unwrap_or(Value::Null);
    let (os, ts) = (o.as_str().unwrap_or(""), t.as_str().unwrap_or(""));
    let keep_ours = if latest { os >= ts } else { os <= ts };
    if keep_ours {
        o
    } else {
        t
    }
}

/// Union-merge an append-only `history` array: every entry present on either
/// side (the shared base prefix is common to both), de-duplicated by value and
/// ordered by the entry's `at` timestamp. Returns `None` if either side isn't
/// an array, so the caller falls back to the generic three-way rule.
// REQ-0207 / SR-0010: unioning (rather than conflicting on) two appended
// histories is part of the no-silent-loss merge contract — neither side's
// reasoned-history entries are dropped.
fn merge_history(
    base: Option<&Value>,
    ours: Option<&Value>,
    theirs: Option<&Value>,
) -> Option<Value> {
    let o = ours.and_then(|v| v.as_array())?;
    let t = theirs.and_then(|v| v.as_array())?;
    let _ = base; // base entries are a prefix of both sides, so already covered
    let mut out: Vec<Value> = Vec::new();
    for e in o.iter().chain(t.iter()) {
        if !out.contains(e) {
            out.push(e.clone());
        }
    }
    let at_of = |v: &Value| {
        v.get("at")
            .and_then(|a| a.as_str())
            .unwrap_or("")
            .to_string()
    };
    out.sort_by_key(&at_of);
    Some(Value::Array(out))
}

fn describe(path: &str, ours: Option<&Value>, theirs: Option<&Value>) -> String {
    let kind = match (ours, theirs) {
        (None, Some(_)) => "deleted on our side, changed on theirs",
        (Some(_), None) => "changed on our side, deleted on theirs",
        _ => "changed differently on both sides",
    };
    let at = if path.is_empty() { "<root>" } else { path };
    format!("{} — {}", at, kind)
}

// --- ID-collision reconciliation (REQ-0207 / SR-0010) --------------------
//
// The minimal surface any artifact family needs to be renumbered: a collision
// key (created + title), an ID to rewrite, link targets to follow, and a
// history trail to annotate. Mirrors the trait used by `req renumber`; kept
// local so the merge driver does not perturb that command's source anchor.

trait MergeArtifact {
    fn created(&self) -> DateTime<Utc>;
    fn title(&self) -> &str;
    fn set_id(&mut self, id: String);
    fn note_renamed(&mut self, old: &str);
    fn links_mut(&mut self) -> &mut Vec<Link>;
}

macro_rules! impl_merge_artifact {
    ($t:ty) => {
        impl MergeArtifact for $t {
            fn created(&self) -> DateTime<Utc> {
                self.created
            }
            fn title(&self) -> &str {
                &self.title
            }
            fn set_id(&mut self, id: String) {
                self.id = id;
            }
            fn note_renamed(&mut self, old: &str) {
                self.history.push(super::history(
                    format!(
                        "renumbered from {} (merge collision with the other side)",
                        old
                    ),
                    None,
                ));
            }
            fn links_mut(&mut self) -> &mut Vec<Link> {
                &mut self.links
            }
        }
    };
}
impl_merge_artifact!(Requirement);
impl_merge_artifact!(Hazard);
impl_merge_artifact!(SafetyFunction);
impl_merge_artifact!(SafetyRequirement);

/// Renumber every artifact on THEIRS that collides with OURS on an ID neither
/// inherited from the base (i.e. both sides independently created a different
/// artifact at the same auto-assigned ID). Both artifacts are preserved — only
/// their side's ID changes — and their inbound links are rewritten. Returns the
/// number of artifacts renumbered.
fn deconflict_added_ids(base: &Project, ours: &Project, theirs: &mut Project) -> usize {
    let mut next_req = base.next_id.max(ours.next_id).max(theirs.next_id);
    let mut next_haz = base
        .next_haz_id
        .max(ours.next_haz_id)
        .max(theirs.next_haz_id);
    let mut next_sf = base.next_sf_id.max(ours.next_sf_id).max(theirs.next_sf_id);
    let mut next_sr = base.next_sr_id.max(ours.next_sr_id).max(theirs.next_sr_id);

    // REQ-0207 / SR-0010: renumber collisions on THEIRS (keeping both artifacts), drawing
    // fresh ids from the max of all three sides' counters.
    let mut renames: Vec<(String, String)> = Vec::new();
    plan_collisions(
        &base.requirements,
        &ours.requirements,
        &theirs.requirements,
        "REQ",
        &mut next_req,
        &mut renames,
    );
    plan_collisions(
        &base.hazards,
        &ours.hazards,
        &theirs.hazards,
        "HAZ",
        &mut next_haz,
        &mut renames,
    );
    plan_collisions(
        &base.safety_functions,
        &ours.safety_functions,
        &theirs.safety_functions,
        "SF",
        &mut next_sf,
        &mut renames,
    );
    plan_collisions(
        &base.safety_requirements,
        &ours.safety_requirements,
        &theirs.safety_requirements,
        "SR",
        &mut next_sr,
        &mut renames,
    );

    if renames.is_empty() {
        return 0;
    }

    rekey(&mut theirs.requirements, &renames);
    rekey(&mut theirs.hazards, &renames);
    rekey(&mut theirs.safety_functions, &renames);
    rekey(&mut theirs.safety_requirements, &renames);

    let map: HashMap<String, String> = renames.iter().cloned().collect();
    rewrite_links(theirs.requirements.values_mut(), &map);
    rewrite_links(theirs.hazards.values_mut(), &map);
    rewrite_links(theirs.safety_functions.values_mut(), &map);
    rewrite_links(theirs.safety_requirements.values_mut(), &map);

    theirs.next_id = next_req;
    theirs.next_haz_id = next_haz;
    theirs.next_sf_id = next_sf;
    theirs.next_sr_id = next_sr;
    renames.len()
}

fn plan_collisions<T: MergeArtifact>(
    base: &BTreeMap<String, T>,
    ours: &BTreeMap<String, T>,
    theirs: &BTreeMap<String, T>,
    prefix: &str,
    next: &mut u32,
    out: &mut Vec<(String, String)>,
) {
    let mut ids: Vec<&String> = theirs.keys().collect();
    ids.sort();
    for id in ids {
        if base.contains_key(id) {
            continue; // existed at the fork point → an edit, handled by the value merge
        }
        let (Some(t), Some(o)) = (theirs.get(id), ours.get(id)) else {
            continue; // only their side added it → no collision
        };
        // REQ-0207 / SR-0010: both sides added something at this ID. If they are the same
        // artifact (identical creation + title) the value merge dedups it; only
        // a genuinely different artifact needs renumbering — and even then both
        // are kept.
        if t.created() != o.created() || t.title() != o.title() {
            let new_id = format!("{}-{:04}", prefix, *next);
            *next += 1;
            out.push((id.clone(), new_id));
        }
    }
}

fn rekey<T: MergeArtifact>(map: &mut BTreeMap<String, T>, renames: &[(String, String)]) {
    for (old, new) in renames {
        if let Some(mut a) = map.remove(old) {
            a.set_id(new.clone());
            a.note_renamed(old);
            map.insert(new.clone(), a);
        }
    }
}

fn rewrite_links<'a, T: MergeArtifact + 'a>(
    artifacts: impl Iterator<Item = &'a mut T>,
    map: &HashMap<String, String>,
) {
    for a in artifacts {
        for link in a.links_mut().iter_mut() {
            if let Some(new) = map.get(&link.target) {
                link.target = new.clone();
            }
        }
    }
}
