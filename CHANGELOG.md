# Changelog

## 2026-11-23

- **Vault lint** (`lint.py`, MCP tool `hive_lint`): frontmatter keys and values, `_memory`/active-work line
  caps, broken wikilinks (including Obsidian's table-escaped aliases), stale project memory, superseded ADRs
  without a successor link, ambiguous note names, orphan project notes, journal header format, and `source:` on
  new agent-written notes.
- **Laya inbox triage** (`triage.py`): proposes project and kind for each inbox note using laya 0.5.3's
  Learner, trained on labels harvested from the vault, improved by confirmed answers. Proposals only.
  Review routing adds a 0.6 confidence floor after the first real scan showed singleton prediction sets at
  28% confidence.
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
