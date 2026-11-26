"""Quorum states for playbook bullets (v2.7.0, QUOR-01 + PROV-04); Python mirror of src/quorum.rs.

State and origin live in a sidecar `playbook.meta.jsonl` next to each `playbook.md`, one compact JSON line
per bullet id ever issued: {"id","state","origin_session","origin_agent"}. The playbook line format is
unchanged, so older readers (which drop unknown lines) keep working. A bullet with no entry is `canonical`
(zero-rewrite migration of existing playbooks). Pruned bullets stay as `retired`, so their ids are never
reissued and a new scout can never inherit a stale state.
"""
import json
import os
import re
from pathlib import Path

STATES = ("scout", "quorum", "canonical", "retired")
DEFAULT_STATE = "canonical"
FIELDS = ("id", "state", "origin_session", "origin_agent")
EVIDENCE_FIELDS = ("session", "agent", "kind")
# QUOR-02: per-session weight; a session counts once, at its strongest kind (same-session repeat adds 0)
WEIGHTS = {"test_pass": 1.0, "confirm": 1.0, "self_tag": 0.35}
# QUOR-03: canonical at score >= PROMOTE_SCORE from >= PROMOTE_SESSIONS sessions; >= 2 sessions below it = quorum
PROMOTE_SCORE = 1.5
PROMOTE_SESSIONS = 2
INJECT_QUORUM_ENV = "HIVE_INJECT_QUORUM"
ANONYMOUS = "anonymous"
TRUSTED_ONLY_FIELDS = ("session", "evidence")  # set only by `ledger score`, stripped from CLI deltas


def _clean(rec):
    out = {k: str(rec.get(k, "")) for k in FIELDS}
    try:  # GOV-01: when the bullet was proposed (0 = unknown / legacy), so --agent --since rules can match it
        out["origin_ts"] = int(rec.get("origin_ts") or 0)
    except (TypeError, ValueError):
        out["origin_ts"] = 0
    ev = rec.get("evidence") if isinstance(rec.get("evidence"), list) else []
    out["evidence"] = [{k: str(e.get(k, "")) for k in EVIDENCE_FIELDS} for e in ev
                       if isinstance(e, dict) and e.get("kind") in WEIGHTS and e.get("session")]
    return out


def meta_path(playbook_path):
    p = Path(playbook_path)
    return p.with_name(p.stem + ".meta.jsonl")


def dumps(rec):
    """Compact, fixed key order: byte-identical to the Rust serializer."""
    return json.dumps(_clean(rec), separators=(",", ":"), ensure_ascii=False)


def load(path):
    """{id: record}; unreadable lines and unknown states are skipped (fail-open)."""
    out = {}
    try:
        text = Path(path).read_text(encoding="utf-8")
    except OSError:
        return out
    for line in text.splitlines():
        try:
            rec = json.loads(line)
        except json.JSONDecodeError:
            continue
        if isinstance(rec, dict) and isinstance(rec.get("id"), str) and rec.get("state") in STATES:
            out[rec["id"]] = _clean(rec)
    return out


def save(path, meta):
    path = Path(path)
    tmp = path.with_suffix(".jsonl.tmp")
    tmp.write_text("".join(dumps(meta[i]) + "\n" for i in sorted(meta)), encoding="utf-8", newline="\n")
    os.replace(tmp, path)


def state_of(meta, bullet_id):
    return meta.get(bullet_id, {}).get("state", DEFAULT_STATE)


def reconcile(meta, added, pruned, session, agent, now=0):
    """New meta: added ids become scouts with origin; pruned ids become retired; others untouched."""
    out = {i: dict(r) for i, r in meta.items()}
    for i in added:
        out[i] = {"id": i, "state": "scout", "origin_session": session, "origin_agent": agent,
                  "origin_ts": int(now), "evidence": []}
    for i in pruned:
        out[i] = dict(_entry(out, i), state="retired")
    return out


def _entry(meta, bullet_id):
    return meta.get(bullet_id) or {"id": bullet_id, "state": DEFAULT_STATE, "origin_session": "",
                                   "origin_agent": "", "evidence": []}


def add_evidence(meta, tags, gates_by_id=None):
    """QUOR-02. tags: [(bullet_id, session, agent, hint)] for helpful TAGs. hint 'test_pass' (from ledger score)
    keeps its kind; otherwise the origin session tagging its own bullet is 'self_tag', any other is 'confirm'.
    A (session, kind) pair is recorded once."""
    out = {i: _clean(r) for i, r in meta.items()}
    for bullet_id, session, agent, hint in tags:
        rec = _clean(_entry(out, bullet_id))
        if not session or str(session).startswith("proc-"):
            # anonymous per-process id: it changes every CLI call, so it proves no independent session.
            # All of it shares one bucket and earns self-tag weight at most.
            session, kind = ANONYMOUS, "self_tag"
        else:
            kind = "test_pass" if hint == "test_pass" else "self_tag" if session == rec["origin_session"] else "confirm"
        if not any(e["session"] == session and e["kind"] == kind for e in rec["evidence"]):
            rec["evidence"].append({"session": session, "agent": agent, "kind": kind})
        out[bullet_id] = promote(rec, (gates_by_id or {}).get(bullet_id, PROMOTE_SCORE))
    return out


def promote(rec, gate=PROMOTE_SCORE):
    """QUOR-03: scout -> quorum (>= 2 sessions) -> canonical (and score >= gate, 1.5 unless a QUOR-04 conflict
    raises it). Never demotes here; retired stays."""
    if rec["state"] not in ("scout", "quorum") or len(sessions(rec)) < PROMOTE_SESSIONS:
        return rec
    return dict(rec, state="canonical" if score(rec) >= gate else "quorum")


# ---------------------------------------------------------------- QUOR-04 conflicts
# Heuristic on purpose: a false positive only holds a promotion and adds a report line; playbook lines are
# never touched. Anchors are paths, `backticked` spans, --flags and snake_case identifiers.
ANCHOR_RE = re.compile(r"`([^`]+)`|((?:[\w.-]+[/\\])+[\w.-]+|[\w-]+\.(?:rs|py|md|exe|json|jsonl|toml|sqlite|ps1|ts|js))"
                       r"|(--[\w-]+)|(\b[a-z]+_[a-z_]+\b)")
NEGATIVE_RE = re.compile(r"\b(never|don'?t|do not|avoid|instead of|until|fails?|cannot|can'?t|blocks?|denied)\b", re.I)
MIN_SHARED_ANCHORS = 2  # one shared generic anchor (a tool name) is not a conflict: real-playbook calibration
CHALLENGE_MARGIN = 1.0


def anchors(text):
    return {next(g for g in m.groups() if g).lower().strip(".,;:") for m in ANCHOR_RE.finditer(text)}


def conflicts(bullets):
    """[(a_id, b_id, sorted shared anchors)] for opposite-polarity pairs in one playbook, ordered by id number
    (input order does not matter, so the Rust side, which iterates a HashMap, reports identically)."""
    ordered = sorted(bullets, key=lambda b: (int(b["id"].rsplit("-", 1)[-1]), b["id"]))
    rows = [(b["id"], anchors(b["content"]), bool(NEGATIVE_RE.search(b["content"]))) for b in ordered]
    out = []
    for i, (a, aa, an) in enumerate(rows):
        for b, ba, bn in rows[i + 1:]:
            shared = aa & ba
            if an != bn and len(shared) >= MIN_SHARED_ANCHORS:
                out.append((a, b, sorted(shared)))
    return out


def gates(meta, pairs):
    """{challenger_id: promotion threshold}: a non-canonical bullet contesting a canonical one needs
    max(1.5, incumbent + 1.0); a canonical incumbent counts at least 1.5 (legacy bullets carry no evidence)."""
    out = {}
    for a, b, _ in pairs:
        for inc, ch in ((a, b), (b, a)):
            if state_of(meta, inc) == "canonical" and state_of(meta, ch) in ("scout", "quorum"):
                need = max(PROMOTE_SCORE, max(score(_entry(meta, inc)), PROMOTE_SCORE) + CHALLENGE_MARGIN)
                out[ch] = max(out.get(ch, PROMOTE_SCORE), need)
    return out


def displace(before, after, pairs):
    """A challenger that just became canonical sends its canonical rival to quorum: hidden, kept, and able to
    win back under the same rule."""
    out = {i: dict(r) for i, r in after.items()}
    for a, b, _ in pairs:
        for inc, ch in ((a, b), (b, a)):
            won = state_of(before, ch) != "canonical" and state_of(out, ch) == "canonical"
            if won and state_of(out, inc) == "canonical":
                out[inc] = dict(_entry(out, inc), state="quorum")
    return out


def _fmt(meta, i):
    return f"`{i}` ({state_of(meta, i)}, {score(_entry(meta, i)):g})"


def write_report(path, scope, pairs, meta, today):
    """Keep one `## <scope>` section per playbook in _meta/_conflicts.md: ids, states, scores and anchors, no
    lesson text (the indexer reads it). Atomic, rewritten only when the body changes, removed when empty."""
    path = Path(path)
    sections = {}
    if path.is_file():
        current = None
        for line in path.read_text(encoding="utf-8").split("\n---\n", 1)[-1].splitlines():
            if line.startswith("## "):
                current = sections.setdefault(line[3:].strip(), [])
            elif current is not None and line.startswith("- "):
                current.append(line)
    old = {k: list(v) for k, v in sections.items()}
    lines = [f"- {_fmt(meta, a)} vs {_fmt(meta, b)}: {', '.join(s)}" for a, b, s in pairs]
    if lines:
        sections[scope] = lines
    else:
        sections.pop(scope, None)
    if sections == old:
        return
    if not sections:
        path.unlink(missing_ok=True)
        return
    body = ["# Playbook conflicts", "",
            "> Generated by `hive ace apply` (QUOR-04). Opposite-polarity bullets sharing anchors; both are kept.",
            "> A challenger needs incumbent score + 1.0 (incumbent counts at least 1.5) to displace it."]
    for name in sorted(sections):
        body += ["", f"## {name}"] + sections[name]
    head = ["---", "type: note", "project: all", "agent: claude", "status: active", f"updated: {today}",
            "source: hive ace apply (QUOR-04 conflict detector)", "---"]
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_suffix(".md.tmp")
    tmp.write_text("\n".join(head + body) + "\n", encoding="utf-8", newline="\n")
    os.replace(tmp, path)


def injectable(state):
    """Only canonical bullets reach agents; quorum too when HIVE_INJECT_QUORUM=1. Scouts never, so the
    confirmations that promote them stay independent."""
    return state == "canonical" or (state == "quorum" and os.environ.get(INJECT_QUORUM_ENV) in ("1", "true"))


def sessions(rec):
    return {e["session"] for e in _clean(rec)["evidence"]}


def score(rec):
    """Sum over sessions of each session's strongest evidence weight."""
    best = {}
    for e in _clean(rec)["evidence"]:
        best[e["session"]] = max(best.get(e["session"], 0.0), WEIGHTS[e["kind"]])
    return round(sum(best.values()), 6)
