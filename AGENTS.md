# AGENTS.md

Vary: gestor de paquetes comunitarios (helper VUR) para Void Linux — port de [paru](https://github.com/Morganamilo/paru) (Arch). Rust, crate único. Rama por defecto: `vary-mvp`. El historial de paru vive en el remote `upstream`; nunca hagas merge/rebase contra él sin pedido explícito.

## Comandos

- Test completo: `cargo test` — 68 passed / 4 ignored (los ignorados requieren Void Linux real + `xbps-query`; pasan en contenedores Void).
- Un solo test: `cargo test <filtro>` (p. ej. `cargo test resolver`, `cargo test vur_sin_binario`).
- Release: `cargo build --release` (`lto=true`, `codegen-units=1` — lento a propósito).
- No hay rustfmt/clippy configurados; la verificación es build + test. Corre `cargo test` tras cada cambio.
- MSRV fijado en DOS sitios: `Cargo.toml` (`rust-version = "1.88"`) y `.github/workflows/ci.yml` (`dtolnay/rust-toolchain@1.88`) — cámbialos juntos. No bajar de 1.88: la dependencia transitiva `time@0.3.x` lo exige.

## CI

- `ci.yml` — test + build release en push/PR.
- `xbps.yml` — construye `.xbps` para glibc+musl en contenedores oficiales de Void, verifica cada paquete funcionalmente en un contenedor FRESCO (instalar + `vary -V/-h/--repo list`) y publica GitHub Release al pushear tag `v*`.
- Gotcha de contenedores Void: `voidlinux/voidlinux:latest` apunta a `alpha.de.repo.voidlinux.org` cuyo certificado TLS no coincide — hay que reescribir `/etc/xbps.d/00-repository-main.conf` a `https://repo-default.voidlinux.org/current` y correr `xbps-install -u xbps -y` antes de instalar nada (la imagen musl no necesita esto). Esa lógica vive en `.github/scripts/{build,verify}-xbps.sh` — edita ahí, no YAML inline.

## Arquitectura

Entrada: `src/main.rs` → `vary::run()` (`src/lib.rs:58`) → `Config::new()` → `handle_cmd()`; las operaciones de sync se despachan en `handle_sync()` (`src/lib.rs:126`).

- `resolver.rs` — resolver DAG puro, sin I/O; dependencias inyectadas vía `trait PackageSource`. **Los paquetes del repo oficial son hojas** (no se recursa en sus deps); solo `Action::Build` expande hostmakedepends+makedepends+depends.
- `metadata.rs` — esquema `.VURINFO` v1 + validación (parsea objeto único o array).
- `vur_client.rs` — clona repos VUR, fusiona índice desde `srcpkgs/*/.VURINFO` + `.VURINFO` raíz + fallback parseando template; pkgname duplicado = error. Copia (no symlink) las plantillas a `void-packages/srcpkgs/`.
- `bootstrap.rs`/`masterdir.rs` — idempotentes: clonar void-packages → `binary-bootstrap` → escribir `/etc/xbps.d/10-vary.conf`. `-Syu` auto-bootstrapea.
- `config.rs`/`command_line.rs` — flags compatibles con pacman + subcomandos `--repo`.
- Todos los binarios externos pasan por config: `git` (`--git`), sudo (`--sudo`). Nunca invoques `std::process::Command` sobre ellos directamente.

## Protocolo VUR

- Reglas duras del esquema v1 (en `metadata.rs`): `format_version == 1`; version con regex `^[A-Za-z0-9._+]+$` (sin guiones); `revision >= 1`; `archs` no vacío; checksums con prefijo `sha256:` o `SKIP`; los subpaquetes heredan archs del padre y no pueden duplicar su nombre.
- `scripts/vur-generator.sh` DEBE correr dentro de un checkout de void-packages con masterdir bootstrapped, usando `./xbps-src show <pkg> | jq -f scripts/vurinfo.jq`. Nunca hagas `source template` en el host.
- El `.VURINFO` generado refleja solo los `XBPS_PKG_OPTIONS` por defecto (limitación MVP aceptada).

## Config y rutas

- `~/.config/vary/vary.conf` (ejemplo en `etc/vary.conf.example`); archivo corrupto advierte, no aborta.
- `~/.config/vary/repos.conf` gestionado con `vary --repo add|list|remove|rekey`; gana el `priority` más bajo; `vary --repo add <url>` deriva el nombre del último segmento de la URL sin `.git`.
- Caché: `~/.cache/vary/` (checkout void-packages, caché de búsqueda con clave `name:sha`, installed.json); clones VUR en `~/.local/share/vary/vurs/`.

## Gotchas

- En runtime necesita `git` (verificado en bootstrap) y herramientas xbps; si falta `xbps-query` sale el error "¿estás en Void Linux?".
- Debug: `VARY_DEBUG=1`; backtraces: `RUST_BACKTRACE=1`.
- Resolución de arquitectura: override `--arch` → `xbps-query -R -p architecture base-system` → `std::env::consts::ARCH`.
- Un `.VURINFO` inválido se salta con warning, nunca es fatal.
- Builds secuenciales en MVP (`max_concurrent_builds` es placeholder).

## Proyectos complementarios (mismo autor)

- [aur2xbps](https://github.com/SrDicov/aur2xbps) transpila AUR → plantillas de void-packages (lee solo `.SRCINFO`, nunca PKGBUILDs). La relación con vary es a nivel de documento: los mantenedores lo usan para generar templates, los usuarios usan vary. Ver `docs/AUR2XBPS.md`. La automatización planeada `vary --repo regen-aur` NO está implementada (roadmap fase 2).
