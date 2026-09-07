//! Auditoría estática de templates VUR (P0-3).
//!
//! `audit_template()` califica el contenido COMPLETO (primer install /
//! review: propiedades del template); `audit_template_diff()` califica un
//! UPGRADE (regresiones + líneas añadidas). Los hallazgos son SIEMPRE
//! consultivos: se muestran SOBRE el diff/plantilla en el review gate pero
//! no cambian su lógica (en no-TTY sin `--yes` el gate ya aborta solo, A3).
//!
//! Reglas (heurísticas documentadas, sin parseo shell):
//!
//! * `checksum-ausente` (High): hay descargas (`distfiles=` no vacío o
//!   `do_fetch()` con `curl|wget` fuera de dependencias) pero el template no
//!   declara `checksum=` ni `sha256sums=`. En diffs solo dispara si el viejo
//!   SÍ verificaba (regresión); si nunca hubo checksum, cada upgrade
//!   re-avisar sería ruido.
//! * `descarga-en-build` (High): línea con descarga en build (`npm
//!   install|ci`, `pip[3] install`, `wget`, o `curl` con pipe a shell / `-o`
//!   / URL): código no auditado en tiempo de compilación.
//! * `url-nueva` (Medium, solo diffs): línea añadida con `http(s)://`.
//!   En contenido completo todo sería "nuevo": ruido, se omite.
//! * `hook-nuevo` (Medium, solo diffs): función `do_*|pre_*|post_*`
//!   añadida. En contenido completo todo hook existiría: ruido, se omite.

/// Severidad de un hallazgo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    High,
    Medium,
}

impl Severity {
    fn tag(self) -> &'static str {
        match self {
            Severity::High => "HIGH",
            Severity::Medium => "MEDIUM",
        }
    }
}

/// Un hallazgo: regla + detalle accionable (cita la línea).
#[derive(Debug, Clone, PartialEq)]
pub struct AuditFinding {
    pub severity: Severity,
    pub rule: &'static str,
    pub detail: String,
}

fn finding(severity: Severity, rule: &'static str, detail: String) -> AuditFinding {
    AuditFinding {
        severity,
        rule,
        detail,
    }
}

/// ¿el template declara verificación de checksums?
fn has_checksum(text: &str) -> bool {
    text.lines().any(|l| {
        let l = l.trim_start();
        l.starts_with("checksum=") || l.starts_with("checksum+=") || l.starts_with("sha256sums=")
    })
}

/// ¿el template descarga algo? `distfiles=` no vacío, o `do_fetch()` con
/// `curl|wget` fuera de líneas de dependencias (heurística: `curl` como
/// dependencia de build no cuenta como descarga).
fn has_downloads(text: &str) -> bool {
    let mut has_distfiles = false;
    let mut has_do_fetch = false;
    let mut has_fetcher = false;
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with("distfiles=") || l.starts_with("distfiles+=") {
            let rhs: String = l
                .split_once('=')
                .map(|(_, v)| v.trim().trim_matches('"').trim().to_string())
                .unwrap_or_default();
            if !rhs.is_empty() {
                has_distfiles = true;
            }
            continue;
        }
        if l.starts_with("do_fetch") {
            has_do_fetch = true;
            continue;
        }
        if l.contains("depends=") {
            continue;
        }
        if l.contains("curl") || l.contains("wget") {
            has_fetcher = true;
        }
    }
    has_distfiles || (has_do_fetch && has_fetcher)
}

/// Limpia una línea para los matchers (quita `+` de diff y espacios).
fn clean(line: &str) -> &str {
    line.trim_start_matches('+').trim()
}

/// ¿línea de descarga en build? (`curl` exige contexto de descarga: pipe a
/// shell, `-o`/`-O` o URL. `curl` suelto —p. ej. en comentarios— no alcanza.)
fn is_build_download(line: &str) -> bool {
    let l = clean(line);
    l.contains("npm install")
        || l.contains("npm ci")
        || l.contains("pip install")
        || l.contains("pip3 install")
        || l.contains("wget")
        || (l.contains("curl")
            && (l.contains("| sh")
                || l.contains("|sh")
                || l.contains("| bash")
                || l.contains("|bash")
                || l.contains(" -o")
                || l.contains(" -O")
                || l.contains("http")))
}

/// ¿definición de hook `do_*|pre_*|post_*()`?
fn is_hook_def(line: &str) -> bool {
    let l = clean(line);
    let Some(name_end) = l.find("()") else {
        return false;
    };
    let name = &l[..name_end];
    (name.starts_with("do_") || name.starts_with("pre_") || name.starts_with("post_"))
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// ¿línea con URL nueva?
fn has_url(line: &str) -> bool {
    let l = clean(line);
    l.contains("http://") || l.contains("https://")
}

/// Recorta citas largas para el detalle.
fn trunc(s: &str) -> String {
    const MAX: usize = 120;
    let s = clean(s);
    if s.len() <= MAX {
        s.to_string()
    } else {
        format!("{}…", &s[..MAX])
    }
}

/// P0-3: audita el contenido COMPLETO de un template (propiedades, no
/// cambios): checksum + descargas en build.
pub fn audit_template(text: &str) -> Vec<AuditFinding> {
    let mut out = Vec::new();
    if has_downloads(text) && !has_checksum(text) {
        out.push(finding(
            Severity::High,
            "checksum-ausente",
            "descargas sin `checksum=`/`sha256sums=` que las verifiquen".to_string(),
        ));
    }
    for line in text.lines() {
        if is_build_download(line) {
            out.push(finding(
                Severity::High,
                "descarga-en-build",
                format!("descarga en build: {}", trunc(line)),
            ));
        }
    }
    out
}

/// P0-3: audita un UPGRADE (`old`/`new` completos + `patch` unificado):
/// regresión de checksum + reglas sobre líneas añadidas.
pub fn audit_template_diff(old_text: &str, new_text: &str, patch: &str) -> Vec<AuditFinding> {
    let mut out = Vec::new();
    // Regresión: se verificaba y ya no.
    if has_checksum(old_text) && !has_checksum(new_text) {
        out.push(finding(
            Severity::High,
            "checksum-ausente",
            "el nuevo template ELIMINÓ la verificación `checksum=`/`sha256sums=`".to_string(),
        ));
    }
    for line in patch
        .lines()
        .filter(|l| l.starts_with('+') && !l.starts_with("+++"))
    {
        if is_build_download(line) {
            out.push(finding(
                Severity::High,
                "descarga-en-build",
                format!("descarga en build: {}", trunc(line)),
            ));
            continue;
        }
        if is_hook_def(line) {
            out.push(finding(
                Severity::Medium,
                "hook-nuevo",
                format!("nuevo hook: {}", trunc(line)),
            ));
            continue;
        }
        if has_url(line) {
            out.push(finding(
                Severity::Medium,
                "url-nueva",
                format!("nuevo origen: {}", trunc(line)),
            ));
        }
    }
    out
}

/// P0-3: bloque estable para mostrar SOBRE el diff/plantilla. Vacío si no
/// hay hallazgos (el llamador no imprime nada en ese caso).
pub fn format_findings(pkg: &str, repo: &str, findings: &[AuditFinding]) -> String {
    if findings.is_empty() {
        return String::new();
    }
    let (mut hi, mut med) = (0, 0);
    for f in findings {
        match f.severity {
            Severity::High => hi += 1,
            Severity::Medium => med += 1,
        }
    }
    let mut out = format!(":: AUDIT template {pkg} (repo {repo}): {hi} HIGH, {med} MEDIUM\n");
    for f in findings {
        out.push_str(&format!(
            "   [{} {}] {}\n",
            f.severity.tag(),
            f.rule,
            f.detail
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const CON_CHECKSUM: &str = "pkgname=a\nversion=1.0\nrevision=1\ndistfiles=\"https://x/y-1.0.tar.gz\"\nchecksum=abc123\n";
    const SIN_CHECKSUM: &str =
        "pkgname=a\nversion=1.0\nrevision=1\ndistfiles=\"https://x/y-1.0.tar.gz\"\n";
    const FETCH_SIN_CHECKSUM: &str = "pkgname=s\nhostmakedepends=\"bsdtar curl\"\ndo_fetch() {\n\tcurl -sL \"https://x/f\" -o f\n}\n";
    const SOLO_DEP_CURL: &str =
        "pkgname=s\nhostmakedepends=\"bsdtar curl\"\ndo_install() {\n\tvbin bin\n}\n";
    const SIN_DESCARGAS: &str = "pkgname=m\nversion=1\nrevision=1\ndo_install() {\n\tvbin bin\n}\n";

    /// Corpus P0-3 (contenido): veredicto esperado por regla.
    #[test]
    fn checksum_solo_cuando_hay_descargas() {
        // Con checksum: silencio total.
        assert!(audit_template(CON_CHECKSUM).is_empty());
        // distfiles sin checksum => solo checksum-ausente (las URLs del
        // contenido completo NO se reportan: todo sería "nuevo").
        let f = audit_template(SIN_CHECKSUM);
        assert_eq!(f.len(), 1, "{f:?}");
        assert_eq!(f[0].rule, "checksum-ausente");
        assert_eq!(f[0].severity, Severity::High);
        // do_fetch+curl sin checksum => checksum + descarga en build.
        let f = audit_template(FETCH_SIN_CHECKSUM);
        assert!(f.iter().any(|x| x.rule == "checksum-ausente"), "{f:?}");
        assert!(f.iter().any(|x| x.rule == "descarga-en-build"), "{f:?}");
        // curl solo como dependencia: silencio total.
        assert!(audit_template(SOLO_DEP_CURL).is_empty());
        // Sin descargas ni hooks reportables: silencio total.
        assert!(audit_template(SIN_DESCARGAS).is_empty());
    }

    /// Corpus P0-3 (diff): cambios inocuos no reportan; quitadas se ignoran.
    #[test]
    fn diff_solo_mira_anadidas() {
        let none = |patch: &str| audit_template_diff("pkgver=1\n", "pkgver=1\n", patch);
        assert!(none("--- a\n+++ b\n@@\n pkgver=1\n").is_empty());
        assert!(none("@@\n-pkgver=1\n+pkgver=2\n").is_empty());
        assert!(none("@@\n-npm install x\n").is_empty());
        assert!(none("--- a\n+++ b/https://x\n").is_empty());
    }

    #[test]
    fn diff_descarga_en_build_es_high() {
        for line in [
            "+	npm install --silent",
            "+	pip install -r req.txt",
            "+\tcurl -sL https://x/i.sh | sh",
            "+\twget https://x/f.tar.gz",
        ] {
            let patch = format!("@@\n{line}\n");
            let f = audit_template_diff("pkgver=1\n", "pkgver=1\n", &patch);
            assert!(
                f.iter()
                    .any(|x| x.rule == "descarga-en-build" && x.severity == Severity::High),
                "{line}: {f:?}"
            );
        }
        // curl suelto sin contexto de descarga no alcanza.
        let f = audit_template_diff("pkgver=1\n", "pkgver=1\n", "@@\n+# usa curl para la API\n");
        assert!(!f.iter().any(|x| x.rule == "descarga-en-build"), "{f:?}");
    }

    #[test]
    fn diff_hook_y_url_son_medium() {
        let diff =
            |line: &str| audit_template_diff("pkgver=1\n", "pkgver=1\n", &format!("@@\n{line}\n"));
        let f = diff("+do_install() {");
        assert!(
            f.iter()
                .any(|x| x.rule == "hook-nuevo" && x.severity == Severity::Medium),
            "{f:?}"
        );
        let f = diff("+pre_configure() {");
        assert!(f.iter().any(|x| x.rule == "hook-nuevo"), "{f:?}");
        let f = diff("+distfiles=\"https://nuevo.example/f.tar.gz\"");
        assert!(
            f.iter()
                .any(|x| x.rule == "url-nueva" && x.severity == Severity::Medium),
            "{f:?}"
        );
        // Llamada a función existente no es hook nuevo.
        let f = diff("+\tdo_install_extra");
        assert!(!f.iter().any(|x| x.rule == "hook-nuevo"), "{f:?}");
    }

    #[test]
    fn diff_regresion_de_checksum_es_high() {
        let old = "pkgname=a\ndistfiles=\"https://x/f\"\nchecksum=abc\n";
        let new = "pkgname=a\ndistfiles=\"https://x/f\"\n";
        let f = audit_template_diff(old, new, "@@\n-checksum=abc\n");
        assert!(f.iter().any(|x| x.rule == "checksum-ausente"), "{f:?}");
        // Sin checksum en ninguno: no re-avisar cada upgrade.
        let f = audit_template_diff(new, new, "@@\n-pkgver=1\n+pkgver=2\n");
        assert!(!f.iter().any(|x| x.rule == "checksum-ausente"), "{f:?}");
    }

    #[test]
    fn formato_resume_y_vacio_silencioso() {
        assert_eq!(format_findings("a", "r", &[]), "");
        let f = vec![
            finding(Severity::High, "checksum-ausente", "d".to_string()),
            finding(Severity::Medium, "url-nueva", "e".to_string()),
        ];
        let s = format_findings("a", "r", &f);
        assert!(s.contains("1 HIGH, 1 MEDIUM"), "{s}");
        assert!(s.contains("[HIGH checksum-ausente]"), "{s}");
    }
}
