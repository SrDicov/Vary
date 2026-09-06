//! Flujo de instalación binaria desde repos VUR firmados (Fase 6).
//!
//! Pasos exactos:
//! 1. Detectar que el VUR tiene `binary_repo_url`.
//! 2. Descubrir la llave pública RSA en el repo git (`keys/*.rsa|*.pem`).
//! 3. Mostrar fingerprint SHA256 al usuario para confirmación interactiva,
//!    comparándolo con `key_fingerprint` de repos.conf si está declarado.
//! 4. Copiar la llave a /etc/xbps.d/keys/vur-<nombre>.pem.
//! 5. Crear /etc/xbps.d/20-vur-<nombre>.conf apuntando al binary_repo_url.
//! 6. `xbps-install -S <pkg>` valida las firmas nativamente (XBPS rechaza
//!    cualquier binario alterado sin lógica extra en vary).
use crate::reposconf::RepoEntry;
use crate::util::confirm;
use crate::vur_client::VurRepo;
use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};


pub fn keys_dir() -> &'static str {
    "/etc/xbps.d/keys"
}

/// Valida estrictamente el nombre de un repositorio para prevenir path traversal
/// o sobreescritura de archivos de configuración de sistema oficiales.
pub fn validate_repo_name(name: &str) -> Result<()> {
    if name.is_empty() {
        bail!("el nombre del repositorio no puede estar vacío");
    }
    let first = name.chars().next().unwrap();
    if !first.is_ascii_alphanumeric() {
        bail!(
            "el nombre del repositorio '{}' debe comenzar con un carácter alfanumérico",
            name
        );
    }
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-') {
        bail!(
            "el nombre del repositorio '{}' contiene caracteres inválidos (solo a-z, 0-9, ., _, -)",
            name
        );
    }
    if name.contains("..") || name.contains('/') || name.contains('\\') {
        bail!(
            "intento de path traversal detectado en el nombre del repositorio '{}'",
            name
        );
    }
    if name.starts_with("00-") || name == "keys" {
        bail!("nombre de repositorio reservado o no permitido: '{}'", name);
    }
    Ok(())
}

pub fn repo_conf_path(name: &str) -> Result<String> {
    validate_repo_name(name)?;
    Ok(format!("/etc/xbps.d/20-vur-{}.conf", name))
}

pub fn key_dest_path(name: &str) -> Result<String> {
    validate_repo_name(name)?;
    Ok(format!("{}/vary-vur-{}.pem", keys_dir(), name))
}

pub(crate) fn write_root_file(
    contents: &str,
    dest: &str,
    mode: &str,
    sudo_bin: &str,
    sudo_flags: &[String],
) -> Result<()> {
    let tmp = std::env::temp_dir().join(format!(
        "vary-{}.tmp",
        std::process::id()
    ));
    std::fs::write(&tmp, contents).context("escribiendo archivo temporal")?;
    let status = crate::elevate::elevate(sudo_bin, sudo_flags, "install")?
        .args(["-m", mode])
        .arg(&tmp)
        .arg(dest)
        .status()
        .context("elevando privilegios para escribir archivo del sistema")?;
    let _ = std::fs::remove_file(&tmp);
    if !status.success() {
        bail!("no se pudo escribir {} (código {:?})", dest, status.code());
    }
    Ok(())
}

/// Prepara el repo binario del VUR para xbps-install: llave + conf.
///
/// Si `expected_fingerprint` (de repos.conf) no coincide con el calculado,
/// aborta sugiriendo `vary --repo rekey <nombre>`.
pub fn setup_binary_repo(
    repo: &VurRepo,
    entry: &RepoEntry,
    sudo_bin: &str,
    sudo_flags: &[String],
    no_confirm: bool,
) -> Result<()> {
    // (1) ¿tiene binary_repo_url?
    let Some(binary_url) = entry.binary_repo_url.as_deref().filter(|u| !u.trim().is_empty())
    else {
        bail!(
            "el VUR '{}' es source-only (sin binary_repo_url); solo puede compilarse",
            repo.name
        );
    };

    // (2) Llave pública dentro del clon git del VUR
    let Some(key_path) = repo.discover_public_key() else {
        bail!(
            "el VUR '{}' declara binary_repo_url pero no incluye llave pública \
             en keys/ (convención: keys/<fingerprint>.rsa)",
            repo.name
        );
    };

    // (3) Fingerprint + confirmación interactiva
    let fp = VurRepo::fingerprint_sha256(&key_path)?;
    if let Some(expected) = entry.key_fingerprint.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        let expected_norm = expected.to_lowercase();
        let fp_norm = fp.to_lowercase();
        if expected_norm != fp_norm {
            bail!(
                "fingerprint de llave del VUR '{}' NO coincide:\n  esperado (repos.conf): {}\n  recibido:              {}\n\
                 Si el mantenedor rotó la llave legítimamente, ejecuta: vary --repo rekey {}",
                repo.name, expected, fp, repo.name
            );
        }
    } else {
        tracing::warn!(
            "el VUR '{}' no declara key_fingerprint en repos.conf; verifica visualmente",
            repo.name
        );
    }
    println!("VUR '{}': llave pública {}", repo.name, key_path.display());
    println!("  SHA256: {}", fp);
    println!("  binary repo: {}", binary_url);
    if !confirm("¿Confiás en esta llave y deseas registrar este repositorio binario?", no_confirm)?
    {
        bail!("registro de repositorio binario cancelado por el usuario");
    }

    // (4) Verificar contra llave preexistente en disco (protección TOFU contra rotación no verificada)
    let dest_key = key_dest_path(&repo.name)?;
    verify_key_tofu(std::path::Path::new(&dest_key), &fp, &repo.name)?;

    // Copiar llave a /etc/xbps.d/keys/
    let pem = std::fs::read_to_string(&key_path)
        .with_context(|| format!("leyendo {}", key_path.display()))?;
    write_root_file(&pem, &dest_key, "644", sudo_bin, sudo_flags)?;

    // (5) Registrar el repositorio binario
    let conf = format!("repository={}\n", binary_url);
    let conf_path = repo_conf_path(&repo.name)?;
    write_root_file(&conf, &conf_path, "644", sudo_bin, sudo_flags)?;
    tracing::info!("repositorio binario '{}' registrado en {}", repo.name, conf_path);
    Ok(())
}

/// Registra un repo binario estilo VUP (adaptador Fase 1).
///
/// A diferencia de `setup_binary_repo` (una sola `binary_repo_url` + llave
/// PEM en el clon), aquí hay una URL de repo por tupla categoría-arquitectura
/// y la llave viene ya decodificada del plist del repo (`keys/*.plist`).
/// Se escriben TODAS las `repository=` necesarias en el mismo conf, que es
/// lo que xbps espera (un conf admite varias líneas `repository=`).
pub fn setup_vup_binary_repo(
    name: &str,
    repo_urls: &[String],
    key_pem: &str,
    entry: &RepoEntry,
    sudo_bin: &str,
    sudo_flags: &[String],
    no_confirm: bool,
) -> Result<()> {
    let mut urls: Vec<&str> = Vec::new();
    for u in repo_urls {
        let u = u.trim();
        if !u.is_empty() && !urls.contains(&u) {
            urls.push(u);
        }
    }
    if urls.is_empty() {
        bail!(
            "el repo VUP '{}' no aporta ninguna URL binaria para esta arquitectura",
            name
        );
    }

    // (1) Fingerprint de la llave recibida
    let der = crate::vur_client::decode_pem_body(key_pem)?;
    let digest = Sha256::digest(&der);
    let fp = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join(":");

    if let Some(expected) = entry.key_fingerprint.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        let expected_norm = expected.to_lowercase();
        let fp_norm = fp.to_lowercase();
        if expected_norm != fp_norm {
            bail!(
                "fingerprint de llave del repo VUP '{}' NO coincide:\n  esperado (repos.conf): {}\n  recibido:              {}\n\
                 Si el mantenedor rotó la llave legítimamente, ejecuta: vary --repo rekey {}",
                name, expected, fp, name
            );
        }
    } else {
        tracing::warn!(
            "el repo VUP '{}' no declara key_fingerprint en repos.conf; verifica visualmente",
            name
        );
    }

    println!("Repo VUP '{}': llave pública del índice", name);
    println!("  SHA256: {}", fp);
    println!("  binary repos ({} urls):", urls.len());
    for u in &urls {
        println!("    {}", u);
    }
    if !confirm("¿Confiás en esta llave y deseas registrar estos repositorios binarios?", no_confirm)?
    {
        bail!("registro de repositorios binarios cancelado por el usuario");
    }

    // (2) Verificar contra llave preexistente en disco (protección TOFU contra rotación no verificada)
    let dest_key = key_dest_path(name)?;
    verify_key_tofu(std::path::Path::new(&dest_key), &fp, name)?;

    // Copiar llave a /etc/xbps.d/keys/
    write_root_file(key_pem, &dest_key, "644", sudo_bin, sudo_flags)?;

    // (3) Registrar el conf con todas las URLs
    let mut conf = String::new();
    for url in urls {
        conf.push_str(&format!("repository={}\n", url));
    }
    let conf_path = repo_conf_path(name)?;
    write_root_file(&conf, &conf_path, "644", sudo_bin, sudo_flags)?;
    tracing::info!("repositorios binarios '{}' registrados en {}", name, conf_path);
    Ok(())
}

fn verify_key_tofu(dest_path: &std::path::Path, fp: &str, name: &str) -> Result<()> {
    if dest_path.exists() {
        if let Ok(existing_pem) = std::fs::read_to_string(dest_path) {
            if let Ok(existing_der) = crate::vur_client::decode_pem_body(&existing_pem) {
                let existing_digest = Sha256::digest(&existing_der);
                let existing_fp = existing_digest
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<Vec<_>>()
                    .join(":");
                if existing_fp.to_lowercase() != fp.to_lowercase() {
                    bail!(
                        "ALERTA DE SEGURIDAD CRÍTICA (Posible rotación no confiable o suplantación):\n\
                         La llave pública del repo '{}' ha cambiado respecto a la instalada en el sistema.\n  \
                         Instalada previamente: {}\n  \
                         Recibida remotamente:  {}\n\
                         Operación BLOQUEADA (fallo cerrado).\n\
                         Si la rotación es legítima y verificada, ejecuta: vary --repo rekey {}",
                        name, existing_fp, fp, name
                    );
                }
            }
        }
    }
    Ok(())
}

/// Elimina llave y conf de un VUR binario (--repo remove / rekey).
pub fn teardown_binary_repo(
    name: &str,
    sudo_bin: &str,
    sudo_flags: &[String],
) -> Result<()> {
    validate_repo_name(name)?;
    let key_dest = key_dest_path(name)?;
    let conf_dest = repo_conf_path(name)?;
    for dest in [key_dest, conf_dest] {
        match std::fs::metadata(&dest) {
            Ok(_) => {
                let status = crate::elevate::elevate(sudo_bin, sudo_flags, "rm")?
                    .args(["-f"])
                    .arg(&dest)
                    .status()
                    .context("elevando privilegios para eliminar archivo del sistema")?;
                if !status.success() {
                    bail!("no se pudo eliminar {}", dest);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e).with_context(|| format!("inspeccionando {}", dest)),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rutas_derivadas_son_correctas() {
        assert_eq!(repo_conf_path("mi-repo").unwrap(), "/etc/xbps.d/20-vur-mi-repo.conf");
        assert_eq!(key_dest_path("mi-repo").unwrap(), "/etc/xbps.d/keys/vary-vur-mi-repo.pem");
    }

    #[test]
    fn rechaza_path_traversal_en_nombres_de_repo() {
        assert!(validate_repo_name("../evil").is_err());
        assert!(validate_repo_name("foo/bar").is_err());
        assert!(validate_repo_name("..").is_err());
        assert!(validate_repo_name("00-repository-main").is_err());
        assert!(validate_repo_name("-bad").is_err());
        assert!(validate_repo_name("").is_err());
        assert!(validate_repo_name("good_repo-1.0").is_ok());
    }

    #[test]
    fn confirm_respeta_no_confirm() {
        assert!(confirm("¿?", true).unwrap());
    }

    #[test]
    fn verify_key_tofu_bloquea_rotacion_no_confiable() {
        let dir = tempfile::tempdir().unwrap();
        let key_file = dir.path().join("test.pem");

        let pem_a = "-----BEGIN PUBLIC KEY-----\naGVsbG8=\n-----END PUBLIC KEY-----\n";
        std::fs::write(&key_file, pem_a).unwrap();

        let der_a = crate::vur_client::decode_pem_body(pem_a).unwrap();
        let fp_a = Sha256::digest(&der_a)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<Vec<_>>()
            .join(":");

        let pem_b = "-----BEGIN PUBLIC KEY-----\nd29ybGQ=\n-----END PUBLIC KEY-----\n";
        let der_b = crate::vur_client::decode_pem_body(pem_b).unwrap();
        let fp_b = Sha256::digest(&der_b)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<Vec<_>>()
            .join(":");

        let non_existent = dir.path().join("nonexistent.pem");
        assert!(verify_key_tofu(&non_existent, &fp_a, "repo-test").is_ok());
        assert!(verify_key_tofu(&key_file, &fp_a, "repo-test").is_ok());

        let err = verify_key_tofu(&key_file, &fp_b, "repo-test").unwrap_err();
        let err_msg = err.to_string();
        assert!(err_msg.contains("ALERTA DE SEGURIDAD CRÍTICA"));
        assert!(err_msg.contains("BLOQUEADA (fallo cerrado)"));
        assert!(err_msg.contains("vary --repo rekey repo-test"));
    }
}
