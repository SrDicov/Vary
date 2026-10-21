//! Auditoría de discrepancias entre lo COMPILADO y la configuración OBJETIVO.
//!
//! ITEM 1 de `docs/vary2/ORDEN-v2.md`: detectar que cambiar `-march`, las
//! opciones de build o los parches de usuario invalida un paquete ya
//! compilado aunque su `pkgver` no se mueva.
//!
//! Regla de honestidad (misma que los pins de sonames): si no hay baseline
//! (paquete instalado antes de este esquema, o captura fallida) NO hay señal
//! -> no se propone rebuild. Ausencia de dato no es "todo cambió".

use crate::config::Config;
use crate::db::{Entry, InstallType};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::Path;

/// Por qué hay que recompilar un paquete.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RebuildReason {
    /// Hay versión nueva en el árbol de fuentes (ITEM 2 lo alimenta).
    NewVersion,
    /// Cambió el conjunto de opciones activas.
    OptionsChanged,
    /// Cambiaron las banderas del compilador.
    FlagsChanged,
    /// Cambiaron los parches/archivos de usuario.
    PatchChanged,
    /// Cambió el template del paquete.
    TemplateChanged,
}

impl RebuildReason {
    /// Texto corto para el plan y los avisos.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NewVersion => "versión nueva",
            Self::OptionsChanged => "opciones cambiadas",
            Self::FlagsChanged => "banderas cambiadas",
            Self::PatchChanged => "parches de usuario cambiados",
            Self::TemplateChanged => "template cambiado",
        }
    }

    /// ¿Lo dispara `--newoptions`? (cambios del lado fuente: opciones,
    /// parches de usuario y template).
    pub fn triggers_options(self) -> bool {
        matches!(
            self,
            Self::OptionsChanged | Self::PatchChanged | Self::TemplateChanged
        )
    }

    /// ¿Lo dispara `--changed-flags`?
    pub fn triggers_flags(self) -> bool {
        matches!(self, Self::FlagsChanged)
    }
}

/// Estado objetivo con el que se compila HOY (lo que el usuario pide ahora).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BuildTarget {
    /// Opciones activas (sin el prefijo `~`), ordenadas.
    pub options: Vec<String>,
    pub cflags: String,
    pub cxxflags: String,
    pub ldflags: String,
    /// Hashes de los ficheros de usuario (parches + files).
    pub patches_sha256: Vec<String>,
    /// Hash del template con el que se compila.
    pub template_sha256: Option<String>,
}

/// Auditoría pura: primera discrepancia en orden fijo, o `None`.
///
/// Orden: versión → opciones → banderas → parches → template. `portdb_version`
/// es la versión en el árbol de fuentes; `None` = desconocido → sin señal.
pub fn audit(
    entry: &Entry,
    target: &BuildTarget,
    portdb_version: Option<&str>,
) -> Option<RebuildReason> {
    // Un binario oficial puro no se recompila por esto: nunca se compiló
    // localmente, no hay nada nuestro que invalidar.
    if entry.install_type != InstallType::Source {
        return None;
    }
    if let Some(version) = portdb_version {
        if version != entry.version {
            return Some(RebuildReason::NewVersion);
        }
    }
    if let Some(active) = entry.active_options.as_ref() {
        if active != &target.options {
            return Some(RebuildReason::OptionsChanged);
        }
    }
    if let Some(cflags) = entry.cflags.as_ref() {
        if cflags != &target.cflags
            || entry.cxxflags.as_ref() != Some(&target.cxxflags)
            || entry.ldflags.as_ref() != Some(&target.ldflags)
        {
            return Some(RebuildReason::FlagsChanged);
        }
    }
    if let Some(patches) = entry.patches_sha256.as_ref() {
        if patches != &target.patches_sha256 {
            return Some(RebuildReason::PatchChanged);
        }
    }
    if let (Some(installed), Some(current)) = (
        entry.template_sha256.as_ref(),
        target.template_sha256.as_ref(),
    ) {
        if installed != current {
            return Some(RebuildReason::TemplateChanged);
        }
    }
    None
}

/// Opciones activas para `pkg`.
///
/// Semántica de xbps-src: lo por paquete SUSTITUYE a lo global (no se
/// fusionan). Un `~opcion` desactiva: no cuenta como activa.
pub fn effective_options(config: &Config, pkg: &str) -> Vec<String> {
    let raw = config
        .pkg_options
        .get(pkg)
        .unwrap_or(&config.make.global_options);
    let mut opts: Vec<String> = raw
        .iter()
        .filter(|o| !o.starts_with('~'))
        .cloned()
        .collect();
    opts.sort();
    opts.dedup();
    opts
}

/// Hash del template del paquete en el clon de void-packages (si está).
pub fn template_hash(void_packages: &Path, pkg: &str) -> Option<String> {
    let path = void_packages.join("srcpkgs").join(pkg).join("template");
    let bytes = std::fs::read(path).ok()?;
    Some(format!("sha256:{:x}", Sha256::digest(bytes)))
}

/// Objetivo de compilación completo para `pkg`.
pub fn target_for(config: &Config, pkg: &str) -> BuildTarget {
    BuildTarget {
        options: effective_options(config, pkg),
        cflags: config.make.cflags.clone(),
        cxxflags: config.make.cxxflags.clone(),
        ldflags: config.make.ldflags.clone(),
        patches_sha256: crate::overlay::user_file_hashes(&config.config_dir, pkg),
        template_sha256: template_hash(&config.void_packages_dir(), pkg),
    }
}

/// Lookup de versión en el árbol de fuentes (ITEM 2 lo alimenta).
pub type VersionLookup<'a> = &'a dyn Fn(&str) -> Option<String>;

/// Paquetes instalados desde fuente que hay que recompilar, con su motivo.
///
/// `portdb_version` permite el cruce con el árbol de fuentes (ITEM 2); sin él
/// solo se evalúan opciones/banderas/parches/template.
pub fn candidates(
    config: &Config,
    portdb_version: Option<VersionLookup<'_>>,
) -> BTreeMap<String, RebuildReason> {
    let mut out = BTreeMap::new();
    let Ok(db) = crate::db::InstalledDb::load(config.installed_db_path()) else {
        return out;
    };
    for (name, entry) in db.entries_snapshot() {
        if entry.install_type != InstallType::Source {
            continue;
        }
        let target = target_for(config, &name);
        let version = portdb_version.and_then(|f| f(&name));
        if let Some(reason) = audit(&entry, &target, version.as_deref()) {
            out.insert(name, reason);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Entry;

    fn entry_source() -> Entry {
        Entry {
            version: "pkg-1.0_1".to_string(),
            vur: "void".to_string(),
            install_date: 0,
            build_date: Some(1),
            install_type: InstallType::Source,
            repo_commit: None,
            artifact_sha256: None,
            soname_pins: None,
            active_options: Some(vec!["pgo".to_string()]),
            cflags: Some("-O2".to_string()),
            cxxflags: Some("-O2".to_string()),
            ldflags: Some(String::new()),
            patches_sha256: Some(vec!["sha256:aa".to_string()]),
            template_sha256: Some("sha256:bb".to_string()),
        }
    }

    fn target() -> BuildTarget {
        BuildTarget {
            options: vec!["pgo".to_string()],
            cflags: "-O2".to_string(),
            cxxflags: "-O2".to_string(),
            ldflags: String::new(),
            patches_sha256: vec!["sha256:aa".to_string()],
            template_sha256: Some("sha256:bb".to_string()),
        }
    }

    #[test]
    fn sin_discrepancia_no_hay_rebuild() {
        assert_eq!(audit(&entry_source(), &target(), None), None);
    }

    #[test]
    fn binario_oficial_nunca_se_recompila() {
        let mut e = entry_source();
        e.install_type = InstallType::Binary;
        let mut t = target();
        t.cflags = "-O3".to_string();
        assert_eq!(audit(&e, &t, None), None);
    }

    #[test]
    fn detecta_cada_causa() {
        let mut t = target();
        t.options = vec!["wayland".to_string()];
        assert_eq!(
            audit(&entry_source(), &t, None),
            Some(RebuildReason::OptionsChanged)
        );

        let mut t = target();
        t.cflags = "-O3 -march=native".to_string();
        assert_eq!(
            audit(&entry_source(), &t, None),
            Some(RebuildReason::FlagsChanged)
        );

        let mut t = target();
        t.patches_sha256 = vec!["sha256:cc".to_string()];
        assert_eq!(
            audit(&entry_source(), &t, None),
            Some(RebuildReason::PatchChanged)
        );

        let mut t = target();
        t.template_sha256 = Some("sha256:dd".to_string());
        assert_eq!(
            audit(&entry_source(), &t, None),
            Some(RebuildReason::TemplateChanged)
        );

        assert_eq!(
            audit(&entry_source(), &target(), Some("pkg-2.0_1")),
            Some(RebuildReason::NewVersion)
        );
    }

    /// Sin baseline no hay señal: no inventamos rebuilds.
    #[test]
    fn sin_baseline_no_hay_senal() {
        let mut e = entry_source();
        e.active_options = None;
        e.cflags = None;
        e.cxxflags = None;
        e.ldflags = None;
        e.patches_sha256 = None;
        e.template_sha256 = None;
        let mut t = target();
        t.cflags = "-O3".to_string();
        t.options = vec!["otra".to_string()];
        assert_eq!(audit(&e, &t, None), None);
    }

    #[test]
    fn cada_flag_dispara_su_causa() {
        assert!(RebuildReason::OptionsChanged.triggers_options());
        assert!(RebuildReason::PatchChanged.triggers_options());
        assert!(RebuildReason::TemplateChanged.triggers_options());
        assert!(!RebuildReason::FlagsChanged.triggers_options());
        assert!(RebuildReason::FlagsChanged.triggers_flags());
        assert!(!RebuildReason::OptionsChanged.triggers_flags());
    }
}
