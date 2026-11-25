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


def meta_path(playbook_path):
    p = Path(playbook_path)
    return p.with_name(p.stem + ".meta.jsonl")


def dumps(rec):
    """Compact, fixed key order: byte-identical to the Rust serializer."""
    return json.dumps({k: str(rec.get(k, "")) for k in FIELDS}, separators=(",", ":"), ensure_ascii=False)


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
            out[rec["id"]] = {k: str(rec.get(k, "")) for k in FIELDS}
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
        out[i] = {"id": i, "state": "scout", "origin_session": session, "origin_agent": agent}
    for i in pruned:
        rec = out.get(i) or {"id": i, "state": DEFAULT_STATE, "origin_session": "", "origin_agent": ""}
        out[i] = dict(rec, state="retired")
    return out
