use anyhow::Result;
use heed::{EnvOpenOptions, Database, types::*};
use serde::{Deserialize, Serialize};
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

pub struct InstalledDb {
    path: PathBuf,
    env: heed::Env,
    db: Database<Str, SerdeBincode<Entry>>,
}

impl InstalledDb {
    pub fn load(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        if path.is_file() {
            let _ = std::fs::remove_file(&path);
        }
        std::fs::create_dir_all(&path)?;
        let env = unsafe { EnvOpenOptions::new()
            .map_size(100 * 1024 * 1024)
            .max_dbs(1)
            .open(&path)? };
            
        let mut txn = env.write_txn()?;
        let db = env.create_database(&mut txn, None)?;
        txn.commit()?;
        
        Ok(Self { path, env, db })
    }

    pub fn save(&self) -> Result<()> {
        Ok(())
    }

    pub fn upsert(&mut self, name: &str, version: &str, vur: &str, install_type: InstallType) {
        if let Ok(mut txn) = self.env.write_txn() {
            let entry = Entry {
                version: version.to_string(),
                vur: vur.to_string(),
                install_date: now_epoch(),
                install_type,
            };
            let _ = self.db.put(&mut txn, name, &entry);
            let _ = txn.commit();
        }
    }

    pub fn get(&self, name: &str) -> Option<Entry> {
        let txn = self.env.read_txn().ok()?;
        self.db.get(&txn, name).ok()?
    }

    pub fn remove(&mut self, name: &str) -> bool {
        if let Ok(mut txn) = self.env.write_txn() {
            let res = self.db.delete(&mut txn, name).unwrap_or(false);
            let _ = txn.commit();
            res
        } else {
            false
        }
    }

    pub fn names(&self) -> Vec<String> {
        let mut names = Vec::new();
        if let Ok(txn) = self.env.read_txn() {
            if let Ok(iter) = self.db.iter(&txn) {
                for item in iter {
                    if let Ok((k, _)) = item {
                        names.push(k.to_string());
                    }
                }
            }
        }
        names
    }

    pub fn retain(&mut self, pred: impl Fn(&str, &Entry) -> bool) {
        if let Ok(mut txn) = self.env.write_txn() {
            let mut to_remove = Vec::new();
            if let Ok(iter) = self.db.iter(&txn) {
                for item in iter {
                    if let Ok((k, v)) = item {
                        if !pred(k, &v) {
                            to_remove.push(k.to_string());
                        }
                    }
                }
            }
            for k in to_remove {
                let _ = self.db.delete(&mut txn, &k);
            }
            let _ = txn.commit();
        }
    }

    pub fn entries_snapshot(&self) -> Vec<(String, Entry)> {
        let mut entries = Vec::new();
        if let Ok(txn) = self.env.read_txn() {
            if let Ok(iter) = self.db.iter(&txn) {
                for item in iter {
                    if let Ok((k, v)) = item {
                        entries.push((k.to_string(), v));
                    }
                }
            }
        }
        entries
    }

    pub fn len(&self) -> usize {
        if let Ok(txn) = self.env.read_txn() {
            self.db.len(&txn).unwrap_or(0) as usize
        } else {
            0
        }
    }
}
