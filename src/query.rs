//! `vary -Q`: consulta de paquetes instalados (compatible con pacman).
//!
//! El hueco que deja ver `kyo list` de kyoz (wrapper de xbps): Vary tenía
//! `xbps-query -l` internamente (para el set `@installed`) pero **no** una
//! forma de listar lo instalado desde la CLI. `-Q` es el verbo de pacman para
//! eso, así que se implementa con su nombre y su forma.
//!
//! Salida: `nombre version` por línea (dos columnas, parseable por script,
//! como el resto de salidas de Vary).

use crate::config::Config;
use anyhow::{Context, Result};

/// Una línea de `xbps-query -l` → `(nombre, version)`.
///
/// Formato real (void-linux/xbps, `bin/xbps-query/list.c`):
/// `ii bash-5.2.21_1  The GNU Bourne Again shell`
/// (estado, pkgver, descripción). El estado es `ii` (installed), `uu`
/// (unpacked) o `hr` (half-removed) y se ignora: la lista es de instalados.
///
/// La versión se separa del nombre por el ÚLTIMO guion (mismo criterio que
/// `xbps::split_pkgver_name`): en `pkgver` = `nombre-version_rev` la versión
/// no lleva guiones.
pub fn parse_installed_line(line: &str) -> Option<(String, String)> {
    let mut cols = line.split_whitespace();
    let _state = cols.next()?;
    let pkgver = cols.next()?;
    let (name, version) = match pkgver.rsplit_once('-') {
        Some((name, version)) => (name, version),
        None => (pkgver, ""),
    };
    Some((name.to_string(), version.to_string()))
}

/// ¿El paquete casa con alguno de los patrones? Subcadena del nombre (como
/// el filtro de `xbps-query -l <patrón>`). Sin patrones: todos.
pub fn matches_patterns(name: &str, patterns: &[String]) -> bool {
    patterns.is_empty() || patterns.iter().any(|p| name.contains(p.as_str()))
}

/// Lista los paquetes instalados: `nombre version` por línea.
pub fn list(config: &Config) -> Result<i32> {
    let out = crate::xbps::run_capture("xbps-query", &["-l"])
        .with_context(|| "no se pudo ejecutar `xbps-query -l`")?;
    if !out.status.success() {
        anyhow::bail!(
            "`xbps-query -l` terminó con {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines() {
        let Some((name, version)) = parse_installed_line(line) else {
            continue;
        };
        if !matches_patterns(&name, &config.targets) {
            continue;
        }
        println!("{name} {version}");
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn parsea_la_linea_real_de_xbps_query_l() {
        // Formato real: estado, pkgver, descripción.
        let (name, version) =
            parse_installed_line("ii bash-5.2.21_1              The GNU Bourne Again shell")
                .expect("línea válida");
        assert_eq!(name, "bash");
        assert_eq!(version, "5.2.21_1");
    }

    #[test]
    fn el_nombre_con_guiones_no_se_parte_mal() {
        let (name, version) =
            parse_installed_line("ii x86_64-elf-gnu-gcc-14.2.1_1  GCC").expect("línea válida");
        assert_eq!(name, "x86_64-elf-gnu-gcc");
        assert_eq!(version, "14.2.1_1");
    }

    #[test]
    fn lineas_raras_no_rompen_la_lista() {
        assert!(parse_installed_line("").is_none());
        assert!(parse_installed_line("ii").is_none());
        // Sin guion: el pkgver entero es el nombre y no hay versión.
        let (name, version) = parse_installed_line("ii weirdpkg  desc").expect("línea válida");
        assert_eq!(name, "weirdpkg");
        assert_eq!(version, "");
    }

    #[test]
    fn filtro_por_patron() {
        assert!(matches_patterns("bash", &[]));
        assert!(matches_patterns("bash", &s(&["ba"])));
        assert!(!matches_patterns("bash", &s(&["zsh"])));
        assert!(matches_patterns("bash", &s(&["zsh", "bash"])));
    }
}
