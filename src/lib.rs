mod args;
mod bootstrap;
mod cache;
mod command_line;
mod config;
mod db;
mod elevate;
mod help;
mod keys;
mod lock;
mod logging;
mod masterdir;
mod metadata;
mod remove;
mod repo;
mod reposconf;
mod resolver;
mod signal;
mod util;
mod vup_index;
mod vur_client;
mod xbps;

use crate::config::{Config, Op};

use std::env;
use std::path::PathBuf;

use ansiterm::Style;
use anyhow::{bail, Error, Result};

fn debug_enabled() -> bool {
    env::var("VARY_DEBUG").as_deref().unwrap_or("0") != "0"
}

fn print_error(color: Style, err: Error) {
    let backtrace_enabled = match env::var("RUST_LIB_BACKTRACE") {
        Ok(s) => s != "0",
        Err(_) => match env::var("RUST_BACKTRACE") {
            Ok(s) => s != "0",
            Err(_) => false,
        },
    };

    if backtrace_enabled {
        let backtrace = err.backtrace();
        eprint!("{backtrace}");
    }

    let mut iter = err.chain().peekable();

    eprint!("{} ", color.paint("error:"));
    while let Some(link) = iter.next() {
        eprint!("{link}");
        if iter.peek().is_some() {
            eprint!(": ");
        }
    }
    eprintln!();
}

pub fn run<S: AsRef<str>>(args: &[S]) -> i32 {
    // Inicializar logging temprano (cache_dir aún no se conoce con precisión,
    // usamos dirs::home_dir fallback; Config::new() lo reconfigura)
    let _guard = {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
        let cache = home.join(".cache").join("vary");
        logging::init(&cache, 0)
    };
    let _ = &_guard;

    if debug_enabled() {
        tracing::debug!("VARY_DEBUG activo");
    }

    let mut config = match Config::new() {
        Ok(config) => config,
        Err(err) => {
            print_error(Style::new(), err);
            return 1;
        }
    };

    // Observador SIGINT/SIGTERM (hilo + registro global; el handler solo
    // marca un flag). Idempotente.
    crate::signal::init();

    // Una sola instancia: protege /etc/xbps.d, la db heed y los mounts.
    // El guardián vive hasta el final de run() y libera el flock al salir.
    let _instance_lock = match crate::lock::acquire(&config.cache_dir) {
        Ok(lock) => lock,
        Err(err) => {
            print_error(Style::new(), err);
            return 1;
        }
    };

    // Barrido de temporales huérfanos de corridas interrumpidas (/tmp/vary-*,
    // solo uid propio). Tras el lock: imposible borrarle nada a otra instancia.
    let stale = crate::signal::sweep_stale_tmp_files();
    if stale > 0 {
        tracing::debug!("barridos {stale} temporales huérfanos de /tmp");
    }

    match run2(&mut config, args) {
        Err(err) => {
            print_error(Style::new(), err);
            1
        }
        Ok(ret) => ret,
    }
}

fn run2<S: AsRef<str>>(config: &mut Config, args: &[S]) -> Result<i32> {
    if args.is_empty() {
        let default: Vec<String> = vec!["-Syu".to_string()];
        config.parse_args(&default)?;
    } else {
        config.parse_args(args)?;
    }

    if config.help {
        help::help();
        return Ok(0);
    }
    if config.version {
        println!("vary {}", env!("CARGO_PKG_VERSION"));
        return Ok(0);
    }

    tracing::debug!("config: {config:?}");

    handle_cmd(config)
}

fn handle_cmd(config: &mut Config) -> Result<i32> {
    // Manejo de subcomando --repo
    if let Some(cmd) = command_line::take_repo_cmd() {
        return repo::handle_repo_cmd(config, cmd);
    }

    let ret = match config.op {
        Op::Sync => handle_sync(config)?,
        Op::Remove => remove::remove(config)?,
        Op::Default => handle_default(config)?,
    };

    Ok(ret)
}

fn handle_sync(config: &mut Config) -> Result<i32> {
    if config.args.has_arg("p", "print") {
        bail!("el flag --print / -p no está soportado (reservado para Roadmap P0-4). Para instalar use vary -S <pkg>");
    }

    let has_search = config.args.has_arg("s", "search");
    let has_info = config.args.has_arg("i", "info");
    let has_refresh = config.args.has_arg("y", "refresh");
    let has_upgrade = config.args.has_arg("u", "sysupgrade");
    let has_downloadonly = config.args.has_arg("w", "downloadonly");

    if has_search {
        return search::search(config);
    }
    if has_info {
        return info::info(config);
    }
    if has_downloadonly {
        return install::download_only(config);
    }
    if has_refresh && has_upgrade && config.targets.is_empty() {
        return upgrade::upgrade(config);
    }
    if has_refresh && config.targets.is_empty() && !has_upgrade {
        // solo -Sy : sincronizar repos VUR (git pull)
        return upgrade::refresh_repos(config);
    }

    // -S <pkg> o -Syu con targets, o -Su
    if has_upgrade && config.targets.is_empty() {
        return upgrade::upgrade(config);
    }

    if !config.targets.is_empty() {
        // instalar paquetes específicos
        return install::install(config);
    }

    bail!("no targets specified (use -h for help)");
}

fn handle_default(config: &mut Config) -> Result<i32> {
    if !config.targets.is_empty() {
        return handle_sync(config);
    }
    // Sin operación ni objetivos: comportamiento documentado de `vary` => -Syu
    // (igual que invocar vary sin argumentos, p. ej. `vary --noconfirm`).
    upgrade::upgrade(config)
}

mod info;
mod init;
mod install;
mod review;
mod search;
mod upgrade;
