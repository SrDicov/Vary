# Changelog

All notable changes to vary will be documented in this file.

## 0.4.1 (2026-09-07) — P1-3 + pulido (pendiente tag)

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
