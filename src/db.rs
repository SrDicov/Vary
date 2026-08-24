use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn now_epoch() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum InstallType {
    Source,
    Binary,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Entry {
    pub version: String,
    pub vur: String,
    pub install_date: u64,
    pub install_type: InstallType,
}

#[derive(Debug, Default)]
pub struct InstalledDb {
    path: PathBuf,
    entries: BTreeMap<String, Entry>,
}

impl InstalledDb {
    pub fn load(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        let mut db = Self {
            path,
            entries: BTreeMap::new(),
        };
        let text = match std::fs::read_to_string(&db.path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                tracing::warn!(
                    "{} no existe, iniciando base de datos vacía",
                    db.path.display()
                );
                return Ok(db);
            }
            Err(err) => {
                return Err(err).with_context(|| format!("no se pudo leer {}", db.path.display()));
            }
        };
        match serde_json::from_str(&text) {
            Ok(entries) => db.entries = entries,
            Err(err) => tracing::warn!(
                "{} corrupta ({err}), iniciando base de datos vacía",
                db.path.display()
            ),
        }
        Ok(db)
    }

    pub fn save(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("no se pudo crear {}", parent.display()))?;
            }
        }
        std::fs::write(&self.path, serde_json::to_string_pretty(&self.entries)?)
            .with_context(|| format!("no se pudo escribir {}", self.path.display()))?;
        Ok(())
    }

    pub fn upsert(&mut self, name: &str, version: &str, vur: &str, install_type: InstallType) {
        self.entries.insert(
            name.to_string(),
            Entry {
                version: version.to_string(),
                vur: vur.to_string(),
                install_date: now_epoch(),
                install_type,
            },
        );
    }

    pub fn get(&self, name: &str) -> Option<&Entry> {
        self.entries.get(name)
    }

    pub fn remove(&mut self, name: &str) -> bool {
        self.entries.remove(name).is_some()
    }

    pub fn names(&self) -> Vec<String> {
        self.entries.keys().cloned().collect()
    }

    pub fn retain(&mut self, pred: impl Fn(&str, &Entry) -> bool) {
        self.entries.retain(|name, entry| pred(name, entry));
    }

    /// Snapshot (nombre, entrada) para iterar sin pelearse con el borrow.
    pub fn entries_snapshot(&self) -> Vec<(String, Entry)> {
        self.names()
            .into_iter()
            .filter_map(|k| self.get(&k).cloned().map(|v| (k, v)))
            .collect()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_preserves_entries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("installed.json");
        let mut db = InstalledDb::load(&path).unwrap();
        assert_eq!(db.len(), 0);

        db.upsert("hello-vur", "1.0_1", "mi-repo", InstallType::Source);
        db.upsert("kernel-vur", "6.12_1", "void-repo", InstallType::Binary);
        db.save().unwrap();

        let reloaded = InstalledDb::load(&path).unwrap();
        assert_eq!(reloaded.len(), 2);

        let hello = reloaded.get("hello-vur").unwrap();
        assert_eq!(hello.version, "1.0_1");
        assert_eq!(hello.vur, "mi-repo");
        assert_eq!(hello.install_type, InstallType::Source);
        assert!(hello.install_date > 1_500_000_000, "epoch razonable");

        assert_eq!(
            reloaded.get("kernel-vur").unwrap().install_type,
            InstallType::Binary
        );

        let mut db3 = reloaded;
        assert!(db3.remove("hello-vur"));
        assert!(!db3.remove("hello-vur"));
        assert_eq!(db3.names(), vec!["kernel-vur".to_string()]);
    }

    #[test]
    fn json_shape_matches_contract() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("installed.json");
        let mut db = InstalledDb::load(&path).unwrap();
        db.upsert("hello-vur", "1.0_1", "mi-repo", InstallType::Source);
        db.save().unwrap();

        let raw = std::fs::read_to_string(&path).unwrap();
        let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
        let hello = &value["hello-vur"];
        let obj = hello.as_object().unwrap();
        assert_eq!(obj.len(), 4);
        assert!(obj.contains_key("version"));
        assert!(obj.contains_key("vur"));
        assert!(obj.contains_key("install_date"));
        assert!(obj.contains_key("install_type"));
        assert_eq!(hello["version"], "1.0_1");
        assert_eq!(hello["vur"], "mi-repo");
        assert_eq!(hello["install_type"], "source");
        assert!(hello["install_date"].as_u64().unwrap_or(0) > 1_500_000_000, "epoch razonable");
    }

    #[test]
    fn retain_prunes_selected_entries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("installed.json");
        let mut db = InstalledDb::load(&path).unwrap();
        db.upsert("keep-a", "1", "r", InstallType::Source);
        db.upsert("drop-b", "2", "r", InstallType::Source);
        db.upsert("keep-c", "3", "r", InstallType::Binary);
        db.retain(|name, _| name.starts_with("keep-"));

        assert_eq!(db.len(), 2);
        assert_eq!(
            db.names(),
            vec!["keep-a".to_string(), "keep-c".to_string()]
        );
        assert!(db.get("drop-b").is_none());
    }

    #[test]
    fn corrupt_db_starts_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("installed.json");
        std::fs::write(&path, "{esto no es json").unwrap();
        let db = InstalledDb::load(&path).unwrap();
        assert_eq!(db.len(), 0);
    }

    #[test]
    fn upsert_replaces_existing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("installed.json");
        let mut db = InstalledDb::load(&path).unwrap();
        db.upsert("hello-vur", "1.0_1", "mi-repo", InstallType::Source);
        db.upsert("hello-vur", "1.1_1", "mi-repo", InstallType::Binary);
        assert_eq!(db.len(), 1);
        assert_eq!(db.get("hello-vur").unwrap().version, "1.1_1");
        assert_eq!(
            db.get("hello-vur").unwrap().install_type,
            InstallType::Binary
        );
    }
}
