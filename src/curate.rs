use crate::ace::{load, MAX_BULLET_CHARS};
use crate::llm::{chat_completion, load_llm_config, resolve_combo};
use crate::secrets::is_secret;
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

    for bullets in playbook_sections.values() {
        for b in bullets {
            known_bullet_ids.insert(b.id.clone());
            playbook_summary.push(format!(
                "- [{}] (seen={}, +{}, -{}) {}",
                b.id, b.seen, b.helpful, b.harmful, b.content
            ));
        }
    }

    // Zero-hit queries from miner filtered by target project
    let target_project = if scope == "global" { None } else { Some(scope) };
    let gaps = crate::miner::scan_ledger_gaps(ledger_path, target_project);
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
    let llm_response =
        match chat_completion(&llm_cfg, &model, system_prompt, &user_prompt, Some(&schema)) {
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

    let total_proposals =
        merges_count + contradictions_count + adr_drifts_count + alias_proposals_count;
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
        format!(
            "> Generated by OmniRoute-backed intelligence engine (`{}`).",
            model
        ),
        "> Validated by deterministic ACE gate. Review before applying.".to_string(),
        "".to_string(),
    ];

    if !validated_merges.is_empty() {
        lines.push("## Paraphrase Merges".to_string());
        for m in &validated_merges {
            lines.push(format!(
                "- [ ] Target `[{}]`: {} (Rationale: {})",
                m.target_id, m.merged_content, m.rationale
            ));
        }
        lines.push("".to_string());
    }

    if !validated_contradictions.is_empty() {
        lines.push("## Contradiction Flags".to_string());
        for c in &validated_contradictions {
            lines.push(format!(
                "- [ ] Potential conflict between `[{}]` and `[{}]`: {}",
                c.bullet_a, c.bullet_b, c.issue
            ));
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
            lines.push(format!(
                "- [ ] Topic `{}` -> note `{}` (aliases: {:?})",
                a.cluster_topic, a.suggested_note, a.aliases
            ));
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

// ---------------------------------------------------------------- QUOR-05 conflict judge (mirror of curate.py)
pub const MAX_VERDICTS: usize = 10;
pub const MAX_REASON_CHARS: usize = 500;
pub const MAX_DIFF_CHARS: usize = 4000;
const CONFLICT_SYSTEM: &str = "You judge contradicting lessons in an AI agents' playbook. For each pair, say which lesson is right now (its id), or 'both' when they apply in different situations. Cite only anchors or file paths that appear in the input. Conform strictly to the JSON schema.";

#[derive(Debug, Clone, Default, Serialize)]
pub struct ConflictResult {
    pub model: String,
    pub conflicts: usize,
    pub verdicts: usize,
    pub proposal_file: Option<String>,
    pub dry_run: bool,
    pub skipped: bool,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Verdict {
    pub bullet_a: String,
    pub bullet_b: String,
    pub winner: String,
    pub reason: String,
    pub citations: Vec<String>,
}

fn conflict_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {"verdicts": {"type": "array", "items": {
            "type": "object",
            "properties": {"bullet_a": {"type": "string"}, "bullet_b": {"type": "string"},
                           "winner": {"type": "string"}, "reason": {"type": "string"},
                           "citations": {"type": "array", "items": {"type": "string"}}},
            "required": ["bullet_a", "bullet_b", "winner", "reason", "citations"]}}},
        "required": ["verdicts"]
    })
}

/// Recent history of the anchor paths in the scope's repo (fail-open: "" for global or no repo).
pub fn git_diff(projects_root: &Path, scope: &str, paths: &[String]) -> String {
    let files: Vec<&String> = paths
        .iter()
        .filter(|p| p.contains('/') || p.contains('.'))
        .collect();
    if scope == "global" || files.is_empty() {
        return String::new();
    }
    let repo = fs::read_dir(projects_root).ok().and_then(|rd| {
        rd.filter_map(Result::ok).map(|e| e.path()).find(|p| {
            p.is_dir()
                && crate::common::slugify(&p.file_name().unwrap_or_default().to_string_lossy())
                    == scope
        })
    });
    let Some(repo) = repo else {
        return String::new();
    };
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(&repo)
        .args(["log", "-p", "-n", "5", "--format=%h %s", "--"])
        .args(files)
        .output();
    match out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout)
            .chars()
            .take(MAX_DIFF_CHARS)
            .collect(),
        _ => String::new(),
    }
}

/// Deterministic gate: known pair, winner in the pair or "both", a short secret-free reason, and >= 1 citation,
/// every one of which appears in the anchors or diff the model was shown.
pub fn validate_verdicts(
    payload: &serde_json::Value,
    pairs: &[crate::quorum::Conflict],
    corpus: &str,
) -> Vec<Verdict> {
    let known: HashSet<(String, String)> = pairs
        .iter()
        .flat_map(|(a, b, _)| [(a.clone(), b.clone()), (b.clone(), a.clone())])
        .collect();
    let mut out = Vec::new();
    for v in payload
        .get("verdicts")
        .and_then(|x| x.as_array())
        .into_iter()
        .flatten()
    {
        let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
        // ids as the model may echo them: "[pit-00012]" or "`pit-00012`" -> "pit-00012"
        let id = |k: &str| {
            s(k).trim()
                .trim_matches(|c| "[]`'\" ".contains(c))
                .to_string()
        };
        let (a, b, winner) = (id("bullet_a"), id("bullet_b"), id("winner"));
        let reason = s("reason").split_whitespace().collect::<Vec<_>>().join(" ");
        let cites: Vec<String> = v
            .get("citations")
            .and_then(|x| x.as_array())
            .map(|c| {
                c.iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let ok = known.contains(&(a.clone(), b.clone()))
            && (winner == a || winner == b || winner == "both")
            && !reason.is_empty()
            && reason.chars().count() <= MAX_REASON_CHARS
            && !is_secret(&reason)
            && !cites.is_empty()
            && cites
                .iter()
                .all(|c| !c.is_empty() && corpus.contains(&c.to_lowercase()));
        if ok {
            out.push(Verdict {
                bullet_a: a,
                bullet_b: b,
                winner,
                reason,
                citations: cites,
            });
            if out.len() >= MAX_VERDICTS {
                break;
            }
        }
    }
    out
}

type Complete<'a> = &'a dyn Fn(&str, &str, &str, &serde_json::Value) -> Result<String, String>;

/// QUOR-05: send each detected conflict (both claims + recent git history) to OmniRoute and write a cited
/// proposal to inbox/ only. Never changes playbook lines or quorum states; a human applies it.
pub fn run_curate_conflicts(
    vault_path: &Path,
    project: Option<&str>,
    dry_run: bool,
    config_path: Option<&Path>,
    complete: Option<Complete>,
) -> Result<ConflictResult, String> {
    let llm_cfg = load_llm_config(config_path);
    let scope = crate::common::slugify(project.unwrap_or("global"));
    let model = resolve_combo(&llm_cfg, "deep", Some(&scope)); // private projects stay on hive-private
    let mut res = ConflictResult {
        model: model.clone(),
        dry_run,
        skipped: true,
        ..Default::default()
    };
    if !llm_cfg.enabled {
        res.model = "disabled".into();
        res.reason = Some("LLM disabled in configuration".into());
        return Ok(res);
    }
    let sections = load(vault_path, &scope)?;
    let meta = crate::quorum::load(&crate::quorum::meta_path(&crate::ace::scope_path(
        vault_path, &scope,
    )?));
    let bullets: Vec<(String, String)> = sections
        .values()
        .flatten()
        .map(|b| (b.id.clone(), b.content.clone()))
        .collect();
    let content: std::collections::HashMap<&str, &str> = bullets
        .iter()
        .map(|(i, c)| (i.as_str(), c.as_str()))
        .collect();
    let pairs = crate::quorum::conflicts(&bullets);
    res.conflicts = pairs.len();
    if pairs.is_empty() {
        res.reason = Some("no conflicts in this playbook".into());
        return Ok(res);
    }
    let anchors: Vec<String> = pairs
        .iter()
        .flat_map(|(_, _, s)| s.iter().cloned())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    let history = git_diff(&crate::common::default_projects(), &scope, &anchors);
    let mut lines = vec![format!("Scope: {scope}"), String::new()];
    for (a, b, shared) in &pairs {
        for i in [a, b] {
            let m = meta.get(i).cloned();
            let score = m.as_ref().map(crate::quorum::score).unwrap_or(0.0);
            lines.push(format!(
                "[{i}] ({}, score {score}) {}",
                crate::quorum::state_of(&meta, i),
                content[i.as_str()]
            ));
        }
        lines.push(format!("shared anchors: {}", shared.join(", ")));
        lines.push(String::new());
    }
    lines.push("Recent git history of the anchors:".into());
    lines.push(if history.is_empty() {
        "(none)".into()
    } else {
        history.clone()
    });
    let schema = conflict_schema();
    let user = lines.join("\n");
    let raw = match complete {
        Some(f) => f(&model, CONFLICT_SYSTEM, &user, &schema),
        None => chat_completion(&llm_cfg, &model, CONFLICT_SYSTEM, &user, Some(&schema)),
    };
    let payload: serde_json::Value =
        match raw.and_then(|r| serde_json::from_str(&r).map_err(|e| e.to_string())) {
            Ok(p) => p,
            Err(e) => {
                res.reason = Some(format!("OmniRoute call failed (fail-open): {e}"));
                return Ok(res);
            }
        };
    let corpus = format!("{}\n{}", anchors.join(" "), history).to_lowercase();
    let verdicts = validate_verdicts(&payload, &pairs, &corpus);
    res.skipped = false;
    res.verdicts = verdicts.len();
    if verdicts.is_empty() {
        res.reason = Some("no verdict passed deterministic validation".into());
        return Ok(res);
    }
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let mut body = vec![
        format!("# Conflict verdicts: {scope} ({today})"),
        String::new(),
        format!("> Proposed by `{model}` via `hive curate --conflicts` (QUOR-05). Nothing was applied: review, then"),
        "> tag the winner helpful with `hive-ace` or leave it; playbook lines and quorum states are unchanged.".into(),
        String::new(),
        "## Verdicts".into(),
    ];
    for v in &verdicts {
        body.push(format!(
            "- [ ] `{}` vs `{}`: winner `{}`. {} (cites: {})",
            v.bullet_a,
            v.bullet_b,
            v.winner,
            v.reason,
            v.citations.join(", ")
        ));
    }
    let body_text = body.join("\n") + "\n";
    let project_field = if scope == "global" {
        "all".to_string()
    } else {
        scope.clone()
    };
    let head = [
        "---".to_string(),
        "type: note".into(),
        format!("project: {project_field}"),
        "agent: claude".into(),
        "status: active".into(),
        format!("updated: {today}"),
        format!("source: hive curate --conflicts ({model})"),
        "---".into(),
    ];
    let name = format!(
        "conflicts-{scope}-{today}-{}.md",
        &crate::common::sha256(body_text.as_bytes())[..8]
    );
    let proposal = vault_path.join("inbox").join(name);
    if !dry_run {
        fs::create_dir_all(vault_path.join("inbox")).map_err(|e| e.to_string())?;
        fs::write(&proposal, head.join("\n") + "\n" + &body_text).map_err(|e| e.to_string())?;
    }
    res.proposal_file = Some(proposal.to_string_lossy().to_string());
    Ok(res)
}

#[cfg(test)]
mod conflict_tests {
    use super::*;

    #[test]
    fn test_conflict_judge_gates_and_writes_inbox_only() {
        let vault = std::env::temp_dir().join(format!("hive-judge-{}", std::process::id()));
        let _ = fs::remove_dir_all(&vault);
        fs::create_dir_all(vault.join("_meta")).unwrap();
        let pb = "## pitfalls\n- [pit-00001] seen=3 helpful=3 harmful=0 :: cargo build --release fails with Access is denied on target/release/hive.exe; use cargo check until sessions close.\n## commands\n- [cmd-00002] seen=1 helpful=1 harmful=0 :: rebuild anyway: rename target/release/hive.exe, then cargo build --release\n";
        fs::write(vault.join("_meta").join("playbook.md"), pb).unwrap();
        let reply = r#"{"verdicts":[
            {"bullet_a":"[pit-00001]","bullet_b":"`cmd-00002`","winner":"[cmd-00002]","reason":"Renaming a locked exe works.","citations":["target/release/hive.exe"]},
            {"bullet_a":"pit-00001","bullet_b":"cmd-00002","winner":"str-00003","reason":"x","citations":["--release"]},
            {"bullet_a":"pit-00001","bullet_b":"cmd-00002","winner":"both","reason":"y","citations":["src/invented.rs"]}]}"#;
        let fake =
            |_: &str, _: &str, user: &str, _: &serde_json::Value| -> Result<String, String> {
                assert!(user.contains("pit-00001"));
                Ok(reply.to_string())
            };
        let r = run_curate_conflicts(&vault, None, false, None, Some(&fake)).unwrap();
        if r.model == "disabled" {
            let _ = fs::remove_dir_all(&vault);
            return; // LLM disabled on this machine's config: nothing to judge
        }
        assert_eq!((r.conflicts, r.verdicts, r.skipped), (1, 1, false));
        let text = fs::read_to_string(r.proposal_file.unwrap()).unwrap();
        assert!(text.contains("- [ ] `pit-00001` vs `cmd-00002`: winner `cmd-00002`."));
        assert!(!text.contains("rebuild anyway"));
        assert_eq!(
            fs::read_to_string(vault.join("_meta").join("playbook.md")).unwrap(),
            pb
        );
        let _ = fs::remove_dir_all(&vault);
    }
}
