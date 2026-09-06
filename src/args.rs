use std::fmt::{Display, Formatter, Result};

pub static PACMAN_FLAGS: &[&str] = &[
    "disable-download-timeout",
    "d",
    "nodeps",
    "assume-installed",
    "dbonly",
    "absdir",
    "noprogressbar",
    "noscriptlet",
    "p",
    "print",
    "print-format",
    "asdeps",
    "asdep",
    "asexplicit",
    "asexp",
    "ignore",
    "ignoregroup",
    "needed",
    "overwrite",
    "f",
    "force",
    "c",
    "changelog",
    "deps",
    "e",
    "explicit",
    "g",
    "groups",
    "i",
    "info",
    "k",
    "check",
    "l",
    "list",
    "m",
    "foreign",
    "n",
    "native",
    "o",
    "owns",
    "file",
    "q",
    "quiet",
    "s",
    "search",
    "t",
    "unrequired",
    "u",
    "upgrades",
    "cascade",
    "nosave",
    "recursive",
    "unneeded",
    "clean",
    "optional",
    "sysupgrade",
    "w",
    "downloadonly",
    "y",
    "refresh",
    "x",
    "regex",
    "machinereadable",
];

pub static PACMAN_GLOBALS: &[&str] = &[
    "b",
    "dbpath",
    "r",
    "root",
    "v",
    "verbose",
    "ask",
    "arch",
    "cachedir",
    "color",
    "config",
    "debug",
    "gpgdir",
    "hookdir",
    "logfile",
    "disable-download-timeout",
    "sysroot",
    "noconfirm",
    "confirm",
    "h",
    "help",
];

#[derive(Default, Debug, Clone)]
pub struct Arg {
    pub key: String,
    pub value: Option<String>,
}

impl Display for Arg {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        if self.key.len() == 1 {
            f.write_str("-")?;
        } else {
            f.write_str("--")?;
        }

        f.write_str(&self.key)?;

        if let Some(ref value) = self.value {
            if self.key.len() != 1 {
                f.write_str("=")?;
            }
            f.write_str(value)?;
        }

        Ok(())
    }
}

#[derive(Default, Debug, Clone)]
pub struct Args {
    pub args: Vec<Arg>,
}

impl Args {
    pub fn has_arg(&self, s1: &str, s2: &str) -> bool {
        self.args.iter().any(|a| a.key == s1 || a.key == s2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arg_display_corto_largo_y_valor() {
        let corto = Arg {
            key: "S".to_string(),
            value: None,
        };
        assert_eq!(corto.to_string(), "-S");
        let largo = Arg {
            key: "noconfirm".to_string(),
            value: None,
        };
        assert_eq!(largo.to_string(), "--noconfirm");
        let con_valor = Arg {
            key: "branch".to_string(),
            value: Some("main".to_string()),
        };
        assert_eq!(con_valor.to_string(), "--branch=main");
    }

    #[test]
    fn has_arg_matchea_cualquiera_de_los_dos_alias() {
        let args = Args {
            args: vec![Arg {
                key: "s".to_string(),
                value: None,
            }],
        };
        assert!(args.has_arg("s", "search"));
        assert!(!args.has_arg("y", "refresh"));
    }
}
