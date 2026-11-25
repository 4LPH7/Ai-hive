//! Quorum states for playbook bullets (v2.7.0, QUOR-01 + PROV-04); mirror of `quorum.py`.
//! State and origin live in a sidecar `playbook.meta.jsonl` next to each `playbook.md`, one compact JSON line
//! per bullet id ever issued. The playbook line format is unchanged. A bullet with no entry is `canonical`
//! (zero-rewrite migration). Pruned bullets stay as `retired`, so their ids are never reissued.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const STATES: [&str; 4] = ["scout", "quorum", "canonical", "retired"];
pub const DEFAULT_STATE: &str = "canonical";

/// Field order is the wire order: byte-identical to `quorum.dumps`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Meta {
    pub id: String,
    pub state: String,
    #[serde(default)]
    pub origin_session: String,
    #[serde(default)]
    pub origin_agent: String,
}

pub type MetaMap = BTreeMap<String, Meta>;

pub fn meta_path(playbook: &Path) -> PathBuf {
    let stem = playbook.file_stem().unwrap_or_default().to_string_lossy();
    playbook.with_file_name(format!("{stem}.meta.jsonl"))
}

/// Unreadable lines and unknown states are skipped (fail-open).
pub fn load(path: &Path) -> MetaMap {
    let Ok(text) = std::fs::read_to_string(path) else {
        return MetaMap::new();
    };
    text.lines()
        .filter_map(|l| serde_json::from_str::<Meta>(l).ok())
        .filter(|m| STATES.contains(&m.state.as_str()))
        .map(|m| (m.id.clone(), m))
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
    meta.get(id).map(|m| m.state.as_str()).unwrap_or(DEFAULT_STATE)
}

/// Added ids become scouts with origin; pruned ids become retired; others untouched.
pub fn reconcile(meta: &MetaMap, added: &[String], pruned: &[String], session: &str, agent: &str) -> MetaMap {
    let mut out = meta.clone();
    for id in added {
        out.insert(
            id.clone(),
            Meta { id: id.clone(), state: "scout".into(), origin_session: session.into(), origin_agent: agent.into() },
        );
    }
    for id in pruned {
        let mut m = out.get(id).cloned().unwrap_or(Meta {
            id: id.clone(),
            state: DEFAULT_STATE.into(),
            origin_session: String::new(),
            origin_agent: String::new(),
        });
        m.state = "retired".into();
        out.insert(id.clone(), m);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_serialization_matches_python_mirror() {
        let m = Meta { id: "pit-00001".into(), state: "scout".into(), origin_session: "s".into(), origin_agent: "a".into() };
        assert_eq!(
            serde_json::to_string(&m).unwrap(),
            r#"{"id":"pit-00001","state":"scout","origin_session":"s","origin_agent":"a"}"#
        );
    }

    #[test]
    fn test_apply_with_lock_records_scout_then_retired_without_id_reuse() {
        let vault = std::env::temp_dir().join(format!("hive-quorum-apply-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&vault);
        let add = |c: &str| crate::ace::parse_delta(&format!(r#"{{"add":[{{"section":"pitfalls","content":"{c}"}}]}}"#)).unwrap();
        crate::ace::apply_with_lock(&vault, "demo", &add("first scout lesson about tokenizer")).unwrap();
        let mf = meta_path(&crate::ace::scope_path(&vault, "demo").unwrap());
        let m = load(&mf);
        let (id, rec) = m.iter().next().unwrap();
        assert_eq!(rec.state, "scout");
        let id = id.clone();
        let tag = crate::ace::parse_delta(&format!(r#"{{"tag":[{{"id":"{id}","harmful":9}}]}}"#)).unwrap();
        crate::ace::apply_with_lock(&vault, "demo", &tag).unwrap();
        assert_eq!(state_of(&load(&mf), &id), "retired");
        crate::ace::apply_with_lock(&vault, "demo", &add("another unrelated graph cache rule")).unwrap();
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
        let r = reconcile(&m, &["pit-00004".into()], &["pit-00003".into()], "s1", "claude");
        assert_eq!((state_of(&r, "pit-00004"), state_of(&r, "pit-00003")), ("scout", "retired"));
        assert_eq!(meta_path(Path::new("x/playbook.md")), Path::new("x/playbook.meta.jsonl"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
