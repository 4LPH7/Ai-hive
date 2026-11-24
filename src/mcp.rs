use crate::ace::{format_bullets, load, select_bullets};
use crate::common::{
    active_work_cell_matches, default_db_path, default_projects, note_age, slugify, validate_slug,
};
use crate::graph::{fmt_node, repo_dir, Graph};
use crate::index::{parse_note, Index};
use crate::lint::{format_report, lint};
use crate::packer::{default_section_weight, pack, Section};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

pub const SERVER_NAME: &str = "hive";
pub const SERVER_VERSION: &str = "1.2.0";
pub const MODERN_VERSIONS: &[&str] = &["2026-07-28"];
pub const LEGACY_VERSIONS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];
pub const META_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
pub const INSTRUCTIONS: &str =
    "Shared memory for all coding agents (Obsidian vault + graphify graphs). Read-only. \
    Call hive_context at the start of a task, hive_search before asking the user for context, \
    hive_graph for code structure. Write memory as Markdown files following the vault's AGENTS.md.";

pub fn tool_list() -> Value {
    let read_only = json!({
        "readOnlyHint": true,
        "destructiveHint": false,
        "idempotentHint": true,
        "openWorldHint": false
    });

    let tools = vec![
        json!({
            "name": "hive_context",
            "description": "Call first on any task. Token-budgeted memory pack for a project: its active-work row, _memory.md, ADR list and, if a task is given, matching vault notes and ranked code-graph symbols.",
            "inputSchema": {
                "type": "object",
                "required": ["project"],
                "properties": {
                    "project": {"type": "string", "description": "Project slug: folder name under projects root, lowercased, spaces to hyphens"},
                    "task": {"type": "string", "description": "What you are about to do (optional, improves ranking)"},
                    "budget": {"type": "integer", "description": "Max tokens to return (default 1500)"}
                },
                "additionalProperties": false
            },
            "annotations": read_only,
            "_meta": {"anthropic/alwaysLoad": true}
        }),
        json!({
            "name": "hive_search",
            "description": "BM25 search over vault notes. Default scope is project-isolated ('project'); use scope='all' for cross-project.",
            "inputSchema": {
                "type": "object",
                "required": ["query"],
                "properties": {
                    "query": {"type": "string", "description": "Keywords"},
                    "project": {"type": "string", "description": "Project slug to filter/boost (optional)"},
                    "scope": {"type": "string", "description": "Search scope: 'project' (default, project + global) or 'all' (cross-project)"},
                    "limit": {"type": "integer", "description": "Max hits (default 8)"}
                },
                "additionalProperties": false
            },
            "annotations": read_only
        }),
        json!({
            "name": "hive_graph",
            "description": "Ranked code-graph neighbourhood from graphify (Personalized PageRank with hub dampening): symbols with file:line, edges between them, related vault notes, and graph staleness.",
            "inputSchema": {
                "type": "object",
                "required": ["repo", "query"],
                "properties": {
                    "repo": {"type": "string", "description": "Repo folder name or slug under projects root, or a path"},
                    "query": {"type": "string", "description": "Symbol names or keywords"},
                    "limit": {"type": "integer", "description": "Max symbols (default 15)"}
                },
                "additionalProperties": false
            },
            "annotations": read_only
        }),
        json!({
            "name": "hive_status",
            "description": "Index size per project and freshness of each repo's code graph.",
            "inputSchema": {
                "type": "object",
                "required": [],
                "properties": {},
                "additionalProperties": false
            },
            "annotations": read_only
        }),
        json!({
            "name": "hive_lint",
            "description": "Vault health against AGENTS.md rules: frontmatter, line caps, broken links, stale memory, superseded ADRs without successor, ambiguous names, orphans. Run before session end after writing notes.",
            "inputSchema": {
                "type": "object",
                "required": [],
                "properties": {},
                "additionalProperties": false
            },
            "annotations": read_only
        }),
        json!({
            "name": "hive_skill",
            "description": "Auto-select best skill for task, or discover and install from remote registry (find-skills).",
            "inputSchema": {
                "type": "object",
                "required": ["task"],
                "properties": {
                    "task": {"type": "string", "description": "Task description or user prompt (required)"},
                    "auto_install": {"type": "boolean", "description": "Whether to auto-install missing skills from registry (default: true)"},
                    "scope": {"type": "string", "description": "Search scope: 'local' (installed only) or 'all' (local + remote fallback, default: 'all')"}
                },
                "additionalProperties": false
            },
            "annotations": read_only
        }),
    ];

    json!(tools)
}

pub fn validate_tool_args(name: &str, args: &Value) -> Result<(), String> {
    let obj = args.as_object().ok_or_else(|| {
        format!(
            "arguments must be a JSON object, got {}",
            if args.is_null() { "null" } else { "non-object" }
        )
    })?;

    match name {
        "hive_context" => {
            if !obj.contains_key("project") || obj["project"].is_null() {
                return Err("missing required argument 'project'".to_string());
            }
            for (k, v) in obj {
                match k.as_str() {
                    "project" => {
                        if !v.is_string() {
                            return Err("argument 'project' must be string".to_string());
                        }
                    }
                    "task" => {
                        if !v.is_string() {
                            return Err("argument 'task' must be string".to_string());
                        }
                    }
                    "budget" => {
                        if !v.is_i64() && !v.is_u64() {
                            return Err("argument 'budget' must be integer".to_string());
                        }
                    }
                    other => {
                        return Err(format!(
                            "unexpected argument {:?}; allowed: [\"budget\", \"project\", \"task\"]",
                            other
                        ));
                    }
                }
            }
        }
        "hive_search" => {
            if !obj.contains_key("query") || obj["query"].is_null() {
                return Err("missing required argument 'query'".to_string());
            }
            for (k, v) in obj {
                match k.as_str() {
                    "query" => {
                        if !v.is_string() {
                            return Err("argument 'query' must be string".to_string());
                        }
                    }
                    "project" => {
                        if !v.is_string() {
                            return Err("argument 'project' must be string".to_string());
                        }
                    }
                    "scope" => {
                        if !v.is_string() {
                            return Err("argument 'scope' must be string".to_string());
                        }
                    }
                    "limit" => {
                        if !v.is_i64() && !v.is_u64() {
                            return Err("argument 'limit' must be integer".to_string());
                        }
                    }
                    other => {
                        return Err(format!(
                            "unexpected argument {:?}; allowed: [\"limit\", \"project\", \"query\", \"scope\"]",
                            other
                        ));
                    }
                }
            }
        }
        "hive_graph" => {
            if !obj.contains_key("repo") || obj["repo"].is_null() {
                return Err("missing required argument 'repo'".to_string());
            }
            if !obj.contains_key("query") || obj["query"].is_null() {
                return Err("missing required argument 'query'".to_string());
            }
            for (k, v) in obj {
                match k.as_str() {
                    "repo" => {
                        if !v.is_string() {
                            return Err("argument 'repo' must be string".to_string());
                        }
                    }
                    "query" => {
                        if !v.is_string() {
                            return Err("argument 'query' must be string".to_string());
                        }
                    }
                    "limit" => {
                        if !v.is_i64() && !v.is_u64() {
                            return Err("argument 'limit' must be integer".to_string());
                        }
                    }
                    other => {
                        return Err(format!(
                            "unexpected argument {:?}; allowed: [\"limit\", \"query\", \"repo\"]",
                            other
                        ));
                    }
                }
            }
        }
        "hive_status" | "hive_lint" => {
            if !obj.is_empty() {
                let first_key = obj.keys().next().unwrap();
                return Err(format!("unexpected argument {:?}; allowed: []", first_key));
            }
        }
        "hive_skill" => {
            if !obj.contains_key("task") || obj["task"].is_null() {
                return Err("missing required argument 'task'".to_string());
            }
            for (k, v) in obj {
                match k.as_str() {
                    "task" => {
                        if !v.is_string() {
                            return Err("argument 'task' must be string".to_string());
                        }
                    }
                    "auto_install" => {
                        if !v.is_boolean() {
                            return Err("argument 'auto_install' must be boolean".to_string());
                        }
                    }
                    "scope" => {
                        if !v.is_string() {
                            return Err("argument 'scope' must be string".to_string());
                        }
                    }
                    other => {
                        return Err(format!(
                            "unexpected argument {:?}; allowed: [\"auto_install\", \"scope\", \"task\"]",
                            other
                        ));
                    }
                }
            }
        }
        other => return Err(format!("unknown tool {:?}", other)),
    }

    Ok(())
}

pub fn graph_freshness(dir: &Path, g: &Graph) -> String {
    let manifest_path = dir.join("graphify-out").join("manifest.json");
    if let Ok(manifest_text) = fs::read_to_string(&manifest_path) {
        if let Ok(val) = serde_json::from_str::<HashMap<String, Value>>(&manifest_text) {
            let mut changed = Vec::new();
            for (rel, info) in val {
                let target_file = dir.join(&rel);
                if let Ok(meta) = fs::metadata(&target_file) {
                    let dur = meta
                        .modified()
                        .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
                        .duration_since(std::time::SystemTime::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs_f64();
                    let recorded_mtime = info.get("mtime").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    if (dur - recorded_mtime).abs() > 1e-3 {
                        changed.push(rel);
                    }
                } else {
                    changed.push(rel);
                }
            }
            if changed.is_empty() {
                let commit_short = if g.commit.len() >= 8 {
                    &g.commit[..8]
                } else {
                    &g.commit
                };
                return format!(
                    "graph current (no tracked file changed since build at {})",
                    if commit_short.is_empty() {
                        "unknown commit"
                    } else {
                        commit_short
                    }
                );
            } else {
                let count = changed.len();
                let names = changed
                    .iter()
                    .take(5)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ");
                let more = if count > 5 {
                    format!(" +{} more", count - 5)
                } else {
                    String::new()
                };
                return format!(
                    "graph STALE for {} file(s): {}{}; run `graphify update .` in {}",
                    count,
                    names,
                    more,
                    dir.display()
                );
            }
        }
    }
    let commit_short = if g.commit.len() >= 8 {
        &g.commit[..8]
    } else {
        &g.commit
    };
    format!("graph built at {}", commit_short)
}

pub fn tool_graph(
    idx: &mut Index,
    repo: &str,
    query: &str,
    limit: usize,
) -> Result<String, String> {
    let d = repo_dir(repo).ok_or_else(|| {
        format!(
            "no repo folder matching {:?} under {}",
            repo,
            default_projects().display()
        )
    })?;
    let graph_path = d.join("graphify-out").join("graph.json");
    if !graph_path.is_file() {
        return Err(format!(
            "{} has no graphify-out/graph.json; run `graphify update .` there",
            d.file_name().and_then(|f| f.to_str()).unwrap_or("repo")
        ));
    }
    let graph_str = fs::read_to_string(&graph_path).map_err(|e| e.to_string())?;
    let g = Graph::from_json_str(&graph_str)?;

    let (ranked, edges) = g.query(query, limit);
    let freshness = graph_freshness(&d, &g);
    let repo_name = d.file_name().and_then(|f| f.to_str()).unwrap_or("repo");

    if ranked.is_empty() {
        return Ok(format!(
            "no symbols in {} match {:?}. {}",
            repo_name, query, freshness
        ));
    }

    let mut lines = vec![
        format!(
            "{}: {} nodes, {}. * = matched seed",
            repo_name,
            g.nodes.len(),
            freshness
        ),
        String::new(),
    ];

    for (nid, _, is_seed) in &ranked {
        lines.push(fmt_node(&g.nodes[nid], *is_seed));
    }

    if !edges.is_empty() {
        lines.push(String::new());
        lines.push("edges among these:".to_string());
        for (a, rel, b) in edges.iter().take(25) {
            let label_a = g.nodes[a].label.as_deref().unwrap_or(a);
            let label_b = g.nodes[b].label.as_deref().unwrap_or(b);
            lines.push(format!("  {} -{}-> {}", label_a, rel, label_b));
        }
    }

    let mut stems = HashSet::new();
    for (nid, _, _) in ranked.iter().take(6) {
        if let Some(src) = &g.nodes[nid].source_file {
            if let Some(stem) = Path::new(src).file_stem().and_then(|f| f.to_str()) {
                if !stem.is_empty() {
                    stems.insert(stem.to_string());
                }
            }
        }
    }

    if !stems.is_empty() {
        let stem_query = stems.into_iter().collect::<Vec<_>>().join(" ");
        let slug = slugify(repo_name);
        if let Ok(hits) = idx.search(&stem_query, &slug, 8, "project") {
            let notes: Vec<_> = hits
                .into_iter()
                .filter(|h| h.project == slug)
                .take(4)
                .collect();
            if !notes.is_empty() {
                lines.push(String::new());
                lines.push("vault notes about these files:".to_string());
                for h in notes {
                    lines.push(format!("  {} - {}", h.path, h.title));
                }
            }
        }
    }

    Ok(lines.join("\n"))
}

pub fn tool_context(
    idx: &mut Index,
    project: &str,
    task: &str,
    budget: usize,
) -> Result<String, String> {
    let slug = validate_slug(project)?;

    // 1. Memory Section
    let mut mem_parts = Vec::new();
    let active_work_text = idx.note("_meta/active-work.md");
    let mut active_row = String::new();
    for line in active_work_text.lines() {
        if line.starts_with('|') {
            let cells: Vec<&str> = line.split('|').map(|c| c.trim()).collect();
            if cells.len() > 1 && active_work_cell_matches(cells[1], &slug) {
                active_row = line.to_string();
                break;
            }
        }
    }
    if !active_row.is_empty() {
        mem_parts.push(format!("active-work: {}", active_row));
    }

    let mem_rel = format!("projects/{}/_memory.md", slug);
    let memory_text = idx.note(&mem_rel);
    if !memory_text.is_empty() {
        let (meta, _, body) = parse_note(&memory_text);
        let agent = meta.get("agent").map(|s| s.as_str()).unwrap_or("unknown");
        let source = meta.get("source").map(|s| s.as_str()).unwrap_or("unknown");
        let updated = meta.get("updated").map(|s| s.as_str()).unwrap_or("");
        let age = note_age(updated);
        let header = format!(
            "> [!NOTE] Untrusted vault note: path=\"{}\", agent=\"{}\", source=\"{}\", age=\"{}\"",
            mem_rel, agent, source, age
        );
        let quoted_body: Vec<String> = body.trim().lines().map(|l| format!("> {}", l)).collect();
        let framed_body = format!("{}\n{}", header, quoted_body.join("\n"));
        mem_parts.push(format!("## {} ({})\n{}", mem_rel, age, framed_body));
    } else {
        mem_parts.push(format!(
            "## {}\n(none yet: create it from templates/project-memory.md)",
            mem_rel
        ));
    }

    if let Ok(adrs) = idx.by_project(&slug, "adr") {
        if !adrs.is_empty() {
            let adr_lines: Vec<String> = adrs
                .into_iter()
                .map(|(p, t, s, _)| format!("- {} [{}] {}", p, s, t))
                .collect();
            mem_parts.push(format!("## ADRs\n{}", adr_lines.join("\n")));
        }
    }
    let memory_section = mem_parts.join("\n\n");

    // 2. Playbook Section
    let mut playbook_section = String::new();
    let mut combined_bullets = Vec::new();
    if let Ok(glob_sec) = load(&idx.vault, "global") {
        combined_bullets.extend(select_bullets(&glob_sec, task, 8));
    }
    if let Ok(proj_sec) = load(&idx.vault, &slug) {
        combined_bullets.extend(select_bullets(&proj_sec, task, 8));
    }
    if !combined_bullets.is_empty() {
        playbook_section = format!(
            "## Playbook (ACE; note ids you use, run skill hive-ace after substantial work)\n{}",
            format_bullets(&combined_bullets)
        );
    }

    // 3. Task notes & code graph
    let mut task_notes_section = String::new();
    let mut code_graph_section = String::new();

    if !task.is_empty() {
        if let Ok(hits) = idx.search(task, &slug, 6, "project") {
            let filtered: Vec<_> = hits
                .into_iter()
                .filter(|h| {
                    !h.path.ends_with("/_memory.md")
                        && (h.project == slug || h.project.is_empty() || h.project == "global")
                })
                .collect();
            if !filtered.is_empty() {
                let note_lines: Vec<String> = filtered
                    .into_iter()
                    .map(|h| {
                        format!(
                            "- {} ({}, age {}): {}",
                            h.path,
                            h.agent,
                            note_age(&h.updated),
                            h.snippet
                        )
                    })
                    .collect();
                task_notes_section = format!("## notes for this task\n{}", note_lines.join("\n"));
            }
        }

        match tool_graph(idx, &slug, task, 12) {
            Ok(res) => {
                code_graph_section = format!("## code graph for this task\n{}", res);
            }
            Err(e) => {
                code_graph_section = format!("## code graph\n{}", e);
            }
        }
    }

    let sections = [
        Section {
            name: "memory",
            text: &memory_section,
            weight: default_section_weight("memory"),
        },
        Section {
            name: "playbook",
            text: &playbook_section,
            weight: default_section_weight("playbook"),
        },
        Section {
            name: "task_notes",
            text: &task_notes_section,
            weight: default_section_weight("task_notes"),
        },
        Section {
            name: "code_graph",
            text: &code_graph_section,
            weight: default_section_weight("code_graph"),
        },
    ];

    Ok(pack(&sections, budget))
}

pub fn tool_skill(task: &str, auto_install: bool, scope: &str) -> Result<String, String> {
    let t0 = std::time::Instant::now();
    let task_str = task.trim();
    if task_str.is_empty() {
        return Ok("task description cannot be empty".to_string());
    }

    let skills = crate::skill::scan_skills(None);
    let match_res = crate::skill::match_skill(task_str, &skills, 0.65);
    let dur_ms = t0.elapsed().as_secs_f64() * 1000.0;

    if match_res.is_match {
        if let Some(s) = match_res.skill {
            let p = Path::new(&s.path);
            let ctx = crate::skill::load_skill_context(p);
            let _ = crate::ledger::record(
                &crate::ledger::default_ledger_path(),
                "skill",
                &s.name,
                task_str,
                std::slice::from_ref(&s.path),
                &[],
                dur_ms,
                None,
                None,
            );
            return Ok(ctx);
        }
    }

    if scope == "all" {
        let candidates = crate::skill::find_remote_skills(task_str);
        if !candidates.is_empty() && auto_install {
            let top = &candidates[0];
            match crate::skill::install_skill(&top.package_name, None, false) {
                Ok(installed_path) => {
                    let ctx = crate::skill::load_skill_context(&installed_path);
                    let dur_ms = t0.elapsed().as_secs_f64() * 1000.0;
                    let _ = crate::ledger::record(
                        &crate::ledger::default_ledger_path(),
                        "skill",
                        &top.package_name,
                        task_str,
                        &[installed_path.display().to_string()],
                        &[],
                        dur_ms,
                        None,
                        None,
                    );
                    return Ok(format!(
                        "> [!NOTE] Automatically installed skill `{}` from registry\n\n{}",
                        top.package_name, ctx
                    ));
                }
                Err(err) => {
                    return Ok(format!(
                        "> [!WARNING] Auto-install failed for {}: {}",
                        top.package_name, err
                    ));
                }
            }
        } else if !candidates.is_empty() && !auto_install {
            let dur_ms = t0.elapsed().as_secs_f64() * 1000.0;
            let _ = crate::ledger::record(
                &crate::ledger::default_ledger_path(),
                "skill",
                "",
                task_str,
                &[],
                &[],
                dur_ms,
                None,
                None,
            );
            let mut lines = vec![format!(
                "No matching local skill for task: {:?}. Top remote candidates:",
                task_str
            )];
            for c in candidates.iter().take(3) {
                lines.push(format!(
                    "- `{}` ({} stars): {}",
                    c.package_name, c.stars, c.description
                ));
            }
            lines.push(format!(
                "\nTo install: npx skills add {}",
                candidates[0].package_name
            ));
            return Ok(lines.join("\n"));
        }
    }

    let dur_ms = t0.elapsed().as_secs_f64() * 1000.0;
    let _ = crate::ledger::record(
        &crate::ledger::default_ledger_path(),
        "skill",
        "",
        task_str,
        &[],
        &[],
        dur_ms,
        None,
        None,
    );
    Ok(format!(
        "no local or remote skills found matching task: {:?}",
        task_str
    ))
}

pub fn handle_rpc(idx: &mut Index, req: &Value) -> Option<Value> {
    let id = req.get("id");
    let method = req.get("method")?.as_str()?;

    match method {
        "initialize" => {
            let client_ver = req
                .pointer("/params/protocolVersion")
                .and_then(|v| v.as_str())
                .unwrap_or("2024-11-05");

            let supported_ver =
                if MODERN_VERSIONS.contains(&client_ver) || LEGACY_VERSIONS.contains(&client_ver) {
                    client_ver
                } else {
                    "2024-11-05"
                };

            Some(json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "protocolVersion": supported_ver,
                    "serverInfo": {
                        "name": SERVER_NAME,
                        "version": SERVER_VERSION
                    },
                    "instructions": INSTRUCTIONS,
                    "capabilities": {
                        "tools": {
                            "listChanged": false
                        }
                    }
                }
            }))
        }
        "notifications/initialized" => None,
        "ping" => Some(json!({"jsonrpc": "2.0", "id": id, "result": {}})),
        "tools/list" => Some(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "tools": tool_list()
            }
        })),
        "tools/call" => {
            let tool_name = req
                .pointer("/params/name")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let empty_args = json!({});
            let tool_args = req.pointer("/params/arguments").unwrap_or(&empty_args);

            if let Err(err) = validate_tool_args(tool_name, tool_args) {
                return Some(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "isError": true,
                        "content": [{
                            "type": "text",
                            "text": format!("invalid arguments for {}: {}", tool_name, err)
                        }]
                    }
                }));
            }

            let call_res: Result<String, String> = match tool_name {
                "hive_context" => {
                    let project = tool_args
                        .get("project")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let task = tool_args.get("task").and_then(|v| v.as_str()).unwrap_or("");
                    let budget = tool_args
                        .get("budget")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(1500) as usize;
                    tool_context(idx, project, task, budget)
                }
                "hive_search" => {
                    let query = tool_args
                        .get("query")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let project = tool_args
                        .get("project")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let scope = tool_args.get("scope").and_then(|v| v.as_str()).unwrap_or({
                        if !project.is_empty() {
                            "project"
                        } else {
                            "all"
                        }
                    });
                    let limit =
                        tool_args.get("limit").and_then(|v| v.as_u64()).unwrap_or(8) as usize;
                    match idx.search(query, project, limit, scope) {
                        Ok(hits) => {
                            if hits.is_empty() {
                                Ok("no matching notes".to_string())
                            } else {
                                let lines: Vec<String> = hits
                                    .into_iter()
                                    .map(|h| {
                                        format!(
                                            "- {} ({}, {}, {}) {}\n  {}",
                                            h.path,
                                            h.r#type,
                                            h.status,
                                            h.updated,
                                            h.title,
                                            h.snippet
                                        )
                                    })
                                    .collect();
                                Ok(lines.join("\n"))
                            }
                        }
                        Err(e) => Err(e),
                    }
                }
                "hive_graph" => {
                    let repo = tool_args.get("repo").and_then(|v| v.as_str()).unwrap_or("");
                    let query = tool_args
                        .get("query")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let limit = tool_args
                        .get("limit")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(15) as usize;
                    tool_graph(idx, repo, query, limit)
                }
                "hive_status" => {
                    let _ = idx.refresh(true);
                    let mut lines = Vec::new();
                    let total: i64 = idx
                        .conn
                        .query_row("SELECT count(*) FROM notes", [], |r| r.get(0))
                        .unwrap_or(0);
                    lines.push(format!(
                        "vault {}: {} notes indexed ({})",
                        idx.vault.display(),
                        total,
                        default_db_path().display()
                    ));
                    Ok(lines.join("\n"))
                }
                "hive_lint" => {
                    let findings = lint(&idx.vault, None);
                    Ok(format_report(&findings))
                }
                "hive_skill" => {
                    let task = tool_args.get("task").and_then(|v| v.as_str()).unwrap_or("");
                    let auto_install = tool_args
                        .get("auto_install")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(true);
                    let scope = tool_args
                        .get("scope")
                        .and_then(|v| v.as_str())
                        .unwrap_or("all");
                    tool_skill(task, auto_install, scope)
                }
                other => Err(format!("tool {:?} not found", other)),
            };

            match call_res {
                Ok(content) => Some(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "content": [{
                            "type": "text",
                            "text": content
                        }],
                        "isError": false
                    }
                })),
                Err(err_msg) => Some(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "isError": true,
                        "content": [{
                            "type": "text",
                            "text": err_msg
                        }]
                    }
                })),
            }
        }
        _ => Some(json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {
                "code": -32601,
                "message": format!("Method not found: {}", method)
            }
        })),
    }
}

pub fn serve(vault: Option<PathBuf>, db_path: Option<PathBuf>) -> Result<(), String> {
    let mut idx = Index::new(vault, db_path)?;
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    for line_res in stdin.lock().lines() {
        let line = match line_res {
            Ok(l) => l,
            Err(_) => break,
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if let Ok(req) = serde_json::from_str::<Value>(trimmed) {
            if let Some(resp) = handle_rpc(&mut idx, &req) {
                let resp_str = serde_json::to_string(&resp).unwrap_or_default();
                writeln!(stdout, "{}", resp_str).map_err(|e| e.to_string())?;
                stdout.flush().map_err(|e| e.to_string())?;
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initialize_and_tool_list() {
        let mut idx = Index::in_memory().unwrap();
        let init_req = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {"protocolVersion": "2026-07-28"}
        });
        let resp = handle_rpc(&mut idx, &init_req).unwrap();
        assert_eq!(resp["result"]["protocolVersion"], "2026-07-28");
        assert_eq!(resp["result"]["serverInfo"]["name"], "hive");

        let list_req = json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list"
        });
        let list_resp = handle_rpc(&mut idx, &list_req).unwrap();
        let tools = list_resp["result"]["tools"].as_array().unwrap();
        assert!(tools.iter().any(|t| t["name"] == "hive_context"));
        assert!(tools.iter().any(|t| t["name"] == "hive_search"));
        assert!(tools.iter().any(|t| t["name"] == "hive_graph"));
    }

    #[test]
    fn test_argument_validation() {
        assert!(validate_tool_args("hive_context", &json!({"project": "ai-hive"})).is_ok());
        assert!(validate_tool_args("hive_context", &json!({})).is_err());
        assert!(
            validate_tool_args("hive_context", &json!({"project": "ai-hive", "extra": 123}))
                .is_err()
        );
    }
}
