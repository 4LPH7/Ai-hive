---
last_mapped_commit: 3262a0c60dadc1e9078e9af568cbe4ed8b190dcb
last_mapped_at: 2026-11-24
---
# Technical Debt & Architecture Concerns

## Current Limitations & Debt

1. **Hardcoded Root Paths (Scheduled for Milestone 2 - F6)**:
   - Default paths `D:\Valut\OBS` and `D:\PROJECTS` exist as hardcoded fallback defaults across `hive.py`, `ace.py`, and `lint.py`.
   - Non-Windows environments require environment variable overrides (`VAULT_PATH`, `PROJECTS_PATH`).
   - Target fix in v1.2: Centralize config in `common.py` with hierarchical resolution (`CLI -> ENV -> config file -> OS cache dir`).

2. **Timestamp-Based Refresh Blindspot (Scheduled for Milestone 2 - F7)**:
   - Indexer relies on filesystem `mtime` to detect modified notes.
   - External sync tools or git operations that preserve timestamps can bypass index updates.
   - Target fix in v1.2: Fast-path on `(size, mtime_ns)` with SHA-256 verification.

3. **Single Process File Locks for Concurrent ACE Updates (Scheduled for Milestone 3 - F9)**:
   - Concurrent agent execution (e.g. multiple CLI sessions simultaneously updating playbooks) can produce lost updates.
   - Target fix in v1.5: Atomic `O_EXCL` lockfile with automatic stale-lock expiration.

4. **Counter Inflation on Repeated Additions (Scheduled for Milestone 3 - F8)**:
   - Multiple `ADD` operations for similar lessons currently lack explicit user/outcome validation tags.
   - Target fix in v1.5: Decouple `seen` count from `helpful`/`harmful` feedback score.

5. **Graph Freshness & Dependency on External Graphify**:
   - `hive_graph` depends entirely on `graphify-out/graph.json`. If code changes without running `graphify`, the graph reflects stale call paths.
   - Symbol-anchored ADR staleness checks needed in v1.5.
