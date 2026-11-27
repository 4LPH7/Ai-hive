//! Quarantine rules and read-path filters (GOV-01/02). Storage and matching only: no audit or write logic,
//! so ace, index, trace and codex can filter without depending on governance.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Field order is the wire order: byte-identical to `governance.py`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rule {
    pub id: String,
    #[serde(default)]
    pub session: String,
    #[serde(default)]
    pub agent: String,
    #[serde(default)]
    pub since: i64,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub created: i64,
}

pub fn rules_path(vault: &Path) -> PathBuf {
    vault.join("_meta").join("quarantine.jsonl")
}

/// Epoch seconds from "", digits, YYYY-MM-DD (UTC midnight) or an RFC 3339 datetime.
pub fn parse_since(value: &str) -> Result<i64, String> {
    let s = value.trim();
    if s.is_empty() {
        return Ok(0);
    }
    if s.chars().all(|c| c.is_ascii_digit()) {
        return s.parse().map_err(|e| format!("bad --since {s:?}: {e}"));
    }
    if let Ok(d) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return Ok(d.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp());
    }
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Ok(dt.timestamp());
    }
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S") {
        return Ok(dt.and_utc().timestamp());
    }
    Err(format!(
        "bad --since {s:?}: use YYYY-MM-DD, an ISO datetime or epoch seconds"
    ))
}

pub fn date_ts(updated: &str) -> i64 {
    parse_since(updated.get(..10).unwrap_or(updated)).unwrap_or(0)
}

pub fn load_rules(vault: &Path) -> Vec<Rule> {
    let Ok(text) = std::fs::read_to_string(rules_path(vault)) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|l| serde_json::from_str::<Rule>(l).ok())
        .filter(|r| !r.id.is_empty() && (!r.session.is_empty() || !r.agent.is_empty()))
        .collect()
}

pub fn save(vault: &Path, rules: &[Rule]) -> Result<(), String> {
    let path = rules_path(vault);
    if rules.is_empty() {
        let _ = std::fs::remove_file(&path);
        return Ok(());
    }
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    let mut out = String::new();
    for r in rules {
        out.push_str(&serde_json::to_string(r).map_err(|e| e.to_string())?);
        out.push('\n');
    }
    let tmp = path.with_extension("jsonl.tmp");
    std::fs::write(&tmp, out).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
}

pub fn rule_matches(rule: &Rule, session: &str, agent: &str, ts: i64) -> bool {
    (rule.session.is_empty() || rule.session == session)
        && (rule.agent.is_empty() || rule.agent == agent)
        && ts >= rule.since
}

pub fn is_quarantined(rules: &[Rule], session: &str, agent: &str, ts: i64) -> bool {
    rules.iter().any(|r| rule_matches(r, session, agent, ts))
}

/// inbox/*.md and codices/*/*.md, the note kinds a rule can quarantine.
pub fn note_files(vault: &Path) -> Vec<(&'static str, PathBuf)> {
    let mut notes: Vec<(&'static str, PathBuf)> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(vault.join("inbox")) {
        notes.extend(rd.flatten().map(|e| ("inbox", e.path())));
    }
    if let Ok(rd) = std::fs::read_dir(vault.join("codices")) {
        for proj in rd.flatten() {
            if let Ok(inner) = std::fs::read_dir(proj.path()) {
                notes.extend(inner.flatten().map(|e| ("codices", e.path())));
            }
        }
    }
    notes.retain(|(_, p)| p.extension().is_some_and(|x| x == "md"));
    notes
}

pub fn source_session(source: &str) -> String {
    regex::Regex::new(r"\bsession\s+(\S+)")
        .unwrap()
        .captures(source)
        .map(|c| c[1].to_string())
        .unwrap_or_default()
}

/// (session, agent, ts) of an inbox proposal or codex from its frontmatter.
pub fn note_origin(text: &str) -> (String, String, i64) {
    let (meta, _, _) = crate::note::parse_note(text);
    let get = |k: &str| meta.get(k).cloned().unwrap_or_default();
    (
        source_session(&get("source")),
        get("agent"),
        date_ts(&get("updated")),
    )
}

// ---------------------------------------------------------------- GOV-02 read-path filters

/// Vault-relative posix paths of inbox proposals and codices that an active rule matches.
pub fn quarantined_paths(vault: &Path, rules: &[Rule]) -> std::collections::HashSet<String> {
    if rules.is_empty() {
        return Default::default();
    }
    note_files(vault)
        .into_iter()
        .filter(|(_, p)| {
            std::fs::read_to_string(p).is_ok_and(|t| {
                let (s, a, ts) = note_origin(&t);
                is_quarantined(rules, &s, &a, ts)
            })
        })
        .filter_map(|(_, p)| {
            p.strip_prefix(vault)
                .ok()
                .map(|r| r.to_string_lossy().replace('\\', "/"))
        })
        .collect()
}

/// An indexed inbox proposal or codex whose origin an active rule matches (from index columns).
pub fn note_hidden(
    rules: &[Rule],
    rel_path: &str,
    source: &str,
    agent: &str,
    updated: &str,
) -> bool {
    (rel_path.starts_with("inbox/") || rel_path.starts_with("codices/"))
        && is_quarantined(rules, &source_session(source), agent, date_ts(updated))
}

/// A scout/quorum bullet whose origin an active rule matches.
pub fn bullet_hidden(rules: &[Rule], rec: Option<&crate::quorum::Meta>) -> bool {
    rec.is_some_and(|m| {
        !rules.is_empty()
            && matches!(m.state.as_str(), "scout" | "quorum")
            && is_quarantined(rules, &m.origin_session, &m.origin_agent, m.origin_ts)
    })
}
