---
last_mapped_commit: 3262a0c60dadc1e9078e9af568cbe4ed8b190dcb
last_mapped_at: 2026-11-24
---
# System Architecture

## Core Architectural Invariants

1. **Markdown as Sole Canonical Truth**:
   The filesystem Markdown files in the Obsidian vault are the authoritative source of truth.
2. **Disposable Indices**:
   The SQLite FTS5 database (`hive.sqlite`) and in-memory graph cache are transient accelerators. Deleting the database results in an automatic, deterministic rebuild with zero data loss.
3. **Read-Only MCP Server**:
   The `hive.py serve` process exposes query capabilities only. It does not mutate vault notes or codefiles directly over MCP.
4. **Deterministic Merges**:
   Playbook rule updates follow deterministic delta merge logic in `ace.py` with no hallucinated counts or non-deterministic rewrites.
5. **Stdlib Purity**:
   Core logic relies strictly on Python standard libraries without external package bloat.

## Data Flow & Components

```
                   +----------------------------+
                   |    AI Agent (CLI/IDE)      |
                   +-------------+--------------+
                                 | stdio JSON-RPC
                                 v
+---------------------------------------------------------------+
| hive.py (MCP Server)                                          |
|                                                               |
|  [JSON-RPC Dispatcher] <--> validate_tool_args()              |
|          |                                                    |
|          +---> tool_context() -> Quota-Packer (1500 tokens)   |
|          +---> tool_search()  -> SQLite FTS5 BM25 Engine      |
|          +---> tool_graph()   -> Directed PPR Engine (Graph)  |
|          +---> tool_status()  -> Vault & Index Metainfo       |
|          +---> tool_lint()    -> AGENTS.md Rule Auditor       |
+--------------------------------+------------------------------+
                                 |
        +------------------------+------------------------+
        v                                                 v
+-------------------------------+             +----------------------+
| Obsidian Vault (Markdown)     |             | graphify-out/        |
| D:\Valut\OBS                  |             | graph.json           |
| - projects/<slug>/_memory.md  |             | (Directed Call Graph)|
| - projects/<slug>/playbooks/  |             +----------------------+
| - _meta/active-work.md        |
+-------------------------------+
        ^
        | (Merge / Lint)
+-------+-------+
| ace.py        | (Playbook Delta Merges: ADD, TAG, REMOVE)
| lint.py       | (Pre-commit Staged Rule & Secret Verification)
+---------------+
```

## Subsystem Breakdown

- **`hive.py` Index**:
  - Incremental SQLite scanner tracking `(path, mtime, hash)`
  - FTS5 virtual table for BM25 ranking across title, tags, and body
  - Path traversal defense enforcing `validate_slug` and `is_relative_to(VAULT)`
- **`hive.py` Graph Engine**:
  - Adjacency structures: `out_adj` and `in_adj`
  - Personalized PageRank with forward damping (`1.0`) and reverse damping (`0.5`)
- **`ace.py` Curator**:
  - Deterministic playbook delta parser
  - Bullet structure: `- {rule} <!-- id:{hash} helpful:{n} harmful:{m} -->`
  - Atomic merge algorithm applying operations in-memory before file replace
- **`lint.py` Auditor**:
  - Frontmatter, active-work, source attribution, and regex-based secret scanner
