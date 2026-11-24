# Requirements: ai-hive

**Defined:** 2026-11-24
**Core Value:** High-performance, deterministic memory retrieval and playbook curation with Markdown as sole canonical truth and zero external runtime dependencies.

## v1 Requirements

### Core Portability & Packaging (Phase 2)
- [x] **RUST-01**: Crate compiled to standalone static binary `hive` with bundled SQLite and FTS5
- [ ] **CONF-01**: Hierarchical config resolution (`--vault` flag -> `HIVE_VAULT` env -> default OS path)
- [ ] **SYNC-01**: Hash-verified incremental refresh (`(size, mtime_ns)` fast-path + SHA-256)
- [x] **CI-01**: GitHub Actions workflow testing Rust and Python on Ubuntu and Windows
- [ ] **SHIM-01**: Cross-agent compatibility shims for Claude Code, Codex, and Antigravity

### Learning Foundation & Parity (Phase 3)
- [ ] **ACE-01**: Decouple `seen` repetition counter from `helpful`/`harmful` tag counters
- [ ] **LOCK-01**: Atomic `O_EXCL` lockfile with stale-lock expiration for concurrent ACE merges
- [ ] **LEDG-01**: Local JSONL session ledger outside vault tracking injected bullets and zero-hit queries
- [ ] **HOOK-01**: Strictly fail-open Codex Stop-hook and Antigravity reflection sync

## Traceability

| Requirement | Phase | Status |
|---|---|---|
| RUST-01 | Phase 2 | Completed |
| CONF-01 | Phase 2 | Pending |
| SYNC-01 | Phase 2 | Pending |
| CI-01 | Phase 2 | Completed |
| SHIM-01 | Phase 2 | Pending |
| ACE-01 | Phase 3 | Pending |
| LOCK-01 | Phase 3 | Pending |
| LEDG-01 | Phase 3 | Pending |
| HOOK-01 | Phase 3 | Pending |
