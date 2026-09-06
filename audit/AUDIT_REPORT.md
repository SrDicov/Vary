# AUDIT REPORT — Reporte Consolidado de Auditoría Técnica (`vary`)

**Fecha:** 2026-09-06  
**Proyecto:** `vary` (Void User Repository helper para Void Linux, en Rust)  
**Rama / Commit:** `vary-mvp` @ `a781acd` (incluyendo remediación H-001)  
**Metodología:** Auditoría integral con escepticismo radical mediante 7 subagentes especializados (SA-1 a SA-7), verificación de código ejecutable e inspección de vectores de ataque, concurrencia, persistencia y UX.

---

## 1. Resumen Ejecutivo de Hallazgos

| Severidad | Total Detectados | Corregidos | Pendientes |
|---|:---:|:---:|:---:|
| 🔴 **Critical** | 8 | 8 ([H-001], [H-002], [H-003], [H-004], [H-005], [H-006], [H-007], [H-008]) | 0 |
| 🟠 **High** | 16 | 5 ([H-009], [H-010], [H-011], [H-012], [H-022]) | 11 |
| 🟡 **Medium** | 15 | 0 | 15 |
| 🟢 **Low / Info** | 8 | 0 | 8 |
| **TOTAL** | **47** | **13** | **34** |

---

## 2. Inventario Consolidado de Hallazgos

### 🔴 Severidad: CRITICAL

---

#### [H-001] Severidad: Critical
- **Módulo:** `src/util.rs:32-42`, `src/review.rs:50-57`
- **Título:** `confirm()` y `ask()` en entornos no-TTY tratan EOF en stdin como aprobación, auto-aprobando builds y registros con privilegios escalados
- **Evidencia:**
  ```rust
  let mut line = String::new();
  stdin().read_line(&mut line)?;
  let t = line.trim().to_lowercase();
  Ok(t.is_empty() || t == "y" || t == "yes" || t == "s" || t == "si")
  ```
- **Impacto:** En tuberías, CI o scripts sin TTY, `read_line()` devuelve 0 bytes (EOF). La cadena resultante está vacía, haciendo que `t.is_empty()` evalúe a `true`. El sistema auto-aprueba la compilación de templates no revisados con privilegios de root (vector análogo al incidente "Atomic Arch").
- **Fix propuesto:** Tratar `bytes_read == 0` (EOF) y errores de lectura como denegación estricta (`Ok(false)`). Imprimir template plano en no-TTY sin abrir paginadores.
- **Validación:** `cargo test util::tests` (6 pruebas unitarias con `Cursor::new(b"")`).
- **Estado:** ✅ CORREGIDO (Commit `a781acd`, registrado en `audit/FIX_LOG.md`).

---

#### [H-002] Severidad: Critical
- **Módulo:** `src/keys.rs:22-45`, `src/repo.rs:33-40, 155-195`, `src/vur_client.rs:692`
- **Título:** Path traversal en nombres de repositorios VUR hacia `/etc/xbps.d/` permitiendo sobrescritura arbitraria de archivos de sistema
- **Evidencia:**
  ```rust
  // src/keys.rs:23
  pub fn conf_path_for_repo(repo_name: &str) -> PathBuf {
      PathBuf::from("/etc/xbps.d").join(format!("vary-{}.conf", repo_name))
  }
  ```
- **Impacto:** Si un atacante convence a un usuario de agregar un repo con nombre `../../etc/xbps.d/00-repository-main` o mediante manipulación de URLs como `https://evil.org/foo/../../00-repository-main.git`, el path resultante sobreescribe archivos de configuración críticos del sistema, permitiendo secuestro del gestor de paquetes de Void.
- **Fix propuesto:** Sanitizar estrictamente el nombre del repositorio: rechazar caracteres que no sean `[a-zA-Z0-9._-]`, prohibir secuencias `..` o `/`, y generar un nombre seguro determinista con prefijo fijo y hash (`vary-<nombre_sanitizado>-<sha256_url_prefix>.conf`).
- **Validación:** Test unitario intentando registrar repositorios con `../` y nombres de archivos de sistema oficiales.
- **Estado:** ✅ CORREGIDO (Commit `37391a4`, registrado en `audit/FIX_LOG.md`).

---

#### [H-003] Severidad: Critical
- **Módulo:** `src/repo.rs:101-114`, `src/keys.rs:86-108`, `src/install.rs:364-382`
- **Título:** Ausencia de verificación TOFU ante reemplazo o rotación de claves RSA en repositorios binarios
- **Evidencia:**
  ```rust
  // src/keys.rs:98-100
  let pem_path = pem_path_for_repo(repo_name);
  if !pem_path.exists() {
      // Solo verifica interactividad si pem_path NO existe
  ```
  ```rust
  // src/keys.rs:113
  write_root_file(&pem_path, pem_content, sudo_bin, sudo_flags)?;
  ```
- **Impacto:** Si un repositorio VUP/binario cambia su clave RSA remota en una actualización posterior (ej. secuestro de repositorio o adopción maliciosa al estilo "Atomic Arch"), `vary` sobreescribe la clave pública en `/etc/xbps.d/` sin alertar al usuario ni fallar cerrado, aceptando binarios firmados por un atacante.
- **Fix propuesto:** Si la clave existente en `/var/db/xbps/keys/` o `/etc/xbps.d/` difiere del fingerprint descargado, **fallar cerrado** inmediatamente con error crítico de seguridad y exigir intervención manual explícita (`vary repo re-trust <name>`).
- **Validación:** Test donde un repositorio cambia su clave RSA y se verifica que `vary` aborta con error de seguridad sin modificar el sistema.
- **Estado:** ✅ CORREGIDO (Commit `614485c`, registrado en `audit/FIX_LOG.md`).

---

#### [H-004] Severidad: Critical
- **Módulo:** `src/masterdir.rs:12-140`, `src/install.rs:329-333`
- **Título:** Conflicto de concurrencia y corrupción destructiva en masterdir (`OverlayGuard`): mutación de lower y colisión de índices binpkgs
- **Evidencia:**
  ```rust
  // src/install.rs:329-330
  repo.project_pkg(&md.srcpkgs_dir(), &parent_pkg, explicit)?;
  let res = md.build_pkg(&parent_pkg, &config.sudo_bin, &config.sudo_flags);
  ```
  ```rust
  // src/masterdir.rs:225-230
  std::process::Command::new("cp")
      .args(["-aT", &merged_binpkgs.display().to_string(), &target_binpkgs.display().to_string()])
  ```
- **Impacto:**
  1. `project_pkg` escribe directamente en `void-packages/srcpkgs` (`lower`). Modificar la capa inferior mientras overlays están montados es Comportamiento Indefinido (UB) en Linux OverlayFS y genera colisiones inmediatas entre hilos paralelos.
  2. Al terminar cada build, `cp -aT` sobreescribe `<arch>-repodata` en `target_binpkgs` sin exclusión mutua, destruyendo del catálogo binario los paquetes recién construidos por otros workers.
  3. Si falla el montaje de overlay (sin sudo/fuse), compila directo sobre `lower` sin aislamiento.
- **Fix propuesto:** Adopción de la Opción A aprobada en PC-2: eliminación del OverlayGuard defectuoso, compilación secuencial con delegación a `xbps-src`, paralelismo intra-paquete con `XBPS_MAKEJOBS` (configurable con default `nproc`), pre-fetch en paralelo para todas las fuentes del DAG, y grupos de proceso (`setpgid` + `kill(-pid)`) ante señales.
- **Validación:** `cargo test masterdir::tests`, `cargo test`.
- **Estado:** ✅ CORREGIDO (Commit `2ceab29`, registrado en `audit/FIX_LOG.md`).

---

#### [H-005] Severidad: Critical
- **Módulo:** `src/db.rs:35-37`
- **Título:** Destrucción incondicional de base de datos previa `installed.json` mediante `remove_file` al iniciar
- **Evidencia:**
  ```rust
  // src/db.rs:33-37
  pub fn load(path: impl Into<PathBuf>) -> Result<Self> {
      let path = path.into();
      if path.is_file() {
          let _ = std::fs::remove_file(&path);
      }
      std::fs::create_dir_all(&path)?;
  ```
- **Impacto:** Si un usuario actualiza desde una versión previa de `vary` que usaba `installed.json`, `InstalledDb::load` **elimina permanentemente el archivo JSON** de su disco para crear un directorio LMDB, borrando todo el registro de paquetes comunitarios instalados y su procedencia.
- **Fix propuesto:** Pausa arquitectónica obligatoria (Punto de Control PC-2): decidir entre revertir a `installed.json` atómico o migrar datos desde `installed.json` ANTES de cualquier manipulación de disco, con respaldo garantizado.
- **Validación:** Test de actualización con archivo `installed.json` preexistente verificando preservación total de entradas.
- **Estado:** ✅ CORREGIDO (Commit `473902b`, registrado en `audit/FIX_LOG.md`).

---

#### [H-006] Severidad: Critical
- **Módulo:** `src/reposconf.rs:53-69`, `src/repo.rs:101-115`
- **Título:** Supresión silenciosa de errores sintácticos en `repos.conf` y sobreescritura destructiva en `repo add`
- **Evidencia:**
  ```rust
  // src/repo.rs:101-102
  let mut repos_conf = ReposConf::load(config.repos_conf_path()).unwrap_or_default();
  // ... añade un nuevo repo a repos_conf vacío ...
  repos_conf.save(config.repos_conf_path())?;
  ```
- **Impacto:** Si `repos.conf` contiene cualquier error de sintaxis TOML, `load().unwrap_or_default()` traga el error sin alertar al usuario devolviendo una lista vacía. Al correr `vary repo add`, `save()` sobreescribe el archivo en disco, eliminando de forma irreversible todos los repositorios configurados previamente.
- **Fix propuesto:** `load()` debe propagar errores sintácticos con contexto claro (archivo, línea y columna). `repo add` debe negarse rotundamente a guardar sobre un archivo con errores sintácticos.
- **Validación:** Test intentando agregar un repo cuando `repos.conf` tiene un error tipográfico, verificando que aborta sin sobreescribir el archivo.
- **Estado:** ✅ CORREGIDO (Commit `f1f6e01`, registrado en `audit/FIX_LOG.md`).

---

#### [H-007] Severidad: Critical
- **Módulo:** `src/args.rs:12-13`, `src/command_line.rs:281-286`, `src/lib.rs:144-179`
- **Título:** Flag `--print` / `-p` ejecuta compilación e instalación real en el sistema en vez de dry-run
- **Evidencia:**
  ```rust
  // src/lib.rs:144
  fn handle_sync(config: &mut Config) -> Result<i32> {
      // ...
      if !config.targets.is_empty() {
          return crate::install::install(config); // Ejecuta instalación real aunque -p esté activo
      }
  ```
- **Impacto:** Un usuario o script que ejecuta `vary -Sp <pkg>` esperando únicamente listar las acciones y dependencias que se instalarían (comportamiento estándar de pacman/paru) sufre la ejecución real e irreversible de `xbps-install` y compilación en su máquina.
- **Fix propuesto:** Conectar la bandera `--print` en `handle_sync()` e `install.rs` para que imprima el plan de ejecución topológico (targets, fuentes, elevaciones necesarias) y retorne inmediatamente con código 0 sin alterar el sistema.
- **Validación:** `vary -Sp <pkg>` no invoca comandos de elevación ni `xbps-install`.
- **Estado:** ✅ CORREGIDO (Commit `33ee53d`, registrado en `audit/FIX_LOG.md`).

---

#### [H-008] Severidad: Critical
- **Módulo:** `src/vur_client.rs:592-601`
- **Título:** Bug lógico en parser de templates Bash (`while quote_count % 2 == 1`): condición inmutable y fallo en comillas multilínea
- **Evidencia:**
  ```rust
  // src/vur_client.rs:592-601
  let quote_count = buf.matches('"').count();
  while quote_count % 2 == 1 {
      if let Some(next) = lines.next() {
          buf.push('\n');
          buf.push_str(next);
          if next.contains('"') { break; }
      } else {
          break;
      }
  }
  ```
- **Impacto:** La variable `quote_count` es inmutable y no se recalcula dentro del bucle (`clippy::while_immutable_condition`). Si la siguiente línea contiene comillas internas o escapadas `\"`, el `break` se dispara prematuramente con comillas aún impares o salta líneas incorrectamente, corrompiendo la extracción de dependencias y versiones de paquetes.
- **Fix propuesto:** Reemplazar el bucle por un escaneo dinámico que recalcule la paridad ignorando comillas escapadas, o implementar un tokenizador estricto línea por línea.
- **Validación:** Test unitario con variables multilínea complejas con comillas internas escapadas y comillas simples.
- **Estado:** ✅ CORREGIDO (Commit `2ccae8b`, registrado en `audit/FIX_LOG.md`).

---

### 🟠 Severidad: HIGH

---

#### [H-009] Severidad: High
- **Módulo:** `src/xbps.rs:351-355`, `src/xbps.rs:367-371`, `src/install.rs:81`, `src/info.rs:23`
- **Título:** Inyección de argumentos en `xbps-install`, `xbps-remove` y `xbps-query` por omisión de `--`
- **Evidencia:**
  ```rust
  // src/xbps.rs:351-354
  let mut cmd = crate::elevate::elevate(sudo_bin, sudo_flags, "xbps-install")?;
  cmd.arg("-S").args(extra_flags).args(targets);
  ```
- **Impacto:** Si un paquete o dependencia comienza con `-` (ej. `-y`, `--repository=...`), `xbps-install` lo interpreta como una bandera en vez de un nombre de paquete, alterando la semántica de la transacción transaccional.
- **Fix propuesto:** Agregar explícitamente el separador `--` antes de `targets`: `cmd.arg("-S").args(extra_flags).arg("--").args(targets);`.
- **Validación:** Test unitario verificando que el comando generado incluya `--`.
- **Estado:** ✅ CORREGIDO Y VALIDADO (Commit `b630f1a`)

---

#### [H-010] Severidad: High
- **Módulo:** `src/command_line.rs:215-217`, `src/metadata.rs:101-124`
- **Título:** Ausencia de validación con regex estricta en nombres de paquetes CLI y omisión en subpaquetes
- **Evidencia:**
  ```rust
  // src/command_line.rs:215
  targets.push(arg.to_string()); // Se aceptan nombres arbitrarios sin sanitizar
  ```
- **Impacto:** Nombres de paquete maliciosos con caracteres especiales de control o secuencias de escape pueden inyectar argumentos en subprocesos o corromper rutas de archivo locales.
- **Fix propuesto:** Validar todos los nombres de targets con la regex estricta `^[a-zA-Z0-9][a-zA-Z0-9._+-]*$`, y aplicar la misma validación a `subpackages[i].pkgname` en `metadata.rs`.
- **Validación:** Tests rechazando nombres con caracteres ilegales (espacios, saltos de línea, barras, caracteres de escape).
- **Estado:** ✅ CORREGIDO Y VALIDADO (Commit `48eb719`)

---

#### [H-011] Severidad: High
- **Módulo:** `src/keys.rs:117, 185-189`
- **Título:** Inyección de directivas XBPS arbitrarias por falta de sanitización de newlines y esquemas inseguros en URLs
- **Evidencia:**
  ```rust
  // src/keys.rs:188
  for url in urls {
      content.push_str(&format!("repository={}\n", url));
  }
  ```
- **Impacto:** Si una URL binaria de un repositorio VUP contiene saltos de línea (`\n`), puede inyectar directivas arbitrarias de configuración en los archivos generados bajo `/etc/xbps.d/`.
- **Fix propuesto:** Validar que las URLs utilicen exclusivamente esquemas seguros (`https://` o `file://`) y no contengan caracteres de control ni saltos de línea.
- **Validación:** Test unitario intentando registrar repositorios con URLs manipuladas.
- **Estado:** ✅ CORREGIDO Y VALIDADO (Commit `b65acb8`)

---

#### [H-012] Severidad: High
- **Módulo:** `src/vur_client.rs:62-73, 91-95`
- **Título:** Inyección de argumentos y ejecución arbitraria mediante transportes peligrosos en `git clone` y `git ls-remote`
- **Evidencia:**
  ```rust
  // src/vur_client.rs:62-72
  Command::new(&self.git_bin)
      .args(["clone", "--filter=blob:none", "--no-checkout", "--depth", "1", "--branch", branch])
      .arg(&self.entry.url)
      .arg(&self.path)
  ```
- **Impacto:** Si `self.entry.url` comienza con `-` (ej. `--upload-pack=evil`), Git ejecuta el binario local especificado. Además, transportes maliciosos en submódulos o URLs tipo `ext::` pueden ejecutar comandos arbitrarios de shell.
- **Fix propuesto:** Validar esquema de URL (`https://`, `git://`, `ssh://`), pasar `-c protocol.ext.allow=never -c protocol.file.allow=user`, y colocar `--` antes de `&self.entry.url`.
- **Validación:** Test pasando URLs con flags a `git_clone` y verificando rechazo seguro.
- **Estado:** ✅ CORREGIDO Y VALIDADO (Commit: pendiente de commit H-012)

---

#### [H-013] Severidad: High
- **Módulo:** `src/keys.rs:37-41`
- **Título:** Condición de carrera (TOCTOU) y archivo temporal predecible `/tmp/vary-<PID>.tmp` en `write_root_file`
- **Evidencia:**
  ```rust
  // src/keys.rs:37
  let tmp = PathBuf::from(format!("/tmp/vary-{}.tmp", std::process::id()));
  std::fs::write(&tmp, content)?;
  ```
- **Impacto:** Emplea una ruta estática predecible en `/tmp` con permisos mundiales (la vulnerabilidad clásica de `vura`). Un atacante local puede crear un enlace simbólico previo a un archivo sensible de root antes de que `vary` invoque el comando elevado `mv`, sobrescribiendo archivos del sistema.
- **Fix propuesto:** Usar `tempfile::NamedTempFile` en un directorio privado con permisos `0700` dentro de `$XDG_CACHE_HOME/vary` o `tempfile::Builder` en `/tmp` con nombre aleatorio criptográfico y `O_EXCL`.
- **Validación:** Test verificando creación con permisos `0600` sin nombres predecibles en `/tmp`.
- **Estado:** PENDIENTE

---

#### [H-014] Severidad: High
- **Módulo:** `src/init.rs:15-25`
- **Título:** Bypass del wrapper agnóstico de elevación y ejecución descontrolada con hardcode de `Command::new("sudo")`
- **Evidencia:**
  ```rust
  // src/init.rs:15-16
  let _ = Command::new("sudo").args(["dinitctl", "enable", pkg_name]).status();
  let _ = Command::new("sudo").args(["dinitctl", "start", pkg_name]).status();
  ```
- **Impacto:** En sistemas que utilizan `doas` o `run0` (o cuando vary corre como root), el post-install hook falla silenciosamente o intenta invocar un binario `sudo` inexistente, rompiendo la configuración de servicios.
- **Fix propuesto:** Conectar `src/init.rs` con `crate::elevate::elevate(sudo_bin, sudo_flags, ...)`.
- **Validación:** Test ejecutando hooks con wrapper no-sudo.
- **Estado:** PENDIENTE

---

#### [H-015] Severidad: High
- **Módulo:** `src/install.rs:301-341`, `src/config.rs:115, 203`
- **Título:** Incumplimiento contractual R1: scheduler de builds puramente secuencial y opción fantasma `max_concurrent_builds`
- **Evidencia:**
  ```rust
  // src/install.rs:301
  for item in &plan.builds {
      // Bucle secuencial for que procesa Action::Build uno por uno
      let res = md.build_pkg(&parent_pkg, &config.sudo_bin, &config.sudo_flags);
  ```
- **Impacto:** El sistema no aprovecha el hardware multinúcleo para compilar ramas no interdependientes del DAG, incumpliendo el requisito R1 del contrato.
- **Fix propuesto:** Diseñar el scheduler topológico por niveles con `JoinHandle` y semáforo de concurrencia una vez resuelto el aislamiento de masterdirs en PC-2.
- **Validación:** Test de compilación concurrente de grafos independientes.
- **Estado:** DIFERIDO A P1-3 (DECISIÓN PC-2)

---

#### [H-016] Severidad: High
- **Módulo:** `src/signal.rs:167-176`, `src/xbps.rs:398-417`
- **Título:** Carreras TOCTOU en señales (PGID desacoplado) y fuga permanente de directorios temporales `/tmp/vary-*`
- **Evidencia:**
  ```rust
  // src/xbps.rs:411-413
  let mut child = cmd.spawn()?;
  let pid = child.id();
  crate::signal::register_child(pid); // Ventana de carrera si SIGINT ocurre entre spawn y register
  ```
  ```rust
  // src/signal.rs:175
  std::process::exit(128 + sig); // Omite el Drop del hilo principal, dejando TempDirs huérfanos en /tmp
  ```
- **Impacto:** Si el proceso recibe `SIGINT` inmediatamente después del `spawn`, el hijo queda en su propio Process Group huérfano sin ser terminado. Además, `exit()` aborta el proceso sin ejecutar destructores RAII, acumulando basura en `/tmp`.
- **Fix propuesto:** Registrar el Process Group de forma atómica o usar una máscara de señales durante el spawn. Asegurar la limpieza de los `TempDir` en el manejador de salida.
- **Validación:** Simular SIGINT concurrente al spawn y verificar que no queden procesos ni directorios residuales.
- **Estado:** PENDIENTE

---

#### [H-017] Severidad: High
- **Módulo:** `src/vur_client.rs:569-577, 674`
- **Título:** Omisión total de subpaquetes y sobreescritura destructiva de variables padre por subpaquetes en parser shell
- **Evidencia:**
  ```rust
  // src/vur_client.rs:674
  subpackages: vec![], // Subpaquetes hardcodeados a vacío
  ```
- **Impacto:** Las funciones `<subpkg>_package()` no son detectadas como funciones de subpaquetes y sus asignaciones de variables sobreescriben las del paquete padre en el HashMap `vars`. Los subpaquetes requeridos por dependencias se vuelven completamente invisibles.
- **Fix propuesto:** Extraer bloques `<subpkg>_package()` identificando el nombre del subpaquete y asignando sus variables a estructuras `VurSubpackage`.
- **Validación:** Test parseando templates con subpaquetes como `foo-devel` o `foo-doc`.
- **Estado:** PENDIENTE

---

#### [H-018] Severidad: High
- **Módulo:** `src/vur_client.rs:333, 346, 365, 418`
- **Título:** Supresión silenciosa de errores con `if let Ok(...)` en lectura y proyección de plantillas VUR
- **Evidencia:**
  ```rust
  // src/vur_client.rs:418
  if let Ok(info) = parse_template_text(&text, p.join("template").to_string_lossy().as_ref()) {
  ```
- **Impacto:** Si una plantilla tiene un error de sintaxis o formato, el sistema lo descarta en silencio y reporta que el paquete "no existe en el repositorio", ocultando la causa raíz del fallo al usuario.
- **Fix propuesto:** Reportar advertencias descriptivas visibles en consola cuando una plantilla falle en parsear.
- **Validación:** Test con template con sintaxis rota verificando que emite un warning claro.
- **Estado:** PENDIENTE

---

#### [H-019] Severidad: High
- **Módulo:** `src/install.rs:295-297`, `src/util.rs:41-53`
- **Título:** Falta de mensaje de error explicativo y hint accionable al denegar en no-TTY por EOF
- **Evidencia:**
  ```rust
  // src/install.rs:295-297
  if !confirm("Proceed with installation?", config.no_confirm)? {
      return Ok(1); // Aborta silenciosamente sin explicar por qué falló
  }
  ```
- **Impacto:** Tras la remediación de seguridad H-001, las invocaciones en pipes o CI son denegadas correctamente, pero terminan silenciosamente con código 1 sin indicar que se requiere `--noconfirm`.
- **Fix propuesto:** Emitir mensaje visible: `"Error: stdin reached EOF without confirmation. In non-interactive environments (CI/pipes), use --noconfirm."`.
- **Validación:** Ejecución en `/dev/null` verificando la emisión del mensaje orientativo.
- **Estado:** PENDIENTE

---

#### [H-020] Severidad: High
- **Módulo:** `src/xbps.rs:399-416`, `Cargo.toml`
- **Título:** Ausencia de canalización de logs y spinners (`indicatif`), inundando la consola (A7)
- **Evidencia:**
  ```rust
  // src/xbps.rs:399
  let mut cmd = Command::new("./xbps-src"); // Hereda stdio crudo hacia la consola
  ```
- **Impacto:** Miles de líneas de salida de compiladores ensucian la terminal del usuario, y la salida no se registra en `vary.log`.
- **Fix propuesto:** Agregar `indicatif`, canalizar stdout/stderr de `xbps_src` hacia `vary.log` mediante hilos/mpsc, y mostrar spinner elegante en la terminal.
- **Validación:** Compilación de prueba verificando terminal limpia con spinner y log completo en disco.
- **Estado:** PENDIENTE

---

#### [H-021] Severidad: High
- **Módulo:** `src/config.rs:175-179`
- **Título:** Violación de la especificación XDG Base Directory por hardcodeo de rutas `$HOME/.config`, `$HOME/.cache`
- **Evidencia:**
  ```rust
  // src/config.rs:175-179
  let config_dir = home.join(".config").join("vary");
  let cache_dir = home.join(".cache").join("vary");
  let vurs_dir = home.join(".local").join("share").join("vary").join("vurs");
  ```
- **Impacto:** Ignora las variables de entorno estándar `XDG_CONFIG_HOME`, `XDG_CACHE_HOME` y `XDG_DATA_HOME`, violando las directrices de empaquetado de distribuciones Linux.
- **Fix propuesto:** Utilizar las funciones del crate `dirs`: `dirs::config_dir()`, `dirs::cache_dir()`, `dirs::data_dir()`.
- **Validación:** Test configurando `XDG_CONFIG_HOME=/tmp/custom_config` y verificando que vary lo respeta.
- **Estado:** PENDIENTE

---

#### [H-022] Severidad: High
- **Módulo:** Múltiples módulos en `src/`
- **Título:** 57 errores de compilación reportados por `cargo clippy --all-targets -- -D warnings`
- **Evidencia:** Salida de FASE 0: 57 errores de clippy bloqueando el build bajo `-D warnings`.
- **Impacto:** Código no idiomático, malas prácticas de Rust, castings redundantes y bugs latentes de paridad.
- **Fix propuesto:** Remediación integral de las 15 categorías de lints de clippy sin usar `#[allow]` cosméticos.
- **Validación:** `cargo clippy --all-targets -- -D warnings` 100% limpio.
- **Estado:** ✅ CORREGIDO Y VALIDADO (Commit `8612906`)

---

#### [H-023] Severidad: High
- **Módulo:** `src/install.rs:367`, `src/config.rs:126`, `src/init.rs:24`, `src/main.rs:37-39`
- **Título:** Pánicos potenciales por `unwrap()` y `expect()` en rutas alcanzables por el usuario
- **Evidencia:**
  ```rust
  // src/install.rs:367
  let entry = repos_conf.vur.get(repo).unwrap(); // Pánico si el repo no está en la configuración
  ```
- **Impacto:** Crash directo del binario si se presenta una inconsistencia en nombres de repositorio o codificación de caracteres.
- **Fix propuesto:** Reemplazar `unwrap()` por propagación con `ok_or_else(|| anyhow!("..."))` con contexto.
- **Validación:** `cargo test` y fuzzing de entradas.
- **Estado:** PENDIENTE

---

#### [H-024] Severidad: High
- **Módulo:** `db.rs`, `cache.rs`, `search.rs`, `upgrade.rs`, `install.rs`, `repo.rs`, `remove.rs`, `config.rs`, `init.rs`, `review.rs`
- **Título:** Vacío crítico de cobertura de pruebas unitarias en 10 módulos centrales del sistema
- **Evidencia:** Cero tests en `src/install.rs`, `src/config.rs`, `src/upgrade.rs`, `src/db.rs`, etc.
- **Impacto:** Alto riesgo de regresiones funcionales no detectadas durante las fases de remediación.
- **Fix propuesto:** Desarrollar pruebas unitarias sistemáticas para cada uno de estos módulos.
- **Validación:** Incremento de cobertura y tests automatizados en verde.
- **Estado:** PENDIENTE

---

### 🟡 Severidad: MEDIUM

---

#### [H-025] Severidad: Medium
- **Módulo:** `src/cache.rs:31`, `src/db.rs:38`, `src/lock.rs:21`, `src/logging.rs:52`
- **Título:** Creación de directorios de caché, logs, lock y DB sin permisos Unix restringidos `0700`
- **Evidencia:** `std::fs::create_dir_all(&path)` utiliza la umask heredada del usuario, pudiendo crear directorios con permisos mundiales de lectura en entornos multiusuario.
- **Fix propuesto:** Usar `std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700)`.
- **Estado:** PENDIENTE

---

#### [H-026] Severidad: Medium
- **Módulo:** `src/command_line.rs:313-326`
- **Título:** Pánico incontrolado (`panic!`) en el analizador de argumentos ante banderas sin valor al final de `argv`
- **Evidencia:**
  ```rust
  // src/command_line.rs:317
  let val = args[i + 1].clone(); // Panic por out of bounds si --sudo es el último argumento
  ```
- **Impacto:** `vary -S pkg --sudo` produce un crash con panic en vez de un mensaje de error elegante.
- **Fix propuesto:** Usar `args.get(i + 1).ok_or_else(...)` con mensaje amigable.
- **Estado:** PENDIENTE

---

#### [H-027] Severidad: Medium
- **Módulo:** `src/lib.rs:90`, `src/lock.rs:43-47`
- **Título:** Lockfile adquirido prematuramente bloqueando comandos de solo lectura y mensaje inductor de split-brain
- **Evidencia:** `lock::acquire` se llama antes de evaluar si el comando es de solo lectura (`--help`, `--version`, `-Ss`), y el mensaje sugiere borrar el lockfile manualmente (lo que destruye la semántica de `flock` e induce split-brain).
- **Fix propuesto:** Adquirir el lockfile únicamente para operaciones mutantes (`install`, `upgrade`, `remove`, `repo add/remove`) y corregir el mensaje eliminando el consejo de borrar el archivo.
- **Estado:** PENDIENTE

---

#### [H-028] Severidad: Medium
- **Módulo:** `src/logging.rs:29, 68-73`, `src/lib.rs:65-70`
- **Título:** Inicialización inmutable del logger (`Once`), desconexión de `-v`/`--verbose` y pérdida de logs en `exit()`
- **Evidencia:** `logging::init` se ejecuta antes de parsear la CLI con nivel fijo 0 y no puede reconfigurarse. Además, el `WorkerGuard` no hace flush si se llama a `process::exit()`.
- **Fix propuesto:** Inicializar el logger tras parsear CLI/TOML y asegurar el flush de `WorkerGuard`.
- **Estado:** PENDIENTE

---

#### [H-029] Severidad: Medium
- **Módulo:** `src/db.rs:18-24`, `src/upgrade.rs:90-97`
- **Título:** Campo fantasma `install_date: u64`, ausencia contractual de `build_date` ms y cero recompilación preventiva (A5)
- **Evidencia:** `install_date` solo se escribe en `upsert()` y jamás se lee. No existe `build_date` en milisegundos ni lógica para recompilar ante cambios en librerías compartidas.
- **Fix propuesto:** Incorporar `build_date` (ms con `time`) y conectar la verificación en `upgrade.rs`.
- **Estado:** PENDIENTE (Sujeto a PC-2)

---

#### [H-030] Severidad: Medium
- **Módulo:** `src/xbps.rs:275`, `src/install.rs:229`
- **Título:** Consulta masiva con comodín erróneo `""` en vez de `'*'` y ejecución previa a la sincronización de repositorios (A4)
- **Evidencia:** Pasa cadena vacía `""` a `xbps-query -Rs` y consulta antes de invocar `xbps-install -S`, arriesgando metadatos obsoletos.
- **Fix propuesto:** Cambiar `""` por `'*'` y ordenar el sync antes de la consulta masiva.
- **Estado:** PENDIENTE

---

#### [H-031] Severidad: Medium
- **Módulo:** `src/metadata.rs:44-45, 84-100`
- **Título:** Inconsistencia de validación de checksum vacío en `metadata.rs` frente a la especificación VURINFO v1
- **Evidencia:** La especificación exige rechazar checksums vacíos, pero el código emite un warning y lo acepta.
- **Fix propuesto:** Alinear código con especificación o documentar la excepción justificada.
- **Estado:** PENDIENTE

---

#### [H-032] Severidad: Medium
- **Módulo:** `src/command_line.rs:330, 352-357`
- **Título:** Banderas heredadas de pacman/paru aceptadas sin error e ignoradas en silencio (`--asdeps`, etc.)
- **Evidencia:** `--asdeps` se parsea pero no añade `-A` a `xbps-install`.
- **Fix propuesto:** Conectar `--asdeps` pasando `-A` o emitir advertencia clara de flag no soportada.
- **Estado:** PENDIENTE

---

#### [H-033] Severidad: Medium
- **Módulo:** `src/command_line.rs:340-343`
- **Título:** Inconsistencia en flags de confirmación: `-y` interpretado como refresh en vez de yes
- **Evidencia:** `-y` colisiona con el alias de `--refresh` de paru/pacman.
- **Fix propuesto:** Soportar `--yes` como alias explícito de `--noconfirm`.
- **Estado:** PENDIENTE

---

#### [H-034] Severidad: Medium
- **Módulo:** `src/main.rs:50-64`, `src/signal.rs:167-176`
- **Título:** Falta de restauración del modo de terminal tras pánico o interrupción
- **Evidencia:** Si el usuario interrumpe mientras corre un paginador o spinner, el cursor puede quedar oculto.
- **Fix propuesto:** Instalar hook de pánico que restaure el cursor y modo canónico de terminal.
- **Estado:** PENDIENTE

---

#### [H-035] Severidad: Medium
- **Módulo:** `src/lib.rs:78-102`
- **Título:** Inconsistencia en códigos de salida (colapso en 1, fuga de -1)
- **Evidencia:** Errores de comando no encontrado deben retornar 127, y subprocesos terminados por señal deben mapear a `128 + sig` en lugar de -1.
- **Fix propuesto:** Estandarizar códigos de salida POSIX consistentes.
- **Estado:** PENDIENTE

---

#### [H-036] Severidad: Medium
- **Módulo:** `src/command_line.rs:324`, `src/config.rs:243-245`
- **Título:** Flag CLI `--sudoflags` concatena en vez de sobrescribir flags de `vary.conf`
- **Evidencia:** `config.sudo_flags.extend(...)` acumula flags en vez de reemplazarlas.
- **Fix propuesto:** Reemplazar el vector de flags cuando se especifica en CLI.
- **Estado:** PENDIENTE

---

#### [H-037] Severidad: Medium
- **Módulo:** `src/config.rs:124-128`
- **Título:** `Config::default()` efectúa I/O al disco del host y causa pánico si `$HOME` no existe
- **Evidencia:** `Config::default()` invoca `Self::new().expect(...)`.
- **Fix propuesto:** Implementar un `Default` puramente en memoria sin I/O ni pánicos.
- **Estado:** PENDIENTE

---

#### [H-038] Severidad: Medium
- **Módulo:** `src/main.rs:37-39`
- **Título:** Abandono inseguro de privilegios usando `expect()` en vez de propagación de error
- **Evidencia:** Pánico si falla `setresuid`.
- **Fix propuesto:** Manejar el error y abortar limpiamente.
- **Estado:** PENDIENTE

---

#### [H-039] Severidad: Medium
- **Módulo:** `Cargo.toml`
- **Título:** Dependencias contractuales ausentes (`diffy = "0.4"`, `indicatif`) para A3 y A7
- **Evidencia:** No están presentes en `dependencies`.
- **Fix propuesto:** Añadirlas a `Cargo.toml`.
- **Estado:** PENDIENTE

---

### 🟢 Severidad: LOW / INFO

---

#### [H-040] Severidad: Low
- **Módulo:** `src/elevate.rs:60-86`
- **Título:** Falta de sanitización de entorno en elevación de privilegios
- **Estado:** PENDIENTE

#### [H-041] Severidad: Low
- **Módulo:** `src/cache.rs:19`, `src/db.rs:27, 51, 68, 83`
- **Título:** Campos muertos (`path`) y métodos no usados en base de datos y caché
- **Estado:** PENDIENTE

#### [H-042] Severidad: Low
- **Módulo:** `src/bootstrap.rs:54-57`, `src/elevate.rs:51-55`
- **Título:** Hints con comandos inexactos (`xbps-install git` sin sudo ni -S)
- **Estado:** PENDIENTE

#### [H-043] Severidad: Low
- **Módulo:** `src/command_line.rs:303-392`
- **Título:** Banderas `--config` y `--cachedir` tratadas como paquetes posicionales
- **Estado:** PENDIENTE

#### [H-044] Severidad: Low
- **Módulo:** `src/config.rs:1-298`
- **Título:** Cero pruebas unitarias de carga TOML y precedencia en `config.rs`
- **Estado:** PENDIENTE

#### [H-045] Severidad: Low
- **Módulo:** `src/masterdir.rs:73, 225`, `src/signal.rs:62, 65`
- **Título:** Acoplamiento rígido a binarios auxiliares (`cp`, `fuse-overlayfs`, `umount`)
- **Estado:** PENDIENTE

#### [H-046] Severidad: Low
- **Módulo:** `src/install.rs:334, 422`
- **Título:** Paquetes resueltos por `provides` utilizan nombre virtual en el plan de construcción
- **Estado:** PENDIENTE

#### [H-047] Severidad: Info
- **Módulo:** `/run/media/dicov/LudoDrive/dicov-op/Vary/guia`
- **Título:** Archivo de texto residual `guia` (29 KB) en la raíz del repositorio
- **Estado:** PENDIENTE
