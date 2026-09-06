use crate::config::Config;

use std::io::{stdin, stdout, BufRead, Write};

use anyhow::Result;

pub fn ask_from_reader<R: BufRead>(reader: &mut R, default: bool) -> bool {
    let mut input = String::new();
    let n = match reader.read_line(&mut input) {
        Ok(n) => n,
        Err(_) => return false,
    };
    if n == 0 {
        // EOF en stdin (pipe cerrado, /dev/null): denegación estricta
        return false;
    }
    let input = input.to_lowercase();
    let input = input.trim();
    if input == "y" || input == "yes" || input == "s" || input == "si" {
        true
    } else if input.is_empty() {
        default
    } else {
        false
    }
}

pub fn ask(config: &Config, question: &str, default: bool) -> bool {
    let action = config.color.action;
    let bold = config.color.bold;
    let yn = if default { "[Y/n]:" } else { "[y/N]:" };
    print!(
        "{} {} {} ",
        action.paint("::"),
        bold.paint(question),
        bold.paint(yn)
    );
    let _ = stdout().lock().flush();
    if config.no_confirm {
        println!();
        return default;
    }
    ask_from_reader(&mut stdin().lock(), default)
}

pub fn confirm_from_reader<R: BufRead>(reader: &mut R) -> Result<bool> {
    let mut line = String::new();
    let n = match reader.read_line(&mut line) {
        Ok(n) => n,
        Err(_) => return Ok(false),
    };
    if n == 0 {
        // EOF en stdin (pipe cerrado, /dev/null, no-TTY): denegación estricta
        return Ok(false);
    }
    let t = line.trim().to_lowercase();
    Ok(t.is_empty() || t == "y" || t == "yes" || t == "s" || t == "si")
}

/// Confirmación y/n compartida por install/keys. `no_confirm` => true.
pub fn confirm(prompt: &str, no_confirm: bool) -> Result<bool> {
    if no_confirm {
        return Ok(true);
    }
    print!("{} [Y/n] ", prompt);
    let _ = stdout().lock().flush();
    confirm_from_reader(&mut stdin().lock())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn confirm_eof_results_in_denial() {
        let mut eof = Cursor::new(b"");
        assert!(!confirm_from_reader(&mut eof).unwrap());
    }

    #[test]
    fn confirm_newline_accepts_default() {
        let mut newline = Cursor::new(b"\n");
        assert!(confirm_from_reader(&mut newline).unwrap());
    }

    #[test]
    fn confirm_explicit_denial() {
        let mut no = Cursor::new(b"n\n");
        assert!(!confirm_from_reader(&mut no).unwrap());
    }

    #[test]
    fn confirm_explicit_approval() {
        let mut yes = Cursor::new(b"yes\n");
        assert!(confirm_from_reader(&mut yes).unwrap());
        let mut si = Cursor::new(b"si\n");
        assert!(confirm_from_reader(&mut si).unwrap());
    }

    #[test]
    fn ask_eof_results_in_denial_even_if_default_is_true() {
        let mut eof = Cursor::new(b"");
        assert!(!ask_from_reader(&mut eof, true));
    }

    #[test]
    fn ask_newline_uses_default() {
        let mut newline = Cursor::new(b"\n");
        assert!(ask_from_reader(&mut newline, true));
        let mut newline2 = Cursor::new(b"\n");
        assert!(!ask_from_reader(&mut newline2, false));
    }
}
