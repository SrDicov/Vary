# Changelog

All notable changes to vary will be documented in this file.

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
