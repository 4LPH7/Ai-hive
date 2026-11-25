//! Quorum states for playbook bullets (v2.7.0, QUOR-01 + PROV-04); mirror of `quorum.py`.
//! State and origin live in a sidecar `playbook.meta.jsonl` next to each `playbook.md`, one compact JSON line
//! per bullet id ever issued. The playbook line format is unchanged. A bullet with no entry is `canonical`
//! (zero-rewrite migration). Pruned bullets stay as `retired`, so their ids are never reissued.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const STATES: [&str; 4] = ["scout", "quorum", "canonical", "retired"];
pub const DEFAULT_STATE: &str = "canonical";

/// QUOR-02: per-session weight; a session counts once, at its strongest kind (same-session repeat adds 0).
pub const WEIGHTS: [(&str, f64); 3] = [("test_pass", 1.0), ("confirm", 1.0), ("self_tag", 0.35)];

/// QUOR-03: canonical at score >= PROMOTE_SCORE from >= PROMOTE_SESSIONS sessions; >= 2 sessions below it = quorum.
pub const PROMOTE_SCORE: f64 = 1.5;
pub const PROMOTE_SESSIONS: usize = 2;
pub const INJECT_QUORUM_ENV: &str = "HIVE_INJECT_QUORUM";
pub const ANONYMOUS: &str = "anonymous";

/// Never demotes; retired and canonical stay.
pub fn promote(rec: &mut Meta) {
    if !matches!(rec.state.as_str(), "scout" | "quorum") || sessions(rec).len() < PROMOTE_SESSIONS {
        return;
    }
    rec.state = if score(rec) >= PROMOTE_SCORE {
        "canonical"
    } else {
        "quorum"
    }
    .into();
}

/// Only canonical bullets reach agents; quorum too when HIVE_INJECT_QUORUM=1. Scouts never, so the
/// confirmations that promote them stay independent.
pub fn injectable(state: &str) -> bool {
    state == "canonical"
        || (state == "quorum"
            && matches!(
                std::env::var(INJECT_QUORUM_ENV).as_deref(),
                Ok("1") | Ok("true")
            ))
}

fn weight(kind: &str) -> Option<f64> {
    WEIGHTS.iter().find(|(k, _)| *k == kind).map(|(_, w)| *w)
}

/// Field order is the wire order: byte-identical to `quorum.dumps`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Evidence {
    pub session: String,
    #[serde(default)]
    pub agent: String,
    pub kind: String,
}

/// Field order is the wire order: byte-identical to `quorum.dumps`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Meta {
    pub id: String,
    pub state: String,
    #[serde(default)]
    pub origin_session: String,
    #[serde(default)]
    pub origin_agent: String,
    #[serde(default)]
    pub evidence: Vec<Evidence>,
}

impl Meta {
    fn legacy(id: &str) -> Self {
        Meta {
            id: id.into(),
            state: DEFAULT_STATE.into(),
            origin_session: String::new(),
            origin_agent: String::new(),
            evidence: Vec::new(),
        }
    }
}

/// A helpful TAG turned into evidence: `hint` is `test_pass` from `ledger score`, else empty.
#[derive(Debug, Clone)]
pub struct Tag {
    pub id: String,
    pub session: String,
    pub agent: String,
    pub hint: String,
}

pub type MetaMap = BTreeMap<String, Meta>;

pub fn meta_path(playbook: &Path) -> PathBuf {
    let stem = playbook.file_stem().unwrap_or_default().to_string_lossy();
    playbook.with_file_name(format!("{stem}.meta.jsonl"))
}

/// Unreadable lines and unknown states are skipped; unknown evidence kinds dropped (fail-open).
pub fn load(path: &Path) -> MetaMap {
    let Ok(text) = std::fs::read_to_string(path) else {
        return MetaMap::new();
    };
    text.lines()
        .filter_map(|l| serde_json::from_str::<Meta>(l).ok())
        .filter(|m| STATES.contains(&m.state.as_str()))
        .map(|mut m| {
            m.evidence
                .retain(|e| weight(&e.kind).is_some() && !e.session.is_empty());
            (m.id.clone(), m)
        })
        .collect()
}

pub fn save(path: &Path, meta: &MetaMap) -> Result<(), String> {
    let mut out = String::new();
    for m in meta.values() {
        out.push_str(&serde_json::to_string(m).map_err(|e| e.to_string())?);
        out.push('\n');
    }
    let tmp = path.with_extension("jsonl.tmp");
    std::fs::write(&tmp, out).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

pub fn state_of<'a>(meta: &'a MetaMap, id: &str) -> &'a str {
    meta.get(id)
        .map(|m| m.state.as_str())
        .unwrap_or(DEFAULT_STATE)
}

/// Added ids become scouts with origin; pruned ids become retired; others untouched.
pub fn reconcile(
    meta: &MetaMap,
    added: &[String],
    pruned: &[String],
    session: &str,
    agent: &str,
) -> MetaMap {
    let mut out = meta.clone();
    for id in added {
        out.insert(
            id.clone(),
            Meta {
                id: id.clone(),
                state: "scout".into(),
                origin_session: session.into(),
                origin_agent: agent.into(),
                evidence: Vec::new(),
            },
        );
    }
    for id in pruned {
        let mut m = out.get(id).cloned().unwrap_or_else(|| Meta::legacy(id));
        m.state = "retired".into();
        out.insert(id.clone(), m);
    }
    out
}

/// QUOR-02: record helpful tags. `test_pass` keeps its kind; otherwise the origin session tagging its own
/// bullet is `self_tag`, any other session `confirm`. A (session, kind) pair is recorded once.
pub fn add_evidence(meta: &MetaMap, tags: &[Tag]) -> MetaMap {
    let mut out = meta.clone();
    for t in tags {
        let rec = out
            .entry(t.id.clone())
            .or_insert_with(|| Meta::legacy(&t.id));
        // anonymous per-process id: changes every CLI call, so it proves no independent session.
        // All of it shares one bucket and earns self-tag weight at most.
        let anonymous = crate::ledger::is_anonymous(&t.session);
        let session = if anonymous {
            ANONYMOUS.to_string()
        } else {
            t.session.clone()
        };
        let kind = if anonymous {
            "self_tag"
        } else if t.hint == "test_pass" {
            "test_pass"
        } else if t.session == rec.origin_session {
            "self_tag"
        } else {
            "confirm"
        };
        if !rec
            .evidence
            .iter()
            .any(|e| e.session == session && e.kind == kind)
        {
            rec.evidence.push(Evidence {
                session: session.clone(),
                agent: t.agent.clone(),
                kind: kind.into(),
            });
        }
        promote(rec);
    }
    out
}

pub fn sessions(rec: &Meta) -> std::collections::BTreeSet<String> {
    rec.evidence.iter().map(|e| e.session.clone()).collect()
}

/// Sum over sessions of each session's strongest evidence weight.
pub fn score(rec: &Meta) -> f64 {
    let mut best: BTreeMap<&str, f64> = BTreeMap::new();
    for e in &rec.evidence {
        if let Some(w) = weight(&e.kind) {
            let slot = best.entry(e.session.as_str()).or_insert(0.0);
            *slot = slot.max(w);
        }
    }
    (best.values().sum::<f64>() * 1e6).round() / 1e6
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_serialization_matches_python_mirror() {
        let mut m = Meta {
            id: "pit-00001".into(),
            state: "scout".into(),
            origin_session: "s".into(),
            origin_agent: "a".into(),
            evidence: Vec::new(),
        };
        assert_eq!(
            serde_json::to_string(&m).unwrap(),
            r#"{"id":"pit-00001","state":"scout","origin_session":"s","origin_agent":"a","evidence":[]}"#
        );
        m.evidence.push(Evidence {
            session: "s2".into(),
            agent: "codex".into(),
            kind: "confirm".into(),
        });
        assert!(serde_json::to_string(&m)
            .unwrap()
            .contains(r#""evidence":[{"session":"s2","agent":"codex","kind":"confirm"}]"#));
    }

    #[test]
    fn test_evidence_weights_and_same_session_repeat() {
        let meta = reconcile(&MetaMap::new(), &["pit-00001".into()], &[], "sa", "claude");
        let tag = |m: &MetaMap, s: &str, hint: &str| {
            add_evidence(
                m,
                &[Tag {
                    id: "pit-00001".into(),
                    session: s.into(),
                    agent: "claude".into(),
                    hint: hint.into(),
                }],
            )
        };
        let m = tag(&meta, "sa", "");
        assert_eq!(score(&m["pit-00001"]), 0.35);
        let m = tag(&m, "sa", "");
        assert_eq!(score(&m["pit-00001"]), 0.35);
        let m = tag(&m, "sa", "test_pass");
        assert_eq!(score(&m["pit-00001"]), 1.0);
        let m = tag(&m, "sb", "");
        assert_eq!(score(&m["pit-00001"]), 2.0);
        assert_eq!(sessions(&m["pit-00001"]).len(), 2);
        // Python-form TAG {id, helpful: n, session, evidence} must survive parse_delta
        let d = crate::ace::parse_delta(
            r#"{"operations":[{"type":"TAG","id":"x","helpful":2,"session":"s9","evidence":"test_pass"}]}"#,
        )
        .unwrap();
        let t = &d.tag.unwrap()[0];
        assert_eq!(
            (t.helpful, t.session.as_deref(), t.evidence.as_deref()),
            (Some(2), None, None),
            "trusted-only fields are dropped from deltas"
        );
        let native = crate::ace::parse_delta(
            r#"{"tag":[{"id":"x","helpful":1,"session":"fake","evidence":"test_pass"}]}"#,
        )
        .unwrap();
        assert_eq!(native.tag.unwrap()[0].session, None);
        // anonymous proc- sessions share one bucket at self-tag weight
        let anon = tag(&tag(&meta, "proc-1-a", "test_pass"), "proc-2-b", "");
        assert_eq!(
            (score(&anon["pit-00001"]), state_of(&anon, "pit-00001")),
            (0.35, "scout")
        );
        // QUOR-03 promotion: 2 sessions at 2.0 -> canonical; 2 sessions at 1.35 -> quorum; 1 session never
        assert_eq!(state_of(&m, "pit-00001"), "canonical");
        let q = tag(&tag(&meta, "sa", ""), "sc", "");
        assert_eq!(
            (score(&q["pit-00001"]), state_of(&q, "pit-00001")),
            (1.35, "quorum")
        );
        let one = tag(&tag(&meta, "sa", "test_pass"), "sa", "");
        assert_eq!(state_of(&one, "pit-00001"), "scout");
        assert!(injectable("canonical") && !injectable("scout") && !injectable("retired"));
        let legacy = tag(&MetaMap::new(), "sa", "");
        assert_eq!(
            (state_of(&legacy, "pit-00001"), score(&legacy["pit-00001"])),
            ("canonical", 1.0)
        );
    }

    #[test]
    fn test_apply_with_lock_records_scout_then_retired_without_id_reuse() {
        let vault = std::env::temp_dir().join(format!("hive-quorum-apply-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&vault);
        let add = |c: &str| {
            crate::ace::parse_delta(&format!(
                r#"{{"add":[{{"section":"pitfalls","content":"{c}"}}]}}"#
            ))
            .unwrap()
        };
        crate::ace::apply_with_lock(&vault, "demo", &add("first scout lesson about tokenizer"))
            .unwrap();
        let mf = meta_path(&crate::ace::scope_path(&vault, "demo").unwrap());
        let m = load(&mf);
        let (id, rec) = m.iter().next().unwrap();
        assert_eq!(rec.state, "scout");
        let id = id.clone();
        let tag = crate::ace::parse_delta(&format!(r#"{{"tag":[{{"id":"{id}","harmful":9}}]}}"#))
            .unwrap();
        crate::ace::apply_with_lock(&vault, "demo", &tag).unwrap();
        assert_eq!(state_of(&load(&mf), &id), "retired");
        crate::ace::apply_with_lock(&vault, "demo", &add("another unrelated graph cache rule"))
            .unwrap();
        let m = load(&mf);
        assert_eq!(m.len(), 2, "second ADD got a fresh id: {m:?}");
        let _ = std::fs::remove_dir_all(&vault);
    }

    #[test]
    fn test_load_skips_bad_lines_and_defaults_canonical() {
        let dir = std::env::temp_dir().join(format!("hive-quorum-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("playbook.meta.jsonl");
        std::fs::write(&p, "not json\n{\"id\":\"pit-00002\",\"state\":\"bogus\"}\n{\"id\":\"pit-00003\",\"state\":\"quorum\"}\n").unwrap();
        let m = load(&p);
        assert_eq!(m.keys().collect::<Vec<_>>(), vec!["pit-00003"]);
        assert_eq!(state_of(&m, "pit-00009"), "canonical");
        let r = reconcile(
            &m,
            &["pit-00004".into()],
            &["pit-00003".into()],
            "s1",
            "claude",
        );
        assert_eq!(
            (state_of(&r, "pit-00004"), state_of(&r, "pit-00003")),
            ("scout", "retired")
        );
        assert_eq!(
            meta_path(Path::new("x/playbook.md")),
            Path::new("x/playbook.meta.jsonl")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
