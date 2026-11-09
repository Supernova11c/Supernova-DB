use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct Collection {
    documents: BTreeMap<String, Value>,
    #[serde(default)]
    text_fields: BTreeSet<String>,
    #[serde(default)]
    vector_fields: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct DatabaseFile {
    format_version: u32,
    collections: BTreeMap<String, Collection>,
}

#[derive(Debug, Clone, Serialize)]
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
        fs::create_dir_all(&root).map_err(err)?;
        let data_path = root.join("database.json");
        let data = if data_path.exists() {
            let bytes = fs::read(&data_path).map_err(err)?;
            if bytes.is_empty() { DatabaseFile::default() } else { serde_json::from_slice(&bytes).map_err(err)? }
        } else { DatabaseFile::default() };
        let mut db = Self { root, data, text_indexes: HashMap::new() };
        db.rebuild_text_indexes();
        Ok(db)
    }

    pub fn create_collection(&mut self, name: &str) -> Result<(), String> {
        validate_name(name)?;
        self.data.collections.entry(name.to_string()).or_default();
        self.persist_and_reindex()
    }

    pub fn delete_collection(&mut self, name: &str) -> Result<bool, String> {
        let removed = self.data.collections.remove(name).is_some();
        if removed { self.persist_and_reindex()?; }
        Ok(removed)
    }

    pub fn collections(&self) -> Vec<CollectionInfo> {
        self.data.collections.iter().map(|(name, collection)| CollectionInfo {
            name: name.clone(), document_count: collection.documents.len(),
            text_fields: collection.text_fields.iter().cloned().collect(),
            vector_fields: collection.vector_fields.clone(),
        }).collect()
    }

    pub fn put(&mut self, collection: &str, id: &str, document: Value) -> Result<(), String> {
        validate_name(collection)?;
        validate_id(id)?;
        let current = self.data.collections.get(collection).cloned().unwrap_or_default();
        validate_vectors(&current, &document)?;
        self.data.collections.entry(collection.to_string()).or_default().documents.insert(id.to_string(), document);
        self.persist_and_reindex()
    }

    pub fn get(&self, collection: &str, id: &str) -> Option<Value> {
        self.data.collections.get(collection).and_then(|c| c.documents.get(id)).cloned()
    }

    pub fn delete(&mut self, collection: &str, id: &str) -> Result<Option<Value>, String> {
        let removed = self.data.collections.get_mut(collection).and_then(|c| c.documents.remove(id));
        if removed.is_some() { self.persist_and_reindex()?; }
        Ok(removed)
    }

    pub fn list(&self, collection: &str) -> Vec<(String, Value)> {
        self.data.collections.get(collection).map(|c| c.documents.iter().map(|(id, doc)| (id.clone(), doc.clone())).collect()).unwrap_or_default()
    }

    pub fn create_text_index(&mut self, collection: &str, field: &str) -> Result<(), String> {
        validate_name(collection)?;
        if field.trim().is_empty() || field.len() > 128 { return Err("field must be 1..=128 bytes".into()); }
        self.data.collections.entry(collection.to_string()).or_default().text_fields.insert(field.to_string());
        self.persist_and_reindex()
    }

    pub fn search_text(&self, collection: &str, field: &str, query: &str, limit: usize) -> Result<Vec<(String, Value, u32)>, String> {
        let index = self.text_indexes.get(&(collection.to_string(), field.to_string())).ok_or_else(|| "text index not found; create it first".to_string())?;
        let mut scores: BTreeMap<String, u32> = BTreeMap::new();
        for term in tokenize(query) {
            if let Some(matches) = index.0.get(&term) {
                for (id, count) in matches { *scores.entry(id.clone()).or_default() += count; }
            }
        }
        let mut results: Vec<_> = scores.into_iter().filter_map(|(id, score)| self.get(collection, &id).map(|doc| (id, doc, score))).collect();
        results.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.0.cmp(&b.0)));
        results.truncate(limit);
        Ok(results)
    }

    pub fn create_vector_index(&mut self, collection: &str, field: &str, dimensions: usize) -> Result<(), String> {
        validate_name(collection)?;
        if field.trim().is_empty() || field.len() > 128 { return Err("field must be 1..=128 bytes".into()); }
        if dimensions == 0 || dimensions > 65536 { return Err("dimensions must be between 1 and 65536".into()); }
        let target = self.data.collections.entry(collection.to_string()).or_default();
        for (id, document) in &target.documents { read_vector(document, field, dimensions).map_err(|e| format!("document {id}: {e}"))?; }
        target.vector_fields.insert(field.to_string(), dimensions);
        self.persist_and_reindex()
    }

    /// Exact cosine-similarity search. This implementation scans documents; the metadata is ready for a future ANN index.
    pub fn search_vector(&self, collection: &str, field: &str, query: &[f64], limit: usize) -> Result<Vec<(String, Value, f64)>, String> {
        let target = self.data.collections.get(collection).ok_or_else(|| "collection not found".to_string())?;
        let dimensions = *target.vector_fields.get(field).ok_or_else(|| "vector index not found; create it first".to_string())?;
        validate_vector(query, dimensions)?;
        let mut results = Vec::new();
        for (id, document) in &target.documents {
            let vector = read_vector(document, field, dimensions)?;
            let score = cosine_similarity(query, &vector);
            results.push((id.clone(), document.clone(), score));
        }
        results.sort_by(|a, b| b.2.total_cmp(&a.2).then_with(|| a.0.cmp(&b.0)));
        results.truncate(limit);
        Ok(results)
    }

    pub fn sql(&self, statement: &str) -> Result<Vec<(String, Value)>, String> {
        let (collection, predicate, limit) = parse_select(statement)?;
        let mut rows = if let Some((field, operator, expected)) = predicate {
            self.data.collections.get(&collection).into_iter().flat_map(|c| c.documents.iter())
                .filter(|(_, doc)| compare_field(doc, &field, &operator, &expected))
                .map(|(id, doc)| (id.clone(), doc.clone())).collect()
        } else { self.list(&collection) };
        rows.sort_by(|a, b| a.0.cmp(&b.0));
        if let Some(limit) = limit { rows.truncate(limit); }
        Ok(rows)
    }

    /// Atomic in-process transaction: changes are persisted together, or not installed in memory.
    pub fn transaction<T>(&mut self, action: impl FnOnce(&mut Transaction) -> Result<T, String>) -> Result<T, String> {
        let mut transaction = Transaction { data: self.data.clone() };
        let result = action(&mut transaction)?;
        persist_file(&self.root, &transaction.data)?;
        self.data = transaction.data;
        self.rebuild_text_indexes();
        Ok(result)
    }

    fn persist_and_reindex(&mut self) -> Result<(), String> {
        persist_file(&self.root, &self.data)?;
        self.rebuild_text_indexes();
        Ok(())
    }

    fn rebuild_text_indexes(&mut self) {
        self.text_indexes.clear();
        for (name, collection) in &self.data.collections {
            for field in &collection.text_fields {
                let mut index = TextIndex::default();
                for (id, document) in &collection.documents {
                    if let Some(text) = field_value(document, field).and_then(Value::as_str) {
                        for term in tokenize(text) { *index.0.entry(term).or_default().entry(id.clone()).or_default() += 1; }
                    }
                }
                self.text_indexes.insert((name.clone(), field.clone()), index);
            }
        }
    }
}

pub struct Transaction { data: DatabaseFile }
impl Transaction {
    pub fn put(&mut self, collection: &str, id: &str, document: Value) -> Result<(), String> {
        validate_name(collection)?; validate_id(id)?;
        let target = self.data.collections.get(collection).cloned().unwrap_or_default();
        validate_vectors(&target, &document)?;
        self.data.collections.entry(collection.to_string()).or_default().documents.insert(id.to_string(), document);
        Ok(())
    }
    pub fn delete(&mut self, collection: &str, id: &str) -> Option<Value> {
        self.data.collections.get_mut(collection).and_then(|c| c.documents.remove(id))
    }
}

fn persist_file(root: &Path, data: &DatabaseFile) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(data).map_err(err)?;
    let temp = root.join("database.json.tmp");
    let dest = root.join("database.json");
    let mut file = File::create(&temp).map_err(err)?;
    file.write_all(&bytes).map_err(err)?;
    file.sync_all().map_err(err)?;
    fs::rename(&temp, &dest).map_err(err)?;
    Ok(())
}

fn validate_name(value: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > 128 || !value.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') { return Err("names must be 1..=128 ASCII letters, digits, '_' or '-'".into()); }
    Ok(())
}
fn validate_id(value: &str) -> Result<(), String> { if value.is_empty() || value.len() > 1024 { Err("ID must be 1..=1024 bytes".into()) } else { Ok(()) } }
fn err(error: impl std::fmt::Display) -> String { error.to_string() }
fn tokenize(text: &str) -> Vec<String> { text.split(|c: char| !c.is_alphanumeric()).filter(|s| !s.is_empty()).map(str::to_lowercase).collect() }
fn field_value<'a>(document: &'a Value, field: &str) -> Option<&'a Value> { document.as_object()?.get(field) }
fn validate_vectors(collection: &Collection, document: &Value) -> Result<(), String> {
    for (field, dimensions) in &collection.vector_fields { if let Some(value) = field_value(document, field) { let vector = value.as_array().ok_or_else(|| format!("vector field '{field}' must be an array"))?; if vector.len() != *dimensions { return Err(format!("vector field '{field}' must have {dimensions} dimensions")); } for item in vector { if !item.as_f64().is_some_and(f64::is_finite) { return Err(format!("vector field '{field}' must contain finite numbers")); } } } }
    Ok(())
}
fn validate_vector(vector: &[f64], dimensions: usize) -> Result<(), String> { if vector.len() != dimensions { return Err(format!("expected vector with {dimensions} dimensions")); } if vector.iter().any(|v| !v.is_finite()) { return Err("vector values must be finite".into()); } Ok(()) }
fn read_vector(document: &Value, field: &str, dimensions: usize) -> Result<Vec<f64>, String> {
    let value = field_value(document, field).ok_or_else(|| format!("missing vector field '{field}'"))?;
    let array = value.as_array().ok_or_else(|| format!("vector field '{field}' must be an array"))?;
    let result: Result<Vec<_>, _> = array.iter().map(|v| v.as_f64().ok_or_else(|| format!("vector field '{field}' must contain numbers"))).collect();
    let result = result?; validate_vector(&result, dimensions)?; Ok(result)
}
fn cosine_similarity(a: &[f64], b: &[f64]) -> f64 {
    let dot: f64 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norm_a = a.iter().map(|x| x * x).sum::<f64>().sqrt();
    let norm_b = b.iter().map(|x| x * x).sum::<f64>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 { 0.0 } else { dot / (norm_a * norm_b) }
}

fn parse_select(sql: &str) -> Result<(String, Option<(String, String, Value)>, Option<usize>), String> {
    let sql = sql.trim().trim_end_matches(';').trim();
    let upper = sql.to_ascii_uppercase();
    if !upper.starts_with("SELECT * FROM ") { return Err("supported SQL: SELECT * FROM collection [WHERE field op value] [LIMIT n]".into()); }
    let rest = sql[14..].trim();
    let upper_rest = rest.to_ascii_uppercase();
    let where_pos = upper_rest.find(" WHERE ");
    let limit_pos = upper_rest.find(" LIMIT ");
    let end_table = [where_pos, limit_pos].into_iter().flatten().min().unwrap_or(rest.len());
    let collection = rest[..end_table].trim().to_string();
    validate_name(&collection)?;
    let predicate = if let Some(pos) = where_pos {
        let start = pos + 7;
        let end = limit_pos.filter(|limit| *limit > pos).unwrap_or(rest.len());
        Some(parse_condition(rest[start..end].trim())?)
    } else { None };
    let limit = if let Some(pos) = limit_pos {
        Some(rest[pos + 7..].trim().parse::<usize>().map_err(|_| "LIMIT must be a non-negative integer")?)
    } else { None };
    Ok((collection, predicate, limit))
}
fn parse_condition(condition: &str) -> Result<(String, String, Value), String> {
    for operator in [">=", "<=", "!=", "=", ">", "<", "~"] {
        if let Some(pos) = condition.find(operator) {
            let field = condition[..pos].trim(); let raw = condition[pos + operator.len()..].trim();
            if field.is_empty() || raw.is_empty() { return Err("WHERE needs a field and value".into()); }
            let value = serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.trim_matches(['\'', '"']).to_string()));
            return Ok((field.into(), operator.into(), value));
        }
    }
    Err("unsupported WHERE predicate".into())
}
fn compare_field(document: &Value, field: &str, operator: &str, expected: &Value) -> bool {
    let Some(actual) = field_value(document, field) else { return false; };
    match operator {
        "=" => actual == expected,
        "!=" => actual != expected,
        "~" => actual.as_str().zip(expected.as_str()).is_some_and(|(a, b)| a.to_lowercase().contains(&b.to_lowercase())),
        ">" => compare_values(actual, expected).is_gt(),
        "<" => compare_values(actual, expected).is_lt(),
        ">=" => !compare_values(actual, expected).is_lt(),
        "<=" => !compare_values(actual, expected).is_gt(),
        _ => false,
    }
}
fn compare_values(a: &Value, b: &Value) -> std::cmp::Ordering {
    match (a, b) { (Value::Number(a), Value::Number(b)) => a.as_f64().partial_cmp(&b.as_f64()).unwrap_or(std::cmp::Ordering::Equal), (Value::String(a), Value::String(b)) => a.cmp(b), (Value::Bool(a), Value::Bool(b)) => a.cmp(b), _ => a.to_string().cmp(&b.to_string()) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::{SystemTime, UNIX_EPOCH};
    fn dir() -> PathBuf { std::env::temp_dir().join(format!("supernova_test_{}", SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos())) }

    #[test]
    fn document_text_vector_sql_and_reopen() {
        let path = dir();
        {
            let mut db = SupernovaDb::open(&path).unwrap();
            db.create_collection("items").unwrap();
            db.create_text_index("items", "body").unwrap();
            db.create_vector_index("items", "embedding", 2).unwrap();
            db.put("items", "a", json!({"body":"Rust database engine","embedding":[1.0,0.0],"rank":2})).unwrap();
            db.put("items", "b", json!({"body":"Database search","embedding":[0.0,1.0],"rank":1})).unwrap();
            assert_eq!(db.search_text("items", "body", "RUST", 5).unwrap()[0].0, "a");
            assert_eq!(db.search_vector("items", "embedding", &[0.9,0.1], 1).unwrap()[0].0, "a");
            assert_eq!(db.sql("SELECT * FROM items WHERE rank >= 2 LIMIT 1").unwrap().len(), 1);
            let before = db.get("items", "a");
            assert!(db.transaction(|tx| { tx.put("items", "c", json!({"embedding":[1.0]}))?; Ok(()) }).is_err());
            assert_eq!(db.get("items", "a"), before);
        }
        let db = SupernovaDb::open(&path).unwrap();
        assert!(db.get("items", "a").is_some());
        assert_eq!(db.search_text("items", "body", "engine", 5).unwrap().len(), 1);
        fs::remove_dir_all(path).unwrap();
    }
}
