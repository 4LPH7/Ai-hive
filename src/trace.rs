//! Stigmergic trace layer (v2.6.0, TRACE-01/02). Agents leave short, decaying signals in the disposable
//! `hive.sqlite` instead of messaging each other: strength halves every `half_life_h` hours, faint traces
//! are hidden, old ones purged. Canonical Markdown is never touched.

use rusqlite::{params, Connection};

/// Kinds and half-lives in hours. `ACTIVE_LEASE` waits for evidence of concurrent edits (TRACE-06).
pub const KINDS: [(&str, f64); 3] = [("DEAD_END", 24.0), ("RECENT_FIX", 48.0), ("HOT_FILE", 12.0)];
pub const MIN_STRENGTH: f64 = 0.15;
pub const MAX_STRENGTH: f64 = 3.0;
pub const MAX_SUMMARY_CHARS: usize = 200;
pub const PURGE_AFTER_SECS: i64 = 7 * 24 * 3600;

#[derive(Debug, Clone, PartialEq)]
pub struct Trace {
    pub id: i64,
    pub project: String,
    pub kind: String,
    pub target: String,
    pub summary: String,
    pub agent: String,
    pub session_id: String,
    pub w0: f64,
    pub created_at: i64,
    pub half_life_h: f64,
}

pub fn half_life(kind: &str) -> Option<f64> {
    KINDS.iter().find(|(k, _)| *k == kind).map(|(_, h)| *h)
}

/// w(t) = w0 * 2^(-dt / h), dt in hours; never negative time.
pub fn strength(w0: f64, created_at: i64, now: i64, half_life_h: f64) -> f64 {
    let dt_h = (now - created_at).max(0) as f64 / 3600.0;
    w0 * 2f64.powf(-dt_h / half_life_h)
}

pub fn ensure_table(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS traces(id INTEGER PRIMARY KEY AUTOINCREMENT, project TEXT NOT NULL,
            kind TEXT NOT NULL, target TEXT NOT NULL, summary TEXT NOT NULL, agent TEXT NOT NULL,
            session_id TEXT NOT NULL, w0 REAL NOT NULL, created_at INTEGER NOT NULL, half_life_h REAL NOT NULL);
         CREATE INDEX IF NOT EXISTS traces_project ON traces(project, kind, target);
         CREATE TABLE IF NOT EXISTS trace_sessions(trace_id INTEGER NOT NULL, session_id TEXT NOT NULL,
            PRIMARY KEY(trace_id, session_id));",
    )
    .map_err(|e| e.to_string())
}

/// Add a trace, or reinforce one: the same project + kind + target from another session adds 1.0 to the
/// current strength (capped) and restarts the clock; a repeat from the same session changes nothing.
#[allow(clippy::too_many_arguments)]
pub fn add(
    conn: &Connection,
    project: &str,
    kind: &str,
    target: &str,
    summary: &str,
    agent: &str,
    session_id: &str,
    now: i64,
) -> Result<i64, String> {
    let h = half_life(kind).ok_or_else(|| {
        let known: Vec<&str> = KINDS.iter().map(|(k, _)| *k).collect();
        format!("unknown trace kind {kind:?}; use one of {known:?}")
    })?;
    let summary = summary.split_whitespace().collect::<Vec<_>>().join(" ");
    if project.trim().is_empty() || target.trim().is_empty() || summary.is_empty() {
        return Err("project, target and summary are required".into());
    }
    if summary.chars().count() > MAX_SUMMARY_CHARS {
        return Err(format!("summary over {MAX_SUMMARY_CHARS} characters"));
    }
    if crate::ace::is_secret(&summary) || crate::ace::is_secret(target) {
        return Err("trace looks like it contains a secret; not stored".into());
    }
    ensure_table(conn)?;
    let existing: Option<(i64, String, f64, i64, f64)> = conn
        .query_row(
            "SELECT id, session_id, w0, created_at, half_life_h FROM traces
             WHERE project=?1 AND kind=?2 AND target=?3 ORDER BY created_at DESC LIMIT 1",
            params![project, kind, target],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .ok();
    if let Some((id, sess, w0, created, hl)) = existing {
        // only a distinct, identifiable session reinforces
        if sess == session_id
            || crate::ledger::is_anonymous(&sess)
            || crate::ledger::is_anonymous(session_id)
        {
            return Ok(id);
        }
        let w = (strength(w0, created, now, hl) + 1.0).min(MAX_STRENGTH);
        conn.execute(
            "UPDATE traces SET w0=?1, created_at=?2, summary=?3, agent=?4, session_id=?5 WHERE id=?6",
            params![w, now, summary, agent, session_id, id],
        )
        .map_err(|e| e.to_string())?;
        record_session(conn, id, session_id)?;
        return Ok(id);
    }
    conn.execute(
        "INSERT INTO traces(project, kind, target, summary, agent, session_id, w0, created_at, half_life_h)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1.0, ?7, ?8)",
        params![project, kind, target, summary, agent, session_id, now, h],
    )
    .map_err(|e| e.to_string())?;
    let id = conn.last_insert_rowid();
    if !crate::ledger::is_anonymous(session_id) {
        record_session(conn, id, session_id)?;
    }
    Ok(id)
}

/// GOV-04: remember each distinct identifiable session that left a trace.
fn record_session(conn: &Connection, id: i64, session_id: &str) -> Result<(), String> {
    conn.execute(
        "INSERT OR IGNORE INTO trace_sessions VALUES (?1, ?2)",
        params![id, session_id],
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}

/// GOV-04: kinds that become playbook ADD proposals once >= PROPOSE_SESSIONS distinct sessions left them.
pub const PROPOSE_SECTIONS: [(&str, &str); 2] =
    [("DEAD_END", "pitfalls"), ("RECENT_FIX", "strategies")];
pub const PROPOSE_SESSIONS: usize = 2;

/// GOV-04: one inbox/ playbook ADD proposal per active DEAD_END / RECENT_FIX trace left by >= 2 distinct
/// identifiable sessions (never applied; quarantined traces skipped). The file name is stable per
/// (project, kind, target), so reruns never duplicate. Returns the paths written. Mirror of traces.py.
pub fn propose(
    vault: &std::path::Path,
    conn: &Connection,
    project: &str,
    now: i64,
) -> Result<Vec<std::path::PathBuf>, String> {
    let rules = crate::governance::load_rules(vault);
    let mut written = Vec::new();
    for (t, w) in crate::governance::visible_traces(&rules, active(conn, project, now)?) {
        let Some(section) = PROPOSE_SECTIONS
            .iter()
            .find(|(k, _)| *k == t.kind)
            .map(|(_, s)| *s)
        else {
            continue;
        };
        let mut stmt = conn
            .prepare("SELECT session_id FROM trace_sessions WHERE trace_id=?1 ORDER BY session_id")
            .map_err(|e| e.to_string())?;
        let sessions: Vec<String> = stmt
            .query_map(params![t.id], |r| r.get(0))
            .map_err(|e| e.to_string())?
            .flatten()
            .collect();
        if sessions.len() < PROPOSE_SESSIONS {
            continue;
        }
        let key = crate::common::sha256(format!("{project}|{}|{}", t.kind, t.target).as_bytes());
        let path = vault.join("inbox").join(format!(
            "trace-{}-{}.md",
            crate::common::slugify(project),
            &key[..8]
        ));
        if path.exists() {
            continue;
        }
        let today = chrono::DateTime::from_timestamp(now, 0)
            .unwrap_or_default()
            .format("%Y-%m-%d")
            .to_string();
        let lesson = format!("{}: {}", t.target, t.summary);
        let delta = serde_json::json!({"operations": [{"type": "ADD", "section": section, "content": lesson}]});
        let text = [
            "---".to_string(),
            "type: note".into(),
            format!("project: {project}"),
            "agent: claude".into(),
            "status: active".into(),
            format!("updated: {today}"),
            format!("source: hive traces (trace {}, sessions {})", t.id, sessions.join(", ")),
            "---".into(),
            format!("# Playbook ADD proposal: {} {}", t.kind, t.target),
            String::new(),
            format!(
                "> {} independent sessions left this {} trace (strength {w:.2}). Nothing was",
                sessions.len(),
                t.kind
            ),
            format!(
                "> applied. If it holds, save the delta below to a file and run `hive ace apply {project} <file>`; it enters as a scout and needs its own quorum."
            ),
            String::new(),
            "```json".into(),
            delta.to_string(),
            "```".into(),
            String::new(),
        ]
        .join("\n");
        std::fs::create_dir_all(vault.join("inbox")).map_err(|e| e.to_string())?;
        std::fs::write(&path, text).map_err(|e| e.to_string())?;
        written.push(path);
    }
    Ok(written)
}

/// Traces still above `MIN_STRENGTH`, strongest first, after purging anything older than 7 days.
pub fn active(conn: &Connection, project: &str, now: i64) -> Result<Vec<(Trace, f64)>, String> {
    ensure_table(conn)?;
    let cutoff = now - PURGE_AFTER_SECS;
    let stale: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM traces WHERE created_at < ?1)",
            params![cutoff],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if stale {
        // write + fsync only when something is stale: reads stay read-only (Phase 7 gate < 10 ms)
        conn.execute("DELETE FROM traces WHERE created_at < ?1", params![cutoff])
            .map_err(|e| e.to_string())?;
        conn.execute(
            "DELETE FROM trace_sessions WHERE trace_id NOT IN (SELECT id FROM traces)",
            [],
        )
        .map_err(|e| e.to_string())?;
    }
    let mut stmt = conn
        .prepare(
            "SELECT id, project, kind, target, summary, agent, session_id, w0, created_at, half_life_h
             FROM traces WHERE project=?1",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![project], |r| {
            Ok(Trace {
                id: r.get(0)?,
                project: r.get(1)?,
                kind: r.get(2)?,
                target: r.get(3)?,
                summary: r.get(4)?,
                agent: r.get(5)?,
                session_id: r.get(6)?,
                w0: r.get(7)?,
                created_at: r.get(8)?,
                half_life_h: r.get(9)?,
            })
        })
        .map_err(|e| e.to_string())?;
    let mut out: Vec<(Trace, f64)> = rows
        .filter_map(Result::ok)
        .map(|t| {
            let w = strength(t.w0, t.created_at, now, t.half_life_h);
            (t, w)
        })
        .filter(|(_, w)| *w >= MIN_STRENGTH)
        .collect();
    out.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    Ok(out)
}

pub const CONTEXT_MAX_TRACES: usize = 5;

pub(crate) fn words(s: &str) -> std::collections::HashSet<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 3)
        .map(str::to_string)
        .collect()
}

/// TRACE-04: `hive_context` section. With a task, keep traces whose target/summary share a word with it.
pub fn context_section(items: &[(Trace, f64)], task: &str) -> String {
    let want = words(task);
    let lines: Vec<String> = items
        .iter()
        .filter(|(t, _)| {
            want.is_empty() || !words(&format!("{} {}", t.target, t.summary)).is_disjoint(&want)
        })
        .take(CONTEXT_MAX_TRACES)
        .map(|(t, w)| {
            format!(
                "- {} {} ({}, w={:.2}): {}",
                t.kind, t.target, t.agent, w, t.summary
            )
        })
        .collect();
    if lines.is_empty() {
        return String::new();
    }
    format!(
        "## live traces (decaying signals from other agents)\n{}",
        lines.join("\n")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const H: i64 = 3600;

    fn db() -> Connection {
        Connection::open_in_memory().unwrap()
    }

    #[test]
    fn test_strength_halves_per_half_life() {
        assert!((strength(1.0, 0, 24 * H, 24.0) - 0.5).abs() < 1e-9);
        assert!((strength(1.0, 0, 48 * H, 24.0) - 0.25).abs() < 1e-9);
        assert_eq!(strength(1.0, 100, 0, 24.0), 1.0); // clock skew never inflates
    }

    #[test]
    fn test_dead_end_visible_then_evaporates_then_purged() {
        let c = db();
        add(
            &c,
            "demo",
            "DEAD_END",
            "src/a.rs",
            "patching tokenizer breaks static link",
            "claude",
            "s1",
            0,
        )
        .unwrap();
        assert_eq!(active(&c, "demo", 24 * H).unwrap().len(), 1); // 0.5
        assert!(active(&c, "demo", 72 * H).unwrap().is_empty()); // 0.125 < 0.15
        assert!(active(&c, "other", 0).unwrap().is_empty()); // project isolation
        active(&c, "demo", 8 * 24 * H).unwrap();
        let n: i64 = c
            .query_row("SELECT COUNT(*) FROM traces", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn test_two_sessions_propose_once_and_hot_file_never() {
        let c = db();
        let vault = std::env::temp_dir().join(format!("hive-propose-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&vault);
        add(
            &c,
            "demo",
            "DEAD_END",
            "src/a.rs",
            "patching breaks link",
            "claude",
            "s1",
            0,
        )
        .unwrap();
        assert!(propose(&vault, &c, "demo", 0).unwrap().is_empty());
        add(
            &c,
            "demo",
            "DEAD_END",
            "src/a.rs",
            "patching breaks link",
            "codex",
            "s2",
            H,
        )
        .unwrap();
        add(&c, "demo", "HOT_FILE", "src/d.rs", "x", "claude", "s1", H).unwrap();
        add(&c, "demo", "HOT_FILE", "src/d.rs", "x", "codex", "s2", H).unwrap();
        let written = propose(&vault, &c, "demo", H).unwrap();
        assert_eq!(written.len(), 1);
        let text = std::fs::read_to_string(&written[0]).unwrap();
        assert!(
            text.contains(r#""section":"pitfalls""#) && text.contains("sessions s1, s2"),
            "{text}"
        );
        assert!(
            propose(&vault, &c, "demo", H).unwrap().is_empty(),
            "never duplicated"
        );
        let _ = std::fs::remove_dir_all(&vault);
    }

    #[test]
    fn test_anonymous_process_sessions_never_reinforce() {
        let c = db();
        add(
            &c, "demo", "HOT_FILE", "src/b.rs", "x", "claude", "proc-1-a", 0,
        )
        .unwrap();
        add(
            &c, "demo", "HOT_FILE", "src/b.rs", "x", "claude", "proc-2-b", 0,
        )
        .unwrap();
        add(
            &c, "demo", "HOT_FILE", "src/b.rs", "x", "codex", "real-s2", 0,
        )
        .unwrap();
        assert!((active(&c, "demo", 0).unwrap()[0].1 - 1.0).abs() < 1e-9);
    }

    #[test]
    fn test_reinforcement_only_from_another_session() {
        let c = db();
        let id = add(
            &c,
            "demo",
            "HOT_FILE",
            "src/b.rs",
            "3 failed runs",
            "claude",
            "s1",
            0,
        )
        .unwrap();
        assert_eq!(
            add(&c, "demo", "HOT_FILE", "src/b.rs", "again", "claude", "s1", H).unwrap(),
            id
        );
        let (_, w) = active(&c, "demo", H).unwrap().remove(0);
        assert!(w < 1.0); // same session: no reinforcement
        add(
            &c,
            "demo",
            "HOT_FILE",
            "src/b.rs",
            "codex saw it too",
            "codex",
            "s2",
            12 * H,
        )
        .unwrap();
        let (t, w) = active(&c, "demo", 12 * H).unwrap().remove(0);
        assert!((w - 1.5).abs() < 1e-9); // 0.5 decayed + 1.0
        assert_eq!((t.agent.as_str(), t.id), ("codex", id));
    }

    #[test]
    fn test_rejects_bad_kind_long_summary_and_secrets() {
        let c = db();
        assert!(add(&c, "demo", "ACTIVE_LEASE", "x", "y", "a", "s", 0).is_err());
        assert!(add(&c, "demo", "DEAD_END", "x", &"y".repeat(201), "a", "s", 0).is_err());
        assert!(add(
            &c,
            "demo",
            "DEAD_END",
            "x",
            "token ghp_123456789012345678901234567890123456",
            "a",
            "s",
            0
        )
        .is_err());
        assert!(add(&c, "demo", "DEAD_END", " ", "y", "a", "s", 0).is_err());
    }

    #[test]
    fn test_context_section_filters_by_task_overlap() {
        let c = db();
        add(
            &c,
            "demo",
            "DEAD_END",
            "src/tokenizer.rs",
            "patching breaks static link",
            "claude",
            "s1",
            0,
        )
        .unwrap();
        add(
            &c,
            "demo",
            "HOT_FILE",
            "src/ui.rs",
            "3 failed runs",
            "codex",
            "s2",
            0,
        )
        .unwrap();
        let items = active(&c, "demo", H).unwrap();
        let s = context_section(&items, "fix the tokenizer link error");
        assert!(s.starts_with("## live traces"));
        assert!(s.contains("DEAD_END src/tokenizer.rs"));
        assert!(!s.contains("src/ui.rs"));
        assert!(context_section(&items, "").contains("src/ui.rs"));
        assert_eq!(context_section(&items, "unrelated words only"), "");
        assert_eq!(context_section(&[], ""), "");
    }
}
