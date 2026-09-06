# FIX LOG — Registro de Remediaciones

> **Regla de cierre (2026-09-06):** un hallazgo se considera cerrado solo con un
> run de CI verde (fmt + clippy `-D warnings` + test) en el commit del fix;
> la validación local no cuenta. Los commits que solo tocan `audit/**`,
> `docs/**`, `roadmap/**` o `*.md` no disparan CI (paths-ignore) y quedan
> exentos. Los fixes se citan por subject (`git log --oneline
> --grep="H-###"`); el hash exacto no puede autocontenerse en el propio commit.

---

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
  - **Enmienda (2026-09-06):** el fix original nunca se validó en CI y quedó
    incompleto ante el clippy de Actions 1.88 (`uninlined_format_args`,
    112 errores en el baseline del 2026-09-06). Cierre real en CI:
    `976d1af` (`fix(clippy): inline format args`, reescritura mecánica
    verificada por muestreo) + `302a966` (último sitio en `metadata.rs`);
    run verde con 119 passed / 4 ignored. Ver regla de cierre al inicio.
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
- **Commit:** `91e233c` (`fix(H-017): parse subpackages and isolate parent variables in template parser`)
  - Nota (2026-09-06): esta entrada citaba `6571158`, hash que no corresponde al commit del fix. Reconciliado contra `git log`.
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






---

### [H-018] Supresión silenciosa de errores con `if let Ok(...)` en lectura y proyección de plantillas VUR
- **Severidad:** High
- **Módulo:** `src/vur_client.rs` (índice git-show + fallback en disco + `project_pkg` + `copy_dir_recursive`)
- **Commit:** `fix(H-018)` (`git log --oneline --grep="H-018"`)
- **Descripción del problema:** `if let Ok(...)` descartaba en silencio plantillas/.VURINFO con errores de sintaxis o I/O, reportando "paquete no existe" y ocultando la causa raíz.
- **Remediación:**
  1. Convertidos todos los `if let Ok` a `match` explícito con rama `Err`.
  2. Criterio duro: el aviso por índice/plantilla inválida va a **stderr vía `eprintln!`**, no solo a tracing (la capa de consola escribe a stdout y en tests no hay subscriber: repetir `warn!` habría repetido el bug de invisibilidad).
  3. Mensaje construido por `skipped_index_warning()` (función pura: kind + ubicación + causa), testeable sin capturar stderr.
  4. Ausencias normales (sin `.VURINFO` donde se espera fallback) quedan en `tracing::trace!`, sin ruido al usuario.
  5. `copy_dir_recursive`: `read_link` propaga contexto en vez de saltar symlinks en silencio.
- **Validación:**
  - `vur_client::tests::skipped_index_warning_mentions_kind_location_and_cause` (texto exacto que ve el usuario en stderr).
  - `vur_client::tests::load_index_ignora_vurinfo_invalido_sin_abortar` (fixture git real: `.VURINFO` con `revision: 0` se ignora, `hello` sigue cargando).
  - Run CI verde (fmt + clippy + test) en el commit del fix.
- **Estado:** ✅ CORREGIDO Y VALIDADO

---

### [H-047] Archivo residual `guia` en la raíz reubicado a `docs/CONTRATO.md`
- **Severidad:** Info
- **Módulo:** `guia` → `docs/CONTRATO.md`
- **Commit:** `fix(H-047)` (`git log --oneline --grep="H-047"`)
- **Descripción del problema:** Archivo de 29 KB en la raíz fuera de cualquier convención de layout. Contenido verificado: es el contrato original (análisis comparativo vary/vuru/vura/vouru, base de COMPLIANCE_MATRIX), por lo que se mueve, no se elimina.
- **Remediación:**
  1. Reubicación a `docs/CONTRATO.md`.
  2. Actualizada la entrada H-047 en `AUDIT_REPORT.md` (módulo + resolución + estado). Sin otras referencias a la ruta vieja en el repo (verificado con grep en README, docs, src, etc.).
- **Validación:** Commit docs-only (exento de CI por paths-ignore); verificación de ausencia de referencias rotas.
- **Estado:** ✅ CORREGIDO Y VALIDADO
---

### [H-016] Carreras TOCTOU en señales (PGID desacoplado) y fuga de temporales `/tmp/vary-*`
- **Severidad:** High
- **Módulo:** `src/signal.rs`, `src/xbps.rs:419-460`, `src/lib.rs`
- **Commit:** `fix(H-016)` (`git log --oneline --grep="H-016"`)
- **Descripción del problema:** (1) Si SIGINT llegaba entre `spawn()` y `register_child()`, el observador salía sin matar al hijo (grupo huérfano). (2) `process::exit()` se salta los `Drop` de `NamedTempFile` (`keys.rs`), acumulando `/tmp/vary-*`.
- **Remediación (simple, sin cirugía de procesos):**
  1. `block_term_signals()` (guardia RAII con `sigprocmask`) envuelve spawn+registro en `xbps_src`; se suelta ANTES del `wait()` largo para no cegar al observador.
  2. Tras registrar, si `is_shutting_down()` ya es true, el padre mata el grupo él mismo (`kill_child_group`, `kill(-pid)`) y aborta con error, sin esperar al poll.
  3. `sweep_stale_tmp_files[_in]()`: borra `vary-*` solo con uid propio y solo archivos/symlinks; se corre al arrancar (tras el lock, sin riesgo a otra instancia) y en `observer_cleanup_and_exit` antes de `exit()`.
- **Validación:**
  - `signal::tests::sweep_borra_solo_prefijo_propio` (no toca archivos ajenos).
  - `signal::tests::bloqueo_de_senales_se_restaura_con_drop` (la máscara no fuga).
  - Run CI verde en el commit del fix. El caso SIGINT-durante-spawn queda para el smoke en Void real (FASE 5, humano).
- **Estado:** ✅ CORREGIDO Y VALIDADO
---

### [H-019] Falta de mensaje explicativo al denegar por EOF en no-TTY
- **Severidad:** High
- **Módulo:** `src/util.rs:46-80`
- **Commit:** `fix(H-019)` (`git log --oneline --grep="H-019"`)
- **Descripción del problema:** Tras H-001, stdin EOF denegaba bien pero `install` abortaba con código 1 en silencio, sin indicar `--noconfirm`.
- **Remediación:**
  1. `confirm_from_reader` devuelve `Result<Option<bool>>` (`None` = EOF/error I/O), distinguiendo denegación explícita de falta de entrada.
  2. `confirm()` emite a stderr `EOF_DENIAL_HINT` ("...usa --noconfirm.") antes de denegar. Firma intacta: `init.rs`, `install.rs`, `keys.rs` sin cambios.
- **Validación:**
  - Tests actualizados a `Option` + `eof_hint_points_to_noconfirm` (el texto fija `--noconfirm`).
  - Emisión real con stdin `/dev/null` queda para el smoke en Void real (FASE 5, humano; va a VALIDATION.md).
  - Run CI verde en el commit del fix.
- **Estado:** ✅ CORREGIDO Y VALIDADO
---

### [H-039] Dependencias contractuales ausentes (`diffy`, `indicatif`)
- **Severidad:** Medium
- **Módulo:** `Cargo.toml`, `Cargo.lock`
- **Commit:** `fix(H-039)` (`git log --oneline --grep="H-039"`)
- **Descripción del problema:** A3 (diff pager) y A7 (spinner/logs) requerían `diffy` e `indicatif`, ausentes del manifiesto.
- **Remediación:** `cargo add indicatif@0.17 diffy@0.4` (resolución + lock; `indicatif` se usa ya en H-020, `diffy` queda disponible para A3/Fase 6).
- **Validación:** Run CI verde (build compila las nuevas deps) en el commit del fix.
- **Estado:** ✅ CORREGIDO Y VALIDADO
---

### [H-020] Ausencia de canalización de logs y spinners: xbps-src inundaba la consola
- **Severidad:** High
- **Módulo:** `src/xbps.rs`, `src/masterdir.rs`, `src/bootstrap.rs`
- **Commit:** `fix(H-020)` (`git log --oneline --grep="H-020"`)
- **Descripción del problema:** `xbps-src` heredaba stdio crudo: miles de líneas de compilador en la terminal y nada en `vary.log`.
- **Remediación:**
  1. `xbps_src()` acepta `log_file: Option<&Path>`. Con `Some`, stdout/stderr van por pipes a hilos de bombeo (`pump_stream_to_log`) que escriben append al archivo, con separador `=== xbps-src <args> ===` por invocación.
  2. La terminal muestra solo un spinner `indicatif` en stderr con la ruta del log; fuera de TTY el spinner se oculta explícitamente (`IsTerminal`, sin escape codes en CI/pipes).
  3. `Masterdir.log_file` (nuevo campo, `None` por defecto) se fija en `initialize_environment` a `<cache>/logs/xbps-src.log`; el código de salida y el tracking anti-huérfanos (H-016) no cambian.
- **Validación:**
  - `xbps::tests::pump_vuelca_lineas_al_log` (el bombeo vuelca íntegro al archivo).
  - Verificación visual del spinner + contenido del log con build real queda para el smoke en Void real (FASE 5, humano).
  - Run CI verde en el commit del fix.
- **Estado:** ✅ CORREGIDO Y VALIDADO
---

### [H-021] Violación XDG: rutas `$HOME/.config` etc. hardcodeadas
- **Severidad:** High
- **Módulo:** `src/config.rs:176-196`
- **Commit:** `fix(H-021)` (`git log --oneline --grep="H-021"`)
- **Descripción del problema:** `Config::new` ignoraba `XDG_CONFIG_HOME`, `XDG_CACHE_HOME`, `XDG_DATA_HOME`.
- **Remediación:** Nuevo `default_dirs(home)` que usa `dirs::cache_dir/data_dir/config_dir` (el crate ya respeta XDG) con fallback a `$HOME/.cache`, `$HOME/.local/share`, `$HOME/.config` si la variable no existe. Precedencia intacta: `vary.conf` explícito sigue ganando.
- **Validación:**
  - `config::tests::default_dirs_respeta_xdg_con_fallback_a_home` (XDG seteado + fallback; restaura el entorno).
  - Run CI verde en el commit del fix.
- **Estado:** ✅ CORREGIDO Y VALIDADO
---

### [H-023] Pánicos por `unwrap()`/`expect()` en rutas alcanzables por el usuario (+H-038)
- **Severidad:** High (H-038: Medium, cerrado por el mismo cambio)
- **Módulo:** `src/command_line.rs`, `src/install.rs:163`, `src/keys.rs:28`, `src/main.rs:37-45`, `src/vur_client.rs:664,936`
- **Commit:** `fix(H-023)` (`git log --oneline --grep="H-023"`)
- **Descripción del problema:** Barrido completo: 154 `unwrap`/`expect` en el árbol, 146 en tests (convención aceptada) y 8 en código productivo. Los 8: valores de flags CLI sin valor, `repos_conf` inconsistente, abandono de privilegios con `expect`, `parent()` de symlink y primera comilla del parser.
- **Remediación:**
  1. `command_line.rs`: `split.next()`, `chars.next()` y los 5 brazos `--arch/--sudo/--sudoflags/--git/--curl` usan let-else/`is_none` con `bail!` accionable. (Los 5 ya tenían guard `TakesValue::Required`; el let-else es defensa sin panic si la clasificación cambia.)
  2. `install.rs:163`: `ok_or_else` con contexto en vez de `unwrap()` sobre `repos_conf`.
  3. `keys.rs:28`: let-else subsume el chequeo de vacío.
  4. `main.rs`: `drop_privileges()` devuelve `Result`; `main` imprime a stderr y sale 1. Cierra también H-038 (su única evidencia eran estas líneas).
  5. `vur_client.rs:664`: `parent()` con `ok_or_else`; `:936`: `if let` en vez de `unwrap()` (el `remove(0)` posterior habría hecho panic en cadena).
  6. `config.rs:128` (`Default` con `expect`) queda para H-037: `Default` no puede propagar errores, requiere rediseño propio.
- **Validación:**
  - `command_line::tests::opciones_con_valor_sin_valor_devuelven_error_en_vez_de_panic` (los 5 flags: error con nombre, sin panic).
  - Test `drop_privileges_sin_root_devuelve_ok` en el binario (no-root → Ok).
  - Fuzzing de entradas queda para FASE 5 (humano, opcional).
  - Run CI verde en el commit del fix.
- **Estado:** ✅ CORREGIDO Y VALIDADO
---

### [H-024] Vacío de cobertura en módulos centrales
- **Severidad:** High
- **Módulo:** transversal (`src/`)
- **Commit:** `fix(H-024)` (`git log --oneline --grep="H-024"`)
- **Descripción del problema:** Al abrirse el hallazgo, `install/config/upgrade/db` y otros tenían cero tests.
- **Remediación:** Cobertura acumulada durante la auditoría (cada fix trae sus tests) + cierre de huecos puros restantes: `args.rs` (`Arg::fmt`, `Args::has_arg`), `review.rs` (ruta no-TTY con fallback en disco). Tally final: 27 de 29 módulos con tests; quedan en cero solo `upgrade.rs`, `search.rs`, `lib.rs` y `help.rs`, todos integration-bound (requieren repos/xbps/flujos completos) y cubiertos por el smoke de FASE 5 en Void real.
- **Validación:** `cargo test` (suite en verde) + run CI verde en el commit del fix.
- **Estado:** ✅ CORREGIDO Y VALIDADO

---

### [H-029] Campo fantasma `install_date`, `build_date` ausente y cero recompilación preventiva
- **Severidad:** Medium
- **Módulo:** `src/db.rs`, `src/upgrade.rs`
- **Commit:** Sin cambio de código (resuelto-obsoleto; veredicto documentado aquí)
- **Evidencia contra el hallazgo (código actual, post-H-005):**
  1. `build_date: Option<u64>` en ms EXISTE (`db.rs:36`, doc A5) y se puebla en `upsert()` para `InstallType::Source` (`db.rs:162-172`), en la migración LMDB (`db.rs:265-267`) y por backfill tolerante desde `install_date` (`db.rs:92-96`).
  2. `install_date` NO es fantasma: se lee como fuente del backfill (`db.rs:92-96`) y se decodifica del LMDB (`db.rs:249,265`).
  3. Resto real: `upgrade.rs` nunca consulta `build_date`; no hay trigger de recompilación preventiva ante drift de sonames. Eso requiere diseño (fuente de sonames, política) fuera del alcance de la auditoría: se registra como trabajo futuro en `roadmap/STATUS.md` (P2).
- **Validación:** Tests H-005 (`roundtrip_preserves_entries_and_build_date`, `migrate_legacy_v1_json`) + suite verde.
- **Estado:** ✅ CERRADO POR OBSOLESCENCIA PARCIAL (veredicto con evidencia; resto a roadmap)
---

### [H-025] Directorios de caché/logs/lock/DB sin permisos restringidos `0700`
- **Severidad:** Medium
- **Módulo:** `src/util.rs`, `src/cache.rs`, `src/db.rs`, `src/lock.rs`, `src/logging.rs`, `src/masterdir.rs`, `src/repo.rs`, `src/reposconf.rs`, `src/vup_index.rs`, `src/vur_client.rs`, `src/xbps.rs`
- **Commit:** `fix(H-025)` (`git log --oneline --grep="H-025"`)
- **Descripción del problema:** Todos los `create_dir_all` usaban la umask heredada; el defecto excedía los 4 sitios citados (13 sitios en total).
- **Remediación:** Nuevo `util::ensure_private_dir()` (`DirBuilder` recursivo con `mode(0o700)`) usado en los 13 sitios. Solo aplica en creación: directorios preexistentes no se tocan.
- **Validación:**
  - `util::tests::ensure_private_dir_crea_con_0700` (modo exacto + idempotencia).
  - Run CI verde en el commit del fix.
- **Estado:** ✅ CORREGIDO Y VALIDADO
---

### [H-026] Pánico por bandera sin valor al final de `argv`
- **Severidad:** Medium
- **Módulo:** `src/command_line.rs:219`
- **Commit:** `fix(H-026)` (`git log --oneline --grep="H-026"`)
- **Descripción del problema:** La evidencia citaba `args[i + 1].clone()` con pánico out-of-bounds. El parser actual ya usa `raw.get(idx + 1)` (nunca indexa): el defecto no existe en el árbol.
- **Remediación:** Sin cambio productivo necesario. Se añade test de regresión que fija el comportamiento: flag trailing sin valor → error limpio "expects a value", sin panic.
- **Validación:**
  - `command_line::tests::flag_con_valor_al_final_sin_valor_da_error_limpio`.
  - Run CI verde en el commit.
- **Estado:** ✅ CERRADO POR OBSOLESCENCIA (ya resuelto en el árbol; test lo fija)
---

### [H-027] Lock global prematuro + mensaje que induce split-brain
- **Severidad:** Medium
- **Módulo:** `src/lib.rs`, `src/command_line.rs`, `src/lock.rs`
- **Commit:** `fix(H-027)` (`git log --oneline --grep="H-027"`)
- **Descripción del problema:** (1) `flock` se adquiría antes de parsear: `--help`, `-V`, `-Ss`, `-Si`, `--repo list` se bloqueaban tras otra instancia. (2) El mensaje sugería borrar `vary.lock`; como el flock vive en el inode, borrar+recrear permite doble ejecución (split-brain).
- **Remediación:**
  1. Nuevo `command_line::peek_repo_cmd()` (lectura no destructiva del thread-local).
  2. `run()` parsea primero, atiende help/version sin lock y adquiere el lock solo si `needs_lock()`: mutantes (`-S` con targets, `-Sy/-Su/-Syu`, `-Sw`, `-R`, default, `--repo add/remove/rekey`); solo-lectura (`-Ss`, `-Si`, `--repo list`) corre sin lock. El barrido de temporales corre solo con lock.
  3. Mensaje sin "borra": el lock se libera solo al salir; el pid orienta.
- **Validación:**
  - `lib::tests::solo_lectura_no_requiere_lock_mutacion_si` y `repo_list_no_requiere_lock_add_si` (con limpieza del thread-local).
  - Test de lock exige ausencia de "borra".
  - Run CI verde en el commit del fix.
- **Estado:** ✅ CORREGIDO Y VALIDADO
---

### [H-028] Logger inmutable (`Once`), `-v` desconectado y pérdida de logs en `exit()`
- **Severidad:** Medium
- **Módulo:** `src/logging.rs`, `src/lib.rs`, `src/signal.rs`, `src/main.rs`, `Cargo.toml`
- **Commit:** `fix(H-028)` (`git log --oneline --grep="H-028"`)
- **Descripción del problema:** `init()` con `Once` congelaba nivel INFO; `-v` y `[general] log_level` (cargado pero jamás leído) no surtían efecto; `process::exit` en señales/pipe roto perdía el buffer del log.
- **Remediación:**
  1. Filtros tras `reload::Handle` (feature `reload` en tracing-subscriber): `init()` instala una vez, `apply_runtime_config(verbose, log_level)` ajusta tras el parse con precedencia `RUST_LOG` > `-v` > TOML > default.
  2. Worker global + `logging::shutdown()` (flush vía Drop) invocado en `observer_cleanup_and_exit` antes de `exit()` y en la rama de pipe roto de `main` (vía `vary::shutdown_logging()`).
- **Validación:**
  - `logging::tests::init_es_idempotente_y_no_paniquea` (doble init + shutdown idempotentes).
  - Nivel efectivo con `-v`/TOML queda para verificación manual (una línea `tracing::debug!` visible con `-v`).
  - Run CI verde en el commit del fix.
- **Estado:** ✅ CORREGIDO Y VALIDADO
---

### [H-030] Comodín masivo `""` y orden respecto a la sincronización
- **Severidad:** Medium
- **Módulo:** `src/xbps.rs:270-288`, `src/install.rs:251-263`
- **Commit:** `fix(H-030)` (`git log --oneline --grep="H-030"`)
- **Descripción del problema:** La evidencia afirmaba que `""` era un comodín erróneo (debía ser `'*'`) y que consultar antes del sync usaba metadatos obsoletos.
- **Veredicto con evidencia (Void real, 2026-09-06):** `xbps-query -Rs ""` y `-Rs "*"` devuelven exactamente lo mismo en este sistema. El comodín NO es erróneo; se fija con comentario en el código para que nadie lo "arregle". Sobre el orden: el bulk es snapshot solo-acelerador — cada miss se confirma con query escalar en vivo (`official_exists_remote`), así que la dirección peligrosa (paquete nuevo tras el snapshot) se resuelve bien; un hit obsoleto (paquete retirado a mitad de corrida) aflora como error claro de `xbps-install`, no como corrupción silenciosa.
- **Remediación:** Comentario fijando `""` + test de parsing ya existente (`bulk_names_stripea_repo_y_version` cubre `[*]`/`[-]`/`[repo]`).
- **Validación:** Comandos reales en Void + run CI verde en el commit del fix.
- **Estado:** ✅ CERRADO (premisa del comodín refutada con evidencia; orden seguro por diseño)
---

### [H-032] Banderas heredadas aceptadas sin error e ignoradas en silencio
- **Severidad:** Medium
- **Módulo:** `src/command_line.rs`, `src/help.rs`, `README.md`, `README.es.md`
- **Commit:** `fix(H-032, H-033)` (`git log --oneline --grep="H-032"`)
- **Descripción del problema:** `--asdeps`/`--asexplicit` (+alias) se parseaban y registraban sin ningún efecto: el usuario creía marcar dependencias y vary instalaba todo como explícito.
- **Remediación:** Las 4 grafías (`asdeps/asdep/asexplicit/asexp`) se rechazan con `bail!` explicativo (precedente H-007: un flag que miente es peor que ausente). README bilingüe documenta el rechazo.
- **Validación:**
  - `command_line::tests::asdeps_y_alias_se_rechazan_sin_mentir` (las 4 grafías).
  - Run CI verde en el commit del fix.
- **Estado:** ✅ CORREGIDO Y VALIDADO

---

### [H-033] `-y` interpretado como refresh en vez de yes
- **Severidad:** Medium
- **Módulo:** `src/command_line.rs`, `src/help.rs`, `README.md`, `README.es.md`
- **Commit:** `fix(H-032, H-033)` (`git log --oneline --grep="H-033"`)
- **Descripción del problema:** Herencia pacman: `-y` es refresh en vary, pero usuarios esperan "yes"; `--yes` directamente fallaba como opción desconocida.
- **Remediación:** `Arg::Long("yes")` como alias de `--noconfirm` (incluido en la allow-list de opciones vary; `TakesValue::No` rechaza `--yes=x` con el check existente). `-y` intacto como refresh. Help + README bilingüe lo explicitan.
- **Validación:**
  - `command_line::tests::yes_es_alias_de_noconfirm_y_guion_y_sigue_refresh` (`--yes` activa, `-y` no confirma, `--yes=x` falla).
  - Run CI verde en el commit del fix.
- **Estado:** ✅ CORREGIDO Y VALIDADO
---

### [H-031] Inconsistencia de validación de checksum vacío frente a VURINFO v1
- **Severidad:** Medium
- **Módulo:** `src/metadata.rs`
- **Commit:** `fix(H-031)` (`git log --oneline --grep="H-031"`)
- **Descripción del problema:** El doc en código (regla 5) decía "la lista no puede estar vacía" y un comentario de test afirmaba "validate exige >= 1 checksum", pero el código acepta lista vacía con warning (plantillas con `do_fetch` propio) y la especificación (`docs/VURINFO.md`: campo opcional, regla por elemento) lo ampara.
- **Remediación (excepción justificada, sin cambio de conducta):** Doc de la regla 5 y comentario del test alineados con código + especificación: lista vacía aceptada con warning; elementos deben ser `sha256:`/`SKIP` (vacío rechazado, ya testeado).
- **Validación:** Tests existentes (`allows_empty_checksum_for_custom_fetch`, `rejects_checksum_elemento_vacio`) + run CI verde.
- **Estado:** ✅ CORREGIDO Y VALIDADO
---

### [H-034] Falta de restauración del modo de terminal tras pánico o interrupción
- **Severidad:** Medium
- **Módulo:** `src/xbps.rs`, `src/signal.rs`, `src/main.rs`, `src/lib.rs`, `src/review.rs`
- **Commit:** `fix(H-034)` (`git log --oneline --grep="H-034"`)
- **Descripción del problema:** `indicatif` oculta el cursor durante el spinner; un panic/`exit()` a mitad de build lo dejaba invisible. El pager tampoco estaba contemplado.
- **Remediación (residual real, sin teatro):**
  1. `CursorGuard` (RAII) en `run_logged`: restaura el cursor al salir del build por cualquier vía, incluido unwind. (El guardián entró con `006dc8b`; el resto aquí.)
  2. `signal::restore_terminal()` (`\x1b[?25h\x1b[0m`, solo en TTY) invocado en `observer_cleanup_and_exit` antes de `exit()` y en el hook de panic de `main` (vía `vary::restore_terminal()`).
  3. Pager evaluado y DESCARTADO a propósito: el observador mata por grupo (`-pid`) y el pager no es líder (comparte el frontal para recibir SIGINT); registrarlo arriesgaría matar un grupo ajeno por reutilización de pid. Comentario en `review.rs`, caso residual benigno (sin locks).
- **Validación:** Ruta normal + cursor idempotente; cursor tras Ctrl+C y pager quedan para el smoke en Void real (FASE 5, humano). Run CI verde.
- **Estado:** ✅ CORREGIDO Y VALIDADO
---

### [H-035] Códigos de salida inconsistentes (colapso en 1, fuga de -1)
- **Severidad:** Medium
- **Módulo:** `src/xbps.rs`, `src/lib.rs`
- **Commit:** `fix(H-035)` (`git log --oneline --grep="H-035"`)
- **Descripción del problema:** Todo error colapsaba en 1; muerte por señal filtraba `-1` (el shell lo ve como 255, perdiendo la señal); binario ausente indistinguible.
- **Remediación:**
  1. `xbps::exit_code_of_status()`: código del hijo o `128+señal` en unix; reemplaza los 3 `unwrap_or(-1)` (doc actualizado).
  2. `run()`: error de parseo → 2 (mal uso CLI); `is_not_found_error()` camina la cadena buscando `io::NotFound` → 127. `spawn_error` conserva la fuente con `.context()` para que la cadena exista.
  3. El wrapper `restore_terminal()` de `lib.rs` (H-034) entró en este commit por compartir archivo; el hook vive en `fix(H-034)`.
- **Validación:**
  - `xbps::tests::exit_code_mapea_salida_normal_y_senal` (ExitStatus crudos unix), `not_found_se_detecta_en_cadena_de_error`, `lib::tests::cli_mal_usado_devuelve_2` (`run()` real con flag desconocido).
  - Run CI verde en el commit del fix.
- **Estado:** ✅ CORREGIDO Y VALIDADO
---

### [H-036] Flag CLI `--sudoflags` concatena en vez de sobrescribir
- **Severidad:** Medium
- **Módulo:** `src/command_line.rs`, `src/config.rs`, `src/help.rs`, `etc/vary.conf.example`
- **Commit:** `fix(H-036, H-037)` (`git log --oneline --grep="H-036"`)
- **Descripción del problema:** Único caso donde CLI extendía en vez de reemplazar (`sudo_bin`/`git`/`curl` reemplazan): imposible quitar un flag de `vary.conf` desde CLI.
- **Remediación:** `Config.sudo_flags_from_cli`: el primer `--sudoflags` reemplaza, repetirlo acumula sobre lo dado en CLI. Help + ejemplo documentan la semántica.
- **Validación:**
  - `command_line::tests::sudoflags_cli_reemplaza_conf_y_repetir_acumula`.
  - Run CI verde en el commit del fix.
- **Estado:** ✅ CORREGIDO Y VALIDADO

---

### [H-037] `Config::default()` con I/O y pánico sin `$HOME`
- **Severidad:** Medium
- **Módulo:** `src/config.rs`
- **Commit:** `fix(H-036, H-037)` (`git log --oneline --grep="H-037"`)
- **Descripción del problema:** `Default` llamaba a `new().expect()`: leía `$HOME`, `vary.conf` del host, `available_parallelism` y sonda TTY; pánico sin `HOME` y contaminación de tests con la config del desarrollador.
- **Remediación:** `Config::in_memory_defaults()` (rutas vacías, `makejobs: 1`, `Colors::default()` sin sonda); `Default` lo usa; `new()` parte de ahí y añade entorno real + `load_vary_conf()`. Conducta productiva intacta (`lib.rs` solo usa `new()`).
- **Validación:**
  - `config::tests::default_es_puro_sin_io_ni_host`.
  - Toda la suite ya usaba `Default` en tests: sigue verde = sin regresión.
  - Run CI verde en el commit del fix.
- **Estado:** ✅ CORREGIDO Y VALIDADO
