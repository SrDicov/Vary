#!/usr/bin/env jq
# scripts/vurinfo.jq
#
# Convierte la salida de `./xbps-src show <pkg>` (líneas clave=valor con las
# variables ya evaluadas por xbps-src dentro del masterdir) al índice
# declarativo .VURINFO en JSON (esquema v1 — ver docs/VURINFO.md).
#
# USO OBLIGATORIO EN MODO SLURP + RAW (-n -R): el programa agrega TODA la
# entrada (una línea clave=valor por registro) en un único objeto JSON:
#
#   ./xbps-src show foo | jq -S -n -R -f scripts/vurinfo.jq > srcpkgs/foo/.VURINFO
#
# Campos ausentes se emiten como [] / false / null según el esquema v1.

def lista: split(" ") | map(select(length > 0));

# xbps-src suele volcar los valores entrecomillados: quitar comillas exteriores.
def sin_comillas: sub("^\""; "") | sub("\"$"; "");

[inputs
 | select(test("^[A-Za-z_][A-Za-z0-9_]*="))
 | capture("^(?<clave>[A-Za-z_][A-Za-z0-9_]*)=(?<valor>.*)$")
 | { (.clave): (.valor | sin_comillas) }
]
| (add // {}) as $v
| {
    format_version: 1,
    pkgname: ($v.pkgname // null),
    version: ($v.version // null),
    revision: (try (($v.revision // "1") | tonumber) catch 1),
    archs: (if $v.archs == null then ["x86_64"] else ($v.archs | lista) end),
    subpackages: [],
    depends: (($v.depends // "") | lista),
    hostmakedepends: (($v.hostmakedepends // "") | lista),
    makedepends: (($v.makedepends // "") | lista),
    checkdepends: (($v.checkdepends // "") | lista),
    build_style: ($v.build_style // null),
    distfiles: (($v.distfiles // "") | lista),
    checksum: (($v.checksum // "")
               | lista
               # Normaliza al formato del esquema: prefijo sha256: salvo SKIP.
               | map(if . == "SKIP" or startswith("sha256:") then . else "sha256:\(.)" end)),
    provides: (($v.provides // "") | lista),
    replaces: (($v.replaces // "") | lista),
    restricted: (($v.restricted // "no") | test("^(yes|true|1)$"; "i")),
    maintainer: ($v.maintainer // null)
}
