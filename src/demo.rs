//! Proyecto de demostración incluido en el binario (carpeta `demo/` del repo).
//! Se copia a ~/.local/share/rutex/demo la primera vez que se abre.

use std::fs;
use std::path::PathBuf;

const FILES: &[(&str, &str)] = &[
    ("main.tex", include_str!("../demo/main.tex")),
    ("preamble.tex", include_str!("../demo/preamble.tex")),
    ("references.bib", include_str!("../demo/references.bib")),
    ("figures/pipeline.tex", include_str!("../demo/figures/pipeline.tex")),
    ("chapters/introduction.tex", include_str!("../demo/chapters/introduction.tex")),
    ("chapters/mathematics.tex", include_str!("../demo/chapters/mathematics.tex")),
    ("chapters/figures.tex", include_str!("../demo/chapters/figures.tex")),
    ("chapters/tables.tex", include_str!("../demo/chapters/tables.tex")),
    ("chapters/algorithms.tex", include_str!("../demo/chapters/algorithms.tex")),
    ("chapters/planning.tex", include_str!("../demo/chapters/planning.tex")),
    ("chapters/appendix.tex", include_str!("../demo/chapters/appendix.tex")),
];

/// Copia la demo (sin pisar lo que el usuario ya haya cambiado) y devuelve su carpeta.
pub fn install() -> std::io::Result<PathBuf> {
    let root = gtk::glib::user_data_dir().join("rutex").join("demo");
    for (rel, content) in FILES {
        let path = root.join(rel);
        if !path.exists() {
            fs::create_dir_all(path.parent().unwrap())?;
            fs::write(&path, content)?;
        }
    }
    Ok(root)
}
