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
