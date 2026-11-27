# Changelog

All notable changes to `ai-hive` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased] - v2.8.0 (Phase 8 addendum)

### Added
- **RATCHET-01 failure refs (`quorum.py`, `src/quorum.rs`, `ace.py`, `src/ace.rs`)**: every ACE ADD may carry
  `failure_ref` (commit sha, error hash or session id; 4-120 safe chars, secret-checked). A new bullet without a
  valid ref lands as a scout with weight 0 that never promotes until a later ADD of the same lesson cites one.
  Sidecar records written before 2.8.0 carry no `failure_ref` key and keep their old bytes and behaviour.
  `ace apply` reports `unsourced` ids; the `hive-ace` skill documents the field.
- **MCP-02 schema budget**: Python and Rust tests cap the six MCP tool schemas at 1,140 est tokens (1,037
  measured at v2.7.2 + 10%), so tool descriptions cannot creep into context confusion. MCP-01 (≤ 6 tools) holds.
- **RATCHET-02 `hive ratchet` (`ratchet.py`, `src/ratchet.rs`)**: drafts `inbox/ratchet-<rule-id>.md` for each
  canonical lesson that bans a `backticked` token; `hive ratchet accept <rule-id> [--target vault|repo]
  [--glob G]` (a human step) ships it to `_meta/ratchet_rules.jsonl`. Vault rules are `ratchet-violation`
  ERRORs in `hive lint`; repo rules block staged files in `hive pre-commit`; the source bullet stops being
  injected (`ace.select`, `load_injectable`). Patterns are literal substrings; globs support `*` and `?` only,
  with identical semantics in both languages. Phase 8 gate covered by Python and Rust end-to-end tests.
- **RATCHET-03 `hive ratchet release [scope]`**: proposes retiring (`inbox/retire-<scope>-<id>.md`) canonical
  lessons injected into >= 20 distinct real sessions (from `context` ledger events; per-process ids ignored)
  that were never tagged helpful and have no shipped rule. A proposal only: the human applies
  `{"remove": [id]}` with `hive ace apply`.
- **QUOR-06 `hive verify <scope> <bullet-id> <artifact-file>`**: cross-agent verification weighs 1.5 (above
  test pass 1.0) only from a different agent and session than the lesson's author, with an evidence artifact
  stored as its sha256. Otherwise it counts as a plain confirm or self-tag. Verify evidence without a valid
  artifact, or from the author's agent or session, is dropped when the sidecar loads. A verify plus the
  author's self-tag (1.85) reaches canonical; a plain confirm plus self-tag (1.35) stays quorum.

### Fixed
- **`est_tokens` parity**: the Rust estimator counted UTF-8 bytes and a newline after every line, including a
  final line without one; it now counts characters and real line endings like `hive.est_tokens`, so budgets,
  packing and gates agree across languages (pinned on 14 cases incl. CRLF and non-ASCII).

### Changed
- **Signal-preserving offload excerpt (`artifacts.excerpt`, `artifact::excerpt`)**: an offloaded result's inline
  excerpt keeps the head (55% of 400 tokens), the tail (15%) and every error/fatal/panic/traceback/exception/
  failed line in between, numbered `L<n>:`, instead of only the head. A FATAL at line 900 of a 1,000-line output
  now survives. Byte-identical in both languages (pinned). Approach adapted from Headroom's log compressor
  (headroomlabs-ai/headroom); the code is our own. The offload pointer now prints `os.path.abspath`, matching
  Rust.
## [2.7.2] - 2026-11-26

### Security
- **Provenance seal on codices and handoffs (`codex.py`, `src/codex.rs`)**: new notes carry
  `provenance_sha256 = sha256(project, agent, source, updated, content_sha256)`. `verify()` (and so the
  `codex-tampered` lint ERROR and `hive audit`) now catches an in-place edit of who/when/which session wrote a
  note, for example rewriting the session line to slip a quarantined writer's handoff back into the read path.
  Handoffs without the seal (written by 2.7.1) are refused on read; plain codices without it still verify. The
  seal is unkeyed: it detects edits, it does not authenticate the writer.

### Fixed
- `handoff_view`'s `full envelope:` pointer prints the same absolute path in Python (`os.path.abspath`) and Rust
  (`std::path::absolute`).

### Tests
- Full note text (frontmatter included) pinned byte-identical across Python and Rust.
- `hive serve` end-to-end test for concise and detailed `hive_search` / `hive_graph` and the `detail` enum check.
- Python and Rust tests for the `hive handoff write` warning when the protected core exceeds the role budget.

## [2.7.1] - 2026-11-26

### Added
- **Handoff envelopes (HAND-01/02, `hive handoff write`, `src/codex.rs`, `codex.py`)**: a codex record plus
  `status` (SUCCESS|PARTIAL|ERROR), `role` (implementer|reviewer|verifier), `completed_plan_items` and
  `artifact_paths`, written immutable to `codices/<slug>/YYYY-MM-DD-<sha8>.md` (at most 6 dead ends, 6 paths of
  200 chars, 80 lines). Artifact paths must be repo-relative or `artifact:<sha256>`.
- **Read by id**: `hive_context(project, handoff=<id>)` and `hive context <p> --handoff <id>` return only the
  framed, untrusted envelope. Goal, outcome, every dead end and every artifact path are never truncated; the
  rest is packed into the writer's role budget (implementer 1200, reviewer/verifier 800) with a pointer to the
  note. Bad ids, tampered notes, plain codices and quarantined writers are refused.
- **Concise MCP responses (DETAIL-01)**: `detail: concise|detailed` on `hive_search`, `hive_graph` and
  `hive_lint`; MCP defaults to concise, the CLI and `tool_*` functions stay detailed (bytes pinned by
  `tests/fixtures/detail_snapshots.json`). New `lint.format_concise`. Gate: concise/detailed = 0.18 on the fixture.
- **Artifact offload (DETAIL-02, `src/artifact.rs`, `artifacts.py`)**: tool results over 2,000 est tokens
  (except `hive_context` and errors) are secret-scrubbed into a content-addressed cache outside the vault
  (`HIVE_ARTIFACTS`, else `LOCALAPPDATA`/`XDG_CACHE_HOME` `hive/artifacts`), purged after 7 days; the response
  keeps a 400-token head and a pointer.
- **README**: animated SVG cards (`assets/readme/`), agent and tool logos (Simple Icons CC0, Lobe Icons MIT with
  bundled notice), and an offline interactive playground for trace decay and quorum (`docs/playground/`).

### Changed
- Python tests moved to `tests/python/` and shared fixtures to `tests/fixtures/`; `python -m unittest` still
  discovers all of them from the repo root. The Phase 7 timing gate now takes the best of 3 calls.

## [2.5.0] - 2026-11-24

### Added
- **OmniRoute Tiered Intelligence Engine (`src/llm.rs`, `llm.py`)**:
  - Connects to local OmniRoute proxy (`http://127.0.0.1:20128/v1`) via minimal blocking HTTP client (`ureq`).
  - Supports 3 named combos: `hive-private` (local-only, fail-closed), `hive-fast` (local / fast cloud, fail-open), and `hive-deep` (surplus cloud quota, fail-open).
  - Pre-egress secret scrubbing (`F10`) redacts API keys, tokens, and private keys before JSON serialization.
  - Privacy hard-locking: `llm_privacy = "private"` in `[projects.<slug>]` or `[llm]` hard-locks to `hive-private` and rejects cloud fallbacks.
  - Fail-open resilience: returns empty structured fallbacks when OmniRoute is offline or unreachable.
- **Curation Engine (`hive curate`, `src/curate.rs`, `curate.py`)**:
  - Reads session telemetry from `ledger.jsonl`, playbooks, and zero-hit queries.
  - Enforces deterministic validation gate: bullet ID existence, secret checks, max 300 char lengths, max 5 merges.
  - Writes structured synthesis proposals exclusively to `vault/inbox/curate-<date>-<hash>.md` for human review; canonical notes are never modified directly.
- **ADR 0006**: Documented OmniRoute-backed tiered intelligence engine and curation gate.

## [2.2.0] - 2026-11-24

### Added
- **Objective Outcome Scorer (`hive ledger score`, `src/ledger.rs`, `ledger.py`)**:
  - Correlates session ledger (`ledger.jsonl`) against git commit hashes and test exit codes.
  - Deterministically reinforces helpful bullets (+1) and penalizes harmful bullets (-1) under atomic `PlaybookLock`.
- **Recurring Gap Miner (`hive mine`, `src/miner.rs`, `miner.py`)**:
  - Analyzes zero-hit queries and unfulfilled skill intents from `ledger.jsonl`.
  - Clusters unfulfilled user needs via token Jaccard similarity and drafts structured proposal notes in `vault/inbox/gap-<hash>.md`.
- **Playbook Time Decay & ADR Symbol-Staleness (`src/ace.rs`, `src/lint.rs`)**:
  - Time-based decay automatically prunes inactive bullets (`seen >= 10`, 0 helpful, >30 days old) during `apply`.
  - Symbol-anchored lint cross-checks ADR backticked code identifiers against `graphify-out/manifest.json`.
- **ADR 0005**: Documented deterministic self-improvement loop.

## [2.1.1] - 2026-11-24

### Security & Hardening
- **Supply-Chain Security Audit Gate (`audit_skill_safety`)**:
  - Blocks CVE-2021-42574 Trojan Source bidirectional Unicode overrides, hidden BOMs, zero-width characters.
  - Blocks prompt injection directives (`system prompt override`, `ignore previous instructions`).
  - Blocks dangerous shell exfiltration patterns (`curl | sh`, `rm -rf`, env dumps).
  - Enforces cryptographic SHA-256 tree hashing in `.skill-lock.json` with atomic rollback on audit failure.
- **Skill Routing Calibration**:
  - Calibrated prefix-stemming matcher with exact trigger priority against 20-query test set (`skill_golden.json`), reaching 100% routing accuracy.
- **Cross-Platform Pre-Commit Gate (`hive pre-commit`)**:
  - Unified native pre-commit subcommand executing deterministic lint and test suites across Windows, Linux, and macOS in <8 seconds.

## [2.1.0] - 2026-11-24

### Added
- **Auto-Skill Discovery & Dynamic Installation Engine (`hive_skill`, `hive skill`)**:
  - Multi-root filesystem scanner (`.agent/skills`, `.claude/skills`, `.gemini/skills`, `~/.config/skills`).
  - Intent matching with stopword removal, token stemming, and confidence scoring.
  - Stdio MCP tool `hive_skill(query, limit=3)` providing on-demand instruction injection into agent context.
  - Remote skill search and safe installation pipeline.

## [2.0.0] - 2026-11-24

### Added
- **Standalone Native Rust Core (`src/main.rs`, `src/lib.rs`)**:
  - Static 1.8 MB binary `hive.exe` with bundled SQLite FTS5.
  - Sub-millisecond cold start (<1 ms).
  - Graph Personalized PageRank in 7.8–32.6 ms (2-3x speedup).
  - Full feature parity with Python implementation across all subcommands.
- **Hash-Verified Refresh (`SYNC-01`)**:
  - Database schema user_version 4 with file size and SHA-256 hashing to skip redundant reindexing of unchanged notes.
- **Unified 4-Tier Configuration (`CONF-01`)**:
  - Priority chain: CLI flags -> Environment variables -> `config.toml` -> OS platform cache defaults.
- **Binary Delegation Shims (`SHIM-01`)**:
  - Transparent redirection in `hive.py`, `ace.py`, and `lint.py` to `hive.exe` when compiled, with `HIVE_FORCE_PYTHON=1` fallback.
- **ADR 0004**: Documented full Rust core migration.

## [1.5.0] - 2026-11-24

### Added
- **Atomic Playbook Lockfile (`LOCK-01`)**:
  - Cross-platform atomic file locking (`O_CREAT | O_EXCL`) in Rust and Python with dead-PID stale lock recovery.
- **Session Telemetry Ledger (`LEDG-01`)**:
  - Fail-open append-only JSONL event logging (`ledger.jsonl`) recording queries, hits, and injected bullets.
- **Cross-Agent Lifecycle Hooks (`HOOK-01`)**:
  - Claude Code and Codex hooks automatically recording session starts and reflections.

## [1.1.0] - 2026-11-24

### Added
- Milestone 1 correctness and security lock:
  - FTS5 BM25 search with title weighting and type priors.
  - graphify code graph loader and hub-dampened Personalized PageRank.
  - Context packer with 4-char token estimation and strict budget truncation.
  - Deterministic vault lint checks and git pre-commit hook.
