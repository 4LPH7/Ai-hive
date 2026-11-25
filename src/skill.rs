use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SkillMeta {
    pub name: String,
    pub description: String,
    pub triggers: Vec<String>,
    pub path: String,
    pub root_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MatchResult {
    pub skill: Option<SkillMeta>,
    pub confidence: f64,
    pub is_match: bool,
}

pub const STOP_WORDS: &[&str] = &[
    "a",
    "about",
    "above",
    "after",
    "again",
    "against",
    "all",
    "am",
    "an",
    "and",
    "any",
    "are",
    "as",
    "at",
    "be",
    "because",
    "been",
    "before",
    "being",
    "below",
    "between",
    "both",
    "but",
    "by",
    "can",
    "did",
    "do",
    "does",
    "doing",
    "don",
    "down",
    "during",
    "each",
    "few",
    "for",
    "from",
    "further",
    "had",
    "has",
    "have",
    "having",
    "he",
    "her",
    "here",
    "hers",
    "herself",
    "him",
    "himself",
    "his",
    "how",
    "if",
    "in",
    "into",
    "is",
    "it",
    "its",
    "itself",
    "just",
    "me",
    "more",
    "most",
    "my",
    "myself",
    "no",
    "nor",
    "not",
    "now",
    "of",
    "off",
    "on",
    "once",
    "only",
    "or",
    "other",
    "our",
    "ours",
    "ourselves",
    "out",
    "over",
    "own",
    "same",
    "she",
    "should",
    "so",
    "some",
    "such",
    "than",
    "that",
    "the",
    "their",
    "theirs",
    "them",
    "themselves",
    "then",
    "there",
    "these",
    "they",
    "this",
    "those",
    "through",
    "to",
    "too",
    "under",
    "until",
    "up",
    "very",
    "was",
    "we",
    "were",
    "what",
    "when",
    "where",
    "which",
    "while",
    "who",
    "whom",
    "why",
    "with",
    "you",
    "your",
    "yours",
    "yourself",
    "yourselves",
    "due",
];

pub fn tokenize(text: &str) -> HashSet<String> {
    let re = Regex::new(r"[^A-Za-z0-9_-]+").unwrap();
    re.split(text)
        .map(|s| s.to_lowercase())
        .filter(|s| s.len() > 1 && !STOP_WORDS.contains(&s.as_str()))
        .collect()
}

pub fn parse_skill_md(file_path: &Path, root_type: &str) -> Option<SkillMeta> {
    if !file_path.is_file() {
        return None;
    }
    let content = match fs::read_to_string(file_path) {
        Ok(c) => c,
        Err(_) => return None,
    };

    // Strip BOM
    let clean = content.strip_prefix('\u{feff}').unwrap_or(&content);

    let mut name = file_path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string();

    let mut description = String::new();
    let mut triggers = Vec::new();

    let lines: Vec<&str> = clean.lines().collect();
    if !lines.is_empty() && lines[0].trim() == "---" {
        let mut yaml_lines = Vec::new();
        for &line in &lines[1..] {
            if line.trim() == "---" {
                break;
            }
            yaml_lines.push(line);
        }

        let mut in_triggers = false;
        for &yl in &yaml_lines {
            let trimmed = yl.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            if in_triggers {
                if trimmed.starts_with('-') {
                    triggers.push(
                        trimmed
                            .trim_start_matches('-')
                            .trim()
                            .trim_matches('\'')
                            .trim_matches('"')
                            .to_string(),
                    );
                    continue;
                } else {
                    in_triggers = false;
                }
            }

            if let Some((k, v)) = yl.split_once(':') {
                let key = k.trim().to_lowercase();
                let val = v.trim().trim_matches('\'').trim_matches('"').trim();
                match key.as_str() {
                    "name" if !val.is_empty() => name = val.to_string(),
                    "description" if !val.is_empty() => description = val.to_string(),
                    "triggers" => {
                        if !val.is_empty() {
                            for t in val.split(',') {
                                let item = t.trim().trim_matches('\'').trim_matches('"');
                                if !item.is_empty() {
                                    triggers.push(item.to_string());
                                }
                            }
                        } else {
                            in_triggers = true;
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    if description.is_empty() {
        for &line in &lines {
            let s = line.trim();
            if s.starts_with("# ")
                && name
                    == file_path
                        .parent()
                        .unwrap()
                        .file_name()
                        .unwrap()
                        .to_str()
                        .unwrap()
            {
                name = s.trim_start_matches("# ").trim().to_string();
            } else if !s.is_empty() && !s.starts_with('#') && description.is_empty() {
                description = s.chars().take(300).collect();
                break;
            }
        }
    }

    if triggers.is_empty() && !description.is_empty() {
        let re_use = Regex::new(r"(?i)(?:use when|trigger on|when)\s+([^,.;\n]+)").unwrap();
        for cap in re_use.captures_iter(&description) {
            let t = cap[1].trim().trim_matches('\'').trim_matches('"').trim();
            if t.len() > 3 {
                triggers.push(t.to_string());
            }
        }
    }

    Some(SkillMeta {
        name,
        description,
        triggers,
        path: file_path.display().to_string(),
        root_type: root_type.to_string(),
    })
}

pub fn discover_skill_roots(custom_roots: Option<&[PathBuf]>) -> Vec<(PathBuf, String)> {
    let mut roots = Vec::new();
    if let Some(custom) = custom_roots {
        for cr in custom {
            if cr.is_dir() {
                roots.push((cr.clone(), "custom".to_string()));
            }
        }
    }

    // 1. Project local roots take top priority
    if let Ok(cwd) = std::env::current_dir() {
        let proj_skills = cwd.join(".agent").join("skills");
        if proj_skills.is_dir() {
            roots.push((proj_skills, "project".to_string()));
        }
        let proj_skills_alt = cwd.join(".skills");
        if proj_skills_alt.is_dir() {
            roots.push((proj_skills_alt, "project".to_string()));
        }
        let codex_local = cwd.join(".codex").join("skills");
        if codex_local.is_dir() {
            roots.push((codex_local, "codex".to_string()));
        }
        let claude_local = cwd.join(".claude").join("skills");
        if claude_local.is_dir() {
            roots.push((claude_local, "claude".to_string()));
        }
    }

    // 2. User global roots
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .ok();
    if let Some(h_str) = home {
        let h = PathBuf::from(h_str);
        // Antigravity / Gemini
        let g_cfg = h.join(".gemini").join("config").join("skills");
        if g_cfg.is_dir() {
            roots.push((g_cfg, "antigravity".to_string()));
        }
        let g_builtin = h
            .join(".gemini")
            .join("antigravity")
            .join("builtin")
            .join("skills");
        if g_builtin.is_dir() {
            roots.push((g_builtin, "antigravity".to_string()));
        }

        let plugins_dir = h.join(".gemini").join("config").join("plugins");
        if plugins_dir.is_dir() {
            if let Ok(entries) = fs::read_dir(plugins_dir) {
                for entry in entries.flatten() {
                    let pskills = entry.path().join("skills");
                    if pskills.is_dir() {
                        roots.push((pskills, "antigravity".to_string()));
                    }
                }
            }
        }

        // Codex
        let codex_home = h.join(".codex").join("skills");
        if codex_home.is_dir() {
            roots.push((codex_home, "codex".to_string()));
        }

        // Claude
        let claude_home = h.join(".claude").join("skills");
        if claude_home.is_dir() {
            roots.push((claude_home, "claude".to_string()));
        }
    }

    roots
}

pub fn scan_skills(roots: Option<&[(PathBuf, String)]>) -> Vec<SkillMeta> {
    let default_roots;
    let roots_ref = match roots {
        Some(r) => r,
        None => {
            default_roots = discover_skill_roots(None);
            &default_roots
        }
    };

    let mut skills = Vec::new();
    let mut seen_names = HashSet::new();

    for (root_dir, root_type) in roots_ref {
        if !root_dir.is_dir() {
            continue;
        }
        if let Ok(entries) = fs::read_dir(root_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    let mut skill_file = path.join("SKILL.md");
                    if !skill_file.is_file() {
                        skill_file = path.join("skill.md");
                    }
                    if skill_file.is_file() {
                        if let Some(meta) = parse_skill_md(&skill_file, root_type) {
                            if seen_names.insert(meta.name.clone()) {
                                skills.push(meta);
                            }
                        }
                    }
                } else if path.is_file()
                    && path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .map(|s| s.to_lowercase())
                        == Some("skill.md".to_string())
                {
                    if let Some(meta) = parse_skill_md(&path, root_type) {
                        if seen_names.insert(meta.name.clone()) {
                            skills.push(meta);
                        }
                    }
                }
            }
        }
    }

    skills
}

pub fn match_token(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    if a.len() >= 4 && b.len() >= 4 {
        let prefix_len = a
            .chars()
            .zip(b.chars())
            .take_while(|(ca, cb)| ca == cb)
            .count();
        if prefix_len >= 4 {
            return true;
        }
    }
    false
}

pub fn score_skill(task: &str, skill: &SkillMeta) -> f64 {
    let task_lower = task.to_lowercase();
    let task_toks = tokenize(&task_lower);
    if task_toks.is_empty() {
        return 0.0;
    }

    let mut trigger_hit = false;
    let mut trigger_exact = false;
    for trig in &skill.triggers {
        let t_low = trig.to_lowercase();
        if !t_low.is_empty() && task_lower.contains(&t_low) {
            trigger_hit = true;
            trigger_exact = true;
            break;
        }
        let trig_toks = tokenize(&t_low);
        if !trig_toks.is_empty() {
            let overlap = trig_toks
                .iter()
                .filter(|tt| task_toks.iter().any(|t| match_token(tt, t)))
                .count();
            if trig_toks.is_subset(&task_toks)
                || overlap == trig_toks.len()
                || (trig_toks.len() >= 4 && overlap >= trig_toks.len() - 1)
            {
                trigger_hit = true;
                break;
            }
        }
    }

    let name_toks = tokenize(&skill.name);
    let name_overlap = if !name_toks.is_empty() {
        let hits = name_toks
            .iter()
            .filter(|n| task_toks.iter().any(|t| match_token(n, t)))
            .count();
        hits as f64 / name_toks.len() as f64
    } else {
        0.0
    };

    let desc_toks = tokenize(&skill.description);
    let desc_overlap = if !desc_toks.is_empty() {
        let hits = desc_toks
            .iter()
            .filter(|d| task_toks.iter().any(|t| match_token(d, t)))
            .count();
        hits as f64 / desc_toks.len() as f64
    } else {
        0.0
    };

    let mut score = 0.0;
    if trigger_hit {
        score += 0.65;
        if trigger_exact {
            score += 0.10;
        }
    }
    score += name_overlap * 0.45;
    score += desc_overlap * 0.25;

    score.min(1.0)
}

pub fn match_skill(task: &str, skills: &[SkillMeta], threshold: f64) -> MatchResult {
    let mut best_skill: Option<SkillMeta> = None;
    let mut best_score = 0.0;

    for s in skills {
        let sc = score_skill(task, s);
        if sc > best_score {
            best_score = sc;
            best_skill = Some(s.clone());
        }
    }

    let is_match = best_score >= threshold && best_skill.is_some();
    MatchResult {
        skill: best_skill,
        confidence: (best_score * 1000.0).round() / 1000.0,
        is_match,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RemoteSkillCandidate {
    pub package_name: String,
    pub description: String,
    pub author: String,
    pub stars: u64,
    pub downloads: u64,
    pub source_url: String,
}

pub fn find_npx() -> Option<PathBuf> {
    if let Some(paths) = std::env::var_os("PATH") {
        for p in std::env::split_paths(&paths) {
            if cfg!(windows) {
                let c1 = p.join("npx.cmd");
                if c1.is_file() {
                    return Some(c1);
                }
                let c2 = p.join("npx.exe");
                if c2.is_file() {
                    return Some(c2);
                }
                let c3 = p.join("npx.bat");
                if c3.is_file() {
                    return Some(c3);
                }
            } else {
                let c = p.join("npx");
                if c.is_file() {
                    return Some(c);
                }
            }
        }
    }
    None
}

pub fn find_remote_skills(query: &str) -> Vec<RemoteSkillCandidate> {
    if query.trim().is_empty() {
        return Vec::new();
    }

    let npx_bin = match find_npx() {
        Some(b) => b,
        None => {
            eprintln!(
                "find_remote_skills: diagnostic: npx not found on PATH; remote discovery skipped"
            );
            return Vec::new();
        }
    };

    let mut child = match std::process::Command::new(npx_bin)
        .args(["--yes", "skills", "find", query, "--json"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "find_remote_skills: diagnostic: remote query failed ({}); falling back cleanly",
                e
            );
            return Vec::new();
        }
    };

    let start = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(8);
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    eprintln!("find_remote_skills: diagnostic: remote query timed out after 8s; falling back cleanly");
                    break None;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(e) => {
                eprintln!("find_remote_skills: diagnostic: error waiting on npx ({}); falling back cleanly", e);
                break None;
            }
        }
    };

    if let Some(s) = status {
        if s.success() {
            if let Ok(output) = child.wait_with_output() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&stdout) {
                    let mut candidates = Vec::new();
                    let items = if let Some(arr) = val.as_array() {
                        arr.as_slice()
                    } else if let Some(arr) = val.get("skills").and_then(|s| s.as_array()) {
                        arr.as_slice()
                    } else {
                        &[]
                    };
                    for item in items {
                        if let Some(name) = item.get("name").and_then(|n| n.as_str()) {
                            candidates.push(RemoteSkillCandidate {
                                package_name: name.to_string(),
                                description: item
                                    .get("description")
                                    .and_then(|d| d.as_str())
                                    .unwrap_or("")
                                    .to_string(),
                                author: item
                                    .get("author")
                                    .and_then(|a| a.as_str())
                                    .unwrap_or("")
                                    .to_string(),
                                stars: item.get("stars").and_then(|s| s.as_u64()).unwrap_or(0),
                                downloads: item
                                    .get("downloads")
                                    .and_then(|d| d.as_u64())
                                    .unwrap_or(0),
                                source_url: item
                                    .get("url")
                                    .and_then(|u| u.as_str())
                                    .unwrap_or("")
                                    .to_string(),
                            });
                        }
                    }
                    return candidates;
                }
            }
        }
    }

    Vec::new()
}

pub fn install_skill(
    package_name: &str,
    target_dir: Option<&Path>,
    dry_run: bool,
) -> Result<PathBuf, String> {
    let dest_dir = if let Some(td) = target_dir {
        td.to_path_buf()
    } else {
        let slug = package_name
            .trim()
            .trim_start_matches('@')
            .replace('/', "-");
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        cwd.join(".agent").join("skills").join(slug)
    };

    if dry_run {
        return Ok(dest_dir);
    }

    let npx_bin = match find_npx() {
        Some(b) => b,
        None => return Err("npx not found on PATH; cannot install skill".to_string()),
    };

    let staging_base = std::env::temp_dir().join(format!(
        "hive-skill-stage-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let staging_dir = staging_base.join("skill");
    if let Err(e) = fs::create_dir_all(&staging_dir) {
        return Err(format!("failed to create staging directory: {}", e));
    }

    let mut child = match std::process::Command::new(npx_bin)
        .args([
            "--yes",
            "skills",
            "add",
            package_name,
            "--dest",
            &staging_dir.display().to_string(),
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            let _ = fs::remove_dir_all(&staging_base);
            return Err(format!("failed to run npx skills add: {}", e));
        }
    };

    let start = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(30);
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = fs::remove_dir_all(&staging_base);
                    return Err("npx skills add timed out after 30s".to_string());
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(e) => {
                let _ = fs::remove_dir_all(&staging_base);
                return Err(format!("error waiting on npx skills add: {}", e));
            }
        }
    };

    if let Some(s) = status {
        if !s.success() {
            let stderr = child
                .wait_with_output()
                .map(|o| String::from_utf8_lossy(&o.stderr).to_string())
                .unwrap_or_default();
            let _ = fs::remove_dir_all(&staging_base);
            return Err(format!("skills add failed: {}", stderr.trim()));
        }
    } else {
        let _ = fs::remove_dir_all(&staging_base);
        return Err("skills add process failed to execute".to_string());
    }

    let (is_safe, reason) = audit_skill_safety(&staging_dir);
    if !is_safe {
        let _ = fs::remove_dir_all(&staging_base);
        return Err(format!(
            "Security audit failed: {}",
            reason.unwrap_or_default()
        ));
    }

    // Staging and audit passed; copy files into dest_dir safely
    if let Err(e) = fs::create_dir_all(&dest_dir) {
        let _ = fs::remove_dir_all(&staging_base);
        return Err(format!("failed to create skill directory: {}", e));
    }

    for entry in walkdir::WalkDir::new(&staging_dir).into_iter().flatten() {
        if entry.file_type().is_file() {
            let p = entry.path();
            if let Ok(rel) = p.strip_prefix(&staging_dir) {
                let target_p = dest_dir.join(rel);
                if let Some(parent) = target_p.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                let _ = fs::copy(p, target_p);
            }
        }
    }
    let _ = fs::remove_dir_all(&staging_base);

    // Pin package with SHA-256 tree hash in .skill-lock.json
    use sha2::{Digest, Sha256};
    let mut file_entries = Vec::new();
    for entry in walkdir::WalkDir::new(&dest_dir).into_iter().flatten() {
        if entry.file_type().is_file() {
            let p = entry.path();
            if p.file_name().and_then(|n| n.to_str()) == Some(".skill-lock.json") {
                continue;
            }
            if let Ok(bytes) = fs::read(p) {
                let mut hasher = Sha256::new();
                hasher.update(&bytes);
                let hash_hex = format!("{:x}", hasher.finalize());
                let rel = p
                    .strip_prefix(&dest_dir)
                    .map(|r| r.to_string_lossy().replace('\\', "/"))
                    .unwrap_or_default();
                file_entries.push(serde_json::json!({
                    "path": rel,
                    "sha256": hash_hex
                }));
            }
        }
    }
    let lock_data = serde_json::json!({
        "package": package_name,
        "installed_path": dest_dir.display().to_string(),
        "installed_at": chrono::Utc::now().to_rfc3339(),
        "files": file_entries
    });
    let _ = fs::write(
        dest_dir.join(".skill-lock.json"),
        serde_json::to_string_pretty(&lock_data).unwrap_or_default(),
    );

    Ok(dest_dir)
}

pub fn audit_skill_safety(dir_or_file: &Path) -> (bool, Option<String>) {
    let re_eval = Regex::new(r"(?i)eval\s*\(").unwrap();
    let re_curl = Regex::new(r"(?i)curl\s+-[A-Za-z]*d\s+.*https?://").unwrap();
    let re_curl_sh = Regex::new(r"(?i)(?:curl|wget)\s+.*\|\s*(?:ba)?sh\b").unwrap();
    let re_ps = Regex::new(r"(?i)powershell(?:\.exe)?\s+.*(?:-enc|-encodedcommand|invoke-expression|iex)\b|Invoke-Expression\b").unwrap();
    let re_env = Regex::new(r"(?i)\b(?:printenv|Get-ChildItem\s+env:)\b").unwrap();
    let re_rev_shell =
        Regex::new(r"(?i)/dev/tcp/\d+\.\d+\.\d+\.\d+|\b(?:nc|ncat|netcat)\s+-[e|c]\b").unwrap();
    let re_injection = Regex::new(r"(?i)\b(?:ignore|disregard|override)\s+(?:all\s+)?(?:previous|prior)\s+(?:instructions|prompts|rules)\b|\b(?:system\s+prompt\s*:|jailbreak|you\s+are\s+now\s+in\s+developer\s+mode)\b").unwrap();

    let is_trojan_char = |c: char| -> bool {
        matches!(c,
            '\u{202A}'..='\u{202E}' |
            '\u{2066}'..='\u{2069}' |
            '\u{061C}' | '\u{200E}' | '\u{200F}' |
            '\u{200B}'..='\u{200D}'
        )
    };

    let mut paths = Vec::new();
    if dir_or_file.is_file() {
        paths.push(dir_or_file.to_path_buf());
    } else if dir_or_file.is_dir() {
        for entry in walkdir::WalkDir::new(dir_or_file).into_iter().flatten() {
            if entry.file_type().is_file() {
                let ext = entry
                    .path()
                    .extension()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_lowercase();
                if matches!(
                    ext.as_str(),
                    "md" | "py" | "sh" | "bash" | "js" | "ts" | "ps1"
                ) {
                    paths.push(entry.path().to_path_buf());
                }
            }
        }
    }

    for p in paths {
        if let Ok(content) = fs::read_to_string(&p) {
            for (i, c) in content.char_indices() {
                if is_trojan_char(c) {
                    return (
                        false,
                        Some(format!(
                            "Invisible/Trojan Source character U+{:04X} detected in {}",
                            c as u32,
                            p.display()
                        )),
                    );
                }
                if c == '\u{feff}' && i > 0 {
                    return (
                        false,
                        Some(format!(
                            "Hidden zero-width BOM character detected inside text in {}",
                            p.display()
                        )),
                    );
                }
            }

            if re_eval.is_match(&content)
                || re_curl.is_match(&content)
                || re_curl_sh.is_match(&content)
                || re_ps.is_match(&content)
                || re_env.is_match(&content)
                || re_rev_shell.is_match(&content)
                || re_injection.is_match(&content)
            {
                return (
                    false,
                    Some(format!("Dangerous pattern matched in {}", p.display())),
                );
            }
        }
    }
    (true, None)
}

pub fn load_skill_context(file_path: &Path) -> String {
    let cand = if file_path.is_dir() {
        let s1 = file_path.join("SKILL.md");
        if s1.is_file() {
            s1
        } else {
            file_path.join("skill.md")
        }
    } else {
        file_path.to_path_buf()
    };

    if !cand.is_file() {
        return format!(
            "> [!WARNING] Skill instruction file not found: {}",
            cand.display()
        );
    }

    let text = match fs::read_to_string(&cand) {
        Ok(t) => t,
        Err(_) => return format!("> [!WARNING] Failed to read skill file: {}", cand.display()),
    };

    let name = cand
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("skill");

    let lines: Vec<&str> = text.lines().collect();
    let mut body_lines = lines.as_slice();
    if !lines.is_empty() && lines[0].trim() == "---" {
        for (i, &l) in lines.iter().enumerate().skip(1) {
            if l.trim() == "---" {
                body_lines = &lines[i + 1..];
                break;
            }
        }
    }
    let body = body_lines.join("\n").trim().to_string();

    format!(
        "> [!NOTE] Active Agent Skill: {}\n> Source: {}\n> Follow the instructions below to execute the task:\n\n{}",
        name,
        cand.display(),
        body
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_skill_md_and_matching() {
        let tmp = std::env::temp_dir().join(format!("hive_skill_test_{}", std::process::id()));
        let skill_dir = tmp.join("postgres-tools");
        fs::create_dir_all(&skill_dir).unwrap();

        let skill_file = skill_dir.join("SKILL.md");
        fs::write(
            &skill_file,
            "---\nname: postgres-tools\ndescription: Postgres query optimization, explain analyze and schema management\ntriggers:\n  - pg_stat\n  - postgres explain\n---\n# Instructions\nFollow these tips.\n",
        ).unwrap();

        let meta = parse_skill_md(&skill_file, "custom").expect("Failed to parse SKILL.md");
        assert_eq!(meta.name, "postgres-tools");
        assert!(meta.triggers.contains(&"pg_stat".to_string()));

        let skills = vec![meta];

        // Trigger match -> confidence >= 0.65
        let res1 = match_skill("we need to check pg_stat output", &skills, 0.65);
        assert!(res1.is_match);
        assert!(res1.confidence >= 0.65);
        assert_eq!(res1.skill.unwrap().name, "postgres-tools");

        // Keyword overlap match
        let res2 = match_skill("optimize postgres query execution", &skills, 0.40);
        assert!(res2.is_match);

        // Completely unrelated task -> miss
        let res3 = match_skill("how to write css animation keyframes", &skills, 0.65);
        assert!(!res3.is_match);
        assert!(res3.confidence < 0.3);

        let _ = fs::remove_dir_all(&tmp);
    }
}
