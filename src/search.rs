use crate::cache::CacheIndex;
use crate::config::Config;
use crate::reposconf::ReposConf;
use crate::vur_client::VurRepo;
use crate::xbps;
use anyhow::Result;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Rank {
    Official = 0,
    VulBinary = 1,
    Source = 2,
}

struct Row {
    name: String,
    pkgver: String,
    desc: String,
    rank: Rank,
    repo: String,
}

pub fn search(config: &Config) -> Result<i32> {
    let pattern = config.targets.join(" ");
    if pattern.is_empty() {
        anyhow::bail!("no search pattern specified");
    }
    // Búsqueda multi-palabra: cada palabra debe coincidir (AND) con
    // nombre o descripción.
    let words: Vec<String> = pattern
        .split_whitespace()
        .map(|w| w.to_lowercase())
        .filter(|w| !w.is_empty())
        .collect();

    // Coincide si TODAS las palabras aparecen en nombre o descripción.
    fn matches_words(words: &[String], name: &str, desc: &str) -> bool {
        if words.is_empty() {
            return false;
        }
        let name_lc = name.to_lowercase();
        let desc_lc = desc.to_lowercase();
        words
            .iter()
            .all(|w| name_lc.contains(w) || desc_lc.contains(w))
    }

    let mut rows: BTreeMap<String, Row> = BTreeMap::new();

    // Official search SOLO contra repos oficiales del sistema (excluye el
    // repo local binpkgs de vary: nuestros builds no son "oficiales").
    match xbps::search_official(&pattern) {
        Ok(pkgs) => {
            for p in pkgs {
                let repo_raw = p.repository.clone().unwrap_or_default();
                if repo_raw.contains("hostdir/binpkgs") || repo_raw.starts_with('/') {
                    continue;
                }
                let desc = p.short_desc.clone().unwrap_or_default();
                if !matches_words(&words, &p.name, &desc) {
                    continue;
                }
                rows.entry(p.name.clone()).or_insert(Row {
                    name: p.name.clone(),
                    pkgver: p.pkgver.clone(),
                    desc,
                    rank: Rank::Official,
                    repo: repo_raw.is_empty().then(|| "official".to_string()).unwrap_or(repo_raw),
                });
            }
        }
        Err(e) => tracing::warn!("xbps search failed: {}", e),
    }

    // VUR search (federated, sequential for MVP simplicity)
    let repos_conf = ReposConf::load(config.repos_conf_path()).unwrap_or_default();
    let mut cache = CacheIndex::load(config.cache_index_path())?;
    let ttl = Some(config.ttl_cache_seconds);

    for (name, entry) in repos_conf.sorted_by_priority() {
        if !entry.enabled_or(true) {
            continue;
        }
        let path = config.vurs_dir().join(&name);
        let repo = VurRepo { name: name.clone(), path, entry: entry.clone() };
        if repo.ensure_cloned().is_err() {
            continue;
        }
        let infos = match repo.load_index(&mut cache, ttl) {
            Ok(v) => v,
            Err(_) => continue,
        };
        for info in infos {
            let candidates = std::iter::once((info.pkgname.clone(), None::<&crate::metadata::Subpackage>))
                .chain(info.subpackages.iter().map(|s| (s.pkgname.clone(), Some(s))));
            for (pkgname, sub) in candidates {
                let desc_owned: String = sub
                    .and_then(|s| s.short_desc.clone())
                    .unwrap_or_else(|| {
                        // descripción del padre: short_desc no está en VurInfo base;
                        // usamos build_style/version como fallback informativo
                        format!(
                            "{} {}",
                            info.version,
                            info.build_style.clone().unwrap_or_default()
                        )
                    });
                if !matches_words(&words, &pkgname, &desc_owned) {
                    continue;
                }
                let rank = if entry.has_binary() { Rank::VulBinary } else { Rank::Source };
                let repo_label = if rank == Rank::VulBinary {
                    format!("vur:{}", name)
                } else {
                    format!("vur-source:{}", name)
                };
                let row = Row {
                    name: pkgname.clone(),
                    pkgver: info.pkgver(),
                    desc: desc_owned,
                    rank: rank.clone(),
                    repo: repo_label,
                };
                // Dedup: keep lowest rank (Official wins)
                rows.entry(pkgname.clone())
                    .and_modify(|existing| {
                        if row.rank < existing.rank {
                            *existing = Row { name: row.name.clone(), pkgver: row.pkgver.clone(), desc: row.desc.clone(), rank: row.rank.clone(), repo: row.repo.clone() };
                        }
                    })
                    .or_insert(row);
            }
        }
    }

    if rows.is_empty() {
        // Persistir igualmente lo aprendido para próximas búsquedas
        let _ = cache.save();
        println!("No packages found for '{}'", pattern);
        return Ok(1);
    }

    // Guardar índices recién parseados para acelerar búsquedas futuras
    let _ = cache.save();

    let c = &config.color;
    for row in rows.values() {
        if config.quiet {
            println!("{}", row.name);
        } else {
            let repo_colored = c.sl_repo.paint(&row.repo);
            let name_colored = c.ss_name.paint(&row.name);
            let ver_colored = c.ss_ver.paint(&row.pkgver);
            println!("{} {} {}  {}", repo_colored, name_colored, ver_colored, row.desc);
        }
    }

    Ok(0)
}
