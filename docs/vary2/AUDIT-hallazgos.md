# AUDIT — tabla de hallazgos (FASE 2: fusión y triaje)

Base auditada: `8c14628`. Fusión de los 7 dominios A1-A7 (subagents) +
recomprobación del 100 % de los S1 y ~20 % aleatorio del resto por el fusor.

Reglas aplicadas:
- Una afirmación que toque semántica de xbps/void-packages se verifica contra la
  fuente real (`/tmp/xbps-0.60.7/`) o el clon real (`~/.cache/vary/void-packages/`),
  nunca de memoria.
- Un hallazgo sin file:line + reproducción vuelve a su dominio.
- Lo que ya está diferido con trigger en `PROGRESS.md` §5.5 es **RECHAZADO** con
  motivo (no es un hallazgo: es una decisión cerrada).

Severidad: S1 crítico · S2 mayor · S3 menor · S4 eliminación · RECHAZADO.

---

## A2 — Procesos y shell

**ID: A2-01** · **S3** · `src/xbps.rs:284-290` (`search_official`)

```rust
    args.push("-Rs");
    args.push(pattern);          // ← sin "--", a diferencia de search_remote:236
```

- **Reproducción**: `vary search -- '--repository=file:///tmp/evil/repodata'`.
  `command_line.rs:254-256` deja en `targets` los que empiezan por `-` tras `--`;
  `search.rs` los une y `search_official` los inyecta en el getopt de `xbps-query`
  (`/tmp/xbps-0.60.7/bin/xbps-query/main.c:128`, `getopt_long` con permute).
- **Impacto**: `xbps-query` consulta el repo del atacante y sus resultados se
  presentan con `Rank::Official` (`search.rs:53-75`): manipulación de la búsqueda.
- **Fix**: `args.push("--")` antes del pattern (1 línea, 1 módulo).
- **Verificación**: test del argv + smoke en Void.

**ID: A2-02** · **S3** · `src/vur_client.rs:163-166` (`detect_default_branch`) → `:197` (`pull`)

```rust
            if !branch.is_empty() {
                return Some(branch.to_string());   // ref del remoto SIN validar
```

```rust
            .args(["fetch", "--depth", "1", "origin", branch])
```

- **Reproducción**: verificada en vivo — `git fetch --depth 1 origin --dry-run`
  consume `--dry-run` como opción de fetch (no como refspec): el `branch` viene
  del `git ls-remote --symref` del remoto (dato T3) o de `repos.conf` (dato T4).
- **Impacto**: opciones arbitrarias de `git fetch` desde un VUR hostil; en el
  mejor caso el índice diverge en silencio.
- **Fix**: `args(["fetch","--depth","1","origin","--", branch])` (verificado:
  `git fetch --depth 1 origin -- main` → rc 0) + validación del refname.
- **Verificación**: test de `detect_default_branch` + clon local fake.

**ID: A2-03** · **S3** · `src/install.rs:119-126` (`make_repo`) y 8 sitios análogos
(`upgrade.rs:250,352,394,425`, `why.rs:69`, `search.rs:89`, `challenge.rs:203`)

```rust
        path: config.vurs_dir().join(name),   // nombre crudo de repos.conf
```

- **Reproducción**: `~/.config/vary/repos.conf` (TOML) editado a mano con clave
  `../../../tmp/arbitrary` → `git -C /tmp/arbitrary`.
  `keys.rs:149 validate_repo_name` valida **solo** la entrada CLI
  (`repo add|rekey|re-trust|remove`); nunca la carga de la conf.
- **Impacto**: path traversal fuera de `~/.local/share/vary/`. Sin frontera de
  privilegio (mismo usuario), pero la defensa que ya existe no se aplica donde
  el dato es el objetivo.
- **Fix**: validar las claves en `ReposConf::load` (`reposconf.rs`) con el
  validador que ya existe (1 bucle, 1 módulo, 0 líneas nuevas de lógica).
- **Verificación**: test con `repos.conf` contaminado → error accionable.

**ID: A2-04** · **S4** · `src/xbps.rs:501-503` (`pkgdb_mode`)

```rust
    cmd.args(["-m", mode, pkg]);           // sin "--"
```

- **Reproducción**: `installed.json` editado a mano con un nombre `-a` →
  `upgrade.rs:314` → getopt de `xbps-pkgdb` (`bin/xbps-pkgdb/main.c:147`) lo
  parsea como opción. Los nombres de `.VURINFO` sí pasan `is_valid_pkgname`;
  los de `installed.json` no.
- **Impacto**: bajo (quien edita `installed.json` ya es dueño de la máquina),
  pero es el **único argv elevado sin `--`**.
- **Fix**: `cmd.args(["-m", mode]).arg("--").arg(pkg)` (1 línea).
- **Verificación**: test del argv.

**ID: A2-05** · **S4** · `src/vup_index.rs:108-111` (`fetch_index`)

```rust
        .args(["-fsSL", "--max-time", "60", "-o"])
        .arg(&tmp_path)
        .arg(url)                  // index_url de repos.conf, sin "--"
```

- **Reproducción**: `index_url: "-o /tmp/evil"` → curl lo interpreta como
  opción (curl parsea opciones en cualquier posición; `-K <archivo>` además).
  `validate_repository_url` solo se aplica a `binary_url` (`keys.rs:313,419`).
- **Impacto**: `repos.conf` editada a mano dirige curl con opciones arbitrarias.
- **Fix**: `.arg("--").arg(url)` (verificado: curl 8.23 honora `--` como fin de
  opciones → `-o` deja de interpretarse como opción) **+ exigir esquema
  `http(s)://`** para que una URL absurda falle con error claro y no como DNS.
- **Verificación**: test de `fetch_index` con url que empieza por `-`.

**ID: A2-06** · **S4** · `src/vur_client.rs:154,120,198`, `src/portdb.rs:533`,
`src/xbps.rs:61-65`

```rust
        let out = Command::new(git_bin).args([... "ls-remote", "--symref", "--", url, "HEAD"]).output();
```

- **Reproducción**: no hay espera con límite en todo `src/` (solo `sleep` de
  `signal.rs` y `--max-time 60` de curl). RMI en TCP medio abierto → `vary`
  bloquea en `child.wait()` para siempre; Ctrl-C solo lo mata en TTY
  (`signal.rs` mata el grupo; en cron no hay TTY).
- **Impacto**: no es un agujero de seguridad, es una suspensión indefinida en
  operaciones de red.
- **Fix**: `-c http.lowSpeedLimit=1 -c http.lowSpeedTime=60` en clone/fetch/
  ls-remote (barato y confiable para git-http(s)); NO inventar watchdog.
- **Verificación**: smoke con un remoto TCP estancado.

**Notas A2 (fuera del modelo de amenaza, no son hallazgos)**: `masterdir.rs:104`
(`show_pkg`) no registra el hijo en el observador de señales; `sweep_stale_tmp_files`
puede borrar el tempfile de `write_root_file` durante su ventana (el write elevado
falla y aborta en modo fail-closed: seguro).

---

## A3 — Sistema de archivos, paths y extracción

**ID: A3-01** · **S2** · `src/vur_client.rs:1034-1038` (origen) · sinks: `vur_client.rs:690`, `install.rs:399-402`

```rust
    let pkgname = vars
        .get("pkgname")
        .cloned()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("template sin pkgname: {debug_path}"))?;
```

- **Reproducción (verificada por el fusor en código)**: `metadata.rs:176 parse()`
  llama `info.validate()?` (línea 179) → la vía `.VURINFO` **sí** valida; la vía
  `parse_template_text` (fallback para repos VUR **sin** `.VURINFO`) **no**.
  Asimetría real: el nombre del padre entra crudo al índice mientras el de los
  subpaquetes sí se filtra (`vur_client.rs:882`).
  Sink: `master_srcpkgs.join(pkgname)` (`vur_client.rs:690`) → con
  `pkgname=../escape` la escritura sale de `void-packages/srcpkgs/`;
  `install.rs:402` (`format!("{pkgver}.{arch}.xbps")` → `Path::join`) lee un path
  arbitrario con la `version` sin validar.
- **Impacto**: escritura (y borrado en el unproject) de ficheros con contenido de
  plantilla del atacante **fuera** de `void-packages/srcpkgs/`, con permisos del
  usuario; persistencia si el proceso muere entre project y unproject
  (`install.rs:1467` descarta el retorno con `let _ =`).
- **Fix**: validar `pkgname` y `version` en `parse_template_text` con los
  predicados que YA existen (`is_valid_pkgname` público; `is_valid_version`
  privado → hacerlo `pub`). 2 validaciones, 1 módulo. Toca join/shape: **sí**
  (el `pkgname` es la clave de join).
- **Verificación**: test de regresión con **forma de producción** + smoke en
  contenedor con un repo hostil.
- **Evidencia de no-break (medido por el fusor sobre el clon real)**: en
  `~/.cache/vary/void-packages/srcpkgs` hay **15.162 templates** y **9.708
  `pkgname` distintos**; **0** tienen `version=` con guion (el predicado
  `is_valid_version` lo rechaza) y solo **4** no pasan `is_valid_pkgname`:
  `cross-${_triplet}-${_pkgname}`, `cross-${_triplet}-gcc`,
  `cross-${_triplet}-libstdc++` (nombres *computados*, no paquetes) y
  `"opencsg"` (entrecomillado: el parser de Vary lo desenvuelve ✓).
  El fix no rechaza ningún repo real.

**ID: A3-02** · **S3** · `src/reposconf.rs:84` (`ReposConf::save`)

```rust
    std::fs::write(path, toml::to_string_pretty(self)?)
```

- **Reproducción**: `kill -9` durante `vary --repo add/remove/rekey`
  (`stamp_trust`, `keys.rs:644`, usa este mismo punto) → `repos.conf` truncado →
  `ReposConf::load` (`reposconf.rs:73`) falla al parsear → **todo** comando
  dependiente (`install`, `upgrade`, `--print`, `--lock`) falla en seco hasta
  edición manual.
- **Impacto**: corte de servicio post-corte (T6). `installed.json` sí preserva una
  copia `.corrupt.<epoch>` (`db.rs:149-151`); `repos.conf` no.
- **Fix**: el patrón atómico YA existe y está probado en `db.rs:192-201`
  (NamedTempFile en el mismo dir + `write_all` + `flush` + `sync_all` +
  `persist`). Extraerlo a `util::write_atomic` y usarlo en los sitios que hoy
  escriben a pelo (`reposconf.rs:84`, `lockfile.rs:66`).
- **Verificación**: test + CI.

**ID: A3-03** · **S4** · `src/lockfile.rs:66` (`Lockfile::save`)

```rust
    std::fs::write(path, toml::to_string_pretty(self)?)
```

- **Reproducción**: corte a mitad de `vary --lock` → `vary.lock` truncado.
- **Impacto**: advisory-only — `verify_if_pinned` (`lockfile.rs:154-161`) avisa
  "ilegible regenera": el hash de reproducibilidad se pierde en silencio y sin
  copia del corrupto.
- **Fix**: mismo helper `util::write_atomic` (mismo commit que A3-02: es el mismo
  bug en otro módulo).
- **Verificación**: test de persistencia atómica.

**ID: A3-04** · **S3** · `src/challenge.rs:127`

```rust
            let mut bytes = Vec::new();
            use std::io::Read;
            entry.read_to_end(&mut bytes)?;
```

- **Reproducción**: un `.xbps` con un header de tar cuyo `size` es enorme
  (artefacto corrupto por disco lleno o descarga cortada) → `read_to_end`
  reserva esa cantidad en RAM → OOM.
- **Impacto**: recurso sin límite (S2 en la taxonomía, pero el artefacto nace en
  el hostdir del propio usuario → se degrada a S3).
- **Fix**: `entry.take(LIMITE)` (límite razonable, p. ej. 1 GiB por entrada).
  1 línea.
- **Verificación**: test con `.xbps` sintético de tamaño absurdo.

**Descartados con evidencia (A3)**: Vary **no extrae nada a disco** (el único
`tar::Archive` es `challenge.rs:99`, streaming solo a memoria); `bootstrap.rs:82`
y `vup_index.rs:107` usan tmp de nombre fijo pero el `flock` global serializa las
instancias; `vup_index.rs:129` (`let _ = std::fs::write`) es **intencional** y
documentado ("la caché es solo una optimización") y su lectura tolera caché
corrupta (re-descarga) → no es un hallazgo; permisos: `ensure_private_dir(0700)`
en todo el estado y llaves públicas en `644` (correcto: son públicas por
diseño); overlay/masterdir con RAII + `cleanup_stale_sandboxes` + señales ✓.

---

---

## A1 — Contratos de forma y joins

**ID: A1-01** · **S2** · `src/portdb.rs:94` (lectura) vs `src/install.rs:1998` (escritura)

```rust
            Some(format!("{}-{}", e.name, e.pkgver()))   // "foo-devel-1.0_1"
```
```rust
            db.upsert(&sub.pkgname, &version, ...)       // version = pkgver del PADRE
```

- **Reproducción (medida)**: el árbol real de void-packages tiene **5.449 symlinks**
  en `srcpkgs/` (`66-devel -> 66`, `7zip-unrar -> 7zip`): el índice de fuentes
  tiene entrada "foo-devel" (sourcepkg "foo") mientras la DB guarda bajo la
  clave del subpaquete el pkgver del padre ("foo-1.0_1") → la comparación de la
  auditoría daba "versión nueva" para TODO subpaquete (100 % falso positivo).
- **Impacto**: hoy INERTE (`NewVersion` no activa ningún rebuild: su comentario
  lo dice) → es una mina armada para el siguiente que conecte esa razón.
- **Fix (aplicado, `5c90863`)**: `pkgver_of` usa `e.sourcepkg` (el paquete
  fuente, que es lo que guarda la DB) y la auditoría solo cruza con el índice
  los paquetes construidos desde el árbol OFICIAL (`entry.vur == OFFICIAL_REPO`):
  para un paquete de un VUR su fuente vive en el clon del VUR, así que el
  homónimo del índice no es su fuente.
- **Verificación**: tests en `portdb.rs` (symlink real) y `rebuild.rs` (VUR vs
  oficial). **Doble revisión subagent**: OK / OK CON SALVEDADES (sin bloqueantes).

**ID: A1-02** · **S2** · `src/vur_client.rs` (`parse_template_text`)

```rust
    depends: split_deps("depends"),          // el PADRE: sin expandir
```
```rust
                let expanded = s.replace("${sourcepkg}", &pkgname) ...  // los SUBS: sí
```

- **Reproducción (medida)**: template REAL de void-packages (CastXML):
  `_llvmver=21` y `depends="clang${_llvmver}"` → el token crudo llegaba al DAG
  y el resolver fallaba con "paquete no encontrado: clang${_llvmver}" (el
  paquete real se llama `clang21`). **1.643 líneas de deps** del clon real
  contienen `${`.
- **Impacto**: un template válido de void-packages mataba la resolución
  completa de un repo VUR sin `.VURINFO`.
- **Fix (aplicado, `a4777a1`)**: el padre y los subpaquetes expanden con la
  MISMA función (`expand_template_vars`), que además resuelve las variables del
  propio template (`${_llvmver}` → las asignaciones del template).
- **Verificación**: test con el fixture real de CastXML.
- **Doble revisión subagent**: las dos notas S3 están aplicadas (techos
  documentados con `// ponytail:`, asserts que fijan el criterio).

**ID: A1-03** · **S3** · `src/xbps.rs` (parser) + `src/search.rs` (filtro) → **RECHAZADO como bug, resuelto como poda**

- La salida real de `xbps-query -Rs` (xbps 0.60.7, `bin/xbps-query/search.c:68`)
  es `[marca] pkgver desc`: **nunca** un token de repositorio. El filtro que
  "excluía el repo local" era código muerto, y el test que lo cubría usaba una
  forma que la producción nunca genera.
- **Fix (aplicado)**: fuera el token `[repo]`, fuera la marca `x` (xbps solo
  emite `*` y `-`) y el test passa a la forma real (verificada en Void real).

**ID: A1-04** · **S3** · `src/xbps.rs::parse_installed_props` → **DIFERIDO (trigger)**

- El `-p a,b,c` de xbps **omite** las props ausentes (`continue`), así que la
  lectura posicional de líneas se desplaza (mismo patrón de forma que el bug de
  `shlibs`). Afecta al endpoint de props (`xbps-query -p ... <pkg>`), no al
  camino de installed.json.
- **No arreglado en la auditoría**: el arreglo correcto es una llamada por prop
  (más E/S) y toca el contrato de `query_installed`. **Trigger**: que un
  consumidor real lea props posicionales y se desplace. Coste: ~15 líneas,
  1 módulo.

**ID: A1-05** · **S3** · `src/portdb.rs::parse_shlibs` (multi-proveedor) → **DIFERIDO (trigger)**

- `common/shlibs` puede traer varios proveedores del mismo SONAME (p. ej.
  `libc.so` de musl y de glibc); el índice guarda el primero. En un host musl
  eso puede dar un bump falso (aviso consultivo).
- **No arreglado**: el arreglo es indexar por arch (multi-mapa) y cambia el
  formato del índice. **Trigger**: un bump falso observado en un host musl.
  Coste: ~20 líneas, 1 módulo + formato del índice.

**ID: A1-06** · **S4** · `src/xbps.rs` (rama de la marca `x`) → **resuelto en la poda** (ver A1-03).

**ID: A1-07** / **A5-03** · **S3** · `src/args.rs` + `src/command_line.rs` → **DIFERIDO (CLI congelado)**

- Flags de pacman que se aceptan y no tienen efecto (`--needed`, `--ignore`,
  `--overwrite`, `--asdeps` se rechazan, etc.): el contrato de CLI miente.
- **No se arregla en la auditoría**: el prompt congela la CLI durante la
  auditoría ("CLI ... quedan CONGELADOS"). **Trigger**: un script migrado de
  pacman/xbps que dependa de `--ignore`/`--overwrite`. Coste: ~20 líneas
  (lista de rechazo) + decisión de qué flags se reenvían a xbps.

---

## A4 — Estado, atomicidad y recuperación

**ID: A4-01** · **S2** · `src/portdb.rs::load` → **ARREGLADO (`00480c1`)**

- `portdb.json` corrupto → `Err` propagado → el fichero IMPIDÍA reindexar para
  siempre (el índice quedaba inservible sin mano). `db.rs` ya tenía el
  patrón correcto (respaldo `.corrupt.<epoch>` + arranque vacío).
- **Fix**: espejo del patrón de `db.rs` (+ `db::now_epoch_secs` reutilizado).

**ID: A4-02** · **S2** · `src/reposconf.rs:84` → **ARREGLADO (`66dd292`)**

- `repos.conf` (el fichero de CONFIANZA) se escribía a pelo: un corte lo
  truncaba y TODOS los comandos con repos morían hasta editarlo a mano.
- **Fix**: escritura atómica (tmp + fsync + rename, el patrón de `db.rs`).

**ID: A4-03** · **S2** · `src/db.rs::load` → **ARREGLADO (`b9c5e26`, test en `7c9921f`)**

- Un `installed.json` con `schema_version` FUTURO se reinterpretaba con la
  semántica v3 y se guardaba degradado.
- **Fix**: un schema futuro se trata como corrupto (respaldo + vacío).
  **OJO**: la migración v2→v3 es INTENCIONAL (H-005) y NO se toca: solo se
  rechaza el futuro. (El auditor proponía rechazar cualquier `!= 3`: eso
  habría roto la migración documentada → correction del fusor.)

**ID: A4-04** · **S3** · `src/lockfile.rs:66` → **ARREGLADO (`66dd292`)** (mismo fix que A4-02).

**ID: A4-05** · **S3** · `src/reposconf.rs` (serde) → **ARREGLADO (`38e328c`)**

- Un typo en `repos.conf` (`enabld = false`) se ignoraba en silencio: el repo
  que el usuario quería deshabilitar quedaba ACTIVO (y el pin, sin aplicar).
- **Fix**: `deny_unknown_fields` (el error de TOML nombra el campo) + validar
  los nombres de repo al cargar (A2-03).

**ID: A4-06** / **A5-09** · **S3** · `src/upgrade.rs:385` → **ARREGLADO (`8293160`)**

- `refresh_repos(config)?` se tragaba el código de fallo: un repo que no se
  refrescaba quedaba con el índice VIEJO y el upgrade terminaba diciendo
  "VUR packages up to date" (una mentira), y el aviso solo iba al LOG
  (invisible en consola).
- **Fix**: aviso a consola por repo + el mensaje final distingue el caso.

**ID: A4-07** · **S3** · patrón atómico duplicado → **resuelto por A4-02/A4-04** (in situ, sin helper nuevo).

**ID: A4-08** · **S4** · `src/journal.rs` (append sin fsync) → **NO ARREGLADO (best-effort por diseño)**

- Una línea tornada en el diario (corte) se pierde en silencio. El diario es
  forense, no estado: no se toca. **Techo declarado**.

**ID: A4-09** · **S4** · `src/vup_index.rs` (caché del índice) → **ARREGLADO (`66dd292`)**: ahora es un `rename` (atómico, una línea menos).

**ID: A4-10** · **S4** · copia elevada a `/etc/xbps.d` (cross-fs, no atómica) → **NO ARREGLADO**: se auto-repara (el bootstrap reescribe si difiere). **Techo declarado**.

**ID: A4-11** · **S4** · `src/remove.rs` (fallo de guardado invisible) → **ARREGLADO (`d0f831d`)**: ahora falla ruidosamente.

---

## A5 — Lógica de negocio y CLI

**ID: A5-01** · **S2** · `src/install.rs:840` → **ARREGLADO (`41e808c`)**

- El nombre del artefacto oficial usaba el arch CRUDO (`x86_64`) mientras
  xbps-src nombra el artefacto con el arch del sistema (`x86_64-musl`): el
  build "terminaba OK" pero el artefacto no aparecía → **fallo al 100 % en
  hosts musl** (verificado en el clon real: `XBPS_TARGET_MACHINE = x86_64-musl`).
- **Fix**: el arch resuelto llega al build secuencial + helper `artifact_name`
  con test.

**ID: A5-02** · **S2** · `build_options_default` no_MODELADO → **PARADA §8.2 (plan presentado)**

- `xbps-src` activa por defecto las opciones de `build_options_default`
  (`common.sh set_build_options`), pero Vary no las conoce: un `$(vopt_if vala
  vala)` se reduce a la rama OFF y la dep sale del DAG → el build falla.
  **1.685 templates** del clon real declaran `build_options_default`.
- **Por qué NO se arregla aquí**: el arreglo correcto exige tocar el esquema
  VURINFO v1 (`metadata.rs` + `vur_client.rs` + `scripts/vur-generator.sh` +
  `scripts/vurinfo.jq` + `docs/VURINFO.md` + el resolver) → ≥4 módulos y un
  cambio de schema: parada obligatoria (§8.1/§8.2). **Plan**: (1) añadir
  `build_options_default` al `.VURINFO` (mismo criterio que `build_options`),
  (2) la conf por build ya genera `etc/conf.<id>` con las opciones activas →
  un único origen, (3) `use_reduce` recibe las defaults del paquete.
  **Trigger**: un build de VUR falla por una dependencia que solo existe con
  una opción por defecto.

**ID: A5-03** (= A1-07) · **S3** · flags de pacman sin efecto → **DIFERIDO (CLI congelado)**.

**ID: A5-04** · **S3** · `--interactive` (parseado, nunca leído) → **DIFERIDO (CLI congelado)**. Trigger: alguien lo usa esperando un modo interactivo.

**ID: A5-05** · **S3** · `src/query.rs` + `src/xbps.rs` (estado de instalación) → **ARREGLADO (`1bdf93b`)**

- `xbps-query -l` emite `ii` (instalado), `uu` (desempaquetado), `hr` (borrado a
  medias) y `??` (xbps 0.60.7, `list.c:62-68`). Vary losignoraba: un paquete a
  medias desinstalado salía en `-Q` como instalado y `@installed` lo tomaba
  como target de instalación.
- **Fix**: un solo parser (`xbps::installed_entry_from_line`) que solo acepta
  `ii`, usado por `-Q` y por `@installed` (dos parsers del mismo formato → uno).

**ID: A5-06** · **S4** · `src/resolver.rs:435` → **ARREGLADO (`c031b2d`)**: el lookup usa el nombre traducido.

**ID: A5-07** · **S4** · `src/resolver.rs:480` → **ARREGLADO (`c031b2d`)**: el plan usa el mismo nombre que las otras ramas.

**ID: A5-08** · **S4** · `src/resolver.rs` (colisión de niveles) + flags con `-Q` → **DIFERIDO**: solo afecta a `--experimental`; la parte de CLI está congelada.

**ID: A5-09** (= A4-06) · **S3** → **ARREGLADO (`8293160`)**.

**ID: A5-10** · **S3** · `src/rebuild.rs::template_hash` (no cubre `files/`) → **DIFERIDO (schema congelado)**

- El hash de template solo cubre `srcpkgs/<pkg>/template`: los parches y
  ficheros de `srcpkgs/<pkg>/files/` (2.419 paquetes, 1.079 ficheros) NO
  entran, aunque cambian el build.
- **Por qué NO se arregla**: cambiar el formato del hash guardado toca el
  valor en `installed.json` (schema v3 congelado) y, sin versionar el hash,
  provoca un "template cambió" masivo (recompilación de todo lo construido
  por vary) en la primera auditoría posterior. **Plan**: versionar el hash
  (`v2:sha256:...` con `files/**` dentro) y comparar solo hashes de la misma
  versión (los baselines antiguos = "sin señal", no "falsa señal").
  **Trigger**: un build de vary que queda obsoleto por un parche del upstream.

---

## A6 — Tests vs producción y CI

**ID: A6-01** · **S2** · tres tests sin asserts (`bulk_sin_xbps...`, `mide_indice_frio_e_incremental`, `integracion_query_installed_real`) → **ARREGLADO (`342c063`)**: ahora afirman (según el entorno, pero YA PUEDEN FALLAR).

**ID: A6-02** · **S2** · fixtures de `db.rs` con versión "desnuda" → **ARREGLADO (`3c113e5`)**: la forma de producción es el pkgver completo (`{nombre}-{versión}_{rev}`).

**ID: A6-03** · **S2** · la decisión de repolock/repounlock sin test → **ARREGLADO (`7116e7f`)**: criterio extraído a función pura (`suelta_repolock`) con test de tabla.

**ID: A6-04** · **S2** · `.github/workflows/xbps.yml` sin `concurrency` → **ARREGLADO (`f169831`)**: el release rodante (URL fija con repodata firmado) se serializa (`cancel-in-progress: false`): dos publicaciones concurrentes podían dejar assets viejos o ninguno = downgrade silencioso.

**ID: A6-05** · **S3** · el mock del resolver descarta el arch → **NO ARREGLADO**: el caso "binario disponible para otra arch" no está cubierto. Trigger: un VUR con índice por arch. Coste: ~20 líneas de mock.

**ID: A6-06** · **S3** · `paths-ignore: '*.md'` no cubre `test/*.md` → **ARREGLADO (`f169831`)**: `'**/*.md'`.

**ID: A6-07** · **S4** · el fix A4-03 viajó sin su test (llegó en el commit contiguo) → **nota de proceso** en el ledger.

**ID: A6-08** · **S4** · `test/RESULTS.md` con un conteo de tests ignorados que ya no cuadra → **nota**: es un registro histórico.

**ID: A6-09** · **S4** · el `warn` de repounlock dispara en el caso normal → **NO ARREGLADO**: ruido de log; no toca el estado. Trigger: un repounlock que falle por motivo real y haya que distinguirlo.

---

## A7 — Código muerto y redundancia

**Nota de proceso**: el primer informe de A7 (subagent) salió **degenerado**
(texto sin evidencia, pseudo-código inventado). Se **descartó** y el dominio
se repitió con un brief estrecho ("cita el código literal"); la segunda vuelta
(A7b) sí aportó file:line y texto real.

**ID: A7b-01** · **S4** · `src/config.rs:33-36` → **ARREGLADO**: `Colors.enabled` y
`Colors.error` no tenían ningún lector (el `#[allow(dead_code)]` los tapaba).

**ID: A7b-02** · **S4** · `src/portdb.rs` (`get`, `sourcepkg`, `providers`) → **ARREGLADO**:
los tres accesores solo los usaban sus propios tests (el comentario del
archivo describía "consumidores" que nunca llegaron). `soname.rs` tiene su
propio `provider_parts` (nombre+versión+revisión) ✓ no era duplicado.

**ID: A7b-03** · **S4** · `src/db.rs::get_build_date` → **ARREGLADO**: solo un test lo usaba.

**ID: A7b-04** · **S4** · `src/signal.rs` (3) + `src/db.rs` (1) → **ARREGLADO**: cuatro
`#[allow(dead_code)]` sobre código **vivo** (montajes, `InstalledDb::get`): el
atributo mentía ("esto está muerto a propósito" sobre código en producción).

**ID: A7b-05** · **S3** · `src/metadata.rs` (`VurInfo.checkdepends`, `VurInfo.distfiles`)
→ **ARREGLADO**: se parseaban y se escribían, nadie los leía. `VurInfo` nunca se
serializa (solo se parsea) → borrarlos no cambia ningún formato en disco.

**ID: A7b-06** · **S3** (el subagent loRated S2) · `README.md:22` / `README.es.md:22`
→ **ARREGLADO**: el README promete bootstrap automático "en el primer
`vary -Syu`"; el código solo lo hace cuando hay algo que compilar o instalar
(`upgrade → install::install → initialize_environment`), no en un `-Syu` sin
trabajo. **Corrección del fusor**: la Severity baja a S3 porque la ruta
`upgrade → install::install` SÍ hace bootstrap (el subagent solo miró
`upgrade.rs`).

**ID: A7b-07** · **S3** · `README.md:265` / `README.es.md:266` → **ARREGLADO**: la hoja de
ruta anuncia builds concurrentes "vía `tokio::sync::Semaphore`" — **no hay
tokio** (ni lo hubo); los builds por niveles con sandboxes existen desde 0.4.1
(`--experimental` + `max_concurrent_builds > 1`).

**ID: A7b-08** · **S3** · `docs/CONTRATO.md` → **ARREGLADO (nota)**: es un ensayo
comparativo point-in-time (septiembre 2026) que describe un vary secuencial
y con `.VURINFO` estricto. Reescribir un ensayo no es un fix: se le añade una
cabecera que declara su desfase y remite a README/VURINFO.md.

**ID: A7b-09** · **S3** · `src/args.rs` (43 de 64 flags de pacman) → **DIFERIDO (CLI congelado)**

- Se aceptan y se registran, y nadie los consulta; además, `takes_value()` no
  exige valor para ellos, así que el valor siguiente cae como **paquete**
  posicional (`vary -S --assume-installed 1.0 foo` → "1.0" y "foo" son
  targets). El daño real es un error confuso, no una instalación equivocada.
- **No se arregla en la auditoría**: el arreglo es una lista de rechazo o un
  reenvío real a xbps → cambio de CLI (congelado) y decisión de producto.
  **Trigger**: un script migrado de pacman/xbps que use `--ignore`,
  `--overwrite`, `--needed` o `--assume-installed`.

**Sin hallazgos A7**: los 20 crates de `Cargo.toml` tienen call site en
PRODUCCIÓN (no solo en tests) → ninguno se elimina ni se mueve a
`dev-dependencies`.
