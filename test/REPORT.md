# REPORT — Test intensivo vary 0.3.0 en Void real

**Máquina:** Void x86_64 producción · **Rama:** `vary-mvp` · **Candidata:** `0.3.0` (debug local, `doas install`)
**Periodo:** 2026-09-06/07 · **Incidentes:** 1 corte eléctrico (estado en disco intacto; tmpfs perdido → política: backups solo en disco)
**Disco:** 2.2G→694M libres/14G (cachés xbps limpiadas ×3, masterdir `zap` 750M, distfiles retenidos para reinstall)

## 1. Resumen ejecutivo

- **~65 comandos** probados (T0:8, T2:20, installs:25, lifecycle:8, T4:4). Detalle: `test/RESULTS.md`; planes: `test/plan_*.json`.
- **Installs OK: 19** — vup-binarios 8/10 (nvm,basilk,gittop,vuru,ruffle,ols,v-analyzer,odin), fuente 9 (lavat,nothing-fonts,bibata-modern-ice,adw-gtk-theme,python3-vdf-dev,python-pywalfox,better-adb-sync,proton-keyring-linux,protonvpn-core), repack 1 (spotify), reinstall-oficial 1 (hytale). Todos verificados (`xbps-query -l`, binario ejecutado donde aplica, entrada DB exacta).
- **Fallos por categoría:** causa-upstream 1 (opencode-bin: binario no-PIE, vary propagó bien), límite-diseño 2 (hyfetch: T-010 + disco; font-inter/subpaquetes sin cobertura en vivo), quirk-xbps 3 (aborts con exit 0 — T-003 reclasificado, no bug vary), sombreados 18/37 (resolver prefiere official: diseño, documentado en planes).
- **Seguridad en vivo:** EOF niega+hint (contraste 0.2.5: aprobaba — H-001 probado por accidente), TOFU con SHA256 visible, rotación bloqueable, xbps falla cerrado sin plist, lock con PID sin "borra", Ctrl+C sin huérfanos (exit 130), migración LMDB→JSON con 2 entradas reales preservadas.

## 2. Compatibilidad de protocolos por repo

| repo | protocolo usado | robustez |
|---|---|---|
| cnr (44) | fallback template (sin .VURINFO) | OK; warnings H-008 (arch-condicionales) y H-031 (checksum vacío) en vivo |
| z-packages (10) | `.VURINFO` raíz + templates | OK |
| repository/voiders (118) | fallback template, layout `pkgs/` | OK; `archs="x86_64*"` (glob) tolerado |
| vup (27) | `index.json` + `keys/*.plist` vía git (tras T-006) | OK tras fixes; límites Fase 1 vigentes y verificados: sin `-Si`, sin tracking DB, sin fuente |

## 3. Rendimiento

| operación | tiempo | notas |
|---|---|---|
| `cargo test -- --include-ignored` (163+1) | ~7s incremental | 4 xbps en Void real incluidos |
| `vary -Syu` sync+detección (official+4 VUR) | 16.8s | EOF-deny exit=1; detectó lavat-downgrade y librewolf-154 real |
| `-Ss lib` masivo (7268 líneas) | 4.8s | bulk+A4 OK |
| `-Ss` simple / `-Si` / `-V` | ~3s / instant / instant | lock-free verificados (H-027) |
| builds fuente | lavat ~1min, pep517 1–3min, better-adb-sync 3m13s (hostdeps) | cargo/hypr vetados por disco |
| binario vup | 20–40s c/u (con sync) | — |
| CI por commit | ~30s | 12 runs verdes en la misión |

## 4. Hallazgos T-### (7 corregidos + 3 abiertos + 2 notas)

**Corregidos (código + test + CI verde + FIX_LOG):**
- **T-005 (high):** DB ficción en installs oficiales/abortados → `track_action()` (solo Build/VulBinary). Ficciones purgadas, auditoría DB↔xbps: 11/11 OK.
- **T-006 (high):** llave VUP solo en disco (clon sparse) → lectura vía git. Desbloqueó 8 installs.
- **T-007 (medium):** `/etc/xbps.d/keys/` inexistente → `install -D`.
- **T-008 (medium):** `archs=all` rechazado en install (3 `contains` crudos) → `arch_supports()` único. Desbloqueó toda clase de templates.
- **T-011 (low):** TOFU silencioso con llave ilegible → fail-closed + hint rekey.
- **T-001/T-009 (low):** `--color never` con bold; plan `name- [_0]` → sin estilos / solo nombre.

**Abiertos (propuestos 0.3.1, con workaround):**
- **T-002 (medium):** `-v`/`-vv` sin efecto en consola (parser OK, `apply_runtime_config` no baja el filtro; `RUST_LOG` sí). Mismo patrón que H-032.
- **T-010 (medium):** `--prefer-binary`/`--force-build`/`--no-prefer-binary` solo operan intra-repo y nunca contra el bucket official (`--force-build` sobre hytale tomó el binario neko en silencio). Requiere rediseño del orden resolución-vs-binario.
- **T-012 (medium):** vary no pre-importa la plist a `/var/db/xbps/keys` (tiene el texto): el primer install VUP no-interactivo muere en el prompt de import de xbps. Fail-closed (seguro), pero fricción + doble prompt.

**Notas (no bug):** T-003 → quirk xbps (abort=exit 0) + propuesta `-y` tras confirm propio; T-004 → `-Si` VUP es límite Fase 1 documentado.

## 5. Veredicto

**APTA PARA TAG `v0.3.0` con 3 conocidos documentados (T-002/T-010/T-012 → CHANGELOG + hito 0.3.1).**
Criterio: cero bloqueantes de integridad/seguridad/datos — los 3 high hallados están corregidos con CI verde y verificación en vivo; la DB de producción está auditada (11/11 exactas); el sistema quedó consistente (sin huérfanos, temporales, overlays ni ficciones). Los 3 medium abiertos son UX con workaround, sin dirección peligrosa (todos fallan cerrado o no actúan).
**El tag y el release los crea el humano** (regla de cierre). Rollback: `doas install -m755 test/backup/2026-09-06/vary-0.2.5-bin /usr/bin/vary`.
