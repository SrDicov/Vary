use crate::config::Config;
use crate::db::InstalledDb;
use anyhow::Result;

pub fn remove(config: &Config) -> Result<i32> {
    if config.targets.is_empty() {
        anyhow::bail!("no targets specified (use -h for help)");
    }

    let targets = config.targets.clone();

    tracing::info!("removing packages: {:?}", targets);

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
        }
        if changed {
            if let Err(e) = db.save() {
                tracing::warn!("failed to update installed db: {}", e);
            }
        }
    }

    Ok(code)
}
