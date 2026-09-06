use anyhow::{anyhow, bail, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VurInfo {
    pub format_version: u32,
    pub pkgname: String,
    pub version: String,
    pub revision: u32,
    pub archs: Vec<String>,
    #[serde(default)]
    pub subpackages: Vec<Subpackage>,
    #[serde(default)]
    pub depends: Vec<String>,
    #[serde(default)]
    pub hostmakedepends: Vec<String>,
    #[serde(default)]
    pub makedepends: Vec<String>,
    #[serde(default)]
    pub checkdepends: Vec<String>,
    #[serde(default)]
    pub build_style: Option<String>,
    #[serde(default)]
    pub distfiles: Vec<String>,
    #[serde(default)]
    pub checksum: Vec<String>,
    #[serde(default)]
    pub provides: Vec<String>,
    #[serde(default)]
    pub replaces: Vec<String>,
    #[serde(default)]
    pub restricted: bool,
    #[serde(default)]
    pub maintainer: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Subpackage {
    pub pkgname: String,
    #[serde(default)]
    pub depends: Vec<String>,
    #[serde(default)]
    pub short_desc: Option<String>,
}

const SUPPORTED_FORMAT_VERSION: u32 = 1;

impl VurInfo {
    /// Valida los metadatos según las reglas del formato v1.
    ///
    /// Reglas:
    /// 1. `format_version` debe ser exactamente 1.
    /// 2. `version` no vacía y con caracteres `[A-Za-z0-9._+]` únicamente
    ///    (los guiones están prohibidos: delimitan `pkgname-version_revision`
    ///    según la convención de nombrado de paquetes de Void Linux).
    /// 3. `revision` >= 1.
    /// 4. `archs` no vacío.
    /// 5. cada elemento de `checksum` no vacío y con prefijo `sha256:`; la
    ///    lista no puede estar vacía (un paquete sin sumas es inválido).
    /// 6. `pkgname` no vacío, caracteres `[a-zA-Z0-9._+-]` y sin empezar por `-`.
    /// 7. subpaquetes: nombre no vacío y distinto del padre; cada dependencia
    ///    una cadena no vacía ni blanca.
    pub fn validate(&self) -> Result<()> {
        if self.format_version != SUPPORTED_FORMAT_VERSION {
            bail!(
                "format_version no soportado: {} (soportado: {})",
                self.format_version,
                SUPPORTED_FORMAT_VERSION
            );
        }
        if self.pkgname.is_empty() {
            bail!("pkgname vacío: el nombre del paquete es obligatorio");
        }
        if !is_valid_pkgname(&self.pkgname) {
            bail!(
                "pkgname inválido: '{}' contiene caracteres prohibidos o empieza por '-' \
                 (permitidos: [a-zA-Z0-9._+-], sin '-' inicial)",
                self.pkgname
            );
        }
        if self.version.is_empty() {
            bail!("version vacía: la versión upstream es obligatoria");
        }
        if !is_valid_version(&self.version) {
            bail!(
                "version inválida: '{}' contiene caracteres prohibidos \
                 (permitidos: [A-Za-z0-9._+]); los guiones están prohibidos en la versión \
                 porque delimitan 'pkgname-version_revision' según la convención de Void Linux",
                self.version
            );
        }
        if self.revision < 1 {
            bail!("revision inválida: {} (debe ser >= 1)", self.revision);
        }
        if self.archs.is_empty() {
            bail!("archs vacío: debe declararse al menos una arquitectura objetivo");
        }
        if self.checksum.is_empty() {
            tracing::warn!(
                "{}: checksum vacío (paquete con do_fetch personalizado?)",
                self.pkgname
            );
        } else {
            for (i, sum) in self.checksum.iter().enumerate() {
                if sum == "SKIP" {
                    continue;
                }
                if sum.is_empty() || !sum.starts_with("sha256:") {
                    bail!(
                        "checksum[{i}] inválido: '{sum}' debe empezar por 'sha256:' o ser 'SKIP' \
                         (las cadenas vacías tampoco son válidas)"
                    );
                }
            }
        }
        for (i, sub) in self.subpackages.iter().enumerate() {
            if sub.pkgname.trim().is_empty() {
                bail!("subpackages[{i}].pkgname vacío: el nombre del subpaquete es obligatorio");
            }
            if !is_valid_pkgname(&sub.pkgname) {
                bail!(
                    "subpackages[{}].pkgname inválido: '{}' contiene caracteres prohibidos o no empieza por alfanumérico \
                     (permitidos: [a-zA-Z0-9._+-], primer carácter alfanumérico)",
                    i,
                    sub.pkgname
                );
            }
            if sub.pkgname == self.pkgname {
                bail!(
                    "subpackages[{}].pkgname duplicado: '{}' coincide con el pkgname del padre",
                    i,
                    sub.pkgname
                );
            }
            for (j, dep) in sub.depends.iter().enumerate() {
                if dep.trim().is_empty() {
                    bail!("subpackages[{i}].depends[{j}] inválida: la dependencia está vacía o solo contiene espacios");
                }
            }
        }
        Ok(())
    }

    /// Normaliza los metadatos tras el parseo.
    ///
    /// En el formato v1 TODOS los subpaquetes comparten las `archs` del paquete
    /// padre: `Subpackage` no tiene campo propio `archs`, así que no hay nada
    /// que heredar/migrar aquí. La herencia se expresa mediante
    /// los archs del padre. Si una versión futura añade un campo de
    /// arquitecturas por subpaquete, este método será el punto donde se
    /// materialice dicha herencia.
    pub fn normalize(&mut self) {}

    /// Arquitecturas efectivas de un subpaquete.
    ///
    /// En v1 todos los subpaquetes comparten las arquitecturas del padre, por
    /// lo que siempre devuelve `&self.archs`.
    /// `pkgver` estilo Void: `<pkgname>-<version>_<revision>`.
    pub fn pkgver(&self) -> String {
        format!("{}-{}_{}", self.pkgname, self.version, self.revision)
    }
}

/// Deserializa un único `VurInfo`, lo valida y lo normaliza.
pub fn parse(json: &str) -> Result<VurInfo> {
    let mut info: VurInfo =
        serde_json::from_str(json).map_err(|e| anyhow!("JSON malformado: {e}"))?;
    info.validate()?;
    info.normalize();
    Ok(info)
}

/// Deserializa un array JSON `[VurInfo, ...]`; acepta también un único objeto
/// `VurInfo` como fallback. Valida y normaliza cada entrada.
///
/// Los errores indican el índice y el `pkgname` de la entrada problemática.
pub fn parse_many(json: &str) -> Result<Vec<VurInfo>> {
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|e| anyhow!("JSON malformado: {e}"))?;
    let items: Vec<serde_json::Value> = match value {
        serde_json::Value::Array(items) => items,
        obj @ serde_json::Value::Object(_) => vec![obj],
        _ => bail!("JSON inesperado: se esperaba un array de VurInfo o un único objeto VurInfo"),
    };
    let mut out = Vec::with_capacity(items.len());
    for (idx, item) in items.into_iter().enumerate() {
        let label = item
            .get("pkgname")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("<sin pkgname>")
            .to_owned();
        let mut info: VurInfo = serde_json::from_value(item)
            .map_err(|e| anyhow!("entrada[{idx}] (pkgname '{label}'): {e}"))?;
        info.validate()
            .map_err(|e| anyhow!("entrada[{idx}] (pkgname '{label}'): {e}"))?;
        info.normalize();
        out.push(info);
    }
    Ok(out)
}

fn is_valid_version(v: &str) -> bool {
    !v.is_empty()
        && v.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '+'))
}

pub fn is_valid_pkgname(n: &str) -> bool {
    let mut chars = n.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphanumeric() => {
            chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '+' | '-'))
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CANONICAL: &str = r#"{"format_version":1,"pkgname":"foo","version":"1.2.3","revision":2,"archs":["x86_64","aarch64"],"subpackages":[{"pkgname":"foo-devel","depends":["foo>=1.2.3"],"short_desc":"Development files for foo"}],"depends":["libbar>=2.0"],"hostmakedepends":["pkg-config","ninja"],"makedepends":["libbar-devel"],"build_style":"cmake","distfiles":["https://example.com/foo-1.2.3.tar.gz"],"checksum":["sha256:abc123"],"provides":["libfoo.so.1"],"replaces":["old-foo"],"restricted":false,"maintainer":"name <email>"}"#;

    fn base_json(pkgname: &str, version: &str, revision: u32, archs_json: &str) -> String {
        format!(
            r#"{{"format_version":1,"pkgname":"{}","version":"{}","revision":{},"archs":{},"checksum":["sha256:abc123"]}}"#,
            pkgname, version, revision, archs_json
        )
    }

    #[test]
    fn parses_full_canonical_example() {
        let v = parse(CANONICAL).expect("debe parsear");
        assert_eq!(v.format_version, 1);
        assert_eq!(v.pkgname, "foo");
        assert_eq!(v.version, "1.2.3");
        assert_eq!(v.revision, 2);
        assert_eq!(v.archs, vec!["x86_64", "aarch64"]);
        assert_eq!(v.subpackages.len(), 1);
        let sub = &v.subpackages[0];
        assert_eq!(sub.pkgname, "foo-devel");
        assert_eq!(sub.depends, vec!["foo>=1.2.3"]);
        assert_eq!(sub.short_desc.as_deref(), Some("Development files for foo"));
        assert_eq!(v.depends, vec!["libbar>=2.0"]);
        assert_eq!(v.hostmakedepends, vec!["pkg-config", "ninja"]);
        assert_eq!(v.makedepends, vec!["libbar-devel"]);
        assert!(v.checkdepends.is_empty());
        assert_eq!(v.build_style.as_deref(), Some("cmake"));
        assert_eq!(v.distfiles, vec!["https://example.com/foo-1.2.3.tar.gz"]);
        assert_eq!(v.checksum, vec!["sha256:abc123"]);
        assert_eq!(v.provides, vec!["libfoo.so.1"]);
        assert_eq!(v.replaces, vec!["old-foo"]);
        assert!(!v.restricted);
        assert_eq!(v.maintainer.as_deref(), Some("name <email>"));
        assert_eq!(v.pkgver(), "foo-1.2.3_2");
    }

    #[test]
    fn validate_ok_on_canonical() {
        let v = parse(CANONICAL).unwrap();
        assert!(v.validate().is_ok());
    }

    #[test]
    fn applies_defaults_for_omitted_fields() {
        let json = r#"{"format_version":1,"pkgname":"bar","version":"0.9","revision":1,"archs":["x86_64"]}"#;
        // checksum omitido => lista vacía; para validar el default usamos
        // deserialización directa (validate exige >= 1 checksum).
        let mut v: VurInfo = serde_json::from_str(json).unwrap();
        assert!(v.subpackages.is_empty());
        assert!(v.depends.is_empty());
        assert!(v.hostmakedepends.is_empty());
        assert!(v.makedepends.is_empty());
        assert!(v.checkdepends.is_empty());
        assert_eq!(v.build_style, None);
        assert!(v.distfiles.is_empty());
        assert!(v.checksum.is_empty());
        assert!(v.provides.is_empty());
        assert!(v.replaces.is_empty());
        assert!(!v.restricted);
        assert_eq!(v.maintainer, None);
        // Y una vez rellenado checksum, validate pasa:
        v.checksum.push("sha256:def456".to_string());
        assert!(v.validate().is_ok());

        // Defaults vía parse() completo (con checksum presente):
        let full = parse(
            r#"{"format_version":1,"pkgname":"bar","version":"0.9","revision":1,"archs":["x86_64"],"distfiles":["https://example.com/bar-0.9.tar.gz"],"checksum":["sha256:def456"]}"#,
        )
        .expect("debe parsear");
        assert!(full.subpackages.is_empty());
        assert!(full.depends.is_empty());
        assert!(full.hostmakedepends.is_empty());
        assert!(full.makedepends.is_empty());
        assert!(full.checkdepends.is_empty());
        assert_eq!(full.build_style, None);
        assert!(full.provides.is_empty());
        assert!(full.replaces.is_empty());
        assert!(!full.restricted);
        assert_eq!(full.maintainer, None);
    }

    #[test]
    fn rejects_unsupported_format_version() {
        let json = CANONICAL.replace(r#""format_version":1"#, r#""format_version":2"#);
        let err = parse(&json).unwrap_err();
        assert!(
            err.to_string()
                .contains("format_version no soportado: 2 (soportado: 1)"),
            "error inesperado: {err}"
        );
    }

    #[test]
    fn rejects_version_with_hyphen() {
        let json = base_json("foo", "1.2-3", 1, r#"["x86_64"]"#);
        let err = parse(&json).unwrap_err();
        assert!(
            err.to_string().contains("guiones"),
            "error inesperado: {err}"
        );
    }

    #[test]
    fn rejects_empty_version() {
        let json = base_json("foo", "", 1, r#"["x86_64"]"#);
        let err = parse(&json).unwrap_err();
        assert!(
            err.to_string().contains("version vacía"),
            "error inesperado: {err}"
        );
    }

    #[test]
    fn rejects_zero_revision() {
        let json = base_json("foo", "1.2.3", 0, r#"["x86_64"]"#);
        let err = parse(&json).unwrap_err();
        assert!(
            err.to_string().contains("revision inválida: 0"),
            "error inesperado: {err}"
        );
    }

    #[test]
    fn rejects_checksum_without_sha256_prefix() {
        let json = base_json("foo", "1.2.3", 1, r#"["x86_64"]"#)
            .replace(r#"["sha256:abc123"]"#, r#"["abcdef"]"#);
        let err = parse(&json).unwrap_err();
        assert!(
            err.to_string().contains("checksum[0]") && err.to_string().contains("sha256:"),
            "error inesperado: {err}"
        );
    }

    #[test]
    fn rejects_checksum_elemento_vacio() {
        let json = base_json("foo", "1.2.3", 1, r#"["x86_64"]"#)
            .replace(r#"["sha256:abc123"]"#, r#"["sha256:x",""]"#);
        let err = parse(&json).unwrap_err();
        assert!(
            err.to_string().contains("checksum[1]"),
            "error inesperado: {err}"
        );
    }

    #[test]
    fn allows_empty_checksum_for_custom_fetch() {
        let json = base_json("foo", "1.2.3", 1, r#"["x86_64"]"#)
            .replace(r#""checksum":["sha256:abc123"], "#, "")
            .replace(r#","checksum":["sha256:abc123"]"#, "");
        let v = parse(&json).expect("checksum vacío debe permitirse para do_fetch personalizado");
        assert!(v.checksum.is_empty());
        assert!(v.validate().is_ok());
    }

    #[test]
    fn rejects_empty_archs() {
        let json = base_json("foo", "1.2.3", 1, r#"[]"#);
        let err = parse(&json).unwrap_err();
        assert!(
            err.to_string().contains("archs vacío"),
            "error inesperado: {err}"
        );
    }

    #[test]
    fn rejects_invalid_pkgname() {
        for bad in [
            "-foo", ".foo", "_foo", "+foo", "fo o", "foo$", "foo/bar", "foo;bar", "",
        ] {
            let json = base_json(bad, "1.2.3", 1, r#"["x86_64"]"#);
            let err = parse(&json).unwrap_err();
            assert!(
                err.to_string().contains("pkgname inválido")
                    || err.to_string().contains("pkgname vacío"),
                "error inesperado para '{bad}': {err}"
            );
        }
    }

    #[test]
    fn rejects_invalid_subpackage_name() {
        for bad in ["-sub", ".sub", "_sub", "+sub", "sub space", "sub$"] {
            let json = format!(
                r#"{{"format_version":1,"pkgname":"foo","version":"1.0","revision":1,"archs":["x86_64"],"checksum":["sha256:aa"],"subpackages":[{{"pkgname":"{bad}"}}]}}"#
            );
            let err = parse(&json).unwrap_err();
            assert!(
                err.to_string().contains("subpackages[0].pkgname inválido"),
                "error inesperado para subpaquete '{bad}': {err}"
            );
        }
    }

    #[test]
    fn rejects_subpackage_named_as_parent() {
        let json = r#"{"format_version":1,"pkgname":"foo","version":"1.0","revision":1,"archs":["x86_64"],"checksum":["sha256:aa"],"subpackages":[{"pkgname":"foo"}]}"#.to_string();
        let err = parse(&json).unwrap_err();
        assert!(
            err.to_string()
                .contains("coincide con el pkgname del padre"),
            "error inesperado: {err}"
        );
    }

    #[test]
    fn rejects_empty_subpackage_name() {
        let json = r#"{"format_version":1,"pkgname":"foo","version":"1.0","revision":1,"archs":["x86_64"],"checksum":["sha256:aa"],"subpackages":[{"pkgname":""}]}"#.to_string();
        let err = parse(&json).unwrap_err();
        assert!(
            err.to_string().contains("subpackages[0].pkgname vacío"),
            "error inesperado: {err}"
        );
    }

    #[test]
    fn rejects_blank_subpackage_dependency() {
        for bad_dep in ["", "   "] {
            let json = format!(
                r#"{{"format_version":1,"pkgname":"foo","version":"1.0","revision":1,"archs":["x86_64"],"checksum":["sha256:aa"],"subpackages":[{{"pkgname":"foo-devel","depends":["{bad_dep}"]}}]}}"#
            );
            let err = parse(&json).unwrap_err();
            assert!(
                err.to_string().contains("subpackages[0].depends[0]"),
                "error inesperado: {err}"
            );
        }
    }

    #[test]
    fn rejects_malformed_json() {
        let err = parse("{not json").unwrap_err();
        assert!(
            err.to_string().contains("JSON malformado"),
            "error inesperado: {err}"
        );
    }

    #[test]
    fn parse_many_accepts_array_of_two() {
        let a = base_json("foo", "1.0", 1, r#"["x86_64"]"#);
        let b = base_json("baz", "2.0", 3, r#"["aarch64"]"#);
        let json = format!("[{a},{b}]");
        let out = parse_many(&json).expect("debe parsear");
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].pkgname, "foo");
        assert_eq!(out[1].pkgname, "baz");
        assert_eq!(out[1].pkgver(), "baz-2.0_3");
    }

    #[test]
    fn parse_many_accepts_single_object_fallback() {
        let out = parse_many(CANONICAL).expect("debe parsear");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].pkgname, "foo");
        assert_eq!(out[0].pkgver(), "foo-1.2.3_2");
    }

    #[test]
    fn parse_many_reports_index_and_pkgname_on_invalid_entry() {
        let good = base_json("ok", "1.0", 1, r#"["x86_64"]"#);
        let bad = base_json("bad", "1.0", 0, r#"["x86_64"]"#);
        let json = format!("[{good},{bad}]");
        let err = parse_many(&json).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("entrada[1]") && msg.contains("'bad'") && msg.contains("revision"),
            "error inesperado: {msg}"
        );
    }

    #[test]
    fn serde_roundtrip_preserves_equality() {
        let v = parse(CANONICAL).unwrap();
        let s = serde_json::to_string(&v).unwrap();
        let w = parse(&s).unwrap();
        assert_eq!(v, w);
    }
}
