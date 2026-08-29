use anyhow::Result;
use std::process::{Command, Stdio};
use std::path::Path;

pub fn prompt_review(pkg_name: &str, clone_dir: &Path) -> Result<()> {
    use std::io::Write;

    println!("Reviewing changes for {} in {}...", pkg_name, clone_dir.display());
    
    let candidates = [
        clone_dir.join("srcpkgs").join(pkg_name).join("template"),
        clone_dir.join(pkg_name).join("template"),
    ];
    
    let template_path = candidates.iter().find(|p| p.exists()).cloned();
    
    let content = match template_path {
        Some(path) => std::fs::read_to_string(&path).unwrap_or_default(),
        None => return Ok(()),
    };
    
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
