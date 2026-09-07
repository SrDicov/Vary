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

## T3 — Repos VUR (candidata 0.3.0 + fixes T-006/7/8/11,11)

> Repos: `cnr`+`z-packages` (preexistentes), `repository`=voiders-community (add OK: rama `main` autodetectada, nombre deriva a `repository`, sin `.VURINFO` → fallback template, sin confirm), `vup` (add OK con `--index-url`; fetch de índice perezoso; SIN prompt TOFU en add).
> Catálogos: cnr 44 templates, z-packages 10 (.VURINFO raíz), repository 118 (layout `pkgs/`), vup 27 binarios.
> LECCIÓN: 18/37 planes sombreados por binarios (neko/zrepo/official/rolling) → el resolver prefiere official (diseño); la matriz real usa VUR-exclusivos. `vary -Ss <vup>` SÍ consulta el índice (falsa alarma inicial por `head -6`).
> Disco: 2.2G→695M (xbps-cache 450M×2 limpiada, masterdir `zap` 750M tras ENOSPC en build hyfetch-cargo, distfile opencode 58M con hardlink — no liberó).

| app | repo | camino | resultado | notas |
|---|---|---|---|---|
| nvm 0.40.3_1 | vup | binario+TOFU | OK exit=0, funcional (script) | T-006 (llave vía git) + T-007 (`install -D` crea `keys/`) fijados para lograrlo; TOFU muestra SHA256+urls+confirm; `yes\|` aprobó también el import-propio de xbps (doble TOFU, ver T-012) |
| basilk/gittop/vuru/ruffle/ols/v-analyzer/odin | vup | binario | OK 7/7 exit=0 | ruffle/odin ejecutados (`--version` OK); DB sin tracking (diseño Fase 1, verificado: KeyError ausente) |
| hyfetch | vup 2.0.5 vs repository 2.1.0 | — | NO instalado | el resolver elige repository-fuente (mayor versión) y `--prefer-binary` NO cambia de repo → **T-010 (medium)**: flags solo operan intra-repo; `--force-build` sobre hytale tampoco evita el bucket official. Intento de build cargo murió por ENOSPC (rust-std, ambiental, bien propagado exit=1) |
| hytale-installer | neko-binario (reinstall) | binario official | OK exit=0 | `--force-build` tomó el binario igual (ver T-010); cosmético `official/hytale-installer- [hytale-installer-_0]` → **T-009 (low)**; el reinstall escribió ficción DB → purgada, ver T-005 |
| lavat 3.0.0_2 | repository | fuente (gnu-makefile) | OK exit=0, binario corre, DB exacta | PRIMER build fuente e2e: plan source + review con template + prefetch + xbps-src + install; ciclo `-R` (xbps+DB limpios) + reinstall OK |
| nothing-fonts/bibata-modern-ice/adw-gtk-theme | repository | fuente/datos | OK 3/3 | — |
| python3-vdf-dev/python-pywalfox | cnr/repository | fuente pep517/module | OK 2/2 | — |
| better-adb-sync 1.4.0_1 | repository | fuente pep517 | OK (3m13s, hostdeps toleran "already installed") | Ctrl+C a 10s (pre-fetch): exit=130, SIN huérfanos/temporales/overlays, cero estado (ni xbps ni DB) → H-016/H-034 PASS |
| proton-keyring-linux + protonvpn-core | repository | fuente+cadena-VUR | OK exit=0 | plan mixto binario+2 builds, review por paquete; `archs="x86_64*"` (glob) tolerado; proton-vpn-* eran preexistentes (05-09); lock `index.lock` transitorio auto-limpiado |
| spotify 1.2.92.147_1 | cnr | repack binario | OK exit=0, ficheros+DB exactos | 700M de disco (distfile+binpkgs); checksum-vacío warning H-031 en vivo |
| opencode-bin | cnr | fuente | FAIL exit=1 (causa upstream) | binario prebuilt no-PIE rechazado por hook void; vary propagó bien, SIN rastro DB/xbps → error-path correcto |
| bibata-cursor-theme (foto) | z-packages/official | binario official | abortado por xbps (faltó `y`), exit=0 | T-003 RECLASIFICADO: xbps devuelve 0 al abortar (verificado directo `xbps_abort=0`);vary propaga fielmente. Propuesta: `-y` a xbps tras confirm propio |
| font-inter | official-shadow | — | no instalable vía VUR | caso subpaquetes sin cobertura en vivo (unit H-017 cubre); aquamarine alternativo exige clang21 (descartado por disco) |
| `-Syu` + downgrade lavat 0.0.1_1 | — | upgrade-detect | OK | detecta `lavat-0.0.1_1 -> 3.0.0_2` + librewolf 154.0.2_1 real upstream; EOF deny exit=1; DB restaurada |
| ciclo vup remove/purge/re-add | vup | lifecycle | OK | teardown borra key+conf (`keys/` queda vacío); `-p` purga clon; re-add OK; re-setup perezoso en próximo install |
| H-003 rotación | vup | TOFU-negativo | Hallazgo→fix | llave basura: setup ni se ejerce post-registro (vía Official); rekey intacto (teardown sin verify); **T-011 (low)**: `verify_key_tofu` Procedía con llave ilegible → fix fail-closed + test |
| cadena de firma vup | xbps | defensa-profundidad | OK | sin plist en `/var/db/xbps/keys` xbps falla cerrado (`verifying RSA signature → Transaction failed`); con plist ausente el sync pregunta import (TOFU xbps). La `.pem` de `/etc/xbps.d/keys` es anchor propio de vary, xbps no la lee → **T-012 (medium, propuesta)**: vary debería pre-importar la plist (tiene el texto) para installs no-interactivos y evitar el doble prompt |

| paso | comando | resultado | notas |
|---|---|---|---|
| 0 | backups + `vary -V` + `git status` | OK | `vary 0.2.5`; árbol limpio @ `0de05c1`; HALLAZGO-previo: `~/.cache/vary/installed.json` es DIRECTORIO LMDB (`data.mdb`+`lock.mdb`, mtime 07:40) — el binario instalado es pre-H-005; la migración a JSON la disparará la candidata 0.3.0 (ver paso 6 / T1) |
| 1 | `cargo test integracion_ -- --include-ignored` | OK 4/4 (`query_installed/query_manual/query_architecture/search_remote`, 2.32s) | dentro del 157+1 de T0.2 |
| 7-baseline | exit codes con vary 0.2.5 (foto "antes") | `badflag=1`, `asdeps=1`, `configflag=1` | DIVERGENCIA-ESPERADA vs HEAD: pre-H-035 (1 en vez de 2); `--asdeps`/`--config` TRAGADOS (H-032/H-043 reproducidos: resuelve `foo`, falla por "no encontrado"). T2 repite contra 0.3.0 |
| 2 | `VARY_DEBUG=1 vary -Syu </dev/null` (0.2.5) | OK-sync; exit=0 | sync oficial + refresh VURs (`cnr 1b4eaa33`, `z-packages 43031639`) sin errores; warnings H-031 (spotify, xbps-triggers) visibles; xbps abortó upgrades en EOF ("Aborting!") pero vary devolvió 0 — OBSERVACIÓN: upgrade abortado enmascarado como éxito en 0.2.5; verificar en candidata (posible T-###) |
| 5-baseline | doble instancia con 0.2.5 (lock vía fifo bloqueado en confirm de `vary -S cowsay`) | lecturas serializan (search=0, version=0 tras espera); 2º `-S` NO es rechazado: espera el lock y procede | baseline pre-H-027: flock bloqueante global, sin mensaje "otra instancia"; first=1 tras `n`; H-001 EN VIVO: el 2º `-S nano` aprobó el confirm con EOF y lanzó `xbps-install` sin consentimiento (inocuo aquí: `ERROR: Package 'nano' already installed`, preinstalado 2026-08-26; second=0 pese al error de xbps — otro enmascaramiento de exit en 0.2.5). T2 repite contra 0.3.0 |
| 3,4 | — | DIFERIDOS a T3 (requieren build VUR real) | review-EOF e INT-mid-build se prueban contra 0.3.0 con paquete fuente pequeño |
| 6 | migración LMDB→JSON + legacy-v1 (candidata 0.3.0) | OK (con nota) | `vary -R vary-noexiste-xyz` migró LMDB real: 2 entradas (hytale-installer/cnr, librewolf/z-packages, schema 2, build_date backfill) + `installed.lmdb.bak`; legacy-v1 con fixture CORRECTO (`"source"` minúsculas) migra (`migradas 1 entradas`, sin corrupt-backup). NOTA-DOC: el fixture de VALIDATION.md usa `"Source"` capitalizado → cae en ruta corrupt-con-respaldo (conducta correcta del código; enmendar VALIDATION). La migración es load-time en memoria; `-R` no re-persiste el archivo (diseño: sin pérdida hasta mutación real) |
