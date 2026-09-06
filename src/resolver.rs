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
    /// arquitectura actual del sistema/masterdir (ej "x86_64")
    fn arch(&self) -> String;
}

/// Opciones de resolución.
#[derive(Debug, Clone)]
pub struct ResolveOptions {
    /// Si hay binario firmado disponible, prefiero instalarlo antes que compilar.
    pub prefer_binary: bool,
    /// Fuerza compilar desde fuente aunque haya binario disponible.
    pub force_build: bool,
}

impl Default for ResolveOptions {
    fn default() -> Self {
        Self {
            prefer_binary: true,
            force_build: false,
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
        }
    }

    /// Punto 5: ¿el paquete soporta la arquitectura actual?
    fn arch_supported(&self, info: &VurInfo) -> bool {
        info.archs.contains(&self.arch) || info.archs.iter().any(|a| a == "all" || a == "noarch")
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
            bail!("ciclo de dependencias detectado: {}", cycle);
        }
        // Punto 4: ya resuelto por otro camino (diamante): reutilizar nodo.
        if let Some(&idx) = self.index.get(name) {
            if let Some(parent) = dependent {
                self.link(idx, parent);
            }
            return Ok(idx);
        }

        // Punto 1a: oficial -> Install(Official), NODO HOJA (sin recursión).
        if self.source.official_exists(name) {
            let item = Self::official_item(name, &self.arch);
            return Ok(self.push_item(item, dependent));
        }

        // Puntos 1b/1c y 5: VUR por nombre o provides, filtrando arquitectura.
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
        let Some((repo, info)) = candidate else {
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
            bail!("paquete no encontrado en repos oficiales ni VURs: {}", name);
        };

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
        VurInfo {
            format_version: 1,
            pkgname: pkgname.to_string(),
            version: "1.0".to_string(),
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
        provides: HashMap<&'static str, (Repo, VurInfo)>,
        binaries: HashSet<(Repo, &'static str)>,
        arch: &'static str,
    }

    impl Default for MockSource {
        fn default() -> Self {
            Self {
                official: HashSet::new(),
                vur: HashMap::new(),
                provides: HashMap::new(),
                binaries: HashSet::new(),
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
}
