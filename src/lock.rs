//! Lock de instancia única (E1, agregado B).
//!
//! Dos instancias de vary concurrentes se pisan `/etc/xbps.d/`, la db `heed`
//! y los mounts. Un `flock` exclusivo no bloqueante sobre
//! `<cache_dir>/vary.lock` lo evita en ~10 líneas: si el lock está tomado se
//! lee el pid grabado y se aborta con mensaje accionable.

use std::io::{Read, Write};
use std::path::Path;

use anyhow::{Context, Result};

/// Guardián: el `flock` se libera al dropear (`Flock::drop` hace unlock y
/// cierra el `File`).
#[derive(Debug)]
pub struct InstanceLock {
    _locked: nix::fcntl::Flock<std::fs::File>,
}

pub fn acquire(cache_dir: &Path) -> Result<InstanceLock> {
    crate::util::ensure_private_dir(cache_dir)
        .with_context(|| format!("creando {}", cache_dir.display()))?;
    let path = cache_dir.join("vary.lock");
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .with_context(|| format!("abriendo {}", path.display()))?;

    match nix::fcntl::Flock::lock(file, nix::fcntl::FlockArg::LockExclusiveNonblock) {
        Ok(locked) => {
            // Lock adquirido: grabar nuestro pid para el mensaje del próximo.
            let mut file = locked;
            let _ = file.set_len(0);
            let _ = file.write_all(format!("{}\n", std::process::id()).as_bytes());
            Ok(InstanceLock { _locked: file })
        }
        Err((mut file, nix::errno::Errno::EWOULDBLOCK)) => {
            let mut pid = String::new();
            let _ = file.read_to_string(&mut pid);
            let pid = pid.trim();
            anyhow::bail!(
                "otra instancia de vary está en ejecución{}; espera a que termine (el lock se libera solo al salir: no borres el archivo)",
                if pid.is_empty() {
                    String::new()
                } else {
                    format!(" (pid {pid})")
                },
            );
        }
        Err((_, e)) => {
            anyhow::bail!("no se pudo bloquear {}: {}", path.display(), e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segunda_instancia_ve_el_pid_de_la_primera() {
        let dir = tempfile::tempdir().unwrap();
        let _first = acquire(dir.path()).expect("primer lock");
        let err = acquire(dir.path()).expect_err("segundo lock debe fallar");
        let msg = format!("{err:#}");
        assert!(msg.contains("otra instancia"), "mensaje accionable: {msg}");
        assert!(
            !msg.contains("borra"),
            "nunca sugerir borrar el lock (split-brain): {msg}"
        );
        assert!(
            msg.contains(&std::process::id().to_string()),
            "el pid ayuda a decidir: {msg}"
        );
    }

    #[test]
    fn al_dropear_se_libera() {
        let dir = tempfile::tempdir().unwrap();
        {
            let _first = acquire(dir.path()).expect("primer lock");
        }
        // Drop liberó el flock: se puede adquirir de nuevo.
        let _second = acquire(dir.path()).expect("lock tras drop");
    }
}
