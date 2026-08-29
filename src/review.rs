use anyhow::Result;
use std::process::{Command, Stdio};
use std::path::Path;

pub fn prompt_review(pkg_name: &str, clone_dir: &Path, git_bin: &str) -> Result<()> {
    use std::io::Write;

    println!("Reviewing changes for {} in {}...", pkg_name, clone_dir.display());
    
    // Intentar leer vía git show (funciona sin checkout)
    let prefixes = ["srcpkgs", ""];
    let mut content = String::new();
    
    for prefix in &prefixes {
        let path = if prefix.is_empty() {
            format!("{}/template", pkg_name)
        } else {
            format!("{}/{}/template", prefix, pkg_name)
        };
        
        let output = Command::new(git_bin)
            .arg("-C").arg(clone_dir)
            .args(["show", &format!("HEAD:{}", path)])
            .output();
        
        if let Ok(out) = output {
            if out.status.success() {
                content = String::from_utf8_lossy(&out.stdout).to_string();
                break;
            }
        }
    }
    
    if content.is_empty() {
        // Fallback: leer desde disco si ya materializado
        let candidates = [
            clone_dir.join("srcpkgs").join(pkg_name).join("template"),
            clone_dir.join(pkg_name).join("template"),
        ];
        if let Some(path) = candidates.iter().find(|p| p.exists()) {
            content = std::fs::read_to_string(path).unwrap_or_default();
        }
    }
    
    if content.is_empty() {
        return Ok(());
    }

    // Try bat first, fallback to less
    let mut pager = Command::new("bat")
        .args(["--paging=always", "--language=bash", "--style=plain"])
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

    let _ = pager.wait()?;
    Ok(())
}
