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
