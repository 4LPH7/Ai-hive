# Laya inbox triage

`triage.py` uses [Laya](https://huggingface.co/convaiinnovations/laya) (`pip install laya`, 0.5.3) to answer two
questions about every note in `inbox/`:

| Question | Options |
|----------|---------|
| `project` | every `projects/<slug>/` that has a `_memory.md` (described by its first prose line), plus `global` |
| `kind` | `decision` (ADR candidate), `lesson` (playbook candidate), `status`, `research`, `noise` |

Laya is not a text generator. It is a decision head on a frozen reader LLM (Qwen3.5-0.8B by default) that
returns a typed answer, a calibrated confidence and a conformal prediction set in one forward pass. It
cannot hallucinate prose, which makes it a good gatekeeper; the writing stays with your agents.

## What it may and may not do

- It **proposes only**: it writes one report note, `inbox/_triage.md` (`agent: laya`, with a `source:`).
- It **never** moves, edits, renames or deletes a note, and never writes ADRs or playbooks.
- Its state (fitted bundle, last proposals, your confirmations) lives in `%LOCALAPPDATA%\hive\laya`,
  outside the vault.

## The loop

```
vault labels ─┐
              ├─► Learner fit (cached bundle per question set) ─► scan ─► inbox/_triage.md
confirmations ┘                                                             │
      ▲                                                                     ▼
      └──────────── labels.jsonl ◄── triage.py confirm <note> [project=..] [kind=..] ◄── you
```

1. **Day-one training set from the vault itself.** Every ADR is a `decision` of its project, every
   `_memory.md` and journal entry is `status`, every playbook bullet is a `lesson`. Journal headers name the
   project, so they are stripped: Laya has to learn the project from content.
2. **Scan.** Each inbox note (title plus the first 3,000 characters) gets both answers. A row is routed
   `review` when Laya abstains, when its prediction set holds more than one option, or when confidence is
   below 0.6.
3. **Confirm.** `triage.py confirm inbox/x.md` accepts the proposal; add `project=<slug>` or `kind=<kind>` to
   correct it. Confirmations are validated against the current options and appended to `labels.jsonl`.
4. **Learn.** The next scan feeds new confirmations into the cached model as corrections (about a minute).
   A new project changes the question set and triggers a full refit from vault labels plus all
   confirmations.

## Commands

Run with the Laya venv's interpreter, from the ai-hive folder:

```bash
set HF_HOME=D:\PROJECTS\ai-hive-laya\hf
set PY=D:\PROJECTS\ai-hive-laya\.venv\Scripts\python.exe
%PY% triage.py eval                                   # fit on 75% of vault labels, test on 25%
%PY% triage.py scan                                   # propose for every inbox note
%PY% triage.py confirm inbox/idea.md                  # accept the proposal
%PY% triage.py confirm inbox/idea.md kind=decision    # correct one answer
```

## Measured results

On the author's vault (76 labelled examples, laptop CPU, no GPU), 2026-11-23:

| Test | Result |
|------|--------|
| Held-out vault notes, project | 18/19 (95%) |
| Held-out vault notes, kind | 19/19 (100%) |
| Held-out rows routed `auto` that were fully right | 18/19 (95%): one wrong answer passed the gate |
| First real inbox scan, 2 notes | **both wrong** (project at 26-28% confidence, kind `status` instead of `research`), yet routed `auto` |
| Same 2 notes after confirming them | both right at 95% (memorised, not yet evidence of generalisation) |
| Fit from scratch / cached scan with new confirmations | about 15 min / about 1 min |
| One decision | about 0.5 s |

The held-out score flatters it: vault notes have strong structural cues (ADR sections, journal style),
while inbox captures do not, and the vault has no `research` or `noise` examples at all. The first real
scan exposed exactly that, and also showed that the uncalibrated prediction set can be a single option at
28% confidence. That is why the 0.6 confidence floor exists.

## Limits

- Expect to confirm 40-100 answers before trusting it; Laya's own benchmark reaches 91.6% after 40
  confirmations and 96.0% after 120 on its templates, not on your notes.
- Keep it proposal-only until a confirmed track record exists. Auto-filing is the natural next step, gated
  on route `auto` and a precision you have measured.
- `Learner.calibrate()` needs at least 30 labelled rows that were not used for fitting. Once you have them,
  calibrating would make the conformal sets meaningful and could replace the fixed floor.
- CPU only here: a fit reads every example through the reader. A GPU makes it seconds.
- Long notes are cut at 3,000 characters.
