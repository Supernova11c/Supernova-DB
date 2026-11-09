use serde_json::json;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};
use supernova_db::SupernovaDb;
fn temp()->std::path::PathBuf{std::env::temp_dir().join(format!("sdb_integration_{}",SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()))}
#[test]
fn hybrid_starter_modes_and_persistence_work(){let path=temp();{let mut db=SupernovaDb::open(&path).unwrap();db.create_collection("notes").unwrap();db.create_text_index("notes","body").unwrap();db.create_vector_index("notes","embedding",2).unwrap();db.put("notes","a",json!({"body":"Rust database","embedding":[1.0,0.0],"rank":2})).unwrap();db.put("notes","b",json!({"body":"Vector search","embedding":[0.0,1.0],"rank":1})).unwrap();assert_eq!(db.search_text("notes","body","RUST",5).unwrap()[0].0,"a");assert_eq!(db.search_vector("notes","embedding",&[0.9,0.1],1).unwrap()[0].0,"a");assert_eq!(db.sql("SELECT * FROM notes WHERE rank = 2 LIMIT 1").unwrap().len(),1);let before=db.get("notes","a");assert!(db.transaction(|tx|{tx.put("notes","bad",json!({"embedding":[1.0]}))?;Ok(())}).is_err());assert_eq!(db.get("notes","a"),before);}let db=SupernovaDb::open(&path).unwrap();assert_eq!(db.search_text("notes","body","database",5).unwrap().len(),1);fs::remove_dir_all(path).unwrap();}
