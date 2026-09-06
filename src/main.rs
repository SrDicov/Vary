use std::process::exit;
use vary::run;

#[cfg(target_env = "musl")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn drop_privileges() {
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
            setgroups(&[]).expect("failed to drop supplementary groups");
            setresgid(gid, gid, gid).expect("failed to set gid");
            setresuid(uid, uid, uid).expect("failed to set uid");
        }
    }
}

fn main() {
    drop_privileges();
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
            exit(0);
        }
        default_hook(info);
    }));

    let args = std::env::args().skip(1).collect::<Vec<_>>();
    exit(run(&args));
}
