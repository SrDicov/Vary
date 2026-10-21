# PROGRESS — Vary 2.0

Gobierno: `AGENTS.md` del repo es la ley suprema (ramas `main` + `testing`; merges solo
desde `testing` con CI verde; tests/clippy/release SIEMPRE en CI; operativa de PC lento).
Plan ejecutable: `docs/vary2/ORDEN-v2.md` (untracked, no se commitea).
Material de investigación: `docs/vary2/PROMPT-MAESTRO-VARY2-OpenCode.md` (untracked, con
correcciones `CORRECCIÓN-E<n>` inline: 27 notas).
Fuente de dominio persistente autorizada: `.opencode/skills/xbps-src/SKILL.md` (v2), con
copia global en `~/.config/opencode/skills/xbps-src/SKILL.md`.

## Estado por item (§3 de ORDEN-v2)

| Item | Estado | Commit / evidencia |
|------|--------|--------------------|
| ITEM 0 — Neutralización | ✅ | `49d6258` (skill + PROGRESS); docs movidos a `docs/vary2/` untracked |
| ITEM 1 — RebuildReason | ✅ | `542a47c` + `603960a` (clippy) + `9c96a5d` (dimensiones independientes, baseline parcial, subpaquetes) + `edcffbb` (clippy test) |
| ITEM 2 — PortDb sin SQLite | ✅ | `b29c808` + `3928ebd` + `2287155` (detección de no-literales, test) |
| ITEM 3 — use_reduce + virtuals | ✅ parcial | `e9db0ac` + `f79f387` + `a39c59e`: reducción condicional + virtuals + `$(vopt_if ...)` parseados sin romperlos. **Normalización subpaquete→sourcepkg: DEFERIDA** (ver desviaciones) |
| ITEM 4 — Detección SONAME | ✅ (sin preservación física) | `32d5191` + `a39c59e`: bumps de proveedor contra `common/shlibs`, consumidores al plan de recompilación ANTES del merge (E14). Preservación en `/usr/lib`: **parada obligatoria** (sin caso ABI real ni aprobación) |

## Decisiones tomadas

1. **Corrección del auditor (E10):** el doc v2 tiene razón y la auditoría previa se
   equivocó: `emerge -U = --changed-use` **existe** en Portage (`man/emerge.1:527`).
   Conclusión práctica mantenida: `--newoptions` sin short (`-N` en xbps-src = "sin
   repos remotos" y ya lo usa `--deep`), `-U` verificado libre en vary (shorts actuales:
   S R h V v q p s i y u w b r).
2. **`etc/conf`**: se mantiene el mecanismo actual (`etc/conf.<id>` por build, borrado en
   `Drop`, `src/conf.rs`). Los bloques `case` por paquete quedan **prohibidos** (E1).
3. **`XBPS_PKG_OPTIONS`**: separados por coma. Corregido y en `main` (`3c92440`).
4. **Cero dependencias nuevas** (E15): persistencia con `BTreeMap` + JSON; sin tokio,
   sin rusqlite, sin blake3. `sha2` y `serde` ya estaban.
5. **Preservación física de `.so`**: fuera de alcance hasta que exista un caso ABI real
   verificado y aprobación explícita del usuario (parada obligatoria de ITEM 4). Lo que
   SÍ se entrega es la **detección** (bumps de proveedor + lista de consumidores).
6. **Skill de dominio**: vive en `.opencode/skills/xbps-src/SKILL.md`, que el
   `.gitignore` del repo excluye a propósito (`.opencode/`): es *herramienta del agente*,
   no verdad del proyecto — mismo criterio que los planes de trabajo. Copia persistente
   en `~/.config/opencode/skills/xbps-src/SKILL.md` para que aplique a todos los
   proyectos.
7. **Auditoría de ITEM 1 (dos revisores subagent):** se corrigieron un falso positivo por
   baseline parcial, un falso negativo por dimensiones tapadas (`--changed-flags` no
   veía un cambio de banderas si antes había un cambio de opciones), subpaquetes sin
   baseline, y el warning que se emitía antes de saber la acción.

## Verificado contra upstream (no de memoria)

- `xbps-src` usage real: `-N` = "Disable use of remote repositories to resolve
  dependencies"; `-I` = "Ignore required dependencies, useful for extracting/fetching
  sources"; `-t` = masterdir efímero (requiere uchroot + overlayfs); `-r <repo>` = binpkgs
  en `<hostdir>/binpkgs/<repo>`; `-c <cfg>` = `etc/conf.<cfg>` (si existe, `etc/conf` no
  se intenta); `-f` = "Force running the specified stage ... even if it ran successfully";
  `-p <vars>` imprime variables extra del template (`show -p build_options`).
- `set_build_options` (`common/xbps-src/shutils/common.sh`): el re-source de `etc/conf`
  ocurre en un command substitution con `XBPS_CURRENT_PKG` exportado; `IFS=','` al
  partir la lista de opciones; `build_options_default` se aplica si la opción no fue
  tocada.
- `etc/conf` **ya no existe** en void-packages master (solo `etc/defaults.conf`,
  `etc/defaults.virtual`, `etc/xbps.d/`); `etc/virtual` es el override local
  ("EDIT etc/virtual INSTEAD").
- `XBPS_RUSTFLAGS` no existe (ni en `Manual.md` ni en `etc/defaults.conf`).
- Subpaquetes: `srcpkgs/openssl-devel -> openssl`, `srcpkgs/llvm19-devel -> llvm19` son
  symlinks reales (`curl-devel` no existe → no todos los `-devel` son symlink).
- `xbps-pkgdb -m repolock|repounlock` existe (`bin/xbps-pkgdb/main.c:75-78`).
- `xbps-query -X PKG` = "Show reverse dependencies for PKG" (`bin/xbps-query/main.c:73`).
- `common/shlibs`: formato `SONAME pkg-version [arch]` → el proveedor es lo anterior al
  primer guion del final.
- Esquema real de permisos de opencode: claves por tool (`bash`, `edit`, `read`, `grep`,
  `glob`, `webfetch`, `skill`, ...) con `"ask"|"allow"|"deny"` o mapa patrón→acción; **no**
  existe la tool `shell` ni la forma `{action,resource,effect}`. La config viva está en
  `~/.config/opencode/opencode.jsonc`: no se toca.
- El clon de void-packages lo gestiona vary con `--depth 1` (`src/masterdir.rs:465`); el
  incremental de ITEM 2 usa `git diff-tree --no-commit-id --name-only -r <sha>`.

## Mediciones (clon real `~/.cache/vary/void-packages`, depth 1)

- 9.713 templates en `srcpkgs/`.
- I/O del índice frío (lectura + sha256 de todos los templates): **0,41 s** (el parseo
  léxico y el JSON adds poco) → objetivo <60 s cumplido con margen amplio.
- `git diff-tree` del commit nuevo: **0,022 s** → el incremental (reindex de lo tocado)
  queda muy por debajo del objetivo de <1 s.
- Medición end-to-end en Void real: pendiente (test `#[ignore]` en `src/portdb.rs`:
  `cargo test -- --ignored portdb`).

## Desviaciones registradas

| Fecha | Desviación | Motivo |
|-------|-----------|--------|
| 2026-10-21 | ITEM 0 no commitea `ORDEN-v2.md` ni el prompt maestro v1 | §1.2/§1.5: un plan de trabajo no es verdad del proyecto |
| 2026-10-21 | El skill `xbps-src` no se commitea | `.gitignore` excluye `.opencode/`; se instala también en `~/.config/opencode/skills/` |
| 2026-10-21 | ITEM 3 sin normalización subpaquete→sourcepkg | Cambia la IDENTIDAD de los nodos de build (plan, DB, lockfile, install). Sin evidencia de duplicación real en uso, el rewrite es riesgo sin beneficio medido. Se reevalúa si aparece un caso real (mismo criterio que Tarjan/SCC: sin ciclo demostrado no hay rewrite) |
| 2026-10-21 | Sin `cargo test` local | Regla del repo: los tests solo en CI. Los fallos locales se detectaron por `cargo check --all-targets` (type-check de los tests) y se corrigieron con fix-forward |
| 2026-10-21 | Preservación física de `.so` NO implementada | Parada obligatoria de ITEM 4: cirugía de root (`/usr/lib`, `ldconfig`) que requiere un caso ABI real y aprobación explícita |

## Siguiente

1. Mergear `testing` → `main` con CI verde (CI + XBPS).
2. Cerrar ITEM 3 (normalización subpaquete→sourcepkg) SOLO con un caso real medido.
3. Medir el índice end-to-end en Void real (contenedor) con el test ignorado.
4. La preservación física de `.so` sigue bloqueada por decisión humana: requiere
   OK explícito y un caso ABI verificable.
