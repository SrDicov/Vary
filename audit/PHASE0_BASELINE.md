# PHASE 0 — Línea Base del Proyecto `vary`

**Fecha y hora:** 2026-09-06
**Rama git:** `vary-mvp`
**Commit HEAD:** `1e32f1f fix(tests): unwrap list_packages Result in pkgs layout test`

---

## 1. Mapa del Repositorio

### Conteo de Líneas por Módulo (`src/`)

```
   131 src/args.rs
   102 src/bootstrap.rs
    87 src/cache.rs
   469 src/command_line.rs
   297 src/config.rs
   137 src/db.rs
   146 src/elevate.rs
    52 src/help.rs
   104 src/info.rs
    31 src/init.rs
   494 src/install.rs
   233 src/keys.rs
   195 src/lib.rs
    81 src/lock.rs
   112 src/logging.rs
    68 src/main.rs
   324 src/masterdir.rs
   465 src/metadata.rs
    40 src/remove.rs
   206 src/repo.rs
   212 src/reposconf.rs
   603 src/resolver.rs
    71 src/review.rs
   211 src/search.rs
   232 src/signal.rs
   111 src/upgrade.rs
    42 src/util.rs
   364 src/vup_index.rs
  1017 src/vur_client.rs
   610 src/xbps.rs
------
  7247 total (30 módulos)
```

### Estado de Git (`git status`)

```
On branch vary-mvp
Your branch is up to date with 'origin/vary-mvp'.

Changes not staged for commit:
	modified:   AGENTS.md
	modified:   Cargo.toml
	modified:   src/install.rs
	modified:   src/lib.rs
	modified:   src/masterdir.rs
	modified:   src/xbps.rs

Untracked files:
	guia
	src/lock.rs
	src/signal.rs
```

### Historial Reciente (`git log --oneline -n 10`)

```
1e32f1f fix(tests): unwrap list_packages Result in pkgs layout test
b89fd43 feat(vur): branch autodetect, VUP binary adapter, pkgs/ layout (0.2.5)
65fe6f0 fix(build): relock Cargo.lock for v0.2.4
94d78f4 fix(resolver): support 'noarch' and empty archs in metadata correctly
7ca5677 feat(vur): implement partial clones and sparse checkout for VUR sync
6fa620c release: v0.2.3 — fix review showing all templates
759bbb4 fix(review): show specific package template instead of repo HEAD commit
8d7f767 Merge Release 0.2.2
17955c5 chore(release): version 0.2.2 and architecture optimizations
d8ebd27 chore: bump version to 0.2.1
```

### `Cargo.toml` Completo

```toml
[package]
name = "vary"
version = "0.2.5"
authors = ["Dicov (SrDicov) <https://github.com/SrDicov>"]
edition = "2021"

description = "Void User Repository (VUR) helper and build automator for Void Linux"
homepage = "https://github.com/SrDicov/Vary"
repository = "https://github.com/SrDicov/Vary"
license = "GPL-3.0"
keywords = ["void-linux", "xbps", "vur", "packaging", "helper"]
include = [
    "src/**/*",
    "LICENSE",
    "README.md",
    "README.es.md",
    "docs/**/*",
    "etc/**/*",
    "scripts/**/*",
    "template",
]
rust-version = "1.88"

[dependencies]
ansiterm = "0.12.2"
anyhow = { version = "1.0.100", features = ["backtrace"] }
base64 = "0.22"
dirs = "6.0.0"
petgraph = "0.6"
serde = { version = "1.0.228", features = ["derive"] }
serde_json = "1.0.149"
sha2 = "0.10"
tempfile = "3.24.0"
toml = { version = "0.9.10", features = ["preserve_order"] }
heed = "0.20.0"
tracing = "0.1"
tracing-appender = "0.2"
tracing-subscriber = { version = "0.3", features = ["env-filter", "fmt"] }

[profile.release]
codegen-units = 1
lto = true

[target.'cfg(target_env = "musl")'.dependencies]
mimalloc = { version = "0.1", default-features = false }

[target.'cfg(unix)'.dependencies]
nix = { version = "0.29", features = ["user", "signal", "process", "fs"] }
```

---

## 2. Resultados de Verificación de Compilación y Calidad

### A. `cargo check`
- **Código de salida:** `0` (Éxito)
- **Advertencias:** 3 advertencias de dead code:
```text
warning: field `path` is never read
  --> src/cache.rs:19:5
   |
18 | pub struct CacheIndex {
   |            ---------- field in this struct
19 |     path: PathBuf,
   |     ^^^^
   |
   = note: `#[warn(dead_code)]` (part of `#[warn(unused)]`) on by default

warning: field `path` is never read
  --> src/db.rs:27:5
   |
26 | pub struct InstalledDb {
   |            ----------- field in this struct
27 |     path: PathBuf,
   |     ^^^^

warning: methods `get` and `names` are never used
  --> src/db.rs:68:12
   |
32 | impl InstalledDb {
   | ---------------- methods in this implementation
...
68 |     pub fn get(&self, name: &str) -> Option<Entry> {
   |            ^^^
...
83 |     pub fn names(&self) -> Vec<String> {
   |            ^^^^^
```

### B. `cargo clippy --all-targets -- -D warnings`
- **Código de salida:** `101` (Fallo)
- **Estado de herramienta:** Componente `clippy` requirió instalación mediante `rustup component add clippy`.
- **Total de errores generados:** 57 errores (53 en biblioteca principal, 4 en tests).
- **Categorías principales:**
  - `clippy::unnecessary-cast` (ej. `sig as i32` en `src/signal.rs:135`)
  - `clippy::collapsible-if` (ej. `src/vur_client.rs:484`)
  - `clippy::while-let-on-iterator` (ej. `src/vur_client.rs:572`)
  - `clippy::while_immutable_condition` (ej. `src/vur_client.rs:593`)
  - `clippy::manual-is-multiple-of` (ej. `src/vur_client.rs:620`)
  - `clippy::obfuscated-if-else` (ej. `src/search.rs:69`)
  - `clippy::manual-unwrap-or-default` (ej. `src/search.rs:94`)
  - `clippy::useless-format` (ej. `src/metadata.rs:379`, `391`)
  - `clippy::single-element-loop` (ej. `src/vur_client.rs:826`, `847`)
  - `clippy::print_literal` (ej. `src/repo.rs:148`)

### C. `cargo fmt --check`
- **Código de salida:** `1` (Fallo)
- **Estado de herramienta:** Componente `rustfmt` requirió instalación mediante `rustup component add rustfmt`.
- **Archivos desformateados:** Múltiples módulos afectados (`src/vur_client.rs`, `src/xbps.rs`, `src/main.rs`, etc.).

### D. `cargo test`
- **Código de salida:** `0` (Éxito)
- **Resultado:** 89 tests en total: 85 pasados, 0 fallidos, 4 ignorados (los ignorados corresponden a integración real con Void Linux / `xbps-query`):
  - `xbps::tests::integracion_query_architecture_real`
  - `xbps::tests::integracion_query_installed_real`
  - `xbps::tests::integracion_query_manual_real`
  - `xbps::tests::integracion_search_remote_real`

### E. `cargo build --release`
- **Código de salida:** `0` (Éxito)
- **Tiempo:** 10m 08s (debido a `lto = true`, `codegen-units = 1`)
- **Binario generado:** `target/release/vary`
- **Advertencias:** Las mismas 3 advertencias de dead code detectadas en `cargo check`.

---

## 3. Observaciones Preliminares del Estado

1. **Estado de Refactorización Previa Incompleta:**
   - Existen cambios no commiteados en el árbol de trabajo (`src/install.rs`, `src/lib.rs`, `src/masterdir.rs`, `src/xbps.rs`, `Cargo.toml`, `AGENTS.md`) y archivos sin seguimiento (`src/lock.rs`, `src/signal.rs`, `guia`).
   - Dependencias requeridas por el plan original (como `diffy`, `chrono`, `indicatif`, `tree-sitter-bash`) **no** están presentes en `Cargo.toml`.
   - El paralelismo de compilación (R1) está completamente ausente en `src/install.rs`: el bucle de compilación sigue siendo un `for` estrictamente secuencial (`md.build_pkg(...)`).
   - El lockfile de instancia única y el manejo de señales están parcialmente implementados en `src/lock.rs` y `src/signal.rs`, pero requieren auditoría rigurosa.
2. **Higiene de Código:**
   - Clippy arroja 57 advertencias tratadas como error bajo `-D warnings`.
   - `cargo fmt` no está aplicado al código.
   - Existen campos y métodos muertos en `src/cache.rs` y `src/db.rs`.
