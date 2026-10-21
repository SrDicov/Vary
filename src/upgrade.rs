use crate::cache::CacheIndex;
use crate::config::Config;
use crate::db::{InstallType, InstalledDb};
use crate::reposconf::ReposConf;
use crate::vur_client::VurRepo;
use crate::xbps;
use anyhow::{Context, Result};

/// Parsea `version=` y `revision=` de la salida de `xbps-src show <pkg>`.
/// Pura y testeable con fixtures; `revision` ausente o no numérica => 1
/// (el default del template histórico); sin `version=` => None.
pub(crate) fn parse_template_version(text: &str) -> Option<(String, u32)> {
    let mut version = None;
    let mut revision = 1u32;
    for line in text.lines() {
        let line = line.trim();
        // `xbps-src show` emite `version:\t1.2`; templates e índices VUR
        // históricos usan `version=1.2`. Aceptamos ambos separadores.
        let Some(pos) = line.find([':', '=']) else {
            continue;
        };
        let key = line[..pos].trim();
        let value = line[pos + 1..].trim();
        match key {
            "version" => version = Some(value.to_string()),
            "revision" => revision = value.parse().unwrap_or(1),
            _ => {}
        }
    }
    version.filter(|v| !v.is_empty()).map(|v| (v, revision))
}

/// `name-<ver>_<rev>` de xbps => `(ver, rev)`. Puro y testeable.
fn split_installed_pkgver(name: &str, pkgver: &str) -> Option<(String, u32)> {
    let rest = pkgver.strip_prefix(name)?.strip_prefix('-')?;
    let (ver, rev) = rest.rsplit_once('_')?;
    Some((ver.to_string(), rev.parse().ok()?))
}

/// Fase híbrida del upgrade: oficiales registrados como Source en la DB que
/// matcheen `source_always` y cuyo template del clon (vía `xbps-src show`)
/// sea MÁS NUEVO que lo instalado se acumulan en `targets` — el plan los
/// emite como `BuildOfficial` y `xbps-src` resuelve sus deps y el artefacto.
/// Best-effort por paquete: fallo de show/parseo/xbps => warn y omitir.
fn add_official_source_upgrades(
    config: &Config,
    db: &InstalledDb,
    vur_managed: &std::collections::HashSet<String>,
    targets: &mut Vec<String>,
) {
    if config.source_always.is_empty() {
        return;
    }
    let md = crate::masterdir::Masterdir::new(config.void_packages_dir());
    for (name, entry) in db.entries_snapshot() {
        if entry.install_type != InstallType::Source
            || !config
                .source_always
                .iter()
                .any(|p| crate::config::matches_glob(p, &name))
            || vur_managed.contains(&name)
            || targets.contains(&name)
        {
            continue;
        }
        // Solo oficial: gestionado por un VUR lo maneja la fase VUR normal.
        let installed = match xbps::query_installed(&name) {
            Ok(Some(cur)) => cur,
            _ => {
                tracing::warn!(
                    "'{name}': no consultable vía xbps-query; omitido del upgrade híbrido"
                );
                continue;
            }
        };
        let Some((iver, irev)) = split_installed_pkgver(&name, &installed.pkgver) else {
            tracing::warn!(
                "'{name}': pkgver instalado '{}' no parseable; omitido",
                installed.pkgver
            );
            continue;
        };
        let (tver, trev) = match md
            .show_pkg(&name)
            .ok()
            .as_deref()
            .and_then(parse_template_version)
        {
            Some(v) => v,
            None => {
                tracing::warn!(
                    "'{name}': `xbps-src show` falló o sin version=/revision=; omitido del upgrade híbrido"
                );
                continue;
            }
        };
        // Template más nuevo que lo instalado => recompilar desde fuente.
        if crate::resolver::cmp_pkgver(&tver, trev, &iver, irev) != std::cmp::Ordering::Greater {
            continue;
        }
        println!("  {name} (oficial desde fuente) {iver}_{irev} -> {tver}_{trev}");
        targets.push(name);
    }
}

/// Refresca el índice de fuentes e informa del resultado (contador de
/// plantillas dinámicas). Usado por `-Sy` y por el upgrade (antes de la
/// detección de bumps: si el índice se refrescara después, la detección
/// usaría datos del ciclo anterior).
fn sync_source_index(config: &Config) {
    match crate::portdb::sync_index(config) {
        Ok(stats) if stats.reindexed > 0 => {
            println!(
                "void-packages indexado: {} paquete(s) ({}).",
                stats.reindexed,
                if stats.full_reindex {
                    "índice completo"
                } else {
                    "incremental"
                }
            );
            // 5.3: contador de plantillas que el parser estático no puede
            // resolver (se resuelven por `xbps-src show` en el upgrade).
            println!(
                "portdb: {} plantillas, {} requieren resolución dinámica (vía xbps-src show en upgrade).",
                stats.entries, stats.needs_dynamic
            );
        }
        Ok(_) => {}
        Err(e) => tracing::warn!("no se pudo actualizar el índice de fuentes: {e:#}"),
    }
}

/// ITEM 4: paquetes instalados desde fuente que requieren un SONAME cuyo
/// proveedor cambió en el árbol de fuentes (`common/shlibs`).
///
/// Se calcula ANTES del merge (`xbps-install -u`): los consumidores entran en
/// el plan de recompilación de la MISMA transacción (E14). Sin índice de
/// fuentes no hay señal (no se inventa).
fn soname_bump_consumers(
    config: &Config,
    db: &InstalledDb,
) -> Vec<(String, Vec<crate::soname::SonameBump>)> {
    let Ok(index) = crate::portdb::load(config.cache_dir.join("portdb.json")) else {
        return Vec::new();
    };
    if index.shlibs.is_empty() {
        return Vec::new();
    }
    let mut pins_by_pkg = std::collections::BTreeMap::new();
    for (name, entry) in db.entries_snapshot() {
        if entry.install_type != InstallType::Source {
            continue;
        }
        let Some(pins) = entry.soname_pins.as_ref().filter(|p| !p.is_empty()) else {
            continue;
        };
        pins_by_pkg.insert(name, pins.clone());
    }
    crate::soname::bump_consumers(&pins_by_pkg, &index.shlibs)
}

/// Diff de template pendiente de revisión en un upgrade (A3).
struct TemplateDiff {
    pkg: String,
    repo: String,
    patch: String,
    /// P0-3: hallazgos sobre el diff (califican, no bloquean).
    findings: Vec<crate::template_audit::AuditFinding>,
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
        // P0-3: hallazgos SOBRE el diff (consultivos; el gate decide igual).
        let header = crate::template_audit::format_findings(&d.pkg, &d.repo, &d.findings);
        if !header.is_empty() {
            print!("{header}");
        }
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
    // ITEM 2: el clon de void-packages también se sincroniza (estaba
    // congelado en el commit del clone) y el índice de fuentes se actualiza
    // de forma incremental (frío si no existe). Best-effort: un fallo aquí
    // no invalida la sincronización de los VURs.
    // El índice de fuentes se refresca también en el sync suelto (`-Sy`).
    sync_source_index(config);
    if any_failed {
        Ok(1)
    } else {
        // P1-1: `vary -Sy --lock` regenera tras sincronizar (solo en éxito).
        if config.args.has_arg("lock", "lock") {
            crate::lockfile::regenerate(config)?;
        }
        Ok(0)
    }
}

pub fn upgrade(config: &mut Config) -> Result<i32> {
    // Fase híbrida previa: anclar (repolock) todo lo instalado desde fuente
    // ANTES de `-Su`, para que el binario oficial no pise un paquete que
    // vary gestiona como Source. Idempotente y best-effort (warn, no aborte).
    {
        let db = InstalledDb::load(config.installed_db_path())?;
        for (name, entry) in db.entries_snapshot() {
            if entry.install_type != InstallType::Source {
                continue;
            }
            // Entrada Source ya desinstalada (DB desactualizada): se salta
            // en silencio; evita warnings por residuos que ya no existen.
            if matches!(xbps::query_installed(&name), Ok(None)) {
                tracing::debug!("repolock de '{name}' omitido: ya no está instalado");
                continue;
            }
            if let Err(err) = xbps::pkgdb_mode(config, "repolock", &name) {
                tracing::warn!("repolock de '{name}' falló (best-effort): {err:#}");
            }
        }
    }

    // ITEM 4 (E14): los consumidores de un bump de SONAME se detectan ANTES
    // del merge: entran en el plan de recompilación de esta misma
    // transacción (si se detectaran después, el binario oficial ya habría
    // pisado el paquete fuente).
    sync_source_index(config);
    let bump_consumers = {
        let db = InstalledDb::load(config.installed_db_path())?;
        soname_bump_consumers(config, &db)
    };

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

    // P1-1: verificación contra vary.lock (VUR; oficiales los gestiona xbps).
    {
        let repos_ref: Vec<_> = repos_conf
            .sorted_by_priority()
            .into_iter()
            .filter(|(_, entry)| entry.enabled_or(true))
            .map(|(name, _)| (name.clone(), config.vurs_dir().join(&name)))
            .collect();
        let items: Vec<(String, String)> = current_map
            .iter()
            .map(|(n, v)| (n.clone(), v.clone()))
            .collect();
        for w in crate::lockfile::verify_if_pinned(config, &repos_ref, &items) {
            tracing::warn!("{w}");
        }
    }

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

    // P2-soname: aviso consultivo de drift de proveedores. Solo lectura
    // (nunca bloquea, nunca recompila, sin prompts: apto no-TTY/--yes).
    // Sin baseline pineada no hay aviso.
    let mut drift_pkgs: usize = 0;
    for (name, entry) in db.entries_snapshot() {
        if entry.install_type != InstallType::Source {
            continue;
        }
        let Some(pins) = entry.soname_pins.as_ref().filter(|p| !p.is_empty()) else {
            continue;
        };
        let drifts = crate::soname::check_db_entry(&name, pins);
        if !drifts.is_empty() {
            drift_pkgs += 1;
        }
        for drift in drifts {
            tracing::warn!("{}", crate::soname::format_drift(&name, &drift));
        }
    }
    if drift_pkgs > 0 {
        tracing::warn!(
            "drift de sonames en {drift_pkgs} paquete(s); ejecuta `vary -S --force-build @preserved-rebuild` para recompilarlos (no se recompila automáticamente)"
        );
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

    // ITEM 4: los consumidores de un bump de SONAME se recompilan (E14).
    // Rastro auditable (5.2): proveedor, soname viejo/nuevo y lista de
    // consumidores, vía tracing (sin infraestructura de log nueva).
    for (name, bumps) in &bump_consumers {
        for bump in bumps {
            // Solo los consumidores de ESE soname (no la lista global).
            let affected: Vec<&str> = bump_consumers
                .iter()
                .filter(|(_, bs)| bs.iter().any(|b| b.shlib == bump.shlib))
                .map(|(n, _)| n.as_str())
                .collect();
            tracing::warn!(
                "soname con bump: '{name}' requiere {} (proveedor {} -> {}); consumidores: {}",
                bump.shlib,
                bump.from,
                bump.to,
                affected.join(", ")
            );
        }
        if !outdated.contains(name) {
            println!(
                "  {name} (soname con bump: {})",
                bumps
                    .iter()
                    .map(|b| b.shlib.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            outdated.push(name.clone());
        }
    }

    // Fase híbrida, después de la detección de builds VUR: oficiales Source
    // (DB) que matcheen `source_always` con template más nuevo que el
    // instalado. Se añaden a los targets del MISMO plan (sin reordenar
    // fases): el resolver los emite como BuildOfficial y el install reusa
    // el flujo de fuente. Se ejecuta ANTES del early-return para que un
    // upgrade solo-oficial también proceda.
    {
        let vur_managed: std::collections::HashSet<String> = current_map.keys().cloned().collect();
        add_official_source_upgrades(config, &db, &vur_managed, &mut outdated);
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
                // P0-3: calificar el diff (regresión de checksum + añadidas).
                let findings = crate::template_audit::audit_template_diff(&old, &new, &patch);
                diffs.push(TemplateDiff {
                    pkg: name.clone(),
                    repo: repo_name.clone(),
                    patch,
                    findings,
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
        // Solo se caen los paquetes cuyo template cambió y NO se aprobó el
        // diff: los consumidores de un bump de SONAME (ITEM 4) no tienen
        // diff de template y deben seguir en el plan (E14).
        outdated.retain(|n| !diffs.iter().any(|d| d.pkg == *n) || approved.contains(n));
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

    // --- parseo/comparación de versión para la fase híbrida (fixtures) ---

    #[test]
    fn parse_template_version_minimo() {
        let text = "version=6.6.4\nrevision=1\n";
        assert_eq!(parse_template_version(text), Some(("6.6.4".to_string(), 1)));
        // Formato real de `xbps-src show`: clave con dos puntos y tab.
        let text = "pkgname:\tbase-files\nversion:\t0.146\nrevision:\t2\n";
        assert_eq!(parse_template_version(text), Some(("0.146".to_string(), 2)));
        // revision sin definir => 1; con líneas de ruido intercaladas.
        let text = "# deps: xbps\nversion=156.1\nother=x\n";
        assert_eq!(parse_template_version(text), Some(("156.1".to_string(), 1)));
        // Revisión no numérica => default 1.
        assert_eq!(
            parse_template_version("version=1.0\nrevision=raro\n"),
            Some(("1.0".to_string(), 1))
        );
        // Sin version => None (el paquete se omite con warn).
        assert_eq!(parse_template_version("revision=2\n"), None);
        assert_eq!(parse_template_version(""), None);
        assert_eq!(parse_template_version("version=\n"), None);
    }

    #[test]
    fn split_installed_pkgver_separa_version_revision() {
        assert_eq!(
            split_installed_pkgver("linux", "linux-6.6.4_1"),
            Some(("6.6.4".to_string(), 1))
        );
        assert_eq!(
            split_installed_pkgver("mesa-dri", "mesa-dri-24.0.1_2"),
            Some(("24.0.1".to_string(), 2))
        );
        assert_eq!(split_installed_pkgver("linux", "otro-1.0_1"), None);
        assert_eq!(
            split_installed_pkgver("linux", "linux-1.0"),
            None,
            "sin _rev"
        );
    }

    /// Fixture end-to-end del flujo: show -> (tver, trev) vs instalado
    /// (parse/split/comparación juntos; cmp_pkgver directo lo cubre el resolver).
    #[test]
    fn comparador_version_revision_template_mas_nuevo_que_instalado() {
        let (tver, trev) = parse_template_version("version=156.1\nrevision=3\n").expect("parse");
        let (iver, irev) = split_installed_pkgver("pkg", "pkg-155.0_1").expect("split");
        assert_eq!(
            crate::resolver::cmp_pkgver(&tver, trev, &iver, irev),
            std::cmp::Ordering::Greater
        );
    }

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
            findings: Vec::new(),
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

    #[test]
    fn review_con_hallazgos_atraviesa_el_gate() {
        // P0-3: diffs con findings no rompen el gate (califican, no
        // bloquean). En CI (no TTY) con --yes: aprueba e imprime el bloque.
        use crate::template_audit::{AuditFinding, Severity};
        let diffs = vec![TemplateDiff {
            pkg: "foo".to_string(),
            repo: "mi-repo".to_string(),
            patch: "--- viejo\n+++ nuevo\n".to_string(),
            findings: vec![AuditFinding {
                severity: Severity::High,
                rule: "checksum-ausente",
                detail: "d".to_string(),
            }],
        }];
        let config = Config {
            no_confirm: true,
            ..Config::default()
        };
        assert_eq!(
            review_template_diffs(&config, &diffs).unwrap(),
            vec!["foo".to_string()]
        );
    }
}
