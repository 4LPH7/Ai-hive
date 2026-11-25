use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

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
    /// PROV-02: commit of the working repo when an ACE delta was applied (empty on other events).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub repo_sha: String,
    /// PROV-02: sha256 of the playbook file after an ACE delta was applied.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub content_sha256: String,
}

// ---------------------------------------------------------------- provenance (PROV-01)
// Process-wide context set once by whoever knows it best: MCP `initialize` clientInfo, or a hook's stdin JSON.
// `record()` falls back to it, then to env, then to a per-process session id, so session_id is never empty.

#[derive(Default)]
struct Provenance {
    agent: Option<String>,
    session_id: Option<String>,
}

static PROVENANCE: OnceLock<Mutex<Provenance>> = OnceLock::new();
static PROCESS_SESSION: OnceLock<String> = OnceLock::new();

fn provenance() -> &'static Mutex<Provenance> {
    PROVENANCE.get_or_init(|| Mutex::new(Provenance::default()))
}

fn non_empty(s: Option<&str>) -> Option<String> {
    s.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Set process-wide provenance. Empty or missing values leave the current value alone.
pub fn set_context(agent: Option<&str>, session_id: Option<&str>) {
    if let Ok(mut p) = provenance().lock() {
        if let Some(a) = non_empty(agent) {
            p.agent = Some(a);
        }
        if let Some(s) = non_empty(session_id) {
            p.session_id = Some(s);
        }
    }
}

/// Map an MCP `clientInfo.name` or a transcript path to a stable agent label.
/// Unknown hints become a sanitized short label instead of "unknown".
pub fn agent_from_hint(hint: &str) -> Option<String> {
    let h = hint.to_ascii_lowercase();
    if h.trim().is_empty() {
        return None;
    }
    if h.contains("claude") {
        return Some("claude".into());
    }
    if h.contains("codex") {
        return Some("codex".into());
    }
    if h.contains("antigravity") || h.contains("gemini") {
        return Some("antigravity".into());
    }
    if h.contains('/') || h.contains('\\') {
        return None; // an unrecognised path says nothing about the agent
    }
    let label: String = h
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .take(40)
        .collect();
    let label = label.trim_matches('-').to_string();
    (!label.is_empty()).then_some(label)
}

/// One id per process: an MCP stdio server lives exactly as long as one agent session.
pub fn process_session_id() -> String {
    PROCESS_SESSION
        .get_or_init(|| {
            format!(
                "proc-{}-{}",
                std::process::id(),
                chrono::Utc::now().format("%Y%m%dT%H%M%S")
            )
        })
        .clone()
}

pub fn resolve_agent(explicit: Option<&str>) -> String {
    non_empty(explicit)
        .or_else(|| provenance().lock().ok().and_then(|p| p.agent.clone()))
        .unwrap_or_else(detect_agent)
}

pub fn resolve_session(explicit: Option<&str>) -> String {
    non_empty(explicit)
        .or_else(|| provenance().lock().ok().and_then(|p| p.session_id.clone()))
        .or_else(|| non_empty(std::env::var("HIVE_SESSION_ID").ok().as_deref()))
        .unwrap_or_else(process_session_id)
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
    let entry = LedgerEntry {
        ts: chrono::Utc::now().to_rfc3339(),
        session_id: resolve_session(session_id),
        agent: resolve_agent(agent),
        project: project.to_string(),
        event: event.to_string(),
        query: query.to_string(),
        hits: hits.to_vec(),
        injected_bullets: injected_bullets.to_vec(),
        duration_ms: (duration_ms * 100.0).round() / 100.0,
        repo_sha: String::new(),
        content_sha256: String::new(),
    };
    append_entry(ledger_path, &entry)
}

/// PROV-02/04: an ACE delta was merged. `added` lists the new bullet ids, which records each bullet's
/// origin session and agent without a second store.
pub fn record_ace_apply(
    ledger_path: &Path,
    scope: &str,
    added: &[String],
    content_sha256: &str,
    repo_sha: &str,
) -> bool {
    let entry = LedgerEntry {
        ts: chrono::Utc::now().to_rfc3339(),
        session_id: resolve_session(None),
        agent: resolve_agent(None),
        project: scope.to_string(),
        event: "ace_apply".to_string(),
        query: String::new(),
        hits: Vec::new(),
        injected_bullets: added.to_vec(),
        duration_ms: 0.0,
        repo_sha: repo_sha.to_string(),
        content_sha256: content_sha256.to_string(),
    };
    append_entry(ledger_path, &entry)
}

/// Short HEAD sha of the repo at `dir`, or empty when it is not a git repo (fail-open).
pub fn repo_head(dir: &Path) -> String {
    std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .current_dir(dir)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

/// PROV-03: how many events carry provenance. `fallback_session` counts process-scoped ids
/// (`proc-...`), which identify a process but not the client's own session.
#[derive(Debug, Default, PartialEq)]
pub struct Coverage {
    pub total: usize,
    pub with_session: usize,
    pub with_agent: usize,
    pub fallback_session: usize,
}

impl Coverage {
    pub fn percent(n: usize, total: usize) -> f64 {
        if total == 0 {
            0.0
        } else {
            n as f64 * 100.0 / total as f64
        }
    }
}

pub fn coverage(entries: &[LedgerEntry]) -> Coverage {
    let mut c = Coverage {
        total: entries.len(),
        ..Default::default()
    };
    for e in entries {
        if !e.session_id.trim().is_empty() {
            c.with_session += 1;
            if e.session_id.starts_with("proc-") {
                c.fallback_session += 1;
            }
        }
        if !e.agent.trim().is_empty() && e.agent != "unknown" {
            c.with_agent += 1;
        }
    }
    c
}

fn append_entry(ledger_path: &Path, entry: &LedgerEntry) -> bool {
    if let Some(parent) = ledger_path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let json_line = match serde_json::to_string(entry) {
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

    #[test]
    fn test_agent_from_hint_maps_clients_and_paths() {
        assert_eq!(agent_from_hint("claude-code").as_deref(), Some("claude"));
        assert_eq!(
            agent_from_hint(r"C:\Users\x\.claude\projects\a\s.jsonl").as_deref(),
            Some("claude")
        );
        assert_eq!(
            agent_from_hint("codex-mcp-client").as_deref(),
            Some("codex")
        );
        assert_eq!(
            agent_from_hint("Antigravity IDE").as_deref(),
            Some("antigravity")
        );
        assert_eq!(
            agent_from_hint("gemini-cli").as_deref(),
            Some("antigravity")
        );
        assert_eq!(
            agent_from_hint("Cursor Editor").as_deref(),
            Some("cursor-editor")
        );
        assert_eq!(agent_from_hint("  "), None);
        assert_eq!(agent_from_hint(r"C:\Temp\tmp1\t.jsonl"), None);
    }

    #[test]
    fn test_session_is_never_empty_and_explicit_wins() {
        assert_eq!(resolve_session(Some("s-explicit")), "s-explicit");
        let fallback = resolve_session(Some("   "));
        assert!(!fallback.trim().is_empty());
        assert_eq!(resolve_agent(Some("codex")), "codex");
    }

    #[test]
    fn test_coverage_and_ace_apply_provenance_roundtrip() {
        let tmp =
            std::env::temp_dir().join(format!("hive_ledger_prov_{}.jsonl", std::process::id()));
        let _ = fs::remove_file(&tmp);
        // an old v2.5.0 line without the new fields must still parse
        fs::write(
            &tmp,
            "{\"ts\":\"t\",\"session_id\":\"\",\"agent\":\"unknown\",\"project\":\"\",\"event\":\"query\",\"query\":\"q\",\"hits\":[],\"injected_bullets\":[],\"duration_ms\":1.0}\n",
        )
        .unwrap();
        assert!(record_ace_apply(
            &tmp,
            "global",
            &["pit-00012".into()],
            "abc123",
            "949fe75"
        ));
        let entries = read_ledger(&tmp, None, None);
        assert_eq!(entries.len(), 2);
        let apply = &entries[1];
        assert_eq!(apply.event, "ace_apply");
        assert_eq!(apply.injected_bullets, vec!["pit-00012"]);
        assert_eq!(
            (apply.repo_sha.as_str(), apply.content_sha256.as_str()),
            ("949fe75", "abc123")
        );
        assert!(!apply.session_id.is_empty());

        let c = coverage(&entries);
        assert_eq!(c.total, 2);
        assert_eq!(c.with_session, 1);
        assert_eq!(Coverage::percent(c.with_session, c.total), 50.0);
        let _ = fs::remove_file(tmp);
    }
}
