use crate::args::{PACMAN_FLAGS, PACMAN_GLOBALS};
use crate::config::{Config, Op};

use std::cell::RefCell;
use std::fmt;

use anyhow::{bail, Result};

thread_local! {
    static REPO_CMD: RefCell<Option<RepoCmd>> = const { RefCell::new(None) };
}

#[derive(Debug, Clone)]
pub enum RepoCmd {
    Add { url: String, name: Option<String> },
    List,
    Remove { name: String, purge: bool },
    Rekey(String),
}

pub fn take_repo_cmd() -> Option<RepoCmd> {
    REPO_CMD.with(|c| c.borrow_mut().take())
}

fn set_repo_cmd(cmd: RepoCmd) {
    REPO_CMD.with(|c| *c.borrow_mut() = Some(cmd));
}

#[derive(Debug, Copy, Clone)]
enum Arg<'a> {
    Short(char),
    Long(&'a str),
}

impl<'a> fmt::Display for Arg<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Arg::Short(c) => write!(f, "-{}", c),
            Arg::Long(l) => write!(f, "--{}", l),
        }
    }
}

impl<'a> Arg<'a> {
    fn arg(self) -> String {
        match self {
            Arg::Long(arg) => arg.to_string(),
            Arg::Short(arg) => arg.to_string(),
        }
    }

    fn is_pacman_arg(self) -> bool {
        match self {
            Arg::Long(arg) => PACMAN_FLAGS.contains(&arg),
            Arg::Short(arg) => {
                let mut buff = [0, 0, 0, 0];
                let arg = arg.encode_utf8(&mut buff);
                PACMAN_FLAGS.contains(&&*arg)
            }
        }
    }

    fn is_pacman_global(self) -> bool {
        match self {
            Arg::Long(arg) => PACMAN_GLOBALS.contains(&arg),
            Arg::Short(arg) => {
                let mut buff = [0, 0, 0, 0];
                let arg = arg.encode_utf8(&mut buff);
                PACMAN_GLOBALS.contains(&&*arg)
            }
        }
    }
}

#[derive(PartialEq)]
enum TakesValue {
    Required,
    No,
}

pub fn parse_args<S: AsRef<str>>(config: &mut Config, args: &[S]) -> Result<()> {
    // Detect --repo subcommand before normal parsing
    let raw: Vec<String> = args.iter().map(|s| s.as_ref().to_string()).collect();
    if !raw.is_empty() && raw[0] == "--repo" {
        if raw.len() < 2 {
            bail!("--repo requires a subcommand: add|list|remove|rekey");
        }
        match raw[1].as_str() {
            "add" => {
                if raw.len() < 3 {
                    bail!("--repo add requires <url>");
                }
                let url = raw[2].clone();
                let name = raw.get(3).cloned();
                set_repo_cmd(RepoCmd::Add { url, name });
                return Ok(());
            }
            "list" => {
                set_repo_cmd(RepoCmd::List);
                return Ok(());
            }
            "remove" => {
                let mut name: Option<String> = None;
                let mut purge = false;
                for tok in &raw[2..] {
                    match tok.as_str() {
                        "-p" | "--purge" => purge = true,
                        t => {
                            if name.is_none() && !t.starts_with('-') {
                                name = Some(t.to_string());
                            } else if t.starts_with('-') && t != "-" {
                                bail!(format!("unknown option {} for --repo remove", t));
                            }
                        }
                    }
                }
                let Some(name) = name else {
                    bail!("--repo remove requires <name>");
                };
                set_repo_cmd(RepoCmd::Remove { name, purge });
                return Ok(());
            }
            "rekey" => {
                if raw.len() < 3 {
                    bail!("--repo rekey requires <name>");
                }
                set_repo_cmd(RepoCmd::Rekey(raw[2].clone()));
                return Ok(());
            }
            other => bail!(format!("unknown --repo subcommand: {}", other)),
        }
    }

    let mut op_count: u8 = 0;
    let mut end_of_ops = false;
    let mut idx = 0;
    while idx < raw.len() {
        let arg = &raw[idx];
        let next = raw.get(idx + 1).map(|s| s.as_str());
        let consumed_next = config.parse_arg(arg, next, &mut op_count, &mut end_of_ops)?;
        idx += 1;
        if consumed_next {
            idx += 1;
        }
    }
    Ok(())
}

impl Config {
    pub fn parse_arg(
        &mut self,
        arg: &str,
        value: Option<&str>,
        op_count: &mut u8,
        end_of_ops: &mut bool,
    ) -> Result<bool> {
        let mut forced = false;

        if arg == "-" || *end_of_ops {
            self.targets.push(arg.to_string());
            return Ok(false);
        }
        if arg == "--" {
            *end_of_ops = true;
            return Ok(false);
        }

        if arg.starts_with("--") {
            let mut value = value;
            let mut split = arg.splitn(2, '=');
            let arg_str = split.next().unwrap();
            let arg = Arg::Long(arg_str.trim_start_matches("--"));
            let mut used_next = takes_value(arg) == TakesValue::Required;
            if let Some(val) = split.next() {
                value = Some(val);
                used_next = false;
                forced = true;
            }
            self.handle_arg(arg, value, op_count, forced)?;
            Ok(used_next)
        } else if arg.starts_with('-') {
            let mut chars = arg.chars();
            chars.next().unwrap();
            while let Some(c) = chars.next() {
                let arg = Arg::Short(c);
                if takes_value(arg) == TakesValue::Required {
                    if chars.as_str().is_empty() {
                        self.handle_arg(arg, value, op_count, false)?;
                        return Ok(true);
                    } else {
                        self.handle_arg(arg, Some(chars.as_str()), op_count, false)?;
                        return Ok(false);
                    }
                }
                self.handle_arg(arg, None, op_count, false)?;
            }
            Ok(false)
        } else {
            self.targets.push(arg.to_string());
            Ok(false)
        }
    }

    fn handle_arg(
        &mut self,
        arg: Arg,
        mut value: Option<&str>,
        op_count: &mut u8,
        forced: bool,
    ) -> Result<()> {
        match takes_value(arg) {
            TakesValue::Required if value.is_none() => bail!(format!("option {} expects a value", arg)),
            _ => (),
        }
        if takes_value(arg) != TakesValue::Required && !forced {
            value = None;
        }

        if arg.is_pacman_global() {
            self.args.args.push(crate::args::Arg {
                key: arg.arg(),
                value: value.map(|s| s.to_string()),
            });
        }
        if arg.is_pacman_arg() {
            self.args.args.push(crate::args::Arg {
                key: arg.arg(),
                value: value.map(|s| s.to_string()),
            });
        }

        // Validate op count
        if *op_count > 0 {
            match arg {
                Arg::Long("sync") | Arg::Short('S') | Arg::Long("remove") | Arg::Short('R') => {
                    bail!("only one operation may be used at a time");
                }
                _ => {}
            }
        }

        match arg {
            Arg::Long("help") | Arg::Short('h') => self.help = true,
            Arg::Long("version") | Arg::Short('V') => self.version = true,
            Arg::Long("noconfirm") => self.no_confirm = true,
            Arg::Long("confirm") => self.no_confirm = false,
            Arg::Long("color") => self.color = crate::config::Colors::from(value.unwrap_or("auto")),
            Arg::Long("verbose") | Arg::Short('v') => self.verbose = self.verbose.saturating_add(1),
            Arg::Long("quiet") | Arg::Short('q') => self.quiet = true,
            Arg::Long("arch") => self.arch_override = Some(value.unwrap().to_string()),
            Arg::Long("sudo") => self.sudo_bin = value.unwrap().to_string(),
            Arg::Long("sudoflags") => self.sudo_flags.extend(value.unwrap().split_whitespace().map(|s| s.to_string())),
            Arg::Long("git") => self.git_bin = value.unwrap().to_string(),
            Arg::Long("force-build") => self.force_build = true,
            Arg::Long("prefer-binary") => self.prefer_binary = true,
            Arg::Long("no-prefer-binary") => self.prefer_binary = false,
            Arg::Long("interactive") => self.interactive = true,
            // Generic pacman-style flags that we just record in args
            Arg::Long("search") | Arg::Short('s') => {
                self.args.args.push(crate::args::Arg { key: "s".to_string(), value: None });
                self.args.args.push(crate::args::Arg { key: "search".to_string(), value: None });
            }
            Arg::Long("info") | Arg::Short('i') => {
                self.args.args.push(crate::args::Arg { key: "i".to_string(), value: None });
                self.args.args.push(crate::args::Arg { key: "info".to_string(), value: None });
            }
            Arg::Long("refresh") | Arg::Short('y') => {
                self.args.args.push(crate::args::Arg { key: "y".to_string(), value: None });
                self.args.args.push(crate::args::Arg { key: "refresh".to_string(), value: None });
            }
            Arg::Long("sysupgrade") | Arg::Short('u') => {
                self.args.args.push(crate::args::Arg { key: "u".to_string(), value: None });
                self.args.args.push(crate::args::Arg { key: "sysupgrade".to_string(), value: None });
            }
            Arg::Long("downloadonly") | Arg::Short('w') => {
                self.args.args.push(crate::args::Arg { key: "w".to_string(), value: None });
                self.args.args.push(crate::args::Arg { key: "downloadonly".to_string(), value: None });
            }
            Arg::Long("asdeps") => {
                self.args.args.push(crate::args::Arg { key: "asdeps".to_string(), value: None });
            }
            Arg::Long("asexplicit") => {
                self.args.args.push(crate::args::Arg { key: "asexplicit".to_string(), value: None });
            }
            // ops
            Arg::Long("sync") | Arg::Short('S') => {
                self.op = Op::Sync;
                *op_count += 1;
            }
            Arg::Long("remove") | Arg::Short('R') => {
                self.op = Op::Remove;
                *op_count += 1;
            }
            Arg::Long(a) if !arg.is_pacman_arg() && !arg.is_pacman_global() => {
                // Allow vary-specific long opts already handled above
                match a {
                    "force-build" | "prefer-binary" | "no-prefer-binary" | "interactive" | "sudo" | "sudoflags" | "git" | "arch" | "help" | "version" | "noconfirm" | "confirm" | "color" | "verbose" | "quiet" => {},
                    _ => bail!(format!("unknown option --{}", a)),
                }
            }
            Arg::Short(a) if !arg.is_pacman_arg() && !arg.is_pacman_global() => {
                match a {
                    'h' | 'V' | 'v' | 'q' | 'S' | 'R' | 's' | 'i' | 'y' | 'u' | 'w' => {},
                    _ => bail!(format!("unknown option -{}", a)),
                }
            }
            _ => {}
        }

        match takes_value(arg) {
            TakesValue::No if forced => bail!(format!("option {} does not allow a value", arg)),
            _ => (),
        }

        Ok(())
    }
}

fn takes_value(arg: Arg) -> TakesValue {
    match arg {
        Arg::Long("sudo") => TakesValue::Required,
        Arg::Long("sudoflags") => TakesValue::Required,
        Arg::Long("git") => TakesValue::Required,
        Arg::Long("arch") => TakesValue::Required,
        Arg::Long("color") => TakesValue::Required,
        _ => TakesValue::No,
    }
}
