//! Conf de xbps-src por invocación (compilación híbrida desde fuente).
//!
//! `xbps-src -c <cfg>` lee `etc/conf.<cfg>`: un archivo por build, generado
//! justo antes de invocar xbps-src y borrado en Drop. El `etc/conf` normal
//! no sirve para per-paquete: se sourcea UNA vez al arrancar, antes de que
//! existan `$pkgname`/`$sourcepkg`.
//!
//! Restricciones verificadas contra xbps-src: al chroot solo pasan líneas
//! que empiezan por `XBPS_` (RUSTFLAGS/CFLAGS no se copian y
//! `XBPS_RUSTFLAGS` NO existe; xbps-src hace `unset RUSTFLAGS` en nativo),
//! así que aquí no se emiten.
use crate::config::Config;
use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Conf relevante para un build, capturado de Config (owned: lo clonan los
/// workers sin pelear con &Config).
#[derive(Debug, Clone, Default)]
pub struct PkgConf {
    pub make: crate::config::MakeSettings,
    pub options: BTreeMap<String, Vec<String>>,
}

impl PkgConf {
    pub fn capture(config: &Config) -> Self {
        Self {
            make: config.make.clone(),
            options: config.pkg_options.clone(),
        }
    }
}

/// Normaliza el nombre para la variable `XBPS_PKG_OPTIONS_<pkg>`: repone por
/// `_` cualquier carácter fuera de [A-Za-z0-9_] (regla de xbps-src).
pub fn normalize_pkgname(pkg: &str) -> String {
    pkg.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Valor seguro para shell: entre comillas simples (`'` -> `'\''`); rechaza
/// saltos de línea (no representables sin here-doc).
fn quote(v: &str) -> Result<String> {
    anyhow::ensure!(
        !v.contains('\n'),
        "valor con salto de línea no representable en el conf xbps-src: {v:?}"
    );
    Ok(v.replace('\'', "'\\''"))
}

/// Renderiza el conf para UN paquete. Puro y testeable: emite SOLO las
/// líneas que aplican (sin `case`, sin `XBPS_RUSTFLAGS`, sin
/// `XBPS_MAKEJOBS`: el `-j` existente ya lo controla).
pub fn render(pkg: &str, conf: &PkgConf) -> Result<String> {
    let m = &conf.make;
    let mut lines: Vec<String> = Vec::new();
    for (var, val) in [
        ("XBPS_CFLAGS", &m.cflags),
        ("XBPS_CXXFLAGS", &m.cxxflags),
        ("XBPS_LDFLAGS", &m.ldflags),
        ("XBPS_DISTCC_HOSTS", &m.distcc_hosts),
    ] {
        if !val.is_empty() {
            lines.push(format!("{var}='{}'", quote(val)?));
        }
    }
    if m.ccache {
        lines.push("XBPS_CCACHE=yes".to_string());
    }
    if m.distcc {
        lines.push("XBPS_DISTCC=yes".to_string());
    }
    if !m.global_options.is_empty() {
        // xbps-src parte la lista por COMAS (`IFS=','` en set_build_options,
        // common/xbps-src/shutils/common.sh): con espacios el token no
        // matchea ninguna opción y TODO lo que no esté en
        // `build_options_default` queda desactivado en silencio.
        let joined = m.global_options.join(",");
        lines.push(format!("XBPS_PKG_OPTIONS='{}'", quote(&joined)?));
    }
    if let Some(opts) = conf.options.get(pkg) {
        if !opts.is_empty() {
            let var = format!("XBPS_PKG_OPTIONS_{}", normalize_pkgname(pkg));
            let joined = opts.join(",");
            lines.push(format!("{var}='{}'", quote(&joined)?));
        }
    }
    if m.debug_pkgs {
        lines.push("XBPS_DEBUG_PKGS=yes".to_string());
    }
    if m.check_pkgs {
        lines.push("XBPS_CHECK_PKGS=yes".to_string());
    }
    if m.allow_restricted {
        lines.push("XBPS_ALLOW_RESTRICTED=yes".to_string());
    }
    let mut text = lines.join("\n");
    if !lines.is_empty() {
        text.push('\n');
    }
    Ok(text)
}

/// Id auto por invocación: `pid-contador` (contador atómico del proceso).
pub fn auto_id() -> String {
    static N: AtomicU64 = AtomicU64::new(1);
    format!(
        "{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    )
}

/// Guard RAII del archivo de conf: lo borra en Drop (best-effort).
pub struct ConfGuard {
    path: Option<PathBuf>,
}

impl Drop for ConfGuard {
    fn drop(&mut self) {
        if let Some(p) = self.path.take() {
            if let Err(err) = std::fs::remove_file(&p) {
                tracing::debug!("no se pudo limpiar el conf {}: {err}", p.display());
            }
        }
    }
}

/// Escribe `etc/conf.vary-<id>` con el conf para `pkg`: primero a un temp
/// único en el mismo directorio y `rename` al destino (atómico); devuelve el
/// sufijo tal cual lo espera `xbps-src -c` (`vary-<id>`) y el guard.
pub fn write(conf_dir: &Path, id: &str, pkg: &str, conf: &PkgConf) -> Result<(String, ConfGuard)> {
    let suffix = format!("vary-{id}");
    let path = conf_dir.join(format!("conf.{suffix}"));
    let text = render(pkg, conf).with_context(|| format!("generando {}", path.display()))?;
    crate::util::ensure_private_dir(conf_dir)
        .with_context(|| format!("preparando {}", conf_dir.display()))?;
    let tmp = conf_dir.join(format!("conf.{suffix}.tmp"));
    std::fs::write(&tmp, text).with_context(|| format!("escribiendo {}", tmp.display()))?;
    if let Err(err) = std::fs::rename(&tmp, &path) {
        // No dejar residuo del temp ante un rename fallido.
        let _ = std::fs::remove_file(&tmp);
        return Err(anyhow::Error::new(err).context(format!(
            "renombrando {} a {}",
            tmp.display(),
            path.display()
        )));
    }
    Ok((suffix, ConfGuard { path: Some(path) }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal() -> PkgConf {
        PkgConf::default()
    }

    /// Vacío: sin líneas que aplican => archivo vacío.
    #[test]
    fn render_vacio_no_emite_nada() {
        assert_eq!(render("foo", &minimal()).unwrap(), "");
    }

    #[test]
    fn render_emite_lineas_aplicables_exactly() {
        use crate::config::MakeSettings;
        let conf = PkgConf {
            make: MakeSettings {
                cflags: "-O2 -march=native".into(),
                cxxflags: "-O3".into(),
                ldflags: "-Wl,-O1".into(),
                ccache: true,
                distcc: true,
                distcc_hosts: "localhost/4".into(),
                global_options: vec!["static".into(), "docs".into()],
                debug_pkgs: true,
                check_pkgs: true,
                allow_restricted: true,
            },
            options: [(
                "foo".to_string(),
                vec!["pgo".to_string(), "wayland".to_string()],
            )]
            .into_iter()
            .collect(),
        };
        let text = render("foo", &conf).unwrap();
        let expected = "\
XBPS_CFLAGS='-O2 -march=native'
XBPS_CXXFLAGS='-O3'
XBPS_LDFLAGS='-Wl,-O1'
XBPS_DISTCC_HOSTS='localhost/4'
XBPS_CCACHE=yes
XBPS_DISTCC=yes
XBPS_PKG_OPTIONS='static,docs'
XBPS_PKG_OPTIONS_foo='pgo,wayland'
XBPS_DEBUG_PKGS=yes
XBPS_CHECK_PKGS=yes
XBPS_ALLOW_RESTRICTED=yes
";
        assert_eq!(text, expected);
        // Restricciones duras del esquema verificado en Void.
        assert!(!text.contains("case "), "sin case: {text}");
        assert!(!text.contains("RUSTFLAGS"), "sin RUSTFLAGS: {text}");
        assert!(!text.contains("MAKEJOBS"), "sin MAKEJOBS: {text}");
    }

    /// Las opciones van separadas por COMA: xbps-src parte la lista con
    /// `IFS=','`; con espacios el token entero no matchea ninguna opción y
    /// desactiva todo lo que no esté en `build_options_default`.
    #[test]
    fn opciones_separadas_por_coma() {
        use crate::config::MakeSettings;
        let conf = PkgConf {
            make: MakeSettings {
                global_options: vec!["static".into(), "docs".into()],
                ..MakeSettings::default()
            },
            options: [(
                "foo".to_string(),
                vec!["pgo".to_string(), "wayland".to_string()],
            )]
            .into_iter()
            .collect(),
        };
        let text = render("foo", &conf).unwrap();
        assert!(text.contains("XBPS_PKG_OPTIONS='static,docs'\n"), "{text}");
        assert!(
            text.contains("XBPS_PKG_OPTIONS_foo='pgo,wayland'\n"),
            "{text}"
        );
    }

    #[test]
    fn escapa_comillas_simples_y_rechaza_saltos() {
        use crate::config::MakeSettings;
        let conf = PkgConf {
            make: crate::config::MakeSettings {
                cflags: "it's".to_string(),
                ..MakeSettings::default()
            },
            ..minimal()
        };
        let text = render("foo", &conf).unwrap();
        assert_eq!(text, "XBPS_CFLAGS='it'\\''s'\n");
        let conf_nl = PkgConf {
            make: crate::config::MakeSettings {
                cflags: "una\nlinea".to_string(),
                ..MakeSettings::default()
            },
            ..minimal()
        };
        assert!(render("foo", &conf_nl).is_err(), "salto de línea rechazado");
    }

    #[test]
    fn normaliza_foo_bar_a_pkg_options_foo_bar() {
        assert_eq!(normalize_pkgname("foo-bar"), "foo_bar");
        assert_eq!(normalize_pkgname("foo+bar.baz"), "foo_bar_baz");
        assert_eq!(normalize_pkgname("latin1"), "latin1");
        assert_eq!(normalize_pkgname("multilib-32"), "multilib_32");
        // La clave del índice responde al mismo nombre normalizado.
        let conf = PkgConf {
            options: [("foo-bar".to_string(), vec!["pgo".to_string()])]
                .into_iter()
                .collect(),
            ..minimal()
        };
        let text = render("foo-bar", &conf).unwrap();
        assert!(text.contains("XBPS_PKG_OPTIONS_foo_bar='pgo'\n"), "{text}");
        // Otro paquete no hereda opciones ajenas.
        assert!(!render("baz", &conf).unwrap().contains("PKG_OPTIONS"));
    }
}
