// Implements REQ-0018 (structured, sectioned help browsable by name).
// REQ-0115: all user-facing help text lives here; this file is the
// source for the agents block written into AGENTS.md and the
// `req help <section>` surface, so it is the canonical place to
// document new commands and format changes.
pub struct Section {
    pub name: &'static str,
    pub summary: &'static str,
    pub body: &'static str,
}

pub fn sections() -> &'static [Section] {
    SECTIONS
}

pub fn section(name: &str) -> Option<&'static Section> {
    SECTIONS.iter().find(|s| s.name.eq_ignore_ascii_case(name))
}

/// REQ-0045 / REQ-0089 / REQ-0093: render the validator rule catalogue from the
/// single source of truth (`crate::validate::RULES`) so the `errors` and
/// `best-practice` help sections list every code the validator can emit and
/// cannot silently drift behind it.
pub fn rule_code_table() -> String {
    crate::validate::RULES
        .iter()
        .map(|(code, desc)| format!("  {code}  {desc}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Expand dynamic placeholders in a section body before it is printed,
/// installed into AGENTS.md, or emitted as JSON. Currently substitutes the
/// `{{RULE_CODES}}` token with the live validator rule catalogue.
pub fn render_body(body: &str) -> String {
    body.replace("{{RULE_CODES}}", &rule_code_table())
}

const SECTIONS: &[Section] = &[
    Section {
        name: "overview",
        summary: "What `req` is and why it exists.",
        body: "`req` is a managed requirements tool. Requirements live in a
git-tracked `project.req` file: pretty-printed JSON so diffs are
reviewable, but every mutation goes through this CLI, which enforces
best-practice rules. A SHA-256 `_integrity` field over the canonical
payload catches hand edits — the CLI refuses to load a tampered file
and tells the user to run `req repair --confirm-direct-edit`.

Humans get a `tui` browser, a local web server (`serve`), and exports
to Markdown/JSON/CSV/HTML. Agents get a `mcp` mode that speaks JSON-RPC
over stdio, exposing the same managed operations as MCP tools.

See `req help file-format` for the on-disk layout and `req help agents`
for the agent trigger table.",
    },
    Section {
        name: "concepts",
        summary: "Requirement, kind, priority, status, links.",
        body: "A REQUIREMENT has: id (REQ-NNNN), title, statement, rationale,
acceptance criteria, kind, priority, status, tags, and links.

KIND — Functional, NonFunctional, Constraint, Interface, Business.
PRIORITY — Must / Should / Could / Wont (MoSCoW).
STATUS  — Draft → Proposed → Approved → Implemented → Verified.
          Obsolete is a terminal state for retired requirements.
LINKS   — Parent, DependsOn, Conflicts, Refines, Verifies.

Functional requirements MUST have at least one acceptance criterion.
Approved/Implemented/Verified functional reqs cannot lack acceptance.",
    },
    Section {
        name: "best-practice",
        summary: "Rules the validator enforces.",
        body: "Enforced (errors block save):
  * title 5-120 characters (counted as Unicode chars, not bytes), non-empty
  * statement >= 5 words, contains shall/must/should/will, not a question
  * URLs and `inline code` are stripped before the modal-verb check, so
    `https://shall.example.com/` does NOT satisfy the rule
  * rationale non-empty
  * functional requirements need acceptance criteria
  * link targets must exist; no self-links; parent links cannot cycle
  * approved/implemented/verified functional reqs need acceptance
  * Verified requirements (REQ + SR) need a passing validation dossier
    (REQ-0139: plan → analysis → testing → statement → verdict). An
    ordinary requirement may instead carry a `validation-exempt` tag or an
    audited back-fill; safety requirements have NO exemption — neither a tag
    nor a back-fill, only a genuine dossier (REQ-0143). See REQ-V-0032 /
    REQ-V-0033.

Warned (saved but flagged):
  * weasel words: etc, and/or, user-friendly, fast, robust, TBD, ...
  * compound statements — flagged when ANY of these hold:
      - the statement contains a semicolon
      - the statement contains more than one normative modal verb
      - the statement has 3+ comma-separated clauses joined by ' and '
  * statements longer than 80 words (likely non-atomic)
  * trailing period on the title
  * very short rationale / vague acceptance criteria

BACKTICK ESCAPE (use sparingly)

  The compound, weasel-word, and modal-verb checks all run against
  the statement AFTER stripping URLs and `inline code` spans. This
  lets you cite forbidden terms when documenting a rule
  ('the validator shall warn on `etc`, `TBD`, ...') and embed
  enumerations of identifiers, CLI flags, or REQ-IDs without
  tripping the heuristics. Use it for descriptive code citations,
  NOT to launder genuinely compound obligations into a single
  statement — that would game the validator and weaken the spec.

VALIDATOR RULE CODES (every code the validator can emit, with its meaning)

{{RULE_CODES}}",
    },
    Section {
        name: "workflow",
        summary: "Typical lifecycle for a requirement.",
        body: "1. `req init -n MyProject`               create project.req
2. `req add`                              guided interactive add
3. `req list`                             review the table
4. `req show REQ-0001`                    inspect one
5. `req update REQ-0001 --status proposed --reason \"team review\"`
6. `req link REQ-0002 REQ-0001 -k parent` build hierarchy
7. `req validate`                         pre-flight check
8. `req export -f markdown -o reqs.md`    publish",
    },
    Section {
        name: "file-format",
        summary: "What's in a .req file and how integrity is checked.",
        body: "On-disk format is pretty-printed JSON so the file is git-diffable.
Four reserved top-level fields sit above the project payload:

  _warning      DO NOT EDIT THIS FILE BY HAND. (one-line headline)
  _instructions multi-line array of CLI directions for whoever opens it
  _format       schema tag, currently \"req-v1\"
  _integrity    sha256:<hex> of the canonical payload

The integrity hash covers every other top-level field in canonical
form (sorted keys, no whitespace). Whitespace-only edits do not change
the hash. Semantic edits — even one priority changed from Must to
Should — do, and the CLI refuses to load until you run:

  req repair --confirm-direct-edit

This is the core safety property: structured edits only. Agents must
go through `req`; the file is plain JSON for diffability, not for
direct editing.",
    },
    // REQ-0104: agents section rewritten with a warmer voice and a
    // "run `req brief` first" recommendation, matching the framing
    // shift toward spec-memory rather than enforcement.
    Section {
        name: "agents",
        summary: "What req does for you as an agent, and how to use it.",
        body: "Hey. If you're an LLM agent picking up this project — this is for you.

WHY THIS EXISTS (the short version)

  Vibecoding sessions are conversational. The user describes what they
  want, you build it, the conversation ends. Without something carrying
  the spec between sessions, the next conversation starts blind: you
  re-discover the project from source files, you re-derive the intent,
  and small things drift.

  `req` is the spec-memory that survives between conversations. The
  project's requirements live in a git-tracked JSON file managed by
  this CLI. The tool tells you what's there, what's queued, and what's
  loose. Hooks remind you at commit time. It's there so you can pick
  up where the last session left off, instead of guessing.

START HERE

  req brief                    one-line summary of where the project is
                               right now. Run this first in any session.
                               Now leads with the project's `_purpose`
                               (REQ-0111) plus its top three Must/Verified
                               requirements — the spine — so you learn
                               what the project is FOR before what's queued.
  req list                     full list of requirements with status
  req show REQ-0007            details + history for one requirement
  req next                     suggests what to work on, dependency-aware

WHEN THE USER ASKS FOR SOMETHING NEW

  req add --title \"...\" \\        record the requirement BEFORE you write
          --statement \"...\" \\    the code. The statement should have a
          --rationale \"...\" \\    modal verb (shall / must / should / will)
          --kind functional \\    and describe one obligation. The validator
          --priority must \\      tells you if it doesn't.
          --accept \"...\"

  Then drop a `// REQ-NNNN:` comment in the file that implements it.
  When you commit, the pre-commit hook checks that source files cite
  the REQs they implement. The post-commit hook prints a one-line
  summary so you see what landed.

WHILE YOU WORK

  req coverage --path src      where are the markers? what's orphaned?
  req validate                 are the requirements well-formed?
  req lint                     softer audit (rationale length, etc.)
  req precheck                 run the local CI gate suite (REQ-0114) —
                               fmt + clippy + test + validate + coverage
                               + review, in CI's order. Catches the
                               environment-skew failures (rustfmt drift,
                               fixture-config flakiness) that otherwise
                               only show up after push.

WORKING WITH AN EXISTING PROJECT (RETROFIT)

  req adopt REQ-0001 REQ-0002  walk a list of requirements through the
                               lifecycle to Verified in one invocation
                               (REQ-0109). One history entry per hop,
                               auto-placeholder acceptance for functional
                               reqs that lack one, inspection evidence
                               recorded when the target is Verified.
  req adopt --all-drafts       same, scoped to every requirement at Draft.
  req adopt --to implemented   stop short of Verified.
  req adopt --dry-run          show the plan without writing.

  The retrofit path matters because the lifecycle state machine exists
  to make ongoing work disciplined — not to make loading existing state
  painful. `req adopt` is the explicit acknowledgement that those are
  two different modes.

WHEN YOU FINISH SOMETHING

  req update <id> --status implemented --reason \"...\"

  Then VALIDATE it before claiming Verified. Don't one-shot it — walk
  the validation dossier so the pass/fail is backed by real analysis
  and testing (REQ-0139):

    req validation plan     <id> --plan \"how I'll review + test this\"
    req validation analysis <id> --findings \"code-review notes\" --result pass
    req validation test     <id> --findings \"what I ran\" --result pass
    req validation conclude <id> --statement \"why this passes\" --promote

  `conclude` derives the verdict (Pass only when BOTH analysis and
  testing passed) and `--promote` flips status to Verified. Promotion
  is BLOCKED without a passing dossier — this holds for `req verify`
  and `req sreq verify --promote` too. A trivial ordinary requirement
  can carry a `validation-exempt` tag (or use `req verify --no-dossier
  --reason \"...\"`); safety requirements have no exemption. Works on
  both REQ-NNNN and SR-NNNN ids.

  The post-commit hook nudges you about advancing status — if you
  cited a REQ but didn't advance it, the hook prints a suggestion.

  CODE CHANGED LATER? The dossier anchors a hash of the linked source,
  so `req stale` flags a Verified item whose code moved since you
  validated it. Re-validate with `req validation plan <id> --reopen
  --reason \"...\"`.

HOW THE FILE IS PROTECTED

  `project.req` is just JSON — there's nothing at the filesystem
  level stopping you from opening it in an editor. The contract is
  *post-hoc*: every change passes through an integrity hash, and
  any edit that didn't go via the CLI will fail your next `req`
  call until `req repair --confirm-direct-edit` re-signs it.
  Agents that bypass the CLI don't break anything silently — they
  just trigger a visible repair audit on the next operation.

  This isn't about gatekeeping you. It's so the diff in any PR
  reflects something the CLI was willing to record — the
  guarantee humans rely on when reviewing.

RULES THAT MATTER (the short list)

  * One obligation per requirement (the validator catches compounds).
  * A normative modal verb in every statement.
  * Pass `--reason` on every update so history attributes the why.
  * `// REQ-NNNN:` markers in source link spec to code.
  * Status only goes forward one step at a time; backwards needs
    `--force --reason`. Same for skips.

NEW SESSION? RUN THIS FIRST.

  req brief

  That tells you where the project is. From there, `req next` to
  pick something up or just start fixing what the user described.

INSTALL THIS GUIDANCE

  req help agents --install      writes a managed block into AGENTS.md
                                 (between sentinel markers — idempotent,
                                 re-run any time to refresh).

MCP (Model Context Protocol)

  An MCP server is built in. Run `req mcp` and connect from an
  MCP-capable client (Claude Code, etc.). Or `req mcp --init-config`
  to write `.mcp.json` for auto-launch. The full surface (25 tools)
  is documented at `req help mcp`.

ONE-LINE BOOTSTRAP FOR A NEW PROJECT

  req setup     # init + hooks + AGENTS.md, all in one.",
    },
    Section {
        name: "web",
        summary: "Running the local web server for humans.",
        body: "`req serve --host 127.0.0.1 --port 7878` starts a minimal HTML
browser/editor. Pass `--read-only` to disable mutation endpoints.

Endpoints:
  GET  /                list view (HTML)
  GET  /r/{id}          single requirement (HTML)
  GET  /api/list        JSON list
  GET  /api/r/{id}      JSON single
  POST /api/update/{id} JSON patch (disabled if --read-only)
  POST /api/add         JSON body (disabled if --read-only)",
    },
    Section {
        name: "tui",
        summary: "The interactive terminal UI.",
        body: "`req tui` opens an interactive menu that mirrors the agent-relevant
CLI commands so a human can drive the tool without memorising flags.
The current menu covers: browse, status, next, add, update, link,
delete, validate, coverage, stale, doctor, diff, audit, export,
version, quit. REQ-0083 obliges the menu to stay one-to-one with the
CLI's agent-relevant subset; a parity test fails the build if a new
command lands without a menu entry.

Built on dialoguer — works on any terminal, no full-screen TUI.",
    },
    Section {
        name: "integration",
        summary: "Wiring `req` into a real project, including CI.",
        body: "Put `project.req` at the repo root. Keep `AGENTS.md` next to it so
agents pick up the workflow on first read.

PER-CLONE SETUP

  req hooks install                  # pre-commit (default mode)
                                     # + .gitattributes
  req hooks install --strict         # strict mode: hunk-level marker
                                     # check (REQ-0100)
  req hooks install --claude-code    # also writes .claude/settings.json
                                     # (allowlist + Stop hook)

`req hooks install` writes `.git/hooks/pre-commit` that runs:

  1. `req validate` on every staged `.req` file (integrity + rules).
  2. `req review --staged --gate` on every commit with staged files
     (catches new code without a REQ marker).

The pre-commit gate has two modes:

  DEFAULT (file-level)
    A staged source file passes if ANY of its lines carry a valid
    `// REQ-NNNN:` marker. Good baseline; catches new files with no
    spec but lets in-file edits through. Use this when your project
    treats one marker per file as sufficient.

  STRICT (hunk-level, `--strict`)
    A staged source file passes only if a valid marker appears
    within 50 lines of every changed hunk. Use this when you want
    every meaningful edit to cite the requirement it implements.

Both modes honour `REQ_SKIP_GATE=1 git commit ...` for genuine
WIP / merge / rebase commits — the env var leaves a trace in
shell history rather than being silent. Re-running
`req hooks install` (with or without `--strict`) swaps modes
deterministically.

PER-COMMIT vs WHOLE-PROJECT FINDINGS (REQ-0131)

  The pre-commit gate runs `req review --staged`, which implies
  `--new`: the validator section is scoped to requirements ADDED or
  CHANGED by this commit. You are NOT shown — and not blocked by —
  compound-statement warnings on requirements you never touched. A
  linter that reprints the same six backlog warnings every commit
  trains you to stop reading; `--new` keeps the per-commit signal
  about THIS change.

  Whole-project error enforcement is unaffected. Step 1 above runs
  the full `req validate` whenever a `.req` file is staged, and CI
  runs it on the whole project — a structurally broken spec still
  cannot be committed or merged. `--new` only quiets the advisory
  backlog at the per-commit boundary.

    req review              # whole project (advisory, default)
    req review --all        # whole project, explicit name
    req review --new        # only findings this range introduced
    req review --staged     # staged diff; implies --new

  Run `req review --all` when you want the deliberate hygiene sweep
  over the existing spec — a chosen action, not a per-commit tax.

HONEST OPT-OUT FOR NON-SPEC FILES (REQ-0132)

  Some source files legitimately implement no requirement — a 50-line
  diagnostic script, a throwaway harness. Citing a tangentially
  related REQ is dishonest, and `REQ_SKIP_GATE=1` bypasses the whole
  gate. Instead, declare the exemption in the file:

    // REQ-NONE: one-off flash-timing probe, not shipped

  A `REQ-NONE` comment with a NON-EMPTY reason satisfies the marker
  gate for that file. The reason stays in the diff and is surfaced
  under \"Gate opt-outs (REQ-NONE)\" in `req review --all`, so you can
  later audit where — and why — people opted out. A bare `REQ-NONE`
  with no reason does NOT pass: the honesty is the point.

`req hooks install` also adds these `.gitattributes` lines:

  *.req merge=req-merge        # merge driver for ID collisions
  project.req -text eol=lf     # line-ending pin (Windows autocrlf
  *.req       -text eol=lf       cannot break the integrity hash)

Activate the merge driver once per clone:

  git config merge.req-merge.name 'req merge driver'
  git config merge.req-merge.driver 'req renumber --base %O || true'

Confirm the setup any time:

  req doctor                          # exits non-zero if anything missing

CI / BUILD INTEGRATION

  Three commands belong in any CI pipeline; this project wires
  exactly these in .github/workflows/ci.yml:

  # GATING — fail the build on any of these
  req validate                                     # zero errors required
  req coverage --strict \\
    --allow REQ-XXXX --allow REQ-YYYY              # orphan/ghost gate;
                                                   # whitelist verification-only
                                                   # or policy-only requirements
  req review --gate --no-defects                   # fail if ANY requirement's
                                                   # latest test record is a Fail
                                                   # (REQ-0126) — blocks shipping
                                                   # known-broken behaviour

  # ADVISORY — print but don't fail
  req doctor                                       # per-clone health
  req stale --path .                               # records vs current HEAD

  Optionally enforce signed commits on the spec file in CI:

  req audit --gate --require-good-signature        # exits non-zero if any
                                                   # commit touching project.req
                                                   # lacks a verifiable signature

PR-COMMENT NARRATIVE (REQ-0108)

  Reviewers want the spec impact inline in the PR UI, not a CLI
  command they have to remember to run. Copy this project's
  `.github/workflows/spec-review.yml` into your own repo to get
  a single bot comment per PR carrying the `req review` markdown.

  The comment is updated in place on re-pushes (not stacked). Large
  reports are truncated at 60k chars with a link to the workflow
  artefact for the full file.

  Doesn't gate. The `ci` workflow's `req review --gate` step is
  what fails the build; this one is just the human-readable layer.

CROSS-LINKING CODE TO REQUIREMENTS

  Drop `// REQ-NNNN` comments in your source. Then:

  req coverage                # orphans / ghosts / test-only / obsolete-in-code
  req coverage --by-file      # per-file -> REQ IDs
  req coverage --unlinked-files   # code files lacking any marker
  req coverage --remap REQ-OLD=REQ-NEW --apply    # rewrite markers in source

CODE REVIEW

  req diff origin/main..HEAD  # per-requirement changes since the base ref
  req check origin/main       # incremental validate + coverage scoped to
                              # files changed since the ref

LOCAL CI EQUIVALENT (REQ-0114)

  req precheck runs the exact gate suite CI runs, in CI's order, with
  one invocation. Wire it into your editor's save action or a pre-push
  hook to catch format/clippy/test drift in the same loop where the
  code lives — before pushing.

    req precheck                          # full suite, stops on first fail
    req precheck --skip clippy            # repeat --skip for tight loops
    req precheck --keep-going             # report every failure, not just first

  Steps, in order:

    1. cargo fmt --all -- --check
    2. cargo clippy --all-targets -- -D warnings
    3. cargo test --all
    4. req validate
    5. req coverage --strict
    6. req review --gate

  Exits non-zero on the first failure (or after running all when
  --keep-going is set) and points to which step blew up. Three of the
  0.3.2 CI failures were rustfmt or test-fixture drift that this would
  have caught locally.",
    },
    Section {
        name: "version-control",
        summary: "Diffs, merges, ID collisions.",
        body: "The .req file is pretty-printed JSON specifically so it diffs
cleanly in code review. The merge story has three moving parts:

  1. _integrity hash. A text merge produces a file whose hash no longer
     matches. The merge driver runs `req renumber --base %O` which
     force-loads, fixes ID collisions, and re-signs the file. If you
     resolved conflicts by hand, run `req repair --confirm-direct-edit`.

  2. ID collisions. Two branches that both ran `req add` while diverged
     allocate the same REQ-NNNN. After merging, run:

        req renumber --base origin/main

     This loads the base from git, finds requirements on your branch
     whose IDs are taken upstream, and shifts them to fresh IDs.
     Internal links are rewritten and a history entry records the move.

  3. Content conflicts (same requirement edited on both sides). These
     are real conflicts — resolve in your editor, then run
     `req repair --confirm-direct-edit` to re-sign.

CROSS-REPO ID NAMESPACING (convention, not enforced)

  REQ IDs are allocated per repo, so REQ-0008 means one thing here and
  something unrelated in another component's `project.req`. The tool
  deliberately does NOT impose a global namespace — repo-local scoping
  is the right granularity, and each component owns its own spec.

  When you reference a requirement that lives in ANOTHER repo (a commit
  message, a cross-cutting note, an issue), qualify it:

        at_test_runner#REQ-0008      not just REQ-0008

  This is a writing convention to keep humans unambiguous across a
  multi-repo workspace; `req` does not parse or validate the prefix.",
    },
    Section {
        name: "audit",
        summary: "Who changed which requirement, and was it signed?",
        body: "`req audit` walks `git log --follow` on the .req file and reports
each commit alongside its GPG/SSH signature status and signer name. Use
this in regulated environments to prove provenance:

  req audit                   # last 50 commits
  req audit -n 200 --json     # machine-readable

Signature codes mirror git's %G?:
  good           verified signature, key trusted
  good-unknown   verified, key not in your trust store
  bad            signature does not match
  expired        key expired
  revoked        key revoked
  cannot-check   key unavailable
  no-signature   commit was not signed

The _integrity hash inside the file is for *integrity* (the CLI wrote
it last) not *authenticity* (a trusted human approved it). Lean on
signed commits for the latter.",
    },
    Section {
        name: "format-policy",
        summary: "How `_format` versions evolve and what guarantees the CLI makes.",
        body: "`project.req` carries a `_format` tag at the top of the file
(currently `req-v2`). This section pins down what changes that tag
and what to expect when it does.

CURRENT VERSION

  req-v2 — the current released format. All 0.4.x binaries read and
  write it. The schema is:
    _warning / _instructions   informational, ignored by the loader
    _format                    schema tag (required)
    _integrity                 sha256 of canonical payload (required)
    _purpose                   optional, project purpose statement (REQ-0111)
    _config                    optional, per-project configuration (REQ-0110)
    name / created / updated   project metadata
    next_id                    monotonic ID counter
    requirements               map of REQ-NNNN to Requirement objects

PRIOR VERSIONS

  req-v1 — shipped in 0.1.0 through 0.3.2. Lacked `_purpose` and
  `_config`. Files at v1 must be migrated with `req migrate`; the
  CLI refuses to load them directly to avoid silently mis-reading.

WHEN THE TAG BUMPS

  We bump `_format` only for changes that cannot be expressed as
  backwards-compatible additions. Backwards-compatible additions —
  new optional fields with a serde default, new enum variants used
  only in newer files — DO NOT bump the tag. Examples of changes
  that WOULD bump:
    * removing or renaming an existing field
    * changing the canonical-hash rule
    * changing how `next_id` is interpreted

ENCOUNTERING AN OLDER FORMAT

  The CLI refuses to load a file whose `_format` is older than the
  one it understands, and tells you to run:

    req migrate

  `req migrate` writes a sibling backup (project.req.bak-<oldver>),
  upgrades the file in place, recomputes `_integrity`, and appends
  a synthetic history entry to each requirement whose shape changed.

ENCOUNTERING A NEWER FORMAT

  The CLI refuses to load a file whose `_format` is NEWER than its
  own. The error advises upgrading the binary rather than silently
  attempting to read it — silent reads of unknown-shape files are
  the bug class this policy exists to prevent.

DOWNGRADE

  The CLI does not provide a downgrade path. Once a file is migrated
  to a newer format, the prior `.bak` is the only way back; restoring
  it requires no special tooling (it's just JSON on disk).

PROMISE

  No `_format` bump will silently change semantic behaviour of an
  already-stored requirement. Migrations preserve per-requirement
  history; new fields synthesize a single history entry recording
  the migration.

OBLIGATION ON FUTURE SCHEMA CHANGES

  The first commit that bumps `_format` to `req-v2` (or later) MUST
  also register a migration body in `src/commands/migrate.rs` —
  shipping a new format without a path forward from `req-v1` is a
  release-blocking bug. The current binary intentionally errors out
  if it encounters an older format it has no migration for, rather
  than silently passing the data through.",
    },
    Section {
        name: "env",
        summary: "Environment variables read by the tool.",
        body: "REQ_FILE         Override the default .req file path. Equivalent
                 to passing --file PATH.

REQ_ACTOR        Override the actor name recorded on history entries.
                 Falls back to USER / USERNAME.

REQ_ACTOR_KIND   Tag history entries (and downstream audit output) with
                 'human' or 'agent'. Defaults to 'unknown' if unset.
                 Agents driving req over MCP or CLI should set this to
                 'agent' so reviewers can separate human vs automated
                 edits when auditing.

REQ_VALIDATE_LLM_CMD
                 OPTIONAL statement-quality hook. When set, `req
                 validate` invokes this command once per non-obsolete
                 requirement and surfaces the verdict as REQ-V-0023.
                 The command is run via the platform shell (`sh -c`
                 on Unix, `cmd /C` on Windows).

                 Contract:
                   - stdin: JSON object {id, title, statement, rationale}
                     followed by EOF (we close the pipe immediately so
                     a `read_to_end()` hook returns at once).
                   - stdout: JSON {ok: bool, message: string}.
                   - exit 0 expected; non-zero surfaces as a transport
                     warning (not a validate error).
                   - 10s hard timeout per requirement; timeouts surface
                     as transport warnings, validate continues.
                   - ok: true is silent; ok: false produces a
                     REQ-V-0023 warning carrying `message`.

                 Example (sh): echo '{\"ok\":false,\"message\":\"too vague\"}'

                 Notes:
                   - Calls are sequential, so a 1s hook × 100 reqs is
                     ~100s. Cache verdicts by sha256(statement) in your
                     hook if you call a paid model.
                   - Default validator stays deterministic and offline
                     when this var is unset.

REQ_VALIDATE_LLM_SHELL
                 Override the shell used to invoke the hook. Default is
                 `sh` on Unix and `cmd` on Windows. Set to `bash`,
                 `pwsh`, or `sh` to run shell-script hooks on Windows
                 when Git Bash or PowerShell are on PATH and cmd.exe is
                 not finding the interpreter.

REQ_VALIDATE_LLM_CONCURRENCY
                 Maximum number of hook invocations in flight at once.
                 Default 1 (sequential, matches 0.2.x behaviour). Set
                 to a small integer (e.g. 4) to fan out across reqs
                 when your hook calls a slow remote model. Findings
                 are sorted by id afterwards so output order is stable
                 regardless of completion order. The 10s timeout is
                 per-call, not aggregate.",
    },
    Section {
        name: "verification",
        summary: "What it takes to mark a requirement as Verified.",
        body: "Verified means there is a HEAD-current evidence record on file.
Evidence comes in three kinds, all stored on the same TestRecord
shape with a `kind` discriminator:

  automated     captured by `req test run` from a cargo test suite
  composition   verified by citing another requirement's passing tests
                (e.g. REQ-0020 'agents shall not edit directly' is
                discharged by REQ-0003's integrity-hash test)
  inspection    verified by human review of the code at the recorded
                commit (use when the requirement is a negative
                assertion or pure policy not testable in code)

Every record pins the current git HEAD SHA. A record is FRESH if its
commit matches current HEAD. `req show REQ-XXXX` annotates the latest
record with [matches HEAD] or [drifted — HEAD now ...] so reviewers
see staleness at a glance.

RECORDING

  # automated — usually via the runner
  req test run --promote                     # runs cargo test + flips
                                             # status to Verified for any
                                             # req with a fresh pass

  # composition — cite tests/reqs that imply this one
  req verify REQ-0020 --by composition \\
    --cites REQ-0003 --cites req_0003_integrity_blocks_load_after_semantic_tamper \\
    --notes \"agents-shall-not-edit-directly is enforced by integrity hash\" \\
    --promote

  # inspection — human review at HEAD
  req verify REQ-0028 --by inspection \\
    --notes \"reviewed src/ for crypto deps; only sha2 used for hashing\" \\
    --promote

POLICY

  * Verified requires fresh evidence on the current HEAD.
  * Composition records should name the cited test or REQ in --cites.
  * Inspection records should describe what was reviewed; if the
    code at the cited commit has drifted, re-inspect or re-verify.
  * `req test run --promote` is safe to wire into CI: it only
    flips Implemented -> Verified, never the other way.

STALENESS HAS TWO LEVELS

  fresh        record commit equals current HEAD
  drifted      HEAD moved but none of the requirement's linked source
               files (files containing its REQ-NNNN marker) changed
  STALE        at least one linked file changed since the record commit

`req show` annotates the latest record with the strongest applicable
tag. `req stale` reports the same three states across the whole
project; `req stale --only-stale` exits non-zero if anything is
actually stale (CI-friendly).

When HEAD moves, drifted/STALE records remain visible in `req show`
and `req stale` but do NOT automatically demote status. Re-running
`req test run --promote` lands fresh records; STALE entries become
fresh when their evidence is re-affirmed.",
    },
    Section {
        name: "lint",
        summary: "Project-wide quality audit beyond the validator.",
        body: "`req lint` is to `req validate` what `clippy` is to `rustc`: same
domain, softer signal, opt-in by running the command. The validator
gates ship; lint surfaces things you might want to fix that wouldn't
block a release.

WHAT LINT REPORTS

  validator findings    Same as `req validate`, included for context.
  markerless_active     Non-Draft, non-Obsolete requirements with no
                        `// REQ-NNNN:` reference in the scanned source
                        tree. May be verification-only or policy meta-
                        reqs that legitimately carry no code marker —
                        document the exception in the rationale.
  short_rationale       Active rationales under 10 words. The validator
                        catches very short ones (REQ-V-0013); lint
                        catches the 3-9 word band the validator lets
                        through. De-duped against the validator so a
                        single req is never flagged twice.
  single_acceptance     Functional requirements with one or zero
                        acceptance criteria. Functional reqs usually
                        deserve multiple observable checkpoints.
  no_test_record        Proposed-or-later requirements with no evidence
                        record at all. Three legitimate evidence
                        channels: `req test record`, `req verify --by
                        inspection`, `req test run --promote`.
                        Requirements tagged `inspection-only` are
                        excluded — use that tag for things that are
                        the spec but aren't unit-testable (CORS rules,
                        no-cache headers, audit policies, etc.).
  verification_kinds    Distribution across automated / composition /
                        inspection. Informational. A composition or
                        inspection desert may mean over-reliance on
                        end-to-end tests.

OUTPUT MODES

  req lint                   Markdown to stdout (review-friendly).
  req lint --json            Machine-readable; pipe to jq / CI.
  req lint --path src        Restrict marker scan to a subdirectory.

EXIT CODE

  Reflects validator errors only. Quality observations NEVER gate.
  Zero exit on a healthy project. Non-zero only when `req validate`
  would also fail.

CI USE

  Lint is informational by design; do not gate on it. Print it as a
  PR comment or upload as a workflow artefact alongside `req review`.
  If you want a gate, raise it via the validator (add a new REQ-V
  rule), not via lint.

WHEN TO RUN

  Before each release.
  After a `req split` to confirm the children stand on their own.
  Quarterly for projects in maintenance mode — short rationales and
  missing evidence drift in over time.",
    },
    Section {
        name: "testing",
        summary: "How to wire cargo tests into requirement test records.",
        body: "Convention: name every #[test] function `req_NNNN_description` where
NNNN is the 4-digit ID of the requirement it exercises. Multiple tests
may share an ID — they are aggregated into one record per run.

  #[test]
  fn req_0006_modal_verb_required_rejects_missing() { ... }
  #[test]
  fn req_0006_modal_verb_present_passes() { ... }

Then drive the suite through `req test run`:

  req test run                          # default: cargo test --release
  req test run --dry-run                # preview without writing
  req test run --json                   # machine-readable result map
  req test run --cmd \"cargo test\"       # custom command

The runner shells out, parses stdout for `^test req_NNNN_* ... (ok|FAILED|ignored)`,
groups by REQ-NNNN, and attaches one TestRecord per requirement:
outcome = fail if any covered test failed, else pass. Each record
captures the current git HEAD SHA, the actor (REQ_ACTOR), the test
name list, and a UTC timestamp.

Effect on the project:
  * `req show REQ-NNNN` displays the run with a [matches HEAD] /
    [drifted] marker, so reviewers can see at a glance whether the
    evidence is current.
  * Test records round-trip through the integrity hash.

The runner ignores test names that don't match the convention and
skips records for REQ-IDs that no longer exist in project.req
(orphan markers in code).",
    },
    Section {
        name: "errors",
        summary: "Stable error and rule codes for agents and tooling.",
        body: "Every CLI subcommand that supports --json emits its error envelope
as a single JSON object on stdout (the conventional channel for
tool-readable data — REQ-0039), with three fields:

  { \"code\": \"REQ-E-...\", \"message\": \"...\", \"hint\": \"...\" }

ERROR CODES (stable)

  REQ-E-INTEGRITY       File integrity hash mismatch. Hint names
                        `req repair --confirm-direct-edit`.
  REQ-E-NOT-FOUND       Referenced requirement / file / ref does not exist.
  REQ-E-VALIDATION      Validator rejected the input.
  REQ-E-CYCLE           A parent link would create a cycle.
  REQ-E-DUPLICATE       Link already exists, or another uniqueness clash.
  REQ-E-INVALID-INPUT   Malformed arguments or unknown enum value.
  REQ-E-IO              File or process error (read/write/exec).

VALIDATOR RULE CODES (stable)

{{RULE_CODES}}

Codes are append-only — adding a code is backwards compatible; renumbering
existing codes is NOT. Agents may match on codes and treat messages as
informational text.",
    },
    Section {
        name: "mcp",
        summary: "Run `req` as an MCP server for LLM agents.",
        body: "`req mcp` speaks the Model Context Protocol over stdio: each line
is a JSON-RPC 2.0 message (newline-delimited, no Content-Length header).
Pair it with any MCP-capable client.

BOOTSTRAP — once per project

  req mcp --init-config           # writes .mcp.json at the repo root
  req mcp --init-config --path .somewhere/req.json --force

The generated .mcp.json registers a server named `req` with a
description aimed at agents on first contact. Edit it freely OUTSIDE
the values you want to keep.

TOOLS EXPOSED

  req_list       List requirements with filters. Call FIRST.
  req_show       Full detail for one ID (statement, rationale, ACs, history).
  req_add        Create. Validator rejects bad input — rewrite, don't bypass.
  req_update     Modify; `reason` mandatory. Prefer add_acceptance over
                 acceptance (append vs replace).
  req_delete     Soft by default (status→Obsolete). hard=true refuses if
                 inbound links exist.
  req_link       parent / depends_on / refines / conflicts / verifies.
                 Parent links cycle-checked.
  req_validate   Run rules across the whole project.
  req_coverage   default / unlinked_files=true / by_file=true modes.
  req_export     markdown / json (csv & html via CLI only for now).
  req_help       Fetch any documentation section by name.

NOT EXPOSED

  `repair` is human-only. The integrity-recovery escape hatch is
  deliberately not on the agent surface so an agent cannot accept an
  integrity violation and continue.

SCHEMAS

  Every tool has a JSON Schema in its description that names required
  vs optional fields, enumerated values for kind/priority/status, and
  short per-field descriptions. Honour the schema; the server validates.

DEBUGGING

  Pipe JSON-RPC into the binary directly:

    printf '%s\\n%s\\n' \\
      '{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\"}' \\
      '{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\"}' \\
      | req mcp

  Each response is a single JSON line on stdout. Notifications (no `id`)
  produce no response.",
    },
    Section {
        name: "export",
        summary: "Export formats.",
        body: "  markdown   human-friendly, default
  json       full project including history
  csv        spreadsheet-friendly summary
  html       single-file HTML

Use `-o -` to write to stdout (default), or `-o reqs.md` to a file.",
    },
    Section {
        name: "safety",
        summary: "Authoring hazards, safety functions, and safety requirements (IEC 61508).",
        body: "Functional-safety work in `req` follows the IEC 61508 chain. Four
artifacts, each with its own id space:

  HAZ-NNNN  Hazard            a hazardous event; risk-assessed to a SIL
  SF-NNNN   Safety function   the measure that reaches/keeps a safe state
  SR-NNNN   Safety requirement  a normative obligation realizing a function
  REQ-NNNN  Requirement       ordinary requirements, unchanged

ENABLING THE FEATURES — a human signs on first. The safety features are
OFF until a person accepts the liability disclaimer:

  req safety accept --name \"Your Name <you@example.com>\"

This writes `req-safety-acceptance.json` beside project.req — COMMIT it.
Its presence (for the current disclaimer version) is what activates
hazards / safety functions / safety requirements; delete it and safety
mutations stop. This is deliberately a HUMAN action: `req safety` is not
on the agent/MCP surface, and `accept` refuses when REQ_ACTOR_KIND=agent.
An agent can author hazards/SF/SR once a human has signed on, but it can
never accept on your behalf. `req safety status` shows the current state.

HUMAN CONFIRMATION OF SAFETY VALIDATION (REQ-0145). An agent may author and
validate a safety requirement's dossier (analysis + testing), but the result
is NOT considered passed until a HUMAN co-signs it:

  req validation confirm SR-0007

`confirm` refuses `REQ_ACTOR_KIND=agent`. A Verified safety requirement that
carries an agent's dossier but no human confirmation is flagged `REQ-V-0034`
until a person runs it — so safety verification always has a human in the loop.

CALIBRATION — the risk-graph table is the IEC 61508-5 Annex D worked
example, which the standard says you must calibrate per project/sector.
Override only the leaves that differ; the rest keep the default:

  req safety calibrate --label \"Acme rail rev 3\" \\
    --set \"C_D/F_B/P_B=W3:4,W2:3,W1:2\"
  req safety calibrate --show      # current calibration
  req safety calibrate --reset     # back to the Annex D default

Every SIL req then derives uses your calibration.

THE WORKING RULE: you never type a SIL. It is DERIVED — from the hazard's
risk parameters, then aggregated up the chain. There is no `--sil` flag,
by design. This stops casual fudging and fat-finger errors; it does not
make the result objective — the SIL is only as sound as the four letters
you choose (see WHAT REQ DOES NOT DO).

  hazard risk (C/F/P/W)  ─►  required SIL        (per hazard)
  max over its hazards   ─►  allocated SIL       (per safety function)
  inherited from its SF  ─►  governs verification rigour (per safety req)

AUTHORING A HAZARD — write the harm first, in plain words.

  req hazard add \\
    --title \"Blade restarts during cleaning\" \\
    --harm \"an operator's hand could be severed\" \\
    --context \"maintenance with the guard removed\"

  `harm` is free text and is NOT the same as the severity class. Write
  what actually happens to a person. Then risk-assess:

  req hazard assess HAZ-0001 -C C_D -F F_B -P P_B -W W2

  The four IEC 61508-5 risk-graph parameters:
    C  consequence     C_A minor · C_B serious/one death · C_C several ·
                       C_D many killed
    F  exposure        F_A rare→occasional · F_B frequent→continuous
    P  avoidance       P_A possible · P_B almost impossible
    W  probability     W1 very slight · W2 slight · W3 relatively high
  These derive the required SIL (— / a / SIL1..4 / b). Pick honestly and
  have a competent assessor review the choice: the classification is
  where the safety argument STARTS, not the whole of it. When unsure of
  C, let the worst credible outcome in `harm` guide you.

  CALIBRATION: the table req uses is the *worked example* from IEC
  61508-5 Annex D. The standard is explicit that a risk graph must be
  CALIBRATED for your project/sector (the boundaries between SIL bands
  are an organisational risk-acceptance decision). Treat req's output as
  the default-calibration result and confirm — or recalibrate — it
  against your own scheme before relying on it.

DRIVING OUT SAFETY FUNCTIONS — one per independent way to reach a safe
state. Link it to the hazard it mitigates; its allocated SIL appears
automatically as the worst of the hazards it covers.

  req sf add --title \"Guard interlock halts blade\" \\
    --safe-state \"blade de-energised within 200ms\" --mitigates HAZ-0001

DRIVING OUT SAFETY REQUIREMENTS — atomic, testable, `shall`. Each
realizes a safety function and inherits its SIL.

  req sreq add --title \"Interlock cuts power <=200ms\" \\
    --statement \"The interlock shall cut blade power within 200 ms of \\
guard opening.\" --rationale \"Bounds exposure to a moving blade.\" \\
    --accept \"Power removed <=200ms on the bench rig\" --realizes SF-0001

  Mark the implementing code with `// SR-NNNN:` just as you would a
  requirement, then record evidence. Prefer evidence PRODUCED BY A RUN
  over a hand-asserted record: name the test `sr_NNNN_*` and let
  `req test run` attach the result, stamped with the commit and a hash
  of the linked files —

  req test run --promote        # maps sr_0001_* -> SR-0001, records pass/fail

  so `req stale` flags the SR when its code later changes (a SIL 3/4
  \"automated\" claim that has gone stale is no longer trustworthy). Wire
  `req stale` into CI to keep the safety evidence honest. The manual form

  req sreq verify SR-0001 --by automated --notes \"bench rig log\" --promote

  is fine when there is no automated test to point at.

THE VERIFICATION GATE — a SIL 3/4 safety requirement CANNOT reach
Verified on inspection alone. Provide automated or composition
evidence. If you genuinely must accept inspection, `--force` records an
AUDITED exception (it is logged and re-flagged at every `req validate`).
Do not reach for `--force` to make a red gate green; fix the evidence.

SEEING THE WHOLE PICTURE — `req trace` is the single best command. Given
any HAZ/SF/SR id it prints the end-to-end chain and a traceability roll-up:

  req trace HAZ-0001        # linked? verified? what's blocking?

  TRACE STATUS is COMPLETE when every link is present and every realizing
  safety requirement is Verified with evidence whose rigour meets its SIL.
  The SIL line is an *allocation* check (allocated ≥ required). `req
  validate` enforces the same rules, so a broken chain fails CI.

  Read \"complete\" as *traceability complete*, NOT \"safe\". See the
  limits below before you treat a green trace as assurance.

WHAT THE VALIDATOR WILL HOLD YOU TO (REQ-V-0025..0031):
  • a hazard needs a harm narrative;
  • an assessed hazard needs all four C/F/P/W;
  • a mitigated hazard needs a live safety function;
  • mitigates/realizes links must resolve;
  • a Verified safety requirement needs passing evidence of adequate
    rigour for its SIL.
Don't argue with the validator — assess, link, and verify properly.

WHAT REQ DOES NOT DO — and you must not let it imply otherwise:

  • req is NOT a qualified safety tool. Per IEC 61508-3 §7.4.4 (and
    ISO 26262-8), a tool whose output you rely on without independent
    verification needs a tool-confidence/qualification argument. req
    provides none. If you work to a standard, you own that gap — qualify
    it yourself or independently check every SIL it computes.
  • The SIL it shows is a CANDIDATE derived from four letters you chose.
    \"Derived, never typed\" stops fat-finger and casual fudging; it does
    NOT launder out the subjectivity of picking C/F/P/W. Pick honestly,
    and have the classification reviewed by someone competent.
  • req tracks REQUIRED/allocated/inherited SIL — the *target*. It does
    NOT model ACHIEVED integrity: no PFD/PFH, no diagnostic coverage, no
    safe-failure-fraction, no systematic capability, no hardware fault
    tolerance. \"allocation >= required\" is not evidence of risk reduction.
  • No SIL decomposition or independence modelling. If you mitigate one
    hazard with several independent functions, req stamps each with the
    full SIL; apportioning integrity across redundant elements is your
    judgement to make and record outside req.
  • req is the wrong instrument for the analysis itself (HARA/HAZOP,
    FMEA, FTA, quantification). It is a place to RECORD and TRACE the
    results of that analysis so they survive between sessions and stay
    linked to code — a requirements tool with a safety-aware workflow,
    not a safety-engineering tool.

  Treat req's output as an organised aid for a competent assessor, never
  as the assessment.",
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    // REQ-0045: `req help best-practice` (and `errors`) list EVERY validator
    // rule code with its meaning, rendered from the single source of truth so
    // they cannot drift behind the rules the validator emits.
    #[test]
    fn req_0045_help_lists_every_validator_rule_code() {
        let errors = render_body(section("errors").unwrap().body);
        let bp = render_body(section("best-practice").unwrap().body);
        for (code, _desc) in crate::validate::RULES.iter() {
            assert!(errors.contains(*code), "errors help is missing {code}");
            assert!(bp.contains(*code), "best-practice help is missing {code}");
        }
    }

    // REQ-0093: the REQ-V-0019 catalogue entry documents the Implemented-or-later
    // precondition (the rule is suppressed below Implemented).
    #[test]
    fn req_0093_help_documents_v0019_precondition() {
        let errors = render_body(section("errors").unwrap().body);
        let line = errors
            .lines()
            .find(|l| l.contains("REQ-V-0019"))
            .expect("REQ-V-0019 is listed");
        assert!(
            line.to_lowercase().contains("implemented"),
            "REQ-V-0019 entry must document the Implemented precondition: {line}"
        );
    }

    // REQ-0089: REQ-V-0022 (stacked uncertainty hedges) appears in the errors
    // catalogue.
    #[test]
    fn req_0089_help_lists_v0022_stacked_hedges() {
        let errors = render_body(section("errors").unwrap().body);
        assert!(
            errors.contains("REQ-V-0022"),
            "errors help must list REQ-V-0022"
        );
    }

    // REQ-0126: `--no-defects` is documented alongside `req help integration`.
    #[test]
    fn req_0126_integration_help_documents_no_defects() {
        let integ = render_body(section("integration").unwrap().body);
        assert!(
            integ.contains("--no-defects"),
            "integration help must document --no-defects"
        );
    }

    // REQ-0039: the errors section states the --json envelope goes to stdout,
    // matching the implementation (it must not claim stderr).
    #[test]
    fn req_0039_errors_help_says_stdout_not_stderr() {
        let errors = render_body(section("errors").unwrap().body);
        assert!(errors.contains("stdout"), "errors help must say stdout");
        assert!(
            !errors.contains("on stderr"),
            "errors help must not claim the envelope goes to stderr"
        );
    }
}
