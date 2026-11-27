"""Launcher: `python -I ace.py <args>` runs `hive ace <args>` (hook-start, hook-stop, apply, show). The hook
JSON on stdin is passed through unchanged. See hive.py for how the binary is found."""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from hive import run  # noqa: E402

if __name__ == "__main__":
    sys.exit(run(["ace", *sys.argv[1:]]))
