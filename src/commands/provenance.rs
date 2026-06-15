// REQ-0142 / SR-0004: verification provenance — the *true* status behind a
// Verified item. `Verification::passed()` short-circuits on `exempt`, so a
// backfilled or `--no-dossier` waiver is indistinguishable from a genuine
// concluded dossier in every headline surface. This classifier recovers that
// distinction so an agent (or auditor) can tell real verification from a
// grandfathered exemption in one query.
//
// SR-0004 anchors its verification dossier on THIS file alone, so the safety
// requirement only re-verifies when the provenance classifier itself changes
// — not when unrelated code in the (large) `req verification` surface is edited.
use std::path::Path;

use crate::model::{Project, Status, TestOutcome, Verification};

/// How a *Verified* item's verification stands up to scrutiny. Ordered loosely
/// from weakest to strongest trust.
#[derive(Copy, Clone, PartialEq, Eq)]
pub enum Provenance {
    /// Verified with no verification dossier at all (pre-gate residue, or a
    /// status forced by other means). The weakest possible standing.
    Ungated,
    /// A passing dossier exists but its `exempt` flag is set: an audited
    /// `req verification backfill` (grandfathered) waiver.
    ExemptBackfilled,
    /// A passing dossier exists but its `exempt` flag is set: an audited
    /// `req verify --no-dossier` waiver (ordinary requirements only).
    ExemptNoDossier,
    /// A genuine concluded Pass dossier whose anchored source has since
    /// drifted — the verification no longer stands until re-verified.
    Stale,
    /// REQ-0150: a safety requirement with a genuine, fresh, concluded Pass
    /// dossier that nonetheless lacks the mandatory human confirmation
    /// (REQ-0145 / REQ-V-0034). The agent's work is real, but the co-sign that
    /// makes a safety requirement truly pass is missing, so it is NOT yet
    /// genuine standing. Only ever applies to safety requirements.
    Unconfirmed,
    /// A genuine concluded dossier: real analysis + testing + statement,
    /// Pass verdict, anchor still fresh (or no git context to judge).
    Genuine,
}

impl Provenance {
    pub fn as_str(self) -> &'static str {
        match self {
            Provenance::Ungated => "ungated",
            Provenance::ExemptBackfilled => "exempt:backfilled",
            Provenance::ExemptNoDossier => "exempt:no-dossier",
            Provenance::Stale => "stale",
            // REQ-0150: unconfirmed safety requirement (genuine dossier, no co-sign).
            Provenance::Unconfirmed => "unconfirmed",
            Provenance::Genuine => "genuine",
        }
    }

    /// True only for a genuine, non-stale passing dossier — the bar the
    /// headline "verified" number should really be measuring.
    pub fn is_genuine(self) -> bool {
        matches!(self, Provenance::Genuine)
    }
}

/// Classify a single dossier's provenance. `source_root` is where linked
/// files are hashed to judge staleness; pass `None` to skip the staleness
/// probe (treats a genuine dossier as Genuine regardless of drift).
pub fn classify(v: Option<&Verification>, source_root: Option<&Path>, id: &str) -> Provenance {
    let v = match v {
        None => return Provenance::Ungated,
        Some(v) => v,
    };
    if v.exempt {
        // REQ-0162: read the structured waiver kind. Legacy exempt dossiers
        // written before the field existed (and not yet migrated) carry None;
        // treat those as backfilled — the conservative default — rather than
        // parsing the free-text plan. `req migrate` populates the field.
        return match v.exemption_kind {
            Some(crate::model::ExemptionKind::NoDossier) => Provenance::ExemptNoDossier,
            Some(crate::model::ExemptionKind::Backfilled) => Provenance::ExemptBackfilled,
            None => Provenance::ExemptBackfilled,
        };
    }
    // A non-exempt dossier only counts as genuine if it actually concluded
    // Pass with both activity stages and a statement recorded.
    let genuine = matches!(v.verdict, Some(TestOutcome::Pass))
        && v.analysis.is_some()
        && v.testing.is_some()
        && v.statement.is_some();
    if !genuine {
        return Provenance::Ungated;
    }
    if let (Some(root), Some(hash)) = (source_root, v.content_hash.as_deref()) {
        let s = crate::commands::test_cmd::staleness_by_content(
            hash,
            v.linked_files.as_ref(),
            id,
            root,
        );
        if matches!(s, crate::commands::test_cmd::Staleness::Stale { .. }) {
            return Provenance::Stale;
        }
    }
    Provenance::Genuine
}

/// REQ-0187/0188: a safety requirement that has a genuine concluded Pass
/// dossier and sits at Implemented awaiting the human co-sign that will
/// promote it to Verified. This is the clean, committable resting state that
/// replaced the old "Verified-but-unconfirmed" hard error.
pub fn sr_awaiting_cosign(sr: &crate::model::SafetyRequirement) -> bool {
    matches!(sr.status, Status::Implemented)
        && classify(sr.verification.as_ref(), None, &sr.id) == Provenance::Genuine
        && sr
            .verification
            .as_ref()
            .and_then(|v| v.human_confirmation.as_ref())
            .is_none()
}

/// REQ-0188: a one-word standing for any safety requirement, regardless of
/// status, so the report can enumerate every SR and none is hidden by a
/// status filter or silently read as done.
pub fn sr_standing(
    sr: &crate::model::SafetyRequirement,
    source_root: Option<&std::path::Path>,
) -> &'static str {
    if matches!(sr.status, Status::Obsolete) {
        return "obsolete";
    }
    if sr_awaiting_cosign(sr) {
        return "awaiting-cosign";
    }
    if matches!(sr.status, Status::Verified) {
        return match classify(sr.verification.as_ref(), source_root, &sr.id) {
            Provenance::Genuine => {
                if sr
                    .verification
                    .as_ref()
                    .and_then(|v| v.human_confirmation.as_ref())
                    .is_some()
                {
                    "verified"
                } else {
                    "unconfirmed"
                }
            }
            Provenance::Stale => "stale",
            _ => "ungated",
        };
    }
    // Pre-conclusion: report how far the dossier got.
    match sr.verification.as_ref() {
        None => "no-dossier",
        Some(v) if v.analysis.is_none() => "plan-only",
        Some(v) if v.testing.is_none() => "analysed-untested",
        Some(_) => "pending-conclusion",
    }
}

/// One row of the provenance report.
pub struct ProvenanceRow {
    pub id: String,
    pub family: &'static str,
    pub provenance: Provenance,
    pub sil: Option<String>,
}

/// REQ-0142: classify every *Verified* requirement and safety requirement.
/// SR-0004: this classification is the tool-confidence control for HAZ-0002 —
/// it makes non-genuine verification (fabricated/shallow dossier, exemption,
/// stale evidence, ungated) visible rather than letting it pass as trustworthy.
/// Rows are sorted by id within family (requirements first, then safety).
pub fn provenance_report(project: &Project, source_root: Option<&Path>) -> Vec<ProvenanceRow> {
    let mut rows = Vec::new();
    let mut reqs: Vec<_> = project
        .requirements
        .values()
        .filter(|r| matches!(r.status, Status::Verified))
        .collect();
    reqs.sort_by(|a, b| a.id.cmp(&b.id));
    for r in reqs {
        rows.push(ProvenanceRow {
            id: r.id.clone(),
            family: "requirement",
            provenance: classify(r.verification.as_ref(), source_root, &r.id),
            sil: None,
        });
    }
    let mut srs: Vec<_> = project
        .safety_requirements
        .values()
        .filter(|sr| matches!(sr.status, Status::Verified))
        .collect();
    srs.sort_by(|a, b| a.id.cmp(&b.id));
    for sr in srs {
        // REQ-0150: a safety requirement whose dossier is otherwise genuine
        // but which lacks the mandatory human co-sign (REQ-0145) is NOT genuine
        // standing — it is exactly what REQ-V-0034 blocks. Surface it as
        // `unconfirmed` so the report (SR-0004's HAZ-0002 tool-confidence
        // control) does not present an un-co-signed safety requirement as
        // trustworthy. The check is SR-only; ordinary requirements have no
        // human-confirmation step. We do NOT fold this into `classify` because
        // promotion to Verified is gated on `classify().is_genuine()` BEFORE the
        // human confirms (see `gate_safety_requirement`).
        let mut provenance = classify(sr.verification.as_ref(), source_root, &sr.id);
        if provenance == Provenance::Genuine
            && sr
                .verification
                .as_ref()
                .map(|v| v.human_confirmation.is_none())
                .unwrap_or(false)
        {
            provenance = Provenance::Unconfirmed;
        }
        rows.push(ProvenanceRow {
            id: sr.id.clone(),
            family: "safety-requirement",
            provenance,
            sil: project.inherited_sil(sr).map(|s| s.as_str().to_string()),
        });
    }
    rows
}
