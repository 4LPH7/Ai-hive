use crate::common::validate_slug;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

pub const SECTIONS: &[(&str, &str)] = &[
    ("strategies", "str"),
    ("pitfalls", "pit"),
    ("commands", "cmd"),
    ("conventions", "con"),
];

pub const MAX_BULLET_CHARS: usize = 300;
pub const MAX_ADDS_PER_APPLY: usize = 5;
pub const MAX_BULLETS: usize = 80;
pub const DUP_JACCARD: f64 = 0.75;
pub const PRUNE_MARGIN: i64 = 3;
pub const INJECT_LIMIT: usize = 12;

pub fn get_hostname() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "localhost".to_string())
}

#[cfg(windows)]
pub fn is_pid_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn OpenProcess(
            dwDesiredAccess: u32,
            bInheritHandle: i32,
            dwProcessId: u32,
        ) -> *mut std::ffi::c_void;
        fn CloseHandle(hObject: *mut std::ffi::c_void) -> i32;
    }
    unsafe {
        let handle = OpenProcess(0x1000 | 0x00100000, 0, pid);
        if handle.is_null() {
            return false;
        }
        CloseHandle(handle);
        true
    }
}

#[cfg(unix)]
pub fn is_pid_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    extern "C" {
        fn kill(pid: i32, sig: i32) -> i32;
    }
    unsafe { kill(pid as i32, 0) == 0 }
}

#[derive(Debug)]
pub struct PlaybookLock {
    pub lock_path: PathBuf,
    pub acquired: bool,
}

impl PlaybookLock {
    pub fn acquire(path: &Path, timeout: std::time::Duration) -> Result<Self, String> {
        let lock_path = path.with_extension("lock");
        let start = std::time::Instant::now();
        let pid = std::process::id();
        let hostname = get_hostname();
        let nonce = format!(
            "{:x}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
                % 0xffff_ffff
        );
        let now_iso = chrono::Utc::now().to_rfc3339();
        let payload = format!("{}:{}:{}:{}\n", pid, hostname, now_iso, nonce);

        loop {
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&lock_path)
            {
                Ok(mut file) => {
                    if let Err(err) = file.write_all(payload.as_bytes()) {
                        let _ = fs::remove_file(&lock_path);
                        return Err(format!("failed to write lock payload: {}", err));
                    }
                    return Ok(Self {
                        lock_path,
                        acquired: true,
                    });
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    // Lock exists, will check age / staleness
                }
                Err(_) => {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                    continue;
                }
            }

            let elapsed = start.elapsed();
            if elapsed > timeout {
                if let Ok(content) = fs::read_to_string(&lock_path) {
                    let parts: Vec<&str> = content.trim().split(':').collect();
                    let lock_host = parts.get(1).copied().unwrap_or("");
                    if !lock_host.is_empty() && lock_host != hostname {
                        return Err(format!("playbook locked by foreign host {}", lock_host));
                    }
                }
                return Err(format!(
                    "could not acquire playbook lock after {:?}: held by {}",
                    timeout,
                    lock_path.display()
                ));
            }

            if elapsed > std::time::Duration::from_millis(500) {
                if let Ok(content) = fs::read_to_string(&lock_path) {
                    let parts: Vec<&str> = content.trim().split(':').collect();
                    let lock_pid = parts
                        .first()
                        .and_then(|p| p.parse::<u32>().ok())
                        .unwrap_or(0);
                    let lock_host = parts.get(1).copied().unwrap_or("");
                    let is_stale = lock_host == hostname && lock_pid > 0 && !is_pid_alive(lock_pid);

                    let age = match lock_path.metadata().and_then(|m| m.modified()) {
                        Ok(m) => std::time::SystemTime::now()
                            .duration_since(m)
                            .unwrap_or(std::time::Duration::ZERO),
                        Err(_) => std::time::Duration::ZERO,
                    };

                    if is_stale || age > timeout {
                        if lock_host != hostname && !is_stale {
                            return Err(format!(
                                "playbook locked by foreign host {} (age {:.1?})",
                                lock_host, age
                            ));
                        }
                        let _ = fs::remove_file(&lock_path);
                        continue;
                    }
                }
            }

            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
}

impl Drop for PlaybookLock {
    fn drop(&mut self) {
        if self.acquired {
            let _ = fs::remove_file(&self.lock_path);
        }
    }
}

pub fn is_secret(text: &str) -> bool {
    let re = Regex::new(
        r"(?i)(sk-[A-Za-z0-9_-]{20,}|ghp_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,}|AKIA[0-9A-Z]{16}|xox[abprs]-[A-Za-z0-9-]{10,}|-----BEGIN [A-Z ]*PRIVATE KEY|(?:api[_-]?key|token|password|secret)\s*[:=]\s*\S{8,})"
    ).unwrap();
    re.is_match(text)
}

pub fn scrub_secrets(text: &str) -> String {
    let re = Regex::new(
        r"(?i)(sk-[A-Za-z0-9_-]{20,}|ghp_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,}|AKIA[0-9A-Z]{16}|xox[abprs]-[A-Za-z0-9-]{10,}|-----BEGIN [A-Z ]*PRIVATE KEY|(?:api[_-]?key|token|password|secret)\s*[:=]\s*\S{8,})"
    ).unwrap();
    re.replace_all(text, "[REDACTED_SECRET]").to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Bullet {
    pub id: String,
    #[serde(default)]
    pub seen: i64,
    #[serde(default)]
    pub updated: Option<String>,
    #[serde(default)]
    pub explicit_updated: Option<String>,
    pub helpful: i64,
    pub harmful: i64,
    pub content: String,
}

pub fn bullet_decay(b: &Bullet, today: chrono::NaiveDate) -> i64 {
    if b.helpful > 0 || b.seen < 10 {
        return 0;
    }
    if let Some(up_str) = &b.updated {
        if let Ok(b_date) = chrono::NaiveDate::parse_from_str(up_str, "%Y-%m-%d") {
            let days = (today - b_date).num_days();
            if days > 30 {
                return std::cmp::max(3, (days - 30) / 10 + 1);
            }
        }
    }
    0
}

pub fn tokenize_words(text: &str) -> HashSet<String> {
    let re = Regex::new(r"[^A-Za-z0-9]+").unwrap();
    re.split(text)
        .map(|s| s.to_lowercase())
        .filter(|s| s.len() > 1)
        .collect()
}

pub fn jaccard_similarity(a: &str, b: &str) -> f64 {
    let set_a = tokenize_words(a);
    let set_b = tokenize_words(b);
    if set_a.is_empty() || set_b.is_empty() {
        return 0.0;
    }
    let intersection = set_a.intersection(&set_b).count();
    let union = set_a.union(&set_b).count();
    (intersection as f64) / (union as f64)
}

pub fn scope_path(vault: &Path, scope: &str) -> Result<PathBuf, String> {
    let s = validate_slug(scope)?;
    if s == "global" {
        Ok(vault.join("_meta").join("playbook.md"))
    } else {
        Ok(vault.join("projects").join(&s).join("playbook.md"))
    }
}

pub fn parse(text: &str) -> HashMap<String, Vec<Bullet>> {
    let mut sections: HashMap<String, Vec<Bullet>> = HashMap::new();
    for &(name, _) in SECTIONS {
        sections.insert(name.to_string(), Vec::new());
    }

    let re_line = Regex::new(
        r"^- \[([a-z]{3}-\d{5})\] (?:seen=(\d+)\s+)?(?:updated=(\d{4}-\d{2}-\d{2})\s+)?helpful=(\d+)\s+harmful=(\d+)\s+::\s+(.+)$"
    ).unwrap();
    let re_heading = Regex::new(r"^##\s+(\w+)").unwrap();
    let re_up = Regex::new(r"^updated:\s*(\d{4}-\d{2}-\d{2})").unwrap();

    let mut default_updated: Option<String> = None;
    for line in text.lines() {
        if let Some(cap) = re_up.captures(line.trim()) {
            default_updated = Some(cap[1].to_string());
            break;
        }
    }

    let mut current_section: Option<String> = None;

    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(cap) = re_heading.captures(trimmed) {
            let name = cap[1].to_lowercase();
            if SECTIONS.iter().any(|&(s, _)| s == name) {
                current_section = Some(name);
            } else {
                current_section = None;
            }
            continue;
        }

        if let Some(ref sec) = current_section {
            if let Some(cap) = re_line.captures(trimmed) {
                let id = cap[1].to_string();
                let helpful = cap[4].parse::<i64>().unwrap_or(0);
                let harmful = cap[5].parse::<i64>().unwrap_or(0);
                let seen = cap
                    .get(2)
                    .and_then(|m| m.as_str().parse::<i64>().ok())
                    .unwrap_or(helpful + harmful);
                let explicit_updated = cap.get(3).map(|m| m.as_str().to_string());
                let updated = explicit_updated.clone().or_else(|| default_updated.clone());
                let content = cap[6].trim().to_string();
                sections.get_mut(sec).unwrap().push(Bullet {
                    id,
                    seen,
                    updated,
                    explicit_updated,
                    helpful,
                    harmful,
                    content,
                });
            }
        }
    }

    sections
}

pub fn render(scope: &str, sections: &HashMap<String, Vec<Bullet>>, today: &str) -> String {
    let proj = if scope == "global" { "all" } else { scope };
    let head = format!(
        "---\ntype: note\nproject: {}\nagent: claude\nstatus: active\nupdated: {}\n---\n# Playbook: {}\n\n\
        > ACE playbook. Changed only by `hive ace apply` (skill `hive-ace`).\n\
        > Do not hand-edit: delta merges keep counters and prevent context collapse.\n",
        proj, today, scope
    );

    let mut body = Vec::new();
    for &(name, _) in SECTIONS {
        body.push(format!("\n## {}\n", name));
        if let Some(bullets) = sections.get(name) {
            for b in bullets {
                if let Some(ref up) = b.explicit_updated {
                    body.push(format!(
                        "- [{}] seen={} updated={} helpful={} harmful={} :: {}",
                        b.id,
                        if b.seen > 0 {
                            b.seen
                        } else {
                            b.helpful + b.harmful
                        },
                        up,
                        b.helpful,
                        b.harmful,
                        b.content
                    ));
                } else {
                    body.push(format!(
                        "- [{}] seen={} helpful={} harmful={} :: {}",
                        b.id,
                        if b.seen > 0 {
                            b.seen
                        } else {
                            b.helpful + b.harmful
                        },
                        b.helpful,
                        b.harmful,
                        b.content
                    ));
                }
            }
        }
    }

    format!("{}{}\n", head, body.join("\n"))
}

pub fn load(vault: &Path, scope: &str) -> Result<HashMap<String, Vec<Bullet>>, String> {
    let path = scope_path(vault, scope)?;
    if path.is_file() {
        let content = fs::read_to_string(&path).map_err(|e| e.to_string())?;
        Ok(parse(&content))
    } else {
        let mut sections = HashMap::new();
        for &(name, _) in SECTIONS {
            sections.insert(name.to_string(), Vec::new());
        }
        Ok(sections)
    }
}

pub fn save(
    vault: &Path,
    scope: &str,
    sections: &HashMap<String, Vec<Bullet>>,
    today: Option<&str>,
) -> Result<(), String> {
    let path = scope_path(vault, scope)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let today_str = today
        .map(|t| t.to_string())
        .unwrap_or_else(|| chrono::Local::now().format("%Y-%m-%d").to_string());
    let rendered = render(scope, sections, &today_str);

    let tmp = path.with_extension("tmp");
    fs::write(&tmp, rendered).map_err(|e| e.to_string())?;
    fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
    Ok(())
}

#[derive(Debug, Clone, Deserialize)]
pub struct AddOp {
    pub section: String,
    pub content: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TagOp {
    pub id: String,
    pub helpful: Option<i64>,
    pub harmful: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeltaOps {
    pub add: Option<Vec<AddOp>>,
    pub tag: Option<Vec<TagOp>>,
    pub remove: Option<Vec<String>>,
}

/// Parse a delta in either format: native `{add, tag, remove}` or the `hive-ace` skill / Python format
/// `{operations: [{type: ADD|TAG, ...}], bullet_tags: [{id, tag}]}`. A JSON object with none of these keys
/// is an error, so a format mismatch can never again apply "0 changes" silently.
pub fn parse_delta(json_str: &str) -> Result<DeltaOps, String> {
    let v: serde_json::Value =
        serde_json::from_str(json_str).map_err(|e| format!("invalid JSON: {e}"))?;
    let known = ["add", "tag", "remove", "operations", "bullet_tags"];
    if !v.is_object() || !known.iter().any(|k| v.get(k).is_some()) {
        return Err(format!("delta has none of the keys {known:?}"));
    }
    let mut ops: DeltaOps = serde_json::from_value(serde_json::json!({
        "add": v.get("add"), "tag": v.get("tag"), "remove": v.get("remove"),
    }))
    .map_err(|e| format!("bad native delta: {e}"))?;
    let mut adds = ops.add.take().unwrap_or_default();
    let mut tags = ops.tag.take().unwrap_or_default();
    let list = |k: &str| {
        v.get(k)
            .and_then(|x| x.as_array())
            .cloned()
            .unwrap_or_default()
    };
    for op in list("operations").iter().chain(list("bullet_tags").iter()) {
        let field = |k: &str| op.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
        let kind = op
            .get("type")
            .and_then(|t| t.as_str())
            .unwrap_or("TAG")
            .to_ascii_uppercase();
        match kind.as_str() {
            "ADD" => adds.push(AddOp {
                section: field("section"),
                content: field("content"),
            }),
            "TAG" => match field("tag").as_str() {
                "helpful" => tags.push(TagOp {
                    id: field("id"),
                    helpful: Some(1),
                    harmful: None,
                }),
                "harmful" => tags.push(TagOp {
                    id: field("id"),
                    helpful: None,
                    harmful: Some(1),
                }),
                _ => {} // neutral
            },
            other => return Err(format!("unknown operation type {other:?}")),
        }
    }
    ops.add = (!adds.is_empty()).then_some(adds);
    ops.tag = (!tags.is_empty()).then_some(tags);
    Ok(ops)
}

pub fn apply(sections: &mut HashMap<String, Vec<Bullet>>, ops: &DeltaOps) -> Result<usize, String> {
    let mut changed = 0usize;

    // 1. Process TAG
    if let Some(tags) = &ops.tag {
        for t in tags {
            let mut found = false;
            for bullets in sections.values_mut() {
                if let Some(b) = bullets.iter_mut().find(|b| b.id == t.id) {
                    found = true;
                    if let Some(h) = t.helpful {
                        b.helpful += h;
                        b.seen = b.seen.max(b.helpful + b.harmful);
                        changed += 1;
                    }
                    if let Some(h) = t.harmful {
                        b.harmful += h;
                        b.seen = b.seen.max(b.helpful + b.harmful);
                        changed += 1;
                    }
                }
            }
            if !found && t.id.starts_with("skill:") {
                let tag_str = if t.helpful.unwrap_or(0) > 0 {
                    "helpful"
                } else {
                    "harmful"
                };
                let skill_name = &t.id[6..];
                let _ = crate::ledger::record(
                    &crate::ledger::default_ledger_path(),
                    "skill_feedback",
                    skill_name,
                    tag_str,
                    &[],
                    &[],
                    0.0,
                    None,
                    None,
                );
                changed += 1;
            }
        }
    }

    // 2. Process REMOVE
    if let Some(removes) = &ops.remove {
        let remove_set: HashSet<&str> = removes.iter().map(|s| s.as_str()).collect();
        for bullets in sections.values_mut() {
            let before = bullets.len();
            bullets.retain(|b| !remove_set.contains(b.id.as_str()));
            changed += before - bullets.len();
        }
    }

    // 3. Process ADD
    if let Some(adds) = &ops.add {
        let add_slice = if adds.len() > MAX_ADDS_PER_APPLY {
            &adds[..MAX_ADDS_PER_APPLY]
        } else {
            adds.as_slice()
        };

        for a in add_slice {
            let sec_name = a.section.to_lowercase();
            if !SECTIONS.iter().any(|&(s, _)| s == sec_name) {
                continue;
            }
            let content = a.content.trim();
            if content.is_empty() || content.len() > MAX_BULLET_CHARS || is_secret(content) {
                continue;
            }

            let bullets = sections.get_mut(&sec_name).unwrap();

            // Deduplication via Jaccard: reinforce seen counter, NOT helpful
            if let Some(existing) = bullets
                .iter_mut()
                .find(|b| jaccard_similarity(&b.content, content) >= DUP_JACCARD)
            {
                existing.seen += 1;
                changed += 1;
                continue;
            }

            // Generate new ID
            let prefix = SECTIONS.iter().find(|&&(s, _)| s == sec_name).unwrap().1;
            let mut next_num = 1;
            for b in bullets.iter() {
                if let Some(num_str) = b.id.strip_prefix(&format!("{}-", prefix)) {
                    if let Ok(num) = num_str.parse::<usize>() {
                        if num >= next_num {
                            next_num = num + 1;
                        }
                    }
                }
            }
            let new_id = format!("{}-{:05}", prefix, next_num);
            let today_str = chrono::Local::now().format("%Y-%m-%d").to_string();
            bullets.push(Bullet {
                id: new_id,
                seen: 1,
                updated: Some(today_str),
                explicit_updated: None,
                helpful: 0,
                harmful: 0,
                content: content.to_string(),
            });
            changed += 1;
        }
    }

    // 4. Prune harmful and decayed bullets
    let today_naive = chrono::Local::now().date_naive();
    for bullets in sections.values_mut() {
        bullets.retain(|b| {
            let decay = bullet_decay(b, today_naive);
            b.harmful + decay < b.helpful + PRUNE_MARGIN
        });
    }

    // 5. Enforce MAX_BULLETS cap
    let total_bullets: usize = sections.values().map(|v| v.len()).sum();
    if total_bullets > MAX_BULLETS {
        let mut all: Vec<(String, Bullet)> = Vec::new();
        for (sec, bullets) in sections.iter() {
            for b in bullets {
                all.push((sec.clone(), b.clone()));
            }
        }
        all.sort_by_key(|a| std::cmp::Reverse(a.1.helpful - a.1.harmful));
        all.truncate(MAX_BULLETS);

        for bullets in sections.values_mut() {
            bullets.clear();
        }
        for (sec, b) in all {
            sections.get_mut(&sec).unwrap().push(b);
        }
    }

    Ok(changed)
}

pub fn apply_with_lock(vault: &Path, scope: &str, ops: &DeltaOps) -> Result<usize, String> {
    let path = scope_path(vault, scope)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let _lock = PlaybookLock::acquire(&path, std::time::Duration::from_secs(30))?;
    let mut sections = load(vault, scope)?;
    let changed = apply(&mut sections, ops)?;
    save(vault, scope, &sections, None)?;
    Ok(changed)
}

pub fn select_bullets(
    sections: &HashMap<String, Vec<Bullet>>,
    task: &str,
    limit: usize,
) -> Vec<Bullet> {
    let mut all: Vec<Bullet> = sections.values().flatten().cloned().collect();
    if task.is_empty() {
        all.sort_by_key(|a| std::cmp::Reverse(a.helpful - a.harmful));
    } else {
        all.sort_by(|a, b| {
            let sc_a = (a.helpful - a.harmful) as f64 + jaccard_similarity(&a.content, task) * 5.0;
            let sc_b = (b.helpful - b.harmful) as f64 + jaccard_similarity(&b.content, task) * 5.0;
            sc_b.partial_cmp(&sc_a).unwrap_or(std::cmp::Ordering::Equal)
        });
    }
    all.truncate(limit);
    all
}

pub fn format_bullets(bullets: &[Bullet]) -> String {
    bullets
        .iter()
        .map(|b| {
            format!(
                "- [{}] helpful={} harmful={} :: {}",
                b.id, b.helpful, b.harmful, b.content
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_secret_detection() {
        assert!(is_secret(
            "sk-ant-api03-abcdefghijklmnopqrstuvwxyz123456789"
        ));
        assert!(is_secret("ghp_123456789012345678901234567890123456"));
        assert!(!is_secret("use cargo test to run unit tests"));
    }

    #[test]
    fn test_parse_delta_accepts_skill_format_and_rejects_unknown() {
        let skill = r#"{"operations":[{"type":"ADD","section":"pitfalls","content":"x"}],
                        "bullet_tags":[{"id":"cmd-00003","tag":"helpful"},{"id":"pit-00001","tag":"neutral"}]}"#;
        let ops = parse_delta(skill).unwrap();
        assert_eq!(ops.add.as_ref().unwrap().len(), 1);
        let tags = ops.tag.unwrap();
        assert_eq!(
            (tags.len(), tags[0].id.as_str(), tags[0].helpful),
            (1, "cmd-00003", Some(1))
        );

        let native =
            r#"{"add":[{"section":"commands","content":"y"}],"tag":[{"id":"a","harmful":1}]}"#;
        let ops = parse_delta(native).unwrap();
        assert_eq!(ops.add.unwrap()[0].section, "commands");
        assert_eq!(ops.tag.unwrap()[0].harmful, Some(1));

        assert!(parse_delta(r#"{"adds":[]}"#).is_err());
        assert!(parse_delta(r#"{"operations":[{"type":"DELETE"}]}"#).is_err());
    }

    #[test]
    fn test_apply_and_deduplication() {
        let mut sections: HashMap<String, Vec<Bullet>> = HashMap::new();
        for &(name, _) in SECTIONS {
            sections.insert(name.to_string(), Vec::new());
        }

        let ops = DeltaOps {
            add: Some(vec![AddOp {
                section: "strategies".to_string(),
                content: "Always check memory before refactoring".to_string(),
            }]),
            tag: None,
            remove: None,
        };

        apply(&mut sections, &ops).unwrap();
        assert_eq!(sections["strategies"].len(), 1);
        assert_eq!(sections["strategies"][0].id, "str-00001");

        // Add near-duplicate -> reinforces existing
        let dup_ops = DeltaOps {
            add: Some(vec![AddOp {
                section: "strategies".to_string(),
                content: "Always check memory before any refactoring".to_string(),
            }]),
            tag: None,
            remove: None,
        };
        apply(&mut sections, &dup_ops).unwrap();
        assert_eq!(sections["strategies"].len(), 1);
        assert_eq!(sections["strategies"][0].seen, 2);
        assert_eq!(sections["strategies"][0].helpful, 0);
    }
}
