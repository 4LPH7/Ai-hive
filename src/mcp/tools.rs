use crate::common::{active_work_cell_matches, default_projects, note_age, slugify, validate_slug};
use crate::graph::{fmt_node, repo_dir, Graph};
use crate::index::{parse_note, Index};
use crate::packer::{default_section_weight, pack, Section};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

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
    concise: bool,
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

    if concise {
        // DETAIL-01: no community tags, edges or vault notes (labels are whitespace-collapsed by fmt_node)
        for (nid, _, is_seed) in &ranked {
            let line = fmt_node(&g.nodes[nid], *is_seed);
            lines.push(match line.rsplit_once("  [") {
                Some((head, _)) => head.to_string(),
                None => line,
            });
        }
        return Ok(lines.join("\n"));
    }

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
    handoff: &str,
) -> Result<String, String> {
    let slug = validate_slug(project)?;
    if !handoff.is_empty() {
        // HAND-01: a receiving agent asked for one envelope; return only that
        let t0 = std::time::Instant::now();
        let view = crate::codex::handoff_view(&idx.vault, &slug, handoff, budget)?;
        let _ = crate::ledger::record(
            &crate::ledger::default_ledger_path(),
            "context",
            &slug,
            &format!("handoff {handoff}"),
            &[format!("codices/{slug}/{handoff}.md")],
            &[],
            t0.elapsed().as_secs_f64() * 1000.0,
            None,
            None,
        );
        return Ok(view);
    }

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
        let age = note_age(meta.get("updated").map(|s| s.as_str()).unwrap_or(""));
        let framed_body = crate::common::frame_untrusted(&mem_rel, &meta, &body);
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
    let mut scopes = vec!["global".to_string()];
    if slug != "global" {
        scopes.push(slug.clone());
    }
    let rows = crate::ace::select(&idx.vault, &scopes, task, crate::ace::INJECT_LIMIT);
    if !rows.is_empty() {
        playbook_section = format!(
            "## Playbook (ACE; note ids you use, run skill hive-ace after substantial work)\n{}",
            crate::ace::format_rows(&rows)
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

        match tool_graph(idx, &slug, task, 12, false) {
            Ok(res) => {
                code_graph_section = format!("## code graph for this task\n{}", res);
            }
            Err(e) => {
                code_graph_section = format!("## code graph\n{}", e);
            }
        }
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let traces_section = match crate::trace::active(&idx.conn, &slug, now) {
        // GOV-02: quarantined traces never reach agents
        Ok(items) => crate::trace::context_section(
            &crate::trace::visible_traces(&crate::governance::load_rules(&idx.vault), items),
            task,
        ),
        Err(e) => {
            eprintln!("hive: traces skipped: {e}");
            String::new()
        }
    };
    // CODEX-03: codex dead ends / open threads share the traces slice
    let codex_notes = crate::codex::context_section(&idx.vault, &slug, task);
    let traces_section = [traces_section, codex_notes]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");

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
            name: "traces",
            text: &traces_section,
            weight: default_section_weight("traces"),
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
