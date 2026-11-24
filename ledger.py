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
