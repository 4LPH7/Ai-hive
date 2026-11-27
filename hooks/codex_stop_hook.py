"""Codex stop adapter: delegates to the native binary's ACE stop hook (`hive ace hook-stop`), with the client's
hook JSON on stdin. Fail-open: never crashes the host Codex process."""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from hive import run  # noqa: E402

if __name__ == "__main__":
    try:
        sys.exit(run(["ace", "hook-stop"]))
    except Exception:
        sys.exit(0)
