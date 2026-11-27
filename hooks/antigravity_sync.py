"""Antigravity lifecycle adapter. `start` injects playbook lessons (SessionStart); `stop` runs the ACE checkpoint
(Stop). Both delegate to the native binary: `hive ace hook-start|hook-stop`, with the client's hook JSON on stdin.
Fail-open: never crashes the host agent."""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from hive import run  # noqa: E402

if __name__ == "__main__":
    action = sys.argv[1] if len(sys.argv) > 1 else "start"
    sub = {"start": "hook-start", "stop": "hook-stop"}.get(action)
    if sub is None:
        sys.exit(0)
    try:
        sys.exit(run(["ace", sub]))
    except Exception:
        sys.exit(0)
