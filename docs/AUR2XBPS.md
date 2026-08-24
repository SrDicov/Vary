# Compatibilidad aur2xbps

[aur2xbps](https://github.com/SrDicov/aur2xbps) transpila paquetes del AUR en artefactos de Void Linux: lee únicamente los metadatos declarativos `.SRCINFO` (nunca evalúa scripts bash de PKGBUILD) y genera plantillas estándar de `void-packages`, flakes de Nix herméticos o binarios `.xbps` firmados. vary apuesta por reutilizar ese ecosistema en lugar de duplicarlo: los mantenedores de repos VUR usan `aur2xbps` para portar y actualizar paquetes, mientras que los usuarios finales solo interactúan con vary.

vary no invoca aur2xbps directamente. La compatibilidad es a nivel de repositorio: sus plantillas son estándar de Void y se compilan con `xbps-src`, exactamente lo que vary usa por debajo.

## Flujo inicial de un paquete AUR

1. Generar template: `aur2xbps template <pkgname>` — por defecto se sincroniza en `void-packages/srcpkgs/<pkgname>/`; usa `--out DIR --no-sync` para escribirlo en otra parte.
2. Copiar a tu VUR: `cp -r void-packages/srcpkgs/<pkgname>/ /tu-vur/srcpkgs/`
3. Regenerar el `.VURINFO` (ver [`docs/VURINFO.md`](./VURINFO.md) y `scripts/vur-generator.sh`).
4. Commit y push: `git add -A && git commit -m "add <pkgname>" && git push`
5. Usuarios instalan con: `vary -S <pkgname>`

## Flujo de actualización cuando el AUR upstream cambia

1. Detectar cambio: `aur2xbps query <pkgname>` imprime los metadatos del AUR RPC v5 en JSON; compara su versión con la de tu VUR.
2. Regenerar template: `aur2xbps template <pkgname>`
3. Reemplazar en tu VUR: `rsync -a void-packages/srcpkgs/<pkgname>/ /tu-vur/srcpkgs/<pkgname>/`
4. Regenerar el `.VURINFO` y commit/push
5. Usuarios ejecutan `vary -Syu` que detecta el nuevo commit

La automatización de este flujo (un comando tipo `vary --repo regen-aur <pkg>`) se pospone a fase 2 del proyecto.
