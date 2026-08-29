//! Bootstrap transaccional del entorno vary.
//!
//! Detecta que los directorios de trabajo existan; si no, clona void-packages,
//! ejecuta `binary-bootstrap` y registra hostdir/binpkgs como repositorio
//! local prioritario en /etc/xbps.d/10-vary.conf. Idempotente.
use crate::masterdir::{clone_void_packages, Masterdir};
use anyhow::{bail, Context, Result};
use std::path::Path;

/// Verifica que git está instalado (vary usa git CLI, no git2)
pub fn git_available(git_bin: &str) -> bool {
    std::process::Command::new(git_bin)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

const XBPSD_VARY_CONF: &str = "/etc/xbps.d/10-vary.conf";

/// Contenido del archivo de repositorio local para xbps.
pub fn vary_conf_contents(binpkgs_root: &Path) -> String {
    format!("repository={}\n", binpkgs_root.display())
}

fn write_root_file(tmp_path: &Path, dest: &str, sudo_bin: &str, sudo_flags: &[String]) -> Result<()> {
    let status = crate::elevate::elevate(sudo_bin, sudo_flags, "install")?
        .args(["-m", "644"])
        .arg(tmp_path)
        .arg(dest)
        .status()
        .context("elevando privilegios para escribir archivo del sistema")?;
    if !status.success() {
        bail!("no se pudo escribir {} (código {:?})", dest, status.code());
    }
    Ok(())
}

/// Inicializa el entorno completo de forma idempotente.
///
/// Pasos:
/// 1. Check de git (Ajuste 5).
/// 2. Clone shallow de void-packages si falta.
/// 3. `./xbps-src binary-bootstrap` si el masterdir falta.
/// 4. `/etc/xbps.d/10-vary.conf` apuntando a hostdir/binpkgs si falta o cambió.
pub fn initialize_environment(
    void_packages_dir: &Path,
    sudo_bin: &str,
    sudo_flags: &[String],
    git_bin: &str,
) -> Result<Masterdir> {
    // Verificar que git está instalado (vary usa git CLI, no git2)
    if !git_available(git_bin) {
        bail!(
            "git es requerido por vary pero no está instalado.\n\
             Instálalo con: xbps-install git"
        );
    }

    clone_void_packages(void_packages_dir, git_bin)?;

    let md = Masterdir::new(void_packages_dir);
    if !md.bootstrapped() {
        md.binary_bootstrap()?;
    }

    let contents = vary_conf_contents(&md.hostdir_binpkgs_root());
    match std::fs::read_to_string(XBPSD_VARY_CONF) {
        Ok(existing) if existing == contents => {}
        _ => {
            let tmp = void_packages_dir.join(".vary-10-conf.tmp");
            std::fs::write(&tmp, &contents)
                .with_context(|| format!("escribiendo {}", tmp.display()))?;
            write_root_file(&tmp, XBPSD_VARY_CONF, sudo_bin, sudo_flags)?;
            let _ = std::fs::remove_file(&tmp);
            tracing::info!("registrado repositorio local en {}", XBPSD_VARY_CONF);
        }
    }

    Ok(md)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conf_tiene_formato_repository() {
        let c = vary_conf_contents(Path::new("/home/u/.cache/vary/void-packages/hostdir/binpkgs"));
        assert_eq!(
            c,
            "repository=/home/u/.cache/vary/void-packages/hostdir/binpkgs\n"
        );
    }

    #[test]
    fn git_disponible_en_host_de_desarrollo() {
        // En hosts de desarrollo normales git existe; en una imagen mínima de
        // prueba podría faltar, así que solo verificamos que no entra en pánico.
        let _ = git_available("git");
    }
}
