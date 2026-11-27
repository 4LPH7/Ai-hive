"""Launcher: `python lint.py [vault] [--staged]` runs `hive lint`. See hive.py for how the binary is found."""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from hive import run  # noqa: E402

if __name__ == "__main__":
    sys.exit(run(["lint", *sys.argv[1:]]))
