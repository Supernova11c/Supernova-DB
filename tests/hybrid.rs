use serde_json::json;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};
use supernova_db::SupernovaDb;

fn temp() -> std::path::PathBuf { std::env::temp_dir().join(format!("sdb_test_{}", SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos())) }

#[test]
fn hybrid_document_keyword_vector_and_sql_work() {
    let path = temp();
    {
        let mut db = SupernovaDb::open(&path).unwrap();
        db.create_collection("notes").unwrap();
        db.create_text_index("notes", "text").unwrap();
        db.create_vector_index("notes", "embedding", 2).unwrap();
        db.put("notes", "one", json!({"text":"Rust database","embedding":[1.0,0.0],"rank":10})).unwrap();
        db.put("notes", "two", json!({"text":"Vector search","embedding":[0.0,1.0],"rank":4})).unwrap();
        assert_eq!(db.search_text("notes", "text", "RUST", 5).unwrap()[0].0, "one");
        assert_eq!(db.search_vector("notes", "embedding", &[1.0,0.1], 1).unwrap()[0].0, "one");
        assert_eq!(db.sql("SELECT * FROM notes WHERE rank >= 5 LIMIT 1").unwrap().len(), 1);
        let before = db.get("notes", "one");
        assert!(db.transaction(|tx| { tx.put("notes", "bad", json!({"embedding":[1.0]}))?; Ok(()) }).is_err());
        assert_eq!(db.get("notes", "one"), before);
    }
    let db = SupernovaDb::open(&path).unwrap();
    assert_eq!(db.search_text("notes", "text", "database", 5).unwrap().len(), 1);
    fs::remove_dir_all(path).unwrap();
}
