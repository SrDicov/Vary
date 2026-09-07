# COMPLIANCE MATRIX — Matriz de Cumplimiento Contractual (`vary`)

**Fecha:** 2026-09-06 (cierre de auditoría FASE 5) + revisión 0.4.0 (2026-09-07)
**Rama / Commit:** `vary-mvp` @ `fc98fe8` (suite CI verde: 235 passed, 0 failed, 4 ignored; XBPS verde)
**Requisitos:** 11 (R1,R2,R3,A1,A2,A3,A4,A5,A6,A7,A8) en 10 filas (R3+A1 comparten fila: batch+elevación única).
**Historial:** matriz inicial @ `1e32f1f` (1/10 implementado). Revisión FASE 5 re-verificó cada ítem. Revisión 0.4.0 cierra A5 (trigger de sonames, alcance consultivo aprobado) — ver §4.

---

## 1. Resumen Ejecutivo de Cumplimiento

| Total Requisitos | ✅ IMPLEMENTADO | 🟡 PARCIAL | ❌ AUSENTE |
|:---:|:---:|:---:|:---:|
| 11 | 10 (R2,R3,A1,A2,A3,A4,A5,A6,A7,A8) | 0 | 1 (R1) |

**Conclusión:** el contrato está ejecutado salvo R1 (diferido a P1-3 por decisión humana PC-2: exige masterdirs aislados estilo xbps-fbulk antes de paralelizar; único ítem de 0.5.0). El trigger preventivo de A5 se cerró en 0.4.0 con alcance consultivo aprobado (P2-soname: pin + aviso, nunca auto-rebuild).

---

## 2. Tabla Detallada de Verificación Contractual

| ID | Requisito Contractual | Estado | Evidencia actual |
|---|---|---|---|
| **R1** | **Paralelismo topológico de builds** (niveles, `max_concurrent_builds`, JoinHandle, sin huérfanos). | ❌ **AUSENTE (DIFERIDO A P1-3)** | Builds secuenciales por Opción A (decisión PC-2) + H-015. `max_concurrent_builds` forzado a 1 en `config.rs`. Hacerlo sin masterdirs aislados colisiona en `binpkgs`/repodata. Spec en `roadmap/STATUS.md` P1-3; aquí vive H-015. |
| **R2** | **`.VURINFO` como caché de prioridad con fallback estructurado** y fallo ruidoso. | ✅ **IMPLEMENTADO** | H-008 (multilínea/comillas/arch-warning), H-017 (subpaquetes), H-018 (errores a stderr vía `eprintln!` + `skipped_index_warning()` testeado). `load_index` fusiona `.VURINFO` + raíz + template. |
| **R3 + A1** | **Batch transaction**: una sola invocación `xbps-install` + una sola elevación. | ✅ **IMPLEMENTADO** | `all_install_names` en una llamada (`install.rs`). H-009 añadió `--` anti-inyección. |
| **A2** | **Shell parser fallback estricto** (comillas, multilínea, arrays, aviso ruidoso). | ✅ **IMPLEMENTADO** | H-008 (`has_unclosed_quote`, sin cuelgues), H-010 (validación `pkgname`), H-017 (subpaquetes con vars aisladas), H-018 (fallo visible). |
| **A3** | **Diff pager con `diffy`** en updates: template local vs remota + consentimiento. | ✅ **IMPLEMENTADO** | `feat(A3)`: `read_template()` pre-pull, `diffy::create_patch`, pager/`$PAGER` en TTY (plano, sin coloreado: mejora P0-3), consentimiento por paquete; no-TTY aborta salvo `--yes`. Tests: diff puro + ambas ramas no-TTY. |
| **A4** | **Bulk query cache en RAM**, sync antes de query. | ✅ **IMPLEMENTADO** | `bulk_official_names()` (`HashSet` en RAM, sin `/tmp`). H-030: `""` verificado equivalente a `'*'` en Void real (comentario fija); orden seguro por diseño (cada miss se confirma escalar en vivo). |
| **A5** | **`build_date` ms + recompilación preventiva**. | ✅ **IMPLEMENTADO (0.4.0, alcance consultivo)** | `build_date` ms existe, se puebla y migra con backfill (H-005). Trigger preventivo cerrado como P2-soname (`src/soname.rs`): pin `shlib → pkgver del proveedor` al instalar Source (intersección requires∩provides vía `xbps-query` local, sin red) + aviso en `-Syu` (Bumped/Orphaned); **nunca auto-rebuild sin flag** (decisión aprobada 2026-09-07). Sin baseline → silencio. Tests puros + roundtrip + e2e vivo (pins de lavat). |
| **A6** | **Descubrimiento 3 niveles** (monorepo profundo, raíz, nombre desde dentro). | ✅ **IMPLEMENTADO** | `feat(A6)`: `con_nivel_extra()` lista `cat/pkg` (profundidad 2) vía un `ls-tree -r`; `load_index`/`project_pkg` ya resolvían el nombre real desde dentro. Límite documentado: profundidad 3+ invisible. |
| **A7** | **Logs + spinners `indicatif`**, flush garantizado. | ✅ **IMPLEMENTADO** | H-020 (`run_logged`: pipes a hilos de bombeo → `<cache>/logs/xbps-src.log` + spinner oculto fuera de TTY; desviación justificada: hilos directos en vez de `mpsc`, equivalente sin intermediario). H-028 (niveles recargables + `shutdown()` antes de cada `exit()`). |
| **A8** | **Review gate pre-build** (y/N, `--yes`, no-TTY aborta con hint). | ✅ **IMPLEMENTADO** | H-001 (EOF = denegación), H-019 (hint `--noconfirm` a stderr), H-033 (`--yes`), review no-TTY plano (sin paginador), más puerta A3 en upgrades. |

---

## 3. Desviaciones Justificadas (vigentes)

1. **R1 → P1-3:** paralelizar sin aislamiento de masterdirs corrompe `binpkgs`/repodata (análisis en `roadmap/SPIKE_MASTERDIR.md`). Decisión humana PC-2 confirmada.
2. **A5 trigger → P2-soname (cerrado 0.4.0):** `build_date` almacenado + pin de sonames al instalar + aviso consultivo en upgrade; auto-rebuild descartado por peligroso (decisión aprobada). Fuera de alcance restante: nada.
3. **A7 sin `mpsc`:** bombeo directo por hilo a handles clonados del log; mismo orden causal por stream, menos piezas móviles.
4. **A3 sin color:** diff unificado plano (seguro en `less -R` y pipes); coloreado en P0-3 (FASE 6).

---

## 4. Estado Final

FASE 4 cerrada: 46/47 hallazgos + A3 + A6 (resta H-015, diferido). Suite CI verde en `c1f7cca` (153 passed, 4 ignored; los 4 exigen Void real y corren en FASE 5 local). Detalle por hallazgo: `audit/FIX_LOG.md` + `audit/AUDIT_REPORT.md`. Smoke en Void real: `audit/VALIDATION.md` (ejecuta el humano).

**Revisión 0.4.0 (2026-09-07):** roadmap P0-1, P0-4, P0-2, P0-5, P0-3, P1-1, P1-2, P2-why, P2-log, P2-soname cerrados (cada uno con CI+XBPS verde en su commit); P2-404 no-ítem por decisión de diseño; P1-3/H-015 único ítem de 0.5.0. Mini-auditoría pre-tag (0 mayores, 7 menores → 6 fixes + 2 gaps aceptados) + smoke en Void real con evidencia (`test/SMOKE-0.4.0.md`, 1 incidente registrado y limpio). Suite CI verde en `fc98fe8` (235 passed, 0 failed, 4 ignored). Estado contractual final: **10/11** (solo R1 ausente, diferido).
