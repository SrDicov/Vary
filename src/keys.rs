//! Flujo de instalación binaria desde repos VUR firmados (Fase 6).
//!
//! Pasos exactos:
//! 1. Detectar que el VUR tiene `binary_repo_url`.
//! 2. Descubrir la llave pública RSA en el repo git (`keys/*.rsa|*.pem`).
//! 3. Mostrar fingerprint SHA256 al usuario para confirmación interactiva,
//!    comparándolo con `key_fingerprint` de repos.conf si está declarado.
//! 4. Copiar la llave a /etc/xbps.d/keys/vur-<nombre>.pem.
//!    4b. VUP (T-012): pre-importar el plist a /var/db/xbps/keys/<fp>.plist
//!    tras verificación+TOFU: un solo consentimiento (el de vary).
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

/// Directorio donde xbps busca llaves de repos firmados (él mismo escribe
/// aquí el plist al importar interactivamente).
pub fn xbps_keys_dir() -> &'static str {
    "/var/db/xbps/keys"
}

/// T-012: fingerprint estilo xbps de una llave pública PEM.
///
/// Replica `xbps_pubkey2fp` (`lib/pubkey2fp.c` de xbps, verificado contra
/// 12 llaves reales incluyendo salida en vivo de xbps): MD5 sobre la
/// codificación OpenSSH (`uint32(7)+"ssh-rsa"+mpint(e)+mpint(n)`), en hex
/// minúsculas con `:`.
///
/// ESTE es el fingerprint canónico: xbps lo muestra en sus prompts, lo usa
/// para nombrar `/var/db/xbps/keys/<fp>.plist` y así lo nombran las
/// herramientas VUP. El SHA256 del DER que vary calculaba antes (32 pares)
/// NO coincide con la visión de xbps (16 pares) y solo se conserva para
/// aceptar pins legacy en repos.conf (ver [`fingerprint_matches_pin`]).
/// Solo RSA (como xbps); cualquier otra estructura falla cerrado.
pub fn xbps_fingerprint_pem(pem: &str) -> Result<String> {
    let der = crate::vur_client::decode_pem_body(pem).context("PEM inválido")?;
    let (n, e) = parse_rsa_spki_pubkey(&der)?;

    fn mpint(v: &[u8]) -> Vec<u8> {
        // Entero mínimo sin ceros a la izquierda + pad si el bit alto arma
        // signo (codificación mpint de RFC 4251 §5, igual que xbps).
        let v = &v
            .iter()
            .position(|&b| b != 0)
            .map(|p| &v[p..])
            .unwrap_or(&v[v.len() - 1..]);
        let mut out = (v.len() as u32 + u32::from(v[0] & 0x80 != 0))
            .to_be_bytes()
            .to_vec();
        if v[0] & 0x80 != 0 {
            out.push(0x00);
        }
        out.extend_from_slice(v);
        out
    }

    let mut blob = [0u8, 0, 0, 7, b's', b's', b'h', b'-', b'r', b's', b'a'].to_vec();
    blob.extend_from_slice(&mpint(&e));
    blob.extend_from_slice(&mpint(&n));
    // `md5::Digest` es el mismo trait ya importado vía `sha2`: MD5 aquí NO
    // es criptografía, es el identificador que xbps define (ver arriba).
    let digest = md5::Md5::digest(&blob);
    Ok(digest
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(":"))
}

/// T-012: parse DER mínimo para extraer (n, e) de una SPKI RSA:
/// `SEQ { SEQ { ... }, BIT STRING { 0x00, SEQ { INTEGER n, INTEGER e } } }`.
/// Falla cerrado ante cualquier desviación (no-RSA, truncado, basura final).
fn parse_rsa_spki_pubkey(der: &[u8]) -> Result<(Vec<u8>, Vec<u8>)> {
    // Lee (tag, longitud) con forma corta/larga; devuelve (inicio, longitud).
    fn read_tl(b: &[u8], mut i: usize, expect: u8) -> Result<(usize, usize)> {
        let got = *b.get(i).context("DER truncado (tag)")?;
        if got != expect {
            bail!("DER inesperado: tag {got:#x}, esperaba {expect:#x} (solo RSA)");
        }
        i += 1;
        let l0 = *b.get(i).context("DER truncado (longitud)")? as usize;
        i += 1;
        let len = if l0 & 0x80 == 0 {
            l0
        } else {
            let n = l0 & 0x7f;
            if n == 0 || n > 4 {
                bail!("longitud DER no soportada");
            }
            let mut v = 0usize;
            for k in 0..n {
                v = (v << 8) | *b.get(i + k).context("DER truncado")? as usize;
            }
            i += n;
            v
        };
        Ok((i, len))
    }
    fn read_int(b: &[u8], i: usize) -> Result<(usize, Vec<u8>)> {
        let (j, len) = read_tl(b, i, 0x02)?;
        let end = j.checked_add(len).context("longitud DER excede")?;
        let int = b.get(j..end).context("DER truncado (INTEGER)")?.to_vec();
        if int.is_empty() {
            bail!("INTEGER DER vacío");
        }
        Ok((end, int))
    }

    let (i, _) = read_tl(der, 0, 0x30)?; // SEQ exterior (SPKI)
    let (j, alg_len) = read_tl(der, i, 0x30)?; // SEQ algoritmo
    let i = j.checked_add(alg_len).context("longitud DER excede")?;
    let (j, bs_len) = read_tl(der, i, 0x03)?; // BIT STRING
    let bs_end = j.checked_add(bs_len).context("longitud DER excede")?;
    if bs_end != der.len() {
        bail!("basura final tras la SPKI");
    }
    let body = der.get(j..bs_end).context("DER truncado (BIT STRING)")?;
    if body.first() != Some(&0x00) {
        bail!("BIT STRING con bits sin usar (solo RSA)");
    }
    // body[0] es el byte de bits sin usar; la SEQ PKCS#1 empieza en body[1].
    let (k, _) = read_tl(body, 1, 0x30)?;
    let (k, n) = read_int(body, k)?;
    let (k, e) = read_int(body, k)?;
    if k != body.len() {
        bail!("basura final tras la llave RSA");
    }
    Ok((n, e))
}

/// T-012: ¿el pin de repos.conf coincide con la llave recibida? Acepta el
/// fingerprint estilo xbps (canónico) o el SHA256 legacy que vary mostraba
/// en 0.3.0 (no romper pins ya declarados con ese formato).
pub fn fingerprint_matches_pin(expected: &str, xbps_fp: &str, sha_fp: &str) -> bool {
    let e = expected.trim().to_lowercase();
    e == xbps_fp.to_lowercase() || e == sha_fp.to_lowercase()
}

/// Valida estrictamente el nombre de un repositorio para prevenir path traversal
/// o sobreescritura de archivos de configuración de sistema oficiales.
pub fn validate_repo_name(name: &str) -> Result<()> {
    let Some(first) = name.chars().next() else {
        bail!("el nombre del repositorio no puede estar vacío");
    };
    if !first.is_ascii_alphanumeric() {
        bail!("el nombre del repositorio '{name}' debe comenzar con un carácter alfanumérico");
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
    {
        bail!("el nombre del repositorio '{name}' contiene caracteres inválidos (solo a-z, 0-9, ., _, -)");
    }
    if name.contains("..") || name.contains('/') || name.contains('\\') {
        bail!("intento de path traversal detectado en el nombre del repositorio '{name}'");
    }
    if name.starts_with("00-") || name == "keys" {
        bail!("nombre de repositorio reservado o no permitido: '{name}'");
    }
    Ok(())
}

pub fn repo_conf_path(name: &str) -> Result<String> {
    validate_repo_name(name)?;
    Ok(format!("/etc/xbps.d/20-vur-{name}.conf"))
}

pub fn key_dest_path(name: &str) -> Result<String> {
    validate_repo_name(name)?;
    Ok(format!("{}/vary-vur-{}.pem", keys_dir(), name))
}

/// T-012: ruta del plist pre-importado para un fingerprint. El fp viene de
/// un hash (hex con `:`), pero se valida igual: es la última defensa contra
/// path traversal hacia el keyring del sistema.
pub fn xbps_key_plist_path(fp: &str) -> Result<String> {
    if fp.is_empty()
        || !fp.chars().all(|c| c.is_ascii_hexdigit() || c == ':')
        || fp.contains("..")
        || fp.contains('/')
        || fp.contains('\\')
    {
        bail!("fingerprint inválido para pre-importar llave xbps: '{fp}'");
    }
    Ok(format!("{}/{fp}.plist", xbps_keys_dir()))
}

/// T-012: pre-importa el plist del repo al keyring de xbps.
///
/// Se llama DESPUÉS de fingerprint verificado + confirmación + TOFU (nunca
/// antes): con el plist presente, el primer `xbps-install` ya no pide
/// importar la llave (un solo consentimiento: el de vary). Falla cerrado:
/// el plist debe decodificar a la MISMA llave del `fp` verificado (liga el
/// artefacto escrito con la verificación TOFU) o no se escribe nada.
pub fn preimport_xbps_key_plist(
    fp: &str,
    key_plist: &str,
    keys_dir: &str,
    sudo_bin: &str,
    sudo_flags: &[String],
    install_bin: &str,
) -> Result<()> {
    // Valida el fp (defensa path traversal) y compone el destino bajo el
    // dir dado (producción: xbps_keys_dir(); tests: tempdir).
    xbps_key_plist_path(fp)?;
    let dest = format!("{keys_dir}/{fp}.plist");
    let pem = crate::vup_index::decode_plist_public_key_pem(key_plist)
        .context("el plist a pre-importar no decodifica a una public key PEM")?;
    // Liga el artefacto con la verificación: el plist debe contener la misma
    // llave (en fingerprint xbps, el que nombra el destino).
    let plist_fp = xbps_fingerprint_pem(&pem).context("PEM del plist inválido")?;
    if plist_fp.to_lowercase() != fp.to_lowercase() {
        bail!(
            "el plist a pre-importar no corresponde a la llave verificada (fallo cerrado):\n  verificado: {fp}\n  del plist:  {plist_fp}"
        );
    }
    write_root_file(key_plist, &dest, "644", sudo_bin, sudo_flags, install_bin)?;
    tracing::info!("llave pre-importada a {dest}");
    Ok(())
}

/// SHA256 del DER (formato que vary mostraba en 0.3.0). Solo se conserva
/// para aceptar pins legacy en repos.conf; lo canónico es
/// [`xbps_fingerprint_pem`].
fn sha256_fingerprint_pem(pem: &str) -> Result<String> {
    let der = crate::vur_client::decode_pem_body(pem)?;
    let digest = Sha256::digest(&der);
    Ok(digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join(":"))
}

pub(crate) fn write_root_file(
    contents: &str,
    dest: &str,
    mode: &str,
    sudo_bin: &str,
    sudo_flags: &[String],
    install_bin: &str,
) -> Result<()> {
    use std::io::Write;
    let mut tmp_file = tempfile::Builder::new()
        .prefix("vary-")
        .tempfile()
        .context("creando archivo temporal seguro")?;
    tmp_file
        .write_all(contents.as_bytes())
        .context("escribiendo contenido en archivo temporal")?;
    tmp_file.flush().context("sincronizando archivo temporal")?;

    let status = crate::elevate::elevate(sudo_bin, sudo_flags, install_bin)?
        // -D: crear directorios padres (p. ej. /etc/xbps.d/keys/, ausente en
        // instalaciones limpias). Sin esto, el primer repo binario falla (T-007).
        .args(["-D", "-m", mode])
        .arg(tmp_file.path())
        .arg(dest)
        .status()
        .context("elevando privilegios para escribir archivo del sistema")?;

    if !status.success() {
        bail!("no se pudo escribir {} (código {:?})", dest, status.code());
    }
    Ok(())
}

pub fn validate_repository_url(url: &str) -> Result<()> {
    if url.contains('\n') || url.contains('\r') || url.chars().any(|c| c.is_control()) {
        bail!("URL de repositorio inválida: contiene saltos de línea o caracteres de control");
    }
    let u = url.trim();
    if u.is_empty() {
        bail!("URL de repositorio vacía");
    }
    if !(u.starts_with("https://") || u.starts_with("http://") || u.starts_with("file://")) {
        bail!("URL de repositorio inválida ('{u}'): esquema no soportado (debe ser https://, http:// o file://)");
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
    install_bin: &str,
    no_confirm: bool,
) -> Result<()> {
    // (1) ¿tiene binary_repo_url?
    let Some(binary_url) = entry
        .binary_repo_url
        .as_deref()
        .filter(|u| !u.trim().is_empty())
    else {
        bail!(
            "el VUR '{}' es source-only (sin binary_repo_url); solo puede compilarse",
            repo.name
        );
    };
    validate_repository_url(binary_url)?;

    // (2) Llave pública dentro del clon git del VUR
    let Some(key_path) = repo.discover_public_key() else {
        bail!(
            "el VUR '{}' declara binary_repo_url pero no incluye llave pública \
             en keys/ (convención: keys/<fingerprint>.rsa)",
            repo.name
        );
    };

    // (3) Fingerprint estilo xbps + confirmación interactiva (el fp que se
    // muestra es el mismo que xbps mostrará: cruzable visualmente).
    let pem_raw = std::fs::read_to_string(&key_path)
        .with_context(|| format!("leyendo {}", key_path.display()))?;
    let fp = xbps_fingerprint_pem(&pem_raw)?;
    let sha_fp = sha256_fingerprint_pem(&pem_raw)?;
    if let Some(expected) = entry
        .key_fingerprint
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        if !fingerprint_matches_pin(expected, &fp, &sha_fp) {
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
    println!("  Fingerprint (xbps): {fp}");
    println!("  binary repo: {binary_url}");
    if !confirm(
        "¿Confiás en esta llave y deseas registrar este repositorio binario?",
        no_confirm,
    )? {
        bail!("registro de repositorio binario cancelado por el usuario");
    }

    // (4) Verificar contra llave preexistente en disco (protección TOFU contra rotación no verificada)
    let dest_key = key_dest_path(&repo.name)?;
    verify_key_tofu(
        std::path::Path::new(&dest_key),
        &fp,
        &repo.name,
        entry.trusted_at,
    )?;

    // Copiar llave a /etc/xbps.d/keys/
    write_root_file(
        &pem_raw,
        &dest_key,
        "644",
        sudo_bin,
        sudo_flags,
        install_bin,
    )?;

    // (5) Registrar el repositorio binario
    let conf = format!("repository={binary_url}\n");
    let conf_path = repo_conf_path(&repo.name)?;
    write_root_file(&conf, &conf_path, "644", sudo_bin, sudo_flags, install_bin)?;
    tracing::info!(
        "repositorio binario '{}' registrado en {}",
        repo.name,
        conf_path
    );
    Ok(())
}

/// Registra un repo binario estilo VUP (adaptador Fase 1).
///
/// A diferencia de `setup_binary_repo` (una sola `binary_repo_url` + llave
/// PEM en el clon), aquí hay una URL de repo por tupla categoría-arquitectura
/// y la llave viene ya decodificada del plist del repo (`keys/*.plist`).
/// Se escriben TODAS las `repository=` necesarias en el mismo conf, que es
/// lo que xbps espera (un conf admite varias líneas `repository=`).
/// Deuda API: 8 args (el lint permite 7). El paso correcto es agrupar
/// elevación en un struct `ElevCtx`, pero eso reescribe ~20 call sites;
/// queda registrado para hacerlo junto a P1-3, no en este fix.
#[allow(clippy::too_many_arguments)]
pub fn setup_vup_binary_repo(
    name: &str,
    repo_urls: &[String],
    key_pem: &str,
    // T-012: texto crudo del plist (`keys/*.plist` del repo git). Se
    // pre-importa a /var/db/xbps/keys/ tras verificación+TOFU para que el
    // primer xbps-install no pida un segundo consentimiento.
    key_plist: &str,
    entry: &RepoEntry,
    sudo_bin: &str,
    sudo_flags: &[String],
    install_bin: &str,
    no_confirm: bool,
) -> Result<()> {
    let mut urls: Vec<&str> = Vec::new();
    for u in repo_urls {
        let u = u.trim();
        if !u.is_empty() && !urls.contains(&u) {
            validate_repository_url(u)?;
            urls.push(u);
        }
    }
    if urls.is_empty() {
        bail!("el repo VUP '{name}' no aporta ninguna URL binaria para esta arquitectura");
    }

    // (1) Fingerprint estilo xbps de la llave recibida (el mismo que xbps
    // mostrará) + SHA256 legacy solo para no romper pins de 0.3.0.
    let fp = xbps_fingerprint_pem(key_pem)?;
    let sha_fp = sha256_fingerprint_pem(key_pem)?;

    if let Some(expected) = entry
        .key_fingerprint
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        if !fingerprint_matches_pin(expected, &fp, &sha_fp) {
            bail!("fingerprint de llave del repo VUP '{name}' NO coincide:\n  esperado (repos.conf): {expected}\n  recibido:              {fp}\n\
                 Si el mantenedor rotó la llave legítimamente, ejecuta: vary --repo rekey {name}");
        }
    } else {
        tracing::warn!(
            "el repo VUP '{name}' no declara key_fingerprint en repos.conf; verifica visualmente"
        );
    }

    println!("Repo VUP '{name}': llave pública del índice");
    println!("  Fingerprint (xbps): {fp}");
    println!("  binary repos ({} urls):", urls.len());
    for u in &urls {
        println!("    {u}");
    }
    if !confirm(
        "¿Confiás en esta llave y deseas registrar estos repositorios binarios?",
        no_confirm,
    )? {
        bail!("registro de repositorios binarios cancelado por el usuario");
    }

    // (2) Verificar contra llave preexistente en disco (protección TOFU contra rotación no verificada)
    let dest_key = key_dest_path(name)?;
    verify_key_tofu(std::path::Path::new(&dest_key), &fp, name, entry.trusted_at)?;

    // Copiar llave a /etc/xbps.d/keys/
    write_root_file(key_pem, &dest_key, "644", sudo_bin, sudo_flags, install_bin)?;

    // T-012: pre-importar el plist al keyring de xbps (falla cerrado si el
    // plist no liga con el fp verificado). Sin esto, el primer xbps-install
    // pide importar la llave (segundo prompt) o aborta sin TTY.
    preimport_xbps_key_plist(
        &fp,
        key_plist,
        xbps_keys_dir(),
        sudo_bin,
        sudo_flags,
        install_bin,
    )?;

    // (3) Registrar el conf con todas las URLs
    let mut conf = String::new();
    for url in urls {
        conf.push_str(&format!("repository={url}\n"));
    }
    let conf_path = repo_conf_path(name)?;
    write_root_file(&conf, &conf_path, "644", sudo_bin, sudo_flags, install_bin)?;
    tracing::info!("repositorios binarios '{name}' registrados en {conf_path}");
    Ok(())
}

fn verify_key_tofu(
    dest_path: &std::path::Path,
    fp: &str,
    name: &str,
    trusted_at: Option<i64>,
) -> Result<()> {
    if dest_path.exists() {
        // T-011: llave instalada ilegible o corrupta = estado no verificable:
        // fallar cerrado en vez de re-confiar en silencio.
        let existing_pem = std::fs::read_to_string(dest_path).with_context(|| {
            format!(
                "no se pudo leer la llave instalada de '{name}' en {}",
                dest_path.display()
            )
        })?;
        // T-012: el fp comparado es estilo xbps (el que xbps muestra y el
        // que nombra el plist pre-importado).
        let existing_fp = xbps_fingerprint_pem(&existing_pem).with_context(|| {
            format!(
                "la llave instalada de '{name}' en {} no es una public key RSA válida",
                dest_path.display()
            )
        })?;
        if existing_fp.to_lowercase() != fp.to_lowercase() {
            bail!("ALERTA DE SEGURIDAD CRÍTICA (Posible rotación no confiable o suplantación):\n\
                 La llave pública del repo '{name}' ha cambiado respecto a la instalada en el sistema.\n  \
                 Instalada previamente (confiada {}): {existing_fp}\n  \
                 Recibida remotamente:  {fp}\n\
                 Operación BLOQUEADA (fallo cerrado).\n\
                 Si la rotación es legítima y verificada: vary --repo re-trust {name}\n\
                 (o rekey para resetear la confianza y re-registrar en el próximo install)",
                fmt_trusted_at(trusted_at));
        }
    }
    Ok(())
}

/// P0-2: TOFU continuo — estado de confianza de un repo binario registrado:
/// lo instalado (confiado) frente a lo actual (clon recién actualizado).
pub struct TrustState {
    pub name: String,
    /// Fingerprint estilo xbps de la llave INSTALADA (/etc, la confiada).
    pub installed_fp: String,
    /// Fingerprint estilo xbps de la llave ACTUAL (clon).
    pub current_fp: String,
    /// URL origen del clon (la confiada); `None` si no se pudo leer (en ese
    /// caso solo se verifica la llave, nunca se aborta por URL).
    pub trusted_url: Option<String>,
    /// URL configurada ahora en repos.conf.
    pub configured_url: String,
    /// Cuándo se confió (repos.conf); `None` = anterior al registro temporal.
    pub trusted_at: Option<i64>,
}

/// P0-2: compara continuidad y aborta forense ante cualquier cambio (fp o
/// URL). `Ok(())` = continuidad intacta. El error describe QUÉ cambió
/// (viejo→nuevo), CUÁNDO se confió y el remedio (`re-trust`, o remove+add
/// si cambió la URL: re-trust no mueve el origen del clon).
pub fn check_trust(state: &TrustState) -> Result<()> {
    if state.installed_fp.to_lowercase() != state.current_fp.to_lowercase() {
        anyhow::bail!(
            "ALERTA DE SEGURIDAD (P0-2, cambio de llave en '{}'):\n  \
             Instalada (confiada {}): {}\n  \
             Recibida ahora:           {}\n\
             El repo cambió su llave fuera de un `re-trust`/`rekey`: posible rotación no confiable o suplantación.\n  \
             NO instales de este repo hasta resolverlo.\n  \
             Si la rotación es legítima y verificada: vary --repo re-trust {}",
            state.name,
            fmt_trusted_at(state.trusted_at),
            state.installed_fp,
            state.current_fp,
            state.name
        );
    }
    match (&state.trusted_url, &state.configured_url) {
        (Some(trusted), configured)
            if normalize_repo_url(trusted) != normalize_repo_url(configured) =>
        {
            anyhow::bail!(
                "ALERTA DE SEGURIDAD (P0-2, cambio de URL en '{}'):\n  \
                 URL confiada (origen del clon, {}): {}\n  \
                 URL configurada ahora:                 {}\n\
                 El origen del clon ya no coincide con repos.conf: posible redirección.\n  \
                 `re-trust` no mueve el origen: re-clona con `vary --repo remove {} -p` + `vary --repo add <url>`.",
                state.name,
                fmt_trusted_at(state.trusted_at),
                trusted,
                configured,
                state.name
            );
        }
        _ => {}
    }
    Ok(())
}

/// P0-2: normaliza URLs de repo para comparar (barra final y `.git`
/// opcionales no son un cambio).
pub fn normalize_repo_url(url: &str) -> String {
    let u = url.trim().trim_end_matches('/');
    u.strip_suffix(".git").unwrap_or(u).to_string()
}

/// P0-2: epoch unix actual (para `trusted_at`).
pub(crate) fn now_epoch() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// P0-2: "cuándo se confió" en legible (relativo + epoch crudo).
pub fn fmt_trusted_at_at(trusted_at: Option<i64>, now: i64) -> String {
    match trusted_at {
        None => "en fecha desconocida (anterior al registro temporal)".to_string(),
        Some(t) => {
            let age = now.saturating_sub(t).max(0);
            let rel = if age < 90 {
                format!("hace ~{age} s")
            } else if age < 5400 {
                format!("hace ~{} min", age / 60)
            } else if age < 172_800 {
                format!("hace ~{} h", age / 3600)
            } else {
                format!("hace ~{} días", age / 86400)
            };
            format!("{rel} (epoch {t})")
        }
    }
}

/// P0-2: [`fmt_trusted_at_at`] con el reloj real.
pub fn fmt_trusted_at(trusted_at: Option<i64>) -> String {
    fmt_trusted_at_at(trusted_at, now_epoch())
}

/// P0-2: fija `trusted_at` en repos.conf. `when`: `Some(t)` = fijar,
/// `None` = borrar (rekey = des-confiar). `only_if_absent`: primer registro
/// (no pisa la fecha original). Repo ausente = no-op (dado de baja).
pub fn stamp_trust(
    repos_conf_path: &std::path::Path,
    name: &str,
    when: Option<i64>,
    only_if_absent: bool,
) -> Result<()> {
    let mut conf = crate::reposconf::ReposConf::load(repos_conf_path)?;
    let Some(entry) = conf.vur.get_mut(name) else {
        return Ok(());
    };
    if only_if_absent && entry.trusted_at.is_some() {
        return Ok(());
    }
    entry.trusted_at = when;
    conf.save(repos_conf_path)?;
    Ok(())
}

/// P0-2: recolecta el [`TrustState`] de un repo (tras `pull` en refresh).
/// `None` = no aplica (no binario, nunca registrado, o llave ilegible en el
/// clon —en ese caso avisa y el gate duro queda en el install vía setup—).
pub fn collect_trust_state(
    repo: &VurRepo,
    entry: &crate::reposconf::RepoEntry,
) -> Option<TrustState> {
    let is_vup = entry.has_vup_index();
    if !is_vup && !entry.has_binary() {
        return None;
    }
    let dest = key_dest_path(&repo.name).ok()?;
    let installed_pem = std::fs::read_to_string(&dest).ok()?;
    let installed_fp = xbps_fingerprint_pem(&installed_pem).ok()?;
    let current_pem: Option<String> = if is_vup {
        crate::vup_index::read_repo_plist_text_git(&repo.git_bin, &repo.path)
            .or_else(|_| crate::vup_index::read_repo_plist_text(&repo.path))
            .and_then(|t| crate::vup_index::decode_plist_public_key_pem(&t))
            .ok()
    } else {
        repo.discover_public_key()
            .and_then(|p| std::fs::read_to_string(p).ok())
    };
    let Some(current_pem) = current_pem else {
        tracing::warn!(
            "P0-2: sin llave legible en el clon de '{n}'; se omite el chequeo de continuidad en refresh (el install lo exige vía setup)",
            n = repo.name
        );
        return None;
    };
    let current_fp = xbps_fingerprint_pem(&current_pem).ok()?;
    Some(TrustState {
        name: repo.name.clone(),
        installed_fp,
        current_fp,
        trusted_url: clone_origin_url(&repo.git_bin, &repo.path),
        configured_url: entry.url.clone(),
        trusted_at: entry.trusted_at,
    })
}

/// URL del remoto `origin` del clon (`None` si no se puede leer).
pub(crate) fn clone_origin_url(git_bin: &str, path: &std::path::Path) -> Option<String> {
    let out = std::process::Command::new(git_bin)
        .arg("-C")
        .arg(path)
        .args(["config", "--get", "remote.origin.url"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let url = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if url.is_empty() {
        None
    } else {
        Some(url)
    }
}

/// P0-2: ¿hay llave instalada para el repo? (confianza previa, con o sin fecha).
pub fn repo_key_installed(name: &str) -> bool {
    key_dest_path(name).is_ok_and(|p| std::path::Path::new(&p).exists())
}

/// P0-2: ¿repo binario registrado? (conf + llave presentes en /etc).
pub fn binary_repo_registered(name: &str) -> bool {
    let conf_ok = repo_conf_path(name).is_ok_and(|p| std::path::Path::new(&p).exists());
    conf_ok && repo_key_installed(name)
}

/// Elimina llave y conf de un VUR binario (--repo remove / rekey).
///
/// T-012: también retira el plist pre-importado de /var/db/xbps/keys/ (si la
/// llave instalada decodifica): sin esto, un `rekey` tras rotación dejaría
/// la llave vieja confiada por xbps. Best-effort (avisa, no falla): la
/// higiene del keyring nunca debe bloquear la baja del repo.
pub fn teardown_binary_repo(name: &str, sudo_bin: &str, sudo_flags: &[String]) -> Result<()> {
    validate_repo_name(name)?;
    let key_dest = key_dest_path(name)?;
    let conf_dest = repo_conf_path(name)?;
    // fp estilo xbps de la llave instalada (si es legible) para localizar
    // su plist pre-importado.
    let stale_plist: Option<String> = std::fs::read_to_string(&key_dest)
        .ok()
        .and_then(|pem| xbps_fingerprint_pem(&pem).ok())
        .and_then(|fp| xbps_key_plist_path(&fp).ok());
    for dest in [key_dest, conf_dest] {
        match std::fs::metadata(&dest) {
            Ok(_) => {
                let status = crate::elevate::elevate(sudo_bin, sudo_flags, "rm")?
                    .args(["-f"])
                    .arg(&dest)
                    .status()
                    .context("elevando privilegios para eliminar archivo del sistema")?;
                if !status.success() {
                    bail!("no se pudo eliminar {dest}");
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e).with_context(|| format!("inspeccionando {dest}")),
        }
    }
    if let Some(plist) = stale_plist {
        match crate::elevate::elevate(sudo_bin, sudo_flags, "rm")?
            .args(["-f"])
            .arg(&plist)
            .status()
        {
            Ok(s) if s.success() => tracing::info!("plist pre-importado retirado: {plist}"),
            Ok(s) => tracing::warn!("no se pudo retirar {plist} (código {:?})", s.code()),
            Err(e) => tracing::warn!("no se pudo retirar {plist}: {e:#}"),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rutas_derivadas_son_correctas() {
        assert_eq!(
            repo_conf_path("mi-repo").unwrap(),
            "/etc/xbps.d/20-vur-mi-repo.conf"
        );
        assert_eq!(
            key_dest_path("mi-repo").unwrap(),
            "/etc/xbps.d/keys/vary-vur-mi-repo.pem"
        );
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

    // --- Fixtures RSA reales (512-bit, generadas para tests; públicas) ---

    const K1_PUB: &str = "-----BEGIN PUBLIC KEY-----\nMFwwDQYJKoZIhvcNAQEBBQADSwAwSAJBAL6BQJEq0JGyD0SR8MLQzdM+PksXk8mL\nF0AGeN3vOTGHbvFAIbq6lxMkYaAtF6YbRIDnRWxIn5RC+eHqvT9gBNUCAwEAAQ==\n-----END PUBLIC KEY-----\n";
    const K1_FP_XBPS: &str = "90:de:62:d8:f1:f3:f8:cf:c6:8f:e2:b3:d0:8d:5e:59";
    const K2_PUB: &str = "-----BEGIN PUBLIC KEY-----\nMFwwDQYJKoZIhvcNAQEBBQADSwAwSAJBAMxCPikCO5uxQDBBrjgJ+vf5bxikBTs2\n1C17C1ips/JLNGFtipNz4ejujSc37aQjHCnspKzxRKaESj3KTPN0ki8CAwEAAQ==\n-----END PUBLIC KEY-----\n";
    const K2_FP_XBPS: &str = "ad:9a:6c:a8:31:6b:68:0d:37:49:21:3e:35:51:28:41";
    // RSA-2048 cuya fingerprint verificó xbps en vivo (47:9b:d3:be:...).
    const K2048_PUB: &str = "-----BEGIN PUBLIC KEY-----\nMIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEA4r8PsCrh/WgmNCLOZ7pz\ncosWCec7w1JmcgiYtJIjFwzXvQycMr3kYUkKAx9xuTxdW3ooteiJlS3AhFV6Gw/4\nrn1pQUyZv/U4rZuWSTlAP73rgLBGTXuDaE7l2xb/1No5LtVgxJYhlYWhG9SOJb29\nRzyroFyUrp/NZMLij2HZZrIk7tB9gR7+aLXKaEGMrNXEFUkHWtb045dfnhtrtpBh\nlYn54f6ML1Xr2wn3zlLz8FK/HGp5CYdlLjjbS17lqDQv1w/gj5lbgtB+6X1ia5d+\nfj945r+wrsyQuzPuWMJl5EV30FVlLVLuIC28mzNlNdk2d+jl4NcvZIJkgdns7Gjq\n1QIDAQAB\n-----END PUBLIC KEY-----\n";
    const K2048_FP_XBPS: &str = "47:9b:d3:be:c2:6e:a7:2e:83:f4:92:05:75:9c:8f:f7";

    #[test]
    fn xbps_fingerprint_coincide_con_xbps_en_vivo() {
        // T-012: el algoritmo replica xbps_pubkey2fp (MD5 sobre OpenSSH).
        assert_eq!(xbps_fingerprint_pem(K1_PUB).unwrap(), K1_FP_XBPS);
        assert_eq!(xbps_fingerprint_pem(K2_PUB).unwrap(), K2_FP_XBPS);
        assert_eq!(xbps_fingerprint_pem(K2048_PUB).unwrap(), K2048_FP_XBPS);
    }

    #[test]
    fn xbps_fingerprint_falla_cerrado_con_no_rsa() {
        use base64::Engine as _;
        // "hello" en base64 no es una SPKI RSA: falla, no inventa fp.
        let fake = "-----BEGIN PUBLIC KEY-----\naGVsbG8=\n-----END PUBLIC KEY-----\n";
        assert!(xbps_fingerprint_pem(fake).is_err());
        assert!(xbps_fingerprint_pem("basura-no-pem").is_err());
        assert!(xbps_fingerprint_pem("").is_err());
        // DER truncado a la mitad.
        let der = crate::vur_client::decode_pem_body(K1_PUB).unwrap();
        let cut = format!(
            "-----BEGIN PUBLIC KEY-----\n{}\n-----END PUBLIC KEY-----\n",
            base64::engine::general_purpose::STANDARD.encode(&der[..der.len() / 2])
        );
        assert!(xbps_fingerprint_pem(&cut).is_err());
    }

    #[test]
    fn fingerprint_matches_pin_acepta_xbps_y_legacy() {
        let sha = sha256_fingerprint_pem(K1_PUB).unwrap();
        assert_ne!(sha, K1_FP_XBPS); // formatos distintos a propósito
        assert!(fingerprint_matches_pin(K1_FP_XBPS, K1_FP_XBPS, &sha));
        assert!(fingerprint_matches_pin(&sha, K1_FP_XBPS, &sha));
        assert!(fingerprint_matches_pin(
            &K1_FP_XBPS.to_uppercase(),
            K1_FP_XBPS,
            &sha
        ));
        assert!(!fingerprint_matches_pin(K2_FP_XBPS, K1_FP_XBPS, &sha));
        assert!(!fingerprint_matches_pin("", K1_FP_XBPS, &sha));
    }

    #[test]
    fn verify_key_tofu_bloquea_rotacion_no_confiable() {
        let dir = tempfile::tempdir().unwrap();
        let key_file = dir.path().join("test.pem");

        std::fs::write(&key_file, K1_PUB).unwrap();
        let fp_a = xbps_fingerprint_pem(K1_PUB).unwrap();
        let fp_b = xbps_fingerprint_pem(K2_PUB).unwrap();

        let non_existent = dir.path().join("nonexistent.pem");
        assert!(verify_key_tofu(&non_existent, &fp_a, "repo-test", None).is_ok());
        assert!(verify_key_tofu(&key_file, &fp_a, "repo-test", None).is_ok());

        // P0-2: la alerta forense cita cuándo se confió.
        let err = verify_key_tofu(&key_file, &fp_b, "repo-test", Some(0)).unwrap_err();
        let err_msg = err.to_string();
        assert!(err_msg.contains("ALERTA DE SEGURIDAD CRÍTICA"));
        assert!(err_msg.contains("BLOQUEADA (fallo cerrado)"));
        assert!(err_msg.contains("vary --repo re-trust repo-test"));
        assert!(err_msg.contains("epoch 0"), "{err_msg}");
    }

    #[test]
    fn verify_key_tofu_falla_cerrado_con_llave_ilegible() {
        // T-011: llave instalada corrupta no debe re-confiar en silencio.
        let dir = tempfile::tempdir().unwrap();
        let key_file = dir.path().join("k.pem");
        std::fs::write(&key_file, "basura-no-pem").unwrap();
        let err = verify_key_tofu(&key_file, "aa:bb", "repo-test", None).unwrap_err();
        assert!(err.to_string().contains("no es una public key RSA válida"));
    }

    #[test]
    fn validate_repository_url_rejects_newlines_and_bad_schemes() {
        assert!(validate_repository_url("https://repo.voidlinux.org/current").is_ok());
        assert!(validate_repository_url("http://local.mirror/void").is_ok());
        assert!(validate_repository_url("file:///var/cache/binpkgs").is_ok());

        assert!(
            validate_repository_url("https://repo.voidlinux.org\nrepository=https://evil.org")
                .is_err()
        );
        assert!(validate_repository_url("https://repo.voidlinux.org\r\n").is_err());
        assert!(validate_repository_url("ftp://repo.voidlinux.org").is_err());
        assert!(validate_repository_url("ext::sh").is_err());
        assert!(validate_repository_url("").is_err());
        assert!(validate_repository_url("   ").is_err());
    }

    #[test]
    fn write_root_file_creates_file_safely() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("dest.conf");
        let dest_str = dest.to_str().unwrap();
        let content = "repository=https://example.com/repo\n";

        write_root_file(content, dest_str, "644", "env", &[], "install").unwrap();

        assert_eq!(std::fs::read_to_string(&dest).unwrap(), content);
    }

    #[test]
    fn write_root_file_crea_padres_inexistentes() {
        // T-007: /etc/xbps.d/keys/ no existe en instalaciones limpias.
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("keys").join("sub").join("k.pem");
        let dest_str = dest.to_str().unwrap();

        write_root_file("PEM", dest_str, "644", "env", &[], "install").unwrap();

        assert_eq!(std::fs::read_to_string(&dest).unwrap(), "PEM");
    }

    // --- T-012: pre-import del plist al keyring de xbps ---

    fn sample_plist(pem: &str) -> String {
        use base64::Engine as _;
        let b64 = base64::engine::general_purpose::STANDARD.encode(pem.as_bytes());
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<plist version=\"1.0\">\n<dict>\n\t<key>public-key</key>\n\t<data>{b64}</data>\n</dict>\n</plist>\n"
        )
    }

    #[test]
    fn xbps_key_plist_path_deriva_y_rechaza_traversal() {
        let fp = "78:b8:1b:87:74:ff:ce:49:f8:f9:06:9a:04:7f:7e:1e";
        assert_eq!(
            xbps_key_plist_path(fp).unwrap(),
            format!("/var/db/xbps/keys/{fp}.plist")
        );
        assert!(xbps_key_plist_path("").is_err());
        assert!(xbps_key_plist_path("../evil").is_err());
        assert!(xbps_key_plist_path("ab/cd").is_err());
        assert!(xbps_key_plist_path("ab\\cd").is_err());
        assert!(xbps_key_plist_path("zz:11").is_err());
        assert!(xbps_key_plist_path("portador: ñ").is_err());
    }

    #[test]
    fn preimport_escribe_plist_exacto_en_dir_dado() {
        // Instalación en claves simuladas: el contenido debe quedar verbatim.
        let dir = tempfile::tempdir().unwrap();
        let keys_dir = dir.path().join("keys");
        let keys_dir_str = keys_dir.to_str().unwrap();
        let fp = K1_FP_XBPS;
        let plist = sample_plist(K1_PUB);

        preimport_xbps_key_plist(fp, &plist, keys_dir_str, "env", &[], "install").unwrap();

        let written = keys_dir.join(format!("{fp}.plist"));
        assert_eq!(std::fs::read_to_string(&written).unwrap(), plist);
    }

    #[test]
    fn preimport_falla_cerrado_si_plist_no_liga_con_fp() {
        // El plist trae otra llave que la verificada: no se escribe nada.
        let dir = tempfile::tempdir().unwrap();
        let keys_dir_str = dir.path().to_str().unwrap();
        let fp_b = K2_FP_XBPS;
        let plist_a = sample_plist(K1_PUB);

        let err = preimport_xbps_key_plist(fp_b, &plist_a, keys_dir_str, "env", &[], "install")
            .unwrap_err();
        assert!(err.to_string().contains("no corresponde"), "{err:#}");
        assert!(std::fs::read_dir(dir.path()).unwrap().next().is_none());
    }

    #[test]
    fn preimport_rechaza_plist_malformado() {
        let dir = tempfile::tempdir().unwrap();
        let keys_dir_str = dir.path().to_str().unwrap();
        let err = preimport_xbps_key_plist(
            K1_FP_XBPS,
            "basura-no-plist",
            keys_dir_str,
            "env",
            &[],
            "install",
        )
        .unwrap_err();
        assert!(err.to_string().contains("no decodifica"), "{err:#}");
    }

    // --- P0-2: TOFU continuo (tabla de estados × veredicto) ---

    fn trust_state() -> TrustState {
        TrustState {
            name: "vur-test".to_string(),
            installed_fp: K1_FP_XBPS.to_string(),
            current_fp: K1_FP_XBPS.to_string(),
            trusted_url: Some("https://example.com/vur.git".to_string()),
            configured_url: "https://example.com/vur.git".to_string(),
            trusted_at: Some(1_000_000),
        }
    }

    #[test]
    fn check_trust_ok_con_continuidad_intacta() {
        assert!(check_trust(&trust_state()).is_ok());
        // Mayúsculas/minúsculas no son un cambio.
        let mut st = trust_state();
        st.current_fp = K1_FP_XBPS.to_uppercase();
        assert!(check_trust(&st).is_ok());
        // Barra final / .git no son un cambio de URL.
        let mut st = trust_state();
        st.configured_url = "https://example.com/vur.git/".to_string();
        assert!(check_trust(&st).is_ok());
        let mut st = trust_state();
        st.trusted_url = Some("https://example.com/vur".to_string());
        st.configured_url = "https://example.com/vur.git".to_string();
        assert!(check_trust(&st).is_ok());
    }

    #[test]
    fn check_trust_aborta_forense_en_cambio_de_llave() {
        let mut st = trust_state();
        st.current_fp = K2_FP_XBPS.to_string();
        let err = check_trust(&st).unwrap_err().to_string();
        assert!(err.contains("P0-2"), "{err}");
        assert!(err.contains(K1_FP_XBPS), "{err}");
        assert!(err.contains(K2_FP_XBPS), "{err}");
        assert!(err.contains("epoch 1000000"), "{err}");
        assert!(err.contains("re-trust"), "{err}");
        assert!(err.contains("NO instales"), "{err}");
    }

    #[test]
    fn check_trust_aborta_forense_en_cambio_de_url() {
        let mut st = trust_state();
        st.configured_url = "https://evil.example.com/vur.git".to_string();
        let err = check_trust(&st).unwrap_err().to_string();
        assert!(err.contains("cambio de URL"), "{err}");
        assert!(err.contains("https://evil.example.com/vur.git"), "{err}");
        assert!(err.contains("remove"), "{err}");
    }

    #[test]
    fn check_trust_sin_origen_no_aborta_por_url() {
        // Origen ilegible: solo se verifica la llave (nunca abortar a ciegas).
        let mut st = trust_state();
        st.trusted_url = None;
        st.configured_url = "https://otra.example.com/x.git".to_string();
        assert!(check_trust(&st).is_ok());
    }

    #[test]
    fn fmt_trusted_at_relativo_y_desconocido() {
        assert!(fmt_trusted_at_at(None, 9_999).contains("desconocida"));
        assert_eq!(
            fmt_trusted_at_at(Some(1_000_000), 1_000_030),
            "hace ~30 s (epoch 1000000)"
        );
        assert_eq!(
            fmt_trusted_at_at(Some(1_000_000), 1_003_000),
            "hace ~50 min (epoch 1000000)"
        );
        assert_eq!(
            fmt_trusted_at_at(Some(1_000_000), 1_010_000),
            "hace ~2 h (epoch 1000000)"
        );
        assert_eq!(
            fmt_trusted_at_at(Some(1_000_000), 1_500_000),
            "hace ~5 días (epoch 1000000)"
        );
    }

    #[test]
    fn normalize_repo_url_ignora_barra_y_dotgit() {
        assert_eq!(
            normalize_repo_url("https://a/b.git/"),
            "https://a/b".to_string()
        );
        assert_eq!(
            normalize_repo_url("  https://a/b  "),
            "https://a/b".to_string()
        );
    }

    #[test]
    fn stamp_trust_fija_solo_primer_registro_y_borra() {
        use crate::reposconf::{RepoEntry, ReposConf};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("repos.conf");
        let mut conf = ReposConf::default();
        conf.vur.insert(
            "r".to_string(),
            RepoEntry {
                url: "https://example.com/r.git".to_string(),
                ..RepoEntry::default()
            },
        );
        conf.save(&path).unwrap();

        stamp_trust(&path, "r", Some(111), true).unwrap();
        let conf = ReposConf::load(&path).unwrap();
        assert_eq!(conf.vur["r"].trusted_at, Some(111));
        // once: no pisa la fecha original.
        stamp_trust(&path, "r", Some(222), true).unwrap();
        let conf = ReposConf::load(&path).unwrap();
        assert_eq!(conf.vur["r"].trusted_at, Some(111));
        // force: renueva (re-trust).
        stamp_trust(&path, "r", Some(333), false).unwrap();
        let conf = ReposConf::load(&path).unwrap();
        assert_eq!(conf.vur["r"].trusted_at, Some(333));
        // borrado (rekey = des-confiar).
        stamp_trust(&path, "r", None, false).unwrap();
        let conf = ReposConf::load(&path).unwrap();
        assert_eq!(conf.vur["r"].trusted_at, None);
        // repo ausente: no-op sin error.
        stamp_trust(&path, "fantasma", Some(1), false).unwrap();
    }
}
