use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

pub fn default_ledger_path() -> PathBuf {
    crate::common::load_config(None, None, None, None).ledger
}

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
    if std::env::var("ANTIGRAVITY_CONFIG_DIR").is_ok() || std::env::var("GEMINI_CONFIG_DIR").is_ok()
    {
        return "antigravity".to_string();
    }
    "unknown".to_string()
}

#[allow(clippy::too_many_arguments)]
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
        agent: agent.map(|s| s.to_string()).unwrap_or_else(detect_agent),
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
            eprintln!(
                "hive ledger: warning: failed to append to {}: {}",
                ledger_path.display(),
                e
            );
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoredSessionResult {
    pub session_id: String,
    pub scored: bool,
    pub reward: i64,
    pub test_exit_code: i32,
    pub bullets: Vec<String>,
    pub skills: Vec<String>,
    pub scope: String,
    pub changes: usize,
}

pub fn score_session(
    ledger_path: &Path,
    vault_path: &Path,
    session_id: Option<&str>,
    test_exit_code: i32,
    project: Option<&str>,
    _repo_dir: Option<&Path>,
) -> Result<ScoredSessionResult, String> {
    let entries = read_ledger(ledger_path, None, None);
    if entries.is_empty() {
        return Ok(ScoredSessionResult {
            session_id: session_id.unwrap_or_default().to_string(),
            scored: false,
            reward: 0,
            test_exit_code,
            bullets: Vec::new(),
            skills: Vec::new(),
            scope: project.unwrap_or("global").to_string(),
            changes: 0,
        });
    }

    let target_session_id = match session_id {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => entries
            .iter()
            .rev()
            .find(|e| !e.session_id.is_empty())
            .map(|e| e.session_id.clone())
            .unwrap_or_default(),
    };

    if target_session_id.is_empty() {
        return Ok(ScoredSessionResult {
            session_id: String::new(),
            scored: false,
            reward: 0,
            test_exit_code,
            bullets: Vec::new(),
            skills: Vec::new(),
            scope: project.unwrap_or("global").to_string(),
            changes: 0,
        });
    }

    let session_entries: Vec<&LedgerEntry> = entries
        .iter()
        .filter(|e| e.session_id == target_session_id)
        .collect();

    if session_entries.is_empty() {
        return Ok(ScoredSessionResult {
            session_id: target_session_id,
            scored: false,
            reward: 0,
            test_exit_code,
            bullets: Vec::new(),
            skills: Vec::new(),
            scope: project.unwrap_or("global").to_string(),
            changes: 0,
        });
    }

    let mut bullets: Vec<String> = Vec::new();
    let mut skills: Vec<String> = Vec::new();
    let mut session_project: Option<String> = project.map(|s| s.to_string());

    for e in &session_entries {
        if session_project.is_none()
            && !e.project.is_empty()
            && e.project != "all"
            && e.project != "global"
            && e.event != "skill"
        {
            session_project = Some(e.project.clone());
        }
        for b in &e.injected_bullets {
            if !bullets.contains(b) {
                bullets.push(b.clone());
            }
        }
        if e.event == "skill" && !e.project.is_empty() && !skills.contains(&e.project) {
            skills.push(e.project.clone());
        }
    }

    let scope = session_project.unwrap_or_else(|| "global".to_string());
    let is_success = test_exit_code == 0;
    let reward: i64 = if is_success { 1 } else { -1 };

    let mut tag_ops = Vec::new();
    for b in &bullets {
        tag_ops.push(crate::ace::TagOp {
            id: b.clone(),
            helpful: if is_success { Some(1) } else { None },
            harmful: if !is_success { Some(1) } else { None },
        });
    }
    for s in &skills {
        tag_ops.push(crate::ace::TagOp {
            id: format!("skill:{}", s),
            helpful: if is_success { Some(1) } else { None },
            harmful: if !is_success { Some(1) } else { None },
        });
    }

    let mut changes = 0;
    if !tag_ops.is_empty() {
        let delta_ops = crate::ace::DeltaOps {
            add: None,
            tag: Some(tag_ops.clone()),
            remove: None,
        };
        if let Ok(ch) = crate::ace::apply_with_lock(vault_path, &scope, &delta_ops) {
            changes += ch;
        }
        if scope != "global" {
            let _ = crate::ace::apply_with_lock(vault_path, "global", &delta_ops)
                .map(|ch| changes += ch);
        }
    }

    record(
        ledger_path,
        "session_scored",
        &scope,
        &format!("exit_code={};reward={}", test_exit_code, reward),
        &skills,
        &bullets,
        0.0,
        None,
        Some(&target_session_id),
    );

    Ok(ScoredSessionResult {
        session_id: target_session_id,
        scored: true,
        reward,
        test_exit_code,
        bullets,
        skills,
        scope,
        changes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_and_read_roundtrip() {
        let tmp =
            std::env::temp_dir().join(format!("hive_ledger_test_{}.jsonl", std::process::id()));
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
