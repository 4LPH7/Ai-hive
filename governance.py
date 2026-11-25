"""Quarantine governance (v2.7.0, GOV-01); Python mirror of src/governance.rs.

A quarantine is a reversible rule, not an edit: `_meta/quarantine.jsonl` in the vault holds one compact JSON
line per rule {"id","session","agent","since","reason","created"}. An item is quarantined while any rule
matches its origin (session, agent, time): traces (hive.sqlite), scout/quorum playbook bullets (sidecar origin),
inbox proposals and codices (frontmatter). Release deletes the rule; nothing else is ever touched.
"""
import datetime
import hashlib
import json
import os
import re
from pathlib import Path

import hive
import quorum

RULE_FIELDS = ("id", "session", "agent", "since", "reason", "created")
KINDS = ("traces", "bullets", "inbox", "codices")
SOURCE_SESSION_RE = re.compile(r"\bsession\s+(\S+)")


def rules_path(vault):
    return Path(vault) / "_meta" / "quarantine.jsonl"


def parse_since(value):
    """Epoch seconds from '', an int, YYYY-MM-DD (UTC midnight) or an ISO datetime."""
    if value in (None, ""):
        return 0
    s = str(value).strip()
    if s.isdigit():
        return int(s)
    try:
        dt = datetime.datetime.fromisoformat(s)
    except ValueError:
        raise ValueError(f"bad --since {value!r}: use YYYY-MM-DD, an ISO datetime or epoch seconds") from None
    if dt.tzinfo is None:
        dt = dt.replace(tzinfo=datetime.timezone.utc)
    return int(dt.timestamp())


def _date_ts(updated):
    try:
        return parse_since(str(updated)[:10])
    except ValueError:
        return 0


def load_rules(vault):
    out = []
    try:
        lines = rules_path(vault).read_text(encoding="utf-8").splitlines()
    except OSError:
        return out
    for line in lines:
        try:
            r = json.loads(line)
        except json.JSONDecodeError:
            continue
        if isinstance(r, dict) and r.get("id") and (r.get("session") or r.get("agent")):
            out.append({k: r.get(k, "" if k not in ("since", "created") else 0) for k in RULE_FIELDS})
    return out


def _save(vault, rules):
    path = rules_path(vault)
    if not rules:
        path.unlink(missing_ok=True)
        return
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_suffix(".jsonl.tmp")
    tmp.write_text("".join(json.dumps({k: r[k] for k in RULE_FIELDS}, separators=(",", ":"), ensure_ascii=False)
                           + "\n" for r in rules), encoding="utf-8", newline="\n")
    os.replace(tmp, path)


def rule_matches(rule, session, agent, ts):
    if rule["session"] and rule["session"] != session:
        return False
    if rule["agent"] and rule["agent"] != agent:
        return False
    return int(ts or 0) >= int(rule["since"] or 0)


def is_quarantined(rules, session, agent, ts):
    return any(rule_matches(r, session, agent, ts) for r in rules)


def _origins(vault, db):
    """Yield (kind, session, agent, ts) for every quarantinable item."""
    vault = Path(vault)
    try:
        for agent, session, ts in db.execute("SELECT agent, session_id, created_at FROM traces"):
            yield "traces", session, agent, ts
    except Exception:  # no trace table yet
        pass
    playbooks = [vault / "_meta" / "playbook.md", *sorted((vault / "projects").glob("*/playbook.md"))]
    for pb in playbooks:
        for rec in quorum.load(quorum.meta_path(pb)).values():
            if rec["state"] in ("scout", "quorum"):
                yield "bullets", rec["origin_session"], rec["origin_agent"], rec["origin_ts"]
    for kind, note in _notes(vault):
        yield (kind, *note_origin(note.read_text(encoding="utf-8", errors="replace")))


NOTE_GLOBS = (("inbox", "inbox/*.md"), ("codices", "codices/*/*.md"))


def _notes(vault):
    for kind, pattern in NOTE_GLOBS:
        for note in sorted(Path(vault).glob(pattern)):
            yield kind, note


def note_origin(text):
    """(session, agent, ts) of an inbox proposal or codex from its frontmatter."""
    meta, _, _ = hive.parse_note(text)
    m = SOURCE_SESSION_RE.search(str(meta.get("source", "")))
    return (m.group(1) if m else ""), meta.get("agent", ""), _date_ts(meta.get("updated", ""))


# ---------------------------------------------------------------- GOV-02 read-path filters
def quarantined_paths(vault, rules=None):
    """Vault-relative posix paths of inbox proposals and codices that an active rule matches."""
    rules = load_rules(vault) if rules is None else rules
    if not rules:
        return set()
    return {note.relative_to(Path(vault)).as_posix() for _, note in _notes(vault)
            if is_quarantined(rules, *note_origin(note.read_text(encoding="utf-8", errors="replace")))}


def note_hidden(rules, rel_path, source, agent, updated):
    """An indexed inbox proposal or codex whose origin an active rule matches (from index columns)."""
    if not rel_path.startswith(("inbox/", "codices/")):
        return False
    m = SOURCE_SESSION_RE.search(str(source or ""))
    return is_quarantined(rules, m.group(1) if m else "", agent or "", _date_ts(updated or ""))


def visible_traces(rules, items):
    """Drop quarantined traces from traces.active() output."""
    return [(t, w) for t, w in items if not is_quarantined(rules, t["session_id"], t["agent"], t["created_at"])]


def bullet_hidden(rules, rec):
    """A scout/quorum bullet whose origin an active rule matches."""
    return bool(rules) and rec.get("state") in ("scout", "quorum") and is_quarantined(
        rules, rec.get("origin_session", ""), rec.get("origin_agent", ""), rec.get("origin_ts", 0))


def scan(vault, db, rule):
    counts = dict.fromkeys(KINDS, 0)
    for kind, session, agent, ts in _origins(vault, db):
        if rule_matches(rule, session, agent, ts):
            counts[kind] += 1
    return counts


def quarantine(vault, db, session="", agent="", since="", reason="", now=0):
    """Add (or reuse) a rule and return (rule, matched counts per kind). Needs --session or --agent."""
    session, agent = str(session or "").strip(), str(agent or "").strip()
    if not session and not agent:
        raise ValueError("quarantine needs --session or --agent")
    since_ts = parse_since(since)
    rid = "q-" + hashlib.sha256(f"{session}|{agent}|{since_ts}".encode("utf-8")).hexdigest()[:8]
    rules = load_rules(vault)
    rule = next((r for r in rules if r["id"] == rid), None)
    if rule is None:
        rule = {"id": rid, "session": session, "agent": agent, "since": since_ts,
                "reason": " ".join(str(reason).split())[:200], "created": int(now)}
        _save(vault, rules + [rule])
    return rule, scan(vault, db, rule)


def release(vault, rule_id):
    """Remove the rule; returns how many were removed. Items come back on every read path."""
    rules = load_rules(vault)
    keep = [r for r in rules if r["id"] != rule_id]
    _save(vault, keep)
    return len(rules) - len(keep)
