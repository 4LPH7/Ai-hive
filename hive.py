"""Launcher for the hive engine. The engine is the native Rust binary; this file exists so existing configs
(`python -I hive.py serve`, `python hive.py status`) keep working. Build it once with `cargo build --release`.
Lookup: HIVE_BIN, then target/release/hive(.exe) in this repo, then `hive` on PATH."""
import os
import shutil
import subprocess
import sys
from pathlib import Path


def find_binary():
    if os.environ.get("HIVE_BIN"):
        return os.environ["HIVE_BIN"]
    here = Path(__file__).resolve().parent
    for name in ("hive.exe", "hive"):
        candidate = here / "target" / "release" / name
        if candidate.is_file():
            return str(candidate)
    return shutil.which("hive")


def run(argv):
    exe = find_binary()
    if not exe:
        sys.stderr.write("hive: native binary not found; run `cargo build --release` in the repo\n")
        return 127
    return subprocess.call([exe, *argv])


if __name__ == "__main__":
    sys.exit(run(sys.argv[1:]))
