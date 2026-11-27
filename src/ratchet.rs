//! RATCHET-02 (v2.8.0): turn canonical playbook lessons that ban something into deterministic lint rules.
//! `hive ratchet [scope]` drafts `inbox/ratchet-<rule-id>.md` for each canonical bullet that bans a
//! `backticked` token; `hive ratchet accept <rule-id>` (a human step) appends the rule to
//! `_meta/ratchet_rules.jsonl`. Vault rules are enforced by `hive lint`, repo rules by `hive pre-commit` on
//! staged files, and the source bullet stops being injected. Patterns are literal substrings, never regexes.
//! Mirror of `ratchet.py`.

use regex::Regex;
use std::path::{Path, PathBuf};

const BAN_RE: &str = r"(?i)\b(never|don't|do not|avoid|must not|banned|forbidden|blocks?)\b";
const TICK_RE: &str = r"`([^`]{3,80})`";
const EXT_RE: &str = r"\.(rs|py|md|toml|json|jsonl|ps1|sh|ts|js)\b";

pub use crate::ratchet_rules::*;

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

// ---------------------------------------------------------------- RATCHET-03 release
/// A canonical lesson injected into >= RELEASE_SESSIONS distinct real sessions that never earned a helpful tag
/// (and has no shipped rule) is proposed for retirement; the human applies `{"remove": [id]}`. Mirror of
/// `ratchet.propose_release`.
pub const RELEASE_SESSIONS: usize = 20;

/// {bullet id: {session ids}} from `context` ledger events (global: every project; a project: its own);
/// anonymous per-process ids never count.
pub fn injection_sessions(
    entries: &[crate::ledger::LedgerEntry],
    scope: &str,
) -> std::collections::BTreeMap<String, std::collections::BTreeSet<String>> {
    let mut out: std::collections::BTreeMap<String, std::collections::BTreeSet<String>> =
        Default::default();
    for e in entries {
        if e.event != "context" || (scope != "global" && e.project != scope) {
            continue;
        }
        if e.session_id.is_empty() || e.session_id.starts_with("proc-") {
            continue;
        }
        for b in &e.injected_bullets {
            out.entry(b.clone())
                .or_default()
                .insert(e.session_id.clone());
        }
    }
    out
}

/// Write `inbox/retire-<scope>-<id>.md` per candidate (existing proposals kept); returns written paths.
pub fn propose_release(
    vault: &Path,
    scopes: &[String],
    entries: &[crate::ledger::LedgerEntry],
    agent: &str,
    session: &str,
    today: &str,
) -> Result<Vec<PathBuf>, String> {
    let rules = load_rules(vault);
    let mut written = Vec::new();
    for scope in scopes {
        let meta = crate::quorum::load(&crate::quorum::meta_path(&crate::ace::scope_path(
            vault, scope,
        )?));
        let sessions = injection_sessions(entries, scope);
        let mut bullets: Vec<crate::ace::Bullet> = crate::ace::load(vault, scope)?
            .into_values()
            .flatten()
            .collect();
        bullets.sort_by(|a, b| a.id.cmp(&b.id));
        for b in bullets {
            let n = sessions.get(&b.id).map_or(0, |s| s.len());
            if crate::quorum::state_of(&meta, &b.id) != "canonical"
                || b.helpful != 0
                || retired(&rules, scope, &b.id)
                || n < RELEASE_SESSIONS
            {
                continue;
            }
            let path = vault
                .join("inbox")
                .join(format!("retire-{scope}-{}.md", b.id));
            if path.exists() {
                continue;
            }
            std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
            std::fs::write(
                &path,
                format!(
                    "---\ntype: note\nproject: {scope}\nagent: {agent}\nstatus: active\nupdated: {today}\nsource: session {session}\n---\n# Retire proposal: {id}\n\nCanonical lesson ({scope}) injected into {n} sessions, never tagged helpful: {content}\n\nRetire (human): write `{{\"remove\": [\"{id}\"]}}` to a file and run `hive ace apply {scope} <file>`. Keep it by tagging it helpful once it earns its place.\n",
                    id = b.id,
                    content = b.content
                ),
            )
            .map_err(|e| e.to_string())?;
            written.push(path);
        }
    }
    Ok(written)
}

/// Scopes a ratchet run covers: the given one, else `global` plus every project folder with a playbook (sorted).
pub fn scopes(vault: &Path, scope: Option<String>) -> Result<Vec<String>, String> {
    if let Some(s) = scope {
        return Ok(vec![crate::common::validate_slug(&s)?]);
    }
    let mut v: Vec<String> = std::fs::read_dir(vault.join("projects"))
        .map(|rd| {
            rd.filter_map(Result::ok)
                .filter(|e| e.path().join("playbook.md").is_file())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    Ok(std::iter::once("global".to_string()).chain(v).collect())
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

    /// RATCHET-03 parity with tests/python/test_ratchet.py RatchetReleaseTest.
    #[test]
    fn test_release_proposes_only_never_helpful_heavily_injected() {
        let vault = std::env::temp_dir().join(format!("hive-ratchet-rel-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&vault);
        let pb = crate::ace::scope_path(&vault, "global").unwrap();
        std::fs::create_dir_all(pb.parent().unwrap()).unwrap();
        std::fs::write(
            &pb,
            "## strategies\n- [str-00001] seen=30 helpful=0 harmful=0 :: never helped lesson\n- [str-00002] seen=30 helpful=3 harmful=0 :: helped lesson\n- [str-00003] seen=5 helpful=0 harmful=0 :: rarely injected lesson\n",
        )
        .unwrap();
        let entry = |event: &str, session: String, bullets: &[&str]| crate::ledger::LedgerEntry {
            ts: String::new(),
            session_id: session,
            agent: "claude".into(),
            project: "demo".into(),
            event: event.into(),
            query: String::new(),
            hits: Vec::new(),
            injected_bullets: bullets.iter().map(|b| b.to_string()).collect(),
            duration_ms: 0.0,
            repo_sha: String::new(),
            content_sha256: String::new(),
        };
        let mut entries: Vec<_> = (0..20)
            .map(|i| {
                let mut b = vec!["str-00001", "str-00002"];
                if i < 5 {
                    b.push("str-00003");
                }
                entry("context", format!("s{i}"), &b)
            })
            .collect();
        entries.extend((0..30).map(|i| entry("context", format!("proc-{i}"), &["str-00003"])));
        entries.push(entry("query", "q1".into(), &["str-00003"]));
        let scopes = vec!["global".to_string()];
        let written =
            propose_release(&vault, &scopes, &entries, "claude", "s1", "2026-11-26").unwrap();
        assert_eq!(written.len(), 1);
        assert!(written[0].ends_with("retire-global-str-00001.md"));
        let note = std::fs::read_to_string(&written[0]).unwrap();
        assert!(
            note.contains("injected into 20 sessions")
                && note.contains(r#"{"remove": ["str-00001"]}"#)
        );
        assert!(
            propose_release(&vault, &scopes, &entries, "claude", "s1", "2026-11-26")
                .unwrap()
                .is_empty()
        );
        assert!(injection_sessions(&entries, "other-project").is_empty());
        let _ = std::fs::remove_dir_all(&vault);
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
