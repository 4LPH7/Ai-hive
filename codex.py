"""Session codices (v2.6.0, CODEX-02); Python mirror of src/codex.rs.

`hive codex write <project> <json>` turns a hive-ace checkpoint record into an immutable vault note
`codices/<project>/YYYY-MM-DD-<sha8>.md` whose frontmatter carries the body's sha256 (tamper check, CODEX-04).
Each dead end also becomes a DEAD_END trace in the disposable hive.sqlite.
"""
import hashlib
import re
import unicodedata
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
# HAND-01: a handoff is a codex plus these 4 keys; all 10 are required and render 1 head line + 2 sections
HANDOFF_SECTIONS = (("Completed", "completed_plan_items"), ("Artifacts", "artifact_paths"))
STATUSES = ("SUCCESS", "PARTIAL", "ERROR")
ROLE_BUDGET = {"implementer": 1200, "reviewer": 800, "verifier": 800}  # HAND-02
HANDOFF_MAX_CORE_ITEMS = 6
MAX_PATH_CHARS = 200
HANDOFF_MAX_LINES = 80
SEP_TOKENS = 16  # pack joiners + omitted marker
ID_RE = re.compile(r"[0-9]{4}-[0-9]{2}-[0-9]{2}-[0-9a-f]{8}")  # ASCII only; use fullmatch
ARTIFACT_RE = re.compile(r"artifact:[0-9a-f]{64}")
BAD_PATH_RE = re.compile(r'[\x00-\x1f\x7f<>"|?*:$%]')  # $ and % would expand to a path outside the repo
# every char str.splitlines() breaks on: one must never forge a heading, item or handoff line on read
LINE_BREAKS = frozenset("\n\r\x0b\x0c\x1c\x1d\x1e\x85  ")


def _check_text(name, value):
    if not isinstance(value, str) or not value.strip():
        raise ValueError(f"codex: {name} must be a non-empty string")
    if LINE_BREAKS.intersection(value) or len(value) > MAX_ITEM_CHARS:
        raise ValueError(f"codex: {name} must be one line of at most {MAX_ITEM_CHARS} chars")
    if ace.SECRET_RE.search(value):
        raise ValueError(f"codex: {name} looks like it contains a secret")


def check_artifact_path(path):
    """Repo-relative path or artifact:<sha256>; a poisoned handoff must not point the receiver at ~/.ssh."""
    if ARTIFACT_RE.fullmatch(path):
        return
    if (len(path) > MAX_PATH_CHARS or path.startswith(("/", "\\", "~")) or BAD_PATH_RE.search(path)
            or any(unicodedata.category(c) == "Cf" for c in path) or ".." in re.split(r"[/\\]", path)):
        raise ValueError(f"handoff: artifact path {path[:60]!r} must be repo-relative (no .., ~, $, %, drive, "
                         f"leading /) or artifact:<sha256>, at most {MAX_PATH_CHARS} chars")


def _safe_label(text):
    """Frontmatter is not hashed: only plain id characters may reach the unquoted envelope heading."""
    return re.sub(r"[^A-Za-z0-9._-]", "?", text)[:64] or "unknown"


def validate(rec, kind="codex"):
    handoff = kind == "handoff"
    text_fields = TEXT_FIELDS + (("status", "role") if handoff else ())
    list_fields = LIST_FIELDS + (("completed_plan_items", "artifact_paths") if handoff else ())
    if not isinstance(rec, dict) or set(rec) != set(text_fields + list_fields):
        raise ValueError(f"{kind}: record needs exactly {', '.join(text_fields + list_fields)}")
    for name in text_fields:
        _check_text(name, rec[name])
    for name in list_fields:
        if not isinstance(rec[name], list):
            raise ValueError(f"{kind}: {name} must be a list")
        for item in rec[name]:
            _check_text(name, item)
    if not handoff:
        return
    if rec["status"] not in STATUSES:
        raise ValueError(f"handoff: status must be one of {', '.join(STATUSES)}")
    if rec["role"] not in ROLE_BUDGET:
        raise ValueError(f"handoff: role must be one of {', '.join(ROLE_BUDGET)}")
    for name in ("dead_ends", "artifact_paths"):
        if len(rec[name]) > HANDOFF_MAX_CORE_ITEMS:
            raise ValueError(f"handoff: at most {HANDOFF_MAX_CORE_ITEMS} {name} (they are never truncated)")
    for path in rec["artifact_paths"]:
        check_artifact_path(path)


def render(project, rec, agent, session, date):
    """(sha256 of body, full note text). Deterministic: the Rust side renders byte-identical output."""
    body = [f"# {rec['goal']}", "", f"outcome: {rec['outcome']}"]
    if rec.get("status"):  # HAND-01; a plain codex renders byte-identical to v2.6.0
        body.append(f"handoff: {rec['status']} role={rec['role']}")
    body.append("")
    for title, key in SECTIONS + (HANDOFF_SECTIONS if rec.get("status") else ()):
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
    import governance
    hidden = governance.quarantined_paths(vault)  # GOV-02
    lines = []
    for note in sorted(folder.glob("*.md"), reverse=True)[:CONTEXT_MAX_CODICES]:
        if note.relative_to(Path(vault)).as_posix() in hidden:
            continue
        secs = _sections(note.read_text(encoding="utf-8", errors="replace"))
        if not traces._words(" ".join(secs.get("Files", []))) & want:
            continue
        lines += [f"- {kind}: {item} ({note.stem})" for title, kind in CONTEXT_KINDS.items()
                  for item in secs.get(title, [])]
    lines = lines[:CONTEXT_MAX_LINES]
    return "## codex notes for this task (dead ends, open threads)\n" + "\n".join(lines) if lines else ""


def write(vault, db, project, rec, agent, session, date, now, kind="codex"):
    """Validate, write the note (never overwrite), leave DEAD_END traces; returns the note path."""
    project = hive.validate_slug(project)
    validate(rec, kind)
    sha, text = render(project, rec, agent, session, date)
    max_lines = HANDOFF_MAX_LINES if kind == "handoff" else MAX_LINES
    if len(text.splitlines()) > max_lines:
        raise ValueError(f"{kind}: note would be {len(text.splitlines())} lines (max {max_lines}); trim the record")
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


def write_handoff(vault, db, project, rec, agent, session, date, now):
    """HAND-01: a handoff is a codex note with the 4 handoff fields (80-line cap)."""
    return write(vault, db, project, rec, agent, session, date, now, kind="handoff")


def core_tokens(rec):
    """Est tokens of the protected core (goal, outcome, handoff line, dead ends, artifact paths)."""
    return hive.est_tokens("\n".join([rec["goal"], rec["outcome"], f"{rec['status']} role={rec['role']}",
                                      *rec["dead_ends"], *rec["artifact_paths"]]))


def _body_head(text):
    """(goal, outcome, handoff line) from the body lines before its first section."""
    goal = outcome = handoff = ""
    for line in text.replace("\r\n", "\n").partition("\n---\n")[2].splitlines():
        if line.startswith("## "):
            break
        if line.startswith("# "):
            goal = line[2:]
        elif line.startswith("outcome: "):
            outcome = line[9:]
        elif line.startswith("handoff: "):
            handoff = line[9:]
    return goal, outcome, handoff


def _quoted(title, items):
    return "\n".join([f"> ## {title}"] + [f"> - {x}" for x in items]) if items else ""


def _load_handoff(vault, project, hid):
    """(vault-relative path, absolute path, text) after the id, integrity, kind and quarantine checks."""
    import governance
    project = hive.validate_slug(project)
    if not isinstance(hid, str) or not ID_RE.fullmatch(hid):
        raise ValueError(f"handoff: bad id {hid!r} (want YYYY-MM-DD-<sha8> from hive handoff write)")
    rel = f"codices/{project}/{hid}.md"
    path = Path(vault) / rel
    try:
        text = path.read_text(encoding="utf-8")
    except FileNotFoundError:
        raise ValueError(f"handoff: {hid} not found in codices/{project}") from None
    if verify(text):
        raise ValueError(f"handoff: {hid} integrity check failed; see hive audit {hid}")
    if not _body_head(text)[2]:
        raise ValueError(f"handoff: {hid} is a codex, not a handoff")
    if governance.is_quarantined(governance.load_rules(vault), *governance.note_origin(text)):
        raise ValueError(f"handoff: {hid} is quarantined")
    return rel, path, text


def handoff_view(vault, project, hid, budget=1500):
    """HAND-01/02: the framed envelope of one handoff. The core (goal, outcome, handoff line, every dead end and
    artifact path) is never packed; the rest is packed into min(role budget, budget); a pointer to the immutable
    vault note is always appended."""
    import governance
    rel, path, text = _load_handoff(vault, project, hid)
    goal, outcome, line = _body_head(text)
    session, agent, _ = governance.note_origin(text)
    meta, _, _ = hive.parse_note(text)
    secs = _sections(text)
    core = [f"# {goal}", f"outcome: {outcome}", f"handoff: {line}"]
    for title in ("Dead ends", "Artifacts"):
        core += [f"## {title}"] + ([f"- {x}" for x in secs.get(title, [])] or ["- none"])
    parts = [f"## handoff {hid} from {_safe_label(agent)} (session {_safe_label(session)}, "
             f"{hive.note_age(meta.get('updated', ''))})\n" + hive.frame_untrusted(rel, meta, "\n".join(core))]
    if any(a.startswith("artifact:") for a in secs.get("Artifacts", [])):
        import artifacts
        parts.append(f"artifact:<sha> items live in {artifacts.artifacts_dir()}/<sha>.md")
    pointer = f"full envelope: {path.resolve()}"
    role = line.partition("role=")[2].strip()
    room = (min(ROLE_BUDGET.get(role, min(ROLE_BUDGET.values())), int(budget))
            - hive.est_tokens("\n\n".join(parts)) - hive.est_tokens(pointer) - SEP_TOKENS)
    rest = [(title, _quoted(title, secs.get(title, [])), weight) for title, weight in
            (("Open threads", 0.30), ("Invariants", 0.30), ("Completed", 0.25), ("Files", 0.15))]
    whole = "\n\n".join(body for _, body, _ in rest if body)
    if hive.est_tokens(whole) <= room:  # pack() drops sections under 15 tokens even when they fit
        packed = whole
    elif room > 0:
        packed = hive.pack(rest, budget=room).strip()
    else:  # the role budget yields, never the core
        names = [name for name, body, _ in rest if body]
        packed = f"[omitted_sections: {', '.join(names)}]" if names else ""
    return "\n\n".join(p for p in parts + [packed, pointer] if p)
