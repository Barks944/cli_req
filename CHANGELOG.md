<!-- REQ-0080: CHANGELOG maintained alongside tagged releases. -->
# Changelog

All notable changes to this project are recorded here. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); the project
adheres to semantic versioning where `_format` schema bumps are major
version moves and CLI surface additions are minor.

## [Unreleased]

## [0.2.1] — 2026-05-17

Closes every finding from the 0.2.0 adversarial agent sweep.

### Fixed — `req review --gate` hardening (P1)
- **Broader source-extension defaults** for the markerless check.
  Now covers 50+ languages (Kotlin, Swift, Scala, C#, Ruby, PHP, Lua,
  Haskell, OCaml, Elixir, Erlang, Clojure, Dart, Zig, Nim, V, Crystal,
  F#, Groovy, Perl, shell, PowerShell, Objective-C, …). `--ext` to
  override. Was: hardcoded 10-entry Rust-leaning list, silently
  invisible to Kotlin/Swift/etc.
- **Fail-closed on missing base ref under --gate.** A CI YAML typo
  (`origin/master` vs `origin/main`) previously disabled the gate
  silently; now `req review --base bogus --gate` exits non-zero with
  a clear error. Advisory mode (no `--gate`) still produces the
  report.
- **Comment-context marker matching.** A `REQ-NNNN` token only
  counts as a marker on lines that look like a comment (start with
  `//`, `#`, `--`, `;`, `*`, or contain `//`/`#` before the token).
  String literals, doc attributes, and incidental string matches no
  longer satisfy the gate.
- **Default `--ignore` patterns** carve out test trees, build
  helpers, generated code, and the .req project file itself. The
  spec file's instructions block contains example REQ-NNNN tokens
  that should never be treated as markers or ghosts. `--ignore
  <glob>` to add more.
- **Ghosts deduplicated** to one finding per (id, file) pair, not
  one per textual occurrence.
- **Path separators normalised** to `/` everywhere in markdown and
  JSON output.

### Fixed — LLM hook usability (P1)
- **Stdin closed after payload write.** A naive
  `sys.stdin.read()` hook used to hang until the 10s timeout because
  the pipe stayed open. Now it returns immediately.
- **`REQ_VALIDATE_LLM_CMD` documented** in `req help env` with the
  full stdin/stdout/exit-code/timeout contract, an example, and a
  caching note.

### Fixed — split + ergonomics (P2)
- **`req split` inherits acceptance** from the parent. Functional
  reqs could not be split pre-0.2.1 because empty acceptance on
  part #1 tripped REQ-V-0014.
- **`req add` prints a follow-up nudge** pointing at the
  `// REQ-NNNN:` marker convention and `req coverage --path src`.
  Closes the discoverability gap between "REQ created" and "REQ
  referenced from code".

### Notes
- Sequential per-requirement hook calls remain the model. For paid
  LLM APIs cache by `sha256(statement)` in your hook; bounded
  parallelism is a 0.3 design question.

## [0.2.0] — 2026-05-17

### Added — new commands and rules
- **`req split <id> --into "..."`** (REQ-0085): interactive (or
  flag-driven) split of a compound requirement into N atomic ones.
  Children inherit kind / priority / tags; the original is
  soft-retired to Obsolete with history naming its replacements.
  `--keep-original` for additive splits.
- **`req review --base <ref>`** (REQ-0086): single-shot PR-style
  markdown report combining validate + coverage + stale + audit +
  the changed-requirement diff. `--gate` makes it CI-friendly: exits
  non-zero on validate errors, coverage ghosts, OR — critically —
  changed source files that contain zero `REQ-NNNN` markers (the
  "shipped new behaviour without a backing requirement" rule that
  let three releases slip through).
- **`req status --tag`** (REQ-0092): milestone-scoped status report.
  AND semantics across multiple `--tag` flags.
- **REQ-V-0023** (REQ-0087): opt-in external statement-quality
  hook. When `REQ_VALIDATE_LLM_CMD` is set, the validator pipes a
  small JSON stub on stdin and surfaces the hook's `{ok, message}`
  verdict. 10s timeout per requirement; never aborts validation on
  hook failure. Default validator stays deterministic and offline.

### Added — MCP surface
- `req_review` and `req_split` mirror the CLI commands for agents.
- `req_validate` output now includes `rule_code` on every finding so
  agents can pattern-match REQ-V-0021 cycles deterministically.

### Added — CI gate
- `.github/workflows/ci.yml` runs `req review --gate` on every PR
  against `origin/${{ base_ref }}`. New source files without a REQ
  marker fail the build.

### Added — process discipline
- `AGENTS.md` cardinal rule 7: "New behaviour gets a REQ first, then
  the code." Names `req review --gate` as the enforcement
  mechanism.

### Backfilled requirements
Catches up the spec to ship-state. Nine new REQs cover the 0.1.2 /
0.1.3 / 0.2.0 surface that landed without a backing requirement:
REQ-0084 (state machine), REQ-0085 (split), REQ-0086 (review),
REQ-0087 (LLM hook), REQ-0088 (REQ-V-0021 cycle), REQ-0089
(REQ-V-0022 hedge stacking), REQ-0090 (ID normalisation),
REQ-0091 (repair --force), REQ-0092 (status --tag). Each cited from
source via a `// REQ-NNNN:` line.

## [0.1.3] — 2026-05-17

### Changed (lifecycle policy)
- **Full state-machine guard on `req update --status`** (was: only the
  approach to Verified). The five-step ladder is now the discipline:
  Draft -> Proposed -> Approved -> Implemented -> Verified. Natural
  (free) transitions are forward-one-step, any-state -> Obsolete, and
  a Draft carve-out that allows Draft -> Proposed or Draft -> Approved
  directly (sketch-then-slot workflow). Everything else — skip-forward,
  any backward move, resurrection from Obsolete, leaving Verified for
  anything-but-Obsolete — requires `--force --reason "..."` so
  irregular moves stay deliberate and traceable.
- Same policy applied to `req batch` `update` mutations via a
  `"force": true` per-mutation flag.

This is a deliberate breaking change for anyone driving `req update`
non-naturally; the fix is to either walk the lifecycle one step at a
time or pass `--force` for the irregular step.

## [0.1.2] — 2026-05-17

### Fixed (P0 — closed bypasses around 0.1.1 guards)
- **`req batch` lifecycle guard**: `{kind: update, status: verified}`
  no longer slides Draft straight to Verified. Pass
  `"force": true` per-mutation to override. Same gate as direct
  `req update --force`.
- **`req batch` cycle detection**: link mutations now cycle-check
  every asymmetric kind (parent, depends-on, refines, verifies).
  Batch can no longer install cycles that `req link` rejects.

### Fixed (P1 — second-line defences and trapped states)
- **`req validate` reports graph cycles** as REQ-V-0021. Each cycle
  is reported once, attributed to its smallest-ID member. Catches
  cycles introduced by old binaries, merges, or hand-edits.
- **`req repair --force`** re-signs even when validation errors
  remain. Closes the deadlock where a hand-edit that broke both the
  hash AND validation left every command stuck. With --force the
  errors surface via `req validate` instead of the integrity check.
- **`req diff <REQ-ID>`** returns a friendly hint pointing at
  `req show` instead of leaking git's
  `fatal: invalid object name`.

### Improved (validator quality)
- **REQ-V-0010 compound-statement rebuilt**. "A and B and C" now
  triggers with a single modal verb (was the headline false-
  negative). "X, Y, and Z" Oxford-comma lists suppressed when there
  are 2+ commas and a single " and " (was the headline false-
  positive).
- **REQ-V-0022 hedge stacking**: 2+ of {perhaps, probably, maybe,
  possibly, might, roughly, potentially} now warns.
- **`req update` quiet-by-default**: only re-emits field-scoped
  warnings whose inputs the user actually edited. Status / priority
  / tag nudges no longer replay the same compound warnings each
  time.

### Improved (CLI polish)
- **Case- and pad-insensitive ID lookup**: `req-1`, `REQ-1`,
  `req-0001`, even bare `1` now resolve to `REQ-0001`. Misses on
  near-IDs suggest "did you mean REQ-0042?".
- **`req add -t/-s/-r` marked required** in `--help` (via
  `required_unless_present_any` on `--interactive`/`--from-json`).
- **`req retire` alias** for `req delete` matches the soft-retire
  semantics (Obsolete with links preserved). `req delete` still
  works for muscle memory.
- **`req doctor` signing is advisory**, not gating. Doctor's overall
  exit code only flips red on load-bearing setup gaps. Signing
  surfaces as `[WARN]`.
- **`req hooks install` writes `.gitattributes` once**, listing all
  added lines under a single update message instead of three.
- **`req batch` malformed JSON** wraps the serde error with a hint
  at `req schema batch`.
- **`req add` flushes stderr** before the stdout "Added" line so
  validation WARN lines can't appear after the success message.

## [0.1.1] — 2026-05-17

### Fixed
- **Lifecycle guard on `req update --status verified`**: reject direct
  jumps from any status other than Implemented. Pass `--force` to
  override (e.g. when correcting history). Closes a path where a Draft
  could be marked Verified in one command.
- **Lifecycle guard on `req verify --promote`**: only auto-promotes
  from Implemented. Adds `--force` (CLI) / `force` (MCP) for history
  fixes.
- **Link cycle detection** generalised across all asymmetric link
  kinds: `parent`, `depends-on`, `refines`, `verifies`. Previously
  only `parent` was checked.
- **`--json` errors** now write the structured envelope to stdout (the
  parseable channel) and exit non-zero without propagating the anyhow
  chain to stderr. Callers can `JSON.parse(stdout)` directly.
- **`req diff <ref>`** accepts a single ref as shorthand for
  `<ref>..HEAD`, matching `git diff <ref>` muscle memory. Same in MCP
  `req_diff`.
- **`req_import` MCP error envelope** no longer concatenates anyhow's
  Display chain — first envelope-shaped line wins.
- **`req_export` MCP** csv/html now actually returns the rendered
  output instead of an isError hint.
- **`req_help` MCP description** points at `section="_index"` as the
  authoritative section list (was a hardcoded list that drifted).
- **`req_next` default** excludes Verified as well as Obsolete — the
  "no filter" call no longer suggests already-shipped work.

### Internal
- New `first_envelope_line` helper for MCP subprocess error rendering;
  scans stdout (the envelope channel) before stderr.
- Regression tests for every fix in `tests/coverage_boost.rs` and
  `tests/mcp_tools.rs`.

## [0.1.0] — 2026-05-17

### Added
- `req batch` for transactional multi-mutation JSON input (REQ-0066).
- `req import` ingest from markdown or JSON, routed through the validator (REQ-0067).
- `req doctor` per-clone setup audit (pre-commit hook, merge driver, gitattributes pin, signing) (REQ-0064).
- `req diff <base>..<head>` per-requirement summary across git revs (REQ-0069).
- `req coverage --by-file` and `--unlinked-files` and `--remap` (REQ-0033/0032/0034).
- `req coverage --strict` exits non-zero on findings; CI/pre-commit gate (REQ-0065).
- `req add --from-json` and `req add` with JSON document input (REQ-0072).
- `req schema [add|batch|import]` publishes the JSON Schemas for structured input (REQ-0078).
- `req audit --gate --require-signer --require-good-signature` enforces a signature policy (REQ-0079).
- `req status` project-level implementation summary with `delivery_progress_pct` (REQ-0054).
- `req next` dependency-aware suggestion for the next requirement (REQ-0040).
- `req check <ref>` incremental validation and coverage scoped to changes (REQ-0041).
- `req version` and `req version --json` (REQ-0037).
- `req verify <id> --by composition|inspection` evidence record kinds (REQ-0056).
- `req test run` parses `cargo test` output and records evidence per REQ (REQ-0055).
- `req test record` attaches a manual test record with HEAD SHA (REQ-0049/REQ-0050).
- `req stale` two-level staleness (commit drift vs content drift) (REQ-0063).
- `req mcp` JSON-RPC stdio server with first-class agent guidance (REQ-0017/0047/0048).
- `req help <section> --install` writes managed blocks into AGENTS.md (REQ-0031).
- `req help <section> --json` structured agent crib (REQ-0042).
- Validator rule codes `REQ-V-NNNN` on every finding (REQ-0045).
- Structured JSON error envelope with stable `REQ-E-NNNN` codes (REQ-0039).
- `actor_kind` (human / agent / unknown) on every history entry (REQ-0043).
- Advisory file lock around mutation cycles (REQ-0062).
- `.gitattributes` line-ending pin to defend the integrity hash from autocrlf (REQ-0071).
- Test-vs-implementation marker classification in `req coverage` (REQ-0070).
- `req migrate` skeleton with backup, lock, and current-format no-op (REQ-0068).
- MIT license and crates.io metadata (`name=req-cli`, `[[bin]] name=req`) (REQ-0060/0061).
- Format-version policy section (REQ-0074).
- Hide `Obsolete` from default `req list` output; `--include-obsolete` to re-include (REQ-0073).
- Duplicate-intent warning when two non-obsolete requirements have semantically overlapping statements (REQ-0076).
- Validator warning for `verifies` links whose source has no test record (REQ-0077).
- GitHub Actions release workflow for tagged `v*` pushes (REQ-0059).

### Notes
- This release predates the first crates.io publish; the `0.1.0` tag will be the first.
