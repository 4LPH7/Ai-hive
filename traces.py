"""Stigmergic trace layer (v2.6.0, TRACE-01/02); Python mirror of src/trace.rs, same table.

Agents leave short decaying signals in the disposable hive.sqlite: strength halves every half_life_h hours,
traces under MIN_STRENGTH are hidden, rows older than 7 days are purged. Canonical Markdown is never touched.
(Named traces.py: `trace` is a stdlib module.)
"""
import json
import os
import re

import ace

KINDS = {"DEAD_END": 24.0, "RECENT_FIX": 48.0, "HOT_FILE": 12.0}  # ACTIVE_LEASE waits for TRACE-06
MIN_STRENGTH = 0.15
MAX_STRENGTH = 3.0
MAX_SUMMARY_CHARS = 200
PURGE_AFTER_SECS = 7 * 24 * 3600

SCHEMA = """
CREATE TABLE IF NOT EXISTS traces(id INTEGER PRIMARY KEY AUTOINCREMENT, project TEXT NOT NULL,
    kind TEXT NOT NULL, target TEXT NOT NULL, summary TEXT NOT NULL, agent TEXT NOT NULL,
    session_id TEXT NOT NULL, w0 REAL NOT NULL, created_at INTEGER NOT NULL, half_life_h REAL NOT NULL);
CREATE INDEX IF NOT EXISTS traces_project ON traces(project, kind, target);
"""


def strength(w0, created_at, now, half_life_h):
    """w(t) = w0 * 2^(-dt / h), dt in hours; clock skew never inflates."""
    dt_h = max(0, now - created_at) / 3600.0
    return w0 * 2 ** (-dt_h / half_life_h)


def ensure_table(db):
    db.executescript(SCHEMA)


def add(db, project, kind, target, summary, agent, session_id, now):
    """Insert a trace, or reinforce one from another session (+1.0, capped, clock restarts)."""
    if kind not in KINDS:
        raise ValueError(f"unknown trace kind {kind!r}; use one of {sorted(KINDS)}")
    summary = " ".join(str(summary).split())
    if not str(project).strip() or not str(target).strip() or not summary:
        raise ValueError("project, target and summary are required")
    if len(summary) > MAX_SUMMARY_CHARS:
        raise ValueError(f"summary over {MAX_SUMMARY_CHARS} characters")
    if ace.SECRET_RE.search(summary) or ace.SECRET_RE.search(str(target)):
        raise ValueError("trace looks like it contains a secret; not stored")
    ensure_table(db)
    row = db.execute("SELECT id, session_id, w0, created_at, half_life_h FROM traces "
                     "WHERE project=? AND kind=? AND target=? ORDER BY created_at DESC LIMIT 1",
                     (project, kind, target)).fetchone()
    with db:
        if row:
            tid, sess, w0, created, hl = row
            if sess == session_id:
                return tid
            w = min(strength(w0, created, now, hl) + 1.0, MAX_STRENGTH)
            db.execute("UPDATE traces SET w0=?, created_at=?, summary=?, agent=?, session_id=? WHERE id=?",
                       (w, now, summary, agent, session_id, tid))
            return tid
        cur = db.execute("INSERT INTO traces(project, kind, target, summary, agent, session_id, w0, created_at, "
                         "half_life_h) VALUES (?,?,?,?,?,?,1.0,?,?)",
                         (project, kind, target, summary, agent, session_id, now, KINDS[kind]))
        return cur.lastrowid


def active(db, project, now):
    """[(trace dict, strength)] above MIN_STRENGTH, strongest first, after purging rows older than 7 days."""
    ensure_table(db)
    cutoff = now - PURGE_AFTER_SECS
    if db.execute("SELECT 1 FROM traces WHERE created_at < ? LIMIT 1", (cutoff,)).fetchone():
        with db:  # write + fsync only when something is stale: reads stay read-only (Phase 7 gate < 10 ms)
            db.execute("DELETE FROM traces WHERE created_at < ?", (cutoff,))
    cols = ("id", "project", "kind", "target", "summary", "agent", "session_id", "w0", "created_at", "half_life_h")
    rows = db.execute(f"SELECT {', '.join(cols)} FROM traces WHERE project=?", (project,)).fetchall()
    out = []
    for r in rows:
        t = dict(zip(cols, r))
        w = strength(t["w0"], t["created_at"], now, t["half_life_h"])
        if w >= MIN_STRENGTH:
            out.append((t, w))
    out.sort(key=lambda x: -x[1])
    return out


CONTEXT_MAX_TRACES = 5
HOT_FILE_MIN_FAILS = 3


def _words(s):
    return {w for w in re.split(r"[\W_]+", s.lower()) if len(w) >= 3}


def context_section(items, task):
    """TRACE-04 hive_context section; with a task, keep traces sharing a word with it."""
    want = _words(task)
    lines = [f"- {t['kind']} {t['target']} ({t['agent']}, w={w:.2f}): {t['summary']}"
             for t, w in items if not want or _words(f"{t['target']} {t['summary']}") & want][:CONTEXT_MAX_TRACES]
    return "## live traces (decaying signals from other agents)\n" + "\n".join(lines) if lines else ""


def _tool_items(fh):
    """Yield tool_use / tool_result dicts from transcript lines; skips malformed lines."""
    for line in fh:
        if '"tool_' not in line:
            continue
        try:
            content = json.loads(line).get("message", {}).get("content", [])
        except (json.JSONDecodeError, AttributeError):
            continue
        yield from (i for i in content if isinstance(i, dict)) if isinstance(content, list) else ()


def failed_files(transcript_path):
    """{file: errored tool calls} from a Claude transcript (tool_use input path + tool_result is_error)."""
    paths, fails = {}, {}
    try:
        with open(transcript_path, encoding="utf-8", errors="replace") as fh:
            for item in _tool_items(fh):
                inp = item.get("input")
                if item.get("type") == "tool_use" and isinstance(inp, dict):
                    path = inp.get("file_path") or inp.get("path")
                    if isinstance(path, str) and path:
                        paths[item.get("id")] = os.path.normcase(os.path.normpath(path))
                elif (item.get("type") == "tool_result" and item.get("is_error") and item.get("tool_use_id") in paths
                      and "PreToolUse:" not in str(item.get("content"))[:300]):  # hook gate denials are not failures
                    path = paths[item["tool_use_id"]]
                    fails[path] = fails.get(path, 0) + 1
    except OSError:
        return {}
    return fails


def project_of(path):
    """(project slug, repo-relative posix path) for a file under ace.PROJECTS, else None. Case-insensitive on
    Windows, so lowercased transcript paths still match."""
    p = os.path.normcase(os.path.normpath(os.path.abspath(path)))
    root = os.path.normcase(os.path.normpath(os.path.abspath(str(ace.PROJECTS))))
    if not p.startswith(root + os.sep):
        return None
    parts = p[len(root) + 1:].split(os.sep)
    if len(parts) < 2:
        return None
    return ace.slugify(parts[0]), "/".join(parts[1:])


def emit_hot_files(db, transcript_path, agent, session_id, now):
    """TRACE-05: HOT_FILE for each file with >= HOT_FILE_MIN_FAILS errored calls, filed under the file's own
    project (not the session's cwd scope) with a repo-relative target; files outside PROJECTS are skipped.
    Returns the emitted (project, target) pairs."""
    emitted = []
    for path, n in failed_files(transcript_path).items():
        where = project_of(path) if n >= HOT_FILE_MIN_FAILS else None
        if where:
            add(db, where[0], "HOT_FILE", where[1], f"{n} failed tool calls in one session", agent, session_id, now)
            emitted.append(where)
    return emitted
