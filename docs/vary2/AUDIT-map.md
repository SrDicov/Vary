# AUDIT — mapa del proyecto (FASE 0)

Verdad del proyecto en el momento de auditar: `testing` = `main` = `8c14628`.
**21298 líneas de Rust en `src/` (45 módulos), 177 `pub fn`, 20 crates, 0 features opcionales.**

> Tabla regenerada desde el propio árbol de `8c14628` (cada fila = un fichero de
> `src/`; la suma cuadra con `wc -l src/*.rs`). La primera versión de esta
> tabla listaba 43 filas (faltaban `portdb.rs` y `elevate.rs`) y la matriz de
> cobertura omitía `vup_index.rs`: dos listas que no se deacuerdan
> ([finding del revisor](https://github.com/SrDicov/Vary)) y un agujero de
> cobertura. Corregido el 2026-10-22 contra los datos reales.

Método: `wc -l`, `grep` de call sites por crate, `grep` de `use crate::` para
el grafo de dependencias, extracción de `pub fn` para la superficie pública.
Ninguna cifra de este documento es estimación.

## 1. Módulos × LOC × superficie × crates

| módulo | LOC | `pub fn` | crates que usa | consumidores (`use crate::`) |
|---|---:|---:|---|---|
| `args.rs` | 166 | 0 | — | — |
| `bootstrap.rs` | 115 | 3 | anyhow, tracing | masterdir |
| `cache.rs` | 120 | 0 | anyhow, serde, serde_json, tempfile | metadata |
| `challenge.rs` | 357 | 2 | anyhow, ruzstd, sha2, tar | — |
| `command_line.rs` | 1055 | 4 | anyhow | args, config |
| `conf.rs` | 284 | 4 | anyhow, tracing | config |
| `conf_update.rs` | 245 | 1 | anyhow, tracing | config |
| `config.rs` | 666 | 1 | ansiterm, anyhow, dirs, serde, tempfile, toml, tracing | args |
| `db.rs` | 727 | 3 | anyhow, serde, serde_json, tempfile, tracing | — |
| `elevate.rs` | 218 | 4 | anyhow, dirs | — |
| `help.rs` | 75 | 1 | — | — |
| `info.rs` | 158 | 1 | anyhow | cache, config, reposconf, vur_client |
| `init.rs` | 59 | 1 | anyhow | — |
| `install.rs` | 2334 | 3 | anyhow, sha2, tempfile, tracing | bootstrap, cache, config, db, metadata, reposconf, resolver, util, vur_client, xbps |
| `journal.rs` | 143 | 4 | anyhow, tempfile | — |
| `keys.rs` | 1118 | 21 | anyhow, base64, md5, sha2, tempfile, tracing | reposconf, util, vur_client |
| `lib.rs` | 453 | 3 | ansiterm, anyhow, dirs, tracing | config, db |
| `lock.rs` | 89 | 1 | anyhow, nix, tempfile | — |
| `lockfile.rs` | 362 | 4 | anyhow, serde, tempfile, toml | config |
| `logging.rs` | 191 | 5 | tempfile, tracing, tracing_appender, tracing_subscriber | — |
| `main.rs` | 93 | 0 | anyhow, mimalloc, nix | — |
| `masterdir.rs` | 596 | 8 | anyhow, dirs, nix, tempfile, tracing | xbps |
| `metadata.rs` | 523 | 4 | anyhow, serde, serde_json, tar, tracing | — |
| `overlay.rs` | 439 | 2 | anyhow, dirs, sha2, tempfile, tracing | — |
| `portdb.rs` | 841 | 9 | anyhow, serde, serde_json, sha2, tempfile, tracing | — |
| `preflight.rs` | 344 | 2 | nix | — |
| `query.rs` | 108 | 3 | anyhow | config |
| `rebuild.rs` | 363 | 5 | sha2 | config, db |
| `remove.rs` | 94 | 1 | anyhow, tracing | config, db |
| `repo.rs` | 509 | 1 | anyhow, tempfile, toml, tracing | command_line, config, keys, reposconf, util, vur_client |
| `reposconf.rs` | 255 | 0 | anyhow, serde, tempfile, toml, tracing | — |
| `resolver.rs` | 1687 | 2 | anyhow, petgraph, tracing | metadata |
| `review.rs` | 173 | 3 | anyhow, tempfile | vur_client |
| `search.rs` | 245 | 1 | anyhow, tracing | cache, config, reposconf, vur_client, xbps |
| `sets.rs` | 410 | 5 | anyhow | config, db, soname |
| `signal.rs` | 388 | 10 | indicatif, nix, tempfile, tracing | — |
| `soname.rs` | 476 | 9 | — | — |
| `template_audit.rs` | 349 | 3 | tar | — |
| `upgrade.rs` | 715 | 2 | anyhow, diffy, tracing | cache, config, db, reposconf, template_audit, vur_client, xbps |
| `use_reduce.rs` | 267 | 4 | tar | — |
| `util.rs` | 164 | 5 | anyhow, tempfile | config |
| `vup_index.rs` | 433 | 8 | anyhow, base64, serde, serde_json, tempfile, tracing | metadata |
| `vur_client.rs` | 1702 | 1 | anyhow, base64, tar, tempfile, tracing | cache, metadata, reposconf |
| `why.rs` | 158 | 2 | anyhow, tracing | cache, config, resolver |
| `xbps.rs` | 1031 | 21 | anyhow, indicatif, nix, serde, tempfile, tracing | config |

Total: **45 módulos, 21298 LOC, 177 `pub fn`.**

## 2. Crates (Cargo.toml): 20, todos con call site

| crate | call sites | notas |
|---|---:|---|
| `anyhow` | 132 | — |
| `tempfile` | 70 | — |
| `serde` | 52 | — |
| `serde_json` | 21 | — |
| `tar` | 20 | metadata, template_audit, challenge, use_reduce, vur_client |
| `base64` | 13 | keys, vup_index, vur_client |
| `nix` (unix) | 14 | lock, main, preflight, signal, masterdir, xbps |
| `toml` | 14 | config, lockfile, repo, reposconf |
| `dirs` | 12 | config, elevate, masterdir, overlay, lib |
| `indicatif` | 7 | xbps, signal |
| `sha2` | 7 | keys, challenge, install, overlay, portdb, rebuild |
| `ansiterm` | 6 | config, lib |
| `tracing-appender` | 5 | logging |
| `tracing-subscriber` | 4 | logging |
| `petgraph` | 3 | resolver (único grafo) |
| `diffy` | 2 | upgrade (único uso: diff de plan) |
| `ruzstd` | 2 | challenge (zstd de `.xbps`) |
| `md-5` | 1 | keys (`md5::Md5::digest`, replica `xbps_pubkey2fp`) |
| `tracing` | many | — |
| `mimalloc` (musl) | 1 | main |

**Sin call site: ninguno.** `md-5` es el que más se presta a confusión: el
crate se importa como `md5`, no `md_5`; se usa en `keys.rs:70`.

## 3. Perfiles y features

- `[profile.release]`: `codegen-units = 1`, `lto = true`. **Sin** `panic =
  "abort"`, **sin** `strip`, **sin** `overflow-checks` explícito.
- `[profile.dev]`: default de cargo (overflow-checks y debug-assertions activos).
- Sin sección `[features]`: no hay features opcionales ni-activated.
- `mimalloc` solo en `cfg(target_env = "musl")`; `nix` solo en `cfg(unix)`.

## 4. Grafo de dependencias interno (resumen)

```
lib.rs ─┬─ config ─── args
        ├─ db ── (serde_json, tempfile)          [binario lmdb-like propio]
        ├─ command_line ── args, config
        ├─ install ── resolver ── metadata, petgraph
        │     ├── bootstrap ── masterdir ── xbps
        │     ├── db, cache, sets ── soname
        │     ├── vur_client ── keys ── reposconf
        │     └── conf, util, overlay
        ├─ upgrade ── template_audit, diffy, db, cache, xbps
        ├─ repo ── keys, reposconf, vur_client
        └─ search/info/why/query/remove/review/journal/lockfile
```

`config.rs` es el nodo de mayor grado (20 consumidores): cambiar su forma de
estado toca todo el sistema. `db.rs` es el segundo (7).

## 5. Artefactos fuera de `src/`

| ruta | LOC | dominio |
|---|---:|---|
| `.github/workflows/ci.yml` | 67 | A6 |
| `.github/workflows/xbps.yml` | 149 | A6 |
| `.github/scripts/build-xbps.sh` | 54 | A6 |
| `.github/scripts/verify-xbps.sh` | 46 | A6 |
| `.github/scripts/publish-repo.sh` | 45 | A6 |
| `scripts/vur-generator.sh` | 89 | A6, A7 |
| `scripts/vurinfo.jq` | 47 | A6, A7 |
| `test/RESULTS.md` | 243 | A6 |
| `test/SMOKE-hybrid-2026-10-21.md` | 63 | A6 |
| `test/SMOKE-0.4.0.md` | 56 | A6 |
| `test/REPORT.md` | 56 | A6 |
| `test/plan_*.json` (4) | 305 | A6 |
| `Cargo.toml` | 52 | F0, A7 |
| `README.md` / `README.es.md` / `CHANGELOG.md` / `docs/*.md` | — | A7 |
| `etc/vary.conf.example`, `template/` | — | A5, A7 |

## 6. Matriz de cobertura (FASE 0 item 4)

Cada archivo con ≥1 dominio; los 7 críticos con ≥2.

| archivo | dominios | crítico |
|---|---|---|
| `src/install.rs` | A1, A2, A3, A5 | sí (2) |
| `src/upgrade.rs` | A1, A2, A4, A6 | sí (2) |
| `src/resolver.rs` | A1, A5, A7 | sí (2) |
| `src/db.rs` | A1, A4, A7 | sí (2) |
| `src/portdb.rs` | A1, A4, A6, A7 | sí (2) |
| `src/soname.rs` | A1, A6, A7 | sí (2) |
| `src/rebuild.rs` | A1, A6, A7 | sí (2) |
| `src/xbps.rs` | A1, A2 | — |
| `src/keys.rs` | A2, A3, A7 | — |
| `src/vur_client.rs` | A1, A2, A3 | — |
| `src/vup_index.rs` | A1, A3 | — |
| `src/command_line.rs` | A1, A5 | — |
| `src/config.rs` | A4, A5 | — |
| `src/metadata.rs` | A1, A3 | — |
| `src/reposconf.rs` | A3, A4 | — |
| `src/masterdir.rs` | A2, A3 | — |
| `src/overlay.rs` | A3, A7 | — |
| `src/sets.rs` | A1, A5 | — |
| `src/use_reduce.rs` | A1, A5 | — |
| `src/conf.rs` | A1, A5 | — |
| `src/conf_update.rs` | A3, A5 | — |
| `src/lockfile.rs` | A4, A7 | — |
| `src/journal.rs` | A3, A4 | — |
| `src/cache.rs` | A3, A4 | — |
| `src/signal.rs` | A2, A3 | — |
| `src/template_audit.rs` | A3, A7 | — |
| `src/challenge.rs` | A2, A3 | — |
| `src/bootstrap.rs` | A2, A3 | — |
| `src/preflight.rs` | A2, A3 | — |
| `src/elevate.rs` | A2 | — |
| `src/lock.rs` | A2, A4 | — |
| `src/logging.rs` | A2, A7 | — |
| `src/args.rs` | A1, A5 | — |
| `src/util.rs` | A4, A5 | — |
| `src/info.rs` | A2, A5 | — |
| `src/search.rs` | A2, A5 | — |
| `src/why.rs` | A1, A5 | — |
| `src/review.rs` | A3, A5 | — |
| `src/query.rs` | A5, A6 | — |
| `src/remove.rs` | A2, A5 | — |
| `src/repo.rs` | A3, A5 | — |
| `src/lib.rs` | A2, A5 | — |
| `src/main.rs` | A2 | — |
| `src/help.rs` | A5 | — |
| `src/init.rs` | A2, A3 | — |
| `Cargo.toml` | A7 | — |
| `.github/**` | A6 | — |
| `test/**` | A6 | — |
| `scripts/**` | A6, A7 | — |
| `README*.md`, `CHANGELOG.md`, `docs/**` | A7 | — |

**Agujeros: ninguno.** 45/45 archivos de `src/` con dueño; los 7 críticos con
≥2 dominios cada uno.

> Corrección 2026-10-22: la primera versión de esta matriz listaba 44 filas
> (`vup_index.rs` quedaba fuera → un agujero de cobertura) y la cabecera
> declaraba "45 módulos" cuando son 45 ficheros. Ambas cosas se detectaron al
> contrastar el mapa consigo mismo: dos listas que no se deacuerdan.
