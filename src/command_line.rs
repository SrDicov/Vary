use crate::args::{PACMAN_FLAGS, PACMAN_GLOBALS};
use crate::config::{Config, Op};

use std::cell::RefCell;
use std::fmt;

use anyhow::{bail, Result};

/// Arquitecturas soportadas por Void Linux. `--arch` solo acepta estos
/// valores para evitar pasar silenciosamente un arch inválido a xbps.
const KNOWN_ARCHS: &[&str] = &[
    "x86_64",
    "x86_64-musl",
    "i686",
    "i686-musl",
    "aarch64",
    "aarch64-musl",
    "armv7l",
    "armv7l-musl",
    "armv6l",
    "armv6l-musl",
    "ppc64le",
    "ppc64le-musl",
    "ppc64",
    "ppc64-musl",
    "ppc",
    "ppc-musl",
    "mips",
    "mips-musl",
    "mipsel",
    "mipsel-musl",
    "riscv64",
    "riscv64-musl",
];

fn is_valid_arch(arch: &str) -> bool {
    KNOWN_ARCHS.contains(&arch)
}

thread_local! {
    static REPO_CMD: RefCell<Option<RepoCmd>> = const { RefCell::new(None) };
}

#[derive(Debug, Clone)]
pub enum RepoCmd {
    Add {
        url: String,
        name: Option<String>,
        branch: Option<String>,
        index_url: Option<String>,
    },
    List,
    Remove {
        name: String,
        purge: bool,
    },
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
            Arg::Short(c) => write!(f, "-{c}"),
            Arg::Long(l) => write!(f, "--{l}"),
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
                if url.starts_with('-') {
                    bail!("--repo add requires <url>");
                }
                let mut name: Option<String> = None;
                let mut branch: Option<String> = None;
                let mut index_url: Option<String> = None;
                let mut rest = raw[3..].iter();
                while let Some(tok) = rest.next() {
                    let t = tok.as_str();
                    if let Some(v) = t.strip_prefix("--branch=") {
                        if v.is_empty() {
                            bail!("--branch requires a value");
                        }
                        branch = Some(v.to_string());
                    } else if t == "--branch" {
                        match rest.next() {
                            Some(v) if !v.starts_with('-') => branch = Some(v.to_string()),
                            _ => bail!("--branch requires a value"),
                        }
                    } else if let Some(v) = t.strip_prefix("--index-url=") {
                        if v.is_empty() {
                            bail!("--index-url requires a value");
                        }
                        index_url = Some(v.to_string());
                    } else if t == "--index-url" {
                        match rest.next() {
                            Some(v) if !v.starts_with('-') => index_url = Some(v.to_string()),
                            _ => bail!("--index-url requires a value"),
                        }
                    } else if t.starts_with('-') {
                        bail!(format!("unknown option {t} for --repo add"));
                    } else if name.is_none() {
                        name = Some(t.to_string());
                    } else {
                        bail!("too many arguments for --repo add (expected <url> [name])");
                    }
                }
                set_repo_cmd(RepoCmd::Add {
                    url,
                    name,
                    branch,
                    index_url,
                });
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
                                bail!(format!("unknown option {t} for --repo remove"));
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
            other => bail!(format!("unknown --repo subcommand: {other}")),
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
            // splitn siempre rinde ≥1 elemento; el let-else es defensa sin panic.
            let Some(arg_str) = split.next() else {
                bail!("opción larga vacía");
            };
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
            // `arg` no está vacío (empieza con '-'); defensa sin panic.
            if chars.next().is_none() {
                bail!("opción corta vacía");
            }
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
            TakesValue::Required if value.is_none() => {
                bail!(format!("option {arg} expects a value"))
            }
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
            Arg::Long("color") => {
                let v = value.unwrap_or("auto");
                if v != "always" && v != "never" && v != "auto" {
                    bail!("invalid --color value '{v}' (expected always|never|auto)");
                }
                self.color = crate::config::Colors::from(v);
            }
            Arg::Long("verbose") | Arg::Short('v') => self.verbose = self.verbose.saturating_add(1),
            Arg::Long("quiet") | Arg::Short('q') => self.quiet = true,
            Arg::Long("arch") => {
                let Some(v) = value else {
                    bail!("la opción --arch requiere un valor");
                };
                if !is_valid_arch(v) {
                    bail!(
                        "invalid --arch value '{}' (supported: {})",
                        v,
                        KNOWN_ARCHS.join(", ")
                    );
                }
                self.arch_override = Some(v.to_string());
            }
            Arg::Long("sudo") => {
                let Some(v) = value else {
                    bail!("la opción --sudo requiere un valor");
                };
                self.sudo_bin = v.to_string();
            }
            Arg::Long("sudoflags") => {
                let Some(v) = value else {
                    bail!("la opción --sudoflags requiere un valor");
                };
                self.sudo_flags
                    .extend(v.split_whitespace().map(|s| s.to_string()));
            }
            Arg::Long("git") => {
                let Some(v) = value else {
                    bail!("la opción --git requiere un valor");
                };
                self.git_bin = v.to_string();
            }
            Arg::Long("curl") => {
                let Some(v) = value else {
                    bail!("la opción --curl requiere un valor");
                };
                self.curl_bin = v.to_string();
            }
            Arg::Long("force-build") => self.force_build = true,
            Arg::Long("prefer-binary") => self.prefer_binary = true,
            Arg::Long("no-prefer-binary") => self.prefer_binary = false,
            Arg::Long("interactive") => self.interactive = true,
            Arg::Long("print") | Arg::Short('p') | Arg::Long("print-format") => {
                bail!("el flag --print / -p no está soportado (reservado para Roadmap P0-4). Para instalar use vary -S <pkg>");
            }
            // Generic pacman-style flags that we just record in args
            Arg::Long("search") | Arg::Short('s') => {
                self.args.args.push(crate::args::Arg {
                    key: "s".to_string(),
                    value: None,
                });
                self.args.args.push(crate::args::Arg {
                    key: "search".to_string(),
                    value: None,
                });
            }
            Arg::Long("info") | Arg::Short('i') => {
                self.args.args.push(crate::args::Arg {
                    key: "i".to_string(),
                    value: None,
                });
                self.args.args.push(crate::args::Arg {
                    key: "info".to_string(),
                    value: None,
                });
            }
            Arg::Long("refresh") | Arg::Short('y') => {
                self.args.args.push(crate::args::Arg {
                    key: "y".to_string(),
                    value: None,
                });
                self.args.args.push(crate::args::Arg {
                    key: "refresh".to_string(),
                    value: None,
                });
            }
            Arg::Long("sysupgrade") | Arg::Short('u') => {
                self.args.args.push(crate::args::Arg {
                    key: "u".to_string(),
                    value: None,
                });
                self.args.args.push(crate::args::Arg {
                    key: "sysupgrade".to_string(),
                    value: None,
                });
            }
            Arg::Long("downloadonly") | Arg::Short('w') => {
                self.args.args.push(crate::args::Arg {
                    key: "w".to_string(),
                    value: None,
                });
                self.args.args.push(crate::args::Arg {
                    key: "downloadonly".to_string(),
                    value: None,
                });
            }
            Arg::Long("asdeps") => {
                self.args.args.push(crate::args::Arg {
                    key: "asdeps".to_string(),
                    value: None,
                });
            }
            Arg::Long("asexplicit") => {
                self.args.args.push(crate::args::Arg {
                    key: "asexplicit".to_string(),
                    value: None,
                });
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
                    "force-build" | "prefer-binary" | "no-prefer-binary" | "interactive"
                    | "sudo" | "sudoflags" | "git" | "curl" | "arch" | "help" | "version"
                    | "noconfirm" | "confirm" | "color" | "verbose" | "quiet" => {}
                    _ => bail!(format!("unknown option --{a}")),
                }
            }
            Arg::Short(a) if !arg.is_pacman_arg() && !arg.is_pacman_global() => match a {
                'h' | 'V' | 'v' | 'q' | 'S' | 'R' | 's' | 'i' | 'y' | 'u' | 'w' => {}
                _ => bail!(format!("unknown option -{a}")),
            },
            _ => {}
        }

        match takes_value(arg) {
            TakesValue::No if forced => bail!(format!("option {arg} does not allow a value")),
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
        Arg::Long("curl") => TakesValue::Required,
        Arg::Long("arch") => TakesValue::Required,
        Arg::Long("color") => TakesValue::Required,
        _ => TakesValue::No,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn take_add(args: &[&str]) -> RepoCmd {
        let mut config = Config::default();
        parse_args(&mut config, args).expect("parse_args debe aceptar el comando");
        take_repo_cmd().expect("se esperaba un RepoCmd")
    }

    #[test]
    fn repo_add_parses_name_branch_index_url() {
        match take_add(&[
            "--repo",
            "add",
            "https://example.com/vur.git",
            "mi-vur",
            "--branch",
            "master",
            "--index-url",
            "https://example.com/index.json",
        ]) {
            RepoCmd::Add {
                url,
                name,
                branch,
                index_url,
            } => {
                assert_eq!(url, "https://example.com/vur.git");
                assert_eq!(name.as_deref(), Some("mi-vur"));
                assert_eq!(branch.as_deref(), Some("master"));
                assert_eq!(index_url.as_deref(), Some("https://example.com/index.json"));
            }
            other => panic!("inesperado: {other:?}"),
        }
    }

    #[test]
    fn repo_add_supports_equals_form() {
        match take_add(&[
            "--repo",
            "add",
            "https://example.com/vur.git",
            "--branch=master",
        ]) {
            RepoCmd::Add {
                url,
                name,
                branch,
                index_url,
            } => {
                assert_eq!(url, "https://example.com/vur.git");
                assert!(name.is_none());
                assert_eq!(branch.as_deref(), Some("master"));
                assert!(index_url.is_none());
            }
            other => panic!("inesperado: {other:?}"),
        }
    }

    #[test]
    fn repo_add_defaults_are_none() {
        match take_add(&["--repo", "add", "https://example.com/vur.git"]) {
            RepoCmd::Add {
                url,
                name,
                branch,
                index_url,
            } => {
                assert_eq!(url, "https://example.com/vur.git");
                assert!(name.is_none() && branch.is_none() && index_url.is_none());
            }
            other => panic!("inesperado: {other:?}"),
        }
    }

    #[test]
    fn repo_add_rejects_bad_options() {
        let mut config = Config::default();
        // --branch sin valor
        assert!(parse_args(
            &mut config,
            &["--repo", "add", "https://example.com/v.git", "--branch"]
        )
        .is_err());
        // opción desconocida
        assert!(parse_args(
            &mut config,
            &["--repo", "add", "https://example.com/v.git", "--nope"]
        )
        .is_err());
        // dos posicionales extra
        assert!(parse_args(
            &mut config,
            &["--repo", "add", "https://example.com/v.git", "a", "b"]
        )
        .is_err());
        // sin URL
        assert!(parse_args(&mut config, &["--repo", "add"]).is_err());
    }

    #[test]
    fn print_flag_is_disabled_with_informative_error() {
        let mut config = Config::default();
        let err1 = parse_args(&mut config, &["-Sp", "foo"]).unwrap_err();
        assert!(format!("{err1:#}").contains("Roadmap P0-4"));

        let err2 = parse_args(&mut config, &["-S", "--print", "foo"]).unwrap_err();
        assert!(format!("{err2:#}").contains("Roadmap P0-4"));

        let err3 = parse_args(&mut config, &["--print-format", "%n"]).unwrap_err();
        assert!(format!("{err3:#}").contains("Roadmap P0-4"));
    }

    #[test]
    fn opciones_con_valor_sin_valor_devuelven_error_en_vez_de_panic() {
        // H-023: --arch/--sudo/--sudoflags/--git/--curl sin valor deben fallar
        // con error accionable, nunca con panic por unwrap. (El guard de
        // TakesValue::Required o el let-else del brazo: ambos sin panic.)
        for name in ["arch", "sudo", "sudoflags", "git", "curl"] {
            let mut config = Config::default();
            let mut n = 0u8;
            let err = config
                .handle_arg(Arg::Long(name), None, &mut n, false)
                .unwrap_err();
            let msg = format!("{err:#}");
            assert!(
                msg.contains(name),
                "--{name} sin valor debe mencionar el flag: {msg}"
            );
        }
    }
}
