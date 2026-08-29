use anyhow::Result;
use std::path::Path;
use std::process::Command;

pub fn post_install_hook(pkg_name: &str, no_confirm: bool) -> Result<()> {
    let sv_dir = Path::new("/etc/sv").join(pkg_name);
    if !sv_dir.exists() {
        // Package does not provide a service
        return Ok(());
    }

    if Path::new("/dev/dinitctl").exists() {
        // Dinit detected
        if no_confirm || crate::util::confirm(&format!("Enable and start dinit service for {}?", pkg_name), no_confirm)? {
            let _ = Command::new("sudo").args(["dinitctl", "enable", pkg_name]).status();
            let _ = Command::new("sudo").args(["dinitctl", "start", pkg_name]).status();
        }
    } else if Path::new("/run/runit").exists() {
        // Runit detected
        if no_confirm || crate::util::confirm(&format!("Enable runit service for {}?", pkg_name), no_confirm)? {
            let service_link = Path::new("/var/service").join(pkg_name);
            if !service_link.exists() {
                let _ = Command::new("sudo")
                    .args(["ln", "-s", sv_dir.to_str().unwrap(), service_link.to_str().unwrap()])
                    .status();
            }
        }
    }

    Ok(())
}
