use std::process::exit;
use vary::run;

#[cfg(target_env = "musl")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn drop_privileges() -> anyhow::Result<()> {
    use nix::unistd::{setgroups, setresgid, setresuid, Gid, Uid, User};
    use std::env;

    if nix::unistd::getuid().is_root() {
        let mut target_uid = None;
        let mut target_gid = None;

        if let Ok(sudo_uid) = env::var("SUDO_UID") {
            if let Ok(uid_val) = sudo_uid.parse::<u32>() {
                target_uid = Some(Uid::from_raw(uid_val));
            }
        }
        if let Ok(sudo_gid) = env::var("SUDO_GID") {
            if let Ok(gid_val) = sudo_gid.parse::<u32>() {
                target_gid = Some(Gid::from_raw(gid_val));
            }
        }

        if target_uid.is_none() {
            if let Ok(doas_user) = env::var("DOAS_USER") {
                if let Ok(Some(user)) = User::from_name(&doas_user) {
                    target_uid = Some(user.uid);
                    target_gid = Some(user.gid);
                }
            }
        }

        if let (Some(uid), Some(gid)) = (target_uid, target_gid) {
            // H-023/H-038: abandonar privilegios puede fallar (seccomp, LSM);
            // error fatal accionable en vez de panic en una ruta del main.
            setgroups(&[])
                .map_err(|e| anyhow::anyhow!("failed to drop supplementary groups: {e}"))?;
            setresgid(gid, gid, gid).map_err(|e| anyhow::anyhow!("failed to set gid: {e}"))?;
            setresuid(uid, uid, uid).map_err(|e| anyhow::anyhow!("failed to set uid: {e}"))?;
        }
    }
    Ok(())
}

fn main() {
    if let Err(e) = drop_privileges() {
        eprintln!("error: {e:#}");
        exit(1);
    }
    // Install a panic hook that swallows the benign "Broken pipe" panic that
    // occurs when a downstream consumer of our stdout closes the pipe early
    // (e.g. `vary -Ss foo | head`, `vary -Si x | less`, `vary ... | true`).
    // Without this, vary aborts with exit code 101 and a scary backtrace.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let is_broken_pipe = match info.payload().downcast_ref::<String>() {
            Some(s) => s.contains("Broken pipe"),
            None => match info.payload().downcast_ref::<&str>() {
                Some(s) => s.contains("Broken pipe"),
                None => false,
            },
        };
        if is_broken_pipe {
            // The reader went away; this is expected, not an error.
            vary::shutdown_logging();
            exit(0);
        }
        default_hook(info);
    }));

    let args = std::env::args().skip(1).collect::<Vec<_>>();
    exit(run(&args));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drop_privileges_sin_root_devuelve_ok() {
        // H-023/H-038: sin root no hay nada que abandonar; debe ser Ok sin
        // tocar nada. Como root real el test no afirma (cambiaría el proceso).
        if !nix::unistd::getuid().is_root() {
            assert!(drop_privileges().is_ok());
        }
    }
}
