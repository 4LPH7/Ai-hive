---
name: architecture-graph
description: Navigate codebase structure, god-nodes, call-paths, and cross-file dependencies via code graph before editing.
triggers:
  - architecture
  - code graph
  - call graph
  - callers
  - callees
  - who calls
  - dependencies
  - repo structure
---

# Architecture & Code Graph Protocol

Execute before major refactors, multi-file features, or API redesigns:

## 1. Inspect God Nodes & Hubs
- Query `hive graph <repo> "<query>"` or inspect `graphify-out/graph.json` to identify hub nodes with high in-degree or out-degree.
- Avoid introducing circular dependencies or cyclic imports across modules.

## 2. Trace Call Paths & Blast Radius
- Identify direct callers and callees of symbols targeted for modification.
- Ensure signature changes are propagated through all call-sites identified by the graph.

## 3. Verify Freshness
- Check if `graphify-out/manifest.json` is stale.
- If tracked files modified since build, run `graphify update .` to sync the structural graph.
