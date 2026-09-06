use anyhow::Result;
use std::path::Path;

pub fn post_install_hook(
    pkg_name: &str,
    no_confirm: bool,
    sudo_bin: &str,
    sudo_flags: &[String],
) -> Result<()> {
    let sv_dir = Path::new("/etc/sv").join(pkg_name);
    if !sv_dir.exists() {
        // Package does not provide a service
        return Ok(());
    }

    if Path::new("/dev/dinitctl").exists() {
        // Dinit detected
        if no_confirm
            || crate::util::confirm(
                &format!("Enable and start dinit service for {}?", pkg_name),
                no_confirm,
            )?
        {
            let _ = crate::elevate::elevate(sudo_bin, sudo_flags, "dinitctl")?
                .args(["enable", pkg_name])
                .status();
            let _ = crate::elevate::elevate(sudo_bin, sudo_flags, "dinitctl")?
                .args(["start", pkg_name])
                .status();
        }
    } else if Path::new("/run/runit").exists() {
        // Runit detected
        if no_confirm
            || crate::util::confirm(
                &format!("Enable runit service for {}?", pkg_name),
                no_confirm,
            )?
        {
            let service_link = Path::new("/var/service").join(pkg_name);
            if !service_link.exists() {
                let _ = crate::elevate::elevate(sudo_bin, sudo_flags, "ln")?
                    .arg("-s")
                    .arg(&sv_dir)
                    .arg(&service_link)
                    .status();
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hook_retorna_ok_si_paquete_no_tiene_servicio() {
        let res = post_install_hook("nonexistent-pkg-service-xyz", true, "sudo", &[]);
        assert!(res.is_ok());
    }
}
