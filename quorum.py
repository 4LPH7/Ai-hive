"""Quorum states for playbook bullets (v2.7.0, QUOR-01 + PROV-04); Python mirror of src/quorum.rs.

State and origin live in a sidecar `playbook.meta.jsonl` next to each `playbook.md`, one compact JSON line
per bullet id ever issued: {"id","state","origin_session","origin_agent"}. The playbook line format is
unchanged, so older readers (which drop unknown lines) keep working. A bullet with no entry is `canonical`
(zero-rewrite migration of existing playbooks). Pruned bullets stay as `retired`, so their ids are never
reissued and a new scout can never inherit a stale state.
"""
import json
import os
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


def _clean(rec):
    out = {k: str(rec.get(k, "")) for k in FIELDS}
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


def reconcile(meta, added, pruned, session, agent):
    """New meta: added ids become scouts with origin; pruned ids become retired; others untouched."""
    out = {i: dict(r) for i, r in meta.items()}
    for i in added:
        out[i] = {"id": i, "state": "scout", "origin_session": session, "origin_agent": agent, "evidence": []}
    for i in pruned:
        out[i] = dict(_entry(out, i), state="retired")
    return out


def _entry(meta, bullet_id):
    return meta.get(bullet_id) or {"id": bullet_id, "state": DEFAULT_STATE, "origin_session": "",
                                   "origin_agent": "", "evidence": []}


def add_evidence(meta, tags):
    """QUOR-02. tags: [(bullet_id, session, agent, hint)] for helpful TAGs. hint 'test_pass' (from ledger score)
    keeps its kind; otherwise the origin session tagging its own bullet is 'self_tag', any other is 'confirm'.
    A (session, kind) pair is recorded once."""
    out = {i: _clean(r) for i, r in meta.items()}
    for bullet_id, session, agent, hint in tags:
        if not session:
            continue
        rec = _clean(_entry(out, bullet_id))
        kind = "test_pass" if hint == "test_pass" else "self_tag" if session == rec["origin_session"] else "confirm"
        if not any(e["session"] == session and e["kind"] == kind for e in rec["evidence"]):
            rec["evidence"].append({"session": session, "agent": agent, "kind": kind})
        out[bullet_id] = promote(rec)
    return out


def promote(rec):
    """QUOR-03: scout -> quorum (>= 2 sessions) -> canonical (and score >= 1.5). Never demotes; retired stays."""
    if rec["state"] not in ("scout", "quorum") or len(sessions(rec)) < PROMOTE_SESSIONS:
        return rec
    return dict(rec, state="canonical" if score(rec) >= PROMOTE_SCORE else "quorum")


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
