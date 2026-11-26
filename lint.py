"""Vault lint: deterministic health checks for the rules in the vault's AGENTS.md. Stdlib only, read-only.

    python lint.py [vault]      report; exit 1 if any ERROR
"""
import json
import os
import re
import sys
from collections import defaultdict
from datetime import date
from pathlib import Path
from typing import NamedTuple

sys.path.insert(0, str(Path(__file__).resolve().parent))  # `python -I` drops the script dir; hive.py sits beside us
import hive  # noqa: E402

ALLOWED = {
    "type": {"project-memory", "adr", "journal", "note", "codex"},
    "status": {"active", "superseded", "archived"},
    "agent": {"claude", "codex", "antigravity", "user"},
}
REQUIRED = ("type", "project", "agent", "status", "updated")
EXEMPT_FRONTMATTER = {"AGENTS.md", "CLAUDE.md", "GEMINI.md"}  # root manual and its pointers
LINE_CAPS = {"_meta/active-work.md": 40}
MEMORY_CAP = 150
STALE_DAYS = 30  # an active project-memory untouched this long probably no longer describes the project
CONVENTIONAL_NAMES = {"_memory", "playbook"}  # repeated per project by design
SKIP = hive.SKIP_DIRS - {"_attachments"}  # attachments are link targets, not notes
SEVERITY_ORDER = {"ERROR": 0, "WARN": 1, "INFO": 2}
SOURCE_RULE_FROM = "2026-11-23"  # `source:` became required on agent-written notes this day; older notes are exempt
SOURCED_PREFIXES = ("projects/", "inbox/")

LINK = re.compile(r"!?\[\[([^\]]+)\]\]")
JOURNAL_HEADER = re.compile(r"^## \d{2}:\d{2} [a-z]+ - \S.*?( \| [a-z-]+)?$")
FENCE = re.compile(r"^(```|~~~).*?^\1", re.M | re.S)
INLINE_CODE = re.compile(r"`[^`\n]*`")
ISO_DATE = re.compile(r"^\d{4}-\d{2}-\d{2}$")
WHAT_IT_IS = re.compile(r"^## What it is\b", re.M)


class Finding(NamedTuple):
    severity: str
    code: str
    path: str
    message: str


def walk(vault):
    """Yield vault-relative posix paths of every file outside skipped dirs."""
    stack = [Path(vault)]
    while stack:
        try:
            entries = list(os.scandir(stack.pop()))
        except PermissionError:  # an unreadable folder is not a vault note; skip it
            continue
        for entry in entries:
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
    if (meta.get("type") in ("adr", "note") and meta.get("agent") != "user" and rel.startswith(SOURCED_PREFIXES)
            and Path(rel).stem not in CONVENTIONAL_NAMES  # playbooks are machine-merged by ace.py
            and updated >= SOURCE_RULE_FROM and not meta.get("source")):
        out.append(Finding("ERROR", "source-missing", rel, "agent-written note needs `source:` (commit, session, URL or file)"))
    return out


def check_codex(rel, meta, text):
    """CODEX-04: a codex whose body no longer matches content_sha256 was edited; codices are immutable."""
    if meta.get("type") != "codex":
        return []
    import codex
    problem = codex.verify(text)
    return [Finding("ERROR", "codex-tampered", rel, problem)] if problem else []


def check_secrets(rel, text):
    ace = hive.ace()
    m = ace.SECRET_RE.search(text)
    if m:
        return [Finding("ERROR", "secret-detected", rel, f"note appears to contain a secret: {m.group(0)[:20]}...")]
    return []


def check_journal(rel, body):
    if not rel.startswith("journal/"):
        return []
    return [Finding("WARN", "journal-header", rel, f"{line!r} is not '## HH:MM <agent> - <project> [| <kind>]'")
            for line in body.splitlines() if line.startswith("## ") and not JOURNAL_HEADER.match(line)]


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
    if rel.endswith("/_memory.md") and not WHAT_IT_IS.search(text):
        out.append(Finding("WARN", "memory-undescribed", rel,
                           "no '## What it is' section: search cannot match the project by what it is"))
    return out


def git_staged_files(vault):
    """Return list of staged .md files in the vault git repo."""
    import subprocess
    try:
        proc = subprocess.run(
            ["git", "diff", "--cached", "--name-only", "--diff-filter=ACM"],
            cwd=str(vault), capture_output=True, text=True, check=True
        )
        return [line.strip().replace("\\", "/") for line in proc.stdout.splitlines() if line.strip().endswith(".md")]
    except Exception as exc:
        print(f"lint: warning: failed to get staged files: {exc}", file=sys.stderr)
        return None


def check_adr_staleness(rel, text, projects_root):
    if not (rel.startswith("projects/") and "/adrs/" in rel):
        return []
    parts = rel.split("/")
    if len(parts) < 3:
        return []
    slug = parts[1]
    proj_dir = Path(projects_root) / slug
    graphify_out = proj_dir / "graphify-out"
    manifest_p = graphify_out / "manifest.json"
    graph_p = graphify_out / "graph.json"

    known_symbols = set()
    deleted_symbols = set()
    if manifest_p.is_file():
        try:
            m = json.loads(manifest_p.read_text(encoding="utf-8"))
            known_symbols.update(m.get("symbols", []))
            deleted_symbols.update(m.get("deleted_symbols", []))
        except Exception:
            pass
    elif graph_p.is_file():
        try:
            g = json.loads(graph_p.read_text(encoding="utf-8"))
            for node in g.get("nodes", []):
                if node.get("name"):
                    known_symbols.add(node["name"])
                if node.get("label"):
                    known_symbols.add(node["label"])
        except Exception:
            pass

    if not known_symbols and not deleted_symbols:
        return []

    raw_symbols = re.findall(r"`([A-Za-z_][A-Za-z0-9_:]*)`", text)
    findings = []
    stopwords = {"true", "false", "null", "none", "str", "int", "dict", "bool", "json", "path", "self", "file"}
    checked = set()
    for sym in raw_symbols:
        if sym.lower() in stopwords or len(sym) < 3 or sym in checked:
            continue
        checked.add(sym)
        if sym in deleted_symbols:
            findings.append(Finding("WARN", "adr-stale", rel, f"ADR references deleted symbol `{sym}`"))
        elif known_symbols and sym not in known_symbols and ("::" in sym or any(c.isupper() for c in sym[1:])):
            findings.append(Finding("WARN", "adr-stale", rel, f"ADR references symbol `{sym}` not found in graphify manifest"))
    return findings


def lint(vault=hive.VAULT, today=None, files=None):
    vault = Path(vault)
    today = today or date.today().isoformat()
    projects_root = hive.load_config().get("projects", hive.PROJECTS)
    all_files = list(walk(vault))
    resolver = Resolver(all_files)
    if files is not None:
        notes = [f.replace("\\", "/") for f in files if f.endswith(".md") and not f.startswith("_attachments/")]
    else:
        notes = [f for f in all_files if f.endswith(".md") and not f.startswith("_attachments/")]
    findings, inbound, outbound = [], defaultdict(set), {}
    import governance
    hidden = governance.quarantined_paths(vault)  # GOV-02: warn on links into a quarantine radius
    for rel in notes:
        file_path = vault / rel if not Path(rel).is_absolute() else Path(rel)
        try:
            rel_posix = file_path.resolve().relative_to(vault.resolve()).as_posix()
        except (ValueError, OSError):
            rel_posix = rel.replace("\\", "/")
        if not file_path.is_file():
            continue
        text = file_path.read_text(encoding="utf-8-sig", errors="replace")
        meta, _, body = hive.parse_note(text)
        findings += check_frontmatter(rel_posix, meta, text.startswith("---"))
        findings += check_secrets(rel_posix, text)
        findings += check_codex(rel_posix, meta, text)
        findings += check_size_and_age(rel_posix, meta, text, today)
        findings += check_journal(rel_posix, body)
        outbound[rel_posix] = set()
        for target in links_in(body):
            hits = resolver.resolve(target)
            if not hits:
                findings.append(Finding("WARN", "broken-link", rel_posix, f"[[{target}]] resolves to no note or file"))
            for hit in hits:
                inbound[hit].add(rel_posix)
                outbound[rel_posix].add(hit)
                if hit in hidden:
                    findings.append(Finding("WARN", "quarantined-link", rel_posix,
                                            f"[[{target}]] points at quarantined {hit}"))
        if meta.get("type") == "adr":
            findings += check_adr_staleness(rel_posix, text, projects_root)
            if meta.get("status") == "superseded" and not (outbound[rel_posix] - {rel_posix}):
                findings.append(Finding("ERROR", "superseded-unlinked", rel_posix, "superseded ADR must link its successor"))
    findings += check_names(notes)
    if files is None:
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


def format_concise(findings):
    """DETAIL-01: the head line, every ERROR row verbatim, one `{SEV} {code} x{n}` line per WARN/INFO code."""
    report = format_report(findings).splitlines()
    errors = [r for r in report[1:] if r.startswith("ERROR")]
    counts = {}
    for f in findings:
        if f.severity != "ERROR":
            counts[(f.severity, f.code)] = counts.get((f.severity, f.code), 0) + 1
    rows = [f"{s} {c} x{n}" for (s, c), n in sorted(counts.items(), key=lambda kv: (SEVERITY_ORDER[kv[0][0]], kv[0][1]))]
    footer = ['detail="detailed" lists every finding'] if rows else []
    return "\n".join([report[0], *errors, *rows, *footer])


def main(argv):
    staged_mode = "--staged" in argv
    rest = [a for a in argv if a != "--staged"]
    vault = Path(rest[0]) if rest else hive.VAULT
    if staged_mode:
        staged = git_staged_files(vault)
        if staged is not None and not staged:
            sys.stdout.reconfigure(encoding="utf-8")
            print("lint: 0 error, 0 warn, 0 info (0 staged files)")
            return 0
        findings = lint(vault, files=staged)
    else:
        findings = lint(vault)
    sys.stdout.reconfigure(encoding="utf-8")
    print(format_report(findings))
    return 1 if any(f.severity == "ERROR" for f in findings) else 0


if __name__ == "__main__":
    native = hive.find_native_binary()
    if native:
        import subprocess
        sys.exit(subprocess.call([native, "lint", *sys.argv[1:]]))
    sys.exit(main(sys.argv[1:]))
