use crate::common::validate_slug;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
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

pub fn is_secret(text: &str) -> bool {
    let re = Regex::new(
        r"(?i)(sk-[A-Za-z0-9_-]{20,}|ghp_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,}|AKIA[0-9A-Z]{16}|xox[abprs]-[A-Za-z0-9-]{10,}|-----BEGIN [A-Z ]*PRIVATE KEY|(?:api[_-]?key|token|password|secret)\s*[:=]\s*\S{8,})"
    ).unwrap();
    re.is_match(text)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Bullet {
    pub id: String,
    pub helpful: i64,
    pub harmful: i64,
    pub content: String,
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

    let re_line = Regex::new(r"^- \[([a-z]{3}-\d{5})\] helpful=(\d+) harmful=(\d+) :: (.+)$").unwrap();
    let re_heading = Regex::new(r"^##\s+(\w+)").unwrap();

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
                let helpful = cap[2].parse::<i64>().unwrap_or(0);
                let harmful = cap[3].parse::<i64>().unwrap_or(0);
                let content = cap[4].trim().to_string();
                sections.get_mut(sec).unwrap().push(Bullet {
                    id,
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
                body.push(format!(
                    "- [{}] helpful={} harmful={} :: {}",
                    b.id, b.helpful, b.harmful, b.content
                ));
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

pub fn save(vault: &Path, scope: &str, sections: &HashMap<String, Vec<Bullet>>, today: Option<&str>) -> Result<(), String> {
    let path = scope_path(vault, scope)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let today_str = today.map(|t| t.to_string()).unwrap_or_else(|| {
        chrono::Local::now().format("%Y-%m-%d").to_string()
    });
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

pub fn apply(
    sections: &mut HashMap<String, Vec<Bullet>>,
    ops: &DeltaOps,
) -> Result<usize, String> {
    let mut changed = 0usize;

    // 1. Process TAG
    if let Some(tags) = &ops.tag {
        for t in tags {
            for bullets in sections.values_mut() {
                if let Some(b) = bullets.iter_mut().find(|b| b.id == t.id) {
                    if let Some(h) = t.helpful {
                        b.helpful += h;
                        changed += 1;
                    }
                    if let Some(h) = t.harmful {
                        b.harmful += h;
                        changed += 1;
                    }
                }
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

            // Deduplication via Jaccard
            if let Some(existing) = bullets.iter_mut().find(|b| jaccard_similarity(&b.content, content) >= DUP_JACCARD) {
                existing.helpful += 1;
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
            bullets.push(Bullet {
                id: new_id,
                helpful: 1,
                harmful: 0,
                content: content.to_string(),
            });
            changed += 1;
        }
    }

    // 4. Prune harmful bullets
    for bullets in sections.values_mut() {
        bullets.retain(|b| b.harmful < b.helpful + PRUNE_MARGIN);
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
        all.sort_by(|a, b| (b.1.helpful - b.1.harmful).cmp(&(a.1.helpful - a.1.harmful)));
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

pub fn select_bullets(
    sections: &HashMap<String, Vec<Bullet>>,
    task: &str,
    limit: usize,
) -> Vec<Bullet> {
    let mut all: Vec<Bullet> = sections.values().flatten().cloned().collect();
    if task.is_empty() {
        all.sort_by(|a, b| (b.helpful - b.harmful).cmp(&(a.helpful - a.harmful)));
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
        .map(|b| format!("- [{}] helpful={} harmful={} :: {}", b.id, b.helpful, b.harmful, b.content))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_secret_detection() {
        assert!(is_secret("sk-ant-api03-abcdefghijklmnopqrstuvwxyz123456789"));
        assert!(is_secret("ghp_123456789012345678901234567890123456"));
        assert!(!is_secret("use cargo test to run unit tests"));
    }

    #[test]
    fn test_apply_and_deduplication() {
        let mut sections: HashMap<String, Vec<Bullet>> = HashMap::new();
        for &(name, _) in SECTIONS {
            sections.insert(name.to_string(), Vec::new());
        }

        let ops = DeltaOps {
            add: Some(vec![
                AddOp { section: "strategies".to_string(), content: "Always check memory before refactoring".to_string() }
            ]),
            tag: None,
            remove: None,
        };

        apply(&mut sections, &ops).unwrap();
        assert_eq!(sections["strategies"].len(), 1);
        assert_eq!(sections["strategies"][0].id, "str-00001");

        // Add near-duplicate -> reinforces existing
        let dup_ops = DeltaOps {
            add: Some(vec![
                AddOp { section: "strategies".to_string(), content: "Always check memory before any refactoring".to_string() }
            ]),
            tag: None,
            remove: None,
        };
        apply(&mut sections, &dup_ops).unwrap();
        assert_eq!(sections["strategies"].len(), 1);
        assert_eq!(sections["strategies"][0].helpful, 2);
    }
}
