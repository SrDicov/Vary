use crate::command_line::RepoCmd;
use crate::config::Config;
use crate::keys::teardown_binary_repo;
use crate::reposconf::{RepoEntry, ReposConf};
use crate::vur_client::VurRepo;
use anyhow::{bail, Context, Result};
use std::process::Command;

fn derive_name_from_url(url: &str) -> String {
    let url = url.trim_end_matches('/');
    let last = url.rsplit('/').next().unwrap_or("vur");
    last.trim_end_matches(".git").to_string()
}

pub fn handle_repo_cmd(config: &Config, cmd: RepoCmd) -> Result<i32> {
    match cmd {
        RepoCmd::Add { url, name, branch, index_url } => {
            repo_add(config, &url, name.as_deref(), branch.as_deref(), index_url.as_deref())
        }
        RepoCmd::List => repo_list(config),
        RepoCmd::Remove { name, purge } => repo_remove(config, &name, purge),
        RepoCmd::Rekey(name) => repo_rekey(config, &name),
    }
}

fn repo_add(
    config: &Config,
    url: &str,
    name_opt: Option<&str>,
    branch_opt: Option<&str>,
    index_url_opt: Option<&str>,
) -> Result<i32> {
    let name = name_opt
        .map(|s| s.to_string())
        .unwrap_or_else(|| derive_name_from_url(url));

    crate::keys::validate_repo_name(&name)?;

    if !crate::vur_client::is_safe_git_url(url) {
        bail!("URL de repositorio git insegura o inválida: '{}'", url);
    }

    // Validar repos.conf antes de cualquier operación remota o de disco
    let mut conf = ReposConf::load(config.repos_conf_path())?;

    // Rama: flag explícita > autodetección del remoto > "main".
    // (z-packages y otros repos clásicos viven en `master`.)
    let branch = match branch_opt.map(str::trim).filter(|s| !s.is_empty()) {
        Some(b) => b.to_string(),
        None => match VurRepo::detect_default_branch(&config.git_bin, url) {
            Some(b) => {
                println!("Detected default branch '{}' for {}", b, url);
                b
            }
            None => {
                tracing::warn!(
                    "no se pudo detectar la rama por defecto de {}; usando 'main' \
                     (si el clon falla, repite con --branch <rama>)",
                    url
                );
                "main".to_string()
            }
        },
    };

    let vurs_dir = config.vurs_dir();
    std::fs::create_dir_all(&vurs_dir).context("creating vurs dir")?;
    let dest = vurs_dir.join(&name);

    let entry = RepoEntry {
        url: url.to_string(),
        branch: Some(branch),
        priority: Some(100),
        key_fingerprint: None,
        binary_repo_url: None,
        index_url: index_url_opt
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string()),
        enabled: Some(true),
    };

    let repo = VurRepo {
        name: name.clone(),
        path: dest.clone(),
        entry: entry.clone(),
        git_bin: config.git_bin.clone(),
    };

    println!("Cloning VUR '{}' from {} (branch {})...", name, url, entry.branch_or_default());
    repo.ensure_cloned().with_context(|| {
        format!(
            "cloning VUR {} (branch {}); si el repo usa otra rama, repite con --branch <rama>",
            name,
            entry.branch_or_default()
        )
    })?;

    // El clon es --no-checkout: inspeccionar el object store, no el worktree.
    // Vale tanto el .VURINFO raíz (array, p. ej. z-packages) como los
    // <prefijo>/*/.VURINFO por plantilla (srcpkgs/ o su alias pkgs/).
    if !repo_has_vurinfo(&repo) {
        tracing::warn!("VUR '{}' has no .VURINFO (neither root nor per-template)", name);
    }

    // Save to repos.conf
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

/// ¿El repo publica algún índice .VURINFO?
///
/// Inspecciona el object store (`git ls-tree -r HEAD`): los clones de vary
/// son `--no-checkout`, así que mirar el worktree siempre daría falso.
/// Detecta tanto el `.VURINFO` raíz (array) como los per-template
/// (`srcpkgs/*/.VURINFO` o su alias `pkgs/*/.VURINFO`).
fn repo_has_vurinfo(repo: &VurRepo) -> bool {
    let output = Command::new(&repo.git_bin)
        .arg("-C")
        .arg(&repo.path)
        .args(["ls-tree", "-r", "--name-only", "HEAD"])
        .output();
    let Ok(out) = output else {
        return false;
    };
    if !out.status.success() {
        return false;
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .any(|l| l == ".VURINFO" || l.ends_with("/.VURINFO"))
}

fn repo_list(config: &Config) -> Result<i32> {
    let conf = ReposConf::load(config.repos_conf_path())?;
    if conf.vur.is_empty() {
        println!("No VURs configured. Add one with: vary --repo add <url>");
        return Ok(0);
    }
    println!("{:<20} {:<8} {:<45} BINARY", "NAME", "PRIO", "URL");
    for (name, entry) in conf.sorted_by_priority() {
        let prio = entry.priority_or(100);
        let binary = if entry.has_binary() { "yes" } else { "no" };
        let enabled = if entry.enabled_or(true) { "" } else { " (disabled)" };
        println!("{:<20} {:<8} {:<45} {}{}", name, prio, entry.url, binary, enabled);
    }
    Ok(0)
}

fn repo_remove(config: &Config, name: &str, purge: bool) -> Result<i32> {
    crate::keys::validate_repo_name(name)?;
    let mut conf = ReposConf::load(config.repos_conf_path())?;
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
    crate::keys::validate_repo_name(name)?;
    let conf = ReposConf::load(config.repos_conf_path())?;
    let entry = conf.vur.get(name).ok_or_else(|| anyhow::anyhow!("VUR '{}' not found", name))?;

    teardown_binary_repo(name, &config.sudo_bin, &config.sudo_flags)?;
    println!("Binary repo artifacts for '{}' removed. They will be re-registered on next binary install.", name);

    if entry.has_binary() {
        println!("Tip: run `vary -S <pkg>` from this VUR to re-trigger key verification.");
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn repo_add_fails_and_preserves_corrupt_repos_conf() {
        let tmp = tempdir().unwrap();
        let config = Config {
            config_dir: tmp.path().to_path_buf(),
            data_dir: tmp.path().join("data"),
            ..Default::default()
        };

        let conf_path = config.repos_conf_path();
        let bad_content = "[vur.broken\nurl = \nnot valid toml :::";
        std::fs::write(&conf_path, bad_content).unwrap();

        let res = repo_add(
            &config,
            "https://example.com/test.git",
            Some("test"),
            Some("main"),
            None,
        );
        assert!(res.is_err(), "repo_add debe fallar si repos.conf está corrupto");

        let content_after = std::fs::read_to_string(&conf_path).unwrap();
        assert_eq!(
            content_after, bad_content,
            "El archivo corrupto debe preservarse intacto sin sobreescribirse"
        );
    }

    #[test]
    fn repo_remove_fails_on_corrupt_repos_conf() {
        let tmp = tempdir().unwrap();
        let config = Config {
            config_dir: tmp.path().to_path_buf(),
            ..Default::default()
        };

        let conf_path = config.repos_conf_path();
        let bad_content = "[vur.broken\nurl = \nnot valid toml :::";
        std::fs::write(&conf_path, bad_content).unwrap();

        let res = repo_remove(&config, "test", false);
        assert!(res.is_err(), "repo_remove debe fallar si repos.conf está corrupto");

        let content_after = std::fs::read_to_string(&conf_path).unwrap();
        assert_eq!(
            content_after, bad_content,
            "El archivo corrupto debe preservarse intacto"
        );
    }

    #[test]
    fn repo_list_fails_on_corrupt_repos_conf() {
        let tmp = tempdir().unwrap();
        let config = Config {
            config_dir: tmp.path().to_path_buf(),
            ..Default::default()
        };

        let conf_path = config.repos_conf_path();
        let bad_content = "[vur.broken\nurl = \nnot valid toml :::";
        std::fs::write(&conf_path, bad_content).unwrap();

        let res = repo_list(&config);
        assert!(res.is_err(), "repo_list debe fallar si repos.conf está corrupto");
    }
}
