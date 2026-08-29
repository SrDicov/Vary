use anyhow::Result;
use std::process::{Command, Stdio};
use std::path::Path;

pub fn prompt_review(pkg_name: &str, clone_dir: &Path) -> Result<()> {
    use std::io::Write;

    println!("Reviewing changes for {} in {}...", pkg_name, clone_dir.display());
    
    // Use git show to present the latest changes from the local clone
    let git_cmd = Command::new("git")
        .current_dir(clone_dir)
        .args(["show", "HEAD", "--color=always"])
        .output()?;
        
    let diff_output = if git_cmd.status.success() {
        String::from_utf8_lossy(&git_cmd.stdout).to_string()
    } else {
        // Fallback: just read the template if git fails
        let template_path = clone_dir.join(pkg_name).join("template");
        if template_path.exists() {
            std::fs::read_to_string(template_path).unwrap_or_default()
        } else {
            String::new()
        }
    };
    
    if diff_output.is_empty() {
        return Ok(());
    }

    // Try bat first, fallback to less
    let mut pager = Command::new("bat")
        .args(["--paging=always"])
        .stdin(Stdio::piped())
        .spawn()
        .or_else(|_| {
            Command::new("less")
                .args(["-R"])
                .stdin(Stdio::piped())
                .spawn()
        })?;

    if let Some(mut stdin) = pager.stdin.take() {
        let _ = stdin.write_all(diff_output.as_bytes());
    }

    let _ = pager.wait()?;
    Ok(())
}
