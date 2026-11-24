# Changelog

## v1.1.0 (2026-11-24)

- **Security (F3):** Close path traversal in `tool_context` slug validation, `Index.note` canonical vault check (`is_relative_to(VAULT)`), and `repo_dir` root validation against `PROJECTS`.
- **Context pack (F1, F15):** Quota-based `pack()` with dynamic redistribution across section shares (35% memory, 15% playbook, 25% task notes, 25% code graph) and `omitted_sections` footer; content-aware token estimator (code vs prose + 10% safety margin) preventing large memory notes from starving task notes or graphs.
- **Isolation (F2):** Strict project isolation in `hive_context` and `hive_search` (default `scope="project"` filters to target project + global; explicit `scope="all"` for cross-project search).
- **Code graph (F4):** True directed graph storage (`out_adj`, `in_adj`), edges printed in caller -> callee direction, separate forward/reverse transition weights in PPR.
- **Portability & exact match (F5, F11):** Strip UTF-8 BOM (`utf-8-sig`) in `parse_note` and index reads; exact table cell matching for `active-work.md` preventing substring match collisions.
- **Provenance & secret gate (F10, F14):** Secret regex enforced as `ERROR` in `lint.py`; `source:` missing on agent notes escalated to `ERROR`; untrusted-data framing (`<untrusted_vault_note>`) with `(agent, source, age)`; pre-commit hook restricted to staged files (`--staged`).
- **MCP hygiene (F13):** Strict argument schema validation in JSON-RPC dispatcher returning structured `isError` messages with allowed keys.

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
