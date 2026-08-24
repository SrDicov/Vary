use crate::config::Config;

use std::io::{stdin, stdout, Write};

use anyhow::Result;

pub fn ask(config: &Config, question: &str, default: bool) -> bool {
    let action = config.color.action;
    let bold = config.color.bold;
    let yn = if default { "[Y/n]:" } else { "[y/N]:" };
    print!("{} {} {} ", action.paint("::"), bold.paint(question), bold.paint(yn));
    let _ = stdout().lock().flush();
    if config.no_confirm {
        println!();
        return default;
    }
    let stdin = stdin();
    let mut input = String::new();
    let _ = stdin.read_line(&mut input);
    let input = input.to_lowercase();
    let input = input.trim();
    if input == "y" || input == "yes" {
        true
    } else if input.is_empty() {
        default
    } else {
        false
    }
}

/// Confirmación y/n compartida por install/keys. `no_confirm` => true.
pub fn confirm(prompt: &str, no_confirm: bool) -> Result<bool> {
    if no_confirm {
        return Ok(true);
    }
    print!("{} [Y/n] ", prompt);
    let _ = stdout().lock().flush();
    let mut line = String::new();
    stdin().read_line(&mut line)?;
    let t = line.trim().to_lowercase();
    Ok(t.is_empty() || t == "y" || t == "yes" || t == "s" || t == "si")
}
