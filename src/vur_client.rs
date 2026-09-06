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

/// Construye el aviso al usuario cuando un índice o plantilla VUR se descarta
/// por inválido (H-018). Función pura para poder testear el texto exacto; los
/// emisores usan `eprintln!` (stderr) para garantizar que llegue a la terminal
/// aunque el subscriber de tracing no esté configurado o filtre por nivel
/// (la capa de consola escribe a stdout y en tests no hay subscriber).
pub(crate) fn skipped_index_warning(
    kind: &str,
    location: &str,
    err: &dyn std::fmt::Display,
) -> String {
    format!("advertencia: se ignoró {kind} en {location}: {err:#}")
}

pub fn is_safe_git_url(url: &str) -> bool {
    if url.contains('\n') || url.contains('\r') || url.chars().any(|c| c.is_control()) {
        return false;
    }
    let u = url.trim();
    if u.is_empty() || u.starts_with('-') {
        return false;
    }
    u.starts_with("https://")
        || u.starts_with("http://")
        || u.starts_with("git://")
        || u.starts_with("ssh://")
        || u.starts_with("file://")
        || (u.starts_with("git@") && u.contains(':'))
}

impl VurRepo {
    pub fn ensure_cloned(&self) -> Result<()> {
        if self.path.join(".git").exists() {
            // Migrar clone legacy (sin --filter) a partial clone
            let promisor = Command::new(&self.git_bin)
                .arg("-C")
                .arg(&self.path)
                .args(["config", "--get", "remote.origin.promisor"])
                .output();
            let is_partial = matches!(promisor, Ok(ref o) if o.status.success());
            if !is_partial {
                tracing::info!(
                    "migrando clone legacy de '{}' a partial clone...",
                    self.name
                );
                let backup = self.path.with_extension("legacy-backup");
                if backup.exists() {
                    let _ = std::fs::remove_dir_all(&backup);
                }
                std::fs::rename(&self.path, &backup).with_context(|| {
                    format!("no se pudo mover {} para migración", self.path.display())
                })?;
                match self.clone_partial() {
                    Ok(()) => {
                        let _ = std::fs::remove_dir_all(&backup);
                    }
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
        if !is_safe_git_url(&self.entry.url) {
            bail!(
                "URL de repositorio git insegura o inválida: '{}'",
                self.entry.url
            );
        }
        if let Some(parent) = self.path.parent() {
            if !parent.as_os_str().is_empty() {
                crate::util::ensure_private_dir(parent)
                    .with_context(|| format!("no se pudo crear {}", parent.display()))?;
            }
        }
        let branch = self.entry.branch_or_default();
        let output = Command::new(&self.git_bin)
            .args([
                "-c",
                "protocol.ext.allow=never",
                "-c",
                "protocol.file.allow=user",
                "clone",
                "--filter=blob:none",
                "--no-checkout",
                "--depth",
                "1",
                "--branch",
                branch,
                "--",
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
        if !is_safe_git_url(url) {
            return None;
        }
        let output = Command::new(git_bin)
            .args([
                "-c",
                "protocol.ext.allow=never",
                "-c",
                "protocol.file.allow=user",
                "ls-remote",
                "--symref",
                "--",
                url,
                "HEAD",
            ])
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
            .arg("-C")
            .arg(&self.path)
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
            .arg("-C")
            .arg(&self.path)
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
            .arg("-C")
            .arg(&self.path)
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
                .arg("-C")
                .arg(&self.path)
                .args(["ls-tree", "--name-only", "HEAD", subdir.as_str()])
                .output()
                .with_context(|| format!("no se pudo ejecutar git ls-tree {subdir}"))?;

            Ok(String::from_utf8_lossy(&output2.stdout)
                .lines()
                .filter_map(|line| line.strip_prefix(subdir.as_str()))
                .filter(|s| !s.is_empty() && !s.starts_with('.'))
                .map(|s| s.to_string())
                .collect::<Vec<_>>())
            .map(|top| self.con_nivel_extra(top, prefix))
        } else {
            // Layout flat (como cnr): cada directorio de nivel 1 = paquete potencial
            // Excluir archivos sueltos (README.md, LICENSE, etc.)
            let output_full = Command::new(&self.git_bin)
                .arg("-C")
                .arg(&self.path)
                .args(["ls-tree", "HEAD"])
                .output()
                .context("no se pudo ejecutar git ls-tree")?;
            Ok(String::from_utf8_lossy(&output_full.stdout)
                .lines()
                .filter(|line| {
                    line.contains("\ttree\t") || line.contains(" tree ") || {
                        // ls-tree format: "<mode> <type> <hash>\t<name>"
                        let parts: Vec<&str> =
                            line.splitn(4, |c: char| c.is_whitespace()).collect();
                        parts.len() >= 4 && parts[1] == "tree"
                    }
                })
                .filter_map(|line| line.split('\t').nth(1))
                .filter(|name| !name.starts_with('.'))
                .map(|s| s.to_string())
                .collect())
        }
    }

    /// Descubrimiento profundo, un nivel extra (A6): en repos con categorías
    /// (`srcpkgs/<cat>/<pkg>`, estilo VUP/monorepo) las entradas de nivel 1
    /// son categorías, no paquetes. Con un solo `ls-tree -r` se detectan los
    /// subdirs con `template`/`.VURINFO` y se listan como `cat/pkg` (forma que
    /// `load_index` ya sabe abrir). La categoría se retira solo si no trae
    /// índice propio; sin anidados el resultado es idéntico al anterior.
    /// Límite documentado: profundidad 2 (una categoría); más hondo sigue
    /// invisible.
    fn con_nivel_extra(&self, top: Vec<String>, prefix: &str) -> Vec<String> {
        let subdir = format!("{prefix}/");
        let output = match Command::new(&self.git_bin)
            .arg("-C")
            .arg(&self.path)
            .args(["ls-tree", "-r", "--name-only", "HEAD", subdir.as_str()])
            .output()
        {
            Ok(o) if o.status.success() => o,
            _ => return top,
        };
        let mut con_indice: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            for marker in ["template", ".VURINFO"] {
                if let Some(dir) = line
                    .strip_suffix(&format!("/{marker}"))
                    .and_then(|d| d.strip_prefix(subdir.as_str()))
                {
                    if !dir.split('/').any(|c| c.is_empty() || c.starts_with('.')) {
                        con_indice.insert(dir.to_string());
                    }
                }
            }
        }
        let mut out: Vec<String> = top
            .into_iter()
            .filter(|e| {
                // Quitar la categoría solo si no es paquete por sí misma.
                e.contains('/') || con_indice.contains(e.as_str())
            })
            .collect();
        for dir in con_indice {
            if dir.matches('/').count() == 1 && !out.contains(&dir) {
                out.push(dir);
            }
        }
        out.sort();
        out
    }

    /// Detecta el prefijo de layout del repo: `srcpkgs`, su alias `pkgs`,
    /// o vacío para flat.
    fn detect_layout_prefix(&self) -> Result<String> {
        let output = Command::new(&self.git_bin)
            .arg("-C")
            .arg(&self.path)
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
    /// Contenido del `template` de `pkg` en HEAD sin tocar el worktree (A3).
    /// Prueba `srcpkgs/`, `pkgs/` y flat. `None` si no existe en HEAD.
    pub(crate) fn read_template(&self, pkg: &str) -> Option<String> {
        TEMPLATE_PREFIXES
            .iter()
            .map(|p| format!("{p}/{pkg}/template"))
            .chain(std::iter::once(format!("{pkg}/template")))
            .find_map(|path| self.git_show_file(&path).ok())
    }

    pub fn git_show_file(&self, tree_path: &str) -> Result<String> {
        let output = Command::new(&self.git_bin)
            .arg("-C")
            .arg(&self.path)
            .args(["show", &format!("HEAD:{tree_path}")])
            .output()
            .with_context(|| format!("no se pudo ejecutar git show HEAD:{tree_path}"))?;
        if !output.status.success() {
            bail!("archivo no encontrado en el repo: {tree_path}");
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
            format!("{prefix}/{pkg_name}")
        };

        // Inicializar sparse-checkout si no está configurado
        if !self.path.join(".git/info/sparse-checkout").exists() {
            let _ = Command::new(&self.git_bin)
                .arg("-C")
                .arg(&self.path)
                .args(["sparse-checkout", "init", "--cone"])
                .output();
        }

        // Añadir el paquete al sparse-checkout
        let add_out = Command::new(&self.git_bin)
            .arg("-C")
            .arg(&self.path)
            .args(["sparse-checkout", "add", &sparse_path])
            .output()
            .with_context(|| format!("no se pudo agregar {sparse_path} al sparse-checkout"))?;

        if !add_out.status.success() {
            tracing::warn!(
                "sparse-checkout add falló para {}: {}",
                sparse_path,
                String::from_utf8_lossy(&add_out.stderr).trim()
            );
        }

        // Hacer checkout (Git descargará solo los blobs faltantes)
        let co_out = Command::new(&self.git_bin)
            .arg("-C")
            .arg(&self.path)
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
            bail!(
                "el paquete '{}' no existe en el VUR '{}'",
                pkg_name,
                self.name
            );
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
                format!("{pkg}/.VURINFO")
            } else {
                format!("{prefix}/{pkg}/.VURINFO")
            };
            match self.git_show_file(&vurinfo_path) {
                Ok(text) => {
                    files_seen += 1;
                    match metadata::parse(&text) {
                        Ok(info) => packages.push(info),
                        Err(err) => {
                            let warning = skipped_index_warning(
                                ".VURINFO",
                                &format!("{}:{vurinfo_path}", self.name),
                                &err,
                            );
                            eprintln!("{warning}");
                        }
                    }
                }
                Err(err) => {
                    tracing::trace!("sin .VURINFO en {}:{vurinfo_path}: {err:#}", self.name);
                }
            }
        }

        // Intentar .VURINFO raíz (array de paquetes)
        match self.git_show_file(".VURINFO") {
            Ok(text) => {
                files_seen += 1;
                match metadata::parse_many(&text) {
                    Ok(mut infos) => packages.append(&mut infos),
                    Err(err) => {
                        let warning = skipped_index_warning(".VURINFO raíz", &self.name, &err);
                        eprintln!("{warning}");
                    }
                }
            }
            Err(err) => {
                tracing::trace!("sin .VURINFO raíz en {}: {err:#}", self.name);
            }
        }

        // Fallback: si no hay .VURINFO, parsear templates vía git show
        if packages.is_empty() && files_seen == 0 {
            for pkg in &pkg_names {
                let tmpl_path = if prefix.is_empty() {
                    format!("{pkg}/template")
                } else {
                    format!("{prefix}/{pkg}/template")
                };
                match self.git_show_file(&tmpl_path) {
                    Ok(text) => {
                        files_seen += 1;
                        match parse_template_text(&text, &format!("{}:{tmpl_path}", self.name)) {
                            Ok(info) => {
                                tracing::debug!(
                                    "template parseado {tmpl_path} -> {}",
                                    info.pkgname
                                );
                                packages.push(info);
                            }
                            Err(err) => {
                                let warning = skipped_index_warning(
                                    "template",
                                    &format!("{}:{tmpl_path}", self.name),
                                    &err,
                                );
                                eprintln!("{warning}");
                            }
                        }
                    }
                    Err(err) => {
                        tracing::trace!("sin template en {}:{tmpl_path}: {err:#}", self.name);
                    }
                }
            }
        }

        if packages.is_empty() && files_seen == 0 {
            bail!(
                "no se encontró ningún .VURINFO ni template en {}",
                self.name
            );
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
        crate::util::ensure_private_dir(master_srcpkgs)
            .with_context(|| format!("no se pudo crear {}", master_srcpkgs.display()))?;
        // Buscar el directorio fuente del paquete (prefijos conocidos + flat)
        let mut candidates = Vec::new();
        for prefix in TEMPLATE_PREFIXES {
            candidates.push(self.path.join(prefix).join(pkgname));
        }
        candidates.push(self.path.join(pkgname));
        let src = candidates
            .iter()
            .find(|p| p.join("template").is_file())
            .cloned()
            .or_else(|| {
                let mut found = None;
                for base in TEMPLATE_PREFIXES
                    .iter()
                    .map(|p| self.path.join(p))
                    .chain(std::iter::once(self.path.clone()))
                {
                    match std::fs::read_dir(&base) {
                        Ok(entries) => {
                            for entry in entries.flatten() {
                                let p = entry.path();
                                if p.is_dir() {
                                    let tmpl = p.join("template");
                                    if tmpl.is_file() {
                                        match std::fs::read_to_string(&tmpl) {
                                            Ok(text) => match parse_template_text(
                                                &text,
                                                &tmpl.to_string_lossy(),
                                            ) {
                                                Ok(info) => {
                                                    if info.pkgname == pkgname
                                                        || info
                                                            .subpackages
                                                            .iter()
                                                            .any(|s| s.pkgname == pkgname)
                                                    {
                                                        found = Some(p);
                                                        break;
                                                    }
                                                }
                                                Err(err) => {
                                                    let warning = skipped_index_warning(
                                                        "plantilla",
                                                        &tmpl.display().to_string(),
                                                        &err,
                                                    );
                                                    eprintln!("{warning}");
                                                }
                                            },
                                            Err(err) => {
                                                let warning = skipped_index_warning(
                                                    "plantilla ilegible",
                                                    &tmpl.display().to_string(),
                                                    &err,
                                                );
                                                eprintln!("{warning}");
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
                        Err(err) if err.kind() != std::io::ErrorKind::NotFound => {
                            tracing::warn!("error al leer directorio {}: {err:#}", base.display());
                        }
                        _ => {}
                    }
                    if found.is_some() {
                        break;
                    }
                }
                found
            })
            .ok_or_else(|| anyhow::anyhow!("template no encontrado para {pkgname}"))?;

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
                    std::fs::remove_dir_all(&dest).with_context(|| {
                        format!("no se pudo limpiar proyección previa {}", dest.display())
                    })?;
                } else if force {
                    // Target explícito del usuario: el VUR manda sobre el
                    // template oficial no-publicado presente en el árbol.
                    tracing::warn!("reemplazando template oficial local de '{pkgname}' por la versión VUR (target explícito)");
                    std::fs::remove_dir_all(&dest)
                        .with_context(|| format!("no se pudo reemplazar {}", dest.display()))?;
                } else {
                    tracing::warn!(
                        "{} ya existe como directorio oficial, se omite proyección de VUR {}",
                        dest.display(),
                        pkgname
                    );
                    return Ok(());
                }
            } else if meta.file_type().is_symlink() {
                std::fs::remove_file(&dest)
                    .with_context(|| format!("no se pudo reemplazar symlink {}", dest.display()))?;
            } else {
                std::fs::remove_file(&dest)
                    .with_context(|| format!("no se pudo reemplazar {}", dest.display()))?;
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
            std::fs::remove_dir_all(&dest)
                .with_context(|| format!("no se pudo eliminar proyección {}", dest.display()))?;
            // Restaurar template oficial si este paquete pertenece al árbol maestro
            if master_srcpkgs.join(pkgname).symlink_metadata().is_err()
                && (master_srcpkgs
                    .parent()
                    .and_then(|p| p.file_name())
                    .map(|n| n == "void-packages")
                    .unwrap_or(false)
                    || master_srcpkgs.join("../.git").exists())
            {
                let _ = Command::new(&self.git_bin)
                    .args(["checkout", "--", &format!("srcpkgs/{pkgname}")])
                    .current_dir(master_srcpkgs.parent().unwrap_or(Path::new(".")))
                    .output();
            }
        } else if dest
            .symlink_metadata()
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false)
        {
            if let Ok(target) = std::fs::read_link(&dest) {
                let abs = if target.is_absolute() {
                    target
                } else {
                    let parent = dest.parent().ok_or_else(|| {
                        anyhow::anyhow!("ruta de destino sin padre: {}", dest.display())
                    })?;
                    parent.join(target)
                };
                let resolved = abs.canonicalize().unwrap_or(abs);
                if resolved.starts_with(
                    self.path
                        .canonicalize()
                        .unwrap_or_else(|_| self.path.clone()),
                ) {
                    std::fs::remove_file(&dest).with_context(|| {
                        format!("no se pudo eliminar symlink {}", dest.display())
                    })?;
                }
            }
        }
        Ok(())
    }

    fn copy_dir_recursive(src: &Path, dest: &Path) -> Result<()> {
        crate::util::ensure_private_dir(dest)
            .with_context(|| format!("no se pudo crear {}", dest.display()))?;
        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            let src_path = entry.path();
            let dest_path = dest.join(entry.file_name());
            let ft = entry.file_type()?;
            if ft.is_dir() {
                Self::copy_dir_recursive(&src_path, &dest_path)?;
            } else if ft.is_symlink() {
                let target = std::fs::read_link(&src_path)
                    .with_context(|| format!("leyendo symlink {}", src_path.display()))?;
                std::os::unix::fs::symlink(target, &dest_path)
                    .with_context(|| format!("creando symlink {}", dest_path.display()))?;
            } else {
                std::fs::copy(&src_path, &dest_path).with_context(|| {
                    format!("copiando {} -> {}", src_path.display(), dest_path.display())
                })?;
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

fn has_unclosed_quote(buf: &str) -> bool {
    let Some(eq) = buf.find('=') else {
        return false;
    };
    let val_part = buf[eq + 1..].trim_start();
    if !val_part.starts_with('"') && !val_part.starts_with('\'') {
        return false;
    }

    let mut in_double = false;
    let mut in_single = false;
    let mut escaped = false;
    let mut prev_char: Option<char> = None;

    for c in val_part.chars() {
        if escaped {
            escaped = false;
            prev_char = Some(c);
            continue;
        }
        if !in_double && !in_single && c == '#' && prev_char.is_some_and(|p| p.is_whitespace()) {
            break;
        }
        match c {
            '\\' if !in_single => escaped = true,
            '"' if !in_single => in_double = !in_double,
            '\'' if !in_double => in_single = !in_single,
            _ => {}
        }
        prev_char = Some(c);
    }

    in_double || in_single
}

fn parse_template_text(content: &str, debug_path: &str) -> Result<VurInfo> {
    // Unir continuaciones con \ y manejar valores multilínea entre comillas
    let mut vars: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut raw_subpackages: Vec<(String, std::collections::HashMap<String, String>)> = Vec::new();
    let mut lines = content.lines().peekable();
    let mut buf = String::new();

    while let Some(line) = lines.next() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        // Detectar y advertir ruidosamente sobre constructos condicionales por arquitectura (A2)
        if (trimmed.starts_with("case ")
            || trimmed.starts_with("if ")
            || trimmed.starts_with("elif "))
            && (trimmed.contains("XBPS_TARGET_")
                || trimmed.contains("XBPS_MACHINE")
                || trimmed.contains("XBPS_ARCH"))
        {
            tracing::warn!("{debug_path}: constructo condicional por arquitectura detectado ('{trimmed}'); la extracción estática puede ser incompleta");
        }
        // Detectar definiciones de funciones shell
        if trimmed.contains("()") {
            let fn_name = trimmed.split("()").next().unwrap_or("").trim();
            if let Some(sub_name) = fn_name.strip_suffix("_package") {
                let sub_name = sub_name.trim();
                if !sub_name.is_empty() && crate::metadata::is_valid_pkgname(sub_name) {
                    let mut sub_vars: std::collections::HashMap<String, String> =
                        std::collections::HashMap::new();
                    let mut brace_depth = trimmed.matches('{').count();
                    let mut found_open = brace_depth > 0;

                    for l in lines.by_ref() {
                        let l_trimmed = l.trim();
                        let open_count = l.matches('{').count();
                        if open_count > 0 {
                            found_open = true;
                        }
                        brace_depth += open_count;
                        brace_depth = brace_depth.saturating_sub(l.matches('}').count());

                        if brace_depth == 1 && !l_trimmed.is_empty() && !l_trimmed.starts_with('#')
                        {
                            if let Some(eq) = l_trimmed.find('=') {
                                let k = l_trimmed[..eq].trim().trim_end_matches('+');
                                if matches!(k, "depends" | "short_desc") {
                                    let mut v = l_trimmed[eq + 1..].trim().to_string();
                                    if (v.starts_with('"') && v.ends_with('"') && v.len() >= 2)
                                        || (v.starts_with('\'')
                                            && v.ends_with('\'')
                                            && v.len() >= 2)
                                    {
                                        v = v[1..v.len() - 1].to_string();
                                    }
                                    v = v.split_whitespace().collect::<Vec<_>>().join(" ");
                                    sub_vars.insert(k.to_string(), v);
                                }
                            }
                        }

                        if found_open && brace_depth == 0 {
                            break;
                        }
                    }

                    raw_subpackages.push((sub_name.to_string(), sub_vars));
                    continue;
                }
            }

            // Otra función: saltar con brace matching
            let mut brace_depth = trimmed.matches('{').count();
            let mut found_open = brace_depth > 0;
            for l in lines.by_ref() {
                let open_count = l.matches('{').count();
                if open_count > 0 {
                    found_open = true;
                }
                brace_depth += open_count;
                brace_depth = brace_depth.saturating_sub(l.matches('}').count());
                if found_open && brace_depth == 0 {
                    break;
                }
            }
            continue;
        }
        if trimmed.starts_with('}') || trimmed.starts_with('{') {
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
        // Manejar valores multilínea entre comillas dobles o simples
        while has_unclosed_quote(&buf) {
            if let Some(next) = lines.next() {
                buf.push('\n');
                buf.push_str(next);
            } else {
                break;
            }
        }

        // Parsear asignaciones var=valor
        if let Some(eq) = buf.find('=') {
            let key = buf[..eq].trim().to_string();
            if !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                continue;
            }
            // Filtrar solo variables relevantes
            let relevant = matches!(
                key.as_str(),
                "pkgname"
                    | "version"
                    | "revision"
                    | "archs"
                    | "only_for_archs"
                    | "depends"
                    | "hostmakedepends"
                    | "makedepends"
                    | "checkdepends"
                    | "build_style"
                    | "distfiles"
                    | "checksum"
                    | "provides"
                    | "replaces"
                    | "restricted"
                    | "maintainer"
                    | "short_desc"
                    | "license"
                    | "homepage"
            );
            if !relevant {
                continue;
            }
            let mut val = buf[eq + 1..].trim().to_string();
            // Quitar comentarios al final (espacio + #), pero no dentro de comillas
            if let Some(hash) = val.find(" #") {
                // Verificar que no esté dentro de comillas
                let before = &val[..hash];
                if before.matches('"').count().is_multiple_of(2)
                    && before.matches('\'').count().is_multiple_of(2)
                {
                    val.truncate(hash);
                    val = val.trim().to_string();
                }
            }
            // Descomillar
            if (val.starts_with('"') && val.ends_with('"') && val.len() >= 2)
                || (val.starts_with('\'') && val.ends_with('\'') && val.len() >= 2)
            {
                val = val[1..val.len() - 1].to_string();
            } else if val.starts_with('"') || val.starts_with('\'') {
                // Valor multilínea entrecomillado sin cierre en misma línea.
                // `starts_with` garantiza no-vacío, pero sin unwrap por
                // principio (H-023): si estuviera vacío se conserva tal cual.
                if let Some(quote) = val.chars().next() {
                    val.remove(0);
                    if let Some(end) = val.rfind(quote) {
                        val.truncate(end);
                    }
                }
            }
            // Colapsar whitespace y newlines a espacios para listas
            val = val.split_whitespace().collect::<Vec<_>>().join(" ");
            vars.insert(key, val);
        }
    }

    let pkgname = vars
        .get("pkgname")
        .cloned()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("template sin pkgname: {debug_path}"))?;
    let version = vars
        .get("version")
        .cloned()
        .unwrap_or_else(|| "1.0".to_string());
    let revision: u32 = vars
        .get("revision")
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);
    let archs_raw = vars
        .get("only_for_archs")
        .or_else(|| vars.get("archs"))
        .cloned()
        .unwrap_or_default();
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
        vars.get(key)
            .map(|s| s.split_whitespace().map(|v| v.to_string()).collect())
            .unwrap_or_default()
    };
    let checksum_raw = split_list("checksum");
    // Normalizar checksum a sha256: prefijo si falta y no es SKIP
    let checksum = checksum_raw
        .into_iter()
        .map(|c| {
            if c == "SKIP" || c.starts_with("sha256:") {
                c
            } else {
                format!("sha256:{c}")
            }
        })
        .collect();

    let mut subpackages = Vec::new();
    for (sub_name, sub_vars) in raw_subpackages {
        if sub_name == pkgname
            || subpackages
                .iter()
                .any(|s: &crate::metadata::Subpackage| s.pkgname == sub_name)
        {
            continue;
        }
        let sub_depends: Vec<String> = sub_vars
            .get("depends")
            .map(|s| {
                let expanded = s
                    .replace("${sourcepkg}", &pkgname)
                    .replace("$sourcepkg", &pkgname)
                    .replace("${pkgname}", &pkgname)
                    .replace("$pkgname", &pkgname)
                    .replace("${version}", &version)
                    .replace("$version", &version)
                    .replace("${revision}", &revision.to_string())
                    .replace("$revision", &revision.to_string());
                expanded
                    .split_whitespace()
                    .map(|d| d.to_string())
                    .filter(|d| !d.trim().is_empty())
                    .collect()
            })
            .unwrap_or_default();

        let sub_short_desc = sub_vars
            .get("short_desc")
            .cloned()
            .filter(|s| !s.is_empty());

        subpackages.push(crate::metadata::Subpackage {
            pkgname: sub_name,
            depends: sub_depends,
            short_desc: sub_short_desc,
        });
    }

    let info = VurInfo {
        format_version: 1,
        pkgname: pkgname.clone(),
        version,
        revision,
        archs,
        subpackages,
        depends: split_list("depends"),
        hostmakedepends: split_list("hostmakedepends"),
        makedepends: split_list("makedepends"),
        checkdepends: split_list("checkdepends"),
        build_style: vars.get("build_style").cloned().filter(|s| !s.is_empty()),
        distfiles: split_list("distfiles"),
        checksum,
        provides: split_list("provides"),
        replaces: split_list("replaces"),
        restricted: vars
            .get("restricted")
            .map(|s| s == "yes" || s == "true" || s == "1")
            .unwrap_or(false),
        maintainer: vars.get("maintainer").cloned(),
    };
    // Validar
    info.validate()?;
    Ok(info)
}

pub(crate) fn decode_pem_body(pem: &str) -> Result<Vec<u8>> {
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
    const TEST_PEM: &str =
        "-----BEGIN PUBLIC KEY-----\naGVsbG8gd29ybGQ=\n-----END PUBLIC KEY-----\n";
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
        {
            let name = "hello";
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
        assert!(
            dest.is_dir(),
            "project debe reparar symlink apuntando a otro destino"
        );
        assert!(dest.join(".vur_projection_marker").exists());

        let externo = fx._clone_tmp.path().join("externo");
        std::fs::create_dir_all(&externo)?;
        std::os::unix::fs::symlink(&externo, master.join("externo"))?;

        {
            let name = "hello";
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
        run_git(
            &fx.origin,
            &["commit", "--no-gpg-sign", "-am", "tercer paquete"],
        )?;

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

    #[test]
    fn parse_template_handles_multiline_and_quotes() -> Result<()> {
        let content = r#"
# Template de prueba
pkgname=multiline-pkg
version=1.2.3
revision=2
archs="x86_64 aarch64"
distfiles="
    https://example.org/tar1.tar.gz
    https://example.org/tar2.tar.gz
"
makedepends="
    rust
    cargo
    pkg-config
"
depends='
    libssl
    glibc
'
maintainer="Maintainer Name <user@example.org> # not a comment" # comentario real con ' quote
"#;
        let info = parse_template_text(content, "test:template")?;
        assert_eq!(info.pkgname, "multiline-pkg");
        assert_eq!(info.version, "1.2.3");
        assert_eq!(info.revision, 2);
        assert_eq!(info.archs, vec!["x86_64", "aarch64"]);
        assert_eq!(
            info.distfiles,
            vec![
                "https://example.org/tar1.tar.gz",
                "https://example.org/tar2.tar.gz"
            ]
        );
        assert_eq!(info.makedepends, vec!["rust", "cargo", "pkg-config"]);
        assert_eq!(info.depends, vec!["libssl", "glibc"]);
        assert_eq!(
            info.maintainer.as_deref(),
            Some("Maintainer Name <user@example.org> # not a comment")
        );
        Ok(())
    }

    #[test]
    fn parse_template_unclosed_quote_at_eof_does_not_hang() -> Result<()> {
        let content =
            "pkgname=broken\nversion=0.1.0\nrevision=1\nshort_desc=\"unclosed quote at eof";
        let info = parse_template_text(content, "test:template")?;
        assert_eq!(info.pkgname, "broken");
        assert_eq!(info.version, "0.1.0");
        Ok(())
    }

    #[test]
    fn is_safe_git_url_validates_and_rejects_dangerous_transports() {
        assert!(is_safe_git_url(
            "https://github.com/void-linux/void-packages.git"
        ));
        assert!(is_safe_git_url("http://git.example.org/repo.git"));
        assert!(is_safe_git_url("git://example.org/repo.git"));
        assert!(is_safe_git_url("ssh://git@example.org/repo.git"));
        assert!(is_safe_git_url("git@github.com:user/repo.git"));
        assert!(is_safe_git_url("file:///local/repo.git"));

        assert!(!is_safe_git_url("--upload-pack=touch /tmp/pwn"));
        assert!(!is_safe_git_url("-u"));
        assert!(!is_safe_git_url("ext::sh -c evil%G"));
        assert!(!is_safe_git_url(
            "https://example.org/repo.git\n--upload-pack=evil"
        ));
        assert!(!is_safe_git_url(""));
        assert!(!is_safe_git_url("   "));
    }

    #[test]
    fn parse_template_extracts_subpackages_and_preserves_parent_vars() -> Result<()> {
        let content = r#"
# Template con subpaquetes
pkgname=myproject
version=2.5.0
revision=3
archs="x86_64 aarch64"
short_desc="My awesome parent project"
depends="glibc openssl"
checksum="SKIP"

do_build() {
    cargo build --release
}

myproject-devel_package() {
    short_desc="My awesome development files"
    depends="${sourcepkg}>=${version}_${revision} headers"
}

myproject-doc_package() {
    short_desc="My awesome documentation"
}
"#;
        let info = parse_template_text(content, "test:subpkgs")?;
        assert_eq!(info.pkgname, "myproject");
        assert_eq!(info.version, "2.5.0");
        assert_eq!(info.revision, 3);
        assert_eq!(info.depends, vec!["glibc", "openssl"]);
        assert_eq!(info.subpackages.len(), 2);

        let devel = info
            .subpackages
            .iter()
            .find(|s| s.pkgname == "myproject-devel")
            .unwrap();
        assert_eq!(
            devel.short_desc.as_deref(),
            Some("My awesome development files")
        );
        assert_eq!(devel.depends, vec!["myproject>=2.5.0_3", "headers"]);

        let doc = info
            .subpackages
            .iter()
            .find(|s| s.pkgname == "myproject-doc")
            .unwrap();
        assert_eq!(doc.short_desc.as_deref(), Some("My awesome documentation"));
        assert!(doc.depends.is_empty());

        Ok(())
    }

    #[test]
    fn skipped_index_warning_mentions_kind_location_and_cause() {
        let msg = skipped_index_warning(
            "template",
            "mi-repo:srcpkgs/foo/template",
            &anyhow::anyhow!("pkgname ausente"),
        );
        assert!(
            msg.contains("template"),
            "el aviso debe nombrar qué se ignoró: {msg}"
        );
        assert!(
            msg.contains("mi-repo:srcpkgs/foo/template"),
            "el aviso debe ubicar el origen: {msg}"
        );
        assert!(
            msg.contains("pkgname ausente"),
            "el aviso debe explicar la causa: {msg}"
        );
    }

    const BROKEN_VURINFO: &str = r#"{"format_version":1,"pkgname":"broken","version":"1.0","revision":0,"archs":["x86_64"]}"#;

    #[test]
    fn load_index_ignora_vurinfo_invalido_sin_abortar() -> Result<()> {
        let fx = setup_repo(&[])?;
        let broken_dir = fx.origin.join("srcpkgs/broken");
        std::fs::create_dir_all(&broken_dir)?;
        std::fs::write(broken_dir.join(".VURINFO"), BROKEN_VURINFO)?;
        run_git(&fx.origin, &["add", "."])?;
        run_git(&fx.origin, &["commit", "--no-gpg-sign", "-m", "broken"])?;

        fx.repo.ensure_cloned()?;
        let (_cache_dir, mut cache) = fresh_cache()?;
        let idx = fx.repo.load_index(&mut cache, None)?;
        assert!(
            idx.iter().any(|i| i.pkgname == "hello"),
            "el paquete válido debe seguir cargando"
        );
        assert!(
            !idx.iter().any(|i| i.pkgname == "broken"),
            "el .VURINFO inválido (revision 0) debe ignorarse con aviso, no abortar"
        );
        Ok(())
    }

    #[test]
    fn lista_nivel_extra_para_categorias() -> Result<()> {
        // A6: srcpkgs/<cat>/<pkg> se descubre como "cat/pkg"; la categoría
        // suelta no aparece como paquete fantasma.
        let origin_tmp = tempfile::tempdir()?;
        let origin = origin_tmp.path().join("origin");
        std::fs::create_dir_all(origin.join("srcpkgs/cat/nested"))?;
        std::fs::write(
            origin.join("srcpkgs/cat/nested/template"),
            "pkgname=nested\n",
        )?;
        std::fs::create_dir_all(origin.join("srcpkgs/solo"))?;
        std::fs::write(origin.join("srcpkgs/solo/template"), "pkgname=solo\n")?;
        run_git(&origin, &["init"])?;
        run_git(&origin, &["config", "user.email", "test@vary.local"])?;
        run_git(&origin, &["config", "user.name", "Vary Test"])?;
        run_git(&origin, &["add", "."])?;
        run_git(&origin, &["commit", "--no-gpg-sign", "-m", "init"])?;
        run_git(&origin, &["branch", "-M", "main"])?;

        let clone_tmp = tempfile::tempdir()?;
        let repo = VurRepo {
            name: "monorepo".into(),
            path: clone_tmp.path().join("monorepo"),
            entry: RepoEntry {
                url: format!("file://{}/", origin.display()),
                branch: Some("main".into()),
                ..Default::default()
            },
            git_bin: "git".to_string(),
        };
        repo.ensure_cloned()?;
        assert_eq!(
            repo.list_packages()?,
            vec!["cat/nested".to_string(), "solo".to_string()]
        );
        let (_cache_dir, mut cache) = fresh_cache()?;
        let idx = repo.load_index(&mut cache, None)?;
        assert!(idx.iter().any(|i| i.pkgname == "nested"));
        assert!(idx.iter().any(|i| i.pkgname == "solo"));
        Ok(())
    }
}
