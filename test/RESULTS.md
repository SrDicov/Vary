# RESULTS — Test intensivo vary 0.3.0 en Void real

Formato por prueba: `comando | contexto | resultado | tiempo | notas`.
Máquina: `x86_64` (Void real, producción). vary instalado: `0.2.5` (`/usr/bin/vary`, pre-H-005, backend LMDB).
Elevación disponible: `doas` (passwordless); `sudo` ausente (127), `run0` ausente.
Backup base: `test/backup/2026-09-06/` + `/root/xbps.d.pre-audit` + `/tmp/vary-{cache,config,data}-backup`.

## T0.2 — Suite unitaria en Void real

| comando | contexto | resultado | tiempo | notas |
|---|---|---|---|---|
| `cargo test -- --include-ignored` | repo @ `0de05c1`, Void real | OK 157+1 passed, 0 failed | 7.81s | incluye los 4 `integracion_*` xbps |

## T0.3 — VALIDATION.md pasos 0-8 (vary instalado 0.2.5 salvo indicación)

| paso | comando | resultado | notas |
|---|---|---|---|
| 0 | backups + `vary -V` + `git status` | OK | `vary 0.2.5`; árbol limpio @ `0de05c1`; HALLAZGO-previo: `~/.cache/vary/installed.json` es DIRECTORIO LMDB (`data.mdb`+`lock.mdb`, mtime 07:40) — el binario instalado es pre-H-005; la migración a JSON la disparará la candidata 0.3.0 (ver paso 6 / T1) |
| 1 | `cargo test integracion_ -- --include-ignored` | OK 4/4 (`query_installed/query_manual/query_architecture/search_remote`, 2.32s) | dentro del 157+1 de T0.2 |
| 7-baseline | exit codes con vary 0.2.5 (foto "antes") | `badflag=1`, `asdeps=1`, `configflag=1` | DIVERGENCIA-ESPERADA vs HEAD: pre-H-035 (1 en vez de 2); `--asdeps`/`--config` TRAGADOS (H-032/H-043 reproducidos: resuelve `foo`, falla por "no encontrado"). T2 repite contra 0.3.0 |
| 2 | `VARY_DEBUG=1 vary -Syu </dev/null` (0.2.5) | OK-sync; exit=0 | sync oficial + refresh VURs (`cnr 1b4eaa33`, `z-packages 43031639`) sin errores; warnings H-031 (spotify, xbps-triggers) visibles; xbps abortó upgrades en EOF ("Aborting!") pero vary devolvió 0 — OBSERVACIÓN: upgrade abortado enmascarado como éxito en 0.2.5; verificar en candidata (posible T-###) |
| 5-baseline | doble instancia con 0.2.5 (lock vía fifo bloqueado en confirm de `vary -S cowsay`) | lecturas serializan (search=0, version=0 tras espera); 2º `-S` NO es rechazado: espera el lock y procede | baseline pre-H-027: flock bloqueante global, sin mensaje "otra instancia"; first=1 tras `n`; H-001 EN VIVO: el 2º `-S nano` aprobó el confirm con EOF y lanzó `xbps-install` sin consentimiento (inocuo aquí: `ERROR: Package 'nano' already installed`, preinstalado 2026-08-26; second=0 pese al error de xbps — otro enmascaramiento de exit en 0.2.5). T2 repite contra 0.3.0 |
| 3,4,6 | — | DIFERIDOS a fase candidata | 0.2.5 no contiene el código nuevo (review gate H-001/H-019, señales H-016/H-034, migración H-005): probar contra 0.2.5 sería medir el bug viejo o dispararlo (EOF aprueba → build real). Se ejecutan en T1/T3 contra 0.3.0 |
