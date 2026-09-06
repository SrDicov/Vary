//! Administración del masterdir de void-packages.
//!
//! Void ya compila en el sandbox nativo de `xbps-src`; este módulo solo
//! administra la ruta base y delega el ciclo de vida de la compilación
//! de forma secuencial y determinista (Opción A).
use crate::xbps::xbps_src;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

pub const VOID_PACKAGES_URL: &str = "https://github.com/void-linux/void-packages";

pub struct Masterdir {
    /// Ruta al checkout local de void-packages (~/.cache/vary/void-packages).
    pub path: PathBuf,
}

impl Masterdir {
    pub fn new(void_packages_dir: impl Into<PathBuf>) -> Self {
        Self {
            path: void_packages_dir.into(),
        }
    }

    /// El entorno aislado (masterdir) ya fue inicializado?
    pub fn bootstrapped(&self) -> bool {
        self.path.join("masterdir").exists()
    }

    /// `./xbps-src binary-bootstrap`: descarga el conjunto de herramientas
    /// base dentro del namespace del masterdir. Idempotente.
    pub fn binary_bootstrap(&self) -> Result<()> {
        tracing::info!("ejecutando ./xbps-src binary-bootstrap (puede tardar)...");
        let code = xbps_src(&self.path, &["binary-bootstrap"], None)
            .context("falló ./xbps-src binary-bootstrap")?;
        if code != 0 {
            anyhow::bail!("./xbps-src binary-bootstrap terminó con código {code}");
        }
        Ok(())
    }

    pub fn srcpkgs_dir(&self) -> PathBuf {
        self.path.join("srcpkgs")
    }

    /// Ruta del repositorio binario local raíz (sin arch) para xbps-install.
    pub fn hostdir_binpkgs_root(&self) -> PathBuf {
        self.path.join("hostdir").join("binpkgs")
    }

    /// Compila un paquete: `./xbps-src pkg <pkg>` dentro del masterdir de
    /// forma secuencial y determinista, pasando `makejobs` a `XBPS_MAKEJOBS`.
    pub fn build_pkg(&self, pkg: &str, makejobs: usize) -> Result<()> {
        if crate::signal::is_shutting_down() {
            anyhow::bail!("interrumpido por señal antes de compilar {pkg}");
        }

        tracing::info!("compilando {pkg} con xbps-src (makejobs: {makejobs})...");
        let code = xbps_src(&self.path, &["pkg", pkg], Some(makejobs))
            .with_context(|| format!("falló ./xbps-src pkg {pkg}"))?;

        if code != 0 {
            anyhow::bail!("xbps-src pkg {pkg} terminó con código {code}");
        }
        Ok(())
    }

    /// Solo descarga fuentes: `./xbps-src fetch <pkg>`.
    pub fn fetch_pkg(&self, pkg: &str) -> Result<()> {
        let code = xbps_src(&self.path, &["fetch", pkg], None)
            .with_context(|| format!("falló ./xbps-src fetch {pkg}"))?;
        if code != 0 {
            anyhow::bail!("xbps-src fetch {pkg} terminó con código {code}");
        }
        Ok(())
    }
}

/// Clona void-packages (shallow) si aún no existe.
pub fn clone_void_packages(target: &Path, git_bin: &str) -> Result<()> {
    if target.join(".git").exists() {
        return Ok(());
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("creando {}", parent.display()))?;
    }
    tracing::info!("clonando void-packages (depth 1)...");
    let out = std::process::Command::new(git_bin)
        .args([
            "-c",
            "protocol.ext.allow=never",
            "-c",
            "protocol.file.allow=user",
            "clone",
            "--depth",
            "1",
            "--",
            VOID_PACKAGES_URL,
            &target.display().to_string(),
        ])
        .status()
        .context(format!("`{git_bin}` no encontrado: ¿está instalado?"))?;
    if !out.success() {
        anyhow::bail!(
            "git clone de void-packages falló con código {:?}",
            out.code()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rutas_derivadas_son_consistentes() {
        let md = Masterdir::new("/tmp/vp");
        assert_eq!(md.srcpkgs_dir(), PathBuf::from("/tmp/vp/srcpkgs"));
        assert_eq!(
            md.hostdir_binpkgs_root().join("x86_64"),
            PathBuf::from("/tmp/vp/hostdir/binpkgs/x86_64")
        );
        assert_eq!(
            md.hostdir_binpkgs_root(),
            PathBuf::from("/tmp/vp/hostdir/binpkgs")
        );
    }

    #[test]
    fn build_pkg_falla_si_no_existe_xbps_src() {
        let dir = tempfile::tempdir().unwrap();
        let md = Masterdir::new(dir.path());
        let res = md.build_pkg("dummy-pkg", 2);
        assert!(res.is_err());
        assert!(format!("{:#}", res.unwrap_err()).contains("no encontrado"));
    }
}
