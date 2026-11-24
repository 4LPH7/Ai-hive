use crate::ace::{is_secret, load, MAX_BULLET_CHARS};
use crate::llm::{chat_completion, load_llm_config, resolve_combo};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ParaphraseMerge {
    pub target_id: String,
    pub merged_content: String,
    pub rationale: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Contradiction {
    pub bullet_a: String,
    pub bullet_b: String,
    pub issue: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AdrDrift {
    pub adr_path: String,
    pub drift_reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct QueryAliasProposal {
    pub cluster_topic: String,
    pub aliases: Vec<String>,
    pub suggested_note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CuratePayload {
    #[serde(default)]
    pub paraphrase_merges: Vec<ParaphraseMerge>,
    #[serde(default)]
    pub contradictions: Vec<Contradiction>,
    #[serde(default)]
    pub adr_drift_warnings: Vec<AdrDrift>,
    #[serde(default)]
    pub query_alias_proposals: Vec<QueryAliasProposal>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CurateResult {
    pub model: String,
    pub merges_count: usize,
    pub contradictions_count: usize,
    pub adr_drifts_count: usize,
    pub alias_proposals_count: usize,
    pub proposal_file: Option<String>,
    pub dry_run: bool,
    pub skipped: bool,
    pub reason: Option<String>,
}

pub fn curate_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "paraphrase_merges": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "target_id": {"type": "string"},
                        "merged_content": {"type": "string"},
                        "rationale": {"type": "string"}
                    },
                    "required": ["target_id", "merged_content", "rationale"]
                }
            },
            "contradictions": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "bullet_a": {"type": "string"},
                        "bullet_b": {"type": "string"},
                        "issue": {"type": "string"}
                    },
                    "required": ["bullet_a", "bullet_b", "issue"]
                }
            },
            "adr_drift_warnings": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "adr_path": {"type": "string"},
                        "drift_reason": {"type": "string"}
                    },
                    "required": ["adr_path", "drift_reason"]
                }
            },
            "query_alias_proposals": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "cluster_topic": {"type": "string"},
                        "aliases": {"type": "array", "items": {"type": "string"}},
                        "suggested_note": {"type": "string"}
                    },
                    "required": ["cluster_topic", "aliases", "suggested_note"]
                }
            }
        },
        "required": ["paraphrase_merges", "contradictions", "adr_drift_warnings", "query_alias_proposals"]
    })
}

pub fn run_curate(
    ledger_path: &Path,
    vault_path: &Path,
    project: Option<&str>,
    dry_run: bool,
    config_path: Option<&Path>,
) -> Result<CurateResult, String> {
    let llm_cfg = load_llm_config(config_path);
    if !llm_cfg.enabled {
        return Ok(CurateResult {
            model: "disabled".to_string(),
            merges_count: 0,
            contradictions_count: 0,
            adr_drifts_count: 0,
            alias_proposals_count: 0,
            proposal_file: None,
            dry_run,
            skipped: true,
            reason: Some("LLM disabled in configuration".to_string()),
        });
    }

    let scope = project.unwrap_or("global");
    let model = resolve_combo(&llm_cfg, "deep", Some(scope));

    // 1. Gather context
    let playbook_sections = load(vault_path, scope).unwrap_or_default();
    let mut known_bullet_ids = HashSet::new();
    let mut playbook_summary = Vec::new();

    for (_sec_name, bullets) in &playbook_sections {
        for b in bullets {
            known_bullet_ids.insert(b.id.clone());
            playbook_summary.push(format!("- [{}] (seen={}, +{}, -{}) {}", b.id, b.seen, b.helpful, b.harmful, b.content));
        }
    }

    // Zero-hit queries from miner
    let gaps = crate::miner::scan_ledger_gaps(ledger_path);
    let top_misses: Vec<String> = gaps.iter().take(15).map(|(q, _)| q.clone()).collect();

    let system_prompt = "You are the hive intelligence curator. Analyze the given ACE playbook and recent query telemetry. Propose high-signal playbook consolidations (paraphrase merges), flag contradictory bullets, check for stale architectural drift, and propose query aliases. Conform strictly to the JSON schema.";

    let joined_playbook = if playbook_summary.is_empty() {
        "(empty playbook)".to_string()
    } else {
        playbook_summary.join("\n")
    };

    let joined_misses = if top_misses.is_empty() {
        "(none)".to_string()
    } else {
        top_misses.join("\n")
    };

    let user_prompt = format!(
        "Scope: {}\n\nPlaybook:\n{}\n\nRecent zero-hit queries:\n{}",
        scope, joined_playbook, joined_misses
    );

    let schema = curate_schema();
    let llm_response = match chat_completion(&llm_cfg, &model, system_prompt, &user_prompt, Some(&schema)) {
        Ok(res) => res,
        Err(e) => {
            // Fail-open guarantee: network/model server offline skips gracefully
            return Ok(CurateResult {
                model,
                merges_count: 0,
                contradictions_count: 0,
                adr_drifts_count: 0,
                alias_proposals_count: 0,
                proposal_file: None,
                dry_run,
                skipped: true,
                reason: Some(format!("OmniRoute call failed (fail-open): {}", e)),
            });
        }
    };

    let payload: CuratePayload = serde_json::from_str(&llm_response).unwrap_or_default();

    // 2. Deterministic Validation Gate (Reject hallucinations, secrets, over-length)
    let mut validated_merges = Vec::new();
    for m in payload.paraphrase_merges {
        if !known_bullet_ids.contains(&m.target_id) {
            continue; // reject hallucinated target bullet ID
        }
        let content = m.merged_content.trim();
        if content.is_empty() || content.len() > MAX_BULLET_CHARS || is_secret(content) {
            continue;
        }
        validated_merges.push(m);
        if validated_merges.len() >= 5 {
            break; // cap at 5 per curation run
        }
    }

    let mut validated_contradictions = Vec::new();
    for c in payload.contradictions {
        if !known_bullet_ids.contains(&c.bullet_a) || !known_bullet_ids.contains(&c.bullet_b) {
            continue; // reject hallucinated IDs
        }
        validated_contradictions.push(c);
    }

    let validated_drifts = payload.adr_drift_warnings;
    let validated_aliases = payload.query_alias_proposals;

    let merges_count = validated_merges.len();
    let contradictions_count = validated_contradictions.len();
    let adr_drifts_count = validated_drifts.len();
    let alias_proposals_count = validated_aliases.len();

    let total_proposals = merges_count + contradictions_count + adr_drifts_count + alias_proposals_count;
    if total_proposals == 0 {
        return Ok(CurateResult {
            model,
            merges_count: 0,
            contradictions_count: 0,
            adr_drifts_count: 0,
            alias_proposals_count: 0,
            proposal_file: None,
            dry_run,
            skipped: false,
            reason: Some("No actionable proposals passed deterministic validation".to_string()),
        });
    }

    // 3. Render Proposal exclusively to inbox/ (Inbox-only guarantee)
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(llm_response.as_bytes());
    let hash = format!("{:x}", hasher.finalize())[..8].to_string();

    let mut lines = vec![
        "---".to_string(),
        "type: proposal".to_string(),
        "status: inbox".to_string(),
        "source: hive-curate".to_string(),
        format!("model: {}", model),
        format!("project: {}", scope),
        format!("updated: {}", today),
        "---".to_string(),
        format!("# Curation Proposal: {} ({})", scope, today),
        "".to_string(),
        format!("> Generated by OmniRoute-backed intelligence engine (`{}`).", model),
        "> Validated by deterministic ACE gate. Review before applying.".to_string(),
        "".to_string(),
    ];

    if !validated_merges.is_empty() {
        lines.push("## Paraphrase Merges".to_string());
        for m in &validated_merges {
            lines.push(format!("- [ ] Target `[{}]`: {} (Rationale: {})", m.target_id, m.merged_content, m.rationale));
        }
        lines.push("".to_string());
    }

    if !validated_contradictions.is_empty() {
        lines.push("## Contradiction Flags".to_string());
        for c in &validated_contradictions {
            lines.push(format!("- [ ] Potential conflict between `[{}]` and `[{}]`: {}", c.bullet_a, c.bullet_b, c.issue));
        }
        lines.push("".to_string());
    }

    if !validated_drifts.is_empty() {
        lines.push("## ADR Drift Warnings".to_string());
        for d in &validated_drifts {
            lines.push(format!("- [ ] `{}`: {}", d.adr_path, d.drift_reason));
        }
        lines.push("".to_string());
    }

    if !validated_aliases.is_empty() {
        lines.push("## Query Aliases".to_string());
        for a in &validated_aliases {
            lines.push(format!("- [ ] Topic `{}` -> note `{}` (aliases: {:?})", a.cluster_topic, a.suggested_note, a.aliases));
        }
        lines.push("".to_string());
    }

    let markdown = lines.join("\n");
    let inbox_dir = vault_path.join("inbox");
    let proposal_file = inbox_dir.join(format!("curate-{}-{}.md", today, hash));

    if !dry_run {
        let _ = fs::create_dir_all(&inbox_dir);
        let _ = fs::write(&proposal_file, &markdown);
    }

    Ok(CurateResult {
        model,
        merges_count,
        contradictions_count,
        adr_drifts_count,
        alias_proposals_count,
        proposal_file: Some(proposal_file.to_string_lossy().to_string()),
        dry_run,
        skipped: false,
        reason: None,
    })
}
