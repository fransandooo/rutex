//! Búsqueda directa e inversa con la herramienta `synctex` de TeX Live.
//! Son procesos de milisegundos; se ejecutan fuera del hilo de la interfaz.

use std::path::{Path, PathBuf};

use gtk::gio;

use crate::compiler;

/// Posición en el PDF: página (desde 0) y coordenadas en puntos desde arriba a la izquierda.
#[derive(Clone, Copy, Debug)]
pub struct PdfPos {
    pub page: usize,
    pub x: f64,
    pub y: f64,
}

/// PDF → fuente. Devuelve la ruta tal y como la registró TeX y la línea.
pub async fn edit(root: PathBuf, build_dir: PathBuf, pdf: PathBuf, pos: PdfPos) -> Option<(String, u32)> {
    let out = run(root, vec![
        "edit".into(),
        "-o".into(),
        format!("{}:{:.2}:{:.2}:{}", pos.page + 1, pos.x, pos.y, pdf.display()),
        "-d".into(),
        build_dir.display().to_string(),
    ])
    .await?;
    let input = field(&out, "Input")?;
    let line = field(&out, "Line")?.parse().ok()?;
    Some((input, line))
}

/// Fuente → PDF. `file` es absoluto; si synctex no lo reconoce probamos con la ruta relativa.
pub async fn view(root: PathBuf, build_dir: PathBuf, pdf: PathBuf, file: PathBuf, line: u32) -> Option<PdfPos> {
    let rel = file.strip_prefix(&root).ok().map(|r| Path::new(".").join(r));
    for candidate in std::iter::once(file.clone()).chain(rel) {
        let out = run(root.clone(), vec![
            "view".into(),
            "-i".into(),
            format!("{line}:0:{}", candidate.display()),
            "-o".into(),
            pdf.display().to_string(),
            "-d".into(),
            build_dir.display().to_string(),
        ])
        .await;
        let Some(out) = out else { continue };
        let page: usize = match field(&out, "Page").and_then(|p| p.parse().ok()) {
            Some(p) if p > 0 => p,
            _ => continue,
        };
        let x = field(&out, "h").and_then(|v| v.parse().ok()).unwrap_or(0.0);
        let y = field(&out, "v").and_then(|v| v.parse().ok()).unwrap_or(0.0);
        return Some(PdfPos { page: page - 1, x, y });
    }
    None
}

async fn run(cwd: PathBuf, args: Vec<String>) -> Option<String> {
    gio::spawn_blocking(move || {
        let out = compiler::command("synctex").current_dir(cwd).args(args).output().ok()?;
        out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
    })
    .await
    .ok()
    .flatten()
}

/// Primer valor de `Clave:valor` en la salida de synctex.
fn field(out: &str, key: &str) -> Option<String> {
    out.lines().find_map(|l| l.strip_prefix(key)?.strip_prefix(':').map(|v| v.trim().to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_fields() {
        let out = "SyncTeX result begin\nOutput:/b/main.pdf\nPage:2\nx:72.0\ny:100.5\nh:71.0\nv:98.2\nInput:/p/./main.tex\nLine:14\n";
        assert_eq!(field(out, "Page").as_deref(), Some("2"));
        assert_eq!(field(out, "v").as_deref(), Some("98.2"));
        assert_eq!(field(out, "Input").as_deref(), Some("/p/./main.tex"));
        assert_eq!(field(out, "Line").as_deref(), Some("14"));
    }
}
