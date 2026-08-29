use crate::command_line::RepoCmd;
use crate::config::Config;
use crate::keys::teardown_binary_repo;
use crate::reposconf::{RepoEntry, ReposConf};
use crate::vur_client::VurRepo;
use anyhow::{bail, Context, Result};

fn derive_name_from_url(url: &str) -> String {
    let url = url.trim_end_matches('/');
    let last = url.rsplit('/').next().unwrap_or("vur");
    last.trim_end_matches(".git").to_string()
}

pub fn handle_repo_cmd(config: &Config, cmd: RepoCmd) -> Result<i32> {
    match cmd {
        RepoCmd::Add { url, name } => repo_add(config, &url, name.as_deref()),
        RepoCmd::List => repo_list(config),
        RepoCmd::Remove { name, purge } => repo_remove(config, &name, purge),
        RepoCmd::Rekey(name) => repo_rekey(config, &name),
    }
}

fn repo_add(config: &Config, url: &str, name_opt: Option<&str>) -> Result<i32> {
    let name = name_opt
        .map(|s| s.to_string())
        .unwrap_or_else(|| derive_name_from_url(url));

    if name.is_empty() {
        bail!("could not derive repo name from url; please provide a name");
    }

    let vurs_dir = config.vurs_dir();
    std::fs::create_dir_all(&vurs_dir).context("creating vurs dir")?;
    let dest = vurs_dir.join(&name);

    let entry = RepoEntry {
        url: url.to_string(),
        branch: Some("main".to_string()),
        priority: Some(100),
        key_fingerprint: None,
        binary_repo_url: None,
        enabled: Some(true),
    };

    let repo = VurRepo {
        name: name.clone(),
        path: dest.clone(),
        entry: entry.clone(),
        git_bin: config.git_bin.clone(),
    };

    println!("Cloning VUR '{}' from {}...", name, url);
    repo.ensure_cloned()
        .with_context(|| format!("cloning VUR {}", name))?;

    // Validate it has at least one .VURINFO
    let has_vurinfo = dest.join(".VURINFO").exists()
        || std::fs::read_dir(dest.join("srcpkgs"))
            .map(|mut d| d.any(|e| e.map(|e| e.path().join(".VURINFO").exists()).unwrap_or(false)))
            .unwrap_or(false);
    if !has_vurinfo {
        tracing::warn!("VUR '{}' has no .VURINFO (neither root nor srcpkgs/*/.VURINFO)", name);
    }

    // Save to repos.conf
    let mut conf = ReposConf::load(config.repos_conf_path()).unwrap_or_default();
    conf.vur.insert(name.clone(), entry);
    conf.save(config.repos_conf_path())?;
    println!("VUR '{}' registered.", name);

    // Show key if present
    if let Some(key) = repo.discover_public_key() {
        println!("Found public key: {}", key.display());
        if let Ok(fp) = VurRepo::fingerprint_sha256(&key) {
            println!("  SHA256 fingerprint: {}", fp);
            println!("  To use binary packages from this VUR, add binary_repo_url and key_fingerprint to repos.conf");
        }
    }

    Ok(0)
}

fn repo_list(config: &Config) -> Result<i32> {
    let conf = ReposConf::load(config.repos_conf_path()).unwrap_or_default();
    if conf.vur.is_empty() {
        println!("No VURs configured. Add one with: vary --repo add <url>");
        return Ok(0);
    }
    println!("{:<20} {:<8} {:<45} {}", "NAME", "PRIO", "URL", "BINARY");
    for (name, entry) in conf.sorted_by_priority() {
        let prio = entry.priority_or(100);
        let binary = if entry.has_binary() { "yes" } else { "no" };
        let enabled = if entry.enabled_or(true) { "" } else { " (disabled)" };
        println!("{:<20} {:<8} {:<45} {}{}", name, prio, entry.url, binary, enabled);
    }
    Ok(0)
}

fn repo_remove(config: &Config, name: &str, purge: bool) -> Result<i32> {
    let mut conf = ReposConf::load(config.repos_conf_path()).unwrap_or_default();
    let in_conf = conf.vur.contains_key(name);
    let clone_path = config.vurs_dir().join(name);

    // Permitir purgar un clon huérfano (repo ya dado de baja sin -p): si no
    // está en repos.conf pero su clon existe, se puede eliminar con -p.
    if !in_conf && !clone_path.exists() {
        bail!("VUR '{}' not found in repos.conf", name);
    }

    if in_conf {
        // Tear down binary repo artifacts (keys/conf)
        let _ = teardown_binary_repo(name, &config.sudo_bin, &config.sudo_flags);

        conf.vur.remove(name);
        conf.save(config.repos_conf_path())?;
        println!("VUR '{}' removed from repos.conf", name);
    }

    if clone_path.exists() {
        if purge {
            std::fs::remove_dir_all(&clone_path)
                .with_context(|| format!("removing {}", clone_path.display()))?;
            println!("Clone purged.");
        } else {
            println!(
                "Clone kept at {}. Use --repo remove {} -p to purge it.",
                clone_path.display(),
                name
            );
        }
    }

    Ok(0)
}

fn repo_rekey(config: &Config, name: &str) -> Result<i32> {
    let conf = ReposConf::load(config.repos_conf_path()).unwrap_or_default();
    let entry = conf.vur.get(name).ok_or_else(|| anyhow::anyhow!("VUR '{}' not found", name))?;

    teardown_binary_repo(name, &config.sudo_bin, &config.sudo_flags)?;
    println!("Binary repo artifacts for '{}' removed. They will be re-registered on next binary install.", name);

    if entry.has_binary() {
        println!("Tip: run `vary -S <pkg>` from this VUR to re-trigger key verification.");
    }
    Ok(0)
}
