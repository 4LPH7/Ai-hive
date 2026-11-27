"""testcache: skip re-running a test command whose result is already known for this exact tree.

Usage (from inside any git repo):
  python testcache.py run -- cargo test          # runs, or reuses a PASS recorded for the same tree
  python testcache.py run --force -- pytest -q   # always run, then record
  python testcache.py show                       # list recorded results for the current tree

Key = HEAD sha + hash of uncommitted changes (tracked diff + untracked file contents) + the command.
Only passing runs are reused; a failure is always re-run. Results live in .git/hive-testcache.json, so
nothing is ever committed.
"""
import datetime
import hashlib
import json
import subprocess
import sys
from pathlib import Path

MAX_ENTRIES = 200


def git(*args, cwd=None):
    return subprocess.run(["git", *args], cwd=cwd, capture_output=True, check=True).stdout


def tree_state():
    """(repo root, HEAD sha, hash of the working tree's uncommitted state)."""
    root = Path(git("rev-parse", "--show-toplevel").decode().strip())
    sha = git("rev-parse", "HEAD", cwd=root).decode().strip()
    h = hashlib.sha256(git("diff", "HEAD", "--binary", cwd=root))
    for rel in sorted(git("ls-files", "--others", "--exclude-standard", "-z", cwd=root).split(b"\0")):
        if rel:
            h.update(rel)
            try:
                h.update(hashlib.sha256((root / rel.decode()).read_bytes()).digest())
            except OSError:
                pass
    return root, sha, h.hexdigest()[:16]


def load(path):
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return {}


def save(path, data):
    items = sorted(data.items(), key=lambda kv: kv[1]["ts"])[-MAX_ENTRIES:]
    tmp = path.with_suffix(".tmp")
    tmp.write_text(json.dumps(dict(items), indent=1), encoding="utf-8")
    tmp.replace(path)


def run(cmd, force):
    root, sha, tree = tree_state()
    store_path = root / ".git" / "hive-testcache.json"
    data = load(store_path)
    key = hashlib.sha256(f"{sha}|{tree}|{json.dumps(cmd)}".encode()).hexdigest()[:20]
    hit = data.get(key)
    if hit and hit["exit"] == 0 and not force:
        print(f"testcache: HIT {' '.join(cmd)} passed at {hit['ts']} (sha {sha[:8]}, tree {tree}); not re-run")
        print(hit["tail"])
        return 0
    proc = subprocess.run(cmd, cwd=Path.cwd(), capture_output=True, text=True, encoding="utf-8", errors="replace")
    out = (proc.stdout or "") + (proc.stderr or "")
    sys.stdout.write(out)
    tail = "\n".join(out.strip().splitlines()[-8:])
    data[key] = {"cmd": cmd, "sha": sha, "tree": tree, "exit": proc.returncode, "tail": tail,
                 "ts": datetime.datetime.now(datetime.timezone.utc).isoformat(timespec="seconds")}
    save(store_path, data)
    print(f"testcache: recorded exit {proc.returncode} for sha {sha[:8]} tree {tree}")
    return proc.returncode


def show():
    root, sha, tree = tree_state()
    rows = [v for v in load(root / ".git" / "hive-testcache.json").values() if v["sha"] == sha and v["tree"] == tree]
    if not rows:
        print(f"testcache: nothing recorded for sha {sha[:8]} tree {tree}")
    for v in rows:
        print(f"{'PASS' if v['exit'] == 0 else 'FAIL'} {' '.join(v['cmd'])} ({v['ts']})")
    return 0


def main(argv):
    if argv[:1] == ["show"]:
        return show()
    if argv[:1] == ["run"] and "--" in argv:
        i = argv.index("--")
        return run(argv[i + 1:], "--force" in argv[1:i]) if argv[i + 1:] else 2
    print(__doc__)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
