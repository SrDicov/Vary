use crate::metadata::VurInfo;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn now_epoch() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

#[derive(Serialize, Deserialize, Debug)]
struct CachedIndex {
    packages: Vec<VurInfo>,
    /// Unix epoch seconds del momento del cacheo.
    cached_at: u64,
}

#[derive(Debug, Default)]
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
        let text = match std::fs::read_to_string(&cache.path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                tracing::warn!("{} no existe, iniciando caché vacía", cache.path.display());
                return Ok(cache);
            }
            Err(err) => {
                return Err(err)
                    .with_context(|| format!("no se pudo leer {}", cache.path.display()));
            }
        };
        match serde_json::from_str(&text) {
            Ok(entries) => cache.entries = entries,
            Err(err) => tracing::warn!(
                "{} corrupta ({err}), iniciando caché vacía",
                cache.path.display()
            ),
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

    pub fn load_valid(&self, key: &str, ttl_seconds: Option<u64>) -> Option<&[VurInfo]> {
        let cached = self.entries.get(key)?;
        if let Some(ttl) = ttl_seconds {
            let age = now_epoch().saturating_sub(cached.cached_at);
            if age >= ttl {
                return None;
            }
        }
        Some(cached.packages.as_slice())
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::VurInfo;

    fn mk_pkg(name: &str) -> VurInfo {
        VurInfo {
            format_version: 1,
            pkgname: name.to_string(),
            version: "1.0_1".to_string(),
            revision: 1,
            archs: vec!["x86_64".to_string()],
            subpackages: vec![],
            depends: vec![],
            hostmakedepends: vec![],
            makedepends: vec![],
            checkdepends: vec![],
            build_style: None,
            distfiles: vec![],
            checksum: vec![],
            provides: vec![],
            replaces: vec![],
            restricted: false,
            maintainer: None,
        }
    }

    #[test]
    fn store_and_load_within_ttl() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.json");
        let mut cache = CacheIndex::load(&path).unwrap();
        assert!(cache.load_valid("mi-repo:abc", None).is_none());

        let pkgs = vec![mk_pkg("hello"), mk_pkg("world")];
        cache.store("mi-repo:abc", pkgs.clone());

        assert_eq!(cache.load_valid("mi-repo:abc", None), Some(pkgs.as_slice()));
        assert_eq!(
            cache.load_valid("mi-repo:abc", Some(3600)),
            Some(pkgs.as_slice())
        );
        assert_eq!(cache.load_valid("mi-repo:abc", Some(60)).unwrap().len(), 2);
        assert!(cache.load_valid("otra-clave", None).is_none());
        assert!(cache.load_valid("mi-repo:abc", Some(0)).is_none());
    }

    #[test]
    fn ttl_expiry_is_honored() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.json");
        let mut cache = CacheIndex::load(&path).unwrap();
        cache.store("k", vec![mk_pkg("x")]);

        let stale = now_epoch() - 120;
        cache.entries.get_mut("k").unwrap().cached_at = stale;

        assert!(cache.load_valid("k", Some(60)).is_none());
        assert_eq!(cache.load_valid("k", Some(600)).unwrap().len(), 1);
        assert_eq!(cache.load_valid("k", None).unwrap().len(), 1);
    }

    #[test]
    fn persistence_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.json");
        let mut cache = CacheIndex::load(&path).unwrap();
        let pkgs = vec![mk_pkg("persistido")];
        cache.store("repo:deadbeef", pkgs.clone());
        cache.save().unwrap();

        let reloaded = CacheIndex::load(&path).unwrap();
        assert_eq!(
            reloaded.load_valid("repo:deadbeef", None),
            Some(pkgs.as_slice())
        );
    }

    #[test]
    fn corrupt_cache_starts_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.json");
        std::fs::write(&path, "{{{").unwrap();
        let cache = CacheIndex::load(&path).unwrap();
        assert!(cache.load_valid("k", None).is_none());
    }
}
