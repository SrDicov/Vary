use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Serialize, Deserialize, Default, Debug, Clone, PartialEq)]
pub struct ReposConf {
    #[serde(rename = "vur", default)]
    pub vur: BTreeMap<String, RepoEntry>,
}

#[derive(Serialize, Deserialize, Default, Debug, Clone, PartialEq)]
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
        toml::from_str(&text).with_context(|| format!("TOML inválido en {}", path.display()))
    }

    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("no se pudo crear {}", parent.display()))?;
            }
        }
        std::fs::write(path, toml::to_string_pretty(self)?)
            .with_context(|| format!("no se pudo escribir {}", path.display()))?;
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
        let err_msg = format!("{:#}", err);
        assert!(
            err_msg.contains("TOML inválido"),
            "Error message must contain 'TOML inválido': {}",
            err_msg
        );
        assert!(
            err_msg.contains("corrupt.conf"),
            "Error message must contain file path: {}",
            err_msg
        );
    }
}
