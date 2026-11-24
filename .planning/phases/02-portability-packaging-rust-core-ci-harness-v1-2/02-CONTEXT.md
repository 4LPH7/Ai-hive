# Phase 02: Portability, Packaging, Rust Core & CI Harness (v1.2) - Context

**Gathered:** 2026-11-24
**Status:** Ready for planning

<domain>
## Phase Boundary
Deliver production packaging, hierarchical configuration, hash-verified refresh, and seamless cross-agent execution for `ai-hive`. The standalone Rust binary (`ai-hive` v1.2.0) is already compiled and tested with exact parity on `eval_golden.json`. Phase 2 completes the surrounding configuration resolution (F6), SHA-256 refresh (F7), dual-support execution shims, and CI pipeline.
</domain>

<decisions>
## Implementation Decisions

### Config Hierarchy (F6)
- **D-01:** Precedence order: CLI flags (`--vault`, `--projects`, `--db`) → `HIVE_*` environment variables (`HIVE_VAULT`, `HIVE_PROJECTS`, `HIVE_DB`) → User config file (`~/.config/hive/config.toml` or `%APPDATA%/hive/config.toml`) → OS default paths (`D:\Valut\OBS`, `D:\PROJECTS`, OS cache dir).

### Hash-Verified Refresh (F7)
- **D-02:** Fast-path check compares `(size, mtime)` against indexed values. If moved, compute SHA-256 hash and update SQLite `notes` / `notes_fts` only if content changed. Eliminates blindspots caused by mtime-preserving git checkouts or cloud sync.

### Dual-Support Shims & Packaging
- **D-03:** Provide dual-support shims: Python scripts (`hive.py`, `lint.py`, `ace.py`) forward CLI and MCP calls to the compiled `hive` binary if present, falling back to pure Python if absent. Pre-commit hooks and agent skills continue functioning seamlessly with zero breaking changes.

### the agent's Discretion
- Exact TOML parser crate choice or minimal parser in Rust (e.g. simple key-value or serde).
- Specific platform paths resolution via `dirs` or standard environment paths.
</decisions>

<specifics>
## Specific Ideas
- Zero-cost fallback: If a user clones `ai-hive` on a machine without Rust installed, Python stdlib scripts still work out of the box. If `hive.exe` or `target/release/hive` is built, it transparently accelerates execution.
</specifics>

<canonical_refs>
## Canonical References
- `docs/decisions.md` — Core design invariants (Markdown sole truth, disposable index, read-only MCP).
- `D:/Valut/OBS/projects/ai-hive/adrs/0004-rust-migration.md` — Standalone Rust binary architecture and bundled SQLite FTS5.
- `D:/Valut/OBS/projects/ai-hive/adrs/0003-milestone-1-correctness-security.md` — Milestone 1 correctness and security locks.
</canonical_refs>

<code_context>
## Existing Code Insights

### Reusable Assets
- `src/common.rs`: Path safety, slugify, token estimation, note age.
- `src/index.rs`: SQLite FTS5 database wrapper with BM25 and priors.
- `src/main.rs`: CLI dispatcher with `clap`.
- `hive.py`, `lint.py`, `ace.py`: Established Python CLI entrypoints ready for thin shim delegation.

### Established Patterns
- Atomic file writes via `.tmp` + `replace`.
- Zero-panic fail-open server responses with structured JSON-RPC errors.
</code_context>

<deferred>
## Deferred Ideas
- Phase 3: Decouple ACE `seen` counter from `helpful`/`harmful` feedback (F8).
- Phase 3: Atomic `O_EXCL` file locking for concurrent ACE merges (F9).
- Phase 3: Local JSONL session ledger outside the vault.
- Phase 4: Objective outcome rewards and inbox LLM curator.
</deferred>

---
*Phase: 02-portability-packaging-rust-core-ci-harness-v1-2*
