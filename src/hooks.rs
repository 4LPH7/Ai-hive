//! Agent session hooks (Claude Code / Codex `SessionStart` and `Stop`). The hook's stdin JSON carries the
//! client's cwd, session id and transcript path; a hook never fails the session.
//!
//! - start: inject the top playbook lessons for the cwd's project into the session.
//! - stop: leave HOT_FILE traces for files this session keeps failing on (TRACE-05), and once 25+ tool calls
//!   with at least one edit have run since the last checkpoint, block the stop with a request to run the
//!   `hive-ace` skill.

use crate::ace;
use crate::common::HiveConfig;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const STOP_TOOL_THRESHOLD: u64 = 25;
pub const HOT_FILE_MIN_FAILS: usize = 3;
const EDIT_TOOLS: [&str; 4] = ["Edit", "Write", "MultiEdit", "NotebookEdit"];
const STATE_KEEP: usize = 200;

/// The hook payload; every field is optional (a bare CLI call has none).
#[derive(Debug, Default)]
pub struct HookInput {
    pub cwd: Option<String>,
    pub session_id: Option<String>,
    pub transcript_path: Option<String>,
    pub stop_hook_active: bool,
    pub present: bool,
}

impl HookInput {
    pub fn parse(stdin: &str) -> Self {
        let Ok(Value::Object(m)) = serde_json::from_str::<Value>(stdin) else {
            return Self::default();
        };
        let s = |k: &str| m.get(k).and_then(Value::as_str).map(str::to_string);
        Self {
            cwd: s("cwd"),
            session_id: s("session_id"),
            transcript_path: s("transcript_path"),
            stop_hook_active: m
                .get("stop_hook_active")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            present: true,
        }
    }

    /// PROV-01: the client's own session id and agent become the provenance of every ledger record.
    fn set_provenance(&self) {
        let agent = self
            .transcript_path
            .as_deref()
            .and_then(crate::ledger::agent_from_hint);
        crate::ledger::set_context(agent.as_deref(), self.session_id.as_deref());
    }

    fn scope(&self, cfg: &HiveConfig, explicit: Option<&str>) -> String {
        if let Some(s) = explicit.filter(|s| !s.is_empty()) {
            return s.to_string();
        }
        let cwd = self
            .cwd
            .clone()
            .map(PathBuf::from)
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_default();
        scope_for_cwd(&cfg.projects, &cwd)
    }
}

/// Project slug when cwd is inside the projects root, else `global`.
pub fn scope_for_cwd(projects: &Path, cwd: &Path) -> String {
    let (Ok(root), Ok(cwd)) = (projects.canonicalize(), cwd.canonicalize()) else {
        return "global".into();
    };
    cwd.strip_prefix(&root)
        .ok()
        .and_then(|rel| rel.components().next())
        .map(|c| crate::common::slugify(&c.as_os_str().to_string_lossy()))
        .unwrap_or_else(|| "global".into())
}

/// SessionStart: the playbook lessons for this session, as hook JSON when called by a client.
pub fn start(cfg: &HiveConfig, input: &HookInput, scope: Option<&str>) -> Option<String> {
    input.set_provenance();
    let scope = input.scope(cfg, scope);
    let mut scopes = vec!["global".to_string()];
    if scope != "global" {
        scopes.push(scope.clone());
    }
    let rows = ace::select(&cfg.vault, &scopes, "", ace::INJECT_LIMIT);
    if rows.is_empty() {
        return None;
    }
    let ids: Vec<String> = rows.iter().map(|(_, _, b)| b.id.clone()).collect();
    crate::ledger::record(
        &cfg.ledger,
        "inject",
        &scope,
        "hook-start",
        &[],
        &ids,
        0.0,
        None,
        None,
    );
    let text = format!(
        "Hive playbook (ACE). Apply what fits; note the ids you relied on. After substantial work run the `hive-ace` skill to tag them and add new lessons.\n{}",
        ace::format_rows(&rows)
    );
    Some(if input.present {
        serde_json::json!({"hookSpecificOutput": {"hookEventName": "SessionStart", "additionalContext": text}})
            .to_string()
    } else {
        text
    })
}

/// Tool items (`message.content` entries) from the transcript lines that hold any.
fn tool_items(transcript: &Path) -> Vec<Value> {
    let Ok(text) = std::fs::read_to_string(transcript) else {
        return Vec::new();
    };
    text.lines()
        .filter(|l| l.contains("\"tool_"))
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter_map(|v| {
            v.pointer("/message/content")
                .and_then(Value::as_array)
                .cloned()
        })
        .flatten()
        .filter(Value::is_object)
        .collect()
}

/// (tool calls, edit tool calls) in a transcript.
pub fn count_tools(transcript: &Path) -> (u64, u64) {
    let uses: Vec<Value> = tool_items(transcript)
        .into_iter()
        .filter(|i| i.get("type").and_then(Value::as_str) == Some("tool_use"))
        .collect();
    let edits = uses
        .iter()
        .filter(|i| {
            i.get("name")
                .and_then(Value::as_str)
                .is_some_and(|n| EDIT_TOOLS.contains(&n))
        })
        .count();
    (uses.len() as u64, edits as u64)
}

/// {file: errored tool calls}; hook gate denials (`PreToolUse:`) are not failures.
pub fn failed_files(transcript: &Path) -> BTreeMap<PathBuf, usize> {
    let mut paths: BTreeMap<String, PathBuf> = BTreeMap::new();
    let mut fails: BTreeMap<PathBuf, usize> = BTreeMap::new();
    for item in tool_items(transcript) {
        let kind = item.get("type").and_then(Value::as_str);
        if kind == Some("tool_use") {
            let path = item
                .get("input")
                .and_then(|i| i.get("file_path").or_else(|| i.get("path")))
                .and_then(Value::as_str)
                .filter(|p| !p.is_empty());
            if let (Some(id), Some(p)) = (item.get("id").and_then(Value::as_str), path) {
                paths.insert(id.to_string(), PathBuf::from(p));
            }
        } else if kind == Some("tool_result")
            && item.get("is_error").and_then(Value::as_bool) == Some(true)
        {
            let denied = item
                .get("content")
                .map(|c| c.to_string().chars().take(300).collect::<String>())
                .is_some_and(|c| c.contains("PreToolUse:"));
            let id = item.get("tool_use_id").and_then(Value::as_str);
            if let Some(p) = id.and_then(|id| paths.get(id)).filter(|_| !denied) {
                *fails.entry(p.clone()).or_default() += 1;
            }
        }
    }
    fails
}

/// (project slug, repo-relative posix path) for a file under the projects root, else None.
/// Case-insensitive, so lowercased transcript paths still match on Windows.
pub fn project_of(projects: &Path, path: &Path) -> Option<(String, String)> {
    let norm = |p: &Path| {
        std::path::absolute(p)
            .unwrap_or_else(|_| p.to_path_buf())
            .to_string_lossy()
            .replace('\\', "/")
    };
    let (file, root) = (norm(path), norm(projects));
    let prefix = format!("{}/", root.trim_end_matches('/'));
    if file.len() <= prefix.len() || !file[..prefix.len()].eq_ignore_ascii_case(&prefix) {
        return None;
    }
    let (project, target) = file[prefix.len()..].split_once('/')?;
    Some((crate::common::slugify(project), target.to_string()))
}

fn state_path(cfg: &HiveConfig) -> PathBuf {
    std::env::var("HIVE_ACE_STATE")
        .ok()
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| cfg.db.with_file_name("ace-state.json"))
}

/// Stop: hot-file traces, then the checkpoint decision (hook JSON) when one is due.
pub fn stop(cfg: &HiveConfig, input: &HookInput, scope: Option<&str>) -> Option<String> {
    input.set_provenance();
    let scope = input.scope(cfg, scope);
    crate::ledger::record(
        &cfg.ledger,
        "reflect",
        &scope,
        "hook-stop",
        &[],
        &[],
        0.0,
        None,
        None,
    );
    if input.stop_hook_active || std::env::var("HIVE_ACE_DISABLE").is_ok_and(|v| !v.is_empty()) {
        return None;
    }
    let (Some(session), Some(transcript)) = (&input.session_id, &input.transcript_path) else {
        return Some(format!("ace: stop hook triggered for scope '{scope}'"));
    };
    let transcript = Path::new(transcript);
    if let Err(e) = emit_hot_files(cfg, transcript, session) {
        eprintln!("ace: hot-file traces skipped: {e}");
    }
    let (tools, edits) = count_tools(transcript);
    let path = state_path(cfg);
    let mut state: BTreeMap<String, u64> = std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    if tools.saturating_sub(*state.get(session).unwrap_or(&0)) < STOP_TOOL_THRESHOLD || edits == 0 {
        return None;
    }
    state.insert(session.clone(), tools);
    while state.len() > STATE_KEEP {
        let first = state.keys().next().cloned().unwrap();
        state.remove(&first);
    }
    let saved = path
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|_| std::fs::write(&path, serde_json::to_string(&state).unwrap_or_default()));
    if let Err(e) = saved {
        eprintln!("ace: checkpoint state not saved: {e}");
    }
    Some(serde_json::json!({"decision": "block", "reason": format!(
        "Hive ACE checkpoint ({tools} tool calls). Run the `hive-ace` skill now for scope `{scope}`: reflect on this session, tag playbook bullets you used, add at most 3 new lessons, apply with ace.py, then finish your previous reply unchanged. Skip if nothing reusable was learned."
    )}).to_string())
}

/// TRACE-05: a HOT_FILE trace for each file with >= HOT_FILE_MIN_FAILS errored calls, filed under the file's
/// own project with a repo-relative target; files outside the projects root are skipped.
fn emit_hot_files(cfg: &HiveConfig, transcript: &Path, session: &str) -> Result<(), String> {
    let hot: Vec<(String, String, usize)> = failed_files(transcript)
        .into_iter()
        .filter(|(_, n)| *n >= HOT_FILE_MIN_FAILS)
        .filter_map(|(p, n)| project_of(&cfg.projects, &p).map(|(proj, t)| (proj, t, n)))
        .collect();
    if hot.is_empty() {
        return Ok(());
    }
    let conn = crate::index::open_db(&cfg.db).map_err(|e| e.to_string())?.0;
    let now = chrono::Utc::now().timestamp();
    for (project, target, n) in hot {
        crate::trace::add(
            &conn,
            &project,
            "HOT_FILE",
            &target,
            &format!("{n} failed tool calls in one session"),
            "claude",
            session,
            now,
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(v: Value) -> String {
        serde_json::json!({"message": {"content": [v]}}).to_string()
    }

    #[test]
    fn test_parse_count_and_failed_files() {
        let dir = std::env::temp_dir().join(format!("hive-hooks-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let t = dir.join("t.jsonl");
        let mut lines = Vec::new();
        for i in 0..4 {
            lines.push(line(
                serde_json::json!({"type": "tool_use", "id": format!("u{i}"), "name": "Edit",
                "input": {"file_path": "D:/PROJECTS/Demo App/src/a.rs"}}),
            ));
            lines.push(line(
                serde_json::json!({"type": "tool_result", "tool_use_id": format!("u{i}"),
                "is_error": true, "content": if i == 3 { "PreToolUse: gate" } else { "boom" }}),
            ));
        }
        lines.push(line(
            serde_json::json!({"type": "tool_use", "id": "r1", "name": "Read", "input": {}}),
        ));
        std::fs::write(&t, lines.join("\n")).unwrap();
        assert_eq!(count_tools(&t), (5, 4));
        let fails = failed_files(&t);
        assert_eq!(fails.values().copied().collect::<Vec<_>>(), vec![3]);
        assert_eq!(
            project_of(
                Path::new("D:/PROJECTS"),
                Path::new("d:/projects/Demo App/src/a.rs")
            ),
            Some(("demo-app".to_string(), "src/a.rs".to_string()))
        );
        assert_eq!(
            project_of(Path::new("D:/PROJECTS"), Path::new("C:/other/a.rs")),
            None
        );
        let input = HookInput::parse(
            r#"{"session_id": "s1", "transcript_path": "x", "stop_hook_active": true}"#,
        );
        assert!(
            input.present && input.stop_hook_active && input.session_id.as_deref() == Some("s1")
        );
        assert!(!HookInput::parse("not json").present);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
