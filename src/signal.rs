//! Limpieza segura ante SIGINT/SIGTERM (E1, enmiendas 1-2).
//!
//! El handler de señal **no hace limpieza**: corre en contexto asíncrono y no
//! puede llamar `umount`, spawnear procesos ni tocar el heap; además
//! `process::exit()` desde el handler se saltaría todos los `Drop`. Por eso:
//!
//! * el handler solo registra el número de señal en un `AtomicI32`
//!   (escritura atómica = async-signal-safe);
//! * un hilo observador (poll 100 ms) hace el trabajo real: mata los hijos
//!   por **grupo de proceso** (`kill(-pid)` para no dejar nietos como
//!   `make`/`ninja` vivos), ejecuta los cleanups de overlay registrados y
//!   recién ahí sale con `128+signo`;
//! * el camino principal no necesita al observador: los hijos de larga vida
//!   (`xbps-src`) corren en su propio grupo (`setpgid` en `pre_exec`) y el
//!   `Drop` de [`crate::masterdir::OverlayGuard`] desmonta en el flujo normal.
//!
//! Se cubren SIGINT **y** SIGTERM (scripts y systemd mandan TERM).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Mutex, Once};

use nix::sys::signal::{kill, sigprocmask, SigHandler, SigSet, SigmaskHow, Signal};
use nix::unistd::Pid;

/// Limpieza pendiente de un overlay montado por
/// [`crate::masterdir::OverlayGuard`].
///
/// Guarda la config de elevación porque vary corre **sin privilegios**: los
/// mounts se hacen con sudo, así que el `Drop` no puede llamar `umount2(2)`
/// directo (EPERM) — debe desmontar por el mismo camino elevado.
#[derive(Clone, Debug)]
pub struct MountCleanup {
    pub merged: PathBuf,
    pub sudo_bin: String,
    pub sudo_flags: Vec<String>,
    /// true = overlay de kernel montado con sudo; false = fuse-overlayfs.
    pub used_sudo: bool,
    /// Binarios desmontadores configurables (H-045); defaults al construir.
    pub umount_bin: String,
    pub fusermount_bin: String,
}

impl MountCleanup {
    /// Constructor con binarios por defecto (`umount`, `fusermount3`).
    /// Hoy solo lo usan tests (overlays durmientes hasta P1-3).
    #[allow(dead_code)]
    pub fn new(
        merged: PathBuf,
        sudo_bin: String,
        sudo_flags: Vec<String>,
        used_sudo: bool,
    ) -> Self {
        Self {
            merged,
            sudo_bin,
            sudo_flags,
            used_sudo,
            umount_bin: "umount".to_string(),
            fusermount_bin: "fusermount3".to_string(),
        }
    }
}

impl MountCleanup {
    /// Orden: `fusermount3 -u` elevado (fuse) o `umount` limpio elevado
    /// (kernel) primero; `umount -l` (MNT_DETACH perezoso) solo como último
    /// recurso — el detach con el daemon FUSE vivo o el TempDir borrándose
    /// encima deja estado raro. **Nunca panickea**: traga errores y loguea
    /// (un panic en `Drop` durante unwinding = abort).
    pub fn run(&self) {
        if self.used_sudo {
            // Kernel overlay montado vía wrapper: desmontar por el mismo camino.
            if elevated_status(
                &self.sudo_bin,
                &self.sudo_flags,
                &self.umount_bin,
                &[self.merged.as_path()],
            ) {
                return;
            }
            tracing::warn!(
                "umount limpio falló en {}; reintentando lazy (-l)",
                self.merged.display()
            );
            if elevated_status(
                &self.sudo_bin,
                &self.sudo_flags,
                &self.umount_bin,
                &[Path::new("-l"), self.merged.as_path()],
            ) {
                return;
            }
            tracing::warn!("no se pudo desmontar {}", self.merged.display());
            return;
        }
        // fuse-overlayfs: primero el helper FUSE (mata el daemon), luego
        // umount plano, detach perezoso al final.
        if plain_status(
            &self.fusermount_bin,
            &["-u", &self.merged.display().to_string()],
        ) {
            return;
        }
        if plain_status(&self.umount_bin, &[&self.merged.display().to_string()]) {
            return;
        }
        if plain_status(
            &self.umount_bin,
            &["-l", &self.merged.display().to_string()],
        ) {
            return;
        }
        tracing::warn!("no se pudo desmontar {}", self.merged.display());
    }
}

fn plain_status(bin: &str, args: &[&str]) -> bool {
    std::process::Command::new(bin)
        .args(args)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn elevated_status(sudo_bin: &str, sudo_flags: &[String], program: &str, args: &[&Path]) -> bool {
    let mut cmd = match crate::elevate::elevate(sudo_bin, sudo_flags, program) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("sin vía de elevación para desmontar: {e:#}");
            return false;
        }
    };
    for a in args {
        cmd.arg(a);
    }
    cmd.status().map(|s| s.success()).unwrap_or(false)
}

static REGISTRY: Mutex<Vec<MountCleanup>> = Mutex::new(Vec::new());
static CHILDREN: Mutex<Vec<u32>> = Mutex::new(Vec::new());
/// 0 = sin señal; si no, número de señal recibido (p. ej. 2, 15).
static GOT_SIGNAL: AtomicI32 = AtomicI32::new(0);
static INIT: Once = Once::new();

/// true si el hilo observador ya recibió SIGINT/SIGTERM (los loops de
/// builds lo consultan para abortar entre paquetes).
pub fn is_shutting_down() -> bool {
    GOT_SIGNAL.load(Ordering::SeqCst) != 0
}

#[allow(dead_code)]
pub fn register_mount(c: MountCleanup) {
    if let Ok(mut r) = REGISTRY.lock() {
        r.push(c);
    }
}

#[allow(dead_code)]
pub fn unregister_mount(merged: &Path) {
    if let Ok(mut r) = REGISTRY.lock() {
        r.retain(|c| c.merged != merged);
    }
}

pub fn register_child(pid: u32) {
    if let Ok(mut c) = CHILDREN.lock() {
        c.push(pid);
    }
}

pub fn unregister_child(pid: u32) {
    if let Ok(mut c) = CHILDREN.lock() {
        c.retain(|p| *p != pid);
    }
}

/// Mata el grupo del hijo (`-pid`) con SIGTERM sin salir del proceso.
/// Lo usa el padre justo tras el spawn si la señal llegó antes del registro
/// (H-016): el observador podría no conocer aún ese pid.
pub fn kill_child_group(pid: u32) {
    let _ = kill(Pid::from_raw(-(pid as i32)), Signal::SIGTERM);
}

/// Guardia RAII que bloquea SIGINT/SIGTERM hasta su `Drop` (H-016).
/// Cierra la ventana spawn→`register_child`: con las señales bloqueadas el
/// handler no corre hasta el desbloqueo, así que ningún observador puede
/// salir sin conocer al hijo recién creado.
pub(crate) struct SignalBlockGuard {
    old: SigSet,
}

impl Drop for SignalBlockGuard {
    fn drop(&mut self) {
        let _ = sigprocmask(SigmaskHow::SIG_SETMASK, Some(&self.old), None);
    }
}

/// Bloquea SIGINT/SIGTERM y devuelve el guardián que restaura la máscara.
/// Uso: envolver solo spawn+registro, soltar ANTES de esperas largas (con la
/// máscara puesta el observador jamás recibiría la señal).
pub(crate) fn block_term_signals() -> SignalBlockGuard {
    let mut set = SigSet::empty();
    set.add(Signal::SIGINT);
    set.add(Signal::SIGTERM);
    let mut old = SigSet::empty();
    let _ = sigprocmask(SigmaskHow::SIG_BLOCK, Some(&set), Some(&mut old));
    SignalBlockGuard { old }
}

/// Borra restos `vary-*` en `dir` propiedad de nuestro uid (H-016).
/// Solo archivos/symlinks con el prefijo y uid propio: nunca toca nada ajeno
/// aunque el temporal sea compartido. Devuelve cuántos borró.
pub fn sweep_stale_tmp_files_in(dir: &Path, prefix: &str) -> usize {
    use std::os::unix::fs::MetadataExt;
    let uid = nix::unistd::getuid().as_raw();
    let mut removed = 0;
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !name.starts_with(prefix) {
            continue;
        }
        let Ok(meta) = std::fs::symlink_metadata(entry.path()) else {
            continue;
        };
        if meta.uid() != uid || (!meta.is_file() && !meta.is_symlink()) {
            continue;
        }
        if std::fs::remove_file(entry.path()).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// Barrido en el temporal del sistema al arrancar y al salir por señal.
pub fn sweep_stale_tmp_files() -> usize {
    sweep_stale_tmp_files_in(&std::env::temp_dir(), "vary-")
}

extern "C" fn on_signal(sig: nix::libc::c_int) {
    // Único trabajo permitido aquí: registrar la señal (atómico).
    GOT_SIGNAL.store(sig, Ordering::SeqCst);
}

fn install_handlers() {
    use nix::sys::signal::{sigaction, SaFlags, SigAction, SigSet};
    for sig in [Signal::SIGINT, Signal::SIGTERM] {
        let action = SigAction::new(
            SigHandler::Handler(on_signal),
            SaFlags::empty(),
            SigSet::empty(),
        );
        if let Err(e) = unsafe { sigaction(sig, &action) } {
            tracing::warn!("no se pudo instalar handler para {sig:?}: {e}");
        }
    }
}

/// Arranca el observador (idempotente). Llamar una vez al inicio de `run()`.
pub fn init() {
    INIT.call_once(|| {
        install_handlers();
        std::thread::spawn(|| loop {
            std::thread::sleep(std::time::Duration::from_millis(100));
            let sig = GOT_SIGNAL.load(Ordering::SeqCst);
            if sig == 0 {
                continue;
            }
            observer_cleanup_and_exit(sig);
        });
    });
}

fn observer_cleanup_and_exit(sig: i32) -> ! {
    // 1. Matar hijos por GRUPO (-pid): los nietos (make/ninja) mueren con el grupo.
    let kids: Vec<u32> = CHILDREN.lock().map(|c| c.clone()).unwrap_or_default();
    for pid in &kids {
        let _ = kill(Pid::from_raw(-(*pid as i32)), Signal::SIGTERM);
    }
    std::thread::sleep(std::time::Duration::from_millis(500));
    for pid in &kids {
        // ¿Sigue vivo el líder? kill con None = solo chequeo de existencia.
        if kill(Pid::from_raw(*pid as i32), None).is_ok() {
            let _ = kill(Pid::from_raw(-(*pid as i32)), Signal::SIGKILL);
        }
    }
    // 2. Desmontar overlays registrados (mismo camino elevado que el Drop).
    let pending: Vec<MountCleanup> = REGISTRY.lock().map(|r| r.clone()).unwrap_or_default();
    for c in &pending {
        c.run();
    }
    // 3. Borrar nuestros temporales huérfanos (/tmp/vary-*): process::exit
    // abajo se salta los Drop de NamedTempFile (H-016).
    sweep_stale_tmp_files();
    // 4. Flush de logs antes de una salida que se salta Drop (H-028).
    crate::logging::shutdown();
    // 5. Restaurar terminal (H-034): el spinner de indicatif oculta el cursor
    // y esta salida se salta su restauración normal.
    restore_terminal();
    // 6. Recién ahora salir; código clásico 128+signo.
    std::process::exit(128 + sig);
}

/// Muestra el cursor y resetea atributos (H-034). Idempotente; fuera de TTY
/// no emite nada.
pub fn restore_terminal() {
    use std::io::{IsTerminal, Write};
    if std::io::stderr().is_terminal() {
        let _ = write!(std::io::stderr(), "\x1b[?25h\x1b[0m");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleanup_en_ruta_inexistente_no_panickea() {
        // umount de algo no montado falla; debe tragarse el error, no panic.
        MountCleanup::new(
            PathBuf::from("/nonexistent-vary-xyz-123"),
            "false".to_string(),
            vec![],
            false,
        )
        .run();
    }

    #[test]
    fn registro_y_deregistro_de_mount() {
        let p = PathBuf::from("/tmp/vary-test-reg");
        register_mount(MountCleanup::new(p.clone(), String::new(), vec![], false));
        assert!(REGISTRY.lock().unwrap().iter().any(|c| c.merged == p));
        unregister_mount(&p);
        assert!(!REGISTRY.lock().unwrap().iter().any(|c| c.merged == p));
    }

    #[test]
    fn registro_de_hijos() {
        register_child(424242);
        assert!(CHILDREN.lock().unwrap().contains(&424242));
        unregister_child(424242);
        assert!(!CHILDREN.lock().unwrap().contains(&424242));
    }

    #[test]
    fn sweep_borra_solo_prefijo_propio() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("vary-stale-123"), b"x").expect("write");
        std::fs::write(dir.path().join("otro-123"), b"x").expect("write");
        let n = sweep_stale_tmp_files_in(dir.path(), "vary-");
        assert_eq!(n, 1, "solo el resto vary-* propio debe borrarse");
        assert!(!dir.path().join("vary-stale-123").exists());
        assert!(dir.path().join("otro-123").exists());
    }

    #[test]
    fn bloqueo_de_senales_se_restaura_con_drop() {
        {
            let _guard = block_term_signals();
            let mut cur = SigSet::empty();
            sigprocmask(SigmaskHow::SIG_BLOCK, None, Some(&mut cur)).expect("mask");
            assert!(cur.contains(Signal::SIGINT));
            assert!(cur.contains(Signal::SIGTERM));
        }
        let mut cur = SigSet::empty();
        sigprocmask(SigmaskHow::SIG_BLOCK, None, Some(&mut cur)).expect("mask");
        assert!(
            !cur.contains(Signal::SIGINT),
            "el Drop del guardián debe restaurar la máscara"
        );
    }

    #[test]
    fn sin_senal_no_hay_shutdown() {
        // init() no se llama en tests: el flag global arranca en 0.
        if GOT_SIGNAL.load(Ordering::SeqCst) == 0 {
            assert!(!is_shutting_down());
        }
    }
}
