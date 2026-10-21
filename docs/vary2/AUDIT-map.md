# AUDIT — mapa del proyecto (FASE 0)

Verdad del proyecto en el momento de auditar: `testing` = `main` = `8c14628`.
21.298 líneas de Rust en `src/` (46 módulos), 20 crates, 0 features opcionales.

Método: `wc -l`, `grep` de call sites por crate, `grep` de `use crate::` para
el grafo de dependencias, extracción de `pub fn` para la superficie pública.
Ninguna cifra de este documento es estimación.

## 1. Módulos × LOC × superficie × crates

| módulo | LOC | `pub fn` | crates que usa | consumidores (`use crate::`) |
|---|---:|---:|---|---|
| `install.rs` | 2334 | 3 | anyhow, serde_json, sha2, tempfile, dirs, nix(vía xbps) | bootstrap, cache, config, db, metadata, reposconf, resolver, util, vur_client, xbps |
| `vur_client.rs` | 1702 | 2 | anyhow, base64, serde, tar, tempfile, dirs | info, keys, repo, review, search, upgrade, vup_index, cache, metadata, reposconf |
| `resolver.rs` | 1687 | 8 | anyhow, petgraph, serde, metadata | install, why |
| `xbps.rs` | 1031 | 23 | anyhow, serde, tempfile, indicatif, nix, config | install, search, upgrade, masterdir, config |
| `command_line.rs` | 1055 | 5 | anyhow, config, args | lib, repo |
| `keys.rs` | 1118 | 22 | anyhow, base64, md5, sha2, tempfile, toml | repo, reposconf, vur_client, util |
| `db.rs` | 727 | 9 | anyhow, serde, serde_json, tempfile | lib, install, rebuild, remove, sets, upgrade, config |
| `upgrade.rs` | 715 | 2 | anyhow, diffy, sha2, tempfile, cache, config, db, reposconf, template_audit, vur_client, xbps | lib |
| `config.rs` | 666 | 5 | anyhow, serde, toml, dirs, ansiterm | 20 módulos (config central) |
| `masterdir.rs` | 596 | 12 | anyhow, nix, tempfile, dirs, xbps | bootstrap, main |
| `metadata.rs` | 523 | 6 | anyhow, serde, serde_json, tar, cache | install, resolver, vup_index, vur_client, cache |
| `repo.rs` | 509 | 1 | anyhow, toml, tempfile, command_line, config, keys, reposconf, util, vur_client | lib |
| `soname.rs` | 476 | 11 | anyhow, serde, sets | sets, rebuild, upgrade |
| `overlay.rs` | 439 | 3 | anyhow, sha2, tempfile, dirs | lib, install |
| `vup_index.rs` | 433 | 10 | anyhow, base64, serde, serde_json, tempfile, metadata | reposconf, vur_client, keys |
| `sets.rs` | 410 | 6 | anyhow, config, db, soname | lib, install |
| `signal.rs` | 388 | 11 | anyhow, indicatif, nix, tempfile | lib, main, masterdir, overlay |
| `rebuild.rs` | 363 | 6 | anyhow, sha2, config, db | upgrade, install |
| `lockfile.rs` | 362 | 7 | anyhow, serde, toml, tempfile, config | lib |
| `challenge.rs` | 357 | 6 | anyhow, ruzstd, tar, sha2, tempfile | lib |
| `preflight.rs` | 344 | 4 | anyhow, nix | main, lib |
| `template_audit.rs` | 349 | 5 | anyhow, tar | upgrade |
| `conf.rs` | 284 | 6 | anyhow, config | install, conf_update |
| `use_reduce.rs` | 267 | 6 | anyhow, tar | rebuild, portdb |
| `reposconf.rs` | 255 | 2 | anyhow, serde, toml, tempfile | info, install, keys, repo, search, upgrade, vur_client |
| `search.rs` | 245 | 1 | anyhow, cache, config, reposconf, vur_client, xbps | lib |
| `conf_update.rs` | 245 | 1 | anyhow, config | lib |
| `logging.rs` | 191 | 6 | tracing, tracing-appender, tracing-subscriber, tempfile | lib |
| `review.rs` | 173 | 3 | anyhow, vur_client, tempfile | lib |
| `util.rs` | 164 | 5 | anyhow, tempfile, config | install, keys, repo, lib |
| `args.rs` | 166 | 4 | anyhow | command_line, config |
| `info.rs` | 158 | 1 | anyhow, cache, config, reposconf, vur_client | lib |
| `why.rs` | 158 | 2 | anyhow, cache, config, resolver | lib |
| `journal.rs` | 143 | 5 | anyhow, tempfile | lib |
| `cache.rs` | 120 | 1 | anyhow, serde, serde_json, tempfile, metadata | info, install, search, upgrade, why, vur_client |
| `bootstrap.rs` | 115 | 3 | anyhow, masterdir | install |
| `query.rs` | 108 | 3 | anyhow, config | lib |
| `remove.rs` | 94 | 1 | anyhow, config, db | lib |
| `main.rs` | 93 | 0 | anyhow, nix, mimalloc, lib | bin |
| `lock.rs` | 89 | 2 | anyhow, nix, tempfile | lib |
| `help.rs` | 75 | 1 | — | lib |
| `init.rs` | 59 | 1 | anyhow | install |
| `lib.rs` | 453 | 3 | anyhow, config, db | bin |

Total: 177 `pub fn`.

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

**Agujeros: ninguno.** 46/46 archivos de `src/` con dueño; los 7 críticos con
≥2 dominios cada uno.
