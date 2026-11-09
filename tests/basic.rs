use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::PathBuf;

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct Snapshot {
    pub entries: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct SupernovaDb {
    path: PathBuf,
    entries: HashMap<String, String>,
}

impl SupernovaDb {
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, Box<dyn std::error::Error>> {
        let path = path.into();

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let entries = if path.exists() {
            let mut file = File::open(&path)?;
            let mut json = String::new();
            file.read_to_string(&mut json)?;

            if json.trim().is_empty() {
                HashMap::new()
            } else {
                let snapshot: Snapshot = serde_json::from_str(&json)?;
                snapshot.entries
            }
        } else {
            HashMap::new()
        };

        Ok(Self { path, entries })
    }

    pub fn set(
        &mut self,
        key: String,
        value: String,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.entries.insert(key, value);
        self.persist()?;
        Ok(())
    }

    pub fn get(&self, key: &str) -> Option<String> {
        self.entries.get(key).cloned()
    }

    pub fn delete(&mut self, key: &str) -> Result<Option<String>, Box<dyn std::error::Error>> {
        let value = self.entries.remove(key);
        self.persist()?;
        Ok(value)
    }

    pub fn list(&self) -> Vec<(String, String)> {
        let mut items: Vec<_> = self.entries.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        items.sort_by(|a, b| a.0.cmp(&b.0));
        items
    }

    fn persist(&self) -> Result<(), Box<dyn std::error::Error>> {
        let snapshot = Snapshot {
            entries: self.entries.clone(),
        };
        let json = serde_json::to_string_pretty(&snapshot)?;
        let mut file = File::create(&self.path)?;
        file.write_all(json.as_bytes())?;
        file.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::SupernovaDb;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn stores_and_returns_values() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = format!("/tmp/supernova_db_test_{unique}.json");

        let mut db = SupernovaDb::open(&path).unwrap();
        db.set("name".to_string(), "supernova".to_string()).unwrap();

        assert_eq!(db.get("name"), Some("supernova".to_string()));

        fs::remove_file(path).unwrap();
    }
}
