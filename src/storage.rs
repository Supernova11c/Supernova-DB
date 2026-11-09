use serde_json::json;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

use supernova_db::SupernovaDb;

fn temp_dir(prefix: &str) -> std::path::PathBuf {
    let id = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    std::env::temp_dir().join(format!("supernova_{prefix}_{id}"))
}

#[test]
fn schema_validation_and_range_query_support() {
    let path = temp_dir("advance");
    {
        let mut db = SupernovaDb::open(&path).unwrap();
        db.create_table_with_schema("users", &["name".into(), "role".into(), "age".into()]).unwrap();
        db.set("users", "alice".to_string(), json!({"name":"Alice","role":"admin","age":30})).unwrap();
        db.set("users", "bob".to_string(), json!({"name":"Bob","role":"user","age":25})).unwrap();
        db.create_index("users", "role").unwrap();

        let admin_rows = db.query("users", "role=admin");
        assert_eq!(admin_rows.len(), 1);

        let older_rows = db.query("users", "age>=25");
        assert_eq!(older_rows.len(), 2);
    }
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn checkpoint_and_transaction_safe_state() {
    let path = temp_dir("checkpoint");
    let mut db = SupernovaDb::open(&path).unwrap();
    db.create_table("items").unwrap();
    let mut tx = db.begin_transaction();
    tx.set("items", "x".to_string(), json!(42)).unwrap();
    tx.commit().unwrap();
    db.checkpoint().unwrap();
    assert_eq!(db.get("items", "x"), Some(json!(42)));
    fs::remove_dir_all(path).unwrap();
}
