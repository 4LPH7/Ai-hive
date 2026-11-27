//! DETAIL-02 (v2.7.1): large MCP tool output goes to a disposable, content-addressed cache outside the vault
//! and repos; the response keeps a packed head plus a pointer. Mirror of `artifacts.py`.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

pub const OFFLOAD_TOKENS: usize = 2000;
pub const HEAD_TOKENS: usize = 400;
pub const PURGE_AGE_S: u64 = 7 * 86400;

/// HIVE_ARTIFACTS, else LOCALAPPDATA (TEMP) on Windows, else XDG_CACHE_HOME (~/.cache); then hive/artifacts.
pub fn artifacts_dir() -> PathBuf {
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    if let Some(dir) = env("HIVE_ARTIFACTS") {
        return PathBuf::from(dir);
    }
    let base = if cfg!(windows) {
        env("LOCALAPPDATA")
            .or_else(|| env("TEMP"))
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
    } else {
        env("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| env("HOME").map(|h| PathBuf::from(h).join(".cache")))
            .unwrap_or_else(std::env::temp_dir)
    };
    base.join("hive").join("artifacts")
}

/// `child` (which may not exist yet) lies under `parent`: canonicalize the nearest existing ancestor and
/// re-append the rest, so the guard can run before anything is created.
fn inside(child: &Path, parent: &Path) -> bool {
    let Ok(parent) = parent.canonicalize() else {
        return false;
    };
    let mut base = std::path::absolute(child).unwrap_or_else(|_| child.to_path_buf());
    let mut rest = Vec::new();
    while !base.exists() {
        match (base.file_name().map(|n| n.to_os_string()), base.parent()) {
            (Some(name), Some(up)) => {
                rest.push(name);
                base = up.to_path_buf();
            }
            _ => return false,
        }
    }
    let Ok(mut full) = base.canonicalize() else {
        return false;
    };
    full.extend(rest.iter().rev());
    full.starts_with(parent)
}

/// `<64 hex>.md` or `<64 hex>.md.tmp-<pid>`: only hive's own cache files are ever purged.
fn is_cache_name(name: &str) -> bool {
    let Some((hex, rest)) = name.split_at_checked(64) else {
        return false;
    };
    hex.bytes().all(|c| matches!(c, b'0'..=b'9' | b'a'..=b'f'))
        && (rest == ".md"
            || rest
                .strip_prefix(".md.tmp-")
                .is_some_and(|pid| !pid.is_empty() && pid.bytes().all(|c| c.is_ascii_digit())))
}

/// Drop cache files older than 7 days; errors ignored.
pub fn purge(folder: &Path, now: SystemTime) {
    let Ok(rd) = std::fs::read_dir(folder) else {
        return;
    };
    for e in rd.filter_map(Result::ok) {
        if !is_cache_name(&e.file_name().to_string_lossy()) {
            continue;
        }
        let old = e
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|m| now.duration_since(m).ok())
            .is_some_and(|age| age > Duration::from_secs(PURGE_AGE_S));
        if old {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

fn store(text: &str, folder: &Path, guard_roots: &[&Path]) -> Result<PathBuf, String> {
    // guard before create_dir_all: a refused cache dir must leave nothing behind in the vault
    if guard_roots.iter().any(|r| inside(folder, r)) {
        return Err(format!(
            "artifacts dir {} is inside the vault or projects root",
            folder.display()
        ));
    }
    std::fs::create_dir_all(folder).map_err(|e| e.to_string())?;
    let sha = crate::common::sha256(text.as_bytes());
    let path = folder.join(format!("{sha}.md"));
    if path.exists() {
        // keep a hot artifact alive past the purge
        std::fs::File::options()
            .write(true)
            .open(&path)
            .and_then(|f| f.set_modified(SystemTime::now()))
            .map_err(|e| e.to_string())?;
    } else {
        let tmp = folder.join(format!("{sha}.md.tmp-{}", std::process::id()));
        std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
        // content-addressed: a concurrent writer wrote the same bytes
        std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
        purge(folder, SystemTime::now());
    }
    Ok(path)
}

/// Text unchanged when <= OFFLOAD_TOKENS; else store it (secrets scrubbed) and return head + pointer.
/// Any IO or guard failure returns the full text.
pub fn offload(text: &str, guard_roots: &[&Path]) -> String {
    offload_in(text, &artifacts_dir(), guard_roots)
}

/// `offload` into an explicit cache folder (tests use a temp dir; no env mutation).
pub fn offload_in(text: &str, folder: &Path, guard_roots: &[&Path]) -> String {
    if crate::common::est_tokens(text) <= OFFLOAD_TOKENS {
        return text.to_string();
    }
    let scrubbed = crate::ace::scrub_secrets(text);
    let path = match store(&scrubbed, folder, guard_roots) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("hive: offload skipped, returning full output: {e}");
            return text.to_string();
        }
    };
    format!(
        "{}\n[full output offloaded: {} (~{} tokens); disposable, purged after 7 days]",
        excerpt(&scrubbed, HEAD_TOKENS),
        std::path::absolute(&path).unwrap_or(path).display(),
        crate::common::est_tokens(&scrubbed)
    )
}

// Signal lines survive offload wherever they sit (idea adapted from Headroom's log compressor, own code): a
// FATAL at line 900 of a 1,000-line output must not be cut because the inline excerpt keeps only the head.
const SIGNAL_RE: &str =
    r"(?i)\b(error|fatal|critical|panic(?:ked)?|traceback|exception|failed|failure)\b|\bFAIL\b";
const HEAD_SHARE: f64 = 0.55;
const TAIL_SHARE: f64 = 0.15; // the rest of the budget goes to signal lines

fn line_cost(l: &str) -> usize {
    l.chars().count() / 4 + 2
}

/// Leading lines whose est tokens (+1 per newline) fit the budget.
fn take<S: AsRef<str>>(lines: &[S], budget: usize) -> Vec<String> {
    let mut used = 0;
    let mut out = Vec::new();
    for l in lines {
        let cost = line_cost(l.as_ref());
        if used + cost > budget {
            break;
        }
        used += cost;
        out.push(l.as_ref().to_string());
    }
    out
}

/// Head + numbered signal lines + tail within ~budget est tokens; the whole text when it already fits.
/// Byte-identical to `artifacts.excerpt` for the same input.
pub fn excerpt(text: &str, budget: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    if lines.iter().map(|l| line_cost(l)).sum::<usize>() <= budget {
        return text.to_string();
    }
    let head = take(&lines, (budget as f64 * HEAD_SHARE) as usize);
    let rest_rev: Vec<&str> = lines[head.len()..].iter().rev().copied().collect();
    let mut tail = take(&rest_rev, (budget as f64 * TAIL_SHARE) as usize);
    tail.reverse();
    let middle_end = lines.len() - tail.len();
    let re = regex::Regex::new(SIGNAL_RE).unwrap();
    let signals: Vec<String> = (head.len()..middle_end)
        .filter(|&i| re.is_match(lines[i]))
        .map(|i| format!("L{}: {}", i + 1, lines[i]))
        .collect();
    let used: usize = head.iter().chain(&tail).map(|l| line_cost(l)).sum();
    let kept = take(&signals, budget.saturating_sub(used + 30));
    let mut parts = head;
    parts.push(format!(
        "...[{} lines omitted; {} of {} signal lines kept]",
        middle_end - parts.len(),
        kept.len(),
        signals.len()
    ));
    parts.extend(kept);
    if !tail.is_empty() {
        parts.push("...".into());
        parts.extend(tail);
    }
    parts.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_offload_small_large_purge_and_guard() {
        let tmp = std::env::temp_dir().join(format!("hive-artifact-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let dir = tmp.join("artifacts");
        assert_eq!(offload_in("short", &dir, &[]), "short");
        let big = format!(
            "{}token: abcdefghijklmnopqrstu\n",
            format!("{}\n", "row parse store cache retry backoff ".repeat(40)).repeat(40)
        );
        let out = offload_in(&big, &dir, &[]);
        assert!(crate::common::est_tokens(&out) < 520, "{}", out.len());
        assert!(out.contains("[full output offloaded: "));
        let stored: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.path())
            .collect();
        assert_eq!(stored.len(), 1);
        let text = std::fs::read_to_string(&stored[0]).unwrap();
        assert!(text.contains("[REDACTED_SECRET]") && !text.contains("abcdefghijklmnopqrstu"));
        let old_time = SystemTime::now() - Duration::from_secs(8 * 86400);
        let old = dir.join(format!("{}.md", "b".repeat(64)));
        let keep = dir.join("notes.md");
        for p in [&old, &keep] {
            std::fs::write(p, "old").unwrap();
            std::fs::File::options()
                .write(true)
                .open(p)
                .unwrap()
                .set_modified(old_time)
                .unwrap();
        }
        offload_in(&format!("{big}x"), &dir, &[]); // a new write purges
        assert!(!old.exists() && keep.exists());
        let inside_vault = tmp.join("vault").join("cache");
        std::fs::create_dir_all(tmp.join("vault")).unwrap();
        assert_eq!(
            offload_in(&big, &inside_vault, &[&tmp.join("vault")]),
            big,
            "a cache inside the vault is refused"
        );
        assert!(
            !inside_vault.exists(),
            "the refused cache dir must not be created"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// Signal lines survive offload wherever they sit; bytes pinned = artifacts.excerpt in Python.
    #[test]
    fn test_excerpt_keeps_buried_signal_lines_python_parity() {
        let mut lines: Vec<String> = (0..1000)
            .map(|i| {
                format!(
                    "2026-11-26T10:{:02}:{:02} INFO worker {} processed batch {i} ok",
                    i / 60,
                    i % 60,
                    i % 7
                )
            })
            .collect();
        lines[66] = "2026-11-26T10:01:06 ERROR worker 3 retry budget exhausted for batch 66".into();
        lines[899] =
            "2026-11-26T10:14:59 FATAL worker 1 panicked: index out of range in shard 899".into();
        let out = excerpt(&lines.join("\n"), 400);
        assert!(crate::common::est_tokens(&out) <= 400);
        assert!(out.contains(&format!("L67: {}", lines[66])));
        assert!(out.contains(&format!("L900: {}", lines[899])));
        assert_eq!(
            crate::common::sha256(out.as_bytes()),
            "882c9d9fec3044f42fe857c63f3361dda3ea11664cd3df8aa073a5edc3ed6397"
        );
        assert_eq!(excerpt("short\ntext", 400), "short\ntext");
    }

    #[test]
    fn test_cache_names() {
        let hex = "a".repeat(64);
        assert!(is_cache_name(&format!("{hex}.md")));
        assert!(is_cache_name(&format!("{hex}.md.tmp-42")));
        for bad in [
            format!("{hex}.txt"),
            format!("{}.md", "A".repeat(64)),
            "notes.md".to_string(),
            format!("{hex}.md.tmp-"),
        ] {
            assert!(!is_cache_name(&bad), "{bad}");
        }
    }
}
