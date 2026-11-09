use supernova_db::SupernovaDb;
use serde_json::json;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn advanced_database_supports_transactions_and_queries() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = format!("/tmp/supernova_db_advance_{unique}");

    let mut db = SupernovaDb::open(&dir).unwrap();
    db.create_table("users").unwrap();
    db.set("users", "alice".to_string(), json!({"name": "Alice", "role": "admin"})).unwrap();

    let mut tx = db.begin_transaction().unwrap();
    tx.set("users", "bob".to_string(), json!({"name": "Bob", "role": "user"})).unwrap();
    tx.commit().unwrap();

    let rows = db.query("users", "role=admin");
    assert!(!rows.is_empty());

    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn tables_and_list_operations_work() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = format!("/tmp/supernova_db_tables_{unique}");

    let mut db = SupernovaDb::open(&dir).unwrap();
    db.create_table("settings").unwrap();
    db.set("settings", "theme".to_string(), json!("dark")).unwrap();

    assert!(db.table_names().contains(&"settings".to_string()));
    assert_eq!(db.get("settings", "theme"), Some(json!("dark")));

    fs::remove_dir_all(dir).unwrap();
}

