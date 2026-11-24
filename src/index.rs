use crate::common::{default_db_path, default_vault, is_relative_to, sha256};
use regex::Regex;
use rusqlite::{params, Connection};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

pub const TYPE_PRIOR: &[(&str, f64)] = &[
    ("project-memory", 1.4),
    ("adr", 1.25),
    ("note", 1.0),
    ("journal", 0.8),
];

pub const STATUS_PRIOR: &[(&str, f64)] = &[
    ("superseded", 0.3),
    ("archived", 0.4),
];

pub const PROJECT_BOOST: f64 = 2.0;
pub const AUTOCUT_RATIO: f64 = 0.30;
pub const REFRESH_EVERY_S: f64 = 2.0;
pub const SKIP_DIRS: &[&str] = &[
    ".git", ".obsidian", ".claude", ".agents", ".trash", "Python",
    "_attachments", "graph", "node_modules", "templates", "_template", ".claude-flow",
];

pub fn type_prior(t: &str) -> f64 {
    for &(k, v) in TYPE_PRIOR {
        if k == t {
            return v;
        }
    }
    1.0
}

pub fn status_prior(s: &str) -> f64 {
    for &(k, v) in STATUS_PRIOR {
        if k == s {
            return v;
        }
    }
    1.0
}

pub fn parse_note(text: &str) -> (HashMap<String, String>, String, String) {
    let clean = if let Some(stripped) = text.strip_prefix('\u{feff}') {
        stripped
    } else {
        text
    };

    let mut meta = HashMap::new();
    let mut body = clean;

    if clean.starts_with("---") {
        if let Some(end) = clean[3..].find("\n---") {
            let front = &clean[3..3 + end];
            for line in front.lines() {
                if let Some((k, v)) = line.split_once(':') {
                    let key = k.trim();
                    if !key.is_empty() && !key.starts_with(' ') && !key.starts_with('\t') && !key.starts_with('-') {
                        let val = v.trim().trim_matches('\'').trim_matches('"');
                        meta.insert(key.to_string(), val.to_string());
                    }
                }
            }
            body = clean[3 + end + 4..].trim_start_matches(|c| c == '-' || c == '\n' || c == '\r');
        }
    }

    let re_heading = Regex::new(r"(?m)^#\s+(.+)$").unwrap();
    let title = re_heading
        .captures(body)
        .map(|cap| cap[1].trim().to_string())
        .unwrap_or_default();

    (meta, title, body.to_string())
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Hit {
    pub path: String,
    pub title: String,
    pub project: String,
    pub r#type: String,
    pub status: String,
    pub updated: String,
    pub agent: String,
    pub source: String,
    pub score: f64,
    pub snippet: String,
}

pub struct Index {
    pub vault: PathBuf,
    pub conn: Connection,
    last_refresh: Option<Instant>,
}

impl Index {
    pub fn new(vault: Option<PathBuf>, db_path: Option<PathBuf>) -> Result<Self, String> {
        let v = vault.unwrap_or_else(default_vault);
        let db = db_path.unwrap_or_else(default_db_path);
        if let Some(parent) = db.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }

        let conn = Connection::open(&db).map_err(|e| e.to_string())?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA timeout=10000;").map_err(|e| e.to_string())?;

        let user_version: i32 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap_or(0);

        if user_version != 4 {
            conn.execute_batch(
                "DROP TABLE IF EXISTS notes; DROP TABLE IF EXISTS notes_fts; PRAGMA user_version=4;",
            ).map_err(|e| e.to_string())?;
        }

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS notes(path TEXT PRIMARY KEY, mtime REAL, size INTEGER, sha256 TEXT, project TEXT,
                type TEXT, status TEXT, agent TEXT, updated TEXT, source TEXT, title TEXT);
             CREATE VIRTUAL TABLE IF NOT EXISTS notes_fts USING fts5(
                path UNINDEXED, title, meta, body, tokenize='porter unicode61');",
        ).map_err(|e| e.to_string())?;

        Ok(Self {
            vault: v,
            conn,
            last_refresh: None,
        })
    }

    pub fn in_memory() -> Result<Self, String> {
        let conn = Connection::open_in_memory().map_err(|e| e.to_string())?;
        conn.execute_batch(
            "PRAGMA user_version=4;
             CREATE TABLE IF NOT EXISTS notes(path TEXT PRIMARY KEY, mtime REAL, size INTEGER, sha256 TEXT, project TEXT,
                type TEXT, status TEXT, agent TEXT, updated TEXT, source TEXT, title TEXT);
             CREATE VIRTUAL TABLE IF NOT EXISTS notes_fts USING fts5(
                path UNINDEXED, title, meta, body, tokenize='porter unicode61');",
        ).map_err(|e| e.to_string())?;

        Ok(Self {
            vault: PathBuf::from("."),
            conn,
            last_refresh: None,
        })
    }

    pub fn walk(&self) -> Vec<PathBuf> {
        let mut out = Vec::new();
        let mut stack = vec![self.vault.clone()];

        while let Some(dir) = stack.pop() {
            if let Ok(entries) = fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    let file_name = path.file_name().and_then(|f| f.to_str()).unwrap_or("");
                    if path.is_dir() {
                        if !SKIP_DIRS.contains(&file_name) {
                            stack.push(path);
                        }
                    } else if path.extension().and_then(|ext| ext.to_str()) == Some("md") {
                        out.push(path);
                    }
                }
            }
        }
        out
    }

    pub fn refresh(&mut self, force: bool) -> Result<usize, String> {
        let now = Instant::now();
        if !force {
            if let Some(last) = self.last_refresh {
                if now.duration_since(last).as_secs_f64() < REFRESH_EVERY_S {
                    return Ok(0);
                }
            }
        }
        self.last_refresh = Some(now);

        let mut known: HashMap<String, (f64, i64, String)> = HashMap::new();
        {
            let mut stmt = self
                .conn
                .prepare("SELECT path, mtime, size, sha256 FROM notes")
                .map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, f64>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                })
                .map_err(|e| e.to_string())?;
            for r in rows.flatten() {
                known.insert(r.0, (r.1, r.2, r.3));
            }
        }

        let mut seen = HashSet::new();
        let mut changed = 0usize;
        let files = self.walk();

        let tx = self.conn.transaction().map_err(|e| e.to_string())?;

        for full_path in files {
            let rel = match full_path.strip_prefix(&self.vault) {
                Ok(r) => r.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            seen.insert(rel.clone());

            let meta_res = fs::metadata(&full_path);
            let (mtime, size) = match meta_res {
                Ok(m) => {
                    let dur = m
                        .modified()
                        .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
                        .duration_since(std::time::SystemTime::UNIX_EPOCH)
                        .unwrap_or_default();
                    (dur.as_secs_f64(), m.len() as i64)
                }
                Err(_) => continue,
            };

            if let Some((k_mtime, k_size, _)) = known.get(&rel) {
                if (k_mtime - mtime).abs() < 1e-4 && *k_size == size {
                    continue;
                }
            }

            let text = match fs::read_to_string(&full_path) {
                Ok(t) => t,
                Err(_) => continue,
            };

            let hash = sha256(text.as_bytes());

            if let Some((_, _, k_hash)) = known.get(&rel) {
                if k_hash == &hash {
                    tx.execute(
                        "UPDATE notes SET mtime = ?, size = ? WHERE path = ?",
                        params![mtime, size, &rel],
                    )
                    .map_err(|e| e.to_string())?;
                    continue;
                }
            }

            let (meta, parsed_title, body) = parse_note(&text);
            let title = if !parsed_title.is_empty() {
                parsed_title
            } else {
                Path::new(&rel)
                    .file_stem()
                    .and_then(|f| f.to_str())
                    .unwrap_or("")
                    .to_string()
            };

            let project = if let Some(p) = meta.get("project") {
                p.clone()
            } else if rel.starts_with("projects/") {
                rel.split('/').nth(1).unwrap_or("").to_string()
            } else {
                String::new()
            };

            let ntype = meta.get("type").cloned().unwrap_or_else(|| "note".to_string());
            let status = meta.get("status").cloned().unwrap_or_else(|| "active".to_string());
            let agent = meta.get("agent").cloned().unwrap_or_default();
            let updated = meta.get("updated").cloned().unwrap_or_default();
            let source = meta.get("source").cloned().unwrap_or_default();

            tx.execute("DELETE FROM notes_fts WHERE path=?", params![&rel])
                .map_err(|e| e.to_string())?;

            tx.execute(
                "INSERT OR REPLACE INTO notes VALUES(?,?,?,?,?,?,?,?,?,?,?)",
                params![&rel, mtime, size, &hash, &project, &ntype, &status, &agent, &updated, &source, &title],
            ).map_err(|e| e.to_string())?;

            let rel_words: Vec<&str> = rel.trim_end_matches(".md")
                .split(|c| c == '/' || c == '_' || c == '.' || c == '-')
                .collect();
            let meta_text = format!("{} {} {}", project, ntype, rel_words.join(" "));

            tx.execute(
                "INSERT INTO notes_fts VALUES(?,?,?,?)",
                params![&rel, &title, &meta_text, &body],
            ).map_err(|e| e.to_string())?;

            changed += 1;
        }

        for (rel, _) in known {
            if !seen.contains(&rel) {
                tx.execute("DELETE FROM notes WHERE path=?", params![&rel])
                    .map_err(|e| e.to_string())?;
                tx.execute("DELETE FROM notes_fts WHERE path=?", params![&rel])
                    .map_err(|e| e.to_string())?;
                changed += 1;
            }
        }

        tx.commit().map_err(|e| e.to_string())?;
        Ok(changed)
    }

    pub fn search(
        &mut self,
        query: &str,
        project: &str,
        limit: usize,
        scope: &str,
    ) -> Result<Vec<Hit>, String> {
        let _ = self.refresh(false);

        let re_terms = Regex::new(r"\w+").unwrap();
        let terms: Vec<&str> = re_terms.find_iter(query).map(|m| m.as_str()).collect();
        if terms.is_empty() {
            return Ok(Vec::new());
        }

        let match_expr = terms
            .iter()
            .map(|t| format!("\"{}\"", t))
            .collect::<Vec<_>>()
            .join(" OR ");

        let fetch_limit = limit * 4;
        let mut hits = Vec::new();

        if scope == "project" && !project.is_empty() {
            let mut stmt = self.conn.prepare(
                "SELECT f.path, n.title, n.project, n.type, n.status, n.updated, n.agent, n.source,
                        -bm25(notes_fts, 0.0, 5.0, 2.0, 1.0) AS s,
                        snippet(notes_fts, 3, '[', ']', '...', 18)
                 FROM notes_fts f JOIN notes n ON n.path = f.path
                 WHERE notes_fts MATCH ? AND n.project IN (?, '', 'global') ORDER BY s DESC LIMIT ?"
            ).map_err(|e| e.to_string())?;

            let rows = stmt.query_map(params![match_expr, project, fetch_limit as i64], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, f64>(8)?,
                    row.get::<_, String>(9)?,
                ))
            }).map_err(|e| e.to_string())?;

            for r in rows.flatten() {
                let (path, title, proj, ntype, status, updated, agent, source, mut score, snip) = r;
                score *= type_prior(&ntype) * status_prior(&status);
                if proj == project {
                    score *= PROJECT_BOOST;
                }
                let snippet_clean = snip.split_whitespace().collect::<Vec<_>>().join(" ");
                hits.push(Hit {
                    path,
                    title,
                    project: proj,
                    r#type: ntype,
                    status,
                    updated,
                    agent,
                    source,
                    score: (score * 1000.0).round() / 1000.0,
                    snippet: snippet_clean,
                });
            }
        } else {
            let mut stmt = self.conn.prepare(
                "SELECT f.path, n.title, n.project, n.type, n.status, n.updated, n.agent, n.source,
                        -bm25(notes_fts, 0.0, 5.0, 2.0, 1.0) AS s,
                        snippet(notes_fts, 3, '[', ']', '...', 18)
                 FROM notes_fts f JOIN notes n ON n.path = f.path
                 WHERE notes_fts MATCH ? ORDER BY s DESC LIMIT ?"
            ).map_err(|e| e.to_string())?;

            let rows = stmt.query_map(params![match_expr, fetch_limit as i64], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, f64>(8)?,
                    row.get::<_, String>(9)?,
                ))
            }).map_err(|e| e.to_string())?;

            for r in rows.flatten() {
                let (path, title, proj, ntype, status, updated, agent, source, mut score, snip) = r;
                score *= type_prior(&ntype) * status_prior(&status);
                if !project.is_empty() && proj == project {
                    score *= PROJECT_BOOST;
                }
                let snippet_clean = snip.split_whitespace().collect::<Vec<_>>().join(" ");
                hits.push(Hit {
                    path,
                    title,
                    project: proj,
                    r#type: ntype,
                    status,
                    updated,
                    agent,
                    source,
                    score: (score * 1000.0).round() / 1000.0,
                    snippet: snippet_clean,
                });
            }
        }

        hits.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

        if !hits.is_empty() {
            let floor = AUTOCUT_RATIO * hits[0].score;
            hits.retain(|h| h.score >= floor);
        }

        hits.truncate(limit);
        Ok(hits)
    }

    pub fn note(&self, rel: &str) -> String {
        let full = self.vault.join(rel);
        if let Ok(resolved) = full.canonicalize() {
            if let Ok(v_resolved) = self.vault.canonicalize() {
                if !is_relative_to(&resolved, &v_resolved) {
                    return String::new();
                }
            }
        }
        fs::read_to_string(&full).unwrap_or_default()
    }

    pub fn by_project(&mut self, project: &str, ntype: &str) -> Result<Vec<(String, String, String, String)>, String> {
        let _ = self.refresh(false);
        let mut stmt = self.conn.prepare(
            "SELECT path, title, status, updated FROM notes WHERE project=? AND type=? ORDER BY path"
        ).map_err(|e| e.to_string())?;

        let rows = stmt.query_map(params![project, ntype], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        }).map_err(|e| e.to_string())?;

        let mut out = Vec::new();
        for r in rows.flatten() {
            out.push(r);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fts5_in_memory() {
        let temp_dir = std::env::temp_dir().join(format!("hive_test_{}", std::process::id()));
        let _ = fs::create_dir_all(&temp_dir);
        let mut idx = Index::new(Some(temp_dir.clone()), Some(temp_dir.join("test.sqlite"))).unwrap();

        let test_file = temp_dir.join("test.md");
        fs::write(&test_file, "---\ntype: note\nproject: proj\nstatus: active\nupdated: 2026-11-24\n---\n# Test Title\nContent about rust migration and fts5 search").unwrap();
        idx.refresh(true).unwrap();

        let hits = idx.search("migration", "proj", 5, "project").unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].path, "test.md");
        assert_eq!(hits[0].title, "Test Title");
        let _ = fs::remove_dir_all(&temp_dir);
    }
}
