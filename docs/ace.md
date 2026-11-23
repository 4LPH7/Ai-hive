# ACE playbooks: how the hive learns

The hive implements **Agentic Context Engineering** (Zhang et al., [arXiv 2510.04618](https://arxiv.org/abs/2510.04618),
reference code [ace-agent/ace](https://github.com/ace-agent/ace)). Instead of fine-tuning a model, ACE grows a
*playbook*: an itemized list of lessons that is injected into context and improved after every task.

The key finding behind the design: when an LLM rewrites its whole context each time, it drifts into short,
generic summaries and loses the details that made it useful ("context collapse"). ACE therefore changes the
playbook only through small, itemized deltas that a deterministic program merges.

## Playbook format

One Markdown note per scope, `_meta/playbook.md` (global) and `projects/<slug>/playbook.md`:

```markdown
## pitfalls

- [pit-00001] helpful=3 harmful=0 :: graphify indexes untracked runtimes; add Python/ to .graphifyignore before graphify update . --force.
```

Sections: `strategies`, `pitfalls`, `commands`, `conventions` (id prefixes `str`, `pit`, `cmd`, `con`).
Never edit a playbook by hand.

## Roles

| ACE role | In ai-hive |
|----------|-----------|
| Generator | The live agent doing the task, with playbook bullets in its context |
| Reflector | The same agent running skill `hive-ace`: what failed, why, what the reusable rule is, which bullets helped or misled |
| Curator | `ace.py apply`: a deterministic merge, no LLM |

## Delta format

```json
{"operations": [{"type": "ADD", "section": "pitfalls", "content": "one concrete sentence"}],
 "bullet_tags": [{"id": "pit-00001", "tag": "helpful"}]}
```

```bash
python ace.py apply global delta.json        # or a project slug instead of global
```

## Merge rules (`apply_ops`)

| Rule | Value |
|------|-------|
| Max ADD operations per apply | 5 (the skill asks for at most 3) |
| Max lesson length | 300 characters |
| Secret guard | rejects API keys, tokens, private keys, `password=...` patterns |
| Near-duplicate | token-set Jaccard >= 0.75 with an existing bullet: reinforce it (helpful + 1) instead of adding |
| Tags | `helpful` or `harmful` increment the counter |
| Prune | drop a bullet once harmful >= helpful + 3 |
| Cap | 80 bullets per playbook; the weakest net-evidence bullets go first |

`apply` prints a report: `added`, `reinforced`, `tagged`, `rejected` (with reasons), `pruned`.

## Read path

- `hive_context` adds the top bullets, ranked by net evidence (helpful minus harmful) times overlap with the task.
- The `SessionStart` hook (`ace.py hook-start`) injects up to 12 bullets for the global scope and the scope of
  the current folder. Wired for Claude Code and Codex (same JSON output format).

## Triggers

- Claude Code `Stop` hook (`ace.py hook-stop`): once 25 or more tool calls have happened since the last
  reflection and at least one edit was made, it blocks the stop once with an instruction to run `hive-ace`,
  then lets the agent finish. It respects `stop_hook_active`, so it cannot loop. Set `HIVE_ACE_DISABLE=1` to
  turn it off.
- Codex and Antigravity: step 5 of the vault's session-end protocol asks them to run the skill.

## Writing good lessons

Bad: "be careful with graphs". Good: "graphify update leaves graph.json untouched when code topology is
unchanged, so built_at_commit lags HEAD; trust manifest-based freshness, not the commit." A lesson names the
tool, file, command or symptom, and says what to do. Project state belongs in `_memory.md`; decisions belong
in ADRs. The playbook holds how-to knowledge only.
