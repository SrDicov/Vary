//! Conjuntos de paquetes `@name` (targets de `-S`).
//!
//! Un target que empieza por `@` no es un paquete: es un set que se expande
//! a una lista de nombres antes de resolver (ver [`expand_targets`] y el
//! dispatch en `lib.rs`). Sets válidos:
//!
//! | Set          | Expande a                                   |
//! |--------------|---------------------------------------------|
//! | `@selected`  | paquetes instalados manualmente (`xbps-query -m`) |
//! | `@installed` | todos los instalados (`xbps-query -l`)      |
//! | `@system`    | `base-system` (árbol completo de deps desde fuente: `--deep` pasa `-N` a xbps-src (deps desde fuente)) |
//! | `@world`     | unión de `@selected` + `@system` (dedup)    |
//! | `@optimized` | paquetes `InstallType::Source` de la DB (compilados por vary: oficiales y VUR) |
//! | `@vur`       | paquetes con origen VUR en la DB (fuera los compuestos `official`) |
//! | `@preserved-rebuild` | compilados por vary con pins de sonames en drift (ver `preserved_from`) |
//!
//! No hay sets anidados ni persistentes: los sets definen un conjunto de
//! TARGETS, no una lista editable ("no world de Gentoo": lo manual ya vive
//! en xbps).

use anyhow::{bail, Context, Result};

use crate::config::Config;
use crate::db::{InstallType, InstalledDb};

pub const VALID_SETS: &[&str] = &[
    "@selected",
    "@installed",
    "@system",
    "@world",
    "@optimized",
    "@vur",
    "@preserved-rebuild",
];

/// ¿es este target un set? (`@`-prefijo).
pub fn is_set(name: &str) -> bool {
    name.starts_with('@')
}

/// Dedupe preservando el orden de primera aparición.
fn dedup(mut names: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    names.retain(|n| seen.insert(n.clone()));
    names
}

/// Paquetes compilados por vary (`InstallType::Source`) cuyos pins de
/// sonames reportan drift — el set `@preserved-rebuild` necesita
/// recompilación. Puro y testeable: el checker se inyecta
/// (en runtime, [`crate::soname::check_db_entry`]). Dedup preservando orden.
pub fn preserved_from(
    entries: &[(String, crate::db::Entry)],
    check: impl Fn(&str, &crate::soname::SonamePins) -> Vec<crate::soname::SonameDrift>,
) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (name, entry) in entries {
        if entry.install_type != InstallType::Source {
            continue;
        }
        let Some(pins) = entry.soname_pins.as_ref().filter(|p| !p.is_empty()) else {
            continue;
        };
        if check(name, pins).is_empty() {
            continue;
        }
        if seen.insert(name.clone()) {
            out.push(name.clone());
        }
    }
    out
}

/// Núcleo puro de expansión (testeable sin xbps ni DB):
/// `selected` = manuales instalados, `installed` = todos los instalados,
/// `sources` = compilados por vary (InstallType::Source),
/// `vurs` = con origen VUR en la DB, `preserved` = en drift de sonames.
/// Dedup preservando orden en todos los
/// sets; un set vacío es error (no hay nada accionable que instalar).
pub fn expand_from(
    name: &str,
    selected: &[String],
    installed: &[String],
    sources: &[String],
    vurs: &[String],
    preserved: &[String],
) -> Result<Vec<String>> {
    let names = match name {
        "@selected" => dedup(selected.to_vec()),
        "@installed" => dedup(installed.to_vec()),
        "@system" => vec!["base-system".to_string()],
        "@world" => {
            let mut world = selected.to_vec();
            world.push("base-system".to_string());
            dedup(world)
        }
        "@optimized" => dedup(sources.to_vec()),
        "@vur" => dedup(vurs.to_vec()),
        "@preserved-rebuild" => dedup(preserved.to_vec()),
        _ => bail!(
            "set '{name}' desconocido; sets válidos: {}",
            VALID_SETS.join(", ")
        ),
    };
    if names.is_empty() {
        bail!("set '{name}' no resuelve ningún paquete (nada que instalar)");
    }
    Ok(names)
}

/// Expande UN set con datos del sistema real: consulta SOLO la fuente que
/// el set necesita (`xbps-query` o la DB de vary, dónde se compiló cada
/// paquete); las listas no usadas quedan vacías.
pub fn expand(name: &str, _config: &Config, db: &InstalledDb) -> Result<Vec<String>> {
    // Validar PRIMERO (sin tocar xbps ni la DB): un set desconocido es error
    // claro incluso si xbps-query no está disponible.
    if !VALID_SETS.contains(&name) {
        bail!(
            "set '{name}' desconocido; sets válidos: {}",
            VALID_SETS.join(", ")
        );
    }
    let (selected, installed, sources, vurs, preserved) = match name {
        // @selected y @world: solo `-m`; @system: un literal, ni xbps ni DB.
        "@selected" | "@world" => (
            crate::xbps::query_manual_names().with_context(|| {
                format!("no se pudo listar lo instalado manualmente (set '{name}')")
            })?,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        ),
        // @installed: solo `-l`.
        "@installed" => (
            Vec::new(),
            crate::xbps::query_installed_names().with_context(|| {
                format!("no se pudo listar los paquetes instalados (set '{name}')")
            })?,
            Vec::new(),
            Vec::new(),
            Vec::new(),
        ),
        // @optimized y @vur: solo DB (entries_snapshot), sin xbps.
        "@optimized" | "@vur" => {
            let snap = db.entries_snapshot();
            let sources: Vec<String> = snap
                .iter()
                .filter(|(_, e)| e.install_type == InstallType::Source)
                .map(|(n, _)| n.clone())
                .collect();
            let vurs: Vec<String> = snap
                .iter()
                .filter(|(_, e)| e.vur != "official")
                .map(|(n, _)| n.clone())
                .collect();
            (Vec::new(), Vec::new(), sources, vurs, Vec::new())
        }
        // @preserved-rebuild: DB (pins) + pkgdb local vía check_db_entry.
        "@preserved-rebuild" => {
            let snap = db.entries_snapshot();
            let preserved = preserved_from(&snap, crate::soname::check_db_entry);
            (Vec::new(), Vec::new(), Vec::new(), Vec::new(), preserved)
        }
        // name ∈ VALID_SETS ya validado arriba; cubre solo "@system".
        _ => (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new()),
    };
    expand_from(name, &selected, &installed, &sources, &vurs, &preserved)
}

/// Expande los sets de una lista de targets mezclándolos con los nombres
/// literales; el total sale deduplicado preservando el orden.
pub fn expand_targets(
    targets: &[String],
    config: &Config,
    db: &InstalledDb,
) -> Result<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    for t in targets {
        if is_set(t) {
            out.extend(expand(t, config, db)?);
        } else {
            out.push(t.clone());
        }
    }
    Ok(dedup(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strs(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_string()).collect()
    }

    fn s(
        selected: &[&str],
        installed: &[&str],
        sources: &[&str],
        vurs: &[&str],
    ) -> (Vec<String>, Vec<String>, Vec<String>, Vec<String>) {
        (strs(selected), strs(installed), strs(sources), strs(vurs))
    }

    #[test]
    fn is_set_solo_prefijo_at() {
        assert!(is_set("@world"));
        assert!(is_set("@"));
        assert!(!is_set("base-system"));
        assert!(!is_set(""));
    }

    #[test]
    fn selected_y_installed_devuelven_lo_dado() {
        let (sel, ins, src, vur) = s(&["a", "b"], &["a", "b", "c"], &[], &[]);
        assert_eq!(
            expand_from("@selected", &sel, &ins, &src, &vur, &[]).unwrap(),
            vec!["a", "b"]
        );
        assert_eq!(
            expand_from("@installed", &sel, &ins, &src, &vur, &[]).unwrap(),
            vec!["a", "b", "c"]
        );
    }

    #[test]
    fn system_es_base_system() {
        let (sel, ins, src, vur) = s(&[], &[], &[], &[]);
        assert_eq!(
            expand_from("@system", &sel, &ins, &src, &vur, &[]).unwrap(),
            vec!["base-system"]
        );
    }

    #[test]
    fn world_union_con_dedup() {
        // base-system ya seleccionado: no se repite.
        let (sel, ins, src, vur) = s(&["base-system", "x", "y"], &[], &[], &[]);
        assert_eq!(
            expand_from("@world", &sel, &ins, &src, &vur, &[]).unwrap(),
            vec!["base-system", "x", "y"]
        );
        let (sel, ins, src, vur) = s(&["a"], &[], &[], &[]);
        assert_eq!(
            expand_from("@world", &sel, &ins, &src, &vur, &[]).unwrap(),
            vec!["a", "base-system"]
        );
    }

    #[test]
    fn optimized_y_vur_filtran_por_db() {
        let (sel, ins, src, vur) = s(&[], &[], &["b1", "b2"], &["v1", "v2"]);
        assert_eq!(
            expand_from("@optimized", &sel, &ins, &src, &vur, &[]).unwrap(),
            vec!["b1", "b2"]
        );
        assert_eq!(
            expand_from("@vur", &sel, &ins, &src, &vur, &[]).unwrap(),
            vec!["v1", "v2"]
        );
    }

    #[test]
    fn orden_preservado_y_dedup_en_entrada() {
        let (sel, ins, src, vur) = s(&["z", "a", "z"], &[], &[], &[]);
        assert_eq!(
            expand_from("@selected", &sel, &ins, &src, &vur, &[]).unwrap(),
            vec!["z", "a"]
        );
    }

    #[test]
    fn set_vacio_es_error() {
        let (sel, ins, src, vur) = s(&[], &["x"], &[], &["v"]);
        let err = format!(
            "{:#}",
            expand_from("@optimized", &sel, &ins, &src, &vur, &[]).unwrap_err()
        );
        assert!(err.contains("no resuelve ningún paquete"), "{err}");
    }

    #[test]
    fn set_desconocido_lista_validos() {
        let (sel, ins, src, vur) = s(&["a"], &[], &[], &[]);
        let err = format!(
            "{:#}",
            expand_from("@noexiste", &sel, &ins, &src, &vur, &[]).unwrap_err()
        );
        assert!(err.contains("@noexiste"), "{err}");
        assert!(err.contains("@selected"), "{err}");
        assert!(err.contains("@vur"), "{err}");
    }

    #[test]
    fn expand_system_funciona_sin_xbps_ni_db() {
        // @system es un literal: no necesita xbps-query ni DB del host.
        let out = expand("@system", &Config::default(), &InstalledDb::default()).unwrap();
        assert_eq!(out, vec!["base-system"]);
    }

    #[test]
    fn targets_mezcla_sets_nombres_y_dedupe_total() {
        // Orden de primera aparición gana; el nombre literal aparece una vez.
        // @system no toca xbps, así que expand_targets corre en el host de test.
        let cfg = Config::default();
        let db = InstalledDb::default();
        let targets = strs(&["foo", "@system", "foo"]);
        assert_eq!(
            expand_targets(&targets, &cfg, &db).unwrap(),
            vec!["foo", "base-system"]
        );
        // expand_targets rechaza sets desconocidos con error claro.
        let targets = strs(&["git", "@noexiste"]);
        let err = format!("{:#}", expand_targets(&targets, &cfg, &db).unwrap_err());
        assert!(err.contains("@noexiste"), "{err}");
    }

    fn entry(t: InstallType, pins: Option<crate::soname::SonamePins>) -> crate::db::Entry {
        crate::db::Entry {
            version: "1.0_1".to_string(),
            vur: "official".to_string(),
            install_date: 0,
            build_date: None,
            install_type: t,
            repo_commit: None,
            artifact_sha256: None,
            soname_pins: pins,
        }
    }

    fn pins() -> Option<crate::soname::SonamePins> {
        Some(
            [("libfoo.so.1".to_string(), "1.0_1".to_string())]
                .into_iter()
                .collect(),
        )
    }

    fn drift_checker(
    ) -> impl Fn(&str, &crate::soname::SonamePins) -> Vec<crate::soname::SonameDrift> {
        |_pkg, _pins| {
            vec![crate::soname::SonameDrift::Orphaned {
                shlib: "libfoo.so.1".to_string(),
                pinned: "libfoo-1.0_1".to_string(),
            }]
        }
    }

    #[test]
    fn preserved_filtra_pins_vacios_no_source_y_dedup() {
        let entries = vec![
            ("a".to_string(), entry(InstallType::Source, pins())),
            (
                "b".to_string(),
                entry(InstallType::Source, Some(crate::soname::SonamePins::new())),
            ),
            ("c".to_string(), entry(InstallType::Binary, pins())),
            ("a".to_string(), entry(InstallType::Source, pins())),
        ];
        assert_eq!(preserved_from(&entries, drift_checker()), vec!["a"]);
        // Sin drift: nada preservado.
        assert!(preserved_from(&entries, |_p, _pins| Vec::new()).is_empty());
    }

    #[test]
    fn expanded_preserved_rebuild_y_vacio() {
        let (sel, ins, src, vur) = s(&[], &[], &[], &[]);
        assert_eq!(
            expand_from(
                "@preserved-rebuild",
                &sel,
                &ins,
                &src,
                &vur,
                &strs(&["a", "b", "a"])
            )
            .unwrap(),
            vec!["a", "b"]
        );
        let err = format!(
            "{:#}",
            expand_from("@preserved-rebuild", &sel, &ins, &src, &vur, &[]).unwrap_err()
        );
        assert!(err.contains("no resuelve ningún paquete"), "{err}");
    }

    #[test]
    fn expand_preserved_con_db_vacia_es_error_sin_xbps() {
        // DB default vacía: preserved_from no invoca el checker, no toca xbps.
        let err = format!(
            "{:#}",
            expand(
                "@preserved-rebuild",
                &Config::default(),
                &InstalledDb::default()
            )
            .unwrap_err()
        );
        assert!(err.contains("no resuelve ningún paquete"), "{err}");
    }
}
