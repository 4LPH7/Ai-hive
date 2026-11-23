# Vault conventions

The hive works because every agent writes notes the same way. The full manual agents read is
[`examples/vault/AGENTS.md`](../examples/vault/AGENTS.md); this page explains the contract and why each rule
exists. `lint.py` enforces the parts that can be checked mechanically.

## Folder layout

| Path | Holds | Write rule | Why |
|------|-------|-----------|-----|
| `AGENTS.md` | The manual | Edit in place; `CLAUDE.md` and `GEMINI.md` only point to it | One source of truth for every agent |
| `_meta/active-work.md` | One table row per project in flight | **Overwrite**, max 40 lines | Read at every session start; must stay short |
| `_meta/environment.md` | Machine, tools, preferences | Edit in place | Stable facts, not a log |
| `projects/<slug>/_memory.md` | Canonical state of one project | **Overwrite**, max 150 lines | State, not history: history is in git and the journal |
| `projects/<slug>/adrs/NNNN-title.md` | One decision per file | Create; never rewrite; supersede | Decisions keep their original reasoning |
| `projects/<slug>/playbook.md`, `_meta/playbook.md` | ACE lessons | Only via `ace.py apply` | Deterministic merges prevent context collapse |
| `journal/YYYY-MM-DD.md` | Session log | **Append** only | Chronological record across agents |
| `inbox/` | Unsorted captures | Create; you (helped by Laya) triage | Nothing lands in canonical notes unreviewed |
| `templates/` | Note templates | Edit only on purpose | Keeps frontmatter uniform |

`<slug>` is the code folder name, lowercased, spaces to hyphens.

## Frontmatter

```yaml
---
type: project-memory | adr | journal | note
project: <slug> | all
agent: claude | codex | antigravity | user | laya
status: active | superseded | archived
updated: YYYY-MM-DD
source: <commit SHA, session, URL or file path>
---
```

- `type` and `status` feed search ranking (ADRs and project memory rank higher; superseded and archived notes
  rank lower) and Obsidian Bases dashboards.
- `updated` decides conflicts: if two notes disagree, the newer one wins and the older is fixed or superseded.
- `source` is required on ADRs and notes that agents write under `projects/` or `inbox/`: a claim nobody can
  trace becomes a wrong prior for every later session.

## Journal entries

```markdown
## 14:05 claude - ai-hive | fix
- What changed (2-5 lines)
- What is unresolved
```

`<kind>` is one of `build`, `fix`, `decision`, `research`, `setup`, `review`, so
`grep "| fix" journal/*.md` finds every fix across projects and agents.

## Session protocol

**Start:** `hive_context(project, task)`. Without the MCP tools: read `active-work.md`, the project's
`_memory.md`, the ADR file names, and search before asking the user to repeat anything.

**End:**
1. Overwrite `_memory.md` to describe the project now.
2. Add an ADR for each lasting, non-obvious decision; supersede old ones explicitly.
3. Append a journal entry.
4. Overwrite this project's row in `active-work.md`.
5. Run `hive_lint` and fix every ERROR.
6. Run skill `hive-ace` to tag the lessons you used and add new ones.

Skip all of it for trivial sessions.

## Hard rules

- Never store secrets. Name where a secret lives, never its value.
- Never delete or rename notes you did not create this session; archive them instead.
- Never put generated output (graph exports, runtimes, `node_modules`) in the vault.
- Do not touch `.obsidian/` or `.git/`.
- Facts from memory can be stale: verify a file, function or flag exists before acting on it.
