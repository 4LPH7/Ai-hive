# Setup, step by step

From an empty folder to Claude Code, Codex and Antigravity sharing one memory. Paths use the author's
Windows layout (`D:\Valut\OBS` vault, `D:\PROJECTS\<repo>` code); substitute your own. Every path the hive
uses can be changed with an environment variable:

| Variable | Default | Meaning |
|----------|---------|---------|
| `HIVE_VAULT` | `D:\Valut\OBS` | Obsidian vault root |
| `HIVE_PROJECTS` | `D:\PROJECTS` | Folder holding one sub-folder per code repo |
| `HIVE_DB` | `%LOCALAPPDATA%\hive\hive.sqlite` | Disposable search index |
| `HIVE_ACE_STATE` | `%LOCALAPPDATA%\hive\ace-state.json` | Stop-hook counters |
| `HIVE_ACE_DISABLE` | unset | Set to `1` to silence the Stop-hook reflection prompt |
| `HIVE_LAYA_STATE` | `%LOCALAPPDATA%\hive\laya` | Laya bundle, proposals and confirmed labels |

## Step 1: Requirements

- Python 3.10 or newer for the hive itself (no packages).
- Git, and an Obsidian vault in a git repository (Obsidian is only a viewer; it never needs to be running).
- Optional: [graphify](https://docs.graphify.com) for code graphs: `uv tool install graphifyy`.
- Optional: Python 3.12 and about 4 GB of disk for Laya triage (Step 8).

## Step 2: Get the code and run the tests

```bash
git clone https://github.com/4LPH7/ai-hive.git D:\PROJECTS\ai-hive
cd D:\PROJECTS\ai-hive
python -m unittest
```

All tests use temporary folders; none touch your vault.

## Step 3: Lay out the vault

Create this structure (see [vault-conventions.md](vault-conventions.md) for the full contract):

```
vault/
├── AGENTS.md                 the manual every agent reads (start from examples/vault/AGENTS.md)
├── CLAUDE.md                 one line: @AGENTS.md
├── GEMINI.md                 one line pointing to AGENTS.md
├── _meta/active-work.md      one table row per project in flight
├── _meta/environment.md      machine, tools, preferences
├── projects/<slug>/_memory.md
├── projects/<slug>/adrs/0001-<title>.md
├── journal/YYYY-MM-DD.md
├── inbox/
└── templates/
```

`<slug>` is the code folder name lowercased with spaces turned into hyphens (`Whatsapp bot` becomes
`whatsapp-bot`). Commit the vault to git so every agent edit is reviewable and reversible.

Check it:

```bash
set HIVE_VAULT=D:\Valut\OBS
python hive.py status
python hive.py lint
```

## Step 4: Build code graphs (optional)

In each repository:

```bash
cd D:\PROJECTS\<repo>
graphify extract . --code-only        # tree-sitter, no model or API key
```

Add stray runtimes and build output to `.graphifyignore` first; an untracked `Python/` folder once inflated
a graph from 1.9k to 37k nodes. Keep `graphify-out/` out of git and out of the vault. Re-run
`graphify update .` after structural changes; `hive_graph` warns you when the graph is stale.

## Step 5: Register the MCP server in each agent

Use an absolute interpreter path and `-I` (isolated mode, ignores user site-packages and env hooks).
Ready-made snippets are in [`examples/`](../examples).

**Claude Code** (`~/.claude.json`, user scope):

```bash
claude mcp add --scope user --transport stdio hive -- C:\Python315\python.exe -I D:\PROJECTS\ai-hive\hive.py serve
```

**Codex** (`~/.codex/config.toml`):

```toml
[mcp_servers.hive]
command = 'C:\Python315\python.exe'
args = ['-I', 'D:\PROJECTS\ai-hive\hive.py', 'serve']
```

**Antigravity** (`~/.gemini/config/mcp_config.json`): see `examples/antigravity-mcp_config.json`.

Restart each agent and ask it to call `hive_status`.

## Step 6: Teach agents the protocol

Put the session start and end protocol in the vault's `AGENTS.md` (template:
[`examples/vault/AGENTS.md`](../examples/vault/AGENTS.md)) and point each agent's global instructions at it,
for example in `~/.claude/CLAUDE.md`:

```markdown
Persistent memory lives in the Obsidian vault at `D:\Valut\OBS`. At the start of any non-trivial task read
`D:\Valut\OBS\AGENTS.md` and follow its session start / session end protocol.
```

The protocol in short: call `hive_context(project, task)` first; at the end, overwrite `_memory.md`, add
ADRs for lasting decisions, append a journal entry, update `active-work.md`, run `hive_lint`, then reflect
with `hive-ace`.

## Step 7: Turn on self-improvement (ACE)

1. Install the skill for each agent: copy `skills/hive-ace/` to `~/.claude/skills/`, `~/.codex/skills/`
   and your Antigravity skills folder. Replace the `D:/PROJECTS/ai-hive` and `D:\Valut\OBS` paths inside
   `SKILL.md` with yours.
2. Add the Claude Code hooks from `examples/claude-settings-hooks.json` to `~/.claude/settings.json`.
   Append them next to any hooks you already have; do not replace the arrays.
   - `SessionStart` runs `ace.py hook-start`: the top 12 playbook lessons go into the session's context.
   - `Stop` runs `ace.py hook-stop`: after 25+ tool calls in a session with edits, it asks the agent once
     to run `hive-ace`. It never loops (`stop_hook_active` is respected).
3. Seed the global playbook with one or two real lessons:

   ```bash
   python ace.py apply global examples/ace-delta.example.json
   python ace.py show global
   ```

Codex and Antigravity have no equivalent hook wired here; `AGENTS.md` asks them to run `hive-ace` at
session end.

## Step 8: Add Laya inbox triage (optional)

```bash
mkdir D:\PROJECTS\ai-hive-laya && cd D:\PROJECTS\ai-hive-laya
uv venv --python 3.12 .venv
uv pip install --python .venv\Scripts\python.exe -r D:\PROJECTS\ai-hive\requirements-laya.txt
set HF_HOME=D:\PROJECTS\ai-hive-laya\hf          # about 2 GB of model files; keep it off a full C: drive

cd D:\PROJECTS\ai-hive
D:\PROJECTS\ai-hive-laya\.venv\Scripts\python.exe triage.py eval    # held-out accuracy on your vault
D:\PROJECTS\ai-hive-laya\.venv\Scripts\python.exe triage.py scan    # writes inbox/_triage.md
```

The first `eval` or `scan` downloads the reader model and fits on your vault's labelled notes (about
15 minutes on a laptop CPU). Later scans load the cached fit and take under a minute. Read
[laya-triage.md](laya-triage.md) before relying on it.

## Step 9: Keep it healthy

Create your own search golden set once you have a dozen notes: copy `examples/eval_golden.example.json` to
`eval_golden.json` (gitignored, it describes your projects) and write 20-30 questions you actually ask, each
with the note paths that should answer it.

| When | Run |
|------|-----|
| Before changing search ranking | `python hive.py eval` (keep Recall@5 at least where it was) |
| After writing notes | `python hive.py lint` (fix every ERROR) |
| After code structure changes | `graphify update .` in the repo |
| Weekly | `triage.py scan`, then confirm or correct the proposals |
| After moving the index or schema change | delete `hive.sqlite`; it rebuilds on the next call |

## Troubleshooting

- **Agent does not see the tools:** check the interpreter path in the registration, restart the agent,
  and run `python hive.py serve` by hand: it should wait silently for JSON on stdin.
- **JSON errors from `ace.py apply`:** shells can collapse backslashes. Write the delta to a file and use
  forward slashes in paths inside it.
- **`hive_graph` says the graph is stale:** run `graphify update .` in that repository.
- **Laya proposals look wrong:** that is expected before you have confirmed a few dozen answers; the report
  marks uncertain ones `review`. See [laya-triage.md](laya-triage.md#limits).
