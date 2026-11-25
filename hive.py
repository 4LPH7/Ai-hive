"""Hive: shared memory for Claude Code, Codex and Antigravity over an Obsidian vault
and graphify code graphs. Stdlib only (Python 3.10+), read-only, MCP over stdio.

    python hive.py serve                      MCP stdio server (what agents launch)
    python hive.py search "query" [project]   BM25 search over vault notes
    python hive.py graph <repo> "query"       ranked code-graph neighbourhood
    python hive.py context <project> ["task"] token-budgeted session pack
    python hive.py status                     index and graph freshness
    python hive.py lint                       vault health report (see lint.py)
"""
import hashlib
import json
import math
import os
import re
import sqlite3
import sys
import time

if sys.version_info < (3, 9):
    sys.exit("hive requires Python 3.9+ (Path.is_relative_to)")

from collections import defaultdict
from pathlib import Path

def canonical_path(p):
    """Normalize Windows casing, 8.3 short paths and symlinks/junctions."""
    try:
        resolved = Path(p).resolve()
        return Path(os.path.normcase(os.path.realpath(str(resolved))))
    except (ValueError, OSError):
        return Path(p)

def _strip_inline_comment(s):
    in_single = False
    in_double = False
    escaped = False
    for i, c in enumerate(s):
        if escaped:
            escaped = False
            continue
        if c == "\\" and in_double:
            escaped = True
            continue
        if c == "'" and not in_double:
            in_single = not in_single
        elif c == '"' and not in_single:
            in_double = not in_double
        elif c == '#' and not in_single and not in_double:
            return s[:i]
    return s

def _unquote(s):
    s = s.strip()
    if len(s) >= 2 and ((s.startswith('"') and s.endswith('"')) or (s.startswith("'") and s.endswith("'"))):
        return s[1:-1]
    return s

def load_config(cli_vault=None, cli_projects=None, cli_db=None, cli_ledger=None, cli_config=None):
    """Hierarchical config: CLI flags -> ENV vars -> config.toml -> OS defaults."""
    config_data = {}
    config_file = Path(cli_config) if cli_config else None
    if not config_file:
        appdata = os.environ.get("APPDATA")
        home = os.environ.get("USERPROFILE") or os.environ.get("HOME")
        candidates = []
        if appdata:
            candidates.append(Path(appdata) / "hive" / "config.toml")
        if home:
            candidates.append(Path(home) / ".config" / "hive" / "config.toml")
        xdg = os.environ.get("XDG_CONFIG_HOME")
        if xdg:
            candidates.append(Path(xdg) / "hive" / "config.toml")
        for c in candidates:
            if c.is_file():
                config_file = c
                break
    if config_file and config_file.is_file():
        try:
            prefix = ""
            for line in config_file.read_text(encoding="utf-8").splitlines():
                line = line.strip()
                if not line or line.startswith("#"):
                    continue
                line = _strip_inline_comment(line).strip()
                if not line:
                    continue
                if line.startswith("[") and line.endswith("]"):
                    sec = line[1:-1].strip().lower()
                    prefix = f"{sec}." if sec and sec != "hive" else ""
                    continue
                k, sep, v = line.partition("=")
                if sep:
                    raw_k = k.strip().lower()
                    val = _unquote(v)
                    if prefix:
                        config_data[f"{prefix}{raw_k}"] = val
                    config_data.setdefault(raw_k, val)
        except OSError:
            pass

    vault = cli_vault or os.environ.get("HIVE_VAULT") or config_data.get("vault")
    if not vault:
        if sys.platform == "win32" and Path(r"D:\Valut\OBS").exists():
            vault = r"D:\Valut\OBS"
        else:
            home = os.environ.get("USERPROFILE") or os.environ.get("HOME") or "."
            doc_vault = Path(home) / "Documents" / "Vault"
            vault = str(doc_vault if doc_vault.exists() else Path(home) / "Documents" / "Vault")

    projects = cli_projects or os.environ.get("HIVE_PROJECTS") or config_data.get("projects")
    if not projects:
        if sys.platform == "win32" and Path(r"D:\PROJECTS").exists():
            projects = r"D:\PROJECTS"
        else:
            home = os.environ.get("USERPROFILE") or os.environ.get("HOME") or "."
            projects = str(Path(home) / "Projects")

    db = cli_db or os.environ.get("HIVE_DB") or config_data.get("db")
    if not db:
        if sys.platform == "win32":
            base = os.environ.get("LOCALAPPDATA") or os.environ.get("TEMP") or "."
            db = str(Path(base) / "hive" / "hive.sqlite")
        else:
            xdg_cache = os.environ.get("XDG_CACHE_HOME")
            base = xdg_cache or (Path(os.environ.get("HOME", ".")) / ".cache")
            db = str(Path(base) / "hive" / "hive.sqlite")

    ledger = cli_ledger or os.environ.get("HIVE_LEDGER") or config_data.get("ledger")
    if not ledger:
        if sys.platform == "win32":
            base = os.environ.get("LOCALAPPDATA") or os.environ.get("TEMP") or "."
            ledger = str(Path(base) / "hive" / "ledger.jsonl")
        else:
            xdg_cache = os.environ.get("XDG_CACHE_HOME")
            base = xdg_cache or (Path(os.environ.get("HOME", ".")) / ".cache")
            ledger = str(Path(base) / "hive" / "ledger.jsonl")

    return {
        "vault": Path(vault),
        "projects": Path(projects),
        "db": Path(db),
        "ledger": Path(ledger),
    }


CFG = load_config()
VAULT = CFG["vault"]
PROJECTS = CFG["projects"]
DB_PATH = CFG["db"]
LEDGER_PATH = CFG["ledger"]
SKIP_DIRS = {".git", ".obsidian", ".claude", ".agents", ".trash", "Python", "_attachments", "graph", "node_modules", "templates", "_template", ".claude-flow"}
REFRESH_EVERY_S = 2.0
TYPE_PRIOR = {"project-memory": 1.4, "adr": 1.25, "note": 1.0, "journal": 0.8}
STATUS_PRIOR = {"superseded": 0.3, "archived": 0.4}
SCHEMA_VERSION = 4
PROJECT_BOOST = 2.0  # eval 2026-11-16: 1.5 -> 28/30, 2.0 -> 29/30 (higher gives no further gain)
AUTOCUT_RATIO = 0.30  # keep hits >= 30% of the best; eval 2026-11-23: 0.35 cut a rank-4 answer (29/30), 0.30 -> 30/30, +0.3 hits
SERVER_INFO = {"name": "hive", "version": "1.1.0"}
MODERN_VERSIONS = ["2026-07-28"]
LEGACY_VERSIONS = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"]
ALL_SUPPORTED_VERSIONS = MODERN_VERSIONS + LEGACY_VERSIONS
META_VERSION = "io.modelcontextprotocol/protocolVersion"
INSTRUCTIONS = ("Shared memory for all coding agents (Obsidian vault + graphify graphs). Read-only. "
                "Call hive_context at the start of a task, hive_search before asking the user for context, "
                "hive_graph for code structure. Write memory as Markdown files following the vault's AGENTS.md.")


READ_ONLY = {"readOnlyHint": True, "destructiveHint": False, "idempotentHint": True, "openWorldHint": False}
LIST_CACHE = {"ttlMs": 3600000, "cacheScope": "public"}  # required on modern tools/list + server/discover
DEFAULT_SECTION_WEIGHTS = {"memory": 0.35, "playbook": 0.15, "task_notes": 0.20, "traces": 0.10, "code_graph": 0.25}
FORWARD_WEIGHT = float(os.environ.get("HIVE_PPR_FWD", 0.7))
REVERSE_WEIGHT = float(os.environ.get("HIVE_PPR_REV", 0.3))


def est_tokens(text):
    """Calibrated content-aware token estimator: code (3.4) vs prose (4.2) + 8% safety margin."""
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
    toks = (code_chars / 3.4 + prose_chars / 4.2) * 1.08
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
        if self.db.execute("PRAGMA user_version").fetchone()[0] != 4:
            # derived index: a schema change just rebuilds it from the vault
            self.db.executescript("DROP TABLE IF EXISTS notes; DROP TABLE IF EXISTS notes_fts;")
            self.db.execute("PRAGMA user_version=4")
        self.db.executescript("""
            CREATE TABLE IF NOT EXISTS notes(path TEXT PRIMARY KEY, mtime REAL, size INTEGER, sha256 TEXT, project TEXT,
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
        """Re-index notes whose mtime or size changed; drop deleted ones. Returns changed count."""
        now = time.monotonic()
        if not force and now - self._last_refresh < REFRESH_EVERY_S:
            return 0
        self._last_refresh = now
        known = {r[0]: (r[1], r[2], r[3]) for r in self.db.execute("SELECT path, mtime, size, sha256 FROM notes")}
        seen, changed = set(), 0
        with self.db:
            for entry in self._walk():
                rel = Path(entry.path).relative_to(self.vault).as_posix()
                seen.add(rel)
                st = entry.stat()
                mtime, size = st.st_mtime, st.st_size
                if not force and rel in known and known[rel][0] == mtime and known[rel][1] == size:
                    continue
                try:
                    text = Path(entry.path).read_text(encoding="utf-8-sig", errors="replace")
                except OSError as exc:
                    print(f"hive: skip {rel}: {exc}", file=sys.stderr)
                    continue
                h = hashlib.sha256(text.encode("utf-8")).hexdigest()
                if rel in known and known[rel][2] == h:
                    self.db.execute("UPDATE notes SET mtime=?, size=? WHERE path=?", (mtime, size, rel))
                    continue
                meta, title, body = parse_note(text)
                title = title or Path(rel).stem
                project = meta.get("project") or (rel.split("/")[1] if rel.startswith("projects/") else "")
                self.db.execute("DELETE FROM notes_fts WHERE path=?", (rel,))
                self.db.execute("INSERT OR REPLACE INTO notes VALUES(?,?,?,?,?,?,?,?,?,?,?)",
                                (rel, mtime, size, h, project, meta.get("type", "note"), meta.get("status", "active"),
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
        if scope == "project" and project:
            rows = self.db.execute("""
                SELECT f.path, n.title, n.project, n.type, n.status, n.updated, n.agent, n.source,
                       -bm25(notes_fts, 0.0, 5.0, 2.0, 1.0) AS s,
                       snippet(notes_fts, 3, '[', ']', '...', 18)
                FROM notes_fts f JOIN notes n ON n.path = f.path
                WHERE notes_fts MATCH ? AND n.project IN (?, '', 'global')
                ORDER BY s DESC LIMIT ?""", (match, project, limit * 4)).fetchall()
        else:
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
            target = canonical_path(self.vault / rel)
            base = canonical_path(self.vault)
            if not target.is_relative_to(base):
                return ""
            return target.read_text(encoding="utf-8-sig", errors="replace") if target.is_file() else ""
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


def get_config_dir():
    if sys.platform == "win32":
        appdata = os.environ.get("APPDATA")
        if appdata:
            return Path(appdata) / "hive"
    xdg = os.environ.get("XDG_CONFIG_HOME")
    if xdg:
        return Path(xdg) / "hive"
    return Path.home() / ".config" / "hive"


def get_registered_roots():
    """Retrieve allowed repository roots from roots.toml, HIVE_ALLOWED_REPOS, and PROJECTS."""
    roots = []
    cfg_file = get_config_dir() / "roots.toml"
    if cfg_file.is_file():
        try:
            content = cfg_file.read_text(encoding="utf-8")
            for line in content.splitlines():
                line = line.strip()
                if line.startswith(('"', "'")):
                    p = line.strip('"\' ,')
                    if p:
                        roots.append(canonical_path(p))
                elif "=" in line and ("roots" in line or "path" in line):
                    _, _, val = line.partition("=")
                    val = val.strip().strip("[]")
                    for part in val.split(","):
                        p = part.strip().strip('"\' ')
                        if p:
                            roots.append(canonical_path(p))
        except OSError:
            pass
    env_roots = os.environ.get("HIVE_ALLOWED_REPOS")
    if env_roots:
        for p in env_roots.split(os.pathsep):
            if p.strip():
                roots.append(canonical_path(p.strip()))
    roots.append(canonical_path(PROJECTS))
    seen = set()
    result = []
    for r in roots:
        s = str(r)
        if s not in seen and r.is_dir():
            seen.add(s)
            result.append(r)
    return result


def repo_dir(repo):
    """Accept a repo folder path under registered roots, a folder name under registered roots, or a vault slug."""
    roots = get_registered_roots()
    if not roots:
        return None
    try:
        cand = canonical_path(repo)
        if cand.is_dir():
            for root in roots:
                if cand != root and cand.is_relative_to(root) and (cand / "graphify-out").is_dir():
                    return cand
    except (ValueError, OSError):
        pass
    want = slugify(Path(repo).name or str(repo))
    if not re.fullmatch(r"[a-z0-9._-]+", want) or want in {".", ".."}:
        return None
    for root in roots:
        try:
            for d in root.iterdir():
                if d.is_dir() and slugify(d.name) == want:
                    can = canonical_path(d)
                    if can != root and can.is_relative_to(root):
                        return can
        except OSError:
            continue
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


def tool_search(query, project="", scope=None, limit=8):
    if scope is None:
        scope = "project" if project else "all"
    if scope not in ("project", "all"):
        raise ValueError(f"invalid scope {scope!r}; must be 'project' or 'all'")
    t0 = time.time()
    slug = validate_slug(project) if project else ""
    hits = index().search(query, slug, int(limit), scope=scope)
    duration_ms = (time.time() - t0) * 1000.0
    try:
        import ledger
        hit_paths = [h["path"] for h in hits]
        ledger.record(LEDGER_PATH, event="query", project=slug, query=query, hits=hit_paths, duration_ms=duration_ms)
    except Exception:
        pass
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


def traces_section(db, project, task):
    """TRACE-04: live traces for hive_context; a broken trace table never blocks context."""
    try:
        import traces
        return traces.context_section(traces.active(db, project, int(time.time())), task)
    except (sqlite3.Error, ValueError) as exc:
        print(f"hive: traces skipped: {exc}", file=sys.stderr)
        return ""


def tool_context(project, task="", budget=1500):
    t0 = time.time()
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
        header = f"> [!NOTE] Untrusted vault note: path=\"projects/{project}/_memory.md\", agent=\"{agent}\", source=\"{source}\", age=\"{age}\""
        quoted_lines = [f"> {line}" for line in body.strip().splitlines()]
        framed_body = header + "\n" + "\n".join(quoted_lines)
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
        "traces": traces_section(idx.db, project, task),
        "code_graph": code_graph_section,
    }
    try:
        import ledger
        hit_paths = [h["path"] for h in hits] if task and hits else []
        bullet_ids = [b["id"] for _, _, _, b in rows] if rows else []
        duration_ms = (time.time() - t0) * 1000.0
        ledger.record(LEDGER_PATH, event="context", project=project, query=task, hits=hit_paths, injected_bullets=bullet_ids, duration_ms=duration_ms)
    except Exception:
        pass
    return pack(sections, budget=int(budget), weights=DEFAULT_SECTION_WEIGHTS)


def tool_skill(task, auto_install=True, scope="all"):
    """Auto-select best skill for task, or discover and install from remote registry (find-skills)."""
    import skill
    import ledger

    t0 = time.time()
    task_str = str(task).strip()
    if not task_str:
        return "task description cannot be empty"

    auto_install_bool = str(auto_install).lower() in ("true", "1", "yes") if isinstance(auto_install, (str, int)) else bool(auto_install)
    scope_str = str(scope or "all").lower()

    # 1. Match local skills
    match_res = skill.match_skill(task_str, threshold=0.65)
    duration_ms = (time.time() - t0) * 1000.0

    if match_res.is_match:
        ctx = skill.load_skill_context(match_res.skill)
        try:
            ledger.record(LEDGER_PATH, event="skill", project=match_res.skill.name, query=task_str, hits=[match_res.skill.path], duration_ms=duration_ms)
        except Exception:
            pass
        return ctx

    # 2. Local miss -> remote discovery
    if scope_str == "all":
        candidates = skill.find_remote_skills(task_str)
        if candidates and auto_install_bool:
            top = candidates[0]
            res = skill.install_skill(top.package_name)
            if res.get("status") == "installed":
                ctx = skill.load_skill_context(res["installed_path"])
                try:
                    ledger.record(LEDGER_PATH, event="skill", project=top.package_name, query=task_str, hits=[res["installed_path"]], duration_ms=(time.time() - t0) * 1000.0)
                except Exception:
                    pass
                return f"> [!NOTE] Automatically installed skill `{top.package_name}` from registry\n\n" + ctx
            elif res.get("status") == "dry_run":
                return f"> [!NOTE] Dry-run install would stage `{top.package_name}` to {res.get('installed_path')}"
            else:
                return f"> [!WARNING] Auto-install failed for {top.package_name}: {res.get('error', 'unknown error')}"

        if candidates and not auto_install_bool:
            try:
                ledger.record(LEDGER_PATH, event="skill", project="", query=task_str, hits=[], duration_ms=(time.time() - t0) * 1000.0)
            except Exception:
                pass
            lines = [f"No matching local skill for task: {task_str!r}. Top remote candidates:"]
            for c in candidates[:3]:
                lines.append(f"- `{c.package_name}` ({c.stars} stars): {c.description}")
            lines.append(f"\nTo install: npx skills add {candidates[0].package_name}")
            return "\n".join(lines)

    try:
        ledger.record(LEDGER_PATH, event="skill", project="", query=task_str, hits=[], duration_ms=(time.time() - t0) * 1000.0)
    except Exception:
        pass
    return f"no local or remote skills found matching task: {task_str!r}"


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
                     {"project": ("string", "Project slug: folder name under projects root, lowercased, spaces to hyphens"),
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
                   {"repo": ("string", "Repo folder name or slug under projects root, or a path"),
                    "query": ("string", "Symbol names or keywords"), "limit": ("integer", "Max symbols (default 15)")},
                   ["repo", "query"]),
    "hive_status": (tool_status, "Index size per project and freshness of each repo's code graph.", {}, []),
    "hive_lint": (tool_lint,
                  "Vault health against AGENTS.md rules: frontmatter, line caps, broken links, stale memory, "
                  "superseded ADRs without successor, ambiguous names, orphans. Run before session end after writing notes.",
                  {}, []),
    "hive_skill": (tool_skill,
                   "Auto-select best skill for task, or discover and install from remote registry (find-skills).",
                   {"task": ("string", "Task description or user prompt (required)"),
                    "auto_install": ("boolean", "Whether to auto-install missing skills from registry (default: true)"),
                    "scope": ("string", "Search scope: 'local' (installed only) or 'all' (local + remote fallback, default: 'all')")},
                   ["task"]),
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
    """Validate arguments against tool definition. Return list of error dicts with messages."""
    fn, _, props, req = TOOLS[name]
    if not isinstance(args, dict):
        return [{"field": "_root", "expected": "object", "got": type(args).__name__,
                 "message": f"arguments must be a JSON object, got {type(args).__name__}"}]
    errors = []
    for r in req:
        if r not in args or args[r] is None:
            errors.append({"field": r, "expected": "provided", "got": "missing",
                           "message": f"missing required argument {r!r}"})
    for k in args:
        if k not in props:
            errors.append({"field": k, "expected": "none", "got": "unexpected",
                           "message": f"unexpected argument {k!r}; allowed: {sorted(props.keys())}"})
        else:
            expected_type, _ = props[k]
            val = args[k]
            if val is not None:
                if expected_type == "string" and not isinstance(val, str):
                    errors.append({"field": k, "expected": "string", "got": type(val).__name__,
                                   "message": f"argument {k!r} must be string, got {type(val).__name__}"})
                elif expected_type == "integer" and (isinstance(val, bool) or not isinstance(val, int)):
                    errors.append({"field": k, "expected": "integer", "got": type(val).__name__,
                                   "message": f"argument {k!r} must be integer, got {type(val).__name__}"})
                elif expected_type == "boolean" and not isinstance(val, bool):
                    errors.append({"field": k, "expected": "boolean", "got": type(val).__name__,
                                   "message": f"argument {k!r} must be boolean, got {type(val).__name__}"})
    return errors


def handle(msg):
    """One JSON-RPC message in, one response dict out (None for notifications).
    Dual-era: legacy `initialize` sessions and modern per-request `_meta` versions."""
    mid, method, params = msg.get("id"), msg.get("method", ""), msg.get("params") or {}
    client = (params.get("clientInfo") or (params.get("_meta") or {}).get("io.modelcontextprotocol/clientInfo") or {})
    if isinstance(client, dict) and client.get("name"):  # PROV-01: the client names itself
        try:
            import ledger
            ledger.set_context(agent=ledger.agent_from_hint(client["name"]))
        except Exception:
            pass
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
        chosen = asked if asked in ALL_SUPPORTED_VERSIONS else LEGACY_VERSIONS[0]
        return ok({"protocolVersion": chosen,
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
            error_msg = f"Invalid arguments for {name}: {'; '.join(e['message'] for e in arg_errors)}"
            return ok({"content": [{"type": "text", "text": error_msg}], "isError": True, "errorDetails": arg_errors})
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


def tool_trace(*args):
    """TRACE-03 CLI fallback: `trace add <project> <kind> <target> <summary>` | `trace list <project>`.
    Writes only the disposable hive.sqlite, never the vault."""
    import sqlite3
    import time
    import ledger
    import traces
    if len(args) >= 5 and args[0] == "add":
        project, kind, target, summary = args[1], args[2].upper(), args[3], " ".join(args[4:])
    elif len(args) == 2 and args[0] == "list":
        project = args[1]
    else:
        raise SystemExit("usage: hive trace add <project> <kind> <target> <summary> | hive trace list <project>")
    Path(DB_PATH).parent.mkdir(parents=True, exist_ok=True)
    db = sqlite3.connect(str(DB_PATH), timeout=10)
    try:
        now = int(time.time())
        if args[0] == "add":
            try:
                tid = traces.add(db, project, kind, target, summary, ledger.resolve_agent(), ledger.resolve_session(), now)
            except ValueError as exc:
                raise SystemExit(f"trace: {exc}")
            return f"trace: {kind} #{tid} on {target} ({project})"
        rows = traces.active(db, project, now)
        if not rows:
            return f"trace: no active traces for {project}"
        return "\n".join(f"{t['kind']:<10} {w:.2f}  {t['target']}  {t['summary']} "
                         f"({t['agent']}, {max(0, now - t['created_at']) // 3600}h ago)" for t, w in rows)
    finally:
        db.close()


def tool_ledger(*args, tail=None, project=None):
    import ledger
    cfg = load_config()
    if args and args[0] == "score":
        import argparse
        parser = argparse.ArgumentParser(prog="hive ledger score")
        parser.add_argument("--session")
        parser.add_argument("--exit-code", type=int, default=0)
        parser.add_argument("--project")
        parser.add_argument("--repo")
        parsed = parser.parse_args(args[1:])
        res = ledger.score_session(
            session_id=parsed.session,
            repo_dir=parsed.repo,
            test_exit_code=parsed.exit_code,
            ledger_path=cfg["ledger"],
            vault_path=cfg["vault"],
            project=parsed.project,
        )
        if res.get("scored"):
            return (f"ledger score: session={res['session_id']} reward={res['reward']} "
                    f"(exit_code={res['test_exit_code']}) bullets={len(res['bullets'])} "
                    f"skills={len(res['skills'])} changes={res['report'].get('tagged', 0)}")
        return f"ledger score: session not scored (session={res.get('session_id')})"

    t = int(tail) if tail is not None else None
    p = project
    want_coverage = "--coverage" in args
    args = tuple(a for a in args if a != "--coverage")
    if args:
        i = 0
        while i < len(args):
            if args[i] == "--tail" and i + 1 < len(args):
                t = int(args[i + 1])
                i += 2
            elif args[i] == "--project" and i + 1 < len(args):
                p = args[i + 1]
                i += 2
            else:
                if t is None and args[i].isdigit():
                    t = int(args[i])
                i += 1
    entries = ledger.read_ledger(cfg["ledger"], tail=t, project=p)
    if want_coverage:
        c = ledger.coverage(entries)
        pct = (lambda n: n * 100.0 / c["total"]) if c["total"] else (lambda n: 0.0)
        return (f"ledger coverage: {c['total']} events | session_id {pct(c['with_session']):.1f}% "
                f"({c['fallback_session']} process-scoped) | agent {pct(c['with_agent']):.1f}%")
    if not entries:
        return f"ledger: no entries in {cfg['ledger']}"
    lines = []
    for e in entries:
        lines.append(f"{e.get('ts')} [{e.get('agent')}] {e.get('event')} project={e.get('project')} "
                     f"hits={len(e.get('hits', []))} bullets={len(e.get('injected_bullets', []))} "
                     f"({e.get('duration_ms', 0):.1f}ms) - query={e.get('query')!r}")
    return "\n".join(lines)


def cmd_pre_commit(*args):
    native = find_native_binary()
    if native:
        import subprocess
        res = subprocess.call([native, "pre-commit", *args])
        if res != 0:
            sys.exit(res)
        return "All pre-commit checks passed cleanly."
    print("=== [1/2] Running hive lint ===")
    rep = tool_lint()
    print(rep)
    if "ERROR" in rep:
        sys.exit(1)
    print("=== [2/2] Running Python tests ===")
    import subprocess
    res = subprocess.call([sys.executable, "-m", "unittest", "discover"])
    if res != 0:
        sys.exit(res)
    return "All pre-commit checks passed cleanly."


def tool_mine(*args):
    import argparse
    import miner
    cfg = load_config()
    parser = argparse.ArgumentParser(prog="hive mine")
    parser.add_argument("--min-count", type=int, default=2)
    parser.add_argument("--dry-run", action="store_true")
    parsed = parser.parse_args(args)
    res = miner.mine_gaps(
        ledger_path=cfg["ledger"],
        vault_path=cfg["vault"],
        min_count=parsed.min_count,
        dry_run=parsed.dry_run,
    )
    lines = [
        f"mine: {res['total_gaps']} gaps analyzed, {res['total_clusters']} clusters, "
        f"{res['proposals_generated']} proposals generated (dry_run={res['dry_run']})"
    ]
    for p in res["proposals"]:
        lines.append(f"  proposal: [{p['hash']}] \"{p['representative']}\" (count={p['count']}) -> {p['file']}")
    return "\n".join(lines)


def tool_curate(*args):
    import argparse
    import curate
    parser = argparse.ArgumentParser(prog="hive curate")
    parser.add_argument("--project")
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--config")
    parsed = parser.parse_args(args)
    cfg = load_config(cli_config=parsed.config)
    res = curate.run_curate(
        ledger_path=cfg["ledger"],
        vault_path=cfg["vault"],
        project=parsed.project,
        dry_run=parsed.dry_run,
        config_path=parsed.config,
    )
    if res.get("skipped"):
        return f"curate: {res.get('model')} ({res.get('reason')})"
    lines = [
        f"curate: model={res['model']} merges={res['merges_count']} "
        f"contradictions={res['contradictions_count']} drifts={res['adr_drifts_count']} "
        f"aliases={res['alias_proposals_count']} (dry_run={res['dry_run']})"
    ]
    if res.get("proposal_file"):
        lines.append(f"  proposal written to: {res['proposal_file']}")
    return "\n".join(lines)


def find_native_binary():
    """Find native compiled hive binary if available."""
    if os.environ.get("HIVE_FORCE_PYTHON") == "1" or os.environ.get("_HIVE_RECURSION_GUARD") == "1":
        return None
    env_bin = os.environ.get("HIVE_BIN")
    if env_bin and os.path.isfile(env_bin):
        return env_bin

    bin_name = "hive.exe" if sys.platform == "win32" else "hive"
    release_bin = Path(__file__).resolve().parent / "target" / "release" / bin_name
    if release_bin.is_file():
        return str(release_bin)

    local_bin = Path(__file__).resolve().parent / bin_name
    if local_bin.is_file():
        return str(local_bin)

    import shutil
    on_path = shutil.which("hive")
    if on_path and os.path.isfile(on_path):
        resolved = Path(on_path).resolve()
        scripts_dir = Path(sys.prefix) / ("Scripts" if sys.platform == "win32" else "bin")
        try:
            if resolved == Path(__file__).resolve() or resolved.is_relative_to(scripts_dir.resolve()):
                return None
        except (ValueError, AttributeError):
            if str(scripts_dir.resolve()).lower() in str(resolved).lower():
                return None
        return str(resolved)

    return None


def main(argv=None):
    if argv is None:
        argv = sys.argv[1:]
    native = find_native_binary()
    if native and os.environ.get("_HIVE_RECURSION_GUARD") != "1":
        import subprocess
        try:
            env = dict(os.environ, _HIVE_RECURSION_GUARD="1")
            return subprocess.call([native, *argv], env=env)
        except Exception:
            pass
    cmd, *rest = argv or ["serve"]
    if cmd in ("--version", "-V", "version"):
        print("hive 2.5.0")
        return 0
    if cmd == "serve":
        return serve()
    commands = {
        "search": tool_search, "graph": tool_graph, "context": tool_context,
        "status": tool_status, "lint": tool_lint, "bench": bench, "eval": tool_eval,
        "ledger": tool_ledger, "skill": tool_skill, "pre-commit": cmd_pre_commit,
        "mine": tool_mine, "curate": tool_curate, "trace": tool_trace,
    }
    if cmd not in commands:
        print(__doc__)
        return 2
    sys.stdout.reconfigure(encoding="utf-8")
    print(commands[cmd](*rest))
    return 0


if __name__ == "__main__":
    sys.exit(main())
