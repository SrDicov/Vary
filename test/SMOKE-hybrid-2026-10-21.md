# SMOKE híbrido — `--deep`, overlay y `--conf-update` (2026-10-21)

Binario: `.xbps` glibc del run XBPS `39578897278` (`36984f9`, CI+verifica
verde), reinstalado después con `36ccebb`. Entorno: Docker local
`voidlinux/voidlinux:latest` (glibc), `--network host`; no se construyó vary
en local (`cargo build` sigue prohibido salvo caída de CI).

## Preparación

- Contenedor Void limpio, repos apuntando a
  `https://repo-default.voidlinux.org/current`, `xbps-install -u xbps -y`.
- Usuario no-root `builder` con sudo sin contraseña (`xbps-src` se niega a
  root).
- `vary` instalado desde el `.xbps` del run (`xbps-rindex -a` +
  `xbps-install --repository=/pkg vary`).
- `git clone --depth=1` de void-packages en
  `~builder/.cache/vary/void-packages` y
  `./xbps-src binary-bootstrap` como `builder`.
- Overlay de usuario:
  `~builder/.config/vary/files/base-files/issue` = `VARY-OVERLAY-OK`.
- `base-files` con `revision=2` en el checkout para que xbps instale el
  artefacto sobre el `_1` ya presente.

## S1 — build oficial con `-N` + overlay ✅

```sh
vary -S --deep --force-build --noconfirm base-files
```

- Log: `xbps-src -N -c vary-<pid>-1 pkg base-files` → `-N` confirmado.
- Se construyeron `xbps-triggers-0.131_1` (dependencia, bajo `-N`) y
  `base-files-0.146_2`.
- `/etc/issue` contiene `VARY-OVERLAY-OK`.
- `git status --porcelain` en el checkout: solo
  `M srcpkgs/base-files/template` (la subida de revisión del smoke); sin
  `files/issue` inyectado, sin `999-vary-user-*`, sin `.vary-bak.*`.
- `pkgdb-0.38.plist`: `repolock` = `true`; `repository` =
  `hostdir/binpkgs` local.

**Bug encontrado y corregido:** el parser de `xbps-src show` esperaba
`version=`/`revision=`, pero el formato real es `version:\tX`/`revision:\tY`;
abortaba con "no trae version/revision parseables". Fix en `36984f9`
(acepta `:` y `=`, test con el formato real) + `e1ced30` (clippy).

## S2 — `--conf-update` ✅

Los `.new-*` reales del build anterior:
`/etc/group.new-0.146_2`, `/etc/passwd.new-0.146_2`.

- Sin TTY: `vary --conf-update` lista
  `nuevo -> original` y sale 0; `sha256sum` de `/etc/passwd` y `/etc/group`
  idéntico antes/después (no escribe).
- `--noconfirm`: mismo listado, sin escrituras.
- `vary --conf-update -S foo`: error de combinación, exit 1.
- Con PTY (`script -qec`), fichero de prueba
  `/etc/vary-conf-test.new-1_1`: `r` reemplaza, deja
  `/etc/vary-conf-test.vary-bak.<pid>` con el contenido viejo y elimina el
  `.new-1_1`.

## Estado

Contenedor desechable; sin cambios en el host ni en el repo. `main` y
`testing` en `36ccebb` (CI y XBPS verdes) al cerrar el smoke.
