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

## Queue (not yet reviewed)
- SR-0001 (older safety requirement, awaiting co-sign)
- SR-0004, SR-0005 (already Verified — spot-check the genuine dossier)
- SF-0001..0008 (safety-function verification dossiers)
- HAZ-0001..0004 (hazard adequacy dossiers)

## Notes
- Same pattern likely affects the other re-anchored SRs (SR-0001/0002/0003/0006/0008/0009) — their narratives were written by the same re-anchor loop. Will confirm per-SR.
- Next iteration: SR-0001 (refuse to load a spec failing its integrity hash) — verify against the load-time hash check in `storage.rs` + the integrity test. Last awaiting-cosign SR; after it, spot-check the already-Verified SR-0004/0005, then the SF and hazard dossiers.
- Recurring finding: the re-anchored older SRs share the thin-boilerplate narrative; strengthening each as reviewed (SR-0002 ✓, SR-0003 ✓, SR-0006 ✓ done).
