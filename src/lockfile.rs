//! Lockfile reproducible del árbol instalado (P1-1).
//!
//! `vary.lock` (TOML en `~/.config/vary/`) congela lo necesario para
//! reproducir el árbol en otra máquina: repos (url/commit/fingerprint) y
//! paquetes (versión/origen/sha256). Sin timestamps dentro: generar dos
//! veces da bytes idénticos (propiedad de reproducibilidad, verificada en
//! vivo).
//!
//! * `vary --lock` (pelado) regenera desde el estado actual.
//! * `vary -S... --lock` / `-Sy --lock` regeneran al final con éxito.
//! * Con lock presente, install/upgrade/`-Sp` AVISAN (no bloquean) ante
//!   divergencias de versiones o commits (el lock verifica lo pineado;
//!   paquetes nuevos no avisan para no fatigar).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::config::Config;

/// Repo pineado: de dónde vino y en qué commit/llave se confió.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct LockedRepo {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
}

/// Paquete pineado: qué versión, de dónde y con qué hash.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct LockedPkg {
    pub version: String,
    /// `official` o `vur:<repo>`.
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
}

/// Contenido de `vary.lock` (claves ordenadas: salida determinista).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct Lockfile {
    #[serde(default)]
    pub repos: BTreeMap<String, LockedRepo>,
    #[serde(default)]
    pub packages: BTreeMap<String, LockedPkg>,
}

impl Lockfile {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("no se pudo leer {}", path.display()))?;
        toml::from_str(&text).with_context(|| format!("TOML inválido en {}", path.display()))
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                crate::util::ensure_private_dir(parent)
                    .with_context(|| format!("no se pudo crear {}", parent.display()))?;
            }
        }
        std::fs::write(path, toml::to_string_pretty(self)?)
            .with_context(|| format!("no se pudo escribir {}", path.display()))?;
        Ok(())
    }
}

/// P1-1: genera el lock desde el estado actual (solo lecturas salvo el
/// propio archivo). Repos habilitados + DB de vary + explícitos de xbps.
pub fn generate(config: &Config) -> Result<Lockfile> {
    let mut lock = Lockfile::default();

    let repos_conf = crate::reposconf::ReposConf::load(config.repos_conf_path())?;
    for (name, entry) in repos_conf.sorted_by_priority() {
        if !entry.enabled_or(true) {
            continue;
        }
        let clone_path = config.vurs_dir().join(&name);
        let commit = if clone_path.join(".git").exists() {
            crate::install::repo_head_commit(&clone_path, &config.git_bin)
        } else {
            None
        };
        let fingerprint = if crate::keys::binary_repo_registered(&name) {
            crate::keys::key_dest_path(&name)
                .ok()
                .and_then(|p| std::fs::read_to_string(p).ok())
                .and_then(|pem| crate::keys::xbps_fingerprint_pem(&pem).ok())
        } else {
            None
        };
        lock.repos.insert(
            name.clone(),
            LockedRepo {
                url: entry.url.clone(),
                commit,
                fingerprint,
            },
        );
    }

    let db = crate::db::InstalledDb::load(config.installed_db_path())?;
    for (name, entry) in db.entries_snapshot() {
        lock.packages.insert(
            name,
            LockedPkg {
                version: entry.version,
                source: format!("vur:{}", entry.vur),
                sha256: entry.artifact_sha256,
            },
        );
    }
    // Explícitos de xbps no gestionados por vary (típicamente oficiales).
    // OJO: `xbps-query -m` rinde pkgvers (`nombre-versión_rev`), no nombres:
    // la clave es siempre `info.name` (así deduplica contra la DB).
    if let Ok(manual) = crate::xbps::query_manual() {
        for pkgver in manual {
            if let Ok(Some(info)) = crate::xbps::query_installed(&pkgver) {
                if lock.packages.contains_key(&info.name) {
                    continue;
                }
                lock.packages.insert(
                    info.name.clone(),
                    LockedPkg {
                        version: info.pkgver,
                        source: "official".to_string(),
                        sha256: None,
                    },
                );
            }
        }
    }

    Ok(lock)
}

/// P1-1: verifica plan + commits contra el lock si existe (avisos).
/// `repos`: (nombre, path del clon) para leer HEADs; `items`: (pkgname,
/// pkgver) resueltos. Sin archivo lock: silencio total. Lock ilegible:
/// un aviso accionable (no silencio cómplice).
pub fn verify_if_pinned(
    config: &Config,
    repos: &[(String, PathBuf)],
    items: &[(String, String)],
) -> Vec<String> {
    let path = config.lock_path();
    if !path.is_file() {
        return Vec::new();
    }
    let lock = match Lockfile::load(&path) {
        Ok(l) => l,
        Err(e) => {
            return vec![format!(
                "lock: {} ilegible ({e:#}); regenera con `vary --lock`",
                path.display()
            )];
        }
    };
    let heads: Vec<(String, Option<String>)> = repos
        .iter()
        .map(|(name, clone_path)| {
            (
                name.clone(),
                crate::install::repo_head_commit(clone_path, &config.git_bin),
            )
        })
        .collect();
    verify_plan(&lock, items, &heads)
}

/// P1-1: `vary --lock` — regenera el lock desde el estado actual.
pub fn regenerate(config: &Config) -> Result<i32> {
    let lock = generate(config)?;
    let path = config.lock_path();
    lock.save(&path)?;
    println!(
        "vary.lock actualizado en {} ({} repos, {} paquetes)",
        path.display(),
        lock.repos.len(),
        lock.packages.len()
    );
    Ok(0)
}

/// P1-1: verifica un plan resuelto contra el lock (puro; avisos, no errores).
/// Solo verifica lo pineado: versiones y commits. Paquetes no pineados
/// (nuevos) no avisan para no fatigar (`vary --lock` tras cambios).
pub fn verify_plan(
    lock: &Lockfile,
    items: &[(String, String)],
    heads: &[(String, Option<String>)],
) -> Vec<String> {
    let mut warnings = Vec::new();
    for (name, pkgver) in items {
        if let Some(pinned) = lock.packages.get(name) {
            if pinned.version != *pkgver {
                warnings.push(format!(
                    "lock: '{name}' resuelve {pkgver} pero vary.lock pinea {} (regenera con `vary --lock`)",
                    pinned.version
                ));
            }
        }
    }
    for (repo, head) in heads {
        if let Some(pinned) = lock.repos.get(repo) {
            match (&pinned.commit, head) {
                (Some(a), Some(b)) if a != b => warnings.push(format!(
                    "lock: repo '{repo}' cambió de commit ({a:.12} -> {b:.12}); regenera con `vary --lock`"
                )),
                _ => {}
            }
        }
    }
    warnings
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_lock() -> Lockfile {
        let mut lock = Lockfile::default();
        lock.repos.insert(
            "mi-repo".to_string(),
            LockedRepo {
                url: "https://example.com/r.git".to_string(),
                commit: Some("aaa111".to_string()),
                fingerprint: Some("11:22".to_string()),
            },
        );
        lock.packages.insert(
            "app".to_string(),
            LockedPkg {
                version: "app-1.0_1".to_string(),
                source: "vur:mi-repo".to_string(),
                sha256: Some("sha256:abc".to_string()),
            },
        );
        lock
    }

    #[test]
    fn roundtrip_es_determinista() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vary.lock");
        let lock = sample_lock();
        lock.save(&path).unwrap();
        let raw1 = std::fs::read(&path).unwrap();
        // Regenerar el texto dos veces: bytes idénticos (sin timestamps).
        lock.save(&path).unwrap();
        let raw2 = std::fs::read(&path).unwrap();
        assert_eq!(raw1, raw2);
        let reloaded = Lockfile::load(&path).unwrap();
        assert_eq!(reloaded, lock);
        // TOML inválido: error, no default silencioso.
        std::fs::write(&path, "esto no es = [toml").unwrap();
        assert!(Lockfile::load(&path).is_err());
    }

    #[test]
    fn verify_solo_mismatch_pineado() {
        let lock = sample_lock();
        // Idéntico: silencio.
        let w = verify_plan(
            &lock,
            &[("app".to_string(), "app-1.0_1".to_string())],
            &[("mi-repo".to_string(), Some("aaa111".to_string()))],
        );
        assert!(w.is_empty(), "{w:?}");
        // Versión distinta: avisa.
        let w = verify_plan(
            &lock,
            &[("app".to_string(), "app-2.0_1".to_string())],
            &[("mi-repo".to_string(), Some("aaa111".to_string()))],
        );
        assert_eq!(w.len(), 1, "{w:?}");
        assert!(w[0].contains("app-2.0_1"), "{w:?}");
        // Commit distinto: avisa.
        let w = verify_plan(
            &lock,
            &[("app".to_string(), "app-1.0_1".to_string())],
            &[("mi-repo".to_string(), Some("bbb222".to_string()))],
        );
        assert_eq!(w.len(), 1, "{w:?}");
        assert!(w[0].contains("bbb222"), "{w:?}");
        // Paquete no pineado (nuevo): silencio.
        let w = verify_plan(
            &lock,
            &[("nuevo".to_string(), "nuevo-1.0_1".to_string())],
            &[],
        );
        assert!(w.is_empty(), "{w:?}");
        // Lock vacío: todo silencio.
        let w = verify_plan(
            &Lockfile::default(),
            &[("app".to_string(), "app-9.0_1".to_string())],
            &[],
        );
        assert!(w.is_empty(), "{w:?}");
    }

    #[test]
    fn verify_if_pinned_match_mismatch_y_nuevo() {
        // D1: `verify_if_pinned` contra lock temporal; `repos` vacío para no
        // invocar git real (el camino de commits lo cubre `verify_plan`).
        let dir = tempfile::tempdir().unwrap();
        let config = crate::config::Config {
            config_dir: dir.path().to_path_buf(),
            ..Default::default()
        };
        // Sin lock: silencio total.
        let w = verify_if_pinned(
            &config,
            &[],
            &[("app".to_string(), "app-1.0_1".to_string())],
        );
        assert!(w.is_empty(), "{w:?}");
        sample_lock().save(&config.lock_path()).unwrap();
        // Match pineado: silencio.
        let w = verify_if_pinned(
            &config,
            &[],
            &[("app".to_string(), "app-1.0_1".to_string())],
        );
        assert!(w.is_empty(), "{w:?}");
        // Mismatch de versión: avisa citando paquete y versiones.
        let w = verify_if_pinned(
            &config,
            &[],
            &[("app".to_string(), "app-2.0_1".to_string())],
        );
        assert_eq!(w.len(), 1, "{w:?}");
        assert!(w[0].contains("'app'"), "{w:?}");
        assert!(w[0].contains("app-2.0_1"), "{w:?}");
        assert!(w[0].contains("app-1.0_1"), "{w:?}");
        // Paquete nuevo no pineado: silencio anti-fatiga.
        let w = verify_if_pinned(
            &config,
            &[],
            &[("nuevo".to_string(), "nuevo-1.0_1".to_string())],
        );
        assert!(w.is_empty(), "{w:?}");
    }

    #[test]
    fn verify_if_pinned_lock_ilegible_avisa() {
        let dir = tempfile::tempdir().unwrap();
        let config = crate::config::Config {
            config_dir: dir.path().to_path_buf(),
            ..Default::default()
        };
        std::fs::write(config.lock_path(), "esto no es = [toml").unwrap();
        let w = verify_if_pinned(&config, &[], &[]);
        assert_eq!(w.len(), 1, "{w:?}");
        assert!(w[0].contains("ilegible"), "{w:?}");
        assert!(w[0].contains("vary --lock"), "{w:?}");
    }
}
