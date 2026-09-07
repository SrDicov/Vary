//! Diario de operaciones (P2): `vary --log`.
//!
//! Append-only en `<data_dir>/operations.log` (sobrevive a limpiezas de
//! caché): una línea por paquete operado, `<epoch> <OP> <pkg> <detalle...>`.
//! `INSTALL` lleva `pkgver repo`; `REMOVE` sin detalle. Los escritores son
//! best-effort con aviso (el diario nunca debe abortar una operación).

use std::path::Path;

use anyhow::{Context, Result};

/// Nombre del diario dentro de `data_dir`.
pub const JOURNAL_FILE: &str = "operations.log";

/// P2: añade una línea al diario (crea el archivo si falta). Best-effort en
/// los llamadores: ante error avisan, no fallan.
pub fn append(data_dir: &Path, op: &str, pkg: &str, detail: &str) -> Result<()> {
    use std::io::Write;
    let path = data_dir.join(JOURNAL_FILE);
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            crate::util::ensure_private_dir(parent)
                .with_context(|| format!("no se pudo crear {}", parent.display()))?;
        }
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut line = format!("{now} {op} {pkg}");
    if !detail.trim().is_empty() {
        line.push(' ');
        line.push_str(detail.trim());
    }
    line.push('\n');
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("no se pudo abrir {}", path.display()))?;
    file.write_all(line.as_bytes())
        .with_context(|| format!("no se pudo escribir {}", path.display()))?;
    Ok(())
}

/// P2: una línea parseada (epoch, op, pkg, detalle).
pub fn parse_line(line: &str) -> Option<(u64, String, String, String)> {
    let mut parts = line.split_whitespace();
    let epoch: u64 = parts.next()?.parse().ok()?;
    let op = parts.next()?.to_string();
    let pkg = parts.next()?.to_string();
    let detail: String = parts.collect::<Vec<_>>().join(" ");
    if op.is_empty() || pkg.is_empty() {
        return None;
    }
    Some((epoch, op, pkg, detail))
}

/// P2: muestra el diario (`filter` opcional por paquete exacto).
pub fn show(data_dir: &Path, filter: Option<&str>) -> Result<i32> {
    let path = data_dir.join(JOURNAL_FILE);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            println!("sin historial (aún no hay operaciones registradas)");
            return Ok(0);
        }
        Err(e) => return Err(e).with_context(|| format!("no se pudo leer {}", path.display())),
    };
    let mut shown = 0;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match parse_line(line) {
            Some((_, _, pkg, _)) if filter.is_some_and(|f| f != pkg) => {}
            _ => {
                println!("{line}");
                shown += 1;
            }
        }
    }
    if shown == 0 {
        println!("sin historial para ese filtro");
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_parse_roundtrip_y_filtro() {
        let dir = tempfile::tempdir().unwrap();
        append(dir.path(), "INSTALL", "a", "a-1.0_1 r").unwrap();
        append(dir.path(), "REMOVE", "a", "").unwrap();
        append(dir.path(), "INSTALL", "b", "b-2.0_1 r").unwrap();

        let text = std::fs::read_to_string(dir.path().join(JOURNAL_FILE)).unwrap();
        assert_eq!(text.lines().count(), 3);
        let (epoch, op, pkg, detail) = parse_line(text.lines().next().unwrap()).unwrap();
        assert!(epoch > 1_700_000_000);
        assert_eq!(op, "INSTALL");
        assert_eq!(pkg, "a");
        assert_eq!(detail, "a-1.0_1 r");

        // Línea rota: None (no paniquea).
        assert!(parse_line("basura").is_none());
        assert!(parse_line("123 INSTALL").is_none());
        assert!(parse_line("").is_none());
    }

    #[test]
    fn show_sin_archivo_no_falla() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(show(dir.path(), None).unwrap(), 0);
    }
}
