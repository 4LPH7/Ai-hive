import json
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

# Common delimiters and word splitter
WORD_RE = re.compile(r"[^A-Za-z0-9]+")


def tokenize(text):
    if not text:
        return set()
    return {w.lower() for w in WORD_RE.split(text) if len(w) > 1}


class SkillMeta:
    def __init__(self, name, description, triggers=None, path=None, root_type="custom"):
        self.name = name.strip()
        self.description = description.strip()
        self.triggers = [t.strip().lower() for t in (triggers or []) if t.strip()]
        self.path = str(path) if path else ""
        self.root_type = root_type

    def to_dict(self):
        return {
            "name": self.name,
            "description": self.description,
            "triggers": self.triggers,
            "path": self.path,
            "root_type": self.root_type,
        }

    def __repr__(self):
        return f"SkillMeta(name={self.name!r}, root_type={self.root_type!r}, triggers={len(self.triggers)})"


def parse_skill_md(file_path, root_type="custom"):
    """Parse SKILL.md extracting YAML frontmatter and title."""
    p = Path(file_path)
    if not p.is_file():
        return None

    try:
        content = p.read_text(encoding="utf-8")
    except UnicodeDecodeError:
        content = p.read_text(encoding="utf-8-sig", errors="replace")

    # Strip Windows UTF-8 BOM
    if content.startswith("\ufeff"):
        content = content[1:]

    name = p.parent.name
    description = ""
    triggers = []

    # Check for YAML frontmatter
    lines = content.splitlines()
    if lines and lines[0].strip() == "---":
        yaml_lines = []
        for line in lines[1:]:
            if line.strip() == "---":
                break
            yaml_lines.append(line)

        in_triggers = False
        for yl in yaml_lines:
            stripped = yl.strip()
            if not stripped or stripped.startswith("#"):
                continue

            if in_triggers:
                if stripped.startswith("-"):
                    triggers.append(stripped.lstrip("- ").strip("'\""))
                    continue
                else:
                    in_triggers = False

            if ":" in yl:
                k, _, v = yl.partition(":")
                k = k.strip().lower()
                v = v.strip().strip("'\"")
                if k == "name" and v:
                    name = v
                elif k == "description" and v:
                    description = v
                elif k == "triggers":
                    if v:
                        triggers.extend([t.strip().strip("'\"") for t in v.split(",") if t.strip()])
                    else:
                        in_triggers = True

    # Fallback to heading / first paragraph if description empty
    if not description:
        body = "\n".join(lines[len(yaml_lines) + 2:]) if lines and lines[0].strip() == "---" else content
        for line in body.splitlines():
            s = line.strip()
            if s.startswith("# ") and name == p.parent.name:
                name = s.lstrip("# ").strip()
            elif s and not s.startswith("#") and not description:
                description = s[:300]
                break

    return SkillMeta(name=name, description=description, triggers=triggers, path=p, root_type=root_type)


def discover_skill_roots(custom_roots=None):
    """Return all standard skill directories across Antigravity, Codex, Claude Code, and project."""
    roots = []
    if custom_roots:
        for cr in custom_roots:
            p = Path(cr)
            if p.is_dir():
                roots.append((p, "custom"))

    home = os.environ.get("USERPROFILE") or os.environ.get("HOME")
    if home:
        h = Path(home)
        # 1. Antigravity / Gemini
        g_cfg = h / ".gemini" / "config" / "skills"
        if g_cfg.is_dir():
            roots.append((g_cfg, "antigravity"))
        g_builtin = h / ".gemini" / "antigravity" / "builtin" / "skills"
        if g_builtin.is_dir():
            roots.append((g_builtin, "antigravity"))

        # Plugins
        plugins_dir = h / ".gemini" / "config" / "plugins"
        if plugins_dir.is_dir():
            try:
                for plugin in plugins_dir.iterdir():
                    pskills = plugin / "skills"
                    if pskills.is_dir():
                        roots.append((pskills, "antigravity"))
            except OSError:
                pass

        # 2. Codex
        codex_home = h / ".codex" / "skills"
        if codex_home.is_dir():
            roots.append((codex_home, "codex"))

        # 3. Claude Code
        claude_home = h / ".claude" / "skills"
        if claude_home.is_dir():
            roots.append((claude_home, "claude"))

    # 4. Project local
    cwd = Path.cwd()
    proj_skills = cwd / ".agent" / "skills"
    if proj_skills.is_dir():
        roots.append((proj_skills, "project"))

    proj_skills_alt = cwd / ".skills"
    if proj_skills_alt.is_dir():
        roots.append((proj_skills_alt, "project"))

    codex_local = cwd / ".codex" / "skills"
    if codex_local.is_dir():
        roots.append((codex_local, "codex"))

    claude_local = cwd / ".claude" / "skills"
    if claude_local.is_dir():
        roots.append((claude_local, "claude"))

    return roots


def scan_skills(roots=None):
    """Scan all skill roots and return list of SkillMeta."""
    if roots is None:
        roots = discover_skill_roots()

    skills = []
    seen_names = set()
    for root_dir, root_type in roots:
        if not root_dir.is_dir():
            continue
        try:
            for item in root_dir.iterdir():
                if item.is_dir():
                    skill_file = item / "SKILL.md"
                    if not skill_file.is_file():
                        skill_file = item / "skill.md"
                    if skill_file.is_file():
                        meta = parse_skill_md(skill_file, root_type=root_type)
                        if meta and meta.name not in seen_names:
                            seen_names.add(meta.name)
                            skills.append(meta)
                elif item.is_file() and item.name.lower() in ("skill.md",):
                    meta = parse_skill_md(item, root_type=root_type)
                    if meta and meta.name not in seen_names:
                        seen_names.add(meta.name)
                        skills.append(meta)
        except OSError:
            continue

    return skills


class MatchResult:
    def __init__(self, skill, confidence, is_match):
        self.skill = skill
        self.confidence = round(confidence, 3)
        self.is_match = is_match

    def to_dict(self):
        return {
            "skill": self.skill.to_dict() if self.skill else None,
            "confidence": self.confidence,
            "is_match": self.is_match,
        }

    def __repr__(self):
        name = self.skill.name if self.skill else "None"
        return f"MatchResult(skill={name!r}, confidence={self.confidence}, is_match={self.is_match})"


def score_skill(task, skill):
    """Compute normalized confidence score [0.0, 1.0] between task description and skill."""
    task_lower = task.lower().strip()
    task_toks = tokenize(task_lower)
    if not task_toks:
        return 0.0

    # 1. Exact trigger match (high weight multiplier)
    trigger_hit = False
    for trig in skill.triggers:
        if trig and trig in task_lower:
            trigger_hit = True
            break
        trig_toks = tokenize(trig)
        if trig_toks and trig_toks.issubset(task_toks):
            trigger_hit = True
            break

    def match_token(a, b):
        return a == b or (len(a) >= 4 and len(b) >= 4 and (a.startswith(b) or b.startswith(a)))

    # 2. Name token overlap
    name_toks = tokenize(skill.name)
    name_overlap = sum(1 for n in name_toks if any(match_token(n, t) for t in task_toks)) / max(1, len(name_toks)) if name_toks else 0.0

    # 3. Description token overlap
    desc_toks = tokenize(skill.description)
    desc_overlap = sum(1 for d in desc_toks if any(match_token(d, t) for t in task_toks)) / max(1, len(desc_toks)) if desc_toks else 0.0

    # Weighted combination
    score = 0.0
    if trigger_hit:
        score += 0.65  # Explicit trigger activation guarantees meeting 0.65 threshold
    score += name_overlap * 0.45
    score += desc_overlap * 0.25

    return min(1.0, score)


def match_skill(task, skills=None, threshold=0.65, custom_roots=None):
    """Find the best local skill matching task. Returns MatchResult."""
    if skills is None:
        skills = scan_skills(roots=discover_skill_roots(custom_roots=custom_roots))

    best_skill = None
    best_score = 0.0

    for s in skills:
        sc = score_skill(task, s)
        if sc > best_score:
            best_score = sc
            best_skill = s

    is_match = best_score >= threshold and best_skill is not None
    return MatchResult(best_skill, best_score, is_match)


# ---------------------------------------------------------------- remote & install

DANGEROUS_PATTERNS = [
    re.compile(r"eval\s*\(", re.I),
    re.compile(r"curl\s+-[A-Za-z]*d\s+.*https?://", re.I),
    re.compile(r"bash\s+-c\s+.*(?:curl|wget)\b", re.I),
    re.compile(r"powershell\s+-enc\b", re.I),
    re.compile(r"Invoke-Expression\b", re.I),
]


class RemoteSkillCandidate:
    def __init__(self, package_name, description="", author="", stars=0, downloads=0, source_url=""):
        self.package_name = package_name.strip()
        self.description = description.strip()
        self.author = author.strip()
        self.stars = int(stars or 0)
        self.downloads = int(downloads or 0)
        self.source_url = source_url.strip()

    def to_dict(self):
        return {
            "package_name": self.package_name,
            "description": self.description,
            "author": self.author,
            "stars": self.stars,
            "downloads": self.downloads,
            "source_url": self.source_url,
        }

    def __repr__(self):
        return f"RemoteSkillCandidate(package={self.package_name!r}, stars={self.stars})"


def find_remote_skills(query):
    """Query remote skill registries via npx skills find <query> --json with graceful degradation."""
    if not query:
        return []

    # Check if npx is on PATH
    npx_bin = shutil.which("npx")
    if not npx_bin:
        print("find_remote_skills: diagnostic: npx not found on PATH; remote discovery skipped", file=sys.stderr)
        return []

    try:
        proc = subprocess.run(
            [npx_bin, "--yes", "skills", "find", query, "--json"],
            capture_output=True,
            text=True,
            timeout=8,
        )
        if proc.returncode == 0 and proc.stdout.strip():
            raw = json.loads(proc.stdout)
            candidates = []
            items = raw if isinstance(raw, list) else raw.get("skills", [])
            for item in items:
                if isinstance(item, dict) and item.get("name"):
                    candidates.append(RemoteSkillCandidate(
                        package_name=item["name"],
                        description=item.get("description", ""),
                        author=item.get("author", ""),
                        stars=item.get("stars", 0),
                        downloads=item.get("downloads", 0),
                        source_url=item.get("url", ""),
                    ))
            return candidates
    except (subprocess.TimeoutExpired, json.JSONDecodeError, OSError) as e:
        print(f"find_remote_skills: diagnostic: remote query failed ({e}); falling back cleanly", file=sys.stderr)

    return []


def audit_skill_safety(dir_or_file_path):
    """Audit skill files for dangerous eval/exfil patterns. Returns (is_safe, error_reason)."""
    p = Path(dir_or_file_path)
    files_to_check = []
    if p.is_file():
        files_to_check.append(p)
    elif p.is_dir():
        for root, _, files in os.walk(p):
            for f in files:
                ext = Path(f).suffix.lower()
                if ext in (".md", ".py", ".sh", ".bash", ".js", ".ts", ".ps1"):
                    files_to_check.append(Path(root) / f)

    for fp in files_to_check:
        try:
            text = fp.read_text(encoding="utf-8", errors="replace")
            for pattern in DANGEROUS_PATTERNS:
                m = pattern.search(text)
                if m:
                    return False, f"Dangerous pattern {pattern.pattern!r} matched in {fp.name}"
        except OSError as e:
            return False, f"Failed to read file {fp.name}: {e}"

    return True, None


def install_skill(package_name, target_dir=None, dry_run=False):
    """Install a remote skill package into target directory with security audit gate."""
    if not target_dir:
        slug = package_name.strip().lstrip("@").replace("/", "-")
        target_dir = Path.cwd() / ".agent" / "skills" / slug
    else:
        target_dir = Path(target_dir)

    if dry_run:
        return {
            "status": "dry_run",
            "package": package_name,
            "installed_path": str(target_dir),
            "message": f"[dry-run] Would install {package_name} to {target_dir}",
        }

    target_dir.mkdir(parents=True, exist_ok=True)

    npx_bin = shutil.which("npx")
    if not npx_bin:
        return {"status": "error", "error": "npx not found on PATH; cannot install skill"}

    try:
        proc = subprocess.run(
            [npx_bin, "--yes", "skills", "add", package_name, "--dest", str(target_dir)],
            capture_output=True,
            text=True,
            timeout=30,
        )
        if proc.returncode != 0:
            shutil.rmtree(target_dir, ignore_errors=True)
            return {"status": "error", "error": f"skills add failed: {proc.stderr.strip()}"}

        # Run security audit on installed files
        is_safe, reason = audit_skill_safety(target_dir)
        if not is_safe:
            # Atomic rollback on security failure
            shutil.rmtree(target_dir, ignore_errors=True)
            return {"status": "error", "error": f"Security audit failed: {reason}"}

        return {
            "status": "installed",
            "package": package_name,
            "installed_path": str(target_dir),
        }
    except Exception as e:
        shutil.rmtree(target_dir, ignore_errors=True)
        return {"status": "error", "error": str(e)}


def load_skill_context(skill_path_or_meta):
    """Read SKILL.md and format actionable agent context block."""
    if isinstance(skill_path_or_meta, SkillMeta):
        p = Path(skill_path_or_meta.path)
        name = skill_path_or_meta.name
    else:
        p = Path(skill_path_or_meta)
        if p.is_dir():
            cand = p / "SKILL.md"
            if not cand.is_file():
                cand = p / "skill.md"
            p = cand
        name = p.parent.name

    if not p.is_file():
        return f"> [!WARNING] Skill instruction file not found: {p}"

    try:
        text = p.read_text(encoding="utf-8")
    except UnicodeDecodeError:
        text = p.read_text(encoding="utf-8-sig", errors="replace")

    # Strip YAML frontmatter from emitted instruction body
    lines = text.splitlines()
    body_lines = lines
    if lines and lines[0].strip() == "---":
        for i, line in enumerate(lines[1:], start=1):
            if line.strip() == "---":
                body_lines = lines[i + 1:]
                break

    body = "\n".join(body_lines).strip()
    return (
        f"> [!NOTE] Active Agent Skill: {name}\n"
        f"> Source: {p}\n"
        "> Follow the instructions below to execute the task:\n\n"
        f"{body}"
    )
