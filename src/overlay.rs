//! Overlay transaccional de parches/archivos de usuario por build.
//!
//! Antes de compilar `pkg` desde fuente, copia
//! `~/.config/vary/patches/<pkg>/*` → `srcpkgs/<pkg>/patches/999-vary-user-*`
//! y `~/.config/vary/files/<pkg>/*` → `srcpkgs/<pkg>/files/` (respaldando los
//! que ya existían). El `Drop` de `UserOverlay` restaura el árbol original
//! (éxito, error o Drop temprano): best-effort, nunca paniquea.

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

/// Hashes de los ficheros de usuario de `pkg` (`patches/<pkg>` + `files/<pkg>`),
/// ordenados por nombre. Es la señal que ITEM 1 compara contra la DB: si el
/// usuario toca sus parches o su config, el paquete compilado queda obsoleto.
///
/// Mismo criterio que el overlay: symlinks y directorios no se copian → no
/// cuentan. Un fichero ilegible se salta (best-effort).
pub fn user_file_hashes(config_dir: &Path, pkg: &str) -> Vec<String> {
    let mut items: Vec<(String, String)> = Vec::new();
    for kind in ["patches", "files"] {
        let dir = config_dir.join(kind).join(pkg);
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(meta) = fs::symlink_metadata(&path) else {
                continue;
            };
            if !meta.is_file() {
                continue;
            }
            let Ok(bytes) = fs::read(&path) else {
                continue;
            };
            let name = entry.file_name().to_string_lossy().into_owned();
            items.push((
                format!("{kind}/{name}"),
                format!("sha256:{:x}", Sha256::digest(bytes)),
            ));
        }
    }
    items.sort();
    items
        .into_iter()
        .map(|(label, hash)| format!("{label} {hash}"))
        .collect()
}

/// Una mutación reversible sobre srcpkgs. Se deshace en orden inverso.
enum Action {
    /// Fichero nuevo copiado: se borra.
    Remove(PathBuf),
    /// Fichero reemplazado: el backup vuelve a su ruta original.
    Restore { target: PathBuf, backup: PathBuf },
    /// Dir creado por nosotros (patches/ o files/): se borra si queda vacío.
    RemoveDirIfEmpty(PathBuf),
}

/// Guard del overlay: `apply` APLICA las mutaciones; el `Drop` las deshace.
/// Sin dirs de usuario es no-op (cero acciones, cero syscalls extra).
pub struct UserOverlay {
    actions: Vec<Action>,
    srcpkg_dir: PathBuf,
}

/// Nombre de backup para `dst`: un `.vary-bak.<pid>.<nombre>` junto al destino.
fn backup_name(dst: &Path, pid: u32) -> PathBuf {
    let name = dst
        .file_name()
        .expect("overlay: dst de backup siempre es user_dir.join(name)");
    dst.with_file_name(format!(".vary-bak.{pid}.{}", name.to_string_lossy()))
}

/// Copia los contenidos de `user_dir` según `kind` (`"patches"` prefija el
/// nombre destino, `"files"` respalda/reemplaza), apilando acciones.
/// Devuelve cuántos ficheros copió.
fn apply_dir(
    srcpkg_dir: &Path,
    user_dir: &Path,
    kind: &str,
    actions: &mut Vec<Action>,
) -> Result<usize> {
    let target = srcpkg_dir.join(kind);
    let mut n = 0;
    // Orden determinista: iterar por nombre, no al orden del FS (arbitrario).
    let mut entries = Vec::new();
    for entry in
        fs::read_dir(user_dir).with_context(|| format!("leyendo {}", user_dir.display()))?
    {
        let entry = entry.with_context(|| format!("iterando {}", user_dir.display()))?;
        entries.push(entry);
    }
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let name = entry.file_name();
        let src = entry.path();
        // symlink_metadata: mirar el ENLACE, no seguirlo a /etc o donde sea.
        let meta = match fs::symlink_metadata(&src) {
            Ok(m) => m,
            Err(err) => {
                tracing::warn!("overlay: saltando {}: {err}", src.display());
                continue;
            }
        };
        if !meta.is_file() {
            tracing::warn!(
                "overlay: saltando {} (no regular; symlinks/dirs no se copian)",
                src.display()
            );
            continue;
        }
        let dst_name = if kind == "patches" {
            format!("999-vary-user-{}", name.to_string_lossy())
        } else {
            name.to_string_lossy().into_owned()
        };
        let dst = target.join(&dst_name);
        if !target.exists() {
            fs::create_dir(&target).with_context(|| format!("creando {}", target.display()))?;
            actions.push(Action::RemoveDirIfEmpty(target.clone()));
        }
        if dst.exists() {
            if kind == "patches" {
                // El prefijo lo hace improbable chocar; si choca, mejor ruidoso.
                bail!("overlay: destino ya existe: {}", dst.display());
            }
            // Backup con pid+nombre: improbable chocar entre procesos.
            let backup = backup_name(&dst, std::process::id());
            // Falla cerrado si el backup ya existe: sobrescribirlo significa
            // destruir la copia de seguridad de un run anterior.
            if backup.exists() {
                bail!("overlay: backup ya existe: {}", backup.display());
            }
            fs::rename(&dst, &backup)
                .with_context(|| format!("respaldando {} → {}", dst.display(), backup.display()))?;
            actions.push(Action::Restore {
                target: dst.clone(),
                backup,
            });
        } else {
            actions.push(Action::Remove(dst.clone()));
        }
        fs::copy(&src, &dst)
            .with_context(|| format!("copiando {} → {}", src.display(), dst.display()))?;
        n += 1;
    }
    Ok(n)
}

/// Aplica el overlay de parches/archivos de usuario para `pkg` sobre
/// `srcpkg_dir`. `config_dir` = `~/.config/vary` (busca `patches/<pkg>` y
/// `files/<pkg>`; dirs vacíos = no-op).
/// ponytail: no hashes en DB por YAGNI; una copia manual o un git stash de
/// `~/.config/vary` cubre la auditoría si algún día hace falta.
pub fn apply(srcpkg_dir: &Path, config_dir: &Path, pkg: &str) -> Result<UserOverlay> {
    if !srcpkg_dir.is_dir() {
        bail!(
            "overlay: srcpkgs/<{pkg}> no existe: {}",
            srcpkg_dir.display()
        );
    }
    let mut actions = Vec::new();
    let mut patches = 0usize;
    let mut files = 0usize;
    for (kind, count) in [("patches", &mut patches), ("files", &mut files)] {
        let user_dir = config_dir.join(kind).join(pkg);
        if !user_dir.is_dir() {
            continue;
        }
        if fs::read_dir(&user_dir)?.next().is_none() {
            continue; // dir vacío
        }
        // Si muere a mitad (parche duplicado, copy fallido tras el rename del
        // backup), deshacemos lo acumulado reusando el Drop del guard.
        match apply_dir(srcpkg_dir, &user_dir, kind, &mut actions) {
            Ok(c) => *count = c,
            Err(err) => {
                // Drop drena en orden de inserción; se invierte para deshacer
                // LIFO (sino RemoveDirIfEmpty corre antes de borrar los ficheros).
                actions.reverse();
                drop(UserOverlay {
                    actions,
                    srcpkg_dir: srcpkg_dir.to_path_buf(),
                });
                return Err(err);
            }
        }
    }
    // Drop recorre en orden inverso: las últimas mutaciones se deshacen antes.
    actions.reverse();
    tracing::info!("overlay de {pkg}: {patches} parches, {files} archivos de usuario inyectados");
    Ok(UserOverlay {
        actions,
        srcpkg_dir: srcpkg_dir.to_path_buf(),
    })
}

impl Drop for UserOverlay {
    fn drop(&mut self) {
        if self.actions.is_empty() {
            return;
        }
        tracing::debug!("overlay::drop: restaurando {}", self.srcpkg_dir.display());
        for a in self.actions.drain(..) {
            match a {
                Action::Remove(p) => {
                    if let Err(err) = fs::remove_file(&p) {
                        tracing::warn!("overlay: no pude borrar {}: {err}", p.display());
                    }
                }
                Action::Restore { target, backup } => {
                    if let Err(err) = fs::rename(&backup, &target) {
                        tracing::warn!(
                            "overlay: no pude restaurar {} desde {}: {err}",
                            target.display(),
                            backup.display()
                        );
                    }
                }
                Action::RemoveDirIfEmpty(p) => {
                    // Solo borra si quedó vacío; si el build dejó algo, no toca.
                    if let Err(err) = fs::remove_dir(&p) {
                        tracing::debug!("overlay: dir {} no borrado: {err}", p.display());
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_to(dir: &Path, name: &str, contents: &str) {
        fs::write(dir.join(name), contents).unwrap();
    }

    /// base temporal + `srcpkgs/foo` + conf vacío. `_base` mantiene vivo el dir.
    fn setup() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let base = tempfile::tempdir().unwrap();
        let srcpkg = base.path().join("srcpkgs").join("foo");
        let cfg = base.path().join("conf");
        fs::create_dir_all(&srcpkg).unwrap();
        fs::create_dir_all(&cfg).unwrap();
        (base, srcpkg, cfg)
    }

    #[test]
    fn no_user_dirs_is_noop() {
        let (base, srcpkg, cfg) = setup();
        let overlay = apply(&srcpkg, &cfg, "foo").unwrap();
        assert!(overlay.actions.is_empty());
        assert!(!srcpkg.join("patches").exists());
        assert!(!srcpkg.join("files").exists());
        drop(base);
    }

    #[test]
    fn patch_copied_and_removed_on_drop() {
        let (base, srcpkg, cfg) = setup();
        let up = cfg.join("patches").join("foo");
        fs::create_dir_all(&up).unwrap();
        write_to(&up, "fix-thing.patch", "user patch");
        let off = srcpkg.join("patches");
        fs::create_dir_all(&off).unwrap();
        write_to(&off, "official.patch", "official");

        {
            let overlay = apply(&srcpkg, &cfg, "foo").unwrap();
            assert_eq!(overlay.actions.len(), 1);
            let injected = off.join("999-vary-user-fix-thing.patch");
            assert_eq!(fs::read_to_string(&injected).unwrap(), "user patch");
        }
        assert!(!off.join("999-vary-user-fix-thing.patch").exists());
        assert_eq!(
            fs::read_to_string(off.join("official.patch")).unwrap(),
            "official"
        );
        drop(base);
    }

    #[test]
    fn existing_file_replaced_then_restored() {
        let (base, srcpkg, cfg) = setup();
        fs::create_dir_all(srcpkg.join("files")).unwrap();
        write_to(&srcpkg, "files/config", "original");
        let up = cfg.join("files").join("foo");
        fs::create_dir_all(&up).unwrap();
        write_to(&up, "config", "replaced");

        {
            let _overlay = apply(&srcpkg, &cfg, "foo").unwrap();
            assert_eq!(
                fs::read_to_string(srcpkg.join("files/config")).unwrap(),
                "replaced"
            );
        }
        assert_eq!(
            fs::read_to_string(srcpkg.join("files/config")).unwrap(),
            "original",
            "restaurado al Drop"
        );
        // Sin backups huérfanos.
        assert_eq!(fs::read_dir(srcpkg.join("files")).unwrap().count(), 1);
        drop(base);
    }

    #[test]
    fn new_file_copied_then_deleted() {
        let (base, srcpkg, cfg) = setup();
        let up = cfg.join("files").join("foo");
        fs::create_dir_all(&up).unwrap();
        write_to(&up, "banner", "new");
        {
            let overlay = apply(&srcpkg, &cfg, "foo").unwrap();
            assert!(
                !overlay.actions.is_empty(),
                "debe registrar al menos el archivo inyectado"
            );
            assert!(srcpkg.join("files/banner").is_file());
        }
        assert!(!srcpkg.join("files/banner").exists());
        // files/ creado por nosotros y vacío al cierre => borrado.
        assert!(!srcpkg.join("files").exists());
        drop(base);
    }

    #[test]
    fn non_regular_entries_skipped() {
        let (base, srcpkg, cfg) = setup();
        let up = cfg.join("patches").join("foo");
        fs::create_dir_all(&up).unwrap();
        std::os::unix::fs::symlink("/etc/hostname", up.join("evil.patch")).unwrap();
        fs::create_dir_all(srcpkg.join("patches")).unwrap();

        let overlay = apply(&srcpkg, &cfg, "foo").unwrap();
        assert!(overlay.actions.is_empty());
        assert!(fs::read_dir(srcpkg.join("patches"))
            .unwrap()
            .next()
            .is_none());
        drop(base);
    }

    #[test]
    fn missing_srcpkg_dir_is_error() {
        let base = tempfile::tempdir().unwrap();
        let r = apply(&base.path().join("nope"), base.path(), "pepa");
        assert!(r.is_err());
    }

    /// Rollback parcial LIFO: `patches/` se crea e inyecta `a.patch` ok, pero
    /// `files/` falla después (colisión de backup) → el guard drop POR COMPLETO
    /// lo acumulado en orden inverso. Antes de invertir en el Err, el Drop
    /// drenaba en orden de inserción: `RemoveDirIfEmpty(patches)` corría antes
    /// que `Remove` y dejaba `patches/` huérfano y vacío.
    #[test]
    fn partial_rollback_removes_fresh_dir_and_injected_files() {
        let (base, srcpkg, cfg) = setup();
        let up = cfg.join("patches").join("foo");
        fs::create_dir_all(&up).unwrap();
        write_to(&up, "a.patch", "user patch");
        let uf = cfg.join("files").join("foo");
        fs::create_dir_all(&uf).unwrap();
        write_to(&uf, "config.h", "user file");
        fs::create_dir_all(srcpkg.join("files")).unwrap();
        write_to(&srcpkg, "files/config.h", "official");
        let bomb = backup_name(&srcpkg.join("files/config.h"), std::process::id());
        fs::write(&bomb, "old backup").unwrap();

        let r = apply(&srcpkg, &cfg, "foo");
        assert!(r.is_err(), "colisión de backup debe fallar");
        // rollback del sello: parche inyectado fuera y dir creado sin huérfano
        assert!(!srcpkg.join("patches/999-vary-user-a.patch").exists());
        assert!(
            !srcpkg.join("patches").exists(),
            "patches/ creado por apply queda huérfano"
        );
        // original y backup previo intactos (fail-closed, sin sobrescribir)
        assert_eq!(
            fs::read_to_string(srcpkg.join("files/config.h")).unwrap(),
            "official"
        );
        assert_eq!(fs::read_to_string(&bomb).unwrap(), "old backup");
        drop(base);
    }

    /// Backup existente: falla cerrado, ni toca el original ni el backup.
    #[test]
    fn backup_collision_fails_closed() {
        let (base, srcpkg, cfg) = setup();
        fs::create_dir_all(srcpkg.join("files")).unwrap();
        write_to(&srcpkg, "files/config.h", "official");
        let uf = cfg.join("files").join("foo");
        fs::create_dir_all(&uf).unwrap();
        write_to(&uf, "config.h", "user file");
        let bomb = backup_name(&srcpkg.join("files/config.h"), std::process::id());
        fs::write(&bomb, "old backup").unwrap();

        let r = apply(&srcpkg, &cfg, "foo");
        assert!(r.is_err());
        assert_eq!(
            fs::read_to_string(srcpkg.join("files/config.h")).unwrap(),
            "official"
        );
        assert_eq!(fs::read_to_string(&bomb).unwrap(), "old backup");
        drop(base);
    }

    /// Colisión en `patches/`: bail ruidoso y el guard deshace lo copiado antes
    /// del choque. El orden es determinista (apply_dir ordena por nombre);
    /// nombrando `a.patch` y `z-dup.patch` copiamos primero y chocamos después.
    #[test]
    fn patch_collision_bails_and_keeps_official() {
        let (base, srcpkg, cfg) = setup();
        let up = cfg.join("patches").join("foo");
        fs::create_dir_all(&up).unwrap();
        write_to(&up, "a.patch", "user patch");
        write_to(&up, "z-dup.patch", "dup");
        let off = srcpkg.join("patches");
        fs::create_dir_all(&off).unwrap();
        // dst del dup ya existe (tras prefijo): 999-vary-user-z-dup.patch.
        write_to(&off, "999-vary-user-z-dup.patch", "official");

        let r = apply(&srcpkg, &cfg, "foo");
        assert!(r.is_err());
        // nada inyectado queda y lo oficial sigue intacto
        assert!(!off.join("999-vary-user-a.patch").exists());
        assert_eq!(
            fs::read_to_string(off.join("999-vary-user-z-dup.patch")).unwrap(),
            "official"
        );
        drop(base);
    }
}
