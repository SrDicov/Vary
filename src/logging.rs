//! Inicialización de logging para `vary`.
//!
//! Dos capas sobre [`tracing_subscriber::Registry`]:
//!
//! * **stdout** (`tracing_subscriber::fmt`, formato compacto, sin timestamp,
//!   con target): nivel por defecto `INFO`; con `verbose >= 1` => `DEBUG`;
//!   con `verbose >= 2` => `TRACE`.
//! * **archivo**: `tracing_appender::rolling::daily(<cache_dir>/vary.log)`
//!   envuelto en `tracing_appender::non_blocking`, con nivel `DEBUG` o
//!   superior SIEMPRE, independiente del nivel de stdout.
//!
//! # Precedencia de niveles
//!
//! `RUST_LOG` > CLI (`-v`) > TOML (`[general] log_level`) > default (`info`).
//! Si `RUST_LOG` está presente, gana sobre todo y las reconfiguraciones
//! posteriores la re-aplican idempotente (mismo valor, sin cambio visible).
//!
//! # Recarga (H-028)
//!
//! Solo la capa de consola es recargable (`reload::Handle`): es la única cuyo
//! nivel cambia tras el parse (el archivo siempre va en DEBUG salvo RUST_LOG,
//! así que su filtro es estático). [`init`] instala una vez y
//! [`apply_runtime_config`] ajusta la consola tras parsear CLI/TOML. El worker
//! de archivo vive en un guardián global; [`shutdown`] lo suelta (flush)
//! antes de salidas que se saltan `Drop` (`process::exit` en señales/pipe
//! roto).
//!
//! (Dos capas recargables no compilan: el `S` de `reload::Layer` debe ser el
//! subscriber final, innombrable para la segunda capa. Una sola basta.)

use std::path::Path;
use std::sync::{Mutex, OnceLock};

use tracing_subscriber::{fmt, layer::SubscriberExt, reload, EnvFilter, Layer, Registry};

static CONSOLE_HANDLE: OnceLock<reload::Handle<EnvFilter, Registry>> = OnceLock::new();
static GUARD: Mutex<Option<LoggingGuard>> = Mutex::new(None);

/// Retiene vivo el [`tracing_appender::non_blocking::WorkerGuard`] del logger.
///
/// Su `Drop` apaga el hilo escritor y hace flush del archivo de log; vive en
/// el global `GUARD` hasta [`shutdown`].
#[derive(Debug)]
pub struct LoggingGuard {
    _guard: Option<tracing_appender::non_blocking::WorkerGuard>,
}

/// Inicializa el logging global de `vary`.
///
/// * `cache_dir`: directorio donde vive `vary.log`; se crea 0700 con
///   [`crate::util::ensure_private_dir`] ignorando errores no fatales.
/// * `verbose`: 0 => INFO en stdout, >=1 => DEBUG, >=2 => TRACE (el archivo
///   siempre registra DEBUG+).
/// * Si `RUST_LOG` está definida, su filtro gana sobre ambos niveles.
///
/// Idempotente: la primera llamada instala las capas y guarda el worker en el
/// global; las siguientes no duplican nada (pero sí pueden reconfigurar
/// niveles vía [`apply_runtime_config`]).
pub fn init(cache_dir: &Path, verbose: u8) {
    let _ = crate::util::ensure_private_dir(cache_dir);

    let rust_log = std::env::var("RUST_LOG").ok();
    let console_default = match verbose {
        0 => "info",
        1 => "debug",
        _ => "trace",
    };
    // RUST_LOG, si existe, GANA sobre ambos niveles por defecto.
    let console_spec = rust_log
        .clone()
        .unwrap_or_else(|| console_default.to_owned());
    let file_spec = rust_log.unwrap_or_else(|| "debug".to_owned());
    if CONSOLE_HANDLE.get().is_some() {
        return;
    }
    let console_filter =
        EnvFilter::try_new(&console_spec).unwrap_or_else(|_| EnvFilter::new(console_default));
    let file_filter = EnvFilter::try_new(&file_spec).unwrap_or_else(|_| EnvFilter::new("debug"));

    // La capa de consola va PRIMERO con `.with` sobre Registry: así el `S`
    // del reload::Layer es Registry (nombrable). La de archivo usa filtro
    // estático (genérico sobre cualquier S).
    let (console_filter, console_handle): (
        reload::Layer<EnvFilter, Registry>,
        reload::Handle<EnvFilter, Registry>,
    ) = reload::Layer::new(console_filter);

    let (log_writer, worker) =
        tracing_appender::non_blocking(tracing_appender::rolling::daily(cache_dir, "vary.log"));

    let console_layer = fmt::layer()
        .compact()
        .without_time()
        .with_filter(console_filter);
    let file_layer = fmt::layer()
        .with_ansi(false)
        .with_writer(log_writer)
        .with_filter(file_filter);

    let subscriber = tracing_subscriber::registry()
        .with(console_layer)
        .with(file_layer);
    let _ = tracing::subscriber::set_global_default(subscriber);

    let _ = CONSOLE_HANDLE.set(console_handle);
    if let Ok(mut guard) = GUARD.lock() {
        *guard = Some(LoggingGuard {
            _guard: Some(worker),
        });
    }
}

/// Nivel efectivo de consola como spec de `EnvFilter`. Pura y testeable
/// (T-002: el test anterior solo afirmaba "no paniquea"; este afirma el
/// NIVEL resultante para cada combinación).
/// Precedencia: `RUST_LOG` > `-v` (1 => debug, 2+ => trace) > `log_level`.
pub fn effective_console_spec(verbose: u8, log_level: &str, rust_log: Option<&str>) -> String {
    if let Some(spec) = rust_log {
        return spec.to_string();
    }
    match verbose {
        0 => log_level.to_owned(),
        1 => "debug".to_owned(),
        _ => "trace".to_owned(),
    }
}

/// Reconfigura niveles tras parsear CLI/TOML (H-028).
/// Precedencia: `RUST_LOG` (si existe, se re-aplica idempotente) > `-v` > `log_level`.
pub fn apply_runtime_config(verbose: u8, log_level: &str) {
    let rust_log = std::env::var("RUST_LOG").ok();
    set_console_level(&effective_console_spec(
        verbose,
        log_level,
        rust_log.as_deref(),
    ));
}

/// Ajusta el filtro de consola si el logger ya está instalado; no-op si no.
pub fn set_console_level(console: &str) {
    if let Some(handle) = CONSOLE_HANDLE.get() {
        let _ = handle.modify(|f| {
            *f = EnvFilter::try_new(console).unwrap_or_else(|_| EnvFilter::new("info"))
        });
    }
}

/// Suelta el worker global (flush del archivo). Llamar antes de salidas que
/// se saltan `Drop` (`process::exit` por señal o pipe roto). Idempotente.
pub fn shutdown() {
    if let Ok(mut guard) = GUARD.lock() {
        drop(guard.take());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_es_idempotente_y_no_paniquea() {
        let tmp = tempfile::tempdir().expect("tempdir");
        init(tmp.path(), 0);
        init(tmp.path(), 2);
        // Si llegamos aquí sin panic ni doble instalación, bien.
        // set_console_level sin init previo tampoco debe hacer nada.
        set_console_level("debug");
        shutdown();
        shutdown();
    }

    /// T-002: la tabla de precedencia del nivel efectivo. (El bug reportado
    /// resultó ser artefacto de medición —caché tibia sin eventos DEBUG—;
    /// estos tests fijan el mapeo en CI para que no regresse de verdad.)
    #[test]
    fn nivel_efectivo_respeta_precedencia() {
        // Sin nada: TOML/default tal cual.
        assert_eq!(effective_console_spec(0, "info", None), "info");
        // TOML log_level llega intacto (verificado en vivo: 162 líneas DEBUG).
        assert_eq!(effective_console_spec(0, "debug", None), "debug");
        assert_eq!(effective_console_spec(0, "warning", None), "warning");
        // -v / -vv pisan TOML.
        assert_eq!(effective_console_spec(1, "info", None), "debug");
        assert_eq!(effective_console_spec(2, "info", None), "trace");
        assert_eq!(effective_console_spec(9, "warning", None), "trace");
        // RUST_LOG gana a todo (verificado en vivo: RUST_LOG=info + -v calla).
        assert_eq!(effective_console_spec(1, "info", Some("info")), "info");
        assert_eq!(effective_console_spec(0, "debug", Some("warn")), "warn");
        assert_eq!(effective_console_spec(2, "info", Some("error")), "error");
    }
}
