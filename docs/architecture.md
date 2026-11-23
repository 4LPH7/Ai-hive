# Architecture

ai-hive is four small programs around one rule: **Markdown in the vault is the only source of truth.**
Everything else (the search index, the loaded graphs, Laya's fitted model) is derived, disposable and rebuilt
from disk.

## Components

| Component | File | Runs as | Writes |
|-----------|------|---------|--------|
| MCP server + CLI | `hive.py` | One stdio process per agent session | Only the disposable index (`%LOCALAPPDATA%\hive\hive.sqlite`) |
| ACE curator | `ace.py` | CLI, Claude Code hooks, the `hive-ace` skill | Playbook notes (`_meta/playbook.md`, `projects/<slug>/playbook.md`) |
| Vault lint | `lint.py` | CLI and `hive_lint` tool | Nothing (read-only report) |
| Inbox triage | `triage.py` | CLI in a separate Python 3.12 venv | `inbox/_triage.md` and state outside the vault |

`hive.py`, `ace.py` and `lint.py` use only the Python standard library (3.10+). `triage.py` imports Laya
lazily, so its pure functions are tested by the same stdlib test run.

## Data flow

```
 agents write Markdown ──────────────► vault (git-backed Obsidian folder)
                                          │
              mtime scan every 2 s ◄──────┤
                     │                    │
                     ▼                    ▼
       hive.sqlite (FTS5, notes)     lint.py  (rules report)
                     │                    
 graphify-out/graph.json per repo         
                     │                    
                     ▼                    
   hive.py tools ── MCP stdio ──► Claude Code / Codex / Antigravity
                                          │
                session ends / Stop hook  ▼
                         skill hive-ace ──► ace.py apply ──► playbook notes
                                          
 inbox/*.md ──► triage.py (Laya) ──► inbox/_triage.md ──► you confirm ──► labels.jsonl ──► Laya learns
```

## Vault index and search (`hive.py`, class `Index`)

- Walks the vault, skipping `.git`, `.obsidian`, agent skill folders, templates, attachments and runtimes.
- Re-reads only files whose mtime changed, at most every 2 s (`REFRESH_EVERY_S`). Deleted files are dropped.
- Frontmatter is parsed as flat `key: value` lines. `project` falls back to the `projects/<slug>/` folder.
- One FTS5 table with columns `title`, `meta` (project, type and path words) and `body`, Porter-stemmed.
- Ranking: `bm25(title x5, meta x2, body x1)`, then multiplied by
  - a type prior: project-memory 1.4, adr 1.25, note 1.0, journal 0.8
  - a status prior: superseded 0.3, archived 0.4
  - a project boost of 2.0 when the note belongs to the requested project
- **Autocut:** hits scoring below 35% of the best are dropped, so agents do not read noise.
- Tuned against a golden question set (`python hive.py eval`): Recall@5 0.97.

The index lives outside the vault and carries a schema version; a version change drops and rebuilds it.

## Code graph (`hive.py`, class `Graph`)

graphify parses each repository with tree-sitter and writes `graphify-out/graph.json` (NetworkX node-link
format: nodes with `source_file` and `source_location`, edges with a `relation`). The hive loads it lazily
and caches it per process.

Query steps:

1. **Seeds:** split the query and every node label into identifier tokens (`camelCase`, `snake_case`);
   score nodes by idf-weighted token overlap, with exact label matches x3.
2. **Neighbourhood:** collect nodes within 2 hops of the seeds.
3. **Personalized PageRank** on that neighbourhood: restart probability 0.15, 25 iterations. Edge weights
   by relation (`calls` 1.0, `inherits` 0.9, `references` 0.7, `imports` 0.5, `contains` 0.4, ...), divided
   by the square root of the target's degree so hub nodes do not soak up all the mass.
4. **Priors:** code 1.5, documents and concepts 0.8, rationale 0.6.
5. Output: ranked symbols with `file:line`, edges among them, vault notes about the same files, and a
   freshness line from `graphify-out/manifest.json` mtimes (lists changed files and tells you to run
   `graphify update .`).

## Context packing (`hive_context`)

Parts in priority order: active-work row, `_memory.md`, ADR list, playbook bullets, notes for the task,
code graph for the task. Each part's cost is estimated at 4 characters per token. Parts are added whole
until the next one would cross the budget; that one is truncated if more than 200 characters of room remain.

## MCP protocol

`hive.py serve` reads JSON-RPC lines on stdin and writes responses on stdout. It answers both the legacy
`initialize` handshake (protocol 2024-11-05 to 2025-11-25) and the modern per-request `_meta` version
(2026-07-28, `server/discover`). Tools carry read-only annotations; `hive_context` is marked
`anthropic/alwaysLoad` so Claude Code loads it upfront while the others stay deferred. A failing tool
returns `isError: true` instead of killing the server.

## Performance (measured on the author's machine)

| Operation | Time |
|-----------|------|
| Vault refresh, 76 notes | about 10 ms |
| Vault search | about 3 ms |
| Graph load / hub query, 3,427 nodes | 114 ms / 60 ms |
| Graph load / hub query, 1,698 nodes | 39 ms / 10 ms |
| Laya decision per note (CPU) | about 0.5 s |
| Laya fit from scratch, 76 examples (CPU) | about 15 min, cached afterwards |

Run `python hive.py bench` for your own numbers.
