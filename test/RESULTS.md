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

- Backup: `test/backup/2026-09-06/` (`xbps.d/`, `installed.json`(LMDB dir), `repos.conf`, `vary-installed.txt`, `vary-0.2.5-bin`, `STAMP`) + `/root/xbps.d.pre-audit` (doas) + `/tmp/vary-{cache,config,data}-backup`. Rollback binario: `doas install -m755 test/backup/2026-09-06/vary-0.2.5-bin /usr/bin/vary`; rollback DB: `cp -a /tmp/vary-cache-backup/installed.json ~/.cache/vary/` (o `installed.lmdb.bak` si la migración ya corrió).
- Bump: `chore: bump 0.3.0` (`62c42aa`, Cargo.toml+Cargo.lock). Tag `v0.3.0` PENDIENTE de veredicto.
- Instalación: `doas install -m755 target/debug/vary /usr/bin/vary` (reemplazo directo del binario debug local, 50MB; sin xbps, sin release-LTO).
- Smoke: `vary -V` → `0.3.0`; `-h` OK; `--repo list` OK (cnr, z-packages); `--opcion-inexistente-xyz` → 2.

## T2 — Matriz CLI (candidata 0.3.0 salvo indicación)

> INCIDENTE 19:31 — corte eléctrico con reinicio (uptime lo confirma, tmpfs vaciado: `/tmp/vary-{cache,config,data}-backup` PERDIDOS).
> Estado en disco intacto: `/usr/bin/vary` 0.3.0, 4 repos, DB JSON migrada, `test/backup/`, `/root/xbps.d.pre-audit` ✓.
> Lección persistencia: NO más backups gigantes en tmpfs; valen `test/backup/` + `installed.lmdb.bak` + `vary-0.2.5-bin`.
> Disco: 2.1G libres/14G → `xbps-remove -O` (443M caché, solo obsoletos) → 2.2G. Política: `df` antes/después de cada build; `target/` (3.0G) se conserva; void-packages+masterdir (1.1G+0.5G) se reutilizan, no duplicar.

| comando | contexto | resultado | tiempo | notas |
|---|---|---|---|---|
| `vary -S pkg-que-no-existe-xyz` | resolución | exit=1 + "no encontrado en repos oficiales ni VURs" | ~3s | OK (nota: con pipe a `tail` el `$?` es del pipe — medir sin pipes) |
| `vary -S 'foo/bar'` | validación H-010 | exit=1 + "nombre de paquete inválido ... ^[a-zA-Z0-9][a-zA-Z0-9._+-]*$" | — | OK |
| `vary -Ss cowsay` / `-Ss zzznadaexiste` | búsqueda | exit=0 con resultados / exit=1 "No packages found" | — | OK |
| `vary -Si xbps` / `-Si librewolf` / `-Si hytale-installer` | info | exit=0, ficha correcta | — | los 3 resolvieron vía `official` (hay repos binarios que los llevan); falta caso `-Si` VUR-puro → T3 |
| `vary --color always\|never -Ss cowsay` | color | always: escapes; never: sin color PERO queda negrita `^[[1m` en el nombre | — | **T-001 (low)**: `--color never` no elimina el bold de `ss_name` |
| `vary -v` / `-vv -Ss cowsay` | verbosidad H-028 | SIN EFECTO: salida idéntica sin `-v`; `RUST_LOG=debug` sí muestra DEBUG (2 líneas) y el log-archivo tiene 447 DEBUG | — | **T-002 (medium)**: `-v`/`-vv` no bajan el filtro de consola (precedente H-032: flag documentado que no hace nada); parser OK (`verbose: 1/2` en dump); `apply_runtime_config` no surte efecto en vivo; workaround `RUST_LOG` |
| `vary -q -Ss cowsay` | quiet | solo nombres, exit=0 | — | OK |
| `vary --arch INVALIDARCH -Ss cowsay` | arch | exit=2 + lista de archs soportadas | — | OK (2=mal uso) |
| `vary -S cowsay </dev/null` | EOF H-001/H-019 | exit=1 + "stdin llegó a EOF sin confirmación ... usa --noconfirm" | — | PASS en vivo (contraste 0.2.5: aprobaba) |
| `vary --noconfirm -S nano </dev/null` | idempotencia | exit=0 aunque xbps dice "already installed" | — | **T-003 (medium)**: ERROR de xbps + exit 0 = éxito mentiroso (mismo patrón que `-Syu` abortado → 0 en baseline). O se detecta instalado antes (0 limpio) o se propaga el error (!=0) |
| `vary --noconfirm -S cowsay --sudo /nonexistent-bin-xyz` | elevación inexistente | exit=1 + "wrapper de elevación no ejecutable", SIN mutación (cowsay ausente) | — | OK: error claro, no panic (sale 1, no 127 — aceptado: es error de config en resolve, no NotFound en spawn) |
| `vary --sudoflags` (sin valor) | H-023 | exit=2 + "expects a value" | — | OK |
| `vary -Sy --git /bin/false` | git falso | exit=1, sin panic; WARN por repo con migración legacy abortada | ~40s | OK-tolerante; clones reales INTACTOS (git log + -Ss verificados). La migración full→partial se intenta en refresh (correcto diferirla del add) |
| doble instancia (fifo en confirm `vary -S cowsay`) | H-027 en vivo | `-Ss`=0 (2.9s, sin bloqueo), `-V`=0 (0.04s), 2º `-S`=1 en 0.04s: "otra instancia ... (pid 8809) ... no borres el archivo"; 1º=1 tras `n` | — | PASS total |
| `--help` vs README.md | docs | sin divergencias: todo flag README existe en help; `--asdeps/--asexplicit` documentados como rechazados | — | OK (README no enumera todo; help es canónico) |
| PENDIENTE-T3 | `--force-build`/`--prefer-binary`/`--no-prefer-binary` (3 caminos mismo pkg), `--sudo doas` real, `-Si` VUR-puro, error-rojo (sin cobertura en vivo: red local OK todo T2) | — | — | — |

| paso | comando | resultado | notas |
|---|---|---|---|
| 0 | backups + `vary -V` + `git status` | OK | `vary 0.2.5`; árbol limpio @ `0de05c1`; HALLAZGO-previo: `~/.cache/vary/installed.json` es DIRECTORIO LMDB (`data.mdb`+`lock.mdb`, mtime 07:40) — el binario instalado es pre-H-005; la migración a JSON la disparará la candidata 0.3.0 (ver paso 6 / T1) |
| 1 | `cargo test integracion_ -- --include-ignored` | OK 4/4 (`query_installed/query_manual/query_architecture/search_remote`, 2.32s) | dentro del 157+1 de T0.2 |
| 7-baseline | exit codes con vary 0.2.5 (foto "antes") | `badflag=1`, `asdeps=1`, `configflag=1` | DIVERGENCIA-ESPERADA vs HEAD: pre-H-035 (1 en vez de 2); `--asdeps`/`--config` TRAGADOS (H-032/H-043 reproducidos: resuelve `foo`, falla por "no encontrado"). T2 repite contra 0.3.0 |
| 2 | `VARY_DEBUG=1 vary -Syu </dev/null` (0.2.5) | OK-sync; exit=0 | sync oficial + refresh VURs (`cnr 1b4eaa33`, `z-packages 43031639`) sin errores; warnings H-031 (spotify, xbps-triggers) visibles; xbps abortó upgrades en EOF ("Aborting!") pero vary devolvió 0 — OBSERVACIÓN: upgrade abortado enmascarado como éxito en 0.2.5; verificar en candidata (posible T-###) |
| 5-baseline | doble instancia con 0.2.5 (lock vía fifo bloqueado en confirm de `vary -S cowsay`) | lecturas serializan (search=0, version=0 tras espera); 2º `-S` NO es rechazado: espera el lock y procede | baseline pre-H-027: flock bloqueante global, sin mensaje "otra instancia"; first=1 tras `n`; H-001 EN VIVO: el 2º `-S nano` aprobó el confirm con EOF y lanzó `xbps-install` sin consentimiento (inocuo aquí: `ERROR: Package 'nano' already installed`, preinstalado 2026-08-26; second=0 pese al error de xbps — otro enmascaramiento de exit en 0.2.5). T2 repite contra 0.3.0 |
| 3,4 | — | DIFERIDOS a T3 (requieren build VUR real) | review-EOF e INT-mid-build se prueban contra 0.3.0 con paquete fuente pequeño |
| 6 | migración LMDB→JSON + legacy-v1 (candidata 0.3.0) | OK (con nota) | `vary -R vary-noexiste-xyz` migró LMDB real: 2 entradas (hytale-installer/cnr, librewolf/z-packages, schema 2, build_date backfill) + `installed.lmdb.bak`; legacy-v1 con fixture CORRECTO (`"source"` minúsculas) migra (`migradas 1 entradas`, sin corrupt-backup). NOTA-DOC: el fixture de VALIDATION.md usa `"Source"` capitalizado → cae en ruta corrupt-con-respaldo (conducta correcta del código; enmendar VALIDATION). La migración es load-time en memoria; `-R` no re-persiste el archivo (diseño: sin pérdida hasta mutación real) |
