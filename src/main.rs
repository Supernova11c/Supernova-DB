use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::error::Error;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ColumnSpec {
    pub name: String,
    pub kind: String,
    pub required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TableSchema {
    pub columns: Vec<ColumnSpec>,
    pub indexes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TableMeta {
    pub schema: TableSchema,
    pub rows: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DbSnapshot {
    pub version: u32,
    pub tables: BTreeMap<String, TableMeta>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum Mutation {
    CreateTable { table: String, columns: Vec<ColumnSpec> },
    DeleteTable { table: String },
    Set { table: String, key: String, value: Value },
    Delete { table: String, key: String },
    CreateIndex { table: String, field: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WalRecord {
    version: u32,
    sequence: u64,
    mutations: Vec<Mutation>,
}

#[derive(Debug, Clone, Default)]
struct SecondaryIndex(BTreeMap<String, BTreeSet<String>>);

pub struct Transaction<'a> {
    db: &'a mut SupernovaDb,
    working: BTreeMap<String, TableMeta>,
    mutations: Vec<Mutation>,
    finished: bool,
}

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
            root: root.clone(),
            snapshot_path: root.join("snapshot.json"),
            wal_path: root.join("wal.log"),
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
        self.create_table_with_schema(table, &[])
    }

    pub fn create_table_with_schema(&mut self, table: &str, columns: &[String]) -> Result<(), Box<dyn Error>> {
        validate_name(table, "table")?;
        let schema_columns = columns
            .iter()
            .map(|name| ColumnSpec { name: name.clone(), kind: infer_kind_from_name(name), required: false })
            .collect();
        self.commit_mutations(vec![Mutation::CreateTable { table: table.to_string(), columns: schema_columns }])
    }

    pub fn delete_table(&mut self, table: &str) -> Result<bool, Box<dyn Error>> {
        if !self.tables.contains_key(table) {
            return Ok(false);
        }
        self.commit_mutations(vec![Mutation::DeleteTable { table: table.to_string() }])?;
        Ok(true)
    }

    pub fn table_names(&self) -> Vec<String> {
        self.tables.keys().cloned().collect()
    }

    pub fn describe(&self, table: &str) -> Option<Vec<ColumnSpec>> {
        self.tables.get(table).map(|meta| meta.schema.columns.clone())
    }

    pub fn create_index(&mut self, table: &str, field: &str) -> Result<(), Box<dyn Error>> {
        validate_name(table, "table")?;
        validate_name(field, "field")?;
        if !self.tables.contains_key(table) {
            return Err(invalid_input("table does not exist"));
        }
        if self.tables[table].schema.indexes.iter().any(|name| name == field) {
            return Ok(());
        }
        self.commit_mutations(vec![Mutation::CreateIndex { table: table.to_string(), field: field.to_string() }])
    }

    pub fn indexes(&self, table: &str) -> Vec<String> {
        self.tables.get(table).map(|meta| meta.schema.indexes.clone()).unwrap_or_default()
    }

    pub fn set(&mut self, table: &str, key: String, value: Value) -> Result<(), Box<dyn Error>> {
        validate_name(table, "table")?;
        validate_key(&key)?;
        let meta = self.tables.entry(table.to_string()).or_default();
        validate_value_against_schema(&meta.schema, &value)?;
        self.commit_mutations(vec![Mutation::Set { table: table.to_string(), key, value }])
    }

    pub fn get(&self, table: &str, key: &str) -> Option<Value> {
        self.tables.get(table).and_then(|meta| meta.rows.get(key).cloned())
    }

    pub fn delete(&mut self, table: &str, key: &str) -> Result<Option<Value>, Box<dyn Error>> {
        let value = self.get(table, key);
        if value.is_none() {
            return Ok(None);
        }
        self.commit_mutations(vec![Mutation::Delete { table: table.to_string(), key: key.to_string() }])?;
        Ok(value)
    }

    pub fn list(&self, table: &str, sort_by: Option<&str>, limit: Option<usize>) -> Vec<(String, Value)> {
        let mut entries: Vec<_> = self
            .tables
            .get(table)
            .map(|meta| meta.rows.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
            .unwrap_or_default();

        if let Some(field) = sort_by {
            entries.sort_by(|a, b| compare_sort_value(&a.1, &b.1, field));
        } else {
            entries.sort_by(|a, b| a.0.cmp(&b.0));
        }

        if let Some(limit) = limit {
            entries.truncate(limit);
        }

        entries
    }

    pub fn query(&self, table: &str, predicate: &str) -> Vec<(String, Value)> {
        let mut results = Vec::new();
        if let Some((field, op, raw_value)) = parse_predicate(predicate) {
            let value = parse_query_value(&raw_value);
            for (key, row) in self.tables.get(table).into_iter().flat_map(|meta| meta.rows.iter()) {
                if row_matches_field(row, &field, &op, &value) {
                    results.push((key.clone(), row.clone()));
                }
            }
        }
        results.sort_by(|a, b| a.0.cmp(&b.0));
        results
    }

    pub fn checkpoint(&mut self) -> Result<(), Box<dyn Error>> {
        self.persist_snapshot()?;
        let mut wal = OpenOptions::new().write(true).truncate(true).open(&self.wal_path)?;
        wal.sync_all()?;
        Ok(())
    }

    pub fn backup(&self, destination: &Path) -> Result<(), Box<dyn Error>> {
        fs::create_dir_all(destination)?;
        let snapshot_dst = destination.join("snapshot.json");
        let wal_dst = destination.join("wal.log");
        fs::copy(&self.snapshot_path, snapshot_dst)?;
        if self.wal_path.exists() {
            fs::copy(&self.wal_path, wal_dst)?;
        }
        Ok(())
    }

    pub fn restore(&mut self, source: &Path) -> Result<(), Box<dyn Error>> {
        let snapshot_src = source.join("snapshot.json");
        let wal_src = source.join("wal.log");
        if snapshot_src.exists() {
            let bytes = fs::read(snapshot_src)?;
            let snapshot: DbSnapshot = serde_json::from_slice(&bytes)?;
            self.tables = snapshot.tables;
            self.persist_snapshot()?;
        }
        if wal_src.exists() {
            fs::copy(wal_src, &self.wal_path)?;
            self.replay_wal()?;
        }
        self.rebuild_indexes();
        Ok(())
    }

    pub fn begin_transaction(&mut self) -> Transaction<'_> {
        Transaction {
            db: self,
            working: self.tables.clone(),
            mutations: Vec::new(),
            finished: false,
        }
    }

    fn commit_mutations(&mut self, mutations: Vec<Mutation>) -> Result<(), Box<dyn Error>> {
        if mutations.is_empty() {
            return Ok(());
        }
        let next = self.sequence + 1;
        let record = WalRecord { version: 1, sequence: next, mutations: mutations.clone() };
        let bytes = serde_json::to_vec(&record)?;
        let mut wal = OpenOptions::new().create(true).append(true).open(&self.wal_path)?;
        wal.write_all(&bytes)?;
        wal.write_all(b"\n")?;
        wal.sync_all()?;
        self.sequence = next;
        for mutation in &mutations {
            apply_mutation(&mut self.tables, mutation);
        }
        self.rebuild_indexes();
        self.persist_snapshot()?;
        Ok(())
    }

    fn load_snapshot(&mut self) -> Result<(), Box<dyn Error>> {
        if !self.snapshot_path.exists() {
            return Ok(());
        }
        let bytes = fs::read_to_string(&self.snapshot_path)?;
        if bytes.trim().is_empty() {
            return Ok(());
        }
        let snapshot: DbSnapshot = serde_json::from_str(&bytes)?;
        self.tables = snapshot.tables;
        Ok(())
    }

    fn persist_snapshot(&self) -> Result<(), Box<dyn Error>> {
        let snapshot = DbSnapshot { version: 6, tables: self.tables.clone() };
        let bytes = serde_json::to_vec_pretty(&snapshot)?;
        let temp = self.root.join("snapshot.json.tmp");
        let mut file = File::create(&temp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(temp, &self.snapshot_path)?;
        Ok(())
    }

    fn replay_wal(&mut self) -> Result<(), Box<dyn Error>> {
        if !self.wal_path.exists() {
            return Ok(());
        }
        let reader = BufReader::new(File::open(&self.wal_path)?);
        for line in reader.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let record: WalRecord = serde_json::from_str(&line)?;
            if record.version != 1 {
                continue;
            }
            if record.sequence <= self.sequence {
                continue;
            }
            self.sequence = record.sequence;
            for mutation in &record.mutations {
                apply_mutation(&mut self.tables, mutation);
            }
        }
        Ok(())
    }

    fn rebuild_indexes(&mut self) {
        self.indexes.clear();
        for (table_name, table) in &self.tables {
            for field in &table.schema.indexes {
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

impl<'a> Transaction<'a> {
    pub fn set(&mut self, table: &str, key: String, value: Value) -> Result<(), Box<dyn Error>> {
        validate_name(table, "table")?;
        validate_key(&key)?;
        let meta = self.working.entry(table.to_string()).or_default();
        validate_value_against_schema(&meta.schema, &value)?;
        let mutation = Mutation::Set { table: table.to_string(), key: key.clone(), value: value.clone() };
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
            let mutation = Mutation::Delete { table: table.to_string(), key: key.to_string() };
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

    pub fn rollback(mut self) {
        self.finished = true;
        self.mutations.clear();
    }
}

impl Drop for Transaction<'_> {
    fn drop(&mut self) {
        if !self.finished {
            self.mutations.clear();
        }
    }
}

fn apply_mutation(tables: &mut BTreeMap<String, TableMeta>, mutation: &Mutation) {
    match mutation {
        Mutation::CreateTable { table, columns } => {
            let meta = tables.entry(table.clone()).or_insert_with(TableMeta::default);
            meta.schema.columns = columns.clone();
            meta.schema.indexes.sort();
        }
        Mutation::DeleteTable { table } => {
            tables.remove(table);
        }
        Mutation::Set { table, key, value } => {
            let meta = tables.entry(table.clone()).or_insert_with(TableMeta::default);
            meta.rows.insert(key.clone(), value.clone());
            update_schema_columns(&mut meta.schema, value);
        }
        Mutation::Delete { table, key } => {
            if let Some(meta) = tables.get_mut(table) {
                meta.rows.remove(key);
            }
        }
        Mutation::CreateIndex { table, field } => {
            if let Some(meta) = tables.get_mut(table) {
                if !meta.schema.indexes.iter().any(|name| name == field) {
                    meta.schema.indexes.push(field.clone());
                    meta.schema.indexes.sort();
                }
            }
        }
    }
}

fn update_schema_columns(schema: &mut TableSchema, value: &Value) {
    if let Value::Object(map) = value {
        for (field, field_value) in map {
            if !schema.columns.iter().any(|col| col.name == *field) {
                schema.columns.push(ColumnSpec { name: field.clone(), kind: infer_kind(field_value), required: false });
            }
        }
        schema.columns.sort_by(|a, b| a.name.cmp(&b.name));
    }
}

fn infer_kind(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(_) => "bool".to_string(),
        Value::Number(_) => "number".to_string(),
        Value::String(_) => "string".to_string(),
        Value::Array(_) => "array".to_string(),
        Value::Object(_) => "object".to_string(),
    }
}

fn infer_kind_from_name(name: &str) -> String {
    if name.contains("age") || name.contains("count") || name.contains("amount") || name.contains("id") {
        "number".to_string()
    } else {
        "string".to_string()
    }
}

fn validate_name(value: &str, what: &str) -> Result<(), Box<dyn Error>> {
    if value.is_empty() || value.len() > 128 || !value.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return Err(invalid_input(format!("invalid {what} name")));
    }
    Ok(())
}

fn validate_key(key: &str) -> Result<(), Box<dyn Error>> {
    if key.is_empty() || key.len() > 1024 {
        return Err(invalid_input("key must be between 1 and 1024 characters"));
    }
    Ok(())
}

fn invalid_input(message: impl Into<String>) -> Box<dyn Error> {
    Box::new(std::io::Error::new(std::io::ErrorKind::InvalidInput, message.into()))
}

fn validate_value_against_schema(schema: &TableSchema, value: &Value) -> Result<(), Box<dyn Error>> {
    if schema.columns.is_empty() {
        return Ok(());
    }
    let object = value.as_object().ok_or_else(|| invalid_input("value must be an object when schema exists"))?;
    for column in &schema.columns {
        if column.required && !object.contains_key(&column.name) {
            return Err(invalid_input(format!("missing required field '{}'", column.name)));
        }
        if let Some(actual) = object.get(&column.name) {
            let expected_kind = column.kind.as_str();
            let actual_kind = infer_kind(actual);
            if expected_kind != actual_kind && expected_kind != "string" {
                return Err(invalid_input(format!("field '{}' expected type '{}' but got '{}'", column.name, expected_kind, actual_kind)));
            }
        }
    }
    Ok(())
}

fn parse_predicate(predicate: &str) -> Option<(String, String, String)> {
    let predicate = predicate.trim();
    if predicate.is_empty() {
        return None;
    }

    let ops = [">=", "<=", "!=", "==", "=", ">", "<", "~"];
    let mut best: Option<(usize, &str)> = None;
    for op in ops {
        if let Some(index) = predicate.find(op) {
            match &best {
                Some((current_index, _)) if index >= *current_index => {}
                _ => best = Some((index, op)),
            }
        }
    }
    let (idx, op) = best?;
    let field = predicate[..idx].trim();
    let raw_value = predicate[idx + op.len()..].trim();
    if field.is_empty() || raw_value.is_empty() {
        return None;
    }
    Some((field.to_string(), op.to_string(), raw_value.to_string()))
}

fn parse_query_value(raw: &str) -> Value {
    serde_json::from_str(raw).unwrap_or(Value::String(raw.to_string()))
}

fn row_matches_field(row: &Value, field: &str, op: &str, expected: &Value) -> bool {
    let candidate = match row {
        Value::Object(obj) => obj.get(field).cloned().unwrap_or(Value::Null),
        _ if field == "value" => row.clone(),
        _ => Value::Null,
    };

    match op {
        "=" => candidate == *expected,
        "==" => candidate == *expected,
        "!=" => candidate != *expected,
        ">" => compare_values(&candidate, expected) == std::cmp::Ordering::Greater,
        "<" => compare_values(&candidate, expected) == std::cmp::Ordering::Less,
        ">=" => compare_values(&candidate, expected) != std::cmp::Ordering::Less,
        "<=" => compare_values(&candidate, expected) != std::cmp::Ordering::Greater,
        "~" => match (&candidate, expected) {
            (Value::String(actual), Value::String(expected_str)) => actual.to_ascii_lowercase().contains(&expected_str.to_ascii_lowercase()),
            _ => false,
        },
        _ => candidate == *expected,
    }
}

fn compare_values(left: &Value, right: &Value) -> std::cmp::Ordering {
    match (left, right) {
        (Value::Number(a), Value::Number(b)) => a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal),
        (Value::String(a), Value::String(b)) => a.cmp(b),
        (Value::Bool(a), Value::Bool(b)) => a.cmp(b),
        _ => left.to_string().cmp(&right.to_string()),
    }
}

fn compare_sort_value(left: &Value, right: &Value, field: &str) -> std::cmp::Ordering {
    let left_value = match left {
        Value::Object(obj) => obj.get(field).unwrap_or(&Value::Null),
        _ => left,
    };
    let right_value = match right {
        Value::Object(obj) => obj.get(field).unwrap_or(&Value::Null),
        _ => right,
    };
    compare_values(left_value, right_value)
}

impl AsRef<Path> for SupernovaDb {
    fn as_ref(&self) -> &Path {
        &self.root
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(label: &str) -> PathBuf {
        let id = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        std::env::temp_dir().join(format!("supernova_{label}_{id}"))
    }

    #[test]
    fn rich_query_and_backup_restore_work() {
        let path = temp_dir("rich_query");
        {
            let mut db = SupernovaDb::open(&path).unwrap();
            db.create_table_with_schema("users", &["name".into(), "role".into(), "age".into()]).unwrap();
            db.set("users", "alice".to_string(), json!({"name":"Alice","role":"admin","age":30})).unwrap();
            db.set("users", "bob".to_string(), json!({"name":"Bob","role":"user","age":25})).unwrap();
            db.create_index("users", "role").unwrap();
            assert_eq!(db.query("users", "role=admin").len(), 1);
            assert_eq!(db.query("users", "age>=25").len(), 2);
            let backup_dir = std::env::temp_dir().join(format!("supernova_backup_{}", SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()));
            db.backup(&backup_dir).unwrap();
            let mut restored = SupernovaDb::open(&backup_dir).unwrap();
            assert_eq!(restored.get("users", "alice"), Some(json!({"name":"Alice","role":"admin","age":30})));
            fs::remove_dir_all(backup_dir).unwrap();
        }
        fs::remove_dir_all(path).unwrap();
    }
}
