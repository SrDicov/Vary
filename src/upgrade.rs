use crate::cache::CacheIndex;
use crate::config::Config;
use crate::db::InstalledDb;
use crate::reposconf::ReposConf;
use crate::vur_client::VurRepo;
use crate::xbps;
use anyhow::{Context, Result};

/// Diff de template pendiente de revisión en un upgrade (A3).
struct TemplateDiff {
    pkg: String,
    repo: String,
    patch: String,
}

/// Patch unificado viejo→nuevo con diffy; `None` si son idénticos.
/// Pura y testeable sin git.
fn template_diff_text(old: &str, new: &str) -> Option<String> {
    if old == new {
        return None;
    }
    Some(diffy::create_patch(old, new).to_string())
}

/// Muestra un patch por el paginador en TTY o plano si no.
fn show_patch(patch: &str) -> Result<()> {
    use std::io::{IsTerminal, Write};
    if std::io::stdout().is_terminal() {
        let (bin, args) = crate::review::pager_cmd();
        let mut pager = std::process::Command::new(&bin)
            .args(&args)
            .stdin(std::process::Stdio::piped())
            .spawn()
            .or_else(|_| {
                std::process::Command::new("less")
                    .args(["-R"])
                    .stdin(std::process::Stdio::piped())
                    .spawn()
            })
            .context("no se pudo abrir el paginador")?;
        if let Some(mut stdin) = pager.stdin.take() {
            let _ = stdin.write_all(patch.as_bytes());
        }
        let _ = pager.wait();
    } else {
        println!("{patch}");
    }
    Ok(())
}

/// Puerta de revisión de templates (A3): devuelve los paquetes aprobados.
/// En TTY muestra cada diff y pide sí explícito; fuera de TTY imprime plano
/// y aborta salvo `--yes`/`--noconfirm` (fail-closed como H-001/H-019).
fn review_template_diffs(config: &Config, diffs: &[TemplateDiff]) -> Result<Vec<String>> {
    use std::io::IsTerminal;
    let tty = std::io::stdout().is_terminal();
    let mut approved = Vec::new();
    for d in diffs {
        println!("--- template {} (repo {}) ---", d.pkg, d.repo);
        show_patch(&d.patch)?;
        if tty {
            if crate::util::ask(
                config,
                &format!("¿Aplicar upgrade de {} con estos cambios?", d.pkg),
                false,
            ) {
                approved.push(d.pkg.clone());
            } else {
                println!("upgrade de {} omitido por el usuario", d.pkg);
            }
        } else if config.no_confirm {
            approved.push(d.pkg.clone());
        } else {
            anyhow::bail!(
                "hay cambios de template sin revisar en entorno no interactivo; re-ejecuta con --yes para aceptarlos o revísalos en TTY"
            );
        }
    }
    Ok(approved)
}

pub fn refresh_repos(config: &Config) -> Result<i32> {
    let repos_conf = ReposConf::load(config.repos_conf_path())?;
    let mut any_failed = false;
    for (name, entry) in repos_conf.sorted_by_priority() {
        if !entry.enabled_or(true) {
            continue;
        }
        let path = config.vurs_dir().join(&name);
        let repo = VurRepo {
            name: name.clone(),
            path,
            entry: entry.clone(),
            git_bin: config.git_bin.clone(),
        };
        if let Err(e) = repo.ensure_cloned() {
            tracing::warn!("failed to clone VUR '{name}': {e}");
            any_failed = true;
            continue;
        }
        match repo.pull() {
            Ok(sha) => {
                println!("VUR '{}' refreshed ({})", name, &sha[..8]);
                // P0-2: continuidad de confianza tras actualizar. Un cambio
                // de llave/URL aborta TODO el refresh (fail-closed: no seguir
                // actualizando otros repos sobre una posible suplantación).
                // Los fallos de red ya avisaron y siguieron arriba; esto es
                // seguridad, no disponibilidad. El install queda doblemente
                // cubierto por el TOFU de setup.
                if let Some(st) = crate::keys::collect_trust_state(&repo, entry) {
                    crate::keys::check_trust(&st)?;
                }
            }
            Err(e) => {
                tracing::warn!("failed to pull VUR '{name}': {e}");
                any_failed = true;
            }
        }
    }
    if any_failed {
        Ok(1)
    } else {
        Ok(0)
    }
}

pub fn upgrade(config: &mut Config) -> Result<i32> {
    // Phase 1: official upgrade
    println!(":: Upgrading official packages...");
    let mut extra = vec!["-u".to_string()];
    if config.no_confirm {
        extra.push("-y".to_string());
    }
    let code = xbps::install(&[], &extra, &config.sudo_bin, &config.sudo_flags)?;
    if code != 0 {
        return Ok(code);
    }

    // Phase 2a: snapshot de templates instalados ANTES del pull (A3).
    // Tras el pull los viejos solo vivirían en objetos que el shallow puede
    // podar; el contenido se captura ahora vía git show HEAD.
    let mut db = InstalledDb::load(config.installed_db_path())?;
    let repos_conf = ReposConf::load(config.repos_conf_path())?;
    let mut old_templates: std::collections::HashMap<(String, String), String> =
        std::collections::HashMap::new();
    let mut parent_of: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    {
        let mut cache = CacheIndex::load(config.cache_index_path())?;
        for (name, entry) in repos_conf.sorted_by_priority() {
            let repo = VurRepo {
                name: name.clone(),
                path: config.vurs_dir().join(&name),
                entry: entry.clone(),
                git_bin: config.git_bin.clone(),
            };
            if repo.ensure_cloned().is_err() {
                continue;
            }
            if let Ok(infos) = repo.load_index(&mut cache, Some(config.ttl_cache_seconds)) {
                for info in infos {
                    let mut wanted = false;
                    for n in std::iter::once(&info.pkgname)
                        .chain(info.subpackages.iter().map(|s| &s.pkgname))
                    {
                        if db.get(n).is_some() {
                            parent_of.insert(n.clone(), info.pkgname.clone());
                            wanted = true;
                        }
                    }
                    if wanted {
                        if let Some(text) = repo.read_template(&info.pkgname) {
                            old_templates.insert((name.clone(), info.pkgname.clone()), text);
                        }
                    }
                }
            }
        }
        let _ = cache.save();
    }

    // Phase 2: refresh VUR repos
    println!(":: Refreshing VUR repositories...");
    refresh_repos(config)?;

    // Phase 3: detect VUR upgrades via installed.json
    let mut cache = CacheIndex::load(config.cache_index_path())?;

    // Build map of current VUR pkgver
    let mut current_map: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    for (name, entry) in repos_conf.sorted_by_priority() {
        let path = config.vurs_dir().join(&name);
        let repo = VurRepo {
            name: name.clone(),
            path,
            entry: entry.clone(),
            git_bin: config.git_bin.clone(),
        };
        if repo.ensure_cloned().is_err() {
            continue;
        }
        if let Ok(infos) = repo.load_index(&mut cache, Some(config.ttl_cache_seconds)) {
            for info in infos {
                current_map
                    .entry(info.pkgname.clone())
                    .or_insert_with(|| info.pkgver());
                for sub in &info.subpackages {
                    current_map
                        .entry(sub.pkgname.clone())
                        .or_insert_with(|| info.pkgver());
                }
            }
        }
    }
    let _ = cache.save();

    // Prune db entries for packages no longer installed
    let installed_names: Vec<String> = xbps::query_manual().unwrap_or_default();
    let manual_set: std::collections::HashSet<String> = installed_names.into_iter().collect();
    let before = db.len();
    db.retain(|name, _| {
        // Keep only if still installed or we can't determine (fallback keep)
        manual_set.contains(name) || {
            // Also check via xbps-query -l
            xbps::query_installed(name)
                .map(|o| o.is_some())
                .unwrap_or(true)
        }
    });
    if db.len() != before {
        let _ = db.save();
    }

    let mut outdated: Vec<String> = Vec::new();
    for (name, entry) in db.entries_snapshot() {
        if let Some(cur) = current_map.get(&name) {
            if cur != &entry.version {
                println!("  {} {} -> {}", name, entry.version, cur);
                outdated.push(name.clone());
            }
        }
    }

    if outdated.is_empty() {
        println!("VUR packages up to date.");
        return Ok(0);
    }

    // Phase 3b: puerta de revisión de templates (A3). Solo los paquetes cuyo
    // template cambió pasan por el diff-gate; el resto sigue como antes.
    let mut diffs = Vec::new();
    for name in &outdated {
        let parent = parent_of.get(name).cloned().unwrap_or_else(|| name.clone());
        let found = old_templates
            .iter()
            .find(|((_, p), _)| p == &parent)
            .map(|((r, _), old)| (r.clone(), old.clone()));
        let Some((repo_name, old)) = found else {
            continue;
        };
        let Some(entry) = repos_conf.vur.get(&repo_name) else {
            continue;
        };
        let repo = VurRepo {
            name: repo_name.clone(),
            path: config.vurs_dir().join(&repo_name),
            entry: entry.clone(),
            git_bin: config.git_bin.clone(),
        };
        if let Some(new) = repo.read_template(&parent) {
            if let Some(patch) = template_diff_text(&old, &new) {
                diffs.push(TemplateDiff {
                    pkg: name.clone(),
                    repo: repo_name.clone(),
                    patch,
                });
            }
        }
    }
    if !diffs.is_empty() {
        println!(
            "\n{} templates cambiaron en esta actualización; revisión obligatoria:",
            diffs.len()
        );
        let approved = review_template_diffs(config, &diffs)?;
        outdated.retain(|n| approved.contains(n));
        if outdated.is_empty() {
            println!("Sin upgrades aprobados.");
            return Ok(0);
        }
    }

    println!("\nVUR upgrades available: {}", outdated.join(", "));
    if !crate::util::ask(config, "Upgrade VUR packages?", true) {
        return Ok(1);
    }

    config.targets = outdated;
    crate::install::install(config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn diff_identico_da_none_cambio_da_patch() {
        // A3: el gate solo dispara con cambios reales.
        assert!(template_diff_text("a\n", "a\n").is_none());
        let patch = template_diff_text("pkgver=1\n", "pkgver=2\n").expect("patch");
        assert!(patch.contains("-pkgver=1"), "línea quitada: {patch}");
        assert!(patch.contains("+pkgver=2"), "línea puesta: {patch}");
    }

    fn un_diff() -> Vec<TemplateDiff> {
        vec![TemplateDiff {
            pkg: "foo".to_string(),
            repo: "mi-repo".to_string(),
            patch: "--- viejo\n+++ nuevo\n".to_string(),
        }]
    }

    #[test]
    fn review_no_tty_sin_yes_aborta() {
        // A3: en CI/pipes (stdout capturado, nunca TTY) sin --yes: abort.
        let config = Config::default();
        assert!(!config.no_confirm);
        assert!(review_template_diffs(&config, &un_diff()).is_err());
    }

    #[test]
    fn review_no_tty_con_yes_aprueba() {
        let config = Config {
            no_confirm: true,
            ..Config::default()
        };
        assert_eq!(
            review_template_diffs(&config, &un_diff()).unwrap(),
            vec!["foo".to_string()]
        );
    }
}
