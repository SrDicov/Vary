# Reporte de Auditoría de Seguridad — Subagente SA-1: Seguridad

**Proyecto:** `vary` (Void User Repository helper)  
**Fecha:** 2026-09-06  
**Auditor:** Subagente SA-1 (Seguridad — Prioridad Máxima)  
**Alcance:** Base de código completa en `/run/media/dicov/LudoDrive/dicov-op/Vary`  
**Rama:** `vary-mvp` @ `1e32f1f` (+ working tree sin commitear)  

---

## 1. Resumen Ejecutivo

Durante la auditoría exhaustiva de seguridad se analizaron todas las superficies de ataque del sistema, con especial énfasis en los privilegios de ejecución (superusuario/root), la interacción con utilidades de bajo nivel (`xbps-install`, `xbps-remove`, `xbps-query`, `xbps-src`, `git`, `install`, `cp`, `ln`), la manipulación del sistema de archivos en `/etc/xbps.d/` y `/tmp`, el protocolo criptográfico TOFU (Trust-On-First-Use) para llaves RSA y el control de robustez ante entradas hostiles o malformadas.

Se identificaron **14 nuevos hallazgos** (desde `H-002` hasta `H-015`), clasificados de acuerdo a su severidad técnica:

| Severidad | Cantidad | IDs de Hallazgos |
|---|:---:|---|
| **CRITICAL** | 2 | [H-004], [H-006] |
| **HIGH** | 6 | [H-002], [H-003], [H-005], [H-007], [H-008], [H-010], [H-013] |
| **MEDIUM** | 3 | [H-009], [H-012], [H-014] |
| **LOW / INFO** | 2 | [H-011], [H-015] |

*(Nota: [H-001] corresponde a la vulnerabilidad de auto-aprobación por EOF en stdin, ya subsanada previamente en `audit/FIX_LOG.md`).*

---

## 2. Auditoría Detallada por Puntos de Control

### Punto 1: Inyección de argumentos
- **Invocaciones a `xbps-*`:** En `src/xbps.rs:351-355` (`install`) y `src/xbps.rs:367-371` (`remove_recursive`), los objetivos (`targets`) se pasan a `Command::args` directamente después de las banderas, sin intercalar el delimitador `--`.
- **Invocaciones de consulta:** En `src/install.rs:81-83` (`official_exists_remote`) y en `src/info.rs:23-25`, `xbps-query` recibe el nombre del paquete como último argumento posicional sin `--`.
- **Validación de nombres:** En `src/command_line.rs:215-217`, cualquier argumento tras `--` en la CLI es volcado a `self.targets` sin validar el formato con regex. No se aplica `^[a-zA-Z0-9][a-zA-Z0-9._+-]*$` a los nombres de paquetes ingresados por el usuario. Si un paquete o argumento empieza con `-` (ej. `-y`, `-f`, `--repository=...`), se inyecta como opción en `xbps-install` ejecutado con privilegios escalados.

### Punto 2: Escritura en `/etc/xbps.d/`
- **Control de nombre de archivo:** En `src/keys.rs:22-28`, `repo_conf_path(name)` construye `/etc/xbps.d/20-vur-{name}.conf` y `key_dest_path(name)` construye `/etc/xbps.d/keys/vary-vur-{name}.pem`. El parámetro `name` no está sanitizado contra secuencias de path traversal (`/`, `..`). Un repositorio malicioso registrado con un nombre que contenga secuencias relativas o absolutas puede provocar la sobrescritura de archivos arbitrarios del sistema, incluyendo repositorios oficiales (`00-repository-main.conf`).
- **Contenido del archivo:** En `src/keys.rs:117` y `187`, las líneas `repository={url}` se escriben directamente sin verificar que el esquema sea exclusivamente `https://` o `file://`. No se comprueba la presencia de caracteres de salto de línea (`\n`, `\r`), lo que permite la inyección de directivas XBPS arbitrarias.

### Punto 3: TOFU de claves RSA y gestión criptográfica
- **Registro inicial:** En `src/repo.rs:101-114`, `vary --repo add` guarda la entrada en `repos.conf` con `key_fingerprint: None`. No solicita confirmación al usuario para anclar (pin) el fingerprint de la clave en el momento del registro.
- **Validación y rotación:** En `src/keys.rs:86-108`, si `key_fingerprint` es `None` y se usa `--noconfirm` (`-y`), `setup_binary_repo` auto-aprueba cualquier llave encontrada en el repositorio git o plist. No comprueba si ya existía una llave previa instalada en `/etc/xbps.d/keys/vary-vur-{name}.pem`. Si la clave remota cambia (vector "Atomic Arch"), el sistema no falla cerrado: reemplaza la llave silenciosamente en el sistema.

### Punto 4: `git clone` de URLs arbitrarias
- **Opciones maliciosas en Git:** En `src/vur_client.rs:62-73` y `src/vur_client.rs:91-95`, `git clone` y `git ls-remote` reciben `&self.entry.url` sin anteponer `--`. Si una URL comienza con `-` (ej. `--upload-pack=<payload>`), Git interpreta la URL como una bandera de opciones en lugar del repositorio de origen. La bandera `--upload-pack` de Git ejecuta el comando indicado en la máquina local.
- **Protocolos inseguros:** No se deshabilitan transportes peligrosos como `protocol.ext` (`ext::sh -c ...`). No se pasa `-c protocol.ext.allow=never -c protocol.file.allow=user`.

### Punto 5: TOCTOU y directorios temporales
- **Archivos estáticos en `/tmp`:** En `src/keys.rs:37-41`, `write_root_file` genera un archivo predecible `/tmp/vary-{PID}.tmp` utilizando `std::env::temp_dir()`. No utiliza `tempfile::NamedTempFile` atómico. Esto expone al proceso a ataques de enlaces simbólicos (symlink attack) y colisiones predecibles antes de invocar `install` con privilegios de root (vulnerabilidad clásica de `vura`).
- **Permisos de directorios:** En `src/cache.rs`, `src/db.rs`, `src/lock.rs`, `src/logging.rs`, y `src/repo.rs`, las llamadas a `std::fs::create_dir_all` se realizan sin forzar permisos Unix `0700` (`0o700`). En sistemas compartidos multi-usuario, otros usuarios locales pueden inspeccionar `vary.log`, la base de datos de paquetes y las cachés.

### Punto 6: Elevación de privilegios (`src/elevate.rs`)
- **Wrapper de elevación:** `src/elevate.rs` implementa una resolución agnóstica (`sudo`, `doas`, `run0`). No pasa credenciales por stdin (hereda el TTY interactivo).
- **Hardcode residual de `sudo`:** En `src/init.rs:15-25`, se invoca `Command::new("sudo")` de forma hardcodeada, ignorando `config.sudo_bin`, `config.sudo_flags` y el estado de ejecución como root.
- **Entorno no saneado:** `src/elevate.rs` no limpia variables de entorno potencialmente peligrosas (`LD_PRELOAD`, `LD_LIBRARY_PATH`, `PATH`) antes de despachar el comando elevado.
- **Comportamiento si binario falta:** Si el usuario configura un `sudo_bin` inexistente en PATH, el mensaje de error emitido confunde al usuario alegando que no se encuentra en Void Linux o no tiene XBPS instalado.

### Punto 7: Fuzzing mental de entradas y robustez
- **Pánicos por `unwrap()`:** Se identificaron llamadas críticas a `.unwrap()` en `src/command_line.rs:313-326` cuando un flag que requiere argumento se pasa al final de argv (ej. `vary -S pkg --sudo`), provocando un crash inmediato.
- **Pánicos en `install.rs`:** `repos_conf.vur.get(&repo.name).unwrap()` en `src/install.rs:156` y `367` produce pánico si la colección en memoria se desincroniza.
- **Destrucción de DB en `src/db.rs`:** `InstalledDb::load` ejecuta `std::fs::remove_file(&path)` si encuentra un archivo previo, destruyendo bases de datos `installed.json` existentes.
- **Bugs en parser Bash:** `src/vur_client.rs:592-601` presenta una condición estática con `quote_count % 2 == 1` que no recalcula el conteo en líneas subsecuentes, pudiendo corromper la extracción de metadatos.

### Punto 8: Inventario de confirmaciones / stdin
- **Estado de H-001:** `confirm_from_reader` y `ask_from_reader` en `src/util.rs` verifican estrictamente `n == 0` (EOF) y errores de I/O, devolviendo denegación estricta (`Ok(false)` / `false`). Esto resolvió con éxito el fallo de auto-aprobación en no-TTY.
- **Falta de feedback en no-TTY:** Al abortar en un entorno sin terminal interactivo (`!stdout().is_terminal()`), el proceso termina silenciosamente con código 1 sin emitir un mensaje de diagnóstico al usuario sugiriendo el uso de `--noconfirm`.

---

## 3. Registro Detallado de Hallazgos

### [H-002] Severidad: High
**Módulo:** `src/xbps.rs:351-355`, `src/xbps.rs:367-371`, `src/install.rs:81-83`, `src/info.rs:23-25`  
**Título:** Inyección de argumentos en comandos XBPS por ausencia del delimitador de opciones `--`  
**Evidencia:**  
```rust
// src/xbps.rs:351-355
pub fn install(
    targets: &[String],
    extra_flags: &[String],
    sudo_bin: &str,
    sudo_flags: &[String],
) -> Result<i32> {
    let mut cmd = crate::elevate::elevate(sudo_bin, sudo_flags, "xbps-install")?;
    cmd.arg("-S")
        .args(extra_flags)
        .args(targets); // <-- Falta "--" antes de targets
    status_code(&mut cmd)
}

// src/xbps.rs:367-371
pub fn remove_recursive(
    targets: &[String],
    extra_flags: &[String],
    sudo_bin: &str,
    sudo_flags: &[String],
) -> Result<i32> {
    let mut cmd = crate::elevate::elevate(sudo_bin, sudo_flags, "xbps-remove")?;
    cmd.arg("-Ro")
        .args(extra_flags)
        .args(targets); // <-- Falta "--" antes de targets
    status_code(&mut cmd)
}
```
**Impacto:** Si un nombre de paquete objetivo inicia con un guión (ej. `-f`, `--repository=http://evil.com`, `-y`), `xbps-install` o `xbps-remove` lo interpretan como un flag del programa y no como un paquete. Al correr con privilegios de root mediante el wrapper de elevación, esto permite alterar el comportamiento de la instalación o forzar repositorios no confiables.  
**Fix propuesto:** Intercalar `.arg("--")` antes de agregar `targets` en `install`, `remove_recursive`, y en las consultas de `xbps-query`.  
```rust
cmd.arg("-S")
    .args(extra_flags)
    .arg("--")
    .args(targets);
```
**Validación:** `cargo test` y verificar con `vary -S -- -f` que `xbps-install` rechace `-f` como nombre de paquete inválido en lugar de activar el flag force.  
**Estado:** PENDIENTE

---

### [H-003] Severidad: High
**Módulo:** `src/command_line.rs:215-217`, `src/metadata.rs:101-124`  
**Título:** Falta de validación estricta con regex para nombres de paquetes y omisión de validación en subpaquetes  
**Evidencia:**  
```rust
// src/command_line.rs:215-217
if arg == "-" || *end_of_ops {
    self.targets.push(arg.to_string());
    return Ok(false);
}

// src/metadata.rs:101-114
for (i, sub) in self.subpackages.iter().enumerate() {
    if sub.pkgname.trim().is_empty() {
        bail!("subpackages[{}].pkgname vacío: el nombre del subpaquete es obligatorio", i);
    }
    if sub.pkgname == self.pkgname {
        bail!("subpackages[{}].pkgname duplicado: '{}' coincide con el pkgname del padre", i, sub.pkgname);
    }
    // NOTA: Nunca se invoca is_valid_pkgname(&sub.pkgname)
}
```
**Impacto:** La CLI acepta cualquier cadena arbitraria en `self.targets` sin comprobar que cumpla el patrón de paquetes de Void Linux (`^[a-zA-Z0-9][a-zA-Z0-9._+-]*$`). Adicionalmente, el validador de `.VURINFO` verifica `is_valid_pkgname` en el paquete principal, pero omite la validación sobre `subpackages[].pkgname`. Un repositorio hostil puede definir un subpaquete llamado `-f` o `../../bad` que pasará desapercibido hasta llegar a herramientas del sistema.  
**Fix propuesto:**
1. Crear una función pública `is_valid_package_name(name: &str) -> bool` con la regex/regla estricta `^[a-zA-Z0-9][a-zA-Z0-9._+-]*$`.
2. Validar cada target en `command_line.rs` y rechazar argumentos malformados antes de iniciar el pipeline.
3. Invocar `is_valid_pkgname(&sub.pkgname)` dentro del bucle de subpaquetes en `metadata.rs`.  
**Validación:** Crear test unitario `test_rejects_subpackage_with_invalid_name` y verificar rechazo en CLI.  
**Estado:** PENDIENTE

---

### [H-004] Severidad: Critical
**Módulo:** `src/keys.rs:22-28`, `src/repo.rs:33-39`  
**Título:** Path traversal en nombres de repositorio VUR permitiendo sobrescritura arbitraria en `/etc/xbps.d/`  
**Evidencia:**  
```rust
// src/keys.rs:22-28
pub fn repo_conf_path(name: &str) -> String {
    format!("/etc/xbps.d/20-vur-{}.conf", name)
}

pub fn key_dest_path(name: &str) -> String {
    format!("{}/vary-vur-{}.pem", keys_dir(), name)
}

// src/repo.rs:33-39
let name = name_opt
    .map(|s| s.to_string())
    .unwrap_or_else(|| derive_name_from_url(url));

if name.is_empty() {
    bail!("could not derive repo name from url; please provide a name");
}
// NOTA: name no se sanitiza contra caracteres '/' o '..'
```
**Impacto:** El nombre del repositorio puede ser provisto por el usuario o inferido de la URL (`vary --repo add <url> <name>`). Si el nombre contiene secuencias como `/../../00-repository-main`, `repo_conf_path` resuelve a `/etc/xbps.d/20-vur-/../../00-repository-main.conf` -> `/00-repository-main.conf`. Al invocarse `write_root_file` con privilegios de root, sobrescribe el archivo de configuración de los repositorios oficiales de Void Linux o cualquier otro archivo en el disco, permitiendo denegación de servicio del sistema gestor de paquetes o escalada de privilegios.  
**Fix propuesto:**
1. Validar estrictamente el nombre del repositorio contra la regex `^[a-zA-Z0-9_-]+$`.
2. Rechazar nombres que contengan `/`, `\`, `..`, o nombres reservados como `main`, `default`, `10-vary`.
3. Sanitizar y asegurar que la ruta canónica generada permanezca dentro de `/etc/xbps.d/` y no apunte a archivos reservados.  
**Validación:** Test unitario `test_repo_name_path_traversal_rejected` verificando rechazo de `../../evil`.  
**Estado:** PENDIENTE

---

### [H-005] Severidad: High
**Módulo:** `src/keys.rs:117`, `src/keys.rs:185-189`, `src/vup_index.rs:175`  
**Título:** Inyección de directivas XBPS arbitrarias y falta de validación de esquema en URLs de repositorios binarios  
**Evidencia:**  
```rust
// src/keys.rs:117-118
let conf = format!("repository={}\n", binary_url);
write_root_file(&conf, &repo_conf_path(&repo.name), "644", sudo_bin, sudo_flags)?;

// src/keys.rs:185-189
let mut conf = String::new();
for u in &urls {
    conf.push_str(&format!("repository={}\n", u));
}
write_root_file(&conf, &repo_conf_path(name), "644", sudo_bin, sudo_flags)?;
```
**Impacto:** Las URLs binarias (provenientes de `repos.conf` o descargadas desde un `index.json` remoto de VUP) no se validan antes de escribirse en un archivo de configuración del sistema (`/etc/xbps.d/*.conf`). Si una URL contiene saltos de línea (`\n` o `\r`), puede inyectar directivas arbitrarias de configuración de XBPS. Asimismo, se permiten esquemas no seguros como `http://` plano (sin cifrado) o esquemas no válidos, exponiendo al usuario a ataques de interceptación y envenenamiento de red.  
**Fix propuesto:**
1. Validar que toda URL de repositorio binario comience estrictamente con `https://` (o `file://` para pruebas locales).
2. Validar que la cadena no contenga caracteres de control o saltos de línea (`\n`, `\r`, `\0`).  
**Validación:** Test unitario `test_reject_binary_url_with_newline_or_bad_scheme`.  
**Estado:** PENDIENTE

---

### [H-006] Severidad: Critical
**Módulo:** `src/repo.rs:101-114`, `src/keys.rs:86-108`, `src/install.rs:364-382`  
**Título:** Falla de TOFU y reemplazo silencioso de llaves RSA en repositorios binarios (Vector "Atomic Arch")  
**Evidencia:**  
```rust
// src/repo.rs:101-105
let mut conf = ReposConf::load(config.repos_conf_path()).unwrap_or_default();
conf.vur.insert(name.clone(), entry); // key_fingerprint es None
conf.save(config.repos_conf_path())?;

// src/keys.rs:86-108
let fp = VurRepo::fingerprint_sha256(&key_path)?;
if let Some(expected) = entry.key_fingerprint.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
    // Solo verifica si repos.conf tiene key_fingerprint no vacío
} else {
    tracing::warn!("el VUR '{}' no declara key_fingerprint en repos.conf; verifica visualmente", repo.name);
}
// Si no_confirm es true, confirm() devuelve true automáticamente
if !confirm("¿Confiás en esta llave y deseas registrar este repositorio binario?", no_confirm)? {
    bail!("...");
}
// Escribe la llave en /etc/xbps.d/keys/ sin verificar si la llave instalada previamente era distinta
write_root_file(&pem, &dest_key, "644", sudo_bin, sudo_flags)?;
```
**Impacto:** Cuando un usuario agrega un repositorio (`vary --repo add`), `key_fingerprint` no se ancla automáticamente en `repos.conf`. En subsecuentes ejecuciones con `--noconfirm` (común en scripts y actualizaciones automáticas), si el repositorio remoto actualiza su llave pública (por ejemplo, tras un compromiso del repositorio git del autor, como ocurrió en el incidente "Atomic Arch" de junio 2026), `vary` acepta y sobrescribe la nueva llave sin advertir que el fingerprint cambió respecto a la llave previamente de confianza. No falla cerrado.  
**Fix propuesto:**
1. Implementar TOFU estricto: al configurar una llave por primera vez, guardar su fingerprint calculado en `repos.conf`.
2. Si `/etc/xbps.d/keys/vary-vur-{name}.pem` ya existe en el sistema, calcular su fingerprint y compararlo con la llave recibida.
3. Si la llave difiere del fingerprint previamente registrado o anclado, **abortar inmediatamente (fail-closed)** con un error crítico requiriendo intervención manual explícita (`vary --repo rekey <name>`), incluso si `--noconfirm` está activo.  
**Validación:** Test unitario simulando cambio de llave en repo sin rekey y comprobando que falle cerrado.  
**Estado:** PENDIENTE

---

### [H-007] Severidad: High
**Módulo:** `src/vur_client.rs:62-73`, `src/vur_client.rs:91-95`  
**Título:** Inyección de argumentos (`--upload-pack`) y ejecución arbitraria mediante transportes peligrosos en `git clone`  
**Evidencia:**  
```rust
// src/vur_client.rs:62-73
let branch = self.entry.branch_or_default();
let output = Command::new(&self.git_bin)
    .args([
        "clone",
        "--filter=blob:none",
        "--no-checkout",
        "--depth", "1",
        "--branch", branch,
    ])
    .arg(&self.entry.url) // <-- Sin "--" antes de la URL
    .arg(&self.path)
    .output()
```
**Impacto:**
1. Si `self.entry.url` comienza con un guión (ej. `--upload-pack=touch /tmp/pwned`), `git clone` y `git ls-remote` interpretan la URL como una bandera de opciones en lugar del repositorio de origen. La bandera `--upload-pack` de Git ejecuta el comando indicado en la máquina local.
2. Git por defecto permite transportes como `ext::` (ej. `ext::sh -c <comando>`), que permiten la ejecución arbitraria de código al clonar repositorios no auditados.  
**Fix propuesto:**
1. Anteponer `.arg("--")` antes de pasar la URL y la ruta de destino a `git clone` y `git ls-remote`.
2. Pasar `-c protocol.ext.allow=never -c protocol.file.allow=user` a todas las invocaciones de Git CLI.
3. Validar que el esquema de la URL de repositorios sea estrictamente `https://`, `git://` o `ssh://` (o `file://` validado).  
**Validación:** Test unitario `test_git_clone_rejects_option_url`.  
**Estado:** PENDIENTE

---

### [H-008] Severidad: High
**Módulo:** `src/keys.rs:37-41`  
**Título:** Condición de carrera (TOCTOU) y archivo temporal estático y predecible en `/tmp` al escribir archivos del sistema  
**Evidencia:**  
```rust
// src/keys.rs:37-48
let tmp = std::env::temp_dir().join(format!(
    "vary-{}.tmp",
    std::process::id()
));
std::fs::write(&tmp, contents).context("escribiendo archivo temporal")?;
let status = crate::elevate::elevate(sudo_bin, sudo_flags, "install")?
    .args(["-m", mode])
    .arg(&tmp)
    .arg(dest)
    .status()
    .context("elevando privilegios para escribir archivo del sistema")?;
let _ = std::fs::remove_file(&tmp);
```
**Impacto:** `std::env::temp_dir()` resuelve al directorio compartido `/tmp`. La ruta `/tmp/vary-{PID}.tmp` es completamente predecible. Un usuario no privilegiado en la misma máquina puede crear un enlace simbólico o un archivo pre-creado en `/tmp/vary-{PID}.tmp` apuntando a un archivo sensible del sistema. Cuando `vary` invoca `elevate ... install -m mode /tmp/vary-{PID}.tmp {dest}`, el binario `install` ejecutado como root lee o sobrescribe el destino basado en el archivo del atacante o aprovecha la ventana entre `std::fs::write` y el spawn del subproceso. Esta es la vulnerabilidad histórica documentada en `vura`.  
**Fix propuesto:** Reemplazar el archivo predecible por `tempfile::Builder::new().prefix("vary-").tempfile_in(config_or_cache_dir)` en un directorio privado con permisos `0700` propiedad del usuario actual, o usar un `NamedTempFile` con permisos restringidos de apertura segura (`O_CREAT | O_EXCL`).  
**Validación:** Test de verificación de creación de temporales en directorios privados sin rutas estáticas en `/tmp`.  
**Estado:** PENDIENTE

---

### [H-009] Severidad: Medium
**Módulo:** `src/cache.rs:31`, `src/db.rs:38`, `src/lock.rs:21`, `src/logging.rs:52`, `src/repo.rs:62`  
**Título:** Creación de directorios de caché, base de datos, lock y logs sin restricción de permisos Unix (ausencia de 0700)  
**Evidencia:**  
```rust
// src/cache.rs:31
std::fs::create_dir_all(&path)?;

// src/db.rs:38
std::fs::create_dir_all(&path)?;

// src/logging.rs:52
let _ = std::fs::create_dir_all(cache_dir);

// src/repo.rs:62
std::fs::create_dir_all(&vurs_dir).context("creating vurs dir")?;
```
**Impacto:** `create_dir_all` respeta la umask del usuario. En configuraciones típicas (`0022`), los directorios son creados con permisos `0755` (lectura y ejecución para cualquier usuario del sistema). En entornos multi-usuario, esto permite que usuarios locales no autorizados lean `vary.log` (que puede contener rutas privadas, trazas y nombres de dependencias), la base de datos de paquetes y las claves temporales.  
**Fix propuesto:** En plataformas Unix (`#[cfg(unix)]`), utilizar `std::os::unix::fs::DirBuilderExt` para aplicar modo `0o700` (`rwx------`) al crear los directorios base de vary (`~/.cache/vary`, `~/.local/share/vary`, `~/.config/vary`).  
**Validación:** Test unitario comprobando que `metadata(&dir).permissions().mode() & 0o777 == 0o700`.  
**Estado:** PENDIENTE

---

### [H-010] Severidad: High
**Módulo:** `src/init.rs:15-25`  
**Título:** Bypass del wrapper agnóstico de elevación y ejecución descontrolada con hardcode de `sudo` en post-install hook  
**Evidencia:**  
```rust
// src/init.rs:14-26
if Path::new("/dev/dinitctl").exists() {
    if no_confirm || crate::util::confirm(&format!("Enable and start dinit service for {}?", pkg_name), no_confirm)? {
        let _ = Command::new("sudo").args(["dinitctl", "enable", pkg_name]).status();
        let _ = Command::new("sudo").args(["dinitctl", "start", pkg_name]).status();
    }
} else if Path::new("/run/runit").exists() {
    if no_confirm || crate::util::confirm(&format!("Enable runit service for {}?", pkg_name), no_confirm)? {
        let service_link = Path::new("/var/service").join(pkg_name);
        if !service_link.exists() {
            let _ = Command::new("sudo")
                .args(["ln", "-s", sv_dir.to_str().unwrap(), service_link.to_str().unwrap()])
                .status();
        }
    }
}
```
**Impacto:**
1. Se invoca directamente `Command::new("sudo")` en lugar de pasar por `crate::elevate::elevate()`. En sistemas donde se utiliza `doas`, `run0` o el usuario ejecuta `vary` como root, estas operaciones fallan o solicitan contraseñas redundantes e incompatibles.
2. `ln -s` y `dinitctl` no usan `--` antes de `pkg_name`. Si el paquete tuviera un nombre con guiones o banderas, altera el comportamiento del gestor de init.
3. Se usan llamadas directas a `.unwrap()` en `to_str()`, que provocan pánicos si los paths contienen bytes no-UTF8.  
**Fix propuesto:**
1. Recibir `sudo_bin` y `sudo_flags` en `post_install_hook` y utilizar `crate::elevate::elevate()`.
2. Anteponer `--` antes de los argumentos posicionales.
3. Reemplazar `.unwrap()` por manejo seguro de rutas.  
**Validación:** Test unitario verificando que el hook use el wrapper de elevación configurado.  
**Estado:** PENDIENTE

---

### [H-011] Severidad: Low
**Módulo:** `src/elevate.rs:60-86`, `src/xbps.rs:65-71`  
**Título:** Falta de sanitización de entorno de ejecución en comandos elevados y mensaje de error engañoso si `sudo_bin` no existe  
**Evidencia:**  
```rust
// src/elevate.rs:63-74
if !configured_bin.is_empty() {
    return Ok(Some((
        configured_bin.to_string(),
        configured_flags.to_vec(),
    )));
}
// src/elevate.rs:80-84
Some((bin, flags)) => {
    let mut cmd = Command::new(&bin);
    cmd.args(flags).arg(program);
    cmd
}
```
**Impacto:**
1. El comando elevado hereda todas las variables de entorno del llamador sin limpiar variables críticas como `LD_PRELOAD`, `LD_LIBRARY_PATH`. Si bien `sudo` suele aplicar `env_reset`, otros wrappers (`doas`, `run0` o scripts personalizados) pueden no hacerlo de forma predeterminada.
2. Si el usuario configura `--sudo /ruta/inexistente`, `resolve()` no valida su existencia en PATH. Cuando `cmd.status()` falla, `spawn_error` emite el mensaje confuso: "`<bin>` no encontrado: ¿estás en Void Linux? ¿instalaste xbps?".  
**Fix propuesto:**
1. Limpiar variables de entorno peligrosas en `elevate()` o documentar el requerimiento para wrappers no-sudo.
2. Validar que `configured_bin` sea ejecutable antes de retornarlo, emitiendo un mensaje claro indicando que el wrapper configurado no existe.  
**Validación:** Ejecutar `vary --sudo /bin/false_nonexistent -S` y verificar mensaje claro de error.  
**Estado:** PENDIENTE

---

### [H-012] Severidad: Medium
**Módulo:** `src/command_line.rs:313-326`  
**Título:** Pánico incontrolado (`panic!`) en parsing de CLI al invocar flags con valor faltante al final de los argumentos  
**Evidencia:**  
```rust
// src/command_line.rs:312-326
Arg::Long("arch") => {
    let v = value.unwrap(); // <-- PANIC si es None
    ...
}
Arg::Long("sudo") => self.sudo_bin = value.unwrap().to_string(), // <-- PANIC si es None
Arg::Long("sudoflags") => self.sudo_flags.extend(value.unwrap().split_whitespace().map(|s| s.to_string())),
Arg::Long("git") => self.git_bin = value.unwrap().to_string(),
Arg::Long("curl") => self.curl_bin = value.unwrap().to_string(),
```
**Impacto:** Si un usuario ingresa una bandera que requiere valor al final de la línea de comandos (ej. `vary -S pkg --sudo` o `vary --arch`), `value` es `None`. Las llamadas a `value.unwrap()` provocan un crash inmediato por pánico de Rust (`called Option::unwrap() on a None value`) en lugar de salir limpiamente con un error de sintaxis y código de retorno apropiado.  
**Fix propuesto:** Reemplazar `.unwrap()` por `value.ok_or_else(|| anyhow!("la opción '--{flag}' requiere un argumento"))?`.  
**Validación:** `cargo test` y probar `vary -S pkg --sudo` verificando que devuelva error en lugar de pánico.  
**Estado:** PENDIENTE

---

### [H-013] Severidad: High
**Módulo:** `src/install.rs:156`, `src/install.rs:367`, `src/db.rs:35-37`  
**Título:** Pánicos por desincronización de repositorios en `install.rs` y destrucción silenciosa de la base de datos previa en `db.rs`  
**Evidencia:**  
```rust
// src/install.rs:156 y 367
let entry = repos_conf.vur.get(&repo.name).unwrap(); // <-- PANIC si falta la clave
...
let entry = repos_conf.vur.get(repo).unwrap(); // <-- PANIC si falta la clave

// src/db.rs:35-37
if path.is_file() {
    let _ = std::fs::remove_file(&path); // <-- BORRADO SILENCIOSO DE DB PREVIA
}
std::fs::create_dir_all(&path)?;
let env = unsafe { EnvOpenOptions::new().open(&path)? };
```
**Impacto:**
1. Las llamadas a `.unwrap()` en `install.rs` causan pánicos durante el flujo de resolución e instalación si hay desincronización entre la lista de repositorios y el archivo `repos.conf`.
2. En `src/db.rs`, la migración arbitraria hacia LMDB (`heed`) verifica si la ruta configurada (`~/.cache/vary/installed.json`) es un archivo, y si lo es, lo **elimina sin confirmación ni migración** con `remove_file(&path)` para crear un directorio con el mismo nombre. Esto provoca la pérdida total del historial de paquetes VUR previamente instalados en el sistema del usuario.  
**Fix propuesto:**
1. Usar `repos_conf.vur.get(...).ok_or_else(...)` en lugar de `.unwrap()`.
2. Implementar migración controlada del formato JSON a la nueva estructura de datos, o conservar `installed.json` como archivo JSON atómico preservando la compatibilidad hacia atrás.  
**Validación:** Test unitario de persistencia de `installed.json` evitando la eliminación accidental.  
**Estado:** PENDIENTE

---

### [H-014] Severidad: Medium
**Módulo:** `src/vur_client.rs:592-601`  
**Título:** Lógica defectuosa en el parser de templates Bash ante valores multilínea y comillas desbalanceadas  
**Evidencia:**  
```rust
// src/vur_client.rs:592-601
let quote_count = buf.matches('"').count();
while quote_count % 2 == 1 {
    if let Some(next) = lines.next() {
        buf.push('\n');
        buf.push_str(next);
        if next.contains('"') { break; } // <-- Si next contiene número par de comillas, buf sigue desbalanceado
    } else {
        break;
    }
}
```
**Impacto:** `quote_count` solo se evalúa al inicio de la línea. Dentro del bucle `while quote_count % 2 == 1`, `quote_count` **nunca se actualiza** (advertencia detectada por clippy como `while_immutable_condition`). Si la siguiente línea contiene comillas que no cierran la cadena o contiene un número par de comillas internas, el bucle se interrumpe prematuramente dejando una cadena truncada o corrompiendo las variables subsiguientes del template. Además, ignora comillas simples (`'`).  
**Fix propuesto:** Recalcular dinámicamente el estado de apertura/cierre de comillas simples y dobles mientras se itera sobre los caracteres de las líneas acumuladas.  
**Validación:** Test unitario con un template que contenga strings multilínea con comillas mezcladas.  
**Estado:** PENDIENTE

---

### [H-015] Severidad: Low
**Módulo:** `src/install.rs:295-298`, `src/util.rs:41-53`  
**Título:** Falta de mensaje de error diagnóstico al abortar por EOF en entornos no interactivos (no-TTY)  
**Evidencia:**  
```rust
// src/install.rs:295-297
if !confirm("Proceed with installation?", config.no_confirm)? {
    return Ok(1);
}
```
**Impacto:** Tras la corrección de H-001, `confirm_from_reader` retorna `Ok(false)` al recibir EOF en stdin. Sin embargo, en un entorno de integración continua, pipe o subshell desatendido sin `--noconfirm`, vary se detiene y sale con código 1 sin imprimir ninguna explicación, lo que dificulta la depuración de pipelines automatizados.  
**Fix propuesto:** Si `!stdout().is_terminal()` o si `confirm()` detecta EOF, emitir un mensaje de orientación: `vary: entrada no interactiva detectada sin confirmación; use --noconfirm para ejecución desatendida`.  
**Validación:** Ejecutar `echo "" | vary -S pkg` y verificar que imprima el hint diagnóstico.  
**Estado:** PENDIENTE

---

## 4. Matriz de Riesgo y Priorización de Remediaciones

| Prioridad | ID | Módulo | Riesgo Principal | Complejidad de Corrección |
|---|---|---|---|---|
| **P0 (Inmediata)** | [H-004] | `src/keys.rs`, `src/repo.rs` | Path traversal y sobrescritura de `/etc/xbps.d/` | Baja (validación regex y ruta) |
| **P0 (Inmediata)** | [H-006] | `src/keys.rs`, `src/repo.rs` | Envenenamiento de llaves criptográficas (Vector "Atomic Arch") | Media (TOFU persistente y fail-closed) |
| **P1 (Alta)** | [H-002] | `src/xbps.rs`, `src/install.rs` | Inyección de argumentos en comandos root | Baja (agregar `--`) |
| **P1 (Alta)** | [H-007] | `src/vur_client.rs` | Ejecución de comandos vía Git (`--upload-pack`, `ext::`) | Baja (agregar `--` y flags Git) |
| **P1 (Alta)** | [H-008] | `src/keys.rs` | Condición de carrera / symlink attack en `/tmp` | Baja (usar directorio seguro o `NamedTempFile`) |
| **P1 (Alta)** | [H-003] | `src/command_line.rs`, `src/metadata.rs` | Nombres de paquetes no validados | Baja (regex XBPS estricta) |
| **P1 (Alta)** | [H-005] | `src/keys.rs`, `src/vup_index.rs` | Inyección de directivas XBPS en `.conf` | Baja (validar https y rechazar newlines) |
| **P1 (Alta)** | [H-010] | `src/init.rs` | Hardcode de `sudo` y falta de wrapper agnóstico | Baja (usar `crate::elevate`) |
| **P1 (Alta)** | [H-013] | `src/install.rs`, `src/db.rs` | Pánicos en instalación y pérdida de datos en DB | Media (reemplazar unwrap y migrar DB) |
| **P2 (Media)** | [H-009] | Múltiples módulos | Exposición de archivos en sistemas multi-usuario | Baja (aplicar `0o700`) |
| **P2 (Media)** | [H-012] | `src/command_line.rs` | Crash por pánico en flags CLI | Baja (reemplazar `.unwrap()`) |
| **P2 (Media)** | [H-014] | `src/vur_client.rs` | Bucle anómalo y corrupción en parser de templates | Media (recalcular quotes) |
| **P3 (Baja)** | [H-011] | `src/elevate.rs` | Diagnóstico confuso en error de wrapper | Baja (validar existencia previa) |
| **P3 (Baja)** | [H-015] | `src/install.rs` | Falta de hint diagnóstico en no-TTY | Baja (imprimir advertencia) |

---
**Fin del Reporte SA-1.**
