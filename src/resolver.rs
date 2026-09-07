// MVP: Builds 100% secuenciales. xbps-src maneja paralelismo interno (-j).
// Fase 2 del proyecto: builds concurrentes con tokio::sync::Semaphore
// usando max_concurrent_builds de vary.conf como límite.

//! Motor de resolución de dependencias de `vary`.
//!
//! Construye un DAG con [`petgraph::graphmap::DiGraphMap`] donde cada nodo es
//! un paquete y cada arista va `dep -> dependiente`. Es 100% puro (sin I/O ni
//! procesos): toda la obtención de datos se inyecta vía [`PackageSource`].
//!
//! ## Semántica de resolución
//!
//! 1. **Precedencia de candidatos** para cada nombre (target o dependencia):
//!    (a) si existe en repos oficiales de Void se emite [`Action::Install`]
//!    con [`BinarySource::Official`] como **nodo hoja** —no se recurren sus
//!    dependencias porque `xbps-install` las resuelve nativamente—; (b) si no,
//!    se busca en los repos VUR por nombre y, si no existe, por `provides`;
//!    (c) sin candidato en ninguno => error.
//!    Con preferencia explícita (T-010: [`CandidateOrder::PreferBinary`] /
//!    [`CandidateOrder::PreferSource`], flags `--prefer-binary` /
//!    `--force-build` / `--no-prefer-binary`): (a') el bucket official solo
//!    gana en automático con preferencia binaria (ya es binario); con
//!    preferencia de fuente un template VUR existente se compila en su lugar
//!    y, si no lo hay, se AVISA "sin efecto" y se sigue con official (nunca
//!    en silencio); (b') entre candidatos VUR multi-repo se ordena por clase
//!    binario/fuente efectiva, luego versión desc y luego prioridad del repo.
//!    Sin flags ([`CandidateOrder::Legacy`]) todo queda como antes: official
//!    primero y candidato VUR fusionado único.
//! 2. Un nodo VUR se convierte en `Install(VulBinary { repo })` si
//!    `!force_build && prefer_binary && vul_binary_available(..)`; si no, en
//!    [`Action::Build`].
//! 3. **Solo** los nodos `Build` expanden dependencias: `hostmakedepends +
//!    makedepends + depends` concatenadas en ese orden y deduplicadas. Cada
//!    string de dep puede llevar restricciones (`pkg>=1.2`, `virtual`) que se
//!    recortan con [`dep_name`] antes del lookup; la dependencia se resuelve
//!    con esta misma precedencia (recursión) formando la arista
//!    `dep -> dependiente`.
//! 4. **Deduplicación**: un mismo paquete aparece UNA sola vez aunque sea
//!    alcanzable por múltiples caminos (diamante); el mapa `nombre -> índice`
//!    hace de conjunto "negro".
//! 5. **Filtro de arquitectura**: si el `info.archs` de un candidato VUR no
//!    contiene [`PackageSource::arch`], ese candidato se descarta y se prueba
//!    el siguiente origen (el lookup por `provides`; si tampoco hay candidato
//!    válido, se considera inexistente).
//! 6. **Ciclos**: detectados antes del toposort durante la propia expansión
//!    (conjunto "gris" = camino activo); se devuelve error listando el ciclo,
//!    p. ej. `a -> b -> a`. El `toposort` actúa como red de seguridad.
//! 7. **Salida**: `installs` con oficiales primero y luego `VulBinary`
//!    (cada grupo por prioridad de llegada), y `builds` en orden topológico
//!    estricto: toda dependencia de un build aparece antes en `builds` o está
//!    en `installs`.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use anyhow::{bail, Result};
use petgraph::algo::toposort;
use petgraph::graphmap::DiGraphMap;

use crate::metadata::VurInfo;

/// Origen de un binario a instalar.
#[derive(Debug, Clone, PartialEq)]
pub enum BinarySource {
    /// Repositorio oficial de Void Linux.
    Official,
    /// Binario firmado publicado por un repo del VUR.
    VulBinary { repo: String },
}

/// Acción a ejecutar sobre un paquete del plan.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Install(BinarySource),
    Build,
}

/// Entrada del plan: paquete resuelto más acción asociada.
#[derive(Debug, Clone)]
pub struct PlanItem {
    /// Nombre con el que se solicitó (target o dependencia tras `dep_name`).
    pub name: String,
    pub info: VurInfo,
    pub action: Action,
}

/// Plan resultante de la resolución.
#[derive(Debug, Clone, Default)]
pub struct Plan {
    /// Oficiales primero, luego `VulBinary`; cada grupo por orden de llegada.
    pub installs: Vec<PlanItem>,
    /// Builds en ORDEN TOPOSÓGICO (dependencias primero).
    pub builds: Vec<PlanItem>,
    /// Avisos no fatales (T-010: flag sin efecto sobre candidato official).
    pub warnings: Vec<String>,
}

/// Fuente de datos inyectable: abstrae toda consulta a repos/xbps/xbps-src.
pub trait PackageSource {
    /// true si el paquete existe en repos oficiales de Void (binario)
    fn official_exists(&self, name: &str) -> bool;
    /// busca paquete VUR por nombre; None si ningún repo VUR lo tiene
    fn vur_lookup(&self, name: &str) -> Option<(String, VurInfo)>;
    /// busca paquete VUR por nombre IGNORANDO la arquitectura; se usa para
    /// detectar paquetes existentes pero incompatibles con la arch actual.
    fn vur_lookup_any_arch(&self, name: &str) -> Option<(String, VurInfo)>;
    /// mejor candidato VUR que PROVIDE el nombre virtual dado (ranking: priority asc de repo, luego mayor version)
    fn vur_lookup_provides(&self, virtual_name: &str) -> Option<(String, VurInfo)>;
    /// hay binario firmado disponible para este pkg en ese repo y arquitectura?
    fn vul_binary_available(&self, repo: &str, info: &VurInfo, arch: &str) -> bool;
    /// T-010: TODOS los candidatos VUR por nombre (multi-repo), SIN filtrar
    /// por arquitectura (el resolver filtra). Default: el candidato único de
    /// [`PackageSource::vur_lookup`] (compatibilidad con implementaciones
    /// que solo ven el índice fusionado).
    fn vur_lookup_all(&self, name: &str) -> Vec<(String, VurInfo)> {
        self.vur_lookup(name).into_iter().collect()
    }
    /// T-010: prioridad del repo (menor = mayor preferencia); default 100
    /// (igual que el fallback de `install.rs`).
    fn repo_priority(&self, _repo: &str) -> i64 {
        100
    }
    /// arquitectura actual del sistema/masterdir (ej "x86_64")
    fn arch(&self) -> String;
}

/// T-010: orden de candidatos entre repos. Sin flags es [`CandidateOrder::Legacy`]
/// (comportamiento histórico: official primero, candidato VUR fusionado
/// único). Los flags explícitos reordenan por clase efectiva ANTES de
/// versión/prioridad.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CandidateOrder {
    /// Sin preferencia explícita: precedencia histórica intacta.
    #[default]
    Legacy,
    /// `--prefer-binary`: candidatos con binario firmado primero.
    PreferBinary,
    /// `--force-build` / `--no-prefer-binary`: candidatos compilables primero.
    PreferSource,
}

/// Opciones de resolución.
#[derive(Debug, Clone)]
pub struct ResolveOptions {
    /// Si hay binario firmado disponible, prefiero instalarlo antes que compilar.
    pub prefer_binary: bool,
    /// Fuerza compilar desde fuente aunque haya binario disponible.
    pub force_build: bool,
    /// T-010: orden entre candidatos de varios repos (default: histórico).
    pub order: CandidateOrder,
}

impl Default for ResolveOptions {
    fn default() -> Self {
        Self {
            prefer_binary: true,
            force_build: false,
            order: CandidateOrder::Legacy,
        }
    }
}

/// Recorta restricciones de versión de un string de dependencia:
/// corta en el primer `<`, `>`, `=` o espacio y conserva el nombre base
/// (`"pkg>=1.2"` / `"pkg <2"` / `"pkg"` -> `"pkg"`).
pub(crate) fn dep_name(dep: &str) -> &str {
    match dep.find(['<', '>', '=', ' ']) {
        Some(i) => &dep[..i],
        None => dep,
    }
}

/// T-010: compara (versión, revisión): tokens alfanuméricos separados por
/// cualquier otro carácter; token numérico vs numérico compara como número,
/// el resto lexicográficamente; a igualdad de prefijo gana la más larga;
/// desempata por revisión.
fn cmp_pkgver(a_ver: &str, a_rev: u32, b_ver: &str, b_rev: u32) -> Ordering {
    fn tokens(s: &str) -> Vec<&str> {
        s.split(|c: char| !c.is_alphanumeric())
            .filter(|t| !t.is_empty())
            .collect()
    }
    fn cmp_token(a: &str, b: &str) -> Ordering {
        match (a.parse::<u64>(), b.parse::<u64>()) {
            (Ok(x), Ok(y)) => x.cmp(&y),
            _ => a.cmp(b),
        }
    }
    let (ta, tb) = (tokens(a_ver), tokens(b_ver));
    for (x, y) in ta.iter().zip(tb.iter()) {
        let ord = cmp_token(x, y);
        if ord != Ordering::Equal {
            return ord;
        }
    }
    ta.len().cmp(&tb.len()).then_with(|| a_rev.cmp(&b_rev))
}

/// Estado interno de una resolución (ver semántica en la doc del módulo).
struct Ctx<'a> {
    source: &'a dyn PackageSource,
    opts: &'a ResolveOptions,
    arch: String,
    items: Vec<PlanItem>,
    /// nombre -> índice de nodo; conjunto "negro" de la deduplicación (punto 4).
    index: HashMap<String, u32>,
    /// DAG con aristas `dep -> dependiente` (los ids son índices en `items`).
    graph: DiGraphMap<u32, ()>,
    /// camino de expansión activo; conjunto "gris" para ciclos (punto 6).
    stack: Vec<String>,
    /// T-010: avisos no fatales que viajan al [`Plan`] (flag sin efecto).
    warnings: Vec<String>,
}

impl<'a> Ctx<'a> {
    fn new(source: &'a dyn PackageSource, opts: &'a ResolveOptions) -> Self {
        let arch = source.arch();
        Self {
            source,
            opts,
            arch,
            items: Vec::new(),
            index: HashMap::new(),
            graph: DiGraphMap::new(),
            stack: Vec::new(),
            warnings: Vec::new(),
        }
    }

    /// Punto 5: ¿el paquete soporta la arquitectura actual?
    fn arch_supported(&self, info: &VurInfo) -> bool {
        crate::metadata::arch_supports(&info.archs, &self.arch)
    }

    /// Puntos 1b y 5: candidato VUR por nombre y, si falla o es descartado
    /// Nodo hoja para un paquete oficial. `VurInfo` es aquí un placeholder
    /// mínimo (los oficiales no tienen template VUR); solo importa el nombre.
    fn official_item(name: &str, arch: &str) -> PlanItem {
        PlanItem {
            name: name.to_string(),
            info: VurInfo {
                format_version: 0,
                pkgname: name.to_string(),
                version: String::new(),
                revision: 0,
                archs: vec![arch.to_string()],
                subpackages: Vec::new(),
                depends: Vec::new(),
                hostmakedepends: Vec::new(),
                makedepends: Vec::new(),
                checkdepends: Vec::new(),
                build_style: None,
                distfiles: Vec::new(),
                checksum: Vec::new(),
                provides: Vec::new(),
                replaces: Vec::new(),
                restricted: false,
                maintainer: None,
            },
            action: Action::Install(BinarySource::Official),
        }
    }

    fn link(&mut self, dep: u32, dependent: u32) {
        self.graph.add_edge(dep, dependent, ());
    }

    /// Registra un ítem nuevo, su nodo en el grafo y su arista entrante.
    fn push_item(&mut self, item: PlanItem, dependent: Option<u32>) -> u32 {
        let idx = self.items.len() as u32;
        let name = item.name.clone();
        self.items.push(item);
        self.index.insert(name, idx);
        self.graph.add_node(idx);
        if let Some(parent) = dependent {
            self.link(idx, parent);
        }
        idx
    }

    /// Resuelve `name` con la precedencia 1-3; `dependent` es el índice del
    /// nodo que declaró esta dependencia (para trazar la arista).
    fn resolve_name(&mut self, name: &str, dependent: Option<u32>) -> Result<u32> {
        // Punto 6: el nombre ya está en el camino de expansión activo -> ciclo.
        // Se lista desde su primera aparición: "a -> b -> a" (a depende de b
        // depende de a).
        if let Some(pos) = self.stack.iter().position(|n| n.as_str() == name) {
            let mut cycle = self.stack[pos..].join(" -> ");
            cycle.push_str(" -> ");
            cycle.push_str(name);
            bail!("ciclo de dependencias detectado: {cycle}");
        }
        // Punto 4: ya resuelto por otro camino (diamante): reutilizar nodo.
        if let Some(&idx) = self.index.get(name) {
            if let Some(parent) = dependent {
                self.link(idx, parent);
            }
            return Ok(idx);
        }

        // T-010: con preferencia de fuente explícita el bucket official NO
        // gana en automático (un template VUR existente se compila en su
        // lugar; ver rama sin-candidato abajo). Con Legacy o preferencia
        // binaria el official (binario) sigue ganando como siempre.
        // Punto 1a: oficial -> Install(Official), NODO HOJA (sin recursión).
        if self.source.official_exists(name)
            && !matches!(self.opts.order, CandidateOrder::PreferSource)
        {
            let item = Self::official_item(name, &self.arch);
            return Ok(self.push_item(item, dependent));
        }

        // Puntos 1b/1c y 5: VUR por nombre o provides, filtrando arquitectura.
        // Legacy usa el candidato fusionado único (comportamiento histórico);
        // con orden explícito se listan todos los candidatos multi-repo y se
        // elige por clase efectiva, versión y prioridad (T-010).
        let found: Option<(String, VurInfo)> = if matches!(self.opts.order, CandidateOrder::Legacy)
        {
            let mut candidate = None;
            if let Some(found) = self.source.vur_lookup(name) {
                if self.arch_supported(&found.1) {
                    candidate = Some(found);
                }
            }
            if candidate.is_none() {
                if let Some(found) = self.source.vur_lookup_provides(name) {
                    if self.arch_supported(&found.1) {
                        candidate = Some(found);
                    }
                }
            }
            candidate
        } else {
            let mut cands = self.source.vur_lookup_all(name);
            if cands.is_empty() {
                // provides sigue siendo fallback solo-nombre (igual que hoy).
                if let Some(found) = self.source.vur_lookup_provides(name) {
                    cands = vec![found];
                }
            }
            let supported: Vec<(String, VurInfo)> = cands
                .into_iter()
                .filter(|(_, info)| self.arch_supported(info))
                .collect();
            self.pick_ordered(supported)
        };
        let Some((repo, info)) = found else {
            // El paquete existe en algún VUR pero es incompatible con la
            // arquitectura actual: lo reportamos como tal en vez de "no encontrado".
            if let Some((mrepo, _)) = self.source.vur_lookup_any_arch(name) {
                bail!(
                    "el paquete '{}' existe en VUR(s) {} pero no está disponible para tu arquitectura '{}'",
                    name,
                    mrepo,
                    self.arch
                );
            }
            // T-010: official + preferencia de fuente + sin template VUR: el
            // flag no puede actuar -> AVISAR (nunca callar) y seguir con el
            // binario official.
            if self.source.official_exists(name) {
                self.warnings.push(format!(
                    "--force-build/--no-prefer-binary sin efecto sobre '{name}': solo existe como binario oficial"
                ));
                let item = Self::official_item(name, &self.arch);
                return Ok(self.push_item(item, dependent));
            }
            bail!("paquete no encontrado en repos oficiales ni VURs: {name}");
        };

        self.push_vur(name, repo, info, dependent)
    }

    /// T-010: clase de un candidato (0 = preferido por el orden efectivo).
    /// Legacy no reordena (toda la lista empata y decide versión/prioridad,
    /// aunque en la práctica Legacy no usa esta vía).
    fn class_rank(&self, repo: &str, info: &VurInfo) -> u8 {
        let has_binary = self.source.vul_binary_available(repo, info, &self.arch);
        match self.opts.order {
            CandidateOrder::Legacy => 0,
            CandidateOrder::PreferBinary => u8::from(!has_binary),
            CandidateOrder::PreferSource => u8::from(has_binary),
        }
    }

    /// T-010: elige ganador entre candidatos multi-repo: (a) clase efectiva,
    /// (b) versión desc, (c) prioridad del repo asc, (d) nombre de repo
    /// (determinismo total).
    fn pick_ordered(&self, mut cands: Vec<(String, VurInfo)>) -> Option<(String, VurInfo)> {
        cands.sort_by(|a, b| {
            self.class_rank(&a.0, &a.1)
                .cmp(&self.class_rank(&b.0, &b.1))
                .then_with(|| cmp_pkgver(&b.1.version, b.1.revision, &a.1.version, a.1.revision))
                .then_with(|| {
                    self.source
                        .repo_priority(&a.0)
                        .cmp(&self.source.repo_priority(&b.0))
                })
                .then_with(|| a.0.cmp(&b.0))
        });
        cands.into_iter().next()
    }

    /// Puntos 2 y 3: decide Install/Build para un candidato VUR, lo registra
    /// y (solo si es Build) expande sus dependencias.
    fn push_vur(
        &mut self,
        name: &str,
        repo: String,
        info: VurInfo,
        dependent: Option<u32>,
    ) -> Result<u32> {
        // Punto 2: binario firmado disponible -> instalar; si no -> construir.
        let action = if !self.opts.force_build
            && self.opts.prefer_binary
            && self.source.vul_binary_available(&repo, &info, &self.arch)
        {
            Action::Install(BinarySource::VulBinary { repo })
        } else {
            Action::Build
        };
        let item = PlanItem {
            name: name.to_string(),
            info,
            action,
        };
        let idx = self.push_item(item, dependent);

        // Punto 3: SOLO los builds expanden dependencias.
        if matches!(self.items[idx as usize].action, Action::Build) {
            let deps = {
                let info = &self.items[idx as usize].info;
                let mut seen = HashSet::new();
                let mut ordered = Vec::new();
                for dep in info
                    .hostmakedepends
                    .iter()
                    .chain(&info.makedepends)
                    .chain(&info.depends)
                {
                    if seen.insert(dep.clone()) {
                        ordered.push(dep.clone());
                    }
                }
                ordered
            };
            self.stack.push(name.to_string());
            for dep in &deps {
                self.resolve_name(dep_name(dep), Some(idx))?;
            }
            self.stack.pop();
        }
        Ok(idx)
    }

    /// Punto 7: toposort + partición installs/builds.
    fn finish(self) -> Result<Plan> {
        // Red de seguridad: los ciclos reales ya se detectan en resolve_name.
        let order = toposort(&self.graph, None)
            .map_err(|_| anyhow::anyhow!("ciclo de dependencias residual tras la expansión"))?;

        let mut builds = Vec::new();
        for idx in order {
            let item = &self.items[idx as usize];
            if matches!(item.action, Action::Build) {
                builds.push(item.clone());
            }
        }

        let mut officials = Vec::new();
        let mut vuls = Vec::new();
        for item in &self.items {
            match &item.action {
                Action::Install(BinarySource::Official) => officials.push(item.clone()),
                Action::Install(BinarySource::VulBinary { .. }) => vuls.push(item.clone()),
                Action::Build => {}
            }
        }
        officials.append(&mut vuls);

        Ok(Plan {
            installs: officials,
            builds,
            warnings: self.warnings,
        })
    }
}

/// Resuelve `targets` contra `source` produciendo un [`Plan`].
///
/// Ver la documentación del módulo para la semántica completa.
pub fn resolve(
    targets: &[String],
    source: &dyn PackageSource,
    opts: &ResolveOptions,
) -> Result<Plan> {
    let mut ctx = Ctx::new(source, opts);
    for target in targets {
        ctx.resolve_name(target, None)?;
    }
    ctx.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    type Repo = &'static str;

    fn strs(xs: &[&str]) -> Vec<String> {
        xs.iter().map(|s| (*s).to_string()).collect()
    }

    fn targets(ts: &[&str]) -> Vec<String> {
        ts.iter().map(|s| (*s).to_string()).collect()
    }

    fn vur_info(pkgname: &str, archs: &[&str]) -> VurInfo {
        vur_info_ver(pkgname, "1.0", archs)
    }

    fn vur_info_ver(pkgname: &str, version: &str, archs: &[&str]) -> VurInfo {
        VurInfo {
            format_version: 1,
            pkgname: pkgname.to_string(),
            version: version.to_string(),
            revision: 1,
            archs: strs(archs),
            subpackages: Vec::new(),
            depends: Vec::new(),
            hostmakedepends: Vec::new(),
            makedepends: Vec::new(),
            checkdepends: Vec::new(),
            build_style: None,
            distfiles: Vec::new(),
            checksum: Vec::new(),
            provides: Vec::new(),
            replaces: Vec::new(),
            restricted: false,
            maintainer: None,
        }
    }

    struct MockSource {
        official: HashSet<&'static str>,
        vur: HashMap<&'static str, (Repo, VurInfo)>,
        /// T-010: candidatos multi-repo por nombre (tiene prioridad sobre
        /// `vur` en `vur_lookup_all`).
        vur_multi: HashMap<&'static str, Vec<(Repo, VurInfo)>>,
        provides: HashMap<&'static str, (Repo, VurInfo)>,
        binaries: HashSet<(Repo, &'static str)>,
        /// T-010: prioridad por repo (ausente = 100, igual que producción).
        priorities: HashMap<Repo, i64>,
        arch: &'static str,
    }

    impl Default for MockSource {
        fn default() -> Self {
            Self {
                official: HashSet::new(),
                vur: HashMap::new(),
                vur_multi: HashMap::new(),
                provides: HashMap::new(),
                binaries: HashSet::new(),
                priorities: HashMap::new(),
                arch: "x86_64",
            }
        }
    }

    impl PackageSource for MockSource {
        fn official_exists(&self, name: &str) -> bool {
            self.official.contains(name)
        }
        fn vur_lookup(&self, name: &str) -> Option<(String, VurInfo)> {
            self.vur
                .get(name)
                .map(|(repo, info)| ((*repo).to_string(), info.clone()))
        }
        fn vur_lookup_any_arch(&self, name: &str) -> Option<(String, VurInfo)> {
            self.vur
                .get(name)
                .map(|(repo, info)| ((*repo).to_string(), info.clone()))
        }
        fn vur_lookup_provides(&self, virtual_name: &str) -> Option<(String, VurInfo)> {
            self.provides
                .get(virtual_name)
                .map(|(repo, info)| ((*repo).to_string(), info.clone()))
        }
        fn vul_binary_available(&self, repo: &str, info: &VurInfo, _arch: &str) -> bool {
            self.binaries.contains(&(repo, info.pkgname.as_str()))
        }
        fn vur_lookup_all(&self, name: &str) -> Vec<(String, VurInfo)> {
            if let Some(v) = self.vur_multi.get(name) {
                return v
                    .iter()
                    .map(|(repo, info)| ((*repo).to_string(), info.clone()))
                    .collect();
            }
            self.vur_lookup(name).into_iter().collect()
        }
        fn repo_priority(&self, repo: &str) -> i64 {
            self.priorities.get(repo).copied().unwrap_or(100)
        }
        fn arch(&self) -> String {
            self.arch.to_string()
        }
    }

    fn build_names(plan: &Plan) -> Vec<&str> {
        plan.builds.iter().map(|i| i.name.as_str()).collect()
    }

    /// Test 1: hello-vur único sin deps, sin binario -> 1 build, 0 installs.
    #[test]
    fn vur_sin_binario_genera_build_unico() {
        let src = MockSource {
            vur: [(
                "hello-vur",
                ("vur-main", vur_info("hello-vur", &["x86_64"])),
            )]
            .into(),
            ..MockSource::default()
        };
        let plan = resolve(&targets(&["hello-vur"]), &src, &ResolveOptions::default()).unwrap();
        assert_eq!(plan.installs.len(), 0);
        assert_eq!(build_names(&plan), vec!["hello-vur"]);
    }

    /// Test 2: cadena a->b->c => builds == [c, b, a].
    #[test]
    fn cadena_en_orden_topologico_estricto() {
        let mut a = vur_info("a", &["x86_64"]);
        a.depends = strs(&["b"]);
        let mut b = vur_info("b", &["x86_64"]);
        b.depends = strs(&["c"]);
        let c = vur_info("c", &["x86_64"]);
        let src = MockSource {
            vur: [
                ("a", ("vur-main", a)),
                ("b", ("vur-main", b)),
                ("c", ("vur-main", c)),
            ]
            .into(),
            ..MockSource::default()
        };
        let plan = resolve(&targets(&["a"]), &src, &ResolveOptions::default()).unwrap();
        assert_eq!(build_names(&plan), vec!["c", "b", "a"]);
        assert!(plan.installs.is_empty());
    }

    /// Test 3: diamante root->[x,y], ambos -> z: z aparece UNA vez y antes.
    #[test]
    fn diamante_deduplica_paquete_compartido() {
        let mut root = vur_info("root", &["x86_64"]);
        root.depends = strs(&["x", "y"]);
        let mut x = vur_info("x", &["x86_64"]);
        x.makedepends = strs(&["z"]);
        let mut y = vur_info("y", &["x86_64"]);
        y.hostmakedepends = strs(&["z"]);
        let z = vur_info("z", &["x86_64"]);
        let src = MockSource {
            vur: [
                ("root", ("vur-main", root)),
                ("x", ("vur-main", x)),
                ("y", ("vur-main", y)),
                ("z", ("vur-main", z)),
            ]
            .into(),
            ..MockSource::default()
        };
        let plan = resolve(&targets(&["root"]), &src, &ResolveOptions::default()).unwrap();
        let names = build_names(&plan);
        assert_eq!(names.len(), 4);
        assert_eq!(names.iter().filter(|n| **n == "z").count(), 1);
        let zpos = names.iter().position(|n| *n == "z").unwrap();
        assert!(zpos < names.iter().position(|n| *n == "x").unwrap());
        assert!(zpos < names.iter().position(|n| *n == "y").unwrap());
    }

    /// Test 4: colisión oficial+VUR -> Install(Official), 0 builds, sin
    /// recurrir deps VUR (si se recurriera, "fantasma" provocaría error).
    #[test]
    fn oficial_tiene_prioridad_y_es_hoja() {
        let mut tool = vur_info("tool", &["x86_64"]);
        tool.depends = strs(&["fantasma"]); // NO debe consultarse nunca
        let src = MockSource {
            official: ["tool"].into(),
            vur: [("tool", ("vur-main", tool))].into(),
            ..MockSource::default()
        };
        let plan = resolve(&targets(&["tool"]), &src, &ResolveOptions::default()).unwrap();
        assert!(plan.builds.is_empty());
        assert_eq!(plan.installs.len(), 1);
        assert_eq!(plan.installs[0].name, "tool");
        assert_eq!(
            plan.installs[0].action,
            Action::Install(BinarySource::Official)
        );
    }

    /// Test 5: "libfoo.so.1" no existe por nombre pero un VUR lo provee.
    #[test]
    fn resuelve_via_provides() {
        let src = MockSource {
            provides: [("libfoo.so.1", ("vur-main", vur_info("foo", &["x86_64"])))].into(),
            ..MockSource::default()
        };
        let plan = resolve(&targets(&["libfoo.so.1"]), &src, &ResolveOptions::default()).unwrap();
        assert_eq!(plan.builds.len(), 1);
        assert_eq!(plan.builds[0].name, "libfoo.so.1");
        assert_eq!(plan.builds[0].info.pkgname, "foo");
    }

    #[test]
    fn info_pkgname_es_siempre_el_nombre_operativo() {
        // H-046: install.rs opera con info.pkgname (name puede ser virtual).
        // Oficiales: sintético con pkgname == name.
        let src = MockSource {
            official: ["git"].into(),
            ..MockSource::default()
        };
        let plan = resolve(&targets(&["git"]), &src, &ResolveOptions::default()).unwrap();
        assert_eq!(plan.installs.len(), 1);
        assert_eq!(plan.installs[0].name, "git");
        assert_eq!(
            plan.installs[0].info.pkgname, "git",
            "en oficiales el operativo coincide con lo pedido"
        );
    }

    /// Test 6: ciclo a<->b -> Err con mensaje conteniendo "->".
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

    /// Test 7: prefer_binary && binario && !force_build -> VulBinary;
    /// con force_build -> Build.
    #[test]
    fn binario_disponible_instala_salvo_force_build() {
        let src = MockSource {
            vur: [("app", ("vur-main", vur_info("app", &["x86_64"])))].into(),
            binaries: [("vur-main", "app")].into(),
            ..MockSource::default()
        };
        let plan = resolve(&targets(&["app"]), &src, &ResolveOptions::default()).unwrap();
        assert!(plan.builds.is_empty());
        assert_eq!(plan.installs.len(), 1);
        assert_eq!(
            plan.installs[0].action,
            Action::Install(BinarySource::VulBinary {
                repo: "vur-main".to_string()
            })
        );

        let opts = ResolveOptions {
            force_build: true,
            ..ResolveOptions::default()
        };
        let plan = resolve(&targets(&["app"]), &src, &opts).unwrap();
        assert!(plan.installs.is_empty());
        assert_eq!(build_names(&plan), vec!["app"]);
    }

    /// Test 8: dep con constraint "z>=2.0" resuelve contra el VUR "z".
    #[test]
    fn dependencia_con_constraint_recorta_nombre() {
        let mut parent = vur_info("parent", &["x86_64"]);
        parent.depends = strs(&["z>=2.0"]);
        let z = vur_info("z", &["x86_64"]);
        let src = MockSource {
            vur: [("parent", ("vur-main", parent)), ("z", ("vur-main", z))].into(),
            ..MockSource::default()
        };
        let plan = resolve(&targets(&["parent"]), &src, &ResolveOptions::default()).unwrap();
        assert_eq!(build_names(&plan), vec!["z", "parent"]);
    }

    /// Test 9: filtro archs. Candidata única aarch64 con arch x86_64 -> Err;
    /// si además existe un proveedor compatible vía provides, se usa ese.
    #[test]
    fn filtro_arquitectura_descarta_y_prueba_alternativa_o_falla() {
        let solo_arm = MockSource {
            vur: [("arm-only", ("vur-arm", vur_info("arm-only", &["aarch64"])))].into(),
            ..MockSource::default()
        };
        let err = resolve(
            &targets(&["arm-only"]),
            &solo_arm,
            &ResolveOptions::default(),
        )
        .unwrap_err();
        // El paquete existe pero es incompatible con la arquitectura: el mensaje
        // debe aclararlo en vez de decir "no encontrado".
        let msg = err.to_string();
        assert!(
            msg.contains("no está disponible para tu arquitectura"),
            "error inesperado: {msg}"
        );
        assert!(
            msg.contains("vur-arm"),
            "el mensaje debe nombrar el VUR: {msg}"
        );

        let con_alternativa = MockSource {
            vur: [("gpukit", ("vur-arm", vur_info("gpukit", &["aarch64"])))].into(),
            provides: [("gpukit", ("vur-x86", vur_info("gpukit-x86", &["x86_64"])))].into(),
            ..MockSource::default()
        };
        let plan = resolve(
            &targets(&["gpukit"]),
            &con_alternativa,
            &ResolveOptions::default(),
        )
        .unwrap();
        assert_eq!(build_names(&plan), vec!["gpukit"]);
        assert_eq!(plan.builds[0].info.pkgname, "gpukit-x86");
    }

    /// Test 10: mixto. Target VUR-build con deps [oficial, otro-vur-build]:
    /// installs contiene el Official y builds queda en orden topológico.
    #[test]
    fn plan_mixto_oficial_mas_builds_topologicos() {
        let mut svc = vur_info("svc", &["x86_64"]);
        svc.makedepends = strs(&["libdata"]);
        svc.depends = strs(&["curl"]);
        let libdata = vur_info("libdata", &["x86_64"]);
        let src = MockSource {
            official: ["curl"].into(),
            vur: [
                ("svc", ("vur-main", svc)),
                ("libdata", ("vur-main", libdata)),
            ]
            .into(),
            ..MockSource::default()
        };
        let plan = resolve(&targets(&["svc"]), &src, &ResolveOptions::default()).unwrap();
        assert_eq!(plan.installs.len(), 1);
        assert_eq!(plan.installs[0].name, "curl");
        assert_eq!(
            plan.installs[0].action,
            Action::Install(BinarySource::Official)
        );
        assert_eq!(build_names(&plan), vec!["libdata", "svc"]);
    }

    // --- T-010: red de seguridad (caracterización del comportamiento ACTUAL
    // sin flags). Deben seguir en verde tras el rediseño: "sin flags,
    // comportamiento actual inalterado". El arbitraje de versiones entre
    // repos vive en el merge (install.rs: or_insert por prioridad); el
    // resolver solo ve el candidato fusionado: estos tests fijan qué hace
    // con él cuando hay otras vías (binario en otro repo, template VUR
    // frente a official) SIN preferencia explícita.

    /// T-010 (hyfetch real: repository-fuente 2.1.0 fusionada, binario vup
    /// 2.0.5 en OTRO repo): sin flags se compila la fuente; el binario de
    /// otro repo es invisible (vul_binary_available solo se consulta para
    /// el repo del candidato).
    #[test]
    fn t010_hyfetch_sin_flags_compila_fuente_aunque_haya_binario_en_otro_repo() {
        let src = MockSource {
            vur: [(
                "hyfetch",
                ("repository", vur_info_ver("hyfetch", "2.1.0", &["x86_64"])),
            )]
            .into(),
            binaries: [("vup", "hyfetch")].into(),
            ..MockSource::default()
        };
        let plan = resolve(&targets(&["hyfetch"]), &src, &ResolveOptions::default()).unwrap();
        assert!(plan.installs.is_empty());
        assert_eq!(build_names(&plan), vec!["hyfetch"]);
        assert_eq!(plan.builds[0].info.version, "2.1.0");
    }

    /// T-010 (hytale-installer real: binario official + template cnr): sin
    /// flags el bucket official gana y el template VUR ni se mira.
    #[test]
    fn t010_hytale_sin_flags_oficial_gana_a_template_vur() {
        let src = MockSource {
            official: ["hytale-installer"].into(),
            vur: [(
                "hytale-installer",
                (
                    "cnr",
                    vur_info_ver("hytale-installer", "2.0.0", &["x86_64"]),
                ),
            )]
            .into(),
            ..MockSource::default()
        };
        let plan = resolve(
            &targets(&["hytale-installer"]),
            &src,
            &ResolveOptions::default(),
        )
        .unwrap();
        assert!(plan.builds.is_empty());
        assert_eq!(plan.installs.len(), 1);
        assert_eq!(
            plan.installs[0].action,
            Action::Install(BinarySource::Official)
        );
    }

    /// T-010 (triple vía sintética: official + binario vup + fuente cnr):
    /// sin flags el orden es official > VUR fusionado, sin importar que
    /// exista binario firmado disponible en otro repo.
    #[test]
    fn t010_triple_via_sin_flags_oficial_gana() {
        let src = MockSource {
            official: ["triapp"].into(),
            vur: [(
                "triapp",
                ("cnr", vur_info_ver("triapp", "3.0.0", &["x86_64"])),
            )]
            .into(),
            binaries: [("vup", "triapp")].into(),
            ..MockSource::default()
        };
        let plan = resolve(&targets(&["triapp"]), &src, &ResolveOptions::default()).unwrap();
        assert!(plan.builds.is_empty());
        assert_eq!(plan.installs.len(), 1);
        assert_eq!(
            plan.installs[0].action,
            Action::Install(BinarySource::Official)
        );
    }

    // --- T-010: matriz de aceptación 3-caminos (flags explícitos). Casos
    // reales: hyfetch (repository-fuente 2.1.0 vs vup-binario 2.0.5),
    // hytale-installer (binario official + template cnr).

    fn opts_con_orden(
        prefer_binary: bool,
        force_build: bool,
        order: CandidateOrder,
    ) -> ResolveOptions {
        ResolveOptions {
            prefer_binary,
            force_build,
            order,
        }
    }

    fn hyfetch_multi() -> MockSource {
        MockSource {
            vur_multi: [(
                "hyfetch",
                vec![
                    ("repository", vur_info_ver("hyfetch", "2.1.0", &["x86_64"])),
                    ("vup", vur_info_ver("hyfetch", "2.0.5", &["x86_64"])),
                ],
            )]
            .into(),
            binaries: [("vup", "hyfetch")].into(),
            ..MockSource::default()
        }
    }

    /// Aceptación T-010 (1/2): `hyfetch --prefer-binary` instala el binario
    /// vup aunque la fuente repository sea más nueva; el plan lo muestra.
    #[test]
    fn t010_prefer_binary_cambia_de_repo_al_binario() {
        let src = hyfetch_multi();
        let opts = opts_con_orden(true, false, CandidateOrder::PreferBinary);
        let plan = resolve(&targets(&["hyfetch"]), &src, &opts).unwrap();
        assert!(plan.builds.is_empty());
        assert!(plan.warnings.is_empty());
        assert_eq!(plan.installs.len(), 1);
        assert_eq!(
            plan.installs[0].action,
            Action::Install(BinarySource::VulBinary {
                repo: "vup".to_string()
            })
        );
        assert_eq!(plan.installs[0].info.version, "2.0.5");
    }

    /// Aceptación T-010 (mitad de 2/2): `--force-build` sobre official CON
    /// template VUR compila el template en vez del binario official.
    #[test]
    fn t010_force_build_sobre_official_compila_template_vur() {
        let src = MockSource {
            official: ["hytale-installer"].into(),
            vur: [(
                "hytale-installer",
                (
                    "cnr",
                    vur_info_ver("hytale-installer", "2.0.0", &["x86_64"]),
                ),
            )]
            .into(),
            ..MockSource::default()
        };
        let opts = opts_con_orden(true, true, CandidateOrder::PreferSource);
        let plan = resolve(&targets(&["hytale-installer"]), &src, &opts).unwrap();
        assert!(plan.installs.is_empty());
        assert!(plan.warnings.is_empty());
        assert_eq!(build_names(&plan), vec!["hytale-installer"]);
    }

    /// Aceptación T-010 (mitad de 2/2): `--force-build` sobre official SIN
    /// template VUR instala el official pero AVISA (nunca en silencio).
    #[test]
    fn t010_force_build_sobre_official_sin_template_avisa() {
        let src = MockSource {
            official: ["solo-official"].into(),
            ..MockSource::default()
        };
        let opts = opts_con_orden(true, true, CandidateOrder::PreferSource);
        let plan = resolve(&targets(&["solo-official"]), &src, &opts).unwrap();
        assert!(plan.builds.is_empty());
        assert_eq!(plan.installs.len(), 1);
        assert_eq!(
            plan.installs[0].action,
            Action::Install(BinarySource::Official)
        );
        assert_eq!(plan.warnings.len(), 1);
        assert!(
            plan.warnings[0].contains("sin efecto"),
            "aviso inesperado: {}",
            plan.warnings[0]
        );
    }

    /// `--no-prefer-binary` ordena la fuente primero aunque el binario sea
    /// de otro repo (hyfetch -> repository 2.1.0 compilada).
    #[test]
    fn t010_no_prefer_binary_ordena_fuente_primero() {
        let src = hyfetch_multi();
        let opts = opts_con_orden(false, false, CandidateOrder::PreferSource);
        let plan = resolve(&targets(&["hyfetch"]), &src, &opts).unwrap();
        assert!(plan.installs.is_empty());
        assert_eq!(build_names(&plan), vec!["hyfetch"]);
        assert_eq!(plan.builds[0].info.version, "2.1.0");
    }

    /// Sin binario en ningún repo, el orden explícito cae a versión desc:
    /// gana la más nueva aunque su repo tenga peor prioridad.
    #[test]
    fn t010_sin_binarios_gana_mayor_version() {
        let src = MockSource {
            vur_multi: [(
                "app",
                vec![
                    ("repo-b", vur_info_ver("app", "1.9.0", &["x86_64"])),
                    ("repo-a", vur_info_ver("app", "1.10.0", &["x86_64"])),
                ],
            )]
            .into(),
            priorities: [("repo-a", 50), ("repo-b", 10)].into(),
            ..MockSource::default()
        };
        let opts = opts_con_orden(true, false, CandidateOrder::PreferBinary);
        let plan = resolve(&targets(&["app"]), &src, &opts).unwrap();
        assert_eq!(build_names(&plan), vec!["app"]);
        assert_eq!(plan.builds[0].info.version, "1.10.0");
    }

    /// A igualdad de clase y versión, gana la prioridad de repo más baja.
    #[test]
    fn t010_empate_version_gana_prioridad_repo() {
        let src = MockSource {
            vur_multi: [(
                "app",
                vec![
                    ("repo-b", vur_info_ver("app", "1.0", &["x86_64"])),
                    ("repo-a", vur_info_ver("app", "1.0", &["x86_64"])),
                ],
            )]
            .into(),
            binaries: [("repo-a", "app"), ("repo-b", "app")].into(),
            priorities: [("repo-a", 10), ("repo-b", 50)].into(),
            ..MockSource::default()
        };
        let opts = opts_con_orden(true, false, CandidateOrder::PreferBinary);
        let plan = resolve(&targets(&["app"]), &src, &opts).unwrap();
        assert_eq!(plan.installs.len(), 1);
        assert_eq!(
            plan.installs[0].action,
            Action::Install(BinarySource::VulBinary {
                repo: "repo-a".to_string()
            })
        );
    }

    /// Comparador de versiones: numérico por token, no lexicográfico.
    #[test]
    fn t010_cmp_pkgver_compara_tokens_numericos() {
        assert_eq!(cmp_pkgver("2.1.0", 1, "2.0.5", 1), Ordering::Greater);
        assert_eq!(cmp_pkgver("1.10.0", 1, "1.9.0", 1), Ordering::Greater);
        assert_eq!(cmp_pkgver("1.0", 2, "1.0", 1), Ordering::Greater);
        assert_eq!(cmp_pkgver("1.0", 1, "1.0", 1), Ordering::Equal);
        assert_eq!(cmp_pkgver("1.0", 1, "1.0.1", 1), Ordering::Less);
    }
}
