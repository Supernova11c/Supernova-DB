use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::error::Error;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TableMeta {
    pub columns: Vec<String>,
    pub rows: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DbSnapshot {
    pub version: u32,
    pub tables: HashMap<String, TableMeta>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WalEvent {
    pub op: String,
    pub table: String,
    pub key: String,
    pub value: Option<Value>,
}

pub struct Transaction<'a> {
    db: &'a mut SupernovaDb,
    snapshot: HashMap<String, TableMeta>,
    active: bool,
}

#[derive(Debug, Clone)]
pub struct SupernovaDb {
    root: PathBuf,
    snapshot_path: PathBuf,
    wal_path: PathBuf,
    tables: HashMap<String, TableMeta>,
}

impl SupernovaDb {
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, Box<dyn Error>> {
        let root = path.into();
        fs::create_dir_all(&root)?;

        let snapshot_path = root.join("snapshot.json");
        let wal_path = root.join("wal.log");

        let mut db = Self {
            root,
            snapshot_path,
            wal_path,
            tables: HashMap::new(),
        };

        db.load_snapshot()?;
        db.replay_wal()?;
        Ok(db)
    }

    pub fn begin_transaction(&mut self) -> Result<Transaction<'_>, Box<dyn Error>> {
        Ok(Transaction {
            db: self,
            snapshot: self.tables.clone(),
            active: true,
        })
    }

    pub fn create_table(&mut self, table: &str) -> Result<(), Box<dyn Error>> {
        self.tables.entry(table.to_string()).or_insert_with(TableMeta::default);
        self.append_wal(&WalEvent {
            op: "create_table".to_string(),
            table: table.to_string(),
            key: String::new(),
            value: None,
        })?;
        self.persist_snapshot()?;
        Ok(())
    }

    pub fn delete_table(&mut self, table: &str) -> Result<Option<TableMeta>, Box<dyn Error>> {
        let removed = self.tables.remove(table);
        self.append_wal(&WalEvent {
            op: "delete_table".to_string(),
            table: table.to_string(),
            key: String::new(),
            value: None,
        })?;
        self.persist_snapshot()?;
        Ok(removed)
    }

    pub fn table_names(&self) -> Vec<String> {
        let mut names: Vec<_> = self.tables.keys().cloned().collect();
        names.sort();
        names
    }

    pub fn set(
        &mut self,
        table: &str,
        key: String,
        value: Value,
    ) -> Result<(), Box<dyn Error>> {
        let table_meta = self.tables.entry(table.to_string()).or_insert_with(TableMeta::default);
        table_meta.rows.insert(key.clone(), value.clone());
        for column in extract_columns(&value) {
            if !table_meta.columns.iter().any(|existing| existing == &column) {
                table_meta.columns.push(column);
            }
        }

        self.append_wal(&WalEvent {
            op: "set".to_string(),
            table: table.to_string(),
            key,
            value: Some(value),
        })?;

        self.persist_snapshot()?;
        Ok(())
    }

    pub fn get(&self, table: &str, key: &str) -> Option<Value> {
        self.tables
            .get(table)
            .and_then(|meta| meta.rows.get(key).cloned())
    }

    pub fn delete(&mut self, table: &str, key: &str) -> Result<Option<Value>, Box<dyn Error>> {
        let removed = self.tables.get_mut(table).and_then(|meta| meta.rows.remove(key));

        self.append_wal(&WalEvent {
            op: "delete".to_string(),
            table: table.to_string(),
            key: key.to_string(),
            value: removed.clone(),
        })?;

        self.persist_snapshot()?;
        Ok(removed)
    }

    pub fn list(&self, table: &str) -> Vec<(String, Value)> {
        let mut rows: Vec<_> = self
            .tables
            .get(table)
            .map(|meta| {
                meta.rows
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        rows.sort_by(|a, b| a.0.cmp(&b.0));
        rows
    }

    pub fn query(&self, table: &str, predicate: &str) -> Vec<(String, Value)> {
        let mut results = Vec::new();
        let Some((field, expected)) = predicate.split_once('=') else {
            return results;
        };

        let normalized_expected = expected.trim();

        if let Some(meta) = self.tables.get(table) {
            for (key, value) in &meta.rows {
                if matches_field(value, field, normalized_expected) {
                    results.push((key.clone(), value.clone()));
                }
            }
        }

        results.sort_by(|a, b| a.0.cmp(&b.0));
        results
    }

    fn load_snapshot(&mut self) -> Result<(), Box<dyn Error>> {
        if !self.snapshot_path.exists() {
            return Ok(());
        }

        let contents = fs::read_to_string(&self.snapshot_path)?;
        if contents.trim().is_empty() {
            return Ok(());
        }

        let snapshot: DbSnapshot = serde_json::from_str(&contents)?;
        self.tables = snapshot.tables;
        Ok(())
    }

    fn persist_snapshot(&self) -> Result<(), Box<dyn Error>> {
        let snapshot = DbSnapshot {
            version: 3,
            tables: self.tables.clone(),
        };

        let json = serde_json::to_string_pretty(&snapshot)?;
        fs::write(&self.snapshot_path, json)?;
        Ok(())
    }

    fn append_wal(&self, event: &WalEvent) -> Result<(), Box<dyn Error>> {
        let mut file = File::options()
            .create(true)
            .append(true)
            .open(&self.wal_path)?;

        let line = serde_json::to_string(event)?;
        file.write_all(line.as_bytes())?;
        file.write_all(b"\n")?;
        file.flush()?;
        Ok(())
    }

    fn replay_wal(&mut self) -> Result<(), Box<dyn Error>> {
        if !self.wal_path.exists() {
            return Ok(());
        }

        let file = File::open(&self.wal_path)?;
        let reader = BufReader::new(file);

        for line in reader.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }

            let event: WalEvent = serde_json::from_str(&line)?;
            match event.op.as_str() {
                "create_table" => {
                    self.tables.entry(event.table).or_insert_with(TableMeta::default);
                }
                "delete_table" => {
                    self.tables.remove(&event.table);
                }
                "set" => {
                    let table = self.tables.entry(event.table).or_insert_with(TableMeta::default);
                    if let Some(value) = event.value {
                        table.rows.insert(event.key, value.clone());
                        for column in extract_columns(&value) {
                            if !table.columns.iter().any(|existing| existing == &column) {
                                table.columns.push(column);
                            }
                        }
                    }
                }
                "delete" => {
                    if let Some(table) = self.tables.get_mut(&event.table) {
                        table.rows.remove(&event.key);
                    }
                }
                _ => {}
            }
        }

        Ok(())
    }
}

impl<'a> Transaction<'a> {
    pub fn set(&mut self, table: &str, key: String, value: Value) -> Result<(), Box<dyn Error>> {
        let table_meta = self.snapshot.entry(table.to_string()).or_insert_with(TableMeta::default);
        table_meta.rows.insert(key, value.clone());
        for column in extract_columns(&value) {
            if !table_meta.columns.iter().any(|existing| existing == &column) {
                table_meta.columns.push(column);
            }
        }
        Ok(())
    }

    pub fn get(&self, table: &str, key: &str) -> Option<Value> {
        self.snapshot
            .get(table)
            .and_then(|meta| meta.rows.get(key).cloned())
    }

    pub fn delete(&mut self, table: &str, key: &str) -> Result<Option<Value>, Box<dyn Error>> {
        let removed = self.snapshot.get_mut(table).and_then(|meta| meta.rows.remove(key));
        Ok(removed)
    }

    pub fn commit(mut self) -> Result<(), Box<dyn Error>> {
        if !self.active {
            return Ok(());
        }

        self.db.tables = self.snapshot.clone();
        self.db.persist_snapshot()?;
        self.active = false;
        Ok(())
    }

    pub fn rollback(mut self) -> Result<(), Box<dyn Error>> {
        self.active = false;
        Ok(())
    }
}

impl AsRef<Path> for SupernovaDb {
    fn as_ref(&self) -> &Path {
        &self.root
    }
}

fn extract_columns(value: &Value) -> Vec<String> {
    match value {
        Value::Object(map) => map
            .keys()
            .cloned()
            .chain(map.values().flat_map(extract_columns))
            .collect(),
        Value::Array(items) => items.iter().flat_map(extract_columns).collect(),
        _ => Vec::new(),
    }
}

fn matches_field(value: &Value, field: &str, expected: &str) -> bool {
    match value {
        Value::Object(obj) => {
            if let Some(actual) = obj.get(field) {
                match actual {
                    Value::String(s) => s == expected,
                    Value::Number(n) => n.to_string() == expected,
                    Value::Bool(b) => b.to_string() == expected,
                    Value::Null => expected == "null",
                    _ => actual.to_string() == expected,
                }
            } else {
                false
            }
        }
        Value::String(s) => s == expected && field == "value",
        Value::Number(n) => n.to_string() == expected && field == "value",
        Value::Bool(b) => b.to_string() == expected && field == "value",
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::SupernovaDb;
    use serde_json::json;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn transaction_and_query_work() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = format!("/tmp/supernova_db_tx_{unique}");

        let mut db = SupernovaDb::open(&dir).unwrap();
        db.create_table("users").unwrap();
        db.set("users", "alice".into(), json!({"name": "Alice", "role": "admin"})).unwrap();

        let mut tx = db.begin_transaction().unwrap();
        tx.set("users", "bob".into(), json!({"name": "Bob", "role": "user"})).unwrap();
        assert_eq!(tx.get("users", "bob"), Some(json!({"name": "Bob", "role": "user"})));
        tx.commit().unwrap();

        let rows = db.query("users", "role=admin");
        assert!(!rows.is_empty());

        fs::remove_dir_all(dir).unwrap();
    }
}
