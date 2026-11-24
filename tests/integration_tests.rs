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
