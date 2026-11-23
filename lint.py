"""Vault lint: deterministic health checks for the rules in the vault's AGENTS.md. Stdlib only, read-only.

    python lint.py [vault]      report; exit 1 if any ERROR
"""
import os
import re
import sys
from collections import defaultdict
from datetime import date
from pathlib import Path
from typing import NamedTuple

import hive

ALLOWED = {
    "type": {"project-memory", "adr", "journal", "note"},
    "status": {"active", "superseded", "archived"},
    "agent": {"claude", "codex", "antigravity", "user", "laya"},
}
REQUIRED = ("type", "project", "agent", "status", "updated")
EXEMPT_FRONTMATTER = {"AGENTS.md", "CLAUDE.md", "GEMINI.md"}  # root manual and its pointers
LINE_CAPS = {"_meta/active-work.md": 40}
MEMORY_CAP = 150
STALE_DAYS = 30  # an active project-memory untouched this long probably no longer describes the project
CONVENTIONAL_NAMES = {"_memory", "playbook"}  # repeated per project by design
SKIP = hive.SKIP_DIRS - {"_attachments"}  # attachments are link targets, not notes
SEVERITY_ORDER = {"ERROR": 0, "WARN": 1, "INFO": 2}

LINK = re.compile(r"!?\[\[([^\]]+)\]\]")
FENCE = re.compile(r"^(```|~~~).*?^\1", re.M | re.S)
INLINE_CODE = re.compile(r"`[^`\n]*`")
ISO_DATE = re.compile(r"^\d{4}-\d{2}-\d{2}$")


class Finding(NamedTuple):
    severity: str
    code: str
    path: str
    message: str


def walk(vault):
    """Yield vault-relative posix paths of every file outside skipped dirs."""
    stack = [Path(vault)]
    while stack:
        for entry in os.scandir(stack.pop()):
            if entry.is_dir(follow_symlinks=False):
                if entry.name not in SKIP:
                    stack.append(Path(entry.path))
            else:
                yield Path(entry.path).relative_to(vault).as_posix()


def links_in(body):
    """Wikilink targets outside code, without alias or heading; same-note links dropped."""
    text = INLINE_CODE.sub("", FENCE.sub("", body))
    targets = (m.group(1).replace(r"\|", "|").split("|")[0].split("#")[0].strip() for m in LINK.finditer(text))
    return [t for t in targets if t]


class Resolver:
    """Obsidian-style, case-insensitive: bare name matches a note stem or file name; a/b matches a path."""

    def __init__(self, files):
        self.by_name, self.by_path = defaultdict(list), {}
        for rel in files:
            name = rel.rsplit("/", 1)[-1].lower()
            self.by_name[name].append(rel)
            self.by_path[rel.lower()] = rel
            if name.endswith(".md"):
                self.by_name[name[:-3]].append(rel)
                self.by_path[rel[:-3].lower()] = rel

    def resolve(self, target):
        key = target.lower().lstrip("/")
        if "/" in key:
            hit = self.by_path.get(key)
            return [hit] if hit else []
        return self.by_name.get(key, [])


def check_frontmatter(rel, meta, has_frontmatter):
    if rel in EXEMPT_FRONTMATTER:
        return []
    if not has_frontmatter:
        return [Finding("ERROR", "frontmatter-missing", rel, "no frontmatter block")]
    out = [Finding("ERROR", "frontmatter-missing", rel, f"missing key {key!r}") for key in REQUIRED if not meta.get(key)]
    for key, allowed in ALLOWED.items():
        value = meta.get(key)
        if value and value not in allowed:
            out.append(Finding("ERROR", "bad-value", rel, f"{key}: {value!r} not in {sorted(allowed)}"))
    updated = meta.get("updated", "")
    if updated and not ISO_DATE.match(updated):
        out.append(Finding("ERROR", "bad-date", rel, f"updated: {updated!r} is not YYYY-MM-DD"))
    return out


def check_size_and_age(rel, meta, text, today):
    out = []
    cap = MEMORY_CAP if rel.endswith("/_memory.md") else LINE_CAPS.get(rel)
    lines = text.count("\n") + 1
    if cap and lines > cap:
        out.append(Finding("ERROR", "line-cap", rel, f"{lines} lines, cap {cap}: rewrite it shorter"))
    updated = meta.get("updated", "")
    if meta.get("type") == "project-memory" and meta.get("status") == "active" and ISO_DATE.match(updated):
        age = (date.fromisoformat(today) - date.fromisoformat(updated)).days
        if age > STALE_DAYS:
            out.append(Finding("WARN", "stale", rel, f"updated {age} days ago; verify it still matches the project"))
    return out


def lint(vault=hive.VAULT, today=None):
    vault = Path(vault)
    today = today or date.today().isoformat()
    files = list(walk(vault))
    notes = [f for f in files if f.endswith(".md") and not f.startswith("_attachments/")]
    resolver = Resolver(files)
    findings, inbound, outbound = [], defaultdict(set), {}
    for rel in notes:
        text = (vault / rel).read_text(encoding="utf-8", errors="replace")
        meta, _, body = hive.parse_note(text)
        findings += check_frontmatter(rel, meta, text.startswith("---"))
        findings += check_size_and_age(rel, meta, text, today)
        outbound[rel] = set()
        for target in links_in(body):
            hits = resolver.resolve(target)
            if not hits:
                findings.append(Finding("WARN", "broken-link", rel, f"[[{target}]] resolves to no note or file"))
            for hit in hits:
                inbound[hit].add(rel)
                outbound[rel].add(hit)
        if meta.get("type") == "adr" and meta.get("status") == "superseded" and not (outbound[rel] - {rel}):
            findings.append(Finding("ERROR", "superseded-unlinked", rel, "superseded ADR must link its successor"))
    findings += check_names(notes)
    findings += check_orphans(notes, inbound)
    return findings


def check_names(notes):
    by_stem = defaultdict(list)
    for rel in notes:
        by_stem[Path(rel).stem.lower()].append(rel)
    return [Finding("WARN", "duplicate-name", rel, f"[[{stem}]] is ambiguous: {', '.join(paths)}")
            for stem, paths in by_stem.items() if len(paths) > 1 and stem not in CONVENTIONAL_NAMES
            for rel in paths]


def check_orphans(notes, inbound):
    """Project notes nothing links to are invisible in Obsidian's graph; journal and inbox are unlinked by design."""
    return [Finding("INFO", "orphan", rel, "no note links here; link it from the project's _memory.md")
            for rel in notes
            if rel.startswith("projects/") and Path(rel).stem not in CONVENTIONAL_NAMES and not inbound[rel] - {rel}]


def format_report(findings):
    counts = {s: sum(f.severity == s for f in findings) for s in SEVERITY_ORDER}
    head = "lint: " + ", ".join(f"{n} {s.lower()}" for s, n in counts.items())
    rows = sorted(findings, key=lambda f: (SEVERITY_ORDER[f.severity], f.code, f.path))
    return "\n".join([head] + [f"{f.severity:5} {f.code:20} {f.path}: {f.message}" for f in rows])


def main(argv):
    findings = lint(Path(argv[0]) if argv else hive.VAULT)
    sys.stdout.reconfigure(encoding="utf-8")
    print(format_report(findings))
    return 1 if any(f.severity == "ERROR" for f in findings) else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
