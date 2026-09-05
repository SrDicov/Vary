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
    /// Binario de git a usar (--git); por defecto "git".
    pub git_bin: String,
}

/// Directorios de plantillas aceptados en un VUR, en orden de preferencia:
/// el clásico `srcpkgs/` de void-packages y el alias `pkgs/` que usan repos
/// como voiders-community/repository. "" (ausente) = repo flat.
pub(crate) const TEMPLATE_PREFIXES: &[&str] = &["srcpkgs", "pkgs"];

impl VurRepo {
    pub fn ensure_cloned(&self) -> Result<()> {
        if self.path.join(".git").exists() {
            // Migrar clone legacy (sin --filter) a partial clone
            let promisor = Command::new(&self.git_bin)
                .arg("-C").arg(&self.path)
                .args(["config", "--get", "remote.origin.promisor"])
                .output();
            let is_partial = matches!(promisor, Ok(ref o) if o.status.success());
            if !is_partial {
                tracing::info!("migrando clone legacy de '{}' a partial clone...", self.name);
                let backup = self.path.with_extension("legacy-backup");
                if backup.exists() {
                    let _ = std::fs::remove_dir_all(&backup);
                }
                std::fs::rename(&self.path, &backup)
                    .with_context(|| format!("no se pudo mover {} para migración", self.path.display()))?;
                match self.clone_partial() {
                    Ok(()) => { let _ = std::fs::remove_dir_all(&backup); }
                    Err(e) => {
                        // Restaurar backup si falló el re-clone
                        let _ = std::fs::rename(&backup, &self.path);
                        return Err(e);
                    }
                }
            }
            return Ok(());
        }
        self.clone_partial()
    }

    fn clone_partial(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("no se pudo crear {}", parent.display()))?;
            }
        }
        let branch = self.entry.branch_or_default();
        let output = Command::new(&self.git_bin)
            .args([
                "clone",
                "--filter=blob:none",
                "--no-checkout",
                "--depth", "1",
                "--branch", branch,
            ])
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

    /// Detecta la rama por defecto del remoto (`main`, `master`, ...) sin clonar.
    ///
    /// Usa `git ls-remote --symref <url> HEAD` y lee la línea
    /// `ref: refs/heads/<rama>`. Devuelve `None` si no se puede determinar
    /// (sin red, remoto vacío o git ausente); el llamador aplica su fallback.
    pub fn detect_default_branch(git_bin: &str, url: &str) -> Option<String> {
        let output = Command::new(git_bin)
            .args(["ls-remote", "--symref", url, "HEAD"])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            if let Some(rest) = line.strip_prefix("ref:") {
                let rest = rest.trim();
                if let Some(branch) = rest.strip_prefix("refs/heads/") {
                    let branch = branch.split_whitespace().next().unwrap_or(branch);
                    if !branch.is_empty() {
                        return Some(branch.to_string());
                    }
                }
            }
        }
        None
    }

    pub fn head_sha(&self) -> Result<String> {
        let output = Command::new(&self.git_bin)
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
        let branch = self.entry.branch_or_default();
        // Fetch sin tocar worktree
        let fetch_out = Command::new(&self.git_bin)
            .arg("-C").arg(&self.path)
            .args(["fetch", "--depth", "1", "origin", branch])
            .output()
            .context("no se pudo ejecutar git fetch")?;
        if !fetch_out.status.success() {
            bail!(
                "git fetch falló en {}:\n{}",
                self.path.display(),
                String::from_utf8_lossy(&fetch_out.stderr).trim()
            );
        }
        // Actualizar HEAD sin checkout completo
        let reset_out = Command::new(&self.git_bin)
            .arg("-C").arg(&self.path)
            .args(["reset", "--soft", "FETCH_HEAD"])
            .output()
            .context("no se pudo ejecutar git reset --soft")?;
        if !reset_out.status.success() {
            bail!(
                "git reset --soft falló en {}:\n{}",
                self.path.display(),
                String::from_utf8_lossy(&reset_out.stderr).trim()
            );
        }
        self.head_sha()
    }

    /// Lista los nombres de paquetes disponibles usando git ls-tree (sin checkout).
    /// Soporta layout flat (`<pkg>/template`), void-packages
    /// (`srcpkgs/<pkg>/template`) y su alias (`pkgs/<pkg>/template`, p. ej. voiders).
    pub fn list_packages(&self) -> Result<Vec<String>> {
        let output = Command::new(&self.git_bin)
            .arg("-C").arg(&self.path)
            .args(["ls-tree", "--name-only", "HEAD"])
            .output()
            .context("no se pudo ejecutar git ls-tree")?;

        if !output.status.success() {
            bail!("git ls-tree falló en {}", self.path.display());
        }

        let top_entries: Vec<String> = String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(|s| s.to_string())
            .collect();

        // Detectar layout: ¿existe `srcpkgs/` o su alias `pkgs/`?
        if let Some(prefix) = TEMPLATE_PREFIXES
            .iter()
            .find(|p| top_entries.iter().any(|e| e == *p))
        {
            let prefix = *prefix;
            let subdir = format!("{prefix}/");
            let output2 = Command::new(&self.git_bin)
                .arg("-C").arg(&self.path)
                .args(["ls-tree", "--name-only", "HEAD", subdir.as_str()])
                .output()
                .with_context(|| format!("no se pudo ejecutar git ls-tree {subdir}"))?;

            Ok(String::from_utf8_lossy(&output2.stdout)
                .lines()
                .filter_map(|line| line.strip_prefix(subdir.as_str()))
                .filter(|s| !s.is_empty() && !s.starts_with('.'))
                .map(|s| s.to_string())
                .collect())
        } else {
            // Layout flat (como cnr): cada directorio de nivel 1 = paquete potencial
            // Excluir archivos sueltos (README.md, LICENSE, etc.)
            let output_full = Command::new(&self.git_bin)
                .arg("-C").arg(&self.path)
                .args(["ls-tree", "HEAD"])
                .output()
                .context("no se pudo ejecutar git ls-tree")?;
            Ok(String::from_utf8_lossy(&output_full.stdout)
                .lines()
                .filter(|line| line.contains("\ttree\t") || line.contains(" tree ") || {
                    // ls-tree format: "<mode> <type> <hash>\t<name>"
                    let parts: Vec<&str> = line.splitn(4, |c: char| c.is_whitespace()).collect();
                    parts.len() >= 4 && parts[1] == "tree"
                })
                .filter_map(|line| line.split('\t').nth(1))
                .filter(|name| !name.starts_with('.'))
                .map(|s| s.to_string())
                .collect())
        }
    }

    /// Detecta el prefijo de layout del repo: `srcpkgs`, su alias `pkgs`,
    /// o vacío para flat.
    fn detect_layout_prefix(&self) -> Result<String> {
        let output = Command::new(&self.git_bin)
            .arg("-C").arg(&self.path)
            .args(["ls-tree", "--name-only", "HEAD"])
            .output()?;
        let entries = String::from_utf8_lossy(&output.stdout);
        for prefix in TEMPLATE_PREFIXES {
            if entries.lines().any(|l| l == *prefix) {
                return Ok(prefix.to_string());
            }
        }
        Ok(String::new())
    }

    /// Lee el contenido de un archivo del repo sin materializarlo en disco.
    /// Usa `git show HEAD:<path>` para acceder directo al object store.
    pub fn git_show_file(&self, tree_path: &str) -> Result<String> {
        let output = Command::new(&self.git_bin)
            .arg("-C").arg(&self.path)
            .args(["show", &format!("HEAD:{}", tree_path)])
            .output()
            .with_context(|| format!("no se pudo ejecutar git show HEAD:{}", tree_path))?;
        if !output.status.success() {
            bail!("archivo no encontrado en el repo: {}", tree_path);
        }
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    /// Materializa solo el directorio de un paquete específico en el worktree
    /// usando git sparse-checkout. Descarga solo los blobs necesarios.
    pub fn materialize_pkg(&self, pkg_name: &str) -> Result<PathBuf> {
        let prefix = self.detect_layout_prefix()?;
        let sparse_path = if prefix.is_empty() {
            pkg_name.to_string()
        } else {
            format!("{}/{}", prefix, pkg_name)
        };

        // Inicializar sparse-checkout si no está configurado
        if !self.path.join(".git/info/sparse-checkout").exists() {
            let _ = Command::new(&self.git_bin)
                .arg("-C").arg(&self.path)
                .args(["sparse-checkout", "init", "--cone"])
                .output();
        }

        // Añadir el paquete al sparse-checkout
        let add_out = Command::new(&self.git_bin)
            .arg("-C").arg(&self.path)
            .args(["sparse-checkout", "add", &sparse_path])
            .output()
            .with_context(|| format!("no se pudo agregar {} al sparse-checkout", sparse_path))?;

        if !add_out.status.success() {
            tracing::warn!(
                "sparse-checkout add falló para {}: {}",
                sparse_path,
                String::from_utf8_lossy(&add_out.stderr).trim()
            );
        }

        // Hacer checkout (Git descargará solo los blobs faltantes)
        let co_out = Command::new(&self.git_bin)
            .arg("-C").arg(&self.path)
            .args(["checkout"])
            .output()
            .context("no se pudo ejecutar git checkout")?;

        if !co_out.status.success() {
            tracing::warn!(
                "git checkout parcial: {}",
                String::from_utf8_lossy(&co_out.stderr).trim()
            );
        }

        let materialized = self.path.join(&sparse_path);
        if !materialized.exists() {
            bail!("el paquete '{}' no existe en el VUR '{}'", pkg_name, self.name);
        }

        Ok(materialized)
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

        // Invalidar entradas antiguas de este repo antes de reconstruir
        cache.invalidate_repo(&self.name);

        let mut packages: Vec<VurInfo> = Vec::new();
        let mut files_seen = 0usize;

        let prefix = self.detect_layout_prefix()?;
        let pkg_names = self.list_packages()?;

        // Intentar leer .VURINFO de cada paquete vía git show (sin checkout)
        for pkg in &pkg_names {
            let vurinfo_path = if prefix.is_empty() {
                format!("{}/.VURINFO", pkg)
            } else {
                format!("{}/{}/.VURINFO", prefix, pkg)
            };
            if let Ok(text) = self.git_show_file(&vurinfo_path) {
                files_seen += 1;
                match metadata::parse(&text) {
                    Ok(info) => packages.push(info),
                    Err(err) => tracing::warn!(
                        ".VURINFO inválido ignorado en {}:{}: {err:#}",
                        self.name, vurinfo_path
                    ),
                }
            }
        }

        // Intentar .VURINFO raíz (array de paquetes)
        if let Ok(text) = self.git_show_file(".VURINFO") {
            files_seen += 1;
            match metadata::parse_many(&text) {
                Ok(mut infos) => packages.append(&mut infos),
                Err(err) => tracing::warn!(
                    ".VURINFO raíz inválido ignorado en {}: {err:#}",
                    self.name
                ),
            }
        }

        // Fallback: si no hay .VURINFO, parsear templates vía git show
        if packages.is_empty() && files_seen == 0 {
            for pkg in &pkg_names {
                let tmpl_path = if prefix.is_empty() {
                    format!("{}/template", pkg)
                } else {
                    format!("{}/{}/template", prefix, pkg)
                };
                if let Ok(text) = self.git_show_file(&tmpl_path) {
                    files_seen += 1;
                    match parse_template_text(&text, &format!("{}:{}", self.name, tmpl_path)) {
                        Ok(info) => {
                            tracing::debug!("template parseado {} -> {}", tmpl_path, info.pkgname);
                            packages.push(info);
                        }
                        Err(err) => tracing::warn!(
                            "template inválido ignorado en {}:{}: {err:#}",
                            self.name, tmpl_path
                        ),
                    }
                }
            }
        }

        if packages.is_empty() && files_seen == 0 {
            bail!("no se encontró ningún .VURINFO ni template en {}", self.name);
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
        // Buscar el directorio fuente del paquete (prefijos conocidos + flat)
        let mut candidates = Vec::new();
        for prefix in TEMPLATE_PREFIXES {
            candidates.push(self.path.join(prefix).join(pkgname));
        }
        candidates.push(self.path.join(pkgname));
        let src = candidates.iter().find(|p| p.join("template").is_file()).cloned()
            .or_else(|| {
                let mut found = None;
                for base in TEMPLATE_PREFIXES
                    .iter()
                    .map(|p| self.path.join(p))
                    .chain(std::iter::once(self.path.clone()))
                {
                    if let Ok(entries) = std::fs::read_dir(&base) {
                        for entry in entries.flatten() {
                            let p = entry.path();
                            if p.is_dir() {
                                if let Ok(text) = std::fs::read_to_string(p.join("template")) {
                                    if let Ok(info) = parse_template_text(&text, p.join("template").to_string_lossy().as_ref()) {
                                        if info.pkgname == pkgname || info.subpackages.iter().any(|s| s.pkgname == pkgname) {
                                            found = Some(p);
                                            break;
                                        }
                                    }
                                }
                                if p.file_name().and_then(|n| n.to_str()) == Some(pkgname) {
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
                    let _ = Command::new(&self.git_bin)
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

fn parse_template_text(content: &str, debug_path: &str) -> Result<VurInfo> {
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
        .ok_or_else(|| anyhow::anyhow!("template sin pkgname: {}", debug_path))?;
    let version = vars.get("version").cloned().unwrap_or_else(|| "1.0".to_string());
    let revision: u32 = vars.get("revision").and_then(|s| s.parse().ok()).unwrap_or(1);
    let archs_raw = vars.get("only_for_archs").or_else(|| vars.get("archs")).cloned().unwrap_or_default();
    // Normalizar wildcards de Void ("x86_64*", "aarch64*") a la base
    let archs = if archs_raw.is_empty() {
        vec!["all".to_string()]
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
        setup_repo_on_branch(extra_pkgs, "main")
    }

    fn setup_repo_on_branch(extra_pkgs: &[&str], branch: &str) -> Result<Fixture> {
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
        run_git(&origin, &["branch", "-M", branch])?;

        let clone_tmp = tempfile::tempdir()?;
        let path = clone_tmp.path().join("mi-repo");
        let entry = RepoEntry {
            url: format!("file://{}/", origin.display()),
            branch: Some(branch.into()),
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
                git_bin: "git".to_string(),
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

        let idx2 = repo.load_index(&mut cache, None)?;
        assert_eq!(idx2.len(), 1, "la segunda lectura debe venir de caché");

        let master = fx._clone_tmp.path().join("master");
        for name in ["hello"] {
            repo.materialize_pkg(name)?;
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
            git_bin: "git".to_string(),
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

    #[test]
    fn clone_supports_master_branch() -> Result<()> {
        // Regresión: los repos clásicos en `master` (p. ej. z-packages)
        // deben clonarse cuando la entrada declara esa rama.
        let fx = setup_repo_on_branch(&[], "master")?;
        fx.repo.ensure_cloned()?;
        assert!(fx.repo.path.join(".git").exists());

        let (_cache_dir, mut cache) = fresh_cache()?;
        let idx = fx.repo.load_index(&mut cache, None)?;
        assert_eq!(idx.len(), 1);
        assert_eq!(idx[0].pkgname, "hello");
        Ok(())
    }

    #[test]
    fn detect_default_branch_reads_remote_head() -> Result<()> {
        let fx_main = setup_repo(&[])?;
        let fx_master = setup_repo_on_branch(&[], "master")?;
        let url_main = format!("file://{}/", fx_main.origin.display());
        let url_master = format!("file://{}/", fx_master.origin.display());
        assert_eq!(
            VurRepo::detect_default_branch("git", &url_main).as_deref(),
            Some("main")
        );
        assert_eq!(
            VurRepo::detect_default_branch("git", &url_master).as_deref(),
            Some("master")
        );
        assert!(VurRepo::detect_default_branch("git", "file:///no/existe").is_none());
        Ok(())
    }

    #[test]
    fn pkgs_layout_resuelve_por_fallback_de_template() -> Result<()> {
        // Forma voiders-community/repository: dir `pkgs/`, sin .VURINFO.
        // El índice debe salir del parseo del template (datos no evaluados).
        let origin_tmp = tempfile::tempdir()?;
        let origin = origin_tmp.path().join("origin");
        std::fs::create_dir_all(origin.join("pkgs/hyfetch"))?;
        std::fs::write(
            origin.join("pkgs/hyfetch/template"),
            "pkgname=hyfetch\nversion=2.1.0\nrevision=1\n",
        )?;
        run_git(&origin, &["init"])?;
        run_git(&origin, &["config", "user.email", "test@vary.local"])?;
        run_git(&origin, &["config", "user.name", "Vary Test"])?;
        run_git(&origin, &["add", "."])?;
        run_git(&origin, &["commit", "--no-gpg-sign", "-m", "init"])?;
        run_git(&origin, &["branch", "-M", "main"])?;

        let clone_tmp = tempfile::tempdir()?;
        let repo = VurRepo {
            name: "voiders".into(),
            path: clone_tmp.path().join("voiders"),
            entry: RepoEntry {
                url: format!("file://{}/", origin.display()),
                branch: Some("main".into()),
                ..Default::default()
            },
            git_bin: "git".to_string(),
        };
        repo.ensure_cloned()?;

        assert_eq!(repo.list_packages()?, vec!["hyfetch".to_string()]);

        let (_cache_dir, mut cache) = fresh_cache()?;
        let idx = repo.load_index(&mut cache, None)?;
        assert_eq!(idx.len(), 1);
        assert_eq!(idx[0].pkgname, "hyfetch");
        assert_eq!(idx[0].version, "2.1.0");
        assert_eq!(idx[0].revision, 1);

        // El template debe ser proyectable al árbol maestro.
        let master = clone_tmp.path().join("master");
        repo.materialize_pkg("hyfetch")?;
        repo.project_pkg(&master, "hyfetch", false)?;
        let dest = master.join("hyfetch");
        assert!(dest.join("template").is_file());
        assert!(dest.join(".vur_projection_marker").exists());
        Ok(())
    }
}
