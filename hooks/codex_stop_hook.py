"""
Codex stop hook adapter for ai-hive.
Triggers on Codex session exit or command completion.
Extracts changed files from git diff and logs reflection intent to ledger.
Strict fail-open guarantee: never crashes the host Codex process.
"""

import json
import os
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(REPO_ROOT))


def get_git_status():
    try:
        res = subprocess.run(
            ["git", "status", "--porcelain"],
            capture_output=True,
            text=True,
            timeout=5,
            cwd=os.getcwd(),
        )
        if res.returncode == 0:
            lines = [l.strip() for l in res.stdout.splitlines() if l.strip()]
            return [l[3:] if len(l) > 3 else l for l in lines]
    except Exception:
        pass
    return []


def run_codex_stop(dry_run=False):
    """Execute fail-open reflection checkpoint for Codex."""
    try:
        import hive
        import ledger

        cfg = hive.load_config()
        if not sys.stdin.isatty():  # PROV-01: Codex hook input carries its session id
            try:
                data = json.loads(sys.stdin.read() or "{}")
                if isinstance(data, dict):
                    ledger.set_context(agent="codex", session_id=data.get("session_id"))
            except (OSError, ValueError):
                pass
        changed_files = get_git_status()
        if not changed_files and not dry_run:
            # No meaningful file edits to reflect on
            return 0

        cwd = Path(os.getcwd())
        scope = hive.slugify(cwd.name)

        # Record reflect event in session ledger
        ledger.record(
            cfg["ledger"],
            event="reflect",
            project=scope,
            query="codex_session_stop",
            hits=changed_files[:10],
            duration_ms=0.0,
            agent="codex",
        )

        msg = {
            "agent": "codex",
            "scope": scope,
            "changed_files_count": len(changed_files),
            "reflection_prompt": (
                f"Hive ACE checkpoint for Codex: {len(changed_files)} file(s) modified in `{scope}`. "
                "Tag bullets used or add new lessons with `hive ace apply <scope> <ops_file>`. "
                "Skip if nothing reusable was learned."
            ),
        }
        print(json.dumps(msg, indent=2))
        return 0
    except Exception as e:
        print(f"codex_stop_hook: warning (fail-open): {e}", file=sys.stderr)
        return 0


if __name__ == "__main__":
    dry_run = "--dry-run" in sys.argv
    sys.exit(run_codex_stop(dry_run=dry_run))
