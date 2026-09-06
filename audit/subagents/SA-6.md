# SA-6: Reporte de Auditoría de Calidad de Código Rust

**Subagente:** SA-6 (Calidad de Código Rust)  
**Proyecto:** `vary` (Void User Repository helper en Rust)  
**Ruta:** `/run/media/dicov/LudoDrive/dicov-op/Vary`  
**Fecha:** 2026-09-06  
**Rama:** `vary-mvp` @ `1e32f1f` (+ cambios no commiteados del árbol de trabajo)  
**Alcance:** Higiene de código, 57 errores de `cargo clippy`, bug lógico en `vur_client.rs`, inventario de `unwrap/expect/panic`, código muerto, dependencias de `Cargo.toml`, MSRV y cobertura de tests.

---

## 1. Resumen Ejecutivo

La auditoría de calidad de código Rust sobre `vary` revela un proyecto con fundamentos arquitectónicos sólidos (separación modular, DAG de resolución sin I/O, tipado estricto con `anyhow`), pero que actualmente exhibe **importantes deudas de calidad, higiene y estabilidad** heredadas de refactorizaciones incompletas:

1. **Clippy bloquea la compilación (`exit code 101`):** Se identificaron y categorizaron exactamente **57 errores** bajo `cargo clippy --all-targets -- -D warnings` repartidos en 15 lints distintos.
2. **Bug de lógica confirmado en `src/vur_client.rs:593`:** El ciclo `while quote_count % 2 == 1` contiene una condición estática/invariante (`while_immutable_condition`) que no recalcula comillas, terminando prematuramente en líneas intermedias o ciclando hasta EOF ante multilíneas complejas.
3. **16 llamadas a `unwrap()` y `expect()` en rutas de producción:** Se detectaron 4 puntos de riesgo crítico/alto de pánico en tiempo de ejecución:
   - `src/install.rs:367`: `repos_conf.vur.get(repo).unwrap()` durante transacciones binarias.
   - `src/config.rs:126`: `Self::new().expect("config defaults")` en `Config::default()` que paniquea si `HOME` no está definido.
   - `src/init.rs:24`: `to_str().unwrap()` en rutas con nombres de paquete potencialmente no-UTF-8.
   - `src/main.rs:37-39`: `expect(...)` en llamadas del sistema de abandono de privilegios (`setgroups`, `setresgid`, `setresuid`).
4. **Código muerto y vestigios de refactor:** Campos `path` no leídos en `CacheIndex` (`src/cache.rs:19`) e `InstalledDb` (`src/db.rs:27`), métodos nunca llamados `get()` y `names()` (`src/db.rs:68, 83`), y un stub `save()` sin efecto operativo tras la migración a LMDB/`heed`.
5. **Inconsistencias en `Cargo.toml`:**
   - La dependencia `heed` (LMDB) añade complejidad C/unsafe innecesaria si la base de datos se revierte al formato contractual JSON (`installed.json` con `build_date` ms para A5).
   - Dependencias contractuales requeridas por la especificación del sistema están **completamente ausentes**: `diffy` para diff interactivo en `src/upgrade.rs` (A3) e `indicatif` para spinners y barras de progreso asíncronas (A7).
   - El MSRV se encuentra sincronizado en `1.88` entre `Cargo.toml` y `.github/workflows/ci.yml`.
6. **Brecha crítica en cobertura de pruebas:** De los 95 tests existentes (91 activos + 4 ignorados por requerir Void Linux nativo), **10 módulos neurálgicos tienen 0 tests unitarios**, incluyendo `db.rs`, `cache.rs`, `search.rs`, `upgrade.rs`, `install.rs` y `config.rs`.

---

## 2. Inventario y Resolución de los 57 Errores de Clippy

Bajo el comando `cargo clippy --all-targets -- -D warnings`, el compilador finaliza con código de salida `101`, emitiendo **57 errores** (53 en biblioteca principal y 4 en tests).

### 2.1 Distribución por Categorías

| Lint / Categoría | Ocurrencias | Archivo(s) Afectado(s) | Severidad |
|---|:---:|---|:---:|
| `clippy::print_literal` | 35 | `src/help.rs:4-51` (34 casos), `src/repo.rs:148` (1 caso) | Baja |
| `clippy::manual_flatten` | 4 | `src/cache.rs:69`, `src/db.rs:87, 101, 120` | Media |
| `dead_code` | 3 | `src/cache.rs:19`, `src/db.rs:27`, `src/db.rs:68, 83` | Media |
| `clippy::manual_is_multiple_of` | 2 | `src/vur_client.rs:620` (doble check en la misma línea) | Baja |
| `clippy::single_element_loop` | 2 | `src/vur_client.rs:826`, `src/vur_client.rs:847` (tests) | Baja |
| `clippy::useless_format` | 2 | `src/metadata.rs:379`, `src/metadata.rs:391` (tests) | Baja |
| `clippy::collapsible_if` | 1 | `src/vur_client.rs:484` | Baja |
| `clippy::while_immutable_condition` | 1 | `src/vur_client.rs:593` | **Alta (Bug)** |
| `clippy::while_let_on_iterator` | 1 | `src/vur_client.rs:572` | Baja |
| `clippy::obfuscated_if_else` | 1 | `src/search.rs:69` | Baja |
| `clippy::manual_unwrap_or_default` | 1 | `src/search.rs:94` | Baja |
| `clippy::unnecessary_cast` | 1 | `src/signal.rs:135` | Baja |
| `clippy::needless_return` | 1 | `src/lock.rs:37` | Baja |
| `clippy::suspicious_open_options` | 1 | `src/lock.rs:27` | Media |
| `clippy::manual_pattern_char_comparison` | 1 | `src/remove.rs:27` | Baja |
| **TOTAL** | **57** | **11 archivos** | |

---

### 2.2 Detalle Técnico y Corrección Idiomática por Categoría

#### Categoría 1: `clippy::print_literal` (35 errores)
* **Archivos y Líneas:**
  - `src/help.rs:4, 5, 6, 9, 10, 11, 12, 13, 16, 17, 18, 19, 20, 23, 24, 25, 26, 27, 28, 31, 32, 33, 34, 35, 36, 37, 40, 41, 42, 43, 48, 49, 50, 51`
  - `src/repo.rs:148`
* **Snippet problemático (`src/help.rs:4` y similares):**
  ```rust
  println!("{}", "    vary");
  println!("{}", "    vary {-h --help}");
  ```
  `src/repo.rs:148`:
  ```rust
  println!("{:<20} {:<8} {:<45} {}", "NAME", "PRIO", "URL", "BINARY");
  ```
* **Diagnóstico:** Se emplea `println!("{}", "literal")` innecesariamente pasando un literal como argumento de formato vacío. En líneas con llaves `{}` de ayuda, la cadena literal solo requiere escape doble `{{` y `}}`. En `repo.rs:148`, el último término `"BINARY"` es estático y debe formar parte del string de formato.
* **Corrección idiomática:**
  ```rust
  // src/help.rs
  println!("    vary");
  println!("    vary {{-h --help}}");
  
  // src/repo.rs:148
  println!("{:<20} {:<8} {:<45} BINARY", "NAME", "PRIO", "URL");
  ```

---

#### Categoría 2: `clippy::manual_flatten` (4 errores)
* **Archivos y Líneas:**
  - `src/cache.rs:69`
  - `src/db.rs:87`
  - `src/db.rs:101`
  - `src/db.rs:120`
* **Snippet problemático (`src/db.rs:87-91`):**
  ```rust
  for item in iter {
      if let Ok((k, _)) = item {
          names.push(k.to_string());
      }
  }
  ```
* **Diagnóstico:** Bucle manual filtrando únicamente la variante `Ok` de los elementos devueltos por un iterador de resultados.
* **Corrección idiomática:**
  ```rust
  // En src/cache.rs:69
  for (key, _) in iter.flatten() {
      if key.starts_with(&format!("{}:", repo_name)) {
          keys_to_delete.push(key.to_string());
      }
  }

  // En src/db.rs:87
  for (k, _) in iter.flatten() {
      names.push(k.to_string());
  }

  // En src/db.rs:101
  for (k, v) in iter.flatten() {
      if !pred(k, &v) {
          to_remove.push(k.to_string());
      }
  }

  // En src/db.rs:120
  for (k, v) in iter.flatten() {
      entries.push((k.to_string(), v));
  }
  ```

---

#### Categoría 3: `dead_code` (3 errores)
* **Archivos y Líneas:**
  - `src/cache.rs:19:5`: campo `path` en struct `CacheIndex`.
  - `src/db.rs:27:5`: campo `path` en struct `InstalledDb`.
  - `src/db.rs:68:12` y `src/db.rs:83:12`: métodos `get(&self, name: &str)` y `names(&self)`.
* **Snippet problemático:**
  ```rust
  // src/cache.rs:18-22
  pub struct CacheIndex {
      path: PathBuf, // <- Nunca leído tras load()
      env: heed::Env,
      db: Database<Str, SerdeBincode<CachedIndex>>,
  }

  // src/db.rs:26-30
  pub struct InstalledDb {
      path: PathBuf, // <- Nunca leído tras load()
      env: heed::Env,
      db: Database<Str, SerdeBincode<Entry>>,
  }
  ```
* **Diagnóstico:** Campos almacenados en structs que no vuelven a consultarse tras la inicialización del entorno LMDB/heed. Asimismo, `get()` y `names()` fueron implementados en `InstalledDb` pero ninguna parte de `vary` los invoca.
* **Corrección idiomática:**
  - Si `InstalledDb` y `CacheIndex` retienen `heed`: remover el campo `path` de las structs o prefijarlo como `_path: PathBuf`.
  - Para `get()` y `names()`: integrarlos en consultas CLI (ej. `vary -Q`) o anotar con `#[allow(dead_code)]` si forman parte de la API pública intencional del crate. Si `InstalledDb` se revierte a `installed.json` (A5), reestructurar la API completa.

---

#### Categoría 4: `clippy::manual_is_multiple_of` (2 errores)
* **Archivo y Línea:** `src/vur_client.rs:620:20` y `src/vur_client.rs:620:60`
* **Snippet problemático:**
  ```rust
  if before.matches('"').count() % 2 == 0 && before.matches(''').count() % 2 == 0 {
      val.truncate(hash);
      val = val.trim().to_string();
  }
  ```
* **Diagnóstico:** Implementación manual de comprobación de paridad mediante `% 2 == 0`. Rust 1.88 provee el método estabilizado `.is_multiple_of(2)` para enteros primitivos (`usize`).
* **Corrección idiomática:**
  ```rust
  if before.matches('"').count().is_multiple_of(2) && before.matches(''').count().is_multiple_of(2) {
      val.truncate(hash);
      val = val.trim().to_string();
  }
  ```

---

#### Categoría 5: `clippy::single_element_loop` (2 errores)
* **Archivo y Líneas:** `src/vur_client.rs:826:9` y `src/vur_client.rs:847:9` (en `mod tests`)
* **Snippet problemático:**
  ```rust
  for name in ["hello"] {
      repo.materialize_pkg(name)?;
      repo.project_pkg(&master, name, false)?;
  }
  ```
* **Diagnóstico:** Un ciclo `for` iterando sobre un arreglo estático de un único elemento (`["hello"]`) es superfluo.
* **Corrección idiomática:**
  ```rust
  let name = "hello";
  repo.materialize_pkg(name)?;
  repo.project_pkg(&master, name, false)?;
  ```

---

#### Categoría 6: `clippy::useless_format` (2 errores)
* **Archivo y Líneas:** `src/metadata.rs:379:20` y `src/metadata.rs:391:20` (en `mod tests`)
* **Snippet problemático:**
  ```rust
  let json = format!(
      r#"{{"format_version":1,"pkgname":"foo","version":"1.0","revision":1,"archs":["x86_64"],"checksum":["sha256:aa"],"subpackages":[{"pkgname":"foo"}]}}"#
  );
  ```
* **Diagnóstico:** Invocación de `format!(...)` sobre una cadena cruda sin especificadores de formato `{}` interactivos.
* **Corrección idiomática:**
  ```rust
  let json = r#"{"format_version":1,"pkgname":"foo","version":"1.0","revision":1,"archs":["x86_64"],"checksum":["sha256:aa"],"subpackages":[{"pkgname":"foo"}]}"#.to_string();
  ```

---

#### Categoría 7: `clippy::collapsible_if` (1 error)
* **Archivo y Línea:** `src/vur_client.rs:484:13`
* **Snippet problemático:**
  ```rust
  if master_srcpkgs.join(pkgname).symlink_metadata().is_err() {
      if master_srcpkgs.parent().and_then(|p| p.file_name()).map(|n| n == "void-packages").unwrap_or(false)
          || master_srcpkgs.join("../.git").exists()
      {
          let _ = Command::new(&self.git_bin)
              .args(["checkout", "--", &format!("srcpkgs/{}", pkgname)])
              .current_dir(master_srcpkgs.parent().unwrap_or(Path::new(".")))
              .output();
      }
  }
  ```
* **Diagnóstico:** Dos bloques `if` anidados inmediatamente sin ramas `else` intermedias, combinables con `&&`.
* **Corrección idiomática:**
  ```rust
  let is_void_packages = master_srcpkgs.parent().and_then(|p| p.file_name()).map(|n| n == "void-packages").unwrap_or(false)
      || master_srcpkgs.join("../.git").exists();

  if master_srcpkgs.join(pkgname).symlink_metadata().is_err() && is_void_packages {
      let _ = Command::new(&self.git_bin)
          .args(["checkout", "--", &format!("srcpkgs/{}", pkgname)])
          .current_dir(master_srcpkgs.parent().unwrap_or(Path::new(".")))
          .output();
  }
  ```

---

#### Categoría 8: `clippy::while_immutable_condition` (1 error)
* **Archivo y Línea:** `src/vur_client.rs:593:15`
* **Diagnóstico y Corrección:** Ver análisis detallado y parche en la **Sección 3**.

---

#### Categoría 9: `clippy::while_let_on_iterator` (1 error)
* **Archivo y Línea:** `src/vur_client.rs:572:17`
* **Snippet problemático:**
  ```rust
  if trimmed.contains("()") {
      while let Some(l) = lines.next() {
          if l.trim() == "}" { break; }
      }
  }
  ```
* **Diagnóstico:** Un ciclo `while let Some(...) = lines.next()` sobre un iterador debe escribirse como un bucle `for` consumiendo el préstamo mutable con `.by_ref()`.
* **Corrección idiomática:**
  ```rust
  if trimmed.contains("()") {
      for l in lines.by_ref() {
          if l.trim() == "}" { break; }
      }
  }
  ```

---

#### Categoría 10: `clippy::obfuscated_if_else` (1 error)
* **Archivo y Línea:** `src/search.rs:69:27`
* **Snippet problemático:**
  ```rust
  repo: repo_raw.is_empty().then(|| "official".to_string()).unwrap_or(repo_raw),
  ```
* **Diagnóstico:** Uso de `.then(...).unwrap_or(...)` que oscurece el flujo lógico condicional en lugar de un `if ... else ...` simple.
* **Corrección idiomática:**
  ```rust
  repo: if repo_raw.is_empty() {
      "official".to_string()
  } else {
      repo_raw
  },
  ```

---

#### Categoría 11: `clippy::manual_unwrap_or_default` (1 error)
* **Archivo y Línea:** `src/search.rs:94:13`
* **Snippet problemático:**
  ```rust
  match repo.load_index(&mut cache, ttl) {
      Ok(v) => v,
      Err(_) => Vec::new(),
  }
  ```
* **Diagnóstico:** `match` con retorno de valor o `Default::default()` simplificable con `.unwrap_or_default()`.
* **Corrección idiomática:**
  ```rust
  repo.load_index(&mut cache, ttl).unwrap_or_default()
  ```

---

#### Categoría 12: `clippy::unnecessary_cast` (1 error)
* **Archivo y Línea:** `src/signal.rs:135:22`
* **Snippet problemático:**
  ```rust
  extern "C" fn handle_signal(sig: i32) {
      GOT_SIGNAL.store(sig as i32, Ordering::SeqCst);
  ```
* **Diagnóstico:** Conversión redundante `sig as i32` cuando `sig` ya es de tipo `i32`.
* **Corrección idiomática:**
  ```rust
  GOT_SIGNAL.store(sig, Ordering::SeqCst);
  ```

---

#### Categoría 13: `clippy::needless_return` (1 error)
* **Archivo y Línea:** `src/lock.rs:37:13`
* **Snippet problemático:**
  ```rust
  return Ok(InstanceLock { _locked: file });
  ```
* **Diagnóstico:** `return` explícito al final del bloque principal de una función de expresión.
* **Corrección idiomática:**
  ```rust
  Ok(InstanceLock { _locked: file })
  ```

---

#### Categoría 14: `clippy::suspicious_open_options` (1 error)
* **Archivo y Línea:** `src/lock.rs:27:10`
* **Snippet problemático:**
  ```rust
  let file = std::fs::OpenOptions::new()
      .read(true)
      .write(true)
      .create(true)
      .open(&path)
  ```
* **Diagnóstico:** Apertura de archivo con `.write(true).create(true)` sin explicitar el comportamiento de truncado. Si otra instancia retiene el bloqueo, `lock.rs:39-42` necesita leer el PID del archivo (`read_to_string`). Si se usara `.truncate(true)`, abrir el archivo destruiría el PID antes de verificar el lock. La omisión genera advertencia en Clippy.
* **Corrección idiomática:** Especificar explícitamente `.truncate(false)`:
  ```rust
  let file = std::fs::OpenOptions::new()
      .read(true)
      .write(true)
      .create(true)
      .truncate(false)
      .open(&path)
  ```

---

#### Categoría 15: `clippy::manual_pattern_char_comparison` (1 error)
* **Archivo y Línea:** `src/remove.rs:27:32`
* **Snippet problemático:**
  ```rust
  let name = t.split(|c| c == '<' || c == '>' || c == '=' || c == ' ').next().unwrap_or(t);
  ```
* **Diagnóstico:** Comparación manual de caracteres mediante clausura booleana en lugar de pasar un arreglo estático `['<', '>', '=', ' ']`.
* **Corrección idiomática:**
  ```rust
  let name = t.split(['<', '>', '=', ' ']).next().unwrap_or(t);
  ```

---

## 3. Confirmación y Clasificación del Bug en `vur_client.rs:593`

### 3.1 Evidencia y Código Real

En `src/vur_client.rs:591-601`, dentro de la función `parse_template_text`:

```rust
// Manejar valores multilínea entre comillas dobles
let quote_count = buf.matches('"').count();
while quote_count % 2 == 1 {
    if let Some(next) = lines.next() {
        buf.push('
');
        buf.push_str(next);
        if next.contains('"') { break; }
    } else {
        break;
    }
}
```

### 3.2 Clasificación y Análisis del Fallo

* **¿Es un bug real de lógica?** **SÍ, ABSOLUTAMENTE.**
* **Diagnóstico:**
  1. **Invarianza del ciclo:** La variable `quote_count` se calcula **una sola vez** sobre el contenido inicial de `buf` (`line`). Dentro del cuerpo del bucle `while quote_count % 2 == 1`, **jamás se reasigna ni muta `quote_count`**.
  2. **Ruptura prematura en líneas intermedias:** El desarrollador intentó suplir la falta de recálculo con la cláusula `if next.contains('"') { break; }`. Esto introduce un comportamiento gravemente defectuoso:
     - Si la siguiente línea contiene comillas intermedias pares (ej. `next = "  echo "compiling" && foo"`), `next.contains('"')` evalúa a `true` y ejecuta `break;` **a pesar de que la comilla multilínea inicial sigue sin cerrarse**.
     - Si la comilla se encuentra escapada (`"`), `contains('"')` la cuenta erróneamente como cierre.
  3. **Omisión de comillas simples (`'`):** En plantillas de Void Linux (`void-packages`), variables como `distfiles`, `checksum`, `short_desc` y dependencias frecuentemente usan comillas simples multilínea (`'...'`). Este bucle ignora por completo las comillas simples, mientras que líneas más adelante (`src/vur_client.rs:629-637`) intentan manipular comillas simples que jamás fueron acumuladas en `buf`.
  4. **Bucle hasta EOF:** Si un template contiene una comilla de apertura desbalanceada por error o sintaxis peculiar, el bucle consume todas las líneas del archivo hasta `lines.next() == None`, arruinando el parseo de todas las variables restantes.

### 3.3 Impacto Funcional

Este bug afecta al **mecanismo de fallback estructurado R2 / A2**: cuando un repositorio VUR carece de `.VURINFO` o este es inválido, `vary` recurre a `parse_template_text()`. Debido a este bug:
- Paquetes con listas multilínea de `distfiles`, `checksum`, `hostmakedepends` o `depends` que contengan comillas internas son truncados o corrompidos.
- Variables críticas quedan vacías o unidas incorrectamente, provocando que el resolutor DAG descarte dependencias o falle en compilar con `xbps-src`.

### 3.4 Corrección Idiomática Propuesta

Se debe recalcular dinámicamente la paridad de comillas sobre `buf` y gestionar tanto comillas dobles como simples sin depender de flags estáticas:

```rust
// Reemplazo idiomático y robusto en src/vur_client.rs:591-601:
while buf.matches('"').count() % 2 == 1 {
    if let Some(next) = lines.next() {
        buf.push('
');
        buf.push_str(next);
    } else {
        break;
    }
}
```

*Nota para Hardening:* Para templates complejos con `"` o `'`, la solución de fondo requiere un tokenizador léxico que ignore comillas escapadas y soporte multilínea con comillas simples `'`.

---

## 4. Inventario y Auditoría de `unwrap()`, `expect()`, `panic!()`

Un escaneo exhaustivo sobre todo el árbol `src/*.rs` arrojó **100 ocurrencias**:
- **84 ocurrencias en código de pruebas** (`#[cfg(test)]` o funciones de test): Válidas para aserciones de pruebas.
- **16 ocurrencias en código de producción** (rutas operativas).

### 4.1 Matriz de Riesgo de las 16 Ocurrencias en Producción

| # | Archivo:Línea | Tipo | Código | Nivel de Riesgo | Justificación y Vector de Fallo |
|:---:|---|:---:|---|:---:|---|
| 1 | `src/install.rs:367` | `unwrap()` | `let entry = repos_conf.vur.get(repo).unwrap();` | **ALTO** | `repo` proviene del DAG de resolución (`Action::Install(VulBinary { repo })`). Si el repositorio fue eliminado de `repos.conf` concurrentemente o no coincide exactamente, paniquea durante la fase de instalación. |
| 2 | `src/config.rs:126` | `expect()` | `Self::new().expect("config defaults")` | **ALTO** | Se ejecuta en `Config::default()`. `Config::new()` falla con error si `dirs::home_dir()` retorna `None`. En entornos de chroot, contenedores mínimos o daemons sin `$HOME`, provoca un crash fatal inmediato. |
| 3 | `src/init.rs:24` | `unwrap()` | `sv_dir.to_str().unwrap()`, `service_link.to_str().unwrap()` | **MEDIO-ALTO** | Si `pkg_name` contiene secuencias de bytes que no son UTF-8 válido (legal en sistemas de archivos Linux), `to_str()` devuelve `None` y paniquea. Innecesario: `Command::args` acepta `&OsStr`. |
| 4 | `src/main.rs:37` | `expect()` | `setgroups(&[]).expect("failed to drop...")` | **MEDIO-ALTO** | En contenedores restringidos o con filtros seccomp que bloquean la modificación de grupos complementarios, aborta con pánico en vez de emitir un error limpio y salir con `exit(1)`. |
| 5 | `src/main.rs:38` | `expect()` | `setresgid(gid, gid, gid).expect(...)` | **MEDIO-ALTO** | Mismo vector: syscall rechazada por permisos/capacidades (`CAP_SETGID`) aborta bruscamente. |
| 6 | `src/main.rs:39` | `expect()` | `setresuid(uid, uid, uid).expect(...)` | **MEDIO-ALTO** | Syscall rechazada por falta de `CAP_SETUID` aborta con backtrace en vez de terminación controlada. |
| 7 | `src/vur_client.rs:496` | `unwrap()` | `dest.parent().unwrap().join(target)` | **MEDIO** | Si `dest` corresponde a una ruta en la raíz del filesystem o sin directorio padre (`parent() == None`), paniquea. Debe usar `.unwrap_or(Path::new("."))`. |
| 8 | `src/install.rs:156` | `unwrap()` | `let entry = repos_conf.vur.get(&repo.name).unwrap();` | **BAJO-MEDIO** | `repo` se construye a partir de las claves de `repos_conf`, pero asume atomicidad con respecto a modificaciones del archivo de configuración. |
| 9 | `src/command_line.rs:313` | `unwrap()` | `let v = value.unwrap();` (`--arch`) | **BAJO-MEDIO** | Acoplado implícitamente a `takes_value(arg) == TakesValue::Required`. Si en un refactor divergen `takes_value` y `handle_arg`, paniquea. |
| 10 | `src/command_line.rs:323` | `unwrap()` | `self.sudo_bin = value.unwrap().to_string()` | **BAJO-MEDIO** | Mismo acoplamiento implícito en flag `--sudo`. |
| 11 | `src/command_line.rs:324` | `unwrap()` | `value.unwrap().split_whitespace()...` | **BAJO-MEDIO** | Mismo acoplamiento implícito en flag `--sudoflags`. |
| 12 | `src/command_line.rs:325` | `unwrap()` | `self.git_bin = value.unwrap().to_string()` | **BAJO-MEDIO** | Mismo acoplamiento implícito en flag `--git`. |
| 13 | `src/command_line.rs:326` | `unwrap()` | `self.curl_bin = value.unwrap().to_string()` | **BAJO-MEDIO** | Mismo acoplamiento implícito en flag `--curl`. |
| 14 | `src/command_line.rs:227` | `unwrap()` | `let arg_str = split.next().unwrap();` | **BAJO** | `splitn(2, '=')` siempre garantiza al menos un elemento, incluso en strings vacíos. Inofensivo, aunque preferible `split_once('=')`. |
| 15 | `src/command_line.rs:239` | `unwrap()` | `chars.next().unwrap();` | **BAJO** | Protegido por `if arg.starts_with('-')`. Inofensivo. |
| 16 | `src/vur_client.rs:632` | `unwrap()` | `let quote = val.chars().next().unwrap();` | **BAJO** | Protegido por `starts_with('"') || starts_with(''')`. Inofensivo. |

---

### 4.2 Detalle y Remediación de Casos Críticos

#### 1. Crash en Transacción de Binarios (`src/install.rs:367`)
* **Código:**
  ```rust
  if let Some(r) = repos.iter().find(|r| &r.name == repo) {
      let entry = repos_conf.vur.get(repo).unwrap();
  ```
* **Remediación con Contexto:**
  ```rust
  if let Some(r) = repos.iter().find(|r| &r.name == repo) {
      let entry = repos_conf.vur.get(repo)
          .with_context(|| format!("repositorio VUR '{}' no encontrado en configuración", repo))?;
  ```

#### 2. Pánico en `Config::default()` sin `$HOME` (`src/config.rs:126`)
* **Código:**
  ```rust
  impl Default for Config {
      fn default() -> Self {
          Self::new().expect("config defaults")
      }
  }
  ```
* **Remediación:** En entornos sin `$HOME`, `Config::default()` debe inicializar rutas fallback (ej. `/tmp/vary` o directorio de trabajo actual) en lugar de abortar la ejecución:
  ```rust
  impl Default for Config {
      fn default() -> Self {
          Self::new().unwrap_or_else(|_| {
              let tmp = std::env::temp_dir().join("vary");
              Config {
                  cache_dir: tmp.join("cache"),
                  data_dir: tmp.join("data"),
                  config_dir: tmp.join("config"),
                  // ... campos con valores por defecto seguros
                  ..Config::default_fallback()
              }
          })
      }
  }
  ```

#### 3. Abandono Inseguro de Privilegios (`src/main.rs:37-39`)
* **Código:**
  ```rust
  if let (Some(uid), Some(gid)) = (target_uid, target_gid) {
      setgroups(&[]).expect("failed to drop supplementary groups");
      setresgid(gid, gid, gid).expect("failed to set gid");
      setresuid(uid, uid, uid).expect("failed to set uid");
  }
  ```
* **Remediación:** En operaciones de seguridad (drop privileges), un fallo no debe desatar un pánico no controlado con vuelco de memoria, ni continuar ejecutando como `root`. Debe emitir un error directo y salir con código `1`:
  ```rust
  if let (Some(uid), Some(gid)) = (target_uid, target_gid) {
      if let Err(e) = setgroups(&[]) {
          eprintln!("vary: error fatal abandonando grupos suplementarios: {}", e);
          std::process::exit(1);
      }
      if let Err(e) = setresgid(gid, gid, gid) {
          eprintln!("vary: error fatal cambiando GID a {}: {}", gid, e);
          std::process::exit(1);
      }
      if let Err(e) = setresuid(uid, uid, uid) {
          eprintln!("vary: error fatal cambiando UID a {}: {}", uid, e);
          std::process::exit(1);
      }
  }
  ```

---

## 5. Auditoría de Código Muerto y Redundante

### 5.1 Campos No Leídos
* **`CacheIndex.path: PathBuf` (`src/cache.rs:19`):** Se recibe en `load()`, se crea el directorio, pero jamás vuelve a leerse ni utilizarse en `store()`, `load_valid()` ni `invalidate_repo()`.
* **`InstalledDb.path: PathBuf` (`src/db.rs:27`):** Mismo comportamiento: tras inicializar el entorno `heed`, el campo queda inerte en memoria.

### 5.2 Métodos Huérfanos
* **`InstalledDb::get(&self, name: &str) -> Option<Entry>` (`src/db.rs:68`):** Método público para consultar un paquete por nombre; no se invoca en ningún módulo de búsqueda, instalación ni remoción.
* **`InstalledDb::names(&self) -> Vec<String>` (`src/db.rs:83`):** Método para listar todos los nombres de paquetes instalados; no se usa en ninguna operación.

### 5.3 Fragmentos Residuales del Refactor
* **`InstalledDb::save(&self) -> Result<()>` (`src/db.rs:51-53`):**
  ```rust
  pub fn save(&self) -> Result<()> {
      Ok(())
  }
  ```
  Contiene un cuerpo vacío `Ok(())`. Proviene de la versión previa donde `InstalledDb` persistía un archivo JSON en disco. Tras introducir `heed`, las transacciones commitean directamente sobre la base de datos binaria, dejando este método como un stub fantasma. En `src/remove.rs:33` se llama a `db.save()` creyendo erróneamente que realiza una operación de sincronización en disco.
* **Configuración muerta `max_concurrent_builds`:** Definida en `Config` (`src/config.rs:115`) y leída del archivo TOML (`src/config.rs:203`), pero **jamás se utiliza** en `src/install.rs` (el bucle de compilación sigue siendo un `for` estrictamente secuencial).
* **Archivo residual `guia` en la raíz:** Archivo de texto plano de 29.273 bytes sin extensión ni seguimiento en Git que contiene la especificación arquitectónica original del proyecto. Debe archivarse en `docs/` o eliminarse del working tree.

---

## 6. Auditoría de `Cargo.toml` y Dependencias

### 6.1 Dependencias Declaradas vs Dependencias Usadas

| Dependencia | Versión Declarada | Uso Real en Código | Estado / Diagnóstico |
|---|---|---|---|
| `ansiterm` | `0.12.2` | `src/config.rs`, `src/lib.rs` | Activa (formato de colores CLI) |
| `anyhow` | `1.0.100` | 25 archivos | Activa (manejo central de errores) |
| `base64` | `0.22` | `src/vup_index.rs`, `src/vur_client.rs` | Activa (decodificación de claves RSA en `.plist`) |
| `dirs` | `6.0.0` | `src/config.rs`, `src/lib.rs` | Activa (resolución de `$HOME`) |
| `petgraph` | `0.6` | `src/resolver.rs` | Activa (resolución DAG topológica) |
| `serde` | `1.0.228` | 7 archivos | Activa (serialización/deserialización) |
| `serde_json` | `1.0.149` | `src/metadata.rs`, `src/vup_index.rs` | Activa (parseo `.VURINFO` e `index.json`) |
| `sha2` | `0.10` | `src/vur_client.rs` | Activa (cálculo de huellas criptográficas) |
| `tempfile` | `3.24.0` | 7 archivos | Activa (archivos temporales y overlays) |
| `toml` | `0.9.10` | `src/config.rs`, `src/reposconf.rs` | Activa (parseo de configuraciones TOML) |
| `heed` | `0.20.0` | `src/cache.rs`, `src/db.rs` | **Cuestionable:** LMDB binario |
| `tracing` | `0.1` | 16 archivos | Activa (instrumentación y logs) |
| `tracing-appender` | `0.2` | `src/logging.rs` | Activa (rotación/salida no bloqueante a `vary.log`) |
| `tracing-subscriber` | `0.3` | `src/logging.rs` | Activa (filtros de entorno para tracing) |
| `mimalloc` (musl) | `0.1` | `src/main.rs` | Activa (allocator rápido para musl) |
| `nix` (unix) | `0.29` | 4 archivos | Activa (signals, locks `flock`, drop privileges) |

### 6.2 El Caso de `heed` (LMDB) vs `installed.json` (A5)
* `heed` fue introducido en un intento de optimizar el acceso concurrente a la base de datos de paquetes y caché.
* **Problemas identificados:**
  1. Dependencia de código en C (LMDB) que incrementa tiempos de compilación y complejidad en builds cross-target de Void Linux (glibc y musl).
  2. Uso de bloques `unsafe`: `unsafe { EnvOpenOptions::new().open(&path)? }` tanto en `cache.rs` como en `db.rs`.
  3. Violación del requisito contractual **A5**: La especificación exige explícitamente `installed.json` legible con formato JSON, timestamp en milisegundos (`build_date: u64`) y migración tolerante.
  4. Borrado destructivo: `InstalledDb::load` ejecuta `std::fs::remove_file(&path)` si encuentra un archivo preexistente en la ruta (destruyendo cualquier `installed.json` previo sin migrar).
* **Conclusión:** Si se implementa fielmente el requisito A5 con `installed.json` y serde_json, y la caché de metadatos se gestiona con archivos JSON o bincode atómicos, **`heed` queda 100% obsoleto** y debe ser removido de `Cargo.toml`.

### 6.3 Dependencias Faltantes para Requisitos Contractuales

1. **Requisito A3 (Diff Pager en `src/upgrade.rs`):**
   - *Funcionalidad esperada:* Generar diffs coloreados unificados entre la versión anterior de un template local y la versión entrante de un VUR antes de compilar.
   - *Dependencia requerida:* **`diffy = "0.4"`** (o equivalente de diffing puro en Rust).
   - *Estado actual:* **Ausente en `Cargo.toml`**.
2. **Requisito A7 (Asincronía Visual y Spinners):**
   - *Funcionalidad esperada:* Monitoreo visual con spinner terminal no bloqueante durante la compilación con `xbps-src` y clonado git.
   - *Dependencia requerida:* **`indicatif = "0.17"`**.
   - *Estado actual:* **Ausente en `Cargo.toml`**.

### 6.4 Auditoría de MSRV (Minimum Supported Rust Version)
* En `Cargo.toml`: `rust-version = "1.88"`.
* En `.github/workflows/ci.yml`: `dtolnay/rust-toolchain@1.88`.
* **Evaluación:** El MSRV está debidamente unificado en `1.88`. La justificación es técnica y rigurosa: la dependencia transitiva `time@0.3.x` (usada indirectamente) exige Rust 1.88+. Ambos archivos respetan la directiva de `AGENTS.md`.

---

## 7. Cobertura de Pruebas

El repositorio cuenta actualmente con **95 pruebas** en la suite `cargo test`:
- **91 pruebas pasando exitosamente.**
- **4 pruebas ignoradas (`#[ignore]`):** Corresponden a integración en un sistema Void Linux real con binarios `xbps-query` instalados (`integracion_query_architecture_real`, `integracion_query_installed_real`, `integracion_query_manual_real`, `integracion_search_remote_real`).

### 7.1 Distribución de Pruebas por Módulo

| Módulo | Tests Unitarios | Estado de Cobertura |
|---|:---:|:---:|
| `metadata.rs` | 20 | Excelente (validación estricta de esquemas .VURINFO v1) |
| `xbps.rs` | 19 (4 ign) | Buena (parseo de salidas de xbps, mockeado) |
| `resolver.rs` | 11 | Excelente (DAG topológico, diamantes, ciclos, provides) |
| `vur_client.rs` | 7 | Media (clones git, layouts de repositorios) |
| `util.rs` | 6 | Buena (confirmaciones interactivas, manejo de EOF) |
| `elevate.rs` | 5 | Buena (sudo, doas, run0, root directo) |
| `reposconf.rs` | 5 | Buena (parseo TOML, orden de prioridad) |
| `vup_index.rs` | 5 | Buena (parseo index.json, claves RSA plist) |
| `command_line.rs` | 4 | Media (solo prueba subcomando `--repo add`) |
| `signal.rs` | 4 | Buena (registro de PIDs y mounts, limpieza) |
| `bootstrap.rs` | 2 | Baja |
| `keys.rs` | 2 | Baja |
| `lock.rs` | 2 | Media (adquisición concurrente y liberación) |
| `masterdir.rs` | 2 | Baja (consistencia de rutas y drop de guard) |
| `logging.rs` | 1 | Baja (idempotencia de inicialización) |
| **Módulos Críticos Huérfanos (0 tests)** | **0** | **CRÍTICA (Completamente desprotegidos)** |

### 7.2 Módulos Críticos Sin Ninguna Prueba Unitaria (0 Tests)

1. **`src/db.rs` (0 tests):** La base de datos local de paquetes instalados no tiene ni una sola prueba para `load()`, `upsert()`, `remove()`, `retain()`, ni `entries_snapshot()`.
2. **`src/cache.rs` (0 tests):** La caché de metadatos no tiene pruebas para TTL, invalidación selectiva (`invalidate_repo`), ni persistencia.
3. **`src/search.rs` (0 tests):** El motor de búsqueda unificada (repos oficiales + VURs) no posee pruebas para filtrado, ordenamiento ni caché en memoria.
4. **`src/upgrade.rs` (0 tests):** El pipeline de `-Syu` carece de pruebas unitarias para la detección de nuevas versiones y cálculo de deltas.
5. **`src/install.rs` (0 tests):** El orquestador de transacciones no posee pruebas para la agrupación en batch ni la resolución de flags.
6. **`src/repo.rs` (0 tests):** Los subcomandos `add`, `list`, `remove` y `rekey` no tienen pruebas de ejecución.
7. **`src/remove.rs` (0 tests):** La desinstalación y limpieza de la base de datos carece de pruebas.
8. **`src/config.rs` (0 tests):** La resolución jerárquica de configuración (defaults -> archivo TOML -> CLI) no está testeada.
9. **`src/init.rs` (0 tests):** Detección de init (runit vs dinit) no testeada.
10. **`src/review.rs` (0 tests):** Paginación interactiva y fallback no-TTY sin pruebas directas.

---

## 8. Matriz Consolidada de Hallazgos [H-601] a [H-607]

### [H-601] 57 errores de compilación reportados por `cargo clippy --all-targets -- -D warnings`
- **Severidad:** High (Bloqueante de CI/Build)
- **Módulos:** 11 módulos (`help.rs`, `vur_client.rs`, `cache.rs`, `db.rs`, `lock.rs`, `metadata.rs`, `remove.rs`, `repo.rs`, `search.rs`, `signal.rs`).
- **Descripción:** Clippy falla con código 101 debido a 57 errores bajo directiva `-D warnings`. Destacan 35 `print_literal`, 4 `manual_flatten`, 3 `dead_code`, 2 `manual_is_multiple_of`, 2 `useless_format`, 2 `single_element_loop`, 1 `collapsible_if`, 1 `while_let_on_iterator`, 1 `obfuscated_if_else`, 1 `manual_unwrap_or_default`, 1 `unnecessary_cast`, 1 `needless_return`, 1 `suspicious_open_options` y 1 `manual_pattern_char_comparison`.
- **Impacto:** Impide la compilación en pipelines de CI estrictos y desluce la calidad del código.
- **Remediación:** Aplicar las correcciones idiomáticas detalladas en la Sección 2.

---

### [H-602] Bug lógico en `src/vur_client.rs:593` rompe parseo de templates multilínea en fallback
- **Severidad:** Critical
- **Módulo:** `src/vur_client.rs:591-601`
- **Descripción:** `while quote_count % 2 == 1` opera sobre una variable invariante en el bucle (`while_immutable_condition`). Se detiene prematuramente en líneas intermedias que contienen comillas internas o comillas escapadas, omite comillas simples y consume archivos enteros hasta EOF ante sintaxis irregular.
- **Impacto:** Inutiliza el fallback de parseo de Bash (R2 / A2) para paquetes de monorepos o repositorios VUR sin `.VURINFO`, provocando fallos silenciosos en la resolución de dependencias.
- **Remediación:** Recalcular dinámicamente la paridad en `buf` o implementar una máquina de estados para comillas escapadas (`"` y `'`).

---

### [H-603] Riesgo de pánico en tiempo de ejecución por uso de `unwrap()` en rutas alcanzables
- **Severidad:** High
- **Módulo:** `src/install.rs:367`, `src/config.rs:126`, `src/init.rs:24`, `src/vur_client.rs:496`
- **Descripción:** Cuatro llamadas a `unwrap()` y `expect()` en rutas operativas sin validación previa segura:
  - `repos_conf.vur.get(repo).unwrap()` en `install.rs:367`.
  - `Config::default()` invocando `Self::new().expect(...)` que paniquea si no existe `$HOME`.
  - `to_str().unwrap()` sobre rutas de servicios en `init.rs:24`.
  - `dest.parent().unwrap()` en `vur_client.rs:496`.
- **Impacto:** Crash fatal de `vary` con vuelco de pila y salida abrupta ante entradas válidas de usuario o entornos de ejecución restringidos.
- **Remediación:** Sustituir por `anyhow::Context` (`with_context`), manejo de fallbacks seguros y uso de `AsRef<OsStr>` en invocaciones de subprocesos.

---

### [H-604] Abandono inseguro de privilegios con `expect()` en `src/main.rs:37-39`
- **Severidad:** Medium
- **Módulo:** `src/main.rs:36-41`
- **Descripción:** `drop_privileges()` invoca `setgroups`, `setresgid` y `setresuid` usando `.expect(...)`.
- **Impacto:** En contenedores con restricciones seccomp o sin capacidades administrativas, aborta con pánico en vez de terminar limpiamente. Si alguien interceptara el pánico, el proceso continuaría en ejecución como `root`.
- **Remediación:** Manejar el error imprimiendo un mensaje claro en `stderr` y llamar inmediatamente a `std::process::exit(1)`.

---

### [H-605] Código muerto y stubs sin efecto operativo en `src/cache.rs` y `src/db.rs`
- **Severidad:** Low
- **Módulo:** `src/cache.rs:19`, `src/db.rs:27, 51, 68, 83`, `src/config.rs:115`
- **Descripción:** Campos no leídos `path`, métodos no usados `get()` y `names()`, stub vacío `save() -> Ok(())` en `InstalledDb`, y configuración inerte `max_concurrent_builds`.
- **Impacto:** Genera advertencias del compilador (`dead_code`), confunde al mantenedor sobre la verdadera persistencia de datos y evidencia refactorizaciones incompletas.
- **Remediación:** Purgar campos y métodos obsoletos, o integrarlos a los subcomandos que los requieren.

---

### [H-606] Inconsistencias de dependencias en `Cargo.toml`: uso de `heed` vs ausencia de `diffy` e `indicatif`
- **Severidad:** Medium
- **Módulo:** `Cargo.toml`
- **Descripción:** `Cargo.toml` incluye `heed` (LMDB) con dependencias C/unsafe que contradicen el requisito A5 (`installed.json`), mientras que carece por completo de las dependencias requeridas para cumplir los requisitos contractuales A3 (`diffy` para diff interactivo) y A7 (`indicatif` para spinners en terminal).
- **Impacto:** Incumplimiento contractual de especificaciones A3 y A7; sobrecarga de compilación con bibliotecas C externas.
- **Remediación:** Incorporar `diffy = "0.4"` e `indicatif = "0.17"`. Evaluar la remoción de `heed` al consolidar `installed.json`.

---

### [H-607] Vacío crítico de cobertura de pruebas en 10 módulos centrales
- **Severidad:** High
- **Módulo:** `db.rs`, `cache.rs`, `search.rs`, `upgrade.rs`, `install.rs`, `repo.rs`, `remove.rs`, `config.rs`, `init.rs`, `review.rs`.
- **Descripción:** El 33% de los módulos de la base de código no tiene ni una sola prueba unitaria. Entre ellos se encuentran componentes críticos que modifican el estado del sistema y la base de datos persistente.
- **Impacto:** Imposibilidad de detectar regresiones en operaciones transaccionales, consultas de paquetes y actualizaciones del sistema.
- **Remediación:** Crear suites de pruebas unitarias modulares con mocks para las operaciones de base de datos, caché y búsqueda.
