use crate::ace::is_secret;
use crate::index::{parse_note, SKIP_DIRS};
use chrono::NaiveDate;
use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::process::Command;

pub const ALLOWED_TYPE: &[&str] = &["project-memory", "adr", "journal", "note", "codex"];
pub const ALLOWED_STATUS: &[&str] = &["active", "superseded", "archived"];
pub const ALLOWED_AGENT: &[&str] = &["claude", "codex", "antigravity", "user"];
pub const REQUIRED_FIELDS: &[&str] = &["type", "project", "agent", "status", "updated"];
pub const EXEMPT_FRONTMATTER: &[&str] = &["AGENTS.md", "CLAUDE.md", "GEMINI.md"];
pub const MEMORY_CAP: usize = 150;
pub const STALE_DAYS: i64 = 30;
pub const CONVENTIONAL_NAMES: &[&str] = &["_memory", "playbook"];
pub const SOURCE_RULE_FROM: &str = "2026-11-23";
pub const SOURCED_PREFIXES: &[&str] = &["projects/", "inbox/"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub severity: String,
    pub code: String,
    pub path: String,
    pub message: String,
}

pub fn git_staged_files(vault: &Path) -> Vec<String> {
    let output = Command::new("git")
        .args(["diff", "--name-only", "--cached"])
        .current_dir(vault)
        .output();

    if let Ok(out) = output {
        if out.status.success() {
            let text = String::from_utf8_lossy(&out.stdout);
            return text
                .lines()
                .map(|line| line.trim().replace('\\', "/"))
                .filter(|line| !line.is_empty())
                .collect();
        }
    }
    Vec::new()
}

pub fn walk_vault(vault: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![vault.to_path_buf()];

    while let Some(dir) = stack.pop() {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                let name = p.file_name().and_then(|f| f.to_str()).unwrap_or("");
                if p.is_dir() {
                    if !SKIP_DIRS.contains(&name) {
                        stack.push(p);
                    }
                } else if p.is_file() {
                    if let Ok(rel) = p.strip_prefix(vault) {
                        out.push(rel.to_string_lossy().replace('\\', "/"));
                    }
                }
            }
        }
    }
    out
}

pub struct Resolver {
    pub by_name: HashMap<String, Vec<String>>,
    pub by_path: HashMap<String, String>,
}

impl Resolver {
    pub fn new(files: &[String]) -> Self {
        let mut by_name: HashMap<String, Vec<String>> = HashMap::new();
        let mut by_path: HashMap<String, String> = HashMap::new();

        for rel in files {
            let name = rel.rsplit('/').next().unwrap_or(rel).to_lowercase();
            by_name.entry(name.clone()).or_default().push(rel.clone());
            by_path.insert(rel.to_lowercase(), rel.clone());

            if name.ends_with(".md") {
                let stem = &name[..name.len() - 3];
                by_name
                    .entry(stem.to_string())
                    .or_default()
                    .push(rel.clone());
                by_path.insert(rel[..rel.len() - 3].to_lowercase(), rel.clone());
            }
        }

        Self { by_name, by_path }
    }

    pub fn resolve(&self, target: &str) -> Vec<String> {
        let key = target.to_lowercase().trim_start_matches('/').to_string();
        if key.contains('/') {
            if let Some(hit) = self.by_path.get(&key) {
                return vec![hit.clone()];
            }
            Vec::new()
        } else {
            self.by_name.get(&key).cloned().unwrap_or_default()
        }
    }
}

pub fn links_in(body: &str) -> Vec<String> {
    let mut clean_lines = Vec::new();
    let mut in_fence = false;
    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if !in_fence {
            clean_lines.push(line);
        }
    }
    let text1 = clean_lines.join("\n");
    let re_inline = Regex::new(r"`[^`\n]*`").unwrap();
    let text = re_inline.replace_all(&text1, "");

    let re_link = Regex::new(r"!?\[\[([^\]]+)\]\]").unwrap();
    let mut targets = Vec::new();

    for cap in re_link.captures_iter(&text) {
        let raw = &cap[1];
        let unescaped = raw.replace(r"\|", "|");
        let pipe_part = unescaped.split('|').next().unwrap_or("");
        let hash_part = pipe_part.split('#').next().unwrap_or("");
        let clean = hash_part.trim();
        if !clean.is_empty() {
            targets.push(clean.to_string());
        }
    }

    targets
}

pub fn check_adr_staleness(rel: &str, text: &str, projects_root: &Path) -> Vec<Finding> {
    if !rel.starts_with("projects/") || !rel.contains("/adrs/") {
        return Vec::new();
    }
    let parts: Vec<&str> = rel.split('/').collect();
    if parts.len() < 3 {
        return Vec::new();
    }
    let slug = parts[1];
    let proj_dir = projects_root.join(slug);
    let graphify_out = proj_dir.join("graphify-out");
    let manifest_p = graphify_out.join("manifest.json");
    let graph_p = graphify_out.join("graph.json");

    let mut known_symbols = HashSet::new();
    let mut deleted_symbols = HashSet::new();

    if manifest_p.is_file() {
        if let Ok(content) = fs::read_to_string(&manifest_p) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
                if let Some(arr) = v.get("symbols").and_then(|s| s.as_array()) {
                    for sym in arr {
                        if let Some(s) = sym.as_str() {
                            known_symbols.insert(s.to_string());
                        }
                    }
                }
                if let Some(arr) = v.get("deleted_symbols").and_then(|s| s.as_array()) {
                    for sym in arr {
                        if let Some(s) = sym.as_str() {
                            deleted_symbols.insert(s.to_string());
                        }
                    }
                }
            }
        }
    } else if graph_p.is_file() {
        if let Ok(content) = fs::read_to_string(&graph_p) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
                if let Some(nodes) = v.get("nodes").and_then(|n| n.as_array()) {
                    for node in nodes {
                        if let Some(name) = node.get("name").and_then(|n| n.as_str()) {
                            known_symbols.insert(name.to_string());
                        }
                        if let Some(label) = node.get("label").and_then(|n| n.as_str()) {
                            known_symbols.insert(label.to_string());
                        }
                    }
                }
            }
        }
    }

    if known_symbols.is_empty() && deleted_symbols.is_empty() {
        return Vec::new();
    }

    let sym_re = Regex::new(r"`([A-Za-z_][A-Za-z0-9_:]*)`").unwrap();
    let mut findings = Vec::new();
    let stopwords: HashSet<&str> = [
        "true", "false", "null", "none", "str", "int", "dict", "bool", "json", "path", "self",
        "file",
    ]
    .into_iter()
    .collect();
    let mut checked = HashSet::new();

    for cap in sym_re.captures_iter(text) {
        if let Some(m) = cap.get(1) {
            let sym = m.as_str();
            if stopwords.contains(&sym.to_lowercase().as_str())
                || sym.len() < 3
                || checked.contains(sym)
            {
                continue;
            }
            checked.insert(sym.to_string());
            if deleted_symbols.contains(sym) {
                findings.push(Finding {
                    severity: "WARN".to_string(),
                    code: "adr-stale".to_string(),
                    path: rel.to_string(),
                    message: format!("ADR references deleted symbol `{}`", sym),
                });
            } else if !known_symbols.is_empty()
                && !known_symbols.contains(sym)
                && (sym.contains("::") || sym.chars().skip(1).any(|c| c.is_uppercase()))
            {
                findings.push(Finding {
                    severity: "WARN".to_string(),
                    code: "adr-stale".to_string(),
                    path: rel.to_string(),
                    message: format!(
                        "ADR references symbol `{}` not found in graphify manifest",
                        sym
                    ),
                });
            }
        }
    }

    findings
}

pub fn lint(vault: &Path, files_filter: Option<&HashSet<String>>) -> Vec<Finding> {
    let cfg = crate::common::load_config(Some(vault.to_path_buf()), None, None, None);
    let projects_root = cfg.projects;
    let all_files = walk_vault(vault);
    let resolver = Resolver::new(&all_files);

    let mut findings = Vec::new();
    let quarantined =
        crate::governance::quarantined_paths(vault, &crate::governance::load_rules(vault));
    let re_journal_header = Regex::new(r"^## \d{2}:\d{2} [a-z]+ - \S.*?( \| [a-z-]+)?$").unwrap();
    let re_what_it_is = Regex::new(r"(?m)^## What it is\b").unwrap();

    let mut active_project_notes: HashMap<String, String> = HashMap::new();
    let mut adr_statuses: HashMap<String, (String, String, Vec<String>)> = HashMap::new();
    let mut stem_counts: HashMap<String, Vec<String>> = HashMap::new();

    for rel in &all_files {
        if !rel.ends_with(".md") {
            continue;
        }

        let full_path = vault.join(rel);
        let content = match fs::read_to_string(&full_path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let has_frontmatter = content.starts_with("---") || content.starts_with("\u{feff}---");
        let (meta, _title, body) = parse_note(&content);

        let stem = Path::new(rel)
            .file_stem()
            .and_then(|f| f.to_str())
            .unwrap_or("")
            .to_string();
        if !CONVENTIONAL_NAMES.contains(&stem.as_str()) && !rel.starts_with("journal/") {
            stem_counts
                .entry(stem.to_lowercase())
                .or_default()
                .push(rel.clone());
        }

        // Secret scan
        for (idx, line) in content.lines().enumerate() {
            if is_secret(line) {
                findings.push(Finding {
                    severity: "ERROR".to_string(),
                    code: "secret-detected".to_string(),
                    path: rel.clone(),
                    message: format!("line {}: suspected secret or credential", idx + 1),
                });
            }
        }

        // CODEX-04: codices are immutable; a body hash mismatch means someone edited one
        if meta.get("type").map(|t| t.as_str()) == Some("codex") {
            if let Some(problem) = crate::codex::verify(&content) {
                findings.push(Finding {
                    severity: "ERROR".to_string(),
                    code: "codex-tampered".to_string(),
                    path: rel.clone(),
                    message: problem,
                });
            }
        }

        // Frontmatter checks
        if !EXEMPT_FRONTMATTER.contains(&rel.as_str()) {
            if !has_frontmatter {
                findings.push(Finding {
                    severity: "ERROR".to_string(),
                    code: "frontmatter-missing".to_string(),
                    path: rel.clone(),
                    message: "no frontmatter block".to_string(),
                });
            } else {
                for &req in REQUIRED_FIELDS {
                    if !meta.contains_key(req) || meta[req].is_empty() {
                        findings.push(Finding {
                            severity: "ERROR".to_string(),
                            code: "frontmatter-missing".to_string(),
                            path: rel.clone(),
                            message: format!("missing key {:?}", req),
                        });
                    }
                }
                if let Some(t) = meta.get("type") {
                    if !ALLOWED_TYPE.contains(&t.as_str()) {
                        findings.push(Finding {
                            severity: "ERROR".to_string(),
                            code: "bad-value".to_string(),
                            path: rel.clone(),
                            message: format!("type: {:?} not in {:?}", t, ALLOWED_TYPE),
                        });
                    }
                }
                if let Some(s) = meta.get("status") {
                    if !ALLOWED_STATUS.contains(&s.as_str()) {
                        findings.push(Finding {
                            severity: "ERROR".to_string(),
                            code: "bad-value".to_string(),
                            path: rel.clone(),
                            message: format!("status: {:?} not in {:?}", s, ALLOWED_STATUS),
                        });
                    }
                }
                if let Some(a) = meta.get("agent") {
                    if !ALLOWED_AGENT.contains(&a.as_str()) {
                        findings.push(Finding {
                            severity: "ERROR".to_string(),
                            code: "bad-value".to_string(),
                            path: rel.clone(),
                            message: format!("agent: {:?} not in {:?}", a, ALLOWED_AGENT),
                        });
                    }
                }
            }
        }

        // Sourced claim checks
        if let Some(updated) = meta.get("updated") {
            if updated.as_str() >= SOURCE_RULE_FROM
                && meta.get("agent").map(|a| a != "user").unwrap_or(false)
                && SOURCED_PREFIXES.iter().any(|p| rel.starts_with(p))
                && !rel.ends_with("/playbook.md")
                && (!meta.contains_key("source") || meta["source"].is_empty())
            {
                findings.push(Finding {
                    severity: "ERROR".to_string(),
                    code: "source-missing".to_string(),
                    path: rel.clone(),
                    message: "source: required on agent notes created since 2026-11-23".to_string(),
                });
            }
        }

        // Line caps
        if rel == "_meta/active-work.md" {
            let line_count = content.lines().count();
            if line_count > 40 {
                findings.push(Finding {
                    severity: "ERROR".to_string(),
                    code: "line-cap".to_string(),
                    path: rel.clone(),
                    message: format!("{} lines exceeds 40-line cap", line_count),
                });
            }
        } else if rel.ends_with("/_memory.md") {
            let line_count = content.lines().count();
            if line_count > MEMORY_CAP {
                findings.push(Finding {
                    severity: "ERROR".to_string(),
                    code: "line-cap".to_string(),
                    path: rel.clone(),
                    message: format!("{} lines exceeds {}-line cap", line_count, MEMORY_CAP),
                });
            }
            if !re_what_it_is.is_match(&body) {
                findings.push(Finding {
                    severity: "ERROR".to_string(),
                    code: "memory-missing-what-it-is".to_string(),
                    path: rel.clone(),
                    message: "missing '## What it is' section".to_string(),
                });
            }
            if meta.get("status").map(|s| s.as_str()) == Some("active") {
                if let Some(up) = meta.get("updated") {
                    active_project_notes.insert(rel.clone(), up.clone());
                }
            }
        }

        // Journal headers
        if rel.starts_with("journal/") {
            for (idx, line) in body.lines().enumerate() {
                if line.starts_with("## ") && !re_journal_header.is_match(line) {
                    findings.push(Finding {
                        severity: "WARN".to_string(),
                        code: "journal-header".to_string(),
                        path: rel.clone(),
                        message: format!(
                            "line {}: header {:?} does not match '## HH:MM agent - Task'",
                            idx + 1,
                            line
                        ),
                    });
                }
            }
        }

        // Links & ADR tracking
        let links = links_in(&body);
        if meta.get("type").map(|t| t.as_str()) == Some("adr") {
            findings.extend(check_adr_staleness(rel, &content, &projects_root));
            adr_statuses.insert(
                rel.clone(),
                (
                    meta.get("status").cloned().unwrap_or_default(),
                    rel.clone(),
                    links.clone(),
                ),
            );
        }

        for link in links {
            let hits = resolver.resolve(&link);
            if hits.is_empty() {
                findings.push(Finding {
                    severity: "WARN".to_string(),
                    code: "broken-link".to_string(),
                    path: rel.clone(),
                    message: format!("unresolvable link [[{}]]", link),
                });
            }
            // GOV-02: warn on links into a quarantine radius
            for hit in hits.iter().filter(|h| quarantined.contains(h.as_str())) {
                findings.push(Finding {
                    severity: "WARN".to_string(),
                    code: "quarantined-link".to_string(),
                    path: rel.clone(),
                    message: format!("[[{}]] points at quarantined {}", link, hit),
                });
            }
        }
    }

    // Duplicate basenames
    for (stem, paths) in stem_counts {
        if paths.len() > 1 {
            for p in &paths {
                findings.push(Finding {
                    severity: "WARN".to_string(),
                    code: "duplicate-basename".to_string(),
                    path: p.clone(),
                    message: format!("basename {:?} also in {:?}", stem, paths),
                });
            }
        }
    }

    // Stale active memory
    let today = chrono::Local::now().date_naive();
    for (rel, updated) in active_project_notes {
        if let Ok(d) = NaiveDate::parse_from_str(&updated, "%Y-%m-%d") {
            let days = (today - d).num_days();
            if days > STALE_DAYS {
                findings.push(Finding {
                    severity: "WARN".to_string(),
                    code: "stale-memory".to_string(),
                    path: rel,
                    message: format!(
                        "updated {} days ago ({}); active projects should be refreshed",
                        days, updated
                    ),
                });
            }
        }
    }

    // Superseded ADR check
    for (path, (status, _, links)) in &adr_statuses {
        if status == "superseded" {
            let has_adr_link = links.iter().any(|target| {
                resolver
                    .resolve(target)
                    .iter()
                    .any(|p| p.contains("/adrs/"))
            });
            if !has_adr_link {
                findings.push(Finding {
                    severity: "ERROR".to_string(),
                    code: "superseded-adr-missing-link".to_string(),
                    path: path.clone(),
                    message: "superseded ADR must link to its successor ADR".to_string(),
                });
            }
        }
    }

    // Filter to staged files if requested
    if let Some(filter) = files_filter {
        findings.retain(|f| filter.contains(&f.path));
    }

    // Sort findings: ERROR (0), WARN (1), INFO (2)
    findings.sort_by(|a, b| {
        let sev_order = |s: &str| match s {
            "ERROR" => 0,
            "WARN" => 1,
            _ => 2,
        };
        sev_order(&a.severity)
            .cmp(&sev_order(&b.severity))
            .then_with(|| a.path.cmp(&b.path))
            .then_with(|| a.code.cmp(&b.code))
    });

    findings
}

pub fn format_report(findings: &[Finding]) -> String {
    let mut counts = [0usize; 3]; // ERROR (0), WARN (1), INFO (2)
    for f in findings {
        match f.severity.as_str() {
            "ERROR" => counts[0] += 1,
            "WARN" => counts[1] += 1,
            _ => counts[2] += 1,
        }
    }
    let head = format!(
        "lint: {} error, {} warn, {} info",
        counts[0], counts[1], counts[2]
    );
    if findings.is_empty() {
        return head;
    }
    let mut lines = vec![head];
    for f in findings {
        lines.push(format!(
            "{:5} {:20} {}: {}",
            f.severity, f.code, f.path, f.message
        ));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_secret_detection_in_lint() {
        let temp_dir = std::env::temp_dir().join(format!("hive_lint_test_{}", std::process::id()));
        let _ = fs::create_dir_all(&temp_dir);

        let file = temp_dir.join("secret_note.md");
        fs::write(&file, "---\ntype: note\nproject: proj\nstatus: active\nupdated: 2026-11-24\n---\napi_key = sk-ant-api03-123456789012345678901234").unwrap();

        let findings = lint(&temp_dir, None);
        assert!(findings
            .iter()
            .any(|f| f.code == "secret-detected" && f.severity == "ERROR"));

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
