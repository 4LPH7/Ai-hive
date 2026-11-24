---
last_mapped_commit: 3262a0c60dadc1e9078e9af568cbe4ed8b190dcb
last_mapped_at: 2026-11-24
---
# Codebase Structure

## Directory Layout

```
d:\PROJECTS\Ai-hive/
├── .github/
│   └── workflows/
│       └── tests.yml          # GitHub Actions CI matrix (Ubuntu, Windows)
├── .planning/
│   └── codebase/              # Codebase documentation & GSD architectural state
├── docs/
│   └── decisions.md           # Architecture Decision Records (ADRs) & milestones
├── examples/
│   └── vault/                 # Reference vault layout, hooks, and starter notes
│       ├── .githooks/
│       │   └── pre-commit     # Example git pre-commit hook calling lint.py
│       └── AGENTS.md          # Example vault agent instructions
├── skills/
│   └── hive-ace/
│       └── SKILL.md           # Antigravity skill specification for ai-hive
├── ace.py                     # ACE playbook delta parser & merge engine
├── CHANGELOG.md               # Version release notes and migration history
├── eval_golden.json           # Evaluation golden set for Recall@5 & MRR
├── hive.py                    # Main executable: MCP server, FTS5 index, PPR graph
├── lint.py                    # Vault rule validation & secret scanner
├── test_ace.py                # Unit test suite for ace.py
├── test_hive.py               # Unit test suite for hive.py
└── test_lint.py               # Unit test suite for lint.py
```

## Module Responsibilities & Key Files

| File | Purpose & Responsibilities |
| --- | --- |
| `hive.py` | Core CLI and MCP server: stdio JSON-RPC handler, SQLite FTS5 database management, directed graph PPR traversal, quota-aware context packer, eval and bench suites. |
| `ace.py` | ACE (Autonomous Cognitive Entity) engine: parse markdown playbooks, apply deterministic deltas (`ADD`, `TAG`, `REMOVE`), prevent counter inflation and duplicate bullets. |
| `lint.py` | Vault verification tool: validates frontmatter syntax, active project records, agent source attributions, and scans for accidental API secrets (`--staged` support). |
| `test_hive.py` | Test coverage for indexing, path traversal barriers, token estimation, quota packing, MCP tool calling, and graph algorithms. |
| `test_ace.py` | Test coverage for playbook parsing, delta merging, deduplication, and atomic write operations. |
| `test_lint.py` | Test coverage for secret detection, frontmatter parsing, BOM stripping, and staging filters. |
| `eval_golden.json` | 30 canonical retrieval evaluation queries with golden note path targets. |

## Organization Rules

- **Pure Stdlib**: New core functionality must only use Python stdlib.
- **Atomic File Writing**: File writes touching vault notes or configs must write to a temporary file (`.tmp`) and atomically rename via `os.replace`.
- **Regression Locking**: All bug fixes and enhancements must have corresponding test cases in `test_*.py`.
