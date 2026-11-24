# Changelog

## v1.2.0 (2026-11-24)

- **Rust Standalone Static Binary (`hive.exe`):** Complete migration to Rust (`ai-hive` v1.2.0 crate) with bundled SQLite + FTS5 (`rusqlite 0.31`). Zero runtime dependencies, instant startup (<1 ms).
- **Tool Parity:** Full implementation of `hive_context`, `hive_search`, `hive_graph`, `hive_status`, `hive_lint`, `ace` CLI subcommands, `eval`, and `bench`.
- **Graph Performance:** 7.8–32.8 ms Personalized PageRank on graphs up to 3,427 nodes (2-3x faster than Python).
- **Retrieval Accuracy:** Exact evaluation parity on `eval_golden.json`: Recall@5 = 30/30 (1.00), MRR = 0.848.
- **CI Matrix:** Multi-platform CI running on Ubuntu and Windows across stable Rust and Python 3.9/3.10/3.12.

## v1.1.0 (2026-11-24)

- **Security (F3, R2, G1, G2):** Canonical path resolution (`canonical_path`) normalizing Windows 8.3 short paths, drive casing, and junctions; registered roots model (`roots.toml`, `HIVE_ALLOWED_REPOS`, `PROJECTS`) with `hive roots add <path>`; Python ≥3.9 floor with runtime guard.
- **Context pack (F1, F15, G7):** Quota-based `pack()` with dynamic surplus redistribution across section shares (35% memory, 15% playbook, 25% task notes, 25% code graph) and `omitted_sections` footer; calibrated token estimator (code 3.4, prose 4.2, 1.08 safety margin) with `tools/calibrate_tokens.py`.
- **Isolation & Search Scope (F2, G5):** Strict project isolation in `hive_context` (`scope="project"`); `hive_search` defaults to `scope="all"` with deprecation warning when omitted; `hive migrate --search-scope` command.
- **Code graph (F4, R1):** Directed graph storage (`out_adj`, `in_adj`), edges printed caller -> callee, asymmetric PPR transition weights (`w_fwd=0.7, w_rev=0.3`).
- **Portability & exact match (F5, F11):** UTF-8 BOM stripping (`utf-8-sig`) across readers and index; exact table cell matching for `active-work.md`.
- **Provenance & secret gate (F10, F14, G6):** Secret regex enforced as `ERROR` in `lint.py`; `source:` missing on agent notes escalated to `ERROR`; markdown blockquote prefix (`> `) for untrusted data framing; pre-commit hook restricted to staged files.
- **MCP hygiene & protocol versions (F13, G8, R3):** Minimal stdlib schema validator returning structured error dicts; dual-era protocol negotiation across all supported versions (`2026-07-28` to `2024-11-05`).
- **ACE hardening (F8, F9, G3, G4):** Decoupled `seen` counter preventing inflation on near-duplicate lessons; automatic v1.0.0 playbook migration; atomic `PlaybookLock` (`{pid}:{hostname}:{iso}:{nonce}`) with 30s stale-steal and dead-process detection preventing lost updates under concurrent writers.

## 2026-11-23 (later)

- **Fix:** `hive_lint` failed in every agent with `No module named 'lint'`: agents launch `python -I`, which
  drops the script folder from the import path. `hive.py` and `lint.py` now add it themselves; a test runs
  both under `-I`.
- **Search:** autocut 35% -> 30%. It had cut a correct rank-4 answer; golden set Recall@5 0.97 -> 1.00.
- **Lint:** new `memory-undescribed` warning for a `_memory.md` without `## What it is`; lint skips unreadable
  folders instead of crashing.
- **Vault pre-commit hook** (`examples/vault/.githooks/pre-commit`): blocks commits with lint ERRORs.
- **Codex** gets playbook lessons at session start (`examples/codex-hooks.json`).
- [Improvements roadmap](docs/improvements.md) for the larger next steps.

## 2026-11-23

- **Vault lint** (`lint.py`, MCP tool `hive_lint`): frontmatter keys and values, `_memory`/active-work line
  caps, broken wikilinks (including Obsidian's table-escaped aliases), stale project memory, superseded ADRs
  without a successor link, ambiguous note names, orphan project notes, journal header format, and `source:` on
  new agent-written notes.
- **Laya inbox triage tried and removed:** a laya 0.5.3 Learner trained on vault labels scored 95% on held-out
  vault notes but got both real inbox notes wrong at under 30% confidence while its gate said `auto`. It needed
  a 4 GB torch venv, 15-minute CPU refits and dozens of confirmations to sort a handful of notes. See
  `docs/decisions.md`.
- Repository published with documentation, examples and CI.

## 2026-11-20

- **ACE self-improvement** (`ace.py`, skill `hive-ace`): itemized playbooks with helpful/harmful counters,
  deterministic delta merges, SessionStart injection and a Stop-hook reflection checkpoint for Claude Code.
- Playbook bullets ranked by net evidence and task overlap inside `hive_context`.

## 2026-11-16

- Search indexes a `meta` field (project, type, path words), adds autocut and a 2.0 project boost; golden-set
  evaluation (`hive.py eval`).
- Graph freshness from graphify's `manifest.json`, naming changed files.
- MCP: read-only tool annotations, modern `server/discover`, `hive_context` always loaded in Claude Code.

## 2026-11-15

- First version: stdlib MCP server with `hive_context`, `hive_search`, `hive_graph`, `hive_status`; FTS5
  vault index; Personalized PageRank over graphify graphs with hub dampening; token-budgeted context packing.
