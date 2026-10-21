# Changelog

All notable changes to vary will be documented in this file.

## Unreleased

- **Índice de fuentes (PortDb sin SQL):** `-Sy` actualiza también el clon de
  void-packages (estaba congelado en el commit del clone) y refresca
  `~/.cache/vary/portdb.json`: versiones, opciones de build y el mapa
  `SONAME -> proveedor` de `common/shlibs`. Incremental (solo los templates
  tocados por el commit nuevo); un cambio en `common/build-style` o
  `common/build-helper` dispara reindex completo. Los templates con versión
  calculada se marcan `needs_dynamic`.
- **Virtuals:** `etc/defaults.virtual` + override local `etc/virtual`:
  `vary -S ntp-daemon` instala el paquete real que lo provee.
- **Deps condicionales reducidas:** los `$(vopt_if ...)` de los templates se
  parten sin romperlos y se reducen con tus opciones activas antes de
  entrar en el grafo (antes se colaban como nombres de paquete basura:
  `$(vopt_if`).
- **Bumps de SONAME del árbol de fuentes:** antes del merge del upgrade se
  cruzan los pins de soname con `common/shlibs`: los paquetes fuente que
  requieren un SONAME cuyo proveedor cambió entran en el plan de
  recompilación de esa misma transacción. **No** se preserva nada en
  `/usr/lib` (requiere caso ABI real y aprobación explícita).

- **Auditoría de rebuild (`--newoptions`, `--changed-flags -U`):** vary graba con
  qué se compiló cada paquete instalado desde fuente (opciones activas,
  banderas, hashes de los parches/ficheros de usuario y hash del template) y
  detecta cuándo quedó obsoleto aunque el `pkgver` no se mueva:
  `--newoptions` recompila por opciones/parches/template cambiados,
  `--changed-flags` por banderas distintas. Sin baseline (paquetes
  anteriores a esta función) no hay señal: no se inventan rebuilds. Los
  binarios oficiales nunca se recompilan por esto. Sin short para
  `--newoptions`: `-N` ya significa "sin repos remotos" en xbps-src.
- **Fix `XBPS_PKG_OPTIONS`:** xbps-src separa la lista de opciones por
  **comas** (`IFS=','` en `set_build_options`), pero vary la emitía con
  espacios: el token entero no matcheaba ninguna opción y todas las que no
  estuvieran en `build_options_default` quedaban **desactivadas en
  silencio** (`[make] global_options` y `[options] <pkg>`). Ahora se
  emiten separadas por coma.
- **`--conf-update`:** resuelve los ficheros `<fichero>.new-<pkgver>` que xbps
  deja en `/etc` tras una actualización: escaneo recursivo (sin symlinks),
  `git diff --no-index` por fichero con TTY y pregunta
  `[r]eemplazar [k]eep [d]iff [q]uit`; al reemplazar deja backup
  `<original>.vary-bak.<pid>` con `cp -p` antes del `mv`, siempre vía
  `elevate` (sudo/doas/run0). Sin TTY o con `--noconfirm` solo lista
  `nuevo -> original` y sale 0 (no escribe nada). Con TTY es el momento de
  revisar y decidir por fichero; `q` corta. Comando independiente: no se
  combina con `-S`/`-R`, targets ni `--repo`; no toca ficheros `.so`
  (esos los cubre `@preserved-rebuild`). Detalles en README.
- **Fix parseo de `xbps-src show`:** el formato real emite
  `version:\tX`/`revision:\tY`; el parser solo aceptaba `version=`/
  `revision=`, así que los builds oficiales forzados y los upgrades
  `source_always` abortaban con "no trae version/revision parseables".
  Ahora acepta ambos separadores.
- **Set `@preserved-rebuild`:** recompilación explícita de los paquetes
  compilados por vary cuyos pins de sonames reportan drift (P2-soname):
  `vary -S --force-build @preserved-rebuild`. El aviso en `-Syu` ahora además
  sugiere el set cuando hay drift. **No hay auto-rebuild** (compilar sin
  consentimiento es peligroso); todavía no preserva el `.so` viejo en disco.
  Detalles en README.
- **Overlay de parches y archivos de usuario:** antes de cada build desde
  fuente, vary copia `~/.config/vary/patches/<sourcepkg>/*` a
  `srcpkgs/<sourcepkg>/patches/` con prefijo `999-vary-user-` (para que tus
  parches corran después de los oficiales del template), y
  `~/.config/vary/files/<sourcepkg>/*` a `srcpkgs/<sourcepkg>/files/`
  (respaldando con `.vary-bak.<pid>.<nombre>` los ficheros que ya existían).
  Al terminar el build —éxito, error o interrupción controlada
  (`SIGINT`/`SIGTERM`)— el árbol se restaura a su estado previo (restauración
  RAII; con `SIGKILL`/abort no hay `Drop` y pueden quedar residuos); se ignora
  todo lo que no sea fichero regular (symlinks y
  dirs no se copian). Sin registro en la base de datos; el restore no toca
  nada que el build haya dejado extra en esos dirs.
- **`--deep` / `--deep-build`:** pasa `-N` a xbps-src (`xbps-src -N`): construye
  sin repos remotos, compilando las dependencias que falten desde fuente y
  reutilizando el repo local `hostdir/binpkgs` ya indexado (viaja por las
  vías secuencial y paralela, incluidos builds oficiales). Puede fallar si
  falta la fuente o el distfile de alguna dependencia; flag solo de CLI
  (sin opción en `vary.conf`). Con esto se cierra el `--deep` que los sets
  dejaban pendiente. Detalles en README.
- **Sets `@name` en `-S`:** targets `@selected`, `@installed`, `@system`,
  `@world`, `@optimized` y `@vur` se expanden a listas de paquetes antes de
  resolver (mezclables con nombres literales, dedup preservando orden;
  set vacío = error). Guardia de template en `BuildOfficial`: un oficial a
  compilar sin `srcpkgs/<pkg>/template` en el checkout degrada a binario
  official con warning. `@system` = `base-system`; construir su árbol de
  dependencias desde fuente (`--deep`) pendiente. Detalles en README.

## 0.4.1 (2026-09-07) — P1-3 + pulido (pendiente tag)

- **Compilación híbrida oficial:** `[build] source_always` (globs) emite
  `BuildOfficial` para oficiales matcheados (nodo hoja: deps delegadas a
  xbps-src); `[make]` + `[options]` vía conf de xbps-src por invocación
  (`etc/conf.vary-<id>` + `-c`, guard RAII); `--force-build` sobre oficial
  sin template VUR ahora compila el oficial; `repolock` tras instalar algo
  desde fuente y `repounlock` al pisarlo con binario (oficial o VUR) sobre
  una entrada Source; para un nombre matcheado el binario firmado VUR nunca
  gana (se compila); `-Syu`
  re-ancla fuentes y recompila oficiales `source_always` con template más
  nuevo. `RUSTFLAGS` no soportado nativamente. Detalles en README.
- **R1 cumplido / H-015 cerrado:** builds paralelos por niveles con overlay
  por slot (estilo xbps-fbulk); `>1 slot` solo con `--experimental` +
  capacidad; índice único en main thread; degradación fail-safe a
  secuencial. Sin experimental, bit-idéntico a 0.4.0.
- `challenge` pide confirmación (defecto No; `--yes` procede).
- Diario simétrico (REMOVE solo si hubo rastreo); musl dinámico
  documentado; tests de `verify_if_pinned`.
- Fixes del vivo: materialize refresca path a HEAD (no más builds con
  plantilla vieja); prefetch materialize serializado (index.lock).
- Deuda restante tras 0.4.1: ninguna del roadmap (0.5.0 por definir).

## 0.4.0 (2026-09-07) — cierre del roadmap post-auditoría

10/11 requisitos contractuales (solo R1 ausente, diferido a P1-3/0.5.0).
Evidencia: `audit/FIX_LOG.md` (Apéndice 0.3.1/0.4.0), `test/RESULTS.md`,
`test/SMOKE-0.4.0.md`. Mini-auditoría pre-tag: 0 hallazgos mayores.

### Garantías (fail-closed, verificadas)
- TOFU continuo: rotación de llave/URL en repos registrados aborta el
  refresh con forense + `vary --repo re-trust` (rechaza `--yes`).
- `vary -Sp`: plan congelado real (sin lock/DB/elevación); semántica honesta
  documentada (el índice VUP refresca caché como `-Sy`).
- Preflight de entorno (root/uchroot abortan; chroot/OCI avisan).
- Orden binario/fuente determinístico (`--prefer-binary`/`--force-build`);
  avisos cuando un flag no tiene efecto.
- Pinning (`repo_commit` + `artifact_sha256`, schema v3) y pins de sonames
  al instalar; drift y soname-drift solo AVISAN (nunca auto-rebuild).
- Sin `unwrap`/`expect` en rutas nuevas; no-TTY/`--yes` sin sorpresas
  (EOF = denegación; `--yes` no oculta hallazgos de auditoría).

### Heurísticos (útiles, no garantías)
- `template_audit`: checksum-ausente + descargas-en-build; consultivo, con
  falsos negativos/positivos posibles fuera del corpus. No sustituye revisar.
- `vary challenge`: compara árboles tras rebuild; bit-reproducibilidad
  observada en lavat, no prometida en general. **Sin guardarraíl de
  tamaño/tiempo: un challenge compila de verdad** (puede tardar horas).
- Avisos de drift/soname/lock: advisory, pueden callar sin baseline.

### No-ítems decididos
- Resiliencia 404 en distfiles: no implementada (capa equivocada; el fetch
  con reintentos/mirrors lo ejecuta `xbps-src`).
- P1-3 (masterdirs aislados + H-015): único ítem de 0.5.0.
- Artefacto musl (`vary-*_1.x86_64-musl.xbps`) dinámico (intérprete
  `/lib/ld-musl-x86_64.so.1`): exige Void musl para ejecutarse, en glibc no
  corre (verificado por el job `verify musl` en contenedor musl).

## Unreleased (auditoría FASE 4–5, 2026-09-06)

Auditoría integral cerrada: 46/47 hallazgos + A3 + A6 (resta H-015, diferido
a P1-3 por decisión PC-2). Detalle: `audit/FIX_LOG.md`.

### Added

- `--yes` como alias de `--noconfirm` (`-y` sigue siendo refresh).
- Puerta de revisión de templates en upgrades (A3): diff unificado con
  `diffy` + consentimiento por paquete; no-TTY aborta salvo `--yes`.
- Descubrimiento de monorepos `srcpkgs/<cat>/<pkg>` (A6, profundidad 2).
- `[tools] install_bin` configurable; `$PAGER` para el revisor.
- Respeto XDG (`XDG_*_HOME` con fallback a `$HOME`).
- Spinner `indicatif` + salida de `xbps-src` a `<cache>/logs/xbps-src.log`.
- Niveles de log recargables (`RUST_LOG` > `-v` > `log_level`).
- Códigos de salida POSIX: parseo → 2, binario ausente → 127, señal → 128+sig.

### Fixed

- Señales: sin hijos huérfanos (máscara en spawn) ni `/tmp/vary-*` residuales.
- EOF en no-TTY explica con hint `--noconfirm` en vez de abortar en silencio.
- Avisos de índice/plantilla inválidos llegan a stderr (antes invisibles).
- Lock solo en operaciones mutantes; mensaje sin "borra" (anti split-brain).
- `--sudoflags` de CLI reemplaza `vary.conf`; directorios propios con 0700.
- Nombres virtuales de `provides` resueltos al real en install/review/DB.
- `unwrap`/`expect` en rutas de usuario convertidos a errores (incluye
  abandono de privilegios); `Config::default()` puro sin I/O.
- Flags `--asdeps`/`--asexplicit` y rutas pacman (`--config`…) rechazados
  con mensaje en vez de ignorarse en silencio.
- Sanitización de entorno en elevación + wrapper en ruta absoluta.

### Test intensivo 0.3.0 en Void real (2026-09-07, `test/REPORT.md`)

- Llave VUP leída vía git (clones sparse), `install -D` crea `keys/`,
  `archs=all/noarch` aceptados en install, TOFU fail-closed con llave
  ilegible, DB solo para Build/VulBinary, `--color never` sin estilos.
- Verificados en vivo: 19 installs (8 binarios VUP con TOFU, 9 fuentes,
  spotify, hytale), migración LMDB→JSON, Ctrl+C sin huérfanos (130),
  lock con PID, upgrade-detect, ciclo remove/reinstall, lifecycle de repos.

### Known issues (hito 0.3.1)

- (vacío: T-002 no-reproducible + T-010/T-012 corregidos con CI verde y
  verificación en vivo; ver `audit/FIX_LOG.md`).

## 0.2.5 - 2026-09-05

### Added

- `vary --repo add` accepts `--branch <rama>` and `--index-url <url>`; the
  remote default branch is auto-detected (`main`, `master`, …) via
  `git ls-remote --symref` with fallback to `main`.
- VUP-style binary index adapter (Phase 1, `src/vup_index.rs`): repos
  publishing `index.json` install binaries for the current architecture
  without templates; key decoded from `keys/*.plist` and verified like any
  VUR key. New `index_url` field in repos.conf, `--curl` / `curl_bin` config.
- `pkgs/` accepted as alias of `srcpkgs/` in VUR layouts
  (voiders-community/repository): listing, index fallback, materialize,
  project and review.

### Fixed

- `repo add` `.VURINFO` check inspected the (always empty) `--no-checkout`
  worktree, warning even when the repo publishes an index; it now inspects
  the git object store and recognizes the root `.VURINFO` array.

## 0.2.1 - 2026-08-29

### Fixed

- CLI robustness and pipe panic (PR #1): no more fatal "Broken pipe" panic
  (`exit 101`) when output is piped to `head`/`less`/`true`; `--git` is now
  honored end-to-end; invalid `--color`/`--arch` are rejected; better
  architecture-incompatibility messages; `--repo remove <name> -p` purges
  orphan clones; `--noconfirm` defaults to `-Syu` and bare `-S` reports missing
  targets correctly.

## 0.2.0 - 2026-08-25

### Added

- Privilege-escalation agnosticism (`src/elevate.rs`): any `BIN [flags] cmd args…`
  wrapper works — `sudo`, `doas`, `run0`. Resolution order: explicit config wins
  (`--sudo`/`--sudoflags` or `[general] sudo_bin/sudo_flags` in vary.conf) →
  running as root executes directly with no wrapper → otherwise auto-detection
  in PATH (`sudo` → `doas` → `run0`) with an actionable error when none exists.

### Fixed

- CI packaging: `.xbps` now declares a versioned `git>=x.y` dependency (bare
  names fail XBPS transaction parsing: "can't guess pkgname for dependency").
- Docs: correct `repos.conf` TOML format in READMEs (`[vur.<name>]`, not
  `[[repo]]`); clarified that auto-bootstrap happens on install flows, not on
  plain `-Syu`.

## 0.1.0 - 2026-08-24

First public MVP release on the `vary-mvp` branch.

### Added

- VUR protocol client: Git-based repos with declarative `.VURINFO` v1 index (per-template and root variants), cached by commit SHA.
- Dependency resolver: pure DAG over `petgraph`, official packages as leaves, topological build order, cycle detection, `provides` fallback, arch filtering.
- Binary installs from signed VUR repos: RSA key discovery (`keys/*.rsa|pem`), SHA256 fingerprint verification against `repos.conf`, `/etc/xbps.d/20-vur-<name>.conf` registration.
- Source builds via `xbps-src`: auto-bootstrap of `void-packages` + masterdir, template projection into the master tree with cleanup markers.
- pacman-style CLI: `-S`, `-Ss`, `-Si`, `-Sw`, `-Syu`, `-R`, `--repo add|list|remove|rekey`, `--force-build`, `--prefer-binary`.
- User config: `~/.config/vary/vary.conf` and `~/.config/vary/repos.conf` (TOML).
- Reference tooling: `scripts/vur-generator.sh` + `scripts/vurinfo.jq` for CI-side `.VURINFO` generation.

### Credits

Fork of [paru](https://github.com/Morganamilo/paru) (GPL-3.0) by Morganamilo and contributors; ported to Void Linux/XBPS by [Dicov (SrDicov)](https://github.com/SrDicov).
