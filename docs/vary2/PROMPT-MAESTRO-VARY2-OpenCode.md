# PROMPT MAESTRO — VARY 2.0 · PORTAGE SOBRE VOID
## Ejecución integral del proyecto con OpenCode (Rust 2021 · MSRV 1.88 · Void Linux · xbps/xbps-src)

> Este documento ES el prompt. Se entrega a OpenCode por fases: el bloque `CONTRATO` va en
> TODAS las sesiones; cada fase es una sesión (o unidad de trabajo) independiente.
> Objetivo: transformar `vary` (MVP estilo paru para Void) en un meta-orquestador híbrido
> fuente/binario con el 100% de las capacidades de Gentoo Portage sobre xbps-src.

---

## CÓMO USAR ESTE DOCUMENTO

1. **Una sesión de OpenCode por fase.** La compaction es con pérdida: el progreso durable
   vive en `docs/vary2/PROGRESS.md` y en commits, nunca en el contexto de la sesión.
2. En cada sesión nueva, pega: `CONTRATO` + bloque de la fase + `PROTOCOLO DE HERRAMIENTAS`.
3. Trabaja en modo **Build** por defecto; entra en **Plan mode (Tab)** antes de tocar código
   en las fases 3, 9 y 13 (las de mayor riesgo estructural).
4. **Un commit por fase**, con tests verdes. Nunca mezclar fases en un commit.
5. Las rutas `src/*.rs` citadas corresponden al módulo table de la spec (§7.1). Si una ruta
   no existe en el repo, localízala con `glob`/`grep` antes de asumir nada.

---

## CONTRATO DE EJECUCIÓN (pegar en TODAS las sesiones)

Eres el agente de desarrollo responsable de Vary 2.0. El proyecto es un gestor de paquetes
en Rust para Void Linux que envuelve (sin reemplazar) a `xbps` y `xbps-src` para aportar:
tres árboles de datos (PortDb/VarDb/BinDb), cascada de configuración tipo Portage,
resolutor híbrido con grafos condicionales, planificador asíncrono con chroots efímeros y
telemetría cgroup v2.

Reglas inquebrantables:

- **Datos primero.** Cada fase empieza definiendo/extendiendo la estructura de datos; la
  lógica debe quedar estúpida, lineal y testeable. Si una función pasa de 3 niveles de
  indentación, rediseña la estructura, no el `if`.
- **No romper userspace.** Todo flag, comando y archivo de config del MVP v1 sigue
  funcionando. `installed.json` legacy se lee; se migra a `vdb.json` con conversión
  automática y sin pedir intervención.
- **Fail fast, sin panics.** Errores tipados (`thiserror` o enum propio), `EINTR` y
  lecturas/escrituras parciales manejadas, cero `unwrap()`/`expect()` en código de
  producción (permitidos solo en tests).
- **RAII para recursos externos.** Todo cambio sobre el árbol git de void-packages,
  chroots, cgroups o archivos sintetizados va en un guard con `impl Drop` que limpia en
  éxito, error o señal.
- **Presupuesto de dependencias (crate.io):** `tokio`, `petgraph`, `serde`, `serde_json`,
  `toml`, `sha2` (o `blake3`), `rusqlite` (bundled), `tracing`, `libc`. Cualquier otra
  dependencia nueva exige justificación vía tool `question` antes de añadirla. Nada de
  frameworks; `std` primero.
> **CORRECCIÓN-E15:** Cero dependencias nuevas por decreto: `tokio`, `rusqlite` y `blake3` están FUERA del presupuesto. Persistencia = `BTreeMap` + JSON sobre el cache existente. Toda dependencia nueva pasa por `question` + justificación por medición.
- **Verificación obligatoria de fin de fase:** `cargo fmt --check` + `cargo clippy
  -- -D warnings` + `cargo test` en verde, luego commit `vary2(fase-N): <resumen>`.
> **CORRECCIÓN-E17:** El gobierno del repo manda: ramas `main` + `testing` únicamente; merges solo desde `testing` con CI verde; tests/clippy/release SIEMPRE en CI, nunca como gate local (operativa de PC lento). No crear ramas de larga vida (`vary2` incluida) ni gates locales alternativos sin `question`.
- **Nunca adivinar el dominio xbps-src.** Lo que no esté en el skill `xbps-src` se
  verifica contra el clon local (`~/.cache/vary/void-packages/`) con `grep`/`read`, o
  contra el `Manual.md` upstream vía `webfetch`.
- **Trazas:** `tracing` con niveles; `VARY_DEBUG=1` activa trazas detalladas. Log de
  builds en `~/.cache/vary/logs/<pkg>-<ts>.log`.

---

## PROTOCOLO DE HERRAMIENTAS OpenCode (transversal, todas las fases)

1. **Contexto antes que código:** antes de editar un módulo, `read` completo del archivo y
   `grep` de todos sus call-sites. Prohibido editar a ciegas.
2. **Investigación que contamina poco:** inventarios grandes, scans de `srcpkgs/`, análisis
   de templates → tool `subagent` (agente `explore` para lectura, `general` para
   multi-paso). El contexto principal recibe solo el resumen.
3. **Conocimiento de dominio:** tool `skill` con el skill `xbps-src` (Fase 0 lo crea) antes
   de cualquier trabajo con templates, etc/conf o chroots.
4. **Edición quirúrgica:** `edit` con `oldString` exacto y único para cambios focalizados;
   `write` solo para archivos nuevos o reescrituras completas; refactor multi-archivo →
   serie de `edit` (o `patch` si el modelo activo lo soporta).
5. **Verificación:** `shell` con `workdir` explícito:
   - `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test` → timeout 300000.
   - Builds xbps-src reales → `shell` con `background: true` (el proceso notifica al
     terminar; output completo queda en archivo gestionado).
6. **Ambigüedad de spec → tool `question`,** nunca adivinar. Concretamente en: elegir
   SQLite vs binario plano (Fase 3), política de fallback sin OverlayFS (Fase 13), y
   sintaxis de `package.env` si hay conflicto con tests reales.
7. **Cambio equivocado → `/undo`** (snapshot) antes de parchar encima. `/redo` si te
   pasas de frenada.
8. **Paralelismo de desarrollo:** fases independientes (ver Apéndice E) en `git worktree`
   vía `shell` (`git worktree add ../vary-wt-faseN vary2-faseN`) + una sesión OpenCode por
   worktree. Alternativa nativa: tool `execute` (Code Mode) con `worktree_create`.
9. **Builds largos (xbps-src real):** siempre `background: true`; mientras tanto, la sesión
   trabaja en tests unitarios del mismo módulo. Nunca bloquear el hilo principal 10+ min.
10. **Web:** `webfetch` (markdown) para `https://raw.githubusercontent.com/void-linux/void-packages/master/Manual.md`
    y secciones relevantes; `websearch` solo para decisiones de diseño puntuales.
11. **Diario de fase:** cada sesión termina actualizando `docs/vary2/PROGRESS.md` con
    `write`: qué entró, qué queda, decisiones tomadas. Es el antidoto contra la compaction.

---

## FASE 0 — MONTAJE DE LA SESIÓN (una sola vez, ~30 min)

**Objetivo:** preparar OpenCode para que las demás fases trabajen con conocimiento de
dominio persistente y permisos correctos.

**Herramientas:** `write`, `shell`, `read`, `question`.

Pasos:
1. Ejecutar `/init` sobre el repo de vary: genera `AGENTS.md`. Completarlo con:
   layout de `src/`, MSRV 1.88, comandos (`cargo build/test/clippy`), convenciones
   (errores tipados, RAII, tracing), y la tabla de módulos objetivo (§7.1) resumida.
> **CORRECCIÓN-E16:** PROHIBIDO `/init` en este repo: el AGENTS.md ya existe (71 líneas) y es la ley suprema; `/init` lo pisaría. Toda sesión empieza leyendo AGENTS.md + `docs/vary2/PROGRESS.md`.
2. Crear el skill de dominio `.opencode/skills/xbps-src/SKILL.md` — contenido EXACTO en
   **Apéndice A**. Contiene: variables de `etc/conf`, semántica `build_options` /
   `XBPS_PKG_OPTIONS_<pkg>` (guiones→guiones bajos), `vopt_if`, fases de xbps-src, flags
   `-t` (uchroot OverlayFS), `-I fetch`, `-N`, `-m <masterdir>`, `-r <repo>`,
   `hostdir/binpkgs`, `xbps-rindex`, `common/shlibs`, `dotconfig-custom`, `patches/` +
   `patch_args`.
3. Crear agentes `.opencode/agents/reviewer.md` y `.opencode/agents/tester.md` — contenido
   EXACTO en **Apéndice B**.
4. Crear `opencode.json` con permisos — contenido EXACTO en **Apéndice C**.
5. `shell`: `git status` limpio → crear rama base `vary2` (`git checkout -b vary2`).
> **CORRECCIÓN-E17:** La rama `vary2` está prohibida: el repo gobierna con `main` + `testing` únicamente y merges solo desde `testing` con CI verde. No crear ramas de larga vida sin `question`.
6. Verificar el clon de void-packages: `shell` → `git -C ~/.cache/vary/void-packages rev-parse HEAD`;
   si no existe, `git clone --depth=50 https://github.com/void-linux/void-packages ~/.cache/vary/void-packages`
   (depth bajo: el indexador incremental solo necesita diff de rangos recientes).
> **CORRECCIÓN-E5:** El clon de void-packages lo gestiona vary con `--depth 1` (`src/masterdir.rs`); prohibido crear clones paralelos y prohibido `git diff OLD NEW` sobre un clon shallow (`--depth=50` incluido). Incremental correcto: ver corrección E5 en Fase 3.

**Criterio de aceptación:** `SKILL.md` y agentes existen; `opencode.json` carga sin
advertencias; rama `vary2` activa; clon de void-packages operable.

---

## FASE 1 — RECON DEL MVP (solo lectura)

**Objetivo:** mapa exacto del código actual con los 4 defectos estructurales localizados a
línea concreta. Cero suposiciones.

**Herramientas:** `glob`, `grep`, `read`, `subagent` (`explore`, background), `write` (solo `docs/vary2/`).

Pasos:
1. `glob src/**/*.rs` → inventario de módulos.
2. `grep` (con `-n`, por módulo) de los puntos calientes:
   `official_exists`, `DiGraphMap`, `toposort`, `execute_plan`, `installed.json`,
   `.VURINFO`, `parse_template_file`, `binary-bootstrap`, `xbps-install -Su`,
   `max_concurrent_builds`, `project_into`, `unproject_from`, `InstallType`,
   `elevate`, `PackageSource`.
3. `read` completo de: `resolver.rs`, `db.rs`, `cache.rs`, `vur_client.rs`, `metadata.rs`,
   `install.rs`, `upgrade.rs`, `masterdir.rs`, `config.rs`, `xbps.rs`, `elevate.rs`,
   `command_line.rs`, `main.rs`.
4. `subagent explore` (background) en paralelo: (a) verificar estructura de
   `~/.cache/vary/void-packages/srcpkgs/` y `common/build-style/`; (b) auditar 5 templates
   de muestra — `firefox`, `llvm19`, `curl`, `linux6.12` (o el kernel más reciente del
   clon), `base-system` — extrayendo: `build_options`, `build_options_default`,
   usos de `vopt_if`, `patches/`, `conf_files`, y en kernels la condición
   `{subarch:-$arch}-dotconfig-custom`. Reportar hallazgos como tabla.
5. Escribir `docs/vary2/RECON.md` con: grafo de módulos actual, los 4 defectos con
   `archivo:línea`, inventario de tipos existentes (`InstalledEntry`, `VurInfo` v1,
   `Action`, `BinarySource`), y riesgos de migración.

**Criterio de aceptación:** `RECON.md` cita `archivo:línea` reales para los 4 defectos;
ninguna afirmación sin fuente en el código.
> **CORRECCIÓN-E8:** Este recon describe un MVP que ya no existe: el repo actual ya tiene sets (`src/sets.rs`), overlay RAII (`src/overlay.rs`), síntesis de conf (`src/conf.rs`), política híbrida y `source_always` (`src/resolver.rs`), `repolock` y orden de upgrade. Prohibido recon de un MVP fantasma: todo item empieza con `grep`/`read` del código ACTUAL.

---

## FASE 2 — DATOS PRIMERO: tipos núcleo y migraciones

**Objetivo:** definir TODAS las estructuras de datos de v2 antes de tocar lógica. La lógica
posterior debe poder escribirse casi lineal sobre estas estructuras.

**Herramientas:** `read`, `write`, `edit`, `shell`, `question` (solo si el schema de SQLite
queda ambiguo).

Pasos:
1. Crear `src/types.rs` (o ampliar `src/metadata.rs`) con:
   - `VurInfo v2` (`format_version: 2`), campos nuevos:
     `build_options: Vec<String>`, `build_options_default: Vec<String>`,
     `option_descriptions: BTreeMap<String, String>`,
     `conditional_deps: BTreeMap<String, OptionDeps>`, `sourcepkg: String`,
     `shlib_provides: Vec<String>`, `template_hash: String`, `helpers_hash: String`.
     (`OptionDeps` = pares on/off: deps si la opción está activa / si está inactiva.)
   - `DepPriority { HostMake, Make, Runtime, Check }` (orden = dureza de la arista).
   - `Action { Build, Install(BinarySource) }` y `InstallType` existente, extendidos.
   - `RebuildReason { NewVersion, OptionsChanged, FlagsChanged, PatchChanged,
     EnvProfileChanged, SonameBroken }` (para la auditoría de VarDb).
2. Reemplazar `InstalledEntry` por `VdbEntry` en `src/db.rs` (estructura EXACTA):
   ```rust
   #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
   pub struct VdbEntry {
       pub pkgname: String,
       pub sourcepkg: String,
       pub pkgver: String,
       pub repo_origin: String,
       pub install_type: InstallType,
       pub build_id: u64,
       pub build_time: u64,
       pub counter: u64,
       pub iuse_available: Vec<String>,
       pub active_options: Vec<String>,
       pub cflags: String,
       pub cxxflags: String,
       pub rustflags: String,
       pub ldflags: String,
       pub env_profile: Option<String>,
       pub patches_sha256: Vec<String>,
       pub custom_files_sha256: Vec<String>,
       pub template_sha256: String,
       pub repolocked: bool,
       pub needed_sonames: Vec<String>,
       pub peak_memory_bytes: Option<u64>,
   }
   ```
> **CORRECCIÓN-E3:** `XBPS_RUSTFLAGS` NO EXISTE en xbps-src (verificado en `etc/defaults.conf` y `Manual.md`: solo CFLAGS/CXXFLAGS/FFLAGS/LDFLAGS). Eliminar el campo `rustflags` de los tipos (también de `BinIndexEntry` en el paso 3); para Rust, cargo lee `RUSTFLAGS` del entorno. No modelar knobs que xbps-src no tiene.
3. Definir `BinIndexEntry` (Fase 5 la usa): `build_id`, `pkgver`, `cflags`, `rustflags`,
   `active_options`, `patches_sha256`, `path`, `mtime`.
4. Migración: leer `installed.json` v1 → completar campos nuevos con valores por defecto
   honestos (`build_id: 0`, flags vacías, `repolocked: false`) → escribir `vdb.json`.
   El lector debe aceptar AMBOS formatos para siempre.
5. Tests: roundtrip serde de todas las estructuras; migración de un fixture v1.

**Criterio de aceptación:** `cargo test` verde con fixtures; un `installed.json` v1 real se
carga sin error ni pérdida de campos v1.

---

## FASE 3 — PortDb: árbol de fuentes unificado e indexación incremental

**Objetivo:** indexar `~/.cache/vary/void-packages/srcpkgs/` como repositorio fuente
primario (`main-repo = void`, prioridad `-1000`) + VURs como overlays (prioridad `100` por
defecto), con actualización incremental guiada por git, en <60s frío y <1s incremental.

**Herramientas:** plan mode primero (Tab). Luego `write` (`src/portdb.rs`), `edit`
(`src/metadata.rs`, `src/vur_client.rs`), `shell`, `subagent general` (diseño del parser
estático), `webfetch` (Manual.md §templates).

Pasos:
1. **Store:** `~/.cache/vary/portdb.sqlite` (rusqlite bundled). Tabla `pkgs`: `name`,
   `repo` (`void` | id VUR), `sourcepkg`, `pkgver`, `revision`, `build_options` (JSON),
   `build_options_default` (JSON), `option_descriptions` (JSON), `conditional_deps` (JSON),
   `shlib_provides` (JSON), `template_hash`, `helpers_hash`, `mtime`. Tabla `meta(key,value)`
   con `void_commit_sha` y `helpers_hash_global`.
2. **Index completo (frío):** recorrer `srcpkgs/` en paralelo (`std::thread` + chunking,
   sin runtime tokio aquí): resolver symlink `srcpkgs/<pkg> -> <sourcepkg>`, parsear
   `template` con `parse_template_file` estático (variables, `build_options`,
   `build_options_default`, `depends`/`makedepends`/`hostmakedepends`/`checkdepends`
   literales, `vopt_if` literales). Persistir en una transacción SQLite.
3. **Fallback de precisión:** los templates que el parser estático no pueda resolver
   (expansión bash compleja, `case` anidado sobre `build_options`) → cola limitada de
   `./xbps-src show <pkg>` + `./xbps-src show-options <pkg>` en paralelo (cap: `min(8,
   ncpu)`), solo para ese subconjunto. El resultado SOBREESCRIBE lo estático y registra un
   flag `needs_dynamic=true` para no reintentarlo en cada refresh.
4. **Index incremental:** en cada sync: `git rev-parse HEAD` → `OLD_SHA` guardado en
   `meta`; `git diff --name-only <OLD_SHA> <NEW_SHA> -- srcpkgs/ common/build-style/
   common/build-helper/`; mapear rutas tocadas → paquetes afectados (incluye: cambio en
   `common/build-style/*.sh` → invalidar por `helpers_hash` los paquetes que usan ese
   style; cambio global de helpers → reindex completo); reevaluar solo esos paquetes en
   paralelo.
> **CORRECCIÓN-E5:** Incorrecto sobre un clon shallow (`--depth 1`, gestionado por vary en `src/masterdir.rs`): `git diff --name-only OLD NEW` no funciona y los clones paralelos están prohibidos. Incremental correcto: `git diff-tree --no-commit-id --name-only -r <new_sha>` tras `git pull --ff-only`, cruzado con `helpers_hash` global.
5. **API consultable en memoria:** `PortDb::get(pkg) -> Option<PortEntry>` con resolución
   de overlay: VUR con prioridad mayor gana; `PortDb::providers(soname)` para ShlibsGuard.
6. Tests con fixture: clon real en `--depth=50` + repo sintético de 20 templates en
   `tests/fixtures/srcpkgs/`; test incremental simulando `git commit` nuevo (solo 1
   template cambia → solo 1 reevaluación, medir con contador).

**Criterio de aceptación:** frío <60s sobre el clon real (medir con `time`); incremental
<1s; `helpers_hash` global dispara reindex completo; `xbps-src show` se invoca SOLO para
`needs_dynamic` y esa decisión queda cacheada.

---

## FASE 4 — VarDb extendida y auditoría de discrepancias

**Objetivo:** persistir el estado de compilación real de cada paquete y detectar qué debe
recompilarse aunque `pkgver` no cambie (núcleo de `--newoptions` / `--changed-flags`).

**Herramientas:** `read`, `edit` (`src/db.rs`), `write` (tests), `shell`.

Pasos:
1. Sincronización con el sistema real: poblar `VdbEntry` desde `xbps-query -p
   pkgver build-options shlib-requires <pkg>` (procesar salida con `shell` capture y
   parser tolerante). Campos no expuestos por xbps (flags, parches, perfil) se rellenan
   al INSTALAR desde BinDb (Fase 5) o quedan vacíos para binarios oficiales puros.
2. API de auditoría: `VarDb::audit(entry, target_config, portdb) -> Option<RebuildReason>`:
   - `pkgver` en PortDb > instalado → `NewVersion`.
   - `active_options` ≠ opciones efectivas calculadas → `OptionsChanged`.
   - flags objetivo (cascada Fase 6) ≠ `cflags/cxxflags/rustflags/ldflags` guardados →
     `FlagsChanged`.
   - `patches_sha256`/`custom_files_sha256` ≠ hashes de `~/.config/vary/patches|files`
     → `PatchChanged`.
   - `env_profile` ≠ perfil asignable por `package.env` → `EnvProfileChanged`.
3. `counter` monótono por instalación (orden temporal, análogo a COUNTER de Portage).
4. Tests: fixture vdb + casos que disparen cada `RebuildReason`; caso negativo
   (binario oficial sin customización → sin rebuild).

**Criterio de aceptación:** cambiar en vary.conf `-O2 -march=x86-64` → `-O3 -march=native
-flto=thin` produce `FlagsChanged` para TODOS los paquetes `InstallType::Source` y para
ninguno de los oficiales puros.

---

## FASE 5 — BinDb multi-instancia

**Objetivo:** terminar con la colisión de `hostdir/binpkgs` (mismo `pkgver` recompilado con
distintas flags se pisa) y añadir respeto de opciones al usar binarios.

**Herramientas:** `read`, `write` (`src/bindb.rs`), `edit` (`src/xbps.rs`), `shell`.

Pasos:
1. Índice enriquecido `hostdir/binpkgs/.vary_binindex.json` (lista de `BinIndexEntry`).
   Cada `.xbps` que xbps-src deposite se registra con `build_id` incremental, flags,
   opciones activas y hashes de parches. Escritura atómica (tmp+rename) con lock de archivo.
> **CORRECCIÓN-E13:** Índice paralelo `.vary_binindex.json` DESCARTADO: usar el soporte NATIVO de xbps-src `-r <repo>` (los binpkgs van a `<hostdir>/binpkgs/<repo>`).
2. Subrepositorio local `hostdir/binpkgs/vary-opt` para binarios optimizados; garantizar
   `/etc/xbps.d/10-vary.conf` apunta a él (verificar con `read` antes de tocar) y
   `20-vur-<name>.conf` para cada VUR.
3. Resolución `--usepkg` / `--prefer-binary`: un binario (local o VUR remoto) sustituye a
   la compilación SOLO si sus opciones/flags registradas coinciden con el objetivo
   (`--binpkg-respect-options`). Si no coincide → `Action::Build`.
4. Instalación desde fuente → tras `xbps-install` exitoso, escribir `VdbEntry` completo y
   marcar `repolocked` (Fase 12 ejecuta el repolock real).
5. Tests: dos builds del mismo pkgver con flags distintas → 2 entradas, cero colisiones;
   resolución rechaza binario con opciones no coincidentes.

**Criterio de aceptación:** reconstruir `curl` con `-O3` tras haberlo construido con `-O2`
no sobreescribe el registro previo; `prefer-binary` elige el binario con opciones exactas
o decide compilar.

---

## FASE 6 — Cascada de configuración declarativa

**Objetivo:** replicar la precedencia de Portage sobre archivos declarativos en
`~/.config/vary/`.

**Herramientas:** `read` (`src/config.rs`), `edit`, `write`, `shell`.

Pasos:
1. Parsear `vary.conf` completo (secciones `[general]`, `[build]`, `[make]`, `[qa]`,
   `[search]` — objetivo EXACTO en Apéndice D). Retrocompatible: claves v1 actuales se
   mantienen; claves nuevas opcionales con defaults sensatos.
2. `~/.config/vary/package.options`: `<patrón-pkg> <opciones...>` aceptando `opt`,
   `-opt` (estilo Gentoo) y `~opt` (estilo Void) como desactivación. Wildcards (`linux*`,
   `ffmpeg*`).
3. `~/.config/vary/package.env`: `<patrón-pkg> <perfil>` → `~/.config/vary/env/<perfil>.conf`
   con override de flags/`makejobs` por paquete.
4. `~/.config/vary/package.mask` / `package.unmask` y `~/.config/vary/world` (set de
   seleccionados).
5. Resolución de opciones efectivas por paquete (función única, pura, testeable):
   `effective_options(pkg, conf) = dedup(build_options_default ∪ global_options ∪ package.options[pkg])`
   con regla de última-gana por opción y validación contra `build_options` conocidas
   (opción desconocida → warning a stderr + ignorar, nunca fallo duro).
6. Precedencia documentada y testeada: defaults del template < `[make].global_options` <
   `package.options` (más específico gana; patrón literal > wildcard, orden de declaración
   como desempate).

**Criterio de aceptación:** tests con `firefox` (`pgo wayland ~jack`), `linux*` wildcards,
y caso de conflicto literal-vs-wildcard resuelto de forma determinista.

---

## FASE 7 — EnvSynthesizer: síntesis dinámica de `void-packages/etc/conf`

**Objetivo:** dar a xbps-src soporte completo de `package.env` / `package.options` mediante
generación transaccional de `etc/conf` con bloques `case` por paquete.

**Herramientas:** `write` (`src/env_synth.rs`), `read`, `shell` (incluye `bash -n`),
`skill xbps-src` antes de empezar.

Pasos:
1. Generar `etc/conf` con esta estructura EXACTA (cabecera obligatoria de advertencia
   "generado por vary — editar ~/.config/vary/ no este archivo"):
   ```bash
   # === ARCHIVO SINTETIZADO AUTOMÁTICAMENTE POR VARY 2.0 ===
   XBPS_ALLOW_RESTRICTED=yes
   XBPS_CHROOT_CMD=uchroot
   XBPS_CCACHE=yes

   # 1. Perfil global (vary.conf [make])
   XBPS_CFLAGS="..."
   XBPS_CXXFLAGS="${XBPS_CFLAGS}"
   XBPS_RUSTFLAGS="..."
   XBPS_LDFLAGS="..."
   XBPS_MAKEJOBS=16

   # 2. Opciones globales y por paquete (package.options)
   XBPS_PKG_OPTIONS="..."
   XBPS_PKG_OPTIONS_firefox="..."
   XBPS_PKG_OPTIONS_curl="..."

   # 3. Perfiles por paquete (package.env → env/*.conf)
   case "${sourcepkg:-$pkgname}" in
      glibc|musl|bash|grub|binutils|gcc)
          XBPS_CFLAGS="-O2 -march=native -pipe"
          XBPS_CXXFLAGS="${XBPS_CFLAGS}"
          XBPS_LDFLAGS="-Wl,-O1 -Wl,--as-needed"
          ;;
      firefox|thunderbird|libreoffice|llvm19|rust)
          XBPS_CFLAGS="-O3 -march=native -flto=thin -pipe"
          XBPS_CXXFLAGS="${XBPS_CFLAGS}"
          XBPS_MAKEJOBS=6
          ;;
   esac
   ```
   Los nombres de variable por paquete: guiones → guiones bajos
   (`gst-plugins-bad` → `XBPS_PKG_OPTIONS_gst_plugins_bad`).
> **CORRECCIÓN-E1:** El bloque `case "${sourcepkg:-$pkgname}"` es CÓDIGO MUERTO: xbps-src sourcea `etc/conf` UNA vez al arrancar; el único re-source por paquete ocurre dentro de `set_build_options` (common/xbps-src/shutils/common.sh) en un command substitution (subshell): las asignaciones se descartan y el stdout debe quedar limpio; la única variable por paquete disponible ahí es `XBPS_CURRENT_PKG`. Anulado el criterio de aceptación de F7-v1: los flags por paquete se resuelven con síntesis per-invocación (`etc/conf.<id>` por build, borrado en Drop, `src/conf.rs`).
> **CORRECCIÓN-E2:** Las listas de opciones van SEPARADAS POR COMAS (`opt1,~opt2`), nunca por espacios: con espacios el token no matchea ninguna opción y todas quedan desactivadas en silencio. Ya corregido en vary (commit 3c92440, `src/conf.rs` une con ",").
> **CORRECCIÓN-E3:** La línea `XBPS_RUSTFLAGS="..."` NO EXISTE en xbps-src (verificado en `etc/defaults.conf` y `Manual.md`): quitarla; cargo lee `RUSTFLAGS` del entorno. No modelar knobs que xbps-src no tiene.
2. Si `qa.verify_compiler_flags = true` → inyectar `-frecord-gcc-switches` en
   `XBPS_CFLAGS`/`XBPS_CXXFLAGS` (la auditoría de Fase 16 lo consume).
3. Escritura atómica (tmp en el mismo FS + `rename`), y backup del conf previo solo si fue
   modificado manualmente (detectar diff con la última síntesis guardada → avisar).
4. Ventaja clave a preservar: xbps-src re-evalúa `etc/conf` (source) en cada paquete de la
   cadena interna; el `case "${sourcepkg:-$pkgname}"` garantiza que cada dependencia recibe
   SUS flags sin contaminar al resto. No "optimizar" eliminando el case.
> **CORRECCIÓN-E1:** FALSO. `etc/conf` NO se re-evalúa por paquete (ver corrección E1 del paso 1): el re-source vive en `set_build_options` dentro de un subshell cuyas asignaciones se descartan. No es "ventaja clave a preservar": el `case` es código muerto.
5. Tests: `bash -n` sobre cada conf sintetizado (syntax check obligatorio); idempotencia
   byte a byte; caso con `$sourcepkg` definida vs solo `$pkgname`.

**Criterio de aceptación:** conf sintetizado pasa `bash -n`; un test de integración
invoca `bash -c 'source etc/conf; echo $XBPS_CFLAGS'` con `sourcepkg=firefox` y obtiene el
perfil heavy-lto; el mismo test con `sourcepkg=glibc` obtiene safe-base.
> **CORRECCIÓN-E1:** Criterio ANULADO: `source etc/conf` desde bash no reproduce el comportamiento que se afirma (el `case` por paquete es código muerto; ver corrección E1 del paso 1). Vary resuelve los perfiles con `etc/conf.<id>` por build (`src/conf.rs`).

---

## FASE 8 — UserOverlayGuard: parches y archivos de usuario (RAII)

**Objetivo:** inyectar transaccionalmente parches y archivos (dotconfig-custom, config.h)
en `srcpkgs/<pkg>/` sin ensuciar el árbol git, con limpieza garantizada incluso por señal.

**Herramientas:** `read` (`src/vur_client.rs` — reutilizar `project_into`/`unproject_from`),
`write` (`src/overlay.rs`), `edit`, `shell`.

Pasos:
1. `UserOverlayGuard::arm(void_pkg_dir, pkgname) -> Result<Guard>`:
   - Escanea `~/.config/vary/patches/<pkgname>/` → copia a
     `srcpkgs/<pkgname>/patches/` con prefijo `999-vary-user-` (se aplican DESPUÉS de los
     parches oficiales musl/glibc; `do_patch()` los ordena lexicográficamente).
   - Escanea `~/.config/vary/files/<pkgname>/` → copia a `srcpkgs/<pkgname>/files/`
     (cubre `x86_64-dotconfig-custom` de kernels y `config.h` de suckless).
   - Calcula SHA256 de todo lo inyectado → devuelto para `VdbEntry`.
2. `impl Drop`: restaurar `srcpkgs/<pkgname>/` a estado git inmaculado
   (`git checkout -- <paths>` + `git clean -fd <paths>` limitado al subárbol; si el árbol
   no es repo limpio, copia de seguridad previa en `~/.cache/vary/overlay-backup/`).
3. Señales: instalar handler para `SIGINT`/`SIGTERM` que provoque unwind limpio para que
   corra `Drop` (documentar límite: `SIGKILL` no es recuperable — dejar
   `git status --porcelain srcpkgs/` en el log para auditoría y comando `vary --recover-overlay`
   de limpieza manual). `// ponytail: SIGKILL queda fuera; upgrade path = recovery scan al inicio de cada sesión`.
4. Tests de propiedad: tras `drop()` → `git status --porcelain srcpkgs/<pkg>` vacío;
   test con panic simulado; test de prefijo 999 con `series` inexistente.

**Criterio de aceptación:** build interrumpida a mitad (kill -INT al proceso) deja el árbol
limpio; hashes registrados coinciden con los archivos inyectados.

---

## FASE 9 — Resolutor híbrido: políticas, reducción condicional, virtuals y ciclos

**Objetivo:** convertir `src/resolver.rs` en el equivalente de `depgraph` de Portage. Es la
fase más crítica: plan mode (Tab) + propuesta de diseño en el chat ANTES de editar.

**Herramientas:** plan mode, `read` (resolver.rs completo), `edit`, `write` (tests),
`subagent general` (fixtures de grafos reales), `shell`, `grep` sobre el clon de
void-packages para verificar casos reales.

Pasos:
1. **Política híbrida** — sustituir la parada en `official_exists` por
   `should_build_from_source(pkg, cli_flags, conf, vdb, portdb) -> bool` con esta tabla
   EXACTA de comportamiento:
   | Modo / flag | Comportamiento del DAG |
   |---|---|
   | Por defecto (`vary -S <pkg>`) | Binario oficial si existe, SALVO que `<pkg>` o alguna dependencia tenga regla en `package.options`, `package.env`, `patches/`, `files/`, coincida con `source_always`, o esté en VarDb como `InstallType::Source` → `Action::Build` |
   | `--force-build` / `-B` | `Action::Build` SOLO para los objetivos de la CLI; dependencias satisfacibles con binarios oficiales |
   | `--deep-build` / `--emptytree` (`-e`) | Expande hostmakedepends+makedepends+depends completo, todo `Action::Build`, `-N` a xbps-src |
   | `--newoptions` (`-N`) / `--changed-flags` (`-U`) | Audita VarDb vs config (Fase 4); discrepancies → `Action::Build` |
   | `source_always` en vary.conf | Wildcards (`linux*`, `mesa*`, `firefox`, `llvm*`, `rust`, `wlroots*`, `pipewire`, `ffmpeg*`) siempre fuente |
2. **Reducción condicional (`use_reduce`):** con las opciones efectivas (Fase 6) y
   `conditional_deps` del VurInfo v2, expandir/podar dependencias. Test de verdad:
   `firefox` con `pgo wayland` activas y `~jack ~xscreensaver` inactivas → inyecta
   `xvfb-run dbus mesa-dri vulkan-loader pciutils` y poda `jack-devel libXScrnSaver-devel`.
3. **Normalización de subpaquetes:** toda referencia a subpaquete (`openssl-devel`,
   `libcurl-devel`, `clang19`, `libllvm19`) se resuelve a su `sourcepkg` (los symlinks de
   `srcpkgs/` ya lo indican) ANTES de insertar el nodo en `DiGraphMap`. Test: `llvm19`
   produce UN nodo de build, no veinte.
4. **Virtuals:** cargar `etc/defaults.virtual` del clon + override `etc/virtual`;
   traducción determinista virtual→paquete. Si hay ambiguidad sin override → `question`.
5. **Ciclos (Tarjan):** aristas tipadas `DepPriority { HostMake, Make, Runtime, Check }`;
   sustituir el abort en pila repetida por `petgraph::algo::tarjan_scc`:
   - SCC cerrado solo por aristas `Runtime` → compilar todos los nodos del SCC e instalar
     en UNA transacción `xbps-install` posterior.
   - SCC con aristas duras (`HostMake`/`Make`): (a) buscar paquete de ruptura en PortDb
     (`glib-bootstrap`, `gobject-introspection-bootstrap`, `cargo-auditable-bootstrap` —
     verificar existencia real con `glob` sobre el clon, no asumir nombres); (b) si la
     arista dura viene de una opción condicional (ej. `gir`), plan de dos fases:
     compilar `~opt`, instalar base, recompilar con `opt`.
   - Ciclo irrompible → error tipado con el SCC completo impreso (fail fast, con datos).
6. **Salida:** toposort estable con partición por niveles de paralelismo (input del
   scheduler de Fase 13). `DagPlan { nodes: Vec<PlanNode>, edges, levels }` serializable
   (para `--status` y para tests golden).

**Criterio de aceptación:** tests (a)-(e) de la tabla + fixtures SCC: ciclo
`glib↔gobject-introspection` resuelto vía bootstrap; ciclo runtime-only resuelto con
instalación conjunta; `official_exists` ya NO corta la expansión recursiva de dependencias.

---

## FASE 10 — SetEngine: conjuntos de paquetes

**Objetivo:** soportar `@set` en cualquier operación, con la semántica exacta de Portage.

**Herramientas:** `write` (`src/sets.rs`), `read`, `shell`.

Pasos (fuente de datos por conjunto — implementar y testear cada uno):
- `@system` → expansión de `depends` de `srcpkgs/base-system/template` (via PortDb).
- `@selected` → `xbps-query -m` ∪ `~/.config/vary/world`.
- `@world` → unión deduplicada `@selected` ∪ `@system`.
- `@installed` → `xbps-query -l` mapeado a sourcepkgs.
- `@optimized` → VarDb where `InstallType::Source`.
- `@vur` → paquetes originados de VURs.
- `@preserved-rebuild` → VarDb con binarios enlazando contra `/usr/lib/vary-preserved/`
  (lectura de `~/.local/share/vary/preserved_libs.json` — Fase 15 lo alimenta).
- Patrones wildcard sueltos (`vary -S 'kitty'` y `vary -S 'linux6.*'`).

**Criterio de aceptación:** cada set tiene unit test con fixture; `vary -S @world` parsea y
expande en seco (`--dry-run` si existe; si no, imprimir el plan sin ejecutar).

---

## FASE 11 — CLI y configuración: banderas nuevas sin romper nada

**Objetivo:** exponer todo lo anterior. Contrato v1 intacto.

**Herramientas:** `read` (`src/command_line.rs`, `src/config.rs`), `edit`, `shell`.

Pasos:
1. Añadir a `parse_args()`: `--deep-build` (`-D`), `--emptytree` (`-e`), `--newoptions`
   (`-N`), `--changed-flags` (`-U`), `--force-build` (`-B`), `--jobs <N>` (`-j`),
   `--load-average <F>`, `--status`, `--regen-cache`, `--conf-update`, `--prefer-binary`,
   `--usepkg`. Verificar colisiones de short flags con los existentes (grep de Shorts v1).
> **CORRECCIÓN-E10:** `--newoptions`/`--newuse` SIN short flag (Portage usa `-N`, pero `-N` en xbps-src significa "sin repos remotos" y ya está en uso en vary por `--deep`; la tabla de Fase 9 repite este error). `-U` sí existe en Portage (`emerge -U` = `--changed-use`) pero en vary hay que verificar que esté libre antes de asignarlo.
2. `VaryConfFile`: secciones `[build]`, `[make]`, `[qa]` (Apéndice D) con defaults si
   faltan; validación temprana (fail fast con mensaje accionable, no silencio).
3. Snapshot test de `--help` (output completo como test golden).

**Criterio de aceptación:** todos los comandos v1 (`-S`, `-Syu`, `-Si`, `-R`, etc.)
parsean idéntico; `cargo test` con golden de help.

---

## FASE 12 — `upgrade()` en 3 fases + repolock nativo

**Objetivo:** eliminar la sobrescritura binaria. Nunca más `xbps-install -Su` pisa un
binario optimizado local.

**Herramientas:** `read` (`src/upgrade.rs`, `src/xbps.rs`, `src/elevate.rs`), `edit`,
`write` (stubs de test), `shell`.

Pasos:
1. `src/xbps.rs`: `repolock(pkg)` → `xbps-pkgdb -m repolock <pkg>` y `repounlock(pkg)` →
   `xbps-pkgdb -m repounlock <pkg>`, ambos vía `src/elevate.rs` (wrapper de privilegios
   existente). Llamar a `repolock` inmediatamente tras cada instalación de
   `InstallType::Source`. `--prefer-binary <pkg>` → `repounlock` antes de `xbps-install -f`.
2. Reescribir `upgrade()` en 3 fases:
   - **F1 Sync:** `git pull --ff-only` de void-packages y cada VUR + PortDb incremental
     (Fase 3). Falla de pull no-ff → abortar con instrucción clara (no rebase automático).
   - **F2 Build-optimizados:** cruzar VarDb(`InstallType::Source`) contra PortDb y config
     local → `RebuildReason` → DAG (Fase 9) → compilar a `hostdir/binpkgs` (scheduler
     Fase 13) → `xbps-rindex -a` sobre lo nuevo.
   - **F3 Merge:** `xbps-install -Su` única transacción: los espejos oficiales actualizan
     lo binario y `hostdir/binpkgs` (ya reindexado) provee los optimizados. Paquetes
     `@system` en el plan → barrera `merge_wait` (Fase 13) antes del merge.
3. Tests: stubs de `xbps-pkgdb`/`xbps-install`/`xbps-rindex` en PATH de test (scripts bash
   que logean invocaciones) → verificar ORDEN y argumentos exactos; caso
   "espejo sube revisión de firefox" → firefox NO se toca (repolock), solo se actualiza
   cuando vary lo compila.

**Criterio de aceptación:** la secuencia grabada por los stubs muestra F1→F2→F3 y cero
llamadas a `xbps-install` para paquetes repolockeados salvo desde el repo local.

---

## FASE 13 — AsyncScheduler: paralelismo real con chroots efímeros

**Objetivo:** reemplazar el bucle secuencial de `execute_plan()` por el scheduler tokio de
4 colas, sin corromper el masterdir. Plan mode antes de escribir.

**Herramientas:** plan mode, `read` (`src/install.rs`, `src/masterdir.rs`), `write`
(`src/scheduler.rs`), `edit`, `shell`, `background shell` (builds reales).

Pasos:
1. **Prerrequisito de aislamiento:** build siempre con `./xbps-src -t pkg <pkgname>` y
   `XBPS_CHROOT_CMD=uchroot` (EnvSynthesizer ya lo escribe): OverlayFS efímero por
   trabajador, masterdir base inmutable como lowerdir, `/host` compartido por bind-mount
   (sources, ccache, repocache, binpkgs). Detección de capacidad: test de montaje overlay
   al inicio del plan; si falla → fallback: pool `masterdir-worker-0..N` con `-m <ruta>`
   y `-H <hostdir>` (preguntar vía `question` si el pool debe crearse automáticamente).
> **CORRECCIÓN-E12:** `-t` (OverlayFS efímero) es CONDICIONAL a la capacidad del filesystem (falla en btrfs/zfs): detectar al inicio del plan y usar fallback (pool `masterdir-worker-N` con `-m`/`-H`). Prohibido "siempre -t".
2. **Cola 1 `fetch_queue` (parallel-fetch):** al aprobarse el plan, lanzar en background
   `./xbps-src -I fetch <pkg>` para TODOS los nodos Build en orden topológico (bandera
   `-I`: ignora deps de build, descarga distfiles a `hostdir/sources` verificando SHA256).
3. **Cola 2 `build_queue`:** nodo Ready cuando su fetch terminó Y todos sus predecesores
   hicieron pkg. Gates: `activas < max_concurrent_builds` y `/proc/loadavg` <=
   `load_average_limit`. `-j` dinámico: un solo nodo Ready → todos los hilos; k nodos →
   reparto proporcional (anti-contención).
4. **Cola 3 `rindex_lock_queue`:** semáforo exclusivo alrededor de la fase final de
   indexación (`xbps-rindex -a hostdir/binpkgs/*.xbps`) — evitar carrera entre
   trabajadores; liberar nodos dependientes tras el índice.
5. **Cola 4 `merge_wait_queue`:** paquetes `@system` (glibc, musl, gcc, xbps, openssl) →
   drenar tareas activas antes de ejecutar `xbps-install` en el host.
6. Parsear stdout de xbps-src para detectar fase (fetch/extract/patch/configure/build/
   check/install/pkg) y publicarla por paquete (input de Fase 14).
7. Cancelación limpia: Ctrl-C → cancelar tasks tokio, esperar hijos (con timeout),
   ejecutar Drops (Fase 8), restaurar estado.
8. Tests: planificador con "builds" = `sleep <n>` en stubs → nunca excede jobs, respeta
   loadavg simulada (inyectar lector de loadavg como trait), orden topológico respetado;
   test de cancelación sin huérfanos (`ps` en el harness).

**Criterio de aceptación:** DAG de 20 nodos con anchura 4 se ejecuta con exactamente
`max_concurrent_builds` en vuelo; un nodo pesado solo no deja hilos sin usar (`-j` = total).

---

## FASE 14 — Cgroups v2, scheduler consciente de memoria y observabilidad

**Objetivo:** cero OOM-killer durante LTO/PGO paralelo; telemetría en vivo.

**Herramientas:** `write` (`src/cgroup.rs`, `src/observability.rs`), `edit` (`src/scheduler.rs`),
`shell`, `read` (`/proc` en tests).

Pasos:
1. `CgroupManager`: si `/sys/fs/cgroup` es cgroup v2 → crear
   `/sys/fs/cgroup/vary/vary-<pid>/<pkgname>/` por cada `./xbps-src`; al terminar leer
   `cpu.stat`, `memory.peak`, `io.stat` → persistir `peak_memory_bytes` en VarDb.
   Feature-degradado: sin cgroup v2 → funciona igual, sin telemetría (warning una vez).
2. **Memory-Aware DAG Scheduler:** antes de despachar un nodo:
   `Σ(peak_memory_bytes de builds en vuelo) + peak histórico del candidato > 0.85 ×
   MemAvailable (/proc/meminfo)` → retener en cola. Sin histórico → heurística por perfil
   de `package.env` (heavy-lto → 16 GB default, etc.). Efecto esperado (doc):
   `firefox` (PGO+LTO ~16 GB) y `rust` (~11 GB) en 24 GB de RAM nunca coinciden en vuelo;
   los pequeños sí rellenan huecos.
3. `ObservabilityMonitor`: escritura atómica de `/run/vary/vary-<pid>.json` (estado del
   DAG, fase por paquete, RAM por cgroup, hit-rate de ccache) + socket UNIX
   `/run/vary/vary-<pid>.sock` con eventos NDJSON (uno por línea). `vary --status` se
   conecta al socket del PID vivo e imprime. Línea NDJSON = protocolo mínimo, sin deps.
4. Tests: con cgroup v2 disponible (skip si no); `--status` contra socket de test;
   cálculo de memoria con fixture de `memory.peak`.

**Criterio de aceptación:** en simulación (mock de MemAvailable y de peaks), firefox+rust
nunca comparten vuelo; pequeños llenan huecos; `vary --status` en otra terminal muestra
progreso real.
> **CORRECCIÓN-E11:** Feature OPCIONAL con detección (cgroup v2 montado + delegación de escritura + kernel >= 6.4 para `memory.peak`); sin delegación degrada con warning único. Prohibido prometer "cero OOM matemático": los picos históricos son cota del pasado, no predicción; los criterios solo-mock no cuentan como verificación.

---

## FASE 15 — ShlibsGuard: integridad ABI/SONAME y preserve-libs

**Objetivo:** que un bump de SONAME no rompa el sistema híbrido.

**Herramientas:** `write` (`src/shlibs_guard.rs`), `read` (clon `common/shlibs`), `shell`,
`grep` (fixtures de shlibs reales).

Pasos:
1. **Reconstrucción proactiva:** comparar SONAMEs provistos instalados
   (`xbps-query -p shlib-provides`) contra `common/shlibs` actualizado (PortDb). Bump
   detectado → `xbps-query -X <lib>` (consumidores) × VarDb → todo consumidor
   `InstallType::Source` entra al DAG del mismo upgrade (Fase 12 F2).
2. **Preservación física:** si un consumidor falla al compilar o el upgrade es parcial →
   antes de `xbps-install -u`: copiar `/usr/lib/lib*.so.*` antiguos con consumidores
   vivos → `/usr/lib/vary-preserved/`; registrar `/etc/ld.so.conf.d/vary-preserved.conf`;
   ejecutar `ldconfig` (via elevate); guardar en
   `~/.local/share/vary/preserved_libs.json`.
3. **Prune:** al completar `vary -S @preserved-rebuild` → eliminar de
   `vary-preserved/` las libs sin consumidores restantes (lógica
   `pruneNonExisting`), re-ejecutar `ldconfig`.
4. Tests: fixture de shlibs con bump `libpoppler.so.145→146`; consumidores fuente vs
   binarios; prune tras rebuild completo.

**Criterio de aceptación:** simulación completa: bump de SONAME → consumidores fuente en
DAG; consumidor roto → lib preservada y app sigue ejecutando (`ldd` la resuelve en
vary-preserved); prune limpia tras rebuild.
> **CORRECCIÓN-E14:** La detección de bump de SONAME + lista de consumidores debe correr SIEMPRE antes de la fase de merge (`xbps-install -u`); el grafo de fases v1 lo tenía invertido (ver corrección E14 en Apéndice E).

---

## FASE 16 — QA post-compilación y `--conf-update`

**Objetivo:** certificar que las banderas pedidas llegaron al binario; gestión de
config files protegidos tipo dispatch-conf.

**Herramientas:** `write` (`src/qa.rs`, `src/conf_update.rs`), `shell` (`readelf`), `read`.

Pasos:
1. Auditoría de flags: tras el `pkg`, si `qa.verify_compiler_flags` → extraer
   `.GCC.command.line` de los ELF del paquete (`readelf -p .GCC.command.line <bin>` o
   parser propio de secciones) y/o parsear el log en `~/.cache/vary/logs/`; verificar que
   `-march=native` (o la flags objetivo del perfil) está presente. Ausente → warning en
   reporte final con lista de paquetes "que ignoraron las optimizaciones" y su build-style
   sospechoso.
2. `debug_pkgs`: `XBPS_DEBUG_PKGS=yes` global o por `package.env` → subpaquetes
   `-dbg` con DWARF separado (`/usr/lib/debug/`).
3. `vary --conf-update`: escanear `conf_files` con hash distinto entre lo instalado y la
   nueva versión (`xbps-query -p conf_files` + diff), auto-fusionar cambios triviales
   (comentarios/whitespace), backup en `~/.cache/vary/conf-backup/`, y menú interactivo
   mínimo: `u` nueva versión / `z` conservar / `m` fusión lado a lado / `e` editar
   ($EDITOR). Sin dependencias de TUI pesadas: stdin puro.
> **CORRECCIÓN-E9:** NO auto-fusionar nada (auto-merge de config = corrupción; dispatch-conf nunca auto-mergea). En no-TTY solo listar. `src/conf_update.rs` ya está implementado y validado en Void real: no tocar salvo bug verificado con caso de reproducción.
4. Tests: ELF fixture con `.GCC.command.line` generado en el test; parser de
   `conf_files`; fusión trivial.

**Criterio de aceptación:** paquete construido sin las flags objetivo aparece en el
reporte; `--conf-update` resuelve el caso sintético completo sin colgar stdin no-tty
(testea con input redirigido).

---

## FASE 17 — Integración, E2E y cierre

**Objetivo:** probar el sistema completo en condiciones reales y dejar el repo entregable.

**Herramientas:** `shell` (background para builds reales), `subagent reviewer` sobre el
diff completo, `write` (docs), `read`, `question` (para decidir entorno E2E real).

Pasos:
1. E2E en Void Linux real o contenedor (preguntar al usuario cuál):
   - `curl` con `package.options` propias → verificar VarDb + repolock + auditoría.
   - `st` (suckless) con `~/.config/vary/files/st/config.h` → verifica UserOverlayGuard
     end-to-end (el caso perfecto: build pequeño, rápido, con FILESDIR custom).
   - Kernel `linux6.x` con `x86_64-dotconfig-custom` (opcional, build larga, background).
   - `vary -Syu` completo en el contenedor → orden F1→F2→F3 verificado con paquetes reales.
2. `subagent reviewer` sobre `git diff vary2...main`: buscar wrappers inútiles, panics,
   locks innecesarios, allocations en hot paths, y violaciones del contrato.
3. Docs: actualizar `README.md` (arquitectura de 3 árboles, ejemplos de vary.conf,
   package.options/package.env), `docs/vary2/PROGRESS.md` final.
4. Run completo: `cargo fmt --check && cargo clippy -- -D warnings && cargo test
   --release` en verde.

**Criterio de aceptación (Definición de Hecho global):**
- [ ] Los 4 defectos del MVP eliminados y cubiertos por tests (opacidad oficial, sin
  cascada de config, sobrescritura binaria, ejecución secuencial).
- [ ] PortDb incremental <1s / frío <60s; VarDb audita los 6 `RebuildReason`; BinDb sin
  colisiones y con respeto de opciones.
- [ ] `etc/conf` sintetizado válido (`bash -n`) con perfiles por paquete.
- [ ] Overlay RAII limpia en éxito/error/señal.
- [ ] Resolutor: use_reduce + subpkg→sourcepkg + virtuals + Tarjan SCC con las 3
  estrategias de ruptura.
- [ ] Sets completos; CLI v1 intacta + flags nuevas.
- [ ] upgrade() F1→F2→F3 con repolock; scheduler 4 colas con `-j` dinámico y gates;
  cgroups + memory-aware + `--status`.
- [ ] ShlibsGuard con preserve+prune; QA de flags; `--conf-update`.
- [ ] Suite completa verde; README y docs al día.

---

## PROTOCOLO DE ERRORES Y DESBLOQUEO (todas las fases)

1. `cargo test` rojo → arreglar ANTES de avanzar. Fallo de spec ambigua → tool `question`.
2. `xbps-src` real falla → `shell` background ya notificó; leer el log completo en
   `~/.cache/vary/logs/` con `read` (páginas de 2000 líneas), aislar la fase exacta, y
   reproducir la fase aislada: `./xbps-src <fase> <pkg>`.
3. Contexto de sesión lleno → terminar la unidad en curso, `write` de
   `docs/vary2/PROGRESS.md` (estado, decisiones, siguiente paso exacto), cerrar sesión,
   abrir nueva con CONTRATO + fase. La compaction es con pérdida: PROGRESS.md + commits
   son la única verdad.
4. Cambio destructivo equivocado → `/undo` (repite tantas veces como haga falta), no
   parchar encima del error.
5. NUNCA ejecutar en la sesión: `xbps-remove`, `rm -rf` fuera de `~/.cache/vary/` y
   worktrees propios, ni operaciones sobre `/sys/fs/cgroup` de otros procesos. Los
   permisos de Apéndice C son parte del contrato.

---

## APÉNDICE A — `.opencode/skills/xbps-src/SKILL.md` (crear en Fase 0, contenido EXACTO)

```markdown
---
name: xbps-src
description: Semántica exacta de xbps-src, etc/conf, build_options y chroots de Void Linux
---
# Semántica xbps-src para Vary 2.0

## Entorno de construcción
- `xbps-src` carga `etc/defaults.conf` → perfil `common/build-profiles/${XBPS_MACHINE}.sh`
  → ejecuta `source etc/conf` (¡es Bash ejecutable, no texto plano!).
- `etc/conf` se evalúa con $pkgname/$sourcepkg del paquete en curso ya definidos:
  por eso los bloques `case "${sourcepkg:-$pkgname}"` aplican flags POR PAQUETE.
> **CORRECCIÓN-E1:** FALSO: `etc/conf` se sourcea UNA vez al arrancar; el único re-source por paquete ocurre en `set_build_options` (common/xbps-src/shutils/common.sh) en un command substitution (subshell): asignaciones descartadas y stdout debe quedar limpio; la única variable por paquete ahí es `XBPS_CURRENT_PKG`. Los `case` por paquete son código muerto; vary resuelve los flags con `etc/conf.<id>` por build (`src/conf.rs`).

## Variables de etc/conf
- Flags: XBPS_CFLAGS, XBPS_CXXFLAGS, XBPS_FFLAGS, XBPS_LDFLAGS, XBPS_RUSTFLAGS.
> **CORRECCIÓN-E3:** `XBPS_RUSTFLAGS` NO EXISTE en xbps-src (verificado en `etc/defaults.conf` y `Manual.md`): solo CFLAGS/CXXFLAGS/FFLAGS/LDFLAGS. Para Rust, cargo lee `RUSTFLAGS` del entorno; no modelar knobs que xbps-src no tiene.
- Paralelismo: XBPS_MAKEJOBS (inyecta -jX en $makejobs), XBPS_CCACHE=yes (hostdir/ccache),
  XBPS_DISTCC=yes + XBPS_DISTCC_HOSTS.
- Opciones: XBPS_PKG_OPTIONS (global), XBPS_PKG_OPTIONS_<pkg> (por paquete;
  guiones del nombre → guiones bajos: gst-plugins-bad → XBPS_PKG_OPTIONS_gst_plugins_bad).
> **CORRECCIÓN-E2:** Los valores de XBPS_PKG_OPTIONS / XBPS_PKG_OPTIONS_<pkg> van SEPARADOS POR COMAS (`opt1,~opt2`), nunca por espacios: con espacios el token no matchea y todas las opciones quedan desactivadas en silencio.
- Otras: XBPS_ALLOW_RESTRICTED, XBPS_DEBUG_PKGS, XBPS_CHECK_PKGS=yes|full,
  XBPS_CHROOT_CMD=uchroot.

## Templates (srcpkgs/<pkg>/template)
- build_options / build_options_default (equivale IUSE / +flag de Portage).
- Dependencias: hostmakedepends, makedepends, depends, checkdepends; condicionales con
  $(vopt_if <opt> 'dep_on' 'dep_off').
- Subpaquetes: srcpkgs/<sub> es symlink → sourcepkg; se generan juntos en un solo pkg.
- Parches: srcpkgs/<pkg>/patches/*.{patch,diff} aplicados en do_patch() con patch_args
  (default -Np1); un archivo `series` fija el orden (orden lexicográfico si no existe).
- Kernels: do_configure() usa ${subarch:-$arch}-dotconfig-custom si existe en files/,
  si no el dotconfig genérico.
- Suckless (st, dwm, dmenu): leen files/config.h del FILESDIR.
- conf_files en template = archivos protegidos contra sobrescritura.

## Operación
- Flujo: ./xbps-src binary-bootstrap (una vez) → ./xbps-src pkg <pkg>.
- Salida de paquetes: hostdir/binpkgs (o subdirs como nonfree); indexar con
  xbps-rindex -a hostdir/binpkgs/*.xbps.
- Flags clave: -t (chroot EFÍMERO OverlayFS vía xbps-uchroot: masterdir como lowerdir
  inmutable, upperdir descartable, /host compartido por bind-mount — permite N builds
  paralelas sin corromper el masterdir), -I fetch (descarga distfiles sin deps, verifica
  SHA256), -N (recompilar sin usar caché), -m <masterdir> y -H <hostdir> (rutas
  alternativas, para pool de workers), -r <repo> (repositorio alternativo), -o (opciones
  ad-hoc).
> **CORRECCIÓN-E4:** `-N` significa "Disable use of remote repositories to resolve dependencies", NO "recompilar sin usar caché". Forzar re-ejecución de una fase = `-f` ("Force running the specified stage (configure/build/install/pkg) even if it ran successfully").
- Fases invocables: fetch, extract, patch, configure, build, check, install, pkg.
- shlibs: common/shlibs mapea SONAME→paquete (libc.so.6 glibc-2.41_1); propiedades
  shlib-provides / shlib-requires por binario; dependencias inversas: xbps-query -X <pkg>.
- Virtuals: etc/defaults.virtual (base) + etc/virtual (override local).
- xbps-pkgdb -m repolock <pkg> vincula el paquete a SU repositorio de origen (evita que
  xbps-install -Su lo pise con el binario oficial); repounlock lo libera.
- Instalados: /var/db/xbps; consultar con xbps-query (-p para propiedades: build-options,
  shlib-provides, conf_files); manuales: xbps-query -m; todos: xbps-query -l.

## Verificación
Ante cualquier duda de sintaxis o comportamiento: grep sobre el clon local
~/.cache/vary/void-packages/ (templates reales no mienten) o webfetch del Manual.md
upstream. NUNCA inventar nombres de variables o flags.
```

---

## APÉNDICE B — Agentes custom (crear en Fase 0, contenido EXACTO)

`.opencode/agents/reviewer.md`:
```markdown
---
description: Revisa diffs de vary 2.0 buscando violaciones del contrato (panics, RAII roto, compat v1)
mode: subagent
permissions:
  - action: edit
    resource: "*"
    effect: deny
  - action: shell
    resource: "*"
    effect: deny
---
Eres revisor de código del proyecto vary 2.0 (Rust 2021, MSRV 1.88). Revisa el diff o los
archivos indicados buscando EXCLUSIVAMENTE: (1) unwrap/expect/panic en código de
producción; (2) recursos externos sin guard RAII (árboles git, chroots, cgroups, archivos
sintetizados); (3) rupturas de compatibilidad v1 (flags CLI, installed.json, rutas);
(4) lock/alloc innecesarios en hot paths; (5) datos duplicados que deberían vivir en
PortDb/VarDb/BinDb. Reporta hallazgos por severidad con archivo:línea y el fix mínimo
propuesto. No elogies; solo hechos.
```

`.opencode/agents/tester.md`:
```markdown
---
description: Ejecuta y depura la suite de vary 2.0 (cargo test/clippy/fmt) y reporta fallos con causa raíz
mode: subagent
permissions:
  - action: shell
    resource: "cargo *"
    effect: allow
  - action: shell
    resource: "git *"
    effect: allow
  - action: edit
    resource: "*"
    effect: deny
---
Eres el corredor de pruebas de vary 2.0. Ejecuta en orden: cargo fmt --check, cargo clippy
-- -D warnings, cargo test (con --test <nombre> para aislar). Ante un fallo: extrae el
test mínimo que falla, la aserción exacta y la causa raíz probable (datos vs lógica vs
fixture). NO corrijas código: reporta con trazas recortadas y sugerencia concreta.
```

---

## APÉNDICE C — `opencode.json` (crear/mergear en Fase 0, contenido EXACTO)

```jsonc
{
  "$schema": "https://opencode.ai/config.json",
  "model": "anthropic/claude-sonnet-4-5",
  "permission": {
    "read": "allow",
    "glob": "allow",
    "grep": "allow",
    "edit": "allow",
    "webfetch": "allow",
    "websearch": "allow",
    "shell": [
      { "action": "shell", "resource": "cargo *", "effect": "allow" },
      { "action": "shell", "resource": "git status*", "effect": "allow" },
      { "action": "shell", "resource": "git diff*", "effect": "allow" },
      { "action": "shell", "resource": "git log*", "effect": "allow" },
      { "action": "shell", "resource": "git add*", "effect": "allow" },
      { "action": "shell", "resource": "git commit*", "effect": "allow" },
      { "action": "shell", "resource": "git worktree*", "effect": "allow" },
      { "action": "shell", "resource": "bash -n *", "effect": "allow" },
      { "action": "shell", "resource": "*", "effect": "ask" }
    ]
  }
}
```

Regla: `shell` en `ask` por defecto — este proyecto toca el gestor de paquetes del
sistema; los allowlist cubren el 90% del trabajo diario y la frontera de confianza se
mantiene explícita. Ajusta `model` a tu proveedor real.
> **CORRECCIÓN-E6:** El esquema real de permisos de opencode usa claves por tool (`bash`, `edit`, `read`, `grep`, `glob`, `webfetch`, `skill`, ...) con valor `"ask"|"allow"|"deny"` o un mapa patrón→acción; NO existe la tool `shell` ni la forma `{action,resource,effect}` (el mismo esquema falso está en Apéndice B). Además ya existe una config viva en `~/.config/opencode/opencode.jsonc` (providers, modelos, MCP) que NO debe pisarse. Validar contra el schema de la versión instalada antes de tocar nada; usar `question` si hay duda.

---

## APÉNDICE D — `~/.config/vary/vary.conf` objetivo (spec, no lo crea OpenCode)

```ini
[general]
cache_dir = "~/.cache/vary"
data_dir = "~/.local/share/vary"
log_level = "info"
sudo_bin = ""
sudo_flags = []

[build]
max_concurrent_builds = 4
load_average_limit = 14.0
force_rebuild = false
ephemeral_chroot = true
auto_repolock_source = true
parallel_fetch = true
cgroup_accounting = true
observability = true

[make]
cflags = "-O3 -march=native -mtune=native -pipe"
cxxflags = "-O3 -march=native -mtune=native -pipe"
rustflags = "-C target-cpu=native -C opt-level=3"
ldflags = "-Wl,-O2 -Wl,--as-needed"
makejobs = 12
ccache = true
distcc = false
distcc_hosts = ""
check_pkgs = false
debug_pkgs = false
global_options = ["wayland", "lto", "pulseaudio", "~sndio", "~jack"]
source_always = ["linux*", "mesa*", "firefox", "llvm*", "rust", "wlroots*", "pipewire", "ffmpeg*"]

[qa]
verify_compiler_flags = true
preserve_libs = true

[search]
ttl_cache_seconds = 3600
```
> **CORRECCIÓN-E2:** Las listas de opciones van SEPARADAS POR COMAS (`opt1,~opt2`), nunca por espacios: con espacios el token no matchea ninguna opción y todas quedan desactivadas en silencio. Ya corregido en vary (commit 3c92440, `src/conf.rs` une con ",").

---

## APÉNDICE E — Grafo de dependencias entre fases y plan de paralelización

```
F0 ── F1 ── F2 ──┬─ F3 (PortDb) ──┬─ F9 (Resolutor) ─┐
                 ├─ F4 (VarDb) ───┤                  ├─ F13 (Scheduler) ─ F14 (Cgroups/Obs)
                 ├─ F5 (BinDb) ───┤                  │        │
                 ├─ F6 (Config) ──┴─ F10 (Sets)      │        │
                 ├─ F7 (EnvSynth) ───────────────────┤        │
                 └─ F8 (Overlay) ────────────────────┴─ F12 (upgrade/repolock) ── F17 (E2E)
                                    F11 (CLI) ──────────────────────────────────┘ (transversal, tras F9/F10)
                                    F15 (ShlibsGuard: tras F4+F9) ── F16 (QA: tras F7) ── F17
```

Paralelizable con sesiones OpenCode + `git worktree` (una rama por fase, merge ordenado):
- Vía A: F3 → F4 → F5 (bases de datos).
- Vía B: F6 → F7 (configuración).
- Vía C: F8 → F15 (integridad del árbol).
- Serie obligatoria: F2 → F9 → F13 → F14 → F12 → F17 (el corazón: no paralelizar).
- F11 (CLI) y F10 (Sets) se pueden hacer en cualquier momento tras F9/F10 respectivamente.

Merge final en `vary2` + `subagent reviewer` sobre el diff acumulado antes de F17.
> **CORRECCIÓN-E14:** El orden del grafo está INVERTIDO: la detección de bump de SONAME + consumidores (ShlibsGuard, Fase 15) debe correr SIEMPRE antes de la fase de merge (`xbps-install -u`).
> **CORRECCIÓN-E17:** No hay rama `vary2` (prohibida): el gobierno del repo manda ramas `main` + `testing` únicamente, merges solo desde `testing` con CI verde, y tests/clippy/release en CI nunca como gate local.

