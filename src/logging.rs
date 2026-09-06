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
//! # Precedencia de `RUST_LOG`
//!
//! Si la variable de entorno `RUST_LOG` está presente, su filtro GANA sobre
//! ambos niveles por defecto: se aplica tal cual a la capa de stdout y a la
//! de archivo. Sin `RUST_LOG` rigen los niveles descritos arriba.
//!
//! [`init`] es idempotente (usa [`std::sync::Once`]): solo la primera llamada
//! instala las capas; las siguientes no duplican capas y devuelven un
//! [`LoggingGuard`] vacío. El `main` debe conservar el guardián devuelto por
//! la primera llamada vivo hasta el final del proceso para garantizar el
//! flush del archivo (el `Drop` del `WorkerGuard` lo hace).

use std::path::Path;
use std::sync::Once;

use tracing_subscriber::{fmt, layer::SubscriberExt, EnvFilter, Layer};

static INIT: Once = Once::new();

/// Retiene vivo el [`tracing_appender::non_blocking::WorkerGuard`] del logger.
///
/// Su `Drop` apaga el hilo escritor y hace flush del archivo de log; el
/// llamador debe mantenerlo hasta el final del proceso.
#[derive(Debug)]
pub struct LoggingGuard {
    _guard: Option<tracing_appender::non_blocking::WorkerGuard>,
}

/// Inicializa el logging global de `vary`.
///
/// * `cache_dir`: directorio donde vive `vary.log`; se crea con
///   [`std::fs::create_dir_all`] ignorando errores no fatales.
/// * `verbose`: 0 => INFO en stdout, >=1 => DEBUG, >=2 => TRACE (el archivo
///   siempre registra DEBUG+).
/// * Si `RUST_LOG` está definida, su filtro gana sobre ambos niveles.
///
/// Idempotente: la primera llamada instala las capas y devuelve un
/// [`LoggingGuard`] con el worker; las siguientes devuelven un guardián vacío
/// y no tocan nada.
pub fn init(cache_dir: &Path, verbose: u8) -> LoggingGuard {
    let _ = std::fs::create_dir_all(cache_dir);

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

    let mut guard = LoggingGuard { _guard: None };

    INIT.call_once(|| {
        let console_filter =
            EnvFilter::try_new(&console_spec).unwrap_or_else(|_| EnvFilter::new(console_default));
        let file_filter =
            EnvFilter::try_new(&file_spec).unwrap_or_else(|_| EnvFilter::new("debug"));

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

        guard._guard = Some(worker);
    });

    guard
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_es_idempotente_y_no_paniquea() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let _primera = init(tmp.path(), 0);
        let segunda = init(tmp.path(), 2);
        assert!(
            segunda._guard.is_none(),
            "la segunda llamada no debe instalar capas ni retener un worker propio"
        );
    }
}
