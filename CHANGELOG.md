<!-- REQ-0080: CHANGELOG maintained alongside tagged releases. -->
# Changelog

All notable changes to this project are recorded here. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); the project
adheres to semantic versioning where `_format` schema bumps are major
version moves and CLI surface additions are minor.

## [Unreleased]

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
- `req stale` two-level staleness (commit drift vs content drift) (REQ-0057).
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
