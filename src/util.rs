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

/// Hint visible cuando la confirmación se deniega por EOF (H-019): sin esto
/// las corridas en CI/pipes abortaban con código 1 sin explicar por qué.
pub(crate) const EOF_DENIAL_HINT: &str = "error: stdin llegó a EOF sin confirmación. En entornos no interactivos (CI/pipes), usa --noconfirm.";

pub fn confirm_from_reader<R: BufRead>(reader: &mut R) -> Result<Option<bool>> {
    let mut line = String::new();
    let n = match reader.read_line(&mut line) {
        Ok(n) => n,
        Err(_) => return Ok(None),
    };
    if n == 0 {
        // EOF en stdin (pipe cerrado, /dev/null, no-TTY): denegación estricta
        return Ok(None);
    }
    let t = line.trim().to_lowercase();
    Ok(Some(
        t.is_empty() || t == "y" || t == "yes" || t == "s" || t == "si",
    ))
}

/// Confirmación y/n compartida por install/keys. `no_confirm` => true.
pub fn confirm(prompt: &str, no_confirm: bool) -> Result<bool> {
    if no_confirm {
        return Ok(true);
    }
    print!("{prompt} [Y/n] ");
    let _ = stdout().lock().flush();
    match confirm_from_reader(&mut stdin().lock())? {
        Some(answer) => Ok(answer),
        // EOF: denegar (H-001) PERO explicando cómo proceder (H-019).
        None => {
            eprintln!("{EOF_DENIAL_HINT}");
            Ok(false)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn confirm_eof_results_in_denial() {
        let mut eof = Cursor::new(b"");
        assert_eq!(confirm_from_reader(&mut eof).unwrap(), None);
    }

    #[test]
    fn confirm_newline_accepts_default() {
        let mut newline = Cursor::new(b"\n");
        assert_eq!(confirm_from_reader(&mut newline).unwrap(), Some(true));
    }

    #[test]
    fn confirm_explicit_denial() {
        let mut no = Cursor::new(b"n\n");
        assert_eq!(confirm_from_reader(&mut no).unwrap(), Some(false));
    }

    #[test]
    fn confirm_explicit_approval() {
        let mut yes = Cursor::new(b"yes\n");
        assert_eq!(confirm_from_reader(&mut yes).unwrap(), Some(true));
        let mut si = Cursor::new(b"si\n");
        assert_eq!(confirm_from_reader(&mut si).unwrap(), Some(true));
    }

    #[test]
    fn eof_hint_points_to_noconfirm() {
        assert!(
            EOF_DENIAL_HINT.contains("--noconfirm"),
            "el hint debe decir cómo proceder en CI/pipes"
        );
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
