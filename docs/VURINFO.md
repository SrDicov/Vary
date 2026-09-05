# Especificación .VURINFO (esquema v1)

`.VURINFO` es el índice declarativo (JSON) que cada repo VUR publica para describir sus paquetes: metadatos, dependencias y opciones de build, ya evaluados. vary lo consume en el cliente para resolver el DAG de dependencias y decidir si instalar un binario firmado o compilar la plantilla con `xbps-src`, sin evaluar código bash del repositorio. El archivo se genera con CI a partir de las plantillas (ver más abajo).

## Ubicaciones admitidas

Un repo VUR puede publicar el índice de dos formas complementarias:

- **Uno por plantilla:** `srcpkgs/<pkg>/.VURINFO` (o `pkgs/<pkg>/.VURINFO` en repos que usen ese alias, p. ej. voiders) — es lo que genera `scripts/vur-generator.sh` (asume `srcpkgs/`).
- **Índice raíz:** un único `.VURINFO` en la raíz del repo, que puede ser un objeto (un paquete) o un array de objetos (varios paquetes).

Al cargar, vary fusiona ambas fuentes; los duplicados por `pkgname` son un error.

## Tabla de campos (esquema v1)

| Campo | Tipo | Requerido | Descripción |
| --- | --- | --- | --- |
| `format_version` | entero | sí | Versión del esquema. Debe ser exactamente `1`. |
| `pkgname` | string | sí | Nombre del paquete. Debe coincidir con el directorio de la plantilla. |
| `version` | string | sí | Versión upstream. Regex: `^[A-Za-z0-9._+]+$` (sin guiones). |
| `revision` | entero | sí | Revisión XBPS. Debe ser `>= 1`. |
| `archs` | array\<string\> | sí | Arquitecturas soportadas (p. ej. `x86_64`, `aarch64`). No puede estar vacío. |
| `subpackages` | array\<objeto\> | no | Subpaquetes derivados. Cada uno: `{ pkgname, depends, short_desc }`. Heredan `archs` del padre. |
| `depends` | array\<string\> | no | Dependencias runtime con restricciones de versión (`libbar>=2.0`). |
| `hostmakedepends` | array\<string\> | no | Herramientas del host necesarias para compilar. |
| `makedepends` | array\<string\> | no | Dependencias de compilación. |
| `checkdepends` | array\<string\> | no | Dependencias de la fase `check`. |
| `build_style` | string \| null | no | Estilo de build de `xbps-src` (`gnu-makefile`, `cmake`, …). |
| `distfiles` | array\<string\> | no | URLs de las fuentes. |
| `checksum` | array\<string\> | no | Digestos correspondientes a `distfiles`, cada uno con prefijo `sha256:` (o `SKIP`). |
| `provides` | array\<string\> | no | Capacidades virtuales que ofrece el paquete. |
| `replaces` | array\<string\> | no | Paquetes que este reemplaza. |
| `restricted` | boolean | no | Si el paquete está restringido (default `false`). |
| `maintainer` | string \| null | no | Mantenedor, formato `nombre <email>`. |

## Ejemplo canónico

```json
{
  "format_version": 1,
  "pkgname": "foo",
  "version": "1.2.3",
  "revision": 2,
  "archs": ["x86_64", "aarch64"],
  "subpackages": [
    { "pkgname": "foo-devel", "depends": ["foo>=1.2.3"], "short_desc": "Development files for foo" }
  ],
  "depends": ["libbar>=2.0"],
  "hostmakedepends": ["pkg-config", "ninja"],
  "makedepends": ["libbar-devel"],
  "build_style": "cmake",
  "distfiles": ["https://example.com/foo-1.2.3.tar.gz"],
  "checksum": ["sha256:abc123..."],
  "provides": ["libfoo.so.1"],
  "replaces": ["old-foo"],
  "restricted": false,
  "maintainer": "name <email>"
}
```

## Reglas de validación al cargar

vary rechaza el índice (con error claro al usuario) si:

- `format_version != 1`: solo se acepta el esquema v1.
- `version` no coincide con la regex `^[A-Za-z0-9._+]+$`: sin guiones.
- `revision < 1`.
- `archs` está vacío o ausente.
- alguna entrada de `checksum` carece del prefijo `sha256:` (se admite `SKIP`).
- algún subpaquete no hereda implícitamente los `archs` del padre (los subpaquetes nunca definen `archs` propios en v1).
- algún `subpackages[].pkgname` coincide con el `pkgname` del padre.

Los campos opcionales ausentes toman sus defaults: arrays → `[]`, `restricted` → `false`, strings → `null`.

## Manejo multi-arquitectura

- `archs` filtra los candidatos: vary cruza las arquitecturas declaradas con la arquitectura actual del sistema (incluyendo variantes musl/glibc) y descarta paquetes sin coincidencia.
- Los repos binarios son per-arquitectura: los binarios publicados residen en `hostdir/binpkgs/<arch>` y la firma/índice (`xbps-rindex`) se verifica por arquitectura.

## Generación con CI

El índice debe regenerarse en cada push del repo VUR, idealmente desde CI sobre Void Linux:

1. Clonar `void-packages` y tu repo VUR dentro de él (los `srcpkgs/` del VUR deben convivir con los oficiales).
2. Arrancar el entorno una vez: `./xbps-src binary-bootstrap` (crea el `masterdir`).
3. Para cada plantilla, obtener las variables ya evaluadas **dentro del masterdir** (nunca hacer `source template` en el host):

   ```sh
   ./xbps-src show <pkg>
   ```

4. Convertir esa salida (`clave=valor`) a JSON con jq:

   ```sh
   ./xbps-src show foo | jq -S -n -R -f scripts/vurinfo.jq > srcpkgs/foo/.VURINFO
   ```

5. O automatizar el bucle completo sobre todos los `srcpkgs/*/`:

   ```sh
   scripts/vur-generator.sh
   ```

6. Commit y push del `.VURINFO`; los clientes lo reciben con `vary -Syu`.

## NOTA SOBRE VARIABLES DINÁMICAS

> NOTA SOBRE VARIABLES DINÁMICAS: El .VURINFO generado por scripts/vur-generator.sh refleja las opciones de compilación POR DEFECTO (sin XBPS_PKG_OPTIONS activos). Si el usuario final compila con opciones personalizadas (XBPS_PKG_OPTIONS_<pkg>), la resolución de dependencias inicial puede ser incompleta. xbps-src manejará las dependencias adicionales durante la compilación real. Esto es aceptable para el MVP: la resolución del DAG es una optimización para minimizar builds innecesarios, no una garantía de completitud.
