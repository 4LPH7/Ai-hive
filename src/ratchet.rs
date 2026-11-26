//! RATCHET-02 (v2.8.0): turn canonical playbook lessons that ban something into deterministic lint rules.
//! `hive ratchet [scope]` drafts `inbox/ratchet-<rule-id>.md` for each canonical bullet that bans a
//! `backticked` token; `hive ratchet accept <rule-id>` (a human step) appends the rule to
//! `_meta/ratchet_rules.jsonl`. Vault rules are enforced by `hive lint`, repo rules by `hive pre-commit` on
//! staged files, and the source bullet stops being injected. Patterns are literal substrings, never regexes.
//! Mirror of `ratchet.py`.

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const TARGETS: [&str; 2] = ["vault", "repo"];
const BAN_RE: &str = r"(?i)\b(never|don't|do not|avoid|must not|banned|forbidden|blocks?)\b";
const TICK_RE: &str = r"`([^`]{3,80})`";
const EXT_RE: &str = r"\.(rs|py|md|toml|json|jsonl|ps1|sh|ts|js)\b";

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

/// A rule draft for a bullet that bans a backticked token, else None.
pub fn detect(scope: &str, bullet_id: &str, content: &str) -> Option<Rule> {
    let ban = Regex::new(BAN_RE).unwrap().find(content)?;
    let tick = Regex::new(TICK_RE)
        .unwrap()
        .captures_at(content, ban.start())?;
    let token = tick.get(1)?.as_str();
    let ext_re = Regex::new(EXT_RE).unwrap();
    let ext = ext_re
        .captures(token)
        .or_else(|| ext_re.captures(content))
        .map(|c| c[1].to_string());
    let glob = ext
        .as_ref()
        .map(|e| format!("**/*.{e}"))
        .unwrap_or_else(|| "**/*".into());
    let target = if ext.as_deref() == Some("md") {
        "vault"
    } else {
        "repo"
    };
    Some(Rule {
        id: format!("rat-{scope}-{bullet_id}"),
        scope: scope.into(),
        bullet: bullet_id.into(),
        pattern: token.into(),
        target: target.into(),
        glob,
        message: content.chars().take(200).collect(),
    })
}

fn render_proposal(rule: &Rule, agent: &str, session: &str, today: &str) -> String {
    format!(
        "---\ntype: note\nproject: {}\nagent: {agent}\nstatus: active\nupdated: {today}\nsource: session {session}\n---\n# Ratchet proposal: {}\n\nCanonical lesson ({}): {}\n\nProposed rule: `{}` files matching `{}` must not contain the literal text below.\n\n```json\n{}\n```\n\nAccept (human): `hive ratchet accept {}` (optional `--target vault|repo`, `--glob <glob>`). The lesson then stops being injected.\n",
        rule.scope,
        rule.bullet,
        rule.scope,
        rule.message,
        rule.target,
        rule.glob,
        serde_json::to_string(rule).unwrap_or_default(),
        rule.id
    )
}

/// Write one inbox proposal per new candidate; returns the written paths (existing ones are kept).
pub fn propose(
    vault: &Path,
    scopes: &[String],
    agent: &str,
    session: &str,
    today: &str,
) -> Result<Vec<PathBuf>, String> {
    let accepted: Vec<String> = load_rules(vault).into_iter().map(|r| r.id).collect();
    let mut written = Vec::new();
    for scope in scopes {
        let meta = crate::quorum::load(&crate::quorum::meta_path(&crate::ace::scope_path(
            vault, scope,
        )?));
        let mut bullets: Vec<crate::ace::Bullet> = crate::ace::load(vault, scope)?
            .into_values()
            .flatten()
            .collect();
        bullets.sort_by(|a, b| a.id.cmp(&b.id));
        for b in bullets {
            if crate::quorum::state_of(&meta, &b.id) != "canonical" {
                continue;
            }
            let Some(rule) = detect(scope, &b.id, &b.content) else {
                continue;
            };
            let path = vault.join("inbox").join(format!("ratchet-{}.md", rule.id));
            if accepted.contains(&rule.id) || path.exists() {
                continue;
            }
            std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
            std::fs::write(&path, render_proposal(&rule, agent, session, today))
                .map_err(|e| e.to_string())?;
            written.push(path);
        }
    }
    Ok(written)
}

/// Append the proposed rule (optionally retargeted) to the rule store; returns the rule.
pub fn accept(
    vault: &Path,
    rule_id: &str,
    target: Option<&str>,
    glob: Option<&str>,
) -> Result<Rule, String> {
    let path = vault.join("inbox").join(format!("ratchet-{rule_id}.md"));
    let text = std::fs::read_to_string(&path).map_err(|_| {
        format!("ratchet: no proposal inbox/ratchet-{rule_id}.md; run `hive ratchet` first")
    })?;
    let block = Regex::new(r"(?s)```json\n(.+?)\n```")
        .unwrap()
        .captures(&text)
        .map(|c| c[1].to_string());
    let mut rule: Rule = block
        .and_then(|b| serde_json::from_str(&b).ok())
        .filter(|r: &Rule| r.id == rule_id)
        .ok_or_else(|| format!("ratchet: proposal {rule_id} has no matching rule block"))?;
    if let Some(t) = target {
        rule.target = t.into();
    }
    if let Some(g) = glob {
        rule.glob = g.into();
    }
    if !TARGETS.contains(&rule.target.as_str()) || rule.pattern.trim().is_empty() {
        return Err("ratchet: target must be vault or repo and the pattern non-empty".into());
    }
    if load_rules(vault).iter().any(|r| r.id == rule_id) {
        return Err(format!("ratchet: rule {rule_id} is already accepted"));
    }
    let store = rules_path(vault);
    std::fs::create_dir_all(store.parent().unwrap()).map_err(|e| e.to_string())?;
    let mut line = serde_json::to_string(&rule).map_err(|e| e.to_string())?;
    line.push('\n');
    use std::io::Write;
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&store)
        .and_then(|mut f| f.write_all(line.as_bytes()))
        .map_err(|e| e.to_string())?;
    Ok(rule)
}

/// Vault files that legitimately quote banned tokens: the lessons, proposals, the rule store.
fn exempt(rel: &str) -> bool {
    rel == "_meta/playbook.md"
        || rel.starts_with("inbox/ratchet-")
        || (rel.starts_with("projects/")
            && rel.ends_with("/playbook.md")
            && rel.matches('/').count() == 2)
}

/// (rule, 1-based line) for every rule of this target whose glob matches rel and whose pattern is in text.
pub fn check_text<'a>(
    rules: &'a [Rule],
    target: &str,
    rel: &str,
    text: &str,
) -> Vec<(&'a Rule, usize)> {
    if target == "vault" && exempt(rel) {
        return Vec::new();
    }
    rules
        .iter()
        .filter(|r| r.target == target && glob_match(rel, &r.glob))
        .filter_map(|r| {
            text.find(&r.pattern)
                .map(|i| (r, text[..i].matches('\n').count() + 1))
        })
        .collect()
}

/// (path, line, rule) for staged files (added/copied/modified) in `repo` that break a `repo` rule.
pub fn staged_violations(vault: &Path, repo: &Path) -> Vec<(String, usize, Rule)> {
    let rules: Vec<Rule> = load_rules(vault)
        .into_iter()
        .filter(|r| r.target == "repo")
        .collect();
    if rules.is_empty() {
        return Vec::new();
    }
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .current_dir(repo)
            .output()
            .map(|o| o.stdout)
            .unwrap_or_default()
    };
    let names: Vec<String> = String::from_utf8_lossy(&git(&[
        "diff",
        "--cached",
        "--name-only",
        "--diff-filter=ACM",
    ]))
    .lines()
    .map(str::to_string)
    .collect();
    let mut out = Vec::new();
    for name in names {
        if !rules.iter().any(|r| glob_match(&name, &r.glob)) {
            continue;
        }
        let text = String::from_utf8_lossy(&git(&["show", &format!(":{name}")])).into_owned();
        for (r, line) in check_text(&rules, "repo", &name, &text) {
            out.push((name.clone(), line, r.clone()));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const LESSON: &str =
        "Never run `python -c` in the lean-ctx shell; write a script file and run python -I script.py.";

    #[test]
    fn test_detect_and_glob_match_python() {
        let r = detect("global", "cmd-00003", LESSON).unwrap();
        assert_eq!(
            (r.pattern.as_str(), r.target.as_str(), r.glob.as_str()),
            ("python -c", "repo", "**/*.py")
        );
        assert!(detect("global", "x", "write JSON to a `file`, not echo").is_none());
        assert!(glob_match("a.py", "**/*.py") && glob_match("src/a/b.py", "**/*.py"));
        assert!(!glob_match("a.rs", "**/*.py") && glob_match("x", ""));
        assert!(exempt("_meta/playbook.md") && exempt("projects/demo/playbook.md"));
        assert!(!exempt("projects/demo/notes/playbook.md") && !exempt("notes/a.md"));
    }

    #[test]
    fn test_propose_accept_lint_and_retire() {
        let vault = std::env::temp_dir().join(format!("hive-ratchet-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&vault);
        let pb = crate::ace::scope_path(&vault, "global").unwrap();
        std::fs::create_dir_all(pb.parent().unwrap()).unwrap();
        std::fs::write(
            &pb,
            format!("## commands\n- [cmd-00003] seen=6 helpful=6 harmful=0 :: {LESSON}\n- [cmd-00004] seen=2 helpful=2 harmful=0 :: write JSON to a file, not echo\n"),
        )
        .unwrap();
        let written = propose(&vault, &["global".into()], "claude", "s1", "2026-11-26").unwrap();
        assert_eq!(written.len(), 1);
        assert!(
            propose(&vault, &["global".into()], "claude", "s1", "2026-11-26")
                .unwrap()
                .is_empty()
        );
        let rule = accept(
            &vault,
            "rat-global-cmd-00003",
            Some("vault"),
            Some("**/*.md"),
        )
        .unwrap();
        assert!(accept(&vault, "rat-global-cmd-00003", None, None).is_err());
        let rules = load_rules(&vault);
        assert_eq!(rules, vec![rule]);
        assert!(retired(&rules, "global", "cmd-00003") && !retired(&rules, "global", "cmd-00004"));
        let text = "---\ntype: note\n---\n# how\n\nrun python -c 'x'\n";
        assert_eq!(check_text(&rules, "vault", "notes/how.md", text)[0].1, 6);
        assert!(check_text(&rules, "vault", "_meta/playbook.md", text).is_empty());
        assert!(check_text(&rules, "repo", "notes/how.md", text).is_empty());
        let injectable = crate::ace::load_injectable(&vault, "global").unwrap();
        let ids: Vec<&str> = injectable
            .values()
            .flatten()
            .map(|b| b.id.as_str())
            .collect();
        assert!(ids.contains(&"cmd-00004") && !ids.contains(&"cmd-00003"));
        let _ = std::fs::remove_dir_all(&vault);
    }
}
