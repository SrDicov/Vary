# STATUS — noche automática 2026-10-22 (Vary)

Rama: `agent/overnight-20261022-0204` (base `6019a1b`).
Contrato de la noche: `AUDIT.md`.

## Hecho

- **P0-1 · El índice de fuentes no se refrescaba tras un pull** (BLOQUEANTE de
  la feature `--changed-deps`): el incremental decidía "qué cambió" con
  `git diff-tree` de **un** commit; en un clon shallow (`--depth 1`, el de
  vary) el tip es frontera de profundidad → `diff-tree` no devuelve nada → tras
  un pull **no se reindexaba nada**, y el resto de commits del pull se
  ignoraban. Ahora decide con el **árbol de trabajo** (el sha256 de cada
  template, que el índice ya guardaba) → borra `git_changed_paths` y la
  propagación manual de aliases (−62 líneas). `38e6d2b`
- **P0-3 · La DB afirmaba una versión que no estaba instalada**: xbpsinstall
  puede devolver 0 pese a rechazar la instalación (`Package 'x' already
  installed`) → se registraba la versión esperada. Ahora se verifica contra el
  sistema antes de escribir, con **tres** vías (registrar / conservar la
  entrada que sigue siendo verdad / retirarla). `9ce14dc`
- **P0-2 · Un binario oficial sobre un build de fuente dejaba la entrada
  "source" con su baseline** → las auditorías podían proponer recompilar un
  binario oficial. Ahora la entrada se retira (y con ella sus subpaquetes,
  que comparten su `version`). `11a6539`
- **Doble revisión aplicada**: 4 ALTA + 4 menores (la revisión A detectó que
  P0-2 no cubría el caso medido; la B, que el `Drop` borraba entradas
  verdadeiras). `58a9676`
- **P1-1 a P1-4 verificados en Void real** (contenedor): la señal dispara con
  el árbol modificado **sin commitear** (el caso real), en las tres rutas:
  oficial (`sl`), **VUR** (`hello-vur`, baseline desde el `.VURINFO`) y con
  bump de SONAME (las dos razones, sin duplicados). `f04fa09`
- **P2-1**: un ceiling `// ponytail:` caducado eliminado (la rama `NewVersion`
  ya tiene consumidor desde que existe el índice de fuentes).

## Bloqueado (no implementado, con su razón)

- **Registro de subpaquetes de builds oficiales** (diferido 5.5): **su
  trigger se ha disparado** esta noche (build real de `gmp` → `gmp-devel` no
  tiene entrada propia en la DB, porque el `VurInfo` de los oficiales es un
  placeholder). Queda fuera del contrato de la noche → anotado en el ledger.
- **Fijar una versión antigua** (`package.mask`/`unmask`): trigger sin
  disparar.
- **`vary --print` + `-u`**: trigger sin disparar.
- **El aviso de bump de SONAME se imprime antes del gate de diffs** (el de
  dep desfasada, después): puede prometer una recompilación que el gate
  cancela. Es la misma clase que sí se corrigió en la ruta de deps → P2, no
  implementado.
- **Coste del barrido del índice**: ~5 s por sync (15.135 templates) en un fs
  lento (contenedor). Se asume: el índice antes no se refrescaba **nada**.

## Siguiente

1. Mergear la rama en `testing` (CI verde) y de ahí a `main`.
2. Smoke del camino **VUR binario** (un VUR que publica binarios firmados):
   la ruta del `index.json` con `build_fingerprint` no existe; no hay nada que
   ejercitar hasta que exista.
3. Cuando aparezca la primera fila de fricción real, mirar si el trigger del
   diferido 5.5 (subpaquetes) exige ya el registro.

## Decisiones tomadas (cerradas 2026-10-22)

1. **Se aceptan los ~5 s por sync.** El barrido por mtime+size añadiría un
   campo a la caché y un modo de fallo **silencioso y corruptor** (mtime puede
   mentir tras un `checkout`, un rsync sin `--times` o un fs con granularidad
   de 1 s) → exactamente el bug que P0-1 acaba de corregir. Antes el índice no
   se refrescaba **nada**: 5 s es estrictamente mejor que 0.
   **Trigger para revisarlo:** el primer usuario que reporte que 5 s de sync le
   resultan fricción. Si no aparece, se queda.
2. **El registro de subpaquetes de builds oficiales entra en el próximo
   ciclo.** Su trigger ya se ha disparado (build real de `gmp` → `gmp-devel`
   sin entrada en la DB) y es el mismo patrón que los tres P0 de la noche:
   "la DB no sabe algo que debería saber". Alcance sugerido: leer los
   subpaquetes de la plantilla real del clon (como ya se hace con las deps),
   registrarlos con su `version` heredada y un test de que `gmp-devel` aparece
   en `vary -Q` tras un build de `gmp`.
3. **El orden del aviso de bump de SONAME sube de P2 a P1 del próximo ciclo.**
   Es la misma clase de bug que el aviso de dep desfasada (que sí se corrigió),
   su fallo es **silencioso para el usuario** (promete una recompilación que el
   gate cancela) y el coste es bajo (mover el anuncio tras el gate + test).
   No esperar a la fricción: una fricción que el usuario no puede ni nombrar
   es una fricción que nunca llega.

## Cerrado en esta noche

- **`--changed-deps` se cierra como NECESARIO** (ledger): la fricción lo tocó
  dos veces con los dos lados medidos (el daño: el binario oficial pisa el
  build local; el bloqueo: sin el fix del índice la feature no disparaba
  nunca en un clon shallow) y está verificado en Void real por tres rutas
  (oficial, VUR, SONAME).
