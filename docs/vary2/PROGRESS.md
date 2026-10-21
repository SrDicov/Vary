# PROGRESS — Vary 2.0

Gobierno: `AGENTS.md` del repo es la ley suprema (ramas `main` + `testing`; merges solo
desde `testing` con CI verde; tests/clippy/release SIEMPRE en CI; operativa de PC lento).
Plan ejecutable: `docs/vary2/ORDEN-v2.md` (untracked, no se commitea).
Material de investigación: `docs/vary2/PROMPT-MAESTRO-VARY2-OpenCode.md` (untracked, con
correcciones `CORRECCIÓN-E<n>` inline: 27 notas).
Fuente de dominio persistente autorizada: `.opencode/skills/xbps-src/SKILL.md` (v2), con
copia global en `~/.config/opencode/skills/xbps-src/SKILL.md`.

## ORDEN v3 — CIERRE DEL ARCO v2 (2026-10-21)

| Punto | Estado | Evidencia |
|-------|--------|-----------|
| 5.1 Sonda subpkg→sourcepkg | ✅ ejecutada (read-only) | contenedor Void + `gmp`; ver abajo |
| 5.2 Preservación `.so` | ✅ solo traza + trigger | `tracing::warn!` con proveedor/sonames/consumidores |
| 5.3 Fallback dinámico | ✅ declinado + contador | una línea en la salida de `-Sy`/upgrade |
| 5.4 Medición de campo | ⏳ ventana abierta | test `#[ignore]` en `src/portdb.rs` |
| 5.5 Diferidos con trigger | ✅ | tabla al final |
| 5.6 Cierre del arco | ✅ | este documento |

### 5.1 — Sonda subpkg→sourcepkg (RESULTADO)

Entorno: contenedor Void (glibc) con vary `6d6fe3d`, void-packages clonado (depth 1) +
`binary-bootstrap`. Paquete elegido: `gmp` (light, con `subpackages="gmpxx gmpxx-devel
gmp-devel"` → tres subpaquetes con nombre ≠ sourcepkg).

| # | Hecho medido | Resultado |
|---|---------------|-----------|
| 1 | `vary -S --force-build --noconfirm gmp` | compilado desde fuente (gmp 6.3.0_1) ✓ |
| 2 | `installed.json` | **una** entrada: `gmp` (source, con pins + hechos). **Ninguna** para `gmpxx`/`gmpxx-devel`/`gmp-devel` |
| 3 | `xbps-query -p repolock` | `gmp` → yes; los tres subpaquetes → **vacío** (no repolockeados) |
| 4 | Auditoría con config por defecto | sin discrepancia (plan del binario oficial) |
| 5 | Auditoría tras cambiar `cflags` | `gmp official-source` + `warning: rebuild de 'gmp': banderas cambiadas` → **el join (Entry ↔ baseline ↔ template) FUNCIONA** |
| 6 | Auditoría sobre los subpaquetes | **vacío silencioso** (no hay Entry que auditar) |
| 7 | `xbps-query -p origin` / `sourcepkg` | **no existen**: el único mapeo binpkg→sourcepkg está en el TEMPLATE (`subpackages="..."` y los symlinks `srcpkgs/<sub>`) |

**Veredicto**: el join **aguanta** para lo que vary registra (el paquete padre): probado
con el cambio de flags (hecho 5). El caso binpkg≠sourcepkg no es un fallo de join sino
un **hueco de registro**: vary no registra los subpaquetes de los builds oficiales → sin
entrada, sin auditoría y **sin repolock** → una actualización oficial puede sustituir un
binario compilado por vary por el genérico (la "vulnerabilidad crítica de
sobrescritura binaria" del doc v1, reintroducida por el lado de los subpaquetes).

**No se implementa** (criterio de la orden: "si lo sientes como rewrite, PARA y reporta"):
el arreglo no es "un campo". Exige (a) extraer `subpackages` en el parser de portdb,
(b) registrar los subpaquetes al instalar, (c) repolockearlos y (d) resolver su template
vía el padre: cuatro módulos → rewrite → **parada, reportado**. Lo que sí queda es el
comentario en `src/rebuild.rs::target_for` que documenta **por qué** el join aguanta y su
límite.

### 5.2 — Preservación física de `.so` (PROHIBIDO implementar; solo traza + trigger)

- Traza auditable ✓ (sin infraestructura de log nueva): `tracing::warn!` por evento con
  **proveedor**, **soname viejo/nuevo** y **lista de consumidores de ese soname**
  (`src/upgrade.rs`).
- **TRIGGER EXPLÍCITO**: *"Primera actualización real en Void que dispare el detector con
  consumidores binarios instalados → diseñar la preservación para ESE caso concreto, con
  el caso como fixture de test y aprobación explícita de root."*

### 5.3 — Fallback dinámico (DECLINADO)

No se invoca en el índice frío: convertir el camino rápido en lento en secreto es mentir
sobre el rendimiento; esos casos se resuelven por `xbps-src show` en la fase de upgrade.
ÚNICO código: una línea en la salida existente (`-Sy` y upgrade):
`portdb: N plantillas, M requieren resolución dinámica (vía xbps-src show en upgrade)`.

### 5.4 — Medición de campo del test `#[ignore]`

Ventana abierta: `cargo test -- --ignored portdb` sobre el clon real, registrando el
número end-to-end. AGENTS.md prohíbe `cargo test` local (PC lento) → queda anotado como
oportunidad (contenedor Void o máquina real). Medición de I/O ya hecha en el clon real:
9.713 templates → sha256 de todos **0,41 s**; `git diff-tree` **0,022 s**.

### 5.5 — Ítems diferidos (condición de disparo, coste y fecha)

| Ítem | Trigger (evento observable) | Coste al disparar | Fecha |
|------|----------------------------|-------------------|-------|
| Preservación física de `.so` | primera actualización real en Void con consumidores binarios instalados que dispare el detector |medio: diseño + fixture + `ldconfig` + aprobación de root | 2026-10-21 |
| Registro de subpaquetes de builds oficiales (+ repolock) | primera actualización oficial que sustituya un subpaquete compilado por vary (hoy: `gmpxx` sin repolock) | alto: parser `subpackages` + escritura de DB + repolock + auditoría + upgrade | 2026-10-21 |
| Normalización subpkg→sourcepkg en el resolver | evidencia medida de trabajo duplicado (varios nodos de build del mismo sourcepkg en un plan real) | medio: identidad de nodos (plan, DB, lockfile, install) | 2026-10-21 |
| Fallback dinámico del índice | primera resolución en frío de una plantilla `needs_dynamic` que no se resuelva por `xbps-src show` en el upgrade | bajo: llamada en paralelo con cap | 2026-10-21 |
| Medición E2E del índice | próxima ventana de upgrade real | bajo: un comando | 2026-10-21 |

### 5.6 — CIERRE DEL ARCO v2

**CERRADO.** No hay "siguiente item": el backlog no existe. El roadmap siguiente lo genera
el **uso**: una semana de `vary -Sy` + `vary -Syu` reales en la máquina, registrando cada
roce en este documento. Los features se ganan con dolor documentado, no con specs viejas.

### Reglas permanentes (no negociables)

- **Doble revisión subagent** en todo item que toque semántica de comparación/join
  (auditoría, rebuild, portdb). Mecánica pura, no.
- **Nada de amend** en commits pusheados (cadena de evidencia de CI). El subject con
  caracteres chinos de `6d6fe3d` no se toca.
- Ausencia de dato ≠ señal: si no hay baseline, no se propone rebuild.

### Hallazgos de la doble revisión de esta ronda (7; 1 crítico)

| Severidad | Hallazgo | Fix |
|-----------|----------|-----|
| **CRÍTICO** | `common/shlibs` guardaba solo el NOMBRE del proveedor (`glibc`) donde el comparador esperaba el pkgver (`glibc-2.41_1`) → la detección de bumps **nunca disparaba** (100 % de los casos) | el índice guarda el pkgver; `providers()` sigue devolviendo el nombre; test de forma cruzada portdb→soname |
| MAYOR | `pkgver_of()` devolvía `"<pkg>-_0"` para entradas `needs_dynamic` → `NewVersion` falso | devuelve `None` sin versión resuelta |
| MAYOR | el gate de revisión de templates (`outdated.retain(approved)`) tiraba los consumidores de bump cuando otro paquete tenía diff | solo se caen los que tienen diff y no se aprobaron |
| MAYOR | `update_incremental` no refrescaba `common/shlibs` si el commit lo tocaba (mapa congelado) | se refresca cuando el commit lo toca |
| MAYOR | el índice se refrescaba en la fase de sync (detrás del merge) → la detección usaba datos del ciclo anterior | `sync_index` se llama antes de la detección, dentro de `upgrade()` |
| MENOR | la traza listaba la lista global de consumidores en cada línea | lista solo los consumidores de ese soname |
| MENOR | README/README.es/CHANGELOG afirmaban "cada paquete" y "nunca recompila solo" | ajustados a la realidad (subpaquetes sin entrada propia; excepción del bump del árbol de fuentes) |

Los dos revisores trabajaron con datos MEDIDOS (contenedor Void), no con supuestos: por eso
encontraron lo que el CI no puede ver.

## Estado por item (§3 de ORDEN-v2)

| Item | Estado | Commit / evidencia |
|------|--------|--------------------|
| ITEM 0 — Neutralización | ✅ | `49d6258` (skill + PROGRESS); docs movidos a `docs/vary2/` untracked |
| ITEM 1 — RebuildReason | ✅ | `542a47c`, `603960a`, `9c96a5d`, `edcffbb` |
| ITEM 2 — PortDb sin SQL | ✅ | `b29c808`, `3928ebd`, `2287155`, `b2990c5` (shlibs/pkgver) |
| ITEM 3 — use_reduce + virtuals | ✅ parcial | `e9db0ac`, `f79f387`, `a39c59e` (normalización subpkg→sourcepkg diferida, ver 5.5) |
| ITEM 4 — Detección SONAME | ✅ (sin preservación física) | `32d5191`, `a39c59e`, `dfc1519` (traza + contador), `b2990c5` (bug crítico del shlibs) |

## Decisiones tomadas

1. **Corrección del auditor (E10):** el doc v2 tiene razón: `emerge -U = --changed-use`
   existe (`man/emerge.1:527`). `--newoptions` sigue sin short (`-N` en xbps-src = "sin
   repos remotos" y ya lo usa `--deep`); `-U` verificado libre en vary.
2. **`etc/conf`:** mecanismo actual (`etc/conf.<id>` por build, borrado en `Drop`); los
   bloques `case` por paquete quedan **prohibidos** (E1).
3. **`XBPS_PKG_OPTIONS`:** separados por coma (corregido en `3c92440`).
4. **Cero dependencias nuevas** (E15): `BTreeMap` + JSON; sin tokio, sin rusqlite, sin blake3.
5. **Preservación física de `.so`:** fuera de alcance; trigger escrito en 5.2.
6. **Skill de dominio:** en `.opencode/skills/` (ignorado por `.gitignore`) + copia global.
7. **Doble revisión** en semántica de join: aplicada (2 revisores) y **productiva**
   (7 hallazgos, 1 crítico que CI no podía ver).

## Verificado contra upstream (no de memoria)

- `xbps-src` usage: `-N` = "Disable use of remote repositories…"; `-I` = "Ignore required
  dependencies, useful for extracting/fetching sources"; `-t` = masterdir efímero (uchroot
  + overlayfs); `-r <repo>` = binpkgs en `<hostdir>/binpkgs/<repo>`; `-c <cfg>` =
  `etc/conf.<cfg>`; `-f` = forzar etapa; `-p <vars>` = variables extra del template.
- `set_build_options` (`common/xbps-src/shutils/common.sh`): re-source de `etc/conf` en un
  command substitution con `XBPS_CURRENT_PKG`; `IFS=','` al partir la lista de opciones.
- `etc/conf` ya no existe en void-packages master; `etc/virtual` es el override local.
- `XBPS_RUSTFLAGS` no existe. `xbps-query` no expone `origin`/`sourcepkg`.
- `xbps-pkgdb -m repolock|repounlock` existe; `xbps-query -X` = reverse deps.
- `common/shlibs`: `SONAME pkg-version [arch]`.
- Permisos de opencode: claves por tool (`bash`, `edit`, …) con `ask|allow|deny`; no
  existe la tool `shell` ni `{action,resource,effect}`.

## Desviaciones registradas

| Fecha | Desviación | Motivo |
|-------|-----------|--------|
| 2026-10-21 | `ORDEN-v2.md` y el prompt maestro v1 no se commitean | un plan de trabajo no es verdad del proyecto |
| 2026-10-21 | El skill `xbps-src` no se commitea | `.gitignore` excluye `.opencode/`; instalado también en `~/.config/opencode/skills/` |
| 2026-10-21 | ITEM 3 sin normalización subpkg→sourcepkg | cambiaría la identidad de los nodos de build; sin caso real medido (ver 5.5) |
| 2026-10-21 | Sin `cargo test` local | regla del repo; los fallos se cazaron con `cargo check --all-targets` + revisores |
| 2026-10-21 | Preservación física de `.so` NO implementada | parada obligatoria de ITEM 4 (root, caso ABI real, aprobación) |
| 2026-10-21 | Registro de subpaquetes de builds oficiales NO implementado | la sonda lo mostró necesario, pero el arreglo toca 4 módulos → rewrite → orden manda parar y reportar |

## Siguiente

El arco v2 está **cerrado**. Lo único que queda abierto son las ventanas de 5.4/5.5
(condiciones de disparo escritas). El roadmap futuro lo genera el uso real.
