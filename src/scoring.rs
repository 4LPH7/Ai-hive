//! Outcome scoring (`hive ledger score`): credits or penalises the playbook lessons and skills a session used,
//! from its test exit code. Lives outside `ledger` so the ledger stays a leaf module (it only records and reads).

use crate::ledger::{read_ledger, record, LedgerEntry};
use serde::{Deserialize, Serialize};
use std::path::Path;

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
            session: Some(target_session_id.clone()),
            // QUOR-02: a passing test run in the bullet's session is the strongest evidence
            evidence: is_success.then(|| "test_pass".to_string()),
        });
    }
    for s in &skills {
        tag_ops.push(crate::ace::TagOp {
            id: format!("skill:{}", s),
            helpful: if is_success { Some(1) } else { None },
            harmful: if !is_success { Some(1) } else { None },
            session: None,
            evidence: None,
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
