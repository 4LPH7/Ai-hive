# ai-hive

**One shared, self-improving memory for every coding agent on your machine.**

ai-hive turns an Obsidian vault and [graphify](https://docs.graphify.com) code graphs into a single memory layer
that Claude Code, OpenAI Codex and Google Antigravity all read from over MCP. Agents write plain Markdown
notes; the hive indexes them, ranks them, packs them into a token budget, checks the vault's health, and
learns reusable lessons from each session.

- **Fast:** stdlib-only Python, SQLite FTS5 index outside the vault. Vault search in about 3 ms, code-graph
  queries in 10-60 ms on 1k-3.5k node graphs.
- **Safe:** the MCP server is read-only and has no network port. Notes stay the source of truth; every index
  is disposable and rebuilds from disk.
- **Self-improving:** ACE playbooks with helpful/harmful counters, merged deterministically, so lessons
  accumulate without being rewritten into vague summaries.
- **Self-checking:** a lint pass enforces the vault's own rules (line caps, frontmatter, broken links,
  superseded decisions, sources on agent-written notes), and a git pre-commit hook blocks commits that
  break them.

```mermaid
flowchart LR
    subgraph Agents
        CC[Claude Code]
        CX[Codex]
        AG[Antigravity]
    end
    subgraph Hive["ai-hive (stdio MCP, read-only)"]
        CTX[hive_context]
        S[hive_search<br/>BM25 / FTS5]
        G[hive_graph<br/>Personalized PageRank]
        L[hive_lint]
    end
    V[(Obsidian vault<br/>Markdown = truth)]
    GJ[(graphify-out/graph.json<br/>per repo)]
    IDX[(hive.sqlite<br/>disposable index)]
    ACE[ace.py<br/>playbook curator]

    CC & CX & AG -- MCP --> Hive
    CC & CX & AG -- write notes --> V
    V --> IDX --> S
    GJ --> G
    V --> L
    CC -- hooks / skill hive-ace --> ACE --> V
```

## Quick start

```bash
git clone https://github.com/4LPH7/ai-hive.git
cd ai-hive
python -m unittest                       # 35 tests, stdlib only
set HIVE_VAULT=D:\path\to\vault          # or export on macOS/Linux
python hive.py status                    # index the vault, list graphs
python hive.py search "why sqlite"       # BM25 over notes
python hive.py lint                      # vault health report
```

Then register `python -I /path/to/ai-hive/hive.py serve` as an MCP server in each agent.
[docs/setup.md](docs/setup.md) walks through the full installation, step by step.

## MCP tools

| Tool | What it returns |
|------|-----------------|
| `hive_context(project, task?, budget=1500)` | Session-start pack: the project's active-work row, `_memory.md`, ADR list, top playbook lessons, and (with a task) matching notes and ranked code symbols, greedily packed to the token budget |
| `hive_search(query, project, limit=8, scope="project")` | BM25 over the selected project's notes plus global notes by default; set `scope="global"` or explicit `scope="all"` for other search modes; title weighted x5; ADRs and project memory boosted; superseded or archived notes demoted; weak tail cut |
| `hive_graph(repo, query, limit=15)` | graphify code graph: identifier seeds, directed Personalized PageRank over the 2-hop neighbourhood with stronger caller-to-callee traversal; symbols with `file:line`, source-to-target edges, related notes, staleness warning |
| `hive_status()` | Notes per project, graph size and freshness per repo |
| `hive_lint()` | Vault health against the vault manual's rules |

## Command line

| Command | Purpose |
|---------|---------|
| `python hive.py serve` | MCP stdio server (what agents launch) |
| `python hive.py search\|graph\|context\|status\|lint ...` | Same tools for humans and hook scripts |
| `python hive.py eval` | Recall@5 and MRR of search over your golden question set |
| `python hive.py bench` | Index, search and graph timings |
| `python ace.py apply <scope> <delta.json>` | Merge a reflection into a playbook |
| `python ace.py show <scope> ["task"]` | Bullets an agent would be shown |
| `python ace.py hook-start\|hook-stop` | Hook entry points (Claude Code: both; Codex: hook-start) |
| `python lint.py [vault]` | Lint report; exit code 1 on any error |

## Repository layout

```
ai-hive/
├── hive.py              MCP server + CLI: vault index, BM25 search, graph PPR, context packing
├── ace.py               ACE curator: playbook parse/merge/prune, SessionStart and Stop hooks
├── lint.py              Vault lint: deterministic checks for the vault manual's rules
├── test_*.py            unittest suites (stdlib only, no network, no model)
├── skills/hive-ace/     Agent skill: the Reflector + Curator procedure
├── examples/            MCP registrations, hook config, vault manual template, golden-set example
└── docs/                Architecture, setup, vault conventions, ACE, lint, decisions
```

## Documentation

1. [Architecture](docs/architecture.md): components, data flow, ranking maths, performance
2. [Setup, step by step](docs/setup.md): from an empty folder to three agents sharing one memory
3. [Vault conventions](docs/vault-conventions.md): the folder and frontmatter contract agents follow
4. [ACE playbooks](docs/ace.md): how the hive learns lessons from sessions
5. [Lint](docs/lint.md): every check, its severity and how to fix it
6. [Design decisions](docs/decisions.md): what was built, what was rejected, and why
7. [Improvements roadmap](docs/improvements.md): what is worth building next, and the signal for each

## Status

Used daily on Windows 11 with a vault of about 55 notes and four graphify graphs (1k-3.5k nodes each).
Search golden set: Recall@5 1.00 (30/30), MRR 0.833. Worst-case full MCP session: 322 ms.

## License

[MIT](LICENSE)
