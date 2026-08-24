#!/usr/bin/env bash
# ============================================================================
# scripts/vur-generator.sh
#
# GENERADOR REFERENCIA DE ÍNDICES .VURINFO (esquema v1, ver docs/VURINFO.md)
#
# ESTADO: REFERENCIA — NO VALIDADO. Este script requiere validación en un
# Void Linux real antes de usarse en producción o CI.
#
# Qué hace:
#   Recorre las plantillas de un checkout de void-packages (srcpkgs/*/template),
#   pide a ./xbps-src las variables YA EVALUADAS de cada paquete y las
#   convierte a .VURINFO (JSON) con jq + scripts/vurinfo.jq.
#
# Seguridad:
#   Las plantillas NUNCA se interpretan con bash del host: aquí no se hace
#   `source template`. Todo el código de plantilla se evalúa dentro del
#   masterdir controlado por xbps-src; este script solo captura su salida
#   (líneas clave=valor) y la filtra/transforma con jq.
#
# Requisitos:
#   - Ejecutar desde la raíz de un checkout de void-packages.
#   - Masterdir booteado (`./xbps-src binary-bootstrap`).
#   - jq instalado en el host.
#
# Uso:
#   scripts/vur-generator.sh              # regenera todos los srcpkgs/*/
#   scripts/vur-generator.sh foo bar      # solo los paquetes indicados
#
# Salida:
#   Un srcpkgs/<pkg>/.VURINFO por plantilla procesada con éxito.
# ============================================================================

set -euo pipefail

die() { printf 'vur-generator: error: %s\n' "$*" >&2; exit 1; }

# --- Comprobaciones de entorno ---------------------------------------------
command -v jq >/dev/null 2>&1 || die "jq no está instalado"
[[ -x ./xbps-src ]] || die "ejecútalo desde la raíz de un checkout de void-packages (no se encontró ./xbps-src)"
[[ -d masterdir ]] || die "masterdir ausente: arranca el entorno con './xbps-src binary-bootstrap'"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
JQ_PROG="${SCRIPT_DIR}/vurinfo.jq"
[[ -f "$JQ_PROG" ]] || die "no se encontró ${JQ_PROG}"

# --- Conversión de una plantilla a .VURINFO ---------------------------------
generate_one() {
    local pkg="$1" vars out tmp
    printf '==> %s\n' "$pkg" >&2
    out="srcpkgs/${pkg}/.VURINFO"
    tmp="${out}.tmp"

    # `xbps-src show` evalúa la plantilla DENTRO del masterdir y vuelca las
    # variables resultantes como líneas clave=valor. Nunca evaluamos esa
    # salida en el host: solo la convertimos con jq (modo slurp + raw).
    if ! vars="$(./xbps-src show "$pkg" 2>/dev/null)"; then
        printf 'vur-generator: aviso: fallo al mostrar %s; se omite\n' "$pkg" >&2
        return 1
    fi

    if ! printf '%s\n' "$vars" | jq -S -n -R -f "$JQ_PROG" >"$tmp"; then
        printf 'vur-generator: aviso: no se pudo convertir %s a JSON; se omite\n' "$pkg" >&2
        rm -f "$tmp"
        return 1
    fi

    # Sanidad: debe ser JSON válido del esquema v1.
    jq -e '.format_version == 1 and (.pkgname | type == "string")' "$tmp" >/dev/null \
        || die "salida inválida para ${pkg} (revisa vurinfo.jq)"

    mv -f "$tmp" "$out"
}

# --- Bucle principal ---------------------------------------------------------
failed=0
if (($#)); then
    for pkg in "$@"; do
        [[ -f "srcpkgs/${pkg}/template" ]] || die "plantilla inexistente: srcpkgs/${pkg}/template"
        generate_one "$pkg" || failed=1
    done
else
    for tmpl in srcpkgs/*/template; do
        pkg="$(basename "$(dirname "$tmpl")")"
        generate_one "$pkg" || failed=1
    done
fi

exit "$failed"
