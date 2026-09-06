use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

fn now_epoch_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn now_epoch_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum InstallType {
    Source,
    Binary,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Entry {
    pub version: String,
    pub vur: String,
    pub install_date: u64,
    /// Fecha de compilación en milisegundos desde UNIX_EPOCH (A5).
    /// Permite lógica de recompilación preventiva ante cambios en plantillas o deps.
    #[serde(default)]
    pub build_date: Option<u64>,
    pub install_type: InstallType,
}

pub const CURRENT_SCHEMA_VERSION: u32 = 2;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct InstalledDbFile {
    pub schema_version: u32,
    pub packages: BTreeMap<String, Entry>,
}

#[derive(Debug, Default)]
pub struct InstalledDb {
    path: PathBuf,
    entries: BTreeMap<String, Entry>,
}

impl InstalledDb {
    pub fn load(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        let mut entries = BTreeMap::new();

        // 1. Caso LMDB intermedio: si path es un directorio con data.mdb
        if path.is_dir() {
            let data_file = path.join("data.mdb");
            if data_file.is_file() {
                if let Ok(bytes) = std::fs::read(&data_file) {
                    let migrated = parse_lmdb_data_file(&bytes);
                    if !migrated.is_empty() {
                        tracing::info!("migradas {} entradas desde base de datos LMDB previa", migrated.len());
                        entries = migrated;
                    }
                }
                // Respaldo de seguridad del directorio LMDB antes de reemplazarlo por el archivo JSON
                let backup_path = path.with_extension("lmdb.bak");
                let _ = std::fs::rename(&path, &backup_path);
            }
        } else if path.is_file() {
            // 2. Caso archivo JSON existente: intentar v2 (InstalledDbFile), luego v1 (BTreeMap plano)
            match std::fs::read_to_string(&path) {
                Ok(text) => {
                    if let Ok(file_v2) = serde_json::from_str::<InstalledDbFile>(&text) {
                        entries = file_v2.packages;
                    } else if let Ok(legacy_v1) = serde_json::from_str::<BTreeMap<String, Entry>>(&text) {
                        tracing::info!("migradas {} entradas desde installed.json v1 legacy", legacy_v1.len());
                        entries = legacy_v1;
                        for ent in entries.values_mut() {
                            if ent.build_date.is_none() && ent.install_type == InstallType::Source && ent.install_date > 0 {
                                ent.build_date = Some(ent.install_date * 1000);
                            }
                        }
                    } else {
                        tracing::warn!(
                            "{} corrupta, preservando respaldo antes de iniciar vacía",
                            path.display()
                        );
                        let corrupt_bak = path.with_extension(format!("corrupt.{}", now_epoch_secs()));
                        let _ = std::fs::copy(&path, &corrupt_bak);
                    }
                }
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => {
                    return Err(err).with_context(|| format!("no se pudo leer {}", path.display()));
                }
            }
        } else {
            // Si no existe path directo pero existe un installed.db hermano
            let alternate_lmdb = path.with_extension("db");
            if alternate_lmdb.is_dir() {
                let data_file = alternate_lmdb.join("data.mdb");
                if data_file.is_file() {
                    if let Ok(bytes) = std::fs::read(&data_file) {
                        entries = parse_lmdb_data_file(&bytes);
                    }
                }
            }
        }

        let db = Self { path, entries };
        // Si veníamos de migrar un LMDB directorio, persistir inmediatamente al formato JSON atómico
        if !db.path.is_file() && !db.entries.is_empty() {
            let _ = db.save();
        }
        Ok(db)
    }

    pub fn save(&self) -> Result<()> {
        let parent = self.path.parent().unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(parent)
            .with_context(|| format!("no se pudo crear directorio {}", parent.display()))?;

        let file_repr = InstalledDbFile {
            schema_version: CURRENT_SCHEMA_VERSION,
            packages: self.entries.clone(),
        };
        let json_bytes = serde_json::to_vec_pretty(&file_repr)?;

        // Escritura atómica vía tempfile en el mismo directorio + fsync + persist/rename
        let mut temp = tempfile::NamedTempFile::new_in(parent)
            .with_context(|| format!("creando archivo temporal en {}", parent.display()))?;
        use std::io::Write;
        temp.write_all(&json_bytes)
            .with_context(|| format!("escribiendo archivo temporal en {}", parent.display()))?;
        temp.flush()?;
        temp.as_file().sync_all()?;
        temp.persist(&self.path)
            .map_err(|e| e.error)
            .with_context(|| format!("renombrando atómicamente a {}", self.path.display()))?;
        Ok(())
    }

    pub fn upsert(&mut self, name: &str, version: &str, vur: &str, install_type: InstallType) {
        let now_ms = now_epoch_ms();
        let build_date = match install_type {
            InstallType::Source => Some(now_ms),
            InstallType::Binary => None,
        };
        self.entries.insert(
            name.to_string(),
            Entry {
                version: version.to_string(),
                vur: vur.to_string(),
                install_date: now_ms / 1000,
                build_date,
                install_type,
            },
        );
    }

    #[allow(dead_code)]
    pub fn get(&self, name: &str) -> Option<&Entry> {
        self.entries.get(name)
    }

    #[allow(dead_code)]
    pub fn get_build_date(&self, name: &str) -> Option<u64> {
        self.entries.get(name).and_then(|e| e.build_date)
    }

    pub fn remove(&mut self, name: &str) -> bool {
        self.entries.remove(name).is_some()
    }

    #[allow(dead_code)]
    pub fn names(&self) -> Vec<String> {
        self.entries.keys().cloned().collect()
    }

    pub fn retain(&mut self, pred: impl Fn(&str, &Entry) -> bool) {
        self.entries.retain(|name, entry| pred(name, entry));
    }

    pub fn entries_snapshot(&self) -> Vec<(String, Entry)> {
        self.entries.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

pub fn decode_bincode_entry(data: &[u8]) -> Option<Entry> {
    let mut cursor = 0;
    if data.len() < cursor + 8 { return None; }
    let v_len = u64::from_le_bytes(data[cursor..cursor+8].try_into().ok()?) as usize;
    cursor += 8;
    if data.len() < cursor + v_len { return None; }
    let version = std::str::from_utf8(&data[cursor..cursor+v_len]).ok()?.to_string();
    cursor += v_len;

    if data.len() < cursor + 8 { return None; }
    let vur_len = u64::from_le_bytes(data[cursor..cursor+8].try_into().ok()?) as usize;
    cursor += 8;
    if data.len() < cursor + vur_len { return None; }
    let vur = std::str::from_utf8(&data[cursor..cursor+vur_len]).ok()?.to_string();
    cursor += vur_len;

    if data.len() < cursor + 8 { return None; }
    let install_date = u64::from_le_bytes(data[cursor..cursor+8].try_into().ok()?);
    cursor += 8;

    if data.len() < cursor + 4 { return None; }
    let itype_val = u32::from_le_bytes(data[cursor..cursor+4].try_into().ok()?);
    let install_type = match itype_val {
        0 => InstallType::Source,
        1 => InstallType::Binary,
        _ => return None,
    };

    Some(Entry {
        version,
        vur,
        install_date,
        build_date: if install_type == InstallType::Source { Some(install_date * 1000) } else { None },
        install_type,
    })
}

pub fn parse_lmdb_data_file(data: &[u8]) -> BTreeMap<String, Entry> {
    let mut entries = BTreeMap::new();
    let page_size = 4096;
    let mut offset = page_size * 2;
    while offset + page_size <= data.len() {
        let page = &data[offset..offset + page_size];
        offset += page_size;
        let flags = u16::from_le_bytes([page[10], page[11]]);
        if (flags & 0x02) != 0 && (flags & 0x01) == 0 {
            let lower = u16::from_le_bytes([page[12], page[13]]) as usize;
            if lower >= 16 && lower <= page_size {
                let mut indx_offset = 16;
                while indx_offset + 2 <= lower {
                    let node_pos = u16::from_le_bytes([page[indx_offset], page[indx_offset + 1]]) as usize;
                    indx_offset += 2;
                    if node_pos + 8 <= page_size {
                        let val_size = u32::from_le_bytes([page[node_pos], page[node_pos+1], page[node_pos+2], page[node_pos+3]]) as usize;
                        let ksize = u16::from_le_bytes([page[node_pos+6], page[node_pos+7]]) as usize;
                        let key_start = node_pos + 8;
                        let val_start = key_start + ksize;
                        if val_start + val_size <= page_size && ksize > 0 && val_size >= 28 {
                            if let Ok(key) = std::str::from_utf8(&page[key_start..key_start + ksize]) {
                                if let Some(entry) = decode_bincode_entry(&page[val_start..val_start + val_size]) {
                                    entries.insert(key.to_string(), entry);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_preserves_entries_and_build_date() {
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
        assert!(hello.build_date.is_some());
        assert!(reloaded.get_build_date("hello-vur").unwrap() > 1_500_000_000_000);

        let kernel = reloaded.get("kernel-vur").unwrap();
        assert_eq!(kernel.install_type, InstallType::Binary);
        assert!(kernel.build_date.is_none());

        let raw = std::fs::read_to_string(&path).unwrap();
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(v["schema_version"], 2);
        assert!(v["packages"]["hello-vur"].is_object());
    }

    #[test]
    fn migrate_legacy_v1_json() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("installed.json");
        let legacy_json = r#"{
            "legacy-pkg": {
                "version": "2.0_1",
                "vur": "custom",
                "install_date": 1700000000,
                "install_type": "source"
            }
        }"#;
        std::fs::write(&path, legacy_json).unwrap();

        let db = InstalledDb::load(&path).unwrap();
        assert_eq!(db.len(), 1);
        let entry = db.get("legacy-pkg").unwrap();
        assert_eq!(entry.version, "2.0_1");
        assert_eq!(entry.build_date, Some(1700000000000));
    }

    #[test]
    fn retain_and_remove() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("installed.json");
        let mut db = InstalledDb::load(&path).unwrap();
        db.upsert("pkg-a", "1.0", "r", InstallType::Source);
        db.upsert("pkg-b", "1.0", "r", InstallType::Binary);
        assert_eq!(db.len(), 2);

        assert!(db.remove("pkg-a"));
        assert!(!db.remove("pkg-a"));
        assert_eq!(db.len(), 1);

        db.retain(|name, _| name == "none");
        assert_eq!(db.len(), 0);
    }

    #[test]
    fn corrupt_db_preserves_backup_and_starts_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("installed.json");
        std::fs::write(&path, "not valid json {{{").unwrap();

        let db = InstalledDb::load(&path).unwrap();
        assert_eq!(db.len(), 0);

        let entries: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert!(entries.iter().any(|name| name.contains("corrupt")));
    }

    #[test]
    fn decode_bincode_entry_extracts_correct_fields() {
        let mut data = Vec::new();
        data.extend_from_slice(&(4u64).to_le_bytes());
        data.extend_from_slice(b"1.01");
        data.extend_from_slice(&(3u64).to_le_bytes());
        data.extend_from_slice(b"vur");
        data.extend_from_slice(&(1700000000u64).to_le_bytes());
        data.extend_from_slice(&(0u32).to_le_bytes());

        let entry = decode_bincode_entry(&data).unwrap();
        assert_eq!(entry.version, "1.01");
        assert_eq!(entry.vur, "vur");
        assert_eq!(entry.install_date, 1700000000);
        assert_eq!(entry.build_date, Some(1700000000000));
        assert_eq!(entry.install_type, InstallType::Source);
    }
}
