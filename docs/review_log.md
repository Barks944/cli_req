# Dossier double-check — progress log

A running log of the self-paced dossier-review loop. Each line: when, what was
reviewed, the outcome, and what's next. The detailed findings live in
[review_report.md](review_report.md).

## Reviewed
- **SR-0007** — 2026-06-17 — behaviour CONFIRMED (gate code + 3 `req_0201` tests pass live); MINOR finding: dossier narrative was thin re-anchor boilerplate. **RESOLVED 2026-06-17** — dossier re-recorded with substantive analysis (cites the 3 enforcement points + source refs) and testing (names the 3 `req_0201` tests); re-concluded Pass, conform clean, awaiting human co-sign.

- **SR-0008** — 2026-06-17 — behaviour CONFIRMED (hazard_update gate, human-only hazard_confirm, conform REQ-V-0043, `req_0202` test passes live); thin boilerplate narrative **RESOLVED** in-pass (re-recorded with enforcement refs + named test). Awaiting human co-sign.

- **SR-0009** — 2026-06-17 — behaviour CONFIRMED (ACHIEVED_INTEGRITY_STAMP printed in sf_show + sreq_show, live-checked, `req_0203` passes); thin boilerplate narrative **RESOLVED** in-pass (named the constant, both print sites, the test). Awaiting human co-sign.

- **SR-0006** — 2026-06-17 — behaviour CONFIRMED (CONFORM_DISCLAIMER on success/failure/--json, points to `req verification status`, `coverage_gap.rs:474` + live check, automated+composition evidence); thin narrative **RESOLVED** in-pass. Additional finding (NOT actioned): SR-0006's own statement is compound (REQ-V-0010) — recommend a split; left for a human (changes the requirement, not the dossier).

- **SR-0003** — 2026-06-17 — behaviour CONFIRMED (18 `super::history` append sites in safety.rs, attributed with actor_kind, live HAZ-0001 has 9 append-only entries, `req_0011` append-only test passes); thin narrative **RESOLVED** in-pass. Awaiting human co-sign.

- **SR-0002** — 2026-06-17 — behaviour CONFIRMED (SIL-rigour gate in promote_preflight/sreq_verify, sil_gate_exception flag, conform REQ-V-0031; three SIL-gate tests pass live incl. the dedicated `sr_0002_sil_gate_at_sil4`); thin narrative **RESOLVED** in-pass. Best-tested of the older SRs. Awaiting human co-sign.

- **SR-0001** — 2026-06-17 — behaviour CONFIRMED (load-time canonical-hash refusal in storage.rs with `req repair` pointer; req_0003 tamper + whitespace-ignore tests + `sr_0001` test pass live); thin narrative **RESOLVED** in-pass. Awaiting human co-sign. **— all 7 awaiting-cosign SRs now done.**

- **SR-0004** — 2026-06-17 — SPOT-CHECK (Verified, co-signed by Tom; not modified). Standing CONFIRMED genuine/fresh; claim verified (provenance.rs classify/report, req_0142 + sr_0004 tests pass live); dossier already substantive (escaped the boilerplate). Caveat: REQ-V-0037 independence (authored+co-signed by Tom). No change made.

- **SR-0005** — 2026-06-17 — SPOT-CHECK (Verified, co-signed by Tom; not modified). Standing CONFIRMED genuine; claim verified (no Validate command in cli.rs, `req help terminology` exists, req_0190 + req_0196 pass live); dossier substantive (honestly flags AC2-by-inspection). Caveats: REQ-V-0037 independence + compound statement (REQ-V-0010). **— all 9 SRs now reviewed.**

- **SF-0001** — 2026-06-17 — dossier audited (in-progress, conclude-blocked until SR-0001 co-signed). Analysis substantive (storage.rs canonical-hash refusal). IMPROVED: testing stage now names req_0003 + sr_0001 tests; coverage note CORRECTED (had over-claimed SR-0001 as co-signed — it's awaiting). New finding: SF coverage notes generally over-state child status; sign-off basis (derived) is accurate. Correct as reached.

- **SF-0002** — 2026-06-17 — dossier audited (in-progress, conclude-blocked until SR-0002 co-signed). Analysis substantive (SIL-rigour gate). IMPROVED: testing named (req_0135/req_0139/sr_0002 SIL-gate tests); coverage note CORRECTED (over-claim → awaiting). conform clean.

## Queue (not yet reviewed)
- SF-0003..0008 (safety-function verification dossiers — correct the coverage-note over-claim on each)
- HAZ-0001..0004 (hazard adequacy dossiers)
- SR-0004, SR-0005 (already Verified — spot-check the genuine dossier)
- SF-0001..0008 (safety-function verification dossiers)
- HAZ-0001..0004 (hazard adequacy dossiers)

## Notes
- Same pattern likely affects the other re-anchored SRs (SR-0001/0002/0003/0006/0008/0009) — their narratives were written by the same re-anchor loop. Will confirm per-SR.
- Next iteration: SF-0003 (append-only history + signed-commit audit trail) — audit dossier vs the history-append code; name tests; correct the SR-0003 coverage-note over-claim.
- Active finding to apply per-SF: coverage notes say realizing SRs are "verified ... human co-sign" but they're awaiting-cosign — correct each note's wording (SF-0001 ✓, SF-0002 ✓ done).
- Recurring finding CLOSED for the awaiting-cosign SRs: all 7 had thin re-anchor narratives, now strengthened (SR-0001/0002/0003/0006/0007/0008/0009 ✓).
