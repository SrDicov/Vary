# AUDIT REPORT — Subagente SA-7: Regresión Funcional y Escenarios de Humo

**Fecha de emisión:** 2026-09-06  
**Subagente auditor:** SA-7 (Regresión Funcional y Escenarios de Humo)  
**Objetivo:** Auditar la integridad de las funcionalidades críticas de `vary`, identificar regresiones funcionales o de seguridad, y definir la suite formal de pruebas de humo para ejecución y validación en entornos Void Linux reales.  
**Ruta del informe:** `audit/subagents/SA-7.md`  

---

## 1. Resumen Ejecutivo

El presente reporte consolida la auditoría de regresión funcional sobre el núcleo operativo de `vary`, cubriendo:
1. **Motor de resolución DAG (`src/resolver.rs`):** La estructura del grafo en [`petgraph::graphmap::DiGraphMap`](file:///run/media/dicov/LudoDrive/dicov-op/Vary/src/resolver.rs#L47) y sus invariantes matemáticas (detección temprana de ciclos, deduplicación en diamante, prioridad de paquetes oficiales como nodos hoja sin recursión, resolución de paquetes virtuales vía `provides` y filtrado estricto de arquitecturas incompatibles) se encuentran **plenamente verificadas** con cobertura exhaustiva de pruebas unitarias puras.
2. **Protocolo de repositorios (`src/repo.rs`, `src/reposconf.rs`, `src/vur_client.rs`, `src/vup_index.rs`):** Los subcomandos `add`, `list`, `remove` y `rekey` están implementados consistentemente. La autodetección de rama mediante `git ls-remote --symref` opera sin checkout local. El adaptador de índices VUP procesa índices remotos y certificados PEM embebidos en XML plist.
3. **Abstracción de elevación (`src/elevate.rs`):** Aunque el módulo `elevate` proporciona una abstracción limpia (soporte agnóstico de `sudo`, `doas`, `run0` y ejecución directa como `root`), persisten **violaciones directas** en módulos secundarios (notablemente en `src/init.rs`), donde `Command::new("sudo")` sigue hardcodeado.
4. **Hallazgos de regresión funcional y riesgos en runtime:** Se identifican 6 nuevos hallazgos estructurados ([H-002] a [H-007]), destacando el riesgo crítico de pérdida de datos de `installed.json` durante actualizaciones y el bloqueo indiscriminado de operaciones de lectura por el lockfile global.

---

## 2. Auditoría del Motor de Resolución DAG (`src/resolver.rs`)

El motor de resolución DAG es una implementación funcional pura sin efectos colaterales de I/O, parametrizada mediante el trait [`PackageSource`](file:///run/media/dicov/LudoDrive/dicov-op/Vary/src/resolver.rs#L86-L100).

```
[Target Request] ──> [Official Exists?] ──(Sí)──> [Action::Install(Official)] (Hoja: stop)
                            │ (No)
                            ▼
                    [VUR Exact Match?] ──(No)──> [VUR Provides Match?]
                            │ (Sí)                      │ (Sí)
                            └─────────────┬─────────────┘
                                          ▼
                               [Arch Supported?] ──(No)──> [Error Incompatible Arch]
                                          │ (Sí)
                                          ▼
                              [Vul Binary Available?]
                                 ├──(Sí)──> [Action::Install(VulBinary)] (Hoja: stop)
                                 └──(No)──> [Action::Build] ──> [Expand Depends & Recurse]
```

### 2.1. Detección de Ciclos Circulares con Mensaje de Flechas

- **Comportamiento:** Antes de expandir recursivamente un nodo de tipo `Action::Build`, `Ctx::resolve_name` inspecciona la pila de llamadas activas `self.stack` (conjunto "gris"). Si el paquete ya se encuentra en la pila, se formatea la trayectoria completa usando flechas ` -> ` y se aborta con error explicativo. Además, `Ctx::finish` aplica `petgraph::algo::toposort` como red de seguridad residual.
- **Evidencia en código (`src/resolver.rs:209-217`):**
  ```rust
  // Punto 6: el nombre ya está en el camino de expansión activo -> ciclo.
  // Se lista desde su primera aparición: "a -> b -> a" (a depende de b
  // depende de a).
  if let Some(pos) = self.stack.iter().position(|n| n.as_str() == name) {
      let mut cycle = self.stack[pos..].join(" -> ");
      cycle.push_str(" -> ");
      cycle.push_str(name);
      bail!("ciclo de dependencias detectado: {}", cycle);
  }
  ```
- **Red de seguridad en `finish` (`src/resolver.rs:297-298`):**
  ```rust
  let order = toposort(&self.graph, None)
      .map_err(|_| anyhow::anyhow!("ciclo de dependencias residual tras la expansión"))?;
  ```
- **Test unitario asociado (`src/resolver.rs:506-517`):**
  ```rust
  #[test]
  fn ciclo_reporta_error_con_flechas() {
      let mut a = vur_info("a", &["x86_64"]);
      a.depends = strs(&["b"]);
      let mut b = vur_info("b", &["x86_64"]);
      b.depends = strs(&["a"]);
      let src = MockSource {
          vur: [("a", ("vur-main", a)), ("b", ("vur-main", b))].into(),
          ..MockSource::default()
      };
      let err = resolve(&targets(&["a"]), &src, &ResolveOptions::default()).unwrap_err();
      assert!(err.to_string().contains("->"), "mensaje sin flechas: {err}");
  }
  ```
- **Evaluación:** ✅ **CONFORME**. Formato exacto garantizado (`ciclo de dependencias detectado: a -> b -> a`).

---

### 2.2. Resolución de Dependencias en Diamante y Deduplicación

- **Comportamiento:** En estructuras en diamante (`root -> [x, y]`, donde tanto `x` como `y` dependen de `z`), el resolver utiliza `self.index: HashMap<String, u32>` como conjunto "negro" (paquetes ya procesados). Si `name` ya está registrado, enlaza la arista en el grafo dirigido hacia el dependiente actual (`self.link(idx, parent)`) y retorna inmediatamente sin duplicar el paquete en la lista de items ni re-expandir dependencias.
- **Evidencia en código (`src/resolver.rs:218-224`):**
  ```rust
  // Punto 4: ya resuelto por otro camino (diamante): reutilizar nodo.
  if let Some(&idx) = self.index.get(name) {
      if let Some(parent) = dependent {
          self.link(idx, parent);
      }
      return Ok(idx);
  }
  ```
- **Dirección de aristas y toposort (`src/resolver.rs:189-203`):**
  ```rust
  fn link(&mut self, dep: u32, dependent: u32) {
      self.graph.add_edge(dep, dependent, ());
  }
  ```
  Al declarar la arista `dep -> dependent`, `toposort(&self.graph, None)` garantiza que `dep` precede estrictamente a `dependent` en la secuencia lineal del plan.
- **Test unitario asociado (`src/resolver.rs:445-471`):**
  `diamante_deduplica_paquete_compartido` valida que el nodo compartido `z` aparece **exactamente una vez** (`count == 1`) y su posición en `builds` es estrictamente previa a `x` y a `y`.
- **Evaluación:** ✅ **CONFORME**.

---

### 2.3. Prioridad de Paquetes Oficiales como Nodos Hoja

- **Comportamiento:** La precedencia de búsqueda evalúa primero `self.source.official_exists(name)`. Si el paquete reside en los repositorios oficiales de Void Linux, se emite de inmediato un nodo `Action::Install(BinarySource::Official)`. La expansión de dependencias se condiciona estrictamente a `Action::Build` (`src/resolver.rs:273`), de modo que los paquetes oficiales actúan como nodos hoja terminales, delegando la resolución de dependencias del sistema operativo a `xbps-install`.
- **Evidencia en código (`src/resolver.rs:226-230` y `272-273`):**
  ```rust
  // Punto 1a: oficial -> Install(Official), NODO HOJA (sin recursión).
  if self.source.official_exists(name) {
      let item = Self::official_item(name, &self.arch);
      return Ok(self.push_item(item, dependent));
  }
  ...
  // Punto 3: SOLO los builds expanden dependencias.
  if matches!(self.items[idx as usize].action, Action::Build) {
      ...
  ```
- **Test unitario asociado (`src/resolver.rs:475-490`):**
  `oficial_tiene_prioridad_y_es_hoja` configura un paquete oficial `tool` que en el VUR declara depender de una dependencia inválida (`fantasma`). El resolver emite `Install(Official)` sin consultar el VUR ni detonar errores por dependencias fantasma.
- **Evaluación:** ✅ **CONFORME**.

---

### 2.4. Resolución vía `provides`

- **Comportamiento:** Si un target o dependencia no coincide con un nombre oficial ni con un `pkgname` en los repositorios VUR registrados, el motor consulta `self.source.vur_lookup_provides(name)`. Si una plantilla declara dicho nombre en su array `provides` y es compatible con la arquitectura del sistema, se adopta como candidata.
- **Evidencia en código (`src/resolver.rs:239-245`):**
  ```rust
  if candidate.is_none() {
      if let Some(found) = self.source.vur_lookup_provides(name) {
          if self.arch_supported(&found.1) {
              candidate = Some(found);
          }
      }
  }
  ```
- **Test unitario asociado (`src/resolver.rs:492-503`):**
  `resuelve_via_provides` resuelve el requerimiento virtual `"libfoo.so.1"` hacia el paquete `"foo"`.
- **Matiz / Hallazgo identificado [H-007]:**
  En `PlanItem`, el campo `name` conserva el nombre virtual solicitado (`"libfoo.so.1"`), mientras que `info.pkgname` contiene el nombre real (`"foo"`). Al compilar en `src/install.rs:334` y registrar en base de datos en `src/install.rs:422`, se usa `item.name` en lugar de `item.info.pkgname`, lo que puede generar discrepancias al invocar `xbps-install` o indexar en el log de instalados.
- **Evaluación:** 🟡 **PARCIALMENTE CONFORME** (Resolución lógica correcta en el DAG, propagación imperfecta en `install.rs`).

---

### 2.5. Filtrado de Arquitecturas Incompatibles

- **Comportamiento:** La compatibilidad de arquitectura se valida contra `self.arch` (ej. `x86_64`, `aarch64`, `x86_64-musl`) aceptando coincidencias exactas o los comodines canónicos `"all"` y `"noarch"`. Si un paquete existe en el VUR pero no soporta la arquitectura local, el resolver no lo trata como "no encontrado", sino que emite un mensaje específico que identifica al VUR y la arquitectura requerida.
- **Evidencia en código (`src/resolver.rs:156-158` y `247-256`):**
  ```rust
  fn arch_supported(&self, info: &VurInfo) -> bool {
      info.archs.contains(&self.arch) || info.archs.iter().any(|a| a == "all" || a == "noarch")
  }
  ...
  if let Some((mrepo, _)) = self.source.vur_lookup_any_arch(name) {
      bail!(
          "el paquete '{}' existe en VUR(s) {} pero no está disponible para tu arquitectura '{}'",
          name,
          mrepo,
          self.arch
      );
  }
  bail!("paquete no encontrado en repos oficiales ni VURs: {}", name);
  ```
- **Test unitario asociado (`src/resolver.rs:559-583`):**
  `filtro_arquitectura_descarta_y_prueba_alternativa_o_falla` verifica tanto el mensaje de error explícito ante arquitectura incompatible como la selección de alternativas viables que satisfagan el requerimiento.
- **Evaluación:** ✅ **CONFORME**.

---

## 3. Auditoría del Protocolo de Gestión de Repositorios

### 3.1. Comandos de Repositorio (`src/repo.rs`, `src/reposconf.rs`)

| Comando | Operación | Implementación | Validación |
|---|---|---|---|
| `vary --repo add <url>` | Registro y clon de VUR | Deriva nombre con `derive_name_from_url` (`src/repo.rs:9-13`). Detecta rama remota sin clonar. Ejecuta partial clone (`--filter=blob:none --no-checkout --depth 1`). Lee llave pública si existe y escribe entrada en `repos.conf` con `priority = 100` por defecto. | ✅ Conforme |
| `vary --repo list` | Listado tabular de VURs | Lee `repos.conf`, ordena con `conf.sorted_by_priority()`. Imprime columnas `NAME`, `PRIO`, `URL`, `BINARY`, `(disabled)`. | ✅ Conforme |
| `vary --repo remove <name> [-p]` | Baja y purga | Elimina configuración y llave pública en `/etc/xbps.d/`. Remueve entrada de `repos.conf`. Si se pasa `-p` / `--purge`, elimina el directorio de clon (`vurs/<name>`); si no, informa su retención. | ✅ Conforme |
| `vary --repo rekey <name>` | Renovación de llave | Ejecuta `teardown_binary_repo` para eliminar llaves y configuraciones locales obsoletas, permitiendo que la próxima instalación solicite confirmación con el nuevo fingerprint. | ✅ Conforme |

### 3.2. Autodetección de Rama (`VurRepo::detect_default_branch`)

- **Módulo:** [`src/vur_client.rs:90-111`](file:///run/media/dicov/LudoDrive/dicov-op/Vary/src/vur_client.rs#L90-L111)
- **Mecanismo:**
  Ejecuta `git ls-remote --symref <url> HEAD` sin descargar blobs ni clonar el repositorio.
  Parsea la línea `ref: refs/heads/<rama>\tHEAD` extrayendo el nombre de la rama por defecto (`main`, `master`, etc.).
- **Fallback:**
  Si el comando falla o no hay conexión de red, devuelve `None`, lo que provoca que `repo_add` emita una advertencia y utilice `"main"` como fallback predeterminado (`src/repo.rs:50-57`).
- **Test:** `vur_client::tests::detect_default_branch_reads_remote_head` (pasa exitosamente).

### 3.3. Adaptador y Descarga de Índices VUP (`src/vup_index.rs`)

- **Módulo:** [`src/vup_index.rs:87-140`](file:///run/media/dicov/LudoDrive/dicov-op/Vary/src/vup_index.rs#L87-L140)
- **Mecanismo:**
  - Descarga `index.json` con `curl` hacia un archivo temporal y valida frescura mediante TTL (`cache_is_fresh`, default 86400s).
  - Si la descarga falla pero existe caché en disco, recurre a la caché previa emitiendo advertencia estructurada.
  - Convierte las entradas a `VurInfo` sintéticos en memoria mediante `to_vur_info`, interpretando versiones en formato `versión_revisión` (`src/vup_index.rs:219-251`).
  - Decodifica la llave RSA/PEM desde archivos `keys/*.plist` (formato XML plist de VUP) extrayendo el tag `<data>` y decodificando Base64 (`src/vup_index.rs:163-195`).

---

## 4. Auditoría de la Abstracción de Elevación de Privilegios (`src/elevate.rs`)

### 4.1. Comportamiento Multi-Wrapper y Detección de Root

La resolución de elevación en [`src/elevate.rs:60-86`](file:///run/media/dicov/LudoDrive/dicov-op/Vary/src/elevate.rs#L60-L86) opera con la siguiente precedencia:

1. **Wrapper configurado explícitamente (`configured_bin` no vacío):**
   Usa el binario y banderas pasadas por CLI (`--sudo <bin> --sudoflags <flags>`) o archivo de configuración (`vary.conf`).
2. **Ejecución como root (`running_as_root()`):**
   Comprueba si `/proc/self` pertenece al UID 0 (`src/elevate.rs:21-23`). Si es root, devuelve `None`, ejecutando el programa directamente sin ningún wrapper prefijado.
3. **Usuario sin privilegios sin configuración:**
   Autodetecta el primer binario disponible y ejecutable en `$PATH` entre la lista:
   `["sudo", "doas", "run0"]`.

### 4.2. Inventario Exhaustivo de Llamadas Hardcodeadas a `sudo` y Helpers

A pesar del diseño de `elevate.rs`, se identificaron múltiples llamadas donde se elude la abstracción o se hardcodean utilidades del sistema:

| Archivo:Línea | Código Real | Violación / Diagnóstico | Severidad |
|---|---|---|---|
| [`src/init.rs:15`](file:///run/media/dicov/LudoDrive/dicov-op/Vary/src/init.rs#L15) | `Command::new("sudo").args(["dinitctl", "enable", pkg_name])` | **Hardcoded `sudo`**. Falla en sistemas con `doas` o `run0`; falla o invoca sudo innecesariamente siendo root. | **High** [H-005] |
| [`src/init.rs:16`](file:///run/media/dicov/LudoDrive/dicov-op/Vary/src/init.rs#L16) | `Command::new("sudo").args(["dinitctl", "start", pkg_name])` | **Hardcoded `sudo`**. Mismo problema para iniciar servicio dinit. | **High** [H-005] |
| [`src/init.rs:23-25`](file:///run/media/dicov/LudoDrive/dicov-op/Vary/src/init.rs#L23-L25) | `Command::new("sudo").args(["ln", "-s", sv_dir, service_link])` | **Hardcoded `sudo`**. Mismo problema para enlazar servicio runit. | **High** [H-005] |
| [`src/masterdir.rs:73`](file:///run/media/dicov/LudoDrive/dicov-op/Vary/src/masterdir.rs#L73) | `Command::new("fuse-overlayfs")` | Helper FUSE hardcodeado sin configuración en `Config`. | **Low** [H-006] |
| [`src/masterdir.rs:225`](file:///run/media/dicov/LudoDrive/dicov-op/Vary/src/masterdir.rs#L225) | `Command::new("cp")` | Comando `cp` hardcodeado en ruta sin privilegios. | **Low** [H-006] |
| [`src/signal.rs:62`](file:///run/media/dicov/LudoDrive/dicov-op/Vary/src/signal.rs#L62) | `plain_status("fusermount3", ...)` | Helper de desmontaje FUSE hardcodeado. | **Low** [H-006] |
| [`src/signal.rs:65, 68`](file:///run/media/dicov/LudoDrive/dicov-op/Vary/src/signal.rs#L65) | `plain_status("umount", ...)` | Invocación no elevada de `umount` hardcodeada. | **Low** [H-006] |
| [`src/review.rs:61, 66`](file:///run/media/dicov/LudoDrive/dicov-op/Vary/src/review.rs#L61) | `Command::new("bat")` / `Command::new("less")` | Paginadores fijos; ignora la variable de entorno `$PAGER`. | **Low** [H-006] |

---

## 5. Suite Completa de Smoke Tests para Void Linux Real

A continuación se define la suite reproducible y formal de pruebas de humo para validar el comportamiento integral de `vary` en una instalación de Void Linux real (o contenedor oficial `voidlinux/voidlinux`).

```
                              SUITE DE SMOKE TESTS
┌─────────────────────────────────────────────────────────────────────────────┐
│  ST-1: Sincronización Global y Bulk Cache (-Syu con comodín '*')           │
│  ST-2: Build Interactivo desde Fuente (Review Gate A8 + xbps-src)           │
│  ST-3: Instalación No-TTY y Rechazo Seguro (Validación H-001 / --noconfirm) │
│  ST-4: Migración de Base de Datos Preexistente (Tolerancia installed.json) │
│  ST-5: Diamante Complejo con Subpaquetes Multinivel                         │
│  ST-6: Cancelación con SIGINT (Ctrl+C) y Desmontaje Limpio de OverlayFS     │
│  ST-7: Exclusión Mutua de Instancias Concurrentes (vary.lock)               │
└─────────────────────────────────────────────────────────────────────────────┘
```

---

### Escenario 1: `vary -Syu` global con caché masivo (A4 con comodín `'*'`)

- **Identificador:** `ST-1-SYNC-BULK-CACHE`
- **Objetivo:** Verificar la sincronización completa del sistema y repositorios VUR, la construcción del bulk cache de repositorios oficiales en RAM en una sola consulta, y la actualización en secuencia correcta.
- **Pre-condiciones:**
  1. Void Linux con conexión a repositorios oficiales en `/etc/xbps.d/`.
  2. Al menos un repositorio VUR registrado con `vary --repo add`.
  3. Paquete `vary` compilado e instalado en el PATH.
- **Comandos de ejecución:**
  ```bash
  # 1. Comprobación preliminar de sintaxis de xbps-query con comodín
  xbps-query --ignore-conf-repos --repository=https://repo-default.voidlinux.org/current -Rs '*' | head -n 5

  # 2. Ejecutar vary con depuración activada para validar bulk cache
  VARY_DEBUG=1 vary -Syu --noconfirm
  ```
- **Salida esperada:**
  1. La consola imprime:
     ```text
     :: Upgrading official packages...
     [*] Updating repository `https://repo-default.voidlinux.org/current/x86_64-repodata' ...
     :: Refreshing VUR repositories...
     VUR '<nombre-repo>' refreshed (<hash>)
     ```
  2. En el flujo de diagnóstico `VARY_DEBUG=1`:
     - Debe observarse: `bulk oficial: <N> paquetes` (donde `N > 10000`).
     - **Criterio de fallo:** Si el log indica `bulk oficial no disponible; path escalar` o `bulk oficial: 0 paquetes`, confirma el error por el uso de `""` en lugar de `'*'`.
  3. Código de salida: `0`.

---

### Escenario 2: Instalación de paquete desde fuente (`Action::Build`) validando review gate interactivo (A8) y compilación

- **Identificador:** `ST-2-BUILD-INTERACTIVE-REVIEW`
- **Objetivo:** Comprobar que en terminal TTY interactiva se lanza el paginador para revisión del template, se solicita confirmación explícita, se compila en masterdir aislado con `OverlayGuard`, y se ejecuta una única transacción de instalación.
- **Pre-condiciones:**
  1. Masterdir inicializado (`~/.cache/vary/void-packages` listo con `./xbps-src binary-bootstrap`).
  2. Terminal interactiva real (`[ -t 1 ]` es verdadero).
  3. Repositorio VUR local con una plantilla de prueba `smoke-app`.
- **Preparación del entorno:**
  ```bash
  mkdir -p /tmp/smoke-vur/srcpkgs/smoke-app
  cat << 'EOF' > /tmp/smoke-vur/srcpkgs/smoke-app/template
  pkgname=smoke-app
  version=1.0
  revision=1
  archs="all"
  short_desc="Smoke test interactive build"
  maintainer="Auditor <audit@example.com>"
  license="MIT"
  homepage="https://example.com"
  do_build() { :; }
  do_install() {
      vmkdir usr/bin
      cat << 'SH' > ${DESTDIR}/usr/bin/smoke-cmd
  #!/bin/sh
  echo "smoke-app: OK"
  SH
      chmod 755 ${DESTDIR}/usr/bin/smoke-cmd
  }
  EOF
  git -C /tmp/smoke-vur init -b main
  git -C /tmp/smoke-vur add .
  git -C /tmp/smoke-vur -c user.name="Audit" -c user.email="audit@test" commit -m "add smoke-app"
  vary --repo add /tmp/smoke-vur --name smoke-vur --branch main
  ```
- **Comandos de ejecución:**
  ```bash
  vary -S smoke-app
  ```
- **Salida esperada:**
  1. Se presenta el plan:
     ```text
     :: Packages to build (source):
       smoke-app/1.0_1
     Builds are sequential in MVP (xbps-src handles -j internally)
     ```
  2. **Review Gate:** Se abre automáticamente el paginador interactivo (`bat` o `less`) mostrando el contenido de `/tmp/smoke-vur/srcpkgs/smoke-app/template`.
  3. Al salir del paginador (pulsando `q`), el prompt interactivo pregunta:
     ```text
     Proceed with installation? [Y/n]
     ```
  4. Al pulsar `y` / Enter:
     - Log: `compilando smoke-app con xbps-src (aislado: true)...`
     - Se crea el paquete `smoke-app-1.0_1.<arch>.xbps` en `hostdir/binpkgs`.
     - Invocación única: `xbps-install -S --repository=... smoke-app`.
  5. Verificación post-instalación:
     ```bash
     smoke-cmd # Imprime "smoke-app: OK"
     xbps-query -l | grep smoke-app # Estado "ii smoke-app-1.0_1"
     ```
  6. Código de salida: `0`.

---

### Escenario 3: Instalación desde fuente en modo no interactivo/no-TTY (validación de H-001 y `--no-confirm`)

- **Identificador:** `ST-3-NON-TTY-EOF-REJECTION`
- **Objetivo:** Validar que las llamadas en entornos no interactivos (pipes, CI, cron) no queden bloqueadas en paginadores y que stdin cerrado (`/dev/null`) trate EOF como **denegación segura**, mientras que `--noconfirm` permita la instalación automatizada.
- **Pre-condiciones:**
  1. Mismo paquete `smoke-app` disponible en el VUR local.
- **Caso 3A: Desatendido con `--noconfirm`:**
  ```bash
  vary -S smoke-app --noconfirm
  ```
  - **Salida esperada:** No se lanza paginador interactivo, no hay prompts; compila, instala y retorna `0`.
- **Caso 3B: No-TTY sin `--noconfirm` (Validación de remediación H-001):**
  ```bash
  # Se inyecta /dev/null a stdin para simular pipe o stream cerrado
  vary -S smoke-app < /dev/null
  ```
  - **Salida esperada:**
    - Se imprime el template en texto plano directo a stdout (sin paginador).
    - Al llegar a `confirm("Proceed with installation?")`, se lee EOF (`n == 0`).
    - `confirm_from_reader` devuelve `Ok(false)`.
    - La ejecución **aborta de inmediato** con código de salida `1`.
    - **Criterio de aprobación:** `xbps-src` NO es ejecutado y el sistema no sufre modificaciones no supervisadas.

---

### Escenario 4: Upgrade desde un `installed.json` preexistente (validación de migración de DB sin pérdida de datos)

- **Identificador:** `ST-4-DB-MIGRATION-PERSISTENCE`
- **Objetivo:** Auditar la transición desde una base de datos legacy `installed.json` en formato JSON plano hacia la base de datos `heed` (LMDB), verificando que no se destruyan los registros históricos.
- **Pre-condiciones:**
  1. Base de datos sintética en formato JSON plano ubicada en la ruta configurada `~/.cache/vary/installed.json`.
- **Preparación del entorno:**
  ```bash
  mkdir -p ~/.cache/vary
  rm -rf ~/.cache/vary/installed.json
  cat << 'EOF' > ~/.cache/vary/installed.json
  {
    "legacy-vur-app": {
      "version": "1.0_1",
      "vur": "smoke-vur",
      "install_date": 1693958400,
      "install_type": "source"
    }
  }
  EOF
  ```
- **Comandos de ejecución:**
  ```bash
  # Ejecutar upgrade de vary
  vary -Syu --noconfirm
  ```
- **Resultado actual (Detección de fallo crítico [H-003]):**
  1. `InstalledDb::load` (`src/db.rs:35-37`) detecta que `path.is_file()` es verdadero y ejecuta:
     ```rust
     let _ = std::fs::remove_file(&path);
     ```
  2. El archivo JSON plano es **eliminado** y reemplazado por un directorio LMDB (`data.mdb`, `lock.mdb`).
  3. `legacy-vur-app` desaparece del catálogo de seguimiento de `vary`.
  4. La salida muestra `VUR packages up to date.` ignorando el paquete existente.
- **Comportamiento requerido tras remediación:**
  1. Si `path.is_file()` es verdadero, leer el JSON plano antes de reemplazar la ruta.
  2. Poblar la base de datos LMDB con las entradas deserializadas.
  3. Conservar un respaldo `installed.json.bak`.

---

### Escenario 5: Manejo de dependencias en diamante y múltiples niveles de subpaquetes

- **Identificador:** `ST-5-DIAMOND-SUBPACKAGES`
- **Objetivo:** Verificar que el grafo en diamante resuelve las dependencias en orden topológico estricto, compila el paquete raíz común una sola vez, y propaga los subpaquetes generados sin compilaciones redundantes.
- **Estructura del diamante de prueba:**
  ```
               [meta-app]
               /        \
              /          \
      [client-gui]    [client-cli]
              \          /
               \        /
            [lib-shared] ──(genera subpaquete: lib-shared-devel)
  ```
- **Preparación de plantillas VUR:**
  ```bash
  # 1. lib-shared (genera subpaquete lib-shared-devel)
  mkdir -p /tmp/smoke-vur/srcpkgs/lib-shared
  cat << 'EOF' > /tmp/smoke-vur/srcpkgs/lib-shared/template
  pkgname=lib-shared
  version=1.0
  revision=1
  archs="all"
  short_desc="Shared core library"
  maintainer="Audit <audit@test>"
  license="MIT"
  homepage="https://example.com"
  subpackages="lib-shared-devel"
  do_build() { :; }
  do_install() {
      vmkdir usr/lib
      touch ${DESTDIR}/usr/lib/libshared.so
  }
  lib-shared-devel_package() {
      short_desc="Shared core library - development files"
      pkg_install() {
          vmkdir usr/include
          touch ${PKGDESTDIR}/usr/include/shared.h
      }
  }
  EOF

  # 2. client-cli (depends on lib-shared)
  mkdir -p /tmp/smoke-vur/srcpkgs/client-cli
  cat << 'EOF' > /tmp/smoke-vur/srcpkgs/client-cli/template
  pkgname=client-cli
  version=1.0
  revision=1
  archs="all"
  depends="lib-shared>=1.0"
  short_desc="CLI client"
  maintainer="Audit <audit@test>"
  license="MIT"
  homepage="https://example.com"
  do_build() { :; }
  do_install() {
      vmkdir usr/bin
      touch ${DESTDIR}/usr/bin/client-cli
      chmod 755 ${DESTDIR}/usr/bin/client-cli
  }
  EOF

  # 3. client-gui (depends on lib-shared-devel)
  mkdir -p /tmp/smoke-vur/srcpkgs/client-gui
  cat << 'EOF' > /tmp/smoke-vur/srcpkgs/client-gui/template
  pkgname=client-gui
  version=1.0
  revision=1
  archs="all"
  depends="lib-shared-devel>=1.0"
  short_desc="GUI client"
  maintainer="Audit <audit@test>"
  license="MIT"
  homepage="https://example.com"
  do_build() { :; }
  do_install() {
      vmkdir usr/bin
      touch ${DESTDIR}/usr/bin/client-gui
      chmod 755 ${DESTDIR}/usr/bin/client-gui
  }
  EOF

  # 4. meta-app (target principal)
  mkdir -p /tmp/smoke-vur/srcpkgs/meta-app
  cat << 'EOF' > /tmp/smoke-vur/srcpkgs/meta-app/template
  pkgname=meta-app
  version=1.0
  revision=1
  archs="all"
  depends="client-cli client-gui"
  short_desc="Meta application package"
  maintainer="Audit <audit@test>"
  license="MIT"
  homepage="https://example.com"
  do_build() { :; }
  do_install() { :; }
  EOF

  # Commit en el repo
  git -C /tmp/smoke-vur add .
  git -C /tmp/smoke-vur -c user.name="Audit" -c user.email="audit@test" commit -m "add diamond packages"
  ```
- **Comando de ejecución:**
  ```bash
  vary -S meta-app --noconfirm
  ```
- **Salida esperada:**
  1. El plan topológico lista los builds en orden estricto de dependencias:
     ```text
     :: Packages to build (source):
       lib-shared/1.0_1
       client-cli/1.0_1
       client-gui/1.0_1
       meta-app/1.0_1
     ```
  2. `lib-shared` aparece una sola vez en el plan y se compila en primer lugar.
  3. El subpaquete `lib-shared-devel` es detectado e incorporado a la lista de instalación sin compilar nuevamente.
  4. La invocación final de instalación contiene todos los artefactos en una sola transacción:
     ```text
     xbps-install -S --repository=... lib-shared lib-shared-devel client-cli client-gui meta-app
     ```
  5. Código de salida: `0`.

---

### Escenario 6: Cancelación con Ctrl+C (SIGINT) a mitad de build de `xbps-src`, verificando terminación de subprocesos y desmontaje limpio de overlays

- **Identificador:** `ST-6-SIGINT-OVERLAY-CLEANUP`
- **Objetivo:** Validar que ante la interrupción por señal (SIGINT/SIGTERM), el hilo observador elimine el Process Group completo de `xbps-src` (incluyendo procesos nietos), ejecute el desmontaje de todos los puntos de OverlayFS registrados, y termine con código `130`.
- **Preparación de paquete con compilación lenta:**
  ```bash
  mkdir -p /tmp/smoke-vur/srcpkgs/slow-pkg
  cat << 'EOF' > /tmp/smoke-vur/srcpkgs/slow-pkg/template
  pkgname=slow-pkg
  version=1.0
  revision=1
  archs="all"
  short_desc="Slow build package"
  maintainer="Audit <audit@test>"
  license="MIT"
  homepage="https://example.com"
  do_build() {
      sleep 120
  }
  do_install() { :; }
  EOF
  git -C /tmp/smoke-vur add .
  git -C /tmp/smoke-vur -c user.name="Audit" -c user.email="audit@test" commit -m "add slow-pkg"
  ```
- **Script de prueba automatizado:**
  ```bash
  # 1. Lanzar vary en segundo plano
  vary -S slow-pkg --noconfirm &
  VARY_PID=$!

  # 2. Esperar 3 segundos a que xbps-src monte el overlay e inicie el sleep
  sleep 3

  # 3. Verificar que el overlay temporal esté montado
  grep "vary-merged" /proc/mounts || echo "Aviso: overlay no detectado o corriendo sin privilegios"

  # 4. Enviar SIGINT (Ctrl+C)
  kill -INT $VARY_PID

  # 5. Esperar finalización del proceso
  wait $VARY_PID
  EXIT_CODE=$?
  ```
- **Verificaciones post-cancelación:**
  1. **Código de salida:** `echo $EXIT_CODE` debe ser exactamente `130` (`128 + 2`).
  2. **Procesos huérfanos:**
     ```bash
     pgrep -f "sleep 120"
     ```
     Debe devolver vacío (código 1). Ni `xbps-src` ni `sleep` deben quedar en ejecución.
  3. **Montajes residuales:**
     ```bash
     grep "vary-merged" /proc/mounts
     ```
     Debe devolver vacío. Ningún punto de montaje de `OverlayGuard` debe quedar colgado en `/proc/mounts`.
  4. **Directorios temporales:** Los directorios `/tmp/vary-merged-*`, `/tmp/vary-upper-*` y `/tmp/vary-work-*` deben haber sido eliminados.

---

### Escenario 7: Detección de colisión de instancia con lockfile (`vary.lock`)

- **Identificador:** `ST-7-LOCK-COLLISION`
- **Objetivo:** Comprobar que dos procesos concurrentes de `vary` no puedan ejecutarse simultáneamente contra el mismo cache_dir, protegiendo las bases de datos y directorios de trabajo compartidos.
- **Comandos de ejecución:**
  ```bash
  # 1. Tomar lockfile simulando una instancia activa en segundo plano
  (
    exec 9> ~/.cache/vary/vary.lock
    flock -n 9
    echo "$$" >&9
    sleep 10
  ) &
  LOCK_HOLDER=$!
  sleep 1

  # 2. Intentar ejecutar una segunda instancia de vary
  vary -S smoke-app
  SECOND_EXIT=$?

  # 3. Esperar a que la primera instancia libere el lock
  wait $LOCK_HOLDER
  ```
- **Salida esperada:**
  1. La segunda instancia finaliza de inmediato con código de salida `1`.
  2. Mensaje exacto de error en stderr:
     ```text
     error: otra instancia de vary está en ejecución (pid <LOCK_HOLDER>); si no es así, borra ~/.cache/vary/vary.lock
     ```
  3. Una vez terminado el titular del lock, la ejecución subsecuente (`vary -V` o `vary -S smoke-app`) procede con éxito sin requerir intervención manual.

---

## 6. Matriz de Hallazgos [H-002] a [H-007]

A continuación se formalizan los hallazgos técnicos detectados durante la auditoría funcional:

### [H-002] `bulk_official_names()` pasa cadena vacía `""` en lugar del comodín canónico `'*'`
- **Severidad:** Medium
- **Módulo:** [`src/xbps.rs:274-275`](file:///run/media/dicov/LudoDrive/dicov-op/Vary/src/xbps.rs#L274-L275)
- **Descripción:** Al construir la consulta masiva rápida contra los repositorios remotos oficiales (`xbps-query -Rs`), el vector de argumentos añade `String::new()`:
  ```rust
  owned.push("-Rs".to_string());
  owned.push(String::new());
  ```
  En la herramienta `xbps-query`, una cadena vacía no constituye el comodín universal esperado para listar todos los paquetes; el comodín verificado y documentado en XBPS es `'*'`. Esto causa que en determinadas versiones o distribuciones de Void Linux la consulta masiva retorne un conjunto vacío, anulando la optimización A4 y degradando a consultas escalares individuales en cada validación.
- **Remediación:** Sustituir `String::new()` por `"*".to_string()`.

---

### [H-003] Destrucción de base de datos previa `installed.json` sin migración hacia LMDB
- **Severidad:** High
- **Módulo:** [`src/db.rs:35-37`](file:///run/media/dicov/LudoDrive/dicov-op/Vary/src/db.rs#L35-L37), [`src/config.rs:280`](file:///run/media/dicov/LudoDrive/dicov-op/Vary/src/config.rs#L280)
- **Descripción:** La ruta configurada para la base de datos de paquetes instalados es `~/.cache/vary/installed.json`. Al cambiar el motor de almacenamiento a LMDB (`heed`), `InstalledDb::load` ejecuta:
  ```rust
  if path.is_file() {
      let _ = std::fs::remove_file(&path);
  }
  ```
  Si un usuario actualiza `vary` teniendo un archivo de seguimiento previo en JSON, la función **elimina físicamente el archivo sin leer ni migrar sus datos**, creando un directorio LMDB con ese mismo nombre de archivo (`installed.json/data.mdb`). Como resultado, se pierde de manera irreversible todo el historial de paquetes instalados desde VUR, imposibilitando que `vary -Syu` identifique actualizaciones disponibles.
- **Remediación:**
  1. Si `path.is_file()` es verdadero, leer y deserializar el contenido JSON existente a un `HashMap<String, EntryLegacy>`.
  2. Renombrar o respaldar el archivo a `installed.json.legacy`.
  3. Inicializar el entorno LMDB en una ruta adecuada y volcar todas las entradas históricas en una transacción de escritura inicial.

---

### [H-004] Lockfile de instancia única adquirido antes de comandos informativos de solo lectura
- **Severidad:** Medium
- **Módulo:** [`src/lib.rs:90-96`](file:///run/media/dicov/LudoDrive/dicov-op/Vary/src/lib.rs#L90-L96)
- **Descripción:** `crate::lock::acquire(&config.cache_dir)` se invoca al inicio de `vary::run` antes de procesar argumentos en `run2()`. Si una instancia de `vary` está realizando una compilación prolongada en segundo plano (p. ej. un paquete que tarda 30 minutos), un usuario en otra terminal no puede ejecutar comandos triviales como `vary --version`, `vary --help` o búsquedas informativas como `vary -Ss <patrón>`, recibiendo el error de bloqueo de instancia.
- **Remediación:** Diferir la adquisición de `_instance_lock` hasta después de verificar si la operación es puramente de lectura informativa (`--help`, `--version`, `-Ss`, `-Si`).

---

### [H-005] Ganchos de servicio en `init.rs` hardcodean invocación a `sudo`
- **Severidad:** High
- **Módulo:** [`src/init.rs:15, 16, 23-25`](file:///run/media/dicov/LudoDrive/dicov-op/Vary/src/init.rs#L15-L25)
- **Descripción:** Tras la instalación de un paquete que provee servicios (`/etc/sv/<pkg>`), `post_install_hook` intenta habilitar o enlazar el servicio ejecutando directamente `Command::new("sudo")`. Esto ignora la abstracción central de `crate::elevate::elevate`, rompiendo la compatibilidad en sistemas donde se utiliza `opendoas`, `run0` o cuando `vary` se ejecuta directamente como usuario root.
- **Remediación:** Modificar la firma de `post_install_hook` para recibir `sudo_bin: &str` y `sudo_flags: &[String]`, y despachar los comandos a través de `crate::elevate::elevate`.

---

### [H-006] Acoplamiento rígido a paginadores y binarios auxiliares del sistema
- **Severidad:** Low
- **Módulo:** [`src/review.rs:61, 66`](file:///run/media/dicov/LudoDrive/dicov-op/Vary/src/review.rs#L61-L66), [`src/masterdir.rs:73, 225`](file:///run/media/dicov/LudoDrive/dicov-op/Vary/src/masterdir.rs#L73-L225), [`src/signal.rs:62, 65`](file:///run/media/dicov/LudoDrive/dicov-op/Vary/src/signal.rs#L62-L65)
- **Descripción:** El review interactivo hardcodea los binarios `"bat"` y `"less"` sin consultar la variable de entorno estándar `$PAGER`. De forma similar, `masterdir.rs` y `signal.rs` invocan `"cp"`, `"fuse-overlayfs"`, `"fusermount3"` y `"umount"` como cadenas fijas en lugar de permitir configuración o comprobación estructurada.
- **Remediación:**
  1. En `review.rs`, evaluar `std::env::var("PAGER")` antes de degradar a `bat` o `less`.
  2. En `masterdir.rs` y `signal.rs`, documentar o centralizar la resolución de binarios de soporte del sistema.

---

### [H-007] Paquetes resueltos por `provides` utilizan el nombre virtual en el plan de construcción
- **Severidad:** Low
- **Módulo:** [`src/install.rs:334, 422`](file:///run/media/dicov/LudoDrive/dicov-op/Vary/src/install.rs#L334-L422), [`src/resolver.rs:269`](file:///run/media/dicov/LudoDrive/dicov-op/Vary/src/resolver.rs#L269)
- **Descripción:** Cuando una dependencia se resuelve mediante `vur_lookup_provides`, `PlanItem.name` contiene el identificador virtual requerido (p. ej. `"libfoo.so.1"`), mientras que `PlanItem.info.pkgname` contiene el nombre real de la plantilla (p. ej. `"foo"`). Al compilar, `built_names.push(item.name.clone())` introduce el nombre virtual en la lista que posteriormente se pasa a `xbps-install --repository=...`, y en `InstalledDb` se guarda la entrada bajo el nombre virtual en vez del nombre canónico del paquete XBPS.
- **Remediación:** Utilizar siempre `item.info.pkgname` para las listas de instalación local y registros en la base de datos de paquetes construidos.

---

## 7. Conclusión y Recomendaciones de Implementación

1. **Estado General del Core:** El motor de resolución DAG en [`src/resolver.rs`](file:///run/media/dicov/LudoDrive/dicov-op/Vary/src/resolver.rs) y el manejo seguro de señales y procesos en [`src/signal.rs`](file:///run/media/dicov/LudoDrive/dicov-op/Vary/src/signal.rs) muestran un nivel de solidez arquitectónica excelente, cumpliendo los requisitos formales de ordenamiento topológico, aislamiento de procesos hijos por PGID y mitigación de ciclos.
2. **Prioridades Inmediatas de Remediación:**
   - **P1:** Corregir la migración de `installed.json` ([H-003]) para evitar pérdida de datos en usuarios existentes.
   - **P2:** Reemplazar las llamadas hardcodeadas a `sudo` en `src/init.rs` ([H-005]) integrando `crate::elevate::elevate`.
   - **P3:** Cambiar la cadena vacía por comodín `'*'` en `src/xbps.rs:275` ([H-002]) para que la caché en RAM opere en todos los entornos Void.
   - **P4:** Ajustar la posición de adquisición del lockfile en `src/lib.rs` ([H-004]) permitiendo operaciones de consulta concurrentes.
3. **Ejecución de Pruebas de Humo:** Se recomienda incorporar los escenarios ST-1 a ST-7 en el script `.github/scripts/verify-xbps.sh` de la integración continua (CI) para asegurar que ningún cambio futuro degrade las garantías funcionales de `vary`.
