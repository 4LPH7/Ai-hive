use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use regex::Regex;
use chrono::NaiveDate;
use sha2::{Digest, Sha256};

pub fn sha256(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

pub fn slugify(name: &str) -> String {
    name.trim().to_lowercase().replace(' ', "-")
}

pub fn validate_slug(name: &str) -> Result<String, String> {
    let s = slugify(name);
    let re = Regex::new(r"^[a-z0-9._-]+$").unwrap();
    if !re.is_match(&s) || s == "." || s == ".." {
        return Err(format!("bad slug {:?}", name));
    }
    Ok(s)
}

pub fn canonical_path(p: &Path) -> PathBuf {
    match p.canonicalize() {
        Ok(c) => {
            let s = c.to_string_lossy().to_lowercase();
            PathBuf::from(s.trim_start_matches(r"\\?\"))
        }
        Err(_) => PathBuf::from(p.to_string_lossy().to_lowercase()),
    }
}

pub fn is_relative_to(path: &Path, base: &Path) -> bool {
    let p = canonical_path(path);
    let b = canonical_path(base);
    p.starts_with(b)
}

pub fn est_tokens(text: &str) -> usize {
    if text.is_empty() {
        return 0;
    }
    let mut code_chars = 0usize;
    let mut prose_chars = 0usize;
    let mut in_fence = false;

    for line in text.lines() {
        let stripped = line.trim();
        if stripped.starts_with("```") || stripped.starts_with("~~~") {
            in_fence = !in_fence;
            code_chars += line.len() + 1;
            continue;
        }
        if in_fence
            || line.starts_with("    ")
            || line.starts_with('\t')
            || (!stripped.is_empty() && (stripped.ends_with('{') || stripped.ends_with('}') || stripped.ends_with(';')))
        {
            code_chars += line.len() + 1;
        } else {
            prose_chars += line.len() + 1;
        }
    }

    let toks = ((code_chars as f64) / 3.4 + (prose_chars as f64) / 4.2) * 1.08;
    toks.ceil() as usize
}

pub fn note_age(updated: &str) -> String {
    if updated.is_empty() {
        return "unknown age".to_string();
    }
    if let Ok(d) = NaiveDate::parse_from_str(updated, "%Y-%m-%d") {
        let today = chrono::Local::now().date_naive();
        let days = (today - d).num_days();
        if days <= 0 {
            "today".to_string()
        } else if days == 1 {
            "1d ago".to_string()
        } else {
            format!("{}d ago", days)
        }
    } else {
        updated.to_string()
    }
}

pub fn active_work_cell_matches(cell: &str, project_slug: &str) -> bool {
    let clean = cell.trim().trim_matches('[').trim_matches(']').trim();
    let candidates = if clean.contains('|') {
        let parts: Vec<&str> = clean.splitn(2, '|').collect();
        let path = parts[0].trim();
        let alias = parts[1].trim();
        let path_proj = if path.starts_with("projects/") {
            path.split('/').nth(1).unwrap_or(path)
        } else {
            path
        };
        vec![slugify(alias), slugify(path_proj)]
    } else {
        let path_proj = if clean.starts_with("projects/") {
            clean.split('/').nth(1).unwrap_or(clean)
        } else {
            clean
        };
        vec![slugify(path_proj)]
    };
    candidates.iter().any(|c| c == project_slug)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HiveConfig {
    pub vault: PathBuf,
    pub projects: PathBuf,
    pub db: PathBuf,
    pub ledger: PathBuf,
}

pub fn parse_toml_config(content: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let mut current_section = String::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            let sec = trimmed[1..trimmed.len() - 1].trim().to_lowercase();
            current_section = if sec.is_empty() || sec == "hive" {
                String::new()
            } else {
                format!("{}.", sec)
            };
            continue;
        }
        if let Some((k, v)) = trimmed.split_once('=') {
            let raw_k = k.trim().to_lowercase();
            let val = v.trim().trim_matches('"').trim_matches('\'').trim().to_string();
            if current_section.is_empty() {
                map.insert(raw_k, val);
            } else {
                let scoped_key = format!("{}{}", current_section, raw_k);
                map.insert(scoped_key, val.clone());
                map.entry(raw_k).or_insert(val);
            }
        }
    }
    map
}

pub fn find_default_config_path() -> Option<PathBuf> {
    if let Ok(appdata) = std::env::var("APPDATA") {
        let p = PathBuf::from(appdata).join("hive").join("config.toml");
        if p.is_file() {
            return Some(p);
        }
    }
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        let p = PathBuf::from(xdg).join("hive").join("config.toml");
        if p.is_file() {
            return Some(p);
        }
    }
    let home = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).ok();
    if let Some(h) = home {
        let p = PathBuf::from(h).join(".config").join("hive").join("config.toml");
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

pub fn load_config(
    cli_vault: Option<PathBuf>,
    cli_projects: Option<PathBuf>,
    cli_db: Option<PathBuf>,
    cli_config: Option<&Path>,
) -> HiveConfig {
    let config_path = cli_config
        .map(PathBuf::from)
        .or_else(find_default_config_path);

    let toml_data = if let Some(cp) = config_path {
        fs::read_to_string(cp)
            .map(|c| parse_toml_config(&c))
            .unwrap_or_default()
    } else {
        HashMap::new()
    };

    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".to_string());

    let vault = cli_vault
        .or_else(|| std::env::var("HIVE_VAULT").ok().map(PathBuf::from))
        .or_else(|| toml_data.get("vault").map(PathBuf::from))
        .unwrap_or_else(|| {
            if cfg!(windows) && Path::new(r"D:\Valut\OBS").exists() {
                PathBuf::from(r"D:\Valut\OBS")
            } else {
                let doc = PathBuf::from(&home).join("Documents").join("Vault");
                if doc.exists() {
                    doc
                } else {
                    PathBuf::from(&home).join("Documents").join("Vault")
                }
            }
        });

    let projects = cli_projects
        .or_else(|| std::env::var("HIVE_PROJECTS").ok().map(PathBuf::from))
        .or_else(|| toml_data.get("projects").map(PathBuf::from))
        .unwrap_or_else(|| {
            if cfg!(windows) && Path::new(r"D:\PROJECTS").exists() {
                PathBuf::from(r"D:\PROJECTS")
            } else {
                PathBuf::from(&home).join("Projects")
            }
        });

    let db = cli_db
        .or_else(|| std::env::var("HIVE_DB").ok().map(PathBuf::from))
        .or_else(|| toml_data.get("db").map(PathBuf::from))
        .unwrap_or_else(|| {
            if cfg!(windows) {
                let base = std::env::var("LOCALAPPDATA")
                    .or_else(|_| std::env::var("TEMP"))
                    .map(PathBuf::from)
                    .unwrap_or_else(|_| PathBuf::from("."));
                base.join("hive").join("hive.sqlite")
            } else {
                let cache = std::env::var("XDG_CACHE_HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(|_| PathBuf::from(&home).join(".cache"));
                cache.join("hive").join("hive.sqlite")
            }
        });

    let ledger = std::env::var("HIVE_LEDGER")
        .ok()
        .map(PathBuf::from)
        .or_else(|| toml_data.get("ledger").map(PathBuf::from))
        .unwrap_or_else(|| {
            if cfg!(windows) {
                let base = std::env::var("LOCALAPPDATA")
                    .or_else(|_| std::env::var("TEMP"))
                    .map(PathBuf::from)
                    .unwrap_or_else(|_| PathBuf::from("."));
                base.join("hive").join("ledger.jsonl")
            } else {
                let cache = std::env::var("XDG_CACHE_HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(|_| PathBuf::from(&home).join(".cache"));
                cache.join("hive").join("ledger.jsonl")
            }
        });

    HiveConfig { vault, projects, db, ledger }
}

pub fn default_vault() -> PathBuf {
    load_config(None, None, None, None).vault
}

pub fn default_projects() -> PathBuf {
    load_config(None, None, None, None).projects
}

pub fn default_db_path() -> PathBuf {
    load_config(None, None, None, None).db
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sha256_vectors() {
        assert_eq!(
            sha256(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256(b"hello world"),
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }

    #[test]
    fn test_parse_toml_config() {
        let content = r#"
        # Sample configuration
        [hive]
        vault = "/custom/vault"
        projects = "/custom/projects"
        db = "/custom/db.sqlite"
        "#;
        let map = parse_toml_config(content);
        assert_eq!(map.get("vault").unwrap(), "/custom/vault");
        assert_eq!(map.get("projects").unwrap(), "/custom/projects");
        assert_eq!(map.get("db").unwrap(), "/custom/db.sqlite");
    }

    #[test]
    fn test_load_config_precedence() {
        let toml_content = r#"
        vault = "/toml/vault"
        projects = "/toml/projects"
        db = "/toml/db"
        "#;
        let tmp = std::env::temp_dir().join("test_hive_config.toml");
        fs::write(&tmp, toml_content).unwrap();

        // 1. TOML fallback when no CLI or ENV
        let cfg1 = load_config(None, None, None, Some(&tmp));
        assert_eq!(cfg1.vault, PathBuf::from("/toml/vault"));
        assert_eq!(cfg1.projects, PathBuf::from("/toml/projects"));

        // 2. CLI overrides TOML
        let cfg2 = load_config(Some(PathBuf::from("/cli/vault")), None, None, Some(&tmp));
        assert_eq!(cfg2.vault, PathBuf::from("/cli/vault"));
        assert_eq!(cfg2.projects, PathBuf::from("/toml/projects"));

        let _ = fs::remove_file(tmp);
    }

    #[test]
    fn test_slug_and_traversal() {
        assert_eq!(validate_slug("ai hive").unwrap(), "ai-hive");
        assert_eq!(validate_slug("my_proj.1").unwrap(), "my_proj.1");
        assert!(validate_slug("../../etc").is_err());
        assert!(validate_slug(".").is_err());
        assert!(validate_slug("..").is_err());
        assert!(validate_slug("a/b").is_err());
    }

    #[test]
    fn test_token_estimator() {
        assert_eq!(est_tokens(""), 0);
        let prose = "This is a regular prose sentence describing memory architecture in the hive.";
        let toks = est_tokens(prose);
        assert!(toks > 0 && toks < 40);

        let code = "```rust\nfn main() {\n    println!(\"hello\");\n}\n```";
        let c_toks = est_tokens(code);
        assert!(c_toks > 0);
    }

    #[test]
    fn test_active_work_cell_matches() {
        assert!(active_work_cell_matches("[[projects/ai-hive/_memory|ai-hive]]", "ai-hive"));
        assert!(active_work_cell_matches("[[projects/ai-hive/_memory]]", "ai-hive"));
        assert!(active_work_cell_matches("projects/ai-hive", "ai-hive"));
        assert!(!active_work_cell_matches("[[projects/fpolink-api/_memory|api]]", "fpolink"));
        assert!(active_work_cell_matches("[[projects/fpolink-api/_memory|api]]", "api"));
    }
}
