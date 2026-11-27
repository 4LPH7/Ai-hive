use chrono::NaiveDate;
use regex::Regex;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

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
            let s = c.to_string_lossy();
            let trimmed = s.trim_start_matches(r"\\?\");
            if cfg!(windows) {
                PathBuf::from(trimmed.to_lowercase())
            } else {
                PathBuf::from(trimmed)
            }
        }
        Err(_) => {
            if cfg!(windows) {
                PathBuf::from(p.to_string_lossy().to_lowercase())
            } else {
                p.to_path_buf()
            }
        }
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

    // Parity with hive.est_tokens (`splitlines(keepends=True)` + `len`): each line counts its characters (not
    // UTF-8 bytes) plus its own line ending only when it has one, so a final line without "\n" adds nothing.
    for line in text.split_inclusive('\n') {
        let stripped = line.trim();
        let n = line.chars().count();
        if stripped.starts_with("```") || stripped.starts_with("~~~") {
            in_fence = !in_fence;
            code_chars += n;
            continue;
        }
        if in_fence
            || line.starts_with("    ")
            || line.starts_with('\t')
            || (!stripped.is_empty()
                && (stripped.ends_with('{') || stripped.ends_with('}') || stripped.ends_with(';')))
        {
            code_chars += n;
        } else {
            prose_chars += n;
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

/// Quote an agent-written note so the reader treats its text as data, never as instructions.
pub fn frame_untrusted(
    rel: &str,
    meta: &std::collections::HashMap<String, String>,
    body: &str,
) -> String {
    let get = |k: &str, d: &'static str| meta.get(k).map(String::as_str).unwrap_or(d).to_string();
    let header = format!(
        "> [!NOTE] Untrusted vault note: path=\"{}\", agent=\"{}\", source=\"{}\", age=\"{}\"",
        rel,
        get("agent", "unknown"),
        get("source", "unknown"),
        note_age(&get("updated", ""))
    );
    let quoted: Vec<String> = body.trim().lines().map(|l| format!("> {l}")).collect();
    format!("{header}\n{}", quoted.join("\n"))
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

pub fn strip_inline_comment(s: &str) -> &str {
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;
    for (i, c) in s.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if c == '\\' && in_double {
            escaped = true;
            continue;
        }
        if c == '\'' && !in_double {
            in_single = !in_single;
        } else if c == '"' && !in_single {
            in_double = !in_double;
        } else if c == '#' && !in_single && !in_double {
            return &s[..i];
        }
    }
    s
}

pub fn unquote(s: &str) -> String {
    let trimmed = s.trim();
    if (trimmed.starts_with('"') && trimmed.ends_with('"') && trimmed.len() >= 2)
        || (trimmed.starts_with('\'') && trimmed.ends_with('\'') && trimmed.len() >= 2)
    {
        trimmed[1..trimmed.len() - 1].to_string()
    } else {
        trimmed.to_string()
    }
}

pub fn parse_toml_config(content: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let mut current_section = String::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let stripped = strip_inline_comment(trimmed).trim();
        if stripped.is_empty() {
            continue;
        }
        if stripped.starts_with('[') && stripped.ends_with(']') {
            let sec = stripped[1..stripped.len() - 1].trim().to_lowercase();
            current_section = if sec.is_empty() || sec == "hive" {
                String::new()
            } else {
                format!("{}.", sec)
            };
            continue;
        }
        if let Some((k, v)) = stripped.split_once('=') {
            let raw_k = k.trim().to_lowercase();
            let val = unquote(v);
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
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .ok();
    if let Some(h) = home {
        let p = PathBuf::from(h)
            .join(".config")
            .join("hive")
            .join("config.toml");
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
    let toml_data = if let Some(explicit) = cli_config {
        match fs::read_to_string(explicit) {
            Ok(c) => parse_toml_config(&c),
            Err(e) => {
                eprintln!(
                    "hive: error reading specified config {}: {}",
                    explicit.display(),
                    e
                );
                HashMap::new()
            }
        }
    } else if let Some(cp) = find_default_config_path() {
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

    HiveConfig {
        vault,
        projects,
        db,
        ledger,
    }
}

static ACTIVE: std::sync::OnceLock<HiveConfig> = std::sync::OnceLock::new();

/// Set once by the binary's `main` right after the CLI/env/config merge. Every default path below reads this
/// one value, so a `--vault`, `--projects`, `--db` or `--config` override reaches every writer (ledger, index,
/// artifacts, graph roots), not only the code path that received the parsed config.
pub fn set_config(cfg: HiveConfig) {
    let _ = ACTIVE.set(cfg);
}

/// The config in effect: the one `set_config` installed, else the config resolved from env and files.
pub fn active_config() -> HiveConfig {
    ACTIVE
        .get()
        .cloned()
        .unwrap_or_else(|| load_config(None, None, None, None))
}

pub fn default_vault() -> PathBuf {
    active_config().vault
}

pub fn default_projects() -> PathBuf {
    active_config().projects
}

pub fn default_db_path() -> PathBuf {
    active_config().db
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
        vault = "/custom/vault" # Trailing inline comment
        projects = "/custom/projects"
        db = "/custom/db.sqlite"

        [llm]
        default_privacy = "private" # "private" forces hive-private unless overridden
        url = "http://127.0.0.1:20128/v1#anchor" # inline comment with hash in url
        "#;
        let map = parse_toml_config(content);
        assert_eq!(map.get("vault").unwrap(), "/custom/vault");
        assert_eq!(map.get("projects").unwrap(), "/custom/projects");
        assert_eq!(map.get("db").unwrap(), "/custom/db.sqlite");
        assert_eq!(map.get("llm.default_privacy").unwrap(), "private");
        assert_eq!(
            map.get("llm.url").unwrap(),
            "http://127.0.0.1:20128/v1#anchor"
        );
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

    /// Values from hive.est_tokens in Python for the same strings (cross-language parity, incl. no trailing
    /// newline, CRLF and non-ASCII).
    #[test]
    fn test_token_estimator_matches_python() {
        let cases: [(&str, usize); 14] = [
            ("", 0),
            ("a", 1),
            ("abc\n", 2),
            ("xxxxxxxxxx", 3),
            ("xxxxxxxxxx\n", 3),
            ("fn main() {", 4),
            ("let x = 1;", 4),
            ("```\ncode\n```", 4),
            ("    indented code\nprose line", 9),
            ("héllo wörld ✓ naïve café", 7),
            ("a\r\nb", 2),
            ("line one\nline two\nline three", 8),
            ("2026-11-26T10:00:00 INFO worker 0 processed batch 0 ok", 14),
            ("{\n  \"k\": 1\n}\n", 4),
        ];
        for (text, want) in cases {
            assert_eq!(est_tokens(text), want, "{text:?}");
        }
    }

    #[test]
    fn test_active_work_cell_matches() {
        assert!(active_work_cell_matches(
            "[[projects/ai-hive/_memory|ai-hive]]",
            "ai-hive"
        ));
        assert!(active_work_cell_matches(
            "[[projects/ai-hive/_memory]]",
            "ai-hive"
        ));
        assert!(active_work_cell_matches("projects/ai-hive", "ai-hive"));
        assert!(!active_work_cell_matches(
            "[[projects/fpolink-api/_memory|api]]",
            "fpolink"
        ));
        assert!(active_work_cell_matches(
            "[[projects/fpolink-api/_memory|api]]",
            "api"
        ));
    }
}
