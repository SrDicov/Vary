//! Overlay transaccional de parches/archivos de usuario por build.
//!
//! Antes de compilar `pkg` desde fuente, copia
//! `~/.config/vary/patches/<pkg>/*` → `srcpkgs/<pkg>/patches/999-vary-user-*`
//! y `~/.config/vary/files/<pkg>/*` → `srcpkgs/<pkg>/files/` (respaldando los
//! que ya existían). El `Drop` de `UserOverlay` restaura el árbol original
//! (éxito, error o Drop temprano): best-effort, nunca paniquea.

use anyhow::{bail, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

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
    for entry in
        fs::read_dir(user_dir).with_context(|| format!("leyendo {}", user_dir.display()))?
    {
        let entry = entry.with_context(|| format!("iterando {}", user_dir.display()))?;
        // file_name(): nunca la ruta completa (prohíbe escapes tipo ../..).
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
            let backup = target.join(format!(".vary-bak.{}.{}", std::process::id(), dst_name));
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
            assert_eq!(overlay.actions.len(), 1);
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
        let r = apply(&base.path().join("nope"), &base.path(), "pepa");
        assert!(r.is_err());
    }
}
