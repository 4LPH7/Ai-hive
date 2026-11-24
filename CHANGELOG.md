## 2026-11-24 — v1.1 correctness and security patch

- **Security:** constrain project slugs, vault note reads and graph repository paths to their canonical roots; reject invalid MCP arguments and unknown fields.
- **Isolation:** scope MCP search and task context to the selected project plus global memory by default; cross-project search now requires `scope="all"`.
- **Context:** allocate separate memory, playbook, task-note and graph budgets so a long project memory cannot starve task context.
- **Graph:** preserve source-to-target edge direction and bias graph diffusion toward forward relationships.
- **Vault integrity:** parse UTF-8 BOM notes, match active-work project cells exactly, scan notes with ACE's shared secret detector, and make missing provenance on new agent notes an ERROR.
- **Trust boundary:** label retrieved vault content as untrusted data with agent/source/update provenance.
- **Git hooks:** pre-commit lint checks staged files while resolving references against the complete vault.

# Changelog

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
