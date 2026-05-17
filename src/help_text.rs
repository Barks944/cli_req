// Implements REQ-0018 (structured, sectioned help browsable by name).
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

const SECTIONS: &[Section] = &[
    Section {
        name: "overview",
        summary: "What `req` is and why it exists.",
        body: "`req` is a managed requirements tool. Requirements live in a binary
.req file. The file is gzipped bincode with a custom header, so an LLM
agent cannot read the file directly or edit it freely — every mutation
goes through this CLI, which enforces best-practice rules.

Humans get a `tui` browser, a local web server (`serve`), and exports
to Markdown/JSON/CSV/HTML. Agents get a `mcp` mode that speaks JSON-RPC
over stdio, exposing the same managed operations as MCP tools.",
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
  * title 5-120 chars, non-empty
  * statement >= 5 words, contains shall/must/should/will, not a question
  * rationale non-empty
  * functional requirements need acceptance criteria
  * link targets must exist; no self-links
  * approved/implemented/verified functional reqs need acceptance

Warned (saved but flagged):
  * weasel words: etc, and/or, user-friendly, fast, robust, TBD, ...
  * compound statements (likely non-atomic)
  * trailing period on the title
  * very short rationale / vague acceptance criteria",
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
        summary: "What's in a .req file and why it's binary.",
        body: "Layout:
  magic   6 bytes  REQDB\\x01
  version u16 LE
  body    gzip-compressed bincode of the Project struct

The binary header + compression means an LLM that reads the file via
`cat` / `read_file` sees noise. To change the file it must run `req`,
which validates the change first. This is the core safety property of
the tool: structured edits only.",
    },
    Section {
        name: "agents",
        summary: "How LLM agents should drive this tool.",
        body: "Two supported modes:

  STDIO MCP   — `req mcp`. The agent speaks JSON-RPC 2.0 over stdio.
                Tools exposed: req.list, req.show, req.add, req.update,
                req.delete, req.link, req.validate, req.export, req.help.

  CLI        — Agents can also shell out to `req <subcommand>`. Each
                command exits non-zero on error and prints structured
                messages, so an agent can parse them.

Rules for agents:
  * Never try to read project.req directly.
  * Always pass --reason on update/delete.
  * Run `req validate` before considering work done.
  * Use `req help <section>` to refresh context.",
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
        body: "`req tui` opens an interactive menu: browse, view, add, update,
delete, validate, export, quit. It's built on dialoguer so it works on
any terminal — no full-screen TUI required.",
    },
    Section {
        name: "integration",
        summary: "Wiring `req` into a real project.",
        body: "Put `project.req` at the repo root. Keep `AGENTS.md` next to it so
agents pick up the workflow on first read. Then:

  req hooks install            # writes .git/hooks/pre-commit + .gitattributes

The pre-commit hook runs `req validate` whenever a *.req file is staged.
The .gitattributes entry registers `req-merge` as the merge driver for
*.req. Activate the driver in your local clone with:

  git config merge.req-merge.name 'req merge driver'
  git config merge.req-merge.driver 'req renumber --base %O || true'

Cross-link requirements to code by writing `// REQ-0007` (or a similar
comment in your language) anywhere in the source tree and running:

  req coverage                 # report orphans / ghosts / obsolete-in-code

Coverage walks the tree, skipping target/, node_modules/, .git/ etc.",
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
     `req repair --confirm-direct-edit` to re-sign.",
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
        name: "export",
        summary: "Export formats.",
        body: "  markdown   human-friendly, default
  json       full project including history
  csv        spreadsheet-friendly summary
  html       single-file HTML

Use `-o -` to write to stdout (default), or `-o reqs.md` to a file.",
    },
];
