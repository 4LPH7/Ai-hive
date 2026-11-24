"""Hive: shared memory for Claude Code, Codex and Antigravity over an Obsidian vault
and graphify code graphs. Stdlib only (Python 3.10+), read-only, MCP over stdio.

    python hive.py serve                      MCP stdio server (what agents launch)
    python hive.py search "query" [project]   BM25 search over vault notes
    python hive.py graph <repo> "query"       ranked code-graph neighbourhood
    python hive.py context <project> ["task"] token-budgeted session pack
    python hive.py status                     index and graph freshness
    python hive.py lint                       vault health report (see lint.py)
"""
import json
import math
import os
import re
import sqlite3
import sys
import time
from collections import defaultdict
from pathlib import Path

VAULT = Path(os.environ.get("HIVE_VAULT", r"D:\Valut\OBS"))
PROJECTS = Path(os.environ.get("HIVE_PROJECTS", r"D:\PROJECTS"))
DB_PATH = Path(os.environ.get("HIVE_DB") or Path(
    os.environ.get("LOCALAPPDATA") or os.environ.get("TEMP") or os.getcwd()) / "hive" / "hive.sqlite")
SKIP_DIRS = {".git", ".obsidian", ".claude", ".agents", ".trash", "Python", "_attachments", "graph", "node_modules", "templates", "_template", ".claude-flow"}
REFRESH_EVERY_S = 2.0
TYPE_PRIOR = {"project-memory": 1.4, "adr": 1.25, "note": 1.0, "journal": 0.8}
STATUS_PRIOR = {"superseded": 0.3, "archived": 0.4}
SCHEMA_VERSION = 3
PROJECT_BOOST = 2.0  # eval 2026-11-16: 1.5 -> 28/30, 2.0 -> 29/30 (higher gives no further gain)
AUTOCUT_RATIO = 0.30  # keep hits >= 30% of the best; eval 2026-11-23: 0.35 cut a rank-4 answer (29/30), 0.30 -> 30/30, +0.3 hits
SERVER_INFO = {"name": "hive", "version": "1.1.0"}
MODERN_VERSIONS = ["2026-07-28"]
LEGACY_VERSIONS = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"]
META_VERSION = "io.modelcontextprotocol/protocolVersion"
INSTRUCTIONS = ("Shared memory for all coding agents (Obsidian vault + graphify graphs). Read-only. "
                "Call hive_context at the start of a task, hive_search before asking the user for context, "
                "hive_graph for code structure. Write memory as Markdown files following the vault's AGENTS.md.")


READ_ONLY = {"readOnlyHint": True, "destructiveHint": False, "idempotentHint": True, "openWorldHint": False}
LIST_CACHE = {"ttlMs": 3600000, "cacheScope": "public"}  # required on modern tools/list + server/discover
DEFAULT_SECTION_WEIGHTS = {"memory": 0.35, "playbook": 0.15, "task_notes": 0.25, "code_graph": 0.25}
FORWARD_WEIGHT = 1.0
REVERSE_WEIGHT = 0.5


def est_tokens(text):
    """Content-aware token estimator: code vs prose + 10% safety margin."""
    if not text:
        return 0
    code_chars, prose_chars = 0, 0
    in_fence = False
    for line in text.splitlines(keepends=True):
        stripped = line.strip()
        if stripped.startswith(("```", "~~~")):
            in_fence = not in_fence
            code_chars += len(line)
            continue
        if in_fence or line.startswith(("    ", "\t")) or (stripped and stripped[-1:] in "{};"):
            code_chars += len(line)
        else:
            prose_chars += len(line)
    toks = (code_chars / 3.0 + prose_chars / 4.0) * 1.1
    return int(math.ceil(toks))


def slugify(name):
    return name.strip().lower().replace(" ", "-")


def validate_slug(name):
    s = slugify(name)
    if not re.fullmatch(r"[a-z0-9._-]+", s) or s in {".", ".."}:
        raise ValueError(f"bad slug {name!r}")
    return s


def note_age(updated):
    if not updated:
        return "unknown age"
    try:
        from datetime import date
        days = (date.today() - date.fromisoformat(updated)).days
        if days == 0:
            return "today"
        if days == 1:
            return "1d ago"
        return f"{days}d ago"
    except (ValueError, TypeError):
        return updated


def active_work_cell_matches(cell, project):
    """Exact match of active-work first table cell to project slug."""
    clean = cell.strip().strip("[]").strip()
    if "|" in clean:
        path, _, alias = clean.partition("|")
        candidates = [slugify(alias), slugify(path.split("/")[1] if path.startswith("projects/") else path)]
    else:
        candidates = [slugify(clean.split("/")[1] if clean.startswith("projects/") else clean)]
    return project in candidates


# ---------------------------------------------------------------- vault index

def parse_note(text):
    """Return (frontmatter dict, title, body). Flat `key: value` frontmatter only."""
    if text.startswith("\ufeff"):
        text = text[1:]
    meta, body = {}, text
    if text.startswith("---"):
        end = text.find("\n---", 3)
        if end != -1:
            for line in text[3:end].splitlines():
                key, sep, value = line.partition(":")
                if sep and key.strip() and not key.startswith((" ", "\t", "-")):
                    meta[key.strip()] = value.strip().strip("'\"")
            body = text[end + 4:].lstrip("-\n")
    heading = re.search(r"^#\s+(.+)$", body, re.M)
    return meta, (heading.group(1).strip() if heading else ""), body


class Index:
    def __init__(self, vault=VAULT, db_path=DB_PATH):
        self.vault = Path(vault)
        Path(db_path).parent.mkdir(parents=True, exist_ok=True)
        self.db = sqlite3.connect(str(db_path), timeout=10, check_same_thread=False)
        self.db.execute("PRAGMA journal_mode=WAL")
        if self.db.execute("PRAGMA user_version").fetchone()[0] != 3:
            # derived index: a schema change just rebuilds it from the vault
            self.db.executescript("DROP TABLE IF EXISTS notes; DROP TABLE IF EXISTS notes_fts;")
            self.db.execute("PRAGMA user_version=3")
        self.db.executescript("""
            CREATE TABLE IF NOT EXISTS notes(path TEXT PRIMARY KEY, mtime REAL, project TEXT,
                type TEXT, status TEXT, agent TEXT, updated TEXT, source TEXT, title TEXT);
            CREATE VIRTUAL TABLE IF NOT EXISTS notes_fts USING fts5(
                path UNINDEXED, title, meta, body, tokenize='porter unicode61');
        """)
        self._last_refresh = 0.0

    def _walk(self):
        stack = [self.vault]
        while stack:
            for entry in os.scandir(stack.pop()):
                if entry.is_dir(follow_symlinks=False):
                    if entry.name not in SKIP_DIRS:
                        stack.append(entry.path)
                elif entry.name.endswith(".md"):
                    yield entry

    def refresh(self, force=False):
        """Re-index notes whose mtime changed; drop deleted ones. Returns changed count."""
        now = time.monotonic()
        if not force and now - self._last_refresh < REFRESH_EVERY_S:
            return 0
        self._last_refresh = now
        known = dict(self.db.execute("SELECT path, mtime FROM notes"))
        seen, changed = set(), 0
        with self.db:
            for entry in self._walk():
                rel = Path(entry.path).relative_to(self.vault).as_posix()
                seen.add(rel)
                mtime = entry.stat().st_mtime
                if known.get(rel) == mtime:
                    continue
                try:
                    text = Path(entry.path).read_text(encoding="utf-8-sig", errors="replace")
                except OSError as exc:
                    print(f"hive: skip {rel}: {exc}", file=sys.stderr)
                    continue
                meta, title, body = parse_note(text)
                title = title or Path(rel).stem
                project = meta.get("project") or (rel.split("/")[1] if rel.startswith("projects/") else "")
                self.db.execute("DELETE FROM notes_fts WHERE path=?", (rel,))
                self.db.execute("INSERT OR REPLACE INTO notes VALUES(?,?,?,?,?,?,?,?,?)",
                                (rel, mtime, project, meta.get("type", "note"), meta.get("status", "active"),
                                 meta.get("agent", ""), meta.get("updated", ""), meta.get("source", ""), title))
                # meta field: project, type and path words, so "fpolink adr mandi" matches on context
                meta_text = " ".join([project, meta.get("type", "note"), *re.split(r"[/_.\-]+", rel[:-3])])
                self.db.execute("INSERT INTO notes_fts VALUES(?,?,?,?)", (rel, title, meta_text, body))
                changed += 1
            for rel in set(known) - seen:
                self.db.execute("DELETE FROM notes WHERE path=?", (rel,))
                self.db.execute("DELETE FROM notes_fts WHERE path=?", (rel,))
                changed += 1
        return changed

    def search(self, query, project="", limit=8, scope="project"):
        """BM25 (title x5) re-ranked by note type, status and project match."""
        self.refresh()
        terms = re.findall(r"\w+", query.lower())
        if not terms:
            return []
        match = " OR ".join(f'"{t}"' for t in terms)
        rows = self.db.execute("""
            SELECT f.path, n.title, n.project, n.type, n.status, n.updated, n.agent, n.source,
                   -bm25(notes_fts, 0.0, 5.0, 2.0, 1.0) AS s,
                   snippet(notes_fts, 3, '[', ']', '...', 18)
            FROM notes_fts f JOIN notes n ON n.path = f.path
            WHERE notes_fts MATCH ? ORDER BY s DESC LIMIT ?""", (match, limit * 4)).fetchall()
        hits = []
        for path, title, proj, ntype, status, updated, agent, source, score, snip in rows:
            if scope == "project" and project and proj not in (project, "", "global"):
                continue
            score *= TYPE_PRIOR.get(ntype, 1.0) * STATUS_PRIOR.get(status, 1.0)
            if project and proj == project:
                score *= PROJECT_BOOST
            hits.append({"path": path, "title": title, "project": proj, "type": ntype, "status": status,
                         "updated": updated, "agent": agent, "source": source,
                         "score": round(score, 3), "snippet": " ".join(snip.split())})
        hits.sort(key=lambda h: -h["score"])
        if hits:  # autocut: drop weak tail hits so agents do not read noise
            floor = AUTOCUT_RATIO * hits[0]["score"]
            hits = [h for h in hits if h["score"] >= floor]
        return hits[:limit]

    def note(self, rel):
        try:
            path = (self.vault / rel).resolve()
            if not path.is_relative_to(self.vault.resolve()):
                return ""
            return path.read_text(encoding="utf-8-sig", errors="replace") if path.is_file() else ""
        except (ValueError, OSError):
            return ""

    def by_project(self, project, ntype):
        self.refresh()
        return self.db.execute("SELECT path, title, status, updated FROM notes WHERE project=? AND type=? "
                               "ORDER BY path", (project, ntype)).fetchall()


# ---------------------------------------------------------------- code graph

REL_WEIGHT = {"calls": 1.0, "indirect_call": 0.8, "inherits": 0.9, "implements": 0.9, "method": 0.8,
              "references": 0.7, "uses": 0.7, "defines": 0.6, "rationale_for": 0.6,
              "imports": 0.5, "imports_from": 0.5, "contains": 0.4}
PPR_RESTART = 0.15
PPR_ITERS = 25
PPR_HOPS = 2


KIND_PRIOR = {"code": 1.5, "document": 0.8, "concept": 0.8, "rationale": 0.6}


def node_prior(n):
    """Prefer real code over docs and docstring nodes, and non-test code over tests."""
    path = (n.get("source_file") or "").replace("\\", "/").lower()
    is_test = "/tests/" in f"/{path}" or Path(path).name.startswith("test_") or path.endswith("_test.py")
    return KIND_PRIOR.get(n.get("file_type"), 0.7) * (0.6 if is_test else 1.0) * (1.0 if path else 0.5)


def split_ident(text):
    spaced = re.sub(r"([a-z0-9])([A-Z])", r"\1 \2", text or "")
    return [p for p in re.split(r"[^A-Za-z0-9]+", spaced.lower()) if len(p) > 1]


def repo_dir(repo):
    """Accept a repo folder path, a folder name under PROJECTS, or a vault slug."""
    try:
        projects_root = PROJECTS.resolve()
    except (ValueError, OSError):
        return None
    path = Path(repo)
    if path.is_dir():
        try:
            resolved = path.resolve()
            if resolved.is_relative_to(projects_root) and (resolved / "graphify-out").is_dir():
                return resolved
        except (ValueError, OSError):
            pass
    want = slugify(path.name or str(repo))
    if not re.fullmatch(r"[a-z0-9._-]+", want) or want in {".", ".."}:
        return None
    if projects_root.is_dir():
        for d in projects_root.iterdir():
            if d.is_dir() and slugify(d.name) == want:
                return d
    return None


def git_head(repo):
    git = Path(repo) / ".git"
    try:
        head = (git / "HEAD").read_text().strip()
        if not head.startswith("ref:"):
            return head
        ref = head[4:].strip()
        if (git / ref).is_file():
            return (git / ref).read_text().strip()
        for line in (git / "packed-refs").read_text().splitlines():
            if line.endswith(" " + ref):
                return line.split()[0]
    except OSError:
        pass
    return ""


class Graph:
    def __init__(self, path):
        data = json.loads(Path(path).read_text(encoding="utf-8"))
        self.commit = data.get("built_at_commit") or ""
        self.nodes = {n["id"]: n for n in data.get("nodes", []) if "id" in n}
        self.out_adj = defaultdict(dict)  # source -> {target: (weight, relation)}
        self.in_adj = defaultdict(dict)   # target -> {source: (weight, relation)}
        for e in data.get("links") or data.get("edges") or []:
            s, t, rel = e.get("source"), e.get("target"), e.get("relation", "")
            if s == t or s not in self.nodes or t not in self.nodes:
                continue
            w = float(e.get("confidence_score") or 0.5) * REL_WEIGHT.get(rel, 0.6)
            if w > self.out_adj[s].get(t, (0.0, ""))[0]:
                self.out_adj[s][t] = (w, rel)
            if w > self.in_adj[t].get(s, (0.0, ""))[0]:
                self.in_adj[t][s] = (w, rel)
        self.tokens = defaultdict(set)
        for nid, n in self.nodes.items():
            words = split_ident(n.get("label", "")) + split_ident(Path(n.get("source_file") or "").stem)
            for tok in set(words):
                self.tokens[tok].add(nid)
        self.idf = {t: math.log(1 + len(self.nodes) / len(ids)) for t, ids in self.tokens.items()}

    def degree(self, nid):
        return len(self.out_adj[nid]) + len(self.in_adj[nid])

    def seeds(self, query, k=6):
        """Lexical seeds: idf-weighted identifier-token overlap, exact label match x3."""
        if query in self.nodes:
            return [query]
        exact = query.strip().lower()
        score = defaultdict(float)
        for tok in split_ident(query):
            for nid in self.tokens.get(tok, ()):
                score[nid] += self.idf[tok]
        for nid in score:
            n = self.nodes[nid]
            score[nid] *= node_prior(n) * (3 if n.get("label", "").lower() == exact else 1)
        return sorted(score, key=score.get, reverse=True)[:k]

    def rank(self, seeds, hops=PPR_HOPS, iters=PPR_ITERS):
        """Personalized PageRank on the k-hop neighbourhood of the seeds, with hub dampening
        (flow into a node is divided by sqrt(degree)) so god nodes do not swamp the result."""
        local, frontier = set(seeds), set(seeds)
        for _ in range(hops):
            frontier = {b for a in frontier for b in (set(self.out_adj[a]) | set(self.in_adj[a]))} - local
            local |= frontier
        out = {}
        for a in local:
            nbrs = []
            for b, (w, _) in self.out_adj[a].items():
                if b in local:
                    nbrs.append((b, w * FORWARD_WEIGHT / math.sqrt(max(1, self.degree(b)))))
            for b, (w, _) in self.in_adj[a].items():
                if b in local:
                    nbrs.append((b, w * REVERSE_WEIGHT / math.sqrt(max(1, self.degree(b)))))
            total = sum(w for _, w in nbrs)
            out[a] = [(b, w / total) for b, w in nbrs] if total else []
        restart = {s: 1 / len(seeds) for s in seeds}
        rank = dict(restart)
        for _ in range(iters):
            nxt = defaultdict(float)
            dangling = 0.0
            for a, p in rank.items():
                if out[a]:
                    for b, w in out[a]:
                        nxt[b] += (1 - PPR_RESTART) * p * w
                else:
                    dangling += p
            for s, p in restart.items():
                nxt[s] += (PPR_RESTART + (1 - PPR_RESTART) * dangling) * p
            rank = nxt
        return rank

    def query(self, query, limit=15):
        seeds = self.seeds(query)
        if not seeds:
            return [], []
        rank = {nid: p * node_prior(self.nodes[nid]) for nid, p in self.rank(seeds).items()}
        # matched seeds lead (they answer the query directly), then their neighbourhood by PPR mass
        top = (sorted(seeds, key=rank.get, reverse=True)
               + sorted(set(rank) - set(seeds), key=rank.get, reverse=True))[:limit]
        keep = set(top)
        # directed edges: caller (source) -> callee (target)
        edges = [(s, rel, t) for s in top for t, (_, rel) in self.out_adj[s].items() if t in keep]
        return [(nid, rank[nid], nid in seeds) for nid in top], edges


_GRAPHS = {}


def load_graph(repo):
    d = repo_dir(repo)
    if not d:
        raise ValueError(f"no repo folder matching {repo!r} under {PROJECTS}")
    path = d / "graphify-out" / "graph.json"
    if not path.is_file():
        raise ValueError(f"{d.name} has no graphify-out/graph.json; run `graphify update .` there")
    mtime = path.stat().st_mtime
    cached = _GRAPHS.get(path)
    if not cached or cached[0] != mtime:
        cached = _GRAPHS[path] = (mtime, Graph(path))
    return d, cached[1]


def fmt_node(n, seed=False):
    loc = n.get("source_location") or ""
    where = f"{n.get('source_file', '')}:{loc[1:] if loc.startswith('L') else loc}".rstrip(":")
    label = " ".join((n.get("label") or n["id"]).split())
    label = label if len(label) <= 80 else label[:77] + "..."
    return f"{'*' if seed else '-'} {label}  {where}  [{n.get('community_name', '')}]"


# ---------------------------------------------------------------- tools

_INDEX = None


def sibling(name):
    """Import ace.py or lint.py from next to this file; `python -I` drops the script dir from sys.path."""
    here = str(Path(__file__).resolve().parent)
    if here not in sys.path:
        sys.path.insert(0, here)
    return __import__(name)


def ace():
    return sibling("ace")


def index():
    global _INDEX
    if _INDEX is None:
        _INDEX = Index()
    return _INDEX


def changed_since_build(d):
    """Files graphify tracked whose mtime moved or that vanished, per graphify-out/manifest.json
    ({path: {mtime, ast_hash, ...}}). None if there is no manifest. New untracked files are not seen."""
    try:
        manifest = json.loads((Path(d) / "graphify-out" / "manifest.json").read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return None
    changed = []
    for rel, info in manifest.items():
        try:
            if abs(os.stat(Path(d) / rel).st_mtime - float(info.get("mtime", 0))) > 1e-3:
                changed.append(rel)
        except OSError:
            changed.append(rel)
    return changed


def graph_freshness(d, g):
    changed = changed_since_build(d)
    if changed is not None:
        if not changed:
            return f"graph current (no tracked file changed since build at {g.commit[:8] or 'unknown commit'})"
        names = ", ".join(changed[:5]) + (f" +{len(changed) - 5} more" if len(changed) > 5 else "")
        return f"graph STALE for {len(changed)} file(s): {names}; run `graphify update .` in {d}"
    head = git_head(d)
    if not g.commit:
        return "graph commit unknown"
    if head and not (head.startswith(g.commit) or g.commit.startswith(head)):
        return (f"graph built at {g.commit[:8]}, HEAD is {head[:8]}: may be stale; "
                f"`graphify update .` in {d} re-checks (no-op if code topology is unchanged)")
    return f"graph built at {g.commit[:8]}"


def tool_search(query, project="", scope="project", limit=8):
    if scope not in ("project", "all"):
        raise ValueError(f"invalid scope {scope!r}; must be 'project' or 'all'")
    slug = validate_slug(project) if project else ""
    hits = index().search(query, slug, int(limit), scope=scope)
    if not hits:
        return "no matching notes"
    return "\n".join(f"- {h['path']} ({h['type']}, {h['status']}, {h['updated']}) {h['title']}\n  {h['snippet']}"
                     for h in hits)


def tool_graph(repo, query, limit=15):
    d, g = load_graph(repo)
    ranked, edges = g.query(query, int(limit))
    if not ranked:
        return f"no symbols in {d.name} match {query!r}. {graph_freshness(d, g)}"
    lines = [f"{d.name}: {len(g.nodes)} nodes, {graph_freshness(d, g)}. * = matched seed", ""]
    lines += [fmt_node(g.nodes[nid], seed) for nid, _, seed in ranked]
    if edges:
        lines += ["", "edges among these:"]
        lines += [f"  {g.nodes[a].get('label')} -{rel}-> {g.nodes[b].get('label')}" for a, rel, b in edges[:25]]
    stems = {Path(g.nodes[nid].get("source_file") or "").stem for nid, _, _ in ranked[:6]} - {""}
    slug = slugify(d.name)
    notes = [h for h in index().search(" ".join(stems), slug, 8, scope="project") if h["project"] == slug][:4] if stems else []
    if notes:
        lines += ["", "vault notes about these files:"] + [f"  {h['path']} - {h['title']}" for h in notes]
    return "\n".join(lines)


def pack(sections, budget=1500, weights=None):
    """Quota-based packing with dynamic redistribution and omitted_sections footer.
    sections: dict of {name: text} or list of (name, text) or list of (name, text, weight) or list of str.
    """
    budget = int(budget)
    if not sections:
        return ""

    items = []
    if isinstance(sections, dict):
        w_map = weights or {}
        for name, text in sections.items():
            w = w_map.get(name, DEFAULT_SECTION_WEIGHTS.get(name, 1.0))
            items.append([name, text or "", float(w)])
    elif isinstance(sections, list):
        for idx, entry in enumerate(sections):
            if isinstance(entry, (list, tuple)):
                if len(entry) == 2:
                    name, text = entry
                    w = (weights or {}).get(name, DEFAULT_SECTION_WEIGHTS.get(name, 1.0))
                    items.append([name, text or "", float(w)])
                elif len(entry) >= 3:
                    name, text, w = entry[:3]
                    items.append([name, text or "", float(w)])
            elif isinstance(entry, str):
                name = f"section_{idx + 1}"
                items.append([name, entry, 1.0])

    active = [item for item in items if item[1].strip()]
    if not active:
        return ""

    costs = {name: est_tokens(text) for name, text, _ in active}
    w_sum = sum(w for _, _, w in active) or 1.0
    alloc = {name: int(budget * (w / w_sum)) for name, _, w in active}

    # Dynamic redistribution of surplus
    surplus = sum(max(0, alloc[name] - costs[name]) for name in alloc)
    for name in alloc:
        if costs[name] < alloc[name]:
            alloc[name] = costs[name]

    while surplus > 0:
        unsatisfied = [name for name, _, _ in active if costs[name] > alloc[name]]
        if not unsatisfied:
            break
        u_w_sum = sum(dict((n, w) for n, _, w in active)[n] for n in unsatisfied) or 1.0
        distributed = 0
        for n in unsatisfied:
            w = dict((name, w) for name, _, w in active)[n]
            share = max(1, int(surplus * (w / u_w_sum)))
            needed = costs[n] - alloc[n]
            add = min(needed, min(surplus - distributed, share))
            alloc[n] += add
            distributed += add
            if distributed >= surplus:
                break
        if distributed == 0:
            break
        surplus -= distributed

    out, omitted = [], []
    for name, text, _ in active:
        al = alloc[name]
        c = costs[name]
        if al < 15:
            omitted.append(name)
            continue
        if c <= al:
            out.append(text.strip())
        else:
            suffix = "\n...[truncated to budget]"
            suffix_tokens = est_tokens(suffix)
            avail_tokens = max(1, al - suffix_tokens)
            lines = text.strip().splitlines()
            low, high, best = 0, len(lines), 0
            while low <= high:
                mid = (low + high) // 2
                cand = "\n".join(lines[:mid])
                if est_tokens(cand) <= avail_tokens:
                    best = mid
                    low = mid + 1
                else:
                    high = mid - 1
            if best > 0:
                out.append("\n".join(lines[:best]) + suffix)
            else:
                chars = avail_tokens * 3
                out.append(text[:chars] + suffix)

    result = "\n\n".join(out)
    if omitted:
        result += f"\n\n[omitted_sections: {', '.join(omitted)}]"
    return result


def tool_context(project, task="", budget=1500):
    project = validate_slug(project)
    idx = index()

    # 1. Memory section
    mem_parts = []
    active_row = ""
    active_work_text = idx.note("_meta/active-work.md")
    for line in active_work_text.splitlines():
        if line.startswith("|"):
            cells = [c.strip() for c in line.split("|")]
            if len(cells) > 1 and active_work_cell_matches(cells[1], project):
                active_row = line
                break
    if active_row:
        mem_parts.append("active-work: " + active_row)

    memory_text = idx.note(f"projects/{project}/_memory.md")
    if memory_text:
        meta, _, body = parse_note(memory_text)
        agent = meta.get("agent", "unknown")
        source = meta.get("source", "unknown")
        updated = meta.get("updated", "")
        age = note_age(updated)
        framed_body = (f'<untrusted_vault_note path="projects/{project}/_memory.md" '
                       f'agent="{agent}" source="{source}" age="{age}">\n'
                       f'{body.strip()}\n'
                       f'</untrusted_vault_note>')
        mem_parts.append(f"## projects/{project}/_memory.md ({age})\n{framed_body}")
    else:
        mem_parts.append(f"## projects/{project}/_memory.md\n(none yet: create it from templates/project-memory.md)")

    adrs = idx.by_project(project, "adr")
    if adrs:
        mem_parts.append("## ADRs\n" + "\n".join(f"- {p} [{s}] {t}" for p, t, s, _ in adrs))

    memory_section = "\n\n".join(mem_parts)

    # 2. Playbook section
    playbook_section = ""
    try:
        rows = ace().select(["global", project], task)
        if rows:
            playbook_section = ("## Playbook (ACE; note ids you use, run skill hive-ace after substantial work)\n"
                                + ace().format_bullets(rows))
    except (OSError, ValueError) as exc:
        print(f"hive: playbook skipped: {exc}", file=sys.stderr)

    # 3. Task notes and code graph
    task_notes_section = ""
    code_graph_section = ""
    if task:
        hits = [h for h in idx.search(task, project, 6, scope="project")
                if not h["path"].endswith("/_memory.md") and h["project"] in (project, "", "global")]
        if hits:
            task_notes_section = ("## notes for this task\n" +
                                  "\n".join(f"- {h['path']} ({h.get('agent', '')}, age {note_age(h.get('updated', ''))}): {h['snippet']}"
                                            for h in hits))
        try:
            code_graph_section = "## code graph for this task\n" + tool_graph(project, task, 12)
        except ValueError as exc:
            code_graph_section = f"## code graph\n{exc}"

    sections = {
        "memory": memory_section,
        "playbook": playbook_section,
        "task_notes": task_notes_section,
        "code_graph": code_graph_section,
    }
    return pack(sections, budget=int(budget), weights=DEFAULT_SECTION_WEIGHTS)


def tool_lint():
    lint = sibling("lint")  # lint imports hive, so load it lazily
    return lint.format_report(lint.lint(VAULT))


def tool_status():
    idx = index()
    idx.refresh(force=True)
    total = idx.db.execute("SELECT count(*) FROM notes").fetchone()[0]
    by = idx.db.execute("SELECT project, count(*) FROM notes GROUP BY project ORDER BY 2 DESC").fetchall()
    lines = [f"vault {idx.vault}: {total} notes indexed ({DB_PATH})",
             "  " + ", ".join(f"{p or '(global)'}={c}" for p, c in by)]
    for d in sorted(PROJECTS.iterdir()) if PROJECTS.is_dir() else []:
        if (d / "graphify-out" / "graph.json").is_file():
            try:
                _, g = load_graph(d)
                lines.append(f"{d.name}: {len(g.nodes)} nodes, {graph_freshness(d, g)}")
            except (ValueError, OSError, json.JSONDecodeError) as exc:
                lines.append(f"{d.name}: {exc}")
    return "\n".join(lines)


TOOLS = {
    "hive_context": (tool_context,
                     "Call first on any task. Token-budgeted memory pack for a project: its active-work row, "
                     "_memory.md, ADR list and, if a task is given, matching vault notes and ranked code-graph symbols.",
                     {"project": ("string", "Project slug: folder name under D:\\PROJECTS, lowercased, spaces to hyphens"),
                      "task": ("string", "What you are about to do (optional, improves ranking)"),
                      "budget": ("integer", "Max tokens to return (default 1500)")}, ["project"]),
    "hive_search": (tool_search,
                    "BM25 search over vault notes. Default scope is project-isolated ('project'); use scope='all' for cross-project.",
                    {"query": ("string", "Keywords"),
                     "project": ("string", "Project slug to filter/boost (optional)"),
                     "scope": ("string", "Search scope: 'project' (default, project + global) or 'all' (cross-project)"),
                     "limit": ("integer", "Max hits (default 8)")}, ["query"]),
    "hive_graph": (tool_graph,
                   "Ranked code-graph neighbourhood from graphify (Personalized PageRank with hub dampening): "
                   "symbols with file:line, edges between them, related vault notes, and graph staleness.",
                   {"repo": ("string", "Repo folder name or slug under D:\\PROJECTS, or a path"),
                    "query": ("string", "Symbol names or keywords"), "limit": ("integer", "Max symbols (default 15)")},
                   ["repo", "query"]),
    "hive_status": (tool_status, "Index size per project and freshness of each repo's code graph.", {}, []),
    "hive_lint": (tool_lint,
                  "Vault health against AGENTS.md rules: frontmatter, line caps, broken links, stale memory, "
                  "superseded ADRs without successor, ambiguous names, orphans. Run before session end after writing notes.",
                  {}, []),
}

def tool_list():
    tools = []
    for name, (_, desc, props, req) in TOOLS.items():
        schema = {"type": "object", "required": req,
                  "properties": {k: {"type": t, "description": d} for k, (t, d) in props.items()},
                  "additionalProperties": False}
        tool = {"name": name, "description": desc, "inputSchema": schema, "annotations": READ_ONLY}
        if name == "hive_context":  # every session starts with it: load upfront in Claude Code, rest stay deferred
            tool["_meta"] = {"anthropic/alwaysLoad": True}
        tools.append(tool)
    return tools


def validate_tool_args(name, args):
    """Validate arguments against tool definition. Return list of error strings."""
    fn, _, props, req = TOOLS[name]
    if not isinstance(args, dict):
        return [f"arguments must be a JSON object, got {type(args).__name__}"]
    errors = []
    for r in req:
        if r not in args or args[r] is None:
            errors.append(f"missing required argument {r!r}")
    for k in args:
        if k not in props:
            errors.append(f"unexpected argument {k!r}; allowed: {sorted(props.keys())}")
        else:
            expected_type, _ = props[k]
            val = args[k]
            if val is not None:
                if expected_type == "string" and not isinstance(val, str):
                    errors.append(f"argument {k!r} must be string, got {type(val).__name__}")
                elif expected_type == "integer" and (isinstance(val, bool) or not isinstance(val, int)):
                    errors.append(f"argument {k!r} must be integer, got {type(val).__name__}")
                elif expected_type == "boolean" and not isinstance(val, bool):
                    errors.append(f"argument {k!r} must be boolean, got {type(val).__name__}")
    return errors


def handle(msg):
    """One JSON-RPC message in, one response dict out (None for notifications).
    Dual-era: legacy `initialize` sessions and modern per-request `_meta` versions."""
    mid, method, params = msg.get("id"), msg.get("method", ""), msg.get("params") or {}
    if mid is None:
        return None
    version = (params.get("_meta") or {}).get(META_VERSION)

    def ok(result):
        return {"jsonrpc": "2.0", "id": mid, "result": {"resultType": "complete", **result} if version else result}

    def err(code, message, data=None):
        error = {"code": code, "message": message, **({"data": data} if data else {})}
        return {"jsonrpc": "2.0", "id": mid, "error": error}

    if version and version not in MODERN_VERSIONS:
        return err(-32022, "Unsupported protocol version", {"supported": MODERN_VERSIONS, "requested": version})
    if method == "initialize":
        asked = params.get("protocolVersion")
        return ok({"protocolVersion": asked if asked in LEGACY_VERSIONS else LEGACY_VERSIONS[0],
                   "capabilities": {"tools": {}}, "serverInfo": SERVER_INFO, "instructions": INSTRUCTIONS})
    if method == "server/discover":
        return ok({"supportedVersions": MODERN_VERSIONS, "capabilities": {"tools": {}},
                   "_meta": {"io.modelcontextprotocol/serverInfo": SERVER_INFO}, "instructions": INSTRUCTIONS,
                   **LIST_CACHE})
    if method == "ping":
        return ok({})
    if method == "tools/list":
        return ok({"tools": tool_list(), **(LIST_CACHE if version else {})})
    if method == "tools/call":
        name, args = params.get("name"), params.get("arguments")
        if args is None:
            args = {}
        if name not in TOOLS:
            return err(-32602, f"unknown tool {name!r}")
        fn, _, props, _ = TOOLS[name]
        arg_errors = validate_tool_args(name, args)
        if arg_errors:
            error_msg = f"Invalid arguments for {name}: {'; '.join(arg_errors)}"
            return ok({"content": [{"type": "text", "text": error_msg}], "isError": True})
        try:
            text = fn(**args)
            return ok({"content": [{"type": "text", "text": text}], "isError": False})
        except Exception as exc:  # a tool failure goes back to the model, never kills the server
            print(f"hive: {name} failed: {exc!r}", file=sys.stderr)
            return ok({"content": [{"type": "text", "text": f"hive error: {exc}"}], "isError": True})
    return err(-32601, f"method not found: {method}")


def serve():
    out = sys.stdout.buffer
    for raw in sys.stdin.buffer:
        line = raw.strip()
        if not line:
            continue
        try:
            msg = json.loads(line)
            resp = handle(msg) if isinstance(msg, dict) else \
                {"jsonrpc": "2.0", "id": None, "error": {"code": -32600, "message": "Invalid Request"}}
        except json.JSONDecodeError:
            resp = {"jsonrpc": "2.0", "id": None, "error": {"code": -32700, "message": "Parse error"}}
        if resp is not None:
            out.write(json.dumps(resp, ensure_ascii=False).encode("utf-8") + b"\n")
            out.flush()


def bench():
    def ms(t0):
        return f"{(time.perf_counter() - t0) * 1000:.1f} ms"
    lines = []
    t0 = time.perf_counter(); index().refresh(force=True); lines.append(f"index refresh: {ms(t0)}")
    t0 = time.perf_counter(); index()._last_refresh = time.monotonic(); index().search("price source deploy")
    lines.append(f"vault search: {ms(t0)}")
    for d in sorted(PROJECTS.iterdir()) if PROJECTS.is_dir() else []:
        if not (d / "graphify-out" / "graph.json").is_file():
            continue
        _GRAPHS.clear()
        t0 = time.perf_counter(); _, g = load_graph(d); cold = ms(t0)
        label = max(g.nodes, key=g.degree, default="")
        t0 = time.perf_counter()
        for _ in range(5):
            g.query(g.nodes[label].get("label", label) if label else "main")
        lines.append(f"{d.name}: {len(g.nodes)} nodes, load {cold}, hub query {(time.perf_counter() - t0) * 200:.1f} ms")
    return "\n".join(lines)


def tool_eval(golden=str(Path(__file__).with_name("eval_golden.json")), k=5):
    """Recall@k and MRR of hive_search over a golden set: [{q, project?, expect: [paths]}].
    Run before and after any ranking change; a hit is any expected path in the top k."""
    cases = json.loads(Path(golden).read_text(encoding="utf-8"))
    k = int(k)
    found, rr, misses = 0, 0.0, []
    for case in cases:
        paths = [h["path"] for h in index().search(case["q"], case.get("project", ""), k)]
        rank = next((i + 1 for i, p in enumerate(paths) if p in case["expect"]), 0)
        found += bool(rank)
        rr += 1 / rank if rank else 0.0
        if not rank:
            misses.append(f"  miss: {case['q']!r} -> {paths[:3]}")
    n = len(cases)
    return "\n".join([f"Recall@{k} {found}/{n} = {found / n:.2f}   MRR {rr / n:.3f}"] + misses)


def main(argv):
    cmd, *rest = argv or ["serve"]
    if cmd == "serve":
        return serve()
    commands = {"search": tool_search, "graph": tool_graph, "context": tool_context, "status": tool_status, "lint": tool_lint, "bench": bench, "eval": tool_eval}
    if cmd not in commands:
        print(__doc__)
        return 2
    sys.stdout.reconfigure(encoding="utf-8")
    print(commands[cmd](*rest))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
