# ai-hive

## What This Is
Shared, self-improving memory for Claude Code, Codex, Antigravity and ZeroClaw over an Obsidian markdown vault and graphify code graphs. Provides deterministic retrieval (BM25 + Directed Personalized PageRank), quota-packed context, and ACE playbook curation with zero external runtime dependencies.

## Core Value
High-performance, deterministic memory retrieval and playbook curation with Markdown as sole canonical truth and zero external runtime dependencies.

## Requirements

### Validated
- ✓ **SEC-01**: Path traversal barred (`validate_slug`, `is_relative_to(VAULT)`) [Milestone 1 - F3]
- ✓ **RET-01**: Quota-based section packing with dynamic redistribution [Milestone 1 - F1/F15]
- ✓ **RET-02**: Project-isolated retrieval with explicit cross-project search scope [Milestone 1 - F2]
- ✓ **GRP-01**: True directed code-graph traversal with hub dampening & directed edges [Milestone 1 - F4]
- ✓ **IO-01**: Windows UTF-8 BOM tolerance & exact active-work cell matching [Milestone 1 - F5/F11]
- ✓ **SEC-02**: Pre-commit secret scanning & untrusted provenance note framing [Milestone 1 - F10/F14]
- ✓ **MCP-01**: Strict tool schema validation and JSON-RPC error framing [Milestone 1 - F13]
- ✓ **RUST-01**: High-performance standalone Rust binary with bundled SQLite + FTS5 [Milestone 2]
- ✓ **CONF-01**: Hierarchical unified configuration resolution (CLI -> ENV -> config -> OS cache) [Milestone 2 - F6]
- ✓ **SYNC-01**: Hash-verified incremental refresh (`size, mtime_ns, SHA-256`) [Milestone 2 - F7]
- ✓ **SHIM-01**: Cross-agent native binary delegation shims with transparent Python fallback [Milestone 2]
- ✓ **ACE-01**: ACE counter split: decouple `seen` count from `helpful`/`harmful` feedback [Milestone 3 - F8]
- ✓ **LOCK-01**: Atomic `O_EXCL` file locking with stale-lock expiration for concurrent ACE merges [Milestone 3 - F9]
- ✓ **LEDG-01**: Local JSONL session ledger outside vault tracking queries, hits, and injected bullets [Milestone 3]
- ✓ **HOOK-01**: Cross-agent reflection parity (fail-open Codex stop-hook & Antigravity ledger sync) [Milestone 3]

### Active
- [ ] **CI-01**: GitHub Actions matrix running Rust and Python test suites on Ubuntu and Windows [Milestone 2 - F16]

### Out of Scope
- Dual-write to database — violates Markdown as sole truth.
- Vector database / embedding daemon — BM25 + PPR reaches Recall@5 1.00 at vault scale; embeddings add unnecessary runtime bloat.
- LLM full playbook rewrite — collapses itemized rules into generic summaries (ACE paper).
- ML models for inbox triage — manual curation is simpler and reliable at current vault size.

## Context
Codebase mapped into `.planning/codebase/`.
Rust core implemented in `src/` (`ai-hive` v1.2.0) with exact parity on `eval_golden.json` (Recall@5 1.00, MRR 0.850).
Shared memory vault resides at `D:\Valut\OBS`.

## Key Decisions
| Decision | Rationale | Outcome |
|---|---|---|
| Rust migration (v1.2.0) | Standalone binary, bundled SQLite FTS5, <1ms startup, 2-3x faster PPR | ✓ Done |
| Single binary `hive` | Unified CLI and MCP server without managing multiple executables | ✓ Done |
| Markdown sole truth | Disposable indexes prevent state drift and migration headaches | ✓ Done |

---
*Last updated: 2026-11-24 after initialization*
