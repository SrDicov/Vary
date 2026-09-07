use crate::command_line::RepoCmd;
use crate::config::Config;
use crate::keys::teardown_binary_repo;
use crate::reposconf::{RepoEntry, ReposConf};
use crate::util::confirm;
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
        RepoCmd::Add {
            url,
            name,
            branch,
            index_url,
        } => repo_add(
            config,
            &url,
            name.as_deref(),
            branch.as_deref(),
            index_url.as_deref(),
        ),
        RepoCmd::List => repo_list(config),
        RepoCmd::Remove { name, purge } => repo_remove(config, &name, purge),
        RepoCmd::Rekey(name) => repo_rekey(config, &name),
        RepoCmd::Retrust(name) => repo_retrust(config, &name),
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
        bail!("URL de repositorio git insegura o inválida: '{url}'");
    }

    // Validar repos.conf antes de cualquier operación remota o de disco
    let mut conf = ReposConf::load(config.repos_conf_path())?;

    // Rama: flag explícita > autodetección del remoto > "main".
    // (z-packages y otros repos clásicos viven en `master`.)
    let branch = match branch_opt.map(str::trim).filter(|s| !s.is_empty()) {
        Some(b) => b.to_string(),
        None => match VurRepo::detect_default_branch(&config.git_bin, url) {
            Some(b) => {
                println!("Detected default branch '{b}' for {url}");
                b
            }
            None => {
                tracing::warn!(
                    "no se pudo detectar la rama por defecto de {url}; usando 'main' \
                     (si el clon falla, repite con --branch <rama>)"
                );
                "main".to_string()
            }
        },
    };

    let vurs_dir = config.vurs_dir();
    crate::util::ensure_private_dir(&vurs_dir).context("creating vurs dir")?;
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
        // P0-2: sin llave aún (se fija al primer registro binario).
        trusted_at: None,
    };

    let repo = VurRepo {
        name: name.clone(),
        path: dest.clone(),
        entry: entry.clone(),
        git_bin: config.git_bin.clone(),
    };

    println!(
        "Cloning VUR '{}' from {} (branch {})...",
        name,
        url,
        entry.branch_or_default()
    );
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
        tracing::warn!("VUR '{name}' has no .VURINFO (neither root nor per-template)");
    }

    // Save to repos.conf
    conf.vur.insert(name.clone(), entry);
    conf.save(config.repos_conf_path())?;
    println!("VUR '{name}' registered.");

    // Show key if present (fingerprint estilo xbps: es el que xbps muestra
    // y el que se pinea en key_fingerprint).
    if let Some(key) = repo.discover_public_key() {
        println!("Found public key: {}", key.display());
        if let Ok(pem) = std::fs::read_to_string(&key) {
            if let Ok(fp) = crate::keys::xbps_fingerprint_pem(&pem) {
                println!("  Fingerprint (xbps): {fp}");
                println!("  To use binary packages from this VUR, add binary_repo_url and key_fingerprint to repos.conf");
            }
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
        let enabled = if entry.enabled_or(true) {
            ""
        } else {
            " (disabled)"
        };
        println!(
            "{:<20} {:<8} {:<45} {}{}",
            name, prio, entry.url, binary, enabled
        );
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
        bail!("VUR '{name}' not found in repos.conf");
    }

    if in_conf {
        // Tear down binary repo artifacts (keys/conf)
        let _ = teardown_binary_repo(name, &config.sudo_bin, &config.sudo_flags);

        conf.vur.remove(name);
        conf.save(config.repos_conf_path())?;
        println!("VUR '{name}' removed from repos.conf");
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
    let entry = conf
        .vur
        .get(name)
        .ok_or_else(|| anyhow::anyhow!("VUR '{name}' not found"))?;

    teardown_binary_repo(name, &config.sudo_bin, &config.sudo_flags)?;
    // P0-2: rekey = des-confiar (borra la fecha; el próximo registro la fija).
    if let Err(e) = crate::keys::stamp_trust(&config.repos_conf_path(), name, None, false) {
        tracing::warn!("no se pudo borrar la fecha de confianza de '{name}': {e:#}");
    }
    println!("Binary repo artifacts for '{name}' removed. They will be re-registered on next binary install.");

    if entry.has_binary() {
        println!("Tip: run `vary -S <pkg>` from this VUR to re-trigger key verification.");
    }
    Ok(0)
}

/// P0-2: `vary repo re-trust <name>` — reafirma la llave ACTUAL del clon.
///
/// Ceremonia explícita para rotaciones legítimas (lo que el abort forense
/// pide): muestra vieja→nueva + fecha original, exige confirmación
/// INTERACTIVA (con `--noconfirm` se rechaza: re-confiar a ciegas mentiría
/// sobre la verificación), retira lo viejo, registra lo nuevo y fija pin +
/// fecha. Si cambió la URL se niega (`re-trust` no mueve el origen del
/// clon: remove `-p` + add).
fn repo_retrust(config: &Config, name: &str) -> Result<i32> {
    crate::keys::validate_repo_name(name)?;
    let mut conf = ReposConf::load(config.repos_conf_path())?;
    let entry = conf
        .vur
        .get(name)
        .ok_or_else(|| anyhow::anyhow!("VUR '{name}' not found"))?
        .clone();
    if !entry.has_binary() && !entry.has_vup_index() {
        anyhow::bail!("'{name}' no es un repo binario (sin binary_repo_url ni index_url): nada que re-confiar");
    }
    let repo = VurRepo {
        name: name.to_string(),
        path: config.vurs_dir().join(name),
        entry: entry.clone(),
        git_bin: config.git_bin.clone(),
    };
    // La URL no la mueve re-trust: si el origen del clon difiere, re-clonar.
    if let Some(origin) = crate::keys::clone_origin_url(&config.git_bin, &repo.path) {
        if crate::keys::normalize_repo_url(&origin) != crate::keys::normalize_repo_url(&entry.url) {
            anyhow::bail!(
                "la URL de '{name}' cambió (origen del clon: {origin}; repos.conf: {}): \
                 `re-trust` no mueve el origen; re-clona con `vary --repo remove {name} -p` + `vary --repo add <url>`",
                entry.url
            );
        }
    }
    // Llave actual del clon (mismos lectores que install, con fallback fs).
    let (key_pem, key_plist): (String, Option<String>) = if entry.has_vup_index() {
        let plist = crate::vup_index::read_repo_plist_text_git(&repo.git_bin, &repo.path)
            .or_else(|_| crate::vup_index::read_repo_plist_text(&repo.path))?;
        let pem = crate::vup_index::decode_plist_public_key_pem(&plist)?;
        (pem, Some(plist))
    } else {
        let path = repo.discover_public_key().ok_or_else(|| {
            anyhow::anyhow!("'{name}' no publica llave en keys/ (¿clon sparse sin materializar?)")
        })?;
        let pem = std::fs::read_to_string(&path)
            .with_context(|| format!("leyendo {}", path.display()))?;
        (pem, None)
    };
    let new_fp = crate::keys::xbps_fingerprint_pem(&key_pem)?;
    let old_fp = crate::keys::key_dest_path(name)
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|pem| crate::keys::xbps_fingerprint_pem(&pem).ok());
    match &old_fp {
        Some(old) if old.to_lowercase() == new_fp.to_lowercase() => {
            println!("La llave de '{name}' no cambió ({new_fp}): se reafirma la confianza.");
        }
        _ => {
            println!("Rotación de llave en '{name}':");
            println!(
                "  instalada: {}",
                old_fp.as_deref().unwrap_or("(ninguna: registro fresco)")
            );
            println!("  actual:    {new_fp}");
            println!(
                "  confiada:  {}",
                crate::keys::fmt_trusted_at(entry.trusted_at)
            );
        }
    }
    // Confirmación interactiva OBLIGATORIA: la verificación es visual y
    // fuera de banda; aceptarla con --noconfirm mentiría (precedente H-032).
    if config.no_confirm {
        anyhow::bail!(
            "re-trust exige confirmación interactiva de la llave (verifica el fingerprint fuera de banda); \
             re-ejecuta sin --noconfirm/--yes"
        );
    }
    if !confirm(
        "¿Confías en esta llave y deseas registrarla (pin + fecha actualizados)?",
        false,
    )? {
        anyhow::bail!("re-trust cancelado por el usuario");
    } // Retirar lo viejo (tolerante si no hay nada) y registrar lo nuevo con
      // el pin actualizado. El pin se persiste SOLO si el setup tiene éxito
      // (si falla, repos.conf queda intacta).
    crate::keys::teardown_binary_repo(name, &config.sudo_bin, &config.sudo_flags)?;
    let mut updated = entry.clone();
    updated.key_fingerprint = Some(new_fp.clone());
    if entry.has_vup_index() {
        let urls = retrust_vup_urls(config, &repo, &entry)?;
        let plist = key_plist.ok_or_else(|| anyhow::anyhow!("interno: falta plist VUP"))?;
        // `true` = no re-preguntar (la confirmación interactiva ya se hizo arriba).
        crate::keys::setup_vup_binary_repo(
            name,
            &urls,
            &key_pem,
            &plist,
            &updated,
            &config.sudo_bin,
            &config.sudo_flags,
            &config.tools_install_bin,
            true,
        )?;
    } else {
        crate::keys::setup_binary_repo(
            &repo,
            &updated,
            &config.sudo_bin,
            &config.sudo_flags,
            &config.tools_install_bin,
            true,
        )?;
    }
    updated.trusted_at = Some(crate::keys::now_epoch());
    conf.vur.insert(name.to_string(), updated);
    conf.save(config.repos_conf_path())?;
    println!("Confianza renovada para '{name}' (pin + fecha actualizados).");
    Ok(0)
}

/// P0-2: URLs binarias VUP para re-trust: reutiliza las ya registradas en
/// el conf (si existe); si no (post-rekey), las deriva del índice remoto.
fn retrust_vup_urls(config: &Config, repo: &VurRepo, entry: &RepoEntry) -> Result<Vec<String>> {
    if let Ok(text) = std::fs::read_to_string(format!("/etc/xbps.d/20-vur-{}.conf", repo.name)) {
        let urls: Vec<String> = text
            .lines()
            .filter_map(|l| l.trim().strip_prefix("repository="))
            .map(str::trim)
            .filter(|u| !u.is_empty())
            .map(str::to_string)
            .collect();
        if !urls.is_empty() {
            return Ok(urls);
        }
    }
    let index_url = entry
        .index_url
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "el repo VUP '{}' no declara index_url y no hay conf previo",
                repo.name
            )
        })?;
    let cache_path = config.cache_dir.join(format!(
        "vup-index-{}.json",
        crate::vup_index::sanitize_repo_name(&repo.name)
    ));
    let arch = config.arch();
    let arch = if config.arch_override.is_none() {
        crate::xbps::query_architecture().unwrap_or(arch)
    } else {
        arch
    };
    let idx = crate::vup_index::fetch_index(
        &config.curl_bin,
        index_url,
        &cache_path,
        config.ttl_cache_seconds,
    )?;
    let mut urls = Vec::new();
    for (pkgname, vpkg) in &idx.packages {
        if let Some((_, repo_url)) = crate::vup_index::to_vur_info(pkgname, vpkg, &arch) {
            if !repo_url.trim().is_empty() && !urls.contains(&repo_url) {
                urls.push(repo_url);
            }
        }
    }
    if urls.is_empty() {
        anyhow::bail!(
            "el índice VUP de '{}' no aporta URLs para {arch}",
            repo.name
        );
    }
    Ok(urls)
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
        assert!(
            res.is_err(),
            "repo_add debe fallar si repos.conf está corrupto"
        );

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
        assert!(
            res.is_err(),
            "repo_remove debe fallar si repos.conf está corrupto"
        );

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
        assert!(
            res.is_err(),
            "repo_list debe fallar si repos.conf está corrupto"
        );
    }
}
