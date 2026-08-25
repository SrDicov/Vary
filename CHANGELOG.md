# Changelog

All notable changes to vary will be documented in this file.

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
