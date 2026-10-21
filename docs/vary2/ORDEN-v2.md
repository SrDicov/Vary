# ORDEN DE EJECUCIÓN — VARY 2.0 · v2 (post-auditoría)
## Directiva permanente para OpenCode · sustituye a PROMPT-MAESTRO-VARY2-OpenCode.md como plan ejecutable

> ESTADO: el prompt maestro v1 queda DEGRADADO a material de investigación. Esta orden es
> el único plan ejecutable. La auditoría del agente (E1–E17) fue verificada y se acepta
> con UNA rectificación (ver §2, nota E10).

---

## §0 — VEREDICTO Y RESPUESTA AL FORK PROPUESTO

La auditoría es correcta en lo esencial: el prompt v1 fue escrito contra el documento de
espec, no contra `xbps-src` ni contra el repo vivo. Fe no verificación. El repo está por
delante del papel (3 de 4 defectos ya cerrados, schema v3, CI verde tras el fix de comas).

Fork propuesto por el auditor ("corregir el doc" vs "empezar por F4"): **se hacen las dos
cosas, en este orden exacto**, porque el doc v1 envenenado sigue en la raíz del repo y
cualquier sesión futura puede leerlo DESPUÉS de AGENTS.md (ya ha pasado una vez):

1. **§1 — Neutralización** (minutos, prioridad de seguridad de contexto).
2. **§3 — Backlog ejecutado en loop continuo** (§4) hasta agotarlo.

---

## §1 — LEY DE PRECEDENCIA DOCUMENTAL (P0, ejecutar primero)

1. **AGENTS.md del repo es la ley suprema.** Este documento no lo reemplaza ni lo
   complementa a escala de gobierno: donde haya contradicción, gana AGENTS.md y el
   conflicto se reporta en el siguiente commit message o `docs/vary2/PROGRESS.md`.
   Gobierno vigente aceptado tal cual: ramas `main` + `testing`; merges solo desde
   `testing` con CI verde; tests/clippy/release en CI, nunca como gate local; operativa
   PC lento respetada.
2. **Mover** `PROMPT-MAESTRO-VARY2-OpenCode.md` de la raíz del repo a `docs/vary2/`.
   Queda untracked y NO se commitea (un plan de trabajo no es verdad del proyecto).
3. **Aplicar las correcciones inline** (§2) dentro del doc movido, marcadas como
   `CORRECCIÓN-E<n>:` al pie de cada bloque afectado, para que ningún lector futuro
   herede las falsedades sin advertencia.
4. **Prohibido `/init`** en este repo (AGENTS.md ya existe, 71 líneas; /init lo pisaría).
   Toda sesión empieza leyendo AGENTS.md + `docs/vary2/PROGRESS.md`.
5. Este documento vive en `docs/vary2/ORDEN-v2.md` y **no se commitea** (misma regla 2).

---

## §2 — CORRECCIONES DE DOMINIO CODIFICADAS (ley operativa a partir de ahora)

| ID | Corrección incorporada como regla |
|----|-----------------------------------|
| E1 | `etc/conf` se sourcea UNA vez al arrancar de xbps-src. NO se re-evalúa por paquete con `$pkgname`/`$sourcepkg` definidos (el re-source de `set_build_options` ocurre en subshell: asignaciones descartadas, stdout debe quedar limpio). **Prohibidos los bloques `case "${sourcepkg:-$pkgname}"`.** Flags por paquete = variables `XBPS_PKG_OPTIONS_<pkg>` (indirección en `setup_pkg`) + síntesis per-invocación (mecanismo actual de `src/conf.rs`: `etc/conf.<id>` por build, borrado en Drop). Anulado el criterio de F7-v1 (`source etc/conf` con `sourcepkg=firefox`) — probaba una ficción. |
| E2 | `XBPS_PKG_OPTIONS` y `XBPS_PKG_OPTIONS_<pkg>` son listas **separadas por COMAS** (`-o option,~option1`; `etc/defaults.conf`). Espacios = token que matchea nada = opciones desactivadas en silencio. Ya arreglado en main (`3c92440`). Nuevo requisito: cualquier serialización futura de opciones usa `","`. |
| E3 | **`XBPS_RUSTFLAGS` NO existe** (verificado en `etc/defaults.conf`: solo CFLAGS/CXXFLAGS/FFLAGS/LDFLAGS). Eliminar `rustflags` de tipos, config, docs y UI. Para Rust: cargo lee `RUSTFLAGS` del entorno; no modelar knobs que xbps-src no tiene. |
| E4 | `-N` = **"Disable use of remote repositories to resolve dependencies"** (README oficial: "avoid using remote repositories completely by using the -N flag"). NO es "recompilar sin caché". Forzar rebuild = `-f` (verificar texto exacto con `./xbps-src -h`). |
| E5 | Clon gestionado con `--depth 1` (`src/masterdir.rs`). Prohibido `git diff OLD NEW` sobre shallow. Incremental = `git diff-tree --no-commit-id --name-only -r <new_sha>` tras `git pull --ff-only`, cruzado con `helpers_hash` global (cambio de helpers → reindex completo). No crear clones paralelos que vary no conoce. |
| E6 | Permisos/config de OpenCode: validar contra el **schema de la versión instalada** y contra la config existente `~/.config/opencode/opencode.jsonc` (providers, modelos, MCP ya viven ahí). Prohibido crear `opencode.json` de proyecto duplicado sin pregunta previa (`question`). El nombre real de la tool de shell y la forma exacta de las reglas (mapa patrón→acción vs lista) se comprueban en la config viva, no en docs de memoria. |
| E7 | `installed.json` está en `CURRENT_SCHEMA_VERSION = 3` con migrador propio (`migrate_legacy_v1_json`) y lector bincode legacy. No reescribir migraciones: extender desde v3. |
| E8 | Los "4 defectos del MVP" ya no describen el repo (sets, overlay RAII, síntesis de conf, política híbrida, repolock y orden de upgrade ya existen). Todo item del backlog empieza con `grep`/`read` del código ACTUAL. Prohibido recon de un MVP que ya no existe. |
| E9 | `conf-update` **sin auto-merge** (auto-merge de config = corrupción). En no-TTY solo lista. `src/conf_update.rs` ya está implementado y validado en Void: no tocar salvo bug verificado con caso de reproducción. |
| E10 | Colisión `-N` real (Portage `--newuse` vs `-N` de xbps-src ya en uso). Corrección: `--newoptions`/`--newuse` **sin short flag**; shorts nuevos solo tras `grep` de los existentes. **NOTA-RECTIFICACIÓN al auditor: `-U` SÍ existe en Portage (`emerge -U = --changed-use`, man emerge).** La conclusión práctica (evitar `-N`, asignar shorts tras verificación) se mantiene; el argumento `-U` era erróneo. Verificar al verificador: también aplica aquí. |
| E11 | Cgroups: feature OPCIONAL con detección (cgroup v2 montado + delegación de escritura disponible + kernel ≥6.4 para `memory.peak`). Sin delegación → degradación silenciosa (warning 1 vez). Prohibido prometer "sin OOM matemático": los picos históricos son cota del pasado, no predicción. Criterios solo-mock NO cuentan como verificación; si no hay hardware real, la feature se marca `experimental` en docs. |
| E12 | `-t` (OverlayFS efímero) es **condicional** a capacidad del FS (falla en btrfs/zfs). Detección al inicio del plan; fallback = pool `masterdir-worker-N` (`-m`/`-H`). Prohibido "siempre -t". |
| E13 | Storage por repo: usar `-r <repo>` nativo (los binpkgs van a `<hostdir>/binpkgs/<repo>`; el repo local ya admite sub-repositorios: debug, multilib…). **Descartado** el índice paralelo `.vary_binindex.json`. |
| E14 | Orden inmutable: detección de bump de SONAME + lista de consumidores SIEMPRE **antes** de la fase de merge (`xbps-install -u`) del upgrade. El grafo de fases v1 tenía esto invertido. |
| E15 | **Cero dependencias nuevas por decreto.** tokio, rusqlite, blake3 FUERA del presupuesto. Persistencia = `BTreeMap` + JSON sobre el cache existente. Cada dependencia nueva pasa por `question` + justificación de medición (no de gusto). |
| E16 | AGENTS.md ya existe y es bueno. Nunca `/init`. |
| E17 | Gobierno del repo (§1.1) manda sobre este documento. El agente no crea ramas de larga vida ni gates locales alternativos sin pregunta. |

Regla de cierre: **cualquier afirmación futura sobre xbps-src se verifica contra el clon
local o `./xbps-src -h` antes de escribirse en código, tests o skills.** El Apéndice A-v2
(§5) es la única fuente de dominio persistente autorizada.

---

## §3 — BACKLOG FINAL (lo único que queda por construir)

Orden estricto. Cada item: recon → plan corto en chat → implementar → validar según
AGENTS.md (CI) → auditoría → siguiente (ver §4).

### ITEM 0 — Neutralización (§1) — minutos
- Mover doc v1 a `docs/vary2/`, aplicar correcciones inline E1–E4, E6, E8, E9, E17.
- Crear `.opencode/skills/xbps-src/SKILL.md` con el contenido EXACTO de §5 (sustituye al
  Apéndice A v1 con sus tres falsedades).
- Actualizar `docs/vary2/PROGRESS.md` (estado, decisiones, apuntando a esta orden).

### ITEM 1 — RebuildReason (núcleo de F4-v1, campo a campo real)
**Valor:** detectar que cambiar `-march`/opciones invalida lo compilado aunque `pkgver`
no se mueva.
- Extender el Entry actual (schema v3) SOLO con campos llenables de verdad:
  `active_options`, `cflags`, `cxxflags`, `ldflags`, `patches_sha256`, `template_sha256`.
  **Sin** `counter` (redundante con build_date), **sin** `rustflags` (E3), **sin**
  `peak_memory_bytes` (depende de feature descartada).
- `RebuildReason { NewVersion, OptionsChanged, FlagsChanged, PatchChanged, TemplateChanged }`.
- API de auditoría: `audit(entry, target_config, portdb) -> Option<RebuildReason>`.
- Conectar a la CLI como `--newoptions` (sin short, E10) y `--changed-flags -U` tras
  verificar que `-U` está libre en vary (grep primero).
- Tests: cambiar cflags en vary.conf → `FlagsChanged` para `InstallType::Source`, nada
  para binarios oficiales puros; cambio de template en PortDb → `TemplateChanged`.
- **Aceptación:** la auditoría distingue las 5 causas con tests sobre el código ACTUAL
  (no sobre el MVP fantasma).

### ITEM 2 — PortDb sin SQLite (núcleo de F3-v1)
- Store: `BTreeMap<SourcePkg, PortEntry>` en memoria + persistencia JSON sobre el
  `src/cache.rs` existente. Cero SQL, cero deps (E15).
- Cold index: walk de `srcpkgs/`, parser estático de templates; fallback
  `./xbps-src show` / `show-options` en paralelo (cap = `nproc`) SOLO para los
  no-parseables, marcados `needs_dynamic` (decisión cacheada).
- Incremental shallow-safe (E5): `git diff-tree --no-commit-id --name-only -r <new_sha>`
  tras `git pull --ff-only`; reindex solo de lo tocado; `helpers_hash` global → full.
- Resolver symlinks de subpaquetes → `sourcepkg` (esto alimenta también ITEM 3).
- API: `get(pkg)` (overlay: VUR gana sobre void), `providers(soname)` (alimenta
  `src/soname.rs`).
- **Aceptación:** medir con `time` sobre el clon real (frío <60s, incremental <1s como
  objetivo de medida, NO como gate de CI); test incremental con fixture sintético donde
  1 commit tocando 1 template reevalúa exactamente 1 paquete.

### ITEM 3 — use_reduce + normalización de subpaquetes (porción útil de F9)
- `use_reduce` puro y testeable: con opciones efectivas + `conditional_deps`, expandir o
  podar dependencias. VERIFICAR los casos de verdad contra el clon (`grep vopt_if
  srcpkgs/firefox/template` etc.) — los ejemplos del doc v1 (xvfb-run/jack) son
  ilustrativos, no especificación.
- Normalización subpaquete→`sourcepkg` antes de insertar nodos en el grafo (evita
  compilar `llvm19` veinte veces).
- Virtuals: cargar `etc/defaults.virtual` + override `etc/virtual` (traducción
  determinista). Tarjan/SCC: **solo si aparece un ciclo real que rompa el resolver hoy** —
  primero capturar el caso que falla (evidencia), después el algoritmo. Sin ciclo
  demostrado, no hay rewrite.
- **Aceptación:** un template real del clon con `vopt_if` resuelve activo/inactivo
  correctamente; `llvm19` (o equivalente real del clon) produce UN nodo.

### ITEM 4 — Detección SONAME completa (mitad útil de F15, E14)
- Extender `src/soname.rs`: comparar `shlib-provides` instalados vs `common/shlibs`
  actualizado (via PortDb ITEM 2); consumidores con `xbps-query -X` cruzado con VarDb →
  lista de rebuild `InstallType::Source` inyectada en el DAG del upgrade **antes del
  merge** (E14).
- **PARADA OBLIGATORIA (`question`)** antes de cualquier preservación física
  (`/usr/lib/vary-preserved/`, `ldconfig`): cirugía de root que requiere caso ABI real
  verificado y aprobación explícita del usuario. Hasta entonces, el output es la lista
  de consumidores, nada más.
- **Aceptación:** fixture con bump `libpoppler.so.145→146` (o SONAME real del clon)
  produce la lista correcta de consumidores fuente; cero escrituras en `/usr/lib`.

### DESCARTADOS (con línea de reactivación)
- **F14 completo (cgroups/observabilidad)** → solo si aparece OOM real en uso y hay
  delegación de cgroups en la máquina objetivo (E11).
- **F13-tokio (rewrite del scheduler)** → el paralelismo experimental actual
  (`<cache>/workers/`) evoluciona solo si hay cuello de botella medido.
- **F5-índice-propio** → `-r` nativo lo cubre (E13). Reactivar solo si `xbps-rindex`
  demostrara insuficiencia.
- **F16-reimplementación** → `conf_update.rs` ya hecho (E9).
- **F7-case / Apéndice A v1** → falsedades, ver E1.
- **Preserve-libs físico** → ITEM 4 con parada obligatoria.

---

## §4 — PROTOCOLO DE EJECUCIÓN CONTINUA (sin parar)

### 4.1 El LOOP
```
ITEM = siguiente del backlog (§3)
1. RECON      grep/read del código ACTUAL implicado (líneas, no suposiciones)
2. PLAN       5–10 líneas en el chat; plan mode SOLO si el item toca 3+ módulos
3. IMPLEMENT  edit/write mínimos; datos antes que lógica; RAII para recursos externos
4. VALIDAR    según AGENTS.md: CI (testing→main con verde); local solo lo que AGENTS.md
              permita (build rápido); NUNCA gates locales alternativos
5. AUDITAR    subagent reviewer sobre el diff del item (panics, RAII, deps, compat)
6. DEBUG      si CI rojo: causa raíz ANTES de push nuevo (ver 4.2). Prohibido push-and-pray
7. REGISTRAR  docs/vary2/PROGRESS.md (estado, decisiones, siguiente item) + commit
              convención del repo
8. NEXT       goto 1 hasta agotar backlog
```

### 4.2 Protocolo de debugging (obligatorio, en orden)
1. Reproducir mínimo (test que falle, invocación corta) antes de tocar código.
2. Leer el log COMPLETO del fallo (`~/.cache/vary/logs/`, output de CI) — no la última
   línea. Páginas con `read` si es largo.
3. Aislar: fase individual (`./xbps-src <fase> <pkg>`), feature flag, o test unitario.
4. Hipótesis única por intento; cada cambio de código = una hipótesis verificable.
5. Si el fallo es de entorno (Void real, FS sin overlayfs, kernel <6.4): documentarlo en
   PROGRESS.md y degradar la feature — no forzar el entorno.
6. Bisect solo con regresión en main y más de 3 commits sospechosos.

### 4.3 Auditoría continua
- **Por item:** reviewer subagent (paso 5 del LOOP). Hallazgos → fix en el mismo item o
  item nuevo al frente del backlog.
- **Al cierre de cada item P0:** checklist §6 completo. Cualquier discrepancia = item
  nuevo con prioridad sobre el backlog restante.
- **Anti-regresión de dominio:** si en cualquier momento una sesión propone código que
  contradiga §2 (case en etc/conf, espacios en PKG_OPTIONS, XBPS_RUSTFLAGS, -N como
  "sin caché"), se rechaza citando la regla y se corrige el origen del error (skill, doc
  o comentario).

### 4.4 Condiciones de PARADA (las únicas cuatro)
1. Cirugía de root no aprobada (preserve-libs, cgroups con delegación, escrituras en
   `/usr/lib`, `/etc/ld.so.conf.d`).
2. Dependencia nueva (E15): `question` + medición.
3. Conflicto entre esta orden y AGENTS.md: gana AGENTS.md, se pausa ese paso y se
   reporta.
4. `main` roto por un merge: reparación inmediata antes de cualquier item nuevo.

Fuera de esas cuatro condiciones, el trabajo continúa item a item sin pedir
confirmación intermedia.

---

## §5 — APÉNDICE A-v2: `.opencode/skills/xbps-src/SKILL.md` (contenido EXACTO)

```markdown
---
name: xbps-src
description: Semántica VERIFICADA de xbps-src (v2, post-auditoría E1-E4; re-verificar contra el clon ante cualquier duda)
---
# Semántica xbps-src para Vary 2.0 — v2

## Protocolo de verificación (obligatorio, siempre)
- Ante cualquier duda de variables/flags: `./xbps-src -h` en el clon local, `grep -n`
  sobre `common/xbps-src/shutils/common.sh`, `etc/defaults.conf` y `Manual.md` del clon.
- NUNCA afirmar una variable, flag o semántica de memoria. El clon no miente; la memoria sí.

## Evaluación de etc/conf (E1 — crítico)
- xbps-src sourcea `etc/conf` UNA vez al arrancar. NO se re-evalúa por paquete con
  $pkgname/$sourcepkg definidos.
- El único re-source ocurre dentro de set_build_options (common/xbps-src/shutils/common.sh)
  en un command substitution (subshell): las asignaciones se DESCARTAN y el stdout debe
  permanecer limpio (un echo contamina pkgopts).
- CONSECUENCIA: bloques `case "${sourcepkg:-$pkgname}"` en etc/conf = código muerto.
  PROHIBIDOS.
- Flags por paquete: xbps-src consulta variables por indirección (setup_pkg) — así
  funcionan XBPS_PKG_OPTIONS_<pkgname>. Para flags de compilador por paquete: síntesis
  per-invocación (vary genera etc/conf.<id> por build y lo borra en Drop).

## Opciones de construcción (E2)
- Listas SEPARADAS POR COMAS: XBPS_PKG_OPTIONS="opt1,~opt2,opt3".
  (README/Manual: `-o option,~option1,~option2`.) Con espacios el token no matchea nada
  y las opciones quedan desactivadas en silencio.
- Por paquete: XBPS_PKG_OPTIONS_<pkgname>; guiones→guiones bajos
  (xorg-server → XBPS_PKG_OPTIONS_xorg_server).
- Consulta: ./xbps-src show-options <pkg>; descripciones en common/options.description.

## Banderas del compilador (E3)
- Existen: XBPS_CFLAGS, XBPS_CXXFLAGS, XBPS_FFLAGS, XBPS_LDFLAGS, XBPS_MAKEJOBS,
  XBPS_CCACHE (hostdir/ccache), XBPS_DISTCC + XBPS_DISTCC_HOSTS, XBPS_SUCMD,
  XBPS_ALLOW_RESTRICTED, XBPS_DEBUG_PKGS, XBPS_CHECK_PKGS=yes|full, XBPS_CHROOT_CMD,
  XBPS_USE_GIT_REVS, XBPS_PKG_COMPTYPE, XBPS_MIRROR, XBPS_INSTALL_ARGS.
- NO existe XBPS_RUSTFLAGS. Para Rust: cargo lee RUSTFLAGS del entorno; no inventar knobs.

## Flags de xbps-src (E4, E12, E13 — texto exacto siempre con ./xbps-src -h)
- -N: Disable use of remote repositories to resolve dependencies (NO es "recompilar sin caché").
- -f: forzar rebuild (verificar semántica exacta en -h).
- -I fetch <pkg>: descargar distfiles ignorando dependencias requeridas (verifica SHA256).
- -t: masterdir EFÍMERO vía xbps-uchroot con OverlayFS. REQUIERE FS con soporte overlayfs
  (falla en btrfs/zfs sin configuración). Uso SIEMPRE condicional a detección de capacidad.
- -m <dir> / -H <dir>: masterdir/hostdir alternativos (pool de workers sin overlayfs).
- -r <repo>: los binpkgs se almacenan en <hostdir>/binpkgs/<repo> (sub-repositorios
  nativos: debug, multilib, ...). USAR ESTO antes de inventar índices paralelos.
- -o <opts>: opciones ad-hoc para esa invocación (comas).
- Fases: fetch, extract, patch, configure, build, check, install, pkg (invocables sueltas).
- binary-bootstrap una vez; paquetes a hostdir/binpkgs; indexar con xbps-rindex -a.

## Integridad y consultas
- common/shlibs: SONAME→paquete mínimo. shlib-provides / shlib-requires por binario.
  xbps-query -X <pkg> = dependencias inversas. xbps-query -p <props> = propiedades
  (build-options, shlib-provides, conf_files...). -m = instalados manuales; -l = todos.
- xbps-pkgdb -m repolock <pkg>: vincula el paquete a su repo de origen (un paquete
  repolockeado no lo pisa xbps-install -u del repo oficial); repounlock libera.
  (Nota: el README oficial sugiere hold; repolock es más fino y existe en xbps-pkgdb.)
- Virtuals: etc/defaults.virtual (base) + etc/virtual (override local: "EDIT etc/virtual
  INSTEAD").
- Subpaquetes: srcpkgs/<sub> es symlink → sourcepkg; se generan juntos en una invocación.
- Parches: srcpkgs/<pkg>/patches/*.{patch,diff} en do_patch() con patch_args (default
  -Np1); archivo `series` fija orden. Kernels: files/${subarch:-$arch}-dotconfig-custom.
  Suckless (st, dwm, dmenu): files/config.h. conf_files en template = protegidos.
```

---

## §6 — CHECKLIST DE AUTO-AUDITORÍA (correr al cierre de cada item P0)

| # | Comprobación | Cómo se verifica |
|---|--------------|------------------|
| 1 | Ningún `case "${sourcepkg:-$pkgname}"` generado por vary | `grep -rn 'sourcepkg:-' src/` → solo en comentarios/documentación si acaso |
| 2 | Opciones serializadas con comas | `grep -rn 'XBPS_PKG_OPTIONS' src/` → join con `","`; test de regresión 3c92440 sigue en verde |
| 3 | Cero referencias a XBPS_RUSTFLAGS | `grep -rn 'RUSTFLAGS' src/ docs/` → solo comentarios E3 o código de entorno Rust directo |
| 4 | `-N` usado solo con semántica "sin repos remotos" | `grep -rn '"-N"' src/` y revisión de contexto |
| 5 | Incremental PortDb usa diff-tree (no git diff de rango) | leer el módulo; test con clon `--depth 1` |
| 6 | Sin clones de void-packages paralelos | `grep -rn 'clone' src/masterdir.rs` → solo la ruta gestionada |
| 7 | Config OpenCode intacta y única | no existe `opencode.json` de proyecto sin pregunta; `~/.config/opencode/opencode.jsonc` sin pisar |
| 8 | Schema v3 respetado | no hay re-escritura de migraciones; tests de migración existentes en verde |
| 9 | conf-update sin auto-merge | `grep -rn 'merge' src/conf_update.rs` → solo fusión interactiva/listado |
| 10 | Shorts de CLI sin colisiones | test golden de `--help` + grep de shorts |
| 11 | Cero deps nuevas en Cargo.toml vs presupuesto repo | `git diff Cargo.toml` |
| 12 | AGENTS.md intacto y leído al inicio de sesión | no diff en AGENTS.md sin decisión explícita |
| 13 | Orden E14 respetado en upgrade | la detección SONAME corre antes del merge; verificar en tests del flujo |
| 14 | -t condicional | el path de build comprueba capacidad overlayfs antes de usar -t |

Discrepancia en cualquiera → item nuevo al frente del backlog, antes de continuar.
