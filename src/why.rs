//! `vary --why <pkg>`: quién trajo este paquete (P2).
//!
//! Cruza tres fuentes (solo lecturas, sin lock): la DB de vary (¿gestionado
//! y con qué pins?), revdeps instalados (`xbps-query -X`) y plantillas VUR
//! que lo declaran como dependencia. Informativo: no muta nada.

use anyhow::{Context, Result};

use crate::cache::CacheIndex;
use crate::config::Config;
use crate::resolver::dep_name;

/// P2: ¿qué plantillas declaran `target` en sus dependencias?
/// `templates`: (repo, pkgname, strings de dep tal cual). Puro y testeable.
pub fn find_declarers(
    target: &str,
    templates: &[(&str, &str, Vec<String>)],
) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (repo, pkgname, deps) in templates {
        if deps.iter().any(|d| dep_name(d) == target) {
            out.push((repo.to_string(), pkgname.to_string()));
        }
    }
    out.sort();
    out.dedup();
    out
}

/// P2: explica la procedencia de `pkg`. Requiere exactamente un objetivo
/// (lo valida el dispatch); aquí solo se informa.
pub fn explain(config: &Config, pkg: &str) -> Result<i32> {
    if !crate::metadata::is_valid_pkgname(pkg) {
        anyhow::bail!(
            "nombre de paquete inválido: '{pkg}' (debe coincidir con ^[a-zA-Z0-9][a-zA-Z0-9._+-]*$)"
        );
    }
    println!("why {pkg}:");

    // 1. ¿gestionado por vary?
    let db = crate::db::InstalledDb::load(config.installed_db_path())?;
    match db.get(pkg) {
        Some(e) => println!(
            "  gestionado-por-vary: {} [{}] ({:?})",
            e.version, e.vur, e.install_type
        ),
        None => println!("  gestionado-por-vary: no (lo gestiona xbps)"),
    }

    // 2. Revdeps instalados (nativo xbps; se muestra tal cual los reporta).
    match installed_rdeps(pkg) {
        Ok(rdeps) if !rdeps.is_empty() => {
            println!("  requerido-por-instalados [xbps -X]:");
            for r in &rdeps {
                println!("    {r}");
            }
        }
        _ => println!("  requerido-por-instalados: (ninguno)"),
    }

    // 3. Plantillas VUR que lo declaran (índices tal cual, sin fetch).
    let mut cache = CacheIndex::load(config.cache_index_path())?;
    let mut declarers: Vec<(String, String)> = Vec::new();
    if let Ok(repos_conf) = crate::reposconf::ReposConf::load(config.repos_conf_path()) {
        for (name, entry) in repos_conf.sorted_by_priority() {
            if !entry.enabled_or(true) {
                continue;
            }
            let repo = crate::vur_client::VurRepo {
                name: name.clone(),
                path: config.vurs_dir().join(&name),
                entry: entry.clone(),
                git_bin: config.git_bin.clone(),
            };
            match repo.load_index(&mut cache, None) {
                Ok(infos) => {
                    let templates: Vec<(&str, &str, Vec<String>)> = infos
                        .iter()
                        .map(|info| {
                            let mut deps = Vec::new();
                            deps.extend(info.hostmakedepends.iter().cloned());
                            deps.extend(info.makedepends.iter().cloned());
                            deps.extend(info.depends.iter().cloned());
                            (name.as_str(), info.pkgname.as_str(), deps)
                        })
                        .collect();
                    declarers.extend(find_declarers(pkg, &templates));
                }
                Err(e) => tracing::warn!("why: sin índice de '{name}': {e:#}"),
            }
        }
    }
    // (Sin guardar caché: solo lectura.)
    match declarers.as_slice() {
        [] => println!("  declarado-en-plantillas-VUR: (ninguno)"),
        ds => {
            println!("  declarado-en-plantillas-VUR:");
            for (repo, template) in ds {
                println!("    {repo}/{template}");
            }
        }
    }
    Ok(0)
}

/// Revdeps instalados vía `xbps-query -X` (una línea por paquete).
fn installed_rdeps(pkg: &str) -> Result<Vec<String>> {
    let out = std::process::Command::new("xbps-query")
        .args(["-X", "--", pkg])
        .output()
        .context("ejecutando xbps-query -X")?;
    if !out.status.success() {
        return Ok(Vec::new());
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_declarers_recorta_constraints_y_ordena() {
        // P2: tabla — matchea por nombre base, ordena y deduplica.
        let templates = vec![
            (
                "r2",
                "app",
                vec!["lib>=2.0".to_string(), "otro".to_string()],
            ),
            ("r1", "lib", vec!["base".to_string()]),
            ("r1", "app2", vec!["lib <3".to_string()]),
            ("r1", "app", vec!["lib>=2.0".to_string()]),
        ];
        let found = find_declarers("lib", &templates);
        assert_eq!(
            found,
            vec![
                ("r1".to_string(), "app".to_string()),
                ("r1".to_string(), "app2".to_string()),
                ("r2".to_string(), "app".to_string()),
            ]
        );
        // Virtuales matchean por nombre pedido.
        let templates = vec![("r", "a", vec!["libfoo.so.1".to_string()])];
        assert_eq!(
            find_declarers("libfoo.so.1", &templates),
            vec![("r".to_string(), "a".to_string())]
        );
        // Sin declarantes: vacío.
        assert!(find_declarers("nadie", &templates).is_empty());
    }
}
