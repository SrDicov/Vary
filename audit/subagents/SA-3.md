# SA-3 — Reporte de Auditoría: Datos, Persistencia, Formatos y Parsers

**Subagente:** SA-3 (Datos y Parsing)  
**Fecha:** 2026-09-06  
**Rama / Commit Base:** `vary-mvp` @ `1e32f1f` (+ cambios no commiteados auditados)  
**Estado:** Finalizado  

---

## 1. Resumen Ejecutivo

Este reporte constituye la auditoría técnica exhaustiva sobre el subsistema de almacenamiento de datos, persistencia, formatos de metadatos y parsers de cadenas en el proyecto `vary`.

Se identificaron deficiencias críticas que comprometen la integridad de datos del usuario (pérdida silenciosa de registros previos de instalación), fragilidades algorítmicas en el parser de scripts de shell de plantillas Void (`src/vur_client.rs`), inconsistencias en el consumo de salida de utilitarios de sistema (`xbps-query`) y divergencias entre la especificación contractual y la implementación del esquema `.VURINFO`.

A continuación, se detalla la matriz de hallazgos encontrados por este subagente:

| ID | Severidad | Módulo | Descripción Resumida |
|---|---|---|---|
| **[H-002]** | 🔴 **CRITICAL** | `src/db.rs:35-37` | **Pérdida de datos:** `InstalledDb::load` destruye incondicionalmente cualquier archivo `installed.json` previo sin migrar. |
| **[H-003]** | 🟡 **MEDIUM** | `src/db.rs:18-24`<br>`src/upgrade.rs:90-97` | **Deficiencia contractual:** Campo fantasma `install_date` nunca leído, ausencia de `build_date` (ms) e inexistencia de recompilación preventiva. |
| **[H-004]** | 🟢 **LOW** | `src/db.rs:27,51,68,83` | **Higiene de código:** Campo muerto `path`, métodos muertos `get()` y `names()` (advertencias del compilador) y stub `save()` no-op. |
| **[H-005]** | 🟡 **MEDIUM** | `src/xbps.rs:264-282`<br>`src/install.rs:229-230` | **Orden de catálogo y sintaxis de búsqueda:** Query masivo `bulk_official_names()` ejecutado antes de sincronizar repositorios y uso de `""` en vez del comodín `'*'`. |
| **[H-006]** | 🔴 **HIGH** | `src/vur_client.rs:592-601` | **Defecto algorítmico en parser shell:** Bucle con condición inmutable `while quote_count % 2 == 1` y manejo frágil de cadenas multilínea. |
| **[H-007]** | 🔴 **HIGH** | `src/vur_client.rs:569-577,674` | **Omisión y corrupción de subpaquetes:** `subpackages: vec![]` hardcodeado y sobreescritura destructiva de variables del padre por funciones de subpaquetes. |
| **[H-008]** | 🔴 **HIGH** | `src/vur_client.rs:333,346,365,418,606` | **Fragilidad sintáctica y omisión silenciosa de errores:** Incapacidad de parsear constructos `case`/`if`/`+=` y múltiples `if let Ok(...)` que silencian fallos en escaneo de repositorios. |
| **[H-009]** | 🟡 **MEDIUM** | `src/metadata.rs:44-45,84-100`<br>`docs/VURINFO.md:30,69` | **Inconsistencia de esquema v1:** Divergencia entre especificación de `checksum` obligatorio y código permisivo ante sumas vacías. |

---

## 2. Evaluación Formal de Base de Datos: LMDB (`heed`) vs `installed.json`

### 2.1 Diagnóstico del Código Actual (`src/db.rs:1-138`)

El módulo `src/db.rs` gestiona la base de datos de paquetes VUR instalados. Recientemente se migró de un archivo JSON a un entorno LMDB embebido usando el crate `heed`. La auditoría detallada de la implementación revela tres fallos estructurales graves:

#### Hallazgo [H-002] (CRITICAL): Destrucción Incondicional de `installed.json` Previo sin Migración
- **Ubicación:** `src/db.rs:33-49`
- **Código real:**
```rust
impl InstalledDb {
    pub fn load(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        if path.is_file() {
            let _ = std::fs::remove_file(&path);
        }
        std::fs::create_dir_all(&path)?;
        let env = unsafe { EnvOpenOptions::new()
            .map_size(100 * 1024 * 1024)
            .max_dbs(1)
            .open(&path)? };
            
        let mut txn = env.write_txn()?;
        let db = env.create_database(&mut txn, None)?;
        txn.commit()?;
        
        Ok(Self { path, env, db })
    }
```
- **Evidencia y Riesgo:**
  1. En `src/config.rs:279-281`, la ruta configurada sigue siendo:
     ```rust
     pub fn installed_db_path(&self) -> PathBuf {
         self.cache_dir.join("installed.json")
     }
     ```
  2. Si un usuario tiene un archivo `~/.cache/vary/installed.json` existente (procedente de cualquier versión anterior o estado previo), `path.is_file()` evalúa a `true`.
  3. La línea 36 ejecuta `std::fs::remove_file(&path)` **borrando incondicionalmente el archivo de datos**.
  4. Acto seguido, la línea 38 ejecuta `std::fs::create_dir_all(&path)` convirtiendo una ruta con extensión `.json` en un directorio para albergar los archivos `data.mdb` y `lock.mdb` de LMDB.
  5. **Impacto:** Pérdida permanente del historial y seguimiento de paquetes VUR instalados por el usuario. Cuando `vary -Syu` se ejecute posteriormente, `vary` no reconocerá ningún paquete VUR preexistente en el sistema, dejando los paquetes huérfanos sin actualizaciones.

#### Hallazgo [H-003] (MEDIUM): Campo Fantasma `install_date`, Ausencia de `build_date` (ms) y Falta de Recompilación Preventiva
- **Ubicación:** `src/db.rs:18-24`, `src/install.rs:422`, `src/upgrade.rs:90-97`
- **Código real:**
```rust
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Entry {
    pub version: String,
    pub vur: String,
    pub install_date: u64,
    pub install_type: InstallType,
}
```
- **Evidencia:**
  1. `install_date` almacena un timestamp UNIX en segundos (`now_epoch()` en `src/db.rs:8,60`).
  2. Una búsqueda global en el repositorio confirma que `install_date` **jamás se lee en ninguna parte del código**: no se muestra en `vary -Qi`, no se expone en consultas ni se evalúa en `vary -Syu`. Es un campo puramente fantasma.
  3. El requisito contractual **A5** exigía `build_date` con precisión en milisegundos (`u64` o `u128`) para sustentar la **lógica de recompilación preventiva** ante cambios en plantillas o dependencias sin incremento de versión upstream.
  4. En `src/upgrade.rs:90-97`, la detección de actualizaciones solo compara strings de versión:
     ```rust
     for (name, entry) in db.entries_snapshot() {
         if let Some(cur) = current_map.get(&name) {
             if cur != &entry.version {
                 println!("  {} {} -> {}", name, entry.version, cur);
                 outdated.push(name.clone());
             }
         }
     }
     ```
  5. Si un paquete VUR modifica su template (por ejemplo, actualizando un parche o una dependencia de biblioteca compartida) pero mantiene `version` y `revision`, `vary` es ciego al cambio y no recompila preventivamente.

#### Hallazgo [H-004] (LOW): Métodos y Campos Muertos en `src/db.rs`
- **Ubicación:** `src/db.rs:27, 51-53, 68-71, 83-95`
- **Evidencia:**
  1. Campo `path: PathBuf` en `struct InstalledDb` (línea 27) se inicializa pero jamás se consulta.
  2. Métodos `pub fn get(&self, name: &str) -> Option<Entry>` (líneas 68-71) y `pub fn names(&self) -> Vec<String>` (líneas 83-95) no se invocan en ningún módulo, produciendo advertencias directas de `rustc`:
     ```text
     warning: methods `get` and `names` are never used --> src/db.rs:68:12
     warning: field `path` is never read --> src/db.rs:27:5
     ```
  3. El método `pub fn save(&self) -> Result<()>` (líneas 51-53) es un stub vacío (`Ok(())`), porque LMDB persiste inmediatamente en cada commit. Sin embargo, `install.rs:428`, `remove.rs:33` y `upgrade.rs:86` continúan invocando `db.save()?` como vestigio redundante.

---

### 2.2 Comparación Exhaustiva y Neutral de las Dos Rutas Técnicas (Para Decisión de Usuario en PC-2)

Para la toma de decisiones en el Punto de Control PC-2, se presenta a continuación la evaluación formal de las dos alternativas de persistencia.

#### Ruta (a): Revertir a `installed.json` (JSON Estructurado con Escritura Atómica)
Regreso al esquema basado en texto serializado (`serde_json`), refactorizado para garantizar atomicidad y compatibilidad hacia atrás.

- **Diseño Técnico Propuesto:**
  1. **Atomicidad POSIX:** Escritura mediante archivo temporal en el mismo sistema de archivos (`~/.cache/vary/installed.json.tmp.<pid>`), `flush()` + `sync_all()`, seguido de `std::fs::rename()` atómico sobre `installed.json`. Esto previene corrupción ante cortes eléctricos o `kill -9`.
  2. **Control de Versiones del Esquema:** Adición de encabezado o campo de versión:
     ```json
     {
       "schema_version": 2,
       "packages": {
         "paquete-ejemplo": {
           "version": "1.0_1",
           "vur": "core",
           "install_type": "source",
           "build_date": 1725624000123
         }
       }
     }
     ```
  3. **Migración Tolerante:** Uso de `#[serde(default)]` en campos como `build_date: Option<u64>` e `install_date: Option<u64>`. Si un archivo v1 o heredado carece de `build_date`, se inicializa como `None` o `0` sin fallar.
  4. **Lógica de Recompilación Preventiva:** En `upgrade.rs`, si el commit HEAD de la plantilla en el repositorio VUR o la fecha de compilación de sus dependencias es posterior al `build_date` registrado, se marca el paquete para recompilación automática.
  5. **Concurrencia:** `vary` ya cuenta con un lockfile de proceso exclusivo mediante `flock` en `src/lock.rs` (`~/.cache/vary/vary.lock`). Ninguna instancia paralela compite por escribir en el archivo.

- **Ventajas:**
  - *Transparencia e Inspección:* Archivo de texto plano legible por humanos; se puede consultar o editar con `cat`, `jq`, `grep` o scripts del usuario sin herramientas binarias especializadas.
  - *Cero Dependencias C:* Elimina completamente el crate `heed` y la dependencia nativa en C `liblmdb`/`lmdb-master-sys`. Reduce tiempos de compilación y elimina riesgos de interoperabilidad estática con `musl`.
  - *Consumo de Recursos Reducido:* Para colecciones habituales de paquetes de usuario (10–200 paquetes), el archivo pesa entre 2 y 50 KB.
  - *Consistencia de Nombres:* La ruta `~/.cache/vary/installed.json` vuelve a ser un archivo real `.json` y no un directorio con archivos LMDB dentro.

- **Desventajas:**
  - *Escalabilidad O(N):* Cada inserción o borrado requiere deserializar y reserializar el mapa completo en memoria. Para miles de paquetes podría haber una penalización medible de CPU/I/O (aunque en entornos VUR los paquetes rara vez superan los cientos).
  - *Mayor Huella de Memoria Momentánea:* El árbol completo reside en RAM durante las operaciones de actualización.

---

#### Ruta (b): Mantener y Reparar LMDB (`heed`)
Consolidar la base de datos embebida LMDB, subsanando todas las deficiencias y riesgos de migración actuales.

- **Diseño Técnico Propuesto:**
  1. **Migración Previa Segura en `load()`:**
     - Comprobar si `path.is_file()`. Si es un archivo, leerlo e intentar deserializarlo como `installed.json` tradicional.
     - Extraer todas las entradas existentes.
     - Renombrar el archivo a `installed.json.bak` (nunca llamar a `remove_file`).
     - Renombrar la ruta interna de la base de datos a `~/.cache/vary/installed.db/` (evitando la aberración de llamar a un directorio con extensión `.json`).
     - Abrir LMDB e insertar en una transacción de escritura única todas las entradas migradas con `build_date: now_epoch_ms()`.
  2. **Incorporación de `build_date`:** Actualizar `Entry` para registrar `build_date: u64` en milisegundos y conectar su lectura a `src/upgrade.rs`.
  3. **Transacciones por Lote:** En lugar de abrir un `write_txn()` por cada entrada en `install.rs:422` (lo que produce múltiples commits a disco innecesarios), agrupar la persistencia de toda la lista de instalación bajo una sola transacción ACID.
  4. **Eliminación de Código Muerto:** Limpiar el campo `path` y conectar o suprimir `get()` y `names()`.

- **Ventajas:**
  - *Transacciones ACID y MVCC Real:* Lectores concurrentes sin bloqueo; resiliencia total frente a caídas imprevistas mediante B-Tree Copy-on-Write a nivel de páginas.
  - *Acceso Directo O(log N) e I/O en Memoria Mapeada:* Cero tiempo de parseo inicial al abrir la base de datos; la memoria mapeada (`mmap`) permite consultar registros individuales sin cargar la colección entera a RAM.
  - *Rendimiento en Escritura Parcial:* Modificar una entrada solo altera una página de 4KB en disco en vez de rescribir todo el catálogo.

- **Desventajas:**
  - *Dependencia de C y Complejidad de Build:* Depende de `heed` y C `liblmdb`. Aumenta el tiempo de compilación y la superficie de código no-Rust en el binario.
  - *Espacio de Direcciones Virtual (`map_size`):* `src/db.rs:40` reserva 100 MB (`100 * 1024 * 1024`) de espacio de memoria virtual con `mmap`. En arquitecturas de 32 bits soportadas por Void Linux (como `armv7l` o `i686`), la fragmentación del espacio virtual de 3 GB para procesos de usuario puede ser problemática si se escala el map size.
  - *Opacidad Binaria:* Base de datos opaca (`data.mdb`). El usuario no puede inspeccionar sus paquetes instalados con comandos estándar del sistema (`jq`, `cat`) sin un subcomando específico de `vary`.
  - *Redundancia con el Lockfile:* La ventaja multiescritura/concurrencia de LMDB queda neutralizada porque `vary` bloquea globalmente su ejecución con `flock` a nivel de proceso.

---

## 3. Auditoría del Parseo de Salida de `xbps-query`

### 3.1 Análisis de `names_from_search_output` y `parse_search_line` (`src/xbps.rs:109-147, 284-291`)

La función `parse_search_line` deserializa líneas crudas emitidas por `xbps-query -Rs` en estructuras `SearchHit`:

```rust
pub fn parse_search_line(line: &str) -> Option<SearchHit> {
    let mut rest = line.trim();
    if rest.is_empty() {
        return None;
    }

    let installed = if bracketed(rest).is_some() {
        let mark = bracketed(rest)?;
        let hit = matches!(mark, "*" | "x");
        rest = after_bracketed(rest)?;
        hit
    } else {
        false
    };

    let repository = if bracketed(rest).is_some() {
        let repo = bracketed(rest)?.to_owned();
        rest = after_bracketed(rest)?;
        Some(repo)
    } else {
        None
    };

    let pkgver_end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    let pkgver = &rest[..pkgver_end];
    if pkgver.is_empty() {
        return None;
    }
    let short_desc = rest[pkgver_end..].trim();
    let name = pkgver.rsplit_once('-').map_or(pkgver, |(n, _)| n);

    Some(SearchHit {
        name: name.to_owned(),
        pkgver: pkgver.to_owned(),
        short_desc: (!short_desc.is_empty()).then(|| short_desc.to_owned()),
        repository,
        installed,
    })
}
```

#### Verificación de Casos Límite:
1. **Descripciones Largas y Unicode:**
   - `stdout_text(&out)` en `src/xbps.rs:73` utiliza `String::from_utf8_lossy(&out.stdout)`. No hay riesgo de panic ante bytes inválidos.
   - `rest.find(char::is_whitespace)` busca sobre caracteres válidos UTF-8 y devuelve índices en los límites de caracteres (`char boundary`).
   - `short_desc` captura el resto de la línea recortando con `trim()`. Las descripciones arbitrariamente largas o con caracteres multietapa (acentos, kanji, emojis) se preservan intactas.
2. **Paquetes con Múltiples Guiones (`foo-bar-baz-1.0_1`):**
   - El extractor utiliza `pkgver.rsplit_once('-')`.
   - Según las directrices oficiales de Void Linux para empaquetado XBPS, el campo de versión nunca puede contener guiones (`-`), sino únicamente `[A-Za-z0-9._+]`. El guion final es siempre el delimitador entre el nombre del paquete y su versión-revisión (`<name>-<version>_<revision>`).
   - Por tanto, `rsplit_once('-')` aísla correctamente el nombre completo del paquete a la izquierda (`foo-bar-baz`), garantizando que paquetes como `linux6.6-headers-6.6.15_1` devuelvan `name = "linux6.6-headers"`.
3. **Marcas de Estado `[*]`, `[x]`, `[-]`, `[?]` y Repositorios `[multilib]`:**
   - La función auxiliar `bracketed(s)` busca el primer `[` inicial y su correspondiente `]`.
   - Si la línea inicia con `[*]`, `installed` es `true`. Con `[-]`, `[?]` o `[ ]`, `installed` es `false`.
   - Tras remover la primera marca con `after_bracketed(rest)`, si el siguiente token vuelve a empezar con `[`, se interpreta correctamente como el repositorio (ej. `[multilib]`).
   - **Caso borde detectado:** Si la descripción corta de un paquete que no tiene repositorio contiene corchetes al inicio, no hay colisión porque `rest` en el segundo chequeo de corchetes apunta a `pkgver`, que no empieza por `[`.
   - **Caso patológico:** Si `xbps-query` alguna vez emitiera una línea sin marca de estado `[-]`/`[*]` pero con `[repo]`, el repositorio sería consumido como marca de estado (marcando `installed = false`) y `pkgver` absorbería el resto. No obstante, en la práctica estándar de Void Linux, la primera columna siempre contiene la marca de estado de 3 caracteres.

---

### 3.2 Hallazgo [H-005] (MEDIUM): Anomalías en `bulk_official_names()` (Comodín y Orden de Invocación)

- **Ubicación:** `src/xbps.rs:264-282`, `src/install.rs:224-234`
- **Código real (`src/xbps.rs:264-282`):**
```rust
pub fn bulk_official_names() -> Option<std::collections::HashSet<String>> {
    let urls = official_repo_urls();
    if urls.is_empty() {
        return None;
    }
    let mut owned: Vec<String> = vec!["--ignore-conf-repos".to_string()];
    for u in &urls {
        owned.push("--repository".to_string());
        owned.push(u.clone());
    }
    owned.push("-Rs".to_string());
    owned.push(String::new()); // <-- ARGUMENTO VACÍO ""
    let args: Vec<&str> = owned.iter().map(|s| s.as_str()).collect();
    let out = run_capture(XBPS_QUERY, &args).ok()?;
    if !out.status.success() {
        return None;
    }
    Some(names_from_search_output(&stdout_text(&out)))
}
```
- **Código real (`src/install.rs:224-234`):**
```rust
    // E2 fast-path: snapshot de oficiales al inicio del sync en UNA sola
    // consulta masiva...
    let bulk: Option<std::sync::Arc<HashSet<String>>> =
        crate::xbps::bulk_official_names().map(std::sync::Arc::new);
    match &bulk {
        Some(s) => tracing::debug!("bulk oficial: {} paquetes", s.len()),
        None => tracing::debug!("bulk oficial no disponible; path escalar"),
    }
```

#### Deficiencias Identificadas:
1. **Comodín no Canónico (`""` vs `'*'`):**
   - En la línea 275, `owned.push(String::new());` pasa una cadena vacía como patrón de búsqueda a `xbps-query -Rs ""`.
   - Aunque la expresión regular de libc empareja cadenas vacías con cualquier texto en ciertas configuraciones, la sintaxis documentada y garantizada por el comando `xbps-query(1)` para enumerar todo el repositorio es el comodín `'*'`: `xbps-query -Rs '*'`.
   - En entornos o versiones de XBPS con compilación minimalista de regex o flags estrictos, `""` puede provocar comportamientos indefinidos o fallos en el filtrado de paquetes.
2. **Orden de Invocación Incorrecto (Query Masivo Antes de Sincronizar):**
   - En `src/install.rs`, `bulk_official_names()` se invoca en el **Paso 5** (línea 230), ANTES de que ocurra la sincronización de repositorios remotos oficiales (`xbps-install -S`), la cual se ejecuta recién en el **Paso 8** (línea 352).
   - En Void Linux, los metadatos remotos residen en `/var/db/xbps/`. `xbps-query` consulta exclusivamente la base de datos local preexistente.
   - Si el usuario ejecuta `vary -S <paquete>` o `vary -Sy <paquete>`, la captura masiva en memoria (`HashSet<String>`) se genera contra un catálogo oficial desactualizado.
   - Si un paquete oficial fue incorporado o actualizado recientemente en los repositorios de Void, el snapshot de `bulk` sufrirá un falso negativo. Aunque existe la confirmación escalar de respaldo (`official_exists_remote`), esta tampoco ha sincronizado `/var/db/xbps/`, induciendo a `vary` a intentar construir desde VUR un paquete que ya existe oficialmente en repositorios binarios, o fallando la resolución de dependencias.
   - **Corrección requerida:** Sincronizar los repositorios del sistema con `xbps-install -S` ANTES de tomar la captura `bulk_official_names()`.

---

## 4. Auditoría del Parser Shell Fallback (A2) en `src/vur_client.rs`

El parser `parse_template_text` (`src/vur_client.rs:557-690`) es el mecanismo de contingencia para extraer metadatos de plantillas Bash cuando no existe el archivo `.VURINFO` precalculado en el repositorio VUR.

### 4.1 Hallazgo [H-006] (HIGH): Condición Inmutable en Bucle de Comillas Multilínea
- **Ubicación:** `src/vur_client.rs:591-601`
- **Código real:**
```rust
        // Manejar valores multilínea entre comillas dobles
        let quote_count = buf.matches('"').count();
        while quote_count % 2 == 1 {
            if let Some(next) = lines.next() {
                buf.push('\n');
                buf.push_str(next);
                if next.contains('"') { break; }
            } else {
                break;
            }
        }
```
- **Evidencia y Riesgo:**
  1. `quote_count` se calcula **una sola vez** en la línea 592 sobre el valor inicial de `buf`.
  2. En el bucle `while quote_count % 2 == 1`, la variable `quote_count` es **inmutable** dentro del bucle (detectado explícitamente por clippy: `clippy::while_immutable_condition`).
  3. Si la primera línea abre una comilla doble (`quote_count = 1`), el bucle itera acumulando líneas subsecuentes en `buf`.
  4. La condición de ruptura interna es `if next.contains('"') { break; }`.
  5. **Falla de paridad:** Si `next` contiene una comilla escapada `\"` o una comilla en un comentario, el `break` se dispara prematuramente dejando la cadena truncada o desbalanceada.
  6. Si `next` abre y cierra otra comilla interna (ej. `VAR="foo"`) manteniendo el total impar, el bucle se rompe prematuramente antes de que la asignación multilínea haya cerrado verdaderamente.
  7. **Ausencia total para comillas simples:** No existe ningún bloque análogo para valores multilínea delimitados por comillas simples (`'`), muy comunes en scripts de empaquetado.

---

### 4.2 Hallazgo [H-007] (HIGH): Omisión de Subpaquetes y Clobbering de Variables del Padre
- **Ubicación:** `src/vur_client.rs:569-576, 674`
- **Código real:**
```rust
        // Ignorar definiciones de funciones y bloques shell
        if trimmed.starts_with("do_") || trimmed.starts_with("pre_") || trimmed.starts_with("post_") || trimmed.starts_with("}") || trimmed.starts_with("{") {
            // Si es inicio de función, saltar hasta }
            if trimmed.contains("()") {
                while let Some(l) = lines.next() {
                    if l.trim() == "}" { break; }
                }
            }
            continue;
        }
...
    let info = VurInfo {
        format_version: 1,
        pkgname: pkgname.clone(),
        version,
        revision,
        archs,
        subpackages: vec![], // <-- HARDCODEADO A VACÍO
...
```
- **Evidencia y Riesgo:**
  1. En las plantillas de Void Linux, los subpaquetes se declaran mediante funciones shell nombradas con el sufijo `_package`:
     ```sh
     libfoo-devel_package() {
         depends="${sourcepkg}>=${version}_${revision}"
         short_desc="Development files for foo"
     }
     ```
  2. El filtro de funciones de la línea 569 solo comprueba si la línea empieza por `do_`, `pre_`, `post_`, `}` o `{`. **No detecta `<subpkg>_package()`**.
  3. Al encontrar la declaración de un subpaquete, el parser no la ignora: continúa iterando línea a línea.
  4. Las líneas dentro de la función del subpaquete (como `depends="..."` o `short_desc="..."`) son detectadas como asignaciones clave-valor válidas en la línea 604.
  5. **Corrupción de datos:** `vars.insert("depends", val)` del subpaquete **sobrescribe silenciosamente las dependencias del paquete padre principal**.
  6. Al final de la función (línea 674), `subpackages: vec![]` se asigna incondicionalmente a un vector vacío. El subpaquete (`libfoo-devel`) desaparece por completo del índice, y el paquete padre queda con las dependencias truncadas del subpaquete.

---

### 4.3 Hallazgo [H-008] (HIGH): Incapacidad para Parsear Constructos Shell Condicionales (`case`/`if`/`+=`) y Silenciamiento de Errores
- **Ubicación:** `src/vur_client.rs:604-615`, `src/vur_client.rs:333,346,365,418`

#### A. Constructos Shell No Soportados:
1. **Asignaciones Acumulativas (`+=`):**
   - En la línea 606: `if !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') { continue; }`.
   - Asignaciones habituales como `makedepends+=" bar"` contienen el carácter `+`, que no pasa el filtro alfanumérico. Como resultado, **todas las adiciones condicionales de dependencias son descartadas silenciosamente**.
2. **Estructuras Condicionales (`case` / `if`):**
   - En Void Linux, muchas plantillas configuran dependencias según la arquitectura objetivo:
     ```sh
     case "$XBPS_TARGET_MACHINE" in
         x86_64*) makedepends+=" nasm" ;;
         aarch64*) makedepends+=" arm-trusted-firmware" ;;
     esac
     ```
   - El parser no interpreta estructuras de control. Si las asignaciones usaran `=`, procesaría todas las ramas secuencialmente, ganando la última que aparezca en el archivo independientemente de la arquitectura real de compilación.
3. **Expansión de Parámetros Shell:**
   - No hay sustitución de variables. Cadenas como `distfiles="https://example.com/${pkgname}-${version}.tar.gz"` quedan almacenadas textualmente como `${pkgname}-${version}`, lo que rompe cualquier proceso posterior que requiera resolver la URL o validar checksums.

#### B. Documentación de Errores Tragados Silenciosamente (`if let Ok(...)`):
La auditoría de `src/vur_client.rs` identificó un patrón recurrente de silenciamiento de errores en el escaneo de repositorios (desviación de R2):
- **Línea 333:** `if let Ok(text) = self.git_show_file(&vurinfo_path)`: Si `git show` falla por corrupción o referencias erróneas, se descarta silenciosamente.
- **Línea 346:** `if let Ok(text) = self.git_show_file(".VURINFO")`: Fallos de lectura del índice raíz no emiten aviso alguno.
- **Línea 365:** `if let Ok(text) = self.git_show_file(&tmpl_path)`: Plantillas inaccesibles no informan del error.
- **Línea 372-375:** Ante fallo de parseo de template:
  ```rust
  Err(err) => tracing::warn!(
      "template inválido ignorado en {}:{}: {err:#}",
      self.name, tmpl_path
  ),
  ```
  Solo emite un evento en el log de `tracing`. Sin `RUST_LOG` o flags de verbosidad, esto es **totalmente invisible en la terminal del usuario**, quien solo verá un error genérico de paquete no encontrado al intentar instalarlo.
- **Línea 413 y 417:** `if let Ok(entries) = std::fs::read_dir(&base)` y `if let Ok(text) = std::fs::read_to_string(p.join("template"))`: Errores de permisos o inodos dañados en el disco se tragan sin trazabilidad.
- **Línea 418:** En `project_pkg`:
  ```rust
  if let Ok(info) = parse_template_text(&text, p.join("template").to_string_lossy().as_ref()) {
      if info.pkgname == pkgname || info.subpackages.iter().any(|s| s.pkgname == pkgname) {
          found = Some(p);
          break;
      }
  }
  ```
  Si una plantilla falla al parsearse, `if let Ok` traga el error sin advertir al usuario, y `project_pkg` concluye que el paquete no existe (`template no encontrado para ...`), impidiendo su compilación.

---

## 5. Auditoría del Esquema `.VURINFO` en `src/metadata.rs`

### 5.1 Reglas del Esquema v1 y Validación Formal

La especificación v1 de `.VURINFO` se implementa en `src/metadata.rs:34-146` mediante `VurInfo::validate()`:

```rust
pub fn validate(&self) -> Result<()> {
    if self.format_version != SUPPORTED_FORMAT_VERSION {
        bail!("format_version no soportado: {} (soportado: {})", self.format_version, SUPPORTED_FORMAT_VERSION);
    }
    if self.pkgname.is_empty() {
        bail!("pkgname vacío: el nombre del paquete es obligatorio");
    }
    if !is_valid_pkgname(&self.pkgname) {
        bail!("pkgname inválido: '{}' contiene caracteres prohibidos o empieza por '-' ...", self.pkgname);
    }
    if self.version.is_empty() {
        bail!("version vacía: la versión upstream es obligatoria");
    }
    if !is_valid_version(&self.version) {
        bail!("version inválida: '{}' contiene caracteres prohibidos ...", self.version);
    }
    if self.revision < 1 {
        bail!("revision inválida: {} (debe ser >= 1)", self.revision);
    }
    if self.archs.is_empty() {
        bail!("archs vacío: debe declararse al menos una arquitectura objetivo");
    }
    if self.checksum.is_empty() {
        tracing::warn!("{}: checksum vacío (paquete con do_fetch personalizado?)", self.pkgname);
    } else {
        for (i, sum) in self.checksum.iter().enumerate() {
            if sum == "SKIP" {
                continue;
            }
            if sum.is_empty() || !sum.starts_with("sha256:") {
                bail!("checksum[{}] inválido: '{}' debe empezar por 'sha256:' o ser 'SKIP' ...", i, sum);
            }
        }
    }
    for (i, sub) in self.subpackages.iter().enumerate() {
        if sub.pkgname.trim().is_empty() {
            bail!("subpackages[{}].pkgname vacío: el nombre del subpaquete es obligatorio", i);
        }
        if sub.pkgname == self.pkgname {
            bail!("subpackages[{}].pkgname duplicado: '{}' coincide con el pkgname del padre", i, sub.pkgname);
        }
        for (j, dep) in sub.depends.iter().enumerate() {
            if dep.trim().is_empty() {
                bail!("subpackages[{}].depends[{}] inválida: la dependencia está vacía ...", i, j);
            }
        }
    }
    Ok(())
}
```

#### Análisis de Cada Restricción:
1. **`format_version == 1`:** Estrictamente validado contra la constante `SUPPORTED_FORMAT_VERSION = 1`. Cualquier versión distinta arroja `bail!`. Correcto.
2. **Validación de `pkgname`:**
   - Se valida con `is_valid_pkgname`: `!n.is_empty() && !n.starts_with('-') && n.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '+' | '-'))`.
   - Se apega fielmente al estándar de nombrado de Void Linux.
3. **Validación de `version`:**
   - Se valida con `is_valid_version`: `!v.is_empty() && v.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '+'))`.
   - **Prohibición de guiones:** Los guiones (`-`) se rechazan explícitamente. Esta es una regla fundamental de diseño para evitar ambigüedades al parsear strings `<pkgname>-<version>_<revision>`.
4. **`revision >= 1`:** El entero debe ser mayor o igual a 1. Cumple con la semántica de XBPS.
5. **`archs` no vacío:** Obliga a especificar al menos una arquitectura válida (ej. `["x86_64"]`, `["all"]`).
6. **Subpaquetes:**
   - Se verifica que el nombre no esté en blanco.
   - Se impide que un subpaquete tenga el mismo nombre que el paquete padre (`sub.pkgname == self.pkgname`).
   - Se comprueba que cada dependencia dentro de `sub.depends` no sea una cadena vacía o de solo espacios.
   - **Herencia de arquitecturas:** En el esquema v1, `Subpackage` no tiene campo `archs` propio; hereda las del padre a través de `VurInfo::normalize()`.

---

### 5.2 Hallazgo [H-009] (MEDIUM): Inconsistencia entre Documentación y Validación de `checksum`

- **Ubicación:** `src/metadata.rs:44-45, 84-86`, `docs/VURINFO.md:30, 69`
- **Contradicción Documentada:**
  1. En `docs/VURINFO.md:69` se establece formalmente:
     > *"vary rechaza el índice (con error claro al usuario) si: ... alguna entrada de checksum carece del prefijo sha256: (se admite SKIP)."*
  2. En el docstring de `src/metadata.rs:44-45`:
     > *"5. cada elemento de `checksum` no vacío y con prefijo `sha256:`; la lista no puede estar vacía (un paquete sin sumas es inválido)."*
  3. Sin embargo, en la implementación real (`src/metadata.rs:84-86`):
     ```rust
     if self.checksum.is_empty() {
         tracing::warn!("{}: checksum vacío (paquete con do_fetch personalizado?)", self.pkgname);
     }
     ```
     La ausencia de checksum **no aborta con error**, sino que solo emite un `tracing::warn!`, permitiendo que el paquete se considere válido.
  4. En `scripts/vurinfo.jq:39-42`:
     Si una plantilla carece de checksum, se genera `checksum: []`, pasando la validación de `metadata.rs` a pesar de que la documentación afirma que debe ser rechazado.
- **Dictamen:** Existe una discrepancia contractual entre la documentación oficial (`docs/VURINFO.md`) y el validador en código. Si se desea permitir paquetes con funciones `do_fetch()` personalizadas (que no descargan distfiles tradicionales), la documentación en `docs/VURINFO.md` y el docstring de `src/metadata.rs` deben actualizarse explícitamente para documentar esta excepción legítima.

---

## 6. Recomendaciones Técnicas para SA-3

1. **Base de Datos (Decisión PC-2):**
   - Si se elige la **Ruta (a)**: Restaurar `src/db.rs` al modelo basado en `serde_json`, agregando escritura atómica (archivo temporal + `rename`), `schema_version`, soporte para `build_date` ms en `Entry` con deserialización tolerante y lógica de recompilación preventiva en `upgrade.rs`.
   - Si se elige la **Ruta (b)**: Corregir `InstalledDb::load` para que migre `installed.json` a LMDB en vez de destruirlo con `remove_file`; renombrar la ruta interna a un directorio `installed.db`; registrar y consultar `build_date` ms; y eliminar campos/métodos muertos.
2. **Parseo de `xbps-query`:**
   - En `src/xbps.rs:275`, cambiar el argumento `String::new()` por `"*".to_string()` para que invoque `xbps-query -Rs '*'`.
   - En `src/install.rs`, asegurar que la sincronización de repositorios (`xbps-install -S`) se efectúe antes de llamar a `bulk_official_names()`.
3. **Parser Shell (A2):**
   - Corregir el bucle de comillas multilínea en `src/vur_client.rs:592-601`, recalculando `buf.matches('"').count()` dinámicamente y evitando rupturas prematuras ante comillas escapadas o impares.
   - Modificar la detección de funciones en la línea 569 para que salte adecuadamente cualquier definición `*_package()` o implemente extracción básica de subpaquetes en lugar de sobreescribir las dependencias del padre.
   - Sustituir los `if let Ok` ciegos por trazabilidad y advertencias visibles al usuario cuando una plantilla VUR no pueda ser parseada.
4. **Metadatos v1:**
   - Sincronizar `docs/VURINFO.md` con `src/metadata.rs` para aclarar si `checksum: []` está permitido bajo `do_fetch()` personalizado o si debe ser obligatorio con `SKIP`.
