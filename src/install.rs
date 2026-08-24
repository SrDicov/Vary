use crate::bootstrap;
use crate::cache::CacheIndex;
use crate::config::Config;
use crate::db::{InstallType, InstalledDb};
use crate::metadata::VurInfo;
use crate::reposconf::ReposConf;
use crate::resolver::{dep_name, Action, BinarySource, PackageSource, ResolveOptions};
use crate::util::confirm;
use crate::vur_client::VurRepo;
use crate::xbps;

use anyhow::{bail, Context, Result};
use std::collections::{HashMap, HashSet};


struct VurSource {
    official_exists_fn: Box<dyn Fn(&str) -> bool>,
    vur_map: HashMap<String, (String, VurInfo)>,
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
        // Filter by arch
        if let Some((repo, info)) = self.vur_map.get(name) {
            if !info.archs.contains(&self.arch) {
                return None;
            }
            return Some((repo.clone(), info.clone()));
        }
        None
    }

    fn vur_lookup_provides(&self, virtual_name: &str) -> Option<(String, VurInfo)> {
        let candidates = self.provides_map.get(virtual_name)?;
        // Choose best by priority (lowest) then return first
        let mut best: Option<(i64, &(String, VurInfo))> = None;
        for cand in candidates {
            let prio = self.priority.get(&cand.0).copied().unwrap_or(100);
            if cand.1.archs.contains(&self.arch) {
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
        if !info.archs.contains(&arch.to_string()) {
            return false;
        }
        *self.binary_check.get(&format!("{}:{}", repo, info.pkgname)).unwrap_or(&false)
    }

    fn arch(&self) -> String {
        self.arch.clone()
    }
}

fn official_exists_remote(name: &str, exclude: &str) -> bool {
    // Consultar la propiedad `repository`: si solo existe en el repo local de
    // vary (hostdir/binpkgs) NO cuenta como oficial.
    let out = std::process::Command::new("xbps-query")
        .args(["-R", "--property=repository", name])
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

pub fn install(config: &mut Config) -> Result<i32> {
    let targets = config.targets.clone();
    if targets.is_empty() {
        bail!("no targets specified");
    }

    // 1. Bootstrap
    let md = bootstrap::initialize_environment(
        &config.void_packages_dir(),
        &config.sudo_bin,
        &config.sudo_flags,
    )?;

    // 2. Load repos
    let repos_conf = ReposConf::load(config.repos_conf_path()).unwrap_or_default();
    let sorted = repos_conf.sorted_by_priority();
    let mut repos: Vec<VurRepo> = Vec::new();
    for (name, entry) in sorted {
        let path = config.vurs_dir().join(&name);
        let repo = VurRepo {
            name: name.clone(),
            path: path.clone(),
            entry: entry.clone(),
        };
        match repo.ensure_cloned() {
            Ok(_) => repos.push(repo),
            Err(e) => tracing::warn!("failed to ensure VUR '{}': {}", name, e),
        }
    }

    // 3. Load indexes
    let mut cache = CacheIndex::load(config.cache_index_path())?;
    let mut vur_map: HashMap<String, (String, VurInfo)> = HashMap::new();
    let mut provides_map: HashMap<String, Vec<(String, VurInfo)>> = HashMap::new();
    let mut binary_check: HashMap<String, bool> = HashMap::new();
    let mut priority_map: HashMap<String, i64> = HashMap::new();

    for repo in &repos {
        let entry = repos_conf.vur.get(&repo.name).unwrap();
        priority_map.insert(repo.name.clone(), entry.priority_or(100));
        let has_binary = entry.has_binary();
        let infos = match repo.load_index(&mut cache, None) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("failed to load index for '{}': {}", repo.name, e);
                continue;
            }
        };
        for info in infos {
            // Record provides
            for prov in &info.provides {
                let key = dep_name(prov).to_string();
                provides_map.entry(key).or_default().push((repo.name.clone(), info.clone()));
            }
            // Main map (first wins by priority since sorted)
            vur_map.entry(info.pkgname.clone()).or_insert((repo.name.clone(), info.clone()));
            for sub in &info.subpackages {
                vur_map.entry(sub.pkgname.clone()).or_insert((repo.name.clone(), info.clone()));
            }
            binary_check.insert(format!("{}:{}", repo.name, info.pkgname), has_binary);
            for sub in &info.subpackages {
                binary_check.insert(format!("{}:{}", repo.name, sub.pkgname), has_binary);
            }
        }
    }
    let _ = cache.save();

    // 4. Build source
    let arch = config.arch();
    // Try xbps-query architecture if not overridden
    let arch = if config.arch_override.is_none() {
        xbps::query_architecture().unwrap_or(arch)
    } else {
        arch
    };

    let binpkgs_root = md.hostdir_binpkgs_root().display().to_string();
    let source = VurSource {
        official_exists_fn: Box::new(move |n| official_exists_remote(n, &binpkgs_root)),
        vur_map,
        provides_map,
        binary_check,
        arch: arch.clone(),
        priority: priority_map,
    };

    let opts = ResolveOptions {
        prefer_binary: config.prefer_binary && !config.force_rebuild,
        force_build: config.force_build || config.force_rebuild,
    };

    let plan = crate::resolver::resolve(&targets, &source, &opts)?;

    if plan.installs.is_empty() && plan.builds.is_empty() {
        println!("nothing to do");
        return Ok(0);
    }

    // 5. Show plan
    let c = &config.color;
    if !plan.installs.is_empty() {
        println!("\n{} Packages to install (binary):", c.bold.paint("::"));
        for item in &plan.installs {
            let src = match &item.action {
                Action::Install(BinarySource::Official) => "official",
                Action::Install(BinarySource::VulBinary { repo }) => repo.as_str(),
                _ => "?",
            };
            println!("  {}/{}-{} [{}]", src, item.name, item.info.version, item.info.pkgver());
        }
    }
    if !plan.builds.is_empty() {
        println!("\n{} Packages to build (source):", c.bold.paint("::"));
        for item in &plan.builds {
            println!("  {}/{}", item.name, item.info.pkgver());
        }
        println!("\n{}", c.warning.paint("Builds are sequential in MVP (xbps-src handles -j internally)"));
    }
    println!();

    if !confirm("Proceed with installation?", config.no_confirm)? {
        return Ok(1);
    }

    // 6. Builds
    let mut built_names: Vec<String> = Vec::new();
    for item in &plan.builds {
        // Ya instalada EXACTAMENTE esa versión => omitir compilación
        // (--force-build / force_rebuild la fuerzan).
        if !config.force_rebuild && !config.force_build {
            match crate::xbps::query_installed(&item.name) {
                Ok(Some(cur)) if cur.pkgver == item.info.pkgver() => {
                    println!(
                        "{} {} ya instalado ({}) ; omitiendo build",
                        c.action.paint("::"),
                        c.bold.paint(&item.name),
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
        let repo = repos.iter().find(|r| r.name == repo_name).ok_or_else(|| anyhow::anyhow!("repo not found for {}", item.name))?;
        // force=true solo para targets explícitos del usuario (o subpaquetes de esos targets)
        let explicit = targets.iter().any(|t| {
            t == &parent_pkg
                || t == &item.name
                || item.info.subpackages.iter().any(|s| s.pkgname == *t)
        });
        repo.project_pkg(&md.srcpkgs_dir(), &parent_pkg, explicit)?;
        let res = md.build_pkg(&parent_pkg);
        // Always unproject (proyectamos parent_pkg)
        let _ = repo.unproject_pkg(&md.srcpkgs_dir(), &parent_pkg);
        res.with_context(|| format!("building {}", parent_pkg))?;
        built_names.push(item.name.clone());
        // También registrar subpaquetes como construidos si el target era el padre
        for sub in &item.info.subpackages {
            if !built_names.contains(&sub.pkgname) {
                built_names.push(sub.pkgname.clone());
            }
        }
    }

    // 7. Installs
    let mut all_install_names: Vec<String> = Vec::new();
    let mut binary_repos_configured: HashSet<String> = HashSet::new();

    for item in &plan.installs {
        match &item.action {
            Action::Install(BinarySource::Official) => all_install_names.push(item.name.clone()),
            Action::Install(BinarySource::VulBinary { repo }) => {
                if !binary_repos_configured.contains(repo) {
                    if let Some(r) = repos.iter().find(|r| &r.name == repo) {
                        let entry = repos_conf.vur.get(repo).unwrap();
                        if let Err(e) = crate::keys::setup_binary_repo(r, entry, &config.sudo_bin, &config.sudo_flags, config.no_confirm) {
                            bail!("failed to setup binary repo '{}': {}", repo, e);
                        }
                    }
                    binary_repos_configured.insert(repo.clone());
                }
                all_install_names.push(item.name.clone());
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
            extra.push(format!("--repository={}", md.hostdir_binpkgs_root().display()));
        }
        if config.no_confirm {
            extra.push("-y".to_string());
        }
        // Also handle --asdeps etc? For now just sync
        let code = xbps::install(&all_install_names, &extra, &config.sudo_bin, &config.sudo_flags)?;
        if code != 0 {
            return Ok(code);
        }
    }

    // 8. Record in db
    let mut db = InstalledDb::load(config.installed_db_path())?;
    for item in plan.installs.iter().chain(plan.builds.iter()) {
        // Only VUR packages
        let is_vur = vur_map_lookup_repo(&item.name, &repos, &mut cache).is_ok();
        if is_vur {
            let repo_name = vur_map_lookup_repo(&item.name, &repos, &mut cache).unwrap_or_else(|_| "unknown".to_string());
            let itype = match &item.action {
                Action::Install(BinarySource::VulBinary { .. }) => InstallType::Binary,
                Action::Build => InstallType::Source,
                _ => InstallType::Source,
            };
            db.upsert(&item.name, &item.info.pkgver(), &repo_name, itype.clone());
            for sub in &item.info.subpackages {
                db.upsert(&sub.pkgname, &item.info.pkgver(), &repo_name, itype.clone());
            }
        }
    }
    db.save()?;

    Ok(0)
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
    bail!("no VUR repo found for {}", name)
}

pub fn download_only(config: &mut Config) -> Result<i32> {
    let targets = config.targets.clone();
    if targets.is_empty() {
        bail!("no targets specified");
    }
    let md = bootstrap::initialize_environment(
        &config.void_packages_dir(),
        &config.sudo_bin,
        &config.sudo_flags,
    )?;

    let repos_conf = ReposConf::load(config.repos_conf_path()).unwrap_or_default();
    let mut repos: Vec<VurRepo> = Vec::new();
    for (name, entry) in repos_conf.sorted_by_priority() {
        let path = config.vurs_dir().join(&name);
        let repo = VurRepo { name: name.clone(), path, entry: entry.clone() };
        if repo.ensure_cloned().is_ok() {
            repos.push(repo);
        }
    }
    let mut cache = CacheIndex::load(config.cache_index_path())?;
    for target in &targets {
        // Buscar en todos los repos (incluye subpaquetes => compilar al padre)
        let mut found = false;
        'repos: for repo in &repos {
            let Ok(infos) = repo.load_index(&mut cache, None) else { continue };
            for info in &infos {
                let is_match = info.pkgname == *target
                    || info.subpackages.iter().any(|s| s.pkgname == *target);
                if is_match {
                    let parent = info.pkgname.clone();
                    repo.project_pkg(&md.srcpkgs_dir(), &parent, false)?;
                    md.fetch_pkg(&parent)?;
                    repo.unproject_pkg(&md.srcpkgs_dir(), &parent)?;
                    println!("Fetched {} (parent {})", target, parent);
                    found = true;
                    break 'repos;
                }
            }
        }
        if !found {
            println!("{} not found in VURs (or is official)", target);
        }
    }
    Ok(0)
}
