# FIX LOG — Registro de Remediaciones

Este documento registra cronológicamente cada corrección atómica realizada sobre el código de `vary`, vinculada a su hallazgo, commit y validación.

---

### [H-001] confirm() / ask() trata EOF en stdin como denegación, no como aprobación
- **Severidad:** Critical
- **Módulo:** `src/util.rs:32-42`, `src/review.rs:50-57`
- **Commit:** `a781acd` (`fix(H-001): confirm() treats EOF as denial, not approval`)
- **Descripción del problema:** En entornos no-TTY (pipes, CI, `/dev/null`), `stdin().read_line()` devolvía EOF (`Ok(0)`). La línea vacía resultante provocaba que `t.is_empty()` evaluara a `true`, auto-aprobando compilaciones con privilegios escalados y registros de repositorios binarios sin consentimiento del usuario (vector A8-Critical).
- **Remediación:**
  1. Se implementó `confirm_from_reader` y `ask_from_reader` verificando `n == 0` (EOF) y errores de I/O para devolver denegación estricta (`Ok(false)` / `false`).
  2. En `src/review.rs`, se agregó verificación `std::io::stdout().is_terminal()`; si no es terminal, imprime el template plano sin bloquear paginadores interactivos (`bat`/`less`).
- **Validación:**
  - `cargo test util::tests` (6 tests unitarios verificando EOF y saltos de línea).
  - `cargo test` (91 tests pasando).
- **Estado:** ✅ CORREGIDO Y VALIDADO

---

### [H-002] Path traversal en nombres de repositorios VUR en /etc/xbps.d/
- **Severidad:** Critical
- **Módulo:** `src/keys.rs:22-45`, `src/repo.rs:33-40, 155-195`, `src/vur_client.rs:692`
- **Commit:** `37391a4` (`fix(H-002): prevent path traversal in /etc/xbps.d with repo name validation`)
- **Descripción del problema:** `repo_conf_path()` y `key_dest_path()` concatenaban directamente el nombre del repositorio a `/etc/xbps.d/20-vur-{}.conf` y `/etc/xbps.d/keys/vary-vur-{}.pem` sin sanitización. Un nombre con `../` o secuencias de escape permitía a un repositorio malicioso sobreescribir archivos críticos del sistema como `/etc/xbps.d/00-repository-main.conf` al invocar `write_root_file` con privilegios de root.
- **Remediación:**
  1. Se implementó `validate_repo_name()` en `src/keys.rs` exigiendo `^[a-zA-Z0-9][a-zA-Z0-9._-]*$`, prohibiendo `..`, `/`, `\`, caracteres nulos y nombres de sistema reservados (`00-*`, `keys`).
  2. `repo_conf_path()` y `key_dest_path()` ahora invocan `validate_repo_name()` antes de construir las rutas.
  3. `repo_add`, `repo_remove` y `repo_rekey` en `src/repo.rs` validan estrictamente el nombre del repositorio.
- **Validación:**
  - `keys::tests::rechaza_path_traversal_en_nombres_de_repo` (test unitario con `../evil`, `foo/bar`, `..`, `00-repository-main`, `-bad`, `""`).
  - `cargo test` (92 tests pasando).
- **Estado:** ✅ CORREGIDO Y VALIDADO

---

### [H-003] Aceptación silenciosa de llaves públicas cambiadas en repositorios binarios (falta de TOFU fail-closed)
- **Severidad:** Critical
- **Módulo:** `src/keys.rs:141-175, 225-275`
- **Commit:** `614485c` (`fix(H-003): enforce fail-closed TOFU on public key changes`)
- **Descripción del problema:** Durante sincronizaciones o reinstalaciones de repositorios binarios VUR/VUP, si la clave remota del repositorio rotaba o cambiaba respecto a la instalada previamente en `/etc/xbps.d/keys/vary-vur-<repo>.pem`, `setup_binary_repo` y `setup_vup_binary_repo` sobrescribían la clave existente sin verificar si coincidía con la ya confiada (incidente Atomic Arch).
- **Remediación:**
  1. Se implementó `verify_key_tofu()` en `src/keys.rs` que compara el digest SHA256 de la clave DER en disco contra la recibida remotamente.
  2. Si existe discrepancia de fingerprints, la operación aborta con error fatal (fallo cerrado), impidiendo cualquier sobreescritura y alertando de potencial suplantación, indicando el comando explícito `vary --repo rekey <repo>` para rotación manual verificada.
  3. Si la clave no existía previamente en disco, procede el flujo TOFU inicial pidiendo confirmación al usuario.
- **Validación:**
  - `keys::tests::verify_key_tofu_bloquea_rotacion_no_confiable` (test unitario con claves PEM distintas verificando el error de seguridad y el mensaje orientativo).
  - `cargo test` (93 tests pasando).
- **Estado:** ✅ CORREGIDO Y VALIDADO

---

### [H-004] Conflicto de concurrencia y corrupción destructiva en masterdir (OverlayGuard): mutación de lower y colisión de índices binpkgs
- **Severidad:** Critical
- **Módulo:** `src/masterdir.rs:1-125`, `src/config.rs:113-255`, `src/xbps.rs:388-420`, `src/install.rs:280-340`
- **Commit:** `2ceab29` (`fix(H-004): sanitize masterdir execution, configure makejobs and add parallel fetch`)
- **Descripción del problema:** La implementación de `OverlayGuard` presentaba fallas estructurales: mutaba `lower/srcpkgs` proyectando paquetes mientras el overlay estaba activo (comportamiento indefinido en el kernel de Linux OverlayFS), requería elevación de privilegios para montar/desmontar en cada compilación, y builds concurrentes sobreescribían `binpkgs` con `cp -aT` colisionando en el archivo de índice `<arch>-repodata`.
- **Remediación:**
  1. Se adoptó formalmente la **Opción A** aprobada en el checkpoint PC-2.
  2. Se eliminó por completo `OverlayGuard` de `src/masterdir.rs`. La compilación se delega directamente a `xbps-src`, el cual ya gestiona su propio aislamiento chroot/namespace de forma nativa.
  3. Se mantuvo la compilación secuencial entre paquetes del DAG, eliminando cualquier colisión sobre `hostdir/binpkgs` o repodata.
  4. Se implementó pre-fetch de fuentes en paralelo para todo el DAG antes de compilar (`std::thread::scope`, con un pool de trabajadores delimitado por `makejobs`).
  5. Se implementó soporte configurable para `XBPS_MAKEJOBS` en `src/config.rs` (`[build] makejobs = N`), con default dinámico a `nproc` (`std::thread::available_parallelism()`).
  6. Se integró el aislamiento de grupo de procesos (`setpgid` + `kill(-pid)`) en `src/signal.rs` y `src/xbps.rs` para abortar limpiamente compiladores subordinados (`make`/`ninja`) ante SIGINT/SIGTERM.
- **Validación:**
  - `masterdir::tests::rutas_derivadas_son_consistentes`
  - `masterdir::tests::build_pkg_falla_si_no_existe_xbps_src`
  - `cargo test` (93 tests pasando, 4 ignorados).
- **Estado:** ✅ CORREGIDO Y VALIDADO

---

### [H-005] Destrucción incondicional de base de datos previa installed.json mediante remove_file al iniciar
- **Severidad:** Critical
- **Módulo:** `src/db.rs:1-240`, `src/cache.rs:1-85`, `Cargo.toml:35`
- **Commit:** `473902b` (`fix(H-005): revert to atomic installed.json with safe migration and drop heed`)
- **Descripción del problema:** La migración arbitraria hacia LMDB (`heed`) borraba `installed.json` si era un archivo llamando incondicionalmente a `remove_file(&path)` para convertir la ruta en un directorio, destruyendo el catálogo histórico de paquetes comunitarios instalados. Además, se omitía `build_date` (ms) y la dependencia C `heed` agregaba sobrecarga de compilación y memoria virtual innecesaria.
- **Remediación:**
  1. Se adoptó formalmente la **Ruta A** aprobada en el checkpoint PC-2: reversión a `installed.json` estructurado con `schema_version: 2`.
  2. Se implementó migración tolerante y **SIN pérdida de datos** desde ambos formatos anteriores:
     - Formato legacy v1 (`installed.json` con `BTreeMap<String, Entry>` plano).
     - Directorio LMDB intermedio (`data.mdb`), parseando directamente las páginas y registros bincode en Rust puro y respaldando a `.lmdb.bak`.
  3. En caso de archivo corrupto, se genera respaldo timestamped (`.corrupt.<ts>`) antes de inicializar vacío (cero destrucción de datos).
  4. Se implementó escritura atómica mediante `tempfile::NamedTempFile` en el mismo directorio padre con `sync_all()` + rename POSIX atómico.
  5. Se implementó `build_date` en milisegundos (`Option<u64>`) con método de consulta `get_build_date()`.
  6. Se revirtió igualmente `src/cache.rs` a JSON plano atómico y se eliminó por completo la dependencia `heed` de `Cargo.toml`.
- **Validación:**
  - `db::tests::roundtrip_preserves_entries_and_build_date`
  - `db::tests::migrate_legacy_v1_json`
  - `db::tests::retain_and_remove`
  - `db::tests::corrupt_db_preserves_backup_and_starts_empty`
  - `db::tests::decode_bincode_entry_extracts_correct_fields`
  - `cache::tests::cache_roundtrip_and_expiry`
  - `cargo test` (99 tests pasando, 4 ignorados).
- **Estado:** ✅ CORREGIDO Y VALIDADO

---

### [H-006] Supresión silenciosa de errores sintácticos en repos.conf y sobreescritura destructiva en repo add
- **Severidad:** Critical
- **Módulo:** `src/reposconf.rs:53-70`, `src/repo.rs:37-207`, `src/install.rs:120,533`, `src/upgrade.rs:10,53`, `src/search.rs:77`, `src/info.rs:12`
- **Commit:** `f1f6e01` (`fix(H-006): prevent data loss on repos.conf parse errors`)
- **Descripción del problema:** `ReposConf::load` devolvía un `Result<Self>` que correctamente diferenciaba un archivo inexistente (`Ok(default)`) de un archivo existente con sintaxis inválida (`Err(...)`). Sin embargo, múltiples llamadas en el código base (`repo_add`, `repo_remove`, `repo_list`, `repo_rekey`, `install`, `upgrade`, `search`, `info`) usaban `.unwrap_or_default()`. Al invocar `vary repo add` o `repo remove` en un sistema con un error tipográfico o sintáctico en `repos.conf`, `vary` descartaba silenciosamente todo el contenido previo y guardaba una configuración nueva conteniendo únicamente el repositorio añadido, destruyendo de forma irreversible todos los repositorios configurados por el usuario.
- **Remediación:**
  1. En `src/repo.rs`: `repo_add` ahora valida y carga `repos.conf` al inicio (`?`) antes de cualquier operación remota (git clone) o de disco. Si `repos.conf` contiene errores de sintaxis o I/O, la operación aborta de inmediato protegiendo el archivo existente.
  2. En `repo_remove`, `repo_list` y `repo_rekey`, se propagan los errores con `?` en lugar de enmascararlos con defaults vacíos.
  3. En `src/install.rs`, `src/upgrade.rs`, `src/search.rs` y `src/info.rs`, se reemplazó `.unwrap_or_default()` por propagación estricta con `?`, garantizando que ninguna operación opere sobre un catálogo truncado debido a errores tipográficos.
  4. Se agregaron pruebas unitarias en `reposconf.rs` (`corrupt_toml_returns_error_with_context`) y en `repo.rs` (`repo_add_fails_and_preserves_corrupt_repos_conf`, `repo_remove_fails_on_corrupt_repos_conf`, `repo_list_fails_on_corrupt_repos_conf`), verificando que los errores TOML reportan archivo y línea y que los archivos corruptos nunca son sobreescritos.
- **Validación:**
  - `reposconf::tests::corrupt_toml_returns_error_with_context`
  - `repo::tests::repo_add_fails_and_preserves_corrupt_repos_conf`
  - `repo::tests::repo_remove_fails_on_corrupt_repos_conf`
  - `repo::tests::repo_list_fails_on_corrupt_repos_conf`
  - `cargo test` (103 tests pasando, 0 fallando, 4 ignorados).
- **Estado:** ✅ CORREGIDO Y VALIDADO

---

### [H-007] Flag --print / -p ejecuta mutación e instalación en lugar de dry-run
- **Severidad:** Critical
- **Módulo:** `src/command_line.rs:328-335,469-485`, `src/lib.rs:142-150`
- **Commit:** `33ee53d` (`fix(H-007): disable --print with informative error message`)
- **Descripción del problema:** Las banderas `-p`, `--print` y `--print-format` eran capturadas como banderas globales/argumentos pacman pero nunca eran procesadas por `handle_sync` ni `install`. Al ejecutar `vary -Sp <pkg>`, el pipeline procedía directamente a compilar e instalar paquetes con privilegios escalados en lugar de simular o imprimir el plan, engañando a usuarios y scripts que esperaban un dry-run no destructivo.
- **Remediación:**
  1. Conforme a la instrucción del checkpoint PC-2 ("un flag que miente es peor que un flag ausente"), se deshabilitó explícitamente `--print`, `-p` y `--print-format` en el parser de argumentos CLI (`command_line.rs`) con un error claro y accionable indicando su reserva para Fase 6 (Roadmap P0-4) y sugiriendo la alternativa `vary -S <pkg>`.
  2. Como defensa en profundidad, se agregó verificación explícita en `handle_sync()` (`lib.rs`) para rechazar cualquier combinación de argumentos que contenga `p` o `print`.
  3. Se incorporaron pruebas unitarias en `command_line::tests::print_flag_is_disabled_with_informative_error` comprobando el rechazo informativo tanto en formato corto agrupado (`-Sp`), como formato largo (`--print`) y opciones auxiliares (`--print-format`).
- **Validación:**
  - `command_line::tests::print_flag_is_disabled_with_informative_error`
  - `cargo test` (104 tests pasando, 0 fallando, 4 ignorados).
- **Estado:** ✅ CORREGIDO Y VALIDADO

---

### [H-008] Bug lógico en parser de templates Bash: condición inmutable y fallo en comillas multilínea
- **Severidad:** Critical
- **Módulo:** `src/vur_client.rs:554-645,1055-1110`
- **Commit:** `2ccae8b` (`fix(H-008): fix template parser multiline quote loop`)
- **Descripción del problema:** En `parse_template_text`, `quote_count` se calculaba una sola vez antes del bucle de comillas multilínea (`let quote_count = buf.matches('"').count(); while quote_count % 2 == 1`). La condición del bucle era inmutable (`clippy::while_immutable_condition`). Si una línea posterior no contenía comillas o contenía comillas escapadas `\"`, el bucle se rompía prematuramente o corría el riesgo de colgarse/recorrer indefinidamente. Además, ignoraba comillas simples `'`, no verificaba escapes de barra invertida y omitía alertar sobre condicionales por arquitectura (`case "$XBPS_TARGET_MACHINE"`).
- **Remediación:**
  1. Se implementó un detector de estado formal `has_unclosed_quote(buf)` que recorre los caracteres del valor reconociendo adecuadamente comillas dobles (`"`), comillas simples (`'`), secuencias de escape (`\"`) y comentarios (`#` precedidos por espacio fuera de cadenas entrecomilladas).
  2. El bucle multilínea evalúa dinámicamente `while has_unclosed_quote(&buf)` avanzando `lines.next()` y acumulando en el buffer, garantizando terminación limpia incluso ante archivos truncados (EOF sin cierre).
  3. Se incorporó detección y advertencia explícita sobre constructos condicionales dependientes de arquitectura (`XBPS_TARGET_`, `XBPS_MACHINE`, `XBPS_ARCH`), dando cumplimiento estricto al requerimiento A2 de FASE 0.
  4. Se agregaron pruebas unitarias completas en `vur_client::tests`:
     - `parse_template_handles_multiline_and_quotes`: verifica extracción correcta de descripciones y listas de dependencias/distfiles a lo largo de múltiples líneas con comillas simples y dobles.
     - `parse_template_unclosed_quote_at_eof_does_not_hang`: verifica que comillas sin cerrar en el fin de archivo no causan cuelgues ni pánicos.
- **Validación:**
  - `vur_client::tests::parse_template_handles_multiline_and_quotes`
  - `vur_client::tests::parse_template_unclosed_quote_at_eof_does_not_hang`
  - `cargo test vur_client` (9 tests pasando).
- **Estado:** ✅ CORREGIDO Y VALIDADO

---

### [H-022] 57 errores de compilación reportados por cargo clippy --all-targets -- -D warnings
- **Severidad:** High
- **Módulo:** Múltiples módulos en `src/` (`src/lock.rs`, `src/resolver.rs`, `src/bootstrap.rs`, `src/db.rs`, etc.)
- **Commit:** `8612906` (`fix(H-022): resolve all compiler warnings and clippy lints across codebase`)
- **Descripción del problema:** La base de código contenía 57 lints de clippy activos bajo `-D warnings`, abarcando conversiones numéricas innecesarias, if colapsables, bucles while-let en iteradores, flags de apertura de archivo sospechosos (`OpenOptions::truncate(false)`), y advertencias de código no idiomático.
- **Remediación:**
  1. Se resolvieron de forma exhaustiva y limpia todas las advertencias e inconsistencias estilísticas e idiomáticas señaladas por clippy sin usar directivas `#[allow]` cosméticas.
  2. En `src/lock.rs`, se especificó explícitamente `.truncate(false)` para eliminar la ambigüedad en `OpenOptions`.
- **Validación:**
  - `cargo clippy --all-targets -- -D warnings` (100% limpio, 0 advertencias, 0 errores).
  - `cargo test` (106 tests pasando).
- **Estado:** ✅ CORREGIDO Y VALIDADO

---

### [H-009] Inyección de argumentos en xbps-install, xbps-remove y xbps-query por omisión de --
- **Severidad:** High
- **Módulo:** `src/xbps.rs:153,185,342-375`, `src/install.rs:81`, `src/info.rs:24`
- **Commit:** `b630f1a` (`fix(H-009): add -- separator before targets in xbps invocations`)
- **Descripción del problema:** Las invocaciones a `xbps-install`, `xbps-remove` y `xbps-query` pasaban los argumentos de destino directamente sin anteponer el delimitador estándar `--`. Si un paquete comenzaba con un guion (`-`), era interpretado por las herramientas de xbps como un flag CLI (argument injection), alterando la semántica de la operación o causando fallos inesperados.
- **Remediación:**
  1. En `src/xbps.rs`, se agregaron las funciones auxiliares `build_install_command` y `build_remove_command`, insertando `cmd.arg("--").args(targets)` siempre que la lista de destinos no esté vacía.
  2. En `src/xbps.rs`, `query_installed` y `search_remote` ahora incluyen `"--"` antes del nombre de paquete o patrón.
  3. En `src/install.rs:81` (`official_exists_remote`) y `src/info.rs:24`, se agregó `"--"` antes del paquete de destino en la llamada a `xbps-query`.
  4. Se implementaron pruebas unitarias verificando la presencia estricta de `"--"` antes de los targets incluso cuando inician con `-`.
- **Validación:**
  - `xbps::tests::build_install_command_incluye_separador_doble_guion`
  - `xbps::tests::build_remove_command_incluye_separador_doble_guion`
  - `xbps::tests::build_install_command_sin_targets_no_pone_doble_guion`
  - `cargo test xbps`
  - `cargo clippy --all-targets -- -D warnings`
- **Estado:** ✅ CORREGIDO Y VALIDADO

---

### [H-010] Ausencia de validación con regex estricta en nombres de paquetes CLI y omisión en subpaquetes
- **Severidad:** High
- **Módulo:** `src/metadata.rs:105-115,195-205`, `src/install.rs:110,535`, `src/remove.rs:11`, `src/info.rs:20`
- **Commit:** `48eb719` (`fix(H-010): strictly validate package target names and subpackage names`)
- **Descripción del problema:** No se validaban los nombres de los targets recibidos por línea de comandos ni los nombres de subpaquetes (`subpackages[i].pkgname`) contra la especificación de nombres de paquete de Void Linux (`^[a-zA-Z0-9][a-zA-Z0-9._+-]*$`). Nombres que empezaban con punto, guion, o contenían caracteres de control, rutas (`/`) o inyección podían corromper rutas locales o provocar comportamientos erráticos.
- **Remediación:**
  1. En `src/metadata.rs`, se hizo pública `is_valid_pkgname(n: &str) -> bool`, validando que el primer carácter sea ASCII alfanumérico y que los restantes sean alfanuméricos o `._+-`.
  2. En `src/metadata.rs:validate()`, se agregó validación estricta de `sub.pkgname` con `is_valid_pkgname`.
  3. En `src/install.rs` (`install`, `download_only`), `src/remove.rs` (`remove`) y `src/info.rs` (`info`), se validan todos los nombres de targets antes de iniciar la transacción.
  4. Se implementaron pruebas unitarias en `metadata::tests`, `install::tests`, `remove::tests` e `info::tests` comprobando el rechazo de nombres que comienzan por punto, guion, contienen barras, espacios o caracteres ilegales.
- **Validación:**
  - `metadata::tests::rejects_invalid_pkgname`
  - `metadata::tests::rejects_invalid_subpackage_name`
  - `install::tests::install_rejects_invalid_target_name`
  - `install::tests::download_only_rejects_invalid_target_name`
  - `remove::tests::remove_rejects_invalid_target_name`
  - `info::tests::info_rejects_invalid_target_name`
  - `cargo test`
  - `cargo clippy --all-targets -- -D warnings`
- **Estado:** ✅ CORREGIDO Y VALIDADO

---

### [H-011] Inyección de directivas XBPS arbitrarias por falta de sanitización de newlines y esquemas inseguros en URLs
- **Severidad:** High
- **Módulo:** `src/keys.rs:85-130, 190-205`
- **Commit:** `b65acb8` (`fix(H-011): sanitize repository URLs and reject newline injection in xbps.d`)
- **Descripción del problema:** Las URLs de repositorios binarios VUR o índices VUP se formateaban directamente en líneas `repository=<url>\n` en archivos bajo `/etc/xbps.d/`. Si una URL contenía saltos de línea (`\n`, `\r`) o esquemas no admitidos por XBPS, se podían inyectar directivas de configuración arbitrarias en el gestor de paquetes de Void Linux o inducir comportamientos inesperados.
- **Remediación:**
  1. Se implementó `validate_repository_url(url: &str) -> Result<()>` en `src/keys.rs`, rechazando URLs vacías, con saltos de línea, caracteres de control, o esquemas distintos a `https://`, `http://` o `file://`.
  2. Se invocó `validate_repository_url` en `setup_binary_repo` antes de registrar la URL en `/etc/xbps.d/`.
  3. Se invocó `validate_repository_url` en `setup_vup_binary_repo` sobre cada una de las URLs binarias recibidas.
  4. Se agregó la prueba unitaria `validate_repository_url_rejects_newlines_and_bad_schemes` en `keys::tests`.
- **Validación:**
  - `keys::tests::validate_repository_url_rejects_newlines_and_bad_schemes`
  - `cargo test keys`
  - `cargo clippy --all-targets -- -D warnings`
- **Estado:** ✅ CORREGIDO Y VALIDADO

---

### [H-012] Inyección de argumentos y ejecución arbitraria mediante transportes peligrosos en git clone y git ls-remote
- **Severidad:** High
- **Módulo:** `src/vur_client.rs:60-120, 1145-1165`, `src/repo.rs:37`, `src/masterdir.rs:90`
- **Commit:** `08c4442` (`fix(H-012): sanitize git transport protocols and prevent URL argument injection`)
- **Descripción del problema:** Invocaciones a comandos Git remotos (`git clone` y `git ls-remote`) pasaban URLs sin verificar que comiencen con flags (ej. `--upload-pack=evil`), sin separar opciones con `--`, y sin deshabilitar protocolos de transporte inseguros (como `ext::`). Esto permitía ejecución de comandos arbitrarios ante URLs manipuladas.
- **Remediación:**
  1. Se implementó `is_safe_git_url(url: &str) -> bool` en `src/vur_client.rs`, exigiendo esquemas admitidos (`https://`, `http://`, `git://`, `ssh://`, `git@`, `file://`), prohibiendo prefijos con guion (`-`), saltos de línea y caracteres de control.
  2. En `clone_partial` y `detect_default_branch` (`src/vur_client.rs`) y en `clone_void_packages` (`src/masterdir.rs`), se agregaron los argumentos de configuración `-c protocol.ext.allow=never -c protocol.file.allow=user` y el separador `--` antes de las URLs.
  3. En `repo_add` (`src/repo.rs`), se valida la URL del repositorio con `is_safe_git_url` antes de cualquier operación.
  4. Se incorporó la prueba unitaria `is_safe_git_url_validates_and_rejects_dangerous_transports` en `vur_client::tests`.
- **Validación:**
  - `vur_client::tests::is_safe_git_url_validates_and_rejects_dangerous_transports`
  - `cargo test vur_client`
  - `cargo clippy --all-targets -- -D warnings`
- **Estado:** ✅ CORREGIDO Y VALIDADO

---

### [H-013] Condición de carrera (TOCTOU) y archivo temporal predecible /tmp/vary-<PID>.tmp en write_root_file
- **Severidad:** High
- **Módulo:** `src/keys.rs:64-88, 400-415`
- **Commit:** `a595235` (`fix(H-013): use secure NamedTempFile for root file writes`)
- **Descripción del problema:** `write_root_file` utilizaba una ruta estática predecible `/tmp/vary-<PID>.tmp` con permisos mundiales. En entornos multiusuario o compartidos, un atacante local podía crear enlaces simbólicos o explotar condiciones de carrera (TOCTOU) antes de la invocación de `install` con privilegios elevados para sobrescribir archivos del sistema.
- **Remediación:**
  1. Se sustituyó la ruta predecible por `tempfile::Builder::new().prefix("vary-").tempfile()`, generando un archivo temporal con nombre criptográficamente aleatorio, flags `O_EXCL` y permisos restringidos `0600`.
  2. Se escribe y descarga (`flush`) el contenido directamente en el archivo temporal antes de invocar el comando de instalación elevado.
  3. Se agregó la prueba unitaria `write_root_file_creates_file_safely` en `keys::tests`.
- **Validación:**
  - `keys::tests::write_root_file_creates_file_safely`
  - `cargo test keys`
  - `cargo clippy --all-targets -- -D warnings`
- **Estado:** ✅ CORREGIDO Y VALIDADO

---

### [H-014] Bypass del wrapper agnóstico de elevación y ejecución descontrolada con hardcode de Command::new("sudo")
- **Severidad:** High
- **Módulo:** `src/init.rs:5-40`, `src/install.rs:488`
- **Commit:** `3a36641` (`fix(H-014): use agnostic elevation in init service hook`)
- **Descripción del problema:** En `src/init.rs`, `post_install_hook` invocaba directamente `Command::new("sudo")` para habilitar/iniciar servicios (dinitctl, ln), ignorando la configuración agnóstica de elevación (`sudo_bin`, `sudo_flags`, opendoas, run0 o ejecución como root directo), y utilizaba `.unwrap()` sobre rutas (`sv_dir`, `service_link`).
- **Remediación:**
  1. Se actualizó la firma de `post_install_hook` para recibir `sudo_bin: &str, sudo_flags: &[String]` y se pasó desde `src/install.rs:488`.
  2. Se reemplazaron todas las invocaciones directas a `sudo` por `crate::elevate::elevate(sudo_bin, sudo_flags, ...)`.
  3. Se eliminaron las llamadas a `.unwrap()` pasando referencias directas de ruta a los argumentos.
  4. Se agregó la prueba unitaria `hook_retorna_ok_si_paquete_no_tiene_servicio` en `init::tests`.
- **Validación:**
  - `init::tests::hook_retorna_ok_si_paquete_no_tiene_servicio`
  - `cargo test`
  - `cargo clippy --all-targets -- -D warnings`
- **Estado:** ✅ CORREGIDO Y VALIDADO

---

### [H-017] Omisión total de subpaquetes y sobreescritura de variables padre en parser de templates
- **Severidad:** High
- **Módulo:** `src/vur_client.rs:650-710, 800-840, 1250-1290`
- **Commit:** `6571158` (`fix(H-017): parse subpackages and isolate parent variables in template parser`)
- **Descripción del problema:** En `parse_template_text`, el campo `subpackages` se inicializaba rígidamente como `vec![]`, omitiendo los subpaquetes generados por plantillas (`<subpkg>_package()`). Asimismo, si una función de subpaquete contenía asignaciones (`depends`, `short_desc`), existía riesgo de sobreescribir las variables globales del paquete padre en el mapa de variables o truncar el parseo.
- **Remediación:**
  1. Se implementó detección estricta de funciones `<subpkg>_package()` con seguimiento de anidamiento de llaves (`brace_depth`), aislando variables locales (`depends`, `short_desc`) y evitando sobreescritura de variables del paquete padre.
  2. Las funciones de subpaquete se procesan expandiendo variables estándares de Void (`${sourcepkg}`, `${pkgname}`, `${version}`, `${revision}`).
  3. Se construyen instancias de `Subpackage` y se asignan a `VurInfo::subpackages`, validando nombres válidos y evitando duplicados con el paquete padre.
  4. Se implementó la prueba unitaria `parse_template_extracts_subpackages_and_preserves_parent_vars` en `vur_client::tests`.
- **Validación:**
  - `vur_client::tests::parse_template_extracts_subpackages_and_preserves_parent_vars`
  - `cargo test vur_client` (11 tests pasando).
  - `cargo clippy --all-targets -- -D warnings`
- **Estado:** ✅ CORREGIDO Y VALIDADO






