//! Elevación de privilegios agnóstica.
//!
//! vary no depende de sudo: sirve cualquier wrapper con la forma
//! `BIN [flags] comando args...` — sudo, doas (opendoas), run0 (systemd)
//! o sudo-rs.
//!
//! Resolución, idéntica en todos los puntos de elevación:
//! 1. Binario configurado explícitamente (`--sudo` o `sudo_bin` en
//!    vary.conf): se usa tal cual, junto a sus flags (`--sudoflags`).
//! 2. Sin configurar y ejecutando como root (uid 0): sin wrapper.
//! 3. Sin configurar y usuario normal: autodetección en PATH
//!    (sudo → doas → run0).
use anyhow::{anyhow, Result};
use std::process::Command;

/// true si el proceso corre con uid 0.
pub fn running_as_root() -> bool {
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::MetadataExt;
        std::fs::metadata("/proc/self")
            .map(|m| m.uid() == 0)
            .unwrap_or(false)
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

/// Primera coincidencia absoluta y ejecutable de `bin` en $PATH.
fn find_in_path(bin: &str) -> Option<String> {
    use std::os::unix::fs::PermissionsExt;
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths).find_map(|dir| {
        let cand = dir.join(bin);
        std::fs::metadata(&cand)
            .ok()
            .filter(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            .map(|_| cand.to_string_lossy().into_owned())
    })
}

/// Variables que nunca deben heredar los hijos elevados (H-040): el cargador
/// dinámico y los arranques de shell permiten inyectar código en procesos que
/// corren como root vía doas/sudo mal configurado. (No se hace `env_clear`
/// total para no romper `http_proxy` y demás entorno legítimo.)
const DANGEROUS_ENV: &[&str] = &[
    "LD_PRELOAD",
    "LD_LIBRARY_PATH",
    "LD_AUDIT",
    "BASH_ENV",
    "ENV",
    "ZDOTDIR",
    "IFS",
];

/// PATH mínimo y absoluto para los hijos del wrapper (H-040): evita que doas
/// (que no fija secure_path como sudo) resuelva el programa en dirs raros.
const SAFE_PATH: &str = "/usr/sbin:/usr/bin:/sbin:/bin";

/// Primer wrapper de elevación disponible en PATH (ruta absoluta, H-040).
pub fn detect() -> Result<String> {
    ["sudo", "doas", "run0"]
        .iter()
        .find_map(|b| find_in_path(b))
        .ok_or_else(|| {
            anyhow!(
                "no hay herramienta de elevación de privilegios (busqué sudo, doas y run0 en PATH);\n\
                 instala una (p. ej. `sudo xbps-install -S opendoas`) o ejecuta vary como root"
            )
        })
}

/// Resuelve cómo elevar privilegios: `None` = ejecutar el programa
/// directamente (somos root); `Some((bin, flags))` = anteponer wrapper.
pub fn resolve(
    configured_bin: &str,
    configured_flags: &[String],
) -> Result<Option<(String, Vec<String>)>> {
    if !configured_bin.is_empty() {
        // H-040: el wrapper queda en ruta absoluta verificada (nada de
        // resolución tardía vía PATH manipulable en el exec).
        let abs = if configured_bin.contains('/') {
            if is_executable_file(configured_bin) {
                configured_bin.to_string()
            } else {
                anyhow::bail!("wrapper de elevación no ejecutable: {configured_bin}");
            }
        } else {
            find_in_path(configured_bin).ok_or_else(|| {
                anyhow!("wrapper de elevación no encontrado en PATH: {configured_bin}")
            })?
        };
        return Ok(Some((abs, configured_flags.to_vec())));
    }
    if running_as_root() {
        return Ok(None);
    }
    Ok(Some((detect()?, Vec::new())))
}

/// true si `path` (con `/`) es archivo ejecutable.
fn is_executable_file(path: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

/// Construye el `Command` para ejecutar `program` con los privilegios adecuados.
///
/// Higiene de entorno (H-040): quita variables de cargador/arranque de shell
/// y fija un PATH mínimo en la rama elevada (doas no fija secure_path).
pub fn elevate(
    configured_bin: &str,
    configured_flags: &[String],
    program: &str,
) -> Result<Command> {
    Ok(match resolve(configured_bin, configured_flags)? {
        None => {
            let mut cmd = Command::new(program);
            sanitize_env(&mut cmd, false);
            cmd
        }
        Some((bin, flags)) => {
            let mut cmd = Command::new(&bin);
            cmd.args(flags).arg(program);
            sanitize_env(&mut cmd, true);
            cmd
        }
    })
}

fn sanitize_env(cmd: &mut Command, elevated: bool) {
    for var in DANGEROUS_ENV {
        cmd.env_remove(var);
    }
    if elevated {
        cmd.env("PATH", SAFE_PATH);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binario_explicito_gana_siempre() {
        // Aunque seamos root o haya sudo en PATH, lo explícito manda.
        // (/bin/sh existe en cualquier unix; /usr/bin/doas quizá no.)
        let r = resolve("/bin/sh", &["-n".to_string()]).unwrap();
        assert_eq!(r, Some(("/bin/sh".to_string(), vec!["-n".to_string()])));
    }

    #[test]
    fn wrapper_inexistente_falla_en_resolve_no_en_exec() {
        // H-040: ni ruta fantasma ni nombre ausente llegan al exec.
        assert!(resolve("/no/existe-xyz", &[]).is_err());
        assert!(resolve("vary-no-existe-xyz", &[]).is_err());
    }

    #[test]
    fn wrapper_relativo_se_resuelve_a_absoluto() {
        // H-040: sin resolución tardía vía PATH manipulable.
        let (bin, _) = resolve("sh", &[]).expect("sh existe").expect("some");
        assert!(
            bin.starts_with('/'),
            "el wrapper debe quedar absoluto: {bin}"
        );
    }

    #[test]
    fn root_sin_config_ejecuta_directo() {
        if !running_as_root() {
            eprintln!("host no es root; caso root cubierto en contenedores CI");
            return;
        }
        assert_eq!(resolve("", &[]).unwrap(), None);
    }

    #[test]
    fn usuario_normal_autodetecta_wrapper_conocido() {
        if running_as_root() {
            eprintln!("corriendo como root: no hay autodetección; nada que probar");
            return;
        }
        let r = resolve("", &[])
            .unwrap()
            .expect("host de desarrollo debe tener sudo/doas/run0");
        let base = std::path::Path::new(&r.0)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        assert!(
            ["sudo", "doas", "run0"].contains(&base),
            "detectado inesperado: {}",
            r.0
        );
        // La autodetección nunca añade flags propios.
        assert!(r.1.is_empty());
    }

    #[test]
    fn elevate_explicito_prefija_el_programa() {
        // Wrapper 'echo': el stdout debe ser exactamente el programa envuelto,
        // probando que el programa queda como argumento y no como argv[0].
        let out = elevate("echo", &[], "/bin/true").unwrap().output().unwrap();
        assert!(out.status.success());
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "/bin/true");
    }

    #[test]
    fn in_path_resuelve_binarios_reales_y_fantasma() {
        assert!(find_in_path("sh").is_some());
        assert!(find_in_path("vary-no-existe-xyz").is_none());
    }
}
