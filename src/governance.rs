//! Quarantine governance (v2.7.0, GOV-01); mirror of `governance.py`.
//! A quarantine is a reversible rule, not an edit: `_meta/quarantine.jsonl` holds one compact JSON line per
//! rule. An item is quarantined while any rule matches its origin (session, agent, time): traces, scout/quorum
//! bullets (sidecar origin), inbox proposals and codices (frontmatter). Release deletes the rule only.

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const KINDS: [&str; 4] = ["traces", "bullets", "inbox", "codices"];

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

fn date_ts(updated: &str) -> i64 {
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

fn save(vault: &Path, rules: &[Rule]) -> Result<(), String> {
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

/// (kind, session, agent, ts) for every quarantinable item.
fn origins(vault: &Path, conn: &Connection) -> Vec<(&'static str, String, String, i64)> {
    let mut out = Vec::new();
    if let Ok(mut stmt) = conn.prepare("SELECT agent, session_id, created_at FROM traces") {
        if let Ok(rows) = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
            ))
        }) {
            for (agent, session, ts) in rows.flatten() {
                out.push(("traces", session, agent, ts));
            }
        }
    }
    let mut playbooks = vec![vault.join("_meta").join("playbook.md")];
    if let Ok(rd) = std::fs::read_dir(vault.join("projects")) {
        let mut more: Vec<PathBuf> = rd.flatten().map(|e| e.path().join("playbook.md")).collect();
        more.sort();
        playbooks.extend(more);
    }
    for pb in playbooks {
        for m in crate::quorum::load(&crate::quorum::meta_path(&pb)).values() {
            if matches!(m.state.as_str(), "scout" | "quorum") {
                out.push((
                    "bullets",
                    m.origin_session.clone(),
                    m.origin_agent.clone(),
                    m.origin_ts,
                ));
            }
        }
    }
    let session_re = regex::Regex::new(r"\bsession\s+(\S+)").unwrap();
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
    for (kind, path) in notes {
        if path.extension().is_none_or(|x| x != "md") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let (meta, _, _) = crate::index::parse_note(&text);
        let get = |k: &str| meta.get(k).cloned().unwrap_or_default();
        let session = session_re
            .captures(&get("source"))
            .map(|c| c[1].to_string())
            .unwrap_or_default();
        out.push((kind, session, get("agent"), date_ts(&get("updated"))));
    }
    out
}

pub fn scan(vault: &Path, conn: &Connection, rule: &Rule) -> BTreeMap<&'static str, usize> {
    let mut counts: BTreeMap<&'static str, usize> = KINDS.iter().map(|k| (*k, 0)).collect();
    for (kind, session, agent, ts) in origins(vault, conn) {
        if rule_matches(rule, &session, &agent, ts) {
            *counts.get_mut(kind).unwrap() += 1;
        }
    }
    counts
}

/// Add (or reuse) a rule; returns it with matched counts per kind. Needs a session or an agent.
pub fn quarantine(
    vault: &Path,
    conn: &Connection,
    session: &str,
    agent: &str,
    since: &str,
    reason: &str,
    now: i64,
) -> Result<(Rule, BTreeMap<&'static str, usize>), String> {
    let (session, agent) = (session.trim(), agent.trim());
    if session.is_empty() && agent.is_empty() {
        return Err("quarantine needs --session or --agent".into());
    }
    let since = parse_since(since)?;
    let id = format!(
        "q-{}",
        &crate::common::sha256(format!("{session}|{agent}|{since}").as_bytes())[..8]
    );
    let mut rules = load_rules(vault);
    let rule = match rules.iter().find(|r| r.id == id) {
        Some(r) => r.clone(),
        None => {
            let r = Rule {
                id,
                session: session.into(),
                agent: agent.into(),
                since,
                reason: reason
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
                    .chars()
                    .take(200)
                    .collect(),
                created: now,
            };
            rules.push(r.clone());
            save(vault, &rules)?;
            r
        }
    };
    let counts = scan(vault, conn, &rule);
    Ok((rule, counts))
}

/// Remove the rule; returns how many were removed.
pub fn release(vault: &Path, rule_id: &str) -> Result<usize, String> {
    let rules = load_rules(vault);
    let keep: Vec<Rule> = rules.iter().filter(|r| r.id != rule_id).cloned().collect();
    save(vault, &keep)?;
    Ok(rules.len() - keep.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_quarantine_session_rule_counts_and_release() {
        let vault = std::env::temp_dir().join(format!("hive-gov-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&vault);
        std::fs::create_dir_all(vault.join("inbox")).unwrap();
        let conn = Connection::open_in_memory().unwrap();
        crate::trace::add(
            &conn,
            "demo",
            "HOT_FILE",
            "src/b.rs",
            "bad",
            "codex",
            "bad-s",
            1_800_000_000,
        )
        .unwrap();
        crate::trace::add(
            &conn,
            "demo",
            "DEAD_END",
            "src/a.rs",
            "good",
            "claude",
            "good-s",
            1_800_000_000,
        )
        .unwrap();
        std::fs::write(
            vault.join("inbox").join("x.md"),
            "---\ntype: note\nproject: demo\nagent: codex\nstatus: active\nupdated: 2027-01-15\nsource: hive curate\n---\n# x\n",
        )
        .unwrap();
        assert!(quarantine(&vault, &conn, "", "", "", "", 0).is_err());
        assert!(quarantine(&vault, &conn, "x", "", "nope", "", 0).is_err());
        let (rule, counts) = quarantine(&vault, &conn, "bad-s", "", "", "poisoned", 1).unwrap();
        assert_eq!((counts["traces"], counts["inbox"]), (1, 0));
        let (_, by_agent) = quarantine(&vault, &conn, "", "codex", "2027-01-15", "", 1).unwrap();
        assert_eq!((by_agent["traces"], by_agent["inbox"]), (1, 1));
        let line = std::fs::read_to_string(rules_path(&vault)).unwrap();
        assert!(line.starts_with(&format!(r#"{{"id":"{}","session":"bad-s","agent":"","since":0,"reason":"poisoned","created":1}}"#, rule.id)));
        assert!(is_quarantined(&load_rules(&vault), "bad-s", "codex", 5));
        assert!(!is_quarantined(&load_rules(&vault), "good-s", "claude", 5));
        assert_eq!(release(&vault, &rule.id).unwrap(), 1);
        assert_eq!(load_rules(&vault).len(), 1);
        assert_eq!(parse_since("2027-01-15").unwrap(), 1_800_000_000 - 8 * 3600);
        let _ = std::fs::remove_dir_all(&vault);
    }
}
