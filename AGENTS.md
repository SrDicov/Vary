# AGENTS.md

Vary: gestor de paquetes comunitarios (helper VUR) para Void Linux — port de [paru](https://github.com/Morganamilo/paru) (Arch). Rust, crate único. Modelo de ramas: `main` = estable/release-ready (solo merges desde `testing` con CI verde; tags `v*` desde `main`), `testing` = integración (todo feature/fix, CI obligatorio). El historial de paru vive en el remote `upstream` (puede no estar configurado en este clon); nunca hagas merge/rebase contra él sin pedido explícito.

## Comandos

- Test completo: `cargo test` (suite unitaria; 4 `#[ignore]` en `src/xbps.rs` requieren Void Linux real + `xbps-query`, se corren con `cargo test -- --ignored` según `audit/VALIDATION.md`).
- Un solo test: `cargo test <filtro>` (p. ej. `cargo test resolver`).
- Gates CI exactos: `cargo fmt --all -- --check` + `cargo clippy --all-targets -- -D warnings` + `cargo test` (en local, ver "Operativa PC lento").
- Release: `cargo build --release` (`lto=true`, `codegen-units=1` — lento a propósito; NUNCA en el PC dev, solo CI por dispatch/tags).
- MSRV fijado en DOS sitios: `Cargo.toml` (`rust-version = "1.88"`) y `.github/workflows/ci.yml` (`dtolnay/rust-toolchain@1.88`) — cámbialos juntos; no bajar de 1.88.
- Docs bilingües: todo cambio user-facing actualiza `README.md` + `README.es.md`; todo cambio de formato VUR actualiza `docs/VURINFO.md` + `src/metadata.rs` (ver `CONTRIBUTING.md`).
- `test/plan_*.json`, `test/RESULTS.md` y `test/SMOKE-*.md` son evidencia manual de runs en Void real, no una suite ejecutable.

## Definition of Done (auditoría 2026-09-06, vigente)

- Cada hallazgo se cierra con run CI verde en su commit de CIERRE: `fmt` + `clippy --all-targets -- -D warnings` + `test`. **La validación local no cuenta.** Los pushes intermedios en rojo se admiten solo como cadena fix-forward trazada (el verde cabeza valida el árbol final; no amend de commits pusheados). Commits solo-docs (`audit/**`, `docs/**`, `roadmap/**`, `*.md`) no disparan CI (paths-ignore) y quedan exentos. Excepción histórica aceptada 2026-09-06 (T0.1/D7): fixes previos a los gates con cobertura acumulativa de suites verdes posteriores.
- Fixes se citan por subject (`git log --oneline --grep="H-###"`); FIX_LOG + AUDIT_REPORT se actualizan EN el mismo commit que el código.
- Un hallazgo se cierra solo con su run verde; la tabla de `AUDIT_REPORT.md` debe cuadrar con `git log` en todo momento (cero huecos).

## Operativa PC lento (vigente)

- En local SOLO: editar + `cargo fmt` + `git push`. `cargo check` únicamente como diagnóstico. Tests SIEMPRE en CI, nunca local (compila toda la crate).
- `cargo build --release`, `cargo test` completo y `cargo clippy` completos, prohibidos en local salvo caída de CI (registrar desviación).
- Push serial: mientras CI valida N, el fix N+1 se commitea local SIN push. Nunca dos pushes concurrentes (el `concurrency` cancela el run anterior y se pierde su evidencia).
- Verificación: `gh run watch <id>` / `gh run list --workflow CI`; ante rojo, `gh run view --job <id> --log-failed` y fix-forward (no amend de commits pusheados).

## CI

- `ci.yml` — gates del DoD en push/PR (paths-ignore para solo-docs): `fmt --check` + `clippy --all-targets -- -D warnings` + `test` en paralelo; `concurrency` cancela runs viejos; `rust-cache` en jobs pesados; job `release` solo por dispatch/tags.
- `xbps.yml` — construye `.xbps` para glibc+musl en contenedores oficiales de Void, verifica cada paquete funcionalmente en un contenedor FRESCO (instalar + `vary -V/-h/--repo list`), publica GitHub Release al pushear tag `v*`, y mantiene el release rodante `repo` (URL fija con repodata firmado por arch; clave privada en el secret `XBPSSIGN_PRIVKEY`, pública en `keys/vary-repo.pub.pem`).
- Gotcha de contenedores Void: `voidlinux/voidlinux:latest` apunta a `alpha.de.repo.voidlinux.org` cuyo certificado TLS no coincide — hay que reescribir `/etc/xbps.d/00-repository-main.conf` a `https://repo-default.voidlinux.org/current` y correr `xbps-install -u xbps -y` antes de instalar nada (la imagen musl no necesita esto). Esa lógica vive en `.github/scripts/{build,verify}-xbps.sh` — edita ahí, no YAML inline.

## Arquitectura

Entrada: `src/main.rs` → `vary::run()` (`src/lib.rs`) → `Config::new()` → `handle_cmd()`; las operaciones de sync se despachan en `handle_sync()` (`src/lib.rs`).

- `resolver.rs` — resolver DAG puro, sin I/O; dependencias inyectadas vía `trait PackageSource`. **Los paquetes del repo oficial son hojas** (no se recursa en sus deps); solo `Action::Build` expande hostmakedepends+makedepends+depends.
- `metadata.rs` — esquema `.VURINFO` v1 + validación (parsea objeto único o array).
- `vur_client.rs` — clona repos VUR, fusiona índice desde `srcpkgs/*/.VURINFO` + `.VURINFO` raíz + fallback parseando template; pkgname duplicado = error. Layouts: `srcpkgs/<pkg>`, alias `pkgs/<pkg>` (voiders) o flat (subdirs de nivel 1 como paquetes si no hay prefijo). Lee índices con `git show HEAD:<path>` sin materializar; solo el paquete a compilar se materializa (sparse-checkout). Copia (no symlink) las plantillas a `void-packages/srcpkgs/`. `detect_default_branch()` lee la rama por defecto del remoto (`git ls-remote --symref`); `repo add` la usa salvo `--branch` explícito.
- `vup_index.rs` — adaptador binario Fase 1 para repos estilo VUP (index.json): descarga con curl, convierte a `VurInfo` sintéticos solo para la arch actual, decodifica la llave de `keys/*.plist`. Sin compilación desde fuente (layout `srcpkgs/<cat>/<pkg>` no soportado), sin `-Si`, sin tracking en installed.json.
- `bootstrap.rs`/`masterdir.rs` — idempotentes: clonar void-packages → `binary-bootstrap` → escribir `/etc/xbps.d/10-vary.conf`. Por defecto builds secuenciales con `XBPS_MAKEJOBS` (`[build] makejobs`, default `nproc`) y pre-fetch paralelo de distfiles del DAG. Paralelismo por niveles solo con `--experimental` + `max_concurrent_builds > 1`: sandboxes overlayfs en `<cache>/workers/w<N>`, artefactos fusionados, y fallback al camino secuencial histórico si faltan prerequisitos (overlayfs/espacio). El bootstrap corre en flujos de instalación (`-S <pkg>`), NO en `-Syu` plano; `xbps-src` se niega a correr como root, así que los flujos completos requieren usuario normal + wrapper de elevación.
- Señales y exclusión: `signal.rs` — el handler solo marca un `AtomicI32`; un hilo observador mata hijos por grupo (`kill(-pid)`, cubre SIGINT+SIGTERM) y desmonta overlays registrados; `xbps-src` corre con `setpgid` propio. `lock.rs` — `flock` exclusivo en `<cache_dir>/vary.lock` (una instancia; el mensaje cita el pid).
- `install.rs` — `official_exists` usa bulk fast-path (`xbps::bulk_official_names`, snapshot por sesión contra repos oficiales) con confirmación escalar en cada miss; el bulk acelera, nunca decide.
- `config.rs`/`command_line.rs` — flags compatibles con pacman + subcomandos `--repo`.
- `git` (`--git`/`git_bin`), `curl` (`--curl`/`curl_bin`) y sudo (`--sudo`/`sudo_bin`) son configurables: el código nuevo debe pasar por `Config`, nunca hardcodear otro binario para ellos. Quedan hardcodes conocidos fuera de `Config` (`review.rs`: `bat`/`less`; `install.rs`: `xbps-query`; `masterdir.rs`: `mount`) — no los tomes como patrón.

## Protocolo VUR

- Reglas duras del esquema v1 (en `metadata.rs`): `format_version == 1`; version con regex `^[A-Za-z0-9._+]+$` (sin guiones); `revision >= 1`; `archs` no vacío; checksums con prefijo `sha256:` o `SKIP`; los subpaquetes heredan archs del padre y no pueden duplicar su nombre.
- `scripts/vur-generator.sh` (referencia aún NO validada en Void real) DEBE correr dentro de un checkout de void-packages con masterdir bootstrapped, usando `./xbps-src show <pkg> | jq -f scripts/vurinfo.jq`. Nunca hagas `source template` en el host.
- El `.VURINFO` generado refleja solo los `XBPS_PKG_OPTIONS` por defecto (limitación MVP aceptada).

## Config y rutas

- `~/.config/vary/vary.conf` (ejemplo en `etc/vary.conf.example`); archivo corrupto advierte, no aborta.
- `~/.config/vary/repos.conf` gestionado con `vary --repo add|list|remove|rekey`; gana el `priority` más bajo; `vary --repo add <url>` deriva el nombre del último segmento de la URL sin `.git`, autodetecta la rama (`--branch` la fuerza) y acepta `--index-url` para repos estilo VUP.
- Caché: `~/.cache/vary/` (checkout void-packages, caché de búsqueda con clave `name:sha`, installed.json); clones VUR en `~/.local/share/vary/vurs/`.

## Gotchas

- En runtime necesita `git` (verificado en bootstrap) y herramientas xbps; si falta `xbps-query` sale el error "¿estás en Void Linux?".
- Debug: `VARY_DEBUG=1`; backtraces: `RUST_BACKTRACE=1`.
- Resolución de arquitectura: override `--arch` → `xbps-query -R -p architecture base-system` → `std::env::consts::ARCH`.
- Un `.VURINFO` inválido se salta con warning, nunca es fatal.
- Builds secuenciales por defecto. `max_concurrent_builds > 1` no hace nada sin `--experimental` (y sin overlayfs/espacio degrada con warning). `--challenge` también exige `--experimental`.
- Elevación de privilegios agnóstica (`src/elevate.rs`): binario explícito gana → root (uid 0) ejecuta sin wrapper → autodetección en PATH sudo→doas→run0. `sudo_bin` vacío = auto; configurable vía `--sudo`/`--sudoflags` o `[general] sudo_bin/sudo_flags` en vary.conf. Ojo: `preflight.rs` aborta root directo fuera de un contenedor OCI (`sudo vary` sí funciona: `main.rs` baja a `SUDO_UID`).

## Proyectos complementarios (mismo autor)

- [aur2xbps](https://github.com/SrDicov/aur2xbps) transpila AUR → plantillas de void-packages (lee solo `.SRCINFO`, nunca PKGBUILDs). La relación con vary es a nivel de documento: los mantenedores lo usan para generar templates, los usuarios usan vary. Ver `docs/AUR2XBPS.md`. La automatización planeada `vary --repo regen-aur` NO está implementada (roadmap fase 2).
