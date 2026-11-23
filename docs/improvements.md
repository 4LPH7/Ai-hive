# Improvements roadmap

Ideas that are worth doing but are not one-line changes. Each has the problem it solves, the smallest
version that would work, and the signal that says it is time. Simple fixes are not listed here; they were
just done (see the [changelog](../CHANGELOG.md)).

Order is by value for effort at the current scale (tens of notes, graphs of 1k-3.5k nodes).

## 1. Lint only what is being committed

**Problem.** The vault pre-commit hook lints the whole working folder, so one untracked broken note blocks
every agent's commit, even commits that do not touch it.

**Smallest version.** In the hook, pass `git diff --cached --name-only --diff-filter=ACM -- '*.md'` to
`lint.py` and report per-file checks only for those paths. Link checks still need the full file list to
resolve targets, so lint gets two inputs: all files (for resolution) and staged files (for reporting).

**When.** The first time an agent's unrelated untracked note blocks a commit.

## 2. Codex reflection checkpoint

**Problem.** Claude Code gets the Stop-hook nudge to run `hive-ace` after substantial work; Codex only gets
playbook injection at session start and relies on the `AGENTS.md` session-end protocol for reflection.

**Smallest version.** Codex fires a `Stop` hook with a `transcript_path`. Add a Codex transcript reader to
`ace.count_tools` (its rollout format differs from Claude Code's JSONL) and register `ace.py hook-stop` in
`~/.codex/hooks.json`. Return `{"decision": "block", "reason": ...}` the same way.

**When.** Codex sessions end without new playbook lessons while Claude sessions keep adding them.

## 3. Automatic graph refresh after commits

**Problem.** `hive_graph` warns when a graph is stale, but someone still has to run `graphify update .`.

**Smallest version.** A `post-commit` hook per code repository that starts `graphify update .` in the
background and returns immediately (`start /b` on Windows, `&` elsewhere). graphify is incremental and leaves
`graph.json` untouched when topology did not change, so the cost is seconds.

**When.** Stale-graph warnings show up in more than one session a week. Install it per repo, never globally:
it writes into each repository's `.git/hooks`.

## 4. Note age inside `hive_context`

**Problem.** Agents see a note's content but not how old it is unless they read the frontmatter. Old project
memory is the most common source of wrong assumptions.

**Smallest version.** Prefix each part in `hive_context` with the note's `updated:` date and age in days
(`_memory.md, updated 2026-11-09, 14 days ago`). It costs a few tokens per part.

**When.** An agent acts on an outdated `_memory.md` even once.

## 5. Public evaluation fixture in CI

**Problem.** Ranking changes are measured on a personal golden set that cannot be published, so CI cannot
catch a ranking regression.

**Smallest version.** A small synthetic vault under `tests/fixture_vault/` (ten projects, ADRs, journals)
with its own `eval_golden.json`, and a test that asserts Recall@5 stays at its measured value.

**When.** Before anyone other than the author changes `hive.py` ranking constants.

## 6. Symbol-anchored staleness

**Problem.** A decision note that names a function stays "true" after the function changes.

**Smallest version.** An optional `affects:` frontmatter list of `file::symbol`; lint looks the symbols up in
the repo's `graph.json` and warns when the file changed after the note's `updated` date. graphify records
start lines, not ranges, so file-level change detection is the honest first step.

**When.** A stale decision note misleads an agent about code it governs.

## 7. Scale paths (not needed yet)

| Signal | Step |
|--------|------|
| A repo graph passes about 50k nodes (load over 1 s) | Cache the parsed graph per repo (pickle keyed on `graph.json` mtime) |
| Graph queries still slow after caching | Move Personalized PageRank to `rustworkx` (Rust, Python API) |
| Golden-set Recall@5 below about 0.9, or the vault passes a few thousand notes | Add embedding search and fuse it with BM25 (reciprocal rank fusion) |
| Two agents regularly edit the same repo at the same time | Advisory leases on files or symbols |

Measured today: a worst-case full MCP session (process start, handshake, `hive_context` with a 3.4k-node
graph) takes 322 ms, so none of these pay off yet. A full Rust rewrite would save about 0.3 s per session.
