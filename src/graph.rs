use crate::common::{default_projects, is_relative_to, slugify};
use regex::Regex;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

pub const REL_WEIGHT: &[(&str, f64)] = &[
    ("calls", 1.0),
    ("indirect_call", 0.8),
    ("inherits", 0.9),
    ("implements", 0.9),
    ("method", 0.8),
    ("references", 0.7),
    ("uses", 0.7),
    ("defines", 0.6),
    ("rationale_for", 0.6),
    ("imports", 0.5),
    ("imports_from", 0.5),
    ("contains", 0.4),
];

pub const PPR_RESTART: f64 = 0.15;
pub const PPR_ITERS: usize = 25;
pub const PPR_HOPS: usize = 2;
pub const FORWARD_WEIGHT: f64 = 1.0;
pub const REVERSE_WEIGHT: f64 = 0.5;

pub fn rel_weight(rel: &str) -> f64 {
    for &(r, w) in REL_WEIGHT {
        if r == rel {
            return w;
        }
    }
    0.6
}

pub fn kind_prior(k: &str) -> f64 {
    match k {
        "code" => 1.5,
        "document" | "concept" => 0.8,
        "rationale" => 0.6,
        _ => 0.7,
    }
}

pub fn node_prior(n: &Node) -> f64 {
    let path = n.source_file.as_deref().unwrap_or("").replace('\\', "/").to_lowercase();
    let is_test = path.contains("/tests/")
        || Path::new(&path).file_name().and_then(|f| f.to_str()).map_or(false, |name| name.starts_with("test_"))
        || path.ends_with("_test.py");
    let base = kind_prior(n.file_type.as_deref().unwrap_or(""));
    let test_mult = if is_test { 0.6 } else { 1.0 };
    let path_mult = if !path.is_empty() { 1.0 } else { 0.5 };
    base * test_mult * path_mult
}

pub fn split_ident(text: &str) -> Vec<String> {
    let re_camel = Regex::new(r"([a-z0-9])([A-Z])").unwrap();
    let spaced = re_camel.replace_all(text, "$1 $2");
    let re_non_alnum = Regex::new(r"[^A-Za-z0-9]+").unwrap();
    re_non_alnum
        .split(&spaced)
        .map(|s| s.to_lowercase())
        .filter(|s| s.len() > 1)
        .collect()
}

#[derive(Debug, Clone, Deserialize)]
pub struct Node {
    pub id: String,
    pub label: Option<String>,
    pub file_type: Option<String>,
    pub source_file: Option<String>,
    pub source_location: Option<String>,
    pub community_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Link {
    pub source: String,
    pub target: String,
    pub relation: Option<String>,
    pub confidence_score: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GraphData {
    pub built_at_commit: Option<String>,
    pub nodes: Option<Vec<Node>>,
    pub links: Option<Vec<Link>>,
    pub edges: Option<Vec<Link>>,
}

#[derive(Debug, Clone)]
pub struct Graph {
    pub commit: String,
    pub nodes: HashMap<String, Node>,
    pub out_adj: HashMap<String, HashMap<String, (f64, String)>>,
    pub in_adj: HashMap<String, HashMap<String, (f64, String)>>,
    pub tokens: HashMap<String, HashSet<String>>,
    pub idf: HashMap<String, f64>,
}

impl Graph {
    pub fn from_json_str(json_str: &str) -> Result<Self, String> {
        let data: GraphData = serde_json::from_str(json_str).map_err(|e| e.to_string())?;
        let commit = data.built_at_commit.unwrap_or_default();
        let raw_nodes = data.nodes.unwrap_or_default();
        let mut nodes = HashMap::new();
        for n in raw_nodes {
            if !n.id.is_empty() {
                nodes.insert(n.id.clone(), n);
            }
        }

        let mut out_adj: HashMap<String, HashMap<String, (f64, String)>> = HashMap::new();
        let mut in_adj: HashMap<String, HashMap<String, (f64, String)>> = HashMap::new();

        let raw_links = data.links.or(data.edges).unwrap_or_default();
        for e in raw_links {
            let s = e.source;
            let t = e.target;
            let rel = e.relation.unwrap_or_default();
            if s == t || !nodes.contains_key(&s) || !nodes.contains_key(&t) {
                continue;
            }
            let w = e.confidence_score.unwrap_or(0.5) * rel_weight(&rel);
            let prev_out = out_adj.entry(s.clone()).or_default().get(&t).map(|x| x.0).unwrap_or(0.0);
            if w > prev_out {
                out_adj.entry(s.clone()).or_default().insert(t.clone(), (w, rel.clone()));
            }
            let prev_in = in_adj.entry(t.clone()).or_default().get(&s).map(|x| x.0).unwrap_or(0.0);
            if w > prev_in {
                in_adj.entry(t).or_default().insert(s, (w, rel));
            }
        }

        let mut tokens: HashMap<String, HashSet<String>> = HashMap::new();
        for (nid, n) in &nodes {
            let stem = Path::new(n.source_file.as_deref().unwrap_or(""))
                .file_stem()
                .and_then(|f| f.to_str())
                .unwrap_or("");
            let words = [split_ident(n.label.as_deref().unwrap_or("")), split_ident(stem)].concat();
            for tok in words {
                tokens.entry(tok).or_default().insert(nid.clone());
            }
        }

        let num_nodes = nodes.len() as f64;
        let mut idf = HashMap::new();
        for (t, ids) in &tokens {
            let val = (1.0 + num_nodes / (ids.len() as f64)).ln();
            idf.insert(t.clone(), val);
        }

        Ok(Self {
            commit,
            nodes,
            out_adj,
            in_adj,
            tokens,
            idf,
        })
    }

    pub fn degree(&self, nid: &str) -> usize {
        let out_deg = self.out_adj.get(nid).map(|m| m.len()).unwrap_or(0);
        let in_deg = self.in_adj.get(nid).map(|m| m.len()).unwrap_or(0);
        out_deg + in_deg
    }

    pub fn seeds(&self, query: &str, k: usize) -> Vec<String> {
        if self.nodes.contains_key(query) {
            return vec![query.to_string()];
        }
        let exact = query.trim().to_lowercase();
        let mut score: HashMap<String, f64> = HashMap::new();
        for tok in split_ident(query) {
            if let Some(ids) = self.tokens.get(&tok) {
                let idf_val = self.idf.get(&tok).copied().unwrap_or(0.0);
                for nid in ids {
                    *score.entry(nid.clone()).or_default() += idf_val;
                }
            }
        }

        let mut scored_list: Vec<(String, f64)> = score
            .into_iter()
            .map(|(nid, mut sc)| {
                let n = &self.nodes[&nid];
                let is_exact = n.label.as_deref().unwrap_or("").to_lowercase() == exact;
                let mult = if is_exact { 3.0 } else { 1.0 };
                sc *= node_prior(n) * mult;
                (nid, sc)
            })
            .collect();

        scored_list.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored_list.into_iter().take(k).map(|(nid, _)| nid).collect()
    }

    pub fn rank(&self, seeds: &[String], hops: usize, iters: usize) -> HashMap<String, f64> {
        if seeds.is_empty() {
            return HashMap::new();
        }
        let mut local: HashSet<String> = seeds.iter().cloned().collect();
        let mut frontier: HashSet<String> = seeds.iter().cloned().collect();

        for _ in 0..hops {
            let mut next_frontier = HashSet::new();
            for a in &frontier {
                if let Some(out_map) = self.out_adj.get(a) {
                    for b in out_map.keys() {
                        if !local.contains(b) {
                            next_frontier.insert(b.clone());
                        }
                    }
                }
                if let Some(in_map) = self.in_adj.get(a) {
                    for b in in_map.keys() {
                        if !local.contains(b) {
                            next_frontier.insert(b.clone());
                        }
                    }
                }
            }
            local.extend(next_frontier.iter().cloned());
            frontier = next_frontier;
        }

        let mut out: HashMap<String, Vec<(String, f64)>> = HashMap::new();
        for a in &local {
            let mut nbrs = Vec::new();
            if let Some(out_map) = self.out_adj.get(a) {
                for (b, &(w, _)) in out_map {
                    if local.contains(b) {
                        let deg = self.degree(b).max(1) as f64;
                        nbrs.push((b.clone(), w * FORWARD_WEIGHT / deg.sqrt()));
                    }
                }
            }
            if let Some(in_map) = self.in_adj.get(a) {
                for (b, &(w, _)) in in_map {
                    if local.contains(b) {
                        let deg = self.degree(b).max(1) as f64;
                        nbrs.push((b.clone(), w * REVERSE_WEIGHT / deg.sqrt()));
                    }
                }
            }
            let total: f64 = nbrs.iter().map(|(_, w)| *w).sum();
            if total > 0.0 {
                out.insert(a.clone(), nbrs.into_iter().map(|(b, w)| (b, w / total)).collect());
            } else {
                out.insert(a.clone(), Vec::new());
            }
        }

        let restart_prob = 1.0 / (seeds.len() as f64);
        let mut rank: HashMap<String, f64> = seeds.iter().map(|s| (s.clone(), restart_prob)).collect();

        for _ in 0..iters {
            let mut nxt: HashMap<String, f64> = HashMap::new();
            let mut dangling = 0.0;

            for (a, &p) in &rank {
                if let Some(edges) = out.get(a) {
                    if !edges.is_empty() {
                        for (b, w) in edges {
                            *nxt.entry(b.clone()).or_default() += (1.0 - PPR_RESTART) * p * w;
                        }
                    } else {
                        dangling += p;
                    }
                } else {
                    dangling += p;
                }
            }

            for s in seeds {
                *nxt.entry(s.clone()).or_default() += (PPR_RESTART + (1.0 - PPR_RESTART) * dangling) * restart_prob;
            }
            rank = nxt;
        }

        rank
    }

    pub fn query(&self, query: &str, limit: usize) -> (Vec<(String, f64, bool)>, Vec<(String, String, String)>) {
        let seeds = self.seeds(query, 6);
        if seeds.is_empty() {
            return (Vec::new(), Vec::new());
        }

        let raw_rank = self.rank(&seeds, PPR_HOPS, PPR_ITERS);
        let mut rank: HashMap<String, f64> = HashMap::new();
        for (nid, p) in raw_rank {
            let pr = node_prior(&self.nodes[&nid]);
            rank.insert(nid, p * pr);
        }

        let seed_set: HashSet<String> = seeds.iter().cloned().collect();
        let mut sorted_seeds = seeds.clone();
        sorted_seeds.sort_by(|a, b| {
            rank.get(b).unwrap_or(&0.0).partial_cmp(rank.get(a).unwrap_or(&0.0)).unwrap_or(std::cmp::Ordering::Equal)
        });

        let mut rest: Vec<String> = rank.keys().filter(|k| !seed_set.contains(*k)).cloned().collect();
        rest.sort_by(|a, b| {
            rank.get(b).unwrap_or(&0.0).partial_cmp(rank.get(a).unwrap_or(&0.0)).unwrap_or(std::cmp::Ordering::Equal)
        });

        let mut top = sorted_seeds;
        top.extend(rest);
        top.truncate(limit);

        let keep: HashSet<String> = top.iter().cloned().collect();
        let mut edges = Vec::new();
        for s in &top {
            if let Some(out_map) = self.out_adj.get(s) {
                for (t, &(_, ref rel)) in out_map {
                    if keep.contains(t) {
                        edges.push((s.clone(), rel.clone(), t.clone()));
                    }
                }
            }
        }

        let scored_top = top
            .into_iter()
            .map(|nid| {
                let is_seed = seed_set.contains(&nid);
                let score = rank.get(&nid).copied().unwrap_or(0.0);
                (nid, score, is_seed)
            })
            .collect();

        (scored_top, edges)
    }
}

pub fn repo_dir(repo: &str) -> Option<PathBuf> {
    let projects = default_projects().canonicalize().ok()?;
    let path = PathBuf::from(repo);
    if path.is_dir() {
        if let Ok(resolved) = path.canonicalize() {
            if is_relative_to(&resolved, &projects) && resolved.join("graphify-out").is_dir() {
                return Some(resolved);
            }
        }
    }
    let want = slugify(path.file_name().and_then(|f| f.to_str()).unwrap_or(repo));
    let re = Regex::new(r"^[a-z0-9._-]+$").unwrap();
    if !re.is_match(&want) || want == "." || want == ".." {
        return None;
    }
    if projects.is_dir() {
        if let Ok(entries) = fs::read_dir(projects) {
            for entry in entries.flatten() {
                if entry.path().is_dir() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    if slugify(&name) == want {
                        return Some(entry.path());
                    }
                }
            }
        }
    }
    None
}

pub fn fmt_node(n: &Node, seed: bool) -> String {
    let loc = n.source_location.as_deref().unwrap_or("");
    let loc_clean = if loc.starts_with('L') { &loc[1..] } else { loc };
    let file = n.source_file.as_deref().unwrap_or("");
    let where_str = if !file.is_empty() && !loc_clean.is_empty() {
        format!("{}:{}", file, loc_clean)
    } else if !file.is_empty() {
        file.to_string()
    } else {
        String::new()
    };

    let label_raw = n.label.as_deref().unwrap_or(&n.id);
    let label_clean: String = label_raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let label = if label_clean.len() <= 80 {
        label_clean
    } else {
        format!("{}...", &label_clean[..77])
    };

    let comm = n.community_name.as_deref().unwrap_or("");
    let marker = if seed { '*' } else { '-' };
    format!("{} {}  {}  [{}]", marker, label, where_str, comm)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_graph_seeds_and_directed_ppr() {
        let json_data = r#"{
            "built_at_commit": "abc1234",
            "nodes": [
                {"id": "func_a", "label": "func_a", "file_type": "code", "source_file": "src/a.py"},
                {"id": "func_b", "label": "func_b", "file_type": "code", "source_file": "src/b.py"},
                {"id": "hub_node", "label": "hub_node", "file_type": "code", "source_file": "src/hub.py"}
            ],
            "links": [
                {"source": "func_a", "target": "func_b", "relation": "calls", "confidence_score": 1.0},
                {"source": "func_b", "target": "hub_node", "relation": "calls", "confidence_score": 0.8}
            ]
        }"#;

        let g = Graph::from_json_str(json_data).unwrap();
        assert_eq!(g.degree("func_a"), 1);
        assert_eq!(g.degree("func_b"), 2);
        assert_eq!(g.degree("hub_node"), 1);

        let (ranked, edges) = g.query("func_a", 10);
        assert!(!ranked.is_empty());
        assert_eq!(ranked[0].0, "func_a");
        assert!(ranked[0].2); // is_seed

        // Directed edge: func_a -calls-> func_b
        assert!(edges.iter().any(|(s, r, t)| s == "func_a" && r == "calls" && t == "func_b"));
    }
}
