"""RATCHET-02 (v2.8.0): turn canonical playbook lessons that ban something into deterministic lint rules.

`hive ratchet [scope]` drafts a proposal into `inbox/ratchet-<rule-id>.md` for every canonical bullet that bans a
`backticked` token. A human accepts one with `hive ratchet accept <rule-id> [--target vault|repo] [--glob G]`,
which appends the rule to `_meta/ratchet_rules.jsonl`. From then on:
- target `vault`: `hive lint` reports the token in vault notes matching the glob (ERROR ratchet-<id>);
- target `repo`: `hive pre-commit` blocks staged repo files matching the glob that contain the token;
- the source bullet stops being injected: the zero-token rule replaces the prompt rule.
Python mirror of src/ratchet.rs. Patterns are literal substrings, never regexes (no ReDoS, no surprises).
"""
import datetime
import json
import re
import subprocess
from pathlib import Path

RULES_FILE = Path("_meta") / "ratchet_rules.jsonl"
TARGETS = ("vault", "repo")
RULE_KEYS = ("id", "scope", "bullet", "pattern", "target", "glob", "message")
BAN_RE = re.compile(r"\b(never|don't|do not|avoid|must not|banned|forbidden|blocks?)\b", re.I)
TICK_RE = re.compile(r"`([^`]{3,80})`")
EXT_RE = re.compile(r"\.(rs|py|md|toml|json|jsonl|ps1|sh|ts|js)\b")
# vault files that legitimately quote banned tokens: the lessons themselves, proposals, the rule store
EXEMPT = ("_meta/playbook.md", "inbox/ratchet-")
EXEMPT_RE = re.compile(r"^projects/[^/]+/playbook\.md$")


def rules_path(vault):
    return Path(vault) / RULES_FILE


def load_rules(vault):
    out = []
    try:
        lines = rules_path(vault).read_text(encoding="utf-8").splitlines()
    except OSError:
        return out
    for line in lines:
        try:
            r = json.loads(line)
        except json.JSONDecodeError:
            continue
        if isinstance(r, dict) and r.get("pattern") and r.get("target") in TARGETS and r.get("id"):
            out.append({k: str(r.get(k, "")) for k in RULE_KEYS})
    return out


def _glob_re(glob):
    return re.compile("".join(".*" if c == "*" else "." if c == "?" else re.escape(c) for c in glob))


def glob_match(path, glob):
    """`*` matches any run of characters (crossing `/`), `?` one character, nothing else is special; a leading
    `**/` also matches at the top level. Same semantics as src/ratchet.rs."""
    glob = glob or "**/*"
    return bool(_glob_re(glob).fullmatch(path) or (glob.startswith("**/") and _glob_re(glob[3:]).fullmatch(path)))


def retired(rules, scope, bullet_id):
    """A bullet whose rule shipped is no longer injected."""
    return any(r["scope"] == scope and r["bullet"] == bullet_id for r in rules)


def detect(scope, bullet):
    """A rule draft for a bullet that bans a backticked token, else None."""
    text = bullet["content"]
    ban = BAN_RE.search(text)
    tick = TICK_RE.search(text, ban.start()) if ban else None
    if not tick:
        return None
    ext = EXT_RE.search(tick.group(1)) or EXT_RE.search(text)
    glob = f"**/*.{ext.group(1)}" if ext else "**/*"
    target = "vault" if ext and ext.group(1) == "md" else "repo"
    return {"id": f"rat-{scope}-{bullet['id']}", "scope": scope, "bullet": bullet["id"], "pattern": tick.group(1),
            "target": target, "glob": glob, "message": text[:200]}


def propose(vault, scopes, agent, session, today=None):
    """Write one inbox proposal per new candidate; returns the written paths (existing ones are kept)."""
    import ace
    import quorum
    today = today or datetime.date.today().isoformat()
    accepted = {r["id"] for r in load_rules(vault)}
    written = []
    for scope in scopes:
        meta = quorum.load(quorum.meta_path(ace.scope_path(scope)))
        for bullets in ace.load(scope).values():
            for b in bullets:
                rule = detect(scope, b) if quorum.state_of(meta, b["id"]) == "canonical" else None
                if not rule or rule["id"] in accepted:
                    continue
                path = Path(vault) / "inbox" / f"ratchet-{rule['id']}.md"
                if path.exists():
                    continue
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(render_proposal(rule, agent, session, today), encoding="utf-8", newline="\n")
                written.append(path)
    return written


def render_proposal(rule, agent, session, today):
    return (f"---\ntype: note\nproject: {rule['scope']}\nagent: {agent}\nstatus: active\nupdated: {today}\n"
            f"source: session {session}\n---\n# Ratchet proposal: {rule['bullet']}\n\n"
            f"Canonical lesson ({rule['scope']}): {rule['message']}\n\n"
            f"Proposed rule: `{rule['target']}` files matching `{rule['glob']}` must not contain the literal "
            f"text below.\n\n```json\n{json.dumps(rule, ensure_ascii=False)}\n```\n\n"
            f"Accept (human): `hive ratchet accept {rule['id']}` (optional `--target vault|repo`, "
            f"`--glob <glob>`). The lesson then stops being injected.\n")


def accept(vault, rule_id, target=None, glob=None):
    """Append the proposed rule (optionally retargeted) to the rule store; returns the rule."""
    path = Path(vault) / "inbox" / f"ratchet-{rule_id}.md"
    try:
        text = path.read_text(encoding="utf-8")
    except (FileNotFoundError, OSError):
        raise ValueError(f"ratchet: no proposal inbox/ratchet-{rule_id}.md; run `hive ratchet` first") from None
    m = re.search(r"```json\n(.+?)\n```", text, re.S)
    try:
        rule = json.loads(m.group(1)) if m else None
    except json.JSONDecodeError:
        rule = None
    if not isinstance(rule, dict) or rule.get("id") != rule_id:
        raise ValueError(f"ratchet: proposal {rule_id} has no matching rule block")
    rule = {k: str(rule.get(k, "")) for k in RULE_KEYS}
    if target:
        rule["target"] = target
    if glob:
        rule["glob"] = glob
    if rule["target"] not in TARGETS or not rule["pattern"].strip():
        raise ValueError(f"ratchet: target must be one of {TARGETS} and the pattern non-empty")
    if any(r["id"] == rule_id for r in load_rules(vault)):
        raise ValueError(f"ratchet: rule {rule_id} is already accepted")
    store = rules_path(vault)
    store.parent.mkdir(parents=True, exist_ok=True)
    with open(store, "a", encoding="utf-8", newline="\n") as fh:
        fh.write(json.dumps(rule, ensure_ascii=False, separators=(",", ":")) + "\n")
    return rule


def check_text(rules, target, rel, text):
    """[(rule, line_no)] for every rule of this target whose glob matches rel and whose pattern is in text."""
    if target == "vault" and (rel.startswith(EXEMPT) or EXEMPT_RE.match(rel)):
        return []
    hits = []
    for r in rules:
        if r["target"] != target or not glob_match(rel, r["glob"]) or r["pattern"] not in text:
            continue
        hits.append((r, text[:text.index(r["pattern"])].count("\n") + 1))
    return hits


def staged_violations(vault, repo="."):
    """[(path, line, rule)] for staged files (added/copied/modified) that break a `repo` rule."""
    rules = [r for r in load_rules(vault) if r["target"] == "repo"]
    if not rules:
        return []
    names = subprocess.run(["git", "diff", "--cached", "--name-only", "--diff-filter=ACM"], cwd=repo,
                           capture_output=True, text=True, check=False).stdout.splitlines()
    out = []
    for name in names:
        if not any(glob_match(name, r["glob"]) for r in rules):
            continue
        blob = subprocess.run(["git", "show", f":{name}"], cwd=repo, capture_output=True, check=False).stdout
        out += [(name, line, r) for r, line in check_text(rules, "repo", name, blob.decode("utf-8", "replace"))]
    return out


# ---------------------------------------------------------------- RATCHET-03 release
# A canonical lesson injected into >= RELEASE_SESSIONS distinct real sessions that never earned a helpful tag (and
# has no shipped rule) costs tokens every session for no measured benefit: propose retiring it. A proposal only;
# the human applies `{"remove": [id]}` with `hive ace apply`. Ids are matched per scope from `context` ledger
# events (global: every project's events; a project: its own), so a project id that shadows a global id can
# inflate a count. That only makes a proposal, never a removal.
RELEASE_SESSIONS = 20


def injection_sessions(entries, scope):
    """{bullet_id: {session ids}} from `context` ledger events; anonymous per-process ids never count."""
    out = {}
    for e in entries:
        if e.get("event") != "context" or (scope != "global" and e.get("project") != scope):
            continue
        session = str(e.get("session_id") or "")
        if not session or session.startswith("proc-"):
            continue
        for b in e.get("injected_bullets") or []:
            out.setdefault(str(b), set()).add(session)
    return out


def release_candidates(scope, bullets, meta, sessions, rules):
    """[(bullet, n_sessions)] canonical, never helpful, no shipped rule, injected in >= RELEASE_SESSIONS sessions."""
    import quorum
    out = []
    for b in sorted(bullets, key=lambda b: b["id"]):
        n = len(sessions.get(b["id"], ()))
        if (quorum.state_of(meta, b["id"]) == "canonical" and int(b.get("helpful", 0)) == 0
                and not retired(rules, scope, b["id"]) and n >= RELEASE_SESSIONS):
            out.append((b, n))
    return out


def propose_release(vault, scopes, entries, agent, session, today=None):
    """Write `inbox/retire-<scope>-<id>.md` per candidate (existing proposals kept); returns written paths."""
    import ace
    import quorum
    today = today or datetime.date.today().isoformat()
    rules = load_rules(vault)
    written = []
    for scope in scopes:
        meta = quorum.load(quorum.meta_path(ace.scope_path(scope)))
        bullets = [b for bs in ace.load(scope).values() for b in bs]
        for b, n in release_candidates(scope, bullets, meta, injection_sessions(entries, scope), rules):
            path = Path(vault) / "inbox" / f"retire-{scope}-{b['id']}.md"
            if path.exists():
                continue
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(
                f"---\ntype: note\nproject: {scope}\nagent: {agent}\nstatus: active\nupdated: {today}\n"
                f"source: session {session}\n---\n# Retire proposal: {b['id']}\n\n"
                f"Canonical lesson ({scope}) injected into {n} sessions, never tagged helpful: {b['content']}\n\n"
                f"Retire (human): write `{{\"remove\": [\"{b['id']}\"]}}` to a file and run "
                f"`hive ace apply {scope} <file>`. Keep it by tagging it helpful once it earns its place.\n",
                encoding="utf-8", newline="\n")
            written.append(path)
    return written
