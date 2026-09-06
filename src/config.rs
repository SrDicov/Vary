use crate::args::Args;
use ansiterm::Style;
use anyhow::{Context, Result};
use serde::Deserialize;
use std::fmt;
use std::io::{stderr, stdout, IsTerminal};
use std::path::{Path, PathBuf};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    #[default]
    Default,
    Sync,
    Remove,
}

impl fmt::Display for Op {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Op::Default => "default",
            Op::Sync => "sync",
            Op::Remove => "remove",
        };
        write!(f, "{s}")
    }
}

#[derive(Debug, Clone)]
pub struct Colors {
    // Superficie de vary.conf ([colors]/--color); el estilo se aplica vía
    // los campos Style y la detección de TTY de From<&str>.
    #[allow(dead_code)]
    pub enabled: bool,
    #[allow(dead_code)]
    pub error: Style,
    pub warning: Style,
    pub bold: Style,
    pub action: Style,
    pub sl_repo: Style,
    pub ss_name: Style,
    pub ss_ver: Style,
}

impl From<&str> for Colors {
    fn from(s: &str) -> Self {
        match s {
            "always" => Colors::new(),
            "never" => Colors::default(),
            _ if stdout().is_terminal() && stderr().is_terminal() => Colors::new(),
            _ => Colors::default(),
        }
    }
}

impl Default for Colors {
    fn default() -> Self {
        Colors {
            enabled: false,
            error: Style::new(),
            warning: Style::new(),
            bold: Style::new(),
            action: Style::new(),
            sl_repo: Style::new(),
            ss_name: Style::new().bold(),
            ss_ver: Style::new(),
        }
    }
}

impl Colors {
    pub fn new() -> Colors {
        use ansiterm::Color::*;
        Colors {
            enabled: true,
            error: Style::new().fg(Red),
            warning: Style::new().fg(Yellow),
            bold: Style::new().bold(),
            action: Style::new().fg(Blue).bold(),
            sl_repo: Style::new().fg(Green),
            ss_name: Style::new().bold(),
            ss_ver: Style::new().fg(Cyan),
        }
    }
}

/// Configuración global de vary: valores por defecto + ~/.config/vary/vary.conf
/// (Ajuste 1) + flags en tiempo de ejecución.
#[derive(Debug, Clone)]
pub struct Config {
    pub op: Op,
    pub help: bool,
    pub version: bool,
    pub targets: Vec<String>,
    pub args: Args,
    pub color: Colors,
    pub quiet: bool,
    pub interactive: bool,
    pub no_confirm: bool,
    /// Conteo de -v (0=info, 1=debug stdout, 2+=trace stdout)
    pub verbose: u8,

    pub sudo_bin: String,
    pub sudo_flags: Vec<String>,
    pub git_bin: String,
    /// Override de arquitectura (--arch); si es None se consulta a xbps.
    pub curl_bin: String,
    pub arch_override: Option<String>,

    // Rutas base
    pub cache_dir: PathBuf,
    pub data_dir: PathBuf,
    pub config_dir: PathBuf,

    // Valores de vary.conf
    pub log_level: String,
    pub max_concurrent_builds: u32,
    pub makejobs: usize,
    pub force_rebuild: bool,
    pub ttl_cache_seconds: u64,

    // Flags runtime
    pub force_build: bool,
    pub prefer_binary: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self::new().expect("config defaults")
    }
}

#[derive(Debug, Deserialize, Default)]
struct VaryConfFile {
    #[serde(default)]
    general: GeneralSection,
    #[serde(default)]
    build: BuildSection,
    #[serde(default)]
    search: SearchSection,
}

#[derive(Debug, Deserialize, Default)]
struct GeneralSection {
    cache_dir: Option<String>,
    data_dir: Option<String>,
    log_level: Option<String>,
    /// Herramienta de elevación (sudo, doas, run0). Vacío/ausente = autodetectar.
    sudo_bin: Option<String>,
    /// Flags extra para el wrapper de elevación.
    sudo_flags: Option<Vec<String>>,
    /// Binario curl para descargar índices remotos (index.json estilo VUP).
    curl_bin: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct BuildSection {
    max_concurrent_builds: Option<u32>,
    makejobs: Option<usize>,
    force_rebuild: Option<bool>,
}

#[derive(Debug, Deserialize, Default)]
struct SearchSection {
    ttl_cache_seconds: Option<u64>,
}

fn expand_home(p: &str) -> PathBuf {
    if let Some(rest) = p.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(p)
}

/// Resuelve los directorios base de vary (H-021): respeta
/// `XDG_CACHE_HOME` / `XDG_DATA_HOME` / `XDG_CONFIG_HOME` vía el crate `dirs`
/// y solo recurre a `$HOME/.cache` etc. si la variable no está definida.
fn default_dirs(home: &Path) -> (PathBuf, PathBuf, PathBuf) {
    let cache = dirs::cache_dir()
        .unwrap_or_else(|| home.join(".cache"))
        .join("vary");
    let data = dirs::data_dir()
        .unwrap_or_else(|| home.join(".local").join("share"))
        .join("vary");
    let config = dirs::config_dir()
        .unwrap_or_else(|| home.join(".config"))
        .join("vary");
    (cache, data, config)
}

impl Config {
    pub fn new() -> Result<Self> {
        let home = dirs::home_dir().context("no se pudo determinar el directorio HOME")?;
        let (cache_dir, data_dir, config_dir) = default_dirs(&home);

        let makejobs = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1);

        let mut config = Config {
            op: Op::Default,
            help: false,
            version: false,
            targets: Vec::new(),
            args: Args::default(),
            color: Colors::from("auto"),
            quiet: false,
            interactive: false,
            no_confirm: false,
            verbose: 0,
            // Vacío = autodetectar (sudo→doas→run0) o directo si somos root.
            // Ver crate::elevate.
            sudo_bin: String::new(),
            sudo_flags: Vec::new(),
            git_bin: "git".to_string(),
            curl_bin: "curl".to_string(),
            arch_override: None,
            cache_dir,
            data_dir,
            config_dir,
            log_level: "info".to_string(),
            // Builds secuenciales en Opción A; paralelismo intra-paquete vía makejobs (XBPS_MAKEJOBS).
            max_concurrent_builds: 1,
            makejobs,
            force_rebuild: false,
            ttl_cache_seconds: 3600,
            force_build: false,
            prefer_binary: true,
        };
        config.load_vary_conf()?;
        Ok(config)
    }

    /// Carga ~/.config/vary/vary.conf si existe (TOML, Ajuste 1).
    /// Un archivo corrupto NO aborta: se advierte y continúan los defaults.
    pub fn load_vary_conf(&mut self) -> Result<()> {
        let path = self.vary_conf_path();
        let Ok(raw) = std::fs::read_to_string(&path) else {
            tracing::debug!("{} no existe; usando defaults", path.display());
            return Ok(());
        };
        let file: VaryConfFile = match toml::from_str(&raw) {
            Ok(f) => f,
            Err(err) => {
                tracing::warn!(
                    "{} inválido ({err}); ignorando configuración y usando defaults",
                    path.display()
                );
                return Ok(());
            }
        };
        if let Some(d) = file.general.cache_dir.as_deref() {
            self.cache_dir = expand_home(d);
        }
        if let Some(d) = file.general.data_dir.as_deref() {
            self.data_dir = expand_home(d);
        }
        if let Some(l) = file.general.log_level.as_deref() {
            self.log_level = l.to_string();
        }
        if let Some(b) = file.general.sudo_bin.as_deref() {
            self.sudo_bin = b.to_string();
        }
        if let Some(f) = file.general.sudo_flags.clone() {
            self.sudo_flags = f;
        }
        if let Some(c) = file.general.curl_bin.as_deref() {
            self.curl_bin = c.to_string();
        }
        if let Some(n) = file.build.max_concurrent_builds {
            self.max_concurrent_builds = n;
        }
        if let Some(j) = file.build.makejobs {
            if j > 0 {
                self.makejobs = j;
            }
        }
        if let Some(f) = file.build.force_rebuild {
            self.force_rebuild = f;
        }
        if let Some(t) = file.search.ttl_cache_seconds {
            self.ttl_cache_seconds = t;
        }
        Ok(())
    }

    pub fn vary_conf_path(&self) -> PathBuf {
        self.config_dir.join("vary.conf")
    }

    pub fn repos_conf_path(&self) -> PathBuf {
        self.config_dir.join("repos.conf")
    }

    /// Árbol maestro inmutable de void-packages.
    pub fn void_packages_dir(&self) -> PathBuf {
        self.cache_dir.join("void-packages")
    }

    /// Directorio base donde viven los clones de los repos VUR.
    pub fn vurs_dir(&self) -> PathBuf {
        self.data_dir.join("vurs")
    }

    pub fn installed_db_path(&self) -> PathBuf {
        self.cache_dir.join("installed.json")
    }

    pub fn cache_index_path(&self) -> PathBuf {
        self.cache_dir.join("cache.json")
    }

    /// Arquitectura efectiva: override del CLI o la real del sistema.
    pub fn arch(&self) -> String {
        self.arch_override
            .clone()
            .unwrap_or_else(|| std::env::consts::ARCH.to_string())
    }

    pub fn parse_args<S: AsRef<str>>(&mut self, args: &[S]) -> Result<()> {
        crate::command_line::parse_args(self, args)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_dirs_respeta_xdg_con_fallback_a_home() {
        // Nota: muta env del proceso; usa valores únicos y restaura al final.
        // Ningún otro test afirma sobre estos defaults (H-044), así que el
        // riesgo de cross-talk entre hilos es nulo en la práctica.
        let home = PathBuf::from("/tmp/vary-fake-home-xyz");
        let prev = (
            std::env::var("XDG_CACHE_HOME").ok(),
            std::env::var("XDG_DATA_HOME").ok(),
            std::env::var("XDG_CONFIG_HOME").ok(),
        );
        std::env::set_var("XDG_CACHE_HOME", "/tmp/vary-xdg-cache");
        std::env::set_var("XDG_DATA_HOME", "/tmp/vary-xdg-data");
        std::env::set_var("XDG_CONFIG_HOME", "/tmp/vary-xdg-config");
        let (cache, data, config) = default_dirs(&home);
        assert_eq!(cache, PathBuf::from("/tmp/vary-xdg-cache/vary"));
        assert_eq!(data, PathBuf::from("/tmp/vary-xdg-data/vary"));
        assert_eq!(config, PathBuf::from("/tmp/vary-xdg-config/vary"));
        // Sin variables XDG: fallback a $HOME. Luego se restaura el entorno.
        std::env::remove_var("XDG_CACHE_HOME");
        std::env::remove_var("XDG_DATA_HOME");
        std::env::remove_var("XDG_CONFIG_HOME");
        let (cache, data, config) = default_dirs(&home);
        assert_eq!(cache, home.join(".cache").join("vary"));
        assert_eq!(data, home.join(".local").join("share").join("vary"));
        assert_eq!(config, home.join(".config").join("vary"));
        restore_env("XDG_CACHE_HOME", prev.0);
        restore_env("XDG_DATA_HOME", prev.1);
        restore_env("XDG_CONFIG_HOME", prev.2);
    }

    fn restore_env(key: &str, val: Option<String>) {
        if let Some(v) = val {
            std::env::set_var(key, v);
        } else {
            std::env::remove_var(key);
        }
    }
}
