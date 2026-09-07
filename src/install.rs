use crate::bootstrap;
use crate::cache::CacheIndex;
use crate::config::Config;
use crate::db::{InstallType, InstalledDb};
use crate::metadata::VurInfo;
use crate::reposconf::ReposConf;
use crate::resolver::{
    dep_name, Action, BinarySource, CandidateOrder, PackageSource, ResolveOptions,
};
use crate::util::confirm;
use crate::vur_client::VurRepo;
use crate::xbps;

use anyhow::{bail, Context, Result};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

struct VurSource {
    official_exists_fn: Box<dyn Fn(&str) -> bool>,
    vur_map: HashMap<String, (String, VurInfo)>,
    /// T-010: TODOS los candidatos por nombre (multi-repo), sin filtrar arch.
    vur_all: HashMap<String, Vec<(String, VurInfo)>>,
    provides_map: HashMap<String, Vec<(String, VurInfo)>>,
    binary_check: HashMap<String, bool>,
    arch: String,
    priority: HashMap<String, i64>,
}

impl PackageSource for VurSource {
    fn official_exists(&self, name: &str) -> bool {
        (self.official_exists_fn)(name)
    }

    fn vur_lookup(&self, name: &str) -> Option<(String, VurInfo)> {
        // Filter by arch (T-008: vale exacta, "all" o "noarch").
        if let Some((repo, info)) = self.vur_map.get(name) {
            if !crate::metadata::arch_supports(&info.archs, &self.arch) {
                return None;
            }
            return Some((repo.clone(), info.clone()));
        }
        None
    }

    fn vur_lookup_any_arch(&self, name: &str) -> Option<(String, VurInfo)> {
        // Raw lookup: no filtra por arquitectura (sirve para detectar
        // paquetes existentes pero incompatibles con la arch actual).
        self.vur_map
            .get(name)
            .map(|(repo, info)| (repo.clone(), info.clone()))
    }

    fn vur_lookup_all(&self, name: &str) -> Vec<(String, VurInfo)> {
        self.vur_all.get(name).cloned().unwrap_or_default()
    }

    fn repo_priority(&self, repo: &str) -> i64 {
        self.priority.get(repo).copied().unwrap_or(100)
    }

    fn vur_lookup_provides(&self, virtual_name: &str) -> Option<(String, VurInfo)> {
        let candidates = self.provides_map.get(virtual_name)?;
        // Choose best by priority (lowest) then return first
        let mut best: Option<(i64, &(String, VurInfo))> = None;
        for cand in candidates {
            let prio = self.priority.get(&cand.0).copied().unwrap_or(100);
            if crate::metadata::arch_supports(&cand.1.archs, &self.arch) {
                match best {
                    None => best = Some((prio, cand)),
                    Some((bp, _)) if prio < bp => best = Some((prio, cand)),
                    _ => {}
                }
            }
        }
        best.map(|(_, (r, i))| (r.clone(), i.clone()))
    }

    fn vul_binary_available(&self, repo: &str, info: &VurInfo, arch: &str) -> bool {
        if !crate::metadata::arch_supports(&info.archs, arch) {
            return false;
        }
        *self
            .binary_check
            .get(&format!("{}:{}", repo, info.pkgname))
            .unwrap_or(&false)
    }

    fn arch(&self) -> String {
        self.arch.clone()
    }
}

/// Mapas del índice fusionado (paso 4 de install/print; ver
/// [`load_source_maps`]).
struct SourceMaps {
    vur_map: HashMap<String, (String, VurInfo)>,
    vur_all: HashMap<String, Vec<(String, VurInfo)>>,
    provides_map: HashMap<String, Vec<(String, VurInfo)>>,
    binary_check: HashMap<String, bool>,
    priority_map: HashMap<String, i64>,
    vup_binary_urls: HashMap<String, String>,
}

/// Construye un [`VurRepo`] sin tocar red ni disco (el ensure/fetch lo hace
/// cada flujo por separado: install con red, print nunca).
fn make_repo(name: &str, entry: &crate::reposconf::RepoEntry, config: &Config) -> VurRepo {
    VurRepo {
        name: name.to_string(),
        path: config.vurs_dir().join(name),
        entry: entry.clone(),
        git_bin: config.git_bin.clone(),
    }
}

/// Carga los índices de `repos` en memoria (paso 4, compartido por install y
/// print; movido verbatim).
///
/// Nota P0-4: el índice VUP SÍ puede descargarse aquí en ambos flujos: es
/// lectura de red sin elevación que solo escribe su caché (lo mismo que hace
/// `vary -Sy` rutinariamente; sin comando standalone que lo refresque, el
/// print quedaría inservible la mitad del tiempo). Lo congelado de verdad es:
/// sin ensure/fetch de clones (para eso existe `-Sy`), sin guardar
/// CacheIndex, sin lock/review/DB y sin elevación.
fn load_source_maps(
    repos: &[VurRepo],
    repos_conf: &ReposConf,
    cache: &mut CacheIndex,
    arch: &str,
    config: &Config,
) -> Result<SourceMaps> {
    let mut vur_map: HashMap<String, (String, VurInfo)> = HashMap::new();
    // T-010: multi-mapa con todos los candidatos (el resolver ordena).
    let mut vur_all: HashMap<String, Vec<(String, VurInfo)>> = HashMap::new();
    let mut provides_map: HashMap<String, Vec<(String, VurInfo)>> = HashMap::new();
    let mut binary_check: HashMap<String, bool> = HashMap::new();
    let mut priority_map: HashMap<String, i64> = HashMap::new();
    // URL binaria por paquete para repos con índice VUP ("repo:pkg" -> URL).
    let mut vup_binary_urls: HashMap<String, String> = HashMap::new();

    for repo in repos {
        let entry = repos_conf.vur.get(&repo.name).ok_or_else(|| {
            anyhow::anyhow!("repositorio '{}' sin entrada en repos.conf", repo.name)
        })?;
        priority_map.insert(repo.name.clone(), entry.priority_or(100));
        let has_binary = entry.has_binary();
        let infos = match repo.load_index(cache, None) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("failed to load index for '{}': {}", repo.name, e);
                Vec::new()
            }
        };
        for info in infos {
            // Record provides
            for prov in &info.provides {
                let key = dep_name(prov).to_string();
                provides_map
                    .entry(key)
                    .or_default()
                    .push((repo.name.clone(), info.clone()));
            }
            // Main map (first wins by priority since sorted)
            vur_map
                .entry(info.pkgname.clone())
                .or_insert((repo.name.clone(), info.clone()));
            // T-010: todos los candidatos (el resolver ordena por clase,
            // versión y prioridad).
            vur_all
                .entry(info.pkgname.clone())
                .or_default()
                .push((repo.name.clone(), info.clone()));
            for sub in &info.subpackages {
                vur_map
                    .entry(sub.pkgname.clone())
                    .or_insert((repo.name.clone(), info.clone()));
                vur_all
                    .entry(sub.pkgname.clone())
                    .or_default()
                    .push((repo.name.clone(), info.clone()));
            }
            binary_check.insert(format!("{}:{}", repo.name, info.pkgname), has_binary);
            for sub in &info.subpackages {
                binary_check.insert(format!("{}:{}", repo.name, sub.pkgname), has_binary);
            }
        }

        // 4b. Adaptador VUP Fase 1: índice remoto estilo index.json.
        // Los repos VUP no traen .VURINFO (layout srcpkgs/<cat>/<pkg>), así que
        // el load_index normal los omite; aquí se inyectan sus paquetes como
        // entradas binarias para la arquitectura actual.
        if entry.has_vup_index() {
            if let Some(index_url) = entry
                .index_url
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                let cache_path = config.cache_dir.join(format!(
                    "vup-index-{}.json",
                    crate::vup_index::sanitize_repo_name(&repo.name)
                ));
                match crate::vup_index::fetch_index(
                    &config.curl_bin,
                    index_url,
                    &cache_path,
                    config.ttl_cache_seconds,
                ) {
                    Ok(idx) => {
                        for (pkgname, vpkg) in &idx.packages {
                            let Some((info, repo_url)) =
                                crate::vup_index::to_vur_info(pkgname, vpkg, arch)
                            else {
                                continue;
                            };
                            vur_map
                                .entry(pkgname.clone())
                                .or_insert((repo.name.clone(), info.clone()));
                            // T-010: el candidato binario VUP también ordena.
                            vur_all
                                .entry(pkgname.clone())
                                .or_default()
                                .push((repo.name.clone(), info));
                            binary_check.insert(format!("{}:{}", repo.name, pkgname), true);
                            vup_binary_urls
                                .entry(format!("{}:{}", repo.name, pkgname))
                                .or_insert(repo_url);
                        }
                    }
                    Err(e) => tracing::warn!("índice VUP '{}' no disponible: {:#}", repo.name, e),
                }
            }
        }
    }

    Ok(SourceMaps {
        vur_map,
        vur_all,
        provides_map,
        binary_check,
        priority_map,
        vup_binary_urls,
    })
}

/// Ensambla `VurSource` + `ResolveOptions` (paso 5, compartido por install y
/// print; movido verbatim).
fn assemble_source(
    maps: SourceMaps,
    arch: &str,
    binpkgs_root: &str,
    config: &Config,
) -> (VurSource, ResolveOptions) {
    // E2 fast-path: snapshot de oficiales al inicio del sync en UNA sola
    // consulta masiva. Se invalida al terminar (se dropea con esta función)
    // y no refleja deps instaladas durante la transacción. Miss => la
    // confirmación escalar de abajo preserva la corrección (virtuales,
    // falsos negativos de formato); el bulk solo acelera, nunca decide.
    let bulk: Option<std::sync::Arc<HashSet<String>>> =
        crate::xbps::bulk_official_names().map(std::sync::Arc::new);
    match &bulk {
        Some(s) => tracing::debug!("bulk oficial: {} paquetes", s.len()),
        None => tracing::debug!("bulk oficial no disponible; path escalar"),
    }
    let binpkgs_root = binpkgs_root.to_string();
    let source = VurSource {
        official_exists_fn: Box::new(move |n| {
            if bulk.as_ref().is_some_and(|s| s.contains(n)) {
                return true;
            }
            official_exists_remote(n, &binpkgs_root)
        }),
        vur_map: maps.vur_map,
        vur_all: maps.vur_all,
        provides_map: maps.provides_map,
        binary_check: maps.binary_check,
        arch: arch.to_string(),
        priority: maps.priority_map,
    };

    let opts = ResolveOptions {
        prefer_binary: config.prefer_binary && !config.force_rebuild,
        force_build: config.force_build || config.force_rebuild,
        // T-010: force_rebuild equivale a --force-build en cada operación
        // (ver etc/vary.conf.example): también ordena fuente primero.
        order: if config.force_rebuild {
            CandidateOrder::PreferSource
        } else {
            config.candidate_order
        },
    };
    (source, opts)
}

/// P0-2: fecha el primer registro binario (best-effort con aviso; la
/// higiene de confianza nunca debe abortar un install válido).
fn stamp_first_trust(config: &Config, repo: &str) {
    if let Err(e) = crate::keys::stamp_trust(
        &config.repos_conf_path(),
        repo,
        Some(crate::keys::now_epoch()),
        true,
    ) {
        tracing::warn!("no se pudo fechar la confianza de '{repo}': {e:#}");
    }
}

/// P0-5: commit HEAD del clon (pin de procedencia; best-effort silencioso).
/// P1-1 lo reutiliza para el lockfile.
pub(crate) fn repo_head_commit(clone_path: &std::path::Path, git_bin: &str) -> Option<String> {
    let out = std::process::Command::new(git_bin)
        .arg("-C")
        .arg(clone_path)
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let sha = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if sha.is_empty() {
        None
    } else {
        Some(sha)
    }
}

/// P0-5: sha256 del artefacto `.xbps` (`{pkgver}.{arch}.xbps`), buscando en
/// el repo local de vary y en la caché de xbps. Best-effort silencioso.
fn package_artifact_hash(binpkgs_root: &str, pkgver: &str, arch: &str) -> Option<String> {
    use sha2::{Digest, Sha256};
    let file = format!("{pkgver}.{arch}.xbps");
    let found = [binpkgs_root, "/var/cache/xbps"]
        .into_iter()
        .map(|d| std::path::Path::new(d).join(&file))
        .find(|p| p.is_file())?;
    let bytes = std::fs::read(found).ok()?;
    Some(format!("sha256:{:x}", Sha256::digest(&bytes)))
}

/// P0-4: `vary -Sp <pkg>` — resuelve e imprime el plan SIN mutar nada.
///
/// Congelado: sin bootstrap (el `binpkgs_root` es ruta pura), sin
/// ensure/fetch de clones (se usan tal cual; clon ausente = error
/// accionable; para refrescarlos existe `vary -Sy`), sin guardar caché,
/// sin lock (ver `needs_lock`), sin review/materialize, sin confirm, sin
/// installs/builds, sin DB y —sobre todo— sin elevación. El índice VUP sí
/// puede refrescarse (lectura sin elevación que solo escribe su caché, como
/// `-Sy`). "No toca disco/red" = cero escrituras en el sistema, cero red
/// elevada, cero elevación (las lecturas son inevitables para resolver).
pub fn print_plan(config: &Config) -> Result<i32> {
    // Forma válida: -S con targets y sin banderas que cambien el significado
    // (search/info/downloadonly/upgrade); ver handle_sync.
    for (short, long) in [
        ("u", "sysupgrade"),
        ("s", "search"),
        ("i", "info"),
        ("w", "downloadonly"),
    ] {
        if config.args.has_arg(short, long) {
            anyhow::bail!("--print no se combina con -{short} (solo `-Sp <pkg>`)");
        }
    }
    let targets = config.targets.clone();
    if targets.is_empty() {
        anyhow::bail!("--print solo soporta `-Sp <pkg>` con targets (para upgrades no hay plan imprimible aún)");
    }
    for target in &targets {
        if !crate::metadata::is_valid_pkgname(target) {
            anyhow::bail!("nombre de paquete inválido: '{target}' (debe coincidir con ^[a-zA-Z0-9][a-zA-Z0-9._+-]*$)");
        }
    }

    // 2'. Repos tal cual están (sin ensure: clon ausente = error accionable,
    // nunca fetch silencioso).
    let repos_conf = ReposConf::load(config.repos_conf_path())?;
    let mut repos: Vec<VurRepo> = Vec::new();
    for (name, entry) in repos_conf.sorted_by_priority() {
        let repo = make_repo(&name, entry, config);
        if !repo.path.join(".git").exists() {
            anyhow::bail!(
                "el repo '{name}' no está clonado y --print no descarga: \
                 corre `vary --repo add` o `vary -Sy` (con red) primero"
            );
        }
        repos.push(repo);
    }

    // 3'. Arquitectura (igual que install; solo lecturas).
    let arch = config.arch();
    // Try xbps-query architecture if not overridden
    let arch = if config.arch_override.is_none() {
        xbps::query_architecture().unwrap_or(arch)
    } else {
        arch
    };

    // 4'. Índices (sin guardar caché) + fuente (binpkgs_root como
    // ruta pura: sin bootstrap no hay masterdir que consultar).
    let mut cache = CacheIndex::load(config.cache_index_path())?;
    let maps = load_source_maps(&repos, &repos_conf, &mut cache, &arch, config)?;
    let binpkgs_root = config
        .void_packages_dir()
        .join("hostdir")
        .join("binpkgs")
        .display()
        .to_string();
    let (source, opts) = assemble_source(maps, &arch, &binpkgs_root, config);

    let plan = crate::resolver::resolve(&targets, &source, &opts)?;
    print_stable_plan(&targets, &plan);
    // P1-1: verificación de solo-lectura (no rompe lo congelado: leer el
    // lock y los HEADs no escribe, no descarga ni eleva).
    if config.lock_path().is_file() {
        println!("lock [{}]:", config.lock_path().display());
        let repos_ref: Vec<_> = repos
            .iter()
            .map(|r| (r.name.clone(), r.path.clone()))
            .collect();
        let mut items = Vec::new();
        for it in plan.installs.iter().chain(plan.builds.iter()) {
            // Oficiales omitidos (ver arriba: placeholder T-009).
            if matches!(it.action, Action::Install(BinarySource::Official)) {
                continue;
            }
            items.push((it.info.pkgname.clone(), it.info.pkgver()));
        }
        let warnings = crate::lockfile::verify_if_pinned(config, &repos_ref, &items);
        if warnings.is_empty() {
            println!("  ok");
        } else {
            for w in &warnings {
                println!("  {w}");
            }
        }
    }
    Ok(0)
}

/// P0-4: imprime el plan en formato estable sin color (para scripts).
/// Reutiliza las formas de línea del display de `-S` para que lo que hoy
/// parsea `-S` siga valiendo.
fn print_stable_plan(targets: &[String], plan: &crate::resolver::Plan) {
    println!(
        ":: PRINT-ONLY plan for: {} (no changes will be made)",
        targets.join(" ")
    );
    println!("installs [single binary transaction]:");
    if plan.installs.is_empty() {
        println!("  (none)");
    }
    for item in &plan.installs {
        let src = match &item.action {
            Action::Install(BinarySource::Official) => "official",
            Action::Install(BinarySource::VulBinary { repo }) => repo.as_str(),
            _ => "?",
        };
        // H-046: lo operativo es siempre `info.pkgname`.
        let real = &item.info.pkgname;
        // T-009: sin versión => solo repo/nombre (xbps resuelve la versión).
        if item.info.version.is_empty() {
            println!("  {src}/{real}");
            continue;
        }
        if real == &item.name {
            println!(
                "  {src}/{real}-{} [{}]",
                item.info.version,
                item.info.pkgver()
            );
        } else {
            println!(
                "  {src}/{real}-{} [{}] (pedido como {})",
                item.info.version,
                item.info.pkgver(),
                item.name
            );
        }
    }
    println!("builds [topological levels]:");
    let levels = crate::resolver::build_levels(plan);
    if levels.is_empty() {
        println!("  (none)");
    }
    for (n, level) in levels.iter().enumerate() {
        println!("  level {n}:");
        for item in level {
            let real = &item.info.pkgname;
            if real == &item.name {
                println!("    {real}/{}", item.info.pkgver());
            } else {
                println!(
                    "    {real}/{} (pedido como {})",
                    item.info.pkgver(),
                    item.name
                );
            }
        }
    }
    let (setups, transaction) = elevation_estimate(plan);
    println!("elevations [estimated total: {}]:", setups + transaction);
    println!("  repo-setup: {setups}");
    println!("  install-transaction: {transaction}");
    for w in &plan.warnings {
        println!("warning: {w}");
    }
}

/// P0-4: elevaciones que el plan implicaría si se ejecutara: registros
/// binarios pendientes (conf o llave ausentes en /etc) + 1 transacción de
/// instalación si hay trabajo. Regla honesta y estable (típicamente 1 en
/// régimen: solo la transacción). Asume corrida no-root estándar.
fn elevation_estimate(plan: &crate::resolver::Plan) -> (usize, usize) {
    use std::collections::HashSet;
    let mut repos = HashSet::new();
    for item in &plan.installs {
        if let Action::Install(BinarySource::VulBinary { repo }) = &item.action {
            repos.insert(repo.clone());
        }
    }
    let mut setups = 0;
    for repo in &repos {
        if !crate::keys::binary_repo_registered(repo) {
            setups += 1;
        }
    }
    let transaction = usize::from(!plan.installs.is_empty() || !plan.builds.is_empty());
    (setups, transaction)
}

fn official_exists_remote(name: &str, exclude: &str) -> bool {
    // Consultar la propiedad `repository`: si solo existe en el repo local de
    // vary (hostdir/binpkgs) NO cuenta como oficial.
    let out = std::process::Command::new("xbps-query")
        .args(["-R", "--property=repository", "--", name])
        .output();
    match out {
        Ok(o) if o.status.success() && !o.stdout.is_empty() => {
            let text = String::from_utf8_lossy(&o.stdout);
            let mut non_local = false;
            for line in text.lines() {
                let l = line.trim();
                if l.is_empty() {
                    continue;
                }
                // Repositorio local de vary: ruta absoluta al hostdir/binpkgs
                if l.contains("hostdir/binpkgs") || (!exclude.is_empty() && l == exclude) {
                    continue;
                }
                non_local = true;
            }
            non_local
        }
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// P1-3: ejecución de builds (E3 niveles + E4 índice único).
//
// `WorkerParams` lleva TODO lo que un worker necesita en owned (los hilos
// solo comparten `&VurRepo`, que es Sync por construcción con tiposowned).
// ---------------------------------------------------------------------------

/// Parámetros de build para workers (todo owned/clonable).
/// (Sin `arch`: el layout de binpkgs es plano, ver `binpkgs_dir`.)
struct WorkerParams {
    makejobs: usize,
    logs_dir: Option<std::path::PathBuf>,
    log_file_default: Option<std::path::PathBuf>,
    sudo_bin: String,
    sudo_flags: Vec<String>,
}

/// Un build pendiente ya resuelto (main thread): repo por índice (los hilos
/// comparten `&VurRepo`), flags y subpaquetes para `built_names`.
struct BuildJob {
    parent_pkg: String,
    repo_idx: usize,
    explicit: bool,
    subs: Vec<String>,
}

/// Vía secuencial INTACTA (0.4.0): mismo cuerpo que el loop histórico
/// (materialize→project→build→unproject + mismo error). La usa el camino
/// slots==1 y el fallback cuando el overlay no monta.
fn build_one_sequential(
    repo: &VurRepo,
    md: &crate::masterdir::Masterdir,
    parent_pkg: &str,
    explicit: bool,
    makejobs: usize,
) -> Result<()> {
    repo.materialize_pkg(parent_pkg)?;
    repo.project_pkg(&md.srcpkgs_dir(), parent_pkg, explicit)?;
    let res = md.build_pkg(parent_pkg, makejobs);
    // Always unproject (proyectamos parent_pkg)
    let _ = repo.unproject_pkg(&md.srcpkgs_dir(), parent_pkg);
    res.with_context(|| format!("building {parent_pkg}"))?;
    Ok(())
}

/// Log del worker: dir de logs + sufijo de slot (`sb.log_file`); si no hay
/// dir, hermano del default (`xbps-src.log` → `xbps-src.log.w<N>`); si no
/// hay nada, None (hereda stdio, como el secuencial sin log).
/// Puro sobre paths salvo el nombre del slot (testeable).
fn worker_log_file(
    logs_dir: Option<&std::path::Path>,
    log_default: Option<&std::path::Path>,
    sb: &crate::masterdir::WorkerSandbox,
) -> Option<std::path::PathBuf> {
    if let Some(d) = logs_dir {
        return Some(sb.log_file(d));
    }
    log_default.map(|p| {
        let mut s = p.as_os_str().to_owned();
        s.push(format!(".w{}", sb.slot()));
        std::path::PathBuf::from(s)
    })
}

/// UN build dentro del sandbox del worker. El binpkgs del worker es privado
/// (su upper): xbps-src lo indexa solo allí, sin carreras; el hilo
/// principal fusiona e indexa una vez (E4). Devuelve los .xbps nuevos.
fn build_one_in_sandbox(
    repo: &VurRepo,
    sb: &crate::masterdir::WorkerSandbox,
    slot: usize,
    parent_pkg: &str,
    explicit: bool,
    params: &WorkerParams,
) -> Result<Vec<std::path::PathBuf>> {
    if crate::signal::is_shutting_down() {
        anyhow::bail!("interrumpido por señal antes de compilar {parent_pkg}");
    }
    let srcpkgs = sb.srcpkgs_dir();
    repo.project_pkg(&srcpkgs, parent_pkg, explicit)?;
    let binpkgs = sb.binpkgs_dir();
    let before = crate::masterdir::list_xbps(&binpkgs);
    tracing::info!(
        "compilando {parent_pkg} con xbps-src en worker w{slot} (makejobs: {})...",
        params.makejobs
    );
    let log = worker_log_file(
        params.logs_dir.as_deref(),
        params.log_file_default.as_deref(),
        sb,
    );
    // SIN -m a propósito: xbps-src resuelve masterdir-<arch> por defecto
    // (visible por lowerdir con su marker; pasar -m al legacy provocaba un
    // bootstrap completo por worker).
    let code = crate::xbps::xbps_src(
        sb.checkout_root(),
        &["pkg", parent_pkg],
        Some(params.makejobs),
        log.as_deref(),
    )
    .with_context(|| format!("falló ./xbps-src pkg {parent_pkg} en worker w{slot}"))?;
    let _ = repo.unproject_pkg(&srcpkgs, parent_pkg);
    if code != 0 {
        anyhow::bail!("xbps-src pkg {parent_pkg} terminó con código {code} en worker w{slot}");
    }
    Ok(crate::masterdir::new_files_since(
        &before,
        &crate::masterdir::list_xbps(&binpkgs),
    ))
}

/// Ejecuta UN nivel en paralelo (scope: ningún hilo escapa).
/// Devuelve `(posición del job, resultado)`; el llamador aplica en orden.
fn build_level_parallel(
    level: &[usize],
    jobs: &[BuildJob],
    repos: &[VurRepo],
    sandboxes: &[crate::masterdir::WorkerSandbox],
    params: &WorkerParams,
) -> Vec<(usize, Result<Vec<std::path::PathBuf>>)> {
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    };
    let queue = Mutex::new(level.to_vec());
    let out: Mutex<Vec<(usize, Result<Vec<PathBuf>>)>> = Mutex::new(Vec::new());
    let failed = AtomicBool::new(false);
    // Refs compartidas (Copy): el `move` del hilo las posee sin mover los Mutex.
    // (Sin shadowing: el shadowing rompe el análisis de vidas del scope.)
    let queue_ref = &queue;
    let out_ref = &out;
    let failed_ref = &failed;
    let jobs_ref = &jobs[..];
    let repos_ref = &repos[..];
    std::thread::scope(|s| {
        for (slot, sb) in sandboxes.iter().enumerate() {
            // `move`: el hilo posee sus copias (slot usize + &ref a sandbox
            // que vive más que el scope); sin move tomaría prestado el local
            // del loop, que muere al final de la iteración.
            s.spawn(move || loop {
                if failed_ref.load(Ordering::Relaxed) || crate::signal::is_shutting_down() {
                    break;
                }
                let pos = {
                    let mut q = match queue_ref.lock() {
                        Ok(g) => g,
                        Err(p) => p.into_inner(),
                    };
                    q.pop()
                };
                let Some(pos) = pos else { break };
                let job = &jobs_ref[pos];
                let r = build_one_in_sandbox(
                    &repos_ref[job.repo_idx],
                    sb,
                    slot,
                    &job.parent_pkg,
                    job.explicit,
                    params,
                );
                if r.is_err() {
                    // Parada temprana best-effort (los workers terminan su
                    // pkg en curso; el llamador decide en orden).
                    failed_ref.store(true, Ordering::Relaxed);
                }
                match out_ref.lock() {
                    Ok(mut g) => g.push((pos, r)),
                    Err(p) => p.into_inner().push((pos, r)),
                }
            });
        }
    });
    // El scope terminó: ningún hilo vive; extraer resultados (take, no clone:
    // Result no es Clone). Envenenado = nos quedamos con lo que haya.
    // (El guard se liga primero: moverlo desde el temporal del match no vive.)
    let mut guard = match out_ref.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    std::mem::take(&mut *guard)
}

/// E4: índice ÚNICO del repo local tras fusionar artefactos de workers.
/// `xbps-rindex -a` añade las entradas (mismo efecto final que los N
/// auto-índices de xbps-src en secuencial, pero en una sola pasada y sin
/// carreras). stdin nulo: nunca cuelga.
fn index_merged_artifacts(files: &[std::path::PathBuf]) -> Result<()> {
    if files.is_empty() {
        return Ok(());
    }
    let mut cmd = std::process::Command::new("xbps-rindex");
    cmd.arg("-a");
    for f in files {
        cmd.arg(f);
    }
    cmd.stdin(std::process::Stdio::null());
    let out = cmd
        .output()
        .with_context(|| "no se pudo invocar xbps-rindex para el índice fusionado")?;
    if !out.status.success() {
        anyhow::bail!(
            "xbps-rindex -a falló: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

pub fn install(config: &mut Config) -> Result<i32> {
    let targets = config.targets.clone();
    if targets.is_empty() {
        bail!("no targets specified");
    }
    for target in &targets {
        if !crate::metadata::is_valid_pkgname(target) {
            bail!("nombre de paquete inválido: '{target}' (debe coincidir con ^[a-zA-Z0-9][a-zA-Z0-9._+-]*$)");
        }
    }

    // 1. Bootstrap
    let md = bootstrap::initialize_environment(
        &config.void_packages_dir(),
        &config.sudo_bin,
        &config.sudo_flags,
        &config.tools_install_bin,
        &config.git_bin,
    )?;

    // 2. Load repos
    let repos_conf = ReposConf::load(config.repos_conf_path())?;
    let sorted = repos_conf.sorted_by_priority();
    let mut repos: Vec<VurRepo> = Vec::new();
    for (name, entry) in sorted {
        let repo = make_repo(&name, entry, config);
        match repo.ensure_cloned() {
            Ok(_) => repos.push(repo),
            Err(e) => tracing::warn!("failed to ensure VUR '{name}': {e}"),
        }
    }

    // 3. Arquitectura (antes de los índices: el adaptador VUP filtra por arch).
    let arch = config.arch();
    // Try xbps-query architecture if not overridden
    let arch = if config.arch_override.is_none() {
        xbps::query_architecture().unwrap_or(arch)
    } else {
        arch
    };

    // 4. Load indexes (ver load_source_maps).
    let mut cache = CacheIndex::load(config.cache_index_path())?;
    let maps = load_source_maps(&repos, &repos_conf, &mut cache, &arch, config)?;
    // URLs binarias VUP por paquete (sección 8 las necesita tras mover maps).
    let vup_binary_urls = maps.vup_binary_urls.clone();
    let _ = cache.save();

    // 5. Build source
    let binpkgs_root = md.hostdir_binpkgs_root().display().to_string();
    let (source, opts) = assemble_source(maps, &arch, &binpkgs_root, config);

    let plan = crate::resolver::resolve(&targets, &source, &opts)?;

    // P1-1: verificación contra vary.lock (avisos, solo si existe). Los
    // oficiales se omiten: xbps es su fuente de verdad y el plan lleva
    // placeholder sin versión (T-009); verificarlos daría falsos avisos.
    {
        let repos_ref: Vec<_> = repos
            .iter()
            .map(|r| (r.name.clone(), r.path.clone()))
            .collect();
        let mut items = Vec::new();
        for it in plan.installs.iter().chain(plan.builds.iter()) {
            if matches!(it.action, Action::Install(BinarySource::Official)) {
                continue;
            }
            items.push((it.info.pkgname.clone(), it.info.pkgver()));
        }
        for w in crate::lockfile::verify_if_pinned(config, &repos_ref, &items) {
            tracing::warn!("{w}");
        }
    }

    if plan.installs.is_empty() && plan.builds.is_empty() {
        println!("nothing to do");
        return Ok(0);
    }

    // 6. Show plan
    let c = &config.color;
    if !plan.installs.is_empty() {
        println!("\n{} Packages to install (binary):", c.bold.paint("::"));
        for item in &plan.installs {
            let src = match &item.action {
                Action::Install(BinarySource::Official) => "official",
                Action::Install(BinarySource::VulBinary { repo }) => repo.as_str(),
                _ => "?",
            };
            // H-046: `name` es lo pedido (puede ser virtual de `provides`);
            // lo operativo es siempre `info.pkgname`.
            let real = &item.info.pkgname;
            // T-009: los oficiales llevan placeholder sin versión; no imprimir
            // ficción (`name- [_0]`), solo el nombre (xbps resuelve la versión).
            if item.info.version.is_empty() {
                println!("  {src}/{real}");
                continue;
            }
            if real == &item.name {
                println!(
                    "  {src}/{real}-{} [{}]",
                    item.info.version,
                    item.info.pkgver()
                );
            } else {
                println!(
                    "  {src}/{real}-{} [{}] (pedido como {})",
                    item.info.version,
                    item.info.pkgver(),
                    item.name
                );
            }
        }
    }
    if !plan.builds.is_empty() {
        println!("\n{} Packages to build (source):", c.bold.paint("::"));
        for item in &plan.builds {
            let real = &item.info.pkgname;
            if real == &item.name {
                println!("  {real}/{}", item.info.pkgver());
            } else {
                println!(
                    "  {real}/{} (pedido como {})",
                    item.info.pkgver(),
                    item.name
                );
            }
        }
        println!(
            "\n{}",
            c.warning.paint(format!(
                "Builds son secuenciales (paralelismo vía XBPS_MAKEJOBS={})",
                config.makejobs
            ))
        );
    }
    // T-010: avisos del resolver (flag sin efecto sobre official) junto al
    // plan, antes del confirm: ACTUAR o AVISAR, nunca callar.
    for w in &plan.warnings {
        println!("\n{}", c.warning.paint(format!("advertencia: {w}")));
    }
    println!();

    if !config.no_confirm {
        for item in &plan.builds {
            // H-046: revisar por nombre real (lo pedido puede ser virtual).
            let real = &item.info.pkgname;
            if let Ok(repo_name) = vur_map_lookup_repo(real, &repos, &mut cache)
                .or_else(|_| vur_map_lookup_repo(&item.name, &repos, &mut cache))
            {
                if let Some(repo) = repos.iter().find(|r| r.name == repo_name) {
                    let _ = repo.materialize_pkg(real);
                    let _ =
                        crate::review::prompt_review(real, &repo.name, &repo.path, &config.git_bin);
                }
            }
        }
    } else {
        // P0-3: con --yes no hay pager ni prompt, pero los hallazgos HIGH
        // siguen imprimiéndose (lectura vía git, sin materializar).
        for item in &plan.builds {
            let real = &item.info.pkgname;
            if let Ok(repo_name) = vur_map_lookup_repo(real, &repos, &mut cache)
                .or_else(|_| vur_map_lookup_repo(&item.name, &repos, &mut cache))
            {
                if let Some(repo) = repos.iter().find(|r| r.name == repo_name) {
                    let content =
                        crate::review::read_template_text(real, &repo.path, &config.git_bin);
                    if !content.is_empty() {
                        crate::review::print_audit_header(real, &repo.name, &content);
                    }
                }
            }
        }
    }

    if !confirm("Proceed with installation?", config.no_confirm)? {
        return Ok(1);
    }

    // 7. Pre-fetch en paralelo para todas las fuentes del DAG antes de compilar
    let to_build_items: Vec<_> = plan
        .builds
        .iter()
        .filter(|item| {
            if !config.force_rebuild && !config.force_build {
                // H-046: xbps conoce el nombre real, no el virtual pedido.
                if let Ok(Some(cur)) = crate::xbps::query_installed(&item.info.pkgname) {
                    if cur.pkgver == item.info.pkgver() {
                        return false;
                    }
                }
            }
            true
        })
        .collect();

    if !to_build_items.is_empty() {
        let mut distinct_parents: Vec<String> = Vec::new();
        let mut pkg_to_repo: HashMap<String, String> = HashMap::new();
        for item in &to_build_items {
            let parent = item.info.pkgname.clone();
            if !distinct_parents.contains(&parent) {
                if let Ok(repo_name) = vur_map_lookup_repo(&parent, &repos, &mut cache)
                    .or_else(|_| vur_map_lookup_repo(&item.name, &repos, &mut cache))
                {
                    pkg_to_repo.insert(parent.clone(), repo_name);
                    distinct_parents.push(parent);
                }
            }
        }

        if !distinct_parents.is_empty() {
            println!(
                "{} Pre-descargando fuentes en paralelo ({} paquetes)...",
                c.action.paint("::"),
                distinct_parents.len()
            );

            let workers = config.makejobs.clamp(1, 8);
            let queue = std::sync::Mutex::new(distinct_parents.into_iter());
            // P1-3: el clon git compartido no admite sparse-checkout
            // concurrente (index.lock): materialize va serializado; el fetch
            // (la parte lenta, red) sigue en paralelo.
            let git_serial = std::sync::Mutex::new(());
            std::thread::scope(|s| {
                for _ in 0..workers {
                    s.spawn(|| loop {
                        let next_pkg = {
                            let mut q = match queue.lock() {
                                Ok(guard) => guard,
                                Err(poisoned) => poisoned.into_inner(),
                            };
                            q.next()
                        };
                        let Some(pkg) = next_pkg else { break };
                        if crate::signal::is_shutting_down() {
                            break;
                        }
                        if let Some(repo_name) = pkg_to_repo.get(&pkg) {
                            if let Some(repo) = repos.iter().find(|r| &r.name == repo_name) {
                                {
                                    let _g = git_serial.lock().unwrap_or_else(|p| p.into_inner());
                                    let _ = repo.materialize_pkg(&pkg);
                                }
                                let _ = repo.project_pkg(&md.srcpkgs_dir(), &pkg, false);
                                tracing::debug!("pre-fetching fuentes para {pkg}");
                                let _ = md.fetch_pkg(&pkg);
                                let _ = repo.unproject_pkg(&md.srcpkgs_dir(), &pkg);
                            }
                        }
                    });
                }
            });
        }
    }

    // P1-3: scheduler de builds por niveles (E3) + índice único (E4).
    //
    // Pre-resolución en main thread (mensajes, orden y fallos deterministas;
    // el `?` conserva el comportamiento secuencial previo). Los niveles salen
    // de build_levels() (P0-4, testeado). Por nivel: 1 item o slots==1 van por
    // la vía secuencial intacta; si no, workers con sandbox (E2). El clon git
    // compartido NO admite sparse-checkout concurrente: materialize corre en
    // main thread por nivel, antes de soltar los workers.
    let mut built_names: Vec<String> = Vec::new();
    let mut merged_artifacts: Vec<PathBuf> = Vec::new();
    let mut merged_any = false;

    let mut jobs: Vec<BuildJob> = Vec::new();
    let mut key_to_job: std::collections::HashMap<(String, String), usize> =
        std::collections::HashMap::new();
    for item in plan.builds.iter() {
        // Ya instalada EXACTAMENTE esa versión => omitir compilación
        // (--force-build / force_rebuild la fuerzan).
        if !config.force_rebuild && !config.force_build {
            match crate::xbps::query_installed(&item.info.pkgname) {
                Ok(Some(cur)) if cur.pkgver == item.info.pkgver() => {
                    println!(
                        "{} {} ya instalado ({}) ; omitiendo build",
                        c.action.paint("::"),
                        c.bold.paint(&item.info.pkgname),
                        cur.pkgver
                    );
                    continue;
                }
                _ => {}
            }
        }
        let parent_pkg = item.info.pkgname.clone();
        let repo_name = vur_map_lookup_repo(&parent_pkg, &repos, &mut cache)
            .or_else(|_| vur_map_lookup_repo(&item.name, &repos, &mut cache))?;
        let repo_idx = repos
            .iter()
            .position(|r| r.name == repo_name)
            .ok_or_else(|| anyhow::anyhow!("repo not found for {}", item.info.pkgname))?;
        // force=true solo para targets explícitos del usuario (o subpaquetes de esos targets)
        let explicit = targets.iter().any(|t| {
            t == &parent_pkg
                || t == &item.name
                || item.info.subpackages.iter().any(|s| s.pkgname == *t)
        });
        key_to_job.insert((item.name.clone(), item.info.pkgname.clone()), jobs.len());
        jobs.push(BuildJob {
            parent_pkg,
            repo_idx,
            explicit,
            subs: item
                .info
                .subpackages
                .iter()
                .map(|s| s.pkgname.clone())
                .collect(),
        });
    }
    // Niveles sobre jobs (los ya-instalados simplemente no están).
    let levels: Vec<Vec<usize>> = crate::resolver::build_levels(&plan)
        .into_iter()
        .map(|lvl| {
            lvl.into_iter()
                .filter_map(|it| {
                    key_to_job
                        .get(&(it.name.clone(), it.info.pkgname.clone()))
                        .copied()
                })
                .collect()
        })
        .filter(|v: &Vec<usize>| !v.is_empty())
        .collect();

    let nproc = std::thread::available_parallelism()
        .map(|p| p.get())
        .unwrap_or(1);
    let slots = crate::masterdir::effective_build_slots(
        config.experimental,
        config.max_concurrent_builds,
        nproc,
    );
    let workers_dir = config.cache_dir.join("workers");
    // Barrido de sandboxes residuales SIEMPRE (solo w<N> bajo workers/).
    crate::masterdir::cleanup_stale_sandboxes(&workers_dir, &config.sudo_bin, &config.sudo_flags);
    // Parámetros propios de los workers (todo owned: sin pelear Sync).
    let logs_dir = md
        .log_file
        .as_ref()
        .and_then(|p| p.parent().map(Path::to_path_buf));
    let wparams = WorkerParams {
        makejobs: config.makejobs,
        logs_dir,
        log_file_default: md.log_file.clone(),
        sudo_bin: config.sudo_bin.clone(),
        sudo_flags: config.sudo_flags.clone(),
    };
    // Sandboxes perezosos: se crean al primer nivel que los necesite; si la
    // creación falla, ese nivel (y los siguientes) van en secuencial.
    let mut sandboxes: Vec<crate::masterdir::WorkerSandbox> = Vec::new();
    let mut parallel_usable =
        slots > 1 && crate::masterdir::parallel_available(&config.cache_dir).is_ok();
    if slots > 1 {
        if let Err(e) = crate::masterdir::parallel_available(&config.cache_dir) {
            tracing::warn!("P1-3: {e:#}");
        }
    }
    for level in &levels {
        if level.len() < 2 || !parallel_usable {
            // Vía secuencial intacta (1 item, slots==1 o sin capacidad).
            for pos in level {
                let job = &jobs[*pos];
                let repo = &repos[job.repo_idx];
                build_one_sequential(repo, &md, &job.parent_pkg, job.explicit, wparams.makejobs)?;
                // H-046: registrar el nombre REAL.
                built_names.push(job.parent_pkg.clone());
                for sub in &job.subs {
                    if !built_names.contains(sub) {
                        built_names.push(sub.clone());
                    }
                }
            }
            continue;
        }
        // Materialize secuencial en main (el clon no admite sparse concurrente).
        for pos in level {
            let job = &jobs[*pos];
            repos[job.repo_idx].materialize_pkg(&job.parent_pkg)?;
        }
        // Sandboxes (una vez; reuso entre niveles: el upper acumula, bien).
        if sandboxes.is_empty() {
            let mut ok = true;
            for slot in 0..slots {
                match crate::masterdir::WorkerSandbox::create(
                    &md.path,
                    &workers_dir,
                    slot,
                    &wparams.sudo_bin,
                    &wparams.sudo_flags,
                ) {
                    Ok(sb) => sandboxes.push(sb),
                    Err(e) => {
                        tracing::warn!(
                            "P1-3: sandbox {slot} no montable ({e:#}); nivel en secuencial"
                        );
                        ok = false;
                        break;
                    }
                }
            }
            if !ok || sandboxes.is_empty() {
                sandboxes.clear();
                parallel_usable = false;
                // Reintento secuencial del nivel (materialize ya hecho: vale).
                for pos in level {
                    let job = &jobs[*pos];
                    let repo = &repos[job.repo_idx];
                    build_one_sequential(
                        repo,
                        &md,
                        &job.parent_pkg,
                        job.explicit,
                        wparams.makejobs,
                    )?;
                    built_names.push(job.parent_pkg.clone());
                    for sub in &job.subs {
                        if !built_names.contains(sub) {
                            built_names.push(sub.clone());
                        }
                    }
                }
                continue;
            }
            println!(
                "{} builds paralelos: {} slots con overlay (experimental)",
                c.action.paint("::"),
                sandboxes.len()
            );
        }
        // Nivel en paralelo (scope: los hilos no escapan; resultados en orden).
        let results = build_level_parallel(level, &jobs, &repos, &sandboxes, &wparams);
        // Aplicar en orden de plan; al primer error se fusiona lo previo y se
        // aborta (paridad con el secuencial: estado parcial + error).
        for pos in level {
            let found = results.iter().find(|(p, _)| p == pos);
            let Some((_, res)) = found else {
                anyhow::bail!("sin resultado del worker para {}", jobs[*pos].parent_pkg)
            };
            let arts = res
                .as_ref()
                .map_err(|e| anyhow::anyhow!("building {}: {e:#}", jobs[*pos].parent_pkg))?;
            for art in arts {
                // Layout plano (sin subdir de arch), igual que xbps-src.
                let dest = md
                    .hostdir_binpkgs_root()
                    .join(art.file_name().unwrap_or_default());
                if let Some(parent) = dest.parent() {
                    crate::util::ensure_private_dir(parent)?;
                }
                std::fs::copy(art, &dest)
                    .with_context(|| format!("fusionando artefacto {}", art.display()))?;
                merged_artifacts.push(dest);
                merged_any = true;
            }
            built_names.push(jobs[*pos].parent_pkg.clone());
            for sub in &jobs[*pos].subs {
                if !built_names.contains(sub) {
                    built_names.push(sub.clone());
                }
            }
        }
    }
    // E4: índice ÚNICO tras fusionar (nunca en workers: sin carreras).
    if merged_any {
        tracing::info!(
            "P1-3: fusionados {} artefactos de workers; índice único",
            merged_artifacts.len()
        );
        index_merged_artifacts(&merged_artifacts)?;
    }
    // Limpieza explícita de sandboxes (umount + rmdir). Ante un `?` previo
    // el Drop ya desmontó y el dir lo barre cleanup_stale en el próximo run.
    for sb in sandboxes {
        sb.destroy();
    }

    // 8. Installs
    let mut all_install_names: Vec<String> = Vec::new();
    let mut binary_repos_configured: HashSet<String> = HashSet::new();

    // URLs binarias VUP por repo (una por categoría), precalculadas UNA vez
    // fuera del loop (antes se reconstruían por cada repo configurado).
    let mut vup_urls_by_repo: HashMap<String, Vec<String>> = HashMap::new();
    for it in &plan.installs {
        if let Action::Install(BinarySource::VulBinary { repo: rr }) = &it.action {
            // H-046: las claves son pkgname reales (ver construcción arriba).
            if let Some(u) = vup_binary_urls.get(&format!("{}:{}", rr, it.info.pkgname)) {
                let urls = vup_urls_by_repo.entry(rr.clone()).or_default();
                if !urls.contains(u) {
                    urls.push(u.clone());
                }
            }
        }
    }

    for item in &plan.installs {
        // H-046: instalar por nombre real (oficiales: info.pkgname == name).
        let real = item.info.pkgname.clone();
        match &item.action {
            Action::Install(BinarySource::Official) => all_install_names.push(real),
            Action::Install(BinarySource::VulBinary { repo }) => {
                if !binary_repos_configured.contains(repo) {
                    if let Some(r) = repos.iter().find(|r| &r.name == repo) {
                        let entry = repos_conf.vur.get(repo).ok_or_else(|| {
                            anyhow::anyhow!("repositorio '{repo}' no encontrado en repos.conf")
                        })?;
                        if entry.has_vup_index() {
                            // Repo estilo VUP: una URL binaria por categoría;
                            // registrar las necesarias para los paquetes del plan.
                            let urls: Vec<String> =
                                vup_urls_by_repo.get(repo).cloned().unwrap_or_default();
                            // T-006: el clon suele ser sparse (keys/ no está en
                            // disco); leer la llave vía git con fallback a fs.
                            // T-012: se necesita el plist CRUDO (byte-idéntico
                            // al que xbps almacena) para el pre-import.
                            let key_plist =
                                crate::vup_index::read_repo_plist_text_git(&r.git_bin, &r.path)
                                    .or_else(|_| crate::vup_index::read_repo_plist_text(&r.path))?;
                            let key_pem =
                                crate::vup_index::decode_plist_public_key_pem(&key_plist)?;
                            // P0-2: ¿primer registro? Solo se fecha la confianza
                            // si la llave NO pre-existía (si ya había, la
                            // fecha original —o su ausencia— se preserva).
                            let had_key = crate::keys::repo_key_installed(repo);
                            if let Err(e) = crate::keys::setup_vup_binary_repo(
                                repo,
                                &urls,
                                &key_pem,
                                &key_plist,
                                entry,
                                &config.sudo_bin,
                                &config.sudo_flags,
                                &config.tools_install_bin,
                                config.no_confirm,
                            ) {
                                bail!("failed to setup VUP binary repo '{repo}': {e}");
                            }
                            if !had_key {
                                stamp_first_trust(config, repo);
                            }
                        } else {
                            let had_key = crate::keys::repo_key_installed(repo);
                            if let Err(e) = crate::keys::setup_binary_repo(
                                r,
                                entry,
                                &config.sudo_bin,
                                &config.sudo_flags,
                                &config.tools_install_bin,
                                config.no_confirm,
                            ) {
                                bail!("failed to setup binary repo '{repo}': {e}");
                            }
                            if !had_key {
                                stamp_first_trust(config, repo);
                            }
                        }
                    }
                    binary_repos_configured.insert(repo.clone());
                }
                all_install_names.push(real);
            }
            _ => {}
        }
    }
    // Built packages also need install from local repo
    all_install_names.extend(built_names.clone());

    if !all_install_names.is_empty() {
        // If we built anything, add local repository flag
        let mut extra: Vec<String> = Vec::new();
        if !built_names.is_empty() {
            extra.push(format!(
                "--repository={}",
                md.hostdir_binpkgs_root().display()
            ));
        }
        if config.no_confirm {
            extra.push("-y".to_string());
        }
        // Also handle --asdeps etc? For now just sync
        let code = xbps::install(
            &all_install_names,
            &extra,
            &config.sudo_bin,
            &config.sudo_flags,
        )?;
        if code != 0 {
            return Ok(code);
        }
        for name in &all_install_names {
            let _ = crate::init::post_install_hook(
                name,
                config.no_confirm,
                &config.sudo_bin,
                &config.sudo_flags,
            );
        }
    }

    // 9. Record in db
    let mut db = InstalledDb::load(config.installed_db_path())?;
    for item in plan.installs.iter().chain(plan.builds.iter()) {
        // Only VUR packages (por nombre real: lo pedido puede ser virtual).
        let real = &item.info.pkgname;
        // T-005: los oficiales los gestiona xbps; registrarlos aquí escribía
        // ficción. Solo Build y VulBinary dejan rastro. P0-5: los VulBinary
        // registran por su repo de acción (el gate `is_vur` los dejaba fuera
        // al no tener template en el clon, contradiciendo la regla T-005).
        let repo_name: Option<String> = match &item.action {
            Action::Install(BinarySource::VulBinary { repo }) => Some(repo.clone()),
            Action::Build => vur_map_lookup_repo(real, &repos, &mut cache).ok(),
            Action::Install(BinarySource::Official) => None,
        };
        let (Some(repo_name), Some(itype)) = (repo_name, track_action(&item.action)) else {
            continue;
        };
        // P0-5: pins best-effort (nunca fatales) + aviso de drift.
        let version = item.info.pkgver();
        let repo_commit = repo_head_commit(&config.vurs_dir().join(&repo_name), &config.git_bin);
        let artifact_sha256 = package_artifact_hash(&binpkgs_root, &version, &arch);
        for w in crate::db::drift_warnings(db.get(real), &version, &repo_commit, &artifact_sha256) {
            tracing::warn!("{w}");
        }
        db.upsert(
            real,
            &version,
            &repo_name,
            itype.clone(),
            repo_commit.clone(),
            artifact_sha256.clone(),
        );
        // P2-soname: pin de proveedores al instalar desde fuente
        // (best-effort: sin xbps local no hay baseline, sin ruido).
        if itype == crate::db::InstallType::Source {
            let pins = crate::soname::capture(real);
            if pins.is_empty() {
                tracing::debug!("{real}: sin pins de sonames (paquete sin shlibs o no instalado)");
            }
            db.set_soname_pins(real, pins);
        }
        // P2: diario (best-effort: nunca aborta un install válido).
        if let Err(e) = crate::journal::append(
            &config.data_dir,
            "INSTALL",
            real,
            &format!("{version} {repo_name}"),
        ) {
            tracing::warn!("no se pudo anotar el diario: {e:#}");
        }
        for sub in &item.info.subpackages {
            // Los subpaquetes comparten commit pero su artefacto propio no se
            // rastrea (None honesto en vez de ficción).
            db.upsert(
                &sub.pkgname,
                &version,
                &repo_name,
                itype.clone(),
                repo_commit.clone(),
                None,
            );
            // P2-soname: los subs son paquetes instalados propios.
            if itype == crate::db::InstallType::Source {
                db.set_soname_pins(&sub.pkgname, crate::soname::capture(&sub.pkgname));
            }
        }
    }
    db.save()?;

    // P1-1: --lock deja el lock reflejando el estado final (solo en éxito).
    if config.args.has_arg("lock", "lock") {
        crate::lockfile::regenerate(config)?;
    }

    Ok(0)
}

/// Decide si una acción del plan deja rastro en installed.json (T-005).
/// `None` = xbps es la fuente de verdad (oficiales y resto): no registrar.
fn track_action(action: &Action) -> Option<InstallType> {
    match action {
        Action::Install(BinarySource::VulBinary { .. }) => Some(InstallType::Binary),
        Action::Build => Some(InstallType::Source),
        _ => None,
    }
}

fn vur_map_lookup_repo(name: &str, repos: &[VurRepo], cache: &mut CacheIndex) -> Result<String> {
    for repo in repos {
        // TTL None: hit de caché por commit-sha (ya cargado en la fase de índices)
        if let Ok(infos) = repo.load_index(cache, None) {
            for info in infos {
                if info.pkgname == name || info.subpackages.iter().any(|s| s.pkgname == name) {
                    return Ok(repo.name.clone());
                }
            }
        }
    }
    bail!("no VUR repo found for {name}")
}

pub fn download_only(config: &mut Config) -> Result<i32> {
    let targets = config.targets.clone();
    if targets.is_empty() {
        bail!("no targets specified");
    }
    for target in &targets {
        if !crate::metadata::is_valid_pkgname(target) {
            bail!("nombre de paquete inválido: '{target}' (debe coincidir con ^[a-zA-Z0-9][a-zA-Z0-9._+-]*$)");
        }
    }
    let md = bootstrap::initialize_environment(
        &config.void_packages_dir(),
        &config.sudo_bin,
        &config.sudo_flags,
        &config.tools_install_bin,
        &config.git_bin,
    )?;

    let repos_conf = ReposConf::load(config.repos_conf_path())?;
    let mut repos: Vec<VurRepo> = Vec::new();
    for (name, entry) in repos_conf.sorted_by_priority() {
        let path = config.vurs_dir().join(&name);
        let repo = VurRepo {
            name: name.clone(),
            path,
            entry: entry.clone(),
            git_bin: config.git_bin.clone(),
        };
        if repo.ensure_cloned().is_ok() {
            repos.push(repo);
        }
    }
    let mut cache = CacheIndex::load(config.cache_index_path())?;
    for target in &targets {
        // Buscar en todos los repos (incluye subpaquetes => compilar al padre)
        let mut found = false;
        'repos: for repo in &repos {
            let Ok(infos) = repo.load_index(&mut cache, None) else {
                continue;
            };
            for info in &infos {
                let is_match = info.pkgname == *target
                    || info.subpackages.iter().any(|s| s.pkgname == *target);
                if is_match {
                    let parent = info.pkgname.clone();
                    let _ = repo.materialize_pkg(&parent);
                    repo.project_pkg(&md.srcpkgs_dir(), &parent, false)?;
                    md.fetch_pkg(&parent)?;
                    repo.unproject_pkg(&md.srcpkgs_dir(), &parent)?;
                    println!("Fetched {target} (parent {parent})");
                    found = true;
                    break 'repos;
                }
            }
        }
        if !found {
            println!("{target} not found in VURs (or is official)");
        }
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resolver::{Plan, PlanItem};

    fn fake_info(pkgname: &str) -> VurInfo {
        VurInfo {
            format_version: 1,
            pkgname: pkgname.to_string(),
            version: "1.0".to_string(),
            revision: 1,
            archs: vec!["x86_64".to_string()],
            subpackages: vec![],
            depends: vec![],
            hostmakedepends: vec![],
            makedepends: vec![],
            checkdepends: vec![],
            build_style: None,
            distfiles: vec![],
            checksum: vec![],
            provides: vec![],
            replaces: vec![],
            restricted: false,
            maintainer: None,
        }
    }

    fn install_item(name: &str, action: Action) -> PlanItem {
        PlanItem {
            name: name.to_string(),
            info: fake_info(name),
            action,
        }
    }

    #[test]
    fn elevation_sin_trabajo_es_cero() {
        let plan = Plan::default();
        assert_eq!(elevation_estimate(&plan), (0, 0));
    }

    #[test]
    fn elevation_oficial_solo_transaccion() {
        // Sin VulBinary no se lee /etc: puro y estable.
        let plan = Plan {
            installs: vec![install_item(
                "curl",
                Action::Install(BinarySource::Official),
            )],
            builds: vec![],
            warnings: vec![],
        };
        assert_eq!(elevation_estimate(&plan), (0, 1));
    }

    #[test]
    fn elevation_build_cuenta_transaccion() {
        let plan = Plan {
            installs: vec![],
            builds: vec![install_item("demo", Action::Build)],
            warnings: vec![],
        };
        assert_eq!(elevation_estimate(&plan), (0, 1));
    }

    #[test]
    fn repo_head_commit_lee_head_y_falla_cerrado() {
        let dir = tempfile::tempdir().unwrap();
        // No es repo: None, no panic.
        assert!(repo_head_commit(dir.path(), "git").is_none());
        let run = |args: &[&str]| {
            std::process::Command::new("git")
                .arg("-C")
                .arg(dir.path())
                .args(args)
                .output()
                .unwrap()
        };
        run(&["init", "-q"]);
        run(&[
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "x",
        ]);
        let sha = repo_head_commit(dir.path(), "git").unwrap();
        assert_eq!(sha.len(), 40);
        assert!(sha.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn package_artifact_hash_hashea_y_falla_cerrado() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_string_lossy().to_string();
        assert!(package_artifact_hash(&root, "demo-1.0_1", "x86_64").is_none());
        std::fs::write(dir.path().join("demo-1.0_1.x86_64.xbps"), b"fake-xbps").unwrap();
        let got = package_artifact_hash(&root, "demo-1.0_1", "x86_64").unwrap();
        assert!(got.starts_with("sha256:"));
        assert_eq!(got.len(), "sha256:".len() + 64);
    }

    #[test]
    fn install_rejects_invalid_target_name() {
        let mut config = Config {
            targets: vec!["-flag".to_string()],
            ..Default::default()
        };
        let err = install(&mut config).unwrap_err();
        assert!(err.to_string().contains("nombre de paquete inválido"));

        let mut config2 = Config {
            targets: vec!["bad/slash".to_string()],
            ..Default::default()
        };
        let err2 = install(&mut config2).unwrap_err();
        assert!(err2.to_string().contains("nombre de paquete inválido"));
    }

    #[test]
    fn track_action_solo_build_y_binario_vur() {
        // T-005: los oficiales no dejan rastro (xbps manda); Build y
        // VulBinary sí, con su tipo correspondiente.
        assert_eq!(track_action(&Action::Build), Some(InstallType::Source));
        assert_eq!(
            track_action(&Action::Install(BinarySource::VulBinary {
                repo: "vup".to_string()
            })),
            Some(InstallType::Binary)
        );
        assert_eq!(track_action(&Action::Install(BinarySource::Official)), None);
    }

    #[test]
    fn download_only_rejects_invalid_target_name() {
        let mut config = Config {
            targets: vec![".dotfile".to_string()],
            ..Default::default()
        };
        let err = download_only(&mut config).unwrap_err();
        assert!(err.to_string().contains("nombre de paquete inválido"));
    }

    // --- P1-3: log por worker (puro sobre paths) ---

    #[test]
    fn worker_log_prefiere_dir_y_hermana_default() {
        use std::path::PathBuf;
        // Sin WorkerSandbox real: paths inventados bastan (solo se leen).
        // (sb se construye vía dir temporal vacío: slot() parsea w<N>.)
        let base = tempfile::tempdir().unwrap();
        let slot_dir = base.path().join("w2");
        std::fs::create_dir(&slot_dir).unwrap();
        // WorkerSandbox no es construible sin mount; testear vía paths:
        // worker_log_file solo usa sb.log_file() y sb.slot().
        let got = worker_log_file(
            Some(std::path::Path::new("/logs")),
            Some(std::path::Path::new("/c/xbps-src.log")),
            &fake_sandbox(&slot_dir),
        );
        assert_eq!(got, Some(PathBuf::from("/logs/xbps-src-w2.log")));
        let got = worker_log_file(
            None,
            Some(std::path::Path::new("/c/xbps-src.log")),
            &fake_sandbox(&slot_dir),
        );
        assert_eq!(got, Some(PathBuf::from("/c/xbps-src.log.w2")));
        let got = worker_log_file(None, None, &fake_sandbox(&slot_dir));
        assert_eq!(got, None);
    }

    /// WorkerSandbox mínimo sin montar (los paths no necesitan existir para
    /// derivar nombres; el mount real solo se prueba en vivo).
    fn fake_sandbox(slot_dir: &std::path::Path) -> crate::masterdir::WorkerSandbox {
        // SAFETY del test: WorkerSandbox expone constructor solo con mount;
        // derivamos slot/log vía funciones asociadas puras en su lugar.
        // (Si esto no compila, el test recuerda cablear un ctor de test.)
        crate::masterdir::WorkerSandbox::for_test(slot_dir.to_path_buf())
    }
}
