import json
import os
import sys
from datetime import datetime, timezone
from pathlib import Path


def detect_agent():
    if os.environ.get("HIVE_AGENT"):
        return os.environ["HIVE_AGENT"]
    if os.environ.get("CLAUDE_PROJECT_DIR") or os.environ.get("CLAUDE_ENV_FILE"):
        return "claude"
    if os.environ.get("CODEX_HOME") or os.environ.get("CODEX_SESSION"):
        return "codex"
    if os.environ.get("ANTIGRAVITY_CONFIG_DIR") or os.environ.get("GEMINI_CONFIG_DIR"):
        return "antigravity"
    return "unknown"


def record(ledger_path, event, project="", query="", hits=None, injected_bullets=None, duration_ms=0.0, agent=None, session_id=None):
    """Append a structured JSONL event to the local ledger. Fail-open guarantee."""
    try:
        p = Path(ledger_path)
        p.parent.mkdir(parents=True, exist_ok=True)
        entry = {
            "ts": datetime.now(timezone.utc).isoformat(),
            "session_id": session_id or os.environ.get("HIVE_SESSION_ID") or "",
            "agent": agent or detect_agent(),
            "project": project,
            "event": event,
            "query": query,
            "hits": hits or [],
            "injected_bullets": injected_bullets or [],
            "duration_ms": round(float(duration_ms), 2),
        }
        line = json.dumps(entry, ensure_ascii=False) + "\n"
        with open(p, "a", encoding="utf-8") as f:
            f.write(line)
            f.flush()
        return True
    except Exception as e:
        # Fail-open guarantee: telemetry write failure never breaks core retrieval
        print(f"hive ledger: warning: failed to append to {ledger_path}: {e}", file=sys.stderr)
        return False


def read_ledger(ledger_path, tail=None, project=None):
    """Read recent ledger entries."""
    p = Path(ledger_path)
    if not p.is_file():
        return []
    entries = []
    try:
        with open(p, "r", encoding="utf-8") as f:
            for line in f:
                line = line.strip()
                if not line:
                    continue
                try:
                    obj = json.loads(line)
                    if project and obj.get("project") != project:
                        continue
                    entries.append(obj)
                except json.JSONDecodeError:
                    continue
    except OSError:
        return []
    if tail is not None and tail > 0:
        return entries[-tail:]
    return entries


def score_session(session_id=None, repo_dir=None, test_exit_code=0, ledger_path=None, vault_path=None, project=None):
    """Correlate a ledger session with test exit code, reinforcing or penalizing bullets & skills."""
    import ace, hive
    cfg = hive.load_config()
    ledger_p = Path(ledger_path or cfg.get("ledger", Path(os.environ.get("HIVE_LEDGER", "ledger.jsonl"))))

    entries = read_ledger(ledger_p)
    if not entries:
        return {"session_id": session_id or "", "scored": False, "reason": "empty ledger"}

    if not session_id:
        for e in reversed(entries):
            if e.get("session_id"):
                session_id = e["session_id"]
                break

    if not session_id:
        return {"session_id": "", "scored": False, "reason": "no session_id found in ledger"}

    session_entries = [e for e in entries if e.get("session_id") == session_id]
    if not session_entries:
        return {"session_id": session_id, "scored": False, "reason": f"session {session_id} not found"}

    injected_bullets = []
    triggered_skills = []
    session_project = project

    for e in session_entries:
        if not session_project and e.get("project") and e.get("project") not in ("all", "global") and e.get("event") != "skill":
            session_project = e["project"]
        for b in e.get("injected_bullets") or []:
            if b not in injected_bullets:
                injected_bullets.append(b)
        if e.get("event") == "skill" and e.get("project"):
            skill_name = e["project"]
            if skill_name not in triggered_skills:
                triggered_skills.append(skill_name)

    scope = session_project or "global"
    is_success = (test_exit_code == 0)
    reward = 1 if is_success else -1
    tag_field = "helpful" if is_success else "harmful"

    ops = []
    for b in injected_bullets:
        ops.append({"type": "TAG", "id": b, tag_field: 1})
    for s in triggered_skills:
        ops.append({"type": "TAG", "id": f"skill:{s}", tag_field: 1})

    report = {"tagged": 0, "pruned": []}
    if ops:
        old_vault = ace.VAULT
        old_ledger_env = os.environ.get("HIVE_LEDGER")
        os.environ["HIVE_LEDGER"] = str(ledger_p)
        if vault_path:
            ace.VAULT = Path(vault_path)
        try:
            report = ace.apply(scope, ops)
            if scope != "global":
                g_sections = ace.load("global")
                g_by_id = {b["id"]: b for bullets in g_sections.values() for b in bullets}
                g_ops = [op for op in ops if op.get("id") in g_by_id]
                if g_ops:
                    g_rep = ace.apply("global", g_ops)
                    report["tagged"] = report.get("tagged", 0) + g_rep.get("tagged", 0)
                    report.setdefault("pruned", []).extend(g_rep.get("pruned", []))
        finally:
            if vault_path:
                ace.VAULT = old_vault
            if old_ledger_env is not None:
                os.environ["HIVE_LEDGER"] = old_ledger_env
            else:
                os.environ.pop("HIVE_LEDGER", None)

    record(
        ledger_p,
        event="session_scored",
        project=scope,
        query=f"exit_code={test_exit_code};reward={reward}",
        hits=triggered_skills,
        injected_bullets=injected_bullets,
        duration_ms=0.0,
        session_id=session_id
    )

    return {
        "session_id": session_id,
        "scored": True,
        "reward": reward,
        "test_exit_code": test_exit_code,
        "bullets": injected_bullets,
        "skills": triggered_skills,
        "scope": scope,
        "report": report
    }

