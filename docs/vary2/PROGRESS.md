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

**DESVIACIÓN (2026-10-22):** la doble revisión subagent del fix de join
A8-03 (`to_vur_info` valida el nombre del paquete del `index.json`) **no se pudo
ejecutar**: 4 intentos (2 con el agente `reviewer`, 2 con `general`), todos con
`all upstream attempts failed` → indisponibilidad del servicio de subagents, no
un criterio del prompt. En su lugar, el autor hizo **2 pasadas propias con
evidencia**: (1) *lente de join* — el `pkgname` validado es exactamente la
clave que consume `install.rs` (`to_vur_info(pkgname, vpkg, arch)`), y si se
descarta no entra en `vur_map`, ni en `vup_binary_urls`, ni en `binary_check`
→ no llega al plan, ni a la DB, ni al nombre del artefacto; (2) *lente de
datos* — todo lo que entra del remoto tiene puerta (`repo_urls` →
`keys.rs:423 validate_repository_url`; PEM → `keys.rs:43 xbps_fingerprint_pem`
→ `parse_rsa_spki_pubkey`; versión → `split_pkgver`; plist → tope antes de
leer). **La revisión subagent queda PENDIENTE** para cuando la infra vuelva:
es un fix de join y el gobierno no lo cierra sin dos revisores.

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

Fricción **medida** entre el binario viejo (`8c14628`) y el nuevo (`f27bc64`)
en un Void limpio (contenedor, artefactos de CI):

| Camino | Viejo | Nuevo |
|--------|-------|-------|
| `vary --repo add` con un repo llamado `official` | se registraba | **error: nombre de repositorio reservado** |
| `repos.conf` con `enabld = false` (typo) | se ignoraba en silencio (el repo quedaba ACTIVO) | **error: unknown field `enabld`** (nombra el campo) |
| refresh de un VUR caído | solo el WARN en el log | **"aviso: no se pudo clonar el repo VUR 'x'"** en consola |
| paquete con `pkgname=../escape` | rechazado con aviso | igual (el falso positivo A3-01: ya lo rechazaba) |
| `vary -Q` | 95 paquetes | 95 (idéntico) |
