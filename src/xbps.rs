//! Capa envoltorio síncrona (std::process::Command, sin tokio) sobre los
//! binarios de XBPS.
//!
//! * Las funciones de consulta NO usan sudo.
//! * `install` y `remove_recursive` delegan la elevación al binario de sudo
//!   que el llamador indique (`sudo_bin` + `sudo_flags`) y heredan stdio para
//!   progreso en vivo; devuelven el código de salida (-1 si muere por señal).
//! * Los errores usan `anyhow` con mensajes accionables: si un binario no
//!   existe => "`<bin>` no encontrado: ¿estás en Void Linux? ¿instalaste xbps?".

use std::borrow::Cow;
use std::io::ErrorKind;
use std::path::Path;
use std::process::{Command, Output};

use anyhow::{Result, anyhow, bail};

const XBPS_QUERY: &str = "xbps-query";

/// Paquete resultado de una consulta o búsqueda.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct PkgInfo {
    pub name: String,
    /// "name-version_revision" (el pkgver completo tal como lo reporta xbps).
    pub pkgver: String,
    pub short_desc: Option<String>,
    pub repository: Option<String>,
    pub installed: bool,
}

/// Línea cruda parseada de `xbps-query -Rs`; ver [`parse_search_line`].
#[derive(Debug, Clone, PartialEq)]
pub struct SearchHit {
    pub name: String,
    /// "name-version_revision".
    pub pkgver: String,
    pub short_desc: Option<String>,
    pub repository: Option<String>,
    pub installed: bool,
}

impl From<SearchHit> for PkgInfo {
    fn from(h: SearchHit) -> Self {
        Self {
            name: h.name,
            pkgver: h.pkgver,
            short_desc: h.short_desc,
            repository: h.repository,
            installed: h.installed,
        }
    }
}

/// Ejecuta `bin args...` y devuelve el [`Output`] completo (capturado).
///
/// Error si falla el spawn; en particular si el binario no existe:
/// "`{bin}` no encontrado: ¿estás en Void Linux? ¿instalaste xbps?".
pub fn run_capture(bin: &str, args: &[&str]) -> Result<Output> {
    Command::new(bin)
        .args(args)
        .output()
        .map_err(|e| spawn_error(bin, e))
}

fn spawn_error(bin: &str, e: std::io::Error) -> anyhow::Error {
    if e.kind() == ErrorKind::NotFound {
        anyhow!("`{bin}` no encontrado: ¿estás en Void Linux? ¿instalaste xbps?")
    } else {
        anyhow!("no se pudo ejecutar `{bin}`: {e}")
    }
}

fn stdout_text(out: &Output) -> Cow<'_, str> {
    String::from_utf8_lossy(&out.stdout)
}

/// Contenido del token `[...]` inicial de `s`, si lo hay.
fn bracketed(s: &str) -> Option<&str> {
    let inner = s.strip_prefix('[')?;
    let close = inner.find(']')?;
    Some(&inner[..close])
}

/// `s` sin su primer token `[...]`, recortando espacios a la izquierda.
fn after_bracketed(s: &str) -> Option<&str> {
    let inner = s.strip_prefix('[')?;
    let close = inner.find(']')?;
    s.get(close + 2..).map(str::trim_start)
}

/// Parsea UNA línea de la salida de `xbps-query -Rs`.
///
/// Supuesto EXACTO de formato implementado (tolerante):
///
/// ```text
/// [<marca>] [[<repo>]] <pkgver>[ <descripción corta>]
/// ```
///
/// * Primer token entre corchetes = marca de estado. `[*]` o `[x]` marcan el
///   paquete como INSTALADO; cualquier otra marca (`[-]`, `[?]`, `[ ]`, …)
///   como no instalado. Sin marca inicial también se acepta (no instalado).
/// * Si INMEDIATAMENTE después hay otro token entre corchetes, se interpreta
///   como nombre del repositorio (p. ej. `[multilib]`).
/// * El primer token sin corchetes es `<pkgver>` = "nombre-version_revision";
///   el nombre se separa con `rsplit_once('-')` porque las versiones XBPS no
///   contienen guiones (`1.2.3_1`), así `foo-bar-1.2.3_1` => nombre `foo-bar`.
/// * Todo lo demás tras `<pkgver>` es la descripción corta (`None` si vacía).
/// * Líneas vacías o sin `<pkgver>` => [`None`].
pub fn parse_search_line(line: &str) -> Option<SearchHit> {
    let mut rest = line.trim();
    if rest.is_empty() {
        return None;
    }

    let installed = if bracketed(rest).is_some() {
        let mark = bracketed(rest)?;
        let hit = matches!(mark, "*" | "x");
        rest = after_bracketed(rest)?;
        hit
    } else {
        false
    };

    let repository = if bracketed(rest).is_some() {
        let repo = bracketed(rest)?.to_owned();
        rest = after_bracketed(rest)?;
        Some(repo)
    } else {
        None
    };

    let pkgver_end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    let pkgver = &rest[..pkgver_end];
    if pkgver.is_empty() {
        return None;
    }
    let short_desc = rest[pkgver_end..].trim();
    let name = pkgver.rsplit_once('-').map_or(pkgver, |(n, _)| n);

    Some(SearchHit {
        name: name.to_owned(),
        pkgver: pkgver.to_owned(),
        short_desc: (!short_desc.is_empty()).then(|| short_desc.to_owned()),
        repository,
        installed,
    })
}

/// Consulta un paquete INSTALADO por nombre exacto:
/// `xbps-query -p pkgver,short_desc,repository <name>`.
///
/// Supuesto de formato: una propiedad por línea, EN ESE ORDEN (pkgver,
/// short_desc, repository) y valores sin clave; propiedades vacías o líneas
/// faltantes se toleran (campo `None`). rc != 0 o salida vacía => `Ok(None)`.
pub fn query_installed(name: &str) -> Result<Option<PkgInfo>> {
    let out = run_capture(XBPS_QUERY, &["-p", "pkgver,short_desc,repository", name])?;
    if !out.status.success() {
        return Ok(None);
    }
    Ok(parse_installed_props(name, &stdout_text(&out)))
}

fn parse_installed_props(name: &str, text: &str) -> Option<PkgInfo> {
    let mut lines = text.lines().map(str::trim);
    let pkgver = lines.next().unwrap_or("");
    if pkgver.is_empty() {
        return None;
    }
    let short_desc = lines.next().unwrap_or("");
    let repository = lines.next().unwrap_or("");
    Some(PkgInfo {
        name: name.to_owned(),
        pkgver: pkgver.to_owned(),
        short_desc: (!short_desc.is_empty()).then(|| short_desc.to_owned()),
        repository: (!repository.is_empty()).then(|| repository.to_owned()),
        installed: true,
    })
}

/// Busca en los repositorios remotos: `xbps-query -Rs <pattern>`.
///
/// Cada línea se parsea según el supuesto exacto documentado en
/// [`parse_search_line`] (marca opcional `[...]` con `[*]`/`[x]` =>
/// instalado, repo opcional `[repo]`, luego `pkgver` y descripción). Se
/// filtran las líneas que no producen resultados (vacías o malformadas). Un
/// rc != 0 se tolera: salida sin hits equivale a `Ok(vec![])`.
pub fn search_remote(pattern: &str) -> Result<Vec<PkgInfo>> {
    let out = run_capture(XBPS_QUERY, &["-Rs", pattern])?;
    Ok(stdout_text(&out)
        .lines()
        .filter_map(parse_search_line)
        .map(PkgInfo::from)
        .collect())
}

/// URLs de repositorios configurados en el sistema (`xbps-query -L`),
/// EXCLUYENDO los locales de vary (rutas absolutas / hostdir/binpkgs).
///
/// Formato de línea: `<idx> <url> (RSA signed|unsigned)`; se ignora cualquier
/// línea cuyo segundo token no empiece por `http://`, `https://` o `file://`
/// que apunte fuera de vary. Si `-L` falla, devuelve vacío.
pub fn official_repo_urls() -> Vec<String> {
    let out = match run_capture(XBPS_QUERY, &["-L"]) {
        Ok(o) if o.status.success() => o,
        _ => return Vec::new(),
    };
    stdout_text(&out)
        .lines()
        .filter_map(|line| {
            let url = line.split_whitespace().nth(1)?;
            if !url.starts_with("http://") && !url.starts_with("https://") && !url.starts_with("file://") {
                return None;
            }
            // Repositorio local de vary: excluirlo siempre
            if url.contains("hostdir/binpkgs") {
                return None;
            }
            Some(url.to_string())
        })
        .collect()
}

/// Igual que [`search_remote`] pero consultando SOLO los repos oficiales del
/// sistema (ignora `xbps.d` y pasa explícitamente las URLs no-locales), para
/// que los builds locales de vary no contaminen la búsqueda federada.
///
/// Sin URLs oficiales detectadas => degrada a [`search_remote`] normal.
pub fn search_official(pattern: &str) -> Result<Vec<PkgInfo>> {
    let urls = official_repo_urls();
    if urls.is_empty() {
        return search_remote(pattern);
    }
    let mut args: Vec<&str> = vec!["--ignore-conf-repos"];
    for u in &urls {
        args.push("--repository");
        args.push(u);
    }
    args.push("-Rs");
    args.push(pattern);
    let out = run_capture(XBPS_QUERY, &args)?;
    Ok(stdout_text(&out)
        .lines()
        .filter_map(parse_search_line)
        .map(PkgInfo::from)
        .collect())
}

/// Lista los paquetes instalados manualmente: `xbps-query -m`.
///
/// Cada línea se parte por espacios en blanco: si hay >=2 columnas se
/// devuelve la segunda (formato tipo `ii name-ver ...`); si solo hay una, la
/// línea entera recortada. Líneas vacías se ignoran; rc != 0 se tolera igual
/// que en [`search_remote`] (sin hits => `Ok(vec![])`).
pub fn query_manual() -> Result<Vec<String>> {
    let out = run_capture(XBPS_QUERY, &["-m"])?;
    Ok(manual_names(&stdout_text(&out)))
}

fn manual_names(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|l| {
            let mut cols = l.split_whitespace();
            let first = cols.next().unwrap_or_default();
            match cols.next() {
                Some(second) => second.to_owned(),
                None => first.to_owned(),
            }
        })
        .collect()
}

/// Arquitectura objetivo del sistema. La usa:
///
/// * el resolver, para filtrar entradas `.VURINFO.archs`, y
/// * vur_client, para construir rutas per-arquitectura del host
///   (`hostdir/binpkgs/<arch>`).
///
/// OPCIÓN B PRIMARIA: `xbps-query -R -p architecture base-system`. Si el
/// binario falta o el comando falla, FALLBACK determinista:
/// [`std::env::consts::ARCH`].
pub fn query_architecture() -> Result<String> {
    let out = run_capture(XBPS_QUERY, &["-R", "-p", "architecture", "base-system"])?;
    if out.status.success() {
        if let Some(line) = stdout_text(&out).lines().next() {
            let arch = line.trim();
            if !arch.is_empty() {
                return Ok(arch.to_owned());
            }
        }
    }
    Ok(std::env::consts::ARCH.to_owned())
}

/// Instala `targets` ejecutando
/// `[wrapper] xbps-install -S ...extra_flags ...targets`
/// heredando stdio; devuelve el código de salida (-1 si muere por señal).
/// La elevación es agnóstica (ver `crate::elevate`): sudo, doas, run0 o
/// directa si ya somos root.
pub fn install(
    targets: &[String],
    extra_flags: &[String],
    sudo_bin: &str,
    sudo_flags: &[String],
) -> Result<i32> {
    let mut cmd = crate::elevate::elevate(sudo_bin, sudo_flags, "xbps-install")?;
    cmd.arg("-S")
        .args(extra_flags)
        .args(targets);
    status_code(&mut cmd)
}

/// Elimina recursivamente `targets` ejecutando
/// `[wrapper] xbps-remove -Ro ...extra_flags ...targets` heredando
/// stdio; devuelve el código de salida (-1 si muere por señal).
pub fn remove_recursive(
    targets: &[String],
    extra_flags: &[String],
    sudo_bin: &str,
    sudo_flags: &[String],
) -> Result<i32> {
    let mut cmd = crate::elevate::elevate(sudo_bin, sudo_flags, "xbps-remove")?;
    cmd.arg("-Ro")
        .args(extra_flags)
        .args(targets);
    status_code(&mut cmd)
}

fn status_code(cmd: &mut Command) -> Result<i32> {
    // get_program() es el wrapper si hay elevación, o el programa directo
    // como root: el mensaje de error siempre nombra al binario que falló.
    let prog = cmd.get_program().to_string_lossy().to_string();
    let status = cmd.status().map_err(|e| spawn_error(&prog, e))?;
    Ok(status.code().unwrap_or(-1))
}

/// Ejecuta `./xbps-src <args>` con `current_dir(masterdir)` heredando stdio
/// (progreso en vivo); devuelve el código de salida.
///
/// Error claro si `masterdir/xbps-src` no existe.
pub fn xbps_src(masterdir: &Path, args: &[&str]) -> Result<i32> {
    let script = masterdir.join("xbps-src");
    if !script.is_file() {
        bail!(
            "`{}` no encontrado: ¿clonaste void-packages y está el script xbps-src marcado como ejecutable?",
            script.display()
        );
    }
    let mut cmd = Command::new("./xbps-src");
    cmd.current_dir(masterdir).args(args);
    status_code(&mut cmd)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------- parse_search_line ----------

    #[test]
    fn search_line_basica_no_instalada() {
        let hit = parse_search_line("[-] foo-1.2.3_1  Short description here").unwrap();
        assert_eq!(hit.name, "foo");
        assert_eq!(hit.pkgver, "foo-1.2.3_1");
        assert_eq!(hit.short_desc.as_deref(), Some("Short description here"));
        assert_eq!(hit.repository, None);
        assert!(!hit.installed);
    }

    #[test]
    fn search_line_nombre_con_guiones() {
        let hit = parse_search_line("[-] foo-bar-1.2.3_1  A package with-hyphens").unwrap();
        assert_eq!(hit.name, "foo-bar");
        assert_eq!(hit.pkgver, "foo-bar-1.2.3_1");
        assert_eq!(hit.short_desc.as_deref(), Some("A package with-hyphens"));
        assert!(!hit.installed);
    }

    #[test]
    fn search_line_marcas_de_instalado() {
        for mark in ["[*]", "[x]"] {
            let hit = parse_search_line(&format!("{mark} zzz-0.1_1  desc")).unwrap();
            assert!(hit.installed, "marca {mark} debe ser instalada");
        }
        for mark in ["[-]", "[ ]", "[?]"] {
            let hit = parse_search_line(&format!("{mark} zzz-0.1_1  desc")).unwrap();
            assert!(!hit.installed, "marca {mark} no debe ser instalada");
        }
    }

    #[test]
    fn search_line_con_repositorio() {
        let hit = parse_search_line("[-] [multilib] gcc-multilib-4.1_1  GCC with multilib").unwrap();
        assert_eq!(hit.repository.as_deref(), Some("multilib"));
        assert_eq!(hit.name, "gcc-multilib");
        assert_eq!(hit.pkgver, "gcc-multilib-4.1_1");
        assert_eq!(hit.short_desc.as_deref(), Some("GCC with multilib"));
        assert!(!hit.installed);
    }

    #[test]
    fn search_line_sin_marca_inicial() {
        let hit = parse_search_line("bare-0.1_1  Desc").unwrap();
        assert_eq!(hit.name, "bare");
        assert_eq!(hit.pkgver, "bare-0.1_1");
        assert_eq!(hit.short_desc.as_deref(), Some("Desc"));
        assert!(!hit.installed);
    }

    #[test]
    fn search_line_sin_descripcion() {
        let hit = parse_search_line("[-] solo-9_1").unwrap();
        assert_eq!(hit.name, "solo");
        assert_eq!(hit.pkgver, "solo-9_1");
        assert_eq!(hit.short_desc, None);
    }

    #[test]
    fn search_line_vacias_o_degeneradas_devuelven_none() {
        for line in ["", "   \t ", "[-]", "[-]   "] {
            assert!(parse_search_line(line).is_none(), "linea {line:?}");
        }
    }

    // ---------- parse_installed_props (query_installed) ----------

    #[test]
    fn props_completas() {
        let info = parse_installed_props("foo", "foo-1.0_1\nShort desc\nhttps://repo.void\n")
            .unwrap();
        assert_eq!(info.name, "foo");
        assert_eq!(info.pkgver, "foo-1.0_1");
        assert_eq!(info.short_desc.as_deref(), Some("Short desc"));
        assert_eq!(info.repository.as_deref(), Some("https://repo.void"));
        assert!(info.installed);
    }

    #[test]
    fn props_toleran_campos_vacios_o_faltantes() {
        let info = parse_installed_props("foo", "foo-1.0_1\n\n").unwrap();
        assert_eq!(info.pkgver, "foo-1.0_1");
        assert_eq!(info.short_desc, None);
        assert_eq!(info.repository, None);

        let info = parse_installed_props("foo", "foo-2.0_1\ndesc\n").unwrap();
        assert_eq!(info.short_desc.as_deref(), Some("desc"));
        assert_eq!(info.repository, None);
    }

    #[test]
    fn props_sin_pkgver_es_none() {
        assert!(parse_installed_props("foo", "").is_none());
        assert!(parse_installed_props("foo", "\n\ndesc\n").is_none());
    }

    // ---------- manual ----------

    #[test]
    fn manual_segunda_columna() {
        assert_eq!(
            manual_names("ii foo-1.0_1  descripcion larga\n"),
            vec!["foo-1.0_1".to_owned()]
        );
    }

    #[test]
    fn manual_una_sola_columna_usa_toda_la_linea() {
        assert_eq!(
            manual_names("bar-2.0_1\n"),
            vec!["bar-2.0_1".to_owned()]
        );
    }

    #[test]
    fn manual_ignora_lineas_vacias() {
        assert_eq!(
            manual_names("\n   \nii a-1_1 x\nb-1_1\n"),
            vec!["a-1_1".to_owned(), "b-1_1".to_owned()]
        );
    }

    // ---------- run_capture (mensaje accionable, sin invocar xbps real) ----------

    #[test]
    fn run_capture_binario_ausente_da_mensaje_accionable() {
        let err = run_capture("definitivamente-no-existe-xyz-1234", &[]).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("`definitivamente-no-existe-xyz-1234` no encontrado"), "{msg}");
        assert!(msg.contains("Void Linux"), "{msg}");
    }

    // ---------- integración (requieren Void Linux; ignoradas por defecto) ----------

    #[test]
    #[ignore = "requiere Void Linux con xbps-query instalado"]
    fn integracion_query_installed_real() {
        if let Some(info) = query_installed("bash").unwrap() {
            assert_eq!(info.name, "bash");
            assert!(info.installed);
            assert!(info.pkgver.starts_with("bash-"));
        }
    }

    #[test]
    #[ignore = "requiere Void Linux con xbps-query y repos configurados"]
    fn integracion_search_remote_real() {
        let hits = search_remote("xbps").unwrap();
        assert!(!hits.is_empty());
    }

    #[test]
    #[ignore = "requiere Void Linux con xbps-query instalado"]
    fn integracion_query_manual_real() {
        let _ = query_manual().unwrap();
    }

    #[test]
    #[ignore = "requiere Void Linux con xbps-query y repos configurados"]
    fn integracion_query_architecture_real() {
        let arch = query_architecture().unwrap();
        assert!(!arch.is_empty());
    }
}
