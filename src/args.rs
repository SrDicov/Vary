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
