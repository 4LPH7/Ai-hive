# Vault lint

`lint.py` checks the vault against the rules in its `AGENTS.md` manual. It is deterministic, stdlib-only and
read-only: it reports, you or an agent fix.

```bash
python lint.py [vault]        # exit code 1 if any ERROR
python hive.py lint           # same report
```

Both work under `python -I`, the way agents and git hooks launch them. As a git pre-commit hook
(`examples/vault/.githooks/pre-commit`), lint blocks any vault commit that has an ERROR.

Agents call it as the `hive_lint` MCP tool before declaring a session done.

## Checks

| Code | Severity | Fires when | Fix |
|------|----------|-----------|-----|
| `frontmatter-missing` | ERROR | A note has no frontmatter, or lacks `type`, `project`, `agent`, `status` or `updated`. Root `AGENTS.md`, `CLAUDE.md`, `GEMINI.md` are exempt | Add the block from the matching template |
| `bad-value` | ERROR | `type`, `status` or `agent` is not an allowed value | Use a listed value |
| `bad-date` | ERROR | `updated` is not `YYYY-MM-DD` | Fix the date |
| `line-cap` | ERROR | A `_memory.md` exceeds 150 lines or `_meta/active-work.md` exceeds 40 | Rewrite it shorter; these are state, not logs |
| `superseded-unlinked` | ERROR | An ADR has `status: superseded` but links to no other note | Link the ADR that replaced it |
| `broken-link` | WARN | A `[[wikilink]]` matches no note or file. Code spans, fenced blocks, aliases (`\|`, including table-escaped `\\|`), headings and attachments are handled | Fix the target or create the note |
| `stale` | WARN | An active `_memory.md` was not updated for 30 days | Verify it still describes the project |
| `duplicate-name` | WARN | Two notes share a file name, so `[[name]]` is ambiguous. `_memory` and `playbook` are exempt | Rename one, or link by path |
| `journal-header` | WARN | A `## ` heading in a journal is not `## HH:MM <agent> - <project> [\| <kind>]` | New entries follow the format; old ones are append-only history |
| `source-missing` | WARN | An ADR or note written by an agent under `projects/` or `inbox/`, updated on or after 2026-11-23, has no `source:` | Add the commit, session, URL or file the claims come from |
| `memory-undescribed` | WARN | A `_memory.md` has no `## What it is` section. Without it, search cannot match the project by what it is (a portfolio site's memory scored 0 for "personal website") | Add one or two sentences saying what the project is |
| `orphan` | INFO | A project note has no inbound links, so it is invisible in Obsidian's graph. Journal and inbox are exempt | Link it from the project's `_memory.md` |

## Why `source:`

Agent memory compounds: a wrong note becomes the prior for every later session. A `source:` makes each claim
traceable, so it can be checked and superseded instead of silently trusted. The rule applies only from the
day it was introduced, so the existing history does not light up with warnings.

## Changing the rules

Constants at the top of `lint.py`: `ALLOWED`, `REQUIRED`, `LINE_CAPS`, `MEMORY_CAP`, `STALE_DAYS`,
`CONVENTIONAL_NAMES`, `SOURCE_RULE_FROM`. Add a test in `tests/python/test_lint.py` first.
