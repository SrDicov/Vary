# PROGRESS — Vary 2.0

Gobierno: `AGENTS.md` del repo es la ley suprema (ramas `main` + `testing`; merges solo
desde `testing` con CI verde; tests/clippy/release SIEMPRE en CI; operativa de PC lento).
Plan ejecutable: `docs/vary2/ORDEN-v2.md` (untracked, no se commitea).
Material de investigación: `docs/vary2/PROMPT-MAESTRO-VARY2-OpenCode.md` (untracked, con
correcciones `CORRECCIÓN-E<n>` inline).
Fuente de dominio persistente autorizada: `.opencode/skills/xbps-src/SKILL.md` (v2).

## Estado por item (§3 de ORDEN-v2)

| Item | Estado | Commit / evidencia |
|------|--------|--------------------|
| ITEM 0 — Neutralización | ✅ hecho | `docs/vary2/PROGRESS.md`; docs y skill siguen untracked (`.gitignore` excluye `.opencode/`) |
| ITEM 1 — RebuildReason | ⏳ en curso | — |
| ITEM 2 — PortDb sin SQLite | ⏳ pendiente | — |
| ITEM 3 — use_reduce + subpaquetes | ⏳ pendiente | — |
| ITEM 4 — Detección SONAME completa | ⏳ pendiente | — |

## Decisiones tomadas

1. **Corrección del auditor (E10):** el doc v2 tiene razón y la auditoría previa se
   equivocó: `emerge -U = --changed-use` **existe** en Portage (`man/emerge.1:527`).
   Se mantiene la conclusión práctica: `--newoptions` sin short (`-N` en xbps-src = "sin
   repos remotos" y ya está en uso por `--deep`), `-U` solo si está libre en vary
   (verificado: shorts actuales = S R h V v q p s i y u w b r → `-U` libre).
2. **`etc/conf`**: se mantiene el mecanismo actual (`etc/conf.<id>` por build, borrado en
   `Drop`, `src/conf.rs`). Los bloques `case` por paquete quedan **prohibidos** (E1).
3. **`XBPS_PKG_OPTIONS`**: separados por coma. Ya corregido y en `main` (`3c92440`).
4. **Cero dependencias nuevas** (E15): persistencia con `BTreeMap` + JSON; sin tokio,
   sin rusqlite, sin blake3.
5. **Preservación física de `.so`**: fuera de alcance hasta que exista un caso ABI real
   verificado y aprobación explícita del usuario (parada obligatoria de ITEM 4).
6. **Skill de dominio**: vive en `.opencode/skills/xbps-src/SKILL.md`, que el
   `.gitignore` del repo excluye a propósito (`.opencode/`): es *herramienta del agente*,
   no verdad del proyecto — mismo criterio que los planes de trabajo. Copia persistente
   en `~/.config/opencode/skills/xbps-src/SKILL.md` para que aplique a todos los
   proyectos.

## Verificado contra upstream (no de memoria)

- `xbps-src` usage real: `-N` = "Disable use of remote repositories to resolve
  dependencies"; `-I` = "Ignore required dependencies, useful for extracting/fetching
  sources"; `-t` = masterdir efímero (requiere uchroot + overlayfs); `-r <repo>` = binpkgs
  en `<hostdir>/binpkgs/<repo>`; `-c <cfg>` = `etc/conf.<cfg>` (si existe, `etc/conf` no
  se intenta); `-f` = "Force running the specified stage ... even if it ran successfully".
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
- `xbps-query -X PKG` = "Show reverse dependencies for PKG"
  (`bin/xbps-query/main.c:73`).
- Esquema real de permisos de opencode: claves por tool (`bash`, `edit`, `read`, `grep`,
  `glob`, `webfetch`, `skill`, ...) con `"ask"|"allow"|"deny"` o mapa patrón→acción; **no**
  existe la tool `shell` ni la forma `{action,resource,effect}`. La config viva está en
  `~/.config/opencode/opencode.jsonc` (providers + modelos + MCP): no se toca.
- El clon de void-packages lo gestiona vary con `--depth 1` (`src/masterdir.rs:465`); el
  incremental de ITEM 2 usará `git diff-tree --no-commit-id --name-only -r <sha>`.

## Desviaciones registradas

| Fecha | Desviación | Motivo |
|-------|-----------|--------|
| 2026-10-21 | ITEM 0 no commitea `ORDEN-v2.md` ni el prompt maestro v1 | §1.2/§1.5: un plan de trabajo no es verdad del proyecto; se mueven a `docs/vary2/` untracked |
| 2026-10-21 | ITEM 1 implementa `--changed-flags` con `-U` (el doc v2 lo pedía "tras verificar") | `-U` verificado libre en `command_line.rs` |

## Siguiente

ITEM 1 — `RebuildReason`: extender `Entry` (schema v3) con `active_options`, `cflags`,
`cxxflags`, `ldflags`, `patches_sha256`, `template_sha256` (todos `Option` + `serde
default`, sin reescribir migraciones); `src/rebuild.rs` puro con `audit()`; CLI
`--newoptions` (sin short) y `--changed-flags -U`; tests con el código actual.
