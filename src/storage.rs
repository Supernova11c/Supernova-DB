use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::error::Error;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TableMeta {
    #[serde(default)]
    pub columns: Vec<String>,
    #[serde(default)]
    pub indexes: Vec<String>,
    #[serde(default)]
    pub rows: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DbSnapshot {
    pub version: u32,
    #[serde(default)]
    pub tables: BTreeMap<String, TableMeta>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum Mutation {
    CreateTable { table: String },
    DeleteTable { table: String },
    Set { table: String, key: String, value: Value },
    Delete { table: String, key: String },
    CreateIndex { table: String, field: String },
}

#[derive(Debug, Serialize, Deserialize)]
struct WalRecord {
    version: u32,
    sequence: u64,
    mutations: Vec<Mutation>,
}

#[derive(Debug, Clone, Default)]
struct SecondaryIndex(BTreeMap<String, BTreeSet<String>>);

#[derive(Debug, Clone)]
pub struct SupernovaDb {
    root: PathBuf,
    snapshot_path: PathBuf,
    wal_path: PathBuf,
    tables: BTreeMap<String, TableMeta>,
    indexes: HashMap<(String, String), SecondaryIndex>,
    sequence: u64,
}

impl SupernovaDb {
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, Box<dyn Error>> {
        let root = path.into();
        fs::create_dir_all(&root)?;
        let mut db = Self {
            snapshot_path: root.join("snapshot.json"),
            wal_path: root.join("wal.log"),
            root,
            tables: BTreeMap::new(),
            indexes: HashMap::new(),
            sequence: 0,
        };
        db.load_snapshot()?;
        db.replay_wal()?;
        db.rebuild_indexes();
        Ok(db)
    }

    pub fn create_table(&mut self, table: &str) -> Result<(), Box<dyn Error>> {
        validate_name(table, "table")?;
        self.commit_mutations(vec![Mutation::CreateTable { table: table.into() }])
    }

    pub fn delete_table(&mut self, table: &str) -> Result<bool, Box<dyn Error>> {
        if !self.tables.contains_key(table) { return Ok(false); }
        self.commit_mutations(vec![Mutation::DeleteTable { table: table.into() }])?;
        Ok(true)
    }

    pub fn table_names(&self) -> Vec<String> { self.tables.keys().cloned().collect() }

    pub fn set(&mut self, table: &str, key: String, value: Value) -> Result<(), Box<dyn Error>> {
        validate_name(table, "table")?;
        validate_key(&key)?;
        self.commit_mutations(vec![Mutation::Set { table: table.into(), key, value }])
    }

    pub fn get(&self, table: &str, key: &str) -> Option<Value> {
        self.tables.get(table).and_then(|meta| meta.rows.get(key).cloned())
    }

    pub fn delete(&mut self, table: &str, key: &str) -> Result<Option<Value>, Box<dyn Error>> {
        let Some(value) = self.get(table, key) else { return Ok(None); };
        self.commit_mutations(vec![Mutation::Delete { table: table.into(), key: key.into() }])?;
        Ok(Some(value))
    }

    pub fn list(&self, table: &str) -> Vec<(String, Value)> {
        self.tables.get(table).map(|meta| meta.rows.iter().map(|(k, v)| (k.clone(), v.clone())).collect()).unwrap_or_default()
    }

    pub fn create_index(&mut self, table: &str, field: &str) -> Result<(), Box<dyn Error>> {
        validate_name(table, "table")?;
        validate_name(field, "field")?;
        if !self.tables.contains_key(table) { return Err(invalid_input("table does not exist")); }
        if self.tables[table].indexes.iter().any(|name| name == field) { return Ok(()); }
        self.commit_mutations(vec![Mutation::CreateIndex { table: table.into(), field: field.into() }])
    }

    pub fn indexes(&self, table: &str) -> Vec<String> {
        self.tables.get(table).map(|meta| meta.indexes.clone()).unwrap_or_default()
    }

    /// Equality lookup. Uses a secondary index when one exists for `field`.
    pub fn query_eq(&self, table: &str, field: &str, expected: &Value) -> Vec<(String, Value)> {
        let canonical = expected.to_string();
        if let Some(index) = self.indexes.get(&(table.to_owned(), field.to_owned())) {
            return index.0.get(&canonical).into_iter().flatten()
                .filter_map(|key| self.get(table, key).map(|value| (key.clone(), value)))
                .collect();
        }
        self.tables.get(table).into_iter().flat_map(|meta| meta.rows.iter())
            .filter(|(_, row)| row.get(field) == Some(expected))
            .map(|(key, value)| (key.clone(), value.clone())).collect()
    }

    /// Compatibility helper for simple `field=value` string predicates.
    pub fn query(&self, table: &str, predicate: &str) -> Vec<(String, Value)> {
        let Some((field, raw)) = predicate.split_once('=') else { return Vec::new(); };
        let field = field.trim();
        let raw = raw.trim();
        let expected = serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.to_owned()));
        self.query_eq(table, field, &expected)
    }

    /// Write a consistent snapshot, then truncate the WAL only after the snapshot is installed.
    pub fn checkpoint(&mut self) -> Result<(), Box<dyn Error>> {
        self.persist_snapshot()?;
        let file = File::create(&self.wal_path)?;
        file.sync_all()?;
        Ok(())
    }

    pub fn begin_transaction(&mut self) -> Transaction<'_> {
        let working = self.tables.clone();
        Transaction { db: self, working, mutations: Vec::new(), finished: false }
    }

    fn commit_mutations(&mut self, mutations: Vec<Mutation>) -> Result<(), Box<dyn Error>> {
        if mutations.is_empty() { return Ok(()); }
        let next = self.sequence.checked_add(1).ok_or_else(|| invalid_input("WAL sequence exhausted"))?;
        let record = WalRecord { version: 1, sequence: next, mutations: mutations.clone() };
        let line = serde_json::to_vec(&record)?;
        let mut file = OpenOptions::new().create(true).append(true).open(&self.wal_path)?;
        file.write_all(&line)?;
        file.write_all(b"\n")?;
        file.sync_data()?;
        for mutation in &mutations { apply_mutation(&mut self.tables, mutation); }
        self.sequence = next;
        self.rebuild_indexes();
        Ok(())
    }

    fn load_snapshot(&mut self) -> Result<(), Box<dyn Error>> {
        if !self.snapshot_path.exists() { return Ok(()); }
        let contents = fs::read_to_string(&self.snapshot_path)?;
        if contents.trim().is_empty() { return Ok(()); }
        let snapshot: DbSnapshot = serde_json::from_str(&contents)?;
        self.tables = snapshot.tables;
        Ok(())
    }

    fn persist_snapshot(&self) -> Result<(), Box<dyn Error>> {
        let snapshot = DbSnapshot { version: 4, tables: self.tables.clone() };
        let bytes = serde_json::to_vec_pretty(&snapshot)?;
        let temp = self.root.join("snapshot.json.tmp");
        let mut file = File::create(&temp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(temp, &self.snapshot_path)?;
        Ok(())
    }

    fn replay_wal(&mut self) -> Result<(), Box<dyn Error>> {
        if !self.wal_path.exists() { return Ok(()); }
        let reader = BufReader::new(File::open(&self.wal_path)?);
        let lines: Vec<String> = reader.lines().collect::<Result<_, _>>()?;
        for (index, line) in lines.iter().enumerate() {
            if line.trim().is_empty() { continue; }
            let record: WalRecord = match serde_json::from_str(line) {
                Ok(record) => record,
                Err(_) if index + 1 == lines.len() => break, // Ignore an incomplete final append after a crash.
                Err(error) => return Err(Box::new(error)),
            };
            if record.version != 1 { return Err(invalid_input("unsupported WAL record version")); }
            if record.sequence <= self.sequence { continue; }
            if record.sequence != self.sequence + 1 { return Err(invalid_input("WAL sequence gap")); }
            for mutation in &record.mutations { apply_mutation(&mut self.tables, mutation); }
            self.sequence = record.sequence;
        }
        Ok(())
    }

    fn rebuild_indexes(&mut self) {
        self.indexes.clear();
        for (table_name, table) in &self.tables {
            for field in &table.indexes {
                let mut index = SecondaryIndex::default();
                for (key, row) in &table.rows {
                    if let Some(value) = row.get(field) {
                        index.0.entry(value.to_string()).or_default().insert(key.clone());
                    }
                }
                self.indexes.insert((table_name.clone(), field.clone()), index);
            }
        }
    }
}

pub struct Transaction<'a> {
    db: &'a mut SupernovaDb,
    working: BTreeMap<String, TableMeta>,
    mutations: Vec<Mutation>,
    finished: bool,
}

impl Transaction<'_> {
    pub fn create_table(&mut self, table: &str) -> Result<(), Box<dyn Error>> {
        validate_name(table, "table")?;
        self.mutations.push(Mutation::CreateTable { table: table.into() });
        apply_mutation(&mut self.working, self.mutations.last().unwrap());
        Ok(())
    }

    pub fn set(&mut self, table: &str, key: String, value: Value) -> Result<(), Box<dyn Error>> {
        validate_name(table, "table")?;
        validate_key(&key)?;
        let mutation = Mutation::Set { table: table.into(), key, value };
        apply_mutation(&mut self.working, &mutation);
        self.mutations.push(mutation);
        Ok(())
    }

    pub fn get(&self, table: &str, key: &str) -> Option<Value> {
        self.working.get(table).and_then(|meta| meta.rows.get(key).cloned())
    }

    pub fn delete(&mut self, table: &str, key: &str) -> Result<Option<Value>, Box<dyn Error>> {
        let value = self.get(table, key);
        if value.is_some() {
            let mutation = Mutation::Delete { table: table.into(), key: key.into() };
            apply_mutation(&mut self.working, &mutation);
            self.mutations.push(mutation);
        }
        Ok(value)
    }

    pub fn commit(mut self) -> Result<(), Box<dyn Error>> {
        self.db.commit_mutations(std::mem::take(&mut self.mutations))?;
        self.finished = true;
        Ok(())
    }

    pub fn rollback(mut self) { self.mutations.clear(); self.finished = true; }
}

impl Drop for Transaction<'_> {
    fn drop(&mut self) {
        if !self.finished { self.mutations.clear(); }
    }
}

fn apply_mutation(tables: &mut BTreeMap<String, TableMeta>, mutation: &Mutation) {
    match mutation {
        Mutation::CreateTable { table } => { tables.entry(table.clone()).or_default(); }
        Mutation::DeleteTable { table } => { tables.remove(table); }
        Mutation::Set { table, key, value } => {
            let meta = tables.entry(table.clone()).or_default();
            meta.rows.insert(key.clone(), value.clone());
            if let Value::Object(object) = value {
                for field in object.keys() {
                    if !meta.columns.contains(field) { meta.columns.push(field.clone()); }
                }
                meta.columns.sort();
            }
        }
        Mutation::Delete { table, key } => { if let Some(meta) = tables.get_mut(table) { meta.rows.remove(key); } }
        Mutation::CreateIndex { table, field } => {
            if let Some(meta) = tables.get_mut(table) {
                if !meta.indexes.contains(field) { meta.indexes.push(field.clone()); meta.indexes.sort(); }
            }
        }
    }
}

fn validate_name(value: &str, what: &str) -> Result<(), Box<dyn Error>> {
    if value.is_empty() || value.len() > 128 || !value.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return Err(invalid_input(format!("invalid {what} name")));
    }
    Ok(())
}

fn validate_key(key: &str) -> Result<(), Box<dyn Error>> {
    if key.is_empty() || key.len() > 1024 { return Err(invalid_input("key must be 1..=1024 bytes")); }
    Ok(())
}

fn invalid_input(message: impl Into<String>) -> Box<dyn Error> {
    Box::new(std::io::Error::new(std::io::ErrorKind::InvalidInput, message.into()))
}

impl AsRef<Path> for SupernovaDb { fn as_ref(&self) -> &Path { &self.root } }

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir() -> PathBuf {
        let id = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        std::env::temp_dir().join(format!("supernova_db_test_{id}"))
    }

    #[test]
    fn wal_recovery_indexes_and_atomic_transaction() {
        let path = temp_dir();
        {
            let mut db = SupernovaDb::open(&path).unwrap();
            db.create_table("users").unwrap();
            db.create_index("users", "role").unwrap();
            let mut tx = db.begin_transaction();
            tx.set("users", "alice".into(), json!({"role":"admin"})).unwrap();
            tx.set("users", "bob".into(), json!({"role":"user"})).unwrap();
            tx.commit().unwrap();
            assert_eq!(db.query_eq("users", "role", &json!("admin")).len(), 1);
        }
        let db = SupernovaDb::open(&path).unwrap();
        assert_eq!(db.get("users", "alice"), Some(json!({"role":"admin"})));
        assert_eq!(db.query_eq("users", "role", &json!("user")).len(), 1);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn checkpoint_preserves_data_and_clears_wal() {
        let path = temp_dir();
        let mut db = SupernovaDb::open(&path).unwrap();
        db.set("items", "one".into(), json!(1)).unwrap();
        db.checkpoint().unwrap();
        assert_eq!(fs::metadata(path.join("wal.log")).unwrap().len(), 0);
        drop(db);
        assert_eq!(SupernovaDb::open(&path).unwrap().get("items", "one"), Some(json!(1)));
        fs::remove_dir_all(path).unwrap();
    }
}
