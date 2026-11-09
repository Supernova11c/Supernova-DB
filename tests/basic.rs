use supernova_db::SupernovaDb;
use serde_json::json;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

fn test_dir(label: &str) -> std::path::PathBuf {
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    std::env::temp_dir().join(format!("supernova_{label}_{nonce}"))
}

#[test]
fn tables_transactions_indexes_and_recovery_work() {
    let path = test_dir("integration");
    {
        let mut db = SupernovaDb::open(&path).unwrap();
        db.create_table("users").unwrap();
        db.create_index("users", "role").unwrap();
        let mut tx = db.begin_transaction();
        tx.set("users", "alice".into(), json!({"role":"admin","name":"Alice"})).unwrap();
        tx.set("users", "bob".into(), json!({"role":"user","name":"Bob"})).unwrap();
        tx.commit().unwrap();
        assert_eq!(db.query_eq("users", "role", &json!("admin")).len(), 1);
    }
    let db = SupernovaDb::open(&path).unwrap();
    assert_eq!(db.get("users", "alice"), Some(json!({"role":"admin","name":"Alice"})));
    assert_eq!(db.query("users", "role=admin").len(), 1);
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn rollback_and_checkpoint_are_safe() {
    let path = test_dir("rollback");
    let mut db = SupernovaDb::open(&path).unwrap();
    db.create_table("items").unwrap();
    let mut tx = db.begin_transaction();
    tx.set("items", "uncommitted".into(), json!(42)).unwrap();
    tx.rollback();
    assert_eq!(db.get("items", "uncommitted"), None);
    db.set("items", "saved".into(), json!(true)).unwrap();
    db.checkpoint().unwrap();
    drop(db);
    assert_eq!(SupernovaDb::open(&path).unwrap().get("items", "saved"), Some(json!(true)));
    fs::remove_dir_all(path).unwrap();
}
