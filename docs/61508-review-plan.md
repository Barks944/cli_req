# Plan: closing the 61508 review gaps in `req`

*Working plan derived from [61508-hazard-sf-review.md](61508-hazard-sf-review.md). Branch
`61508-review-fixes` (off `next-level-features`). Dated 2026-06-15.*

## Framing

The review's central finding (§3B-12) is an **asymmetry of rigour inside the
chain**: the SR node is heavily gated (full dossier + human co-sign + SIL gate +
staleness), but the two nodes it rolls up into — SafetyFunction `Verified` and
Hazard `Verified`/adequacy — are ungated free-text labels an agent can set in one
call. A fully-green trace can therefore rest unbacked SF/Hazard flags on top of a
rigorous SR.

The two highest-leverage fixes (§5) both **extend machinery `req` already has**
rather than opening new 61508 surface:

1. Give SafetyFunctions the dossier + gate treatment SRs already have (verification-flavoured).
2. Add a mitigation-adequacy / residual-risk argument at the Hazard level (validation-flavoured).

The remaining items (§3A) are recorded as a backlog, not in scope for the first pass.

## Existing machinery we reuse (verified against source)

- `Verification` / `VerificationActivity` / `TestOutcome` / `ExemptionKind` — `src/model.rs:493-649`.
  Staged dossier: `plan` → `analysis` → `testing` → `statement` → derived `verdict`,
  plus `human_confirmation`, `content_hash` + `linked_files` (staleness anchor),
  `exempt`/`exemption_kind`.
- SR dossier workflow + gates — `src/commands/verification.rs`:
  `op_plan` (257), `op_activity` (295), `op_conclude` (368), `op_confirm` (774, **blocks
  `REQ_ACTOR_KIND=agent`**), `op_backfill` (564), `gate_safety_requirement` (651),
  `promote_preflight` (524, SIL-rigour + `sil_gate_exception`).
- Validator (note: file is now **`src/conform.rs`**, not `validate.rs` — the
  `validate`→`conform` rename): SR rules REQ-V-0030..0038 (877-1096); SF rules
  REQ-V-0028/0029 (841-875); Hazard rules REQ-V-0025/26/27 (782-839); SIL-escalation
  (1002-1024).
- Persistence — `src/storage.rs` req-v4 JSON, `_integrity` SHA256, `_schema_rev`.
  New optional fields are additive; `#[serde(skip_serializing_if)]` keeps old files
  byte-stable. Bump `SCHEMA_REV` only if a shape change isn't purely additive.
- `req` dogfoods itself in `project.req` (REQ-/SR-/REQ-V- IDs). Every new behaviour
  below must be added via `req_add` and self-verified. **Mutate with
  `./target/debug/req`, not the session MCP server (stale build strips validation).**

---

## Phase 1 — SafetyFunction verification dossier (§3B-1..6, §5-1)

**Goal:** `SF → Verified` is earned through a dossier, not typed. Reuse the SR pattern
almost verbatim. This is *built-it-right* verification: "does this function achieve its
declared `safe_state`?"

### Model (`src/model.rs`)
- Add `verification: Option<Verification>` to `SafetyFunction` (~1335-1355), mirroring
  `SafetyRequirement`. `#[serde(default, skip_serializing_if = "Option::is_none")]`.
- Decide on co-sign: SFs carry an `allocated_sil` (derived). A high allocated SIL should
  require human co-sign just as SRs do. Reuse `human_confirmation` on the same struct.

### Commands (`src/commands/`)
- Generalise the verification dossier surface to accept SF IDs. Cleanest path: extend
  the `Family` enum (`verification.rs:64`) with `Sf`, and teach `resolve` / `ItemMut`
  to expose an SF's `verification`/`status`/`history`. Then `req verification plan|analysis|test|conclude|confirm`
  works for `SF-NNNN` for free.
- Gate the **status-setting path** that is currently a free write:
  - `sf_update` status mutation (`safety.rs:544-583`) — block direct `--status verified`
    (and `implemented`) the way SR promotion is gated; require the dossier to carry it.
  - Add an `op`-style preflight analogous to `promote_preflight` for SFs (status ladder
    Proposed→Allocated→Implemented→Verified; only Implemented may conclude to Verified).
- `Proposed → Allocated` auto-transition stays as-is (`safety.rs:608`, earned by a real
  `mitigates` edge). `Implemented` should also require something behind it (≥1 realizing
  SR or linked code marker) — see validator.

### Validator (`src/conform.rs`, SF block ~841-875)
- **REQ-V-00xx**: an SF at `Verified` must have a genuine concluded-Pass dossier
  (parallel to REQ-V-0033). Reject `exempt`.
- **REQ-V-00xx**: an SF at `Verified` with high allocated SIL must have `human_confirmation`
  (parallel to REQ-V-0034).
- **REQ-V-00xx**: an SF at `Implemented` must have backing (≥1 realizing SR or linked marker).
- **REQ-V-00xx**: SF dossier staleness — if the SF's linked source drifted, Verified→stale
  (parallel REQ-V-0035), closing §3B-13.

### Self-tracking
- New REQs for: SF dossier field, SF promotion gate, SF human co-sign, SF staleness, the
  new validator rules. Verify each through its own dossier (dogfood).

**Why first:** lower risk, arguably more urgent — it's a pattern the tool already trusts,
and it closes the *softest* node in the chain.

---

## Phase 2 — Hazard mitigation-adequacy / residual-risk argument (§3B-7..10, §5-2)

**Goal:** record *why* the mitigations together reduce residual risk to acceptable —
without ever printing "validated". This is *validation-flavoured* and crosses the
"validation out of scope" line (§3A-6), so framing matters: `req` **records and forces
the reasoning**, it does not perform the HARA.

### Model (`src/model.rs`, `Hazard` ~1260-1308)
- Add an `adequacy` record (new small struct or reuse the dossier shape): a required
  free-text **residual-risk argument**, the **author/actor**, timestamp, and a **human
  co-sign** before a hazard may reach `Verified`. Optionally a `credited_external_measures`
  note (§3A-4 / the W-axis-credits-non-SIS-reduction gap).
- Optional rationale fields for the C/F/P/W classification itself (§3B-9) — a `rationale`
  per parameter or one classification-rationale string.

### Commands (`src/commands/safety.rs`)
- New `req hazard adequacy` (or `assess-residual`): records the residual-risk argument;
  a separate human co-sign step to flip `Mitigated → Verified`.
- Keep the computed `adequate = allocated_sil >= required_sil` (`safety.rs:1121`) but
  rename/surface it honestly as **"SIL target met"**, distinct from the recorded
  residual-risk judgement (the review stresses these are not the same — `help_text.rs:1113`).

### Validator (`src/conform.rs`, Hazard block ~782-839)
- **REQ-V-00xx**: a hazard at `Verified` requires a recorded adequacy argument + human co-sign.
- **REQ-V-00xx**: `Mitigated` should mean more than "a live SF exists" (§3B-10) — consider
  requiring the mitigating SF(s) to be at least Allocated with matching SIL coverage.

### Help / disclaimer
- Extend `req help safety` to explain the adequacy record is *recorded reasoning*, not a
  validation claim, mirroring the existing verification framing.

---

## Phase 3 — Cross-cutting, lower-cost (§3B-11, §3A-3)

- **Edge notes (§3B-11):** extend `mitigates` / `realizes` links (`model.rs:676`,
  currently `{kind, target}`) with an optional `note` — *how much* an SF contributes /
  *why* an SR realizes its SF. Additive, touches link construction + display.
- **"Target only — no PFD/arch evidence" stamp (§3A-3 / §5):** a fixed advisory rendered
  on every SF/SR show output (and export) so the achieved-integrity gap travels with the
  artifact, not just the README. Low code, high clarity win.

---

## Backlog (record as REQs, not this pass) — §3A

| Item | Review ref | Shape |
| --- | --- | --- |
| Tool-error-analysis appendix (§7.4.4) | §3A-1, §5 | Doc/artifact, not code-gated |
| Record which risk method used + why | §3A-2 | `Hazard`/project field |
| Achieved-integrity (PFD/PFH, HFT/SFF, DC, SC) | §3A-3 | Large; stays out of scope, but the stamp (Phase 3) makes the boundary visible |
| Hazard fields: safe-state/process-safety-time, credited external measures | §3A-4 | Partly folded into Phase 2 |
| Verification technique-table coverage mapping | §3A-5 | Extends dossier |
| Safety validation as a distinct phase | §3A-6 | New lifecycle surface |
| FSM / safety-plan / competence / FSA | §3A-7 | New artifact type |
| `a`/`b` band downstream handling | §3A-8 | Audit conform rules for SF inheriting `b` |

---

## Sequencing & risk

1. **Phase 1** first — reuses trusted machinery, closes the softest node, lowest risk.
2. **Phase 2** second — new shape + crosses the validation boundary; needs careful framing.
3. **Phase 3** opportunistically — small, independent, high-clarity.
4. Each new behaviour added via `req_add` and self-verified through its own dossier.
5. Watch: integrity hash + `SCHEMA_REV` — keep additions serde-optional; only bump rev
   if a change isn't purely additive. Build the real binary (`cargo build`) and gate with
   the **release** binary, since the session MCP server is a stale build.

## Open questions for the user

- Should SF co-sign be required for **all** Verified SFs, or only above a SIL threshold
  (e.g. ≥ SIL3, mirroring the SR SIL-rigour gate)?
- Phase 2 adequacy: a lightweight single-argument record, or the full plan→analysis→
  conclude→co-sign dossier shape reused from `Verification`?
- Do Phase 1 + Phase 2 land in this one branch, or split per phase for review?
