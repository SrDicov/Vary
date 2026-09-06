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

/// Busca un ejecutable por nombre en $PATH (verifica permiso de ejecución).
fn in_path(bin: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;
    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&paths).any(|dir| {
        std::fs::metadata(dir.join(bin))
            .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    })
}

/// Primer wrapper de elevación disponible en PATH.
pub fn detect() -> Result<String> {
    ["sudo", "doas", "run0"]
        .iter()
        .find(|b| in_path(b))
        .map(|b| b.to_string())
        .ok_or_else(|| {
            anyhow!(
                "no hay herramienta de elevación de privilegios (busqué sudo, doas y run0 en PATH);\n\
                 instala una (p. ej. `xbps-install opendoas`) o ejecuta vary como root"
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
        return Ok(Some((
            configured_bin.to_string(),
            configured_flags.to_vec(),
        )));
    }
    if running_as_root() {
        return Ok(None);
    }
    Ok(Some((detect()?, Vec::new())))
}

/// Construye el `Command` para ejecutar `program` con los privilegios adecuados.
pub fn elevate(
    configured_bin: &str,
    configured_flags: &[String],
    program: &str,
) -> Result<Command> {
    Ok(match resolve(configured_bin, configured_flags)? {
        None => Command::new(program),
        Some((bin, flags)) => {
            let mut cmd = Command::new(&bin);
            cmd.args(flags).arg(program);
            cmd
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binario_explicito_gana_siempre() {
        // Aunque seamos root o haya sudo en PATH, lo explícito manda.
        let r = resolve("/usr/bin/doas", &["-n".to_string()]).unwrap();
        assert_eq!(
            r,
            Some(("/usr/bin/doas".to_string(), vec!["-n".to_string()]))
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
        assert!(
            ["sudo", "doas", "run0"].contains(&r.0.as_str()),
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
        assert!(in_path("sh"));
        assert!(!in_path("vary-no-existe-xyz"));
    }
}
