"""ACE (Agentic Context Engineering, arXiv 2510.04618) for the hive.

Evolving playbooks of itemized bullets `[id] helpful=X harmful=Y :: content`, changed only by
deterministic (non-LLM) delta merges, so they never collapse into a rewritten summary.
The live agent is the Generator and Reflector (skill `hive-ace`); this file is the Curator's merge step.

    python ace.py apply <scope> [ops.json]   merge JSON ops (stdin or file) into a playbook
    python ace.py show <scope> ["task"]      bullets hive_context would inject
    python ace.py hook-start                 SessionStart hook: inject top bullets (stdin = hook JSON)
    python ace.py hook-stop                  Stop hook: ask the agent to reflect after enough work
scope = "global" or a project slug (folder under projects root, lowercased, spaces to hyphens).
"""
import datetime
import json
import os
import re
import socket
import sys
import time
import uuid
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import hive
import quorum

VAULT = Path(os.environ.get("HIVE_VAULT", r"D:\Valut\OBS"))
PROJECTS = Path(os.environ.get("HIVE_PROJECTS", r"D:\PROJECTS"))
STATE_PATH = Path(os.environ.get("HIVE_ACE_STATE") or Path(
    os.environ.get("LOCALAPPDATA") or os.environ.get("TEMP") or os.getcwd()) / "hive" / "ace-state.json")

SECTIONS = {"strategies": "str", "pitfalls": "pit", "commands": "cmd", "conventions": "con"}
LINE_RE = re.compile(r"^- \[([a-z]{3}-\d{5})\] (?:seen=(\d+)\s+)?(?:updated=(\d{4}-\d{2}-\d{2})\s+)?helpful=(\d+)\s+harmful=(\d+)\s+::\s+(.+)$")
MAX_BULLET_CHARS = 300
MAX_ADDS_PER_APPLY = 5      # a noisy reflection cannot flood the playbook
MAX_BULLETS = 80            # grow-and-refine cap per playbook
DUP_JACCARD = 0.75          # ponytail: token-set Jaccard stands in for ACE's embedding dedupe; upgrade if near-dupes slip in
PRUNE_MARGIN = 3            # drop a bullet once harmful >= helpful + 3
INJECT_LIMIT = 12
STOP_TOOL_THRESHOLD = 25    # tool calls since the last reflection before the Stop hook asks for one
EDIT_TOOLS = {"Edit", "Write", "MultiEdit", "NotebookEdit"}
SECRET_RE = re.compile(
    r"(sk-[A-Za-z0-9_-]{20,}|ghp_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,}|AKIA[0-9A-Z]{16}|xox[abprs]-[A-Za-z0-9-]{10,}"
    r"|-----BEGIN [A-Z ]*PRIVATE KEY|(?:api[_-]?key|token|password|secret)\s*[:=]\s*\S{8,})", re.I)


def scrub_secrets(text):
    return SECRET_RE.sub("[REDACTED_SECRET]", text)


def is_pid_alive(pid):
    """Check if process with PID is alive (Windows & POSIX)."""
    if pid <= 0:
        return False
    if sys.platform == "win32":
        import ctypes
        PROCESS_QUERY_LIMITED_INFORMATION = 0x1000
        SYNCHRONIZE = 0x00100000
        handle = ctypes.windll.kernel32.OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE, False, pid)
        if not handle:
            return False
        ctypes.windll.kernel32.CloseHandle(handle)
        return True
    else:
        try:
            os.kill(pid, 0)
            return True
        except (OSError, ProcessLookupError):
            return False


class PlaybookLock:
    """Atomic file lock with PID:hostname:timestamp:nonce and 30s stale-steal."""

    def __init__(self, path, timeout=30.0):
        self.lock_path = Path(path).with_suffix(".lock")
        self.timeout = timeout
        self.fd = None
        self.hostname = socket.gethostname()

    def acquire(self):
        start = time.time()
        nonce = uuid.uuid4().hex[:8]
        payload = f"{os.getpid()}:{self.hostname}:{datetime.datetime.now(datetime.timezone.utc).isoformat()}:{nonce}\n"
        while True:
            try:
                self.fd = os.open(str(self.lock_path), os.O_CREAT | os.O_EXCL | os.O_RDWR)
                try:
                    os.write(self.fd, payload.encode("utf-8"))
                except BaseException:
                    os.close(self.fd)
                    self.fd = None
                    try:
                        self.lock_path.unlink()
                    except OSError:
                        pass
                    raise
                return self
            except FileExistsError:
                pass
            except OSError:
                time.sleep(0.01)
                continue

            elapsed = time.time() - start
            if elapsed > self.timeout:
                raise TimeoutError(f"could not acquire playbook lock after {self.timeout}s: held by {self.lock_path}")

            # Only inspect lock details after a brief wait
            if elapsed > 0.5:
                content = ""
                try:
                    content = self.lock_path.read_text(encoding="utf-8").strip()
                    parts = content.split(":")
                    lock_pid = int(parts[0]) if len(parts) > 0 and parts[0].isdigit() else -1
                    lock_host = parts[1] if len(parts) > 1 else ""
                except (OSError, ValueError):
                    lock_pid, lock_host = -1, ""

                is_stale = False
                if lock_host == self.hostname and lock_pid > 0 and not is_pid_alive(lock_pid):
                    is_stale = True

                try:
                    age = time.time() - self.lock_path.stat().st_mtime
                except OSError:
                    age = 0.0

                if is_stale or age > self.timeout:
                    if lock_host != self.hostname and not is_stale:
                        raise TimeoutError(f"playbook locked by foreign host {lock_host} (age {age:.1f}s)")
                    try:
                        self.lock_path.unlink()
                    except OSError:
                        pass
                    continue

            time.sleep(0.02)

    def release(self):
        if self.fd is not None:
            try:
                os.close(self.fd)
            except OSError:
                pass
            self.fd = None
        for _ in range(100):
            try:
                if not self.lock_path.exists():
                    break
                self.lock_path.unlink()
                break
            except OSError:
                time.sleep(0.01)

    def __enter__(self):
        return self.acquire()

    def __exit__(self, exc_type, exc_val, exc_tb):
        self.release()


def slugify(name):
    return name.strip().lower().replace(" ", "-")


def scope_path(scope):
    scope = slugify(scope)
    if not re.fullmatch(r"[a-z0-9._-]+", scope) or scope in {".", ".."}:
        raise ValueError(f"bad scope {scope!r}")
    return VAULT / "_meta" / "playbook.md" if scope == "global" else VAULT / "projects" / scope / "playbook.md"


def scope_for_cwd(cwd):
    """Project slug when cwd is inside PROJECTS, else 'global'."""
    try:
        rel = Path(cwd).resolve().relative_to(PROJECTS.resolve())
        return slugify(rel.parts[0]) if rel.parts else "global"
    except (ValueError, OSError):
        return "global"


# ---------------------------------------------------------------- playbook file

def parse(text):
    """-> {section: [bullet]}; bullet = {id, seen, helpful, harmful, content}. Unknown lines are dropped:
    the playbook is machine-managed."""
    sections, current = {name: [] for name in SECTIONS}, None
    for line in text.splitlines():
        heading = re.match(r"^##\s+(\w+)", line)
        if heading:
            current = heading.group(1).lower() if heading.group(1).lower() in SECTIONS else None
            continue
        m = LINE_RE.match(line.strip())
        if m and current:
            helpful = int(m.group(4))
            harmful = int(m.group(5))
            seen = int(m.group(2)) if m.group(2) is not None else (helpful + harmful)
            bullet = {
                "id": m.group(1),
                "seen": seen,
                "helpful": helpful,
                "harmful": harmful,
                "content": m.group(6).strip(),
            }
            if m.group(3):
                bullet["updated"] = m.group(3)
            sections[current].append(bullet)
    return sections


def render(scope, sections, today):
    head = (f"---\ntype: note\nproject: {'all' if scope == 'global' else scope}\nagent: claude\n"
            f"status: active\nupdated: {today}\n---\n# Playbook: {scope}\n\n"
            "> ACE playbook. Changed only by `hive ace apply` (skill `hive-ace`).\n"
            "> Do not hand-edit: delta merges keep counters and prevent context collapse.\n")
    body = []
    for name in SECTIONS:
        body.append(f"\n## {name}\n")
        lines = []
        for b in sections[name]:
            up = f" updated={b['updated']}" if b.get("updated") else ""
            lines.append(f"- [{b['id']}] seen={b.get('seen', b['helpful'] + b['harmful'])}{up} helpful={b['helpful']} harmful={b['harmful']} :: {b['content']}")
        body += lines
    return head + "\n".join(body) + "\n"


def load(scope):
    path = scope_path(scope)
    return parse(path.read_text(encoding="utf-8")) if path.is_file() else {name: [] for name in SECTIONS}


def save(scope, sections, today=None):
    path = scope_path(scope)
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_suffix(".md.tmp")
    tmp.write_text(render(slugify(scope), sections, today or time.strftime("%Y-%m-%d")), encoding="utf-8")
    os.replace(tmp, path)  # atomic: a concurrent reader sees old or new, never half


# ---------------------------------------------------------------- curator (deterministic merge)

def tokens(text):
    return {t for t in re.findall(r"[a-z0-9]+", text.lower()) if len(t) > 2}


def jaccard(a, b):
    return len(a & b) / len(a | b) if a and b else 0.0


def bullet_decay(b, today_date=None):
    if b.get("helpful", 0) > 0:
        return 0
    if b.get("seen", 0) < 10:
        return 0
    up_str = b.get("updated")
    if not up_str:
        return 0
    try:
        t_date = today_date or datetime.date.today()
        b_date = datetime.date.fromisoformat(up_str)
        days = (t_date - b_date).days
        if days > 30:
            return max(3, (days - 30) // 10 + 1)
    except (ValueError, TypeError):
        pass
    return 0


def net(b, today_date=None):
    return b["helpful"] - b["harmful"] - bullet_decay(b, today_date)


def apply_ops(sections, ops, today=None, reserved=()):
    """Pure: returns (new_sections, report). ops: [{type: ADD, section, content} | {type: TAG, id, tag}]."""
    t_date = datetime.date.fromisoformat(today) if today else datetime.date.today()
    new = {name: [dict(b) for b in bullets] for name, bullets in sections.items()}
    by_id = {b["id"]: b for bullets in new.values() for b in bullets}
    report = {"added": [], "reinforced": [], "tagged": 0, "rejected": [], "pruned": []}
    next_n = max((int(i.split("-")[1]) for i in [*by_id, *reserved]), default=0) + 1  # retired ids never reissued
    adds = 0
    for op in ops if isinstance(ops, list) else []:
        kind = str(op.get("type", "")).upper() if isinstance(op, dict) else ""
        if kind == "TAG":
            b = by_id.get(str(op.get("id")))
            tag = op.get("tag")
            if not tag:
                if op.get("helpful"):
                    tag = "helpful"
                elif op.get("harmful"):
                    tag = "harmful"
            delta = op.get(tag, 1) if isinstance(op.get(tag), int) else 1
            if b and tag in ("helpful", "harmful"):
                b[tag] += delta
                b["seen"] = max(b.get("seen", 0), b["helpful"] + b["harmful"])
                report["tagged"] += 1
            elif str(op.get("id", "")).startswith("skill:") and tag in ("helpful", "harmful"):
                report["tagged"] += 1
                try:
                    import ledger, hive
                    ledger_dest = os.environ.get("HIVE_LEDGER") or hive.load_config()["ledger"]
                    skill_name = str(op.get("id"))[6:]
                    ledger.record(ledger_dest, event="skill_feedback", project=skill_name, query=tag, hits=[], duration_ms=0.0)
                except Exception:
                    pass
            continue
        if kind != "ADD":
            report["rejected"].append(f"unknown op {op!r:.60}")
            continue
        section = str(op.get("section", "")).lower()
        content = " ".join(str(op.get("content", "")).split())
        reason = ("unknown section" if section not in SECTIONS else "empty" if not content
                  else "too long" if len(content) > MAX_BULLET_CHARS else "looks like a secret" if SECRET_RE.search(content)
                  else "add limit reached" if adds >= MAX_ADDS_PER_APPLY else "")
        if reason:
            report["rejected"].append(f"{reason}: {content[:60]}")
            continue
        twin = max(by_id.values(), key=lambda b: jaccard(tokens(b["content"]), tokens(content)), default=None)
        if twin and jaccard(tokens(twin["content"]), tokens(content)) >= DUP_JACCARD:
            twin["seen"] = twin.get("seen", twin["helpful"] + twin["harmful"]) + 1  # reinforce seen count without inflating helpful
            report["reinforced"].append(twin["id"])
            continue
        bullet = {"id": f"{SECTIONS[section]}-{next_n:05d}", "seen": 1, "helpful": 0, "harmful": 0, "content": content}
        next_n += 1
        adds += 1
        new[section].append(bullet)
        by_id[bullet["id"]] = bullet
        report["added"].append(bullet["id"])
    for name in new:  # refine: prune bullets the evidence says hurt or decayed
        keep = [b for b in new[name] if (b["harmful"] + bullet_decay(b, t_date)) < b["helpful"] + PRUNE_MARGIN]
        report["pruned"] += [b["id"] for b in new[name] if b not in keep]
        new[name] = keep
    everything = [b for bullets in new.values() for b in bullets]
    if len(everything) > MAX_BULLETS:  # over cap: drop lowest net score, oldest first
        drop = sorted(everything, key=lambda b: (net(b, t_date), int(b["id"].split("-")[1])))[:len(everything) - MAX_BULLETS]
        ids = {b["id"] for b in drop}
        report["pruned"] += sorted(ids)
        new = {name: [b for b in bullets if b["id"] not in ids] for name, bullets in new.items()}
    return new, report


def helpful_tags(ops, live_ids, session, agent):
    """QUOR-02 evidence from helpful TAG ops on live bullets: [(id, session, agent, hint)]. An op may name the
    session it speaks for (`ledger score` credits the scored session) and an `evidence` hint (`test_pass`)."""
    out = []
    for op in ops if isinstance(ops, list) else []:
        if not isinstance(op, dict) or str(op.get("type", "")).upper() != "TAG" or op.get("id") not in live_ids:
            continue
        if op.get("tag") == "helpful" or (not op.get("tag") and op.get("helpful")):
            out.append((op["id"], str(op.get("session") or session), agent, op.get("evidence", "")))
    return out


def apply(scope, ops, today=None):
    """Load, apply ops under atomic lock, save, return report."""
    path = scope_path(scope)
    path.parent.mkdir(parents=True, exist_ok=True)
    with PlaybookLock(path):
        current = load(scope)
        meta_file = quorum.meta_path(path)
        meta = quorum.load(meta_file)
        new_sections, report = apply_ops(current, ops, today=today, reserved=meta.keys())
        save(scope, new_sections, today=today)
        import ledger
        session, agent = ledger.resolve_session(), ledger.resolve_agent()
        live = {b["id"] for bullets in new_sections.values() for b in bullets}
        evidence = helpful_tags(ops, live, session, agent)
        # QUOR-03: another session independently proposing the same lesson is a confirmation
        evidence += [(i, session, agent, "") for i in report["reinforced"] if i in live]
        # QUOR-04: conflicts inside this playbook gate promotion and are reported; lines are never touched
        pairs = quorum.conflicts([b for bullets in new_sections.values() for b in bullets])
        if report["added"] or report["pruned"] or evidence:  # QUOR-01/02, PROV-04: sidecar, same lock
            base = quorum.reconcile(meta, report["added"], report["pruned"], session, agent, int(time.time()))
            meta = quorum.displace(base, quorum.add_evidence(base, evidence, quorum.gates(base, pairs)), pairs)
            quorum.save(meta_file, meta)
        quorum.write_report(VAULT / "_meta" / "_conflicts.md", slugify(scope), pairs, meta,
                            today or time.strftime("%Y-%m-%d"))
    try:  # PROV-02/04: who merged which bullets, at which commit; fail-open
        import hashlib
        import ledger
        ledger.record_ace_apply(hive.load_config()["ledger"], scope, report.get("added", []),
                                content_sha256=hashlib.sha256(path.read_bytes()).hexdigest(),
                                repo_sha=ledger.repo_head(os.getcwd()))
    except Exception as exc:
        print(f"ace: provenance record skipped: {exc}", file=sys.stderr)
    return report


def select(scopes, task="", limit=INJECT_LIMIT):
    """Top bullets across scopes: net evidence first, task-term overlap as tie-breaker boost."""
    import governance
    want = tokens(task)
    pool = []
    rules = governance.load_rules(VAULT)
    for scope in scopes:
        meta = quorum.load(quorum.meta_path(scope_path(scope)))  # QUOR-03: only injectable states
        for name, bullets in load(scope).items():
            for b in bullets:
                if not quorum.injectable(quorum.state_of(meta, b["id"])):
                    continue
                if governance.bullet_hidden(rules, meta.get(b["id"], {})):  # GOV-02
                    continue
                overlap = len(want & tokens(b["content"])) if want else 0
                pool.append(((net(b) + 1) * (1 + overlap), scope, name, b))
    pool.sort(key=lambda x: -x[0])
    return [(scope, name, b) for _, scope, name, b in pool[:limit]]


def format_bullets(rows):
    return "\n".join(f"- [{b['id']}] ({scope}/{name}, +{b['helpful']}/-{b['harmful']}) {b['content']}"
                     for scope, name, b in rows)


# ---------------------------------------------------------------- hooks

def read_hook_input():
    try:
        data = json.loads(sys.stdin.read() or "{}")
    except json.JSONDecodeError:
        return {}
    if isinstance(data, dict):  # PROV-01: hooks carry the client's own session id and transcript path
        try:
            import ledger
            ledger.set_context(agent=ledger.agent_from_hint(data.get("transcript_path", "")),
                               session_id=data.get("session_id"))
        except Exception:
            pass
        return data
    return {}


def hook_start(scope=None):
    data = read_hook_input()
    resolved_scope = scope or scope_for_cwd(data.get("cwd") or os.getcwd())
    rows = select(["global"] + ([resolved_scope] if resolved_scope != "global" else []))
    if not rows:
        return
    text = ("Hive playbook (ACE). Apply what fits; note the ids you relied on. "
            "After substantial work run the `hive-ace` skill to tag them and add new lessons.\n" + format_bullets(rows))
    try:
        import ledger
        cfg = hive.load_config()
        ledger.record(cfg["ledger"], event="inject", project=resolved_scope, query="hook-start", injected_bullets=[b["id"] for _, _, b in rows])
    except Exception:
        pass
    if data:
        print(json.dumps({"hookSpecificOutput": {"hookEventName": "SessionStart", "additionalContext": text}}))
    else:
        print(text)


def count_tools(transcript_path):
    tools = edits = 0
    try:
        with open(transcript_path, encoding="utf-8", errors="replace") as fh:
            for line in fh:
                if '"tool_use"' not in line:
                    continue
                try:
                    content = json.loads(line).get("message", {}).get("content", [])
                except json.JSONDecodeError:
                    continue
                for item in content if isinstance(content, list) else []:
                    if isinstance(item, dict) and item.get("type") == "tool_use":
                        tools += 1
                        edits += item.get("name") in EDIT_TOOLS
    except OSError:
        pass
    return tools, edits


def emit_hot_files(transcript, session):
    """TRACE-05: leave HOT_FILE traces for files this session keeps failing on; fail-open."""
    try:
        import sqlite3
        import time
        import traces
        db = hive.open_db(hive.load_config()["db"])[0]  # heals a corrupt index
        try:
            with db:
                traces.emit_hot_files(db, transcript, "claude", session, int(time.time()))
        finally:
            db.close()
    except Exception as exc:  # a hook must never break the session
        print(f"ace: hot-file traces skipped: {exc!r}", file=sys.stderr)


def hook_stop(scope=None, *args):
    data = read_hook_input()
    resolved_scope = scope or scope_for_cwd(data.get("cwd") or os.getcwd())
    try:
        import ledger
        cfg = hive.load_config()
        ledger.record(cfg["ledger"], event="reflect", project=resolved_scope, query="hook-stop")
    except Exception:
        pass
    if data.get("stop_hook_active") or os.environ.get("HIVE_ACE_DISABLE"):
        return
    session, transcript = data.get("session_id"), data.get("transcript_path")
    if not session or not transcript:
        print(f"ace: stop hook triggered for scope '{resolved_scope}'")
        return
    emit_hot_files(transcript, session)  # each file is filed under its own project, not the cwd scope
    tools, edits = count_tools(transcript)
    try:
        state = json.loads(STATE_PATH.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        state = {}
    if tools - state.get(session, 0) < STOP_TOOL_THRESHOLD or not edits:
        return
    state[session] = tools
    state = dict(list(state.items())[-200:])
    STATE_PATH.parent.mkdir(parents=True, exist_ok=True)
    STATE_PATH.write_text(json.dumps(state), encoding="utf-8")
    print(json.dumps({"decision": "block", "reason": (
        f"Hive ACE checkpoint ({tools} tool calls). Run the `hive-ace` skill now for scope `{resolved_scope}`: "
        "reflect on this session, tag playbook bullets you used, add at most 3 new lessons, apply with ace.py, "
        "then finish your previous reply unchanged. Skip if nothing reusable was learned.")}))


# ---------------------------------------------------------------- cli

def main(argv):
    cmd, *rest = argv or ["help"]
    if cmd in ("hook-start", "hook-stop"):
        try:  # a hook must never break the session
            (hook_start if cmd == "hook-start" else hook_stop)(*rest)
        except Exception as exc:
            print(f"hive ace {cmd}: {exc!r}", file=sys.stderr)
        return 0
    sys.stdout.reconfigure(encoding="utf-8")
    if cmd == "apply" and rest:
        raw = Path(rest[1]).read_text(encoding="utf-8") if len(rest) > 1 else sys.stdin.read()
        try:
            payload = json.loads(raw)
        except json.JSONDecodeError as exc:  # usually shell-mangled backslashes: pass a file or use / in paths
            print(f"ace apply: invalid JSON ({exc}); nothing changed", file=sys.stderr)
            return 1
        ops = payload.get("operations", []) + payload.get("bullet_tags", []) if isinstance(payload, dict) else payload
        # agent-written deltas cannot name a session or claim test evidence (QUOR-02 trust boundary)
        ops = [{k: v for k, v in dict(o, type=o.get("type", "TAG")).items() if k not in quorum.TRUSTED_ONLY_FIELDS}
               for o in ops if isinstance(o, dict)]
        report = apply(rest[0], ops)
        print(json.dumps(report, indent=1))
        return 0
    if cmd == "show" and rest:
        print(format_bullets(select(["global"] + ([rest[0]] if rest[0] != "global" else []),
                                    rest[1] if len(rest) > 1 else "")) or "(empty playbook)")
        return 0
    print(__doc__)
    return 2


if __name__ == "__main__":
    native = hive.find_native_binary()
    # Hooks stay in Python: the Rust `ace hook-*` commands ignore the hook's stdin JSON (no cwd scope, no
    # session id, no Stop-checkpoint decision), so delegating them silently disabled the ACE checkpoint.
    if native and len(sys.argv) > 1 and sys.argv[1] in ("apply", "show"):
        import subprocess
        sys.exit(subprocess.call([native, "ace", *sys.argv[1:]]))
    sys.exit(main(sys.argv[1:]))
