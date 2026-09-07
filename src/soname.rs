//! P2-soname: pin de sonames al instalar + aviso consultivo de drift en upgrade.
//!
//! Alcance aprobado (2026-09-07): al instalar un paquete **Source** se graba
//! qué versión de cada proveedor servía cada `shlib-requires`; en `vary -Syu`,
//! si un proveedor cambió de versión (o el soname ya no lo provee nadie) se
//! emite un AVISO que sugiere recompilar. **Nunca recompila solo**: compilar
//! sin consentimiento explícito es peligroso, así que no hay flag ni prompt
//! (apto para no-TTY y `--yes`).
//!
//! Fuente de datos: `xbps-query -S --property=...` en modo LOCAL (pkgdb del
//! sistema, sin red). Sin baseline (paquetes pre-0.4.0) no hay aviso ni ruido.
//! Toda captura es best-effort: si xbps no responde, no hay pins y punto.

use std::collections::{BTreeMap, BTreeSet};

/// Pins: `shlib` → `pkgver` del proveedor que lo servía al instalar
/// (p. ej. `"libcurl.so.4" → "libcurl-8.22.0_1"`).
pub type SonamePins = BTreeMap<String, String>;

/// Drift detectado entre baseline pineada y estado actual.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SonameDrift {
    /// El proveedor cambió de versión pero sigue proveyendo el soname.
    /// Sospecha de ABI distinta → recompilar sugerido.
    Bumped {
        shlib: String,
        from: String,
        to: String,
    },
    /// El soname sigue requerido pero ningún proveedor actual lo sirve
    /// (bump de soname mayor, p. ej. `libfoo.so.1` → `libfoo.so.2`).
    Orphaned { shlib: String, pinned: String },
}

/// Nombre base de un rundep (`libcurl>=8.22.0_1` → `libcurl`).
/// Operadores de restricción de xbps: `>=`, `<=`, `=`, `>`, `<`.
pub fn rundep_base(dep: &str) -> &str {
    let end = dep.find(['<', '>', '=']).unwrap_or(dep.len());
    dep[..end].trim()
}

/// Foto actual `shlib → pkgver del proveedor` para un paquete INSTALADO.
///
/// `query(pkg, prop)` devuelve las líneas de
/// `xbps-query -S --property=prop pkg` (vacío si falla). El mapeo
/// shlib→proveedor se resuelve intersectando `shlib-requires` del paquete
/// con `shlib-provides` de cada rundep: sin ficción, lo no mapeable se omite.
pub fn snapshot(query: &dyn Fn(&str, &str) -> Vec<String>, pkg: &str) -> SonamePins {
    let requires: BTreeSet<String> = query(pkg, "shlib-requires")
        .into_iter()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    if requires.is_empty() {
        return BTreeMap::new();
    }
    let mut pins = SonamePins::new();
    for dep_line in query(pkg, "run_depends") {
        let dep = rundep_base(&dep_line);
        if dep.is_empty() {
            continue;
        }
        let provides: BTreeSet<String> = query(dep, "shlib-provides")
            .into_iter()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect();
        if provides.is_empty() {
            continue;
        }
        let ver = query(dep, "pkgver")
            .into_iter()
            .map(|l| l.trim().to_string())
            .find(|l| !l.is_empty());
        let Some(ver) = ver else { continue };
        for shlib in requires.intersection(&provides) {
            pins.entry(shlib.clone()).or_insert_with(|| ver.clone());
        }
        if pins.len() == requires.len() {
            break;
        }
    }
    pins
}

/// Compara baseline pineada contra foto actual (puro, testeable).
///
/// `fresh` = snapshot actual; `still_required` = `shlib-requires` actual del
/// paquete. Un pin ausente en `fresh` solo es huérfano si el paquete SIGUE
/// requiriendo ese soname (si ya no lo requiere, el paquete cambió por otra
/// vía y los pins se refrescarán en su próxima instalación).
pub fn detect(
    pinned: &SonamePins,
    fresh: &SonamePins,
    still_required: &BTreeSet<String>,
) -> Vec<SonameDrift> {
    let mut out = Vec::new();
    for (shlib, old) in pinned {
        match fresh.get(shlib) {
            Some(now) if now != old => out.push(SonameDrift::Bumped {
                shlib: shlib.clone(),
                from: old.clone(),
                to: now.clone(),
            }),
            None if still_required.contains(shlib) => out.push(SonameDrift::Orphaned {
                shlib: shlib.clone(),
                pinned: old.clone(),
            }),
            _ => {}
        }
    }
    out
}

/// Texto del aviso para un drift (puro).
pub fn format_drift(pkg: &str, drift: &SonameDrift) -> String {
    match drift {
        SonameDrift::Bumped { shlib, from, to } => format!(
            "{pkg}: el proveedor de {shlib} cambió ({from} → {to}); \
             el binario se compiló contra la versión anterior — \
             considera recompilar con `vary -S {pkg}`"
        ),
        SonameDrift::Orphaned { shlib, pinned } => format!(
            "{pkg}: {shlib} (pineado de {pinned}) ya no lo provee ningún \
             paquete instalado — probable bump de soname mayor; \
             recompila con `vary -S {pkg}`"
        ),
    }
}

/// Query real contra el pkgdb local (sin red). rc != 0 o error → vacío.
pub fn live_query(pkg: &str, prop: &str) -> Vec<String> {
    let out = std::process::Command::new("xbps-query")
        .args(["-S", &format!("--property={prop}"), "--", pkg])
        .output();
    let Ok(out) = out else { return Vec::new() };
    if !out.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

/// Captura best-effort de pins para un paquete instalado.
pub fn capture(pkg: &str) -> SonamePins {
    snapshot(&live_query, pkg)
}

/// Chequeo completo de una entrada de la DB: foto actual + detección.
/// Nunca falla (errores → sin avisos, sin ruido).
pub fn check_db_entry(pkg: &str, pinned: &SonamePins) -> Vec<SonameDrift> {
    if pinned.is_empty() {
        return Vec::new();
    }
    let fresh = snapshot(&live_query, pkg);
    let required: BTreeSet<String> = live_query(pkg, "shlib-requires").into_iter().collect();
    detect(pinned, &fresh, &required)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Query falsa con claves propias (evita E0700 en el `impl Fn`).
    fn fake(table: BTreeMap<(String, String), Vec<String>>) -> impl Fn(&str, &str) -> Vec<String> {
        move |pkg: &str, prop: &str| {
            table
                .get(&(pkg.to_string(), prop.to_string()))
                .cloned()
                .unwrap_or_default()
        }
    }

    fn table() -> BTreeMap<(String, String), Vec<String>> {
        let raw: Vec<((&str, &str), Vec<&str>)> = vec![
            (
                ("demo", "shlib-requires"),
                vec!["libc.so.6", "libcurl.so.4", "libz.so.1"],
            ),
            (
                ("demo", "run_depends"),
                vec![
                    "ca-certificates>=0",
                    "glibc>=2.41_1",
                    "libcurl>=8.22.0_1",
                    "zlib>=1.2.3_1",
                ],
            ),
            (("glibc", "shlib-provides"), vec!["libc.so.6"]),
            (("glibc", "pkgver"), vec!["glibc-2.41_1"]),
            (("libcurl", "shlib-provides"), vec!["libcurl.so.4"]),
            (("libcurl", "pkgver"), vec!["libcurl-8.22.0_1"]),
            (("zlib", "shlib-provides"), vec!["libz.so.1"]),
            (("zlib", "pkgver"), vec!["zlib-1.3.1_1"]),
        ];
        raw.into_iter()
            .map(|((p, k), v)| {
                (
                    (p.to_string(), k.to_string()),
                    v.into_iter().map(str::to_string).collect(),
                )
            })
            .collect()
    }

    #[test]
    fn snapshot_mapea_cada_shlib_a_su_proveedor() {
        let q = fake(table());
        let pins = snapshot(&q, "demo");
        assert_eq!(
            pins.get("libc.so.6").map(String::as_str),
            Some("glibc-2.41_1")
        );
        assert_eq!(
            pins.get("libcurl.so.4").map(String::as_str),
            Some("libcurl-8.22.0_1")
        );
        assert_eq!(
            pins.get("libz.so.1").map(String::as_str),
            Some("zlib-1.3.1_1")
        );
        // ca-certificates no provee shlibs → no inventa pins para él.
        assert_eq!(pins.len(), 3);
    }

    #[test]
    fn snapshot_omite_shlib_sin_proveedor_conocido() {
        let mut t = table();
        t.insert(
            ("demo".to_string(), "shlib-requires".to_string()),
            vec!["libc.so.6".to_string(), "libfantasma.so.9".to_string()],
        );
        let q = fake(t);
        let pins = snapshot(&q, "demo");
        assert!(!pins.contains_key("libfantasma.so.9"));
        assert_eq!(pins.len(), 1);
    }

    #[test]
    fn snapshot_vacio_si_paquete_sin_shlibs_o_sin_xbps() {
        let q = fake(BTreeMap::new());
        assert!(snapshot(&q, "inexistente").is_empty());
        let mut t = table();
        t.insert(("demo".to_string(), "shlib-requires".to_string()), vec![]);
        let q = fake(t);
        assert!(snapshot(&q, "demo").is_empty());
    }

    #[test]
    fn rundep_base_recorta_operadores() {
        assert_eq!(rundep_base("libcurl>=8.22.0_1"), "libcurl");
        assert_eq!(rundep_base("foo<=1.0"), "foo");
        assert_eq!(rundep_base("bar=2.0_1"), "bar");
        assert_eq!(rundep_base("simple"), "simple");
    }

    fn pinned_demo() -> SonamePins {
        BTreeMap::from([
            ("libc.so.6".to_string(), "glibc-2.41_1".to_string()),
            ("libcurl.so.4".to_string(), "libcurl-8.22.0_1".to_string()),
        ])
    }

    #[test]
    fn detect_silencio_si_todo_igual() {
        let p = pinned_demo();
        let required: BTreeSet<String> = p.keys().cloned().collect();
        assert!(detect(&p, &p.clone(), &required).is_empty());
    }

    #[test]
    fn detect_bumped_cuando_proveedor_cambia_version() {
        let p = pinned_demo();
        let fresh: SonamePins = BTreeMap::from([
            ("libc.so.6".to_string(), "glibc-2.41_1".to_string()),
            ("libcurl.so.4".to_string(), "libcurl-8.23.0_1".to_string()),
        ]);
        let required: BTreeSet<String> = p.keys().cloned().collect();
        let drifts = detect(&p, &fresh, &required);
        assert_eq!(drifts.len(), 1);
        assert_eq!(
            drifts[0],
            SonameDrift::Bumped {
                shlib: "libcurl.so.4".to_string(),
                from: "libcurl-8.22.0_1".to_string(),
                to: "libcurl-8.23.0_1".to_string(),
            }
        );
    }

    #[test]
    fn detect_orfano_si_soname_desaparece_de_proveedores() {
        let p = pinned_demo();
        // libcurl nuevo ya no provee .so.4 (bump mayor) y nada más lo da.
        let fresh: SonamePins =
            BTreeMap::from([("libc.so.6".to_string(), "glibc-2.41_1".to_string())]);
        let required: BTreeSet<String> = p.keys().cloned().collect();
        let drifts = detect(&p, &fresh, &required);
        assert_eq!(drifts.len(), 1);
        assert!(matches!(drifts[0], SonameDrift::Orphaned { .. }));
    }

    #[test]
    fn detect_silencio_si_paquete_ya_no_requiere_el_soname() {
        let p = pinned_demo();
        let fresh: SonamePins =
            BTreeMap::from([("libc.so.6".to_string(), "glibc-2.41_1".to_string())]);
        // El paquete cambió por otra vía (ya no pide libcurl) → no es drift.
        let required: BTreeSet<String> = BTreeSet::from(["libc.so.6".to_string()]);
        assert!(detect(&p, &fresh, &required).is_empty());
    }

    #[test]
    fn format_menciona_paquete_y_versiones() {
        let d = SonameDrift::Bumped {
            shlib: "libcurl.so.4".to_string(),
            from: "libcurl-8.22.0_1".to_string(),
            to: "libcurl-8.23.0_1".to_string(),
        };
        let msg = format_drift("demo", &d);
        assert!(msg.contains("demo") && msg.contains("8.23.0_1"), "{msg}");
        let o = SonameDrift::Orphaned {
            shlib: "libx.so.1".to_string(),
            pinned: "libx-1.0_1".to_string(),
        };
        assert!(format_drift("demo", &o).contains("recompila"), "{o:?}");
    }
}
