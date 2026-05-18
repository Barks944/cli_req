<!-- REQ-0036: project README is the canonical landing page. -->
# req

**Spec-memory for vibecoded projects.**

`req` keeps software-project requirements in a single git-tracked JSON file (`project.req`) and mediates every change through a CLI that enforces requirements best practice. The file diffs cleanly, history is append-only, and an integrity hash catches hand edits before they corrupt your spec.

It exists because **conversational coding loses track of requirements.** The agent and the user have a session, they build something, the conversation ends. Without something that survives the conversation, the next session starts blind. `req` is what survives: a tool both humans and agents reach for, with hooks that nudge at commit time and a session-start brief that says "where are we right now?"

---

## TL;DR for agents

If you're an LLM agent reading this in someone else's project, the short story:

```sh
req brief             # what's the spec? what's queued? what's loose?
req next              # what should I work on?
req add ...           # record a new requirement (before you implement)
req validate          # is everything well-formed?
```

The pre-commit hook will catch you if you write code without citing a `// REQ-NNNN:` marker. The post-commit hook prints what just landed and suggests the next status change. The full agent guide is `req help agents` — written for you, not at you.

---

## TL;DR for humans

```sh
cargo install req-cli
cd your-project
req setup             # one-shot: init + git hooks + AGENTS.md
req add -t "..." -s "The system shall ..." -r "..." -k functional -p must
```

That's it. The hooks make sure agents (and other humans) keep the spec and the code in sync. `req brief` is your dashboard; `req lint` is your quality audit; `req review --gate` is your CI check.

---

## Mission

Make software requirements a first-class, machine-checked artifact that lives next to the code it governs — readable in a diff, enforceable in CI, and equally usable by humans and LLM agents — so that a project's intent and its implementation can never quietly drift apart.

---

## Why `req`?

Requirements rot when they live in wikis, drift when they live in code comments, and become unreviewable when they live in a database. They evaporate entirely when they only existed in a chat that's now archived. `req` puts them in your repo as JSON, but stops anyone — human or agent — from editing them in ways that break the rules you'd want a senior engineer to enforce:

- **One obligation per requirement**, with a normative modal verb (`shall` / `must` / `should` / `will`).
- **Append-only history** with a required `--reason` on every change.
- **Integrity hash** so silent corruption shows up immediately, not three weeks later.
- **Code traceability** via `// REQ-NNNN` markers and `req coverage`.
- **Git-native**: pre-commit / post-commit hooks, merge driver for ID collisions, signature-based audit trail.
- **Agent-shaped**: a session-start `req brief`, an MCP server (`req mcp`), and an AGENTS.md template that explains the workflow in the agent's voice.

The validator IS the product. The CLI is the only legitimate way to mutate the file.

---

## Install

**Prebuilt binaries** — grab the archive for your platform from the [latest release](https://github.com/Barks944/cli_req/releases/latest), extract, and put `req` (or `req.exe`) on your `PATH`.

**From crates.io** (Rust toolchain required):

```sh
cargo install req-cli
```

**From source**:

```sh
git clone https://github.com/Barks944/cli_req
cd cli_req
cargo install --path .
```

### Version compatibility

After installing, check the version against any existing project file:

```sh
req --version
```

If you're joining a project that already has a `project.req`, your installed
`req` must be at least the version that last wrote the file. Older binaries
refuse to read newer formats and tell you to upgrade rather than silently
mis-reading; the symptom is an `unsupported _format` error pointing you at
this section. Pre-commit hooks and Claude Code Stop hooks invoke `req
validate`, so a stale binary will fail every commit until upgraded.

---

## Quick start

```sh
# one-shot bootstrap: init + git hooks (pre + post commit) + AGENTS.md
req setup

# session-start brief — where is the project right now?
req brief

# add a requirement (non-interactive form, agent-friendly)
req add \
  --title "Persist user sessions" \
  --statement "The system shall persist sessions across restarts." \
  --rationale "Users lose work on deploys today." \
  --kind functional \
  --priority should \
  --accept "Session survives process restart" \
  --tag auth

# see what you have
req list
req show REQ-0001

# change something — always with a reason
req update REQ-0001 --status approved --reason "Reviewed in 2026-05-17 sync"

# validate before you commit (the pre-commit hook does this for you)
req validate
```

For everything else: `req help` lists the section index, `req help <section>` drills in. The agent guide is `req help agents`.

---

## The workflow

### 1. Reference requirements from code

Drop markers where you implement them:

```rust
// REQ-0003: integrity hash verification
fn load(path: &Path) -> Result<Project> { ... }
```

Then:

```sh
req coverage --path src
```

shows you:

- **Orphans** — requirements at `Implemented` with no code reference.
- **Ghosts** — markers in code that point at non-existent or obsolete requirements.

Spec and code stay in sync, or you find out fast.

### 2. Link for hierarchy and trace

```sh
req link REQ-0026 REQ-0019 -k parent
req link REQ-0026 REQ-0019 -k depends-on
req link REQ-0030 REQ-0026 -k verifies
```

### 3. Retire, don't delete

```sh
req delete REQ-0007 --reason "Superseded by REQ-0042"
```

Soft delete by default — links and history are preserved. `--hard` is gated on having no inbound links.

### 4. Ship

```sh
cargo build --release      # or whatever your project uses
req validate               # must be 0 errors
req coverage --path src    # no new ghosts
git diff project.req       # human-readable, by design
```

If you changed behaviour that has a requirement, the diff of `project.req` and the diff of your source should tell the same story.

---

## Git integration

```sh
req hooks install
```

installs:

- `.git/hooks/pre-commit` — runs `req validate` on staged `.req` files and rejects the commit on errors.
- `.gitattributes` line — `*.req merge=req-merge` so merges run `req renumber --base %O` and auto-fix ID collisions. The command prints the two `git config` lines needed to activate the driver in your clone.

If you ever merge by hand and IDs collide:

```sh
req renumber --base origin/main
```

If someone hand-edits the file and breaks the integrity hash:

```sh
req repair --confirm-direct-edit
```

---

## Provenance & audit

`req` does not invent its own signature scheme. Authenticity comes from **signed git commits**:

```sh
git config commit.gpgsign true
req audit                  # walks git log on project.req, prints signer + signature status
req audit --json -n 500    # for tooling / compliance review
```

`req help audit` documents the signature-status codes (good / bad / expired / no-signature).

---

## Working with agents

`req` was designed with LLM agents as first-class users. Drop this into your `AGENTS.md` (or equivalent) and your agents will know how to drive the tool:

```sh
req help agents --install
```

That command writes a managed block, between sentinel markers, into `AGENTS.md`. Re-run any time to refresh; edit outside the markers freely. The block tells agents:

- Never read or write `project.req` directly.
- Every mutation goes through `req <subcommand>` with a `--reason`.
- Use `// REQ-NNNN` markers in source; `req coverage` ties spec to code.
- Don't argue with the validator — rewrite.

---

## Command surface

```
Project lifecycle
  req init -n <name> [--layout directory]   Create project.req (file or dir layout)
  req tui                                   Interactive menu (mirrors CLI surface)
  req validate                              Run all rules; 0 errors to ship
  req status                                Per-status counts + delivery_progress_pct
  req version                               Print binary version (--json for tooling)
  req export -f markdown -o reqs.md         Publish (markdown / json / csv / html)
  req serve [--read-only]                   Local web UI (HTML + /api/* JSON)
  req mcp                                   JSON-RPC stdio server for agents
  req mcp --init-config                     Write .mcp.json for MCP-capable clients
  req schema [add|batch|import]             JSON Schema for structured CLI inputs

Day-to-day
  req add ...                               Add a requirement (also: --from-json)
  req list [--status ...] [--tag ...]       Filter (Obsolete hidden by default)
  req show REQ-0007                         Full detail + history + test records
  req update REQ-0007 --status approved --reason "..."
    [--add-acceptance "..." | --remove-acceptance N]
  req link <from> <to> -k <kind>            parent | depends-on | verifies | …
  req delete REQ-0007 --reason "..."        Soft by default; --hard if no inbound links
  req next [--status ... --tag ...]         Suggest one requirement to work on
  req batch path/to/changes.json            Transactional multi-mutation
  req import -f markdown spec.md            Bulk ingest through the validator

Evidence & verification
  req test record REQ-0007 --result pass --notes "..."
  req test run [--from-file <log>] [--promote]
                                            Drive cargo test, attach records,
                                            optionally flip Implemented -> Verified
  req verify REQ-0007 --by composition --cites REQ-0003 --notes "..." [--promote]
  req verify REQ-0007 --by inspection --notes "reviewed src/..."        [--promote]
  req stale [--only-stale]                  Records vs HEAD; three-state staleness

Integration & review
  req hooks install [--claude-code]         Pre-commit + merge driver
                                            (+ .claude/settings.json allowlist)
  req doctor                                Per-clone setup audit (gates 5 checks)
  req renumber --base origin/main           Post-merge ID collisions
  req coverage [--path src]                 Orphans / ghosts / test-only / obsolete-in-code
  req coverage --by-file                    Per-file -> REQ IDs
  req coverage --unlinked-files             Code files with zero markers
  req coverage --remap REQ-OLD=REQ-NEW --apply
  req coverage --strict --allow REQ-NNNN... CI gate; non-zero on findings
  req diff origin/main..HEAD                Per-requirement changes between revs
  req check origin/main                     Incremental validate + scoped coverage
  req audit [--gate --require-good-signature --require-signer NAME]
                                            Git signature trail / CI gate
  req migrate                               Schema migration (currently a no-op stub)

Recovery
  req repair --confirm-direct-edit          After intentional hand edits

Docs
  req help                                  Section index
  req help <section>                        overview | concepts | best-practice |
                                            workflow | integration | version-control |
                                            agents | mcp | audit | testing |
                                            verification | format-policy | errors |
                                            env | file-format | tui | web | export
  req help <section> --install              Inject the section into AGENTS.md
  req help <section> --json                 Structured form for tooling
  req help all                              Everything
```

## CI / build integration

Drop these three commands into your CI pipeline. The repo's own
[.github/workflows/ci.yml](.github/workflows/ci.yml) is the reference setup.

```yaml
# Gating: any of these failing should fail the build.
- run: req validate
- run: |
    req coverage --path . --strict \
      --allow REQ-NNNN --allow REQ-MMMM     # whitelist verification-only reqs

# Advisory: print but don't fail.
- run: req doctor || true
- run: req stale --path . || true
```

`req validate` checks every requirement against the rule set (0 errors required to ship).
`req coverage --strict` turns orphan / ghost / obsolete-in-code findings into a non-zero exit.
`req doctor` audits per-clone setup — useful as a warning when contributors skip `req hooks install`.
`req stale` is informational; staleness trips on every commit that touches a tested file, so blocking on it would block every PR.

For repos that require signed commits on the spec file, replace the `req audit` line with:

```yaml
- run: req audit --gate --require-good-signature --require-signer "Alice <alice@example.com>"
```

---

## File format

`project.req` is pretty-printed JSON with four reserved top-level fields:

```jsonc
{
  "_warning":      "DO NOT EDIT THIS FILE BY HAND. Managed by the `req` CLI.",
  "_instructions": ["...directions for humans and agents..."],
  "_format":       "req-v1",
  "_integrity":    "sha256:<hex>",
  "name": "...",
  "requirements": { "REQ-0001": { ... }, ... }
}
```

The hash covers everything except those four reserved fields, in canonical form (sorted keys, no whitespace). Hand edits break the hash; the CLI refuses to load the file and points you at `req repair --confirm-direct-edit`.

The hash gives **integrity** ("the CLI wrote this last"). For **authenticity** ("a trusted human approved this"), use signed git commits and `req audit`.

---

## Status

Single static binary, Rust, no runtime dependencies. `req serve` runs a local web view of the spec; `req mcp` exposes the same operations as MCP tools over JSON-RPC, and `req mcp --init-config` writes a `.mcp.json` so MCP-capable clients (Claude Code, etc.) can launch the server automatically.

Issues and contributions: <https://github.com/Barks944/cli_req/issues>

---

## License

[MIT](LICENSE).
