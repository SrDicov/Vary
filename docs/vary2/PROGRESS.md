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

**MEDICIÓN (2026-10-22, ventana real: host con el clon de void-packages,
15.162 templates)**:

```
cargo test --lib -- --ignored portdb --nocapture
  frío: 15162 entradas en 9.291053796s (3 dinámicos)
  incremental: 41.813893ms (reindexados: 0, completo: false)
```

Ratio 222× (índice frío 9,29 s en este PC vs incremental 41,8 ms). El test
ahora **afirma** (antes solo imprimía): `n_cold > 100` y el incremental no
cambia el número de entradas (findings A6-01). El incremental sobre un árbol
sin cambios reindexa 0 paquetes y NO hace reindexado completo ✓.

### 5.5 — Ítems diferidos (condición de disparo, coste y fecha)

| Ítem | Trigger (evento observable) | Coste al disparar | Fecha |
|------|----------------------------|-------------------|-------|
| Preservación física de `.so` | primera actualización real en Void con consumidores binarios instalados que dispare el detector |medio: diseño + fixture + `ldconfig` + aprobación de root | 2026-10-21 |
| Registro de subpaquetes de builds oficiales (+ repolock) | primera actualización oficial que sustituya un subpaquete compilado por vary (hoy: `gmpxx` sin repolock) | **medio-bajo** para el mínimo útil (parser `subpackages` + repolock al instalar: 2 módulos); medio-alto para el completo (con entradas y auditoría propias en la DB). Workaround manual mientras tanto (ver ORDEN v4/A) | 2026-10-21 |
| Normalización subpkg→sourcepkg en el resolver | evidencia medida de trabajo duplicado (varios nodos de build del mismo sourcepkg en un plan real) | medio: identidad de nodos (plan, DB, lockfile, install) | 2026-10-21 |
| Fallback dinámico del índice | primera resolución en frío de una plantilla `needs_dynamic` que no se resuelva por `xbps-src show` en el upgrade | bajo: llamada en paralelo con cap | 2026-10-21 |
| Medición E2E del índice | próxima ventana de upgrade real | bajo: un comando | 2026-10-21 |

### 5.3-bis — Verificación final en Void real (contenedor, vary `fd50af2`)

```
void-packages indexado: 15162 paquete(s) (índice completo).
portdb: 15162 plantillas, 3 requieren resolución dinámica (vía xbps-src show en upgrade).
```

- El índice guarda el **pkgver** del proveedor (fix crítico):
  `"libc.so.6": "glibc-2.41_1"`, `"libssl.so.3": "libssl3-3.5.4_1"` (antes: `"glibc"`).
- Auditoría con `cflags` cambiadas: `gmp official-source` +
  `warning: rebuild de 'gmp': banderas cambiadas` → el join funciona en real.
- 15.162 entradas = 9.713 templates + los alias `-devel` (symlinks), que el
  índice también resuelve a su sourcepkg.

### ORDEN v4 — Remates administrativos (2026-10-21)

**A) Workaround documentado (sin código)**

> Hasta que exista el registro de subpaquetes (diferido, con trigger): si
> mantienes un build local de un paquete **con** subpaquetes, ejecuta
> manualmente `xbps-pkgdb -m repolock <cada-subpaquete>` tras el build.
> Sin eso, una actualización oficial puede sustituir ese binario compilado por
> el genérico (medido: `repolock: gmp=yes`, `gmpxx/gmpxx-devel/gmp-devel=no`).

**Coste REVISADO del diferido** (la estimación de 4 módulos era demasiado
pesada): el índice de portdb **ya resuelve los alias `-devel` → sourcepkg**
(15.162 entradas = 9.713 templates + alias), así que:

| Alcance | Módulos | Coste |
|---------|---------|-------|
| **Mínimo útil**: extraer `subpackages="..."` (una clave literal más en el parser léxico) + repolockear esos subpaquetes al instalar | `portdb.rs` + `install.rs` | **medio-bajo** |
| Completo: además registrar los subpaquetes en la DB (entradas propias) y auditarlos | + `db.rs` + `upgrade.rs` | medio-alto |
| (No hace falta para el mínimo) normalización subpkg→sourcepkg en el resolver | `resolver.rs` | medio, y con riesgo de identidad de nodos |

> **El precio NO autoriza el arreglo.** Esta tabla es información, no un
> crédito: "es barato" es exactamente el razonamiento que convierte un
> diferido en feature sin que nadie lo vote. Lo único que abre el ítem es el
> **trigger** (una actualización real que sustituya un subpaquete compilado
> por el genérico). Cuando dispare, el diseño se decide **con ese caso**,
> no con esta estimación.

**B) Test de contrato del helper de sync — CANCELADO (y anotado por qué)**

El helper `sync_source_index` **no es función pura**: hace I/O (`git pull`,
índice en disco) y escribe a stdout; el formato de la línea vive inline en un
`println!`. Alcanzarlo exigiría **extraer** el formato, y la orden prohíbe
refactorizar para hacerlo testeable (y el cap de 15 min manda: cancelado).
La línea **sí está verificada en Void real** (contenedor, `fd50af2`):
`portdb: 15162 plantillas, 3 requieren resolución dinámica (vía xbps-src show en upgrade).`
Si en el futuro se toca ese texto, el sitio a assert es la salida de `-Sy`, no
un unit test.

**C) Vigilancia de triggers (solo registrar, NO ejecutar)**

| Trigger | Qué hacer | Dónde |
|---------|-----------|-------|
| El `tracing::warn!` de SONAME dispara en una upgrade real | capturar la salida **VERBATIM** (proveedor, soname viejo/nuevo, consumidores) en este documento: es la evidencia que justificará o matará la preservación `.so` | sección "Registro de roces" |
| Ventana disponible | `cargo test -- --ignored portdb` y anotar el número E2E en 5.4 | 5.4 |
| Primer build local real con subpaquetes | anotarlo (dispara el trigger del diferido de registro) | 5.5 |

**D) Semana de uso — la ejecuta el usuario.** El trabajo de OpenCode durante la
semana: **solo registrar**, no arreglar (salvo corrupción de datos o crash del
camino de upgrade; con doble revisión si toca semántica de comparación/join).
Cada roce → entrada con `fecha | comando | qué esperaba | qué pasó | severidad
(bloquea / molesta / cosmético)`. **Prohibido** empezar features durante la
semana: el siguiente arco se elige por **frecuencia de dolor**, no por
interés.

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

## AUDITORÍA 2026-10-22 (abierta sobre `8c14628`)

**Regla de la auditoría: NO reabre el arco.** Audita lo que existe. "Debería
implementarse X" no es un hallazgo: si X ya está diferido con trigger (5.5), es
RECHAZADO con motivo. La auditoría tampoco es la excusa para el rewrite.

- FASE 0 (inventario) → `docs/vary2/AUDIT-map.md` (`4edcb02`): 46 módulos, 21.298
  LOC, 177 `pub fn`, 20 crates (todos con call site), matriz de cobertura sin
  agujeros (los 7 críticos con ≥2 dominios).
- FASE 1: 7 dominios (A1-A7) con subagents, 2-3 en paralelo. **A7 devolvió
  texto degenerado (sin evidencia) → se repitió con brief estrecho.**
- FASE 2 (fusión/triaje) → `docs/vary2/AUDIT-hallazgos.md`.
- FASE 3 (arreglos) → commits `audit(ámbito): ...`, uno por fix lógico.
- FASE 4 (poda S4) → solo con evidencia de inalcanzabilidad, sin cambios de
  comportamiento.
- FASE 5 → `docs/vary2/AUDIT-2026-10-22.md` (informe ejecutivo) + merge a main.

### Hallazgos de la auditoría (resumen; detalle en AUDIT-hallazgos.md)

**Falsos positivos descartados por el fusor (con evidencia):** A3-01 (la
validación del pkgname SÍ estaba: `vur_client.rs:1152 info.validate()?`; la
reproducción en Void real lo rechazó limpio) y A4-03a (la migración v2→v3 es
intencionada: solo se rechaza el schema FUTURO).

**Arreglados (S1/S2 primero):** A1-01 (pkgver del subpaquete), A1-02
(expansión de variables en el padre), A2-01/02/04/05 (argv con `--`), A2-03 +
A4-05 (fichero de confianza), A3-02/A4-02/A4-04/A4-09 (escrituras atómicas),
A4-01 (índice de fuentes corrupto), A4-03 (schema futuro), A4-06/A5-09 (fallo
de refresh visible), A4-11 (DB de remove), A5-01 (arch del artefacto en musl),
A5-05 (estado de instalación), A6-01/02/03/04/06 (tests y CI).

**Diferidos con trigger (NO se arreglan en la auditoría):** A1-04 (props
posicionales), A1-05 (multi-proveedor de shlibs), A1-07/A5-03 (flags pacman
sin efecto), A5-04 (`--interactive`), A5-08 (colisión de niveles), A5-10 (hash
de template sin `files/`), A6-05 (mock sin arch), A6-09 (ruido del repounlock).

**PARADA §8.2 (plan presentado, no ejecutado):** A5-02
(`build_options_default` no está en el esquema VURINFO: el arreglo toca ≥4
módulos y el schema). **Congelados por el prompt:** todo cambio de CLI
(A1-07/A5-03/A5-04/A5-08) y el formato del valor en `installed.json` (A5-10).

### A5-02 — DECIDIDO con medición (2026-10-22): **comodidad ausente → DIFERIDO con trigger**

Regla aplicada: ¿la ausencia produce un **build incorrecto** o una **comodidad
ausente**? Medido en el clon real (`~/.cache/vary/void-packages`, 15.162
templates):

| Medición | Valor |
|---|---|
| templates con `build_options_default` | 1.199 |
| `$(vopt_if X ...)` en líneas de deps con X ∈ defaults | 409 casos en **374 paquetes** |
| deps "on" (las que xbps-src usaría) | 437 (66 únicas) |
| deps "on" **sin paquete en el árbol** (o sea, builds que FALLARÍAN) | **0** |

**Veredicto: el build nunca falla por esto.** xbps-src resuelve la dep "on"
como binario oficial (todas existen) → el paquete compilado es correcto. Lo que
falta es **control**: esa dep no está en el DAG de vary (no se compila desde
fuente), no queda anclada (repolock), no aparece en el plan y no se recompila
cuando cambia. Es la rama de **comodidad ausente** → se difiere.

**Trigger (concreto):** el primer paquete de un VUR cuya compilación falle, o
cuya salida difiera del plan, por una opción activa **por defecto** (no por el
usuario). **Coste si dispara:** añadir `build_options_default` al esquema
VURINFO v1 (`metadata.rs` + `vur_client.rs` + `scripts/vur-generator.sh` +
`scripts/vurinfo.jq` + `docs/VURINFO.md`) y pasarlo al reduce → ≥4 módulos +
schema (motivo de la PARADA §8.2).

Salto de la auditoría: `8c14628` (la forma con palabras estilo kyoz ya estaba
mergeada a main cuando empezó la auditoría).

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
| 2026-10-22 | `cargo clippy` + `cargo test --lib` **locales** (clippy con `-A clippy::collapsible_match`; 362 tests verdes) | excepción de la regla: la CI estaba **en rojo** (clippy `field_reassign_with_default` + 2 tests nuevos mal escritos). El fix-forward (`9d25e20`) quedó verde en CI |

## Siguiente

El arco v2 está **cerrado**. Lo único que queda abierto son las ventanas de 5.4/5.5
(condiciones de disparo escritas). El roadmap futuro lo genera el uso real.

**Excepción en curso (2026-10-22):** la auditoría total (arriba). Termina con la
matriz de cobertura sin agujeros y la tabla de hallazgos sin opiniones; después
vuelve a mandar la semana de uso real (registrar roces, hotfix solo con
corrupción o crash).

### Semana de uso — REINICIADA con el binario nuevo (f27bc64)

26 fixes cambiaron comportamiento: **la fricción contra el binario viejo es
dato podrido**. Plantilla de registro (una fila por roce):

```
| Fecha | Comando | Qué esperaba | Qué pasó | Severidad |
|-------|---------|--------------|----------|-----------|
|       |         |              |          | S1/S2/S3   |
```

Reglas de la semana (sin excepciones):

- **Hotfix solo con corrupción de datos o crash del upgrade.** Nada más.
- **Features prohibidas**: el backlog lo escribe el dolor, no las ideas.
- **Si dispara el detector de SONAME**: el log **verbatim** es la pieza más
  valiosa que puede dar esta herramienta → pegarlo tal cual (sin resumir) en
  este ledger.
- **Ausencia de dato ≠ señal**: un roce sin evidencia (comando + salida) no
  entra; entra como pregunta, no como hallazgo.
- Cada roce con severidad >= S2 se tria el mismo día (¿bug, característica,
  o uso incorrecto?); los S3 se anotan y se agrupan.

**DESVIACIÓN (2026-10-22) — DEUDA EN CURSO:** la doble revisión subagent del
fix de join A8-03 (`to_vur_info` valida el nombre del paquete del `index.json`)
**no se pudo ejecutar** en la primera ventana: **6 intentos** (3 con el agente
`reviewer`, 3 con `general`), todos con `all upstream attempts failed` →
indisponibilidad del servicio de subagents, no un criterio del prompt.
**Trigger de cierre (observado, no "cuando pueda"):** la primera sesión en la
que una llamada subagent tenga éxito → se paga esta deuda antes que cualquier
otra cosa.
**Trigger CUMPLIDO (intentos 7 y 8):** la infra volvió y **la doble revisión
de A8-03 está PAGADA**:

- **Revisor A — OK CON SALVEDADES** (solo S4): la puerta es correcta y está en
  el punto de entrada único de datos remotos a nombres. Sus dos S4 aplicadas: el
  aviso dice **qué repo** declara el nombre (`H1`) y hay un test con la **forma
  real** del `index.json` de VUP (`_meta` hermano de `packages`) (`H2`). Midió
  además la predicate contra los **15.162** nombres reales del clon de
  void-packages → **0 falsos positivos**.
- **Revisor B — OK CON SALVEDADES** (sin S1/S2): 4 salvedades **aplicadas**
  (`40bacc0`): los tres topes eran **fail-open** en su camino excepcional
  (`H-B2` → fail-closed: si no se puede MEDIR, no se sigue sin tope); la
  **versión** del índice no pasaba por `is_valid_version` (`H-B4` → ahora sí,
  misma simetría que la vía .VURINFO); el huérfano por SIGKILL (`H-B1` → prefijo
  reconocible + barrido **por EDAD**, que no reintroduce la carrera que el nombre
  único evita); y `--max-filesize` solo actúa con Content-Length (`H-B3` →
  `--limit-rate` acota los bytes + el comentario corregido para no sobreestimar
  lo que cubre el flag).

**La deuda de revisión queda CERRADA.** Y con ella la pregunta que la dejó
abierta: el `schema_version` del `installed.json` se justifica (hay fichero que
no se auto-repara) mientras que la caché del índice no lo lleva (se re-descarga
sola) → el número de versión existe exactamente donde hace falta.

El checklist que el segundo par de ojos debía cubrir **quedó respondido con
datos** (ver
`AUDIT-hallazgos.md`, sección A8): `split_pkgver` contra el código real de xbps
0.60.7 (`lib/util.c:45-54` `is_revision`, `:119-143` `xbps_pkg_version`,
`:249-282` `xbps_pkg_name`) + la medición en el clon real (0 versiones con "_"
en 15.162 templates); la escritura de caché (tmp+rename atómico → un kill
deja la caché vieja o la nueva, nunca una mezcla) y el huérfano por SIGKILL
(evidencia: queda `.tmp…`; el sweep solo barre `/tmp` con prefijo `vary-`; NO se
arranca porque barrer por patrón reintroduce la carrera que A8-04 cerró). En su lugar, el autor hizo **2 pasadas propias con
evidencia**: (1) *lente de join* — el `pkgname` validado es exactamente la
clave que consume `install.rs` (`to_vur_info(pkgname, vpkg, arch)`), y si se
descarta no entra en `vur_map`, ni en `vup_binary_urls`, ni en `binary_check`
→ no llega al plan, ni a la DB, ni al nombre del artefacto; (2) *lente de
datos* — todo lo que entra del remoto tiene puerta (`repo_urls` →
`keys.rs:423 validate_repository_url`; PEM → `keys.rs:43 xbps_fingerprint_pem`
→ `parse_rsa_spki_pubkey`; versión → `split_pkgver`; plist → tope antes de
leer). **La revisión subagent queda PENDIENTE** para cuando la infra vuelva:
es un fix de join y el gobierno no lo cierra sin dos revisores.

**Para el revisor cuando la infra vuelva (A8-03):** al argumentar el
`split_pkgver`, **citar la función de SPLIT de xbps, no solo el validador**
(`is_revision`). Con lo citado hasta ahora (`lib/util.c:45-54` `is_revision`,
`:119-143` `xbps_pkg_version`, `:249-282` `xbps_pkg_name`) queda probado que
xbps **acepta** una versión con "_" si lo que sigue es una revisión válida; lo
que falta es ver **dónde corta** el split (último "_" vs primer "_" tras un
dígito). Bajo ambas lecturas el descarte se sostiene (medido: 0 versiones con
"_" en 15.162 templates, y `VurInfo.revision` es `u32` → "01_1" no se puede
representar), pero el puente tiene que apoyarse en la pata correcta. El día
que aparezca un paquete estilo `perl-Digest` (que xbps sí acepta: revisión
"01_1") habrá que saber qué pata lo sostiene.

**O_TMPFILE (decisión, no descuido):** el huérfano por SIGKILL del temporal de
descarga (S4, `AUDIT-hallazgos.md` §A8) tiene una solución de kernel: `O_TMPFILE`
= inodo sin nombre que el kernel borra al morir el proceso → **huérfanos
estructuralmente imposibles**. No se usa porque `std` no lo expone: haría falta
`unsafe` + libc o una dependencia nueva, **vetadas ambas** por el gobierno, para
un S4 con nombre (VUP + SIGKILL + disco). Si el S4 sube de nombre, el precio
cambia y la decisión se revisa.

**Medición de la puerta "no parsea = no hay estado" (2026-10-22):** con curl
real y un `index.json` servido por HTTP: la caché truncada a 20 bytes **se
re-descarga** (no falla duro) → no hay S3 pendiente ahí. La misma puerta en los
JSON con estado: `installed.json` corrupto → backup + arranque vacío
(`db.rs`, test `corrupt_db_preserves_backup_and_starts_empty`) y schema futuro →
respaldo + no reinterpretación (A4-03, test
`schema_futuro_preserva_respaldo_y_no_se_reinterpreta`); `portdb.json` corrupto
→ backup + reindexado (A4-01). **Coherencia**: el número de versión de formato
existe exactamente donde NO hay auto-reparación (el estado se pierde aunque
con respaldo) y no existe donde sí la hay (la caché) → no es YAGNI.

**Leyes nuevas (de la propia auditoría):**

1. **Artefacto mal identificado = dato podrido: hash antes de conclusión.** La
   primera medición de fricción dio "sin cambios" porque el solver de xbps cogió
   el `.xbps` VIEJO (mismo pkgver en dos repos a la vez): el entorno mentía
   sobre el artefacto. El hash lo delató. Igual que el test que mentía sobre
   la producción: **verifica el sujeto antes de concluir sobre él**.
2. **`git add` con rutas explícitas cuando un directorio mezcla verdad y
   planes.** `git add docs/vary2/` se llevó los planes al merge; se
   devolvieron a untracked con un merge (sin reescribir historia publicada).
   La historia es evidencia inmutable; el árbol es la verdad.
   **Precio real de incumplirla: dos merges de corrección en la misma sesión**
   (la ley se escribió después del primer incidente y se incumplió después).
   Regla operativa: `git add <ruta>` con la ruta explícita, o `git add -u` para
   lo ya trackeado; NUNCA un directorio que mezcle verdad y planes.

Fricción **medida** entre el binario viejo (`8c14628`) y el nuevo (`f27bc64`)
en un Void limpio (contenedor, artefactos de CI):

| Camino | Viejo | Nuevo |
|--------|-------|-------|
| `vary --repo add` con un repo llamado `official` | se registraba | **error: nombre de repositorio reservado** |
| `repos.conf` con `enabld = false` (typo) | se ignoraba en silencio (el repo quedaba ACTIVO) | **error: unknown field `enabld`** (nombra el campo) |
| refresh de un VUR caído | solo el WARN en el log | **"aviso: no se pudo clonar el repo VUR 'x'"** en consola |
| paquete con `pkgname=../escape` | rechazado con aviso | igual (el falso positivo A3-01: ya lo rechazaba) |
| `vary -Q` | 95 paquetes | 95 (idéntico) |

## 2026-10-22 — Rebuild por dependencias desfasadas (`--changed-deps`)

**Origen:** una hoja de ruta externa ("Vary → experiencia Gentoo") proponía 40
cambios; al cotejarla con el código, **5 de sus 7 P0 estructurales ya estaban
implementados**, 1 era derivable sin schema nuevo y 1 dependía de datos que
nadie publica. Quedaban 2 huecos reales; este cierra el que sí tiene valor.

**Lo que NO se re-propuso (medido, no de memoria):**

| Propuesta | Realidad (fichero:línea) |
|---|---|
| "opciones globales" | `config.rs:205` + `use_reduce.rs:39` (semántica de sustitución ya resuelta) |
| "fingerprint de build" | `db.rs:31` `BuildFacts` (opciones, banderas, parches, template) |
| "auditoría de rebuild" | `rebuild.rs:74` `audit_all` + `--newoptions`/`--changed-flags` |
| "sets de usuario" | `src/sets.rs:38` |
| "mapa opción→deps en el schema" | `use_reduce.rs:147` `parse_vopt_if` ya lo extrae del template → **no hace falta campo nuevo** |
| "fingerprint de binarios remotos" | `vup_index.rs:41` `VupPackage` no tiene campo para él; el binhost oficial no lo publica → **imposible hoy** |

**Lo implementado** (`c050699`, `9d25e20`, merge `6e861d9` en `main`):

- `db::Entry.deps_pinned` (dep → pkgver del árbol en el build), **opcional y sin
  bumpear el schema** (un vary viejo lo ignora; uno nuevo lee los ficheros
  antiguos sin baseline → ausencia de dato ≠ señal).
- `install.rs`: baseline al instalar **desde fuente** (runtime + build; no
  hostmakedepends: no condicionan el artefacto). Si el índice de fuentes no
  existe, `portdb::ensure_index` lo construye desde el checkout (si no, la
  baseline se perdería para siempre).
- `rebuild.rs`: `RebuildReason::DepChanged` + `changed_deps()` puro; el cruce
  usa el índice de fuentes, como el resto de la auditoría.
- `upgrade.rs`: con `--changed-deps` los dependientes entran en el plan de la
  **misma transacción** que el upgrade (como los consumidores de un bump de
  SONAME) y se **anuncian después** del gate de diffs (un aviso que promete
  una recompilación que el gate puede tirar es peor que ningún aviso).

**Opt-in y por qué:** con enlace dinámico la librería nueva se carga igual en
runtime, y un bump de SONAME ya lo avisa la maquinaria existente
(`@preserved-rebuild`). Lo que aporta esta señal es el caso **entrada de
build** (cabeceras, plantillas C++, header-only).

**Doble revisión PAGADA (A y B, OK CON SALVEDADES)** — 8 salvedades aplicadas:

- A-1 (MEDIA): el índice de fuentes solo lo crea `-Sy`/upgrade → sin índice la
  baseline se perdía **para siempre** → `ensure_index`.
- A-2 (MEDIA): dep presente en void-packages **y** en un VUR → el baseline es
  el del árbol → falso positivo posible (un rebuild de más, opt-in) → documentado.
- A-3 (BAJA): el pkgver comparison incluye el nombre del paquete fuente → un
  re-parentado del árbol da señal sin bump real → techo documentado con
  `// ponytail:` (comparar solo la versión evitaría ese falso positivo, pero
  perdería los forks con la misma versión).
- A-4 (BAJA): una dep **eliminada** del árbol no da señal → el texto de README
  y GUIA ya no promete ese caso.
- A-5 (BAJA): `dep_changed_consumers` sin test → ahora es pura (lookup
  inyectado) y tiene test.
- B-1 (MEDIA): el bloque `--changed-deps` anunciaba antes del gate de diffs →
  ahora anuncia **después** (solo lo que sigue en el plan).
- B-2 (MEDIA): el filtro por flag no tenía test → `select_reason()` extraído a
  función pura + test (una dimensión no tapa a otra).
- B-3/B-4/B-5 (BAJA): índice viejo al grabar la baseline (señal correcta, "from"
  impreciso) → techo documentado; comentario de `rebuild_candidates`
  desactualizado → corregido; falta el assert de que un `upsert` resetea la
  baseline → añadido.

**CI:** `c050699` **ROJO** (clippy `field_reassign_with_default` + 2 tests
nuevos mal escritos) → fix-forward `9d25e20` → **VERDE** (fmt + clippy + test);
merge a `main` en `6e861d9` → verde en `main`.

### Hueco NUEVO detectado (no lo cubría el análisis de la hoja de ruta)

El "keywords/ACCEPT_KEYWORDS" de la hoja de ruta era cargo cult (en Void/VUR
el repo **es** la rama inestable), pero la **necesidad** que detrás escondía es
real y **no está cubierta**: **fijar una versión antigua** (instalar `foo-1.2`
cuando el VUR ya tiene `foo-2.0`). Hoy no hay `package.mask`/`package.unmask`
equivalente: el resolver elige la versión más nueva del repo. `repolock` solo
protege lo compilado desde fuente, no fija una versión.

**Trigger:** el primer roce donde alguien necesite "quédate con la versión
antigua" (un paquete roto en la última, un VUR que actualizó y rompió algo).
**No implementado** (feature especulativa sin dolor medido).

### SMOKE en Void real de `--changed-deps` (2026-10-22) — 2 huecos encontrados

Informe: `test/SMOKE-changed-deps.md` (evidencia cruda en `test/smoke-changed-deps/`).
Binario `target/debug/vary` local; contenedor `voidlinux/voidlinux:latest`
(glibc), usuario normal + sudo. Escenario: `sl` compilado desde fuente →
se mueve `ncurses` en el árbol → el dependiente debe entrar en el plan **solo**
con `--changed-deps`.

**Resultado:** la feature **no hacía nada** en el caso más común. Dos fixes:

1. **La baseline no se grababa para los OFICIALES.** El resolver crea un
   `VurInfo` **placeholder** para los oficiales (nodo hoja: sus deps las
   resuelve xbps-src) → `depends`/`makedepends` vacíos → baseline vacía. La
   feature solo habría funcionado para paquetes de VUR. Fix: para
   `Action::BuildOfficial` las deps salen de la **plantilla real** del clon
   (`parse_template_text`, ahora `pub(crate)`) y se reducen con las opciones
   activas.
2. **Los ALIASES del índice se quedaban viejos** (bug **preexistente** del
   `portdb`): al reindexar un paquete se refrescaba su entrada, no las de sus
   aliases (`srcpkgs/foo-devel` → symlink a `foo`). Medido en el clon real:
   `ncurses` 6.6.1 y **`ncurses-devel` 6.6**. Dos efectos: la señal no salía
   para ninguna dep `-devel` (la mayoría) y el cruce de "versión nueva" de un
   subpaquete `-devel` **ya daba un falso positivo**. Fix: al reindexar un
   paquete se propagan sus campos a los aliases cuyo `sourcepkg` es ese
   paquete (+ test `incremental_refresca_los_alias_del_padre`).

**Verificado en Void real:**

| Paso | Resultado |
|---|---|
| Build real desde fuente | `sl-5.05_1` compilado e instalado (bootstrap `binary-bootstrap` OK) |
| Baseline | `"deps_pinned": { "ncurses-devel": "ncurses-6.6_1" }` |
| `vary -Syu` (sin flag) | `VUR packages up to date.` → `sl` **no** entra |
| `vary -Syu --changed-deps` | `sl (dep cambiada: ncurses-devel)` + `VUR upgrades available: sl` |
| `vary -Sp --changed-deps sl` | build `sl official-source` + `warning: rebuild de 'sl': dep cambiada (ncurses-devel)` |
| `vary -Sp sl` (sin flag) | `installs: official/sl` → el binario oficial **pisaría** el build local |

→ El valor de la feature, medido: sin el flag vary **reemplaza** el build local
por el binario oficial; con él, **recompila desde fuente** y dice por qué.

**Desviación registrada:** `cargo build` local (la operativa lo prohíbe salvo
caída de CI) porque el smoke exige binario con el código nuevo y CI no entrega
binarios; mismo precedente que `SMOKE-0.4.0.md`. Solo build; sin test/clippy
local (la CI estaba verde).

**Lección (la misma que el `git add`):** `CI verde + 362 tests` NO es "la
feature funciona": los tests no pueden ver que el `VurInfo` de un oficial está
vacío, ni que un alias del índice queda viejo. **Sin ejecución real, la
arquitectura es una hipótesis.**

**Sin ejercitar todavía** (para el siguiente smoke): un paquete de **VUR** (la
ruta del `.VURINFO` con deps ya evaluadas), un subpaquete, y la interacción
con un bump de SONAME (deben ser señales distintas: la de soname la da
`@preserved-rebuild`).

**Typo en un asunto ya publicado** (`a881511`: "al跟着"): la historia
publicada no se reescribe (es evidencia); queda anotado aquí.

### Regla que sale del smoke (institucionalizar)

**Toda feature que toque DATOS PERSISTIDOS (DB, índice, lock, caché) se
ejercita en Void real antes de darse por buena.** Motivo medido: la feature
`--changed-deps` pasó la doble revisión (2 revisores, 8 salvedades
aplicadas) y la CI verde (362 tests) **y no hacía nada** en el caso más
común. La revisión estática y la ejecución real atrapan cosas distintas: la
revisión vio el "índice ausente" (MEDIA) y el execution vio el `VurInfo`
placeholder de los oficiales y los aliases stale del índice. **Sin
ejecución, la arquitectura es una hipótesis** (y esta era falsa).

El smoke es barato cuando el escenario es real: un paquete pequeño
(`sl`), un clon de void-packages y un bump de versión. El coste está en
elegir el paquete, no en el smoke.

**Trigger de la regla:** la próxima feature que escriba en `installed.json`,
`portdb.json`, `repos.conf` o `vary.lock` → smoke antes de merge a `main`.

### Fricción candidata anotada (NO es bug; es decisión de CLI)

`vary --print` **no se combina con `-u`**: `vary -Spu` →
`error: --print no se combina con -u (solo '-Sp <pkg>')`. Es una decisión
del parser (el print de un upgrade no tiene un plan que mostrar sin
ejecutar el upgrade), no un defecto.

**Trigger:** el primer usuario que intente `vary -Spu` (o que pida ver el
plan de un upgrade antes de ejecutarlo). **No implementado** (sin dolor
medido). Si aparece, el camino natural es: el upgrade con print imprime la
lista de `outdated` + las razones y no ejecuta (como hace `--changed-deps`
con su anuncio).

### Entorno de la semana de uso

El host es Artix (sin xbps) → la semana de uso se hace en Void. El contenedor
del smoke queda **detenido pero conservado** con el estado ya montado
(`sl` compilado desde fuente, clon de void-packages con `ncurses` movido a
6.6.1, índice refrescado):

```sh
docker start vary-smoke
docker exec -u tester -e HOME=/home/tester vary-smoke sh -c 'vary -Syu --changed-deps'
```

Fricción de esta semana = filas de la plantilla de arriba (fecha | comando |
qué esperaba | qué pasó | severidad). Con la regla nueva: si una fila toca
datos persistidos, el smoke la reproduce antes de cerrar el commit.

### Semana de uso — fila 1 (2026-10-22): la fricción de esta feature es de EXPECTATIVA

| Fecha | Comando | Qué esperaba | Qué pasó | Severidad |
|-------|---------|--------------|----------|-----------|
| 2026-10-22 | `vary -S sl --noconfirm` (contenedor Void; `sl` compilado desde fuente; **sin** `--changed-deps`) | Que el build local sobreviviera, o al menos que vary avisara de que iba a sustituirlo por el binario oficial | El plan ofreció **`official/sl`** (binario) → **sin ningún aviso** de que el build desde fuente se sustituye (el repolock se suelta en silencio). La instalación **no** ocurrió: la versión oficial coincidía (`ERROR: Package 'sl' already installed`) → el build local y su `deps_pinned` quedaron **intactos**. Con `--changed-deps`, el mismo comando da `sl official-source` (rebuild) | **S3** (fricción de expectativa; el daño no se materializó) |

**Lo que esta fila deja escrito (importante para cerrar el commit):**

1. **La fricción de esta feature NO se tropieza sola.** No es una rotura
   (eso lo cubre `@preserved-rebuild` con SONAME): es "esperaba que el
   default hiciera X". Las fricciones de expectativa hay que **buscarlas**:
   esta fila existe porque el escenario se **ejercitó**, no porque algo
   fallara.
2. **"Cerrado como sobrante" ≠ "innecesario".** Significa "no lo pidió nadie
   en una semana". Si alguien lo tropieza dentro de tres meses, el ledger
   tiene que explicar que se cerró por falta de fricción, **no** por estar
   mal pensado. Queda escrito aquí para ese día.
3. **El daño se materializa cuando la versión oficial avanza**: entonces
   xbps instala el binario encima del build local (y con él se pierden tus
   banderas/opciones/parches). Con las versiones iguales, xbps no hace nada →
   el escenario del smoke (`installs: official/sl`) es real en el **plan**,
   pero inocuo en la **ejecución** hasta que el repo oficial se mueve.
4. **Fricción candidata nueva (NO implementada):** el plan no dice "esto va
   a sustituir un build desde fuente". Hoy el `repolock` se suelta en
   silencio (`repounlock_if_source`). Con una línea de aviso en el plan, la
   fricción sería visible antes de que ocurra.
   **Trigger:** el primer build local sustituido por un binario oficial sin
   que el usuario se entere. No implementado (sin daño medido todavía).
5. **Protocolo activo de la semana** (para que la fricción de expectativa
   salga): (a) compilar un paquete desde fuente; (b) correr `vary -S <pkg>`
   **sin** `--changed-deps` → observar si el binario oficial pisa el build
   local; (c) correr `vary -Syu --changed-deps` → observar si el dependiente
   entra en el plan y se anuncia; (d) apuntar la fila.

### Semana de uso — fila 2 (2026-10-22): el daño, MEDIDO (S2)

| Fecha | Comando | Qué esperaba | Qué pasó | Severidad |
|-------|---------|--------------|----------|-----------|
| 2026-10-22 | `vary -S sl --noconfirm` (build local `sl-5.04_1` instalado; el oficial es `5.05_1`) | Que mi build local sobreviviera, o que vary avisara | **El binario oficial sustituyó al build local**: `sl-5.04_1: updating to 5.05_1 ... updated successfully`. Sin aviso. La DB de vary quedó **desincronizada** (dice `sl-5.04_1`/`source` mientras el sistema tiene el binario `5.05_1`) y **conservó la baseline** `deps_pinned` de un paquete que ya no es un build local | **S2** |

→ **La fricción de la feature es REAL y medida**: sin `--changed-deps`, un
`vary -S <pkg>` explícito **reemplaza tu build local por el binario oficial**.
Con `--changed-deps`, el mismo comando da `sl official-source` (rebuild). El
commit se justifica como **necesario**.

Para medirlo hubo que crear el hueco de versiones (build local 5.04 < oficial
5.05): con versiones iguales, xbps responde `Package 'sl' already installed` y
no hace nada (fila 1, S3). El hueco se hizo con un `sl` 5.04 parcheado en el
clon (distfile y checksum reales); el contenedor quedó **restaurado** a las
versiones de upstream (`sl` 5.05, `ncurses` 6.6) y reconstruido.

### Dos hallazgos de integridad de datos (preexistentes; NO implementados)

1. **La DB no se actualiza al sustituir un build de fuente por un oficial.**
   T-005 impide registrar oficiales (escribiría ficción), pero nadie **borra**
   ni **actualiza** la entrada anterior → queda un "source" con baseline de
   un paquete que ya no es un build local. Consecuencia medida: las
   auditorías (`--newoptions`, `--changed-flags`, `--changed-deps`) pueden
   proponer rebuild sobre un **binario oficial**.
   **Trigger:** el primer rebuild propuesto sobre un paquete que ya no es
   build local.
2. **La DB afirma una versión que no está instalada.** Con `sl` 5.04
   compilado y `sl` 5.05 ya instalado, xbpsinstall devolvió **0** pese al
   `ERROR: Package 'sl' already installed` → vary registró `sl-5.04_1`
   (source) y el sistema siguió con `5.05_1`. La DB affirmed una versión
   inexistente ("artefacto mal identificado = dato podrido").
   **Trigger:** el primer install que xbps rechace con "already installed" y
   una versión distinta de la instalada.

### BLOQUEANTE REAL de la feature: el índice de fuentes no se refresca tras un pull

**Medido en el clon real (depth 1):**

- `git diff-tree --no-commit-id --name-only -r <tip>` → **0 paths**: el tip
  de un clon shallow es **frontera de profundidad** (su padre no está), así
  que git no puede diffearlo. Un `pull` deja el tip nuevo exactamente así →
  `update_incremental` **no reindexa NADA**.
- Y solo mira **un** commit (el tip): los demás commits del pull se ignoran
  (medido: un commit intermedio con ncurses revertido **no** se reindexó
  mientras el tip sí).

**Consecuencia:** las versiones del índice quedan congeladas en el primer
reindexado completo → la señal de `--changed-deps` (y el cruce de "versión
nueva" que ya existía) **casi nunca se disparan** tras un pull real. El smoke
no lo vio porque su bump era **el commit del tip** (el único caso que el
incremental ve) ✓.

**Opciones (NO implementadas):**
1. Diff de **rango**: `git diff --name-only <índice>..<nuevo>` → exige que el
   clon tenga el commit viejo (con `--depth 1` no está) → hay que dejar de
   hacer pull shallow o profundizar el clon (coste: tamaño del clon).
2. **Reindexado completo** cuando el clon es shallow: correcto, pero
   tumba el diseño incremental (~15 s por pull).
3. Comparar los hashes de plantilla del árbol contra los del índice (el
   índice ya los guarda): correcto, coste parecido al reindexado.

**Trigger:** el primer `vary -Sy` en un sistema real donde un paquete del
árbol haya cambiado de versión y el índice no lo haya visto (es decir: la
primera vez que la señal no aparece y debería). Hasta entonces: la feature
está **verificada en su mecanismo** (smoke) pero **bloqueada en su trigger
real** por este bug preexistente del `portdb`.

**Consecuencia para el veredicto del commit:** la feature no es "sobrante"
(la fricción S2 está medida), pero **su disparo en el mundo real depende de
arreglar el `portdb`**. El commit queda abierto hasta que haya un `-Sy` real
con el índice refrescado.

## Noche automática 2026-10-22 (rama `agent/overnight-20261022-0204`)

El contrato de la noche está en `AUDIT.md` (temporales o permanente hasta que
se cierre). Cuatro commits, todos a defects **medidos** en el smoke y en la
semana de uso:

| Commit | Qué arregla |
|---|---|
| `38e6d2b` | El índice de fuentes se refresca con el **árbol**, no con el diff de git (en un clon shallow el tip no tiene diff → tras un pull no se reindexaba NADA) |
| `9ce14dc` | La DB solo registra lo que está **instalado de verdad** (xbpsinstall puede devolver 0 pese a rechazar la instalación) |
| `11a6539` | Un binario oficial sobre un build de fuente **retira** la entrada (el paquete deja de ser de vary) |
| `58a9676` | Las 4 ALTA + 4 menores de la doble revisión (A y B) |

**Por qué el P0-1 era el bloqueante de la feature:** sin índice frescos, la
señal de `--changed-deps` (y el cruce de "versión nueva" que ya existía)
casi nunca se disparaban tras un pull real. El smoke de hoy no lo vio porque
su bump era **el commit del tip** (el único caso que el incremental veía).

**Verificado en Void real tras el fix (contenador):** la señal dispara con el
árbol modificado **sin commitear** (el caso real), en las tres rutas:
oficial (`sl`), **VUR** (`hello-vur` con baseline desde el `.VURINFO`) y con
bump de SONAME (las dos razones, sin duplicados).

**Coste del barrido:** ~5 s por sync (15.135 templates) en el fs del
contenedor; el `git pull` solo son 0,6 s. Es el precio de que el índice
refleje el árbol.

**Trigger disparado:** el diferido 5.5 ("registro de subpaquetes de builds
oficiales") → con un build real de `gmp` (que produce `gmp-devel`) se vio que
el subpaquete **no** tiene entrada propia en la DB (el `VurInfo` de los
oficiales es un placeholder). **Anotado, no implementado.**

**Inconsistencia anotada (P2, no implementada):** el aviso de bump de SONAME
se imprime antes del gate de diffs y el de dep desfasada después → el primero
puede prometer una recompilación que el gate cancela.
