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


pub fn keys_dir() -> &'static str {
    "/etc/xbps.d/keys"
}

pub fn repo_conf_path(name: &str) -> String {
    format!("/etc/xbps.d/20-vur-{}.conf", name)
}

pub fn key_dest_path(name: &str) -> String {
    format!("{}/vary-vur-{}.pem", keys_dir(), name)
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

    // (4) Copiar llave a /etc/xbps.d/keys/
    let pem = std::fs::read_to_string(&key_path)
        .with_context(|| format!("leyendo {}", key_path.display()))?;
    let dest_key = key_dest_path(&repo.name);
    write_root_file(&pem, &dest_key, "644", sudo_bin, sudo_flags)?;

    // (5) Registrar el repositorio binario
    let conf = format!("repository={}\n", binary_url);
    write_root_file(&conf, &repo_conf_path(&repo.name), "644", sudo_bin, sudo_flags)?;
    tracing::info!("repositorio binario '{}' registrado en {}", repo.name, repo_conf_path(&repo.name));
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

    // Fingerprint sobre un temporal (reutiliza el cálculo estándar PEM→SHA256).
    let tmp = tempfile::NamedTempFile::new().context("creando temporal para la llave")?;
    std::fs::write(tmp.path(), key_pem).context("escribiendo llave temporal")?;
    let fp = VurRepo::fingerprint_sha256(tmp.path())?;

    if let Some(expected) = entry.key_fingerprint.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        if expected.to_lowercase() != fp.to_lowercase() {
            bail!(
                "fingerprint de llave del repo '{}' NO coincide:\n  esperado (repos.conf): {}\n  recibido:              {}\n\
                 Si el mantenedor rotó la llave legítimamente, ejecuta: vary --repo rekey {}",
                name, expected, fp, name
            );
        }
    } else {
        tracing::warn!(
            "el repo '{}' no declara key_fingerprint en repos.conf; verifica visualmente",
            name
        );
    }
    println!("Repo VUP '{}': llave pública verificada", name);
    println!("  SHA256: {}", fp);
    for u in &urls {
        println!("  binary repo: {}", u);
    }
    if !confirm("¿Confiás en esta llave y deseas registrar estos repositorios binarios?", no_confirm)?
    {
        bail!("registro de repositorio binario cancelado por el usuario");
    }

    let dest_key = key_dest_path(name);
    write_root_file(key_pem, &dest_key, "644", sudo_bin, sudo_flags)?;

    let mut conf = String::new();
    for u in &urls {
        conf.push_str(&format!("repository={}\n", u));
    }
    write_root_file(&conf, &repo_conf_path(name), "644", sudo_bin, sudo_flags)?;
    tracing::info!("repositorios binarios '{}' registrados en {}", name, repo_conf_path(name));
    Ok(())
}

/// Elimina llave y conf de un VUR binario (--repo remove / rekey).
pub fn teardown_binary_repo(
    name: &str,
    sudo_bin: &str,
    sudo_flags: &[String],
) -> Result<()> {
    for dest in [key_dest_path(name), repo_conf_path(name)] {
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
        assert_eq!(repo_conf_path("mi-repo"), "/etc/xbps.d/20-vur-mi-repo.conf");
        assert_eq!(key_dest_path("mi-repo"), "/etc/xbps.d/keys/vary-vur-mi-repo.pem");
    }

    #[test]
    fn confirm_respeta_no_confirm() {
        assert!(confirm("¿?", true).unwrap());
    }
}
