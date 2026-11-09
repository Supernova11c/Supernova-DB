use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::error::Error;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DbSnapshot {
    pub version: u32,
    pub tables: HashMap<String, BTreeMap<String, Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WalEvent {
    pub op: String,
    pub table: String,
    pub key: String,
    pub value: Option<Value>,
}

#[derive(Debug, Clone)]
pub struct SupernovaDb {
    root: PathBuf,
    snapshot_path: PathBuf,
    wal_path: PathBuf,
    tables: HashMap<String, BTreeMap<String, Value>>,
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

    pub fn create_table(&mut self, table: &str) -> Result<(), Box<dyn Error>> {
        self.tables.entry(table.to_string()).or_default();
        self.append_wal(&WalEvent {
            op: "create_table".to_string(),
            table: table.to_string(),
            key: String::new(),
            value: None,
        })?;
        self.persist_snapshot()?;
        Ok(())
    }

    pub fn delete_table(&mut self, table: &str) -> Result<Option<BTreeMap<String, Value>>, Box<dyn Error>> {
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
        let table_entries = self.tables.entry(table.to_string()).or_default();
        table_entries.insert(key.clone(), value.clone());

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
            .and_then(|rows| rows.get(key).cloned())
    }

    pub fn delete(&mut self, table: &str, key: &str) -> Result<Option<Value>, Box<dyn Error>> {
        let removed = self
            .tables
            .get_mut(table)
            .and_then(|rows| rows.remove(key));

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
            .map(|entries| {
                entries
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        rows.sort_by(|a, b| a.0.cmp(&b.0));
        rows
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
            version: 2,
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
                    self.tables.entry(event.table).or_default();
                }
                "delete_table" => {
                    self.tables.remove(&event.table);
                }
                "set" => {
                    let table = self.tables.entry(event.table).or_default();
                    table.insert(event.key, event.value.unwrap_or(Value::Null));
                }
                "delete" => {
                    if let Some(table) = self.tables.get_mut(&event.table) {
                        table.remove(&event.key);
                    }
                }
                _ => {}
            }
        }

        Ok(())
    }
}

impl AsRef<Path> for SupernovaDb {
    fn as_ref(&self) -> &Path {
        &self.root
    }
}
