//! Chequeos previos duros del entorno (P0-1).
//!
//! Se ejecutan al inicio de `run()`, antes de tocar estado compartido:
//!
//! 1. **root directo** (`euid == 0`) => ABORTO. Tras `drop_privileges()` en
//!    `main`, euid 0 solo llega sin wrapper (login root/su directo): vary
//!    nunca opera como root (además `xbps-src` se niega a correr como root).
//! 2. **chroot degradado** (proot/bwrap) => AVISO crítico, seguir.
//!    Heurística documentada: flatpak expone `/.flatpak-info` (+ `$FLATPAK_ID`);
//!    proot no deja marcador estándar y se detecta por `$PROOT_TMP_DIR`.
//! 3. **`xbps-uchroot` sin modo `4750`** => ABORTO con hint. Se mira en
//!    `/usr/bin` y `/usr/libexec`; si no existe en ninguno se OMITE (no se
//!    puede verificar; `xbps-src` lo reportará al compilar).
//! 4. **OCI** (`/.dockerenv` o `/proc/1/cgroup` con `docker|kubepods`) =>
//!    AVISO leve, seguir (decisión propia: la spec solo pide detectar; un
//!    falso positivo nunca debe abortar).
//!
//! Todo acceso al sistema se inyecta vía [`Io`] para la tabla de entornos ×
//! resultado esperado en tests (aceptación P0-1).

use std::path::Path;

/// Candidatos de `xbps-uchroot` por orden de preferencia (Void lo instala en
/// `/usr/bin`; se contempla `/usr/libexec` por compatibilidad).
const UCHROOT_CANDIDATES: &[&str] = &["/usr/bin/xbps-uchroot", "/usr/libexec/xbps-uchroot"];
/// Modo exigido a `xbps-uchroot` (setuid root, grupo con ejecución).
const UCHROOT_MODE: u32 = 0o4750;

/// Resultado: avisos acumulados en orden de chequeo + aborto opcional.
#[derive(Debug, Default, PartialEq)]
pub struct Report {
    pub warnings: Vec<String>,
    pub abort: Option<String>,
}

/// Acceso al sistema necesario para los chequeos (inyectable en tests).
pub trait Io {
    /// euid del proceso.
    fn euid(&self) -> u32;
    /// ¿existe la ruta (fichero o dir)?
    fn file_exists(&self, path: &Path) -> bool;
    /// Modo de la ruta enmascarado a `0o7777`; `None` si no se puede leer.
    fn file_mode(&self, path: &Path) -> Option<u32>;
    /// Variable de entorno (vacía = ausente).
    fn getenv(&self, key: &str) -> Option<String>;
    /// Contenido de `/proc/1/cgroup` si se puede leer.
    fn proc1_cgroup(&self) -> Option<String>;
}

/// Implementación real sobre syscalls/entorno del proceso.
pub struct RealIo;

impl Io for RealIo {
    fn euid(&self) -> u32 {
        nix::unistd::geteuid().as_raw()
    }
    fn file_exists(&self, path: &Path) -> bool {
        std::fs::metadata(path).is_ok()
    }
    fn file_mode(&self, path: &Path) -> Option<u32> {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .ok()
            .map(|m| m.permissions().mode() & 0o7777)
    }
    fn getenv(&self, key: &str) -> Option<String> {
        std::env::var(key).ok().filter(|v| !v.is_empty())
    }
    fn proc1_cgroup(&self) -> Option<String> {
        std::fs::read_to_string("/proc/1/cgroup").ok()
    }
}

/// Ejecuta los chequeos P0-1 en orden de spec. El primer aborto corta
/// (domina sobre avisos posteriores); los avisos se acumulan.
pub fn check(io: &dyn Io) -> Report {
    let mut report = Report::default();

    // 1. root directo: abortar (ver doc del módulo).
    if io.euid() == 0 {
        report.abort = Some(
            "vary no debe ejecutarse como root directo (euid 0): usa tu usuario \
             normal (la elevación puntual la gestiona vary vía --sudo/doas/run0); \
             xbps-src se niega a correr como root"
                .to_string(),
        );
        return report;
    }

    // 2. chroot degradado: aviso crítico, seguir.
    if io.file_exists(Path::new("/.flatpak-info"))
        || io.getenv("FLATPAK_ID").is_some()
        || io.getenv("PROOT_TMP_DIR").is_some()
    {
        report.warnings.push(
            "chroot degradado detectado (proot/bwrap): los builds en masterdir \
             pueden fallar o comportarse raro; preferí Void nativo para compilar"
                .to_string(),
        );
    }

    // 3. xbps-uchroot sin 4750: abortar con hint accionable.
    if let Some(path) = UCHROOT_CANDIDATES
        .iter()
        .map(Path::new)
        .find(|p| io.file_exists(p))
    {
        match io.file_mode(path) {
            Some(UCHROOT_MODE) => {}
            Some(mode) => {
                report.abort = Some(format!(
                    "xbps-uchroot en {} con modo {:o} (se exige 4750 para builds \
                     en chroot); corrige con: doas chmod 4750 {} (o reinstala xbps)",
                    path.display(),
                    mode,
                    path.display()
                ));
                return report;
            }
            // Ilegible o ausente tras existir: no se puede verificar;
            // xbps-src lo reportará al compilar. Nunca abortar a ciegas.
            None => {}
        }
    }

    // 4. OCI: aviso leve, seguir.
    let cgroup_hit = io
        .proc1_cgroup()
        .is_some_and(|c| c.contains("docker") || c.contains("kubepods"));
    if io.file_exists(Path::new("/.dockerenv")) || cgroup_hit {
        report.warnings.push(
            "entorno OCI/contenedor detectado: algunas operaciones privilegiadas \
             (chroot, montajes) pueden fallar; los installs binarios no se ven afectados"
                .to_string(),
        );
    }

    report
}

/// Chequeo con el entorno real del proceso.
pub fn check_real() -> Report {
    check(&RealIo)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::path::PathBuf;

    /// Entorno simulado para la tabla de aceptación P0-1.
    struct FakeIo {
        euid: u32,
        /// ruta -> (existe, modo enmascarado o None si ilegible)
        files: HashMap<PathBuf, (bool, Option<u32>)>,
        env: HashMap<String, String>,
        cgroup: Option<String>,
    }

    impl FakeIo {
        fn normal() -> Self {
            Self {
                euid: 1000,
                files: [(PathBuf::from("/usr/bin/xbps-uchroot"), (true, Some(0o4750)))].into(),
                env: HashMap::new(),
                cgroup: Some("0::/init.scope\n".to_string()),
            }
        }
    }

    impl Io for FakeIo {
        fn euid(&self) -> u32 {
            self.euid
        }
        fn file_exists(&self, path: &Path) -> bool {
            self.files.get(path).is_some_and(|(ex, _)| *ex)
        }
        fn file_mode(&self, path: &Path) -> Option<u32> {
            self.files.get(path).and_then(|(_, m)| *m)
        }
        fn getenv(&self, key: &str) -> Option<String> {
            self.env.get(key).cloned().filter(|v| !v.is_empty())
        }
        fn proc1_cgroup(&self) -> Option<String> {
            self.cgroup.clone()
        }
    }

    fn assert_ok(r: &Report) {
        assert!(r.abort.is_none(), "aborto inesperado: {:?}", r.abort);
        assert!(
            r.warnings.is_empty(),
            "avisos inesperados: {:?}",
            r.warnings
        );
    }

    /// P0-1: entorno normal pasa en silencio.
    #[test]
    fn entorno_normal_pasa_en_silencio() {
        assert_ok(&check(&FakeIo::normal()));
    }

    /// P0-1: root directo aborta aunque todo lo demás esté mal.
    #[test]
    fn root_directo_aborta() {
        let mut io = FakeIo::normal();
        io.euid = 0;
        io.files
            .insert(PathBuf::from("/.dockerenv"), (true, Some(0o644)));
        let r = check(&io);
        let abort = r.abort.as_ref().expect("root debe abortar");
        assert!(abort.contains("root directo"), "{abort}");
        assert!(r.warnings.is_empty(), "el aborto corta avisos: {r:?}");
    }

    /// P0-1: chroot degradado avisa (crítico) y sigue, por cada marcador.
    #[test]
    fn chroot_degradado_avisa_y_sigue() {
        for (path, var) in [
            (Some("/.flatpak-info"), None),
            (None, Some("FLATPAK_ID")),
            (None, Some("PROOT_TMP_DIR")),
        ] {
            let mut io = FakeIo::normal();
            if let Some(p) = path {
                io.files.insert(PathBuf::from(p), (true, Some(0o644)));
            }
            if let Some(v) = var {
                io.env.insert(v.to_string(), "1".to_string());
            }
            let r = check(&io);
            assert!(r.abort.is_none(), "{r:?}");
            assert_eq!(r.warnings.len(), 1, "{r:?}");
            assert!(r.warnings[0].contains("degradado"), "{r:?}");
        }
    }

    /// P0-1: uchroot con modo != 4750 aborta con hint (incluye 4755:
    /// estricto según spec).
    #[test]
    fn uchroot_sin_4750_aborta_con_hint() {
        for mode in [0o755, 0o644, 0o2750, 0o4755] {
            let mut io = FakeIo::normal();
            io.files
                .insert(PathBuf::from("/usr/bin/xbps-uchroot"), (true, Some(mode)));
            let r = check(&io);
            let abort = r
                .abort
                .unwrap_or_else(|| panic!("modo {mode:o} debe abortar"));
            assert!(abort.contains("4750"), "{abort}");
            assert!(abort.contains("chmod"), "{abort}");
        }
    }

    /// P0-1: segundo candidato vale; ausente o ilegible no aborta.
    #[test]
    fn uchroot_alternativo_y_ausente_no_abortan() {
        // Solo en /usr/libexec con 4750: ok.
        let mut io = FakeIo::normal();
        io.files.remove(&PathBuf::from("/usr/bin/xbps-uchroot"));
        io.files.insert(
            PathBuf::from("/usr/libexec/xbps-uchroot"),
            (true, Some(0o4750)),
        );
        assert_ok(&check(&io));
        // Ausente en ambos: se omite (xbps-src lo dirá al compilar).
        let io = FakeIo {
            files: HashMap::new(),
            ..FakeIo::normal()
        };
        assert_ok(&check(&io));
        // Existe pero ilegible: no abortar a ciegas.
        let mut io = FakeIo::normal();
        io.files
            .insert(PathBuf::from("/usr/bin/xbps-uchroot"), (true, None));
        assert_ok(&check(&io));
    }

    /// P0-1: OCI avisa leve y sigue; admite ambos marcadores y los acumula
    /// con chroot degradado.
    #[test]
    fn oci_avisa_leve_y_sigue() {
        let mut io = FakeIo::normal();
        io.files
            .insert(PathBuf::from("/.dockerenv"), (true, Some(0o644)));
        let r = check(&io);
        assert!(r.abort.is_none());
        assert_eq!(r.warnings.len(), 1);
        assert!(r.warnings[0].contains("OCI"), "{:?}", r.warnings);

        let mut io = FakeIo::normal();
        io.cgroup = Some("11:devices:/docker/ab12cd34\n".to_string());
        let r = check(&io);
        assert!(r.abort.is_none());
        assert_eq!(r.warnings.len(), 1);

        let mut io = FakeIo::normal();
        io.cgroup = Some("3:cpu:/kubepods/pod99\n".to_string());
        io.files
            .insert(PathBuf::from("/.flatpak-info"), (true, Some(0o644)));
        let r = check(&io);
        assert!(r.abort.is_none());
        assert_eq!(r.warnings.len(), 2, "{r:?}");
    }
}
