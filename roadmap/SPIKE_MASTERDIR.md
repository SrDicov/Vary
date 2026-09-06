# SPIKE DE MASTERDIRS: Análisis Arquitectónico y Estrategia de Concurrencia (P1-3 / SA-2)

**Fecha:** 2026-09-06  
**Documento de Decisión Técnica (Punto de Control Humano PC-2)**

---

## 1. El Problema Crítico

`xbps-src` en Void Linux asume posesión exclusiva y mutabilidad directa de su árbol `masterdir` y `hostdir/binpkgs`. Si múltiples hilos de compilación (`Action::Build`) invocan `./xbps-src pkg <A>` y `./xbps-src pkg <B>` concurrentemente:
1. **Colisión en `srcpkgs/`:** Los paquetes proyectados desde repositorios VUR se pisan entre sí en `void-packages/srcpkgs/<pkg>`.
2. **Colisión en `masterdir/`:** Los chroots compartidos se contaminan con dependencias intermedias de compilación (`destdir/`, `builddir/`, locks internos de xbps).
3. **Destrucción de repodata:** Al finalizar la compilación, la indexación del repositorio binario local (`<arch>-repodata`) es sobreescrita por el último worker en terminar, perdiendo los paquetes generados por los demás.

El código actual en el working tree (`OverlayGuard` en `src/masterdir.rs`) intenta montar un overlay sobre todo el checkout de `void-packages`, pero muta la capa inferior (`lower/srcpkgs`) antes de montar (lo cual es Comportamiento Indefinido en Linux OverlayFS) y realiza un `cp -aT` no sincronizado hacia `lower/hostdir/binpkgs`. Es un **esqueleto inconcluso y peligroso** si se ejecuta en paralelo.

---

## 2. Lecciones de `xbps-fbulk` (La Solución Oficial de Void)

`xbps-fbulk` (escrito en C, parte del repositorio oficial `xbps-bulk` de Void) es el estándar de la distribución para builds masivos concurrentes. Su arquitectura opera bajo los siguientes principios:

1. **Aislamiento por Worker mediante `xbps-uchroot`:**
   - Exige `XBPS_CHROOT_CMD=uchroot` (con bit setgid `4750` perteneciente al grupo `xbuilder`).
   - Invoca `xbps-uchroot -O -t <masterdir_base> <masterdir_temp>` aprovechando los User Namespaces del kernel Linux para montar OverlayFS efímero **únicamente sobre el directorio `masterdir/`**, sin requerir privilegios de root para el orquestador.
2. **Inmutabilidad de `srcpkgs/`:**
   - El árbol de recetas `srcpkgs/` se mantiene estrictamente de solo lectura.
3. **Serialización de Indexación de `binpkgs`:**
   - Los binarios `.xbps` producidos por cada worker se depositan en una cola o directorio compartido, pero la invocación de `xbps-rindex` (generación de `<arch>-repodata`) se ejecuta de manera **estrictamente secuencial mediante un lock o en el hilo principal** una vez completada la compilación.

---

## 3. Viabilidad de Reflink (CoW) vs Fallback a Copia

Si no se dispone de soporte de kernel para OverlayFS o permisos de namespaces:
- **Reflink (Btrfs / XFS):** `cp --reflink=always` o `btrfs subvolume snapshot` permite clonar un `masterdir` base completo en < 100 ms con 0 bytes de costo de almacenamiento inicial.
- **Fallback a copia estándar (ext4 / ZFS sin clone):** Un `masterdir` mínimo bootstrapped pesa entre 800 MB y 1.4 GB. Copiarlo en frío toma de 8 a 25 segundos por worker y satura el I/O del disco. **No es viable como estrategia de concurrencia en discos lentos sin CoW.**

---

## 4. Opciones de Arquitectura para la Decisión Humana (PC-2)

### Opción 1: Aislamiento por Worker mediante Overlay de Kernel / Namespaces (Estilo `xbps-fbulk`)
- **Mecanismo:** Cada worker concurrente de `vary` monta un overlay efímero sobre `masterdir/` (usando `xbps-uchroot -O` si está disponible, o `fuse-overlayfs`/kernel overlay con elevate). Las plantillas VUR se inyectan en el `upperdir` del overlay del worker (nunca en el `lower` compartido). La indexación de `binpkgs` se serializa en el hilo principal con un mutex.
- **Ventajas:** Máximo rendimiento; paralelismo real de CPU y memoria; masterdir base limpio e intacto.
- **Desventajas:** Requiere privilegios de kernel overlayfs o `xbps-uchroot` configurado; mayor complejidad de código.

### Opción 2: Paralelismo Intra-Paquete + Fetch Concurrente (Enfoque Pragmático Seguro)
- **Mecanismo:** Mantener la compilación de paquetes a nivel de `xbps-src` **estrictamente secuencial** en el `masterdir` único (garantizando 0 colisiones y máxima estabilidad), pero:
  1. Descargar todas las fuentes (`./xbps-src fetch`) de forma concurrente para todos los paquetes antes de compilar.
  2. Asignar todos los núcleos disponibles (`XBPS_MAKEJOBS=$(nproc)`) al compilador de cada paquete.
  3. Desacoplar la preparación y proyección de recetas.
- **Ventajas:** Cero riesgo de colisión o corrupción de `repodata`; 100% compatible con cualquier sistema de archivos y permisos sin requerir overlayfs ni sudo adicional; simplicidad extrema y mantenimiento mínimo.
- **Desventajas:** No paraleliza la compilación entre dos paquetes pequeños independientes (ambos usan `make -jN` secuencialmente).

### Opción 3: Clones de Masterdir vía CoW/Reflink con fallback a Semáforo Secuencial
- **Mecanismo:** Si el filesystem soporta reflink (`cp --reflink=auto`), clona masterdirs efímeros para los N workers; si el filesystem no soporta reflink, degrada automáticamente a la Opción 2 (secuencial).
- **Ventajas:** Aprovecha Btrfs/XFS si existe sin penalizar sistemas ext4.
- **Desventajas:** Detección heurística compleja en tiempo de ejecución.

---

## 5. Recomendación del Auditor

Para la madurez actual de `vary` como helper de usuario:
- **Fase Inmediata (Fase 4):** Adoptar la **Opción 2** (paralelismo de fetch + secuencialidad segura en build con `XBPS_MAKEJOBS` al máximo y eliminación de los bugs destructivos de `OverlayGuard`).
- **Roadmap P1:** Preparar el terreno para la **Opción 1** mediante la spec de sandbox preflight (P0-1) y namespaces `xbps-uchroot`.
