use crate::common::default_projects;
use crate::graph::Graph;
use crate::index::Index;
use std::fs;
use std::time::Instant;

pub fn run_bench(idx: &mut Index) -> Result<String, String> {
    let mut lines = Vec::new();

    // 1. Index refresh
    let t0 = Instant::now();
    let _ = idx.refresh(true)?;
    lines.push(format!("index refresh: {:.1} ms", t0.elapsed().as_secs_f64() * 1000.0));

    // 2. Vault search
    let t0 = Instant::now();
    let _ = idx.search("price source deploy", "", 8, "project")?;
    lines.push(format!("vault search: {:.1} ms", t0.elapsed().as_secs_f64() * 1000.0));

    // 3. Graph benchmarks
    let projects = default_projects();
    if projects.is_dir() {
        if let Ok(entries) = fs::read_dir(&projects) {
            let mut dirs: Vec<_> = entries.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
            dirs.sort();

            for d in dirs {
                let graph_file = d.join("graphify-out").join("graph.json");
                if !graph_file.is_file() {
                    continue;
                }
                let t0 = Instant::now();
                let text = match fs::read_to_string(&graph_file) {
                    Ok(t) => t,
                    Err(_) => continue,
                };
                let g = match Graph::from_json_str(&text) {
                    Ok(graph) => graph,
                    Err(_) => continue,
                };
                let cold_ms = t0.elapsed().as_secs_f64() * 1000.0;

                let hub_id = g.nodes.keys().max_by_key(|nid| g.degree(nid));
                let hub_label = hub_id
                    .and_then(|id| g.nodes.get(id))
                    .and_then(|n| n.label.as_deref())
                    .unwrap_or("main");

                let t0 = Instant::now();
                for _ in 0..5 {
                    let _ = g.query(hub_label, 15);
                }
                let avg_query_ms = (t0.elapsed().as_secs_f64() * 1000.0) / 5.0;

                let dir_name = d.file_name().and_then(|f| f.to_str()).unwrap_or("repo");
                lines.push(format!(
                    "{}: {} nodes, load {:.1} ms, hub query {:.1} ms",
                    dir_name,
                    g.nodes.len(),
                    cold_ms,
                    avg_query_ms
                ));
            }
        }
    }

    Ok(lines.join("\n"))
}
