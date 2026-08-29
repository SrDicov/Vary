use crate::metadata::VurInfo;
use anyhow::{Context, Result};
use heed::{EnvOpenOptions, Database, types::*};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn now_epoch() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

#[derive(Serialize, Deserialize, Debug)]
struct CachedIndex {
    packages: Vec<VurInfo>,
    cached_at: u64,
}

pub struct CacheIndex {
    path: PathBuf,
    env: heed::Env,
    db: Database<Str, SerdeBincode<CachedIndex>>,
}

impl CacheIndex {
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

    pub fn store(&mut self, key: &str, packages: Vec<VurInfo>) {
        if let Ok(mut txn) = self.env.write_txn() {
            let entry = CachedIndex { packages, cached_at: now_epoch() };
            let _ = self.db.put(&mut txn, key, &entry);
            let _ = txn.commit();
        }
    }

    pub fn load_valid(&self, key: &str, ttl_seconds: Option<u64>) -> Option<Vec<VurInfo>> {
        let txn = self.env.read_txn().ok()?;
        let cached = self.db.get(&txn, key).ok()??;
        
        if let Some(ttl) = ttl_seconds {
            if now_epoch().saturating_sub(cached.cached_at) >= ttl {
                return None;
            }
        }
        Some(cached.packages)
    }

    pub fn save(&self) -> Result<()> {
        Ok(())
    }
}
