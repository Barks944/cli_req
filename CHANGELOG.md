<!-- REQ-0080: CHANGELOG maintained alongside tagged releases. -->
# Changelog

All notable changes to this project are recorded here. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); the project
adheres to semantic versioning where `_format` schema bumps are major
version moves and CLI surface additions are minor.

## [Unreleased]

## [0.4.0-rc.4] — 2026-05-22

### Added
- **REQ-0130 / REQ-V-0024:** validator warning on Verified
  requirements whose latest test record is a Fail. Surfaces the
  contradiction in every `req validate` call (CI logs, pre-commit
  output, MCP responses) without flipping the exit code. The hard
  block stays in `req review --gate --no-defects` (REQ-0126); this
  is the soft, ever-present signal.

## [0.4.0-rc.3] — 2026-05-21

Driven by external rc.1 user feedback after a real 64-req migration.

### Fixed
- **REQ-0124 (coverage respects .gitignore):** the source-tree walk
  now goes through a shared `source_walk` module that wraps the
  `ignore` crate (ripgrep's walker). `req coverage`, `req lint`, and
  test-record linked-file discovery all honour `.gitignore`,
  `.ignore`, `.git/info/exclude`, and the user's global git
  excludes. The hard-coded SKIP_DIRS lists are gone — adopters'
  actual ignore patterns are what counts. Closes the bug where
  `tmp/` artefacts appeared as ghost references.

### Added
- **REQ-0125 (defects in status / brief / lint):** a new
  `verified-but-defective` count surfaces Verified requirements
  whose latest test record is a Fail. Appears in `req status`,
  `req brief`, and `req lint`. Shared definition in
  `status::verified_but_defective()` so all three surfaces agree.
- **REQ-0126 (review --gate --no-defects):** opt-in CI gate that
  exits non-zero when any requirement carries a failing latest test
  record. Off by default so existing pipelines stay unchanged.
  Defects also surfaced in the markdown report and JSON output
  regardless of `--gate`.
- **REQ-0127 (coverage --by-req):** symmetric inverse of
  `--by-file`. For each REQ-NNNN with markers, lists the files
  referencing it. JSON emits a flat REQ → [paths] map.
- **REQ-0128 (test run --map):** ecosystems without the
  `req_NNNN_*` naming convention (Node, Python) can pass a JSON
  map of test name → REQ-ID(s). The recorder uses a forgiving
  verdict matcher (ok/PASS, FAILED/FAIL, ignored/SKIP) on lines
  containing the test name. Schema published via
  `req schema test-map`.
- **REQ-0129 (req test list <id>):** dedicated subcommand to
  inspect a requirement's test records without parsing
  `req show --json`. Human format prints
  `<timestamp> <commit> <outcome> <kind> <notes>` per record;
  `--json` emits the records array.

## [0.4.0-rc.2] — 2026-05-20

Bugfix pass on rc.1 driven by external E2E testing.

### Fixed
- **REQ-0123 (doctor worktree):** `req doctor` now resolves the
  hooks directory via `git rev-parse --git-common-dir`, so it finds
  the shared `pre-commit` hook from inside a linked worktree
  instead of falsely reporting it missing. Same root-cause fix as
  REQ-0117 on the install side.
- **REQ-0119 (import schema):** the `req schema import` JSON
  schema now lists `rationale` in `required`, matching the
  validator's REQ-V-0012 rule. The schema no longer lies about
  what the validator will accept.
- **REQ-0120 (AGENTS.md placeholder IDs):** `req help <section>
  --install` rewrites literal `REQ-NNNN` identifiers in the
  installed text to the placeholder `REQ-NNNN` (non-digits).
  Adopters' coverage scanners no longer pick up cli_req's own
  example IDs (REQ-0001 etc.) as ghost references in their
  projects.

### Changed
- **REQ-0121 (coverage report):** `req coverage` now splits the
  unmarked-requirements view into two buckets — `orphans`
  (non-Draft, non-Obsolete; strict-gated) and `drafts_unmarked`
  (Draft; informational only). The human output labels each
  bucket with its semantics. Strict-mode gating behaviour is
  unchanged.

### Added
- **REQ-0122 (auto-migrate on stale _format):** loading a
  project.req at an older `_format` now performs the migration
  in-process on first encounter — writes a sibling backup,
  applies the registered migration chain, re-signs the integrity
  hash, then re-reads the file. Opt out with
  `REQ_NO_AUTO_MIGRATE=1` to keep the prior manual-migrate
  error. Removes the post-upgrade ritual of typing `req migrate`
  on every project.
- **`req version` JSON now carries a `prerelease` boolean** so
  tooling can detect pre-release binaries (`0.4.0-rc.2` etc.)
  without parsing the version string. The human-readable line
  stays exactly `req <version>` to preserve REQ-0037 parity with
  clap's `-v` / `--version`.

## [0.4.0-rc.1] — 2026-05-20

The schema-bump release. Adds two reserved top-level keys to
`project.req` (`_purpose`, `_config`), a retrofit helper for bulk
adoption (`req adopt`), a content-hashed staleness signal that
stops flapping on unrelated commits, and a local CI gate to catch
environment-skew failures before push.

### Format change — req-v1 → req-v2

- **`_format` is now `req-v2`.** Existing v1 files are not auto-loaded;
  run `req migrate` to upgrade in place. The migration writes a sibling
  `*.bak-req-v1` backup, preserves every ID, history entry, link, and
  test record byte-for-byte, and re-signs the integrity hash.
- **Older binaries** opening a v2 file produce a clear `unsupported
  _format` error pointing at `upgrade the req binary` — never a silent
  mis-read.
- Regression fixture: `tests/fixtures/v1_project.req` is checked in and
  exercised by CI to prevent silent breakage as new format versions
  arrive.

### Added — project purpose (REQ-0111)

- **`_purpose` reserved top-level field** holds an optional one-paragraph
  statement of what the project is FOR. Validator-capped at 500
  characters so it stays scannable.
- **`req init --purpose '...'`** sets it at project creation.
- **`req purpose '...'` --reason '...'** edits it later. Print mode
  (`req purpose` with no args) shows the current value.
- **`req brief` leads with the purpose** when set, followed by the top
  three Must-priority Verified requirements (the project's "spine").
  Agents picking up a project cold learn what it's for in one read.

### Added — per-project configuration (REQ-0110)

- **`_config` reserved top-level field** holds per-project defaults
  under the integrity hash. Precedence: CLI flag > `_config` >
  built-in defaults.
- **Exposed surface** (all optional):
  - `coverage.extensions` — source extensions to scan
  - `gate.marker_near_hunks` — strict-mode marker proximity
  - `lint.short_rationale_words` — rationale length floor
  - `lint.inspection_only_tags` — tags that exempt no-test findings
- The SQL-extension case from real adoption (a project that wanted
  `.sql` files scanned but couldn't pass `--ext sql` to the pre-commit
  hook) is now solvable in-file.

### Added — `req adopt` retroactive backfill (REQ-0109)

- **`req adopt <id>... --to <status>`** advances one or more
  requirements through the lifecycle in a single invocation. Walks
  `Draft → Proposed → Approved → Implemented → Verified`, recording
  one `adopt → <status>` history entry per hop so the trail is
  auditable.
- **`--all-drafts`** scopes to every requirement currently at Draft.
- **`--dry-run`** prints what would change without writing.
- **Functional requirements with no acceptance** being adopted to
  Implemented or Verified get an auto-generated placeholder entry
  (`implementation in source at adoption time`) plus a history entry
  flagging the auto-add, so reviewers can spot placeholder acceptance
  lines later. Decision recorded in REQ-0109 for the audit trail.
- **Verified target** also records an inspection-evidence test record
  so `req validate` sees the requirement as properly closed.

### Added — `req precheck` local CI gate (REQ-0114)

- **`req precheck`** runs the same six gates CI runs, in the same
  order: `cargo fmt --check`, `cargo clippy`, `cargo test`,
  `req validate`, `req coverage --strict`, `req review --gate`.
  Exits non-zero on the first failure with a clear pointer at which
  step blew up.
- **`--skip <step>`** (repeatable) for tight inner loops.
- **`--keep-going`** reports every failure, not just the first.
- Wire into a pre-push hook or editor save action to catch rustfmt
  drift and fixture-config flakiness in the same loop as the code,
  not in CI five minutes after push.

### Added — content-hashed staleness (REQ-0112)

- **`TestRecord` carries an optional `content_hash`** (sha256 over
  linked-file contents) and an optional `linked_files` override.
- **`req stale` uses the hash when present**: STALE fires only on
  actual content change of the linked files, not on every HEAD
  movement. The previous SHA-based check was a false-positive
  generator for projects with active commit traffic.
- **Linked files** are auto-discovered by default via `// REQ-NNNN:`
  markers; pass `linked_files` on the record for explicit override.
- **Backwards compatible**: records without `content_hash` continue
  to use the SHA-based check, so existing projects keep working
  without re-recording.

### Added — `req migrate` registry (REQ-0116)

- **`src/migrations.rs`** carries a registered-steps list that
  `req migrate` walks. The v1 → v2 step is the first entry; future
  schema bumps register here.
- **Backup-then-mutate-then-re-sign** contract: every migration
  writes a sibling backup before any change, then walks the chain,
  then re-computes the integrity hash. Integrity is verified BEFORE
  migration so corrupt files get a `repair` hint instead of a
  silent overwrite.

### Added — priority-label strip in REQ-V-0010 (REQ-0113)

- **Validator no longer counts `Must`/`Should`/`Could`/`Wont` as a
  modal verb** when they appear immediately followed by `-priority`
  or `-priorities`. The validator's own ruleset was biting our own
  spec when we wrote phrases like "Must-priority Verified
  requirements". Narrowest possible fix.

### Changed
- **`req coverage` excludes Drafts from the orphan check** by design.
  A Draft has no implementation yet; expecting a marker is a category
  error. Strict mode now fires only on Implemented/Verified
  requirements that lack a code reference.

### Fixed — worktree hooks path (REQ-0117)
- **`req hooks install` and `req setup` work in git worktrees.** The
  prior code joined `.git/hooks` onto the repo path, which inside a
  worktree resolves to `.git/worktrees/<name>/hooks` — a directory
  that doesn't exist. We now call `git rev-parse --git-common-dir`
  so the hook lands in the shared `<main>/.git/hooks`. `req setup`
  also gained a `--repo` flag for the edge case where cwd resolution
  still picks the wrong tree.

### Known limitations
- `req adopt` is CLI-only for now; the MCP tool surface will follow.
  Agents needing bulk adoption can shell out to `req adopt`.

## [0.3.2] — 2026-05-18

The retrofit-friendly release. Driven by an honest one-session
review from a real adopter; closes the no-design-required half of
that punch list.

### Added — schema/migration scanning
- **`sql` in the default extension list** for `req coverage`,
  `req review`, and `req lint`. Schema-as-code (init scripts,
  migrations) is a first-class implementation surface, so SQL files
  participate in the gate by default. Was previously invisible
  unless callers passed `--ext sql` per invocation.

### Added — inspection-only convention for lint (REQ-0107)
- **`req lint` skips the no-test-record finding** for requirements
  tagged `inspection-only`. Use this tag for things that ARE the
  spec but aren't unit-testable: CORS rules, no-cache headers,
  audit-logging policies. Documented in `req help lint`.

### Added — `req batch verify` (REQ-0066 extension)
- **New `kind: verify` mutation** in `req batch` mirrors
  `req verify`: `id`, `by`, `notes`, optional `cites`, `promote`,
  `force`. Lets a retroactive adoption of N requirements record
  evidence + promote in one atomic operation instead of N shell
  invocations. Atomic rollback preserved.

### Added — PR-comment workflow template (REQ-0108)
- **`.github/workflows/spec-review.yml`** is now a canonical
  template. Copy into your repo for an automatic single bot
  comment per PR carrying the `req review` markdown. Updated in
  place on re-pushes (not stacked); truncated at 60k chars with
  workflow-artefact fallback. Doesn't gate — that's `ci`'s job.
  Documented in `req help integration`.

### Fixed — friendlier reframing
- AGENTS.md cardinal rule 1 and `req help agents` "how the file
  is protected" section both rewritten. The integrity model is
  honest about being a post-hoc contract ("agents that bypass
  the CLI don't break anything silently — the next operation
  refuses to load and prompts a manual repair audit") rather
  than overclaiming "agents cannot edit project.req."

### Fixed — quieter post-commit on bulk commits
- The post-commit summary collapses when more than 5 REQs are
  cited in a single commit. Shows count + first 3 + pointer to
  `req brief` for the full picture. Was a 46-line wall during
  the agent-reported retrofit adoption commit; now a calm
  4-liner.

### Fixed — test reliability with global git signing
- `req_0079_audit_gate_exits_nonzero_without_signing` now passes
  `-c commit.gpgsign=false` to its fixture commit. Was silently
  signing under a developer's global config and defeating the
  test's own assertion.

### Deferred to 0.4.0
- `req adopt` (retroactive backfill helper)
- Per-project config via `_config` field or sidecar
- `_purpose` field + richer `req brief` content
- Content-hashing test records

These need design discussion before code. The 0.3.2 batch is
purely additive — no schema bumps, no breaking changes.

## [0.3.1] — 2026-05-18

Closes the four real frictions from the 0.3.0 agent sweep. No new
commands; the loop just actually closes now.

### Fixed — status-aware post-commit (REQ-0106)
- **`req review --summary`** now reads each cited REQ's current
  status and prints the LEGAL next transition for that status, not
  a one-size-fits-all `--promote` suggestion. Draft gets "advance
  to proposed", Approved gets "mark implemented", Implemented gets
  "verify --promote", Verified/Obsolete get a quiet "no action"
  note.
- **Pre-commit hook's pass path is silent.** The 0.3.0 hook
  duplicated the post-commit summary; users saw the same line
  twice per commit. Single source of truth now lives in the
  post-commit hook.

### Fixed — `req setup` is actually one-shot
- **Merge driver auto-registered.** Setup now runs the two `git
  config merge.req-merge.*` commands inline after writing
  `.gitattributes`. No more "next steps you must paste"; `req
  doctor` reports the driver as active on a fresh bootstrap.
- **Next-steps example is copy-paste-runnable.** The 0.3.0 example
  was `req add ... -k functional -p must` with no `-a`, which the
  validator immediately rejects (functional reqs need acceptance).
  Now includes a sample acceptance criterion.

### Fixed — strict mode sticky on reinstall
- `req hooks install` (no flag) now inherits the existing hook's
  mode. A project that's strict stays strict on re-install. Pass
  `--strict` explicitly to flip from default to strict.

### CI
- 0.3.0 `cargo fmt` failure in `src/mcp.rs` fixed in 5b41e9c
  (trailing-comment alignment + vec macro collapse). Green across
  ubuntu/macos/windows + lint.

## [0.3.0] — 2026-05-18

Reframes the tool around **spec-memory that survives between
conversations**, not enforcement that catches mistakes. Same
guarantees, surfaced through reminders an agent actually wants to
read. New command surface and new docs voice.

### Added — adoption surface
- **`req brief` (REQ-0104)** — session-start summary. Project name,
  delivery percentage, what's queued (single highest-priority pick),
  what's loose (Implemented but not Verified, Drafts). Default short
  (5-10 lines, fits an agent context); `--full` for the dashboard;
  `--json` for tooling. The first thing an agent should run in a new
  conversation.
- **Post-commit hook (REQ-0103)** — calm one-line impact summary
  after every commit that touched source files. Never gates. Names
  the cited REQs and points at the natural next status change.
  Installed alongside pre-commit; both removed by `req hooks
  --uninstall`. `req hooks install --force` upgrades an existing
  managed pre-commit hook.
- **`req setup` (REQ-0105)** — one-shot bootstrap: init + git hooks
  (pre + post) + AGENTS.md managed block. Idempotent. `--strict`
  for hunk-level pre-commit; `--no-hooks` / `--no-agents` to opt
  out. `--name` overrides the inferred project name.
- **Pre-commit gate's pass path now prints a summary** instead of
  staying silent: `req: N source file(s) staged · cites REQ-A,
  REQ-B · reminder: ...`. Silent on pure-docs commits.

### Cross-surface parity
- New MCP tool **`req_brief`** with an explicit "run this FIRST"
  framing in the tool description. `req_setup` is humans-only by
  design (interactive bootstrap).
- New TUI menu entry **"Brief (where are we now?)"** as the first
  item.

### Voice / framing — same content, friendlier register
- `req help agents` rewritten. Leads with "why this exists" and
  names the vibecoding problem directly: conversations end, the
  spec must survive. The trigger table becomes "WHEN THE USER ASKS
  FOR SOMETHING NEW" / "WHILE YOU WORK" / "WHEN YOU FINISH
  SOMETHING".
- AGENTS.md cardinal-rules block rewritten as "The handful of
  things that matter" — same discipline, less lawyer.
- README adds two TL;DR sections (agents + humans) before the
  Mission. Quick start now headlines `req setup`.

### Polish (rolled in from the 0.2.5 agent sweep)
- **REQ-V-0019** now names `req test record` / `req verify` as the
  fix path, not just the symptom.
- **REQ-V-0022** quotes the actual hedge words that fired
  (`perhaps`, `maybe`, `probably`) instead of just counting them.
- **REQ-V-0008** trimmed — points at `req help best-practice` for
  the modal-verb gloss.
- **`req lint`** quality section de-dupes against the validator
  (REQ-V-0013) so a single requirement is never named twice with
  different thresholds.
- **`req help lint`** section added (was missing in 0.2.5).
- **`short_rationale` JSON shape** is now `{id, words}` objects,
  consistent with the rest of the lint quality block.
- **Markerless check** excludes Draft status (consistent with the
  no-test-record check). "Active" = Proposed-or-later everywhere.
- **`req validate` findings sort by REQ-ID** ascending. Stable
  output across runs.
- **`req diff HEAD~1` on a fresh repo** returns a friendly hint
  instead of leaking git's raw `fatal: invalid object name`.

### Project state
- 100 / 100 requirements Verified, 100% delivery.
- `req validate` clean.

## [0.2.5] — 2026-05-18

Quality pass. No breaking changes; the project now validates clean
against its own ruleset.

### Exemplar cleanup
- Seven requirement statements rewritten to be atomic:
  REQ-0039, REQ-0066, REQ-0067, REQ-0084, REQ-0085, REQ-0090,
  REQ-0098. Detail moved into acceptance criteria where it belongs.
  `req validate` on `project.req` now reports
  `OK — 95 requirement(s), no findings.`

### Validator wording (REQ-V-0001 .. REQ-V-0015, REQ-V-0022)
- Every message now names the rule and tells you the fix. The
  rule codes are unchanged; only the message text is sharper.
  Examples:
  - REQ-V-0010 names *why* it tripped (`semicolon`, `repeated
    modal`, or `multiple "and" joins`) and suggests `req split`.
  - REQ-V-0009 spells out what "measurable criterion" means.
  - REQ-V-0013 reports the actual word count.
  - REQ-V-0015 quotes the criterion length and shows an example.
  - REQ-V-0008 lists the four normative modals with their meanings.

### Added — `req lint` (REQ-0101)
- New `req lint` command produces a project-wide quality audit
  beyond `req validate`: marker coverage, rationale length,
  acceptance count, test-record presence, verification-kind mix.
- Output is markdown by default; `--json` for tooling. Exit code
  reflects validator errors only — lint observations never gate.
- Available as `req_lint` MCP tool and `Lint (quality audit)` TUI
  menu entry, preserving cross-surface parity.

## [0.2.4] — 2026-05-18

### Added — strict pre-commit mode (REQ-0100)
- **`req hooks install --strict`** writes a pre-commit hook that
  invokes `req review --staged --gate --marker-near-hunks 50`, so
  edits inside an already-marked file still need a marker near the
  changed hunk. Closes the file-level loophole observed live during
  0.2.3 implementation.
- Re-running `req hooks install` (with or without `--strict`)
  swaps modes deterministically.
- **`req doctor`** surfaces the active gate mode (`[strict mode]`
  or `[default mode]`) on the pre-commit-hook check.
- **`req help integration`** documents both modes and the tradeoff.

## [0.2.3] — 2026-05-18

Clears the six Draft requirements added in 0.2.1.

### Validator
- **REQ-0093 / REQ-V-0019 gated on status >= Implemented.** A
  `verifies` link on a Draft/Proposed/Approved source no longer
  triggers the "no test record" warning; it only fires once the
  requirement actually reaches Implemented. Stops the validator
  from crying wolf during the early lifecycle.
- **REQ-0095**: `req add` warns when the new title is highly
  similar (Jaccard ≥ 0.65) to a requirement that became Obsolete
  in the last 60 days, suggesting `req update` or `req split
  --keep-original` instead of a fresh ID.

### CLI surface
- **REQ-0094**: every `value_enum` arg (`--status`, `--kind`,
  `--priority`, `--by`, etc.) is now case-insensitive.
  `Implemented`, `IMPLEMENTED`, and `implemented` all fold to the
  canonical lowercase form.

### LLM hook
- **REQ-0096**: new `REQ_VALIDATE_LLM_SHELL` env var overrides the
  shell used to invoke the hook. Set to `bash`, `pwsh`, or `sh` so
  shell-script hooks work on Windows when Git Bash or PowerShell is
  on PATH and `cmd.exe` is not finding the interpreter.
- **REQ-0097**: new `REQ_VALIDATE_LLM_CONCURRENCY` env var caps the
  number of in-flight hook calls. Default 1 (sequential, matches
  0.2.x). Findings are sorted by id afterwards so output is stable
  regardless of completion order. 10s timeout remains per-call.

### Review gate
- **REQ-0098**: new `--marker-near-hunks N` flag on `req review`
  requires a `// REQ-NNNN:` comment within N lines of each changed
  hunk, not merely somewhere in the file. Default 0 keeps the
  0.2.x file-level behaviour. Use a positive value (e.g. 50) for
  strict hunk-level enforcement on real PRs.

### Test fixture update
- `req_0077_verifies_link_without_test_record_warns` walks the
  fixture to Implemented before asserting the rule fires, matching
  the new REQ-V-0019 gating semantics.

## [0.2.2] — 2026-05-18

### Added — pre-commit gate (REQ-0099)
- **`req hooks install` now wires the gate into the pre-commit
  hook**, not just CI. An agent (or human) committing code without a
  `// REQ-NNNN:` marker is blocked locally with the same educational
  message the CI gate uses, listing the offending files and naming
  the two fix paths (add a marker citing an existing REQ, or
  `req add` a new one then mark the code).
- **`REQ_SKIP_GATE=1 git commit ...`** bypasses the gate for genuine
  WIP / rebase / merge cases. The env var leaves a trace in shell
  history rather than being silent.
- **`req review --staged`** is the new scope mode the hook uses:
  reads `git diff --cached --name-only` instead of `git diff
  <base>...HEAD`. Works on the first commit (HEAD doesn't have to
  exist) — `--gate` does NOT fail-closed in staged mode.
- **CHANGELOG.md / README.md / AGENTS.md added to default `--ignore`**
  so descriptive REQ-ID mentions in docs aren't read as ghosts.

### Upgrade path
- Existing managed pre-commit hooks are upgraded by re-running:
  `req hooks install --force`. Hooks not managed by `req` are left
  alone unless `--force` is passed.

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
