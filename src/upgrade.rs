use crate::cache::CacheIndex;
use crate::config::Config;
use crate::db::InstalledDb;
use crate::reposconf::ReposConf;
use crate::vur_client::VurRepo;
use crate::xbps;
use anyhow::Result;

pub fn refresh_repos(config: &Config) -> Result<i32> {
    let repos_conf = ReposConf::load(config.repos_conf_path()).unwrap_or_default();
    let mut any_failed = false;
    for (name, entry) in repos_conf.sorted_by_priority() {
        if !entry.enabled_or(true) {
            continue;
        }
        let path = config.vurs_dir().join(&name);
        let repo = VurRepo { name: name.clone(), path, entry: entry.clone(), git_bin: config.git_bin.clone() };
        if let Err(e) = repo.ensure_cloned() {
            tracing::warn!("failed to clone VUR '{}': {}", name, e);
            any_failed = true;
            continue;
        }
        match repo.pull() {
            Ok(sha) => println!("VUR '{}' refreshed ({})", name, &sha[..8]),
            Err(e) => {
                tracing::warn!("failed to pull VUR '{}': {}", name, e);
                any_failed = true;
            }
        }
    }
    if any_failed { Ok(1) } else { Ok(0) }
}

pub fn upgrade(config: &mut Config) -> Result<i32> {
    // Phase 1: official upgrade
    println!(":: Upgrading official packages...");
    let mut extra = vec!["-u".to_string()];
    if config.no_confirm {
        extra.push("-y".to_string());
    }
    let code = xbps::install(&[], &extra, &config.sudo_bin, &config.sudo_flags)?;
    if code != 0 {
        return Ok(code);
    }

    // Phase 2: refresh VUR repos
    println!(":: Refreshing VUR repositories...");
    refresh_repos(config)?;

    // Phase 3: detect VUR upgrades via installed.json
    let mut db = InstalledDb::load(config.installed_db_path())?;
    let mut cache = CacheIndex::load(config.cache_index_path())?;
    let repos_conf = ReposConf::load(config.repos_conf_path()).unwrap_or_default();

    // Build map of current VUR pkgver
    let mut current_map: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for (name, entry) in repos_conf.sorted_by_priority() {
        let path = config.vurs_dir().join(&name);
        let repo = VurRepo { name: name.clone(), path, entry: entry.clone(), git_bin: config.git_bin.clone() };
        if repo.ensure_cloned().is_err() {
            continue;
        }
        if let Ok(infos) = repo.load_index(&mut cache, Some(config.ttl_cache_seconds)) {
            for info in infos {
                current_map.entry(info.pkgname.clone()).or_insert_with(|| info.pkgver());
                for sub in &info.subpackages {
                    current_map.entry(sub.pkgname.clone()).or_insert_with(|| info.pkgver());
                }
            }
        }
    }
    let _ = cache.save();

    // Prune db entries for packages no longer installed
    let installed_names: Vec<String> = xbps::query_manual().unwrap_or_default();
    let manual_set: std::collections::HashSet<String> = installed_names.into_iter().collect();
    let before = db.len();
    db.retain(|name, _| {
        // Keep only if still installed or we can't determine (fallback keep)
        manual_set.contains(name) || {
            // Also check via xbps-query -l
            xbps::query_installed(name).map(|o| o.is_some()).unwrap_or(true)
        }
    });
    if db.len() != before {
        let _ = db.save();
    }

    let mut outdated: Vec<String> = Vec::new();
    for (name, entry) in db.entries_snapshot() {
        if let Some(cur) = current_map.get(&name) {
            if cur != &entry.version {
                println!("  {} {} -> {}", name, entry.version, cur);
                outdated.push(name.clone());
            }
        }
    }

    if outdated.is_empty() {
        println!("VUR packages up to date.");
        return Ok(0);
    }

    println!("\nVUR upgrades available: {}", outdated.join(", "));
    if !crate::util::ask(config, "Upgrade VUR packages?", true) {
        return Ok(1);
    }

    config.targets = outdated;
    crate::install::install(config)
}
