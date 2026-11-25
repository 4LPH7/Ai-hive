//! Session codices (v2.6.0, CODEX-02). `hive codex write <project> <json>` turns a hive-ace checkpoint
//! record into an immutable vault note `codices/<project>/YYYY-MM-DD-<sha8>.md`; its frontmatter carries the
//! body's sha256 for tamper detection (CODEX-04). Each dead end also becomes a DEAD_END trace.
//! Rendering is byte-identical to `codex.py`.

use rusqlite::Connection;
use serde::Deserialize;
use std::io::Write;
use std::path::{Path, PathBuf};

pub const MAX_LINES: usize = 40;
pub const MAX_ITEM_CHARS: usize = 300;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub goal: String,
    pub outcome: String,
    pub files: Vec<String>,
    pub dead_ends: Vec<String>,
    pub invariants: Vec<String>,
    pub open_threads: Vec<String>,
}

fn check_text(name: &str, value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err(format!("codex: {name} must be a non-empty string"));
    }
    if value.contains('\n') || value.contains('\r') || value.chars().count() > MAX_ITEM_CHARS {
        return Err(format!("codex: {name} must be one line of at most {MAX_ITEM_CHARS} chars"));
    }
    if crate::ace::is_secret(value) {
        return Err(format!("codex: {name} looks like it contains a secret"));
    }
    Ok(())
}

pub fn parse(json: &str) -> Result<Record, String> {
    let rec: Record = serde_json::from_str(json).map_err(|e| format!("codex: bad record: {e}"))?;
    check_text("goal", &rec.goal)?;
    check_text("outcome", &rec.outcome)?;
    for (name, list) in [
        ("files", &rec.files),
        ("dead_ends", &rec.dead_ends),
        ("invariants", &rec.invariants),
        ("open_threads", &rec.open_threads),
    ] {
        for item in list {
            check_text(name, item)?;
        }
    }
    Ok(rec)
}

/// (sha256 of body, full note text).
pub fn render(project: &str, rec: &Record, agent: &str, session: &str, date: &str) -> (String, String) {
    let mut body = vec![format!("# {}", rec.goal), String::new(), format!("outcome: {}", rec.outcome), String::new()];
    for (title, list) in [
        ("Files", &rec.files),
        ("Dead ends", &rec.dead_ends),
        ("Invariants", &rec.invariants),
        ("Open threads", &rec.open_threads),
    ] {
        body.push(format!("## {title}"));
        if list.is_empty() {
            body.push("- none".to_string());
        }
        body.extend(list.iter().map(|x| format!("- {x}")));
    }
    let body_text = body.join("\n") + "\n";
    let sha = crate::common::sha256(body_text.as_bytes());
    let head = [
        "---".to_string(),
        "type: codex".to_string(),
        format!("project: {project}"),
        format!("agent: {agent}"),
        "status: active".to_string(),
        format!("updated: {date}"),
        format!("source: session {session}"),
        format!("content_sha256: {sha}"),
        "---".to_string(),
    ];
    let text = head.join("\n") + "\n" + &body_text;
    (sha, text)
}

pub const CONTEXT_MAX_LINES: usize = 6;
pub const CONTEXT_MAX_CODICES: usize = 20;
const CONTEXT_KINDS: [(&str, &str); 2] = [("Dead ends", "dead end"), ("Open threads", "open thread")];

/// {section title: items} from a rendered codex body; `- none` placeholders dropped.
fn sections(text: &str) -> std::collections::HashMap<String, Vec<String>> {
    let mut out: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
    let mut cur: Option<String> = None;
    for line in text.lines() {
        if let Some(title) = line.strip_prefix("## ") {
            let title = title.trim().to_string();
            out.entry(title.clone()).or_default();
            cur = Some(title);
        } else if let (Some(c), Some(item)) = (&cur, line.strip_prefix("- ")) {
            if line != "- none" {
                out.get_mut(c).unwrap().push(item.to_string());
            }
        }
    }
    out
}

/// CODEX-03: Dead ends + Open threads from the newest codices whose Files overlap the task.
pub fn context_section(vault: &Path, project: &str, task: &str) -> String {
    let want = crate::trace::words(task);
    let folder = vault.join("codices").join(project);
    if want.is_empty() || !folder.is_dir() {
        return String::new();
    }
    let mut notes: Vec<PathBuf> = std::fs::read_dir(&folder)
        .map(|rd| rd.filter_map(Result::ok).map(|e| e.path()).collect())
        .unwrap_or_default();
    notes.retain(|p| p.extension().is_some_and(|x| x == "md"));
    notes.sort_by(|a, b| b.file_name().cmp(&a.file_name()));
    let mut lines = Vec::new();
    for note in notes.iter().take(CONTEXT_MAX_CODICES) {
        let Ok(text) = std::fs::read_to_string(note) else { continue };
        let secs = sections(&text);
        let files = secs.get("Files").map(|f| f.join(" ")).unwrap_or_default();
        if crate::trace::words(&files).is_disjoint(&want) {
            continue;
        }
        let stem = note.file_stem().unwrap_or_default().to_string_lossy();
        for (title, kind) in CONTEXT_KINDS {
            for item in secs.get(title).into_iter().flatten() {
                lines.push(format!("- {kind}: {item} ({stem})"));
            }
        }
    }
    lines.truncate(CONTEXT_MAX_LINES);
    if lines.is_empty() {
        return String::new();
    }
    format!("## codex notes for this task (dead ends, open threads)\n{}", lines.join("\n"))
}

/// Validate, write the note (never overwrite), leave DEAD_END traces; returns the note path.
#[allow(clippy::too_many_arguments)]
pub fn write(
    vault: &Path,
    conn: &Connection,
    project: &str,
    rec: &Record,
    agent: &str,
    session: &str,
    date: &str,
    now: i64,
) -> Result<PathBuf, String> {
    let project = crate::common::validate_slug(project)?;
    let (sha, text) = render(&project, rec, agent, session, date);
    let lines = text.lines().count();
    if lines > MAX_LINES {
        return Err(format!("codex: note would be {lines} lines (max {MAX_LINES}); trim the record"));
    }
    let path = vault.join("codices").join(&project).join(format!("{date}-{}.md", &sha[..8]));
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    let mut fh = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::AlreadyExists => {
                format!("codex: {} already exists; codices are never edited", path.display())
            }
            _ => e.to_string(),
        })?;
    fh.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
    let target = rec.files.first().map(String::as_str).unwrap_or("session");
    for dead_end in &rec.dead_ends {
        let summary: String = dead_end.chars().take(crate::trace::MAX_SUMMARY_CHARS).collect();
        crate::trace::add(conn, &project, "DEAD_END", target, &summary, agent, session, now)?;
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    const REC: &str = r#"{"goal": "add codex write", "outcome": "done: tests pass",
        "files": ["src/codex.rs"], "dead_ends": ["serde_yaml frontmatter: dropped, hand-rendered is simpler"],
        "invariants": ["eval Recall@5 >= 0.97"], "open_threads": []}"#;

    #[test]
    fn test_writes_immutable_hashed_note_and_dead_end_traces() {
        let tmp = std::env::temp_dir().join(format!("hive-codex-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let conn = Connection::open_in_memory().unwrap();
        let rec = parse(REC).unwrap();
        let p = write(&tmp, &conn, "demo", &rec, "claude", "s1", "2026-11-25", 0).unwrap();
        assert_eq!(p.parent().unwrap(), tmp.join("codices").join("demo"));
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.lines().count() <= MAX_LINES);
        let (head, body) = text.split_once("\n---\n").unwrap();
        let sha = head.lines().find_map(|l| l.strip_prefix("content_sha256: ")).unwrap();
        assert_eq!(sha, crate::common::sha256(body.as_bytes()));
        assert!(head.contains("type: codex") && body.contains("- none"));
        let traces = crate::trace::active(&conn, "demo", 0).unwrap();
        assert_eq!((traces[0].0.kind.as_str(), traces[0].0.target.as_str()), ("DEAD_END", "src/codex.rs"));
        assert!(write(&tmp, &conn, "demo", &rec, "claude", "s1", "2026-11-25", 0).is_err());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_rejects_bad_records() {
        for bad in [
            r#"{"goal": "", "outcome": "o", "files": [], "dead_ends": [], "invariants": [], "open_threads": []}"#,
            r#"{"goal": "g", "outcome": "o", "files": [], "dead_ends": [], "invariants": [], "open_threads": [], "extra": 1}"#,
            r#"{"goal": "g", "outcome": "o", "dead_ends": [], "invariants": [], "open_threads": []}"#,
            r#"{"goal": "g", "outcome": "o", "files": "src/a.rs", "dead_ends": [], "invariants": [], "open_threads": []}"#,
            r#"{"goal": "g", "outcome": "two\nlines", "files": [], "dead_ends": [], "invariants": [], "open_threads": []}"#,
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
        let conn = Connection::open_in_memory().unwrap();
        let tmp = std::env::temp_dir().join(format!("hive-codex-bad-{}", std::process::id()));
        let mut rec = parse(REC).unwrap();
        assert!(write(&tmp, &conn, "../x", &rec, "a", "s", "2026-11-25", 0).is_err());
        rec.open_threads = vec!["x".to_string(); 40];
        assert!(write(&tmp, &conn, "demo", &rec, "a", "s", "2026-11-25", 0).is_err());
        assert!(!tmp.join("codices").exists());
    }

    #[test]
    fn test_context_section_filters_by_file_overlap() {
        let tmp = std::env::temp_dir().join(format!("hive-codex-ctx-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let conn = Connection::open_in_memory().unwrap();
        let mut a = parse(REC).unwrap();
        a.open_threads = vec!["wire CLI".into()];
        write(&tmp, &conn, "demo", &a, "claude", "s1", "2026-11-25", 0).unwrap();
        let mut b = parse(REC).unwrap();
        b.files = vec!["ui/app.ts".into()];
        b.dead_ends = vec!["css grid broke".into()];
        write(&tmp, &conn, "demo", &b, "codex", "s2", "2026-11-24", 0).unwrap();
        let s = context_section(&tmp, "demo", "fix codex write in src/codex.rs");
        assert!(s.starts_with("## codex notes"));
        assert!(s.contains("dead end: serde_yaml") && s.contains("open thread: wire CLI"));
        assert!(!s.contains("css grid") && !s.contains("Recall@5"));
        assert_eq!(context_section(&tmp, "demo", ""), "");
        assert_eq!(context_section(&tmp, "other", "codex"), "");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_render_matches_python_mirror() {
        // sha from codex.render() in codex.py for the same record (cross-language parity)
        let (sha, _) = render("demo", &parse(REC).unwrap(), "claude", "s1", "2026-11-25");
        assert_eq!(sha, "63434b9d59599cbe13045ad781f48a3a4c85f9299c6d7affe1f110a7595c8595");
    }
}
