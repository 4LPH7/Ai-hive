"""Gap Miner (`hive mine`): Scans ledger.jsonl for zero-hit searches and skill misses,
drafting structured Markdown proposals into vault inbox/.
"""
import hashlib
import json
import os
import re
from datetime import datetime
from pathlib import Path

STOPWORDS = {
    "a", "about", "above", "after", "again", "against", "all", "am", "an", "and", "any", "are",
    "as", "at", "be", "because", "been", "before", "being", "below", "between", "both", "but",
    "by", "could", "did", "do", "does", "doing", "down", "during", "each", "few", "for", "from",
    "further", "had", "has", "have", "having", "he", "her", "here", "hers", "herself", "him",
    "himself", "his", "how", "i", "if", "in", "into", "is", "it", "its", "itself", "me", "more",
    "most", "my", "myself", "no", "nor", "not", "of", "off", "on", "once", "only", "or", "other",
    "ought", "our", "ours", "ourselves", "out", "over", "own", "same", "she", "should", "so",
    "some", "such", "than", "that", "the", "their", "theirs", "them", "themselves", "then",
    "there", "these", "they", "this", "those", "through", "to", "too", "under", "until", "up",
    "very", "was", "we", "were", "what", "when", "where", "which", "while", "who", "whom", "why",
    "with", "would", "you", "your", "yours", "yourself", "yourselves",
}


def tokenize(text):
    words = re.findall(r"[a-z0-9_-]+", text.lower())
    return {w for w in words if len(w) >= 2 and w not in STOPWORDS}


def jaccard(s1, s2):
    if not s1 or not s2:
        return 0.0
    return len(s1 & s2) / len(s1 | s2)


class GapCluster:
    def __init__(self, query, entry):
        self.tokens = tokenize(query)
        self.queries = [query]
        self.entries = [entry]
        self.representative = query
        self.first_seen = entry.get("ts", "")
        self.last_seen = entry.get("ts", "")

    def matches(self, other_tokens, threshold=0.35):
        if not self.tokens or not other_tokens:
            return False
        # Overlap ratio or Jaccard
        overlap = len(self.tokens & other_tokens)
        if overlap >= 2:
            return True
        return jaccard(self.tokens, other_tokens) >= threshold

    def add(self, query, entry):
        new_tok = tokenize(query)
        self.tokens.update(new_tok)
        self.queries.append(query)
        self.entries.append(entry)
        ts = entry.get("ts", "")
        if ts:
            if not self.first_seen or ts < self.first_seen:
                self.first_seen = ts
            if not self.last_seen or ts > self.last_seen:
                self.last_seen = ts
        if len(query) > len(self.representative):
            self.representative = query

    @property
    def hash_id(self):
        toks = sorted(self.tokens)
        raw = ":".join(toks) if toks else self.representative
        return hashlib.sha256(raw.encode("utf-8")).hexdigest()[:8]

    def render_markdown(self):
        slug = re.sub(r"[^a-z0-9]+", "-", self.representative.lower()).strip("-")
        lines = [
            "---",
            "type: proposal",
            "status: inbox",
            "source: hive-miner",
            f"cluster: {self.representative}",
            f"first_seen: {self.first_seen}",
            f"last_seen: {self.last_seen}",
            f"count: {len(self.queries)}",
            "---",
            f"# Gap Proposal: {self.representative}",
            "",
            "> Automatically mined from ledger zero-hit queries and misses.",
            "> Review and convert to a permanent note, alias, or skill.",
            "",
            "## Queries",
        ]
        for e, q in zip(self.entries, self.queries):
            lines.append(f"- \"{q}\" (agent: {e.get('agent', 'unknown')}, ts: {e.get('ts', '')})")

        lines.extend([
            "",
            "## Proposed Actions",
            f"- [ ] Create note `notes/{slug}.md` covering this topic",
            "- [ ] Add aliases to existing note",
            "- [ ] Install or author skill for this capability",
            "",
        ])
        return "\n".join(lines)


def scan_ledger_gaps(ledger_path, project=None):
    """Scan ledger for zero-hit search queries and skill misses."""
    import ledger
    entries = ledger.read_ledger(ledger_path, project=project)
    gaps = []
    for e in entries:
        if project and e.get("project") and e.get("project") != project:
            continue
        event = e.get("event")
        hits = e.get("hits") or []
        query = (e.get("query") or "").strip()
        if not query:
            continue
        if event == "query" and len(hits) == 0:
            gaps.append((query, e))
        elif event == "skill" and len(hits) == 0:
            gaps.append((query, e))
    return gaps


def cluster_gaps(gaps):
    """Cluster raw gap entries into grouped recurring topics."""
    clusters = []
    for query, entry in gaps:
        toks = tokenize(query)
        if not toks:
            continue
        matched_cluster = None
        for c in clusters:
            if c.matches(toks):
                matched_cluster = c
                break
        if matched_cluster:
            matched_cluster.add(query, entry)
        else:
            clusters.append(GapCluster(query, entry))
    return clusters


def mine_gaps(ledger_path=None, vault_path=None, min_count=2, dry_run=False):
    """Mine recurring gaps from ledger and draft proposal notes into vault/inbox/."""
    import hive
    cfg = hive.load_config()
    ledger_p = Path(ledger_path or cfg.get("ledger", Path(os.environ.get("HIVE_LEDGER", "ledger.jsonl"))))
    vault_p = Path(vault_path or cfg.get("vault", Path(os.environ.get("HIVE_VAULT", r"D:\Valut\OBS"))))

    gaps = scan_ledger_gaps(ledger_p)
    clusters = cluster_gaps(gaps)

    qualified = [c for c in clusters if len(c.queries) >= min_count]
    proposals = []

    inbox_dir = vault_p / "inbox"
    if not dry_run and qualified:
        inbox_dir.mkdir(parents=True, exist_ok=True)

    for c in qualified:
        h = c.hash_id
        content = c.render_markdown()
        target_file = inbox_dir / f"gap-{h}.md"
        proposals.append({
            "hash": h,
            "representative": c.representative,
            "count": len(c.queries),
            "file": str(target_file),
            "content": content,
        })
        if not dry_run:
            target_file.write_text(content, encoding="utf-8")

    return {
        "total_gaps": len(gaps),
        "total_clusters": len(clusters),
        "proposals_generated": len(proposals),
        "dry_run": dry_run,
        "proposals": proposals,
    }
