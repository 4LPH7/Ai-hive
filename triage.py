"""Inbox triage with Laya (convaiinnovations/laya): which project and what kind is each inbox capture.

Laya only proposes. It writes one report note, inbox/_triage.md, and never moves, edits or deletes a note.
You confirm or correct a proposal; each confirmation is logged outside the vault and fed back to Laya's
Learner, so its answers improve with use. The vault itself is the day-one training set: every ADR,
_memory.md, journal entry and playbook bullet already has a known project and kind.

Runs in the Laya venv (Python 3.12, torch CPU), not the hive's stdlib interpreter:
    D:\\PROJECTS\\ai-hive-laya\\.venv\\Scripts\\python.exe triage.py scan
    ... triage.py confirm inbox/<note>.md [project=<slug>] [kind=<kind>]   no overrides = accept the proposal
    ... triage.py eval                                                     held-out accuracy on vault labels
"""
import hashlib
import json
import os
import random
import re
import sys
import time
from pathlib import Path

import hive

STATE = Path(os.environ.get("HIVE_LAYA_STATE") or hive.DB_PATH.parent / "laya")
LABELS = STATE / "labels.jsonl"        # confirmed answers: the human-verified training signal
PROPOSALS = STATE / "proposals.json"   # last scan, so `confirm <note>` without overrides accepts it
REPORT = "inbox/_triage.md"
MAX_CHARS = 3000  # the reader sees title + start of body; captures state their point early
GLOBAL = "global"
KINDS = {
    "decision": "a lasting technical or product choice, with its reasons (an ADR candidate)",
    "lesson": "a reusable how-to, pitfall or command that applies beyond one session",
    "status": "progress, current state of work, or what to do next",
    "research": "findings from reading papers, docs or tools, with sources",
    "noise": "a scratch capture with no lasting value",
}
JOURNAL_ENTRY = re.compile(r"^## \d{2}:\d{2} [a-z]+ - ([^|\s(]+)", re.M)
BULLET = re.compile(r"^- \[[a-z]+-\d+\] helpful=\d+ harmful=\d+ :: (.+)$", re.M)


def read(path):
    return Path(path).read_text(encoding="utf-8", errors="replace")


def text_of(note_text):
    _, title, body = hive.parse_note(note_text)
    return (title + "\n" + body.strip())[:MAX_CHARS]


def describe(memory_text):
    """First prose line of a _memory.md: what the project is, in its own words."""
    for line in hive.parse_note(memory_text)[2].splitlines():
        line = line.strip()
        if line and not line.startswith(("#", ">", "Canonical state")):
            return line[:200]
    return ""


def projects(vault):
    root = Path(vault) / "projects"
    found = {}
    for d in sorted(root.iterdir()) if root.is_dir() else []:
        memory = d / "_memory.md"
        if d.is_dir() and not d.name.startswith("_") and memory.is_file():
            found[d.name] = describe(read(memory)) or d.name
    return found


def questions(vault):
    criteria = {**projects(vault), GLOBAL: "machine setup, agent tooling, or several projects at once"}
    return {
        "project": {"type": "choice", "instructions": "Which project is this note about?", "criteria": criteria},
        "kind": {"type": "choice", "instructions": "What kind of note is this?", "criteria": dict(KINDS)},
    }


def vault_examples(vault):
    """Labelled (text, answers) pairs the vault already holds; inbox notes are excluded (they are the task)."""
    vault, slugs, out = Path(vault), set(projects(vault)), []
    for slug in slugs:
        out.append((text_of(read(vault / "projects" / slug / "_memory.md")), {"project": slug, "kind": "status"}))
        for adr in sorted((vault / "projects" / slug / "adrs").glob("*.md")):
            out.append((text_of(read(adr)), {"project": slug, "kind": "decision"}))
    for journal in sorted((vault / "journal").glob("*.md")):
        body = hive.parse_note(read(journal))[2]
        starts = list(JOURNAL_ENTRY.finditer(body))
        for i, m in enumerate(starts):
            end = starts[i + 1].start() if i + 1 < len(starts) else len(body)
            project = m.group(1) if m.group(1) in slugs else GLOBAL
            entry = body[m.end():end].split("\n", 1)[-1].strip()  # drop the header: it names the answer
            if entry:
                out.append((entry[:MAX_CHARS], {"project": project, "kind": "status"}))
    for playbook in [vault / "_meta" / "playbook.md", *sorted(vault.glob("projects/*/playbook.md"))]:
        project = playbook.parent.name if playbook.parent.parent.name == "projects" else GLOBAL
        if playbook.is_file():
            out += [(m.group(1), {"project": project, "kind": "lesson"}) for m in BULLET.finditer(read(playbook))]
    return out


def inbox_items(vault):
    inbox = Path(vault) / "inbox"
    return sorted(p.relative_to(vault).as_posix() for p in inbox.glob("*.md") if p.name != Path(REPORT).name)


def confirmed_labels(qs):
    """Logged confirmations whose answers are still valid options (a project may have been removed)."""
    if not LABELS.is_file():
        return []
    out = []
    for line in LABELS.read_text(encoding="utf-8").splitlines():
        row = json.loads(line)
        answers = {q: a for q, a in row["answers"].items() if q in qs and a in qs[q]["criteria"]}
        if answers:
            out.append((row["text"], answers))
    return out


def render_report(rows, stamp):
    lines = ["---", "type: note", "project: all", "agent: laya", "status: active", f"updated: {stamp[:10]}",
             "source: D:/PROJECTS/ai-hive/triage.py scan", "---", "# Inbox triage (Laya proposals)", "",
             f"Generated {stamp}. Proposals only: nothing was moved. `review` means Laya was unsure.",
             "Accept: `triage.py confirm <note>`. Correct: `triage.py confirm <note> project=<slug> kind=<kind>`.", "",
             "| Note | Project | Kind | Route |", "|------|---------|------|-------|"]
    for r in rows:
        cells = [f"{r[q]['answer']} ({r[q]['p']:.0%})" for q in ("project", "kind")]
        lines.append(f"| [[{Path(r['note']).stem}]] | {cells[0]} | {cells[1]} | {r['route']} |")
    if not rows:
        lines.append("| (inbox empty) | | | |")
    return "\n".join(lines) + "\n"


# ------------------------------------------------------------ Laya (heavy imports from here on)

def bundle_path(qs):
    """One fitted bundle per question set: a new project changes the options and forces a refit."""
    key = hashlib.sha256(json.dumps(qs, sort_keys=True).encode("utf-8")).hexdigest()[:12]
    return STATE / f"bundle-{key}.zip"


def learner(vault):
    """Load the cached bundle and teach it confirmations it has not seen; fit from scratch only when the
    questions changed (fitting reads every example through the reader: minutes on CPU)."""
    from laya import Learner  # torch + reader model; imported only when deciding
    qs = questions(vault)
    labels = confirmed_labels(qs)
    bundle, seen_file = bundle_path(qs), STATE / "bundle-seen.json"
    seen = json.loads(seen_file.read_text(encoding="utf-8")).get(bundle.name, 0) if seen_file.is_file() else 0
    if bundle.is_file():
        model = Learner(bundle=str(bundle), auto=False)
        for text, answers in labels[seen:]:
            model.feedback(text, answers, correction=True)
        if len(labels) > seen:
            model.step(min_rows=1)
    else:
        model = Learner(qs, examples=vault_examples(vault) + labels, auto=False)
        for old in STATE.glob("bundle-*.zip"):
            old.unlink()
    if not bundle.is_file() or len(labels) > seen:
        STATE.mkdir(parents=True, exist_ok=True)
        model.save_bundle(str(bundle))
        seen_file.write_text(json.dumps({bundle.name: len(labels)}), encoding="utf-8")
    return model, qs


def summarize(result, qs):
    """Normalise one Learner.decide() result to answer, probability and route per question.
    Route is the model card's unlabelled rule: review when Laya abstains or its conformal
    prediction set holds more than one option. Never gate on the raw confidence number."""
    out, route = {}, "auto"
    for q in qs:
        r = result["answers"][q]
        out[q] = {"answer": r["answer"], "p": float(r.get("confidence") or 0.0)}
        if r.get("abstain") or len(r.get("prediction_set") or [r["answer"]]) > 1:
            route = "review"
    out["route"] = route
    return out


def scan(vault=hive.VAULT):
    vault = Path(vault)
    items = inbox_items(vault)
    model, qs = learner(vault)
    rows = [{"note": rel, **summarize(model.decide(text_of(read(vault / rel))), qs)} for rel in items]
    STATE.mkdir(parents=True, exist_ok=True)
    PROPOSALS.write_text(json.dumps({r["note"]: {q: r[q]["answer"] for q in qs} for r in rows}, indent=1),
                         encoding="utf-8")
    stamp = time.strftime("%Y-%m-%d %H:%M")
    (vault / REPORT).write_text(render_report(rows, stamp), encoding="utf-8")
    return f"triaged {len(rows)} inbox note(s) -> {REPORT}"


def confirm(note, *overrides, vault=hive.VAULT):
    vault, qs = Path(vault), questions(vault)
    proposals = json.loads(PROPOSALS.read_text(encoding="utf-8")) if PROPOSALS.is_file() else {}
    answers = dict(proposals.get(note, {}))
    for item in overrides:
        key, sep, value = item.partition("=")
        if not sep or key not in qs:
            raise ValueError(f"override {item!r} is not project=<slug> or kind=<kind>")
        answers[key] = value
    for q, a in answers.items():
        if a not in qs[q]["criteria"]:
            raise ValueError(f"{q}={a!r} is not one of {sorted(qs[q]['criteria'])}")
    if set(answers) != set(qs):
        raise ValueError(f"no proposal for {note}; run scan first or give project= and kind=")
    STATE.mkdir(parents=True, exist_ok=True)
    row = {"note": note, "answers": answers, "text": text_of(read(vault / note)), "at": time.strftime("%Y-%m-%dT%H:%M")}
    with LABELS.open("a", encoding="utf-8") as fh:
        fh.write(json.dumps(row, ensure_ascii=False) + "\n")
    return f"confirmed {note}: {answers} (learned on next scan; {len(confirmed_labels(qs))} confirmations total)"


def evaluate(vault=hive.VAULT, holdout=0.25, seed=7):
    """Fit on 75% of vault labels, report accuracy on the other 25%: how far Laya can be trusted today."""
    from laya import Learner
    qs = questions(vault)
    rows = vault_examples(vault)
    random.Random(seed).shuffle(rows)
    cut = int(len(rows) * (1 - holdout))
    model = Learner(qs, examples=rows[:cut], auto=False)
    test = rows[cut:]
    hits = {q: 0 for q in qs}
    auto = {"n": 0, "right": 0}
    for text, gold in test:
        s = summarize(model.decide(text), qs)
        for q in qs:
            hits[q] += s[q]["answer"] == gold[q]
        if s["route"] == "auto":
            auto["n"] += 1
            auto["right"] += all(s[q]["answer"] == gold[q] for q in qs)
    n = len(test)
    lines = [f"trained on {cut}, tested on {n} held-out vault notes"]
    lines += [f"  {q}: {hits[q]}/{n} = {hits[q] / n:.0%}" for q in qs]
    if auto["n"]:
        lines.append(f"  routed auto: {auto['n']}/{n}, fully right {auto['right'] / auto['n']:.0%}")
    return "\n".join(lines)


def main(argv):
    cmd, *rest = argv or ["scan"]
    sys.stdout.reconfigure(encoding="utf-8")
    commands = {"scan": scan, "confirm": confirm, "eval": evaluate}
    if cmd not in commands or (cmd == "confirm" and not rest):
        print(__doc__)
        return 2
    print(commands[cmd](*rest))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
