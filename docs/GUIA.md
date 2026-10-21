# GUÍA DE USO DE VARY — de cero a compiling

> Para alguien que **nunca** ha usado `paru` ni `vary` y no conoce el mundo de
> Void Linux. Todo lo que hay aquí está verificado contra el código y contra un
> Void real (contenedor); las salidas de ejemplo son **reales**.
>
> Si vienes de Arch y conoces `pacman`/`paru`, las equivalencias están en
> [§16](#16-equivalencias-con-pacmanparu). Si vienes de Void y conoces `xbps`,
> salta a [§17](#17-equivalencias-con-xbps).

---

## Índice

1. [Qué es vary (y qué no es)](#1-qué-es-vary-y-qué-no-es)
2. [Antes de tocar nada: requisitos](#2-antes-de-tocar-nada-requisitos)
3. [Instalar vary](#3-instalar-vary)
4. [`vary --help`, línea a línea](#4-vary---help-línea-a-línea)
5. [Dos estilos de invocación: flags y palabras](#5-dos-estilos-de-invocación-flags-y-palabras)
6. [Catálogo de operaciones](#6-catálogo-de-operaciones)
   - 6.1 [Instalar](#61-instalar--s-install)
   - 6.2 [Actualizar todo](#62-actualizar-todo--syu-update)
   - 6.3 [Buscar](#63-buscar--ss-search)
   - 6.4 [Ver la ficha de un paquete](#64-ver-la-ficha-de-un-paquete--si-info)
   - 6.5 [Listar lo instalado](#65-listar-lo-instalado--q-list)
   - 6.6 [Desinstalar](#66-desinstalar--r-remove)
   - 6.7 [Ver el plan sin tocar nada](#67-ver-el-plan-sin-tocar-nada--p-print)
   - 6.8 [Solo descargar fuentes](#68-solo-descargar-fuentes--w-downloadonly)
7. [Repos VUR: instalar software de terceros](#7-repos-vur-instalar-software-de-terceros)
8. [Configuración: `vary.conf`](#8-configuración-varyconf)
9. [Los sets (`@...`)](#9-los-sets-)
10. [Opciones de compilación](#10-opciones-de-compilación)
11. [Forzar compilaciones y el modo híbrido](#11-forzar-compilaciones-y-el-modo-híbrido)
12. [Auditoría de recompilaciones](#12-auditoría-de-recompilaciones)
13. [Seguridad: llaves, confianza y anclaje](#13-seguridad-llaves-confianza-y-anclaje)
14. [Reproducibilidad: `vary.lock`](#14-reproducibilidad-varylock)
15. [Diagnóstico: `--why`, `--log`, `--conf-update`, verbosidad](#15-diagnóstico---why---log---conf-update-verbosidad)
16. [Equivalencias con pacman/paru](#16-equivalencias-con-pacmanparu)
17. [Equivalencias con xbps](#17-equivalencias-con-xbps)
18. [Referencia completa de flags](#18-referencia-completa-de-flags)
19. [Ficheros y directorios](#19-ficheros-y-directorios)
20. [Recetas](#20-recetas)
21. [Errores frecuentes y qué hacer](#21-errores-frecuentes-y-qué-hacer)
22. [Glosario](#22-glosario)

---

## 1. Qué es vary (y qué no es)

**Void Linux** no tiene "repositorios de la comunidad": el repositorio oficial
solo publica binarios. Si quieres una aplicación que no está en el oficial
—una versión más nueva, un parche, un programa que no existe en Void— hay que
**compilarla**. Para eso existe `xbps-src`, que es al gestor de paquetes de Void
lo que `makepkg` es a Arch: compila una receta (una *template*) y produce un
paquete.

**vary** es un asistente de compilación: el "`paru` de Void", pero con una idea
central distinta:

> Los paquetes de terceros se publican en **repositorios git** (llamados *VUR*,
> *Void User Repositories*), no en un único repositorio central. vary los
> consulta, **resuelve el grafo de dependencias**, y compila lo que haga falta
  con `xbps-src`.

En la práctica, `vary` hace cuatro cosas:

1. **Habla** con `xbps` (instalar/desinstalar binarios oficiales).
2. **Lee** los índices `.VURINFO` de los repos VUR que tú configures.
3. **Resuelve** qué hay que compilar y en qué orden (un DAG con niveles).
4. **Compila** con `xbps-src` y **ancla** (`repolock`) lo que compila, para que
   una actualización oficial no te pise el binario que construiste.

### Lo que vary NO es

- **No es un gestor de paquetes**: no reemplaza a `xbps-install`. Instalar
  paquetes oficiales se sigue haciendo con `xbps` (o con `vary -S`, que llama a
  `xbps-install` por ti).
- **No descarga binarios de terceros**: los repos VUR publican *plantillas*;
  vary compila. (Hay una excepción: los repos estilo *VUP*, que publican
  binarios firmados — ver [§7.6](#76-repos-estilo-vup-binarios-firmados)).
- **No es root**: **jamás** debes ejecutarlo como root. Ver
  [§2.3](#23-la-regla-de-oro-no-seas-root).
- **No es una TUI**: todo es línea de comandos, texto plano, salida apta para
  scripts y para `grep`.

---

## 2. Antes de tocar nada: requisitos

### 2.1 Qué necesitas instalado

| Herramienta | Para qué | Cómo se instala |
|---|---|---|
| `xbps` | el gestor base de Void | ya está en Void |
| `git` | clonar los repos VUR y `void-packages` | `sudo xbps-install git` |
| `curl` | descargar índices remotos estilo VUP | `sudo xbps-install curl` |
| `sudo` (o `doas`/`run0`) | para que vary eleve solo lo que necesita | `sudo xbps-install sudo` |

`vary` comprueba `git` al arrancar y **falla con un mensaje claro** si falta.

### 2.2 Espacio y tiempo

- **Espacio**: el clon de `void-packages` (el árbol maestro de recetas) ocupa
  **~1,5–2 GiB** en `~/.cache/vary/void-packages/`. Las compilaciones van a
  `~/.cache/vary/` (masterdir) y necesitan **1 GiB libres** como mínimo; la
  realidad son varios GiB si compilas cosas grandes.
- **Tiempo**: clonar `void-packages` tarda unos minutos la primera vez. Una
  compilación normal va de segundos a horas según el paquete.

### 2.3 La regla de oro: NO seas root

```
error: vary no debe ejecutarse como root directo (euid 0): usa tu usuario
normal (la elevación puntual la gestiona vary vía --sudo/doas/run0);
xbps-src se niega a correr como root
```

Esto **no es un capricho**: `xbps-src` se niega a correr como root por
seguridad (compilar código como root es una.recipe para meter cosas en `/`),
y el `masterdir` que usa vary está pensado para un usuario normal.

**Lo correcto:**

```sh
vary -S firefox          # tú como usuario normal
```

…y cuando vary necesita escribir en `/etc` o instalar binarios, **el propio
vary pide elevación** (`sudo`, `doas` o `run0`, autodetectada) solo para esa
operación concreta. Es lo mismo que hace `paru` con `sudo pacman -S`.

**Si necesitas root de verdad** (porque estás en un contenedor, por ejemplo),
usa el patrón de `paru`: un wrapper que baja privilegios al usuario real y
conserva la elevación:

```sh
sudo vary -S firefox     # sudo baja a SUDO_UID y vary re-eleva lo justo
```

### 2.4 La primera vez: el "bootstrap"

La primera vez que instalas algo **desde fuente**, vary necesita:

1. clonar `void-packages` en `~/.cache/vary/void-packages/`,
2. preparar el *masterdir* (un entorno de compilación limpio, como un chroot),
3. registrar su repositorio local en `/etc/xbps.d/10-vary.conf`.

Eso lo hace **automáticamente** en los flujos de instalación (`vary -S <pkg>`),
sin pasos manuales. Un `vary -Syu` que no tenga nada que compilar **no** hace
nada de esto (no necesita el masterdir para nada).

---

## 3. Instalar vary

### 3.1 Desde el repositorio rodante (recomendado)

El proyecto publica un repositorio XBPS propio, firmado:

```sh
echo 'repository=https://github.com/SrDicov/Vary/releases/download/repo' \
  | sudo tee /etc/xbps.d/20-vary.conf
sudo xbps-install -S        # acepta la huella RSA cuando la pregunte
sudo xbps-install vary      # instala; luego: sudo xbps-install -Su lo actualiza
```

Sirve binarios para **glibc** (`x86_64`) y **musl** (`x86_64-musl`) desde la
misma URL. La clave pública está en
[`keys/vary-repo.pub.pem`](https://github.com/SrDicov/Vary/blob/main/keys/vary-repo.pub.pem).

### 3.2 Compilándolo tú mismo

```sh
git clone https://github.com/SrDicov/Vary
cd Vary
cargo build --release          # el binario está en target/release/vary
sudo install -Dm755 target/release/vary /usr/sbin/vary
```

Requiere Rust ≥ 1.88 (`rust-version` en `Cargo.toml`).

### 3.3 Comprobar que quedó

```sh
vary --version
# vary 0.4.1
```

---

## 4. `vary --help`, línea a línea

```
Usage:
    vary
    vary <operation> [...]
    vary <package(s)>
```

- `vary` **sin argumentos** equivale a `vary -Syu` (actualización completa).
- `vary <operación> [...]` es la forma con flags (estilo pacman).
- `vary <paquete(s)>`… es la forma **con palabras** (estilo kyoz), que se
  explica en [§5](#5-dos-estilos-de-invocación-flags-y-palabras).

### Operaciones

| Flag largo | Qué hace |
|---|---|
| `-h`, `--help` | Muestra la ayuda y sale. |
| `-V`, `--version` | Muestra la versión y sale. |
| `-S`, `--sync` | **Operación principal**: instala paquetes (`-S foo`), y con `-y`/`-u` actualiza. |
| `-R`, `--remove` | Desinstala paquetes. |
| `--repo <cmd>` | Gestión de repositorios VUR ([§7](#7-repos-vur-instalar-software-de-terceros)). |

### Opciones de `-S` (sync)

| Flag | Qué hace |
|---|---|
| `-s`, `--search` | Busca paquetes (no instala). Ej.: `vary -Ss firefox`. |
| `-i`, `--info` | Muestra la ficha de un paquete. Ej.: `vary -Si bash`. |
| `-y`, `--refresh` | Refresca (hace `git pull`) los repos VUR. |
| `-u`, `--sysupgrade` | Actualiza **todo** el sistema. |
| `-w`, `--downloadonly` | Solo descarga las fuentes; no compila ni instala. |
| `-p`, `--print` | Muestra el **plan** y no cambia nada. |

### Opciones generales

| Flag | Qué hace |
|---|---|
| `--noconfirm` | No pregunta confirmaciones. (`--yes` es alias.) |
| `--confirm` | Pregunta (es el defecto). |
| `--color <always\|never\|auto>` | Color de la salida. |
| `-v`, `--verbose` | Más detalle. `-vv` = debug. |
| `-q`, `--quiet` | Salida mínima. |
| `--arch <arch>` | Fuerza la arquitectura (p. ej. `aarch64`). |
| `--lock` | (Re)genera `~/.config/vary/vary.lock` desde el estado actual. |
| `--challenge <pkg>` | Recompila un paquete instalado desde fuente y **compara** el resultado con lo instalado. Requiere `--experimental` y pide confirmación. |
| `--why <pkg>` | Explica **por qué** está instalado un paquete. |
| `--log [pkg]` | Muestra el historial de operaciones. |
| `--conf-update` | Resuelve los ficheros `.new-*` que `xbps` deja en `/etc`. |
| `-Q` | Lista los paquetes instalados (compatible con `pacman -Q`). |

### Opciones propias de vary

| Flag | Qué hace |
|---|---|
| `--force-build` | Compila desde fuente aunque exista un binario. |
| `--deep-build` (`--deep`) | Pasa `-N` a `xbps-src`: **sin** repos remotos; las dependencias se compilan también. Puede fallar si falta una fuente. |
| `--prefer-binary` | Prefiere el binario si existe (es el defecto). |
| `--no-prefer-binary` | Compila siempre desde fuente. |
| `--newoptions` | Recompila los paquetes compilados cuyas **opciones**, parches de usuario o plantilla cambiaron. |
| `--changed-flags` (`-U`) | Recompila los paquetes compilados con **otras banderas** de compilador. |
| `--sudo <bin>` | Herramienta de elevación: `sudo`, `doas` o `run0`. |
| `--sudoflags <flags>` | Sustituye los flags del wrapper de elevación. |
| `--git <bin>` | Binario `git` a usar. |
| `--curl <bin>` | Binario `curl` para índices remotos. |

---

## 5. Dos estilos de invocación: flags y palabras

Vary acepta **las dos formas**. Usa la que tesea cómoda:

```sh
vary -S firefox        # forma pacman
vary install firefox   # forma kyoz (palabras)
```

| Palabra | Alias | Equivale a |
|---|---|---|
| `install <pkg...>` | — | `vary -S <pkg...>` |
| `update` | — | `vary -Syu` |
| `remove <pkg...>` | `r` | `vary -R <pkg...>` |
| `search <texto>` | `s` | `vary -Ss <texto>` |
| `info <pkg...>` | `i` | `vary -Si <pkg...>` |
| `list [patrón]` | `ls` | `vary -Q [patrón]` |

**Dos reglas para no equivocarte:**

1. Los alias de una letra coinciden con su flag: `u` = update (como `-u`),
   `r` = remove (como `-R`), `s` = search (como `-s`), `i` = info (como `-i`).
   Ojo: `i` es **info**, no install (a diferencia de kyoz).
2. **Con un flag explícito, la primera palabra es un nombre de paquete.**
   `vary -S s` instala el paquete `s`; `vary s` busca. Es decir: las palabras
   solo se interpretan como subcomando si **no** has puesto ninguna operación.

---

## 6. Catálogo de operaciones

### 6.1 Instalar (`-S`, `install`)

```sh
vary -S firefox
vary install firefox
```

Qué pasa, por dentro:

1. Resuelve `firefox`: mira el repo oficial (¿existe binario?) y los VUR que
   tengas configurados.
2. Si hay un binario oficialSigned y con la arquitectura correcta → lo instala
   con `xbps-install` (rápido).
3. Si no → compila desde fuente con `xbps-src`, **incluyendo** las
   dependencias que no tengan binario.
4. Registra en `installed.json` lo que gestiona (para saber qué tienes que
   recompilar) y **ancla** (`repolock`) lo compilado, para que un `-Su` no te
   lo pise con un binario oficial.

`vary -S` **sin** paquete es un error (no es "actualizar"): te faltan
objetivos.

```sh
$ vary -S
error: no targets specified (use -h for help)
```

Instalar **varios** paquetes de golpe:

```sh
vary -S firefox libreoffice inkscape
```

Ver **qué haría** sin hacerlo (imprescindible la primera vez):

```sh
vary -Sp firefox
```

Salida real:

```
:: PRINT-ONLY plan for: bash (no changes will be made)
installs [single binary transaction]:
  official/bash
builds [topological levels]:
  (none)
elevations [estimated total: 1]:
  repo-setup: 0
  install-transaction: 1
```

Lectura del plan:

- **`installs`** → paquetes que se instalan como binario, en una sola
  transacción de `xbps-install`.
- **`builds [topological levels]`** → paquetes que se **compilan**, agrupados en
  niveles: el nivel 0 no depende de nada del plan, el nivel 1 depende del 0, etc.
  Dentro de un nivel no hay dependencias entre sí → se pueden compilar en
  paralelo.
- **`elevations`** → cuántas veces hay que pedir `sudo` (y por qué).

### 6.2 Actualizar todo (`-Syu`, `update`)

```sh
vary -Syu
vary update
```

Qué hace, en orden:

1. **Fase oficial**: `xbps-install -u` (los binarios de Void se actualizan
   solos).
2. **Anclaje previo**: todo lo que varia compiló y está instalado se marca con
   `repolock` para que la fase oficial no lo pise.
3. **Refresco de VURs**: `git pull` de cada repositorio VUR configurado.
4. **Detección de actualizaciones**: compara el índice VUR con
   `installed.json` → lista de paquetes con versión nueva.
5. **Revisión de plantillas** (si el template cambió, te enseña el diff y
   pregunta).
6. **Compilación e instalación** de lo que toque, en niveles topológicos.

Si un repositorio VUR **no se pudo refrescar** (sin red, caído), vary **te avisa
por consola** y el mensaje final distingue el caso: no afirma "up to date" si
hay índices viejos en juego.

### 6.3 Buscar (`-Ss`, `search`)

```sh
vary -Ss firefox
vary search firefox
```

Salida real (columnas: `repo`, `nombre`, `versión`, `descripción`):

```
official ansi ansi-3.0.1_1  ANSI escape codes in pure Bash
official bash bash-5.3_2  GNU Bourne Again Shell
official bash-completion bash-completion-2.16_0  Programmable completion for the GNU Bash shell
```

El prefijo `official` significa "viene del repositorio oficial de Void". Si
tienes VURs configurados, sus resultados aparecen con el nombre del repo
(`mi-repo firefox firefox-1.0_1  Descripción`).

Buscar en **un repo concreto** no está soportado como flag; usa el prefijo de
la columna para filtrar (o `grep`).

### 6.4 Ver la ficha de un paquete (`-Si`, `info`)

```sh
vary -Si bash
vary info bash
```

Salida real:

```
Repository      : official
Package         : bash
Info            : bash-5.3_2
GNU Bourne Again Shell
https://www.gnu.org/software/bash/bash.html
Enno Boland <gottox@voidlinux.org>
```

`Info` es el **pkgver** completo (`nombre-versión_revisión`).

### 6.5 Listar lo instalado (`-Q`, `list`)

```sh
vary -Q
vary list
```

Salida real (una línea por paquete: `nombre` + `versión`):

```
acl 2.3.1_1
attr 2.5.1_1
base-container 0.3_3
...
```

Con filtro por subcadena del nombre:

```sh
vary -Q bash
```

> Solo cuentan los paquetes **realmente instalados** (estado `ii` en xbps). Un
> paquete a medias desinstalado (`hr`) o solo desempaquetado (`uu`) **no**
> aparece: no está instalado.

### 6.6 Desinstalar (`-R`, `remove`)

```sh
vary -R firefox
vary remove firefox
vary -R firefox thunderbird
```

Desinstala con `xbps-remove` y actualiza `installed.json` (lo que varia gestiona
deja de estar anclado).

### 6.7 Ver el plan sin tocar nada (`-p`, `--print`)

```sh
vary -Sp firefox          # con -p: plan
vary --print -S firefox   # equivalente
```

Se puede combinar con cualquier operación de sync. **No cambia nada** (ni
instala, ni compila, ni clona). Úsalo siempre antes de un `-S` que te Stere.

### 6.8 Solo descargar fuentes (`-w`, `--downloadonly`)

```sh
vary -Sw firefox
```

Descarga los *distfiles* al masterdir sin compilar. Útil para dejar la
descarga hecha antes de una noche de compilaciones.

---

## 7. Repos VUR: instalar software de terceros

### 7.1 Qué es un VUR

Un **VUR** (Void User Repository) es un repositorio git con plantillas de
paquetes, al estilo de los AUR de Arch. Puede publicar:

- **`.VURINFO`** (el índice declarativo que vary lee) — opcional pero
  recomendado.
- **plantillas** (`template`) — el *qué se compila*.

Si un VUR no publica `.VURINFO`, vary parsea las plantillas como plan B (funciona
para plantillas simples; se pierde información de variables).

### 7.2 Añadir un repo

```sh
vary --repo add https://github.com/usuario/mi-repo
```

Con nombre explícito, rama o índice binario:

```sh
vary --repo add https://github.com/usuario/mi-repo mi-repo
vary --repo add https://github.com/usuario/mi-repo --branch desarrollo
vary --repo add https://github.com/usuario/mi-repo --index-url https://.../index.json
```

- El **nombre** por defecto se deriva del último segmento de la URL.
- La **rama** se autodetecta si no la indicas.
- Al añadir un repo que publica **binarios firmados**, vary registra su llave
  RSA en el sistema y te pide **verificar la huella** la primera vez (TOFU:
  "Trust On First Use"). El repo queda anotado en `~/.config/vary/repos.conf`.

### 7.3 Ver tus repos

```sh
vary --repo list
```

Salida real (sin repos):

```
No VURs configured. Add one with: vary --repo add <url>
```

Con repos, una línea por repo con su prioridad y si publica binarios.

### 7.4 Quitar un repo

```sh
vary --repo remove mi-repo
vary --repo remove mi-repo -p     # -p además borra el clon local
```

### 7.5 Rotar la llave de un repo

Si el mantenedor de un repo **rota su llave RSA legítimamente**, los paquetes
binarios de ese repo dejan de verificarse. Para aceptar la nueva:

```sh
vary --repo rekey mi-repo      # re-registra la llave (pide TOFU otra vez)
vary --repo re-trust mi-repo   # re-confirma la llave actual sin cambiarla
```

### 7.6 Repos estilo VUP (binarios firmados)

Un repo estilo **VUP** publica binarios ya compilados y firmados, con un
`index.json` central, en vez de plantillas. Se registra con `--index-url`:

```sh
vary --repo add https://github.com/VUP-Linux/vup --index-url https://.../index.json
```

Limitaciones de este modo (fase 1): **no** se compila desde fuente, no hay
`-Si` detallado, y no hay actualización automática.

### 7.7 Escribir tu propio VUR

Un repo VUR mínimo necesita, por paquete:

```
mi-repo/
└── srcpkgs/
    └── mipaquete/
        ├── template      # la receta (sintaxis xbps-src)
        └── .VURINFO      # el índice declarativo (opcional pero recomendado)
```

El `.VURINFO` es un JSON con este esquema (v1):

```json
{
  "format_version": 1,
  "pkgname": "mipaquete",
  "version": "1.2.3",
  "revision": 1,
  "archs": ["x86_64", "aarch64"],
  "subpackages": [
    { "pkgname": "mipaquete-devel", "depends": ["mipaquete>=1.2.3"], "short_desc": "Headers" }
  ],
  "depends": ["glibc"],
  "hostmakedepends": ["pkg-config"],
  "makedepends": ["cmake"],
  "build_style": "cmake",
  "distfiles": ["https://example.com/mipaquete-1.2.3.tar.gz"],
  "checksum": ["sha256:abc123..."],
  "provides": ["libmipaquete.so.1"],
  "maintainer": "Tu Nombre <tu@correo>"
}
```

Reglas que vary **rechaza** (con error claro):

- `format_version` debe ser exactamente `1`.
- `version` sin guiones (`^[A-Za-z0-9._+]+$`).
- `revision >= 1`.
- `archs` no vacío.
- `checksum` con prefijo `sha256:` (o `SKIP`).
- Los subpaquetes **heredan** los `archs` del padre (no pueden definir los suyos).

El generador oficial está en `scripts/vur-generator.sh` (usa
`xbps-src show <pkg> | jq -f scripts/vurinfo.jq`); hay que ejecutarlo **dentro**
de un clon de `void-packages` con el masterdir preparado.

Detalle completo: [`docs/VURINFO.md`](./VURINFO.md).

---

## 8. Configuración: `vary.conf`

Ruta: `~/.config/vary/vary.conf` (ejemplo completo en
[`etc/vary.conf.example`](https://github.com/SrDicov/Vary/blob/main/etc/vary.conf.example)).
**Todas las claves son opcionales**; lo que no pongas vale el defecto. Si el
fichero está corrupto, vary **avisa y sigue** con los defectos.

### `[general]`

| Clave | Defecto | Qué hace |
|---|---|---|
| `cache_dir` | `~/.cache/vary` | Cachés, clon de void-packages, `installed.json`, logs de compilación. |
| `data_dir` | `~/.local/share/vary` | Clones de los repos VUR y el diario de operaciones. |
| `log_level` | `info` | `error`, `warn`, `info` o `debug`. |
| `sudo_bin` | autodetección | `sudo`, `doas` o `run0`. Vacío = autodetectar en ese orden. |
| `sudo_flags` | `[]` | Flags extra del wrapper de elevación. |
| `curl_bin` | `curl` | Binario `curl` para los índices remotos VUP. |

### `[build]`

| Clave | Defecto | Qué hace |
|---|---|---|
| `makejobs` | nº de CPUs | `XBPS_MAKEJOBS`: cuántos trabajos de `make` en paralelo. |
| `max_concurrent_builds` | `2` | Compilaciones **en paralelo** (por niveles). **Solo con `--experimental`**, y requiere overlayfs + 1 GiB libre; sin capacidad, degrada a secuencial con aviso. |
| `force_rebuild` | `false` | Recompila siempre desde fuente, como `--force-build` en cada operación. |
| `source_always` | `[]` | Globs (`*`, `?`, sensible a mayúsculas) de paquetes **oficiales** que siempre se compilan desde fuente. Ej.: `["linux*", "mesa-*"]`. |

### `[make]`

Ajustes que se inyectan en un conf de `xbps-src` **por invocación**
(`etc/conf.vary-<id>` + `xbps-src -c`). OJO: ese conf **reemplaza** tu
`void-packages/etc/conf` en esa invocación; no se fusiona.

| Clave | Variable XBPS resultante |
|---|---|
| `cflags` | `XBPS_CFLAGS` |
| `cxxflags` | `XBPS_CXXFLAGS` |
| `ldflags` | `XBPS_LDFLAGS` |
| `ccache` | `XBPS_CCACHE` |
| `distcc` | `XBPS_DISTCC` |
| `distcc_hosts` | `XBPS_DISTCC_HOSTS` |
| `global_options` | `XBPS_PKG_OPTIONS` (global; separados por comas) |
| `debug_pkgs` | `XBPS_DEBUG_PKGS` |
| `check_pkgs` | `XBPS_CHECK_PKGS` |
| `allow_restricted` | `XBPS_ALLOW_RESTRICTED` |

> `RUSTFLAGS` **no** se propagan (xbps-src hace `unset RUSTFLAGS` y no existe
> `XBPS_RUSTFLAGS`). `makejobs` va en `[build]`, no aquí.

### `[options]` — opciones por paquete

Tabla libre: `paquete = ["opción", ...]`. Se convierten en
`XBPS_PKG_OPTIONS_<paquete>` (los caracteres fuera de `[A-Za-z0-9_]` del nombre
se sustituyen por `_`, como hace xbps-src):

```toml
[options]
firefox = ["pgo", "wayland"]
"mi.paquete" = ["docs"]
```

### `[search]`

| Clave | Qué hace |
|---|---|
| `ttl_cache_seconds` | Tiempo que se considera válida la caché de búsquedas (por defecto **3600 s**). |

### `[tools]`

| Clave | Qué hace |
|---|---|
| `install_bin` | Binario `install` (coreutils) usado para escribir en `/etc` con elevación. |

---

## 9. Los sets (`@...`)

Un *set* es un grupo de paquetes que puedes pedir en la línea de comandos:

```sh
vary -S @installed
vary -S @world
```

| Set | Qué contiene |
|---|---|
| `@selected` | Los paquetes que tú has pedido explícitamente. |
| `@installed` | Los que vary gestiona que están instalados. |
| `@system` | `base-system` (el grupo base de Void). |
| `@world` | `@installed` + `@system`. |
| `@optimized` | Los que están instalados y cuya versión es la última del repo. |
| `@vur` | Los que vienen de un repo VUR. |
| `@preserved-rebuild` | Los que se deben recompilar por cambios de opciones/parches/plantilla. |

Ejemplos:

```sh
vary -S @world -p          # qué haría una instalación completa
vary -R @optimized         # desinstalar lo que está anticuado
vary -S @preserved-rebuild  # recompilar lo que vary deems que cambió
```

> Los sets solo valen con operaciones de **instalación/desinstalación**
> (`-S`, `-Syu`, `-R`). Con `-Ss`/`-Si` vary **rechaza** el set con un error
> claro (expandir un set en una búsqueda no tiene sentido).

---

## 10. Opciones de compilación

Las *opciones* (`XBPS_PKG_OPTIONS`) activan ramas de la plantilla. Hay dos
lugares para ponerlas:

**1. Global** (afecta a todos los paquetes) — en `vary.conf`:

```toml
[make]
global_options = ["static", "docs"]
```

**2. Por paquete** — en `vary.conf`, sección `[options]`:

```toml
[options]
firefox = ["pgo", "wayland"]
```

**3. En la línea de comandos** — con la variable de entorno de xbps:

```sh
XBPS_PKG_OPTIONS_firefox=pgo,wayland vary -S firefox
```

> Ojo: en las opciones **por paquete** los valores van separados por **comas**
> (`pgo,wayland`), no por espacios: un espacio separa opciones distintas.

Ejemplo real de una plantilla que usa opciones:

```
build_options="pgo wayland lto"
build_options_default="wayland"
makedepends="... $(vopt_if pgo ninja) ..."
```

Con `pgo` activa se usa `ninja`; sin ella, la rama "off".

---

## 11. Forzar compilaciones y el modo híbrido

### Cuándo compilar en vez de instalar el binario

| Situación | Qué hacer |
|---|---|
| Quieres la **última versión** de un paquete que el repo oficial tiene más antigua | `vary -S --force-build pkg` |
| El paquete oficial **no** está para tu arquitectura | `vary -S pkg` (vary compila) |
| Quieres **siempre** fuente para una familia de paquetes | `[build] source_always = ["linux*"]` |
| El repo oficial tiene el binario pero quieres control total | `vary -S --no-prefer-binary pkg` |

### `--deep-build`

`--deep` pasa `-N` a `xbps-src`: **no** se usan repos remotos durante la
compilación, así que **todas** las dependencias se compilan también. Es lento
pero es el que te da un sistema "todo desde fuente". Puede fallar si falta
alguna fuente (distfile) de alguna dependencia.

### `--prefer-binary` / `--no-prefer-binary`

- `--prefer-binary` (defecto): si hay binario, se instala.
- `--no-prefer-binary`: se compila aunque exista binario.

---

## 12. Auditoría de recompilaciones

Vary guarda **con qué** compiló cada paquete (opciones, banderas, parches,
hash de plantilla). Con eso puede decirte "este paquete que compilaste hace un
mes ya no coincide con cómo lo compilarías hoy".

```sh
vary -S --newoptions pkg     # cambió: opción, parche de usuario o plantilla
vary -S --changed-flags pkg   # cambió: banderas del compilador (-U)
vary -S @preserved-rebuild     # lo mismo, para todo lo que lo necesite
```

Ambas flags se pueden combinar con `-p` para ver el plan antes de compilar.

### Parches propios

Vary aplica parches **de usuario** encima de la plantilla antes de compilar
(overlay reversible):

```
~/.config/vary/patches/<paquete>/    # parches propios (.patch)
~/.config/vary/files/<paquete>/       # ficheros extra que sustituyen a los del paquete
```

Esos parches cambian el hash del paquete → `--newoptions` lo detecta y
recompila.

---

## 13. Seguridad: llaves, confianza y anclaje

### Llaves de los repos binarios

Cuando un VUR publica **binarios**, vary:

1. descarga su llave RSA del repo,
2. calcula su **huella** (el mismo formato que muestra `xbps`),
3. te la enseña **la primera vez** (TOFU: confías en la primera sighting),
4. la importa al almacén de llaves de xbps y anota la huella en
   `~/.config/vary/repos.conf`.

En cada instalación, la firma del binario se **verifica** contra esa llave.

Si la huella **no coincide** con la anotada, vary **aborta**:

```
el repo VUR 'mi-repo' NO coincide:
  esperado (repos.conf): 60:ae:0c:d6:...
  recibido:               11:22:33:...
```

Si el mantenedor rotó la llave legítimamente → `vary --repo rekey mi-repo`.

### Anclaje (`repolock`)

Cuando vary compila un paquete, lo marca en la base de datos de xbps
(`repolock`) para que una actualización oficial **no lo pise**. Vary lo suelta
(`repounlock`) justo antes de que un binario tenga que instalarse encima.

Consecuencia: los paquetes que vary gestiona aparecen en `xbps-query` como
"foreign" y no se actualizan solos con `xbps-install -u` (eso lo hace vary con
`-Syu`, que sí los recompila).

---

## 14. Reproducibilidad: `vary.lock`

`vary --lock` genera `~/.config/vary/vary.lock`: una foto de **qué** tienes
instalado, **de qué repo** y **con qué commit**, para poder comparar más tarde.

```sh
vary --lock        # (re)genera el lock
vary -Syu          # avisa si el lock no cuadra con el estado actual
```

El aviso típico:

```
lock: 'foo' resuelve foo-1.0_1 pero vary.lock pinea foo-0.9_1 (regenera con `vary --lock`)
```

Útil para: saber si un repo cambió de commit (y por tanto sus binarios
firmados), y para congelar el estado de una máquina.

---

## 15. Diagnóstico: `--why`, `--log`, `--conf-update`, verbosidad

### `--why <pkg>`: ¿por qué está esto instalado?

```sh
vary --why bash
```

Salida real:

```
why bash:
  gestionado-por-vary: no (lo gestiona xbps)
  requerido-por-instalados: (ninguno)
  declarado-en-plantillas-VUR: (ninguno)
```

Te dice si lo gestiona vary, qué lo requiere, y si algún VUR lo declara.

### `--log [pkg]`: historial

```sh
vary --log           # todo
vary --log firefox   # solo ese paquete
```

Es el diario de operaciones (INSTALL/REMOVE) en
`~/.local/share/vary/operations.log`. Si no hay nada:

```
sin historial (aún no hay operaciones registradas)
```

### `--conf-update`: resolver los `.new-*`

Cuando cambias un fichero de `/etc` que xbps también gestiona, xbps deja
`/etc/foo.conf.new`. `vary --conf-update` te los lista y te pregunta qué hacer
con cada uno. Con `--noconfirm` (o sin TTY) **solo lista**.

### Verbosidad y logs

```sh
vary -v  ...         # más detalle
vary -vv ...         # debug
RUST_LOG=trace vary -S foo
```

El log a disco está en `~/.cache/vary/logs/`. El nivel por defecto en consola
muestra `warn` y arriba; el fichero de log guarda todo.

---

## 16. Equivalencias con pacman/paru

| Tarea | pacman/paru | vary |
|---|---|---|
| Instalar | `sudo pacman -S pkg` | `vary -S pkg` |
| Desinstalar | `sudo pacman -R pkg` | `vary -R pkg` |
| Actualizar todo | `sudo pacman -Syu` | `vary -Syu` |
| Buscar | `pacman -Ss q` | `vary -Ss q` |
| Info | `pacman -Si pkg` | `vary -Si pkg` |
| Listar instalados | `pacman -Q` | `vary -Q` |
| Ver el plan | `paru -Sp` | `vary -Sp` |
| Solo descargar | `paru -Sw` | `vary -Sw` |
| Añadir un repo de terceros | editar `/etc/pacman.conf` | `vary --repo add <url>` |
| Datos instalados | `~/.pacman` | `~/.cache/vary/installed.json` |
| Congelar el estado | — | `vary --lock` |

**Lo que NO translates**: `pacman -Rns` (no hay "-ns"), `-Qo`/`-Qii`. Y ojo
con los flags de motivo: `--asdeps`/`--asexplicit` **se rechazan con error**
(vary instala siempre como explícito), mientras que `--needed`, `--overwrite`,
`--ignore`, `--nodeps`… se aceptan pero **no hacen nada** (ver
[§18](#18-referencia-completa-de-flags)).

---

## 17. Equivalencias con xbps

| Tarea | xbps | vary |
|---|---|---|
| Instalar binario oficial | `sudo xbps-install pkg` | `vary -S pkg` |
| Desinstalar | `sudo xbps-remove pkg` | `vary -R pkg` |
| Actualizar binarios | `sudo xbps-install -u` | `vary -Syu` (además recompila lo tuyo) |
| Buscar en los repos registrados | `xbps-query -s q` | `vary -Ss q` |
| Buscar en un repo concreto | `xbps-query --repository=URL -s q` | (filtra por la columna `repo`) |
| Info de un paquete | `xbps-query -S pkg` | `vary -Si pkg` |
| Listar instalados | `xbps-query -l` | `vary -Q` |
| Listar los instalados a mano | `xbps-query -m` | `vary --log` (historial de vary) |
| Repos | `xbps-install --repository=URL` | `vary --repo add <url>` |

> **Cuidado con los flags que chocan**: en **xbps**, `-S` es *show* (info) y
> `-R` es *repository*; en **vary** (y en pacman) `-S` es *sync/instalar* y
> `-R` es *remove*. La tabla de arriba usa el significado de cada herramienta,
> no el del otro.

---

## 18. Referencia completa de flags

### Flags que **sí** tienen efecto

| Flag | Efecto |
|---|---|
| `-h`, `--help` | Ayuda. |
| `-V`, `--version` | Versión. |
| `-S`, `--sync` | Instalar / (con `-y`,`-u`) actualizar. |
| `-R`, `--remove` | Desinstalar. |
| `-s`, `--search` | Buscar. |
| `-i`, `--info` | Ficha del paquete. |
| `-y`, `--refresh` | Refrescar repos VUR. |
| `-u`, `--sysupgrade` | Actualizar todo. |
| `-w`, `--downloadonly` | Solo descargar. |
| `-p`, `--print` | Plan, sin cambios. |
| `-Q`, `--query` | Listar instalados (filtra por subcadena). |
| `-v`, `--verbose` | Más detalle (`-vv` = debug). |
| `-q`, `--quiet` | Salida mínima. |
| `--noconfirm` / `--yes` | Sin preguntar. |
| `--confirm` | Preguntar (defecto). |
| `--color <when>` | `always`/`never`/`auto`. |
| `--arch <arch>` | Forzar arquitectura. |
| `--lock` | Regenerar `vary.lock`. |
| `--challenge <pkg>` | Recompilar y comparar con lo instalado (requiere `--experimental`). |
| `--why <pkg>` | Por qué está instalado. |
| `--log [pkg]` | Historial. |
| `--conf-update` | Resolver `.new-*` de `/etc`. |
| `--force-build` | Compilar aunque exista binario. |
| `--deep-build`, `--deep` | `-N` a xbps-src (sin repos remotos). |
| `--prefer-binary` | Preferencia por binario (defecto). |
| `--no-prefer-binary` | Siempre fuente. |
| `--newoptions` | Recompilar por opciones/parches/plantilla. |
| `--changed-flags`, `-U` | Recompilar por banderas. |
| `--sudo <bin>`, `--sudoflags <f>` | Elevación. |
| `--git <bin>`, `--curl <bin>` | Binarios externos. |
| `--repo <cmd>` | Gestión de VURs (`add`, `list`, `remove`, `rekey`, `re-trust`). |
| `--experimental` | Puerta de validación de las funciones experimentales (builds en paralelo, `--challenge`). |

### Flags de pacman que se **aceptan pero no hacen nada**

Se parsesan (por compatibilidad) y se ignoran. **No changes el
comportamiento**: no reenvían nada a `xbps-install` y **no** son un error
(para no romper scripts que los usen):

`--nodeps`, `--assume-installed`, `--dbonly`, `--absdir`, `--noprogressbar`,
`--noscriptlet`, `--ignore`, `--ignoregroup`, `--needed`, `--overwrite`,
`-f`/`--force`, `-c`/`--changelog`, `--deps`, `-e`/`--explicit`, `-g`/`--groups`,
`-k`/`--check`, `-l`/`--list`, `-m`/`--foreign`, `-n`/`--native`,
`-o`/`--owns`, `--file`, `-t`/`--unrequired`, `--cascade`, `--nosave`,
`--recursive`, `--unneeded`, `--clean`, `--optional`, `-x`/`--regex`,
`--machinereadable`, `--disable-download-timeout`, `-d`, `--ask`, `--debug`.

### Flags que se **rechazan con error** (a propósito)

Mentir es peor que fallar: vary aborta con un mensaje claro en vez de tragarse
el flag y hacer algo distinto de lo que pediste.

| Flag | Por qué se rechaza |
|---|---|
| `--asdeps`, `--asdep`, `--asexplicit`, `--asexp` | vary instala **siempre** como explícito: no puede mentir sobre el motivo. |
| `--config`, `--cachedir`, `--dbpath`, `--root`, `--gpgdir`, `--hookdir`, `--logfile`, `--sysroot`, `-b`, `-r` | Son rutas de pacman: tragaban su valor y el token siguiente caía como **paquete** por error. vary usa `~/.config/vary/vary.conf` + XDG. |

### Palabras (subcomandos)

`install`, `update`, `remove`/`r`, `search`/`s`, `info`/`i`, `list`/`ls`.

---

## 19. Ficheros y directorios

| Ruta | Qué es | ¿Se borra sin miedo? |
|---|---|---|
| `~/.config/vary/vary.conf` | Tu configuración. | No (se pierde la config) |
| `~/.config/vary/repos.conf` | Repos VUR + huellas de llave + prioridades. | No (se pierde la confianza) |
| `~/.config/vary/vary.lock` | Foto reproducible del estado. | Sí (se regenera) |
| `~/.cache/vary/void-packages/` | Clon del árbol maestro de recetas. | Sí (se reclona) |
| `~/.cache/vary/installed.json` | Qué paquetes gestiona vary y con qué se compiló. | No (se pierde el historial de builds) |
| `~/.cache/vary/portdb.json` | Índice de las fuentes de void-packages. | Sí (se reindexa) |
| `~/.cache/vary/cache.json` | Caché de búsquedas. | Sí |
| `~/.cache/vary/logs/` | Logs de compilación (`xbps-src.log`, y `xbps-src-w<N>.log` por worker). | Sí |
| `~/.local/share/vary/vurs/` | Clones de tus repos VUR. | Sí (se reclonan) |
| `~/.local/share/vary/operations.log` | Historial de operaciones. | Sí (pierdes el historial) |
| `/etc/xbps.d/10-vary.conf` | Repo local de vary (lo escribe el bootstrap). | No (lo recrea el bootstrap) |
| `/etc/xbps.d/20-vur-<repo>.conf` | Repo binario de un VUR. | No (lo recrea `--repo add`) |

---

## 20. Recetas

### Instalar la última versión de un paquete oficial

```sh
vary -S --force-build firefox
```

### Compilar todo desde fuente (sin binarios)

```sh
vary -S --no-prefer-binary pkg
vary -S --deep-build pkg        # además compila las dependencias
```

### Compilar con más velocidad (experimental)

```sh
vary -S --experimental pkg
```
Necesitas overlayfs montado y 1 GiB libre; sin eso, degrada a secuencial con
aviso. Cuidado: `slots × makejobs` puede saturar la CPU.

### Ver qué se recompilaría con tus opciones nuevas

```sh
vary -Sp --newoptions @preserved-rebuild
```

### Recompilar un paquete con una opción activada

```sh
vary -S --newoptions --force-build firefox
```

### Actualizar todo incluyendo lo que compila

```sh
vary -Syu
```

### Comprobar si un paquete oficial "sano" está realmente sano

```sh
vary --challenge firefox        # recompila y compara el árbol de ficheros
```

### Buscar en un repo VUR concreto

```sh
vary -Ss firefox | grep mi-repo
```

### Ver el historial de lo que has instalado

```sh
vary --log
vary --log firefox
```

### Empezar de cero ( conservando la config)

```sh
rm -rf ~/.cache/vary ~/.local/share/vary
```

---

## 21. Errores frecuentes y qué hacer

| Mensaje | Causa | Solución |
|---|---|---|
| `vary no debe ejecutarse como root directo (euid 0)` | Lo lanzaste como root | Usa tu usuario normal; o `sudo vary ...` (baja privilegios y re-eleva lo justo). |
| `git es requerido por vary pero no está instalado` | Falta `git` | `sudo xbps-install git` |
| `¿estás en Void Linux? ¿instalaste xbps?` | `xbps` no está en el PATH | Instala Void, o no uses vary fuera de Void. |
| `el paquete no encontrado en repos oficiales ni VURs` | El nombre no existe, o es un virtual sin resolver | `vary -Ss <nombre>` para ver lo que sí existe; mira si falta un `--repo add`. |
| `el repo VUR 'x' NO coincide: esperado … recibido …` | La llave del repo cambió | Si es una rotación legítima: `vary --repo rekey x`. Si no lo esperabas: **no** lo aceptes (investiga). |
| `no hay herramienta de elevación de privilegios (busqué sudo, doas y run0)` | Falta sudo/doas | `sudo xbps-install sudo` |
| `falló ./xbps-src binary-bootstrap: './xbps-src' no encontrado` | El clon de void-packages está incompleto | `rm -rf ~/.cache/vary/void-packages` y reintenta. |
| `el plist … ocupa N bytes (tope …)` | Repo hostil o corrupto | El repo publica una llave enorme: no se acepta. Cambia de repo. |
| `índice X no disponible (sin red ni caché válida): <motivo de curl>` | Fallo de red/TLS/tamaño | El mensaje incluye ahora el error real de curl. |
| `lock: 'x' resuelve … pero vary.lock pinea …` | El estado cambió desde el lock | `vary --lock` para regenerar. |
| `campo desconocido 'enabld'` en `repos.conf` | Typo en la configuración | Corrige la clave; el mensaje nombra el campo exacto. |
| `nombre de repositorio reservado o no permitido: 'official'` | Nombre reservado (colisiona con el centinela interno) | Usa otro nombre para el repo. |

---

## 22. Glosario

| Término | Qué significa |
|---|---|
| **VUR** | Void User Repository: repo git de recetas de terceros. |
| **`.VURINFO`** | Índice declarativo (JSON, esquema v1) que describe los paquetes de un VUR. |
| **template** | La receta: un fichero de shell con `pkgname=`, `version=`, `depends=`… |
| **pkgver** | `nombre-versión_revisión` (p. ej. `firefox-146.0_1`). |
| **xbps-src** | El compilador de Void: lee una plantilla y produce un `.xbps`. |
| **masterdir** | El entorno limpio donde compila xbps-src. |
| **DAG / niveles** | El grafo de dependencias; los niveles marcan qué se puede compilar en paralelo. |
| **repolock** | Marca en la DB de xbps que un paquete lo gestiona vary (no lo pises con un binario). |
| **TOFU** | "Trust On First Use": la primera vez que ves una llave, te la enseñan y la aceptas. |
| **anchor / anclaje** | Lo que impide que un binario oficial pise lo que vary compiló. |
| **overlay** | Capa de parches de usuario que vary aplica encima de la plantilla y luego deshace. |
| **`--print` / plan** | La lista de lo que se instalaría y compilaría, sin hacer nada. |
| **set (`@...`)** | Un grupo de paquetes con nombre corto (`@world`, `@installed`…). |
| **opción (`XBPS_PKG_OPTIONS`)** |interruptor de la plantilla (p. ej. `pgo`, `wayland`). |

---

## ¿Algo no funciona?

Antes de reportar: `-vv` para el detalle, `~/.cache/vary/logs/` para el log de
compilación, y `vary --log` para el historial. Con eso, un reporte con el
comando exacto, lo que esperabas y lo que pasó se puede arreglar en minutos.
