<!-- ai-hive template: copy to your vault root, replace <VAULT> and <PROJECTS> with your paths,
     and drop the hive sections if you run without the MCP server. -->
# Vault operating manual (Claude Code, Codex, any agent)

This Obsidian vault is the shared, persistent memory for every coding agent on this machine
(Claude Code, Codex, Antigravity).
`AGENTS.md` is the single source of truth. `CLAUDE.md` and `GEMINI.md` only point to it. Edit this file, never a copy.

Vault root: `<VAULT>`. Code lives in `<PROJECTS>\<folder>`.

## Layout

| Path | Holds | Write rule |
|------|-------|-----------|
| `_meta/active-work.md` | What is in flight right now, across projects | **Overwrite** (keep under 40 lines) |
| `_meta/environment.md` | Machine, tools, user preferences, project roots | Edit in place when facts change |
| `projects/<slug>/_memory.md` | Canonical state of one project | **Overwrite** (keep under 150 lines) |
| `projects/<slug>/adrs/NNNN-title.md` | One decision per file | Create; never rewrite, supersede instead |
| `journal/YYYY-MM-DD.md` | Append-only session log | **Append** only |
| `inbox/` | Unsorted captures from agents or the user | Create; the user triages |
| `templates/` | Note templates | Do not edit unless asked |
| `_meta/bases/*.base` | Dashboards (Obsidian Bases) over the frontmatter below | Do not edit unless asked; keep frontmatter accurate so they stay correct |
| `_meta/playbook.md`, `projects/<slug>/playbook.md` | ACE playbooks: reusable how-to lessons with helpful/harmful counters (see [[hive]]) | **Never hand-edit.** Change only via skill `hive-ace` (`ace.py apply`) |

`<slug>` = the project folder name under `<PROJECTS>`, lowercased, spaces to hyphens
(`Whatsapp bot` becomes `whatsapp-bot`).

## Session start (do this before the task)

**Fast path:** if the `hive` MCP tools are available (see [[hive]]), call
`hive_context(project=<slug>, task=<what you are about to do>)`. It returns the active-work row,
`_memory.md`, the ADR list, matching notes and ranked code-graph symbols in one budgeted call,
and replaces steps 1-3. Use `hive_search` instead of `rg` and `hive_graph(repo, query)` for code structure.
Without the tools, do the manual steps:

1. Read `_meta/active-work.md`.
2. Find the project slug from the current working directory. Read `projects/<slug>/_memory.md`.
   If it does not exist, continue; create it from `templates/project-memory.md` when you finish.
3. Skim `projects/<slug>/adrs/` filenames. Open only the ADRs relevant to the task.
4. Search the vault before asking the user to repeat context: `rg -i "<term>" <VAULT> --glob "*.md"`.

## Session end (do this before declaring done)

1. **Overwrite** `projects/<slug>/_memory.md` so it reflects the state *now*: current status, how to run it,
   open problems, next steps. Remove anything stale. It must never become a log.
2. Record each non-obvious, lasting decision (and why) as a new ADR from `templates/adr.md`.
   If it replaces an older ADR, set the old one's `status: superseded` and link the new one.
3. **Append** a short entry to today's `journal/YYYY-MM-DD.md` (create from `templates/daily.md` if missing):
   `## HH:MM <agent> - <project> | <kind>` then 2-5 lines: what changed, what is unresolved.
   `<kind>` is one of `build`, `fix`, `decision`, `research`, `setup`, `review` (optional, but it lets
   `grep "| fix" journal/*.md` find every fix across projects).
4. **Overwrite** the line for this project in `_meta/active-work.md`; drop projects that are finished.
5. **Self-improve:** run skill `hive-ace` (Claude, Codex and Antigravity all have it). Tag the playbook bullets you
   used as helpful or harmful, add at most 3 new concrete lessons. In Claude Code a Stop hook asks for this
   automatically after 25+ tool calls with edits.

Skip all of this for trivial sessions (a typo, a one-off question). Write only what a future session needs.

## Note conventions

- Every note starts with frontmatter:
  ```yaml
  ---
  type: project-memory | adr | journal | note
  project: <slug>
  agent: claude | codex | antigravity | user
  status: active | superseded | archived
  updated: YYYY-MM-DD
  source: <commit SHA, session, URL or file path the claims come from>
  ---
  ```
- `source:` is required on ADRs and notes an agent writes under `projects/` or `inbox/`. A claim nobody can trace
  becomes a wrong prior for every later session; with a source it can be checked and superseded.
- Update `updated:` and `agent:` whenever you edit a note.
- Before declaring done after writing notes, run `hive_lint` (or `python <PROJECTS>\ai-hive\hive.py lint`) and fix
  every ERROR.
- Link with `[[wikilinks]]` (note name, no extension). Link decisions from `_memory.md`.
- One idea per note. Prefer a new small note over growing an existing one past ~150 lines.
- File names: lowercase-kebab-case, except `_memory.md` and dated journal files.

## Hard rules

- **Never store secrets**: no API keys, tokens, passwords, `.env` contents, cookies, or personal IDs. Name where a secret lives, never its value.
- **Never delete or rename** notes you did not create this session. Mark `status: archived` or ask the user.
- **Never append to `_memory.md`, `active-work.md` or `environment.md`**; rewrite them cleanly so they stay canonical.
- Do not duplicate facts that the code, `git log`, or the repo's own `README`/`CLAUDE.md` already record. Link to them.
- Do not touch `.obsidian/` or `.git/`.
- Never put generated output in the vault (graphify Obsidian exports, runtimes, `node_modules`). Keep exports in the repo's `graphify-out/obsidian/`; `hive_graph` reads `graphify-out/graph.json` directly.
- If two notes disagree, the newer `updated:` wins; fix the older one or mark it superseded.
- Facts from memory can be stale. If a note names a file, function, or flag, verify it exists before acting on it.
