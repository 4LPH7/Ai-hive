//! Ratchet rule storage and matching (RATCHET-02): a leaf that ace injection and lint can use without
//! depending on ratchet drafting. `_meta/ratchet_rules.jsonl` holds accepted rules.

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const TARGETS: [&str; 2] = ["vault", "repo"];

/// Field order is the wire order: byte-identical to `ratchet.py` (json.dumps, compact separators).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rule {
    pub id: String,
    #[serde(default)]
    pub scope: String,
    #[serde(default)]
    pub bullet: String,
    pub pattern: String,
    pub target: String,
    #[serde(default)]
    pub glob: String,
    #[serde(default)]
    pub message: String,
}

pub fn rules_path(vault: &Path) -> PathBuf {
    vault.join("_meta").join("ratchet_rules.jsonl")
}

pub fn load_rules(vault: &Path) -> Vec<Rule> {
    let Ok(text) = std::fs::read_to_string(rules_path(vault)) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|l| serde_json::from_str::<Rule>(l).ok())
        .filter(|r| {
            !r.pattern.is_empty() && !r.id.is_empty() && TARGETS.contains(&r.target.as_str())
        })
        .collect()
}

fn glob_re(glob: &str) -> Regex {
    let body: String = glob
        .chars()
        .map(|c| match c {
            '*' => ".*".to_string(),
            '?' => ".".to_string(),
            c => regex::escape(&c.to_string()),
        })
        .collect();
    Regex::new(&format!("^(?s:{body})$")).expect("escaped glob is a valid regex")
}

/// `*` matches any run of characters (crossing `/`), `?` one character; a leading `**/` also matches at the
/// top level. Same semantics as `ratchet.glob_match`.
pub fn glob_match(path: &str, glob: &str) -> bool {
    let glob = if glob.is_empty() { "**/*" } else { glob };
    glob_re(glob).is_match(path)
        || glob
            .strip_prefix("**/")
            .is_some_and(|rest| glob_re(rest).is_match(path))
}
/// A bullet whose rule shipped is no longer injected.
pub fn retired(rules: &[Rule], scope: &str, bullet_id: &str) -> bool {
    rules
        .iter()
        .any(|r| r.scope == scope && r.bullet == bullet_id)
}
