"""Hive Curate: OmniRoute-backed intelligence engine proposing ACE playbook merges,
contradiction flags, ADR drift checks, and query aliases exclusively into vault inbox/.
"""
import hashlib
import json
import os
import sys
from datetime import datetime, timezone
from pathlib import Path

import ace
import hive
import llm
import miner


def curate_schema():
    return {
        "type": "object",
        "properties": {
            "paraphrase_merges": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "target_id": {"type": "string"},
                        "merged_content": {"type": "string"},
                        "rationale": {"type": "string"},
                    },
                    "required": ["target_id", "merged_content", "rationale"],
                },
            },
            "contradictions": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "bullet_a": {"type": "string"},
                        "bullet_b": {"type": "string"},
                        "issue": {"type": "string"},
                    },
                    "required": ["bullet_a", "bullet_b", "issue"],
                },
            },
            "adr_drift_warnings": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "adr_path": {"type": "string"},
                        "drift_reason": {"type": "string"},
                    },
                    "required": ["adr_path", "drift_reason"],
                },
            },
            "query_alias_proposals": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "cluster_topic": {"type": "string"},
                        "aliases": {"type": "array", "items": {"type": "string"}},
                        "suggested_note": {"type": "string"},
                    },
                    "required": ["cluster_topic", "aliases", "suggested_note"],
                },
            },
        },
        "required": ["paraphrase_merges", "contradictions", "adr_drift_warnings", "query_alias_proposals"],
    }


def run_curate(ledger_path=None, vault_path=None, project=None, dry_run=False, config_path=None):
    llm_cfg = llm.load_llm_config(cli_config=config_path)
    if not llm_cfg.get("enabled", True):
        return {
            "model": "disabled",
            "merges_count": 0,
            "contradictions_count": 0,
            "adr_drifts_count": 0,
            "alias_proposals_count": 0,
            "proposal_file": None,
            "dry_run": dry_run,
            "skipped": True,
            "reason": "LLM disabled in configuration",
        }

    cfg = hive.load_config(cli_config=config_path)
    ledger_p = Path(ledger_path or cfg.get("ledger", Path(os.environ.get("HIVE_LEDGER", "ledger.jsonl"))))
    vault_p = Path(vault_path or cfg.get("vault", Path(os.environ.get("HIVE_VAULT", Path.home() / "vault"))))

    scope = project or "global"
    model = llm.resolve_combo(llm_cfg, "deep", scope)

    # 1. Gather context
    old_vault = ace.VAULT
    ace.VAULT = vault_p
    try:
        playbook_sections = ace.load(scope)
    finally:
        ace.VAULT = old_vault

    known_bullet_ids = set()
    playbook_summary = []
    for sec_name, bullets in playbook_sections.items():
        for b in bullets:
            known_bullet_ids.add(b["id"])
            playbook_summary.append(f"- [{b['id']}] (seen={b.get('seen', 1)}, +{b['helpful']}, -{b['harmful']}) {b['content']}")

    target_project = project if scope != "global" else None
    gaps = miner.scan_ledger_gaps(ledger_p, project=target_project)
    top_misses = [q for q, _ in gaps[:15]]

    system_prompt = (
        "You are the hive intelligence curator. Analyze the given ACE playbook and recent query telemetry. "
        "Propose high-signal playbook consolidations (paraphrase merges), flag contradictory bullets, "
        "check for stale architectural drift, and propose query aliases. Conform strictly to the JSON schema."
    )
    user_prompt = (
        f"Scope: {scope}\n\nPlaybook:\n"
        + ("\n".join(playbook_summary) if playbook_summary else "(empty playbook)")
        + "\n\nRecent zero-hit queries:\n"
        + ("\n".join(top_misses) if top_misses else "(none)")
    )

    schema = curate_schema()
    try:
        raw_response = llm.chat_completion(llm_cfg, model, system_prompt, user_prompt, schema=schema)
    except Exception as exc:
        # Fail-open guarantee: network/model server offline skips gracefully
        return {
            "model": model,
            "merges_count": 0,
            "contradictions_count": 0,
            "adr_drifts_count": 0,
            "alias_proposals_count": 0,
            "proposal_file": None,
            "dry_run": dry_run,
            "skipped": True,
            "reason": f"OmniRoute call failed (fail-open): {exc}",
        }

    try:
        payload = json.loads(raw_response)
        if not isinstance(payload, dict):
            payload = {}
    except json.JSONDecodeError:
        payload = {}

    # 2. Deterministic Validation Gate
    validated_merges = []
    merges_raw = payload.get("paraphrase_merges")
    if isinstance(merges_raw, list):
        for m in merges_raw:
            if not isinstance(m, dict):
                continue
            tid = m.get("target_id")
            if tid not in known_bullet_ids:
                continue
            content = str(m.get("merged_content", "")).strip()
            if not content or len(content) > ace.MAX_BULLET_CHARS or ace.SECRET_RE.search(content):
                continue
            validated_merges.append(m)
            if len(validated_merges) >= 5:
                break

    validated_contradictions = []
    contra_raw = payload.get("contradictions")
    if isinstance(contra_raw, list):
        for c in contra_raw:
            if not isinstance(c, dict):
                continue
            ba = c.get("bullet_a")
            bb = c.get("bullet_b")
            if ba not in known_bullet_ids or bb not in known_bullet_ids:
                continue
            validated_contradictions.append(c)

    drifts_raw = payload.get("adr_drift_warnings")
    validated_drifts = [d for d in drifts_raw if isinstance(d, dict)] if isinstance(drifts_raw, list) else []
    aliases_raw = payload.get("query_alias_proposals")
    validated_aliases = [a for a in aliases_raw if isinstance(a, dict)] if isinstance(aliases_raw, list) else []

    total_proposals = len(validated_merges) + len(validated_contradictions) + len(validated_drifts) + len(validated_aliases)
    if total_proposals == 0:
        return {
            "model": model,
            "merges_count": 0,
            "contradictions_count": 0,
            "adr_drifts_count": 0,
            "alias_proposals_count": 0,
            "proposal_file": None,
            "dry_run": dry_run,
            "skipped": False,
            "reason": "No actionable proposals passed deterministic validation",
        }

    # 3. Render Proposal note exclusively in inbox/
    today = datetime.now(timezone.utc).strftime("%Y-%m-%d")
    h = hashlib.sha256(raw_response.encode("utf-8")).hexdigest()[:8]

    lines = [
        "---",
        "type: proposal",
        "status: inbox",
        "source: hive-curate",
        f"model: {model}",
        f"project: {scope}",
        f"updated: {today}",
        "---",
        f"# Curation Proposal: {scope} ({today})",
        "",
        f"> Generated by OmniRoute-backed intelligence engine (`{model}`).",
        "> Validated by deterministic ACE gate. Review before applying.",
        "",
    ]

    if validated_merges:
        lines.append("## Paraphrase Merges")
        for m in validated_merges:
            lines.append(f"- [ ] Target `[{m['target_id']}]`: {m['merged_content']} (Rationale: {m.get('rationale', '')})")
        lines.append("")

    if validated_contradictions:
        lines.append("## Contradiction Flags")
        for c in validated_contradictions:
            lines.append(f"- [ ] Potential conflict between `[{c['bullet_a']}]` and `[{c['bullet_b']}]`: {c.get('issue', '')}")
        lines.append("")

    if validated_drifts:
        lines.append("## ADR Drift Warnings")
        for d in validated_drifts:
            lines.append(f"- [ ] `{d.get('adr_path', '')}`: {d.get('drift_reason', '')}")
        lines.append("")

    if validated_aliases:
        lines.append("## Query Aliases")
        for a in validated_aliases:
            lines.append(f"- [ ] Topic `{a.get('cluster_topic', '')}` -> note `{a.get('suggested_note', '')}` (aliases: {a.get('aliases', [])})")
        lines.append("")

    markdown = "\n".join(lines)
    inbox_dir = vault_p / "inbox"
    proposal_file = inbox_dir / f"curate-{today}-{h}.md"

    if not dry_run:
        inbox_dir.mkdir(parents=True, exist_ok=True)
        proposal_file.write_text(markdown, encoding="utf-8")

    return {
        "model": model,
        "merges_count": len(validated_merges),
        "contradictions_count": len(validated_contradictions),
        "adr_drifts_count": len(validated_drifts),
        "alias_proposals_count": len(validated_aliases),
        "proposal_file": str(proposal_file),
        "dry_run": dry_run,
        "skipped": False,
        "reason": None,
    }


# ---------------------------------------------------------------- QUOR-05 conflict judge
MAX_VERDICTS = 10
MAX_REASON_CHARS = 500
MAX_DIFF_CHARS = 4000
CONFLICT_SCHEMA = {
    "type": "object",
    "properties": {"verdicts": {"type": "array", "items": {
        "type": "object",
        "properties": {"bullet_a": {"type": "string"}, "bullet_b": {"type": "string"},
                       "winner": {"type": "string"}, "reason": {"type": "string"},
                       "citations": {"type": "array", "items": {"type": "string"}}},
        "required": ["bullet_a", "bullet_b", "winner", "reason", "citations"]}}},
    "required": ["verdicts"],
}
CONFLICT_SYSTEM = (
    "You judge contradicting lessons in an AI agents' playbook. For each pair, say which lesson is right now "
    "(its id), or 'both' when they apply in different situations. Cite only anchors or file paths that appear "
    "in the input. Conform strictly to the JSON schema.")


def git_diff(projects_root, scope, paths):
    """Recent history of the anchor paths in the scope's repo (fail-open: '' for global or no repo)."""
    import subprocess
    repo = next((p for p in Path(projects_root).iterdir() if p.is_dir() and ace.slugify(p.name) == scope), None) \
        if scope != "global" and Path(projects_root).is_dir() else None
    files = [p for p in paths if "/" in p or "." in p]
    if not repo or not files:
        return ""
    try:
        out = subprocess.run(["git", "-C", str(repo), "log", "-p", "-n", "5", "--format=%h %s", "--", *files],
                             capture_output=True, text=True, timeout=10, encoding="utf-8", errors="replace")
        return out.stdout[:MAX_DIFF_CHARS] if out.returncode == 0 else ""
    except (OSError, subprocess.SubprocessError):
        return ""


def _bid(value):
    """Bullet id as the model may echo it: '[pit-00012]' or '`pit-00012`' -> 'pit-00012'."""
    return str(value or "").strip().strip("[]`'\" ").strip()


def _reject_reason(v, known, corpus):
    """'' when the verdict passes the gate, else a short reason code."""
    if not isinstance(v, dict):
        return "not-an-object"
    a, b, winner = _bid(v.get("bullet_a")), _bid(v.get("bullet_b")), _bid(v.get("winner"))
    reason = " ".join(str(v.get("reason", "")).split())
    cites = [c for c in v.get("citations", []) if isinstance(c, str)] if isinstance(v.get("citations"), list) else []
    missing = [c for c in cites if not c or c.lower() not in corpus]
    checks = ((frozenset((a, b)) not in known, f"unknown-pair {a} {b}"),
              (winner not in (a, b, "both"), f"bad-winner {winner}"),
              (not reason, "empty-reason"), (len(reason) > MAX_REASON_CHARS, "long-reason"),
              (bool(ace.SECRET_RE.search(reason)), "secret-in-reason"), (not cites, "no-citation"),
              (bool(missing), f"uncited {missing[:2]}"))
    return next((code for failed, code in checks if failed), "")


def validate_verdicts(payload, pairs, corpus, rejected=None):
    """Deterministic gate: known pair, winner in the pair or 'both', a short secret-free reason, and >= 1
    citation, every one of which appears in the anchors or diff the model was shown. Reasons for dropped
    verdicts go to `rejected` when a list is passed."""
    known = {frozenset((a, b)) for a, b, _ in pairs}
    out = []
    for v in payload.get("verdicts", []) if isinstance(payload, dict) else []:
        why = _reject_reason(v, known, corpus)
        if why:
            if rejected is not None:
                rejected.append(why)
            continue
        out.append({"bullet_a": _bid(v["bullet_a"]), "bullet_b": _bid(v["bullet_b"]), "winner": _bid(v["winner"]),
                    "reason": " ".join(str(v["reason"]).split()), "citations": list(v["citations"])})
        if len(out) >= MAX_VERDICTS:
            break
    return out


def run_curate_conflicts(vault_path, project=None, dry_run=False, config_path=None,
                         complete=None, diff=None):
    """QUOR-05: send each detected conflict (both claims + recent git history) to OmniRoute and write a cited
    proposal to inbox/ only. Never changes playbook lines or quorum states; a human applies it."""
    import quorum
    llm_cfg = llm.load_llm_config(cli_config=config_path)
    scope = ace.slugify(project or "global")
    model = llm.resolve_combo(llm_cfg, "deep", scope)  # private projects stay on hive-private
    result = {"model": model, "conflicts": 0, "verdicts": 0, "proposal_file": None, "dry_run": dry_run,
              "skipped": True, "reason": None}
    if not llm_cfg.get("enabled", True):
        return dict(result, model="disabled", reason="LLM disabled in configuration")
    vault_p = Path(vault_path)
    old_vault = ace.VAULT
    ace.VAULT = vault_p
    try:
        sections = ace.load(scope)
        meta = quorum.load(quorum.meta_path(ace.scope_path(scope)))
    finally:
        ace.VAULT = old_vault
    by_id = {b["id"]: b for bullets in sections.values() for b in bullets}
    pairs = quorum.conflicts(list(by_id.values()))
    result["conflicts"] = len(pairs)
    if not pairs:
        return dict(result, reason="no conflicts in this playbook")
    anchors = sorted({s for _, _, shared in pairs for s in shared})
    history = (diff or git_diff)(ace.PROJECTS, scope, anchors)
    lines = [f"Scope: {scope}", ""]
    for a, b, shared in pairs:
        for i in (a, b):
            lines.append(f"[{i}] ({quorum.state_of(meta, i)}, score {quorum.score(quorum._entry(meta, i)):g}) "
                         f"{by_id[i]['content']}")
        lines += [f"shared anchors: {', '.join(shared)}", ""]
    lines += ["Recent git history of the anchors:", history or "(none)"]
    try:
        raw = (complete or llm.chat_completion)(llm_cfg, model, CONFLICT_SYSTEM, "\n".join(lines),
                                                schema=CONFLICT_SCHEMA)
        payload = json.loads(raw)
    except Exception as exc:  # fail-open: the judge is optional
        return dict(result, reason=f"OmniRoute call failed (fail-open): {exc}")
    corpus = (" ".join(anchors) + "\n" + history).lower()
    rejected = []
    verdicts = validate_verdicts(payload, pairs, corpus, rejected)
    result.update(skipped=False, verdicts=len(verdicts), rejected=rejected)
    if not verdicts:
        got = len(payload.get("verdicts", [])) if isinstance(payload, dict) and isinstance(payload.get("verdicts"), list) else 0
        return dict(result, reason=f"no verdict passed deterministic validation ({got} returned)")
    today = datetime.now(timezone.utc).strftime("%Y-%m-%d")
    body = [f"# Conflict verdicts: {scope} ({today})", "",
            f"> Proposed by `{model}` via `hive curate --conflicts` (QUOR-05). Nothing was applied: review, then",
            "> tag the winner helpful with `hive-ace` or leave it; playbook lines and quorum states are unchanged.",
            "", "## Verdicts"]
    body += [f"- [ ] `{v['bullet_a']}` vs `{v['bullet_b']}`: winner `{v['winner']}`. {v['reason']} "
             f"(cites: {', '.join(v['citations'])})" for v in verdicts]
    body_text = "\n".join(body) + "\n"
    head = ["---", "type: note", f"project: {'all' if scope == 'global' else scope}", "agent: claude",
            "status: active", f"updated: {today}", f"source: hive curate --conflicts ({model})", "---"]
    name = f"conflicts-{scope}-{today}-{hashlib.sha256(body_text.encode('utf-8')).hexdigest()[:8]}.md"
    proposal = vault_p / "inbox" / name
    if not dry_run:
        proposal.parent.mkdir(parents=True, exist_ok=True)
        proposal.write_text("\n".join(head) + "\n" + body_text, encoding="utf-8", newline="\n")
    return dict(result, proposal_file=str(proposal))
