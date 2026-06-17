# Dossier double-check report

Independent, skeptical re-checks of individual verification / adequacy dossiers —
one requirement per entry. Each entry confirms (or disputes) the dossier against
the actual source and tests, separate from the dossier's own claims.

Method per entry: read the dossier → independently confirm each acceptance
criterion against the code and run the cited tests → judge whether (a) the
behaviour is genuinely met and (b) the dossier *itself* documents it adequately.

---

## SR-0007 — Gate a safety function's Verified status on a co-signed dossier
*Reviewed 2026-06-17 · status: awaiting human co-sign · inherited SIL3*

**Statement:** "req shall refuse to mark a safety function Verified without a concluded-pass verification dossier that a human has co-signed."

**Behaviour — CONFIRMED (independently).** All three acceptance criteria hold:
1. *Direct `sf update --status verified` refused* — gate in `src/commands/safety.rs:1161`; test `req_0201_direct_sf_verified_is_blocked` ✅ pass.
2. *Human co-sign required (agent refused)* — `src/commands/verification.rs:1034` ("must be done by a human, but REQ_ACTOR_KIND=agent"); test `req_0201_sf_reaches_verified_only_via_dossier_and_human_cosign` ✅ pass.
3. *conform flags REQ-V-0039/0040* — 6 references in `src/conform.rs`; test `req_0201_conform_flags_ungated_verified_sf` ✅ pass.

Tests re-run live this session: `cargo test --test safety_dossier req_0201` → **3 passed, 0 failed**. Provenance is genuine (concluded Pass, composition evidence ×4, anchored at `275daf960`).

**Finding (MINOR) — the dossier narrative is thin re-anchor boilerplate.** The recorded stages read:
- plan: *"Re-verify SR-0007."*
- analysis: *"Code review confirms this SR's behaviour unchanged."*
- testing: *"Full suite green."*
- statement: *"Re-anchored to final source; behaviour unchanged. Awaiting fresh human co-sign."*

None of these cite the actual gate (`sf_update` / `op_confirm` / REQ-V-0039/0040) or name the `req_0201_*` tests. This is residue from the repeated re-anchor cycles (the SR was re-verified several times as shared source changed). The *provenance* is genuine and the *behaviour* is independently confirmed, but the dossier under-documents its own evidence — a reviewer co-signing on the dossier alone would be trusting boilerplate.

**Verdict:** Substantively MET and safe to co-sign on the evidence; **recommend** strengthening the analysis/testing narrative (cite the gate code + name the tests) before co-sign so the evidence travels in the dossier, not just in this external check. Not a blocker.

**Resolution (2026-06-17):** dossier re-opened and re-recorded with substantive content — analysis now cites the three enforcement points (`sf_update`, `op_confirm` agent-refusal, conform REQ-V-0039/0040) and references the source files; testing names the three `req_0201_*` acceptance tests; the statement spells out the gate. Re-concluded Pass, still awaiting human co-sign, `req conform` clean. The minor finding is closed — the evidence now lives in the dossier.

---

## SR-0008 — Gate a hazard's Verified status on a co-signed adequacy argument
*Reviewed 2026-06-17 · status: awaiting human co-sign · inherited SIL3*

**Statement:** "req shall refuse to mark a hazard Verified without a human-co-signed mitigation-adequacy argument."

**Behaviour — CONFIRMED (independently).** All three acceptance criteria hold:
1. *Direct `hazard update --status verified` refused* — `src/commands/safety.rs:704` (directs to the adequacy route).
2. *`hazard confirm` requires a recorded adequacy argument + a human actor* — `src/commands/safety.rs:384` ("co-signing a hazard's adequacy argument must be done by a human"); requires a concluded-Adequate dossier before promoting.
3. *conform flags REQ-V-0043* — 2 references in `src/conform.rs`.

Test re-run live: `cargo test --test safety_dossier req_0202` → `req_0202_hazard_verified_requires_cosigned_adequacy` **1 passed, 0 failed**. Provenance genuine (concluded Pass, composition evidence, anchored).

**Finding (MINOR, same as SR-0007) — thin re-anchor boilerplate**, now **RESOLVED in this pass**: analysis re-recorded to cite `hazard_update` / `hazard_confirm` (agent-refusal) / conform REQ-V-0043 with source refs; testing names the `req_0202` test. Re-concluded Pass, awaiting human co-sign, conform clean.

**Verdict:** MET; dossier strengthened to carry its own evidence. Safe to co-sign.

---

## SR-0009 — Stamp the achieved-integrity boundary on safety views
*Reviewed 2026-06-17 · status: awaiting human co-sign · inherited SIL3*

**Statement:** "req shall display the achieved-integrity boundary notice on every safety-function and safety-requirement view."

**Behaviour — CONFIRMED (independently).** Both acceptance criteria hold:
1. *`req sf show` prints the notice* — `ACHIEVED_INTEGRITY_STAMP` (`src/commands/safety.rs:33`) printed as the `scope:` line at `safety.rs:1124`; live `req sf show SF-0008` emits it.
2. *`req sreq show` prints the notice* — same constant printed at `safety.rs:1465`; live `req sreq show SR-0009` emits it.

Test re-run live: `req_0203_achieved_integrity_stamp_on_sf_and_sr_views` **1 passed**. One shared constant feeds both views, so the two stay consistent.

**Finding (MINOR, same as SR-0007/0008) — thin boilerplate, RESOLVED this pass:** analysis re-recorded to name the `ACHIEVED_INTEGRITY_STAMP` constant + both print sites + the live confirmation; testing names `req_0203`. Re-concluded Pass, awaiting co-sign, conform clean.

**Verdict:** MET; dossier carries its own evidence. Safe to co-sign. *(All three REQ-0204/0205/0206-era safety requirements — SR-0007/0008/0009 — now audited and strengthened.)*

---

## SR-0006 — A consistency check must not present as, and must point to, true V&V status
*Reviewed 2026-06-17 · status: awaiting human co-sign · inherited SIL3*

**Statement:** "A model well-formedness or consistency check shall not present its result as verification or validation status, and shall direct the user to the command that reports the true V&V status of every requirement."

**Behaviour — CONFIRMED (independently).** All three acceptance criteria hold:
1. *Success & failure output disclaims V&V* — `CONFORM_DISCLAIMER` (`src/commands/conform_cmd.rs:166`: "This checks model well-formedness … not verification/validation status") printed on the success path (`:142`) and failure path (`:156`).
2. *Output references the V&V command* — the same disclaimer directs to `req verification status`, and it's also carried in the `--json` `note` (`:125`). Confirmed live: `req conform --json` `note` contains the disclaimer.
3. *That command reports every requirement's standing accurately* — `req verification status` (src/commands/status.rs) enumerates every requirement and safety requirement (cross-checked against REQ-0188/0191 earlier in this branch's history).

Test: `tests/coverage_gap.rs:474` asserts the `--json note` carries the disclaimer. Evidence records include an **automated** run + composition (stronger than the other SRs, which are composition-only).

**Finding (MINOR) — thin boilerplate narrative, RESOLVED this pass** (re-recorded to cite the constant + all three print sites + the test).

**Finding (MINOR, NOT actioned — needs a human decision) — the SR's own statement is compound.** It trips REQ-V-0010 ("shall **not present** … **and shall direct** …") — two obligations in one requirement. The tool's own rule would have it split into two atomic SRs (e.g. "shall not present a well-formedness result as V&V status" + "shall direct the user to the V&V-status command"). I did **not** split it: that changes the requirement itself, which is outside the dossier-improvement remit and is a human call. Recorded here as a recommendation. *(Same compound smell affects SR-0005 and several ordinary reqs — REQ-0187/0188/0189/0191.)*

**Verdict:** MET; dossier strengthened. Safe to co-sign as-is; consider splitting the statement in a follow-up.

---

## SR-0003 — Record append-only reasoned history for safety mutations
*Reviewed 2026-06-17 · status: awaiting human co-sign · inherited SIL3*

**Statement:** "req shall append a reasoned, attributed history entry to every mutation of a hazard, safety function, or safety requirement."

**Behaviour — CONFIRMED (independently).**
- *History appended on every mutation path* — 18 `super::history(...)` push sites across `src/commands/safety.rs` (hazard add/assess/update/adequacy/confirm, sf add/update/mitigate, sreq add/update/realize/verify). `super::history` stamps actor + actor_kind (+ on-behalf-of); irregular changes additionally require `--reason`.
- *Append-only & attributed* — live: `HAZ-0001` carries **9** history entries, each with `actor_kind` (e.g. `created` by Human). The append-only property is the one SF-0003 relies on.

Test re-run live: `req_0011_safety_mutation_records_reasoned_append_only_history` (`tests/safety.rs:792`) asserts each status change **adds** an entry without replacing prior history — **1 passed**.

**Finding (MINOR) — thin boilerplate narrative, RESOLVED this pass** (re-recorded with the 18 push sites, the attribution detail, and the named append-only test).

**Verdict:** MET; dossier carries its own evidence. Safe to co-sign.

---

## SR-0002 — Gate Verified on SIL-adequate evidence
*Reviewed 2026-06-17 · status: awaiting human co-sign · inherited SIL3*

**Statement:** "req shall reject promoting a SIL 3 or SIL 4 safety requirement to Verified on inspection-only evidence unless an audited exception is recorded."

**Behaviour — CONFIRMED (independently).** All three acceptance criteria hold:
1. *SIL3 inspection-only `--promote` without `--force` exits non-zero* — `promote_preflight` (`src/commands/verification.rs:783`): "SIL-rigour gate … needs automated or composition test evidence"; same gate guards `sreq_verify`.
2. *Same at SIL4 / b band* — the gate keys off `sil.rank() >= Sil::Sil3.rank()` (`verification.rs:657,783`), so it covers SIL4 and b.
3. *Audited `--force` exception recorded, doesn't waive the dossier* — `sil_gate_exception=true` written on the TestRecord (`verification.rs:653`); `src/conform.rs` REQ-V-0031 surfaces it as a warning forever (error if not audited); the genuine-dossier requirement (REQ-0143) still applies.

Tests re-run live, all pass: `req_0135_sil_gate_blocks_inspection_and_force_needs_reason` (tests/safety.rs), `req_0139_conclude_promote_respects_sil_gate` (tests/verification.rs), `sr_0002_sil_gate_at_sil4_records_audited_exception` (tests/coverage_gap.rs). This is the **best-tested** of the older SRs — it has a dedicated `sr_0002_*` acceptance test.

**Finding (MINOR) — thin boilerplate narrative, RESOLVED this pass** (re-recorded with the gate location, the `sil_gate_exception` mechanism, conform REQ-V-0031, and the three named tests).

**Verdict:** MET; dossier carries its own evidence. Safe to co-sign.

---

## SR-0001 — Refuse to load a spec whose content fails its integrity hash
*Reviewed 2026-06-17 · status: awaiting human co-sign · inherited SIL3*

**Statement:** "req shall refuse to load a project.req whose canonical payload does not match its stored integrity hash."

**Behaviour — CONFIRMED (independently).**
- *Load-time refusal + repair pointer* — `src/storage.rs` computes the canonical SHA-256 over the payload at load and refuses on mismatch, pointing the user at `req repair --confirm-direct-edit` (`storage.rs:89`).
- *Canonical (whitespace-insensitive)* — the hash is over the canonical payload, so reformatting/whitespace doesn't trip it; only a semantic edit does.

Tests re-run live, all pass: `req_0003_integrity_blocks_load_after_semantic_tamper` (semantic hand-edit refused), `req_0003_integrity_ignores_whitespace_only_change` (whitespace-only still loads), `sr_0001_integrity_tamper_refused_with_repair_pointer` (refusal points at `req repair`). Dedicated `sr_0001_*` acceptance test present.

**Finding (MINOR) — thin boilerplate narrative, RESOLVED this pass** (re-recorded with the load-time check, the `req repair` pointer, the canonical-payload nuance, and the three named tests).

**Verdict:** MET; dossier carries its own evidence. Safe to co-sign.

---

## SR-0004 — Report verification provenance of every Verified requirement
*Reviewed 2026-06-17 · status: **Verified** (human co-signed) · spot-check only — not modified*

**Statement:** "req shall report the verification provenance of every Verified requirement."

**Spot-check (read-only — a Verified item is not re-opened, to preserve its co-sign).**
- Standing: `provenance: genuine`, verdict `PASS`, **human-co-signed by Tom**, anchor fresh.
- Claim CONFIRMED: `src/commands/provenance.rs` `classify` (`:65`), `provenance_report` (`:171`), `sr_standing` (`:121`) classify every Verified item (genuine / exempt:backfilled / exempt:no-dossier / stale / unconfirmed / ungated) and the report emits per-item rows + counts. Tests pass live: `req_0142_report_marks_genuine_dossier`, `sr_0004_provenance_report_classifies_categories`.
- **Dossier quality: already substantive** (unlike the awaiting-cosign SRs). Its analysis names the classifier + categories and its testing names the tests with automated evidence. It escaped the re-anchor boilerplate because its anchored source (`provenance.rs`) was never edited on this branch — a useful contrast that confirms the boilerplate came specifically from the bulk re-anchor cycles, not from genuine verification.

**Caveat (MINOR, pre-existing) — independence:** SR-0004 trips REQ-V-0037 (authored *and* co-signed by the same actor, Tom). IEC 61508 wants independence of assessment; ideally a different competent reviewer co-signs. Same caveat applies to SR-0005. Advisory, not a blocker.

**Verdict:** Verified standing CONFIRMED; dossier genuine and substantive; no change made.

---

## SR-0005 — V&V vocabulary reserved for the evidence workflow and used per the standards
*Reviewed 2026-06-17 · status: **Verified** (human co-signed) · spot-check only — not modified*

**Spot-check (read-only).** Standing: `provenance: genuine`, verdict `PASS`, **co-signed by Tom**.
- Claim CONFIRMED: `src/cli.rs` has **no `Validate` command** (grep count 0 — the well-formedness check is `req conform`); `req help terminology` exists as the single-source V&V mapping. Cited tests pass live: `req_0190_conform_replaces_validate`, `req_0196_terminology_reference_available`.
- Dossier already substantive (cites cli.rs / conform_cmd.rs / help_text.rs and names the tests). The dossier itself honestly flags that one acceptance criterion (AC2 exact wording) rests partly on inspection — good, transparent practice.

**Caveats (MINOR, pre-existing):** REQ-V-0037 independence (authored + co-signed by Tom, as with SR-0004), and the statement is compound (REQ-V-0010, like SR-0006). Advisory.

**Verdict:** Verified standing CONFIRMED; dossier genuine and substantive; no change made. *(All 9 safety requirements now reviewed: 7 awaiting-cosign strengthened, 2 Verified spot-checked.)*

---

## SF-0001 — Integrity hash detects silent corruption of the spec
*Reviewed 2026-06-17 · safety-function verification dossier · in-progress (conclude-blocked until SR-0001 is co-signed)*

**Dossier content audited.** The analysis is substantive (cites `src/storage.rs` canonical SHA-256 load-time refusal = the safe state). Improvements made this pass:
- **Testing stage named the specific tests** (was "the integrity-hash load tests"): now `req_0003_integrity_blocks_load_after_semantic_tamper`, `req_0003_integrity_ignores_whitespace_only_change`, `sr_0001_integrity_tamper_refused_with_repair_pointer` — all re-run live, pass.
- **Coverage note corrected for accuracy.** The note I authored earlier said SR-0001 is "verified through its own dossier and human co-sign" — but **SR-0001 is awaiting co-sign, not yet co-signed**. Re-worded to "SR-0001's own dossier is genuine and awaiting human co-sign."

**Finding (MINOR, applies across the SF dossiers) — coverage notes over-state child status.** The SF coverage notes authored in the bulk "produce dossiers" step uniformly say each realizing SR is "verified through its own dossier and human co-sign." That is **not yet true** — those SRs are at *awaiting-cosign*. The sign-off basis (derived, accurate: "0/1 Verified — NOT yet signable") is the source of truth and is unaffected, but the free-text coverage notes read more confidently than the chain warrants. The loop will correct each SF's note as it reaches it.

**Verdict:** dossier sound and now more precise; correctly **not signable** until SR-0001 is co-signed (gate working as intended). No conclude attempted.

---

### Milestone — all 7 awaiting-cosign safety requirements audited
SR-0001/0002/0003/0006/0007/0008/0009 each independently confirmed against source + live tests, and each had its thin re-anchor narrative strengthened to carry its own evidence. `req conform` clean throughout. All remain at **awaiting-cosign** (no agent co-sign). One human-decision item outstanding: SR-0006's compound statement (recommended split). Next: spot-check the already-Verified SR-0004 / SR-0005, then the SF verification dossiers and the hazard adequacy dossiers.
