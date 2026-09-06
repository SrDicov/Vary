# SA-4: Reporte de Auditoría — UX, CLI, Terminal y Manejo de Banderas

**Fecha de emisión:** 2026-09-06  
**Subagente auditor:** SA-4: UX / CLI / Terminal  
**Proyecto:** `vary` — Helper VUR para Void Linux  
**Rama / Versión base:** `vary-mvp` @ `a781acd` (v0.2.5)  
**Alcance:** CLI, `src/command_line.rs`, `src/config.rs`, `src/args.rs`, puertas interactivas en `src/install.rs`, `src/upgrade.rs`, `src/keys.rs`, utilidades en `src/util.rs` y `src/review.rs`, manejo visual y logging en `src/logging.rs` y `src/xbps.rs`, códigos de retorno y mensajería orientada al usuario.

---

## 1. Resumen Ejecutivo

La auditoría de la superficie de usuario (CLI/UX) de `vary` revela una marcada disparidad entre la interfaz anunciada al usuario y la lógica real ejecutada en el pipeline:

1. **Banderas Fantasma (Phantom Flags):** Se identificó un volumen masivo de opciones heredadas acríticamente del port original (`paru`) o declaradas como marcadores de posición (placeholders) que **no alteran en absoluto el comportamiento del sistema**. Destaca el riesgo crítico de `-p` / `--print`, el cual es aceptado silenciosamente pero, en lugar de simular o imprimir objetivos, **procede a compilar e instalar paquetes con elevación de privilegios**. Asimismo, `max_concurrent_builds`, `--interactive`, `--asdeps`, `--asexplicit`, `-v`/`-vv`/`--verbose` y `log_level` carecen de efectividad real.
2. **Puertas Interactivas y Degradación no-TTY:** El hot-fix `H-001` corrigió exitosamente la auto-aprobación involuntaria por EOF en entornos automatizados (CI/pipes). Sin embargo, la remediación actual causa **abortos silenciosos y sin mensajes explicativos** en `install.rs` y `upgrade.rs`. Por otro lado, la convención de Void Linux (`-y` / `--yes`) choca directamente con la convención de Arch/pacman: `-y` solo sincroniza repositorios VUR y `--yes` falla como argumento desconocido.
3. **Manejo Visual y Terminal:** La ausencia de `indicatif` (compromiso contractual A7) deja el proceso sin barras de progreso ni spinners. Durante compilaciones pesadas de `xbps-src`, el volcado crudo de miles de líneas de compilador inunda la terminal sin canalizarse a `vary.log`. La terminal carece de rutinas de restauración de modo TTY/cursor ante señales o pánicos.
4. **Códigos de Salida y Hints Accionables:** Prácticamente todos los errores de validación, red, I/O o cancelación colapsan en el código genérico `1`, omitiendo códigos semánticos estándar como `127` (Command Not Found) y enmascarando procesos muertos por señal como `-1` (`255`). Varios mensajes de recuperación sugieren comandos sintácticamente erróneos (`xbps-install git` sin `sudo` ni `-S`) o contradictorios con la política de seguridad del sistema ("ejecuta vary como root").

---

## 2. Matriz Consolidada de Hallazgos

| ID | Severidad | Módulo(s) Afectado(s) | Título Breve |
|---|---|---|---|
| **[H-401]** | **CRITICAL** | `src/args.rs:12-13`<br>`src/command_line.rs:281-286`<br>`src/lib.rs:144-179` | Banderas fantasma `--print` / `-p` ejecutan compilación e instalación real en lugar de dry-run |
| **[H-402]** | **MEDIUM** | `src/config.rs:115,203,249`<br>`src/install.rs:301-341`<br>`etc/vary.conf.example:33-34` | `max_concurrent_builds` es una opción de configuración e interfaz fantasma sin uso en el pipeline |
| **[H-403]** | **MEDIUM** | `src/command_line.rs:330,352-357`<br>`src/config.rs:97,188`<br>`src/install.rs:400`<br>`src/args.rs:3-67` | Flags residuales `--interactive`, `--asdeps`, `--asexplicit` y catálogo de pacman en `src/args.rs` son aceptados y descartados en silencio |
| **[H-404]** | **MEDIUM** | `src/lib.rs:65-70`<br>`src/logging.rs:29,68-73`<br>`src/command_line.rs:310`<br>`src/config.rs:100,114,237` | Control de verbosidad roto: `-v`, `-vv`, `--verbose` y `log_level` son banderas fantasma por inicialización prematura e inmutable del logger |
| **[H-405]** | **HIGH** | `src/util.rs:9-16,43-50`<br>`src/install.rs:295-297`<br>`src/upgrade.rs:105-107`<br>`src/review.rs:54-58` | Degradación no-TTY tras H-001 produce abortos silenciosos sin diagnósticos ni hints en entornos automatizados |
| **[H-406]** | **MEDIUM** | `src/command_line.rs:340-343,370`<br>`src/help.rs:18,23-24` | Colisión de convenciones Void/Arch: flag `-y` no autoriza prompts y `--yes` es rechazada con error |
| **[H-407]** | **HIGH** | `Cargo.toml:109-124`<br>`src/xbps.rs:399-416`<br>`src/logging.rs:74-76` | Ausencia total de spinners (`indicatif`), inundación descontrolada de salida de compiladores y pérdida de logs de subprocesos |
| **[H-408]** | **MEDIUM** | `src/main.rs:50-64`<br>`src/review.rs:61-76`<br>`src/signal.rs:121-131,167-176` | Riesgo de corrupción de terminal tras interrupción o pánico y falta de desregistro/restauración de paginadores |
| **[H-409]** | **MEDIUM** | `src/lib.rs:78-81,92-95,99-102`<br>`src/xbps.rs:378-380,416`<br>`src/main.rs:67` | Inconsistencia de códigos de salida: colapso en código 1, fuga de `-1` (255) y omisión del código 127 |
| **[H-410]** | **LOW** | `src/bootstrap.rs:54-57`<br>`src/elevate.rs:51-55`<br>`src/lock.rs:43-47`<br>`src/lib.rs:52-60` | Inconsistencias, errores sintácticos en comandos sugeridos y estilo visual degradado en mensajes de error |

---

## 3. Auditoría Detallada por Área

### 3.1 Inventario de Banderas Fantasma (Phantom Flags)

#### [H-401] Banderas fantasma `--print` / `-p` ejecutan compilación e instalación real en lugar de dry-run
- **Severidad:** Critical
- **Archivos:** [src/args.rs:12-13](file:///run/media/dicov/LudoDrive/dicov-op/Vary/src/args.rs#L12-L13), [src/command_line.rs:281-286](file:///run/media/dicov/LudoDrive/dicov-op/Vary/src/command_line.rs#L281-L286), [src/lib.rs:144-179](file:///run/media/dicov/LudoDrive/dicov-op/Vary/src/lib.rs#L144-L179)
- **Descripción:**  
  En el gestor `pacman` y en el helper original `paru`, la bandera `-p` / `--print` actúa como un dry-run de impresión: resuelve la transacción e imprime en stdout la lista de paquetes o URLs que se descargarían o instalarían, **sin modificar el sistema ni solicitar confirmación de escritura**.  
  En `vary`, `-p` y `print` están registrados en `PACMAN_FLAGS`:
  ```rust
  // src/args.rs:12-13
  "p",
  "print",
  "print-format",
  ```
  Al ser reconocidos por `arg.is_pacman_arg()` en `src/command_line.rs:281-286`, se insertan en `config.args.args`. Sin embargo, en `src/lib.rs:handle_sync()`, **`config.args.has_arg("p", "print")` jamás se evalúa**:
  ```rust
  // src/lib.rs:145-149
  let has_search = config.args.has_arg("s", "search");
  let has_info = config.args.has_arg("i", "info");
  let has_refresh = config.args.has_arg("y", "refresh");
  let has_upgrade = config.args.has_arg("u", "sysupgrade");
  let has_downloadonly = config.args.has_arg("w", "downloadonly");
  ```
  Al no coincidir con ninguna de las opciones anteriores y existir objetivos en `config.targets`, la ejecución se despacha directamente a `install::install(config)`:
  ```rust
  // src/lib.rs:173-176
  if !config.targets.is_empty() {
      // instalar paquetes específicos
      return install::install(config);
  }
  ```
- **Prueba empírica:**  
  Al invocar `./target/debug/vary -S --print foo`, `vary` no imprime los objetivos ni simula la operación; inicializa el `masterdir`, ejecuta `./xbps-src binary-bootstrap` y despacha la resolución completa para compilar/instalar `foo`:
  ```text
  $ ./target/debug/vary -S --print foo
   INFO vary::masterdir: ejecutando ./xbps-src binary-bootstrap (puede tardar)...
  error: paquete no encontrado en repos oficiales ni VURs: foo
  ```
- **Impacto:** Si un usuario invoca `vary -S -p <pkg>` o `vary -S --print <pkg>` con fines de inspección o scripting, el sistema procederá a descargar fuentes, compilar e invocar `sudo xbps-install`.
- **Remediación:** Implementar un manipulador explícito para `-p`/`--print` en `handle_sync()` que resuelva el plan e imprima los objetivos (`plan.installs` y `plan.builds`) en stdout terminando con código 0, o bien rechazar explícitamente el flag en `parse_args` informando que no está implementado.

---

#### [H-402] `max_concurrent_builds` es una opción de configuración e interfaz fantasma
- **Severidad:** Medium
- **Archivos:** [src/config.rs:115,203,249-251](file:///run/media/dicov/LudoDrive/dicov-op/Vary/src/config.rs#L115), [src/install.rs:301-341](file:///run/media/dicov/LudoDrive/dicov-op/Vary/src/install.rs#L301-L341), [etc/vary.conf.example:33-34](file:///run/media/dicov/LudoDrive/dicov-op/Vary/etc/vary.conf.example#L33-L34)
- **Descripción:**  
  El campo `max_concurrent_builds` se define en la estructura `Config` y se parsea desde `~/.config/vary/vary.conf`:
  ```rust
  // src/config.rs:115, 203, 249-251
  pub max_concurrent_builds: u32,
  ...
  max_concurrent_builds: 2,
  ...
  if let Some(n) = file.build.max_concurrent_builds {
      self.max_concurrent_builds = n;
  }
  ```
  Sin embargo, en `src/install.rs`, el bucle de compilación de paquetes VUR es **completamente secuencial**:
  ```rust
  // src/install.rs:301-341
  let mut built_names: Vec<String> = Vec::new();
  for item in &plan.builds {
      ...
      repo.materialize_pkg(&parent_pkg)?;
      repo.project_pkg(&md.srcpkgs_dir(), &parent_pkg, explicit)?;
      let res = md.build_pkg(&parent_pkg, &config.sudo_bin, &config.sudo_flags);
      let _ = repo.unproject_pkg(&md.srcpkgs_dir(), &parent_pkg);
      res.with_context(|| format!("building {}", parent_pkg))?;
      built_names.push(item.name.clone());
  }
  ```
  No existe ningún hilo concurrente, ni scheduler topológico de niveles, ni uso de `config.max_concurrent_builds`.
- **Impacto:** Falsa expectativa de paralelismo para el usuario. Aunque configure `max_concurrent_builds = 8` en su archivo de configuración, las plantillas VUR se construirán de una en una.
- **Remediación:** Implementar el scheduler paralelo contractual (R1) o, en caso de permanecer secuencial en MVP, emitir un aviso claro en `--help` y logs advirtiendo que la concurrencia está restringida a nivel de flags internas de `xbps-src` (-j).

---

#### [H-403] Flags residuales `--interactive`, `--asdeps`, `--asexplicit` y catálogo de pacman son aceptados y descartados en silencio
- **Severidad:** Medium
- **Archivos:** [src/command_line.rs:330,352-357](file:///run/media/dicov/LudoDrive/dicov-op/Vary/src/command_line.rs#L330), [src/config.rs:97,188](file:///run/media/dicov/LudoDrive/dicov-op/Vary/src/config.rs#L97), [src/install.rs:400](file:///run/media/dicov/LudoDrive/dicov-op/Vary/src/install.rs#L400), [src/args.rs:3-67](file:///run/media/dicov/LudoDrive/dicov-op/Vary/src/args.rs#L3-L67)
- **Descripción:**  
  1. `--interactive`: Se declara en `Config.interactive` (`src/config.rs:97`), se parsea en `src/command_line.rs:330` (`self.interactive = true`), pero **jamás se lee en ningún módulo**. No está documentado en `help.rs` ni altera ningún prompt.
  2. `--asdeps` y `--asexplicit`: Se parsean en `src/command_line.rs:352-357`. En `src/install.rs:400`, un comentario confirma que fueron ignorados:
     ```rust
     // Also handle --asdeps etc? For now just sync
     let code = xbps::install(&all_install_names, &extra, &config.sudo_bin, &config.sudo_flags)?;
     ```
     En `xbps-install`, la bandera para registrar un paquete como dependencia automática es `-A` / `--automatic`. Al no mapearse, el paquete queda registrado permanentemente como explícito en el sistema XBPS.
  3. `PACMAN_FLAGS` (65 banderas) y `PACMAN_GLOBALS` (21 banderas): Listas estáticas heredadas de `paru` en `src/args.rs`. El parser `command_line.rs:275-286` admite banderas como `--nodeps`, `--assume-installed`, `--dbonly`, `--needed`, `--ignore`, `--overwrite`, etc., sin lanzar error, pero ninguna de ellas influye en la resolución ni se pasa a `xbps-install`.
  4. `--dry-run`: No figura en las listas ni en `command_line.rs`, provocando un fallo inmediato (`error: unknown option --dry-run`).
- **Impacto:** Pérdida de integridad de estado en el sistema de paquetes (`--asdeps` instala explícitamente) y desorientación del usuario al creer que banderas críticas como `--nodeps` o `--needed` están operativas.
- **Remediación:** Mapear `--asdeps` a `-A` en la llamada a `xbps-install`; mapear `--needed` al chequeo de versiones instaladas; y para todas las banderas no soportadas de `PACMAN_FLAGS`, emitir un error explícito o una advertencia visible en consola indicando que no son aplicables a Void Linux / `vary`.

---

#### [H-404] Control de verbosidad roto: `-v`, `-vv`, `--verbose` y `log_level` son banderas fantasma por inicialización prematura del logger
- **Severidad:** Medium
- **Archivos:** [src/lib.rs:65-70](file:///run/media/dicov/LudoDrive/dicov-op/Vary/src/lib.rs#L65-L70), [src/logging.rs:29,68-73](file:///run/media/dicov/LudoDrive/dicov-op/Vary/src/logging.rs#L29), [src/command_line.rs:310](file:///run/media/dicov/LudoDrive/dicov-op/Vary/src/command_line.rs#L310), [src/config.rs:100,114,237-239](file:///run/media/dicov/LudoDrive/dicov-op/Vary/src/config.rs#L100)
- **Descripción:**  
  En `src/lib.rs`, el sistema de logs se inicializa antes de parsear la línea de comandos y antes de cargar `vary.conf`:
  ```rust
  // src/lib.rs:65-69
  let _guard = {
      let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
      let cache = home.join(".cache").join("vary");
      logging::init(&cache, 0)
  };
  ```
  Nótese que el parámetro `verbose` se fija de forma estática en `0`.  
  En `src/logging.rs`, la función `init()` utiliza un guardián de una sola ejecución (`std::sync::Once`):
  ```rust
  // src/logging.rs:29, 68-73
  static INIT: Once = Once::new();
  ...
  INIT.call_once(|| {
      let console_filter = EnvFilter::try_new(&console_spec)
          .unwrap_or_else(|_| EnvFilter::new(console_default));
      ...
  });
  ```
  Cuando `run2()` parsea la línea de comandos en `src/command_line.rs:310`:
  ```rust
  Arg::Long("verbose") | Arg::Short('v') => self.verbose = self.verbose.saturating_add(1),
  ```
  el valor de `config.verbose` cambia en la estructura, pero **el suscriptor global de `tracing` ya ha sido bloqueado irreversiblemente por `INIT.call_once`**. De igual forma, `config.log_level` leído de `vary.conf` jamás se envía a `logging::init()`.
- **Impacto:** Las opciones documentadas en `help.rs` (`-v`, `-vv`, `--verbose`) y en `vary.conf.example` (`log_level = "debug"`) no tienen absolutamente ningún efecto sobre el nivel de detalle de la salida en consola.
- **Remediación:** Eliminar la inicialización prematura en `lib.rs:68` o permitir la actualización dinámica de filtros mediante `tracing_subscriber::reload::Handle`.

---

### 3.2 Puertas Interactivas y Degradación no-TTY

#### [H-405] Degradación no-TTY tras H-001 produce abortos silenciosos sin diagnósticos ni hints en entornos automatizados
- **Severidad:** High
- **Archivos:** [src/util.rs:9-16,43-50](file:///run/media/dicov/LudoDrive/dicov-op/Vary/src/util.rs#L9-L16), [src/install.rs:295-297](file:///run/media/dicov/LudoDrive/dicov-op/Vary/src/install.rs#L295-L297), [src/upgrade.rs:105-107](file:///run/media/dicov/LudoDrive/dicov-op/Vary/src/upgrade.rs#L105-L107), [src/review.rs:54-58](file:///run/media/dicov/LudoDrive/dicov-op/Vary/src/review.rs#L54-L58)
- **Descripción:**  
  El parche de seguridad `H-001` corrigió la vulnerabilidad de auto-aprobación involuntaria asegurando que ante EOF (`n == 0`), las funciones lectoras devuelvan denegación estricta (`false`):
  ```rust
  // src/util.rs:13-16
  if n == 0 {
      // EOF en stdin (pipe cerrado, /dev/null): denegación estricta
      return false;
  }
  ```
  Sin embargo, en las puertas interactivas del pipeline:
  1. **Prompt de instalación (`src/install.rs:295-297`):**
     ```rust
     if !confirm("Proceed with installation?", config.no_confirm)? {
         return Ok(1);
     }
     ```
     Al ejecutarse en un entorno no interactivo (cron, CI, scripts donde stdin está redirigido a `/dev/null` o tubería) sin `--noconfirm`, `confirm` imprime `Proceed with installation? [Y/n] `, recibe EOF, retorna `false`, y la función retorna inmediatamente `Ok(1)`. **No se emite ningún mensaje de error ni explicación.**
  2. **Prompt de actualización (`src/upgrade.rs:105-107`):**
     ```rust
     if !crate::util::ask(config, "Upgrade VUR packages?", true) {
         return Ok(1);
     }
     ```
     Mismo comportamiento: retorna `Ok(1)` de forma muda.
  3. **Comprobación de TTY incompleta en `review.rs:54`:**
     ```rust
     if !std::io::stdout().is_terminal() {
         println!("{content}");
         return Ok(());
     }
     ```
     Solo se verifica `stdout().is_terminal()`. Si un usuario ejecuta `vary -S pkg < /dev/null` en un terminal donde stdout es TTY pero stdin está cerrado, `review.rs` lanza el paginador interactivo (`bat` / `less -R`), bloqueando el proceso.
- **Impacto:** Ejecuciones desatendidas fallan con código 1 sin informar al usuario de que la causa fue la falta de un terminal interactivo ni instruirle para usar `--noconfirm`.
- **Remediación:** Detectar al inicio si `!std::io::stdin().is_terminal()` en operaciones interactivas que no recibieron `--noconfirm`; si se detecta EOF o no-TTY en un prompt interactivo, abortar con un mensaje explícito:  
  `error: stdin is not a terminal and no confirmation flag was provided. Re-run with --noconfirm.`

---

#### [H-406] Colisión de convenciones Void/Arch: flag `-y` no autoriza prompts y `--yes` es rechazada con error
- **Severidad:** Medium
- **Archivos:** [src/command_line.rs:340-343,370](file:///run/media/dicov/LudoDrive/dicov-op/Vary/src/command_line.rs#L340-L343), [src/help.rs:18,23-24](file:///run/media/dicov/LudoDrive/dicov-op/Vary/src/help.rs#L18)
- **Descripción:**  
  En el ecosistema nativo de Void Linux (`xbps-install`), la bandera `-y` / `--yes` significa responder "sí" a todas las preguntas de confirmación (equivalente a no-confirm). Por el contrario, en Arch Linux (`pacman`/`paru`), `-y` significa `--refresh` (sincronizar bases de datos).  
  En `vary`:
  - `-y` está asignado exclusivamente a `--refresh`:
    ```rust
    // src/command_line.rs:340-343
    Arg::Long("refresh") | Arg::Short('y') => {
        self.args.args.push(crate::args::Arg { key: "y".to_string(), value: None });
        self.args.args.push(crate::args::Arg { key: "refresh".to_string(), value: None });
    }
    ```
  - `--yes` no está reconocido y lanza un error fatal:
    ```text
    $ ./target/debug/vary -S --yes foo
    error: unknown option --yes
    ```
  - Un usuario habitual de Void Linux que intente `vary -S -y <pkg>` descubrirá que `-y` no omite los prompts de confirmación ni el review gate. Solo la bandera `--noconfirm` (estilo pacman) omite las preguntas.
- **Impacto:** Alta fricción cognitiva y errores de usuario en la adopción de `vary` por parte de usuarios nativos de Void Linux acostumbrados a `xbps-install -y`.
- **Remediación:** Soportar formalmente `--yes` como sinónimo de `--noconfirm`. Documentar claramente en la ayuda bilingüe la distinción semántica de `-y` en operaciones de sincronización (`-Syu` vs `-S`).

---

### 3.3 Manejo Visual de Terminal y Spinners

#### [H-407] Ausencia total de spinners (`indicatif`), inundación descontrolada de salida de compiladores y pérdida de logs de subprocesos
- **Severidad:** High
- **Archivos:** [Cargo.toml:109-124](file:///run/media/dicov/LudoDrive/dicov-op/Vary/Cargo.toml#L109-L124), [src/xbps.rs:399-416](file:///run/media/dicov/LudoDrive/dicov-op/Vary/src/xbps.rs#L399-L416), [src/logging.rs:74-76](file:///run/media/dicov/LudoDrive/dicov-op/Vary/src/logging.rs#L74-L76)
- **Descripción:**  
  El requisito contractual A7 establecía la implementación de logging asíncrono con canalización `mpsc` de salidas de subprocesos a `~/.cache/vary/vary.log` y barras de progreso/spinners no bloqueantes mediante la biblioteca `indicatif`.  
  La auditoría constata:
  1. `indicatif` **no está presente** en `Cargo.toml`. La única herramienta visual disponible es `ansiterm = "0.12.2"`, limitada a aplicar códigos de escape de color en strings estáticos.
  2. No existen spinners, barras de progreso ni indicadores de actividad para operaciones que pueden demorar minutos u horas (como `git clone`, descargas de distfiles o compilaciones pesadas).
  3. En `src/xbps.rs:xbps_src()`, la invocación de `./xbps-src` no redirige sus descriptores de archivo:
     ```rust
     // src/xbps.rs:411
     let mut child = cmd.spawn().map_err(|e| spawn_error("./xbps-src", e))?;
     ```
     Al no configurar `stdout` ni `stderr`, el subproceso hereda directamente los canales de la consola del usuario. Si un paquete toma 40 minutos en compilar con GCC/Clang, la terminal se inunda con decenas de miles de líneas de log, haciendo imposible visualizar mensajes previos de `vary`.
  4. La salida de `xbps-src` **no se captura en `vary.log`**. El archivo de log solo almacena los eventos emitidos directamente por macros `tracing::*`. Si una compilación falla, el usuario no dispone de un log persistente para diagnosticar el fallo.
- **Impacto:** Degradación severa de la experiencia de usuario (UI sucia e ilegible) y pérdida crítica de capacidad diagnóstica forense ante fallos de compilación.
- **Remediación:** Incorporar `indicatif` en `Cargo.toml`, canalizar `stdout`/`stderr` de `./xbps-src` mediante tuberías asíncronas hacia el archivo `vary.log` rotado, y mostrar en la consola un spinner limpio con el estado y tiempo transcurrido del paquete en curso.

---

#### [H-408] Riesgo de corrupción de terminal tras interrupción o pánico y falta de desregistro/restauración de paginadores
- **Severidad:** Medium
- **Archivos:** [src/main.rs:50-64](file:///run/media/dicov/LudoDrive/dicov-op/Vary/src/main.rs#L50-L64), [src/review.rs:61-76](file:///run/media/dicov/LudoDrive/dicov-op/Vary/src/review.rs#L61-L76), [src/signal.rs:121-131,167-176](file:///run/media/dicov/LudoDrive/dicov-op/Vary/src/signal.rs#L121-L131)
- **Descripción:**  
  1. **Ausencia de hook de restauración en pánico (`src/main.rs`):**  
     El hook de pánico configurado en `main.rs:51-64` únicamente intercepta errores de "Broken pipe". Para cualquier otro pánico (ej. index out of bounds, unwrap fallido), invoca el hook estándar y finaliza el proceso sin emitir secuencias de restauración de terminal (`\x1b[0m`, `\x1b[?25h`).
  2. **Paginadores huérfanos ante señales (`src/review.rs` y `src/signal.rs`):**  
     En `src/review.rs:61-70`, `prompt_review` ejecuta `bat` o `less -R`. Los paginadores configuran la terminal en modo alternativo (`smcup`) y manipulan el cursor. Sin embargo, el subproceso del paginador **nunca se registra en `crate::signal::register_child`**.  
     Si el usuario envía SIGINT (Ctrl+C) mientras revisa una plantilla, el hilo de señales en `signal.rs:165-185` ejecuta `std::process::exit(128 + sig)`. Como el paginador no figuraba en `CHILDREN`, puede quedar suspendido o morir sin emitir la secuencia de salida (`rmcup`), dejando la terminal del usuario en estado corrupto (sin eco de caracteres, cursor invisible o saltos de línea desalineados).
- **Impacto:** Terminal inutilizada o degradada visualmente tras interrupciones del usuario en la fase de revisión.
- **Remediación:** Registrar el PID del paginador en el subsistema de señales y añadir un hook de salida/pánico que emita incondicionalmente la secuencia de reseteo ANSI a `/dev/tty`.

---

### 3.4 Códigos de Salida y Mensajes con "Hints" Accionables

#### [H-409] Inconsistencia de códigos de salida: colapso en código 1, fuga de `-1` (255) y omisión del código 127
- **Severidad:** Medium
- **Archivos:** [src/lib.rs:78-81,92-95,99-102](file:///run/media/dicov/LudoDrive/dicov-op/Vary/src/lib.rs#L78-L81), [src/xbps.rs:378-380,416](file:///run/media/dicov/LudoDrive/dicov-op/Vary/src/xbps.rs#L378-L380), [src/main.rs:67](file:///run/media/dicov/LudoDrive/dicov-op/Vary/src/main.rs#L67)
- **Descripción:**  
  La auditoría de las rutas de finalización del programa revela inconsistencias en los códigos de retorno:
  1. **Colapso en código 1:** En `src/lib.rs`, toda variante `Err(e)` devuelta por cualquier módulo se mapea uniformemente al entero `1`:
     ```rust
     // src/lib.rs:98-103
     match run2(&mut config, args) {
         Err(err) => {
             print_error(Style::new(), err);
             1
         }
         Ok(ret) => ret,
     }
     ```
     No existe distinción entre:
     - Error de sintaxis en CLI (estándar POSIX suele requerir código 2).
     - Cancelación explícita del usuario ante un prompt.
     - Fallo de permisos de elevación.
     - Paquete inexistente en repositorios.
  2. **Omisión de código 127 (Command Not Found):** Cuando herramientas obligatorias como `git`, `curl` o las utilidades de `xbps` no están presentes en el sistema, `spawn_error()` o `bootstrap.rs` lanzan un `bail!` que finaliza con código 1 en lugar del código estándar de comando no encontrado (`127`).
  3. **Fuga de código `-1` (`exit(255)`):** En `src/xbps.rs:379` y `416`:
     ```rust
     // src/xbps.rs:379
     Ok(status.code().unwrap_or(-1))
     ```
     Si un subproceso hijo es terminado por una señal del sistema operativo, `status.code()` devuelve `None`, haciendo que `status_code` retorne `-1`. En Linux, pasar `-1` a `std::process::exit()` se interpreta como código `255` sin signo, perdiendo la convención POSIX de `128 + señal`.
- **Impacto:** Imposibilidad de construir scripts y automatizaciones fiables que distingan entre errores de entorno (comando no encontrado), errores de parámetros del usuario o fallos de compilación.
- **Remediación:** Implementar una jerarquía tipada de errores (`ExitCode`) que asigne códigos semánticos: `0` para éxito, `2` para errores de argumentos CLI, `127` para ejecutables faltantes en PATH, `128 + sig` para hijos muertos por señal, y `1` para fallos lógicos generales.

---

#### [H-410] Inconsistencias, errores sintácticos en comandos sugeridos y estilo visual degradado en mensajes de error
- **Severidad:** Low
- **Archivos:** [src/bootstrap.rs:54-57](file:///run/media/dicov/LudoDrive/dicov-op/Vary/src/bootstrap.rs#L54-L57), [src/elevate.rs:51-55](file:///run/media/dicov/LudoDrive/dicov-op/Vary/src/elevate.rs#L51-L55), [src/lock.rs:43-47](file:///run/media/dicov/LudoDrive/dicov-op/Vary/src/lock.rs#L43-L47), [src/lib.rs:52-60](file:///run/media/dicov/LudoDrive/dicov-op/Vary/src/lib.rs#L52-L60)
- **Descripción:**  
  1. **Comando erróneo para instalar `git` (`src/bootstrap.rs:54-57`):**
     ```rust
     bail!(
         "git es requerido por vary pero no está instalado.\n\
          Instálalo con: xbps-install git"
     );
     ```
     `xbps-install git` falla inmediatamente en Void Linux porque `xbps-install` requiere privilegios de root (`sudo`) y el flag de sincronización/búsqueda `-S`. El comando exacto y funcional es: `sudo xbps-install -S git`.
  2. **Consejo peligroso y comando incompleto en elevación (`src/elevate.rs:51-55`):**
     ```rust
     anyhow!(
         "no hay herramienta de elevación de privilegios (busqué sudo, doas y run0 en PATH);\n\
          instala una (p. ej. `xbps-install opendoas`) o ejecuta vary como root"
     )
     ```
     Nuevamente omite `sudo` y `-S`. Peor aún, sugiere al usuario *"ejecuta vary como root"*, lo cual colisiona con la política fundamental de seguridad de Void Linux: `xbps-src` **se niega explícitamente a compilar ejecutándose como UID 0**, por lo que seguir dicho consejo resulta en un callejón sin salida.
  3. **Recomendación incorrecta sobre lockfiles (`src/lock.rs:43-47`):**
     ```rust
     anyhow::bail!(
         "otra instancia de vary está en ejecución{}; si no es así, borra {}",
         ...
     );
     ```
     `vary.lock` utiliza bloqueos consultivos del kernel vía `flock(LockExclusiveNonblock)`. Si un proceso anterior murió o paniqueó, el kernel libera el descriptor automáticamente al cerrarse. Decirle al usuario "borra <ruta>" no solo es innecesario, sino que introduciría una condición de carrera si otra instancia legítima está arrancando.
  4. **Pérdida de color en `print_error` (`src/lib.rs:52-60`):**  
     En `src/lib.rs:79, 93, 100`, las llamadas a `print_error` pasan `Style::new()` en vez del estilo configurado `config.color.error` (rojo). Como resultado, la etiqueta `error:` siempre se renderiza en texto plano sin color, degradando la visibilidad de los errores en terminales compatibles.
- **Impacto:** Confusión en usuarios novatos que copian comandos sugeridos que fallan, e inconsistencia visual en la presentación de errores.
- **Remediación:** Corregir todos los comandos sugeridos a su forma válida en Void (`sudo xbps-install -S <pkg>`), eliminar la sugerencia de correr como root, y conectar `config.color.error` a `print_error`.

---

## 4. Plan de Remediación Priorizado (Roadmap SA-4)

### Fase 1: Seguridad e Integridad Inmediata (Prioridad Alta)
1. **Desactivar o Implementar `-p` / `--print`:** Evitar que invocaciones de inspección desencadenen construcciones e instalaciones reales con `sudo` ([H-401]).
2. **Mapear `--asdeps` y rechazar flags incompatibles:** Pasar `-A` a `xbps-install` cuando se reciba `--asdeps` y emitir advertencias explícitas sobre flags no soportadas de `PACMAN_FLAGS` ([H-403]).
3. **Manejo explícito de no-TTY en puertas de confirmación:** Emitir un error diagnóstico antes de abortar en entornos sin terminal interactiva cuando no se especifique `--noconfirm` ([H-405]).

### Fase 2: Robustez y Concurrencia (Prioridad Media)
4. **Conexión de flags de verbosidad y logging:** Reordenar la inicialización en `lib.rs` para que `-v`, `-vv`, `--verbose` y `log_level` en `vary.conf` configuren efectivamente los filtros de `tracing` ([H-404]).
5. **Implementación de `indicatif` y captura de logs (A7):** Añadir spinners no bloqueantes y canalizar la salida de compilación de `xbps_src` hacia `vary.log` rotado ([H-407]).
6. **Estandarización de códigos de salida:** Crear el enum `ExitCode` garantizando códigos semánticos (`0`, `1`, `2`, `127`, `130`, `143`) y eliminando fugas de `-1` ([H-409]).

### Fase 3: Ergonomía y Compatibilidad Void (Prioridad Baja)
7. **Soporte de `--yes`:** Admitir `--yes` como alias de `--noconfirm` para operadores habituados a `xbps-install -y` ([H-406]).
8. **Corrección de hints y estilo visual:** Actualizar los mensajes de ayuda con los comandos exactos de Void Linux y colorear la etiqueta `error:` con `config.color.error` ([H-410]).

---
*Reporte generado por subagente SA-4 — Antigravity Engine.*
