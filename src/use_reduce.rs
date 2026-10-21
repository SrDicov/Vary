//! Reducción condicional de dependencias (`use_reduce` de Portage).
//!
//! ITEM 3 de `docs/vary2/ORDEN-v2.md`. Los templates de Void expresan deps
//! condicionales así (ejemplos REALES del clon):
//!
//! ```text
//! makedepends="nodejs cbindgen nasm which tar xz $(vopt_if wasi wasi-sdk)"
//! hostmakedepends="$(vopt_if pgo 'xvfb-run dbus mesa-dri vulkan-loader pciutils')"
//! ```
//!
//! Sin reducir, esos fragmentos se colarían en el grafo como nombres de
//! paquete (`$(vopt_if` no existe) -> "paquete no encontrado". Aquí:
//!
//! 1. [`split_deps`] parte la lista de forma consciente de `$( ... )`;
//! 2. [`reduce`] expande la construcción según las opciones ACTIVAS del
//!    usuario (las que se mandan a xbps-src como `XBPS_PKG_OPTIONS`).

/// Opciones activas del usuario, como DATOS (el resolver es puro).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ActiveOptions {
    /// Opciones globales (`[make] global_options`).
    pub global: Vec<String>,
    /// Opciones por paquete (`[options] <pkg>`).
    pub per_pkg: std::collections::BTreeMap<String, Vec<String>>,
}

impl ActiveOptions {
    /// Opciones activas de `pkg`.
    ///
    /// Semántica de xbps-src: lo por paquete SUSTITUYE a lo global y una
    /// lista vacía NO sustituye (xbps-src ignora `XBPS_PKG_OPTIONS_<pkg>`
    /// vacío y aplica las globales). Un `~opcion` desactiva.
    pub fn for_pkg(&self, pkg: &str) -> Vec<String> {
        effective_options(&self.global, self.per_pkg.get(pkg))
    }
}

/// Opciones activas (comunes a la auditoría y al resolutor).
pub fn effective_options(global: &[String], per_pkg: Option<&Vec<String>>) -> Vec<String> {
    let raw = match per_pkg {
        Some(v) if !v.is_empty() => v,
        _ => global,
    };
    let mut opts: Vec<String> = raw
        .iter()
        .filter(|o| !o.starts_with('~'))
        .cloned()
        .collect();
    opts.sort();
    opts.dedup();
    opts
}

/// Una construcción `$(vopt_if <opción> <deps_si_activa> <deps_si_inactiva>)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoptIf {
    pub option: String,
    /// Deps cuando la opción está activa.
    pub on: Vec<String>,
    /// Deps cuando la opción está inactiva.
    pub off: Vec<String>,
}

/// Divide un valor de shell en palabras, conservando intactos los grupos
/// `$( ... )` y respetando comillas.
pub fn split_deps(value: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut depth = 0usize;
    let mut quote: Option<char> = None;
    for ch in value.chars() {
        match quote {
            Some(q) => {
                cur.push(ch);
                if ch == q {
                    quote = None;
                }
            }
            None => match ch {
                '\'' | '"' => {
                    quote = Some(ch);
                    cur.push(ch);
                }
                '(' if cur.ends_with('$') => {
                    depth += 1;
                    cur.push(ch);
                }
                ')' if depth > 0 => {
                    depth -= 1;
                    cur.push(ch);
                }
                c if c.is_whitespace() && depth == 0 => {
                    if !cur.is_empty() {
                        out.push(std::mem::take(&mut cur));
                    }
                }
                _c => cur.push(ch),
            },
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Palabras de shell (respeta comillas; `''` produce una palabra vacía).
fn shell_words(value: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut has_word = false;
    let mut quote: Option<char> = None;
    for ch in value.chars() {
        match quote {
            Some(q) => {
                if ch == q {
                    quote = None;
                } else {
                    cur.push(ch);
                }
            }
            None => match ch {
                '\'' | '"' => {
                    quote = Some(ch);
                    has_word = true;
                }
                c if c.is_whitespace() => {
                    if has_word {
                        out.push(std::mem::take(&mut cur));
                        has_word = false;
                    }
                }
                c => {
                    cur.push(c);
                    has_word = true;
                }
            },
        }
    }
    if has_word {
        out.push(cur);
    }
    out
}

/// Parsea un token `$(vopt_if ...)`. `None` si no es esa construcción.
pub fn parse_vopt_if(token: &str) -> Option<VoptIf> {
    let rest = token.strip_prefix("$(vopt_if")?;
    let rest = rest.strip_suffix(')')?;
    let words = shell_words(rest);
    let option = words.first()?.clone();
    let on = words
        .get(1)
        .map(|w| w.split_whitespace().map(str::to_string).collect())
        .unwrap_or_default();
    let off = words
        .get(2)
        .map(|w| w.split_whitespace().map(str::to_string).collect())
        .unwrap_or_default();
    Some(VoptIf { option, on, off })
}

/// Deps efectivas según las opciones activas.
///
/// Los tokens `$(vopt_if ...)` se expanden; el resto se conserva tal cual.
pub fn reduce(deps: &[String], active: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for dep in deps {
        match parse_vopt_if(dep) {
            Some(v) => {
                if active.iter().any(|a| a == &v.option) {
                    out.extend(v.on.iter().cloned());
                } else {
                    out.extend(v.off.iter().cloned());
                }
            }
            None => out.push(dep.clone()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn split_conserva_grupos_de_comando() {
        // Ejemplos reales del clon (srcpkgs/firefox/template).
        let deps = split_deps("nodejs cbindgen nasm which tar xz $(vopt_if wasi wasi-sdk)");
        assert_eq!(
            deps,
            s(&[
                "nodejs",
                "cbindgen",
                "nasm",
                "which",
                "tar",
                "xz",
                "$(vopt_if wasi wasi-sdk)"
            ])
        );
    }

    #[test]
    fn split_con_comillas_multi_palabra() {
        let deps = split_deps(
            "libXdamage-devel freetype-devel $(vopt_if wasi wasi-sdk) $(vopt_if alsa alsa-lib-devel)",
        );
        assert_eq!(deps.len(), 4);
        assert_eq!(deps[3], "$(vopt_if alsa alsa-lib-devel)");
    }

    #[test]
    fn parse_vopt_if_simple_y_citado() {
        let v = parse_vopt_if("$(vopt_if wasi wasi-sdk)").unwrap();
        assert_eq!(v.option, "wasi");
        assert_eq!(v.on, s(&["wasi-sdk"]));
        assert!(v.off.is_empty());

        let v = parse_vopt_if("$(vopt_if pgo 'xvfb-run dbus mesa-dri vulkan-loader pciutils')")
            .unwrap();
        assert_eq!(v.option, "pgo");
        assert_eq!(
            v.on,
            s(&["xvfb-run", "dbus", "mesa-dri", "vulkan-loader", "pciutils"])
        );

        let v = parse_vopt_if("$(vopt_if gir 'gobject-introspection-devel' '')").unwrap();
        assert_eq!(v.option, "gir");
        assert_eq!(v.on, s(&["gobject-introspection-devel"]));
        assert!(v.off.is_empty());

        assert!(parse_vopt_if("$(vopt_with foo '--with-foo')").is_none());
        assert!(parse_vopt_if("glibc").is_none());
    }

    #[test]
    fn reduce_expande_segun_opciones_activas() {
        let deps = s(&[
            "glibc",
            "$(vopt_if pgo 'xvfb-run dbus')",
            "$(vopt_if wasi wasi-sdk)",
        ]);
        // pgo activa, wasi inactiva.
        let out = reduce(&deps, &s(&["pgo"]));
        assert_eq!(out, s(&["glibc", "xvfb-run", "dbus"]));

        // pgo inactiva, wasi activa.
        let out = reduce(&deps, &s(&["wasi"]));
        assert_eq!(out, s(&["glibc", "wasi-sdk"]));

        // ninguna activa: solo las deps literales.
        let out = reduce(&deps, &[]);
        assert_eq!(out, s(&["glibc"]));
    }

    #[test]
    fn reduce_conserva_deps_literales() {
        let deps = s(&["glibc", "openssl>=3.0.8_1"]);
        assert_eq!(reduce(&deps, &[]), deps);
    }
}
