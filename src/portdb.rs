//! PortDb — índice del árbol de fuentes (`void-packages/srcpkgs`).
//!
//! ITEM 2 de `docs/vary2/ORDEN-v2.md`: responder "qué versión y qué opciones
//! tiene este paquete en el árbol de fuentes" SIN lanzar `xbps-src show` por
//! paquete (miles de invocaciones = minutos).
//!
//! Diseño (Ponytail): sin SQL, sin dependencias nuevas. Un `BTreeMap` en
//! memoria + JSON en el caché, reusando el patrón de `src/cache.rs`.
//!
//! - **Índice frío**: walk de `srcpkgs/` + parser LÉXICO de los templates.
//!   Los templates son bash, pero las variables que interesan
//!   (`version`, `revision`, `build_options`) son literales en la enorme
//!   mayoría de los casos; los que no (versión o revisión no resueltas
//!   estáticamente) se marcan `needs_dynamic`.
//! - **Índice incremental**: el clon es shallow (`--depth 1`), así que NO
//!   hay `git diff OLD NEW` (el SHA viejo no es alcanzable). Lo correcto es
//!   `git diff-tree --no-commit-id --name-only -r <nuevo_sha>`: el diff de UN
//!   commit, que sí funciona con clon shallow.
//! - **helpers_hash**: hash global de `common/build-style/` + `common/build-helper/`.
//!   Si cambia, TODAS las entradas dependen de él -> reindex completo.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Entrada del índice para un paquete del árbol de fuentes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct PortEntry {
    /// Nombre del directorio en `srcpkgs/` (puede ser un alias `-devel`).
    pub name: String,
    /// Paquete fuente real (destino del symlink): a él se compila.
    pub sourcepkg: String,
    pub version: String,
    pub revision: u32,
    /// Universo de opciones declaradas por el template.
    pub build_options: Vec<String>,
    /// Opciones activas por defecto en el template.
    pub build_options_default: Vec<String>,
    /// sha256 del template.
    pub template_sha256: String,
    /// El parser estático no pudo resolver la versión/revisión: requiere
    /// `xbps-src show` (o revisión manual).
    pub needs_dynamic: bool,
}

impl PortEntry {
    /// `version_revision` (formato de xbps).
    pub fn pkgver(&self) -> String {
        format!("{}_{}", self.version, self.revision)
    }
}

/// Índice del árbol de fuentes.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PortDb {
    /// Entradas por nombre (directorio de `srcpkgs/`).
    pub entries: BTreeMap<String, PortEntry>,
    /// Hash global de los helpers: si cambia, reindex completo.
    pub helpers_hash: String,
    /// Commit del clon sobre el que se construyó el índice.
    pub void_commit: Option<String>,
    /// `SONAME` → paquete que lo provee (de `common/shlibs`).
    pub shlibs: BTreeMap<String, String>,
}

impl PortDb {
    /// Entrada de un paquete (por su nombre en `srcpkgs/`).
    // ITEM 3/4: consumidores de esta API (normalización de subpaquetes y
    // detección de SONAME). Patrón `#[allow(dead_code)]` ya usado en db.rs.
    #[allow(dead_code)]
    pub fn get(&self, pkg: &str) -> Option<&PortEntry> {
        self.entries.get(pkg)
    }

    /// Paquete fuente real de un nombre (normaliza subpaquetes `-devel`).
    #[allow(dead_code)]
    pub fn sourcepkg(&self, pkg: &str) -> Option<&str> {
        self.entries.get(pkg).map(|e| e.sourcepkg.as_str())
    }

    /// `pkgname-version_revision` en el árbol de fuentes: MISMO formato que
    /// `VurInfo::pkgver()` y que la versión guardada en la DB de instalados
    /// (así la auditoría puede compararlos sin conversiones).
    pub fn pkgver_of(&self, pkg: &str) -> Option<String> {
        self.entries.get(pkg).and_then(|e| {
            // Entrada `needs_dynamic` sin versión resuelta: NO hay dato, y
            // sin dato no hay señal (un "_0" sintético dispararía un
            // NewVersion falso).
            if e.version.is_empty() {
                return None;
            }
            Some(format!("{}-{}", e.name, e.pkgver()))
        })
    }

    /// Paquete que provee un SONAME (`common/shlibs`).
    ///
    /// El índice guarda el pkgver completo (`pkg-ver_rev`); aquí se devuelve
    /// el NOMBRE del proveedor (`pkg`).
    #[allow(dead_code)]
    pub fn providers(&self, soname: &str) -> Option<&str> {
        self.shlibs
            .get(soname)
            .map(|pkgver| match pkgver.rsplit_once('-') {
                Some((pkg, _)) => pkg,
                None => pkgver.as_str(),
            })
    }
}

/// Campos que extraemos de un template por análisis léxico.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct StaticTemplate {
    pkgname: String,
    version: String,
    revision: u32,
    build_options: Vec<String>,
    build_options_default: Vec<String>,
}

/// ¿El valor es un literal (sin evaluación de shell)?
fn is_literal(value: &str) -> bool {
    !value.contains("$(") && !value.contains('`') && !value.contains("${")
}

/// Extrae `key=value` de una línea de template. `None` si no es una
/// asignación simple.
fn assignment(line: &str) -> Option<(String, String)> {
    let line = line.trim();
    let eq = line.find('=')?;
    let key = line[..eq].trim();
    if key.is_empty()
        || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        || key.chars().next().is_some_and(|c| c.is_ascii_digit())
    {
        return None;
    }
    let mut value = line[eq + 1..].trim().to_string();
    if value.len() >= 2
        && ((value.starts_with('"') && value.ends_with('"'))
            || (value.starts_with('\'') && value.ends_with('\'')))
    {
        value = value[1..value.len() - 1].to_string();
    }
    Some((key.to_string(), value))
}

/// Parser LÉXICO del template (no ejecuta bash).
///
/// `None` cuando `version`/`revision` no son literales: entonces el paquete
/// necesita `xbps-src show` (marcado `needs_dynamic`: versión o revisión
/// no resueltas estáticamente).
fn parse_template_static(content: &str) -> Option<StaticTemplate> {
    let mut out = StaticTemplate::default();
    let mut lines: Vec<String> = Vec::new();
    let mut pending = String::new();
    for raw in content.lines() {
        let line = raw.trim_end();
        if line.trim_end().ends_with('\\') {
            pending.push_str(line.trim_end().trim_end_matches('\\').trim());
            pending.push(' ');
            continue;
        }
        pending.push_str(line.trim());
        lines.push(std::mem::take(&mut pending));
    }
    if !pending.is_empty() {
        lines.push(pending);
    }

    for line in lines {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some((key, value)) = assignment(trimmed) else {
            continue;
        };
        match key.as_str() {
            "pkgname" => out.pkgname = value,
            "version" => {
                if is_literal(&value) {
                    out.version = value;
                } else {
                    return None;
                }
            }
            "revision" => {
                if !is_literal(&value) {
                    return None;
                }
                out.revision = value.parse().ok()?;
            }
            "build_options" => {
                if is_literal(&value) {
                    out.build_options = value.split_whitespace().map(str::to_string).collect();
                }
            }
            "build_options_default" => {
                if is_literal(&value) {
                    out.build_options_default =
                        value.split_whitespace().map(str::to_string).collect();
                }
            }
            _ => {}
        }
    }
    if out.version.is_empty() {
        return None;
    }
    Some(out)
}

/// sha256 de un fichero (`sha256:<hex>`).
fn file_sha256(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    Some(format!("sha256:{:x}", Sha256::digest(bytes)))
}

/// Hash global de los helpers (`build-style` + `build-helper`): si cambia,
/// todas las entradas dependen de él.
pub fn helpers_hash(checkout: &Path) -> String {
    let mut hasher = Sha256::new();
    for dir in ["build-style", "build-helper"] {
        let path = checkout.join("common").join(dir);
        let Ok(entries) = std::fs::read_dir(&path) else {
            continue;
        };
        let mut names: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        names.sort();
        for file in names {
            let Ok(bytes) = std::fs::read(&file) else {
                continue;
            };
            hasher.update(file.file_name().unwrap_or_default().as_encoded_bytes());
            hasher.update(bytes);
        }
    }
    format!("sha256:{:x}", hasher.finalize())
}

/// Nombre real del paquete fuente: si `srcpkgs/<name>` es un symlink, su
/// destino; si no, el propio nombre.
fn sourcepkg_of(path: &Path, name: &str) -> String {
    match std::fs::read_link(path) {
        Ok(target) => target
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_else(|| name.to_string()),
        Err(_) => name.to_string(),
    }
}

/// Índice en frío de `srcpkgs/` (parser estático; marca `needs_dynamic`).
pub fn index_checkout(checkout: &Path) -> Result<PortDb> {
    let srcpkgs = checkout.join("srcpkgs");
    let mut entries: BTreeMap<String, PortEntry> = BTreeMap::new();
    let dir =
        std::fs::read_dir(&srcpkgs).with_context(|| format!("leyendo {}", srcpkgs.display()))?;
    for entry in dir.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = entry.path();
        let template = path.join("template");
        let Some(hash) = file_sha256(&template) else {
            continue; // no es un paquete (sin template)
        };
        let sourcepkg = sourcepkg_of(&path, &name);
        let content = std::fs::read_to_string(&template).unwrap_or_default();
        let entry_data = match parse_template_static(&content) {
            Some(st) => PortEntry {
                name: name.clone(),
                sourcepkg,
                version: st.version,
                revision: st.revision,
                build_options: st.build_options,
                build_options_default: st.build_options_default,
                template_sha256: hash,
                needs_dynamic: false,
            },
            None => PortEntry {
                name: name.clone(),
                sourcepkg,
                version: String::new(),
                revision: 0,
                build_options: Vec::new(),
                build_options_default: Vec::new(),
                template_sha256: hash,
                needs_dynamic: true,
            },
        };
        entries.insert(name, entry_data);
    }
    Ok(PortDb {
        entries,
        helpers_hash: helpers_hash(checkout),
        void_commit: None,
        shlibs: parse_shlibs(&checkout.join("common").join("shlibs")),
    })
}

/// `common/shlibs`: `SONAME pkg-version [arch]` → `SONAME` → paquete.
fn parse_shlibs(path: &Path) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let Ok(content) = std::fs::read_to_string(path) else {
        return out;
    };
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut fields = line.split_whitespace();
        let (Some(soname), Some(pkgver)) = (fields.next(), fields.next()) else {
            continue;
        };
        // Se guarda el PKGVER completo (`pkg-version_revision`): la auditoría
        // de bumps compara nombre+versión+revisión contra el pin. Guardar solo
        // el nombre hacía que la comparación nunca tuviese datos.
        out.entry(soname.to_string()).or_insert(pkgver.to_string());
    }
    out
}

/// Resultado de una actualización incremental.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UpdateStats {
    /// Commit nuevo (si hubo cambio).
    pub new_commit: Option<String>,
    /// Paquetes reindexados.
    pub reindexed: usize,
    /// Hubo que reindexar TODO (cambió `helpers_hash`).
    pub full_reindex: bool,
    /// Entradas del índice en total.
    pub entries: usize,
    /// Entradas que el parser estático no pudo resolver (versión calculada
    /// por shell): se resuelven vía `xbps-src show` en la fase de upgrade.
    pub needs_dynamic: usize,
}

/// Actualización incremental tras `git pull --ff-only`.
///
/// Shallow-safe: `git diff-tree` de UN commit (no hay rango OLD..NEW).
pub fn update_incremental(checkout: &Path, git_bin: &str, db: &mut PortDb) -> Result<UpdateStats> {
    let mut stats = UpdateStats::default();
    let head = git_head(checkout, git_bin)?;
    let Some(head) = head else {
        return Ok(stats);
    };
    if db.void_commit.as_deref() == Some(head.as_str()) {
        return Ok(stats); // nada nuevo
    }
    stats.new_commit = Some(head.clone());

    let helpers = helpers_hash(checkout);
    if !db.helpers_hash.is_empty() && db.helpers_hash != helpers {
        // Cambió un helper global: TODAS las entradas dependen de él.
        let fresh = index_checkout(checkout)?;
        db.entries = fresh.entries;
        db.helpers_hash = helpers;
        db.shlibs = fresh.shlibs;
        db.void_commit = Some(head);
        stats.full_reindex = true;
        stats.reindexed = db.entries.len();
        return Ok(stats);
    }

    let touched = git_changed_paths(checkout, git_bin, &head)?;
    let mut names: Vec<String> = Vec::new();
    for path in &touched {
        let mut parts = path.split('/');
        if parts.next() != Some("srcpkgs") {
            continue;
        }
        if let Some(name) = parts.next() {
            if !name.is_empty() && !names.contains(&name.to_string()) {
                names.push(name.to_string());
            }
        }
    }
    // El mapa de SONAMEs del árbol también es parte del índice: si el commit
    // lo toca, se refresca (si no, el mapa queda congelado y los bumps
    // nuevos no se ven hasta que cambie un helper).
    if touched.iter().any(|p| p == "common/shlibs") {
        db.shlibs = parse_shlibs(&checkout.join("common").join("shlibs"));
    }
    for name in names {
        let dir = checkout.join("srcpkgs").join(&name);
        let template = dir.join("template");
        let Some(hash) = file_sha256(&template) else {
            db.entries.remove(&name); // el paquete ya no está
            stats.reindexed += 1;
            continue;
        };
        let sourcepkg = sourcepkg_of(&dir, &name);
        let content = std::fs::read_to_string(&template).unwrap_or_default();
        let data = match parse_template_static(&content) {
            Some(st) => PortEntry {
                name: name.clone(),
                sourcepkg,
                version: st.version,
                revision: st.revision,
                build_options: st.build_options,
                build_options_default: st.build_options_default,
                template_sha256: hash,
                needs_dynamic: false,
            },
            None => PortEntry {
                name: name.clone(),
                sourcepkg,
                version: String::new(),
                revision: 0,
                build_options: Vec::new(),
                build_options_default: Vec::new(),
                template_sha256: hash,
                needs_dynamic: true,
            },
        };
        db.entries.insert(name, data);
        stats.reindexed += 1;
    }
    db.helpers_hash = helpers;
    db.void_commit = Some(head);
    Ok(stats)
}

/// `git rev-parse HEAD` del clon.
fn git_head(checkout: &Path, git_bin: &str) -> Result<Option<String>> {
    let out = std::process::Command::new(git_bin)
        .arg("-C")
        .arg(checkout)
        .args(["rev-parse", "HEAD"])
        .output()
        .with_context(|| format!("no se pudo invocar {git_bin} rev-parse"))?;
    if !out.status.success() {
        return Ok(None);
    }
    Ok(Some(
        String::from_utf8_lossy(&out.stdout).trim().to_string(),
    ))
}

/// Rutas tocadas por el commit `sha` (`git diff-tree`, shallow-safe).
fn git_changed_paths(checkout: &Path, git_bin: &str, sha: &str) -> Result<Vec<String>> {
    let out = std::process::Command::new(git_bin)
        .arg("-C")
        .arg(checkout)
        .args(["diff-tree", "--no-commit-id", "--name-only", "-r", sha])
        .output()
        .with_context(|| format!("no se pudo invocar {git_bin} diff-tree"))?;
    if !out.status.success() {
        return Ok(Vec::new());
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::to_string)
        .collect())
}

/// Tabla de virtuals: `virtual` → paquete real.
///
/// Fuentes (en este orden, el segundo pisa al primero):
/// - `etc/defaults.virtual` del clon (base de Void);
/// - `etc/virtual` (override local: "EDIT etc/virtual INSTEAD").
pub fn parse_virtuals(path: &Path) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let Ok(content) = std::fs::read_to_string(path) else {
        return out;
    };
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut fields = line.split_whitespace();
        let (Some(virtual_name), Some(real)) = (fields.next(), fields.next()) else {
            continue;
        };
        out.insert(virtual_name.to_string(), real.to_string());
    }
    out
}

/// Tabla de virtuals efectiva (base + override local).
pub fn virtuals(checkout: &Path) -> BTreeMap<String, String> {
    let mut out = parse_virtuals(&checkout.join("etc").join("defaults.virtual"));
    out.extend(parse_virtuals(&checkout.join("etc").join("virtual")));
    out
}

/// Carga el índice del caché (JSON). Si no existe, índice vacío.
pub fn load(path: impl Into<PathBuf>) -> Result<PortDb> {
    let path = path.into();
    match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text)
            .with_context(|| format!("índice de fuentes corrupto: {}", path.display())),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(PortDb::default()),
        Err(err) => Err(err).with_context(|| format!("leyendo {}", path.display())),
    }
}

/// Guarda el índice (escritura atómica: tmp + rename en el mismo dir).
pub fn save(db: &PortDb, path: impl Into<PathBuf>) -> Result<()> {
    let path = path.into();
    if let Some(parent) = path.parent() {
        crate::util::ensure_private_dir(parent)
            .with_context(|| format!("creando {}", parent.display()))?;
    }
    let json = serde_json::to_vec_pretty(db)?;
    let mut temp = tempfile::NamedTempFile::new_in(path.parent().unwrap_or_else(|| Path::new(".")))
        .with_context(|| format!("creando temporal para {}", path.display()))?;
    use std::io::Write;
    temp.write_all(&json)?;
    temp.flush()?;
    temp.as_file().sync_all()?;
    temp.persist(&path)
        .map_err(|e| e.error)
        .with_context(|| format!("renombrando a {}", path.display()))?;
    Ok(())
}

/// `git pull --ff-only` del clon de void-packages (si existe).
///
/// El clon es shallow (`--depth 1`): el pull trae el commit nuevo y ya está.
pub fn pull_checkout(checkout: &Path, git_bin: &str) -> Result<Option<String>> {
    if !checkout.join(".git").exists() {
        return Ok(None);
    }
    let out = std::process::Command::new(git_bin)
        .arg("-C")
        .arg(checkout)
        .args(["pull", "--ff-only", "--quiet"])
        .output()
        .with_context(|| format!("no se pudo invocar {git_bin} pull"))?;
    if !out.status.success() {
        anyhow::bail!(
            "git pull --ff-only falló: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    git_head(checkout, git_bin)
}

/// Sincroniza el clon de void-packages y actualiza el índice de fuentes.
///
/// El `pull` es best-effort (sin red no se rompe la operación): el índice se
/// actualiza igualmente con lo que haya en el clon.
pub fn sync_index(config: &crate::config::Config) -> Result<UpdateStats> {
    let checkout = config.void_packages_dir();
    let index_path = config.cache_dir.join("portdb.json");
    if let Err(err) = pull_checkout(&checkout, &config.git_bin) {
        tracing::warn!("no se pudo actualizar void-packages: {err:#}");
    }
    if !checkout.join("srcpkgs").is_dir() {
        return Ok(UpdateStats::default());
    }
    let mut db = load(&index_path)?;
    let mut stats = if db.entries.is_empty() {
        let fresh = index_checkout(&checkout)?;
        let count = fresh.entries.len();
        db = fresh;
        db.void_commit = git_head(&checkout, &config.git_bin)?;
        UpdateStats {
            new_commit: db.void_commit.clone(),
            reindexed: count,
            full_reindex: true,
            ..Default::default()
        }
    } else {
        update_incremental(&checkout, &config.git_bin, &mut db)?
    };
    stats.entries = db.entries.len();
    stats.needs_dynamic = db.entries.values().filter(|e| e.needs_dynamic).count();
    save(&db, &index_path)?;
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn template(pkg: &str, version: &str, revision: u32, opts: &str) -> String {
        format!(
            "pkgname={pkg}\nversion={version}\nrevision={revision}\nbuild_options=\"{opts}\"\nbuild_options_default=\"a\"\n"
        )
    }

    fn git(cwd: &Path, args: &[&str]) {
        let status = Command::new("git")
            .args(args)
            .current_dir(cwd)
            .status()
            .expect("git");
        assert!(status.success(), "git {args:?} falló");
    }

    /// Clon sintético con N templates y un commit.
    fn fixture(dir: &Path) {
        std::fs::create_dir_all(dir.join("srcpkgs")).unwrap();
        std::fs::create_dir_all(dir.join("common/build-style")).unwrap();
        std::fs::write(dir.join("common/build-style/gnu.sh"), "x=1\n").unwrap();
        std::fs::write(dir.join("common/shlibs"), "libc.so.6 glibc-2.41_1\n").unwrap();
        for i in 0..3 {
            std::fs::create_dir_all(dir.join(format!("srcpkgs/pkg{i}"))).unwrap();
            std::fs::write(
                dir.join(format!("srcpkgs/pkg{i}/template")),
                template(&format!("pkg{i}"), "1.0", i as u32 + 1, "a b"),
            )
            .unwrap();
        }
        git(dir, &["init", "-q", "-b", "main"]);
        git(dir, &["config", "user.email", "t@t"]);
        git(dir, &["config", "user.name", "t"]);
        git(dir, &["add", "-A"]);
        git(dir, &["commit", "-qm", "init"]);
    }

    #[test]
    fn indice_frio_extrae_campos() {
        let tmp = tempfile::tempdir().unwrap();
        fixture(tmp.path());
        let db = index_checkout(tmp.path()).unwrap();
        assert_eq!(db.entries.len(), 3);
        let e = db.get("pkg1").unwrap();
        assert_eq!(e.version, "1.0");
        assert_eq!(e.revision, 2);
        assert_eq!(e.build_options, vec!["a", "b"]);
        assert_eq!(e.build_options_default, vec!["a"]);
        assert!(e.template_sha256.starts_with("sha256:"));
        assert!(!e.needs_dynamic);
        assert_eq!(db.providers("libc.so.6"), Some("glibc"));
        // El índice guarda el pkgver completo (lo necesita la auditoría).
        assert_eq!(
            db.shlibs.get("libc.so.6").map(String::as_str),
            Some("glibc-2.41_1")
        );
    }

    #[test]
    fn subpaquete_symlink_apunta_al_fuente() {
        let tmp = tempfile::tempdir().unwrap();
        fixture(tmp.path());
        std::os::unix::fs::symlink("pkg0", tmp.path().join("srcpkgs/pkg0-devel")).unwrap();
        let db = index_checkout(tmp.path()).unwrap();
        assert_eq!(db.sourcepkg("pkg0-devel"), Some("pkg0"));
        assert_eq!(db.get("pkg0-devel").unwrap().version, "1.0");
    }

    #[test]
    fn template_no_literal_marca_needs_dynamic() {
        let tmp = tempfile::tempdir().unwrap();
        fixture(tmp.path());
        std::fs::create_dir_all(tmp.path().join("srcpkgs/dyn")).unwrap();
        std::fs::write(
            tmp.path().join("srcpkgs/dyn/template"),
            "pkgname=dyn\nversion=$(date +%s)\nrevision=1\n",
        )
        .unwrap();
        let db = index_checkout(tmp.path()).unwrap();
        assert!(db.get("dyn").unwrap().needs_dynamic);
    }

    /// Incremental: un commit que toca UN template reindexa EXACTAMENTE uno.
    #[test]
    fn incremental_reindexa_solo_lo_tocado() {
        let tmp = tempfile::tempdir().unwrap();
        fixture(tmp.path());
        let mut db = index_checkout(tmp.path()).unwrap();
        db.void_commit = git_head(tmp.path(), "git").unwrap();
        save(&db, tmp.path().join("portdb.json")).unwrap();

        std::fs::write(
            tmp.path().join("srcpkgs/pkg1/template"),
            template("pkg1", "2.0", 3, "a b c"),
        )
        .unwrap();
        git(tmp.path(), &["add", "-A"]);
        git(tmp.path(), &["commit", "-qm", "touch pkg1"]);

        let mut db = load(tmp.path().join("portdb.json")).unwrap();
        let stats = update_incremental(tmp.path(), "git", &mut db).unwrap();
        assert!(!stats.full_reindex);
        assert_eq!(stats.reindexed, 1, "{stats:?}");
        assert_eq!(db.get("pkg1").unwrap().version, "2.0");
        assert_eq!(db.get("pkg1").unwrap().revision, 3);
        assert_eq!(db.get("pkg1").unwrap().build_options, vec!["a", "b", "c"]);
        // Los demás no se tocan.
        assert_eq!(db.get("pkg0").unwrap().version, "1.0");
    }

    /// Si cambia un helper global -> reindex COMPLETO.
    #[test]
    fn cambio_de_helper_reindexa_todo() {
        let tmp = tempfile::tempdir().unwrap();
        fixture(tmp.path());
        let mut db = index_checkout(tmp.path()).unwrap();
        db.void_commit = git_head(tmp.path(), "git").unwrap();

        std::fs::write(tmp.path().join("common/build-style/gnu.sh"), "x=2\n").unwrap();
        git(tmp.path(), &["add", "-A"]);
        git(tmp.path(), &["commit", "-qm", "helper change"]);

        let stats = update_incremental(tmp.path(), "git", &mut db).unwrap();
        assert!(stats.full_reindex);
        assert_eq!(stats.reindexed, 3);
    }

    #[test]
    fn sin_cambios_no_hay_trabajo() {
        let tmp = tempfile::tempdir().unwrap();
        fixture(tmp.path());
        let mut db = index_checkout(tmp.path()).unwrap();
        db.void_commit = git_head(tmp.path(), "git").unwrap();
        let stats = update_incremental(tmp.path(), "git", &mut db).unwrap();
        assert_eq!(stats.reindexed, 0);
        assert!(stats.new_commit.is_none());
    }

    /// Medición REAL sobre el clon local (necesita
    /// `~/.cache/vary/void-packages`). Ignorado por defecto: requiere el clon
    /// y no debe correr en CI. Ejecutar con
    /// `cargo test -- --ignored portdb` (ver `audit/VALIDATION.md`).
    #[test]
    #[ignore]
    fn mide_indice_frio_e_incremental() {
        let checkout = std::env::var("HOME")
            .map(|h| PathBuf::from(h).join(".cache/vary/void-packages"))
            .unwrap_or_default();
        if !checkout.join("srcpkgs").is_dir() {
            eprintln!("sin clon local en {}: nada que medir", checkout.display());
            return;
        }
        let t0 = std::time::Instant::now();
        let db = index_checkout(&checkout).unwrap();
        let cold = t0.elapsed();
        println!(
            "frío: {} entradas en {:?} ({} dinámicos)",
            db.entries.len(),
            cold,
            db.entries.values().filter(|e| e.needs_dynamic).count()
        );
        let mut db = db;
        let t1 = std::time::Instant::now();
        let stats = update_incremental(&checkout, "git", &mut db).unwrap();
        println!(
            "incremental: {:?} (reindexados: {}, completo: {})",
            t1.elapsed(),
            stats.reindexed,
            stats.full_reindex
        );
    }

    #[test]
    fn virtuals_base_y_override() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("etc")).unwrap();
        std::fs::write(
            tmp.path().join("etc/defaults.virtual"),
            "# comentario\nntp-daemon ntp\n\nfoo bar\n",
        )
        .unwrap();
        let v = parse_virtuals(&tmp.path().join("etc/defaults.virtual"));
        assert_eq!(v.get("ntp-daemon"), Some(&"ntp".to_string()));
        assert_eq!(v.get("foo"), Some(&"bar".to_string()));
        // El override local pisa a la base.
        std::fs::write(tmp.path().join("etc/virtual"), "ntp-daemon chrony\n").unwrap();
        let all = virtuals(tmp.path());
        assert_eq!(all.get("ntp-daemon"), Some(&"chrony".to_string()));
        assert_eq!(all.get("foo"), Some(&"bar".to_string()));
    }

    /// El mapa de `common/shlibs` guarda el PKGVER completo: la auditoría de
    /// bumps compara contra él. Con solo el nombre del proveedor, la
    /// comparación no tenía datos y la detección NUNCA disparaba (bug
    /// encontrado por revisión, no por CI: los tests feeds pkgvers
    /// completos).
    #[test]
    fn shlibs_guarda_pkgver_y_la_auditoria_lo_detecta() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("shlibs");
        // El árbol exige una versión MAYOR que la pineada al instalar.
        std::fs::write(&path, "libpoppler.so.145 poppler-25.02.0_1\n").unwrap();
        let map = parse_shlibs(&path);
        assert_eq!(
            map.get("libpoppler.so.145").map(String::as_str),
            Some("poppler-25.02.0_1")
        );
        // El nombre del proveedor sigue disponible para consulta.
        let db = PortDb {
            shlibs: map.clone(),
            ..PortDb::default()
        };
        assert_eq!(db.providers("libpoppler.so.145"), Some("poppler"));
        // Y el cruce con la auditoría de bumps encuentra el cambio.
        let pinned = crate::soname::SonamePins::from([(
            "libpoppler.so.145".to_string(),
            "poppler-24.08.0_1".to_string(),
        )]);
        let bumps = crate::soname::source_tree_bumps(&pinned, &map);
        assert_eq!(bumps.len(), 1, "{bumps:?}");
        assert_eq!(bumps[0].shlib, "libpoppler.so.145");
    }

    /// Entrada sin versión resuelta (`needs_dynamic`): sin dato, sin señal
    /// (un pkgver sintético dispararía un NewVersion falso).
    #[test]
    fn pkgver_de_entrada_dinamica_es_none() {
        let db = PortDb {
            entries: [(
                "dyn".to_string(),
                PortEntry {
                    name: "dyn".to_string(),
                    sourcepkg: "dyn".to_string(),
                    version: String::new(),
                    revision: 0,
                    needs_dynamic: true,
                    ..PortEntry::default()
                },
            )]
            .into_iter()
            .collect(),
            ..PortDb::default()
        };
        assert_eq!(db.pkgver_of("dyn"), None);
        assert_eq!(db.pkgver_of("noexiste"), None);
    }

    #[test]
    fn persistencia_ida_y_vuelta() {
        let tmp = tempfile::tempdir().unwrap();
        fixture(tmp.path());
        let db = index_checkout(tmp.path()).unwrap();
        let path = tmp.path().join("portdb.json");
        save(&db, &path).unwrap();
        let back = load(&path).unwrap();
        assert_eq!(back.entries.len(), db.entries.len());
        assert_eq!(back.get("pkg2").unwrap().version, "1.0");
    }
}
