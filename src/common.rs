use std::path::{Path, PathBuf};
use regex::Regex;
use chrono::NaiveDate;

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

pub fn default_vault() -> PathBuf {
    std::env::var("HIVE_VAULT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(r"D:\Valut\OBS"))
}

pub fn default_projects() -> PathBuf {
    std::env::var("HIVE_PROJECTS")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(r"D:\PROJECTS"))
}

pub fn default_db_path() -> PathBuf {
    if let Ok(p) = std::env::var("HIVE_DB") {
        return PathBuf::from(p);
    }
    let base = std::env::var("LOCALAPPDATA")
        .or_else(|_| std::env::var("TEMP"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    base.join("hive").join("hive.sqlite")
}

#[cfg(test)]
mod tests {
    use super::*;

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
