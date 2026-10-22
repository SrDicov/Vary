# AUDIT — noche automática 2026-10-22 (rama `agent/overnight-20261022-0204`)

Base: `6019a1b` (tip de `testing` al arrancar). Contrato de la noche: la lista
de abajo es **todo** lo que se toca. Lo que no está aquí, no se toca.

El repo ya pasó una auditoría exhaustiva (45/45 módulos, 35 hallazgos
arreglados). Esta noche **no reaudita el repo entero**: audita lo que la
semana de uso y el smoke destaparon (datos persistidos) y ejecuta los fixes
**medidos**, que es lo único que el gobierno del repo permite (`AGENTS.md`:
el backlog lo escribe el dolor, no las ideas).

## P0 — defects MEDIDOS hoy (con reproducción y archivo:línea)

### P0-1 · El índice de fuentes no se refresca tras un pull (BLOQUEANTE) — **HECHO**

**Medido (contenedor Void, clon real `--depth 1`):**

- `git diff-tree --no-commit-id --name-only -r <tip>` → **0 paths**: el tip de
  un clon shallow es frontera de profundidad (su padre no está) → un `pull`
  deja el tip exactamente así → `update_incremental` **no reindexa nada**.
- Y solo mira **un** commit (el tip): el resto del pull se ignora (medido: un
  commit intermedio que revertía `ncurses` **no** se reindexó).

**Impacto:** las versiones del índice quedan congeladas en el primer
reindexado completo → la señal de `--changed-deps` (nueva) y el cruce de
"versión nueva" (preexistente) **casi nunca se disparan** tras un pull real.

**Fix (HECHO):** el índice decide qué cambió con el **ÁRBOL DE TRABAJO** (el
sha256 de cada template, que el índice ya guardaba) en vez del diff de git:
correcto con cualquier profundidad de clon y **borra** `git_changed_paths` y
la propagación manual de aliases (un alias comparte el fichero con su padre →
el barrido lo actualiza solo). Tests: `incremental_reindexa_el_arbol_aunque_no_haya_commit`,
`incremental_saca_los_paquetes_borrados`, `incremental_refresca_los_alias_del_padre`.

**Pendiente:** verificarlo en Void real (P1-1) y medir el coste del barrido
sobre los 15.135 paquetes.

### P0-2 · La DB no se actualiza al sustituir un build de fuente por un oficial

**Medido:** el sistema tenía `sl-5.05_1` (binario oficial) y la DB decía
`sl-5.04_1` / `install_type: source` **con su baseline `deps_pinned`**.

Causa: T-005 impide registrar oficiales (escribiría ficción), pero nadie
**borra ni actualiza** la entrada anterior → sobrevive un "source" con
baseline de un paquete que ya no es un build local → las auditorías pueden
proponer rebuild sobre un **binario oficial**.

**Fix:** al registrar una instalación, si la entrada anterior era `Source` y
la nueva acción es un binario → la entrada se **retira** (vary ya no gestiona
ese paquete: no es suyo).

### P0-3 · La DB afirma una versión que no está instalada

**Medido:** con `sl-5.04_1` compilado y `sl-5.05_1` ya instalado, xbps
devolvió **0** pese al `ERROR: Package 'sl' already installed` → vary registró
`sl-5.04_1` (source) y el sistema siguió con `5.05_1`.

**Fix:** antes de escribir la entrada, **verificar** la versión realmente
instalada (`xbps-query -p pkgver`); si no es la esperada → no se registra la
baseline y se avisa (el estado que se guarda tiene que ser verdad).

## P1 — cobertura de la feature nueva (la regla de la noche: smoke)

### P1-1 · Re-ejercitar el smoke tras P0-1 (obligatorio)

Sin el fix del índice, la señal no se dispararía tras un pull real. El smoke
de hoy movió la dep en el **commit del tip** (el único caso que el incremental
veía): con P0-1 el escenario pasa a ser el real (árbol modificado, sin tocar
git).

### P1-2 · Ruta de VUR sin ejercitar

`--changed-deps` solo se ejercitó con un paquete **oficial** (la ruta del
`.VURINFO` con deps ya evaluadas no se probó). Es la ruta principal de un
VUR → smoke en el contenedor.

### P1-3 · Subpaquetes sin ejercitar

Un build oficial con subpaquetes (`gmp` → `gmp-devel`): la baseline del
subpaquete (¿se graba? ¿hereda?) y su señal.

### P1-4 · Interacción con un bump de SONAME

Deben ser **señales distintas**: la de soname la da `@preserved-rebuild` (que
avisa), la de dep recompila. Verificar que no se pisan ni se duplican.

## P2 — higiene (sin contrato nuevo)

- P2-1: ceilings `// ponytail:` del código: revisarlos y confirmar que cada
  uno tiene su techo y su ruta de mejora (o borrar el que ya no aplica).
- P2-2: `docs/GUIA.md` / `README.md` / `README.es.md`: coherencia con lo que
  la noche cambie (docs bilingües, mismo commit).

## NO esta noche (trigger no disparado → el backlog lo escribe el dolor)

- Fijar una versión antigua (`package.mask`/`unmask`): trigger en el ledger.
- `vary --print` + `-u`: trigger en el ledger.
- `--print-format=json`, `option_map`, perfiles, `--hybrid`, fingerprint
  remoto, `keywords`: sin fricción medida.
- Los diferidos de la auditoría (A5-02, A1-04, A1-05, A1-07/A5-03, A5-04,
  A5-08, A5-10, A6-05, A6-09): sus triggers no se han disparado.
- El registro de subpaquetes de builds oficiales (5.5): trigger = primer build
  local real con subpaquetes → **P1-3 lo provoca** → se anota, no se
  implementa aquí.

## Reglas de la noche

- Un ítem, un commit, un gate. Nada de lote.
- Gate local: `cargo fmt --all -- --check` + `cargo clippy --all-targets -- -D
  warnings` + `cargo test --lib` (desviación de la operativa del repo, que
  deja los tests para CI: aquí el gate local es el mecanismo de seguridad).
  Release: lo da la CI (build release local = prohibido por `AGENTS.md`).
- Sin `unwrap()`/`expect()` en producción; sin dependencias nuevas; sin
  tocar ficheros ajenos al ítem.
- Doble revisión subagent en cambios de semántica de comparación/join
  (`db`, `portdb`, `soname`, `rebuild`, `resolver`, `install`, `upgrade`).
- Al terminar: los tres informes del prompt (build/lint/tests pegados,
  `git log`, deuda remanente) + `STATUS.md`.
