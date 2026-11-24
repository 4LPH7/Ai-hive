use ai_hive::common::{active_work_cell_matches, validate_slug};
use ai_hive::packer::{pack, Section};
use ai_hive::ace::is_secret;

#[test]
fn test_milestone1_regression_f3_path_traversal() {
    assert!(validate_slug("ai-hive").is_ok());
    assert!(validate_slug("../secret").is_err());
    assert!(validate_slug("../../etc/passwd").is_err());
    assert!(validate_slug(".").is_err());
    assert!(validate_slug("..").is_err());
}

#[test]
fn test_milestone1_regression_f1_f15_packer() {
    let s1 = Section { name: "memory", text: "Alpha memory", weight: 0.35 };
    let s2 = Section { name: "playbook", text: "Beta playbook", weight: 0.15 };
    let s3 = Section { name: "task_notes", text: "Gamma notes", weight: 0.25 };
    let s4 = Section { name: "code_graph", text: "Delta graph", weight: 0.25 };

    let packed = pack(&[s1, s2, s3, s4], 1500);
    assert!(packed.contains("Alpha memory"));
    assert!(packed.contains("Beta playbook"));
    assert!(packed.contains("Gamma notes"));
    assert!(packed.contains("Delta graph"));
}

#[test]
fn test_milestone1_regression_f11_exact_active_work() {
    assert!(active_work_cell_matches("[[projects/ai-hive/_memory|ai-hive]]", "ai-hive"));
    assert!(!active_work_cell_matches("[[projects/fpolink-api/_memory|api]]", "fpolink"));
    assert!(active_work_cell_matches("[[projects/fpolink-api/_memory|api]]", "api"));
}

#[test]
fn test_milestone1_regression_f10_secret_detection() {
    assert!(is_secret("sk-ant-api03-01234567890123456789"));
    assert!(is_secret("ghp_012345678901234567890123456789"));
    assert!(!is_secret("no secret here, just normal safe text"));
}

#[test]
fn test_sync_01_hash_verified_refresh() {
    let tmp = std::env::temp_dir().join(format!("hive_sync_test_{}", std::process::id()));
    let vault = tmp.join("vault");
    let proj_dir = vault.join("projects").join("demo");
    std::fs::create_dir_all(&proj_dir).unwrap();

    let note_path = proj_dir.join("test.md");
    std::fs::write(&note_path, "---\ntype: note\nproject: demo\n---\n# Original\n\nInitial content\n").unwrap();

    let db_path = tmp.join("hive.sqlite");
    let mut idx = ai_hive::index::Index::new(Some(vault.clone()), Some(db_path)).unwrap();

    // Initial indexing
    let changed = idx.refresh(true).unwrap();
    assert_eq!(changed, 1);
    let hits = idx.search("initial", "demo", 5, "project").unwrap();
    assert_eq!(hits.len(), 1);

    // Mtime touch with identical content -> hash matches, 0 reindexed
    let meta = std::fs::metadata(&note_path).unwrap();
    let mtime = meta.modified().unwrap();
    let later = mtime + std::time::Duration::from_secs(10);
    let file = std::fs::File::options().write(true).open(&note_path).unwrap();
    let _ = file.set_modified(later);
    drop(file);

    let changed2 = idx.refresh(true).unwrap();
    assert_eq!(changed2, 0);

    // Content edit -> hash changes -> reindexed cleanly
    std::fs::write(&note_path, "---\ntype: note\nproject: demo\n---\n# Changed\n\nNew flamingo content\n").unwrap();
    let changed3 = idx.refresh(true).unwrap();
    assert_eq!(changed3, 1);
    let hits2 = idx.search("flamingo", "demo", 5, "project").unwrap();
    assert_eq!(hits2.len(), 1);

    // Clean up
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_rust_concurrent_writers_no_lost_updates() {
    let tmp = std::env::temp_dir().join(format!("hive_concurrency_test_{}", std::process::id()));
    let vault = tmp.join("vault");
    let scope = "concurrency-test";

    let v1 = vault.clone();
    let t1 = std::thread::spawn(move || {
        for i in 0..25 {
            let ops = ai_hive::ace::DeltaOps {
                add: Some(vec![ai_hive::ace::AddOp {
                    section: "strategies".to_string(),
                    content: format!("Thread A bullet {:02}", i),
                }]),
                tag: None,
                remove: None,
            };
            ai_hive::ace::apply_with_lock(&v1, scope, &ops).unwrap();
        }
    });

    let v2 = vault.clone();
    let t2 = std::thread::spawn(move || {
        for i in 0..25 {
            let ops = ai_hive::ace::DeltaOps {
                add: Some(vec![ai_hive::ace::AddOp {
                    section: "pitfalls".to_string(),
                    content: format!("Thread B bullet {:02}", i),
                }]),
                tag: None,
                remove: None,
            };
            ai_hive::ace::apply_with_lock(&v2, scope, &ops).unwrap();
        }
    });

    t1.join().unwrap();
    t2.join().unwrap();

    let sections = ai_hive::ace::load(&vault, scope).unwrap();
    let total: usize = sections.values().map(|v| v.len()).sum();
    assert_eq!(total, 50, "Expected all 50 bullets to be preserved under concurrent writes");

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_rust_stale_lockfile_recovery_dead_pid() {
    let tmp = std::env::temp_dir().join(format!("hive_stale_lock_test_{}", std::process::id()));
    let path = tmp.join("playbook.md");
    std::fs::create_dir_all(&tmp).unwrap();

    let lock_path = path.with_extension("lock");
    let hostname = ai_hive::ace::get_hostname();
    // Dead PID 9999999 on local host
    std::fs::write(&lock_path, format!("9999999:{}:2026-01-01T00:00:00Z:deadbeef\n", hostname)).unwrap();

    let lock = ai_hive::ace::PlaybookLock::acquire(&path, std::time::Duration::from_millis(1500));
    assert!(lock.is_ok(), "Expected dead-PID stale lock to be stolen");

    drop(lock);
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_rust_foreign_host_lock_refused() {
    let tmp = std::env::temp_dir().join(format!("hive_foreign_lock_test_{}", std::process::id()));
    let path = tmp.join("playbook.md");
    std::fs::create_dir_all(&tmp).unwrap();

    let lock_path = path.with_extension("lock");
    // Foreign hostname
    std::fs::write(&lock_path, "1234:foreign-node-99:2026-01-01T00:00:00Z:beef1234\n").unwrap();

    let lock = ai_hive::ace::PlaybookLock::acquire(&path, std::time::Duration::from_millis(600));
    assert!(lock.is_err(), "Expected foreign host lock to be refused");
    let err = lock.unwrap_err();
    assert!(err.contains("foreign host foreign-node-99"), "Expected foreign host error, got: {}", err);

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_ledger_01_rust_record_and_read() {
    let tmp = std::env::temp_dir().join(format!("hive_ledger_rust_test_{}.jsonl", std::process::id()));

    let ok = ai_hive::ledger::record(
        &tmp,
        "query",
        "demo-project",
        "how does auth work",
        &["projects/demo/auth.md".to_string()],
        &["str-00001".to_string()],
        8.45,
        Some("codex"),
        Some("session-xyz"),
    );
    assert!(ok, "Expected record to succeed");

    let entries = ai_hive::ledger::read_ledger(&tmp, None, None);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].event, "query");
    assert_eq!(entries[0].project, "demo-project");
    assert_eq!(entries[0].agent, "codex");
    assert_eq!(entries[0].session_id, "session-xyz");
    assert_eq!(entries[0].hits, vec!["projects/demo/auth.md"]);
    assert_eq!(entries[0].injected_bullets, vec!["str-00001"]);
    assert_eq!(entries[0].duration_ms, 8.45);

    // Fail open test
    let bad_dir = std::env::temp_dir().join(format!("hive_bad_ledger_dir_{}", std::process::id()));
    std::fs::create_dir_all(&bad_dir).unwrap();
    let fail_ok = ai_hive::ledger::record(
        &bad_dir, // directory instead of file
        "query",
        "fail-open",
        "",
        &[],
        &[],
        0.0,
        None,
        None,
    );
    assert!(!fail_ok, "Expected directory path to return false without panicking");

    let _ = std::fs::remove_file(&tmp);
    let _ = std::fs::remove_dir_all(&bad_dir);
}

#[test]
fn test_skill_01_rust_handle_rpc_hive_skill() {
    let mut idx = ai_hive::index::Index::in_memory().unwrap();

    // 1. Missing task argument -> error
    let bad_req = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "hive_skill",
            "arguments": {}
        }
    });
    let bad_resp = ai_hive::mcp::handle_rpc(&mut idx, &bad_req).unwrap();
    assert!(bad_resp["result"]["isError"].as_bool().unwrap());
    assert!(bad_resp["result"]["content"][0]["text"].as_str().unwrap().contains("missing required argument 'task'"));

    // 2. Local skill match
    let tmp = std::env::temp_dir().join(format!("hive_mcp_skill_test_{}", std::process::id()));
    let skill_dir = tmp.join(".agent").join("skills").join("docker-debugger");
    std::fs::create_dir_all(&skill_dir).unwrap();
    let skill_file = skill_dir.join("SKILL.md");
    std::fs::write(&skill_file, "---\nname: docker-debugger\ndescription: Debug docker container issues and compose failures\ntriggers:\n  - docker error\n---\n# Docker Debugger\nAlways inspect logs first.\n").unwrap();

    let custom_roots = vec![(tmp.join(".agent").join("skills"), "custom".to_string())];
    let skills = ai_hive::skill::scan_skills(Some(&custom_roots));
    let matched = ai_hive::skill::match_skill("we have a docker error in compose", &skills, 0.65);
    assert!(matched.is_match);
    assert_eq!(matched.skill.as_ref().unwrap().name, "docker-debugger");

    let ctx = ai_hive::skill::load_skill_context(std::path::Path::new(&matched.skill.unwrap().path));
    assert!(ctx.contains("Docker Debugger"));
    assert!(ctx.contains("Active Agent Skill: docker-debugger"));

    let _ = std::fs::remove_dir_all(&tmp);
}
