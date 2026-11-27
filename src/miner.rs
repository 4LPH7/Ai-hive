use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::Path;

const STOPWORDS: &[&str] = &[
    "a",
    "about",
    "above",
    "after",
    "again",
    "against",
    "all",
    "am",
    "an",
    "and",
    "any",
    "are",
    "as",
    "at",
    "be",
    "because",
    "been",
    "before",
    "being",
    "below",
    "between",
    "both",
    "but",
    "by",
    "could",
    "did",
    "do",
    "does",
    "doing",
    "down",
    "during",
    "each",
    "few",
    "for",
    "from",
    "further",
    "had",
    "has",
    "have",
    "having",
    "he",
    "her",
    "here",
    "hers",
    "herself",
    "him",
    "himself",
    "his",
    "how",
    "i",
    "if",
    "in",
    "into",
    "is",
    "it",
    "its",
    "itself",
    "me",
    "more",
    "most",
    "my",
    "myself",
    "no",
    "nor",
    "not",
    "of",
    "off",
    "on",
    "once",
    "only",
    "or",
    "other",
    "ought",
    "our",
    "ours",
    "ourselves",
    "out",
    "over",
    "own",
    "same",
    "she",
    "should",
    "so",
    "some",
    "such",
    "than",
    "that",
    "the",
    "their",
    "theirs",
    "them",
    "themselves",
    "then",
    "there",
    "these",
    "they",
    "this",
    "those",
    "through",
    "to",
    "too",
    "under",
    "until",
    "up",
    "very",
    "was",
    "we",
    "were",
    "what",
    "when",
    "where",
    "which",
    "while",
    "who",
    "whom",
    "why",
    "with",
    "would",
    "you",
    "your",
    "yours",
    "yourself",
    "yourselves",
];

pub fn tokenize(text: &str) -> HashSet<String> {
    let re = regex::Regex::new(r"[a-z0-9_-]+").unwrap();
    let stopwords: HashSet<&str> = STOPWORDS.iter().copied().collect();
    re.find_iter(&text.to_lowercase())
        .map(|m| m.as_str().to_string())
        .filter(|w| w.len() >= 2 && !stopwords.contains(w.as_str()))
        .collect()
}

pub fn jaccard(s1: &HashSet<String>, s2: &HashSet<String>) -> f64 {
    if s1.is_empty() || s2.is_empty() {
        return 0.0;
    }
    let intersection = s1.intersection(s2).count();
    let union = s1.union(s2).count();
    intersection as f64 / union as f64
}

#[derive(Debug, Clone)]
pub struct GapCluster {
    pub tokens: HashSet<String>,
    pub queries: Vec<String>,
    pub entries: Vec<crate::ledger::LedgerEntry>,
    pub representative: String,
    pub first_seen: String,
    pub last_seen: String,
}

impl GapCluster {
    pub fn new(query: &str, entry: crate::ledger::LedgerEntry) -> Self {
        let tokens = tokenize(query);
        let ts = entry.ts.clone();
        Self {
            tokens,
            queries: vec![query.to_string()],
            entries: vec![entry],
            representative: query.to_string(),
            first_seen: ts.clone(),
            last_seen: ts,
        }
    }

    pub fn matches(&self, other_tokens: &HashSet<String>, threshold: f64) -> bool {
        if self.tokens.is_empty() || other_tokens.is_empty() {
            return false;
        }
        let overlap = self.tokens.intersection(other_tokens).count();
        if overlap >= 2 {
            return true;
        }
        jaccard(&self.tokens, other_tokens) >= threshold
    }

    pub fn add(&mut self, query: &str, entry: crate::ledger::LedgerEntry) {
        let new_toks = tokenize(query);
        self.tokens.extend(new_toks);
        let ts = entry.ts.clone();
        if !ts.is_empty() {
            if self.first_seen.is_empty() || ts < self.first_seen {
                self.first_seen = ts.clone();
            }
            if self.last_seen.is_empty() || ts > self.last_seen {
                self.last_seen = ts;
            }
        }
        if query.len() > self.representative.len() {
            self.representative = query.to_string();
        }
        self.queries.push(query.to_string());
        self.entries.push(entry);
    }

    pub fn hash_id(&self) -> String {
        let mut sorted_toks: Vec<String> = self.tokens.iter().cloned().collect();
        sorted_toks.sort();
        let raw = if !sorted_toks.is_empty() {
            sorted_toks.join(":")
        } else {
            self.representative.clone()
        };

        let res = crate::common::sha256(raw.as_bytes());
        res[..8].to_string()
    }

    pub fn render_markdown(&self) -> String {
        let slug = regex::Regex::new(r"[^a-z0-9]+")
            .unwrap()
            .replace_all(&self.representative.to_lowercase(), "-")
            .trim_matches('-')
            .to_string();

        let mut lines = vec![
            "---".to_string(),
            "type: proposal".to_string(),
            "status: inbox".to_string(),
            "source: hive-miner".to_string(),
            format!("cluster: {}", self.representative),
            format!("first_seen: {}", self.first_seen),
            format!("last_seen: {}", self.last_seen),
            format!("count: {}", self.queries.len()),
            "---".to_string(),
            format!("# Gap Proposal: {}", self.representative),
            "".to_string(),
            "> Automatically mined from ledger zero-hit queries and misses.".to_string(),
            "> Review and convert to a permanent note, alias, or skill.".to_string(),
            "".to_string(),
            "## Queries".to_string(),
        ];

        for (e, q) in self.entries.iter().zip(&self.queries) {
            lines.push(format!("- \"{}\" (agent: {}, ts: {})", q, e.agent, e.ts));
        }

        lines.extend(vec![
            "".to_string(),
            "## Proposed Actions".to_string(),
            format!("- [ ] Create note `notes/{}.md` covering this topic", slug),
            "- [ ] Add aliases to existing note".to_string(),
            "- [ ] Install or author skill for this capability".to_string(),
            "".to_string(),
        ]);

        lines.join("\n")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MineProposal {
    pub hash: String,
    pub representative: String,
    pub count: usize,
    pub file: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MineResult {
    pub total_gaps: usize,
    pub total_clusters: usize,
    pub proposals_generated: usize,
    pub dry_run: bool,
    pub proposals: Vec<MineProposal>,
}

pub fn scan_ledger_gaps(
    ledger_path: &Path,
    project: Option<&str>,
) -> Vec<(String, crate::ledger::LedgerEntry)> {
    let entries = crate::ledger::read_ledger(ledger_path, None, project);
    let mut gaps = Vec::new();
    for e in entries {
        if let Some(p) = project {
            if !e.project.is_empty() && e.project != p {
                continue;
            }
        }
        let q = e.query.trim().to_string();
        if q.is_empty() {
            continue;
        }
        if (e.event == "query" || e.event == "skill") && e.hits.is_empty() {
            gaps.push((q, e));
        }
    }
    gaps
}

pub fn cluster_gaps(gaps: Vec<(String, crate::ledger::LedgerEntry)>) -> Vec<GapCluster> {
    let mut clusters: Vec<GapCluster> = Vec::new();
    for (query, entry) in gaps {
        let toks = tokenize(&query);
        if toks.is_empty() {
            continue;
        }
        let mut matched_idx = None;
        for (i, c) in clusters.iter().enumerate() {
            if c.matches(&toks, 0.35) {
                matched_idx = Some(i);
                break;
            }
        }
        if let Some(idx) = matched_idx {
            clusters[idx].add(&query, entry);
        } else {
            clusters.push(GapCluster::new(&query, entry));
        }
    }
    clusters
}

pub fn mine_gaps(
    ledger_path: &Path,
    vault_path: &Path,
    min_count: usize,
    dry_run: bool,
) -> Result<MineResult, String> {
    let gaps = scan_ledger_gaps(ledger_path, None);
    let total_gaps = gaps.len();
    let clusters = cluster_gaps(gaps);
    let total_clusters = clusters.len();

    let qualified: Vec<&GapCluster> = clusters
        .iter()
        .filter(|c| c.queries.len() >= min_count)
        .collect();
    let inbox_dir = vault_path.join("inbox");
    if !dry_run && !qualified.is_empty() {
        fs::create_dir_all(&inbox_dir).map_err(|e| e.to_string())?;
    }

    let mut proposals = Vec::new();
    for c in qualified {
        let hash = c.hash_id();
        let content = c.render_markdown();
        let target_file = inbox_dir.join(format!("gap-{}.md", hash));
        proposals.push(MineProposal {
            hash,
            representative: c.representative.clone(),
            count: c.queries.len(),
            file: target_file.to_string_lossy().to_string(),
            content: content.clone(),
        });
        if !dry_run {
            fs::write(&target_file, content).map_err(|e| e.to_string())?;
        }
    }

    Ok(MineResult {
        total_gaps,
        total_clusters,
        proposals_generated: proposals.len(),
        dry_run,
        proposals,
    })
}
