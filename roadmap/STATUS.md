# STATUS — Hoja de Ruta Post-Auditoría

**Fecha:** 2026-09-06/07 · **Base:** `vary-mvp` @ FASE 5 (46/47 + A3 + A6; resta H-015).
**0.4.0 (2026-09-07):** P0-1, P0-4, P0-2, P0-5, P0-3, P1-1, P1-2, P2-why, P2-log,
P2-soname **implementados y cerrados con CI+XBPS verdes**; P2-404 **no se
implementa** (decisión de diseño, ver abajo). Cada ítem restante es SPEC con
criterios de aceptación.

## 0.5.0 — ÚNICO ítem: P1-3 · Masterdirs aislados estilo xbps-fbulk (+H-015)

---

## P0-1 · `preflight.rs` — chequeos previos duros

- Abort si `euid == 0` (vary nunca como root directo; solo vía wrapper).
- Chroot degradado (`proot`/`bwrap` detectado) → warning crítica, seguir.
- `xbps-uchroot` sin setgid `4750` → error con hint de corrección.
- Detectar OCI (`/.dockerenv`, `/proc/1/cgroup` con `docker|kubepods`).
- Aceptación: tabla de entornos × resultado esperado en tests + CI.

## P0-2 · TOFU cerrado + `re-trust`

- Cambio de fingerprint o URL en repo registrado durante update → abort
  forense (qué cambió, cuándo se confió) + `vary repo re-trust <name>`.
- Justificación: incidente Atomic Arch (jun-2026); H-003 cubre solo el alta.
- Aceptación: test con rotación simulada aborta; `re-trust` reanuda.

## P0-3 · `template_audit.rs` — `audit_template() -> Vec<AuditFinding>`

- Reglas: checksum ausente → High; en updates, parsear `git diff` del
  template (URLs nuevas, descargas en build: `npm`/`pip`/`curl|sh`, hooks
  añadidos).
- Hallazgos visibles SOBRE el diff del pager y dentro del review gate (A3
  muestra el diff; P0-3 lo califica).
- no-TTY aborta salvo `--yes`.
- Aceptación: corpus de templates con veredicto esperado por regla.

## P0-4 · `--print` real (plan-then-mutate)

- Niveles topológicos del plan, batch binaria única, cola de builds,
  nº de elevaciones (= 1). Reemplaza el disable temporal de H-007.
- Aceptación: `vary -Sp foo` no toca disco/red con elevación; salida
  parseable por scripts.

## P0-5 · Pinning (`repo_commit` + `artifact_sha256` en installed.json)

- Migración tolerante (campos opcionales, backfill cuando haya dato).
- Warning de drift binario (binario instalado ≠ artefacto del commit pineado).
- Aceptación: roundtrip + migración v2→v3 sin pérdida (patrón H-005).

## P1-1 · `vary.lock` (TOML)

- Repos (url/commit/fingerprint) + paquetes (version/source/sha256).
- `vary sync --lock` regenera; upgrade contra lock es reproducible.
- Aceptación: dos máquinas, mismo lock, mismo árbol instalado.

## P1-2 · `vary challenge` (build local vs binario servido)

- Recompila local, compara árboles instalados, reporta divergencias.
- Tras `--experimental` hasta ganar confianza estadística.
- Aceptación: divergencia inyectada (timestamp, flag) detectada y reportada.

## P1-3 · Masterdirs aislados estilo xbps-fbulk (+H-015)

- Aquí vive H-015: un masterdir por worker, `xbps-uchroot`, indexación
  serializada de `binpkgs`. Implementar SOLO tras decisión humana expresa
  (riesgo de corrupción de repo local).
- Precondición: P0-1 (detección de uchroot/setgid).
- Aceptación: builds concurrentes de ramas independientes sin colisiones +
  repodata íntegro (ver `roadmap/SPIKE_MASTERDIR.md`).

## P2 · Miscelánea

- `vary why <pkg>` ✅ 0.4.0 — cadena de dependencia que lo trajo.
- `vary log` ✅ 0.4.0 — historial legible (`~/.local/share/vary/operations.log`).
- Resiliencia 404 en distfiles: ❌ **NO IMPLEMENTAR (decisión de diseño
  2026-09-07, confirmada por el mantenedor).** El fetch lo ejecuta `xbps-src`
  (`masterdir.fetch_pkg`), que ya trae reintentos propios y soporta mirrors vía
  su configuración; el pre-fetch de vary es warm-up best-effort (`let _`).
  Duplicar reintentos/backoff en vary no aporta nada observable y los "mirrors
  alternos" no existen en el modelo de datos (requerirían formato VUR nuevo +
  cambios en `metadata.rs` + generador). Capa equivocada; se cierra como
  no-ítem.
- Trigger preventivo de recompilación por drift de sonames (resto de H-029/A5)
  ✅ 0.4.0 con alcance aprobado: pin `shlibs_requires` al instalar + aviso en
  upgrade; **nunca auto-rebuild sin flag explícito** (compilar sin
  consentimiento es peligroso). Ver P2-soname abajo.

## P2-soname · Pin de sonames + aviso de drift (alcance aprobado 2026-09-07)

- Al instalar un paquete Source se graba `shlibs_requires` (salida de
  `xbps-query -R --property rundeps` del artefacto construido... ver
  implementación) en `installed.json`.
- En upgrade, si los sonames requeridos ya no los provee la misma versión que
  al instalar → aviso CONSULTIVO que sugiere recompilar; jamás recompila solo.
- Sin baseline (paquetes pre-0.4.0) → sin aviso, sin ruido.
- Aceptación: pin grabado al instalar; drift simulado avisa; sin flag no hay
  rebuild.

---

## Orden sugerido

P0-1 → P0-4 → P0-2 → P0-5 → P0-3 → P1-1 → P1-2 → P1-3 (con decisión) → P2.
