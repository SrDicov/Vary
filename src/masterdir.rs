//! Administración del masterdir de void-packages.
//!
//! Void ya compila en el sandbox nativo de `xbps-src`; este módulo solo
//! administra la ruta base y delega el ciclo de vida de la compilación
//! de forma secuencial y determinista (Opción A).
use crate::xbps::xbps_src;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

pub const VOID_PACKAGES_URL: &str = "https://github.com/void-linux/void-packages";

pub struct Masterdir {
    /// Ruta al checkout local de void-packages (~/.cache/vary/void-packages).
    pub path: PathBuf,
    /// Archivo donde `xbps-src` vuelca su salida con spinner en terminal
    /// (H-020). `None` = heredar stdio como antes.
    pub log_file: Option<PathBuf>,
}

impl Masterdir {
    pub fn new(void_packages_dir: impl Into<PathBuf>) -> Self {
        Self {
            path: void_packages_dir.into(),
            log_file: None,
        }
    }

    /// El entorno aislado (masterdir) ya fue inicializado?
    pub fn bootstrapped(&self) -> bool {
        self.path.join("masterdir").exists()
    }

    /// `./xbps-src binary-bootstrap`: descarga el conjunto de herramientas
    /// base dentro del namespace del masterdir. Idempotente.
    pub fn binary_bootstrap(&self) -> Result<()> {
        tracing::info!("ejecutando ./xbps-src binary-bootstrap (puede tardar)...");
        let code = xbps_src(
            &self.path,
            &["binary-bootstrap"],
            None,
            self.log_file.as_deref(),
        )
        .context("falló ./xbps-src binary-bootstrap")?;
        if code != 0 {
            anyhow::bail!("./xbps-src binary-bootstrap terminó con código {code}");
        }
        Ok(())
    }

    pub fn srcpkgs_dir(&self) -> PathBuf {
        self.path.join("srcpkgs")
    }

    /// Ruta del repositorio binario local raíz (sin arch) para xbps-install.
    pub fn hostdir_binpkgs_root(&self) -> PathBuf {
        self.path.join("hostdir").join("binpkgs")
    }

    /// Compila un paquete: `./xbps-src pkg <pkg>` dentro del masterdir de
    /// forma secuencial y determinista, pasando `makejobs` a `XBPS_MAKEJOBS`.
    pub fn build_pkg(&self, pkg: &str, makejobs: usize) -> Result<()> {
        if crate::signal::is_shutting_down() {
            anyhow::bail!("interrumpido por señal antes de compilar {pkg}");
        }

        tracing::info!("compilando {pkg} con xbps-src (makejobs: {makejobs})...");
        let code = xbps_src(
            &self.path,
            &["pkg", pkg],
            Some(makejobs),
            self.log_file.as_deref(),
        )
        .with_context(|| format!("falló ./xbps-src pkg {pkg}"))?;

        if code != 0 {
            anyhow::bail!("xbps-src pkg {pkg} terminó con código {code}");
        }
        Ok(())
    }

    /// Solo descarga fuentes: `./xbps-src fetch <pkg>`.
    pub fn fetch_pkg(&self, pkg: &str) -> Result<()> {
        let code = xbps_src(&self.path, &["fetch", pkg], None, self.log_file.as_deref())
            .with_context(|| format!("falló ./xbps-src fetch {pkg}"))?;
        if code != 0 {
            anyhow::bail!("xbps-src fetch {pkg} terminó con código {code}");
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// P1-3: workers aislados estilo xbps-fbulk (E1 capacidad + E2 sandbox).
//
// Aislamiento = overlayfs de kernel por slot sobre el checkout compartido
// (montado con elevación; upper/work/merged en <cache>/workers/w<N>).
// El `srcpkgs/` compartido queda read-only en la práctica: cada worker
// proyecta SOLO en su merged. El `masterdir` base (ya bootstrapped) se
// hereda por lowerdir: sin re-bootstrap por worker. El binpkgs de cada
// worker es privado (su upper); el hilo principal fusiona artefactos e
// indexa UNA vez (E4 en install.rs). Sin capacidad o sin --experimental:
// secuencial (camino actual, cero regresión).
// ---------------------------------------------------------------------------

/// Espacio libre mínimo en caché para atreverse con workers paralelos
/// (los upper son deltas, pero los builds escriben; 1 GiB con margen).
pub const MIN_FREE_BYTES_FOR_WORKERS: u64 = 1 << 30;

/// ¿El texto de `/proc/filesystems` ofrece overlayfs? (puro, testeable).
/// Formato: una fs por línea, opcionalmente prefijada con `nodev`.
pub fn overlay_in_filesystems(text: &str) -> bool {
    text.lines()
        .any(|l| l.split_whitespace().any(|f| f == "overlay"))
}

/// Bytes libres en el filesystem de `path` (`None` si no se puede saber).
pub fn free_bytes(path: &Path) -> Option<u64> {
    let st = nix::sys::statvfs::statvfs(path).ok()?;
    st.blocks_available().checked_mul(st.fragment_size() as u64)
}

/// Slots efectivos de build: >1 SOLO con --experimental y max>1
/// (0.4.1: el paralelismo real es opt-in tras la puerta experimental,
/// igual que P1-2). Puro, testeable.
pub fn effective_build_slots(experimental: bool, max_concurrent: u32, nproc: usize) -> usize {
    if !experimental || max_concurrent <= 1 {
        return 1;
    }
    (max_concurrent as usize).clamp(1, nproc.max(1))
}

/// Precondiciones dinámicas del paralelismo (puro sobre datos ya leídos).
/// `Err` = motivo para degradar a secuencial con aviso (fail-safe).
pub fn check_parallel_prereqs(free: Option<u64>, overlay: bool) -> Result<()> {
    if !overlay {
        anyhow::bail!("sin overlayfs en el kernel; builds secuenciales");
    }
    match free {
        Some(b) if b >= MIN_FREE_BYTES_FOR_WORKERS => Ok(()),
        Some(b) => anyhow::bail!(
            "espacio libre en caché ({b} B) bajo el mínimo de 1 GiB; builds secuenciales"
        ),
        None => anyhow::bail!("espacio libre indetectable; builds secuenciales"),
    }
}

/// Wrapper fino: lee `/proc/filesystems` + statvfs y decide.
pub fn parallel_available(cache_dir: &Path) -> Result<()> {
    let filesystems = std::fs::read_to_string("/proc/filesystems").unwrap_or_default();
    check_parallel_prereqs(free_bytes(cache_dir), overlay_in_filesystems(&filesystems))
}

/// Sandbox de un slot worker: overlay sobre el checkout base.
///
/// Limpieza en dos niveles: `destroy()` (explícita: umount + rmdir) y
/// `Drop` (red de seguridad: umount registrado; el dir lo barre el
/// `cleanup_stale` del próximo run). `MountCleanup::run` nunca paniquea,
/// apto para `Drop`.
pub struct WorkerSandbox {
    /// `merged/`: raíz del checkout vista por el worker (aquí corre xbps-src).
    pub merged: PathBuf,
    slot_dir: PathBuf,
    cleanup: Option<crate::signal::MountCleanup>,
    sudo_bin: String,
    sudo_flags: Vec<String>,
}

impl WorkerSandbox {
    /// Nº de slot (para logs por worker).
    pub fn slot(&self) -> usize {
        self.slot_dir
            .file_name()
            .and_then(|n| n.to_string_lossy()[1..].parse().ok())
            .unwrap_or(999)
    }
    /// Nombre del subdir del slot (puro, testeable): `w<N>`.
    pub fn dir_name(slot: usize) -> String {
        format!("w{slot}")
    }

    /// ¿`name` parece dir de slot nuestro (`w` + dígitos)? (puro, testeable).
    /// Solo para `cleanup_stale`: nunca tocar nada fuera de `workers_dir`.
    pub fn is_slot_dir(name: &str) -> bool {
        let rest = name.strip_prefix('w').unwrap_or("");
        !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit())
    }

    pub fn checkout_root(&self) -> &Path {
        &self.merged
    }

    pub fn srcpkgs_dir(&self) -> PathBuf {
        self.merged.join("srcpkgs")
    }

    /// El masterdir NO se pasa con `-m`: xbps-src resuelve por defecto
    /// `$XBPS_DISTDIR/masterdir-$MACHINE` (= merged/masterdir-<arch>, visible
    /// por lowerdir con su `.xbps_chroot_init`). Pasar `-m merged/masterdir`
    /// (estilo legacy, ausente en checkouts modernos) provocaba un bootstrap
    /// completo POR WORKER (GBs de descarga × N, validado en vivo: w0 lo
    /// sufrió y w1 murió por ello). Sin `-m` no hay bootstrap por worker.

    /// Repo binario local del worker (PLANO, sin subdir de arch: xbps-src
    /// deja los .xbps directos en hostdir/binpkgs/, igual que en secuencial
    /// —validado en vivo: con subdir el diff salía siempre vacío).
    pub fn binpkgs_dir(&self) -> PathBuf {
        self.merged.join("hostdir").join("binpkgs")
    }

    /// Log propio del slot (los workers no deben intercalar un mismo log).
    pub fn log_file(&self, logs_dir: &Path) -> PathBuf {
        logs_dir.join(format!("xbps-src-w{}.log", self.slot()))
    }

    /// Monta el overlay. Cualquier fallo = `Err` y el llamador degrada a
    /// secuencial (fail-safe: peor caso = comportamiento 0.4.0).
    pub fn create(
        base_checkout: &Path,
        workers_dir: &Path,
        slot: usize,
        sudo_bin: &str,
        sudo_flags: &[String],
    ) -> Result<Self> {
        let slot_dir = workers_dir.join(Self::dir_name(slot));
        let upper = slot_dir.join("upper");
        let work = slot_dir.join("work");
        let merged = slot_dir.join("merged");
        for d in [&slot_dir, &upper, &work, &merged] {
            crate::util::ensure_private_dir(d)
                .with_context(|| format!("no se pudo crear {}", d.display()))?;
        }
        // Higiene del upper: un upper con restos (run muerto sin limpieza)
        // ensombrecería al lower Y contaminaría el diff de artefactos
        // (before ya los contendría → fusión vacía silenciosa, validado en
        // vivo). Reset elevado del slot + recrear antes de montar.
        let upper_dirty = std::fs::read_dir(&upper)
            .map(|mut r| r.next().is_some())
            .unwrap_or(false);
        if upper_dirty {
            tracing::warn!("upper residual en slot {slot}; reseteando antes de montar");
            Self::remove_slot_dir_elevated(workers_dir, &slot_dir, sudo_bin, sudo_flags);
            for d in [&slot_dir, &upper, &work, &merged] {
                crate::util::ensure_private_dir(d)
                    .with_context(|| format!("no se pudo recrear {}", d.display()))?;
            }
        }
        let mut cmd = crate::elevate::elevate(sudo_bin, sudo_flags, "mount")?;
        // stdin nulo: si el wrapper pidiera contraseña, fallar rápido en
        // vez de colgar al worker (el fallo degrada a secuencial).
        cmd.arg("-t")
            .arg("overlay")
            .arg("overlay")
            .arg("-o")
            .arg(format!(
                "lowerdir={},upperdir={},workdir={}",
                base_checkout.display(),
                upper.display(),
                work.display()
            ))
            .arg(&merged)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null());
        let out = cmd
            .output()
            .with_context(|| "no se pudo invocar mount para el overlay del worker")?;
        if !out.status.success() {
            anyhow::bail!(
                "mount overlay del worker {slot} falló: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        let cleanup = crate::signal::MountCleanup::new(
            merged.clone(),
            sudo_bin.to_string(),
            sudo_flags.to_vec(),
            true,
        );
        crate::signal::register_mount(cleanup.clone());
        Ok(Self {
            merged,
            slot_dir,
            cleanup: Some(cleanup),
            sudo_bin: sudo_bin.to_string(),
            sudo_flags: sudo_flags.to_vec(),
        })
    }

    /// Borra un dir de slot con elevación (los upper contienen ficheros de
    /// root creados por el chroot de xbps-src; sin elevate quedarían
    /// residuales). VALIDADO: solo `workers_dir/w<N>`; cualquier otra ruta
    /// se rechaza (nunca `rm -rf` a ciegas con privilegios).
    fn remove_slot_dir_elevated(
        workers_dir: &Path,
        slot_dir: &Path,
        sudo_bin: &str,
        sudo_flags: &[String],
    ) -> bool {
        let name = slot_dir
            .file_name()
            .map(|n| n.to_string_lossy().to_string());
        let parent_ok = slot_dir.parent() == Some(workers_dir);
        let name_ok = name.as_deref().is_some_and(WorkerSandbox::is_slot_dir);
        if !parent_ok || !name_ok {
            tracing::warn!(
                "rechazado borrado elevado fuera de workers/w<N>: {}",
                slot_dir.display()
            );
            return false;
        }
        let mut cmd = match crate::elevate::elevate(sudo_bin, sudo_flags, "rm") {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(
                    "sin vía de elevación para limpiar {}: {e:#}",
                    slot_dir.display()
                );
                return false;
            }
        };
        cmd.arg("-rf")
            .arg(slot_dir)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null());
        cmd.output().map(|o| o.status.success()).unwrap_or(false)
    }

    /// Constructor de test: sin montar nada (solo derivación de paths).
    /// El mount real con elevación solo se prueba en vivo (smoke pre-tag).
    #[cfg(test)]
    pub fn for_test(slot_dir: PathBuf) -> Self {
        let merged = slot_dir.join("merged");
        Self {
            merged,
            slot_dir,
            cleanup: None,
            sudo_bin: "doas".to_string(),
            sudo_flags: Vec::new(),
        }
    }

    /// Desmonta (vía registro) y borra el dir del slot (elevado: el upper
    /// trae ficheros de root). Best-effort; lo que quede lo barre
    /// cleanup_stale en el próximo run.
    pub fn destroy(mut self) {
        if let Some(c) = self.cleanup.take() {
            crate::signal::unregister_mount(&c.merged);
            c.run();
        }
        let workers_dir = self.slot_dir.parent().map(Path::to_path_buf);
        if let Some(wd) = workers_dir {
            if !Self::remove_slot_dir_elevated(
                &wd,
                &self.slot_dir,
                &self.sudo_bin,
                &self.sudo_flags,
            ) {
                let _ = std::fs::remove_dir_all(&self.slot_dir);
            }
        }
    }
}

impl Drop for WorkerSandbox {
    fn drop(&mut self) {
        // Red de seguridad ante pánicos/abortos a mitad de nivel: desmontar
        // (el dir residual lo barre cleanup_stale). Nunca paniquea.
        if let Some(c) = self.cleanup.take() {
            crate::signal::unregister_mount(&c.merged);
            c.run();
        }
    }
}

/// Barre sandboxes residuales de runs muertos (solo `w<N>` bajo workers_dir;
/// si algo sigue montado se intenta desmontar best-effort antes de borrar).
pub fn cleanup_stale_sandboxes(workers_dir: &Path, sudo_bin: &str, sudo_flags: &[String]) {
    let entries = std::fs::read_dir(workers_dir).map(|r| r.collect::<Vec<_>>());
    let Ok(entries) = entries else { return };
    for entry in entries.into_iter().flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if !WorkerSandbox::is_slot_dir(&name) || !path.is_dir() {
            continue;
        }
        let merged = path.join("merged");
        if is_mounted(&merged) {
            tracing::warn!(
                "sandbox residual montado en {}; desmontando",
                merged.display()
            );
            crate::signal::MountCleanup::new(
                merged,
                sudo_bin.to_string(),
                sudo_flags.to_vec(),
                true,
            )
            .run();
        }
        if is_mounted(&path.join("merged")) {
            tracing::warn!("no se pudo desmontar {}; se conserva", path.display());
            continue;
        }
        // Elevado primero (upper con ficheros de root); plano como fallback.
        if !WorkerSandbox::remove_slot_dir_elevated(workers_dir, &path, sudo_bin, sudo_flags) {
            let _ = std::fs::remove_dir_all(&path);
        }
    }
}

/// ¿`path` es punto de montaje? (lee /proc/self/mounts; ausente = false).
pub fn is_mounted(path: &Path) -> bool {
    let Ok(mounts) = std::fs::read_to_string("/proc/self/mounts") else {
        return false;
    };
    let target = path.display().to_string();
    mounts
        .lines()
        .any(|l| l.split_whitespace().nth(1) == Some(target.as_str()))
}

/// Diferencia de listados `antes/después` de un dir: ficheros nuevos.
/// (E4: artefactos que aportó UN build; puro, testeable.)
pub fn new_files_since(before: &[PathBuf], after: &[PathBuf]) -> Vec<PathBuf> {
    let known: std::collections::HashSet<&PathBuf> = before.iter().collect();
    let mut new: Vec<PathBuf> = after
        .iter()
        .filter(|p| !known.contains(p))
        .cloned()
        .collect();
    new.sort();
    new
}

/// Lista `.xbps` directos de un dir (vacío si no existe; nunca falla).
pub fn list_xbps(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|r| {
            r.filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "xbps"))
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

/// Clona void-packages (shallow) si aún no existe.
pub fn clone_void_packages(target: &Path, git_bin: &str) -> Result<()> {
    if target.join(".git").exists() {
        return Ok(());
    }
    if let Some(parent) = target.parent() {
        crate::util::ensure_private_dir(parent)
            .with_context(|| format!("creando {}", parent.display()))?;
    }
    tracing::info!("clonando void-packages (depth 1)...");
    let out = std::process::Command::new(git_bin)
        .args([
            "-c",
            "protocol.ext.allow=never",
            "-c",
            "protocol.file.allow=user",
            "clone",
            "--depth",
            "1",
            "--",
            VOID_PACKAGES_URL,
            &target.display().to_string(),
        ])
        .status()
        .context(format!("`{git_bin}` no encontrado: ¿está instalado?"))?;
    if !out.success() {
        anyhow::bail!(
            "git clone de void-packages falló con código {:?}",
            out.code()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rutas_derivadas_son_consistentes() {
        let md = Masterdir::new("/tmp/vp");
        assert_eq!(md.srcpkgs_dir(), PathBuf::from("/tmp/vp/srcpkgs"));
        assert_eq!(
            md.hostdir_binpkgs_root().join("x86_64"),
            PathBuf::from("/tmp/vp/hostdir/binpkgs/x86_64")
        );
        assert_eq!(
            md.hostdir_binpkgs_root(),
            PathBuf::from("/tmp/vp/hostdir/binpkgs")
        );
    }

    #[test]
    fn build_pkg_falla_si_no_existe_xbps_src() {
        let dir = tempfile::tempdir().unwrap();
        let md = Masterdir::new(dir.path());
        let res = md.build_pkg("dummy-pkg", 2);
        assert!(res.is_err());
        assert!(format!("{:#}", res.unwrap_err()).contains("no encontrado"));
    }

    // --- P1-3: capacidad y utilidades de workers (puro) ---

    #[test]
    fn overlay_se_detecta_en_proc_filesystems() {
        assert!(overlay_in_filesystems("ext4\nnodev\toverlay\ntmpfs\n"));
        assert!(overlay_in_filesystems("overlay\n"));
        assert!(!overlay_in_filesystems("ext4\ntmpfs\n"));
        assert!(!overlay_in_filesystems(""));
        // "overlay" como subcadena de otra fs no cuenta.
        assert!(!overlay_in_filesystems("nodev\toverlayfs\n"));
    }

    #[test]
    fn slots_solo_con_experimental_y_max_mayor_que_uno() {
        assert_eq!(effective_build_slots(false, 8, 8), 1);
        assert_eq!(effective_build_slots(true, 1, 8), 1);
        assert_eq!(effective_build_slots(true, 0, 8), 1);
        assert_eq!(effective_build_slots(true, 4, 8), 4);
        // Nunca más que los núcleos.
        assert_eq!(effective_build_slots(true, 16, 2), 2);
        assert_eq!(effective_build_slots(true, 4, 0), 1);
    }

    #[test]
    fn prereqs_exigen_overlay_y_un_gibi() {
        assert!(check_parallel_prereqs(Some(2 << 30), true).is_ok());
        assert!(check_parallel_prereqs(Some(2 << 30), false).is_err());
        assert!(check_parallel_prereqs(Some(512 << 20), true).is_err());
        assert!(check_parallel_prereqs(None, true).is_err());
    }

    #[test]
    fn slot_dirs_solo_w_mas_digitos() {
        assert!(WorkerSandbox::is_slot_dir("w0"));
        assert!(WorkerSandbox::is_slot_dir("w12"));
        assert!(!WorkerSandbox::is_slot_dir("w"));
        assert!(!WorkerSandbox::is_slot_dir("wx"));
        assert!(!WorkerSandbox::is_slot_dir("srcpkgs"));
        assert!(!WorkerSandbox::is_slot_dir(""));
        assert_eq!(WorkerSandbox::dir_name(3), "w3");
    }

    #[test]
    fn new_files_since_devuelve_solo_nuevos_ordenados() {
        let a = PathBuf::from("/b/a.xbps");
        let b = PathBuf::from("/b/b.xbps");
        let c = PathBuf::from("/b/c.xbps");
        assert_eq!(
            new_files_since(&[a.clone()], &[c.clone(), a.clone(), b.clone()]),
            vec![b, c]
        );
        assert!(new_files_since(&[a.clone()], &[a]).is_empty());
    }

    #[test]
    fn list_xbps_solo_xbps_y_ordenado() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("b.xbps"), b"x").unwrap();
        std::fs::write(dir.path().join("a.xbps"), b"x").unwrap();
        std::fs::write(dir.path().join("nota.txt"), b"x").unwrap();
        let got = list_xbps(dir.path());
        assert_eq!(got.len(), 2);
        assert!(got[0].ends_with("a.xbps"));
        assert!(list_xbps(&dir.path().join("inexistente")).is_empty());
    }

    #[test]
    fn borrado_elevado_rechaza_todo_fuera_de_workers_wn() {
        // Solo rutas rechazadas (ninguna invoca rm: la validación va primero).
        let wd = Path::new("/cache/workers");
        assert!(!WorkerSandbox::remove_slot_dir_elevated(
            wd,
            Path::new("/cache/workers/../evil"),
            "doas",
            &[]
        ));
        assert!(!WorkerSandbox::remove_slot_dir_elevated(
            wd,
            Path::new("/cache/workers/srcpkgs"),
            "doas",
            &[]
        ));
        assert!(!WorkerSandbox::remove_slot_dir_elevated(
            wd,
            Path::new("/otro/w0"),
            "doas",
            &[]
        ));
        assert!(!WorkerSandbox::remove_slot_dir_elevated(
            wd,
            Path::new("/cache/workers/w"),
            "doas",
            &[]
        ));
    }
}
