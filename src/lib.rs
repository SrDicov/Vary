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
mod preflight;
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
    {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
        let cache = home.join(".cache").join("vary");
        logging::init(&cache, 0);
    }

    if debug_enabled() {
        tracing::debug!("VARY_DEBUG activo");
    }

    // Chequeos previos duros del entorno (P0-1): root directo y uchroot
    // sin 4750 abortan; chroot degradado / OCI avisan y siguen.
    {
        let report = crate::preflight::check_real();
        for w in &report.warnings {
            tracing::warn!("preflight: {w}");
        }
        if let Some(msg) = report.abort {
            print_error(Style::new(), anyhow::anyhow!(msg));
            return 1;
        }
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

    // Parsear args ANTES del lock (H-027): ayuda/versión y comandos de solo
    // lectura (-Ss/-Si/--repo list) no deben bloquearse tras otra instancia.
    let args_owned: Vec<String> = if args.is_empty() {
        vec!["-Syu".to_string()]
    } else {
        args.iter().map(|s| s.as_ref().to_string()).collect()
    };
    if let Err(err) = config.parse_args(&args_owned) {
        print_error(Style::new(), err);
        return 2; // mal uso de CLI (H-035): distinto de fallo en ejecución (1)
    }

    if config.help {
        help::help();
        return 0;
    }
    if config.version {
        println!("vary {}", env!("CARGO_PKG_VERSION"));
        return 0;
    }

    tracing::debug!("config: {config:?}");

    // Niveles finales tras CLI/TOML (H-028): RUST_LOG > -v > log_level.
    logging::apply_runtime_config(config.verbose, &config.log_level);

    // Una sola instancia para lo que muta estado compartido (/etc/xbps.d,
    // masterdir, clones VUR). El guardián vive hasta el final de run().
    // Solo-lectura corre sin lock (H-027).
    let _instance_lock = if needs_lock(&config) {
        match crate::lock::acquire(&config.cache_dir) {
            Ok(lock) => Some(lock),
            Err(err) => {
                print_error(Style::new(), err);
                return 1;
            }
        }
    } else {
        None
    };

    // Barrido de temporales huérfanos de corridas interrumpidas (/tmp/vary-*,
    // solo uid propio). Solo con lock: imposible borrarle nada a otra instancia.
    if _instance_lock.is_some() {
        let stale = crate::signal::sweep_stale_tmp_files();
        if stale > 0 {
            tracing::debug!("barridos {stale} temporales huérfanos de /tmp");
        }
    }

    match handle_cmd(&mut config) {
        Err(err) => {
            // H-035: clasificar ANTES de mover a print_error.
            let not_found = crate::xbps::is_not_found_error(&err);
            print_error(Style::new(), err);
            // 127 si falta el binario (git/xbps/…), 1 para el resto.
            if not_found {
                127
            } else {
                1
            }
        }
        Ok(ret) => ret,
    }
}

/// Suelta el worker de logs (flush) antes de salidas que se saltan `Drop`.
/// La usa el observador de señales y el hook de pipe roto en `main`.
pub fn shutdown_logging() {
    logging::shutdown();
}

/// Restaura cursor y atributos de terminal. La usa el hook de panic en `main`.
pub fn restore_terminal() {
    signal::restore_terminal();
}

/// H-027: el lock global solo protege operaciones que mutan estado compartido.
/// Búsqueda, info, listado de repos, ayuda y versión corren sin lock.
fn needs_lock(config: &Config) -> bool {
    if let Some(cmd) = command_line::peek_repo_cmd() {
        return !matches!(cmd, command_line::RepoCmd::List);
    }
    match config.op {
        Op::Remove => true,
        Op::Default => true, // `vary` pelado o solo flags => -Syu
        Op::Sync => {
            // -p no muta: sin lock (H-027); -w descarga (muta caché/masterdir).
            !(config.args.has_arg("s", "search")
                || config.args.has_arg("i", "info")
                || config.args.has_arg("p", "print"))
        }
    }
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
    // P0-4: -p imprime el plan congelado sin mutar (ver install::print_plan,
    // que valida su propia forma). Domina sobre el resto de consultas sync.
    if config.args.has_arg("p", "print") {
        return install::print_plan(config);
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
        // `vary -p foo` cae aquí (sin -S explícito): handle_sync lo imprime.
        return handle_sync(config);
    }
    // P0-4: -p pelado no tiene plan que imprimir (no caer al -Syu).
    if config.args.has_arg("p", "print") {
        bail!("--print requiere -S con targets: usa `vary -Sp <pkg>`");
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solo_lectura_no_requiere_lock_mutacion_si() {
        // H-027: -Ss/-Si corren sin lock; -S/-Syu/-R y default sí.
        for (argv, locked) in [
            (vec!["-Ss", "foo"], false),
            (vec!["-Si", "foo"], false),
            (vec!["-Sp", "foo"], false),
            (vec!["-S", "--print", "foo"], false),
            (vec!["-S", "foo"], true),
            (vec!["-Syu"], true),
            (vec!["-R", "foo"], true),
            (vec![], true),
        ] {
            let mut config = Config::new().expect("config de test");
            let owned: Vec<String> = if argv.is_empty() {
                vec!["-Syu".to_string()]
            } else {
                argv.iter().map(|s| s.to_string()).collect()
            };
            config.parse_args(&owned).expect("parse");
            assert_eq!(
                needs_lock(&config),
                locked,
                "clasificación de lock para {argv:?}"
            );
        }
    }
    #[test]
    fn repo_list_no_requiere_lock_add_si() {
        let mut config = Config::new().expect("config de test");
        config
            .parse_args(&["--repo".to_string(), "list".to_string()])
            .expect("parse");
        assert!(!needs_lock(&config));
        crate::command_line::take_repo_cmd();

        let mut config = Config::new().expect("config de test");
        config
            .parse_args(&[
                "--repo".to_string(),
                "add".to_string(),
                "https://example.com/v.git".to_string(),
            ])
            .expect("parse");
        assert!(needs_lock(&config));
        crate::command_line::take_repo_cmd();
    }

    #[test]
    fn cli_mal_usado_devuelve_2() {
        // H-035: error de parseo (opción desconocida) => 2, no 1.
        // run() parsea antes del lock: no toca disco compartido.
        assert_eq!(run(&["--opcion-inexistente-xyz"]), 2);
    }
}
