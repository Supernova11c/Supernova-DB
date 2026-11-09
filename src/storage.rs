use serde_json::json;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

use supernova_db::SupernovaDb;

fn temp_dir(prefix: &str) -> std::path::PathBuf {
    let id = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    std::env::temp_dir().join(format!("supernova_{prefix}_{id}"))
}

#[test]
fn schema_validation_and_query_ops_work() {
    let path = temp_dir("query_ops");
    {
        let mut db = SupernovaDb::open(&path).unwrap();
        db.create_table_with_schema("users", &["name".into(), "role".into(), "age".into()]).unwrap();
        db.set("users", "alice".to_string(), json!({"name":"Alice","role":"admin","age":30})).unwrap();
        db.set("users", "bob".to_string(), json!({"name":"Bob","role":"user","age":25})).unwrap();

        let admin_rows = db.query("users", "role=admin");
        assert_eq!(admin_rows.len(), 1);

        let older_rows = db.query("users", "age>=25");
        assert_eq!(older_rows.len(), 2);

        let name_rows = db.query("users", "name~ali");
        assert_eq!(name_rows.len(), 1);
    }
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn backup_restore_and_checkpoint_are_stable() {
    let path = temp_dir("backup_restore");
    let backup_dir = temp_dir("backup");

    {
        let mut db = SupernovaDb::open(&path).unwrap();
        db.create_table("items").unwrap();
        db.set("items", "one".to_string(), json!(1)).unwrap();
        db.checkpoint().unwrap();
        db.backup(&backup_dir).unwrap();
    }

    let mut db = SupernovaDb::open(&path).unwrap();
    db.restore(&backup_dir).unwrap();
    assert_eq!(db.get("items", "one"), Some(json!(1)));

    fs::remove_dir_all(path).unwrap();
    fs::remove_dir_all(backup_dir).unwrap();
}
