//! Verificación challenge: recompilar local y comparar árboles (P1-2).
//!
//! Tras `--experimental`: para un paquete instalado compilado desde fuente,
//! reconstruye con `xbps-src` y compara el árbol del artefacto contra los
//! ficheros instalados. Reporta divergencias (añadidos/quitados/cambiados)
//! e ignora mtimes/modos (ruido de rebuild: dos compilaciones idénticas
//! difieren en timestamps; una divergencia de *contenido* por timestamps
//! embebidos sí se detecta como cambio).
//!
//! Las divergencias NO bloquean (exit 0 con resumen): informar es el
//! producto, la decisión es del humano. El `.xbps` se lee por streaming
//! (`ruzstd`+`tar`, puro Rust, sin extraer a disco ni depender de
//! `bsdtar`/`tar` del sistema —`tar` no lee zstd aquí—).

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};

/// Estado comparable de un fichero: contenido (sha256 hex) o destino de
/// symlink. Directorios/dispositivos se omiten (siempre existen).
#[derive(Debug, Clone, PartialEq)]
pub enum FileState {
    Content(String),
    Symlink(String),
}

/// Entrada de árbol: ruta absoluta + estado.
#[derive(Debug, Clone, PartialEq)]
pub struct TreeEntry {
    pub path: String,
    pub state: FileState,
}

/// Divergencia entre lo reconstruido y lo instalado.
#[derive(Debug, Clone, PartialEq)]
pub struct Divergence {
    pub path: String,
    pub kind: DivergenceKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DivergenceKind {
    /// En disco pero no en el artefacto.
    Added,
    /// En el artefacto pero no en disco.
    Removed,
    /// En ambos pero distinto (contenido o tipo).
    Changed,
}

/// P1-2: compara árboles (puro). Salida ordenada por ruta (determinista).
pub fn compare_trees(built: &[TreeEntry], installed: &[TreeEntry]) -> Vec<Divergence> {
    let b: HashMap<&str, &FileState> = built.iter().map(|e| (e.path.as_str(), &e.state)).collect();
    let iv: HashMap<&str, &FileState> = installed
        .iter()
        .map(|e| (e.path.as_str(), &e.state))
        .collect();
    let mut out = Vec::new();
    for (path, state) in &b {
        match iv.get(path) {
            None => out.push(Divergence {
                path: path.to_string(),
                kind: DivergenceKind::Removed,
            }),
            Some(other) if other != state => out.push(Divergence {
                path: path.to_string(),
                kind: DivergenceKind::Changed,
            }),
            _ => {}
        }
    }
    for path in iv.keys() {
        if !b.contains_key(path) {
            out.push(Divergence {
                path: path.to_string(),
                kind: DivergenceKind::Added,
            });
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}

/// Miembros de metadata del `.xbps` (nunca se instalan como ficheros).
const XBPS_META_MEMBERS: &[&str] = &["/props.plist", "/files.plist", "/INSTALL", "/REMOVE"];

/// P1-2: lee el árbol de un `.xbps` por streaming (zstd+tar, sin disco).
fn read_built_tree(xbps_path: &Path) -> Result<Vec<TreeEntry>> {
    let file = std::fs::File::open(xbps_path)
        .with_context(|| format!("abriendo {}", xbps_path.display()))?;
    let decoder = ruzstd::decoding::StreamingDecoder::new(file)
        .map_err(|e| anyhow::anyhow!("decodificando zstd de {}: {e:?}", xbps_path.display()))?;
    let mut archive = tar::Archive::new(decoder);
    let mut out = Vec::new();
    for entry in archive
        .entries()
        .with_context(|| format!("leyendo tar de {}", xbps_path.display()))?
    {
        let mut entry =
            entry.with_context(|| format!("entrada corrupta en {}", xbps_path.display()))?;
        let raw = entry.path()?.to_string_lossy().to_string();
        // Normalizar a absoluta: el tar trae "./usr/..." o "usr/...".
        let path = format!("/{}", raw.trim_start_matches("./").trim_start_matches('/'));
        if XBPS_META_MEMBERS.contains(&path.as_str()) {
            continue;
        }
        let header = entry.header().clone();
        if header.entry_type().is_symlink() {
            let target = entry
                .link_name()?
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            out.push(TreeEntry {
                path,
                state: FileState::Symlink(target),
            });
        } else if header.entry_type().is_file() {
            let mut bytes = Vec::new();
            use std::io::Read;
            entry.read_to_end(&mut bytes)?;
            out.push(TreeEntry {
                path,
                state: FileState::Content(sha256_hex(&bytes)),
            });
        }
        // Directorios y especiales se omiten (siempre existen).
    }
    Ok(out)
}

/// P1-2: lee el árbol instalado (`xbps-query -f` + hash por fichero).
/// Ficheros listados pero ilegibles/desaparecidos se omiten (carreras y
/// casos xbps —alternativas, etc.—: no afirmables como divergencia).
fn read_installed_tree(pkg: &str) -> Result<Vec<TreeEntry>> {
    let out = std::process::Command::new("xbps-query")
        .args(["-f", "--", pkg])
        .output()
        .context("ejecutando xbps-query -f")?;
    if !out.status.success() {
        anyhow::bail!("xbps-query -f {pkg} falló (¿paquete instalado?)");
    }
    let mut entries = Vec::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let p = line.trim();
        if p.is_empty() {
            continue;
        }
        let md = match std::fs::symlink_metadata(p) {
            Ok(m) => m,
            Err(_) => continue,
        };
        if md.file_type().is_symlink() {
            let target = std::fs::read_link(p)
                .map(|t| t.to_string_lossy().to_string())
                .unwrap_or_default();
            entries.push(TreeEntry {
                path: p.to_string(),
                state: FileState::Symlink(target),
            });
        } else if md.is_file() {
            if let Ok(bytes) = std::fs::read(p) {
                entries.push(TreeEntry {
                    path: p.to_string(),
                    state: FileState::Content(sha256_hex(&bytes)),
                });
            }
        }
    }
    Ok(entries)
}

/// P1-2: `vary --challenge <pkg>` — reconstruye y compara contra lo instalado.
pub fn challenge(config: &crate::config::Config, pkg: &str) -> Result<i32> {
    if !config.experimental {
        anyhow::bail!(
            "challenge requiere --experimental (P1-2 en validación: la comparación es \
             informativa y el rebuild muta binpkgs)"
        );
    }
    // Solo paquetes compilados desde fuente y gestionados por vary.
    let db = crate::db::InstalledDb::load(config.installed_db_path())?;
    let entry = db.get(pkg).ok_or_else(|| {
        anyhow::anyhow!("'{pkg}' no está gestionado por vary (sin entrada en installed.json)")
    })?;
    if entry.install_type != crate::db::InstallType::Source {
        anyhow::bail!(
            "challenge solo aplica a paquetes compilados desde fuente (esto es {:?})",
            entry.install_type
        );
    }
    let repos_conf = crate::reposconf::ReposConf::load(config.repos_conf_path())?;
    let vur_entry = repos_conf
        .vur
        .get(&entry.vur)
        .ok_or_else(|| anyhow::anyhow!("repo '{}' sin entrada en repos.conf", entry.vur))?;
    let repo = crate::vur_client::VurRepo {
        name: entry.vur.clone(),
        path: config.vurs_dir().join(&entry.vur),
        entry: vur_entry.clone(),
        git_bin: config.git_bin.clone(),
    };

    // Reconstruir (mismo flujo que install: materializar→proyectar→build).
    let md = crate::bootstrap::initialize_environment(
        &config.void_packages_dir(),
        &config.sudo_bin,
        &config.sudo_flags,
        &config.tools_install_bin,
        &config.git_bin,
    )?;
    repo.materialize_pkg(pkg)?;
    repo.project_pkg(&md.srcpkgs_dir(), pkg, true)?;
    let res = md.build_pkg(pkg, config.makejobs);
    let _ = repo.unproject_pkg(&md.srcpkgs_dir(), pkg);
    res.with_context(|| format!("reconstruyendo {pkg}"))?;

    // Localizar artefacto recién construido.
    let arch = config.arch();
    let arch = if config.arch_override.is_none() {
        crate::xbps::query_architecture().unwrap_or(arch)
    } else {
        arch
    };
    let xbps_path = md
        .hostdir_binpkgs_root()
        .join(format!("{}.{arch}.xbps", entry.version));
    if !xbps_path.is_file() {
        anyhow::bail!(
            "artefacto reconstruido no encontrado: {} (¿subpaquete? challenge cubre el padre)",
            xbps_path.display()
        );
    }

    let built = read_built_tree(&xbps_path)?;
    let installed = read_installed_tree(pkg)?;
    let divergences = compare_trees(&built, &installed);
    if divergences.is_empty() {
        println!(
            "challenge {pkg}: sin divergencias ({} ficheros comparados)",
            built.len()
        );
    } else {
        println!(
            "challenge {pkg}: {} divergencias (rebuild {} vs instalado):",
            divergences.len(),
            xbps_path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default()
        );
        for d in divergences.iter().take(50) {
            let kind = match d.kind {
                DivergenceKind::Added => "añadido-en-disco",
                DivergenceKind::Removed => "falta-en-disco",
                DivergenceKind::Changed => "cambiado",
            };
            println!("  [{kind}] {}", d.path);
        }
        if divergences.len() > 50 {
            println!("  ... y {} más", divergences.len() - 50);
        }
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ent(path: &str, state: FileState) -> TreeEntry {
        TreeEntry {
            path: path.to_string(),
            state,
        }
    }

    fn content(s: &str) -> FileState {
        FileState::Content(format!("sha256:{s}"))
    }

    /// P1-2: tabla de aceptación — árboles iguales, añadidos, quitados,
    /// cambiados; symlinks por destino; orden determinista.
    #[test]
    fn compare_tabla_aceptacion() {
        // Iguales (mismo contenido): silencio aunque vengan desordenados.
        let built = vec![ent("/a", content("1")), ent("/b", content("2"))];
        let inst = vec![ent("/b", content("2")), ent("/a", content("1"))];
        assert!(compare_trees(&built, &inst).is_empty());

        // Añadido en disco / quitado en disco / cambiado.
        let built = vec![ent("/same", content("1")), ent("/old", content("1"))];
        let inst = vec![
            ent("/same", content("1")),
            ent("/new", content("9")),
            ent("/old", content("2")),
        ];
        let d = compare_trees(&built, &inst);
        assert_eq!(d.len(), 2, "{d:?}");
        assert!(d
            .iter()
            .any(|x| x.path == "/new" && x.kind == DivergenceKind::Added));
        assert!(d
            .iter()
            .any(|x| x.path == "/old" && x.kind == DivergenceKind::Changed));
        // Orden determinista por ruta.
        let paths: Vec<&str> = d.iter().map(|x| x.path.as_str()).collect();
        let mut sorted = paths.clone();
        sorted.sort();
        assert_eq!(paths, sorted);
    }

    #[test]
    fn compare_symlinks_y_tipos() {
        // Mismo destino: silencio; distinto: cambio.
        let built = vec![ent("/l", FileState::Symlink("a".to_string()))];
        assert!(
            compare_trees(&built, &[ent("/l", FileState::Symlink("a".to_string()))]).is_empty()
        );
        let d = compare_trees(&built, &[ent("/l", FileState::Symlink("b".to_string()))]);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].kind, DivergenceKind::Changed);
        // Fichero vs symlink en la misma ruta: cambio de tipo.
        let d = compare_trees(&built, &[ent("/l", content("1"))]);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].kind, DivergenceKind::Changed);
    }

    #[test]
    fn compare_vacio_total_es_silencio() {
        assert!(compare_trees(&[], &[]).is_empty());
    }
}
