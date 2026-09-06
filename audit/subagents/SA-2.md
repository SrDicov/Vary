# AUDIT REPORT SA-2 — Concurrencia, Paralelismo y Ciclo de Vida de Procesos

**Subagente:** SA-2 — Concurrencia y Paralelismo  
**Fecha:** 2026-09-06  
**Proyecto:** `vary` (Void User Repository helper en Rust)  
**Rama / Commit:** `vary-mvp` @ `1e32f1f` (+ cambios no commiteados auditados)  
**Área de Auditoría:** Concurrencia, Sincronización, Procesos y Gestión de Masterdirs  

---

## 1. Resumen Ejecutivo

La auditoría exhaustiva de concurrencia y gestión de procesos en `vary` revela que el sistema se encuentra en un estado **híbrido y potencialmente inestable**:

1. **Conflicto de Masterdir y Concurrencia (Crítico):** El struct `OverlayGuard` no commiteado en `src/masterdir.rs` representa un **esqueleto inconcluso y peligroso** ante cualquier intento de ejecución paralela. Aunque aísla la compilación de `xbps-src` en un mount de OverlayFS temporal, interactúa de forma destructiva con `lower/srcpkgs` (las plantillas VUR se copian en el árbol base antes del montaje) y con `hostdir/binpkgs` (las copias concurrentes de paquetes binarios sobrescriben y corrompen el archivo `<arch>-repodata`).
2. **Bandera Fantasma `max_concurrent_builds`:** Documentada en el README, configurada en TOML y deserializada en `Config`, pero **100% ignorada en el código**. El bucle de compilación en `src/install.rs` es estrictamente secuencial y el DAG de `petgraph` se aplana a un vector lineal sin noción de olas o niveles topológicos (incumplimiento contractual R1).
3. **Bloqueo Global Prematuro:** `InstanceLock` en `src/lock.rs` usa `flock` exclusivo sobre `vary.lock`. Funciona a nivel de kernel y se libera en caídas o `SIGKILL`, pero se adquiere antes de parsear los argumentos de CLI, bloqueando consultas inocuas (`vary -h`, `vary -V`, `vary -Ss`) durante compilaciones largas. Además, emite un consejo erróneo y peligroso de borrar el archivo de lock si falla la adquisición.
4. **Manejo de Señales Asíncrono pero con Condiciones de Carrera:** El diseño de handler atómico (`AtomicI32`) y observador multihilo con `kill(-pid)` y `setpgid` es conceptualmente correcto, pero adolece de ventanas de carrera críticas (TOCTOU) al registrar hijos y montajes, no restaura el estado del terminal y produce fugas permanentes de directorios `/tmp/vary-*` al interrumpir con `SIGINT`/`SIGTERM` debido a la invocación de `std::process::exit`.
5. **Drenaje de Logging:** `tracing_appender::non_blocking` pierde registros acumulados en memoria cuando el observador de señales o el panic hook invoca `exit()`, ya que se omite el `Drop` del `WorkerGuard`.

---

## 2. Catálogo de Hallazgos

| ID | Severidad | Módulo(s) | Título Breve |
|---|---|---|---|
| **[H-010]** | 🔴 **CRÍTICO** | `src/masterdir.rs:19-143, 199-235`<br>`src/install.rs:328-333`<br>`src/vur_client.rs:396-504` | Conflicto destructivo de concurrencia en Masterdir: `OverlayGuard` colisiona en `lower/srcpkgs` y corrompe `hostdir/binpkgs` |
| **[H-011]** | 🔴 **CRÍTICO** | `src/masterdir.rs:39-103`<br>`src/xbps.rs:391-417` | Deficiencia arquitectónica frente a `xbps-fbulk`: Ausencia de chroot aislado con namespaces (`uchroot`) |
| **[H-012]** | 🟠 **ALTO** | `src/config.rs:115, 203`<br>`src/install.rs:301-341`<br>`src/resolver.rs:1-3, 297-306` | Bandera fantasma `max_concurrent_builds` y ausencia total de scheduler topológico concurrente (Incumplimiento R1) |
| **[H-013]** | 🟡 **MEDIO** | `src/lock.rs:20-53`<br>`src/lib.rs:88-96` | Bloqueo exclusivo prematuro de operaciones de solo lectura y consejo erróneo de remediación que induce a split-brain |
| **[H-014]** | 🟠 **ALTO** | `src/xbps.rs:411-416`<br>`src/masterdir.rs:55-89`<br>`src/signal.rs:97-131, 165-185` | Condiciones de carrera (TOCTOU) en registro de procesos/montajes y fuga de directorios temporales ante señales |
| **[H-015]** | 🟡 **MEDIO** | `src/logging.rs:35-38, 74-76`<br>`src/signal.rs:184`<br>`src/main.rs:61` | Fuga de logs en búfer asíncrono ante terminación abrupta con `exit()` y omisión de captura de stdio de subprocesos |

---

## 3. Análisis Exhaustivo de Hallazgos

```
                                  VARY CONCURRENCY MAP
                                  
  [CLI / User Input]
          |
  (1) lock::acquire() -------> [ vary.lock ] (flock exclusivo prematuro [H-013])
          |
  (2) resolver::finish() ----> petgraph::algo::toposort() -> Vec<PlanItem> (1D lineal [H-012])
          |
  (3) install::build_pkg()
          |
     +----+---------------------------------------------------+
     |                                                        |
     | [Worker A]                                             | [Worker B] (Si R1 existiera)
     |    |                                                   |    |
     | 1. project_pkg() --------------------------------------+----+---> [ lower/srcpkgs/ ]
     |    (Escribe en lowerdir compartido [H-010])            |    |     (¡Colisión y UB en kernel!)
     |    |                                                   |    |
     | 2. OverlayGuard::mount()                               |    |
     |    (lower = void-packages)                             |    |
     |    |                                                   |    |
     | 3. xbps-src pkg A                                      |    |
     |    |                                                   |    |
     | 4. cp -aT merged_binpkgs target_binpkgs ---------------+----+---> [ hostdir/binpkgs/ ]
     |    (Sin mutex de repositorio [H-010])                  |          (¡Corrupción de *-repodata!)
     +--------------------------------------------------------+
```

---

### [H-010] Conflicto destructivo de concurrencia en Masterdir: `OverlayGuard` colisiona en `lower/srcpkgs` y corrompe `hostdir/binpkgs`

- **Severidad:** 🔴 **CRÍTICO**
- **Archivos y Líneas:**
  - `src/masterdir.rs:19-143, 199-235`
  - `src/install.rs:328-333`
  - `src/vur_client.rs:396-476, 478-504`

#### Descripción del Problema

`xbps-src` asume **exclusividad estricta** sobre su `masterdir`: dentro de él monta `/proc`, `/sys`, `/dev`, ejecuta `xbps-install` para satisfacer dependencias en tiempo de compilación dentro del chroot, y manipula directorios de compilación fijos (`/builddir`, `/destdir`).

Para mitigar esto, en `src/masterdir.rs` se introdujo el struct `OverlayGuard`, el cual monta un OverlayFS donde `lower` es el directorio raíz de `void-packages` (`~/.cache/vary/void-packages`). Sin embargo, el análisis del flujo de compilación demuestra que `OverlayGuard` es un **esqueleto inconcluso y destructivo** si se somete a concurrencia:

#### 1. Inyección de plantillas en la capa `lower` compartida (Kernel UB y carreras)

En `src/install.rs:328-333`:

```rust
328: repo.materialize_pkg(&parent_pkg)?;
329: repo.project_pkg(&md.srcpkgs_dir(), &parent_pkg, explicit)?;
330: let res = md.build_pkg(&parent_pkg, &config.sudo_bin, &config.sudo_flags);
331: // Always unproject (proyectamos parent_pkg)
332: let _ = repo.unproject_pkg(&md.srcpkgs_dir(), &parent_pkg);
333: res.with_context(|| format!("building {}", parent_pkg))?;
```

Y en `src/masterdir.rs:200`:

```rust
199: pub fn build_pkg(&self, pkg: &str, sudo_bin: &str, sudo_flags: &[String]) -> Result<()> {
200:     let guard = OverlayGuard::mount(&self.path, sudo_bin, sudo_flags)?;
...
```

**Evidencia de Falla:**
1. `project_pkg` escribe directamente en `md.srcpkgs_dir()`, es decir, en `void-packages/srcpkgs/<pkg>`, que es el **directorio `lower`** del OverlayFS.
2. `OverlayGuard::mount` se invoca **después** de `project_pkg`.
3. Si dos trabajadores (Worker 1 y Worker 2) compilaran en paralelo:
   - Worker 1 proyecta `pkgA` en `lower/srcpkgs/pkgA` y monta su `OverlayGuard`.
   - Worker 2 proyecta `pkgB` en `lower/srcpkgs/pkgB` mientras el overlay de Worker 1 ya está activo.
   - **Comportamiento Indefinido en Linux Kernel OverlayFS:** La documentación oficial del kernel establece taxativamente: *“Changes to the underlying filesystems while part of a mounted overlay filesystem are not allowed. If the underlying filesystem is changed, the behavior of the overlay is undefined”*. Modificar `lower` produce inconsistencias de inodos, entradas `dentry` obsoletas o errores de EIO dentro del chroot.
   - **Carrera de limpieza:** Cuando Worker 1 termina, `unproject_pkg` ejecuta `git checkout -- srcpkgs/pkgA` en el árbol raíz compartido, pudiendo revertir o colisionar con operaciones de git concurrentes de Worker 2.

#### 2. Colisión destructiva en `hostdir/binpkgs` (Corrupción de `*-repodata`)

En `src/masterdir.rs:209-235`:

```rust
209: if let Some(merged_binpkgs) = guard.merged_binpkgs() {
210:     let target_binpkgs = self.path.join("hostdir").join("binpkgs");
211:     if merged_binpkgs.exists() {
212:         std::fs::create_dir_all(&target_binpkgs).ok();
213:         // Copiar el contenido para evitar binpkgs/binpkgs, por el
214:         // mismo camino de elevación que usó el mount.
215:         let copy_ok = if guard.used_sudo() {
216:             crate::elevate::elevate(sudo_bin, sudo_flags, "cp")
217:                 .and_then(|mut cmd| {
218:                     cmd.args(["-aT", &merged_binpkgs.display().to_string(), &target_binpkgs.display().to_string()])
219:                         .status()
220:                         .map(|s| s.success())
221:                         .map_err(anyhow::Error::from)
222:                 })
223:                 .unwrap_or(false)
...
```

**Evidencia de Falla:**
`xbps-src` genera los binarios empaquetados en `hostdir/binpkgs` y ejecuta automáticamente `xbps-rindex -a` para indexar el nuevo archivo `.xbps` en la base de datos `<arch>-repodata`.
- Al realizar un `cp -aT` masivo y no atómico desde `merged/hostdir/binpkgs` hacia el directorio compartido `lower/hostdir/binpkgs`, se copia **todo el árbol**, incluyendo el archivo `<arch>-repodata` generado en el espacio aislado de ese worker.
- Si Worker 1 y Worker 2 terminan de compilar aproximadamente al mismo tiempo, el `cp -aT` de Worker 2 sobrescribirá el `<arch>-repodata` de Worker 1. El paquete compilado por Worker 1 quedará como un archivo `.xbps` huérfano en el disco, ausente del índice del repositorio.
- Cuando la fase de instalación final (`xbps-install --repository=...`) intente instalar los paquetes compilados, fallará informando que el paquete de Worker 1 no fue encontrado en el repositorio.
- No existe ningún mecanismo de sincronización, mutex o lockfile para serializar la indexación de paquetes binarios.

#### 3. Degradación no aislada silenciosa

En `src/masterdir.rs:36-38, 71-78`:
Si `mount -t overlay` con elevación falla (por ejemplo, si no hay sudo configurado) y `fuse-overlayfs` no está instalado en el sistema (lo cual es el caso por defecto en Void Linux base), `mounted` evalúa a `false`.
`OverlayGuard` entra en modo degraded: `is_isolated()` devuelve `false` y `target()` devuelve el `lower` original sin aislar. Dos workers concurrentes en este modo operarían **exactamente sobre el mismo `masterdir`**, corrompiendo de inmediato la base de datos `/var/db/xbps` interna del chroot.

#### Remediación Recomendada

1. **Inmutabilidad absoluta de `lower`:** `lower` (`void-packages`) debe ser estrictamente de solo lectura.
2. **Inyección en `upper`:** La proyección de plantillas VUR (`project_pkg`) debe realizarse directamente dentro de la capa privada del worker (`upper/srcpkgs/<pkg>`), después de crear los directorios y antes de montar el overlayfs, o bien directamente en el punto de montaje `merged/srcpkgs/<pkg>` de ese worker. De este modo, `lower` nunca se modifica y no hay colisiones entre workers.
3. **Indexación atómica de binarios:** No realizar `cp -aT` de todo `binpkgs`. Cada worker debe copiar únicamente sus archivos `.xbps` generados a un búfer común, y la actualización del repodata debe realizarse mediante una única rutina sincronizada:
   ```rust
   // Proteger la invocación de xbps-rindex con un mutex inter-hilos
   let _lock = BINPKGS_INDEX_LOCK.lock().unwrap();
   xbps_rindex_add(&shared_binpkgs, &pkg_xbps_file)?;
   ```

---

### [H-011] Deficiencia arquitectónica frente a `xbps-fbulk`: Ausencia de chroot aislado con namespaces (`uchroot`)

- **Severidad:** 🔴 **CRÍTICO**
- **Archivos y Líneas:**
  - `src/masterdir.rs:39-103`
  - `src/xbps.rs:391-417`
  - Referencia oficial: Manual de `xbps-fbulk(1)` y `xbps-uchroot(1)`

#### Análisis Comparativo con `xbps-fbulk`

Void Linux cuenta con una herramienta oficial desarrollada en C para builds masivos y paralelos de paquetes: `xbps-fbulk`. Al analizar su arquitectura se evidencia por qué el enfoque de `vary` es defectuoso:

| Característica | `xbps-fbulk` (Oficial Void) | `vary` (`OverlayGuard`) |
|---|---|---|
| **Mecanismo de Aislamiento** | Obligatorio `XBPS_CHROOT_CMD=uchroot` (`xbps-uchroot -O -t`) | `sudo mount -t overlay` o `fuse-overlayfs` sobre todo el árbol |
| **Punto de Montaje** | Exclusivo sobre el subdirectorio `masterdir` (chroot) | Monta la raíz completa de `void-packages` (código + scripts + chroot) |
| **Privilegios** | Sin `sudo` en runtime: `xbps-uchroot` utiliza namespaces de Linux (`CONFIG_USER_NS`, `CONFIG_OVERLAY_FS`) | Requiere elevar privilegios (`sudo`/`doas`/`run0`) en cada mount/umount y cada `cp` |
| **Gestión de `binpkgs`** | Cola de indexación serializada con locks | Copia destructiva en bloque con `cp -aT` que machaca `repodata` |
| **Manejo de Templates** | `srcpkgs` es estático y de solo lectura | Muta dinámicamente `srcpkgs` en el árbol raíz |

#### Evidencia en el Ecosistema Void

En la sección `NOTES` del manual de `xbps-fbulk(1)`:
> *"The masterdir in the void-packages repository must be fully populated for chroot operations, and some options need to be set in etc/conf to make xbps-fbulk work correctly:*  
> `XBPS_CHROOT_CMD=uchroot`  
> *The xbps-uchroot(1) utility is required because xbps-fbulk builds packages in temporary masterdirs that are mounted with overlayfs."*

Y en el manual de `xbps-uchroot(1)`:
> *-O: Setups a temporary directory and then creates an overlay layer (via overlayfs) with the lowerdir set to CHROOTDIR. Useful to create a temporary tree that does not preserve changes in CHROOTDIR.*

#### Diagnóstico para `vary`

El diseño actual de `OverlayGuard` intenta reinventar `xbps-uchroot` en el espacio de usuario externo mediante llamadas a `sudo mount` de directorios completos. Esto introduce:
1. Fatiga de credenciales o fallos si el usuario no tiene sudo sin contraseña.
2. Inestabilidad en montajes si el proceso se interrumpe (ver [H-014]).
3. Fragilidad operativa al no utilizar las capacidades nativas que `xbps-src` ya posee para invocar chroots basados en namespaces (`uchroot` o `bwrap`).

---

### [H-012] Bandera fantasma `max_concurrent_builds` y ausencia total de scheduler topológico concurrente (Incumplimiento R1)

- **Severidad:** 🟠 **ALTO**
- **Archivos y Líneas:**
  - `src/config.rs:115, 155, 203, 249-250`
  - `src/install.rs:301-341`
  - `src/resolver.rs:1-3, 297-306`
  - `etc/vary.conf.example:34`
  - `README.md:54, 137`
  - `README.es.md:54, 138`

#### Descripción del Problema

El contrato de arquitectura (Requisito **R1**) y la documentación de usuario prometen compilación paralela basada en grafos dependientes del parámetro `max_concurrent_builds`. Sin embargo, la auditoría confirma que este paralelismo es **completamente inexistente (código fantasma)**:

#### Evidencia en Código Real

1. **Lectura cosmética en `src/config.rs`:**
```rust
115: pub max_concurrent_builds: u32,
...
155: max_concurrent_builds: Option<u32>,
...
203: max_concurrent_builds: 2,
...
249: if let Some(n) = file.build.max_concurrent_builds {
250:     self.max_concurrent_builds = n;
251: }
```
`max_concurrent_builds` se inicializa y se deserializa desde `vary.conf`, pero **jamás se vuelve a referenciar en toda la lógica de ejecución del programa**.

2. **Aplanamiento unidimensional del grafo en `src/resolver.rs`:**
```rust
295: fn finish(self) -> Result<Plan> {
296:     // Red de seguridad: los ciclos reales ya se detectan en resolve_name.
297:     let order = toposort(&self.graph, None)
298:         .map_err(|_| anyhow::anyhow!("ciclo de dependencias residual tras la expansión"))?;
299: 
300:     let mut builds = Vec::new();
301:     for idx in order {
302:         let item = &self.items[idx as usize];
303:         if matches!(item.action, Action::Build) {
304:             builds.push(item.clone());
305:         }
306:     }
...
319:     Ok(Plan { installs: officials, builds })
320: }
```
El resolvedor toma el DAG de `petgraph` y ejecuta `toposort(&self.graph, None)`. Esto aplana toda la estructura topológica a un `Vec<PlanItem>` puramente lineal. Toda información sobre qué ramas del grafo son independientes entre sí se destruye en `finish()`.

3. **Bucle de ejecución estrictamente secuencial en `src/install.rs`:**
```rust
300: let mut built_names: Vec<String> = Vec::new();
301: for item in &plan.builds {
...
328:     repo.materialize_pkg(&parent_pkg)?;
329:     repo.project_pkg(&md.srcpkgs_dir(), &parent_pkg, explicit)?;
330:     let res = md.build_pkg(&parent_pkg, &config.sudo_bin, &config.sudo_flags);
331:     // Always unproject (proyectamos parent_pkg)
332:     let _ = repo.unproject_pkg(&md.srcpkgs_dir(), &parent_pkg);
333:     res.with_context(|| format!("building {}", parent_pkg))?;
334:     built_names.push(item.name.clone());
...
341: }
```
Las compilaciones se ejecutan de una en una mediante un simple bucle `for`. No hay `thread::spawn`, no hay semáforos de concurrencia, ni `JoinHandle`s.

4. **Comentarios que admiten la postergación:**
En `src/resolver.rs:1-3`:
```rust
1: // MVP: Builds 100% secuenciales. xbps-src maneja paralelismo interno (-j).
2: // Fase 2 del proyecto: builds concurrentes con tokio::sync::Semaphore
3: // usando max_concurrent_builds de vary.conf como límite.
```
*Nota:* `tokio` ni siquiera está listado en `Cargo.toml`.

#### Impacto

El hardware moderno con múltiples núcleos se desaprovecha completamente en árboles de dependencias complejos (por ejemplo, compilando dependencias independientes en paralelo). El usuario configura `max_concurrent_builds = 4` esperando aceleración, pero la herramienta ignora el valor en silencio.

---

### [H-013] Bloqueo exclusivo prematuro de operaciones de solo lectura y consejo erróneo de remediación que induce a split-brain

- **Severidad:** 🟡 **MEDIO**
- **Archivos y Líneas:**
  - `src/lock.rs:20-53`
  - `src/lib.rs:88-96`

#### Descripción del Problema

`src/lock.rs` implementa una exclusión mutua de instancia única mediante `nix::fcntl::Flock` sobre `<cache_dir>/vary.lock`. Aunque la implementación del lockfile a nivel de llamada al sistema (`flock`) es sólida frente a fallos del kernel, presenta dos graves deficiencias de diseño:

#### 1. Bloqueo prematuro de comandos de solo lectura

En `src/lib.rs:88-96`:

```rust
88: // Una sola instancia: protege /etc/xbps.d, la db heed y los mounts.
89: // El guardián vive hasta el final de run() y libera el flock al salir.
90: let _instance_lock = match crate::lock::acquire(&config.cache_dir) {
91:     Ok(lock) => lock,
92:     Err(err) => {
93:         print_error(Style::new(), err);
94:         return 1;
95:     }
96: };
97: 
98: match run2(&mut config, args) {
```

`lock::acquire` se ejecuta en la línea 90, **antes** de que `run2()` invoque `config.parse_args(args)`.
**Consecuencia:** Si un usuario está ejecutando una compilación que tarda 45 minutos (`vary -S libreoffice`), cualquier intento en otra terminal de ejecutar operaciones inocuas como:
- `vary --help` o `vary -V`
- `vary -Ss busqueda` (búsqueda en repositorios)
- `vary -Si paquete` (información de paquete)
- `vary --repo list` (listar repositorios)

**Fallará inmediatamente con error**, impidiendo al usuario consultar el sistema mientras se compila.

#### 2. Consejo peligroso de remediación (Vector de Split-Brain)

En `src/lock.rs:39-48`:

```rust
39: Err((mut file, nix::errno::Errno::EWOULDBLOCK)) => {
40:     let mut pid = String::new();
41:     let _ = file.read_to_string(&mut pid);
42:     let pid = pid.trim();
43:     anyhow::bail!(
44:         "otra instancia de vary está en ejecución{}; si no es así, borra {}",
45:         if pid.is_empty() { String::new() } else { format!(" (pid {})", pid) },
46:         path.display()
47:     );
48: }
```

**Análisis de Comportamiento del Kernel ante Crash o `SIGKILL`:**
- En Linux / POSIX, los locks establecidos con `flock(2)` están asociados a la **descripción de archivo abierto** en la tabla del kernel del sistema operativo.
- Si el proceso propietario del lock sufre un kernel panic, abort, `kill -9` (`SIGKILL`) o un segmentation fault, el kernel **cierra automáticamente todos los descriptores de archivo asociados al proceso**.
- Al cerrarse el último descriptor, el kernel **libera el `flock` automáticamente**.
- Por lo tanto, un archivo `vary.lock` abandonado tras una caída del proceso **nunca mantiene el lock activo**. El siguiente proceso que llame a `acquire()` obtendrá el lock de inmediato sin problema alguno.

**El peligro del mensaje:**
Aconsejar al usuario *"si no es así, borra ~/.cache/vary/vary.lock"* es un conocido antipatrón de file locking:
Si una instancia real está en ejecución y el usuario borra `vary.lock`, el archivo queda desvinculado (`unlinked`) del directorio. Cuando una nueva instancia arranca, `std::fs::OpenOptions::new().create(true).open()` creará un **nuevo inodo** en el sistema de archivos. La nueva instancia obtendrá el `flock` sobre el nuevo inodo, mientras la instancia anterior sigue ejecutándose con el `flock` sobre el inodo desvinculado.
**Resultado:** Dos instancias de `vary` ejecutándose concurrentemente, corrompiendo la base de datos `heed`, la configuración `/etc/xbps.d` y los directorios de compilación.

#### Remediación Recomendada

1. **Eliminar el consejo de borrado:** Informar el PID y sugerir comprobar si el proceso sigue activo (`ps -p <pid>`), pero nunca incitar a borrar el archivo.
2. **Granularidad de bloqueo:** Mover `acquire()` después del parseo de argumentos. Solo adquirir el lock exclusivo si la operación es mutante (`-S`, `-R`, `-U`, `--repo add/remove/rekey`). Comandos de consulta (`-Ss`, `-Si`, `-Q`, `--help`, `--version`) no deben adquirir el lock, o a lo sumo deben adquirir un lock compartido no bloqueante (`FlockArg::LockSharedNonblock`).

---

### [H-014] Condiciones de carrera (TOCTOU) en registro de procesos/montajes y fuga de directorios temporales ante señales

- **Severidad:** 🟠 **ALTO**
- **Archivos y Líneas:**
  - `src/xbps.rs:411-416`
  - `src/masterdir.rs:55-89`
  - `src/signal.rs:97-131, 165-185`

#### Descripción del Problema

`src/signal.rs` implementa un observador en segundo plano que atiende `SIGINT` y `SIGTERM`. El handler C (`on_signal`) solo almacena la señal en `static GOT_SIGNAL: AtomicI32` (async-signal-safe), y un hilo en bucle de sondeo (100 ms) ejecuta `observer_cleanup_and_exit(sig)`.

No obstante, la coordinación entre procesos e hilos presenta tres vectores de falla críticos:

#### 1. Condición de carrera en `register_child(pid)` (Hijos huérfanos)

En `src/xbps.rs:403-415`:

```rust
403: #[cfg(unix)]
404: unsafe {
405:     use std::os::unix::process::CommandExt;
406:     cmd.pre_exec(|| {
407:         nix::unistd::setpgid(nix::unistd::Pid::from_raw(0), nix::unistd::Pid::from_raw(0))
408:             .map_err(|e| std::io::Error::from_raw_os_error(e as i32))
409:     });
410: }
411: let mut child = cmd.spawn().map_err(|e| spawn_error("./xbps-src", e))?;
412: let pid = child.id();
413: crate::signal::register_child(pid);
414: let status = child.wait().map_err(|e| spawn_error("./xbps-src", e));
415: crate::signal::unregister_child(pid);
```

**Ventana de Carrera (TOCTOU):**
Entre la línea 411 (`cmd.spawn()`) y la línea 413 (`register_child(pid)`), el subproceso `xbps-src` ya está creado y ejecutándose en su **propio grupo de proceso** (`setpgid(0,0)`).
Si el usuario pulsa `Ctrl+C` exactamente en esa ventana de tiempo:
- El hilo observador despierta y lee `CHILDREN.lock()`.
- La lista de hijos está vacía porque `register_child` aún no se ejecutó.
- El observador sale con `std::process::exit(128 + sig)`.
- Como `xbps-src` corre con un `pgid` desacoplado del de `vary`, la señal de la terminal no le llega automáticamente. `xbps-src` y todos sus hijos (`make`, `gcc`) continúan compilando indefinidamente en segundo plano en estado huérfano.

#### 2. Ventana de carrera en `register_mount`

En `src/masterdir.rs:55-89`:
El comando `mount` (vía `sudo` o `fuse-overlayfs`) se ejecuta en las líneas 56-78. Si el montaje tiene éxito, recién en la línea 88 se invoca `register_mount(c.clone())`. Si llega una señal antes de la línea 88, el sistema de archivos queda montado en el kernel y nunca es registrado para desmontaje, dejando un mount huérfano en `/tmp`.

#### 3. Fuga permanente de directorios temporales en interrupción (`TempDir`)

En `src/signal.rs:165-185`:

```rust
165: fn observer_cleanup_and_exit(sig: i32) -> ! {
166:     // 1. Matar hijos por GRUPO (-pid): los nietos (make/ninja) mueren con el grupo.
...
178:     // 2. Desmontar overlays registrados (mismo camino elevado que el Drop).
179:     let pending: Vec<MountCleanup> = REGISTRY.lock().map(|r| r.clone()).unwrap_or_default();
180:     for c in &pending {
181:         c.run();
182:     }
183:     // 3. Recién ahora salir; código clásico 128+signo.
184:     std::process::exit(128 + sig);
185: }
```

**Mecánica de Fuga:**
`std::process::exit` aborta el proceso de inmediato a nivel del kernel de Linux. **No desenrolla la pila de ejecución (`stack unwinding`) del hilo principal**.
En consecuencia, los destructores de los campos `merged`, `upper` y `work` dentro de `OverlayGuard`:
```rust
28: merged: Option<tempfile::TempDir>,
30: upper: Option<tempfile::TempDir>,
32: work: Option<tempfile::TempDir>,
```
**nunca se ejecutan**. Los directorios `/tmp/vary-upper-*`, `/tmp/vary-work-*` y `/tmp/vary-merged-*` (que pueden contener gigabytes de objetos intermedios de compilación C++) quedan abandonados en el disco cada vez que se interrumpe `vary` con `Ctrl+C`.

#### 4. Ausencia de restauración del estado del terminal

Si la señal se produce mientras un proceso secundario o paginador (`less -R`, `bat`) tenía el terminal configurado en modo no canónico (raw mode o echo desactivado), `vary` aborta sin restaurar los flags termios originales del terminal, dejando la sesión del usuario corrupta.

---

### [H-015] Fuga de logs en búfer asíncrono ante terminación abrupta con `exit()` y omisión de captura de stdio de subprocesos

- **Severidad:** 🟡 **MEDIO**
- **Archivos y Líneas:**
  - `src/logging.rs:35-38, 74-76`
  - `src/signal.rs:184`
  - `src/main.rs:61`
  - `src/xbps.rs:399-411`

#### Descripción del Problema

#### 1. Fuga de logs no vaciados en `tracing_appender`

En `src/logging.rs:74-76`:
```rust
74: let (log_writer, worker) = tracing_appender::non_blocking(
75:     tracing_appender::rolling::daily(cache_dir, "vary.log"),
76: );
...
92: guard._guard = Some(worker);
```
`tracing_appender::non_blocking` utiliza un canal desacoplado con un búfer en memoria. La única forma de garantizar que los eventos encolados se escriban físicamente en el archivo `vary.log` es que el destructor `Drop` de `WorkerGuard` sea ejecutado:

```rust
// Documentación de tracing-appender:
// "WorkerGuard should be assigned in the main function or at least in a thread to keep
// the worker alive. When WorkerGuard is dropped, it will flush the buffer and stop the worker thread."
```

Sin embargo:
1. En `src/signal.rs:184`, `observer_cleanup_and_exit` ejecuta `std::process::exit(128 + sig)`.
2. En `src/main.rs:61`, ante broken pipe se ejecuta `exit(0)`.
3. Si ocurre un panic cuando se compila con `panic = "abort"`, o un abort en `Drop`.

En todos estos casos, `exit()` aborta el proceso sin ejecutar el `Drop` de `_guard` instanciado en `vary::run()`. Cualquier mensaje de log emitido justo antes de la interrupción o error **se pierde silenciosamente sin escribirse en disco**.

#### 2. Salida de compilación (`xbps-src`) no canalizada al archivo de log

En `src/xbps.rs:399-411`:
```rust
399: let mut cmd = Command::new("./xbps-src");
400: cmd.current_dir(masterdir).args(args);
...
411: let mut child = cmd.spawn().map_err(|e| spawn_error("./xbps-src", e))?;
```
`Command::new` no redirige `stdout` ni `stderr`. La salida masiva de `make`/`gcc`/`ninja` se imprime directamente en la consola del usuario y nunca se registra en `vary.log`. Esto representa un incumplimiento parcial del requisito contractual **A7** (que exigía redirigir la salida a `vary.log` mediante canales asíncronos mpsc, mostrando spinners en terminal).

---

## 4. Respuestas Puntuales al Cuestionario de Auditoría

### 1. Masterdir y Paralelismo
* **¿`xbps-src` asume exclusividad sobre `masterdir`?**  
  **Sí.** Utiliza un chroot único con bases de datos `/var/db/xbps` fijas, `/builddir` fijo y variables de entorno fijas. Compilar dos paquetes en el mismo masterdir corrompe las dependencias del chroot.
* **¿Es `OverlayGuard` utilizable, código muerto o un esqueleto inconcluso?**  
  Es un **esqueleto inconcluso y peligroso**. Aunque se invoca en `build_pkg`, no aísla las plantillas en `srcpkgs` (las escribe en el árbol `lower`), no sincroniza las copias a `hostdir/binpkgs` (corrompiendo `*-repodata`), y degrada a compilación sin aislamiento si fallan los privilegios.
* **¿Cómo lo resuelve `xbps-fbulk`?**  
  Exige `XBPS_CHROOT_CMD=uchroot` (`xbps-uchroot -O -t`). Monta overlays efímeros con Linux namespaces únicamente sobre el chroot del `masterdir`, manteniendo `srcpkgs` de solo lectura y gestionando `binpkgs` de forma centralizada.
* **¿Qué pasa si se ejecutan builds concurrentes hoy?**  
  Se pisan en `lower/srcpkgs` violando las invariantes del kernel sobre `lowerdir`, y se pisan mutuamente al copiar `hostdir/binpkgs` destruyendo el índice `<arch>-repodata`.
* **¿Qué ocurre con `max_concurrent_builds`?**  
  Es una **bandera fantasma**. Se lee en TOML pero el bucle de compilación es estrictamente secuencial (`for item in &plan.builds`).

### 2. Lockfile de Instancia Única
* **¿Usa flock exclusivo sobre `<cache_dir>/vary.lock`?**  
  **Sí.** Utiliza `nix::fcntl::Flock` en modo `LockExclusiveNonblock`.
* **¿Muestra el PID de la instancia que retiene el lock?**  
  **Sí.** Lee el PID grabado en el archivo y lo formatea en el mensaje de error.
* **¿Se libera limpiamente en Drop?**  
  **Sí.** El tipo `Flock` de `nix` invoca `unlock` al destruirse.
* **¿Qué pasa si el proceso crashea o recibe SIGKILL?**  
  El kernel de Linux cierra los descriptores de archivo y **libera el `flock` automáticamente**. No queda lock colgado a nivel del kernel.
* **Defecto detectado:** Bloquea indebidamente comandos de lectura (`-h`, `-V`, `-Ss`) y aconseja erróneamente borrar el archivo de lock, lo que puede provocar split-brain.

### 3. Scheduler del DAG y Concurrencia
* **¿Existe algún scheduler concurrente de petgraph o es 100% secuencial?**  
  Es **100% secuencial**. `resolver.rs` ejecuta `toposort(&self.graph, None)` convirtiendo el grafo en una lista plana unidimensional `Vec<PlanItem>`.
* **¿Por qué no hay paralelismo?**  
  Fue postergado contractualmente a una hipotética "Fase 2", dejando el requisito R1 en estado AUSENTE y sin la infraestructura previa requerida (aislamiento seguro de masterdir).

### 4. Manejo de Señales e Interrupciones
* **¿Cómo maneja SIGINT/SIGTERM? ¿El handler solo marca un flag atómico?**  
  **Sí.** El handler `on_signal` solo ejecuta `GOT_SIGNAL.store(sig, Ordering::SeqCst)`, lo cual es 100% async-signal-safe.
* **¿El hilo observador mata al grupo de procesos hijo (`kill(-pid)`)? ¿`xbps-src` corre con `setpgid`?**  
  **Sí.** `xbps_src` configura `setpgid(0,0)` en `pre_exec`, y el observador envía `SIGTERM` y posterior `SIGKILL` al grupo negativo `-pid`.
* **¿Se desmontan los overlays registrados en `MountCleanup`? ¿Qué pasa si falla?**  
  Se intenta desmontar con `umount` y `umount -l`. Si falla, el error se traga con `tracing::warn!` y el montaje queda colgado en el sistema.
* **¿Restaura la terminal? ¿Hay condición de carrera si la señal llega antes de registrar PID o mount?**  
  **No** restaura la terminal. **Sí** hay condición de carrera: si la señal llega entre `spawn()` y `register_child()`, el proceso hijo queda huérfano y nunca es terminado por el observador.

### 5. Hilo de Logging
* **¿Se cierran ordenadamente los canales de `tracing_appender::non_blocking`? ¿WorkerGuard hace flush en Drop?**  
  En terminación normal sí, `WorkerGuard::drop` vacía el canal.
* **¿Se pierden logs si hay `exit()` o `panic`?**  
  **Sí.** Tanto `observer_cleanup_and_exit` como el manejador de broken pipe en `main.rs` llaman a `std::process::exit()`, saltándose el `Drop` de `WorkerGuard` y perdiendo todos los logs encolados en el búfer asíncrono.

---

## 5. Hoja de Ruta de Remediación para FASE 2

1. **Refactorización de `OverlayGuard`:**
   - Montar el overlayfs **únicamente** sobre el subdirectorio `masterdir/` o utilizar `xbps-uchroot` / `bwrap`.
   - Copiar las plantillas VUR dentro del directorio `upper/srcpkgs/<pkg>` para mantener `lower` inmutable.
   - Reemplazar `cp -aT merged_binpkgs target_binpkgs` por copia selectiva de archivos `.xbps` seguida de indexación con `xbps-rindex` protegida por un mutex global de repositorio.
2. **Implementación de R1 (Scheduler por Olas Topológicas):**
   - Implementar partición del DAG en olas independientes (nodos con in-degree 0 que pueden construirse en paralelo).
   - Utilizar un pool de hilos (`std::thread` o `rayon`) acotado por `max_concurrent_builds`.
3. **Corrección de Lockfile:**
   - Mover `lock::acquire` después de `config.parse_args()`.
   - No bloquear en operaciones de lectura.
   - Eliminar el texto que sugiere borrar `vary.lock`.
4. **Cierre de Ventanas de Carrera en Señales:**
   - Usar un pipe o canal de sincronización para asegurar que `register_child` se complete atómicamente antes de permitir que el observador consulte la lista de hijos.
   - Asegurar que `MountCleanup` elimine los directorios temporales en el manejador de salida del observador antes de invocar `exit()`.
5. **Flush Forzado de Logs en Señal:**
   - Exponer un método de flush síncrono en `logging` o permitir que el observador acceda a una referencia de vaciado antes de invocar `exit()`.
