use crate::cache::CacheIndex;
use crate::metadata::{self, VurInfo};
use crate::reposconf::RepoEntry;
use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose, Engine as _};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct VurRepo {
    pub name: String,
    pub path: PathBuf,
    pub entry: RepoEntry,
}

impl VurRepo {
    pub fn ensure_cloned(&self) -> Result<()> {
        if self.path.join(".git").exists() {
            return Ok(());
        }
        if let Some(parent) = self.path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("no se pudo crear {}", parent.display()))?;
            }
        }
        let branch = self.entry.branch_or_default();
        let output = Command::new("git")
            .args(["clone", "--depth", "1", "--branch", branch])
            .arg(&self.entry.url)
            .arg(&self.path)
            .output()
            .context("no se pudo ejecutar git clone")?;
        if !output.status.success() {
            bail!(
                "git clone de {} hacia {} falló:\n{}",
                self.entry.url,
                self.path.display(),
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(())
    }

    pub fn head_sha(&self) -> Result<String> {
        let output = Command::new("git")
            .arg("-C")
            .arg(&self.path)
            .args(["rev-parse", "HEAD"])
            .output()
            .context("no se pudo ejecutar git rev-parse")?;
        if !output.status.success() {
            bail!(
                "git rev-parse HEAD falló en {}:\n{}",
                self.path.display(),
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    pub fn pull(&self) -> Result<String> {
        let output = Command::new("git")
            .arg("-C")
            .arg(&self.path)
            .args(["pull", "--ff-only"])
            .output()
            .context("no se pudo ejecutar git pull")?;
        if !output.status.success() {
            bail!(
                "git pull --ff-only falló en {}:\n{}",
                self.path.display(),
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        self.head_sha()
    }

    pub fn load_index(
        &self,
        cache: &mut CacheIndex,
        ttl_search: Option<u64>,
    ) -> Result<Vec<VurInfo>> {
        let sha = self.head_sha()?;
        let key = format!("{}:{}", self.name, sha);
        if let Some(cached) = cache.load_valid(&key, ttl_search) {
            return Ok(cached.to_vec());
        }

        let mut packages: Vec<VurInfo> = Vec::new();
        let mut files_seen = 0usize;

        let srcpkgs = self.path.join("srcpkgs");
        if srcpkgs.is_dir() {
            let mut dirs: Vec<PathBuf> = std::fs::read_dir(&srcpkgs)?
                .filter_map(|entry| entry.ok())
                .map(|entry| entry.path())
                .filter(|p| p.is_dir())
                .collect();
            dirs.sort();
            for dir in dirs {
                let info_path = dir.join(".VURINFO");
                if !info_path.is_file() {
                    continue;
                }
                files_seen += 1;
                match std::fs::read_to_string(&info_path)
                    .map_err(anyhow::Error::from)
                    .and_then(|text| metadata::parse(&text))
                {
                    Ok(info) => packages.push(info),
                    Err(err) => tracing::warn!(
                        ".VURINFO inválido ignorado en {}: {err:#}",
                        info_path.display()
                    ),
                }
            }
        }

        let root_info = self.path.join(".VURINFO");
        if root_info.is_file() {
            files_seen += 1;
            match std::fs::read_to_string(&root_info)
                .map_err(anyhow::Error::from)
                .and_then(|text| metadata::parse_many(&text))
            {
                Ok(mut infos) => packages.append(&mut infos),
                Err(err) => tracing::warn!(
                    ".VURINFO raíz inválido ignorado en {}: {err:#}",
                    root_info.display()
                ),
            }
        }

        // Fallback: si no hay .VURINFO, intentar parsear plantillas Void directamente
        // Soporta tanto layout void-packages (srcpkgs/<pkg>/template) como
        // layout plano estilo cnr (/<pkg>/template) escaneando recursivo.
        if packages.is_empty() && files_seen == 0 {
            let mut template_files: Vec<PathBuf> = Vec::new();
            // srcpkgs/*/template
            if srcpkgs.is_dir() {
                if let Ok(entries) = std::fs::read_dir(&srcpkgs) {
                    for entry in entries.flatten() {
                        let p = entry.path().join("template");
                        if p.is_file() {
                            template_files.push(p);
                        }
                    }
                }
            }
            // */template (cnr flat) y */*/template por si acaso
            if let Ok(entries) = std::fs::read_dir(&self.path) {
                for entry in entries.flatten() {
                    let p = entry.path();
                    if p.is_dir() {
                        let tmpl = p.join("template");
                        if tmpl.is_file() {
                            if !template_files.contains(&tmpl) {
                                template_files.push(tmpl);
                            }
                        }
                        // Buscar un nivel más profundo (por compatibilidad)
                        if let Ok(subs) = std::fs::read_dir(&p) {
                            for sub in subs.flatten() {
                                let tmpl2 = sub.path().join("template");
                                if tmpl2.is_file() && !template_files.contains(&tmpl2) {
                                    template_files.push(tmpl2);
                                }
                            }
                        }
                    }
                }
            }
            template_files.sort();
            for tmpl_path in template_files {
                files_seen += 1;
                match parse_template_file(&tmpl_path) {
                    Ok(info) => {
                        tracing::debug!("template parseado {} -> {}", tmpl_path.display(), info.pkgname);
                        packages.push(info);
                    }
                    Err(err) => tracing::warn!(
                        "template inválido ignorado en {}: {err:#}",
                        tmpl_path.display()
                    ),
                }
            }
        }

        if packages.is_empty() && files_seen == 0 {
            bail!("no se encontró ningún .VURINFO ni template en {}", self.path.display());
        }

        cache.store(&key, packages.clone());
        Ok(packages)
    }

    /// Proyecta un único paquete (por pkgname) al masterdir.
    /// Copia el directorio (no symlink) para evitar el check de xbps-src que falla con symlinks.
    ///
    /// `force=true` (targets explícitos del usuario) permite REEMPLAZAR un
    /// directorio oficial ya presente en el árbol maestro (se restaura con
    /// `git checkout` en `unproject_pkg`). Con `force=false` (dependencias)
    /// la precedencia oficial gana y se omite la proyección VUR.
    pub fn project_pkg(&self, master_srcpkgs: &Path, pkgname: &str, force: bool) -> Result<()> {
        std::fs::create_dir_all(master_srcpkgs)
            .with_context(|| format!("no se pudo crear {}", master_srcpkgs.display()))?;
        // Buscar el directorio fuente del paquete
        let candidates = [
            self.path.join("srcpkgs").join(pkgname),
            self.path.join(pkgname),
        ];
        let src = candidates.iter().find(|p| p.join("template").is_file()).cloned()
            .or_else(|| {
                let mut found = None;
                for base in [self.path.join("srcpkgs"), self.path.clone()] {
                    if let Ok(entries) = std::fs::read_dir(&base) {
                        for entry in entries.flatten() {
                            let p = entry.path();
                            if p.is_dir() {
                                if let Ok(info) = parse_template_file(&p.join("template")) {
                                    if info.pkgname == pkgname || info.subpackages.iter().any(|s| s.pkgname == pkgname) {
                                        found = Some(p);
                                        break;
                                    }
                                } else if p.file_name().and_then(|n| n.to_str()) == Some(pkgname) {
                                    found = Some(p);
                                    break;
                                }
                            }
                        }
                    }
                    if found.is_some() { break; }
                }
                found
            })
            .ok_or_else(|| anyhow::anyhow!("template no encontrado para {}", pkgname))?;

        // Para COMPILAR se necesita la plantilla real; un dir solo-índice
        // (.VURINFO sin template) no es construible.
        if !src.join("template").is_file() {
            anyhow::bail!(
                "el paquete '{}' existe en el VUR '{}' como índice .VURINFO pero no incluye \
                 una plantilla 'template'; no se puede compilar con xbps-src",
                pkgname,
                self.name
            );
        }

        let dest = master_srcpkgs.join(pkgname);
        // Si ya existe como directorio real (paquete oficial), decidir
        if let Ok(meta) = dest.symlink_metadata() {
            if meta.is_dir() && !meta.file_type().is_symlink() {
                if dest.join(".vur_projection_marker").exists() {
                    // Proyección previa nuestra: reemplazar
                    std::fs::remove_dir_all(&dest).with_context(|| format!("no se pudo limpiar proyección previa {}", dest.display()))?;
                } else if force {
                    // Target explícito del usuario: el VUR manda sobre el
                    // template oficial no-publicado presente en el árbol.
                    tracing::warn!("reemplazando template oficial local de '{}' por la versión VUR (target explícito)", pkgname);
                    std::fs::remove_dir_all(&dest).with_context(|| format!("no se pudo reemplazar {}", dest.display()))?;
                } else {
                    tracing::warn!("{} ya existe como directorio oficial, se omite proyección de VUR {}", dest.display(), pkgname);
                    return Ok(());
                }
            } else if meta.file_type().is_symlink() {
                std::fs::remove_file(&dest).with_context(|| format!("no se pudo reemplazar symlink {}", dest.display()))?;
            } else {
                std::fs::remove_file(&dest).with_context(|| format!("no se pudo reemplazar {}", dest.display()))?;
            }
        }
        // Copiar recursivamente (equivalente a cp -a)
        Self::copy_dir_recursive(&src, &dest)?;
        // Marcar como proyección nuestra para limpieza
        std::fs::write(dest.join(".vur_projection_marker"), "").ok();
        Ok(())
    }

    pub fn unproject_pkg(&self, master_srcpkgs: &Path, pkgname: &str) -> Result<()> {
        let dest = master_srcpkgs.join(pkgname);
        // Solo eliminar si es nuestra proyección (marcada) o symlink viejo
        if dest.join(".vur_projection_marker").exists() {
            std::fs::remove_dir_all(&dest).with_context(|| format!("no se pudo eliminar proyección {}", dest.display()))?;
            // Restaurar template oficial si este paquete pertenece al árbol maestro
            if master_srcpkgs.join(pkgname).symlink_metadata().is_err() {
                if master_srcpkgs.parent().and_then(|p| p.file_name()).map(|n| n == "void-packages").unwrap_or(false)
                    || master_srcpkgs.join("../.git").exists()
                {
                    let _ = Command::new("git")
                        .args(["checkout", "--", &format!("srcpkgs/{}", pkgname)])
                        .current_dir(master_srcpkgs.parent().unwrap_or(Path::new(".")))
                        .output();
                }
            }
        } else if dest.symlink_metadata().map(|m| m.file_type().is_symlink()).unwrap_or(false) {
            if let Ok(target) = std::fs::read_link(&dest) {
                let abs = if target.is_absolute() { target } else { dest.parent().unwrap().join(target) };
                let resolved = abs.canonicalize().unwrap_or(abs);
                if resolved.starts_with(self.path.canonicalize().unwrap_or_else(|_| self.path.clone())) {
                    std::fs::remove_file(&dest).with_context(|| format!("no se pudo eliminar symlink {}", dest.display()))?;
                }
            }
        }
        Ok(())
    }

    fn copy_dir_recursive(src: &Path, dest: &Path) -> Result<()> {
        std::fs::create_dir_all(dest).with_context(|| format!("no se pudo crear {}", dest.display()))?;
        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            let src_path = entry.path();
            let dest_path = dest.join(entry.file_name());
            let ft = entry.file_type()?;
            if ft.is_dir() {
                Self::copy_dir_recursive(&src_path, &dest_path)?;
            } else if ft.is_symlink() {
                if let Ok(target) = std::fs::read_link(&src_path) {
                    std::os::unix::fs::symlink(target, &dest_path).with_context(|| format!("symlink {}", dest_path.display()))?;
                }
            } else {
                std::fs::copy(&src_path, &dest_path).with_context(|| format!("copiando {} -> {}", src_path.display(), dest_path.display()))?;
            }
        }
        Ok(())
    }

    pub fn discover_public_key(&self) -> Option<PathBuf> {
        let keys_dir = self.path.join("keys");
        let mut candidates: Vec<PathBuf> = std::fs::read_dir(keys_dir)
            .ok()?
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|p| {
                p.is_file()
                    && matches!(
                        p.extension().and_then(|ext| ext.to_str()),
                        Some("rsa") | Some("pem")
                    )
            })
            .collect();
        candidates.sort();
        candidates.into_iter().next()
    }

    pub fn fingerprint_sha256(pem_path: &Path) -> Result<String> {
        let pem = std::fs::read_to_string(pem_path)
            .with_context(|| format!("no se pudo leer {}", pem_path.display()))?;
        let der = decode_pem_body(&pem)?;
        let digest = Sha256::digest(&der);
        Ok(digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<Vec<_>>()
            .join(":"))
    }
}

fn parse_template_file(path: &Path) -> Result<VurInfo> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("no se pudo leer {}", path.display()))?;

    // Unir continuaciones con \ y manejar valores multilínea entre comillas
    let mut vars: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut lines = content.lines().peekable();
    let mut buf = String::new();

    while let Some(line) = lines.next() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        // Ignorar definiciones de funciones y bloques shell
        if trimmed.starts_with("do_") || trimmed.starts_with("pre_") || trimmed.starts_with("post_") || trimmed.starts_with("}") || trimmed.starts_with("{") {
            // Si es inicio de función, saltar hasta }
            if trimmed.contains("()") {
                while let Some(l) = lines.next() {
                    if l.trim() == "}" { break; }
                }
            }
            continue;
        }
        // Acumular líneas con continuación \ o comillas abiertas
        buf.clear();
        buf.push_str(line);
        // Manejar continuación con backslash
        while buf.trim_end().ends_with('\\') {
            buf.truncate(buf.trim_end().len() - 1);
            if let Some(next) = lines.next() {
                buf.push(' ');
                buf.push_str(next);
            } else {
                break;
            }
        }
        // Manejar valores multilínea entre comillas dobles
        let quote_count = buf.matches('"').count();
        while quote_count % 2 == 1 {
            if let Some(next) = lines.next() {
                buf.push('\n');
                buf.push_str(next);
                if next.contains('"') { break; }
            } else {
                break;
            }
        }

        // Parsear asignaciones var=valor
        if let Some(eq) = buf.find('=') {
            let key = buf[..eq].trim().to_string();
            if !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') { continue; }
            // Filtrar solo variables relevantes
            let relevant = matches!(key.as_str(),
                "pkgname" | "version" | "revision" | "archs" | "only_for_archs" |
                "depends" | "hostmakedepends" | "makedepends" | "checkdepends" |
                "build_style" | "distfiles" | "checksum" | "provides" | "replaces" |
                "restricted" | "maintainer" | "short_desc" | "license" | "homepage"
            );
            if !relevant { continue; }
            let mut val = buf[eq+1..].trim().to_string();
            // Quitar comentarios al final (espacio + #), pero no dentro de comillas
            if let Some(hash) = val.find(" #") {
                // Verificar que no esté dentro de comillas
                let before = &val[..hash];
                if before.matches('"').count() % 2 == 0 && before.matches('\'').count() % 2 == 0 {
                    val.truncate(hash);
                    val = val.trim().to_string();
                }
            }
            // Descomillar
            if (val.starts_with('"') && val.ends_with('"') && val.len() >= 2)
                || (val.starts_with('\'') && val.ends_with('\'') && val.len() >= 2) {
                val = val[1..val.len()-1].to_string();
            } else if val.starts_with('"') || val.starts_with('\'') {
                // Valor multilínea entrecomillado sin cierre en misma línea
                // Quitar comilla inicial y buscar cierre
                let quote = val.chars().next().unwrap();
                val.remove(0);
                if let Some(end) = val.rfind(quote) {
                    val.truncate(end);
                }
            }
            // Colapsar whitespace y newlines a espacios para listas
            val = val.split_whitespace().collect::<Vec<_>>().join(" ");
            vars.insert(key, val);
        }
    }

    let pkgname = vars.get("pkgname").cloned().filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("template sin pkgname: {}", path.display()))?;
    let version = vars.get("version").cloned().unwrap_or_else(|| "1.0".to_string());
    let revision: u32 = vars.get("revision").and_then(|s| s.parse().ok()).unwrap_or(1);
    let archs_raw = vars.get("only_for_archs").or_else(|| vars.get("archs")).cloned().unwrap_or_default();
    // Normalizar wildcards de Void ("x86_64*", "aarch64*") a la base
    let archs = if archs_raw.is_empty() {
        vec!["x86_64".to_string()]
    } else {
        archs_raw
            .split_whitespace()
            .map(|s| s.trim_end_matches('*').to_string())
            .filter(|s| !s.is_empty())
            .collect()
    };
    let split_list = |key: &str| -> Vec<String> {
        vars.get(key).map(|s| s.split_whitespace().map(|v| v.to_string()).collect()).unwrap_or_default()
    };
    let checksum_raw = split_list("checksum");
    // Normalizar checksum a sha256: prefijo si falta y no es SKIP
    let checksum = checksum_raw.into_iter().map(|c| {
        if c == "SKIP" || c.starts_with("sha256:") { c } else { format!("sha256:{}", c) }
    }).collect();

    let info = VurInfo {
        format_version: 1,
        pkgname: pkgname.clone(),
        version,
        revision,
        archs,
        subpackages: vec![],
        depends: split_list("depends"),
        hostmakedepends: split_list("hostmakedepends"),
        makedepends: split_list("makedepends"),
        checkdepends: split_list("checkdepends"),
        build_style: vars.get("build_style").cloned().filter(|s| !s.is_empty()),
        distfiles: split_list("distfiles"),
        checksum,
        provides: split_list("provides"),
        replaces: split_list("replaces"),
        restricted: vars.get("restricted").map(|s| s == "yes" || s == "true" || s == "1").unwrap_or(false),
        maintainer: vars.get("maintainer").cloned(),
    };
    // Validar
    info.validate()?;
    Ok(info)
}

fn decode_pem_body(pem: &str) -> Result<Vec<u8>> {
    let mut body = String::new();
    let mut inside = false;
    for raw_line in pem.lines() {
        let line = raw_line.trim();
        if line.starts_with("-----BEGIN") {
            inside = true;
            continue;
        }
        if line.starts_with("-----END") {
            break;
        }
        if inside && !line.is_empty() {
            body.push_str(line);
        }
    }
    if body.is_empty() {
        bail!("el PEM no contiene cuerpo base64");
    }
    general_purpose::STANDARD
        .decode(body.as_bytes())
        .context("base64 inválido en el cuerpo del PEM")
}

#[cfg(test)]
mod tests {
    use super::*;

    const HELLO_VURINFO: &str = r#"{"format_version":1,"pkgname":"hello","version":"1.0","revision":1,"archs":["x86_64"],"checksum":["sha256:aa"]}"#;
    const GOODBYE_VURINFO: &str = r#"{"format_version":1,"pkgname":"goodbye","version":"2.0_1","revision":1,"archs":["x86_64"],"checksum":["sha256:bb"]}"#;
    const THIRD_VURINFO: &str = r#"{"format_version":1,"pkgname":"third","version":"3.0_1","revision":1,"archs":["x86_64"],"checksum":["sha256:cc"]}"#;
    const TEST_PEM: &str = "-----BEGIN PUBLIC KEY-----\naGVsbG8gd29ybGQ=\n-----END PUBLIC KEY-----\n";
    const EXPECTED_FP: &str = "b9:4d:27:b9:93:4d:3e:08:a5:2e:52:d7:da:7d:ab:fa:c4:84:ef:e3:7a:53:80:ee:90:88:f7:ac:e2:ef:cd:e9";

    struct Fixture {
        _origin_tmp: tempfile::TempDir,
        _clone_tmp: tempfile::TempDir,
        origin: PathBuf,
        repo: VurRepo,
    }

    fn run_git(dir: &Path, args: &[&str]) -> Result<()> {
        let output = Command::new("git").arg("-C").arg(dir).args(args).output()?;
        anyhow::ensure!(
            output.status.success(),
            "git {:?} falló: {}",
            args,
            String::from_utf8_lossy(&output.stderr).trim()
        );
        Ok(())
    }

    fn vurinfo_for(name: &str) -> &'static str {
        match name {
            "goodbye" => GOODBYE_VURINFO,
            _ => THIRD_VURINFO,
        }
    }

    fn setup_repo(extra_pkgs: &[&str]) -> Result<Fixture> {
        let origin_tmp = tempfile::tempdir()?;
        let origin = origin_tmp.path().join("origin");
        std::fs::create_dir_all(origin.join("srcpkgs/hello"))?;
        std::fs::write(origin.join("srcpkgs/hello/.VURINFO"), HELLO_VURINFO)?;
        std::fs::write(origin.join("srcpkgs/hello/template"), "pkgname=hello\n")?;
        for name in extra_pkgs {
            let dir = origin.join("srcpkgs").join(name);
            std::fs::create_dir_all(&dir)?;
            std::fs::write(dir.join(".VURINFO"), vurinfo_for(name))?;
            std::fs::write(dir.join("template"), format!("pkgname={name}\n"))?;
        }
        run_git(&origin, &["init"])?;
        run_git(&origin, &["config", "user.email", "test@vary.local"])?;
        run_git(&origin, &["config", "user.name", "Vary Test"])?;
        run_git(&origin, &["add", "."])?;
        run_git(&origin, &["commit", "--no-gpg-sign", "-m", "init"])?;
        run_git(&origin, &["branch", "-M", "main"])?;

        let clone_tmp = tempfile::tempdir()?;
        let path = clone_tmp.path().join("mi-repo");
        let entry = RepoEntry {
            url: format!("file://{}/", origin.display()),
            branch: Some("main".into()),
            ..Default::default()
        };
        Ok(Fixture {
            _origin_tmp: origin_tmp,
            _clone_tmp: clone_tmp,
            origin,
            repo: VurRepo {
                name: "mi-repo".into(),
                path,
                entry,
            },
        })
    }

    fn fresh_cache() -> Result<(tempfile::TempDir, CacheIndex)> {
        let dir = tempfile::tempdir()?;
        let cache = CacheIndex::load(dir.path().join("cache.json"))?;
        Ok((dir, cache))
    }

    #[test]
    fn clone_head_load_cache_project_fingerprint() -> Result<()> {
        let fx = setup_repo(&[])?;
        let repo = &fx.repo;

        repo.ensure_cloned()?;
        assert!(repo.path.join(".git").exists());
        repo.ensure_cloned()?;

        let sha = repo.head_sha()?;
        assert_eq!(sha.len(), 40);
        assert!(sha.bytes().all(|b| b.is_ascii_hexdigit()));

        let (_cache_dir, mut cache) = fresh_cache()?;
        let idx = repo.load_index(&mut cache, None)?;
        assert_eq!(idx.len(), 1);
        assert_eq!(idx[0].pkgname, "hello");
        assert_eq!(idx[0].version, "1.0");
        assert_eq!(idx[0].revision, 1);
        assert_eq!(idx[0].archs, vec!["x86_64".to_string()]);
        assert_eq!(idx[0].checksum, vec!["sha256:aa".to_string()]);

        std::fs::remove_file(repo.path.join("srcpkgs/hello/.VURINFO"))?;
        let idx2 = repo.load_index(&mut cache, None)?;
        assert_eq!(idx2.len(), 1, "la segunda lectura debe venir de caché");

        let master = fx._clone_tmp.path().join("master");
        for name in ["hello"] {
            repo.project_pkg(&master, name, false)?;
        }
        let dest = master.join("hello");
        assert!(dest.is_dir(), "project debe crear directorio copiado");
        assert!(dest.join(".vur_projection_marker").exists());

        repo.project_pkg(&master, "hello", false)?;
        assert!(dest.join(".vur_projection_marker").exists());

        std::fs::remove_dir_all(&dest)?;
        std::os::unix::fs::symlink(fx._clone_tmp.path(), &dest)?;
        repo.project_pkg(&master, "hello", false)?;
        assert!(dest.is_dir(), "project debe reparar symlink apuntando a otro destino");
        assert!(dest.join(".vur_projection_marker").exists());

        let externo = fx._clone_tmp.path().join("externo");
        std::fs::create_dir_all(&externo)?;
        std::os::unix::fs::symlink(&externo, master.join("externo"))?;

        for name in ["hello"] {
            repo.unproject_pkg(&master, name)?;
        }
        assert!(!dest.exists(), "la proyección del repo debe eliminarse");
        assert!(
            master.join("externo").symlink_metadata().is_ok(),
            "los symlinks ajenos al repo deben sobrevivir"
        );

        assert!(repo.discover_public_key().is_none());
        std::fs::create_dir_all(repo.path.join("keys"))?;
        std::fs::write(repo.path.join("keys/vur.rsa"), "clave")?;
        assert_eq!(
            repo.discover_public_key(),
            Some(repo.path.join("keys/vur.rsa"))
        );

        let pem = repo.path.join("keys/vur.pem");
        std::fs::write(&pem, TEST_PEM)?;
        let fp1 = VurRepo::fingerprint_sha256(&pem)?;
        let fp2 = VurRepo::fingerprint_sha256(&pem)?;
        assert_eq!(fp1, fp2, "el fingerprint debe ser determinista");
        assert_eq!(fp1.len(), 95);
        assert_eq!(fp1.matches(':').count(), 31);
        assert!(fp1.chars().all(|c| c == ':' || c.is_ascii_hexdigit()));
        assert_eq!(fp1, EXPECTED_FP);
        Ok(())
    }

    #[test]
    fn pull_refreshes_index_via_new_head() -> Result<()> {
        let fx = setup_repo(&["goodbye"])?;
        fx.repo.ensure_cloned()?;

        let (_cache_dir, mut cache) = fresh_cache()?;
        let idx = fx.repo.load_index(&mut cache, None)?;
        assert_eq!(idx.len(), 2);
        let old_sha = fx.repo.head_sha()?;

        std::fs::create_dir_all(fx.origin.join("srcpkgs/third"))?;
        std::fs::write(fx.origin.join("srcpkgs/third/.VURINFO"), THIRD_VURINFO)?;
        run_git(&fx.origin, &["add", "."])?;
        run_git(&fx.origin, &["commit", "--no-gpg-sign", "-am", "tercer paquete"])?;

        let new_sha = fx.repo.pull()?;
        assert_ne!(old_sha, new_sha);

        let idx2 = fx.repo.load_index(&mut cache, None)?;
        assert_eq!(idx2.len(), 3);
        assert!(idx2.iter().any(|p| p.pkgname == "third"));
        Ok(())
    }

    #[test]
    fn empty_repo_reports_error() -> Result<()> {
        let origin_tmp = tempfile::tempdir()?;
        let origin = origin_tmp.path().join("origin");
        std::fs::create_dir_all(&origin)?;
        std::fs::write(origin.join("README.md"), "vacío")?;
        run_git(&origin, &["init"])?;
        run_git(&origin, &["config", "user.email", "test@vary.local"])?;
        run_git(&origin, &["config", "user.name", "Vary Test"])?;
        run_git(&origin, &["add", "."])?;
        run_git(&origin, &["commit", "--no-gpg-sign", "-m", "init"])?;
        run_git(&origin, &["branch", "-M", "main"])?;

        let clone_tmp = tempfile::tempdir()?;
        let repo = VurRepo {
            name: "vacio".into(),
            path: clone_tmp.path().join("repo"),
            entry: RepoEntry {
                url: format!("file://{}/", origin.display()),
                ..Default::default()
            },
        };
        repo.ensure_cloned()?;

        let (_cache_dir, mut cache) = fresh_cache()?;
        assert!(repo.load_index(&mut cache, None).is_err());
        Ok(())
    }

    #[test]
    fn decode_pem_body_rejects_input_without_body() {
        assert!(decode_pem_body("sin marcadores pem").is_err());
    }
}
