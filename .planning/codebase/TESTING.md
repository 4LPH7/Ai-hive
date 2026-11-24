---
last_mapped_commit: 3262a0c60dadc1e9078e9af568cbe4ed8b190dcb
last_mapped_at: 2026-11-24
---
# Testing & Quality Verification

## Test Harness & Architecture

- **Framework**: Python standard library `unittest`.
- **Execution**: Run with syntax warning promotion:
  ```bash
  python -W error::SyntaxWarning -m unittest discover -v
  ```
- **Test Modules**:
  - `test_hive.py`: Indexing, BM25, graph PPR, MCP JSON-RPC, quota packing, and path security.
  - `test_ace.py`: Playbook bullet parsing, delta operations, deduplication, and file updates.
  - `test_lint.py`: Secret detection, YAML frontmatter, table cell matching, and git staging.

## Isolation Strategy

- **Filesystem Isolation**:
  - Tests create temporary directory sandboxes using `tempfile.TemporaryDirectory()`.
  - No test reads from or mutates the live Obsidian vault (`D:\Valut\OBS`).
  - Ephemeral SQLite databases (`:memory:` or temporary paths).

## Golden Evaluation Suite (`eval_golden.json`)

- **Evaluation Metric**: Recall@5 and Mean Reciprocal Rank (MRR).
- **Execution Command**:
  ```bash
  python hive.py eval
  ```
- **Passing Threshold**:
  - Recall@5 >= 0.95
  - MRR >= 0.80

## Benchmark Suite

- **Metrics Tracked**:
  - Vault index build latency (cold and warm)
  - Full-text search latency per query
  - Code graph Personalized PageRank execution latency over multi-thousand node graphs
- **Execution Command**:
  ```bash
  python hive.py bench
  ```
- **Performance Budgets**:
  - Search latency: < 10 ms
  - 3.4k node graph PPR: < 100 ms
