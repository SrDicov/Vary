use crate::config::Config;
use crate::db::InstalledDb;
use anyhow::Result;

pub fn remove(config: &Config) -> Result<i32> {
    // P1-1: --lock no aplica a -R (no hay estado final que pineado tenga
    // sentido distinto; regenerar tras borrar se hace con `vary --lock`).
    if config.args.has_arg("lock", "lock") {
        anyhow::bail!("--lock no se combina con -R (usa `vary --lock` tras los cambios)");
    }
    // P1-2: challenge tiene su propia vía.
    if config.args.has_arg("challenge", "challenge") {
        anyhow::bail!(
            "--challenge no se combina con -R: usa `vary --challenge <pkg> --experimental`"
        );
    }
    // P2: why/log tienen su propia vía.
    if config.args.has_arg("why", "why") {
        anyhow::bail!("--why no se combina con -R: usa `vary --why <pkg>`");
    }
    if config.args.has_arg("log", "log") {
        anyhow::bail!("--log no se combina con -R: usa `vary --log [pkg]`");
    }
    if config.targets.is_empty() {
        anyhow::bail!("no targets specified (use -h for help)");
    }

    let targets = config.targets.clone();
    for target in &targets {
        if !crate::metadata::is_valid_pkgname(target) {
            anyhow::bail!("nombre de paquete inválido: '{target}' (debe coincidir con ^[a-zA-Z0-9][a-zA-Z0-9._+-]*$)");
        }
    }

    tracing::info!("removing packages: {targets:?}");

    let mut extra: Vec<String> = Vec::new();
    if config.no_confirm {
        extra.push("-y".to_string());
    }
    let code =
        crate::xbps::remove_recursive(&targets, &extra, &config.sudo_bin, &config.sudo_flags)?;

    if code == 0 {
        // Update installed db: remove entries for removed packages
        let mut db = InstalledDb::load(config.installed_db_path())?;
        let mut changed = false;
        for t in &targets {
            // strip version constraints for db key
            let name = t.split(['<', '>', '=', ' ']).next().unwrap_or(t);
            if db.remove(name) {
                changed = true;
            }
            // P2: diario (best-effort: nunca aborta un remove válido).
            if let Err(e) = crate::journal::append(&config.data_dir, "REMOVE", name, "") {
                tracing::warn!("no se pudo anotar el diario: {e:#}");
            }
        }
        if changed {
            if let Err(e) = db.save() {
                tracing::warn!("failed to update installed db: {e}");
            }
        }
    }

    Ok(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remove_rejects_invalid_target_name() {
        let config = Config {
            targets: vec![";bad".to_string()],
            ..Default::default()
        };
        let err = remove(&config).unwrap_err();
        assert!(err.to_string().contains("nombre de paquete inválido"));

        let config2 = Config {
            targets: vec![".hidden".to_string()],
            ..Default::default()
        };
        let err2 = remove(&config2).unwrap_err();
        assert!(err2.to_string().contains("nombre de paquete inválido"));
    }
}
