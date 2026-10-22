# SMOKE — `--changed-deps` (rebuild por dependencias desfasadas) (2026-10-22)

Binario: `target/debug/vary` construido en local de `9d25e20` (CI verde en
`39725289602`; merge a `main` en `6e861d9`).
**Desviación registrada:** `cargo build` local (la operativa lo prohíbe salvo
caída de CI) porque el smoke exige binario con el código nuevo y CI no entrega
binarios; mismo precedente que `SMOKE-0.4.0.md`. Solo build; sin test/clippy
local (la CI ya estaba verde).
Entorno: Docker local `voidlinux/voidlinux:latest` (glibc), `--network host`,
usuario normal `tester` (vary se niega a correr como root) + sudo.

Escenario: `sl` (gnu-makefile, `makedepends="ncurses-devel"`) compilado desde
fuente → se mueve la versión de `ncurses` en el árbol de fuentes → el
dependiente debe entrar en el plan **solo** con `--changed-deps`.

Logs crudos: `test/smoke-changed-deps/`.

## S1 — Build real desde fuente ✅ (con dos huecos del entorno)

`vary -S --force-build --noconfirm sl` → bootstrap del masterdir
(`binary-bootstrap` OK) → `sl-5.05_1` compilado e instalado.
Dos fallos del **entorno**, no de vary: la imagen `voidlinux/voidlinux:latest`
no trae `/bin/bash` (el shebang de `xbps-src` lo exige) y sin `--noconfirm` el
prompt no tiene TTY.

## S2 — Baseline: `deps_pinned` se graba ✅ (tras un fix que el smoke destapó)

`installed.json` (tras el build):

```json
"sl": { "install_type": "source",
        "soname_pins": { "libc.so.6": "glibc-2.41_1",
                         "libncursesw.so.6": "ncurses-libs-6.2_4" },
        "deps_pinned": { "ncurses-devel": "ncurses-6.6_1" } }
```

**Hueco 1 (fix):** en la primera pasada `deps_pinned` **no salía**. Motivo: el
resolver crea un `VurInfo` **placeholder** para los oficiales (nodo hoja: sus
deps las resuelve xbps-src) → `depends`/`makedepends` vacíos → la baseline se
grababa vacía. La feature solo habría funcionado para paquetes de VUR. Fix:
para `Action::BuildOfficial` las deps se leen de la **plantilla real** del
clon de void-packages (`vur_client::parse_template_text`) y se reducen con las
opciones activas (`$(vopt_if ...)`).

## S3 — La dep se movió en el árbol ✅

En el clon: `ncurses` 6.6 → 6.6.1 (commit local `be39b98a`).

**Hueco 2 (fix, bug preexistente del `portdb`):** el update incremental
refrescaba el paquete tocado pero **no sus aliases**: tras el bump,
`ncurses` estaba en 6.6.1 y **`ncurses-devel` seguía en 6.6** (el alias es un
symlink a `srcpkgs/ncurses`, y su plantilla no se tocó en el commit). Efecto:
la señal no habríasaltado para ninguna dep `-devel` (la mayoría) y el cruce de
"versión nueva" de un subpaquete `-devel` ya daba un falso positivo. Fix:
al reindexar un paquete se propagan sus campos a los aliases cuyo `sourcepkg`
es ese paquete (+ test `incremental_refresca_los_alias_del_padre`).

## S4 — Upgrade SIN el flag: el dependiente NO entra ✅

`vary -Syu` → `VUR packages up to date.` → `sl` no se propone (opt-in real).

## S5 — Upgrade CON el flag: el dependiente entra y se anuncia ✅

`vary -Syu --changed-deps`:

```
WARN vary::upgrade: dep cambiada en el árbol: 'sl' se compiló contra otra versión de ncurses-devel
  sl (dep cambiada: ncurses-devel)
VUR upgrades available: sl
```

(El `exit 1` es el EOF del prompt de confirmación en un entorno sin TTY: la
propuesta ya estaba impresa ✓.)

## S6 — El plan antes de ejecutar ✅ (aquí está el valor de la feature)

`vary -Sp --changed-deps sl`:

```
INFO vary::install: sl: rebuild por dep cambiada (ncurses-devel)
:: PRINT-ONLY plan for: sl (no changes will be made)
builds [topological levels]:
  level 0:
    sl official-source (lo compila xbps-src, deps y artefactos incluidos)
warning: rebuild de 'sl': dep cambiada (ncurses-devel)
```

`vary -Sp sl` (sin el flag), para contraste:

```
installs [single binary transaction]:
  official/sl
builds [topological levels]:
  (none)
```

→ Sin el flag, vary **installa el binario oficial encima del build local**
(pisándolo); con el flag, **recompila desde fuente** y dice por qué. Esa es
la diferencia que la feature compra.

## Veredicto

La feature está **ejercitada en Void real**, no solo testeada: baseline
grabada, señal ausente sin el flag, presente con él, dependiente en el plan
como build (no como binario) y motivo visible en el plan.

**Lo que el smoke costó:** dos fixes que los tests unitarios no podían ver
(el `VurInfo` placeholder de los oficiales y los aliases stale del índice).
Sin el smoke, la feature habría sido un `CI verde + 362 tests` que no hacía
nada en el caso más común.

### Dos honestidades que el smoke debe (y este documento debe) decir

1. **El bump del smoke (`ncurses` 6.6 → 6.6.1) es sintético**: esa versión
   nunca existió upstream → su distfile da 404 → **el árbol queda sin
   compilar** para lo que dependa de ncurses. Se midió el **plan** (que es lo
   que la feature decide), no un rebuild real de la dep con esa versión.
2. **El bump era el commit del tip**, que es el **único caso que el index
   incremental ve** (`git diff-tree` de un commit, y en un clon shallow el
   tip no tiene diff). El índice **no se refresca tras un pull real** →
   la señal casi nunca se dispararía en el mundo real. Es un bug
   preexistente del `portdb`, medido y anotado en `docs/vary2/PROGRESS.md`
   con sus opciones; la feature queda verificada en su **mecanismo** y
   bloqueada en su **trigger real** hasta que se arregle.

**Lo que NO se ha ejercitado:** un paquete de **VUR** (la ruta del `.VURINFO`
con deps ya evaluadas), un subpaquete, y el caso de un bump de SONAME
interactuando con `--changed-deps` (deben ser señales distintas: la de
soname ya la da `@preserved-rebuild`).

