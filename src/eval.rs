use crate::index::Index;
use serde::Deserialize;
use std::fs;
use std::path::Path;

#[derive(Debug, Deserialize)]
pub struct EvalCase {
    pub q: String,
    pub project: Option<String>,
    pub expect: Vec<String>,
}

pub fn run_eval(idx: &mut Index, golden_path: &Path, k: usize) -> Result<String, String> {
    let content = fs::read_to_string(golden_path)
        .map_err(|e| format!("failed reading {}: {}", golden_path.display(), e))?;

    let cases: Vec<EvalCase> =
        serde_json::from_str(&content).map_err(|e| format!("invalid golden eval json: {}", e))?;

    let mut found = 0usize;
    let mut rr = 0.0f64;
    let mut misses = Vec::new();

    for case in &cases {
        let proj = case.project.as_deref().unwrap_or("");
        let hits = idx.search(&case.q, proj, k, "project")?;
        let paths: Vec<&str> = hits.iter().map(|h| h.path.as_str()).collect();

        let rank = paths
            .iter()
            .enumerate()
            .find(|(_, p)| case.expect.iter().any(|exp| exp == *p))
            .map(|(i, _)| i + 1)
            .unwrap_or(0);

        if rank > 0 {
            found += 1;
            rr += 1.0 / (rank as f64);
        } else {
            let top3: Vec<&str> = paths.iter().take(3).copied().collect();
            misses.push(format!("  miss: {:?} -> {:?}", case.q, top3));
        }
    }

    let n = cases.len();
    let n_f64 = if n == 0 { 1.0 } else { n as f64 };
    let recall = (found as f64) / n_f64;
    let mrr = rr / n_f64;

    let mut lines = vec![format!(
        "Recall@{} {}/{} = {:.2}   MRR {:.3}",
        k, found, n, recall, mrr
    )];
    lines.extend(misses);

    Ok(lines.join("\n"))
}
