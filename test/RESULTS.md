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
| 6 | migración LMDB→JSON + legacy-v1 (candidata 0.3.0) | OK (con nota) | `vary -R vary-noexiste-xyz` migró LMDB real: 2 entradas (hytale-installer/cnr, librewolf/z-packages, schema 2, build_date backfill) + `installed.lmdb.bak`; legacy-v1 con fixture CORRECTO (`"source"` minúsculas) migra (`migradas 1 entradas`, sin corrupt-backup). NOTA-DOC: el fixture de VALIDATION.md usa `"Source"` capitalizado → cae en ruta corrupt-con-respaldo (conducta correcta del código; enmendado). La migración es load-time en memoria; `-R` no re-persiste el archivo (diseño: sin pérdida hasta mutación real) |
| `-Si` ×8 | librewolf/mdevd/ydotool/libudev-zero/font-inter/python3-inputs/bazaar/svc | OK | sombreados→official; bazaar/svc→`vur:repository` con deps + `Subpackages: bazaar-devel` (metadata subpaquetes en vivo; install sin cobertura: sombras/toolchain) |

## T4 — Estrés

| prueba | resultado | tiempo | notas |
|---|---|---|---|
| `vary -Syu </dev/null` completo | sync OK, exit=1 (EOF-deny) | 16.8s | 4 VURs + official; solo librewolf pendiente (154.0.2_1 real, rebuild vetado: horas+disco); official 4 pkgs abortan en xbps (EOF) |
| `vary -Ss ""` | exit=1 "no search pattern specified" | instant | NO es regresión H-030 (ese era el bulk interno, ejercitado en cada `-S`); UX explícito correcto |
| `vary -Ss lib` masivo | exit=0, 7268 líneas | 4.8s | A4 OK |
| consistencia post-fallos | DB válida (schema 2, 11 pkgs), sin temporales/overlays | — | fallos opencode/ENOSPC/INT dejaron cero estado corrupto |

## T5 — Reporte

Ver `test/REPORT.md`. Veredicto: **APTA PARA TAG con 3 conocidos (T-002/T-010/T-012 → 0.3.1)**. Tag humano pendiente.

## Paso 9 — Post-tag v0.3.0 (binario release)
- Workflows del tag: CI verde + XBPS verde (build musl/glibc, verify en contenedores frescos, rolling repo, release). Release `v0.3.0` publicado con `vary-0.3.0_1.x86_64{,-musl}.xbps` + `sha256sums.txt`.
- Artefacto: glibc descargado a `/tmp/vary-rel`, `sha256sum -c` OK.
- Instalación: `doas xbps-install --repository=/tmp/vary-rel -S vary` → upgrade 0.2.5→0.3.0_1 exit=0 (`/usr/bin/vary` 3.2MB root; rollback: `vary-0.2.5-bin` o debug en `target/`).
- Smoke release: `-V`=0.3.0; `-Ss lavat` 3.7s (`vur-source:repository`); `-Si lavat` exit=0 (resuelve `official` — lavat entró a repos oficiales); `-R lavat` exit=0 (xbps+DB limpios); `-S lavat` exit=0 (binpkgs local, 0 descargas); DB exacta (`repository/source/3.0.0_2`).
- Estado final: release en producción, sin rollback necesario.

## Limpieza post-misión (orden humana; 675M → 2.7G libres)

- Desinstalados vía `vary -R` (DB consistente, solo queda `librewolf`): hytale-installer, spotify, lavat, nothing-fonts, bibata-modern-ice, adw-gtk-theme, python3-vdf-dev, python-pywalfox, better-adb-sync, proton-keyring-linux, protonvpn-core (11/11 exit=0).
- Desinstalados vía xbps: Neko-Wizard, waterfox, nvm, basilk, gittop, vuru, ruffle, ols, v-analyzer, odin, pcmanfm, geany (12/12, huérfanos: 0).
- Caché vary podada (solo queda librewolf instalado): `sources/*` + `by_sha256` + `binpkgs/*.xbps` de removidos (~600M: spotify 309M el mayor).
- Perfiles/cachés apps: `~/.cache/{spotify,waterfox}`, `~/.waterfox`, `/opt/waterfox` (310M), `/var/cache/xbps`.
- Sistema: `vkpurge rm 6.12.105_1` (108 en ejecución; 164M módulos + /boot, grub regenerado).
- NO tocado (motivo): headers 6.18/7.2 en `/usr/src` (~380M, dkms instalado los puede necesitar), `~/Descargas/iMe*` (579M, archivos tuyos — dime si los borro), rustup 1.88 + cargo-registry (toolchain MSRV), repos VUR y clones (infra de vary), `test/backup/` + logs (evidencia).

## T-012 en vivo — pre-import plist VUP (evidencia en `test/backup/pre-t012/`)

- **Descubrimiento (bloqueante, rediseñó el fix):** el fingerprint canónico de xbps es **MD5-sobre-OpenSSH** (`lib/pubkey2fp.c`: MD5 de `uint32(7)+"ssh-rsa"+mpint(e)+mpint(n)`), NO SHA256-del-DER que vary mostraba. Verificado: algoritmo replicado coincide en **12/12 llaves** del keyring + vector en vivo. xbps busca `/var/db/xbps/keys/<fp-xbps>.plist` por NOMBRE: un plist bien escrito con nombre SHA256 es invisible (probado: prompt igual). `vary` ahora calcula/muestra/pinea en estilo xbps; pins SHA256 legacy se siguen aceptando.
- **Test0 baseline** (release 0.3.0, sin plist, `</dev/null`): xbps pide importar (`Fingerprint: 78:b8...`) y aborta `ERROR: Failed to import pubkey`, exit 1 → fail-closed confirmado (`test0-xbps.log`).
- **Repo simulado autocontenido** `t012local` (llave RSA + repo xbps firmado + `index.json` + git `keys/*.plist`, todo local; `file://` no sirve a xbps → se sirvió por `http://127.0.0.1:8765/`; `repo add` rechaza paths locales → alta manual en repos.conf con pin):
  - **B4 happy-path:** `vary -S t012dummy --noconfirm </dev/null` → plan `t012local/...`, `Fingerprint (xbps): 47:9b...`, pre-import al nombre correcto, **cero prompts xbps**, exit 0, instalado y firma verificada (`testB4d-vary.log`). Un solo consentimiento ✅.
  - **B5a pin:** pin viejo + llave nueva → `fingerprint ... NO coincide (esperado/recibido)`, exit 1, sin escrituras ✅.
  - **B5b rotación H-003:** pin nuevo + pem instalado viejo → `ALERTA DE SEGURIDAD CRÍTICA ... Instalada previamente / Recibida remotamente ... BLOQUEADA`, exit 1, keyring intacto (mtime preservado, sin plist nuevo) ✅.
  - **B6 rekey:** `vary --repo rekey` retiró conf+pem+plist (`plist pre-importado retirado` en log); reinstalación con llave nueva OK sin prompts ✅.
- **Limpieza/restauración verificada:** `repo remove -p`, `xbps-remove` dummies, server off, `repos.conf`/`installed.json`/`/etc/xbps.d` idénticos a backups (`diff`/comparación exacta), keyring sin restos + plist original restaurado, privadas destruidas (`shred`). Solo quedan logs + llaves públicas + backups en `pre-t012/`.
- **Corrección 2026-09-07 (importante): NO hubo rotación upstream.** Lo anotado como "rotación mid-test" era la misma llave K1 en dos fingerprints: `9f:c1` (SHA256-del-DER, formato viejo de vary) vs `78:b8` (MD5-sobre-OpenSSH, formato xbps — verificado MD5-SSH(K1)==`78:b8...`). El fallo de Test1 fue solo el nombre del plist (SHA256 en vez de xbps). Sistema consistente (K1 en git, keyring y repodata); ninguna acción sobre upstream.
- **Observado fuera de alcance:** installs VUP-binarios no dejan rastro en `installed.json` (`vur_map_lookup_repo` no cubre sintéticos VUP) — candidato a 0.3.1. Tests negativos de regalo: plist con `<data>` vacío y repo sin firmar fallan cerrado con mensajes claros.

## P0-1 en vivo — `preflight.rs` (chequeos previos duros)

- Normal en este host: pasa en silencio (0 warnings) — uchroot 4750, sin OCI/chroot.
- Root directo (`doas env -u DOAS_USER vary -Ss`, euid 0 sin wrapper): `error: vary no debe ejecutarse como root directo...`, exit 1 ✅.
- uchroot a 0755: `error: xbps-uchroot en /usr/bin/xbps-uchroot con modo 755 (se exige 4750...); corrige con: doas chmod 4750...`, exit 1; modo restaurado a 4750 y verificado, run posterior exit 0 ✅.
- Resto de la matriz (OCI, chroot degradado, fallback `/usr/libexec`, ausente/ilegible) en unit tests (`preflight::tests`, tabla de 6).
- **Fix-forward:** el abort-root tumbó los builds XBPS (los contenedores CI son root en docker: `vary -V` del packaging moría exit 1). Excepción: root + OCI detectado avisa y sigue (daño contenido); root fuera de contenedor sigue abortando. Verificado en pipeline real (build+verify verdes) + test `root_en_contenedor_avisa_y_sigue`. Limitación conocida: solo se detecta docker (`/.dockerenv`, cgroup `docker|kubepods`), no podman (`/run/.containerenv`).

## P0-4 en vivo — `vary -Sp` (plan-then-mutate, H-007 superado)

- `-Sp curl` → `official/curl`, sin builds, `elevations: 1`, exit 0; `-Sp python-pywalfox` → installs oficiales + `level 0: python-pywalfox-2.7.4_2`; `-p curl` (sin -S) también imprime.
- Errores honestos: sin targets (exit 1), `--print-format=json` (exit 2), `-p` pelado, `-Ss -p`, paquete inexistente (`no encontrado`, exit 1).
- **Cero mutación del sistema:** `cache.json` mismo mtime antes/después, sin ficheros nuevos, sin lock, sin elevación (congelado: sin bootstrap/ensure-fetch/save/review/DB). Precisión honesta tras hallazgo en vivo: el índice VUP SÍ puede refrescarse por red (solo escribe su caché, como `-Sy`; sin comando standalone que lo refresque, prohibirlo dejaba `-Sp` inservible con caché tibia). "No toca disco/red" = cero escrituras en el sistema + cero red elevada + cero elevación.
- **Regresión ruta normal:** `vary -S lavat` (fuente repository) compiló+instaló OK tras la extracción (exit 0); DB exacta (`repository/source/3.0.0_2`); `vary -R lavat` limpió binario+DB (quedan brave+librewolf).

## P0-2 en vivo — TOFU continuo + `re-trust` (evidencia en `test/backup/pre-p02/`)

- **Corrección previa:** lo anotado en T-012 como "rotación upstream" era la misma llave en dos fingerprints (SHA256 vs xbps); verificado MD5-SSH(K1)==`78:b8...`. No hubo rotación real (FIX_LOG y sección T-012 enmendados).
- **Repo simulado** `p02local` (llave RSA + repo xbps firmado + `index.json` + git, registro manual con pin y sin fecha = estado pre-feature): tras rotar a keyB y re-firmar todo (repodata+pkgs; `xbps-rindex` no sobrescribe `.sig2` sin regenerar),
  - `vary -Sy` aborta forense en `p02local` (exit 1): `ALERTA (P0-2, cambio de llave)`, instalada `17:ab...` (confiada "en fecha desconocida"), recibida `dd:b4...`, hint `re-trust`; el resto de repos no se refresca (fail-closed).
  - `vary --repo re-trust` (interactivo; `--noconfirm` se rechaza): muestra vieja→nueva+fecha, confirma, retira lo viejo (incluye plist), re-registra, fija pin (`dd:b4...`) + fecha (epoch). `install p02dummy` posterior OK sin prompts.
- Fixture famoso: instalé por error el plist XML como `.pem` (el chequeo calló con warn en vez de abortar — por diseño: ausencia de evidencia ≠ evidencia; el gate duro quedó en setup). `pkill -f` se suicida con su propio patrón (usar `pgrep -f "http[.]server"`).
- **Limpieza verificada:** remove `-p`, xbps-remove, server off, `repos.conf`/`installed.json` idénticos, sin restos en `/etc`, keyring, clones ni caché; privadas con `shred`.

## P0-5 en vivo — pinning (`repo_commit` + `artifact_sha256`, schema v3)

- `vary -S lavat` (fuente): DB con `repo_commit` == `git rev-parse HEAD` del clon repository y `artifact_sha256` == `sha256sum` del `.xbps` construido (verificados independientes) ✅.
- Drift simulado (repo_commit tampered + `--force-build`): avisa `drift de procedencia: 'lavat-3.0.0_2' reinstalado desde otro commit (tampered0000 -> 4530c62d7ba6)` y calla el artefacto (mismo caso: rebuild normal) ✅.
- **Gap VUP-DB cerrado:** `vary -S basilk` (binario vup) deja rastro (`vup/binary`, commit del índice, artifact del caché xbps verificado) y `vary -R basilk` lo elimina limpiando la DB ✅ (antes quedaba huérfano).
- Limpieza: `-R` lavat + `xbps-remove` nvm (official, nunca rastreado por diseño); DB de vuelta a brave+librewolf; caché de build podada.

## P0-3 en vivo — auditoría de templates (`template_audit.rs`)

- `vary -S spotify </dev/null` (sin `--noconfirm`): el review muestra `:: AUDIT template spotify (repo cnr): 3 HIGH, 0 MEDIUM` SOBRE el template (checksum-ausente + 2× descarga-en-build con las líneas curl reales) y aborta en el confirm (exit 1, nada instalado, nada descargado) ✅.
- Reglas calibradas para no fatigar: en contenido completo solo checksum+descargas (URLs/hooks serían ruido: todo es "nuevo"); en diffs, checksum solo si regresa (lo tenía y lo pierde) + añadidas (descargas HIGH, URLs/hooks MEDIUM).
- Hallazgos siempre consultivos (el gate decide igual; no-TTY sin `--yes` ya abortaba por A3). Sin flags nuevos (sin cambios en help); sin sección README (el review nunca se documentó ahí).
- Cobertura: corpus en CI (veredicto por regla) + wiring del gate con findings en test + demo viva del install-review. El display del upgrade-diff comparte helper y tests; sin update pendiente real para demo viva de esa rama (documentado).

## P1-1 en vivo — `vary.lock` reproducible

- `vary --lock` genera `~/.config/vary/vary.lock` (4 repos con url/commit/fingerprint + 189 paquetes con versión/origen/sha); **determinista** (sha256 idéntico en regeneraciones; sin timestamps).
- Dos bugs hallados y corregidos en vivo: (1) `xbps-query -m` rinde pkgvers, no nombres → claves duplicadas; fix en `query_installed` (propiedad `pkgname`, tolerante). (2) Semántica congelada de `-Sp`: prohibir red dejaba `-Sp` inservible con caché VUP tibia sin remedio no-mutante; ahora el índice VUP puede refrescarse (solo escribe su caché, como `-Sy`). "No toca disco/red" = cero escrituras en el sistema + cero red elevada + cero elevación.
- Verificación: `-Sp` muestra sección `lock:` (`ok` en limpio); lock tampered (brave `0.0.0_1`) → `lock: 'brave-origin' resuelve brave-origin-1.93.129_1 pero pinea ...` con exit 0 y sin mutación; regenerado idéntico después.
- Oficiales se omiten en la comparación de versiones (placeholder T-009 daría falsos avisos; xbps es su fuente de verdad); commits de repos sí se verifican.
- CLI: `vary --lock` pelado regenera; `-S/-Sy --lock` regeneran al final; `-R --lock` se rechaza; con lock, install/upgrade/`-Sp` avisan divergencias.

## P1-2 en vivo — `vary --challenge lavat --experimental`

- Guards: sin `--experimental` → error; `-S --challenge` → error; 0/2 targets → error.
- Instalado lavat (fuente) → challenge: rebuild + `sin divergencias (2 ficheros comparados)` (el rebuild es bit-reproducible aquí) ✅.
- Divergencia inyectada (template del clon + fichero nuevo en `do_install`, restaurado con `git checkout` después): `challenge lavat: 1 divergencias: [falta-en-disco] /usr/share/lavat/INYECTADO` ✅. Los kinds `Changed`/`Added` restantes van en tabla unitaria CI (el `.xbps` se lee por streaming `ruzstd`+`tar` puro Rust: `tar` del sistema no lee zstd aquí; miembros de metadata `props/files.plist` se excluyen).
- Limpieza: `-R` lavat + poda de binpkgs/sources; DB de vuelta a brave+librewolf; clon repository limpio (`status` vacío).

## P2 en vivo — `vary --why` + `vary --log` (diario)

- `vary --why curl`: no gestionado por vary; requerido por `xtools-0.70_1` (xbps -X); declarado en `cnr/hytale-installer`, `cnr/mullvad-vpn` ✅.
- `vary --why librewolf`: gestionado (`librewolf-153.0.4.1_1 [z-packages] (Source)`), sin revdeps ni declarantes ✅.
- `vary --log` vacío → `sin historial...`; tras install+remove lavat: `... INSTALL lavat lavat-3.0.0_2 repository` + `... REMOVE lavat` (2 líneas exactas; un susto de "duplicado" fue error de lectura de pipes intercalados, verificado con `wc -l`) ✅.
- Nota: el diario conserva esas 2 líneas (historial legítimo; borrar con `rm ~/.local/share/vary/operations.log` si se quiere pristine).

## P2 restante evaluado (no implementado; decisión humana requerida)

- **404 distfiles (mirrors + backoff): NO en vary.** El fetch lo ejecuta `xbps-src` (`md.fetch_pkg`), que ya trae sus propios reintentos y soporta mirrors vía su configuración; el pre-fetch de vary es warm-up best-effort (`let _`). Duplicar reintentos en vary no aporta y los "mirrors alternos" no existen en el modelo de datos (requeriría formato VUR nuevo). Capa equivocada.
- **Soname drift trigger: NO sin decisión de política.** Requiere (1) fuente de sonames + (2) grabar baseline por build (schema v4) + (3) política: ¿avisar o recompilar solo? Recompilar sin consentimiento explícito es peligroso; avisar es útil pero cambia `-Syu`. Propuesta si se aprueba: pin `shlibs_requires` al instalar + aviso en upgrade (nunca auto-rebuild sin flag).
- **P1-3 (masterdirs aislados): NO tocado** — la spec exige decisión humana expresa (riesgo de corrupción del repo local).

## Post-tag v0.4.0 — release en Void real (2026-09-07, VALIDATION paso 9 extendido)

- Tag `v0.4.0` → CI success + XBPS success (ambos workflows del tag en verde).
  Release `v0.4.0` publicado con `.xbps` glibc + musl (artefactos descargados OK).
- Instalación glibc: binario extraído del `.xbps` (`bsdtar`, tar del sistema
  no lee zstd) → `doas install` sobre `/usr/bin/vary` (rollback listo:
  `test/backup/2026-09-07/vary-0.3.0-bin` + `test/backup/2026-09-06/`).
- Smoke release 0.4.0 (5 min, todo exit 0): `-V` (0.4.0), `-Ss lavat`,
  `-Si lavat` (official + vur), `-S vuru --noconfirm` (binario VUP, sin
  compilar) con rastro DB perfecto (`binary/vup` + commit + artifact),
  `-R vuru`, DB final `[brave, librewolf]`, 0 paquetes prueba.
- **basilk ya no es VUP: entró a repos oficiales** → `-S basilk` instala como
  `official/` y NO deja rastro (T-005 por diseño, no regresión; verificado
  vía `-Sp`). El gap VUP-DB de P0-5 sigue cerrado (demostrado con vuru).
  Observación cosmética preexistente (no 0.4.0): `remove` anota REMOVE en el
  diario aunque el paquete no tuviera entrada (install oficial no anota
  INSTALL) — asimetría conocida, sin acción.
- musl: artefacto válido (ELF x86-64) pero **dinámico** (intérprete
  `/lib/ld-musl-x86_64.so.1`, ausente en este host glibc) → no ejecutable
  aquí. Sin defecto: el job `verify musl` del workflow (verde) ya lo ejecutó
  en contenedor musl. El sistema queda con glibc por orden.
