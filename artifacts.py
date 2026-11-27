"""DETAIL-02 (v2.7.1): large MCP tool output goes to a disposable, content-addressed cache outside the vault and
repos; the response keeps a packed head plus a pointer. Python mirror of src/artifact.rs."""
import hashlib
import os
import re
import sys
import tempfile
import time
from pathlib import Path

import ace
import hive

OFFLOAD_TOKENS = 2000
HEAD_TOKENS = 400
PURGE_AGE_S = 7 * 86400
NAME_RE = re.compile(r"^[0-9a-f]{64}\.md(\.tmp-\d+)?$")


def artifacts_dir():
    """HIVE_ARTIFACTS, else LOCALAPPDATA (TEMP) on Windows, else XDG_CACHE_HOME (~/.cache); then hive/artifacts."""
    if os.environ.get("HIVE_ARTIFACTS"):
        return Path(os.environ["HIVE_ARTIFACTS"])
    if os.name == "nt":
        base = os.environ.get("LOCALAPPDATA") or os.environ.get("TEMP") or tempfile.gettempdir()
        return Path(base) / "hive" / "artifacts"
    base = os.environ.get("XDG_CACHE_HOME") or str(Path.home() / ".cache")
    return Path(base) / "hive" / "artifacts"


def _inside(child, parent):
    try:
        return Path(child).resolve().is_relative_to(Path(parent).resolve())
    except (OSError, ValueError):
        return False


def purge(folder, now):
    """Drop cache files older than 7 days; only hive's own names, errors ignored."""
    try:
        entries = list(Path(folder).iterdir())
    except OSError:
        return
    for p in entries:
        if NAME_RE.fullmatch(p.name):
            try:
                if now - p.stat().st_mtime > PURGE_AGE_S:
                    p.unlink()
            except OSError:
                pass


def offload(text, now=None):
    """Text unchanged when <= OFFLOAD_TOKENS; else store it (secrets scrubbed) and return head + pointer.
    Any IO or guard failure returns the full text."""
    if hive.est_tokens(text) <= OFFLOAD_TOKENS:
        return text
    scrubbed = ace.scrub_secrets(text)
    sha = hashlib.sha256(scrubbed.encode("utf-8")).hexdigest()
    try:
        folder = artifacts_dir()
        if _inside(folder, hive.VAULT) or _inside(folder, hive.PROJECTS):  # before mkdir: nothing lands there
            raise OSError(f"artifacts dir {folder} is inside the vault or projects root")
        folder.mkdir(parents=True, exist_ok=True)
        path = folder / f"{sha}.md"
        if path.exists():
            os.utime(path)  # keep a hot artifact alive past the purge
        else:
            tmp = folder / f"{sha}.md.tmp-{os.getpid()}"
            tmp.write_text(scrubbed, encoding="utf-8", newline="\n")
            os.replace(tmp, path)  # content-addressed: a concurrent writer wrote the same bytes
            purge(folder, time.time() if now is None else now)
    except OSError as exc:
        print(f"hive: offload skipped, returning full output: {exc}", file=sys.stderr)
        return text
    return (f"{excerpt(scrubbed, HEAD_TOKENS)}\n[full output offloaded: {os.path.abspath(path)} "
            f"(~{hive.est_tokens(scrubbed)} tokens); disposable, purged after 7 days]")


# Signal lines survive offload wherever they sit (idea adapted from Headroom's log compressor, own code): a FATAL
# at line 900 of a 1,000-line output must not be cut just because the inline excerpt keeps only the head.
SIGNAL_RE = re.compile(r"(?i)\b(error|fatal|critical|panic(?:ked)?|traceback|exception|failed|failure)\b|\bFAIL\b")
HEAD_SHARE, TAIL_SHARE = 0.55, 0.15  # the rest of the budget goes to signal lines


def _cost(line):
    """Per-line token cost (chars / 4 + newline and rounding); identical in Rust, unlike est_tokens on one line."""
    return len(line) // 4 + 2


def _take(lines, budget):
    """Leading lines whose cost fits the budget."""
    out, used = [], 0
    for line in lines:
        cost = _cost(line)
        if used + cost > budget:
            break
        out.append(line)
        used += cost
    return out


def excerpt(text, budget=HEAD_TOKENS):
    """Head + numbered signal lines + tail within ~budget est tokens; the whole text when it already fits."""
    lines = text.splitlines()
    if sum(_cost(l) for l in lines) <= budget:
        return text
    head = _take(lines, int(budget * HEAD_SHARE))
    tail = list(reversed(_take(list(reversed(lines[len(head):])), int(budget * TAIL_SHARE))))
    middle_end = len(lines) - len(tail)
    signals = [f"L{i + 1}: {lines[i]}" for i in range(len(head), middle_end) if SIGNAL_RE.search(lines[i])]
    room = budget - sum(_cost(l) for l in head + tail) - 30
    kept = _take(signals, max(0, room))
    omitted = middle_end - len(head)
    parts = head + [f"...[{omitted} lines omitted; {len(kept)} of {len(signals)} signal lines kept]"] + kept
    return "\n".join(parts + (["..."] + tail if tail else []))
