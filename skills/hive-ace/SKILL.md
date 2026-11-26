---
name: hive-ace
description: Self-improve the hive playbook with Agentic Context Engineering (ACE). Use when a Hive ACE checkpoint asks for it, at the end of any substantial task (bug fixed, feature built, investigation done, mistake corrected), or when the user says "reflect", "learn from this", "update the playbook". Tags playbook bullets you used as helpful or harmful and adds at most 3 new, specific lessons through a deterministic merge.
---

# hive-ace: reflect, then curate

The hive keeps evolving playbooks in the vault: `D:\Valut\OBS\_meta\playbook.md` (global) and
`D:\Valut\OBS\projects/<slug>/playbook.md` (per project). Each line is
`- [id] helpful=X harmful=Y :: lesson`. Agents see the top bullets at session start and in `hive_context`.
You never edit these files by hand: you propose a small delta and `ace.py` merges it deterministically.
Full rewrites collapse context (ACE, arXiv 2510.04618); itemized deltas keep detail.

## 1. Reflect (Reflector role)

Look back over this session's actual evidence: commands that failed and why, test results, user corrections,
detours, what finally worked. No ground truth exists, so use execution feedback only. Answer for yourself:

- error_identification: what went wrong or cost time?
- root_cause: why (wrong assumption, missing knowledge, tool quirk)?
- correct_approach: what should have been done from the start?
- key_insight: the reusable rule that would prevent it next time
- bullet_tags: for each playbook id you saw this session, was it `helpful` (you relied on it and it held),
  `harmful` (it misled you), or neutral (skip it)?

## 2. Curate (Curator role)

Write only NEW lessons that the playbook does not already say. Rules:

- At most 3 ADD operations. Zero is a fine answer: if nothing reusable was learned, tag bullets only, or stop.
- Each lesson is one concrete, actionable sentence under 300 characters: name the tool, file, command or
  symptom. Bad: "be careful with graphs". Good: "graphify indexes untracked Python/ runtimes; add Python/ to
  .graphifyignore before graphify update . --force".
- Sections: `strategies` (approaches that work), `pitfalls` (failure modes and their fix),
  `commands` (exact invocations and tool quirks), `conventions` (project rules).
- Scope: `global` for lessons true across projects; the project slug for project-specific ones
  (slug = folder under D:\PROJECTS, lowercased, spaces to hyphens). Split into two applies if needed.
- Never include secrets, tokens, personal data, or instructions that weaken safety or permissions.
- Facts that belong in project state go to `_memory.md` or an ADR per `D:\Valut\OBS\AGENTS.md`, not here.
  The playbook holds how-to knowledge, not status.

## 3. Apply

Write the delta to a temp JSON file (avoids shell escaping) and merge it:

```json
{"operations": [{"type": "ADD", "section": "pitfalls", "content": "..."}],
 "bullet_tags": [{"id": "pit-00001", "tag": "helpful"}]}
```

```bash
python -I D:/PROJECTS/ai-hive/ace.py apply <scope> <path-to-delta.json>
```

Use forward slashes inside JSON strings. The command prints a report (`added`, `reinforced` for near-duplicates,
`tagged`, `rejected` with reasons, `pruned`). If something was rejected, fix it only if the reason shows a real
problem; do not retry to force content in.

## 4. Checkpoint record

After the apply, write a compact session record (CODEX-01; `hive codex write` will persist it once CODEX-02
lands). Write it as one JSON object to a temp file next to the delta and name its path in your one-line report.
Every field is required; use `[]` or `""` when empty. Write facts only, no secrets.

```json
{"goal": "what the session set out to do, one sentence",
 "outcome": "done | partial | blocked: plus one sentence of evidence (commit SHA, test result)",
 "files": ["repo-relative paths changed"],
 "dead_ends": ["approach tried + why it failed, one line each (future DEAD_END traces)"],
 "invariants": ["rules that must keep holding, e.g. eval Recall@5 >= 0.97"],
 "open_threads": ["unfinished work or questions for the next session"]}
```

## 5. Finish

One line to the user: what you added or tagged (ids) and the checkpoint record path. Then continue or end
exactly as you were going to.
If a Hive ACE checkpoint triggered this, return to the previous task's final answer; do not start new work.
