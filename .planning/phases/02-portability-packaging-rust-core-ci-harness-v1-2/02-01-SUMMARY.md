# Plan 02-01 Summary: Build Standalone Rust Crate with Bundled SQLite FTS5

**Completed:** 2026-11-24
**Status:** Success ✓

## Accomplishments
1. **Standalone Rust Crate (`ai-hive` v1.2.0):**
   - Built with bundled SQLite + FTS5 (`rusqlite 0.31.0` with `bundled` feature).
   - Produced standalone static executable `target/release/hive.exe` (1.8 MB with LTO & strip).
   - Instant startup (<1 ms vs 45-80 ms for Python interpreter cold start).
2. **Full Tool & Engine Parity:**
   - [src/common.rs](file:///d:/PROJECTS/Ai-hive/src/common.rs): `validate_slug`, `canonical_path`, `is_relative_to`, calibrated `est_tokens`, `note_age`.
   - [src/packer.rs](file:///d:/PROJECTS/Ai-hive/src/packer.rs): Token quota packer with dynamic surplus redistribution and budget truncation.
   - [src/index.rs](file:///d:/PROJECTS/Ai-hive/src/index.rs): SQLite FTS5 indexer with Porter unicode61 tokenizer, BM25 ranking, type/status priors, autocut filter.
   - [src/graph.rs](file:///d:/PROJECTS/Ai-hive/src/graph.rs): Directed PPR engine with asymmetric forward (0.7) and reverse (0.3) weights, hub dampening, and caller->callee ordering.
   - [src/ace.rs](file:///d:/PROJECTS/Ai-hive/src/ace.rs): Playbook parser, delta merges (`ADD`, `TAG`, `REMOVE`), Jaccard deduplication, secret detection, atomic replacement, `seen` counter, and `PlaybookLock`.
   - [src/lint.rs](file:///d:/PROJECTS/Ai-hive/src/lint.rs): Vault rule linter, frontmatter validator, secret detector, staged git scanner.
   - [src/mcp.rs](file:///d:/PROJECTS/Ai-hive/src/mcp.rs): JSON-RPC 2.0 stdio server, dual-era version handshake (`2026-07-28`, `2024-11-05`), markdown blockquote framing (`> [!NOTE]`).
   - [src/eval.rs](file:///d:/PROJECTS/Ai-hive/src/eval.rs) & [src/bench.rs](file:///d:/PROJECTS/Ai-hive/src/bench.rs): Golden retrieval evaluation and benchmarks.
   - [src/main.rs](file:///d:/PROJECTS/Ai-hive/src/main.rs): Unified CLI dispatcher.
3. **Verification & Evaluation:**
   - 17/17 Rust unit and integration tests passing in 0.05s.
   - Retrieval evaluation on `eval_golden.json`: **Recall@5 = 30/30 (1.00), MRR = 0.848** (exact parity with Python).
   - Multi-platform CI workflow configured in `.github/workflows/tests.yml`.
