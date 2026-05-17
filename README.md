<!-- REQ-0036: project README is the canonical landing page. -->
# req

**A managed requirements CLI for LLM agents and humans.**

`req` keeps software-project requirements in a single git-tracked JSON file (`project.req`) and mediates every change through a CLI that enforces requirements best practice. The file diffs cleanly, history is append-only, and an integrity hash catches hand edits before they corrupt your spec.

Built for the workflow where agents and humans share the same source of truth.

---

## Why `req`?

Requirements rot when they live in wikis, drift when they live in code comments, and become unreviewable when they live in a database. `req` puts them in your repo as JSON, but stops anyone — human or agent — from editing them in ways that break the rules you'd want a senior engineer to enforce:

- **One obligation per requirement**, with a normative modal verb (`shall` / `must` / `should` / `will`).
- **Append-only history** with a required `--reason` on every change.
- **Integrity hash** so silent corruption shows up immediately, not three weeks later.
- **Code traceability** via `// REQ-NNNN` markers and `req coverage`.
- **Git-native**: pre-commit hook, merge driver for ID collisions, signature-based audit trail.

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

---

## Quick start

```sh
# create a fresh project
req init -n "My Project"

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

# validate before you commit
req validate
```

For everything else: `req help` lists the section index, `req help <section>` drills in.

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
  req init -n <name>                    Create project.req
  req tui                               Interactive browser / editor
  req validate                          Run all rules; 0 errors to ship
  req export -f markdown -o reqs.md     Publish (markdown / json / csv)
  req serve [--read-only]               Local web UI

Day-to-day
  req add ...                           Add a requirement
  req list [--status ...] [--tag ...]   Filter
  req show REQ-0007                     Full detail + history
  req update REQ-0007 --status approved --reason "..."
  req link <from> <to> -k <kind>        parent | depends-on | verifies | …
  req delete REQ-0007 --reason "..."    Soft by default; --hard if no inbound links

Integration
  req hooks install                     Pre-commit + merge driver
  req renumber --base origin/main       Post-merge ID collisions
  req coverage --path src               Orphans / ghosts / by-file / remap
  req audit                             Git signature trail

Recovery
  req repair --confirm-direct-edit      After intentional hand edits

Docs
  req help                              Section index
  req help <section>                    overview | concepts | best-practice
                                        | workflow | integration | audit | agents
  req help all                          Everything
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

Single static binary, Rust, no runtime dependencies. `serve` and `mcp` are reserved subcommands — `serve` ships a read-only HTTP view; the MCP JSON-RPC interface is on the roadmap. For agent use today, shell out to `req <subcommand>`.

---

## License

See repository.
