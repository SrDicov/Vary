use crate::vur_client::TEMPLATE_PREFIXES;
use anyhow::Result;
use std::path::Path;
use std::process::{Command, Stdio};

/// (binario, args) del paginador (H-045): `$PAGER` (p. ej. `less -R`, `most`)
/// gana; si no, `bat` con resaltado bash. `less -R` queda como último recurso
/// en el llamador si el spawn falla.
pub(crate) fn pager_cmd() -> (String, Vec<String>) {
    if let Some(pager) = std::env::var_os("PAGER") {
        let pager = pager.to_string_lossy();
        let mut parts = pager.split_whitespace();
        if let Some(bin) = parts.next() {
            return (bin.to_string(), parts.map(|s| s.to_string()).collect());
        }
    }
    (
        "bat".to_string(),
        vec![
            "--paging=always".to_string(),
            "--language=bash".to_string(),
            "--style=plain".to_string(),
        ],
    )
}

/// Lee el template de un paquete sin checkout (git show) o desde disco si ya
/// está materializado. Vacío si no se encuentra (llamadores: silencio).
pub fn read_template_text(pkg_name: &str, clone_dir: &Path, git_bin: &str) -> String {
    // Intentar leer vía git show (funciona sin checkout).
    // Prefijos conocidos + "" (flat); el primero que acierte gana.
    let mut prefixes: Vec<String> = TEMPLATE_PREFIXES.iter().map(|s| s.to_string()).collect();
    prefixes.push(String::new());

    for prefix in &prefixes {
        let path = if prefix.is_empty() {
            format!("{pkg_name}/template")
        } else {
            format!("{prefix}/{pkg_name}/template")
        };

        let output = Command::new(git_bin)
            .arg("-C")
            .arg(clone_dir)
            .args(["show", &format!("HEAD:{path}")])
            .output();

        if let Ok(out) = output {
            if out.status.success() {
                return String::from_utf8_lossy(&out.stdout).to_string();
            }
        }
    }

    // Fallback: leer desde disco si ya materializado
    let mut candidates: Vec<std::path::PathBuf> = TEMPLATE_PREFIXES
        .iter()
        .map(|p| clone_dir.join(p).join(pkg_name).join("template"))
        .collect();
    candidates.push(clone_dir.join(pkg_name).join("template"));
    candidates
        .iter()
        .find(|p| p.exists())
        .and_then(|p| std::fs::read_to_string(p).ok())
        .unwrap_or_default()
}

/// P0-3: imprime SOLO el encabezado de hallazgos (sin pager, sin prompt).
/// Para corridas `--yes`/no-TTY donde `prompt_review` no corre: los HIGH
/// siguen visibles aunque nada pregunte ni bloquee. Silencio si no hay nada.
pub fn print_audit_header(pkg_name: &str, repo_name: &str, content: &str) {
    let header = crate::template_audit::format_findings(
        pkg_name,
        repo_name,
        &crate::template_audit::audit_template(content),
    );
    if !header.is_empty() {
        print!("{header}");
    }
}

pub fn prompt_review(
    pkg_name: &str,
    repo_name: &str,
    clone_dir: &Path,
    git_bin: &str,
) -> Result<()> {
    use std::io::Write;

    println!(
        "Reviewing changes for {} in {}...",
        pkg_name,
        clone_dir.display()
    );

    let content = read_template_text(pkg_name, clone_dir, git_bin);
    if content.is_empty() {
        return Ok(());
    }

    // P0-3: hallazgos sobre el template (consultivos; el gate decide igual).
    print_audit_header(pkg_name, repo_name, &content);

    use std::io::IsTerminal;
    if !std::io::stdout().is_terminal() {
        // En entornos no interactivos (CI, pipes), imprimir plano directamente
        println!("{content}");
        return Ok(());
    }

    // Paginador: $PAGER manda (convención unix, H-045); si no, bat y luego less.
    let (pager_bin, pager_args) = pager_cmd();
    let mut pager = Command::new(&pager_bin)
        .args(&pager_args)
        .stdin(Stdio::piped())
        .spawn()
        .or_else(|_| {
            Command::new("less")
                .args(["-R"])
                .stdin(Stdio::piped())
                .spawn()
        })?;

    if let Some(mut stdin) = pager.stdin.take() {
        let _ = stdin.write_all(content.as_bytes());
    }

    // Nota (H-034): el pager NO se registra en signal::CHILDREN a propósito.    // El observador mata por GRUPO (-pid) y el pager no es líder de grupo
    // (compar­te el frontal para recibir SIGINT directo); registrarlo
    // arriesgaría matar un grupo ajeno por reutilización de pid. El caso
    // residual (SIGTERM solo a vary con pager abierto) es benigno: el pager
    // no retiene locks ni estado, solo queda visible hasta que el usuario
    // salga de él.
    let _ = pager.wait()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_review_imprime_plano_en_no_tty() {
        // En CI/pipes stdout no es TTY: usa el fallback en disco e imprime
        // plano sin invocar paginadores. (cargo test captura stdout: nunca TTY.)
        let dir = tempfile::tempdir().expect("tempdir");
        let pkgdir = dir.path().join("srcpkgs").join("foo");
        std::fs::create_dir_all(&pkgdir).expect("mkdir");
        std::fs::write(pkgdir.join("template"), "pkgname=foo\n").expect("write");
        assert!(prompt_review("foo", "mi-repo", dir.path(), "git").is_ok());
    }

    #[test]
    fn prompt_review_sin_template_es_ok_silencioso() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(prompt_review("inexistente", "mi-repo", dir.path(), "git").is_ok());
    }

    #[test]
    fn pager_cmd_respeta_pager_y_defecto_bat() {
        // H-045: $PAGER con flags se parte; vacío/ausente => bat.
        let prev = std::env::var_os("PAGER");
        std::env::set_var("PAGER", "less -R");
        assert_eq!(pager_cmd(), ("less".to_string(), vec!["-R".to_string()]));
        std::env::remove_var("PAGER");
        let (bin, args) = pager_cmd();
        assert_eq!(bin, "bat");
        assert!(args.contains(&"--paging=always".to_string()));
        if let Some(v) = prev {
            std::env::set_var("PAGER", v);
        }
    }
}
