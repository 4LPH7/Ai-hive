"""
Antigravity session lifecycle adapter for ai-hive.
Injects active playbook rules at session start and syncs session reflection.
Strict fail-open guarantee: never crashes the host Antigravity agent.
"""

import json
import os
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(REPO_ROOT))


def session_start(scope=None):
    """Fetch active playbook rules and return markdown injection context."""
    try:
        import hive
        import ledger

        cfg = hive.load_config()
        cwd = Path(os.getcwd())
        project = scope or hive.slugify(cwd.name)

        bullets = []
        bullets_text = ""
        try:
            import ace
            rows = ace.select(["global"] + ([project] if project != "global" else []), limit=8)
            bullets = [b["id"] for _, _, b in rows]
            bullets_text = ace.format_bullets(rows)
        except Exception:
            bullets_text = ""

        ledger.record(
            cfg["ledger"],
            event="inject",
            project=project,
            query="antigravity_session_start",
            injected_bullets=bullets,
            agent="antigravity",
        )

        if bullets_text:
            output = (
                f"> [!NOTE] Hive ACE Playbook (`{project}`)\n"
                "> Apply what fits; note IDs relied upon for end-of-session reflection.\n\n"
                f"{bullets_text}"
            )
            return output
        return ""
    except Exception as e:
        print(f"antigravity_sync start: warning (fail-open): {e}", file=sys.stderr)
        return ""


def session_stop(scope=None, summary=""):
    """Record reflection event and return candidate delta ops format."""
    try:
        import hive
        import ledger

        cfg = hive.load_config()
        cwd = Path(os.getcwd())
        project = scope or hive.slugify(cwd.name)

        ledger.record(
            cfg["ledger"],
            event="reflect",
            project=project,
            query=summary or "antigravity_session_stop",
            agent="antigravity",
        )
        return {
            "status": "ok",
            "project": project,
            "message": f"Recorded reflection event for {project}",
        }
    except Exception as e:
        print(f"antigravity_sync stop: warning (fail-open): {e}", file=sys.stderr)
        return {"status": "error", "error": str(e)}


if __name__ == "__main__":
    cmd = sys.argv[1] if len(sys.argv) > 1 else "start"
    scope_arg = sys.argv[2] if len(sys.argv) > 2 else None
    if cmd == "start":
        ctx = session_start(scope_arg)
        if ctx:
            print(ctx)
    elif cmd == "stop":
        res = session_stop(scope_arg)
        print(json.dumps(res))
    else:
        print(f"Usage: {sys.argv[0]} [start|stop] [scope]", file=sys.stderr)
