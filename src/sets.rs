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
//! | `@system`    | `base-system` (con `--deep`, xbps-src construye su árbol de dependencias desde fuente) |
//! | `@world`     | unión de `@selected` + `@system` (dedup)    |
//! | `@optimized` | paquetes `InstallType::Source` de la DB (compilados por vary: oficiales y VUR) |
//! | `@vur`       | paquetes con origen VUR en la DB (fuera los compuestos `official`) |
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

/// Núcleo puro de expansión (testeable sin xbps ni DB):
/// `selected` = manuales instalados, `installed` = todos los instalados,
/// `sources` = compilados por vary (InstallType::Source),
/// `vurs` = con origen VUR en la DB. Dedup preservando orden en todos los
/// sets; un set vacío es error (no hay nada accionable que instalar).
pub fn expand_from(
    name: &str,
    selected: &[String],
    installed: &[String],
    sources: &[String],
    vurs: &[String],
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

/// Expande UN set con datos del sistema real: `xbps-query -m`/`-l` y la DB
/// de vary (la fuente de dónde se compiló cada paquete).
pub fn expand(name: &str, _config: &Config, db: &InstalledDb) -> Result<Vec<String>> {
    // Validar PRIMERO (sin tocar xbps): un set desconocido es error claro
    // incluso si xbps-query no está disponible.
    if !VALID_SETS.contains(&name) {
        bail!(
            "set '{name}' desconocido; sets válidos: {}",
            VALID_SETS.join(", ")
        );
    }
    let selected = crate::xbps::query_manual_names()
        .with_context(|| format!("no se pudo listar lo instalado manualmente (set '{name}')"))?;
    let installed = crate::xbps::query_installed_names()
        .with_context(|| format!("no se pudo listar los paquetes instalados (set '{name}')"))?;
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
    expand_from(name, &selected, &installed, &sources, &vurs)
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
            expand_from("@selected", &sel, &ins, &src, &vur).unwrap(),
            vec!["a", "b"]
        );
        assert_eq!(
            expand_from("@installed", &sel, &ins, &src, &vur).unwrap(),
            vec!["a", "b", "c"]
        );
    }

    #[test]
    fn system_es_base_system() {
        let (sel, ins, src, vur) = s(&[], &[], &[], &[]);
        assert_eq!(
            expand_from("@system", &sel, &ins, &src, &vur).unwrap(),
            vec!["base-system"]
        );
    }

    #[test]
    fn world_union_con_dedup() {
        // base-system ya seleccionado: no se repite.
        let (sel, ins, src, vur) = s(&["base-system", "x", "y"], &[], &[], &[]);
        assert_eq!(
            expand_from("@world", &sel, &ins, &src, &vur).unwrap(),
            vec!["base-system", "x", "y"]
        );
        let (sel, ins, src, vur) = s(&["a"], &[], &[], &[]);
        assert_eq!(
            expand_from("@world", &sel, &ins, &src, &vur).unwrap(),
            vec!["a", "base-system"]
        );
    }

    #[test]
    fn optimized_y_vur_filtran_por_db() {
        let (sel, ins, src, vur) = s(&[], &[], &["b1", "b2"], &["v1", "v2"]);
        assert_eq!(
            expand_from("@optimized", &sel, &ins, &src, &vur).unwrap(),
            vec!["b1", "b2"]
        );
        assert_eq!(
            expand_from("@vur", &sel, &ins, &src, &vur).unwrap(),
            vec!["v1", "v2"]
        );
    }

    #[test]
    fn orden_preservado_y_dedup_en_entrada() {
        let (sel, ins, src, vur) = s(&["z", "a", "z"], &[], &[], &[]);
        assert_eq!(
            expand_from("@selected", &sel, &ins, &src, &vur).unwrap(),
            vec!["z", "a"]
        );
    }

    #[test]
    fn set_vacio_es_error() {
        let (sel, ins, src, vur) = s(&[], &["x"], &[], &["v"]);
        let err = format!(
            "{:#}",
            expand_from("@optimized", &sel, &ins, &src, &vur).unwrap_err()
        );
        assert!(err.contains("no resuelve ningún paquete"), "{err}");
    }

    #[test]
    fn set_desconocido_lista_validos() {
        let (sel, ins, src, vur) = s(&["a"], &[], &[], &[]);
        let err = format!(
            "{:#}",
            expand_from("@noexiste", &sel, &ins, &src, &vur).unwrap_err()
        );
        assert!(err.contains("@noexiste"), "{err}");
        assert!(err.contains("@selected"), "{err}");
        assert!(err.contains("@vur"), "{err}");
    }

    #[test]
    fn targets_mezcla_sets_nombres_y_dedupe_total() {
        // Orden de primera aparición gana; el nombre literal aparece una vez.
        let cfg = Config::default();
        let db = InstalledDb::default();
        // Sin xbps en el entorno de test NO se puede llamar expand (consulta
        // real): probamos la dedupe total con el núcleo + el orden del
        // wrapper vía un set estático.
        let (sel, ins, src, vur) = s(&[], &[], &[], &[]);
        let set = expand_from("@system", &sel, &ins, &src, &vur).unwrap();
        let mut out = vec!["git".to_string()];
        out.extend(set);
        out.push("git".to_string());
        let total = crate::sets::dedup(out);
        assert_eq!(total, vec!["git", "base-system"]);
        // expand_targets rechaza sets desconocidos con error claro.
        let targets = strs(&["git", "@noexiste"]);
        let err = format!("{:#}", expand_targets(&targets, &cfg, &db).unwrap_err());
        assert!(err.contains("@noexiste"), "{err}");
    }
}
