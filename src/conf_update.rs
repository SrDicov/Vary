//! `vary --conf-update`: resuelve los ficheros `<fichero>.new-<pkgver>` que
//! deja xbps en `/etc` tras una actualización. Sin TTY (o con `--noconfirm`)
//! solo los lista a stdout y sale 0; con TTY muestra `git diff --no-index`
//! y pregunta por cada uno. La escritura en /etc siempre pasa por
//! `crate::elevate` (nunca `std::fs` directo).

use crate::config::Config;
use anyhow::Result;
use std::fs;
use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

const ETC: &str = "/etc";

/// `passwd.new-0.146_2` → `passwd`; `foo.new-`/`foo` → None (sufijo vacío
/// o sin marca). Solo ficheros regulares en el mismo directorio cuentan.
fn original_for(file_name: &str) -> Option<String> {
    let idx = file_name.rfind(".new-")?;
    let suffix = &file_name[idx + 5..];
    if suffix.is_empty() || idx == 0 {
        return None;
    }
    Some(file_name[..idx].to_string())
}

/// Escaneo recursivo sin seguir symlinks: pares (nuevo, original) ordenados
/// por ruta.
fn scan_pairs(root: &Path, out: &mut Vec<(PathBuf, PathBuf)>) -> std::io::Result<()> {
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let ft = entry.file_type()?; // no sigue el symlink
        let path = entry.path();
        if ft.is_dir() {
            scan_pairs(&path, out)?;
        } else if ft.is_file() {
            if let Some(orig) = original_for(&entry.file_name().to_string_lossy()) {
                out.push((path.clone(), path.with_file_name(orig)));
            }
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Choice {
    Replace,
    Keep,
    Diff,
    Quit,
}

/// `""` (enter) = keep; inválida = None (re-pregunta).
fn parse_choice(line: &str) -> Option<Choice> {
    match line.trim().to_ascii_lowercase().as_str() {
        "" | "k" => Some(Choice::Keep),
        "r" => Some(Choice::Replace),
        "d" => Some(Choice::Diff),
        "q" => Some(Choice::Quit),
        _ => None,
    }
}

fn show_diff(config: &Config, orig: &Path, new: &Path) -> std::io::Result<()> {
    Command::new(&config.git_bin)
        .args(["diff", "--no-index", "--no-color", "--"])
        .arg(if orig.exists() {
            orig.as_os_str()
        } else {
            std::ffi::OsStr::new("/dev/null")
        })
        .arg(new)
        .status()?;
    Ok(())
}

/// Reemplaza `` `orig` por `new`; devuelve Ok(true) si todo fue bien.
/// Backup `<orig>.vary-bak.<pid>` con `cp -p --`, luego `mv --`.
fn replace(config: &Config, orig: &Path, new: &Path) -> Result<bool> {
    let name = orig.file_name().unwrap_or_default().to_string_lossy();
    let bak = orig.with_file_name(format!("{name}.vary-bak.{}", std::process::id()));
    if orig.exists() {
        let ok = crate::elevate::elevate(&config.sudo_bin, &config.sudo_flags, "cp")?
            .arg("-p")
            .arg("--")
            .arg(orig)
            .arg(&bak)
            .status()?
            .success();
        if !ok {
            tracing::warn!(
                "backup falló para {}: fichero no reemplazado",
                orig.display()
            );
            return Ok(false);
        }
    }
    let ok = crate::elevate::elevate(&config.sudo_bin, &config.sudo_flags, "mv")?
        .arg("--")
        .arg(new)
        .arg(orig)
        .status()?
        .success();
    if !ok {
        tracing::warn!("reemplazo falló para {}", orig.display());
        return Ok(false);
    }
    Ok(true)
}

pub fn run(config: &Config) -> Result<i32> {
    let mut pairs = Vec::new();
    if let Err(err) = scan_pairs(Path::new(ETC), &mut pairs) {
        tracing::warn!("no se pudo escanear {ETC}: {err}");
    }
    pairs.sort_by(|a, b| a.0.cmp(&b.0));

    if pairs.is_empty() {
        return Ok(0);
    }

    let list_only = config.no_confirm || !std::io::stdin().is_terminal();
    if list_only {
        for (new, orig) in &pairs {
            println!("{} -> {}", new.display(), orig.display());
        }
        return Ok(0);
    }

    let mut failures = 0usize;
    for (new, orig) in &pairs {
        println!("{} -> {}", orig.display(), new.display());
        if show_diff(config, orig, new).is_err() {
            tracing::warn!("no se pudo mostrar el diff de {}", orig.display());
        }
        let choice = loop {
            print!("[r]eemplazar [k]eep [d]iff [q]uit: ");
            let _ = std::io::stdout().flush();
            let mut line = String::new();
            let n = std::io::stdin().lock().read_line(&mut line)?;
            // EOF o enter vacío: keep (default).
            match if n == 0 {
                Some(Choice::Keep)
            } else {
                parse_choice(&line)
            } {
                Some(Choice::Diff) => {
                    // `d` re-muestra el diff y re-pregunta.
                    if show_diff(config, orig, new).is_err() {
                        tracing::warn!("no se pudo mostrar el diff de {}", orig.display());
                    }
                    continue;
                }
                Some(c) => break c,
                None => {
                    println!("opción no válida");
                    continue;
                }
            }
        };
        match choice {
            Choice::Replace => {
                if !replace(config, orig, new)? {
                    failures += 1;
                }
            }
            Choice::Quit => return Ok(if failures == 0 { 0 } else { 1 }),
            Choice::Keep | Choice::Diff => {}
        }
    }
    Ok(if failures == 0 { 0 } else { 1 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn original_for_parsea() {
        assert_eq!(
            original_for("passwd.new-0.146_2").as_deref(),
            Some("passwd")
        );
        assert_eq!(original_for("foo.new-"), None);
        assert_eq!(original_for("foo"), None);
        assert_eq!(original_for(".new-1"), None); // prefijo vacío, sin original
    }

    #[test]
    fn escaneo_recursivo_y_orden_sin_seguir_symlinks() {
        let dir = std::env::temp_dir().join(format!("vary-conf-update-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("sub")).expect("sub");
        fs::write(dir.join("a.new-1.0_1"), "a\n").expect("a");
        fs::write(dir.join("sub/b.new-2_1"), "b\n").expect("b");
        fs::write(dir.join("normal"), "n\n").expect("normal");

        let mut pairs = Vec::new();
        scan_pairs(&dir, &mut pairs).expect("scan");
        pairs.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(
            pairs,
            vec![
                (dir.join("a.new-1.0_1"), dir.join("a")),
                (dir.join("sub/b.new-2_1"), dir.join("sub/b")),
            ]
        );

        // symlink a fichero regular: file_type() no sigue, no aparece.
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(dir.join("normal"), dir.join("fake.new-3_1"))
                .expect("symlink");
            let mut pairs = Vec::new();
            scan_pairs(&dir, &mut pairs).expect("scan2");
            assert_eq!(
                pairs
                    .iter()
                    .filter(|p| p.0.ends_with("fake.new-3_1"))
                    .count(),
                0
            );
        }
        fs::remove_dir_all(&dir).expect("limpieza");
    }

    #[test]
    fn parseo_de_eleccion() {
        assert_eq!(parse_choice("r"), Some(Choice::Replace));
        assert_eq!(parse_choice("K"), Some(Choice::Keep));
        assert_eq!(parse_choice("d"), Some(Choice::Diff));
        assert_eq!(parse_choice("q"), Some(Choice::Quit));
        assert_eq!(parse_choice("  "), Some(Choice::Keep)); // enter
        assert_eq!(parse_choice("x"), None);
        assert_eq!(parse_choice("rr"), None);
    }
}
