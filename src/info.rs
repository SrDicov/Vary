use crate::cache::CacheIndex;
use crate::config::Config;
use crate::reposconf::ReposConf;
use crate::vur_client::VurRepo;
use anyhow::Result;

pub fn info(config: &Config) -> Result<i32> {
    if config.targets.is_empty() {
        anyhow::bail!("no targets specified for -Si");
    }

    let repos_conf = ReposConf::load(config.repos_conf_path())?;
    let mut cache = CacheIndex::load(config.cache_index_path())?;
    let ttl = Some(config.ttl_cache_seconds);

    let mut any_found = false;
    let mut exit_code = 0;

    for target in &config.targets {
        let mut found = false;

        // Try official (xbps-query -R)
        let official = std::process::Command::new("xbps-query")
            .args(["-R", "-p", "pkgver,short_desc,homepage,maintainer,depends", target])
            .output();
        if let Ok(out) = official {
            if out.status.success() && !out.stdout.is_empty() {
                let s = String::from_utf8_lossy(&out.stdout);
                println!("Repository      : official");
                println!("Package         : {}", target);
                println!("Info            : {}", s.trim());
                println!();
                found = true;
                any_found = true;
            }
        }

        // Search VUR indexes
        for (name, entry) in repos_conf.sorted_by_priority() {
            let path = config.vurs_dir().join(&name);
            let repo = VurRepo { name: name.clone(), path, entry: entry.clone(), git_bin: config.git_bin.clone() };
            if repo.ensure_cloned().is_err() {
                continue;
            }
            let infos = match repo.load_index(&mut cache, ttl) {
                Ok(v) => v,
                Err(_) => continue,
            };
            for info in infos {
                let is_match = info.pkgname == *target || info.subpackages.iter().any(|s| &s.pkgname == target);
                if is_match {
                    println!("Repository      : vur:{}", repo.name);
                    println!("Package         : {}", info.pkgname);
                    if info.subpackages.iter().any(|s| &s.pkgname == target) {
                        println!("Subpackage      : {}", target);
                    }
                    println!("Version         : {}", info.version);
                    println!("Revision        : {}", info.revision);
                    println!("Pkgver          : {}", info.pkgver());
                    println!("Archs           : {}", info.archs.join(", "));
                    if let Some(bs) = &info.build_style {
                        println!("Build style     : {}", bs);
                    }
                    if !info.depends.is_empty() {
                        println!("Depends         : {}", info.depends.join(" "));
                    }
                    if !info.hostmakedepends.is_empty() {
                        println!("Hostmakedeps    : {}", info.hostmakedepends.join(" "));
                    }
                    if !info.makedepends.is_empty() {
                        println!("Makedepends     : {}", info.makedepends.join(" "));
                    }
                    if !info.provides.is_empty() {
                        println!("Provides        : {}", info.provides.join(" "));
                    }
                    if !info.replaces.is_empty() {
                        println!("Replaces        : {}", info.replaces.join(" "));
                    }
                    if let Some(m) = &info.maintainer {
                        println!("Maintainer      : {}", m);
                    }
                    if !info.subpackages.is_empty() {
                        println!("Subpackages     : {}", info.subpackages.iter().map(|s| s.pkgname.as_str()).collect::<Vec<_>>().join(", "));
                    }
                    println!("Restricted      : {}", info.restricted);
                    println!("Binary avail    : {}", if entry.has_binary() { "yes" } else { "no (source only)" });
                    println!();
                    found = true;
                    any_found = true;
                }
            }
        }

        if !found {
            eprintln!("package '{}' not found in official repos nor VURs", target);
            exit_code = 1;
        }
    }

    if !any_found {
        return Ok(exit_code);
    }
    Ok(exit_code)
}
