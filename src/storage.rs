use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Collection {
    #[serde(default)] documents: BTreeMap<String, Value>,
    #[serde(default)] text_fields: BTreeSet<String>,
    #[serde(default)] vector_fields: BTreeMap<String, usize>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct DatabaseFile {
    format_version: u32,
    #[serde(default)] collections: BTreeMap<String, Collection>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionInfo {
    pub name: String,
    pub document_count: usize,
    pub text_fields: Vec<String>,
    pub vector_fields: BTreeMap<String, usize>,
}
#[derive(Debug, Clone, Default)]
struct TextIndex(HashMap<String, BTreeMap<String, u32>>);

pub struct SupernovaDb {
    root: PathBuf,
    data: DatabaseFile,
    text_indexes: HashMap<(String, String), TextIndex>,
}

impl SupernovaDb {
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, String> {
        let root = path.into();
        fs::create_dir_all(&root).map_err(error)?;
        let file = root.join("database.json");
        let data = if file.exists() {
            let bytes = fs::read(file).map_err(error)?;
            if bytes.is_empty() { DatabaseFile::default() } else { serde_json::from_slice(&bytes).map_err(error)? }
        } else { DatabaseFile::default() };
        let mut db = Self { root, data, text_indexes: HashMap::new() };
        db.rebuild_text_indexes();
        Ok(db)
    }

    pub fn create_collection(&mut self, name: &str) -> Result<(), String> {
        validate_name(name)?;
        if self.data.collections.contains_key(name) { return Err(format!("collection '{name}' already exists")); }
        self.data.collections.insert(name.into(), Collection::default());
        self.persist_and_reindex()
    }
    pub fn delete_collection(&mut self, name: &str) -> Result<bool, String> {
        let removed = self.data.collections.remove(name).is_some();
        if removed { self.persist_and_reindex()?; }
        Ok(removed)
    }
    pub fn collections(&self) -> Vec<CollectionInfo> {
        self.data.collections.iter().map(|(name, c)| CollectionInfo { name: name.clone(), document_count: c.documents.len(), text_fields: c.text_fields.iter().cloned().collect(), vector_fields: c.vector_fields.clone() }).collect()
    }
    pub fn put(&mut self, collection: &str, id: &str, document: Value) -> Result<(), String> {
        validate_name(collection)?; validate_id(id)?;
        let c = self.data.collections.get(collection).ok_or_else(|| format!("collection '{collection}' does not exist"))?;
        validate_vectors(c, &document)?;
        self.data.collections.get_mut(collection).expect("checked collection").documents.insert(id.into(), document);
        self.persist_and_reindex()
    }
    pub fn get(&self, collection: &str, id: &str) -> Option<Value> { self.data.collections.get(collection)?.documents.get(id).cloned() }
    pub fn delete(&mut self, collection: &str, id: &str) -> Result<Option<Value>, String> {
        let removed = self.data.collections.get_mut(collection).and_then(|c| c.documents.remove(id));
        if removed.is_some() { self.persist_and_reindex()?; }
        Ok(removed)
    }
    pub fn list(&self, collection: &str) -> Vec<(String, Value)> {
        self.data.collections.get(collection).map(|c| c.documents.iter().map(|(id, doc)| (id.clone(), doc.clone())).collect()).unwrap_or_default()
    }
    pub fn create_text_index(&mut self, collection: &str, field: &str) -> Result<(), String> {
        validate_field(field)?;
        self.data.collections.get_mut(collection).ok_or_else(|| format!("collection '{collection}' does not exist"))?.text_fields.insert(field.into());
        self.persist_and_reindex()
    }
    pub fn search_text(&self, collection: &str, field: &str, query: &str, limit: usize) -> Result<Vec<(String, Value, u32)>, String> {
        let index = self.text_indexes.get(&(collection.into(), field.into())).ok_or("text index not found; create it first")?;
        let mut scores = BTreeMap::<String, u32>::new();
        for term in tokenize(query) { if let Some(matches) = index.0.get(&term) { for (id, count) in matches { *scores.entry(id.clone()).or_default() += count; } } }
        let mut hits: Vec<_> = scores.into_iter().filter_map(|(id, score)| self.get(collection, &id).map(|doc| (id, doc, score))).collect();
        hits.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.0.cmp(&b.0)));
        hits.truncate(limit);
        Ok(hits)
    }
    pub fn create_vector_index(&mut self, collection: &str, field: &str, dimensions: usize) -> Result<(), String> {
        validate_field(field)?;
        if dimensions == 0 || dimensions > 65536 { return Err("dimensions must be 1..=65536".into()); }
        let c = self.data.collections.get_mut(collection).ok_or_else(|| format!("collection '{collection}' does not exist"))?;
        for (id, doc) in &c.documents { read_vector(doc, field, dimensions).map_err(|e| format!("document {id}: {e}"))?; }
        c.vector_fields.insert(field.into(), dimensions);
        self.persist_and_reindex()
    }
    /// Exact cosine similarity: straightforward for learning, O(number of documents × dimensions).
    pub fn search_vector(&self, collection: &str, field: &str, query: &[f64], limit: usize) -> Result<Vec<(String, Value, f64)>, String> {
        let c = self.data.collections.get(collection).ok_or_else(|| format!("collection '{collection}' does not exist"))?;
        let dims = *c.vector_fields.get(field).ok_or("vector index not found; create it first")?;
        validate_vector(query, dims)?;
        let mut hits = Vec::new();
        for (id, doc) in &c.documents { let vector = read_vector(doc, field, dims)?; hits.push((id.clone(), doc.clone(), cosine(query, &vector))); }
        hits.sort_by(|a, b| b.2.total_cmp(&a.2).then_with(|| a.0.cmp(&b.0)));
        hits.truncate(limit);
        Ok(hits)
    }
    pub fn sql(&self, statement: &str) -> Result<Vec<(String, Value)>, String> {
        let (collection, predicate, limit) = parse_select(statement)?;
        let mut rows: Vec<_> = self.list(&collection).into_iter().filter(|(_, doc)| predicate.as_ref().is_none_or(|(field, expected)| doc.get(field) == Some(expected))).collect();
        if let Some(limit) = limit { rows.truncate(limit); }
        Ok(rows)
    }
    /// Apply changes to a private copy and persist once; errors leave the live database unchanged.
    pub fn transaction<T>(&mut self, action: impl FnOnce(&mut Transaction) -> Result<T, String>) -> Result<T, String> {
        let mut tx = Transaction { data: self.data.clone() };
        let result = action(&mut tx)?;
        persist_file(&self.root, &tx.data)?;
        self.data = tx.data;
        self.rebuild_text_indexes();
        Ok(result)
    }
    fn persist_and_reindex(&mut self) -> Result<(), String> { persist_file(&self.root, &self.data)?; self.rebuild_text_indexes(); Ok(()) }
    fn rebuild_text_indexes(&mut self) {
        self.text_indexes.clear();
        for (name, c) in &self.data.collections { for field in &c.text_fields {
            let mut index = TextIndex::default();
            for (id, doc) in &c.documents { if let Some(text) = doc.get(field).and_then(Value::as_str) { for term in tokenize(text) { *index.0.entry(term).or_default().entry(id.clone()).or_default() += 1; } } }
            self.text_indexes.insert((name.clone(), field.clone()), index);
        } }
    }
}

pub struct Transaction { data: DatabaseFile }
impl Transaction {
    pub fn put(&mut self, collection: &str, id: &str, document: Value) -> Result<(), String> {
        validate_name(collection)?; validate_id(id)?;
        let c = self.data.collections.get(collection).ok_or_else(|| format!("collection '{collection}' does not exist"))?;
        validate_vectors(c, &document)?;
        self.data.collections.get_mut(collection).expect("checked collection").documents.insert(id.into(), document);
        Ok(())
    }
    pub fn delete(&mut self, collection: &str, id: &str) -> Option<Value> { self.data.collections.get_mut(collection)?.documents.remove(id) }
}

fn persist_file(root: &Path, data: &DatabaseFile) -> Result<(), String> {
    let mut copy = data.clone(); copy.format_version = 1;
    let bytes = serde_json::to_vec_pretty(&copy).map_err(error)?;
    let temp = root.join("database.json.tmp"); let dest = root.join("database.json");
    let mut file = File::create(&temp).map_err(error)?; file.write_all(&bytes).map_err(error)?; file.sync_all().map_err(error)?;
    fs::rename(&temp, &dest).map_err(error)?;
    Ok(())
}
fn error(e: impl std::fmt::Display) -> String { e.to_string() }
fn validate_name(s: &str) -> Result<(), String> { if s.is_empty() || s.len() > 128 || !s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') { Err("names must be 1..=128 ASCII letters, digits, '_' or '-'".into()) } else { Ok(()) } }
fn validate_field(s: &str) -> Result<(), String> { if s.is_empty() || s.len() > 128 { Err("field must be 1..=128 bytes".into()) } else { Ok(()) } }
fn validate_id(s: &str) -> Result<(), String> { if s.is_empty() || s.len() > 1024 { Err("ID must be 1..=1024 bytes".into()) } else { Ok(()) } }
fn tokenize(s: &str) -> Vec<String> { s.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_lowercase).collect() }
fn validate_vector(v: &[f64], dims: usize) -> Result<(), String> { if v.len() != dims { return Err(format!("expected {dims} vector dimensions")); } if v.iter().any(|x| !x.is_finite()) { return Err("vector values must be finite".into()); } Ok(()) }
fn validate_vectors(c: &Collection, doc: &Value) -> Result<(), String> { for (field, dims) in &c.vector_fields { if let Some(value) = doc.get(field) { let a = value.as_array().ok_or_else(|| format!("'{field}' must be an array"))?; let v: Result<Vec<f64>, _> = a.iter().map(|x| x.as_f64().ok_or_else(|| format!("'{field}' must contain numbers"))).collect(); validate_vector(&v?, *dims)?; } } Ok(()) }
fn read_vector(doc: &Value, field: &str, dims: usize) -> Result<Vec<f64>, String> { let a = doc.get(field).and_then(Value::as_array).ok_or_else(|| format!("missing or invalid vector field '{field}'"))?; let v: Result<Vec<_>, _> = a.iter().map(|x| x.as_f64().ok_or_else(|| "vector values must be numbers".to_string())).collect(); let v = v?; validate_vector(&v, dims)?; Ok(v) }
fn cosine(a: &[f64], b: &[f64]) -> f64 { let dot: f64 = a.iter().zip(b).map(|(x,y)| x*y).sum(); let na = a.iter().map(|x| x*x).sum::<f64>().sqrt(); let nb = b.iter().map(|x| x*x).sum::<f64>().sqrt(); if na == 0.0 || nb == 0.0 { 0.0 } else { dot/(na*nb) } }
fn parse_select(sql: &str) -> Result<(String, Option<(String, Value)>, Option<usize>), String> {
    let s = sql.trim().trim_end_matches(';').trim();
    let upper = s.to_ascii_uppercase();
    if !upper.starts_with("SELECT * FROM ") { return Err("supported SQL: SELECT * FROM collection [WHERE field = value] [LIMIT n]".into()); }
    let rest = s[14..].trim(); let up = rest.to_ascii_uppercase();
    let wp = up.find(" WHERE "); let lp = up.find(" LIMIT ");
    let end = [wp,lp].into_iter().flatten().min().unwrap_or(rest.len());
    let collection = rest[..end].trim().to_string(); validate_name(&collection)?;
    let predicate = if let Some(pos) = wp { let stop = lp.filter(|x| *x > pos).unwrap_or(rest.len()); let clause = rest[pos+7..stop].trim(); let (field, raw) = clause.split_once('=').ok_or("WHERE supports equality only")?; let field = field.trim(); if field.is_empty() { return Err("WHERE field cannot be empty".into()); } let expected = serde_json::from_str(raw.trim()).unwrap_or_else(|_| Value::String(raw.trim().trim_matches(['\'', '"']).to_string())); Some((field.to_string(), expected)) } else { None };
    let limit = if let Some(pos) = lp { Some(rest[pos+7..].trim().parse::<usize>().map_err(|_| "LIMIT must be a non-negative integer")?) } else { None };
    Ok((collection,predicate,limit))
}

#[cfg(test)]
mod tests {
    use super::*; use serde_json::json; use std::time::{SystemTime,UNIX_EPOCH};
    fn dir() -> PathBuf { std::env::temp_dir().join(format!("sdb_test_{}",SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos())) }
    #[test] fn document_keyword_vector_sql_transaction_and_reopen() {
        let path=dir(); { let mut db=SupernovaDb::open(&path).unwrap(); db.create_collection("items").unwrap(); db.create_text_index("items","body").unwrap(); db.create_vector_index("items","embedding",2).unwrap();
        db.put("items","a",json!({"body":"Rust database","embedding":[1.0,0.0],"rank":2})).unwrap(); db.put("items","b",json!({"body":"Vector search","embedding":[0.0,1.0],"rank":1})).unwrap();
        assert_eq!(db.search_text("items","body","RUST",5).unwrap()[0].0,"a"); assert_eq!(db.search_vector("items","embedding",&[0.9,0.1],1).unwrap()[0].0,"a"); assert_eq!(db.sql("SELECT * FROM items WHERE rank = 2 LIMIT 1").unwrap().len(),1);
        let before=db.get("items","a"); assert!(db.transaction(|tx| {tx.put("items","bad",json!({"embedding":[1.0]}))?; Ok(())}).is_err()); assert_eq!(db.get("items","a"),before); }
        let db=SupernovaDb::open(&path).unwrap(); assert_eq!(db.search_text("items","body","database",5).unwrap().len(),1); fs::remove_dir_all(path).unwrap();
    }
}
