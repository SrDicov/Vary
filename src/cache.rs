use crate::metadata::VurInfo;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn now_epoch() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct CachedIndex {
    packages: Vec<VurInfo>,
    cached_at: u64,
}

pub struct CacheIndex {
    path: PathBuf,
    entries: BTreeMap<String, CachedIndex>,
}

impl CacheIndex {
    pub fn load(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        let mut cache = Self {
            path,
            entries: BTreeMap::new(),
        };
        if cache.path.is_dir() {
            // Antiguo directorio LMDB intermedio: limpiar directorio sin error
            let _ = std::fs::remove_dir_all(&cache.path);
            return Ok(cache);
        }
        let text = match std::fs::read_to_string(&cache.path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(cache),
            Err(err) => {
                return Err(err)
                    .with_context(|| format!("no se pudo leer {}", cache.path.display()));
            }
        };
        if let Ok(entries) = serde_json::from_str(&text) {
            cache.entries = entries;
        }
        Ok(cache)
    }

    pub fn store(&mut self, key: &str, packages: Vec<VurInfo>) {
        self.entries.insert(
            key.to_string(),
            CachedIndex {
                packages,
                cached_at: now_epoch(),
            },
        );
    }

    pub fn load_valid(&self, key: &str, ttl_seconds: Option<u64>) -> Option<Vec<VurInfo>> {
        let cached = self.entries.get(key)?;
        if let Some(ttl) = ttl_seconds {
            let age = now_epoch().saturating_sub(cached.cached_at);
            if age >= ttl {
                return None;
            }
        }
        Some(cached.packages.clone())
    }

    pub fn invalidate_repo(&mut self, repo_name: &str) {
        let prefix = format!("{}:", repo_name);
        self.entries.retain(|key, _| !key.starts_with(&prefix));
    }

    pub fn save(&self) -> Result<()> {
        let parent = self.path.parent().unwrap_or_else(|| std::path::Path::new("."));
        std::fs::create_dir_all(parent)
            .with_context(|| format!("no se pudo crear {}", parent.display()))?;

        let json_bytes = serde_json::to_vec_pretty(&self.entries)?;
        let mut temp = tempfile::NamedTempFile::new_in(parent)
            .with_context(|| format!("creando archivo temporal en {}", parent.display()))?;
        use std::io::Write;
        temp.write_all(&json_bytes)?;
        temp.flush()?;
        temp.as_file().sync_all()?;
        temp.persist(&self.path).map_err(|e| e.error)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_roundtrip_and_expiry() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.json");
        let mut cache = CacheIndex::load(&path).unwrap();

        assert!(cache.load_valid("repo:key", None).is_none());
        cache.store("repo:key", vec![]);
        cache.save().unwrap();

        let reloaded = CacheIndex::load(&path).unwrap();
        assert!(reloaded.load_valid("repo:key", Some(3600)).is_some());
        assert!(reloaded.load_valid("repo:key", Some(0)).is_none());

        let mut cache2 = reloaded;
        cache2.invalidate_repo("repo");
        assert!(cache2.load_valid("repo:key", None).is_none());
    }
}
