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


def run_curate(ledger_path=None, vault_path=None, project=None, dry_run=False):
    llm_cfg = llm.load_llm_config()
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

    cfg = hive.load_config()
    ledger_p = Path(ledger_path or cfg.get("ledger", Path(os.environ.get("HIVE_LEDGER", "ledger.jsonl"))))
    vault_p = Path(vault_path or cfg.get("vault", Path(os.environ.get("HIVE_VAULT", r"D:\Valut\OBS"))))

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

    gaps = miner.scan_ledger_gaps(ledger_p)
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
    except json.JSONDecodeError:
        payload = {}

    # 2. Deterministic Validation Gate
    validated_merges = []
    for m in payload.get("paraphrase_merges", []):
        tid = m.get("target_id")
        if tid not in known_bullet_ids:
            continue
        content = m.get("merged_content", "").strip()
        if not content or len(content) > ace.MAX_BULLET_CHARS or ace.SECRET_RE.search(content):
            continue
        validated_merges.append(m)
        if len(validated_merges) >= 5:
            break

    validated_contradictions = []
    for c in payload.get("contradictions", []):
        ba = c.get("bullet_a")
        bb = c.get("bullet_b")
        if ba not in known_bullet_ids or bb not in known_bullet_ids:
            continue
        validated_contradictions.append(c)

    validated_drifts = payload.get("adr_drift_warnings", [])
    validated_aliases = payload.get("query_alias_proposals", [])

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
