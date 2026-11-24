# Roadmap: ai-hive

## Overview
Comprehensive engineering roadmap transitioning `ai-hive` from Python prototype into a hardened, high-performance, self-optimizing multi-agent memory system.

## Phases

- [x] **Phase 1: Correctness, Security & Regression Lock (v1.1)** - Fixed path traversal, quota packing, isolation, directed graphs, secrets, BOM.
- [ ] **Phase 2: Portability, Packaging, Rust Core & CI Harness (v1.2)** - Rust standalone binary, unified config, SHA-256 refresh, and public CI matrix.
- [ ] **Phase 3: Learning Foundation & Cross-Agent Parity (v1.5)** - ACE counter split, atomic locks, JSONL session ledger, and reflection parity.
- [ ] **Phase 4: Evidence-Gated Intelligence & Scale (v2.0)** - Objective outcome rewards, auto-decay, and inbox LLM curator.
- [ ] **Phase 5: Auto-Skill Discovery, Selection & Dynamic Installation Engine (v2.1)** - Intelligent local skill ranking, remote registry search (find-skills), and dynamic installation for agents.

## Phase Details

### Phase 1: Correctness, Security & Regression Lock (v1.1)
**Goal**: Eliminate confirmed correctness and security audit bugs.
**Status**: Completed ✓
**Success Criteria**:
1. Path traversal barred (`validate_slug` + `is_relative_to`).
2. Quota-based context pack prevents memory starvation.
3. Directed PPR caller->callee edges correctly ordered.
4. All 47 Python unit tests pass in CI.

### Phase 2: Portability, Packaging, Rust Core & CI Harness (v1.2)
**Goal**: High-performance native binary, clean packaging, unified config, and hash-verified refresh.
**Requirements**: [RUST-01, CONF-01, SYNC-01, CI-01, SHIM-01]
**Success Criteria**:
1. Standalone `hive` binary with bundled SQLite + FTS5 passes all unit tests (Recall@5 1.00, MRR ≥0.85).
2. Configuration resolves cleanly via CLI, ENV, and OS cache defaults.
3. Hash verification prevents skipped indexing on mtime-preserving writes.
4. GitHub Actions builds and passes on Linux and Windows.

Plans:
- [x] 02-01: Build standalone Rust crate with bundled SQLite FTS5 and MCP stdio engine
- [ ] 02-02: Implement unified config hierarchy and SHA-256 hash-verified refresh
- [ ] 02-03: Add cross-agent CLI shims and finalize packaging

### Phase 3: Learning Foundation & Cross-Agent Parity (v1.5)
**Goal**: Honest, atomic ACE self-improvement loop shared across all coding agents.
**Requirements**: [ACE-01, LOCK-01, LEDG-01, HOOK-01]
**Success Criteria**:
1. Separate `seen` count from `helpful`/`harmful` feedback.
2. Concurrent ACE updates keep 50/50 bullets via atomic lockfile.
3. Local JSONL session ledger records injected bullets and query logs outside vault.
4. Codex and Antigravity reflection parity feeds the shared ledger.

### Phase 4: Evidence-Gated Intelligence & Scale (v2.0)
**Goal**: Self-optimization unlocked strictly when usage data justifies cost.
**Success Criteria**:
1. Objective outcome rewards score bullets based on test exit codes and git diffs.
2. Optional cheap LLM curator merges duplicate playbook rules.

### Phase 5: Auto-Skill Discovery, Selection & Dynamic Installation Engine (v2.1)
**Goal**: Automatically select the best installed skill for any agent task, and dynamically discover, install, and inject missing skills from registries like `find-skills` (`npx skills`).
**Requirements**: [SKILL-INDEX, SKILL-MATCH, SKILL-RESOLVE, SKILL-FIND, SKILL-INSTALL, SKILL-LOAD, HIVE-TOOL, ACE-LINK]
**Success Criteria**:
1. Skill Indexer discovers and indexes skills across Antigravity, Codex, Claude Code, and project `.agent/skills`.
2. Skill Matcher ranks local skills against agent task intent with confidence thresholding.
3. If no matching local skill meets threshold, remote search queries registries (`npx skills find` / marketplace).
4. Auto-installer safely installs chosen skill into local/project repository and verifies integrity.
5. Injects freshly loaded `SKILL.md` instructions and tool schemas directly into agent context.
6. `hive_skill` MCP tool and CLI command operational in both Python and Rust cores.
7. End-to-end integration test passes: task -> local miss -> remote find -> install -> inject.
