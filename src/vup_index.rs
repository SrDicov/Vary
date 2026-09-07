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

    let tmp_path = cache_path.with_extension("tmp");
    let status = Command::new(curl_bin)
        .args(["-fsSL", "--max-time", "60", "-o"])
        .arg(&tmp_path)
        .arg(url)
        .status()
        .with_context(|| {
            format!("no se pudo ejecutar '{curl_bin}': ¿está instalado? (flag --curl / [general] curl_bin)")
        })?;

    if status.success() {
        match std::fs::read(&tmp_path)
            .ok()
            .and_then(|bytes| parse_index_bytes(&bytes).ok().map(|idx| (bytes, idx)))
        {
            Some((bytes, idx)) => {
                if let Some(parent) = cache_path.parent() {
                    if !parent.as_os_str().is_empty() {
                        let _ = crate::util::ensure_private_dir(parent);
                    }
                }
                // La caché es solo una optimización: si no se puede escribir, seguir igual.
                let _ = std::fs::write(cache_path, &bytes);
                let _ = std::fs::remove_file(&tmp_path);
                return Ok(idx);
            }
            None => {
                let _ = std::fs::remove_file(&tmp_path);
                tracing::warn!("el índice remoto {url} no parsea como index.json");
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
    anyhow::bail!("índice {url} no disponible (sin red ni caché válida)")
}

/// Divide `versión_revisión` (formato XBPS) en sus partes.
/// Devuelve `None` si no hay guion bajo, la revisión no es un entero >= 1
/// o la versión queda vacía.
pub fn split_pkgver(pkgver: &str) -> Option<(String, u32)> {
    let (version, revision) = pkgver.rsplit_once('_')?;
    if version.is_empty() {
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
/// Devuelve también la URL del repo binario que sirve ese paquete en esa
/// arquitectura. `None` si el paquete no soporta `arch`, no declara URL
/// para ella o su versión no parsea.
pub fn to_vur_info(name: &str, pkg: &VupPackage, arch: &str) -> Option<(VurInfo, String)> {
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
        checkdepends: Vec::new(),
        build_style: None,
        distfiles: Vec::new(),
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
        let (info, url) = to_vur_info("vlang", vlang, "x86_64").expect("vlang en x86_64");
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
        assert!(to_vur_info("odin", odin, "x86_64").is_none());
        assert!(to_vur_info("odin", odin, "aarch64").is_some());
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
