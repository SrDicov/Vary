# Compatibilidad aur2xbps

[aur2xbps](https://gitlab.com/xtraeme/aur2xbps) convierte plantillas del AUR (PKGBUILD) en plantillas válidas para `void-packages`. vary apuesta por reutilizar ese ecosistema en lugar de duplicarlo: los mantenedores de repos VUR usan `aur2xbps` para portar y actualizar paquetes, mientras que los usuarios finales solo interactúan con vary.

vary no invoca aur2xbps directamente. La compatibilidad es a nivel de repositorio.

## Flujo inicial de un paquete AUR

1. Generar template: `aur2xbps template <pkgname>`
2. Copiar a tu VUR: `cp -r <template-output>/srcpkgs/<pkgname>/ /tu-vur/srcpkgs/`
3. Regenerar .VURINFO (ver scripts/vur-generator.sh)
4. Commit y push: `git add -A && git commit -m "add <pkgname>" && git push`
5. Usuarios instalan con: `vary -S <pkgname>`

## Flujo de actualización cuando el AUR upstream cambia

1. Detectar cambio: `aur2xbps query <pkgname>` (comparar version con tu VUR)
2. Regenerar template: `aur2xbps template <pkgname>`
3. Reemplazar en tu VUR: `rsync -a <template-output>/srcpkgs/<pkgname>/ /tu-vur/srcpkgs/<pkgname>/`
4. Regenerar .VURINFO y commit/push
5. Usuarios ejecutan `vary -Syu` que detecta el nuevo commit

La automatización de este flujo (un comando tipo `vary --repo regen-aur <pkg>`) se pospone a fase 2 del proyecto.
