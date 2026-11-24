use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LedgerEntry {
    pub ts: String,
    pub session_id: String,
    pub agent: String,
    pub project: String,
    pub event: String,
    pub query: String,
    pub hits: Vec<String>,
    pub injected_bullets: Vec<String>,
    pub duration_ms: f64,
}

pub fn detect_agent() -> String {
    if let Ok(a) = std::env::var("HIVE_AGENT") {
        return a;
    }
    if std::env::var("CLAUDE_PROJECT_DIR").is_ok() || std::env::var("CLAUDE_ENV_FILE").is_ok() {
        return "claude".to_string();
    }
    if std::env::var("CODEX_HOME").is_ok() || std::env::var("CODEX_SESSION").is_ok() {
        return "codex".to_string();
    }
    if std::env::var("ANTIGRAVITY_CONFIG_DIR").is_ok() || std::env::var("GEMINI_CONFIG_DIR").is_ok() {
        return "antigravity".to_string();
    }
    "unknown".to_string()
}

pub fn record(
    ledger_path: &Path,
    event: &str,
    project: &str,
    query: &str,
    hits: &[String],
    injected_bullets: &[String],
    duration_ms: f64,
    agent: Option<&str>,
    session_id: Option<&str>,
) -> bool {
    if let Some(parent) = ledger_path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let entry = LedgerEntry {
        ts: chrono::Utc::now().to_rfc3339(),
        session_id: session_id
            .map(|s| s.to_string())
            .or_else(|| std::env::var("HIVE_SESSION_ID").ok())
            .unwrap_or_default(),
        agent: agent
            .map(|s| s.to_string())
            .unwrap_or_else(detect_agent),
        project: project.to_string(),
        event: event.to_string(),
        query: query.to_string(),
        hits: hits.to_vec(),
        injected_bullets: injected_bullets.to_vec(),
        duration_ms: (duration_ms * 100.0).round() / 100.0,
    };

    let json_line = match serde_json::to_string(&entry) {
        Ok(s) => s + "\n",
        Err(_) => return false,
    };

    match OpenOptions::new()
        .create(true)
        .append(true)
        .open(ledger_path)
    {
        Ok(mut f) => {
            let _ = f.write_all(json_line.as_bytes());
            let _ = f.flush();
            true
        }
        Err(e) => {
            eprintln!("hive ledger: warning: failed to append to {}: {}", ledger_path.display(), e);
            false
        }
    }
}

pub fn read_ledger(
    ledger_path: &Path,
    tail: Option<usize>,
    project: Option<&str>,
) -> Vec<LedgerEntry> {
    if !ledger_path.is_file() {
        return Vec::new();
    }
    let content = match fs::read_to_string(ledger_path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    let mut entries = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(entry) = serde_json::from_str::<LedgerEntry>(trimmed) {
            if let Some(p) = project {
                if entry.project != p {
                    continue;
                }
            }
            entries.push(entry);
        }
    }
    if let Some(n) = tail {
        if n > 0 && entries.len() > n {
            entries = entries.split_off(entries.len() - n);
        }
    }
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_and_read_roundtrip() {
        let tmp = std::env::temp_dir().join(format!("hive_ledger_test_{}.jsonl", std::process::id()));
        let ok = record(
            &tmp,
            "query",
            "ai-hive",
            "search phrase",
            &["notes/test.md".to_string()],
            &["str-00001".to_string()],
            12.34,
            Some("antigravity"),
            Some("session-123"),
        );
        assert!(ok);

        let entries = read_ledger(&tmp, None, None);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].event, "query");
        assert_eq!(entries[0].project, "ai-hive");
        assert_eq!(entries[0].agent, "antigravity");
        assert_eq!(entries[0].session_id, "session-123");
        assert_eq!(entries[0].hits, vec!["notes/test.md"]);
        assert_eq!(entries[0].injected_bullets, vec!["str-00001"]);
        assert_eq!(entries[0].duration_ms, 12.34);

        let _ = fs::remove_file(tmp);
    }
}
