use serde_json::json;
use supernova_db::SupernovaDb;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Keep this example isolated from the database used by the CLI.
    let path = std::env::temp_dir().join(format!("supernova_quickstart_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);

    let mut db = SupernovaDb::open(&path)?;
    db.create_collection("articles")?;
    db.create_text_index("articles", "body")?;
    db.create_vector_index("articles", "embedding", 3)?;

    db.transaction(|tx| {
        tx.put(
            "articles",
            "rust-db",
            json!({
                "title": "A tiny Rust database",
                "body": "Learn documents, keyword search, and vectors",
                "embedding": [0.1, 0.2, 0.3]
            }),
        )?;
        Ok(())
    })?;

    println!("Document: {:?}", db.get("articles", "rust-db"));
    println!("Keyword matches: {:?}", db.search_text("articles", "body", "keyword", 5)?);
    println!("Vector matches: {:?}", db.search_vector("articles", "embedding", &[0.1, 0.2, 0.25], 5)?);
    println!("SQL subset: {:?}", db.sql("SELECT * FROM articles WHERE title = \"A tiny Rust database\" LIMIT 5")?);

    drop(db);
    std::fs::remove_dir_all(path)?;
    Ok(())
}
