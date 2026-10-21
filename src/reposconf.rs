use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

// Campos desconocidos = RECHAZADOS: en el fichero de CONFIANZA un typo
// (`enabld = false`, `key_fingerrprint = ...`) se ignoraba en silencio y
// cambiaba el comportamiento (el repo que el usuario quería deshabilitar
// quedaba ACTIVO, el pin no se aplicaba). Mismo precedente del repo que
// con los flags: "no mentir, rechazar" (H-032/H-043).
#[derive(Serialize, Deserialize, Default, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ReposConf {
    #[serde(rename = "vur", default)]
    pub vur: BTreeMap<String, RepoEntry>,
}

#[derive(Serialize, Deserialize, Default, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RepoEntry {
    pub url: String,
    pub branch: Option<String>,
    pub priority: Option<i64>,
    pub key_fingerprint: Option<String>,
    pub binary_repo_url: Option<String>,
    /// URL opcional de un índice binario estilo VUP (`index.json`).
    /// Si está presente, vary instala los binarios de ese repo sin clonar
    /// plantillas (adaptador Fase 1, ver `vup_index`).
    pub index_url: Option<String>,
    pub enabled: Option<bool>,
    /// P0-2: cuándo se confió por primera vez en la llave binaria del repo
    /// (epoch unix, fijado al registrar; `rekey` lo borra, `re-trust` lo
    /// renueva). Ausente = registrado antes de este campo (fecha desconocida,
    /// no se exige re-confianza). Solo informativo/forense: nunca decide.
    pub trusted_at: Option<i64>,
}

impl RepoEntry {
    pub fn branch_or_default(&self) -> &str {
        self.branch.as_deref().unwrap_or("main")
    }

    pub fn priority_or(&self, default: i64) -> i64 {
        self.priority.unwrap_or(default)
    }

    pub fn enabled_or(&self, default: bool) -> bool {
        self.enabled.unwrap_or(default)
    }

    pub fn has_binary(&self) -> bool {
        self.binary_repo_url
            .as_deref()
            .is_some_and(|url| !url.trim().is_empty())
    }

    pub fn has_vup_index(&self) -> bool {
        self.index_url
            .as_deref()
            .is_some_and(|url| !url.trim().is_empty())
    }
}

impl ReposConf {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                tracing::warn!(
                    "{} no existe, usando configuración de repos por defecto",
                    path.display()
                );
                return Ok(Self::default());
            }
            Err(err) => {
                return Err(err).with_context(|| format!("no se pudo leer {}", path.display()));
            }
        };
        let conf: Self = toml::from_str(&text)
            .with_context(|| format!("TOML inválido en {}", path.display()))?;
        // El nombre del repo se usa como RUTA (`vurs_dir().join(name)`) y como
        // nombre de fichero de llave. La entrada por CLI ya se validaba
        // (`keys::validate_repo_name`); el fichero editado a mano NO pasaba por
        // esa puerta: una clave "../../tmp/x" escapaba del directorio de VURs.
        for name in conf.vur.keys() {
            crate::keys::validate_repo_name(name)
                .with_context(|| format!("repos.conf: nombre de repo inválido '{name}'"))?;
        }
        Ok(conf)
    }

    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        if !parent.as_os_str().is_empty() {
            crate::util::ensure_private_dir(parent)
                .with_context(|| format!("no se pudo crear {}", parent.display()))?;
        }
        // Escritura atómica (tmp + fsync + rename, el patrón de `db.rs`): un
        // corte a mitad dejaba `repos.conf` truncado y TODOS los comandos con
        // repos morían de golpe. Es el fichero de confianza (URLs, pins,
        // trusted_at): perderlo es perder la confianza, no un annoyance.
        let data = toml::to_string_pretty(self)?;
        let mut temp = tempfile::NamedTempFile::new_in(parent)
            .with_context(|| format!("creando temporal para {}", path.display()))?;
        use std::io::Write;
        temp.write_all(data.as_bytes())
            .with_context(|| format!("escribiendo temporal para {}", path.display()))?;
        temp.flush()?;
        temp.as_file().sync_all()?;
        temp.persist(path)
            .map_err(|e| e.error)
            .with_context(|| format!("renombrando a {}", path.display()))?;
        Ok(())
    }

    pub fn sorted_by_priority(&self) -> Vec<(String, &RepoEntry)> {
        let mut repos: Vec<(String, &RepoEntry)> = self
            .vur
            .iter()
            .filter(|(_, entry)| entry.enabled_or(true))
            .map(|(name, entry)| (name.clone(), entry))
            .collect();
        repos.sort_by(|a, b| {
            a.1.priority_or(100)
                .cmp(&b.1.priority_or(100))
                .then_with(|| a.0.cmp(&b.0))
        });
        repos
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_TOML: &str = r#"
[vur.mi-repo]
url = "https://github.com/usuario/mi-repo"
branch = "main"
priority = 10
key_fingerprint = "60:ae:0c:d6:f0:95:17:80:bc:93:46:7a:89:af:a3:2d"
binary_repo_url = "https://usuario.github.io/mi-repo/bin"
enabled = true
"#;

    /// Un nombre de repo con traversal (`../../tmp/x`) NO se acepta: se usa
    /// como ruta del clon VUR. La entrada por CLI ya lo validaba; el
    /// fichero editado a mano se colaba.
    #[test]
    fn nombre_de_repo_con_traversal_se_rechaza_al_cargar() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("repos.conf");
        std::fs::write(
            &path,
            r#"[vur."../../tmp/evil"]
url = "file:///tmp/evil"
"#,
        )
        .unwrap();
        let err = ReposConf::load(&path).unwrap_err();
        assert!(
            format!("{err:#}").contains("nombre de repo inválido"),
            "{err:#}"
        );
    }

    /// Campo desconocido = typo = ERROR (no se ignora en silencio: un
    /// `enabld = false` dejaba el repo ACTIVO).
    #[test]
    fn campo_desconocido_se_rechaza() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("repos.conf");
        std::fs::write(
            &path,
            r#"[vur.mi-repo]
url = "https://example.org/r.git"
enabld = false
"#,
        )
        .unwrap();
        let err = ReposConf::load(&path).unwrap_err();
        assert!(format!("{err:#}").contains("enabld"), "{err:#}");
    }

    /// `save()` escribe atómicamente (tmp + rename): el fichero anterior se
    /// sustituye entero y no queda ningún temporal por el camino.
    #[test]
    fn save_sustituye_el_fichero_y_no_deja_temporales() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("repos.conf");
        let conf: ReposConf = toml::from_str(SAMPLE_TOML).unwrap();
        conf.save(&path).unwrap();
        conf.save(&path).unwrap();

        let back = ReposConf::load(&path).unwrap();
        assert_eq!(back.vur.len(), 1);
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .filter(|name| name != "repos.conf")
            .collect();
        assert!(
            leftovers.is_empty(),
            "temporales sin limpiar: {leftovers:?}"
        );
    }

    #[test]
    fn parses_exact_sample() {
        let conf: ReposConf = toml::from_str(SAMPLE_TOML).unwrap();
        assert_eq!(conf.vur.len(), 1);
        let entry = conf.vur.get("mi-repo").unwrap();
        assert_eq!(entry.url, "https://github.com/usuario/mi-repo");
        assert_eq!(entry.branch_or_default(), "main");
        assert_eq!(entry.priority_or(100), 10);
        assert_eq!(
            entry.key_fingerprint.as_deref(),
            Some("60:ae:0c:d6:f0:95:17:80:bc:93:46:7a:89:af:a3:2d")
        );
        assert_eq!(
            entry.binary_repo_url.as_deref(),
            Some("https://usuario.github.io/mi-repo/bin")
        );
        assert!(entry.enabled_or(false));
        assert!(entry.has_binary());
    }

    #[test]
    fn defaults_are_sane() {
        let entry = RepoEntry::default();
        assert_eq!(entry.branch_or_default(), "main");
        assert_eq!(entry.priority_or(100), 100);
        assert_eq!(entry.priority_or(42), 42);
        assert!(entry.enabled_or(true));
        assert!(!entry.enabled_or(false));
        assert!(!entry.has_binary());
        assert!(!entry.has_vup_index());
        let blank_binary = RepoEntry {
            binary_repo_url: Some("   ".into()),
            ..Default::default()
        };
        assert!(!blank_binary.has_binary());
    }

    #[test]
    fn missing_file_yields_default() {
        let dir = tempfile::tempdir().unwrap();
        let conf = ReposConf::load(dir.path().join("nope.conf")).unwrap();
        assert!(conf.vur.is_empty());
        assert_eq!(conf, ReposConf::default());
    }

    #[test]
    fn save_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let original: ReposConf = toml::from_str(SAMPLE_TOML).unwrap();

        let path = dir.path().join("repos.conf");
        original.save(&path).unwrap();
        assert_eq!(ReposConf::load(&path).unwrap(), original);

        let nested = dir.path().join("sub/dir/repos.conf");
        original.save(&nested).unwrap();
        assert_eq!(ReposConf::load(&nested).unwrap(), original);
    }

    #[test]
    fn sorted_by_priority_orders_and_filters() {
        let mut conf = ReposConf::default();
        conf.vur.insert(
            "gamma".into(),
            RepoEntry {
                priority: Some(30),
                url: "g".into(),
                ..Default::default()
            },
        );
        conf.vur.insert(
            "alpha".into(),
            RepoEntry {
                priority: Some(20),
                url: "a".into(),
                ..Default::default()
            },
        );
        conf.vur.insert(
            "delta".into(),
            RepoEntry {
                priority: Some(1),
                url: "d".into(),
                enabled: Some(false),
                ..Default::default()
            },
        );
        conf.vur.insert(
            "beta".into(),
            RepoEntry {
                priority: Some(10),
                url: "b".into(),
                ..Default::default()
            },
        );
        conf.vur.insert(
            "zeta".into(),
            RepoEntry {
                url: "z".into(),
                ..Default::default()
            },
        );

        let names: Vec<String> = conf
            .sorted_by_priority()
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        assert_eq!(
            names,
            vec![
                "beta".to_string(),
                "alpha".to_string(),
                "gamma".to_string(),
                "zeta".to_string()
            ]
        );
    }

    #[test]
    fn corrupt_toml_returns_error_with_context() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("corrupt.conf");
        std::fs::write(&path, "this is invalid toml = [").unwrap();

        let err = ReposConf::load(&path).unwrap_err();
        let err_msg = format!("{err:#}");
        assert!(
            err_msg.contains("TOML inválido"),
            "Error message must contain 'TOML inválido': {err_msg}"
        );
        assert!(
            err_msg.contains("corrupt.conf"),
            "Error message must contain file path: {err_msg}"
        );
    }
}
