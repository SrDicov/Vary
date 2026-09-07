# SMOKE 0.4.0 — superficies nuevas en Void real (2026-09-07)

Binario: `target/debug/vary` construido de `fc98fe8` (CI+XBPS verdes).
**Desviación registrada:** `cargo build` local (la operativa lo prohíbe salvo
caída de CI) porque el smoke pre-tag exige binario con el código nuevo y CI
no entrega binarios. Solo build; sin test/clippy local. 63 s.
Logs crudos: `test/smoke-0.4.0/`.

## S1 — `-Sp` plan sin mutar ✅
`vary -Sp lavat`: niveles (`level 0: lavat/lavat-3.0.0_2`), batch única,
`elevations 1`, sección `lock: ok`. `installed.json` sha-idéntico después.
Nota: WARN preexistente `failed to load index for 'vup'` (ese repo no sirve
.VURINFO ni templates; no es de este ciclo).

## S2 — `vary --lock` + divergencia ✅
`--lock` regenera (4 repos, 189 pkgs). Lock adulterado (brave `0.0.0_1`) →
`-Sp` avisa verbatim con exit 0 y sin mutar; restaurado byte-idéntico.
Al final del smoke se regeneró (cnr avanzó commit durante `-Sy`).

## S3 — challenge ✅ con INCIDENTE
- Guard: `--challenge curl` (no gestionado) → `error:` + exit 1 ✅.
- **INCIDENTE:** `--challenge librewolf` NO abortó como supuso el operador:
  z-packages SÍ sirve template compilable y xbps-src empezó el build real.
  Kill por timeout a los 5 min. Limpieza verificada: sin procesos, artefacto
  parcial (122 MB) eliminado, DB/journal/checkout intactos, disco 2.1G.
  No es bug (el comando hizo lo pedido: rebuild explícito + `--experimental`),
  es ausencia de guardarraíl de tamaño/tiempo por diseño no-interactivo.
  → Limitación conocida en CHANGELOG 0.4.0.
- `vary -S lavat --noconfirm` → install OK; **DB con pins P2-soname en vivo**:
  `soname_pins={'libc.so.6': 'glibc-2.41_1'}` + commit + artifact ✅.
- `--challenge lavat --experimental` → rebuild + `sin divergencias
  (2 ficheros comparados)`, exit 0 ✅. `-R lavat` limpia (DB sin lavat,
  journal con INSTALL+REMOVE) ✅.

## S4 — re-trust: abort forense por cambio de URL ✅
`vary -Sy` con URL de `vup` adulterada → `error: ALERTA DE SEGURIDAD (P0-2,
cambio de URL en 'vup')` con URL confiada vs configurada + hint correcto
(re-trust no mueve origen; toca remove+add), exit 1, fail-closed.
`repos.conf` restaurado byte-idéntico; `/etc` y keyring intactos.
(Re-trust interactivo completo ya validado en P0-2; el flujo no cambió desde
entonces — `git log` sobre `keys.rs`/`repo.rs` lo confirma en reconciliación.)

## S5 — audit con `--yes` (ruta nueva F1) ✅
Repo simulado `file:///tmp/smoke-audit` con `fakepkg` sin checksum:
`vary -S fakepkg --noconfirm` imprime
`:: AUDIT template fakepkg (repo smokeaudit): 1 HIGH, 0 MEDIUM` +
`[HIGH checksum-ausente]` y luego falla el fetch (exit 1, nada instalado,
nada en DB). Sub-hallazgo: `repo add` y el clone rechazan paths desnudos
(guardia anti-SSRF; `file://` sí pasa) ✅. Sim eliminado por completo.

## S6 — `-v`/`-vv` (T-002, binario nuevo) ✅
`-Sp`: sin flags 0 DEBUG, `-v` 1 DEBUG, `-vv` 9 TRACE.

## Estado final
DB `[brave-origin, librewolf]`; `repos.conf` original; 0 paquetes prueba;
disco 2.1G; `vary.lock` regenerado y `ok`.
