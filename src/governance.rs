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
    for (kind, path) in note_files(vault) {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let (session, agent, ts) = note_origin(&text);
        out.push((kind, session, agent, ts));
    }
    out
}

/// inbox/*.md and codices/*/*.md, the note kinds a rule can quarantine.
fn note_files(vault: &Path) -> Vec<(&'static str, PathBuf)> {
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

fn source_session(source: &str) -> String {
    regex::Regex::new(r"\bsession\s+(\S+)")
        .unwrap()
        .captures(source)
        .map(|c| c[1].to_string())
        .unwrap_or_default()
}

/// (session, agent, ts) of an inbox proposal or codex from its frontmatter.
pub fn note_origin(text: &str) -> (String, String, i64) {
    let (meta, _, _) = crate::index::parse_note(text);
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

/// Drop quarantined traces from trace::active() output.
pub fn visible_traces(
    rules: &[Rule],
    items: Vec<(crate::trace::Trace, f64)>,
) -> Vec<(crate::trace::Trace, f64)> {
    items
        .into_iter()
        .filter(|(t, _)| !is_quarantined(rules, &t.session_id, &t.agent, t.created_at))
        .collect()
}

/// A scout/quorum bullet whose origin an active rule matches.
pub fn bullet_hidden(rules: &[Rule], rec: Option<&crate::quorum::Meta>) -> bool {
    rec.is_some_and(|m| {
        !rules.is_empty()
            && matches!(m.state.as_str(), "scout" | "quorum")
            && is_quarantined(rules, &m.origin_session, &m.origin_agent, m.origin_ts)
    })
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

// ---------------------------------------------------------------- GOV-03 audit (mirror of governance.py)
pub const AUDIT_LEDGER_LINES: usize = 10;

fn quarantine_line(rules: &[Rule], session: &str, agent: &str, ts: i64) -> String {
    let ids: Vec<&str> = rules
        .iter()
        .filter(|r| rule_matches(r, session, agent, ts))
        .map(|r| r.id.as_str())
        .collect();
    if ids.is_empty() {
        "  quarantined: no".into()
    } else {
        format!("  quarantined: yes ({})", ids.join(", "))
    }
}

fn or_q(s: &str) -> &str {
    if s.is_empty() {
        "?"
    } else {
        s
    }
}

fn audit_bullet(
    vault: &Path,
    ledger_path: &Path,
    id: &str,
    rules: &[Rule],
) -> Result<Vec<String>, String> {
    let mut playbooks = vec![(
        "global".to_string(),
        vault.join("_meta").join("playbook.md"),
    )];
    if let Ok(rd) = std::fs::read_dir(vault.join("projects")) {
        let mut more: Vec<(String, PathBuf)> = rd
            .flatten()
            .map(|e| {
                (
                    e.file_name().to_string_lossy().to_string(),
                    e.path().join("playbook.md"),
                )
            })
            .collect();
        more.sort();
        playbooks.extend(more);
    }
    for (scope, pb) in playbooks {
        let Ok(text) = std::fs::read_to_string(&pb) else {
            continue;
        };
        let sections = crate::ace::parse(&text);
        let Some((name, b)) = sections.iter().find_map(|(n, bs)| {
            bs.iter()
                .find(|b| b.id == id)
                .map(|b| (n.clone(), b.clone()))
        }) else {
            continue;
        };
        let meta = crate::quorum::load(&crate::quorum::meta_path(&pb));
        let rec = meta.get(id).cloned().unwrap_or(crate::quorum::Meta {
            id: id.into(),
            state: crate::quorum::DEFAULT_STATE.into(),
            origin_session: String::new(),
            origin_agent: String::new(),
            origin_ts: 0,
            evidence: Vec::new(),
        });
        let mut out = vec![
            format!(
                "bullet {id} ({scope}/{name}) seen={} helpful={} harmful={}",
                b.seen, b.helpful, b.harmful
            ),
            format!("  text: {}", b.content),
            format!(
                "  state: {} score={} sessions={}",
                rec.state,
                crate::quorum::score(&rec),
                crate::quorum::sessions(&rec).len()
            ),
            format!(
                "  origin: session={} agent={} ts={}",
                or_q(&rec.origin_session),
                or_q(&rec.origin_agent),
                rec.origin_ts
            ),
        ];
        out.extend(rec.evidence.iter().map(|e| {
            format!(
                "  evidence: {} session={} agent={}",
                e.kind, e.session, e.agent
            )
        }));
        let all: Vec<(String, String)> = sections
            .values()
            .flatten()
            .map(|b| (b.id.clone(), b.content.clone()))
            .collect();
        for (a, b2, s) in crate::quorum::conflicts(&all) {
            if a == id || b2 == id {
                out.push(format!(
                    "  conflict: vs {} ({})",
                    if a == id { &b2 } else { &a },
                    s.join(", ")
                ));
            }
        }
        let events: Vec<_> = crate::ledger::read_ledger(ledger_path, None, None)
            .into_iter()
            .filter(|e| e.injected_bullets.iter().any(|x| x == id))
            .collect();
        for e in events
            .iter()
            .skip(events.len().saturating_sub(AUDIT_LEDGER_LINES))
        {
            let mut line = format!(
                "  ledger: {} {} session={} agent={}",
                e.ts, e.event, e.session_id, e.agent
            );
            if !e.repo_sha.is_empty() {
                line.push_str(&format!(" repo_sha={}", e.repo_sha));
            }
            if !e.content_sha256.is_empty() {
                line.push_str(&format!(
                    " content_sha256={}",
                    &e.content_sha256[..12.min(e.content_sha256.len())]
                ));
            }
            out.push(line);
        }
        out.push(if matches!(rec.state.as_str(), "scout" | "quorum") {
            quarantine_line(rules, &rec.origin_session, &rec.origin_agent, rec.origin_ts)
        } else {
            "  quarantined: no (only scout/quorum bullets can be)".into()
        });
        return Ok(out);
    }
    Err(format!("audit: no bullet {id} in any playbook"))
}

fn audit_trace(
    conn: &Connection,
    raw: &str,
    rules: &[Rule],
    now: i64,
) -> Result<Vec<String>, String> {
    let tid: i64 = raw
        .parse()
        .map_err(|_| format!("audit: bad trace id {raw:?}"))?;
    crate::trace::ensure_table(conn)?;
    let row = conn
        .query_row(
            "SELECT project, kind, target, summary, agent, session_id, w0, created_at, half_life_h FROM traces WHERE id=?1",
            [tid],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, f64>(6)?,
                    r.get::<_, i64>(7)?,
                    r.get::<_, f64>(8)?,
                ))
            },
        )
        .map_err(|_| format!("audit: no trace {tid} (expired traces are purged after 7 days)"))?;
    let (project, kind, target, summary, agent, session, w0, created, hl) = row;
    let w = crate::trace::strength(w0, created, now, hl);
    Ok(vec![
        format!("trace {tid} {kind} {target} ({project})"),
        format!("  summary: {summary}"),
        format!("  origin: session={session} agent={agent} created_at={created}"),
        format!(
            "  strength: {w:.2} (w0={w0}, half-life {hl} h, hidden below {})",
            crate::trace::MIN_STRENGTH
        ),
        quarantine_line(rules, &session, &agent, created),
    ])
}

fn audit_codex(vault: &Path, stem: &str, rules: &[Rule]) -> Result<Vec<String>, String> {
    let note = std::fs::read_dir(vault.join("codices"))
        .ok()
        .into_iter()
        .flat_map(|rd| rd.flatten())
        .map(|p| p.path().join(format!("{stem}.md")))
        .find(|p| p.is_file())
        .ok_or_else(|| {
            format!("audit: nothing matches {stem:?} (bullet id, trace:<n> or codex file stem)")
        })?;
    let text = std::fs::read_to_string(&note).map_err(|e| e.to_string())?;
    let (meta, title, _) = crate::index::parse_note(&text);
    let (session, agent, ts) = note_origin(&text);
    let rel = note
        .strip_prefix(vault)
        .unwrap_or(&note)
        .to_string_lossy()
        .replace('\\', "/");
    let integrity = crate::codex::verify(&text).unwrap_or_else(|| "ok".into());
    Ok(vec![
        format!(
            "codex {rel} ({})",
            meta.get("project").cloned().unwrap_or_default()
        ),
        format!("  goal: {title}"),
        format!(
            "  origin: session={} agent={} updated={}",
            or_q(&session),
            or_q(&agent),
            meta.get("updated").cloned().unwrap_or_default()
        ),
        format!("  integrity: {integrity}"),
        quarantine_line(rules, &session, &agent, ts),
    ])
}

/// GOV-03: provenance chain of a bullet id, `trace:<n>` or a codex file stem, as printable lines.
pub fn audit(
    vault: &Path,
    conn: &Connection,
    ledger_path: &Path,
    target: &str,
    now: i64,
) -> Result<Vec<String>, String> {
    let target = target.trim();
    let rules = load_rules(vault);
    if regex::Regex::new(r"^[a-z]{3}-\d{5}$")
        .unwrap()
        .is_match(target)
    {
        return audit_bullet(vault, ledger_path, target, &rules);
    }
    if let Some(raw) = target.strip_prefix("trace:") {
        return audit_trace(conn, raw, &rules, now);
    }
    audit_codex(vault, target, &rules)
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
        // GOV-02 filters: the agent rule still active hides codex traces and the codex inbox note
        let rules = load_rules(&vault);
        let items = crate::trace::active(&conn, "demo", 1_800_000_000).unwrap();
        let kept = visible_traces(&rules, items);
        assert_eq!(
            kept.iter()
                .map(|(t, _)| t.agent.as_str())
                .collect::<Vec<_>>(),
            vec!["claude"]
        );
        assert!(quarantined_paths(&vault, &rules).contains("inbox/x.md"));
        assert!(note_hidden(
            &rules,
            "inbox/x.md",
            "hive curate",
            "codex",
            "2027-01-15"
        ));
        assert!(!note_hidden(
            &rules,
            "projects/demo/x.md",
            "",
            "codex",
            "2027-01-15"
        ));
        // GOV-03 audit
        let tid: i64 = conn
            .query_row("SELECT id FROM traces WHERE session_id='bad-s'", [], |r| {
                r.get(0)
            })
            .unwrap();
        let out = audit(
            &vault,
            &conn,
            &vault.join("l.jsonl"),
            &format!("trace:{tid}"),
            1_800_000_000,
        )
        .unwrap()
        .join("\n");
        assert!(
            out.contains(&format!("trace {tid} HOT_FILE src/b.rs (demo)")),
            "{out}"
        );
        assert!(
            out.contains("origin: session=bad-s agent=codex")
                && out.contains("quarantined: yes (q-"),
            "{out}"
        );
        for bad in ["pit-99999", "trace:999", "trace:x", "no-such-thing"] {
            assert!(
                audit(&vault, &conn, &vault.join("l.jsonl"), bad, 0).is_err(),
                "{bad}"
            );
        }
        let _ = std::fs::remove_dir_all(&vault);
    }
}
