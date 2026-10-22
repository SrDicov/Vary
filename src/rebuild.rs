//! Auditoría de discrepancias entre lo COMPILADO y la configuración OBJETIVO.
//!
//! ITEM 1 de `docs/vary2/ORDEN-v2.md`: detectar que cambiar `-march`, las
//! opciones de build o los parches de usuario invalida un paquete ya
//! compilado aunque su `pkgver` no se mueva.
//!
//! Reglas de honestidad (las mismas que los pins de sonames):
//! - si no hay baseline (paquete instalado antes de este esquema, o captura
//!   fallida) NO hay señal -> no se propone rebuild. Ausencia de dato no es
//!   "todo cambió";
//! - cada dimensión se compara por separado: que cambien las opciones NO
//!   puede tapar un cambio de banderas (y al revés). El filtro por flag
//!   (`--newoptions` / `--changed-flags`) decide sobre la lista COMPLETA.

use crate::config::Config;
use crate::db::{BuildFacts, Entry, InstallType};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::Path;

/// Por qué hay que recompilar un paquete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RebuildReason {
    /// Hay versión nueva en el árbol de fuentes.
    // ponytail: sin consumidor hasta que ITEM 2 alimente el lookup de versión;
    // la rama existe para no reescribir la auditoría cuando llegue.
    NewVersion,
    /// Cambió el conjunto de opciones activas.
    OptionsChanged,
    /// Cambiaron las banderas del compilador.
    FlagsChanged,
    /// Cambiaron los parches/archivos de usuario.
    PatchChanged,
    /// Cambió el template del paquete.
    TemplateChanged,
    /// Una dep cambió de versión en el árbol de fuentes desde el build: el
    /// binario quedó compilado contra una dep que ya no es la del árbol.
    DepChanged { dep: String },
}

impl RebuildReason {
    /// Texto corto para el plan y los avisos.
    pub fn describe(&self) -> String {
        match self {
            Self::NewVersion => "versión nueva".to_string(),
            Self::OptionsChanged => "opciones cambiadas".to_string(),
            Self::FlagsChanged => "banderas cambiadas".to_string(),
            Self::PatchChanged => "parches de usuario cambiados".to_string(),
            Self::TemplateChanged => "template cambiado".to_string(),
            Self::DepChanged { dep } => format!("dep cambiada ({dep})"),
        }
    }

    /// ¿Lo dispara `--newoptions`? (cambios del lado fuente: opciones,
    /// parches de usuario y template).
    pub fn triggers_options(&self) -> bool {
        matches!(
            self,
            Self::OptionsChanged | Self::PatchChanged | Self::TemplateChanged
        )
    }

    /// ¿Lo dispara `--changed-flags`?
    pub fn triggers_flags(&self) -> bool {
        matches!(self, Self::FlagsChanged)
    }

    /// ¿Lo dispara `--changed-deps`? (una dep cambió de versión en el árbol).
    pub fn triggers_deps(&self) -> bool {
        matches!(self, Self::DepChanged { .. })
    }
}

/// Estado objetivo con el que se compila HOY (lo que el usuario pide ahora).
///
/// Es el mismo tipo que se graba en la DB (`db::BuildFacts`): un solo tipo,
/// una sola definición de "con qué se compiló".
pub type BuildTarget = BuildFacts;

/// Auditoría pura: TODAS las discrepancias, en orden de prioridad.
///
/// Orden: versión → opciones → banderas → parches → template.
pub fn audit_all(
    entry: &Entry,
    target: &BuildTarget,
    portdb_version: Option<&str>,
    dep_versions: Option<VersionLookup<'_>>,
) -> Vec<RebuildReason> {
    let mut out = Vec::new();
    // Un binario oficial puro no se recompila por esto: nunca se compiló
    // localmente, no hay nada nuestro que invalidar.
    if entry.install_type != InstallType::Source {
        return out;
    }
    if let Some(version) = portdb_version {
        if version != entry.version {
            out.push(RebuildReason::NewVersion);
        }
    }
    if let Some(active) = entry.active_options.as_ref() {
        if active != &target.active_options {
            out.push(RebuildReason::OptionsChanged);
        }
    }
    // Cada bandera se compara con su propia guardia: un baseline parcial
    // (p.ej. cflags sin ldflags) NO puede producir un falso positivo.
    if let Some(cflags) = entry.cflags.as_ref() {
        if cflags != &target.cflags {
            out.push(RebuildReason::FlagsChanged);
        }
    }
    if let Some(cxxflags) = entry.cxxflags.as_ref() {
        if cxxflags != &target.cxxflags {
            out.push(RebuildReason::FlagsChanged);
        }
    }
    if let Some(ldflags) = entry.ldflags.as_ref() {
        if ldflags != &target.ldflags {
            out.push(RebuildReason::FlagsChanged);
        }
    }
    if let Some(patches) = entry.patches_sha256.as_ref() {
        if patches != &target.patches_sha256 {
            out.push(RebuildReason::PatchChanged);
        }
    }
    if let (Some(installed), Some(current)) = (
        entry.template_sha256.as_ref(),
        target.template_sha256.as_ref(),
    ) {
        if installed != current {
            out.push(RebuildReason::TemplateChanged);
        }
    }
    // Una dep cambió de versión en el árbol de fuentes desde el build: el
    // binario quedó ligado a una dep que ya no es la del árbol (cabeceras,
    // ABI o comportamiento). Sin baseline no hay señal, y una dep que el
    // árbol no conoce tampoco (ausencia de dato ≠ señal).
    if let (Some(pinned), Some(lookup)) = (entry.deps_pinned.as_ref(), dep_versions) {
        if let Some(dep) = changed_deps(pinned, lookup) {
            out.push(RebuildReason::DepChanged { dep });
        }
    }
    out
}

/// Primera dep cuya versión en el árbol de fuentes cambió desde el build.
///
/// `pinned` = baseline (dep → pkgver del árbol al compilar); `lookup` =
/// versión actual del árbol. `None` = sin señal: la dep no cambió, el árbol
/// no la conoce, o no hay baseline. Pura y testeable.
// ponytail: se compara el pkgver COMPLETO (nombre del paquete fuente +
// versión), así que un cambio de nombre del paquete fuente en el árbol
// (p. ej. `libfoo-devel` pasa a ser symlink de otro padre) da señal aunque
// la versión no se mueva; comparar solo la versión evitaría ese falso
// positivo, pero perdería los forks con la misma versión. Sube el coste a
// un rebuild extra (opt-in), no la corrección.
pub fn changed_deps(
    pinned: &BTreeMap<String, String>,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> Option<String> {
    for (dep, old) in pinned {
        if let Some(now) = lookup(dep) {
            if now != *old {
                return Some(dep.clone());
            }
        }
    }
    None
}

/// Opciones activas para `pkg`.
///
/// Semántica de xbps-src: lo por paquete SUSTITUYE a lo global (no se
/// fusionan) y una lista vacía NO sustituye (xbps-src ignora
/// `XBPS_PKG_OPTIONS_<pkg>` vacío y aplica las globales). Un `~opcion`
/// desactiva: no cuenta como activa.
pub fn effective_options(config: &Config, pkg: &str) -> Vec<String> {
    crate::use_reduce::effective_options(&config.make.global_options, config.pkg_options.get(pkg))
}

/// Hash del template del paquete en el clon de void-packages (si está).
pub fn template_hash(void_packages: &Path, pkg: &str) -> Option<String> {
    let path = void_packages.join("srcpkgs").join(pkg).join("template");
    let bytes = std::fs::read(path).ok()?;
    Some(format!("sha256:{:x}", Sha256::digest(bytes)))
}

/// Objetivo de compilación completo para `pkg`.
///
/// `with_hashes`: si es falso NO se leen ficheros (los hashes solo se
/// necesitan cuando hay baseline con el que comparar).
///
/// ## Por qué el join (Entry ↔ objetivo) aguanta
///
/// La DB registra el PAQUETE FUENTE de un build oficial (el padre), no sus
/// subpaquetes: la entrada del padre lleva el baseline de flags/opciones y
/// el hash del template, y un rebuild del padre reconstruye TODOS sus
/// subpaquetes (una sola invocación de `xbps-src pkg <padre>`). El template
/// de un subpaquete con symlink en `srcpkgs/` (p.ej. `openssl-devel`) se
/// alcanza por el symlink, así que su hash coincide con el del padre.
///
/// LÍMITE CONOCIDO (sonda 5.1, 2026-10-21): vary NO registra los
/// subpaquetes de los builds oficiales (la ruta oficial usa un VurInfo
/// placeholder sin `subpackages`), así que un subpaquete NO tiene entrada en
/// la DB → ni auditoría ni repolock → una actualización oficial puede
/// sustituir un binario compilado por vary por el genérico. No se corrige
/// aquí: cerrarlo toca la escritura de la DB + repolock + auditoría +
/// upgrade (ver `docs/vary2/PROGRESS.md`, ítem diferido con su trigger).
pub fn target_for(config: &Config, pkg: &str, with_hashes: bool) -> BuildTarget {
    BuildTarget {
        active_options: effective_options(config, pkg),
        cflags: config.make.cflags.clone(),
        cxxflags: config.make.cxxflags.clone(),
        ldflags: config.make.ldflags.clone(),
        patches_sha256: if with_hashes {
            crate::overlay::user_file_hashes(&config.config_dir, pkg)
        } else {
            Vec::new()
        },
        template_sha256: if with_hashes {
            template_hash(&config.void_packages_dir(), pkg)
        } else {
            None
        },
    }
}

/// Paquetes instalados desde fuente con sus discrepancias (una lista por
/// paquete: cada dimensión se evalúa por separado).
///
/// `portdb_version` permite el cruce con el árbol de fuentes (ITEM 2); sin él
/// solo se evalúan opciones/banderas/parches/template.
pub fn candidates(
    config: &Config,
    portdb_version: Option<VersionLookup<'_>>,
) -> BTreeMap<String, Vec<RebuildReason>> {
    let mut out = BTreeMap::new();
    let Ok(db) = crate::db::InstalledDb::load(config.installed_db_path()) else {
        return out;
    };
    for (name, entry) in db.entries_snapshot() {
        if entry.install_type != InstallType::Source {
            continue;
        }
        // Los hashes solo se calculan si hay baseline: si no, la comparación
        // se salta (no hay señal) y leer ficheros sería trabajo inútil.
        let with_hashes = entry.patches_sha256.is_some() || entry.template_sha256.is_some();
        let target = target_for(config, &name, with_hashes);
        // El índice de fuentes es el de void-packages: solo dice algo de los
        // paquetes construidos DESDE ahí. Para un paquete de un VUR su fuente
        // vive en el clon del VUR, así que el homónimo del índice NO es su
        // fuente: compararlos daba "versión nueva" falsa.
        let version = if entry.vur == crate::db::OFFICIAL_REPO {
            portdb_version.and_then(|f| f(&name))
        } else {
            None
        };
        let reasons = audit_all(&entry, &target, version.as_deref(), portdb_version);
        if !reasons.is_empty() {
            out.insert(name, reasons);
        }
    }
    out
}

/// Lookup de versión en el árbol de fuentes (ITEM 2 lo alimenta).
pub type VersionLookup<'a> = &'a dyn Fn(&str) -> Option<String>;

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
            deps_pinned: None,
        }
    }

    /// Primera discrepancia detectada (para asserts legibles).
    fn audit(
        entry: &Entry,
        target: &BuildTarget,
        portdb_version: Option<&str>,
    ) -> Option<RebuildReason> {
        audit_all(entry, target, portdb_version, None)
            .into_iter()
            .next()
    }

    fn target() -> BuildTarget {
        BuildTarget {
            active_options: vec!["pgo".to_string()],
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
        assert!(audit_all(&entry_source(), &target(), None, None).is_empty());
    }

    /// El índice de fuentes es el de void-packages: su versión solo dice
    /// algo de los paquetes construidos DESDE ahí. Para un paquete de un VUR
    /// su fuente vive en el clon del VUR, así que el homónimo del índice no
    /// es su fuente y compararlos daba "versión nueva" falsa.
    #[test]
    fn la_version_del_indice_no_aplica_a_los_paquetes_de_un_vur() {
        let dir = tempfile::tempdir().unwrap();
        let config = crate::config::Config {
            cache_dir: dir.path().to_path_buf(),
            config_dir: dir.path().join("conf"),
            ..crate::config::Config::default()
        };
        let mut db = crate::db::InstalledDb::load(config.installed_db_path()).unwrap();
        db.upsert(
            "oficial",
            "oficial-1.0_1",
            crate::db::OFFICIAL_REPO,
            InstallType::Source,
            None,
            None,
        );
        db.upsert(
            "de-vur",
            "de-vur-1.0_1",
            "mi-vur",
            InstallType::Source,
            None,
            None,
        );
        db.save().unwrap();

        let lookup = |_: &str| Some("otra-9.9_9".to_string());
        let out = candidates(&config, Some(&lookup));
        assert_eq!(
            out.get("oficial"),
            Some(&vec![RebuildReason::NewVersion]),
            "{out:?}"
        );
        assert!(!out.contains_key("de-vur"), "{out:?}");
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
        t.active_options = vec!["wayland".to_string()];
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

    /// Baseline PARCIAL: cflags presente y ldflags ausente. No hay cambio real
    /// de ldflags -> no puede haber falso positivo.
    #[test]
    fn baseline_parcial_no_produce_falso_positivo() {
        let mut e = entry_source();
        e.ldflags = None;
        e.cxxflags = None;
        let mut t = target();
        t.cxxflags = "-O3".to_string();
        t.ldflags = "-Wl,-z,now".to_string();
        assert_eq!(audit(&e, &t, None), None);
    }

    /// Cada dimensión se evalúa por separado: opciones y banderas a la vez NO
    /// se tapan entre sí.
    #[test]
    fn dimensiones_independientes() {
        let mut t = target();
        t.active_options = vec!["wayland".to_string()];
        t.cflags = "-O3".to_string();
        let reasons = audit_all(&entry_source(), &t, None, None);
        assert!(reasons.contains(&RebuildReason::OptionsChanged));
        assert!(reasons.contains(&RebuildReason::FlagsChanged));
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
        t.active_options = vec!["otra".to_string()];
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

    /// Dep desfasada: la dep del árbol cambió de versión desde el build.
    #[test]
    fn dep_cambiada_en_el_arbol_es_rebuild() {
        let mut e = entry_source();
        e.deps_pinned = Some(BTreeMap::from([(
            "libfoo".to_string(),
            "libfoo-1.2.3_1".to_string(),
        )]));
        let lookup = |pkg: &str| match pkg {
            "libfoo" => Some("libfoo-1.3.0_1".to_string()),
            _ => None,
        };
        let reasons = audit_all(&e, &target(), None, Some(&lookup));
        assert!(reasons.contains(&RebuildReason::DepChanged {
            dep: "libfoo".to_string()
        }));
        assert!(RebuildReason::DepChanged {
            dep: "libfoo".to_string()
        }
        .triggers_deps());
        assert!(!RebuildReason::DepChanged {
            dep: "libfoo".to_string()
        }
        .triggers_options());
    }

    /// La dep no cambió → no hay rebuild (ni ruido).
    #[test]
    fn dep_sin_cambio_no_es_rebuild() {
        let mut e = entry_source();
        e.deps_pinned = Some(BTreeMap::from([(
            "libfoo".to_string(),
            "libfoo-1.2.3_1".to_string(),
        )]));
        let lookup = |pkg: &str| match pkg {
            "libfoo" => Some("libfoo-1.2.3_1".to_string()),
            _ => None,
        };
        assert!(audit_all(&e, &target(), None, Some(&lookup)).is_empty());
    }

    /// Sin baseline no hay señal; una dep que el árbol no conoce tampoco
    /// (ausencia de dato ≠ señal).
    #[test]
    fn dep_sin_baseline_o_desconocida_no_es_rebuild() {
        let lookup = |pkg: &str| match pkg {
            "libfoo" => Some("libfoo-1.3.0_1".to_string()),
            _ => None,
        };
        // Sin baseline.
        assert!(audit_all(&entry_source(), &target(), None, Some(&lookup)).is_empty());
        // Con baseline, pero la dep no está en el árbol.
        let mut e = entry_source();
        e.deps_pinned = Some(BTreeMap::from([(
            "solo-vur".to_string(),
            "solo-vur-1.0_1".to_string(),
        )]));
        assert!(audit_all(&e, &target(), None, Some(&lookup)).is_empty());
    }
}
