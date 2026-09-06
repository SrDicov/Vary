# SA-5 — Reporte de Auditoría: Configuración, Cascada de Precedencia y Cumplimiento XDG

**Fecha:** 2026-09-06  
**Auditor:** Subagente SA-5 (Configuración y Entorno)  
**Proyecto:** `vary` — Helper VUR para Void Linux  
**Rama / Commit Base:** `vary-mvp` @ `1e32f1f`  
**Ruta del Reporte:** `/run/media/dicov/LudoDrive/dicov-op/Vary/audit/subagents/SA-5.md`  

---

## 1. Resumen Ejecutivo

El presente informe contiene la auditoría exhaustiva del subsistema de configuración de `vary`, analizando la cascada de precedencia de tres capas (**Valores por Defecto $\rightarrow$ Archivo TOML `vary.conf` $\rightarrow$ Argumentos de Línea de Comandos CLI**), el cumplimiento con la especificación de directorios base de Freedesktop.org (**XDG Base Directory Specification**), y la robustez ante archivos de configuración corruptos o malformados (`vary.conf` y `repos.conf`).

### Matriz Resumen de Hallazgos

| ID | Severidad | Módulo(s) Afectado(s) | Título Breve | Estado |
|---|---|---|---|---|
| **[H-002]** | **Alta** | `src/config.rs:175-179`<br>`src/lib.rs:66-67` | Violación de especificación XDG: `XDG_CONFIG_HOME`, `XDG_CACHE_HOME` y `XDG_DATA_HOME` ignorados por hardcoding sobre `$HOME` | Confirmado |
| **[H-003]** | **Crítica** | `src/reposconf.rs:53-69`<br>`src/repo.rs:102,111`<br>`src/install.rs:120,459` | Supresión silenciosa de errores sintácticos en `repos.conf` y riesgo de destrucción total del archivo en `repo add` | Confirmado |
| **[H-004]** | **Media** | `src/command_line.rs:324`<br>`src/config.rs:243-245` | Ruptura de precedencia en `sudo_flags`: el flag de CLI concatena (`extend`) en lugar de sobreescribir la configuración del archivo TOML | Confirmado |
| **[H-005]** | **Media** | `src/config.rs:114,201,237`<br>`src/lib.rs:63-70`<br>`src/logging.rs:29,51-93` | Desconexión total de `log_level`, inmutabilidad prematura del logger (`Once`) y campo muerto en `Config` | Confirmado |
| **[H-006]** | **Baja** | `src/config.rs:130-163`<br>`src/command_line.rs:303-392`<br>`src/args.rs:78,80` | Asimetría en la cascada de 3 capas y flags globales de CLI huérfanos (`--config`, `--cachedir`, `--force-build`, `[colors]`) | Confirmado |
| **[H-007]** | **Media** | `src/config.rs:124-128`<br>`src/config.rs:174-175` | `Config::default()` efectúa I/O dependiente del host y paniquea si `$HOME` no está definido en el entorno | Confirmado |
| **[H-008]** | **Baja** | `src/config.rs:1-298` | Ausencia total de pruebas unitarias (`#[cfg(test)]`) en el módulo `src/config.rs` | Confirmado |

---

## 2. Auditoría Detallada por Ejes Temáticos

### Eje 1: Precedencia de 3 Capas (Defaults $\rightarrow$ TOML $\rightarrow$ CLI)

La arquitectura contractual de `vary` establece que la resolución de configuraciones debe ocurrir en un orden jerárquico estricto:
1. **Capa 1: Valores internos por defecto** instanciados en memoria.
2. **Capa 2: Archivo TOML de usuario** (`~/.config/vary/vary.conf`), que sobreescribe los defaults si el archivo existe.
3. **Capa 3: Argumentos de CLI** pasados en la invocación, que deben tener la máxima prioridad y sobreescribir cualquier valor de las capas anteriores.

#### 1.1 Trazabilidad del Flujo de Merge en Código Real

El flujo de inicialización y mezcla se ejecuta en los siguientes puntos:

1. **Entrada en `src/lib.rs:76-113`:**
   ```rust
   // src/lib.rs:76-82
   let mut config = match Config::new() {
       Ok(config) => config,
       Err(err) => {
           print_error(Style::new(), err);
           return 1;
       }
   };
   // ...
   // src/lib.rs:107-113 (dentro de run2)
   if args.is_empty() {
       let default: Vec<String> = vec!["-Syu".to_string()];
       config.parse_args(&default)?;
   } else {
       config.parse_args(args)?;
   }
   ```

2. **Capa 1 y Capa 2 en `src/config.rs:174-259` (`Config::new` y `load_vary_conf`):**
   ```rust
   // src/config.rs:174-211
   impl Config {
       pub fn new() -> Result<Self> {
           let home = dirs::home_dir().context("no se pudo determinar el directorio HOME")?;
           let cache_dir = home.join(".cache").join("vary");
           let data_dir = home.join(".local").join("share").join("vary");
           let config_dir = home.join(".config").join("vary");

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
               sudo_bin: String::new(),
               sudo_flags: Vec::new(),
               git_bin: "git".to_string(),
               curl_bin: "curl".to_string(),
               arch_override: None,
               cache_dir,
               data_dir,
               config_dir,
               log_level: "info".to_string(),
               max_concurrent_builds: 2,
               force_rebuild: false,
               ttl_cache_seconds: 3600,
               force_build: false,
               prefer_binary: true,
           };
           config.load_vary_conf()?; // <-- Capa 2
           Ok(config)
       }
   ```

   En `load_vary_conf` (`src/config.rs:231-258`), se sobreescriben selectivamente los campos si están presentes en el TOML:
   ```rust
   if let Some(d) = file.general.cache_dir.as_deref() { self.cache_dir = expand_home(d); }
   if let Some(d) = file.general.data_dir.as_deref() { self.data_dir = expand_home(d); }
   if let Some(l) = file.general.log_level.as_deref() { self.log_level = l.to_string(); }
   if let Some(b) = file.general.sudo_bin.as_deref() { self.sudo_bin = b.to_string(); }
   if let Some(f) = file.general.sudo_flags.clone() { self.sudo_flags = f; }
   if let Some(c) = file.general.curl_bin.as_deref() { self.curl_bin = c.to_string(); }
   if let Some(n) = file.build.max_concurrent_builds { self.max_concurrent_builds = n; }
   if let Some(f) = file.build.force_rebuild { self.force_rebuild = f; }
   if let Some(t) = file.search.ttl_cache_seconds { self.ttl_cache_seconds = t; }
   ```

3. **Capa 3 en `src/command_line.rs:298-388` (`handle_arg`):**
   Los argumentos de CLI se procesan secuencialmente sobre la instancia de `config`.

#### 1.2 Matriz de Precedencia Campo por Campo

| Campo en `Config` | Capa 1: Default | Capa 2: TOML (`vary.conf`) | Capa 3: CLI Flag | ¿Precedencia Correcta (CLI > TOML > Def)? | Comportamiento Anómalo Detectado |
|---|---|---|---|---|---|
| `sudo_bin` | `""` (autodetectar) | `[general] sudo_bin` | `--sudo <bin>` | ✅ Sí | Gana CLI limpiamente (`command_line.rs:323`). |
| `sudo_flags` | `vec![]` | `[general] sudo_flags` | `--sudoflags "<flags>"` | ❌ **No (Anomalía)** | **[H-004]** CLI hace `.extend()`, acumulando sobre el TOML en vez de reemplazar. |
| `curl_bin` | `"curl"` | `[general] curl_bin` | `--curl <bin>` | ✅ Sí | Gana CLI limpiamente (`command_line.rs:326`). |
| `git_bin` | `"git"` | ❌ Inexistente en TOML | `--git <bin>` | 🟡 Parcial | No configurable vía TOML (`VaryConfFile` no lo define). |
| `cache_dir` | `~/.cache/vary` | `[general] cache_dir` | ❌ Inexistente en CLI | 🟡 Parcial | `--cachedir` está en `PACMAN_GLOBALS` pero ignorado en `takes_value`. |
| `data_dir` | `~/.local/share/vary`| `[general] data_dir` | ❌ Inexistente en CLI | 🟡 Parcial | No hay flag CLI para override de `data_dir`. |
| `config_dir` | `~/.config/vary` | N/A | ❌ Inexistente en CLI | 🟡 Parcial | `--config` está en `PACMAN_GLOBALS` pero ignorado en `takes_value`. |
| `log_level` | `"info"` | `[general] log_level` | ❌ Inexistente en CLI | ❌ **Roto** | **[H-005]** El campo se lee pero jamás se pasa al logger. |
| `verbose` | `0` | ❌ Inexistente en TOML | `-v` / `--verbose` | ❌ **Roto** | **[H-005]** Incremente `verbose` en `Config`, pero el logger ya cerró su init. |
| `max_concurrent_builds`| `2` | `[build] max_concurrent_builds` | ❌ Inexistente en CLI | 🟡 Parcial | Sin flag CLI (`-j`/`--jobs`). |
| `force_rebuild` / `force_build` | `false` | `[build] force_rebuild` | `--force-build` | 🟡 Asimétrico | Si TOML tiene `force_rebuild = true`, CLI no puede deshabilitarlo (sin `--no-force-build`). |
| `prefer_binary` | `true` | ❌ Inexistente en TOML | `--prefer-binary` / `--no-prefer-binary` | 🟡 Parcial | Solo operable desde CLI. |
| `color` | `Colors::from("auto")`| ❌ Inexistente en TOML | `--color <always\|never\|auto>` | 🟡 Parcial | Comentario en código menciona `[colors]`, pero TOML no lo soporta. |
| `arch_override` | `None` | ❌ Inexistente en TOML | `--arch <arch>` | ✅ Sí | Override puro en CLI. |
| `no_confirm` | `false` | ❌ Inexistente en TOML | `--noconfirm` / `--confirm` | ✅ Sí | Conmutable en CLI. |

---

### Eje 2: Cumplimiento de la Especificación XDG Base Directory

La especificación **XDG Base Directory** define:
- `$XDG_CONFIG_HOME`: Directorio base para configuración de usuario. Default: `$HOME/.config`.
- `$XDG_CACHE_HOME`: Directorio base para archivos temporales/cachés. Default: `$HOME/.cache`.
- `$XDG_DATA_HOME`: Directorio base para datos persistentes. Default: `$HOME/.local/share`.

#### 2.1 Uso del crate `dirs` en `src/config.rs`

En `Cargo.toml:28` se especifica `dirs = "6.0.0"`. Este crate implementa la especificación XDG completa en Linux mediante las funciones:
- `dirs::config_dir()` $\rightarrow$ `std::env::var_os("XDG_CONFIG_HOME")` con fallback a `$HOME/.config`.
- `dirs::cache_dir()` $\rightarrow$ `std::env::var_os("XDG_CACHE_HOME")` con fallback a `$HOME/.cache`.
- `dirs::data_dir()` $\rightarrow$ `std::env::var_os("XDG_DATA_HOME")` con fallback a `$HOME/.local/share`.
- `dirs::home_dir()` $\rightarrow$ `$HOME`.

Sin embargo, en `src/config.rs:175-179`:
```rust
let home = dirs::home_dir().context("no se pudo determinar el directorio HOME")?;
let cache_dir = home.join(".cache").join("vary");
let data_dir = home.join(".local").join("share").join("vary");
let config_dir = home.join(".config").join("vary");
```
**Diagnóstico:** El código utiliza **únicamente** `dirs::home_dir()` y concatena cadenas fijas (`.cache`, `.local/share`, `.config`). **Las funciones `dirs::config_dir()`, `dirs::cache_dir()` y `dirs::data_dir()` nunca son invocadas.**

Como consecuencia directa:
1. Si un usuario define `export XDG_CONFIG_HOME="/mnt/secure/config"`, `vary` lo ignora y busca forzosamente en `/home/<user>/.config/vary/vary.conf`.
2. Si se define `export XDG_CACHE_HOME="/tmp/cache"` (por ejemplo en un disco RAM o NVMe para compilaciones rápidas), `vary` lo ignora y clona `void-packages` en `/home/<user>/.cache/vary/`.
3. Si se define `export XDG_DATA_HOME="/mnt/storage/data"`, `vary` lo ignora y clona los VURs en `/home/<user>/.local/share/vary/`.

Idéntica anomalía se replica en `src/lib.rs:66-67`:
```rust
let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
let cache = home.join(".cache").join("vary");
logging::init(&cache, 0)
```

#### 2.2 Auditoría de Rutas Hardcodeadas en el Resto del Código Base

Se realizó una búsqueda exhaustiva de cadenas como `~/.cache`, `/home/`, `/tmp/`, `/etc/` en todos los módulos (`src/*.rs`):

1. **`src/bootstrap.rs:89,92`:**
   ```rust
   // src/bootstrap.rs:88-94 (test unitario)
   #[test]
   fn conf_tiene_formato_repository() {
       let c = vary_conf_contents(Path::new("/home/u/.cache/vary/void-packages/hostdir/binpkgs"));
       assert_eq!(
           c,
           "repository=/home/u/.cache/vary/void-packages/hostdir/binpkgs\n"
       );
   }
   ```
   *Evaluación:* No representa un problema de runtime ya que está contenido estrictamente dentro de un test unitario (`#[cfg(test)]`). En tiempo de ejecución, `initialize_environment` recibe `void_packages_dir: &Path` parametrizado desde `config.void_packages_dir()`.

2. **`src/init.rs:6,12,18,21`:**
   ```rust
   let sv_dir = Path::new("/etc/sv").join(pkg_name);
   if Path::new("/dev/dinitctl").exists() { ... }
   else if Path::new("/run/runit").exists() {
       let service_link = Path::new("/var/service").join(pkg_name);
   ```
   *Evaluación:* Son rutas canónicas del sistema de init de Void Linux (`runit`/`dinit`), no rutas de usuario. Sin embargo, nótese que `src/init.rs:15,16,23` hardcodea `"sudo"` en lugar de utilizar `config.sudo_bin` (desviación documentada en `AGENTS.md`).

3. **`src/keys.rs:19,23,27`:**
   ```rust
   pub fn keys_dir() -> &'static str { "/etc/xbps.d/keys" }
   pub fn repo_conf_path(name: &str) -> String { format!("/etc/xbps.d/20-vur-{}.conf", name) }
   pub fn key_dest_path(name: &str) -> String { format!("{}/vary-vur-{}.pem", keys_dir(), name) }
   ```
   *Evaluación:* Son rutas fijas del sistema de repositorios XBPS de Void Linux (`/etc/xbps.d/`), lo cual es correcto conforme al diseño de distribución.

4. **Ubicación de `repos.conf` y `vary.conf`:**
   En `src/config.rs:261-267`:
   ```rust
   pub fn vary_conf_path(&self) -> PathBuf {
       self.config_dir.join("vary.conf")
   }

   pub fn repos_conf_path(&self) -> PathBuf {
       self.config_dir.join("repos.conf")
   }
   ```
   *Evaluación:* Ambas rutas dependen directamente de `self.config_dir`. Al estar `self.config_dir` hardcodeado a `$HOME/.config/vary`, ambas quedan fuera del cumplimiento de `XDG_CONFIG_HOME`.

---

### Eje 3: Manejo de Archivos TOML Corruptos o Inválidos

#### 3.1 Comportamiento de `src/config.rs` ante `vary.conf` Corrupto

En `src/config.rs:215-230`:
```rust
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
    // ...
```
**Evaluación:**
1. **Archivo inexistente:** Retorna `Ok(())` silenciosamente (con evento de debug), procediendo con los valores por defecto. Comportamiento correcto.
2. **Error de sintaxis TOML:** `toml::from_str(&raw)` produce un `toml::de::Error` que incluye de forma nativa la fila y columna del error (ej. `TOML parse error at line 3, column 9`).
3. **No entra en pánico:** Emite un `tracing::warn!` con la descripción del error y la ruta del archivo, y continúa la ejecución usando los defaults.
4. **Problema detectado:** El aviso se emite mediante `tracing::warn!`. Debido a que `tracing_subscriber::fmt` escribe por defecto a `stdout` (no a `stderr`) y a que `tracing` está desacoplado del nivel interactivo antes de `run2`, en pipes o salidas capturadas puede contaminar la salida o no ser visible para el usuario si se ejecutan herramientas de scripting.

#### 3.2 Comportamiento de `src/reposconf.rs` ante `repos.conf` Corrupto (Vulnerabilidad Crítica)

En `src/reposconf.rs:53-69`:
```rust
impl ReposConf {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                tracing::warn!(
                    "{} no existe, usando configuración de repos por defecto",
                    path.display()
                );
                return Ok(Self::default());
            }
            Err(err) => {
                return Err(err).with_context(|| format!("no se pudo leer {}", path.display()));
            }
        };
        toml::from_str(&text).with_context(|| format!("TOML inválido en {}", path.display()))
    }
```
A primera vista, `ReposConf::load` parece retornar un `Result::Err` informativo enriquecido con contexto `anyhow`. Sin embargo, al auditar **todos** los puntos de llamada en el código:

- `src/info.rs:12`: `let repos_conf = ReposConf::load(config.repos_conf_path()).unwrap_or_default();`
- `src/install.rs:120`: `let repos_conf = ReposConf::load(config.repos_conf_path()).unwrap_or_default();`
- `src/install.rs:459`: `let repos_conf = ReposConf::load(config.repos_conf_path()).unwrap_or_default();`
- `src/repo.rs:102`: `let mut conf = ReposConf::load(config.repos_conf_path()).unwrap_or_default();`
- `src/repo.rs:143`: `let conf = ReposConf::load(config.repos_conf_path()).unwrap_or_default();`
- `src/repo.rs:159`: `let mut conf = ReposConf::load(config.repos_conf_path()).unwrap_or_default();`
- `src/repo.rs:196`: `let conf = ReposConf::load(config.repos_conf_path()).unwrap_or_default();`
- `src/search.rs:77`: `let repos_conf = ReposConf::load(config.repos_conf_path()).unwrap_or_default();`
- `src/upgrade.rs:10`: `let repos_conf = ReposConf::load(config.repos_conf_path()).unwrap_or_default();`
- `src/upgrade.rs:53`: `let repos_conf = ReposConf::load(config.repos_conf_path()).unwrap_or_default();`

**Evaluación del desastre:**
1. **Supresión total:** El método `unwrap_or_default()` descarta silenciosamente el `Err`. El usuario jamás recibe una advertencia ni aviso de que su archivo `repos.conf` tiene una sintaxis rota.
2. **Pérdida Catastrófica de Datos en `repo add`:** En `src/repo.rs:101-115`:
   ```rust
   // src/repo.rs:101-106
   let mut conf = ReposConf::load(config.repos_conf_path()).unwrap_or_default();
   // ...
   conf.vur.insert(name.clone(), entry);
   conf.save(config.repos_conf_path())?;
   ```
   Si `repos.conf` contiene 10 repositorios pero tiene un error de sintaxis en una línea, `load` falla, `unwrap_or_default` entrega un `ReposConf` **completamente vacío**, se inserta el nuevo repositorio solicitado y `conf.save` **sobrescribe en disco el archivo original, destruyendo permanentemente todos los repositorios previos del usuario sin posibilidad de recuperación**.

#### 3.3 Comportamiento ante la Ausencia de `$HOME` (`dirs::*` devolviendo `None`)

1. **En `src/config.rs:175`:**
   ```rust
   let home = dirs::home_dir().context("no se pudo determinar el directorio HOME")?;
   ```
   Si el proceso se ejecuta en un entorno sin `$HOME` definido (ej. servicios de systemd, contenedores mínimos, chroots de compilación o usuarios de sistema sin homedir):
   - `Config::new()` falla limpiamente con un error `anyhow` legible (`"no se pudo determinar el directorio HOME"`).
   - En `src/lib.rs:76-81`, `run()` captura el error, lo formatea mediante `print_error` y sale con código 1 sin provocar un pánico feo al usuario.
2. **En `Config::default()` (`src/config.rs:124-128`):**
   ```rust
   impl Default for Config {
       fn default() -> Self {
           Self::new().expect("config defaults")
       }
   }
   ```
   Si una función o test llama a `Config::default()` en un entorno sin `$HOME`, se produce un **pánico no controlado** en el hilo actual (`panic: config defaults`).

---

## 3. Catálogo Detallado de Hallazgos

---

### [H-002] Violación de Especificación XDG: Variables `XDG_*_HOME` Ignoradas
- **Severidad:** Alta
- **Componentes Afectados:** `src/config.rs:175-179`, `src/lib.rs:66-67`
- **Descripción:**
  El estándar de Freedesktop.org para sistemas UNIX define que las herramientas deben respetar las variables de entorno `XDG_CONFIG_HOME`, `XDG_CACHE_HOME` y `XDG_DATA_HOME` si se encuentran presentes. Aunque el proyecto importa la biblioteca `dirs = "6.0.0"`, que provee implementaciones completas y probadas para esta resolución, el código base nunca invoca las funciones especializadas de `dirs` y en su lugar recurre a la concatenación rígida sobre `$HOME`:
  ```rust
  // src/config.rs:175-179
  let home = dirs::home_dir().context("no se pudo determinar el directorio HOME")?;
  let cache_dir = home.join(".cache").join("vary");
  let data_dir = home.join(".local").join("share").join("vary");
  let config_dir = home.join(".config").join("vary");
  ```
- **Impacto:**
  Imposibilita el aislamiento de perfiles de usuario, entornos de pruebas de integración, construcciones reproducibles en contenedores sin acceso a `$HOME`, o el montaje de cachés en unidades rápidas (RAM/NVMe) configuradas vía entorno XDG.
- **Remediación Recomendada:**
  Utilizar las funciones nativas provistas por el crate `dirs`:
  ```rust
  let config_dir = dirs::config_dir()
      .or_else(|| dirs::home_dir().map(|h| h.join(".config")))
      .context("no se pudo determinar el directorio de configuración (XDG_CONFIG_HOME / HOME)")?
      .join("vary");

  let cache_dir = dirs::cache_dir()
      .or_else(|| dirs::home_dir().map(|h| h.join(".cache")))
      .context("no se pudo determinar el directorio de caché (XDG_CACHE_HOME / HOME)")?
      .join("vary");

  let data_dir = dirs::data_dir()
      .or_else(|| dirs::home_dir().map(|h| h.join(".local").join("share")))
      .context("no se pudo determinar el directorio de datos (XDG_DATA_HOME / HOME)")?
      .join("vary");
  ```
  Actualizar igualmente `src/lib.rs:66-67` para que use `dirs::cache_dir()`.

---

### [H-003] Supresión Silenciosa de Errores en `repos.conf` y Destrucción de Datos en `repo add`
- **Severidad:** Crítica
- **Componentes Afectados:** `src/reposconf.rs:53-69`, `src/repo.rs:101-115`, `src/install.rs:120,459`, `src/search.rs:77`, `src/upgrade.rs:10,53`, `src/info.rs:12`
- **Descripción:**
  Cuando el archivo `~/.config/vary/repos.conf` contiene errores gramaticales o de parseo TOML, `ReposConf::load()` devuelve un `Result::Err`. Sin embargo, prácticamente todos los módulos consumidores en `vary` usan el patrón:
  ```rust
  let conf = ReposConf::load(config.repos_conf_path()).unwrap_or_default();
  ```
  Esto suprime el error por completo sin notificar al usuario.
  Lo más grave ocurre en `src/repo.rs:102` (`repo_add`):
  ```rust
  let mut conf = ReposConf::load(config.repos_conf_path()).unwrap_or_default();
  // ...
  conf.vur.insert(name.clone(), entry);
  conf.save(config.repos_conf_path())?;
  ```
  Si un usuario que ya tenía configurados múltiples repositorios añade uno nuevo mientras su archivo tenía un error menor de sintaxis, `ReposConf::load` falla, `unwrap_or_default` crea una lista vacía, y `conf.save` sobreescribe el archivo en disco, **eliminando de forma irrecuperable toda la configuración previa de repositorios**.
- **Impacto:**
  Pérdida irreversible de datos de configuración del usuario y diagnóstico imposible de fallos en sincronización o búsquedas.
- **Remediación Recomendada:**
  1. En `src/reposconf.rs:53-69`, cuando el archivo existe pero no puede ser deserializado por TOML inválido, emitir inmediatamente una advertencia diagnóstica con `eprintln!` y retornar un error explícito.
  2. En `src/repo.rs:102` (y comandos de mutación como `remove` o `rekey`), **no utilizar `unwrap_or_default()`**. Si el archivo existe y está corrupto, la operación debe abortar de inmediato advirtiendo al usuario:
     ```rust
     let mut conf = match ReposConf::load(config.repos_conf_path()) {
         Ok(c) => c,
         Err(e) => {
             anyhow::bail!("No se pudo cargar {}: {}. Corrige los errores de sintaxis antes de modificar los repositorios.", config.repos_conf_path().display(), e);
         }
     };
     ```

---

### [H-004] Ruptura de Precedencia en `sudo_flags` (CLI Acumula en Vez de Sobreescribir)
- **Severidad:** Media
- **Componentes Afectados:** `src/command_line.rs:324`, `src/config.rs:243-245`
- **Descripción:**
  En la cascada estándar de configuración, los argumentos de CLI deben prevalecer y sobreescribir las opciones definidas en archivos de configuración.
  En `src/config.rs:243-245`:
  ```rust
  if let Some(f) = file.general.sudo_flags.clone() {
      self.sudo_flags = f;
  }
  ```
  Sin embargo, en `src/command_line.rs:324`:
  ```rust
  Arg::Long("sudoflags") => self.sudo_flags.extend(value.unwrap().split_whitespace().map(|s| s.to_string())),
  ```
  Al utilizar `extend`, cualquier flag pasado en la línea de comandos (`--sudoflags "-A"`) se concatena al final del vector proveniente de `vary.conf` (`sudo_flags = ["-E"]`), dando como resultado `["-E", "-A"]`.
- **Impacto:**
  El usuario no tiene ningún mecanismo para invalidar, limpiar o sustituir desde el CLI los flags definidos en `vary.conf`. Además, invocaciones múltiples de `--sudoflags` concatenan indefinidamente.
- **Remediación Recomendada:**
  Introducir una bandera booleana o sustituir el vector en la primera invocación de CLI de `--sudoflags`:
  ```rust
  Arg::Long("sudoflags") => {
      self.sudo_flags = value.unwrap().split_whitespace().map(|s| s.to_string()).collect();
  }
  ```

---

### [H-005] Desconexión Total de `log_level` e Inmutabilidad Prematura del Logger
- **Severidad:** Media
- **Componentes Afectados:** `src/config.rs:114,144,201,237-239`, `src/lib.rs:63-70`, `src/logging.rs:29,51-93`
- **Descripción:**
  1. `logging::init(&cache, 0)` se ejecuta en `src/lib.rs:68` **antes** de instanciar `Config::new()` y **antes** de procesar los argumentos de CLI.
  2. Dentro de `logging::init`, la inicialización del suscriptor global se bloquea mediante `static INIT: Once = Once::new();`. Cualquier llamada posterior a `logging::init` es un no-op que retorna un guardián vacío.
  3. `Config::load_vary_conf()` lee `log_level` desde `vary.conf` y lo asigna a `self.log_level` (`src/config.rs:238`). Sin embargo, **`self.log_level` jamás se lee ni se utiliza en ninguna otra parte de todo el código de `vary`**. Es un campo 100% muerto.
  4. El flag de CLI `-v` / `--verbose` incrementa `self.verbose` en `src/command_line.rs:310`, pero como `logging::init` ya ejecutó su `INIT.call_once` con `verbose = 0`, el suscriptor de tracing ya quedó fijado en nivel `info`.
- **Impacto:**
  - Configurar `log_level = "debug"` en `vary.conf` no tiene ningún efecto en el nivel de registro.
  - El comentario en `src/lib.rs:63-64` (`"cache_dir aún no se conoce con precisión... Config::new() lo reconfigura"`) es falso; nunca se reconfigura.
  - Si el usuario configura un `cache_dir` alternativo en `vary.conf`, el archivo `vary.log` continuará escribiéndose en la ruta default de `~/.cache/vary/vary.log`.
- **Remediación Recomendada:**
  Utilizar un manejador dinámico de recarga (`tracing_subscriber::reload::Handle`) o retrasar la inicialización de `logging::init` hasta que `Config::new()` y `parse_args()` hayan consolidado los valores de `cache_dir`, `log_level` y `verbose`.

---

### [H-006] Asimetría en la Cascada de 3 Capas y Opciones Huérfanas de CLI
- **Severidad:** Baja
- **Componentes Afectados:** `src/config.rs:130-163`, `src/command_line.rs:367-402`, `src/args.rs:78,80`
- **Descripción:**
  Existe una discrepancia considerable entre las opciones expuestas en CLI y las admitidas en `vary.conf`:
  1. `git_bin`: Es configurable vía `--git <bin>` en CLI, pero la estructura `GeneralSection` de `vary.conf` carece del campo `git_bin: Option<String>`, a pesar de que la documentación (`AGENTS.md`) afirma que es configurable en `vary.conf`.
  2. `[colors]`: El comentario de `src/config.rs:30` promete soporte de `[colors]` en `vary.conf`, pero no existe tal sección en `VaryConfFile`.
  3. `prefer_binary`: Configurable mediante `--prefer-binary` / `--no-prefer-binary` en CLI, pero no en `vary.conf`.
  4. `--force-build`: CLI permite forzar compilaciones con `--force-build`. `vary.conf` permite fijar `force_rebuild = true`. Sin embargo, no existe un flag simétrico `--no-force-rebuild` en CLI para desactivarlo si el archivo lo activó.
  5. Flags PACMAN `--config` y `--cachedir`: Están definidos en `PACMAN_GLOBALS` (`src/args.rs:78,80`), pero en `src/command_line.rs:392` (`takes_value`) no están registrados. Si un usuario invoca `vary --config /otro/vary.conf`, `takes_value` devuelve `TakesValue::No`, el argumento se trata como un flag sin valor, y la ruta `/otro/vary.conf` se interpreta erróneamente como un paquete a instalar en `self.targets`.
- **Impacto:**
  Inconsistencia en la experiencia de usuario y degradación de la promesa de configuración jerárquica de tres capas.
- **Remediación Recomendada:**
  1. Agregar `git_bin` a `GeneralSection` en `src/config.rs`.
  2. Agregar soporte funcional o eliminar comentarios engañosos sobre `[colors]`.
  3. Implementar `--config <path>` y `--cachedir <path>` en `command_line.rs` vinculándolos con `takes_value` y aplicándolos a `Config`.

---

### [H-007] `Config::default()` Efectúa I/O Dependiente del Host y Paniquea sin `$HOME`
- **Severidad:** Media
- **Componentes Afectados:** `src/config.rs:124-128`, `src/config.rs:174-175`
- **Descripción:**
  La implementación del trait `Default` para `Config` delega en `Config::new()`:
  ```rust
  impl Default for Config {
      fn default() -> Self {
          Self::new().expect("config defaults")
      }
  }
  ```
  Esto introduce dos efectos secundarios graves:
  1. **Panics no controlados:** Si `dirs::home_dir()` retorna `None` (entorno sin `$HOME`), `Config::default()` entra en pánico inmediatamente.
  2. **Contaminación de tests (Test Pollution):** Cada test unitario que crea una configuración con `Config::default()` lee el sistema de archivos real del usuario desarrollador en `~/.config/vary/vary.conf`. Si el desarrollador tiene un archivo local modificado, los tests pueden fallar o comportarse de manera no determinista.
- **Impacto:**
  Inestabilidad en suites de prueba y violación del contrato estándar de `Default` en Rust (debe ser puro y no fallar).
- **Remediación Recomendada:**
  Separar la inicialización de defaults en memoria pura (sin I/O y con rutas de fallback relativas o neutrales) del método `Config::new()` / `Config::load_file()`.

---

### [H-008] Ausencia Total de Tests Unitarios en `src/config.rs`
- **Severidad:** Baja
- **Componentes Afectados:** `src/config.rs:1-298`
- **Descripción:**
  El archivo `src/config.rs` contiene 298 líneas de código crítico para el arranque de la aplicación y la resolución de precedencias, pero carece de un bloque `#[cfg(test)]`. No existen pruebas automatizadas para:
  - Carga y parseo de `vary.conf`.
  - Comportamiento ante sintaxis TOML inválida.
  - Expansión de tilde `expand_home`.
  - Fusión de valores por defecto vs TOML.
- **Impacto:**
  Riesgo de regresiones inadvertidas durante futuras refactorizaciones.
- **Remediación Recomendada:**
  Añadir una suite de pruebas unitarias en `src/config.rs` utilizando `tempfile::TempDir` para verificar el parseo de TOML válido, corrupto, expansión de tilde y mezcla de campos.

---

## 4. Conclusiones y Plan de Remediación

El sistema de configuración de `vary` cuenta con una base conceptual sólida, pero exhibe debilidades importantes en los extremos de su integración:
1. **La integración XDG es nominal pero no efectiva**, dependiendo de rutas hardcodeadas sobre `$HOME`.
2. **El manejo de errores en `repos.conf` representa una brecha de integridad de datos crítica**, donde un error tipográfico puede resultar en el borrado total de la lista de repositorios comunitarios en disco.
3. **El logging está congelado antes del procesamiento de opciones**, anulando la utilidad de `log_level` en `vary.conf` y de los modificadores de verbosidad en CLI.

La corrección de estos hallazgos otorgará al proyecto robustez de nivel de producción, aislamiento ambiental completo y plena compatibilidad con las directrices UNIX y de Void Linux.
