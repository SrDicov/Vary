//! Adaptador binario para repos estilo VUP (Fase 1).
//!
//! Los repos VUP (p. ej. VUP-Linux/vup) no publican `.VURINFO`: distribuyen
//! binarios como repositorios XBPS estándar (un GitHub Release por tupla
//! `categoría-arquitectura` con `*-repodata` firmado) más un `index.json`
//! central que mapea `paquete → {categoría, versión, archs, repo_urls}`.
//!
//! Este módulo:
//! 1. Descarga ese `index.json` con `curl` (binario configurable,
//!    ver `Config::curl_bin`) y lo cachea en disco con TTL.
//! 2. Convierte sus entradas en `VurInfo` sintéticos (solo metadatos para
//!    resolución binaria: sin dependencias evaluadas) para la arquitectura
//!    actual, junto a la URL del repo binario que les corresponde.
//! 3. Decodifica la llave pública del repo desde `keys/*.plist` (formato
//!    plist XML de VUP, que envuelve el PEM en base64) para el flujo de
//!    `keys::setup_vup_binary_repo`.
//!
//! Límites conocidos de la Fase 1: no hay compilación desde fuente de estos
//! repos (el layout `srcpkgs/<categoría>/<pkg>` no lo entiende el loader
//! VUR), ni `-Si` detallado, ni upgrade automático de estos paquetes.

use crate::metadata::VurInfo;
use anyhow::{Context, Result};
use base64::{engine::general_purpose, Engine as _};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};

/// Raíz del `index.json` estilo VUP (generado por `generate_index.py`).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct VupIndex {
    #[serde(default)]
    pub packages: BTreeMap<String, VupPackage>,
}

/// Una entrada del índice: categoría, `versión_revisión`, arquitecturas y
/// una URL de repo binario por arquitectura.
#[derive(Debug, Clone, Deserialize)]
pub struct VupPackage {
    #[serde(default)]
    pub category: String,
    /// Versión en formato XBPS `versión_revisión` (p. ej. `0.5.0_1`).
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub archs: Vec<String>,
    #[serde(default)]
    pub repo_urls: BTreeMap<String, String>,
}

/// Sanitiza un nombre de repo para usarlo como parte de un nombre de archivo
/// de caché (las claves de repos.conf son arbitrarias).
pub fn sanitize_repo_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c == '/' || c == '\\' || c == '\0' {
                '_'
            } else {
                c
            }
        })
        .collect()
}

/// Tope del `index.json` remoto: el índice real de VUP ocupa unos cientos de
/// KB; 64 MiB es generoso y evita que un repo hostil (T3) lo convierta en un
/// OOM (curl descarga a disco, pero el `read` lo carga entero en RAM).
pub const MAX_INDEX_BYTES: u64 = 64 * 1024 * 1024;

/// Tope del plist de la llave (`keys/*.plist`): los reales pesan unos KB.
pub const MAX_PLIST_BYTES: u64 = 1024 * 1024;

/// Comprueba el tamaño del índice descargado.
fn check_index_size(len: u64) -> Result<()> {
    anyhow::ensure!(
        len <= MAX_INDEX_BYTES,
        "el índice remoto ocupa {len} bytes (tope {MAX_INDEX_BYTES}): se rechaza"
    );
    Ok(())
}

/// ¿la caché en disco sigue vigente?
fn cache_is_fresh(path: &Path, ttl_secs: u64) -> bool {
    let mtime = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    let Some(mtime) = mtime else {
        return false;
    };
    let age = SystemTime::now()
        .duration_since(mtime)
        .unwrap_or(Duration::ZERO);
    age < Duration::from_secs(ttl_secs)
}

fn parse_index_bytes(bytes: &[u8]) -> Result<VupIndex> {
    serde_json::from_slice(bytes).context("el índice remoto no es un index.json válido")
}

/// Descarga el `index.json` (con caché en disco).
///
/// Si la caché es más reciente que `ttl_secs`, no toca la red. Si la descarga
/// falla pero hay una caché vieja, la usa con un aviso. Sin red ni caché,
/// falla sugiriendo `--curl` si el problema es el binario.
pub fn fetch_index(
    curl_bin: &str,
    url: &str,
    cache_path: &Path,
    ttl_secs: u64,
) -> Result<VupIndex> {
    if cache_is_fresh(cache_path, ttl_secs) {
        if let Ok(bytes) = std::fs::read(cache_path) {
            if let Ok(idx) = parse_index_bytes(&bytes) {
                return Ok(idx);
            }
        }
    }

    let url = url.trim();
    if url.is_empty() {
        anyhow::bail!("index_url vacío");
    }
    // La URL viene de repos.conf: es un DATO. curl parsea opciones en
    // cualquier posición, así que una URL tipo `-K /tmp/evil` se comía un
    // fichero de configuración externo.
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        anyhow::bail!("index_url debe ser http(s): '{url}'");
    }

    // Descarga a un temporal de nombre ÚNICO (el flock global no cubre los
    // comandos de solo lectura: dos búsquedas concurrentes comparten el
    // mismo "index.tmp" y una renombra el contenido de la otra).
    let parent = cache_path.parent().unwrap_or_else(|| Path::new("."));
    let tmp = tempfile::NamedTempFile::new_in(parent)
        .with_context(|| format!("creando temporal para {}", cache_path.display()))?;
    let tmp_path = tmp.path().to_path_buf();
    let out = Command::new(curl_bin)
        .args([
            "-fsSL",
            "--max-time",
            "60",
            // T3: el repo remoto decide el tamaño; sin tope, 60 s a 100 MB/s
            // son 6 GB y el `read` de más abajo se los come en RAM (OOM).
            "--max-filesize",
            &MAX_INDEX_BYTES.to_string(),
            "-o",
        ])
        .arg(&tmp_path)
        .arg("--")
        .arg(url)
        .output()
        .with_context(|| {
            format!("no se pudo ejecutar '{curl_bin}': ¿está instalado? (flag --curl / [general] curl_bin)")
        })?;

    let status = out.status;
    if status.success() {
        // Tope también al leer (no depender solo de curl). Si el índice es
        // demasiado grande NO se aborta: el dato es inválido, no el estado →
        // se cae al fallback de la caché vieja (fail-soft).
        let size = std::fs::metadata(&tmp_path).map(|m| m.len()).unwrap_or(0);
        if let Err(err) = check_index_size(size) {
            tracing::warn!("{err:#}");
        } else {
            match std::fs::read(&tmp_path)
                .ok()
                .and_then(|bytes| parse_index_bytes(&bytes).ok())
            {
                Some(idx) => {
                    if let Some(parent) = cache_path.parent() {
                        if !parent.as_os_str().is_empty() {
                            let _ = crate::util::ensure_private_dir(parent);
                        }
                    }
                    // La caché es solo una optimización: si no se puede escribir, seguir igual.
                    // El tmp ya tiene los bytes (los escribió curl): renombrar es
                    // atómico y una línea menos que reescribir (un `write` a
                    // medias dejaba la caché truncada).
                    let _ = std::fs::rename(&tmp_path, cache_path);
                    return Ok(idx);
                }
                None => {
                    let _ = std::fs::remove_file(&tmp_path);
                    tracing::warn!("el índice remoto {url} no parsea como index.json");
                }
            }
        }
    } else {
        tracing::warn!("no se pudo descargar el índice {url}");
    }

    // Fallback a caché vieja.
    if cache_path.is_file() {
        if let Ok(bytes) = std::fs::read(cache_path) {
            if let Ok(idx) = parse_index_bytes(&bytes) {
                tracing::warn!("usando caché vieja de {url}");
                return Ok(idx);
            }
        }
    }
    // El stderr de curl se descarta con `.status()`: el usuario veía
    // "no disponible" sin saber si fue DNS, TLS o tamaño.
    let detail = String::from_utf8_lossy(&out.stderr);
    let detail = detail.trim();
    anyhow::bail!(
        "índice {url} no disponible (sin red ni caché válida){}",
        if detail.is_empty() {
            String::new()
        } else {
            format!(": {detail}")
        }
    )
}

/// Divide `versión_revisión` (formato XBPS) en sus partes.
/// Devuelve `None` si no hay guion bajo, la revisión no es un entero >= 1
/// o la versión queda vacía.
pub fn split_pkgver(pkgver: &str) -> Option<(String, u32)> {
    let (version, revision) = pkgver.rsplit_once('_')?;
    // La versión no puede llevar "_" (delimita nombre-versión_revisión): un
    // "1.2.3_4_5" daría un pkgver sintético malformado.
    if version.is_empty() || version.contains('_') {
        return None;
    }
    let revision: u32 = revision.parse().ok()?;
    if revision < 1 {
        return None;
    }
    Some((version.to_string(), revision))
}

/// Convierte una entrada del índice en `VurInfo` sintético para `arch`.
///
/// `repo`: el nombre del repo VUR (solo para los avisos: el índice remoto es
/// la fuente del nombre y el aviso tiene que decir de dónde sale).
///
/// Devuelve también la URL del repo binario que sirve ese paquete en esa
/// arquitectura. `None` si el paquete no soporta `arch`, no declara URL
/// para ella o su versión no parsea.
pub fn to_vur_info(
    repo: &str,
    name: &str,
    pkg: &VupPackage,
    arch: &str,
) -> Option<(VurInfo, String)> {
    // T3/T5: la clave del mapa "packages" la decide el repo remoto. Sin
    // validar, un nombre tipo "../../etc/x" entra en el plan, en la DB de
    // instalados y en el nombre del artefacto. La vía .VURINFO sí valida
    // (metadata::validate): aquí faltaba la misma puerta.
    if !crate::metadata::is_valid_pkgname(name) {
        // Diga QUÉ repo lo declara: sin eso el aviso no es accionable
        // (puede haber varios índices VUR en vuelo).
        tracing::warn!("el repo VUR '{repo}' declara un nombre de paquete inválido: '{name}'");
        return None;
    }
    if !pkg.archs.iter().any(|a| a == arch) {
        return None;
    }
    let repo_url = pkg.repo_urls.get(arch)?.trim().to_string();
    if repo_url.is_empty() {
        return None;
    }
    let (version, revision) = split_pkgver(pkg.version.trim())?;
    let info = VurInfo {
        format_version: 1,
        pkgname: name.to_string(),
        version,
        revision,
        archs: vec![arch.to_string()],
        subpackages: Vec::new(),
        depends: Vec::new(),
        hostmakedepends: Vec::new(),
        makedepends: Vec::new(),
        build_style: None,
        checksum: Vec::new(),
        provides: Vec::new(),
        replaces: Vec::new(),
        restricted: false,
        maintainer: None,
    };
    Some((info, repo_url))
}

/// Localiza la llave pública del repo (`keys/*.plist`, la primera en orden).
pub fn discover_plist_key(repo_path: &Path) -> Option<PathBuf> {
    let keys_dir = repo_path.join("keys");
    let mut candidates: Vec<PathBuf> = std::fs::read_dir(keys_dir)
        .ok()?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|p| p.is_file() && p.extension().and_then(|ext| ext.to_str()) == Some("plist"))
        .collect();
    candidates.sort();
    candidates.into_iter().next()
}

/// Decodifica el PEM envuelto en un plist VUP.
///
/// El plist guarda el texto PEM (`-----BEGIN PUBLIC KEY-----...`) codificado
/// en base64 dentro de `<data>` bajo la clave `public-key`.
pub fn decode_plist_public_key_pem(plist_text: &str) -> Result<String> {
    let key_tag = "<key>public-key</key>";
    let key_pos = plist_text
        .find(key_tag)
        .context("el plist no contiene la clave public-key")?;
    let after = &plist_text[key_pos + key_tag.len()..];
    let data_start_tag = "<data>";
    let data_end_tag = "</data>";
    let data_start = after
        .find(data_start_tag)
        .context("el plist no contiene bloque <data>")?
        + data_start_tag.len();
    let data_len = after[data_start..]
        .find(data_end_tag)
        .context("bloque <data> sin cerrar en el plist")?;
    // T3: el plist lo publica el repo remoto; sin tope, un <data> enorme
    // se decodifica entero en RAM (OOM).
    if data_len as u64 > MAX_PLIST_BYTES {
        anyhow::bail!(
            "el bloque <data> del plist ocupa {data_len} bytes (tope {MAX_PLIST_BYTES}): se rechaza"
        );
    }
    let b64_raw = &after[data_start..data_start + data_len];
    let b64_clean: String = b64_raw
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '+' || *c == '/' || *c == '=')
        .collect();
    let pem_bytes = general_purpose::STANDARD
        .decode(&b64_clean)
        .context("base64 inválido en el plist")?;
    let pem = String::from_utf8(pem_bytes).context("la llave del plist no es texto PEM")?;
    let trimmed = pem.trim();
    anyhow::ensure!(
        trimmed.contains("BEGIN PUBLIC KEY"),
        "la llave del plist no es una public key PEM"
    );
    Ok(format!("{trimmed}\n"))
}

/// Lee `keys/*.plist` del clon y devuelve el texto CRUDO (formato plist XML
/// de xbps: xbps lo almacena verbatim en /var/db/xbps/keys/, verificado en
/// vivo por diff — por eso el pre-import T-012 escribe este texto tal cual).
pub fn read_repo_plist_text(repo_path: &Path) -> Result<String> {
    let key_path = discover_plist_key(repo_path).context(
        "el repo no incluye llave pública en keys/*.plist; \
         sin ella no se pueden verificar los binarios",
    )?;
    // T3: el plist lo publica el repo remoto. El tope va ANTES de leer:
    // leerlo entero a RAM y luego rechazar es tarde (el pico ya ocurrió).
    if let Ok(meta) = std::fs::metadata(&key_path) {
        anyhow::ensure!(
            meta.len() <= MAX_PLIST_BYTES,
            "el plist {} ocupa {} bytes (tope {MAX_PLIST_BYTES}): se rechaza",
            key_path.display(),
            meta.len()
        );
    }
    std::fs::read_to_string(&key_path).with_context(|| format!("leyendo {}", key_path.display()))
}

/// Lee `keys/*.plist` vía git (`ls-tree` + `show`), sin checkout materializado.
///
/// Los clones de vary son sparse/partial: `keys/` rara vez existe en disco
/// aunque sí en HEAD. Sin esta variante, todo install VUP falla con
/// "el repo no incluye llave pública" aunque el upstream sí la publique (T-006).
/// Devuelve el texto crudo (ver [`read_repo_plist_text`]).
pub fn read_repo_plist_text_git(git_bin: &str, repo_path: &Path) -> Result<String> {
    let output = Command::new(git_bin)
        .arg("-C")
        .arg(repo_path)
        .args(["ls-tree", "-r", "--name-only", "HEAD", "keys"])
        .output()
        .with_context(|| format!("no se pudo ejecutar git ls-tree en {}", repo_path.display()))?;
    anyhow::ensure!(
        output.status.success(),
        "git ls-tree falló en {}",
        repo_path.display()
    );
    let mut names: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|l| l.ends_with(".plist"))
        .map(str::to_string)
        .collect();
    names.sort();
    let first = names.into_iter().next().context(
        "el repo no incluye llave pública en keys/*.plist (HEAD); \
         sin ella no se pueden verificar los binarios",
    )?;
    // T3: `git show` metería el plist entero en RAM; `cat-file -s` dice el
    // tamaño SIN leerlo → el tope protege el pico, no solo el decode.
    let size = Command::new(git_bin)
        .arg("-C")
        .arg(repo_path)
        .args(["cat-file", "-s", &format!("HEAD:{first}")])
        .output()
        .with_context(|| format!("no se pudo ejecutar git cat-file -s HEAD:{first}"))?;
    if size.status.success() {
        let bytes: u64 = String::from_utf8_lossy(&size.stdout)
            .trim()
            .parse()
            .unwrap_or(0);
        anyhow::ensure!(
            bytes <= MAX_PLIST_BYTES,
            "el plist {first} del repo ocupa {bytes} bytes (tope {MAX_PLIST_BYTES}): se rechaza"
        );
    }
    let show = Command::new(git_bin)
        .arg("-C")
        .arg(repo_path)
        .args(["show", &format!("HEAD:{first}")])
        .output()
        .with_context(|| format!("no se pudo ejecutar git show HEAD:{first}"))?;
    anyhow::ensure!(
        show.status.success(),
        "llave no legible en el repo: {first}"
    );
    String::from_utf8(show.stdout).context("el plist del repo no es UTF-8 válido")
}

#[cfg(test)]
mod tests {
    /// T3: el repo remoto decide el tamaño del índice; sin tope, el `read`
    /// lo carga entero en RAM (OOM).
    #[test]
    fn el_indice_remoto_tiene_tope_de_tamano() {
        assert!(check_index_size(MAX_INDEX_BYTES).is_ok());
        assert!(check_index_size(MAX_INDEX_BYTES + 1).is_err());
    }

    /// T3/T5: la clave "packages" del índice la decide el repo remoto; un
    /// nombre con traversal no puede entrar en el plan ni en la DB.
    #[test]
    fn el_nombre_del_indice_se_valida() {
        let pkg = VupPackage {
            category: "cat".to_string(),
            version: "1.0_1".to_string(),
            archs: vec!["x86_64".to_string()],
            repo_urls: BTreeMap::from([("x86_64".to_string(), "https://x/y".to_string())]),
        };
        assert!(to_vur_info("repo", "bueno", &pkg, "x86_64").is_some());
        for malo in ["../../etc/x", "-guion", "con espacio", ""] {
            assert!(
                to_vur_info("repo", malo, &pkg, "x86_64").is_none(),
                "el nombre {malo:?} debería rechazarse"
            );
        }
    }

    /// Forma REAL del `index.json` de VUP (la que genera `generate_index.py`):
    /// raíz con "packages" y una clave hermana "_meta" (sources/base_url). La
    /// puerta tiene que seguir descartando el nombre hostil con esa forma
    /// exacta, y "_meta" no puede colarse como si fuera un paquete.
    #[test]
    fn la_puerta_sobrevive_a_la_forma_real_del_indice_vup() {
        let json = r#"{
            "_meta": {
                "sources": {"vup": "https://example.org/vup"},
                "base_url": "https://example.org/"
            },
            "packages": {
                "bueno": {
                    "category": "cat", "version": "1.0_1", "archs": ["x86_64"],
                    "repo_urls": {"x86_64": "https://example.org/y"}
                },
                "../../etc/x": {
                    "category": "cat", "version": "1.0_1", "archs": ["x86_64"],
                    "repo_urls": {"x86_64": "https://example.org/y"}
                }
            }
        }"#;
        let idx = parse_index_bytes(json.as_bytes()).expect("index.json válido");
        // "_meta" es una clave hermana de "packages": no es un paquete.
        assert!(!idx.packages.contains_key("_meta"));
        let bueno = idx.packages.get("bueno").unwrap();
        assert!(to_vur_info("repo", "bueno", bueno, "x86_64").is_some());
        let hostil = idx.packages.get("../../etc/x").unwrap();
        assert!(to_vur_info("repo", "../../etc/x", hostil, "x86_64").is_none());
    }

    /// La versión de un pkgver no puede llevar "_" (delimita el pkgver).
    #[test]
    fn la_version_no_lleva_guion_bajo() {
        assert_eq!(split_pkgver("0.5.0_1"), Some(("0.5.0".to_string(), 1)));
        assert!(split_pkgver("1.2.3_4_5").is_none());
        assert!(split_pkgver("0.5.0_0").is_none());
    }

    /// T3: el plist lo publica el repo remoto; un <data> enorme se
    /// decodificaría entero en RAM.
    #[test]
    fn el_plist_del_repo_remoto_tiene_tope() {
        let enorme = "A".repeat(MAX_PLIST_BYTES as usize + 16);
        let plist = format!(
            "<plist><dict><key>public-key</key><dict><key>data</key><data>{enorme}</data></dict></dict></plist>"
        );
        let err = decode_plist_public_key_pem(&plist).unwrap_err();
        assert!(err.to_string().contains("tope"), "{err:#}");
    }

    /// Integración del camino de descarga REAL (curl de verdad + index.json
    /// servido por HTTP). Uso:
    ///
    /// ```sh
    ///   mkdir -p /tmp/vup && cat > /tmp/vup/index.json <<'JSON'
    ///   {"packages":{"bueno":{"category":"c","version":"1.0_1","archs":["x86_64"],
    ///   "repo_urls":{"x86_64":"https://example.org/y"}},
    ///   "../../etc/x":{"category":"c","version":"1.0_1","archs":["x86_64"],
    ///   "repo_urls":{"x86_64":"https://example.org/y"}}}}
    ///   JSON
    ///   (cd /tmp/vup && python3 -m http.server 8099 &)
    ///   VUP_TEST_URL=http://127.0.0.1:8099/index.json cargo test --lib -- --ignored vup
    /// ```
    #[test]
    #[ignore = "requiere curl y un index.json servido por HTTP"]
    fn descarga_el_indice_real_y_filtra_los_nombres_invalidos() {
        let url = std::env::var("VUP_TEST_URL").expect("VUP_TEST_URL");
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("index.json");
        let idx = fetch_index("curl", &url, &cache, 0).expect("descarga del índice");
        assert!(idx.packages.contains_key("bueno"));
        // El nombre lo decide el remoto: el inválido NO puede llegar al plan.
        let pkg = idx.packages.get("bueno").unwrap();
        assert!(to_vur_info("repo", "bueno", pkg, "x86_64").is_some());
        assert!(to_vur_info("repo", "../../etc/x", pkg, "x86_64").is_none());
        // La caché se escribió (y con el temporal ya renombrado).
        assert!(cache.is_file());
    }

    /// Una caché CORRUPTA (truncada, editada a mano) no es un estado: es un
    /// acierto de la cache → se re-descarga. Verificado con curl real: la
    /// caché truncada a 20 bytes se ignora y el índice se vuelve a bajar.
    #[test]
    #[ignore = "requiere curl y un index.json servido por HTTP"]
    fn la_cache_corrupta_se_re_descarga() {
        let url = std::env::var("VUP_TEST_URL").expect("VUP_TEST_URL");
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("index.json");

        // 1) descarga normal (deja la caché)
        let idx = fetch_index("curl", &url, &cache, 3600).expect("primera descarga");
        assert!(idx.packages.contains_key("bueno"));
        assert!(cache.is_file());

        // 2) la caché se corrompe (truncada a 20 bytes)
        std::fs::write(&cache, b"{\"packages\":{\"bueno\"").unwrap();
        assert!(std::fs::read(&cache).unwrap().len() < 30);

        // 3) con la caché "fresca" pero inválida: se re-descarga y se arregla
        let idx = fetch_index("curl", &url, &cache, 3600).expect("re-descarga");
        assert!(idx.packages.contains_key("bueno"));
        let bytes = std::fs::read(&cache).unwrap();
        assert!(
            parse_index_bytes(&bytes).is_ok(),
            "la caché debería haberse reescrito con el índice válido"
        );
    }

    /// La URL del índice viene de repos.conf (dato): curl parsea opciones en
    /// cualquier posición, así que solo se admite http(s).
    #[test]
    fn index_url_no_http_s_es_rechazada() {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("index.json");
        for url in [
            "-K /tmp/evil",
            "-o /tmp/evil",
            "file:///tmp/evil",
            "ftp://x/y",
        ] {
            let err = fetch_index("curl", url, &cache, 0).unwrap_err();
            assert!(err.to_string().contains("http(s)"), "url {url}: {err:#}");
        }
    }

    use super::*;

    const SAMPLE_INDEX: &str = r#"{
        "packages": {
            "vlang": {
                "category": "languages",
                "version": "0.5.0_1",
                "archs": ["x86_64"],
                "repo_urls": {"x86_64": "https://github.com/VUP-Linux/vup/releases/download/languages-x86_64-current"}
            },
            "odin": {
                "category": "languages",
                "version": "2025.06_1",
                "archs": ["aarch64"],
                "repo_urls": {"aarch64": "https://github.com/VUP-Linux/vup/releases/download/languages-aarch64-current"}
            }
        }
    }"#;

    #[test]
    fn parse_index_and_arch_filter() {
        let idx: VupIndex = serde_json::from_str(SAMPLE_INDEX).unwrap();
        assert_eq!(idx.packages.len(), 2);

        let vlang = idx.packages.get("vlang").unwrap();
        let (info, url) = to_vur_info("repo", "vlang", vlang, "x86_64").expect("vlang en x86_64");
        assert_eq!(info.pkgname, "vlang");
        assert_eq!(info.version, "0.5.0");
        assert_eq!(info.revision, 1);
        assert_eq!(info.archs, vec!["x86_64".to_string()]);
        assert_eq!(
            url,
            "https://github.com/VUP-Linux/vup/releases/download/languages-x86_64-current"
        );
        assert_eq!(info.pkgver(), "vlang-0.5.0_1");

        // odin no soporta x86_64 en este índice
        let odin = idx.packages.get("odin").unwrap();
        assert!(to_vur_info("repo", "odin", odin, "x86_64").is_none());
        assert!(to_vur_info("repo", "odin", odin, "aarch64").is_some());
    }

    #[test]
    fn split_pkgver_edges() {
        assert_eq!(split_pkgver("0.5.0_1"), Some(("0.5.0".to_string(), 1)));
        assert_eq!(split_pkgver("1.2"), None);
        assert_eq!(split_pkgver(""), None);
        assert_eq!(split_pkgver("_1"), None);
        assert_eq!(split_pkgver("1.0_x"), None);
        assert_eq!(split_pkgver("2.0_0"), None);
        // La última parte manda (inverso exacto de version_revision).
        assert_eq!(split_pkgver("2025.06_3"), Some(("2025.06".to_string(), 3)));
    }

    #[test]
    fn plist_decode_roundtrip() {
        let pem = "-----BEGIN PUBLIC KEY-----\naGVsbG8gd29ybGQ=\n-----END PUBLIC KEY-----\n";
        let b64 = general_purpose::STANDARD.encode(pem);
        let plist = format!(
            "<?xml version=\"1.0\"?><plist><dict><key>public-key</key>\
             <data>\n{b64}\n</data></dict></plist>"
        );
        let decoded = decode_plist_public_key_pem(&plist).unwrap();
        assert_eq!(decoded, pem);
    }

    #[test]
    fn plist_decode_rejects_garbage() {
        assert!(decode_plist_public_key_pem("no es un plist").is_err());
        assert!(decode_plist_public_key_pem("<plist></plist>").is_err());
        assert!(
            decode_plist_public_key_pem("<key>public-key</key><data>!!!no-base64!!!</data>")
                .is_err()
        );
    }

    #[test]
    fn sanitize_repo_name_replaces_separators() {
        assert_eq!(sanitize_repo_name("mi-vup"), "mi-vup");
        assert_eq!(sanitize_repo_name("a/b\\c"), "a_b_c");
    }

    #[test]
    fn plist_key_via_git_sin_checkout_materializado() {
        // T-006: keys/ existe en HEAD pero NO en disco (clon sparse). La vía
        // de sistema de archivos debe fallar y la vía git debe resolver.
        use std::process::Command;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let git = |args: &[&str]| {
            let out = Command::new("git")
                .arg("-C")
                .arg(root)
                .args(args)
                .output()
                .expect("git en PATH");
            assert!(
                out.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            out
        };
        git(&["init", "-q"]);
        git(&["config", "user.email", "t@t"]);
        git(&["config", "user.name", "t"]);
        let pem = "-----BEGIN PUBLIC KEY-----\naGVsbG8gd29ybGQ=\n-----END PUBLIC KEY-----\n";
        let b64 = general_purpose::STANDARD.encode(pem);
        let plist = format!(
            "<?xml version=\"1.0\"?><plist><dict><key>public-key</key><data>{b64}</data></dict></plist>"
        );
        std::fs::create_dir(root.join("keys")).unwrap();
        std::fs::write(root.join("keys").join("aa.plist"), &plist).unwrap();
        git(&["add", "."]);
        git(&["commit", "-qm", "llave"]);
        // Simular sparse: borrar keys/ del worktree (los objetos quedan en HEAD).
        std::fs::remove_dir_all(root.join("keys")).unwrap();
        assert!(discover_plist_key(root).is_none());
        assert!(read_repo_plist_text(root).is_err());
        let raw = read_repo_plist_text_git("git", root).expect("plist crudo vía git");
        assert_eq!(raw, plist);
        let got = decode_plist_public_key_pem(&raw).expect("llave vía git");
        assert_eq!(got, pem);
    }
}
