"""Session codices (v2.6.0, CODEX-02); Python mirror of src/codex.rs.

`hive codex write <project> <json>` turns a hive-ace checkpoint record into an immutable vault note
`codices/<project>/YYYY-MM-DD-<sha8>.md` whose frontmatter carries the body's sha256 (tamper check, CODEX-04).
Each dead end also becomes a DEAD_END trace in the disposable hive.sqlite.
"""
import hashlib
from pathlib import Path

import ace
import hive
import traces

MAX_LINES = 40
MAX_ITEM_CHARS = 300
TEXT_FIELDS = ("goal", "outcome")
LIST_FIELDS = ("files", "dead_ends", "invariants", "open_threads")
SECTIONS = (("Files", "files"), ("Dead ends", "dead_ends"), ("Invariants", "invariants"),
            ("Open threads", "open_threads"))


def _check_text(name, value):
    if not isinstance(value, str) or not value.strip():
        raise ValueError(f"codex: {name} must be a non-empty string")
    if "\n" in value or "\r" in value or len(value) > MAX_ITEM_CHARS:
        raise ValueError(f"codex: {name} must be one line of at most {MAX_ITEM_CHARS} chars")
    if ace.SECRET_RE.search(value):
        raise ValueError(f"codex: {name} looks like it contains a secret")


def validate(rec):
    if not isinstance(rec, dict) or set(rec) != set(TEXT_FIELDS + LIST_FIELDS):
        raise ValueError(f"codex: record needs exactly {', '.join(TEXT_FIELDS + LIST_FIELDS)}")
    for name in TEXT_FIELDS:
        _check_text(name, rec[name])
    for name in LIST_FIELDS:
        if not isinstance(rec[name], list):
            raise ValueError(f"codex: {name} must be a list")
        for item in rec[name]:
            _check_text(name, item)


def render(project, rec, agent, session, date):
    """(sha256 of body, full note text). Deterministic: the Rust side renders byte-identical output."""
    body = [f"# {rec['goal']}", "", f"outcome: {rec['outcome']}", ""]
    for title, key in SECTIONS:
        body += [f"## {title}"] + ([f"- {x}" for x in rec[key]] or ["- none"])
    body_text = "\n".join(body) + "\n"
    sha = hashlib.sha256(body_text.encode("utf-8")).hexdigest()
    head = ["---", "type: codex", f"project: {project}", f"agent: {agent}", "status: active",
            f"updated: {date}", f"source: session {session}", f"content_sha256: {sha}", "---"]
    return sha, "\n".join(head) + "\n" + body_text


def verify(text):
    """CODEX-04: None when the body matches its content_sha256, else the reason. CRLF/BOM are normalized
    first so a git autocrlf checkout does not look like tampering."""
    text = text.lstrip("﻿").replace("\r\n", "\n")
    head, sep, body = text.partition("\n---\n")
    if not sep or not head.startswith("---"):
        return "frontmatter block missing"
    stored = next((l.split(":", 1)[1].strip() for l in head.splitlines() if l.startswith("content_sha256:")), "")
    if not stored:
        return "content_sha256 missing"
    if stored != hashlib.sha256(body.encode("utf-8")).hexdigest():
        return "content_sha256 mismatch: body was edited after writing (codices are immutable)"
    return None


CONTEXT_MAX_LINES = 6
CONTEXT_MAX_CODICES = 20
CONTEXT_KINDS = {"Dead ends": "dead end", "Open threads": "open thread"}


def _sections(text):
    """{section title: [items]} from a rendered codex body; '- none' placeholders dropped."""
    out, cur = {}, None
    for line in text.splitlines():
        if line.startswith("## "):
            cur = out.setdefault(line[3:].strip(), [])
        elif cur is not None and line.startswith("- ") and line != "- none":
            cur.append(line[2:])
    return out


def context_section(vault, project, task):
    """CODEX-03: Dead ends + Open threads from the newest codices whose Files overlap the task."""
    want = traces._words(task)
    folder = Path(vault) / "codices" / project
    if not want or not folder.is_dir():
        return ""
    lines = []
    for note in sorted(folder.glob("*.md"), reverse=True)[:CONTEXT_MAX_CODICES]:
        secs = _sections(note.read_text(encoding="utf-8", errors="replace"))
        if not traces._words(" ".join(secs.get("Files", []))) & want:
            continue
        lines += [f"- {kind}: {item} ({note.stem})" for title, kind in CONTEXT_KINDS.items()
                  for item in secs.get(title, [])]
    lines = lines[:CONTEXT_MAX_LINES]
    return "## codex notes for this task (dead ends, open threads)\n" + "\n".join(lines) if lines else ""


def write(vault, db, project, rec, agent, session, date, now):
    """Validate, write the note (never overwrite), leave DEAD_END traces; returns the note path."""
    project = hive.validate_slug(project)
    validate(rec)
    sha, text = render(project, rec, agent, session, date)
    if len(text.splitlines()) > MAX_LINES:
        raise ValueError(f"codex: note would be {len(text.splitlines())} lines (max {MAX_LINES}); trim the record")
    path = Path(vault) / "codices" / project / f"{date}-{sha[:8]}.md"
    path.parent.mkdir(parents=True, exist_ok=True)
    try:
        with open(path, "x", encoding="utf-8", newline="\n") as fh:
            fh.write(text)
    except FileExistsError:
        raise ValueError(f"codex: {path} already exists; codices are never edited") from None
    target = rec["files"][0] if rec["files"] else "session"
    for dead_end in rec["dead_ends"]:
        traces.add(db, project, "DEAD_END", target, dead_end[:traces.MAX_SUMMARY_CHARS], agent, session, now)
    return path
