//! Plantillas para proyectos nuevos.

use std::fs;
use std::path::Path;

pub struct Template {
    pub name: &'static str,
    pub description: &'static str,
    pub icon: &'static str,
    /// (ruta relativa, contenido). Una ruta que acaba en `/` es una carpeta vacía.
    pub files: &'static [(&'static str, &'static str)],
}

pub const TEMPLATES: &[Template] = &[
    Template {
        name: "Artículo",
        description: "Documento sencillo con secciones, fórmulas y figuras",
        icon: "x-office-document-symbolic",
        files: &[("main.tex", ARTICLE), ("figuras/", "")],
    },
    Template {
        name: "TFM / TFG / Tesis",
        description: "Capítulos en archivos separados, bibliografía con biblatex y figuras",
        icon: "accessories-dictionary-symbolic",
        files: &[
            ("main.tex", THESIS),
            ("capitulos/introduccion.tex", "\\chapter{Introducción}\n\\label{cap:introduccion}\n\nContexto y motivación del trabajo~\\cite{knuth1984}.\n\n\\section{Objetivos}\n\n\\section{Estructura del documento}\n"),
            ("capitulos/estado-del-arte.tex", "\\chapter{Estado del arte}\n\\label{cap:estado-del-arte}\n\n"),
            ("capitulos/metodologia.tex", "\\chapter{Metodología}\n\\label{cap:metodologia}\n\n"),
            ("capitulos/resultados.tex", "\\chapter{Resultados}\n\\label{cap:resultados}\n\n"),
            ("capitulos/conclusiones.tex", "\\chapter{Conclusiones}\n\\label{cap:conclusiones}\n\n\\section{Trabajo futuro}\n"),
            ("referencias.bib", BIB),
            ("figuras/", ""),
        ],
    },
    Template {
        name: "Apuntes",
        description: "Notas de clase con teoremas, definiciones y ejemplos",
        icon: "accessories-text-editor-symbolic",
        files: &[("main.tex", NOTES)],
    },
    Template {
        name: "Presentación",
        description: "Diapositivas con Beamer",
        icon: "x-office-presentation-symbolic",
        files: &[("main.tex", BEAMER), ("figuras/", "")],
    },
    Template {
        name: "Vacío",
        description: "Solo lo imprescindible",
        icon: "document-new-symbolic",
        files: &[("main.tex", EMPTY)],
    },
];

/// Escribe la plantilla en `root` (que debe existir) sin pisar archivos existentes.
pub fn write(template: &Template, root: &Path, title: &str) -> std::io::Result<()> {
    for (rel, content) in template.files {
        let path = root.join(rel);
        if rel.ends_with('/') {
            fs::create_dir_all(&path)?;
            continue;
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        if !path.exists() {
            fs::write(&path, content.replace("{{TITULO}}", title))?;
        }
    }
    Ok(())
}

const ARTICLE: &str = r"\documentclass[11pt,a4paper]{article}
\usepackage[T1]{fontenc}
\usepackage[spanish,es-tabla]{babel}
\usepackage[margin=2.5cm]{geometry}
\usepackage{amsmath,amssymb}
\usepackage{graphicx}
\usepackage{booktabs}
\usepackage[hidelinks]{hyperref}

\title{{{TITULO}}}
\author{}
\date{\today}

\begin{document}
\maketitle

\begin{abstract}
  Resumen del trabajo.
\end{abstract}

\section{Introducción}
\label{sec:introduccion}

Empieza a escribir aquí. Las fórmulas van así: $e^{i\pi} + 1 = 0$.

\end{document}
";

const THESIS: &str = r"\documentclass[12pt,a4paper,oneside]{report}
\usepackage[T1]{fontenc}
\usepackage[spanish,es-tabla]{babel}
\usepackage[margin=2.5cm]{geometry}
\usepackage{amsmath,amssymb}
\usepackage{graphicx}
\graphicspath{{figuras/}}
\usepackage{booktabs}
\usepackage{csquotes}
\usepackage[backend=biber,style=numeric,sorting=none]{biblatex}
\addbibresource{referencias.bib}
\usepackage[hidelinks]{hyperref}

\title{{{TITULO}}}
\author{Nombre Apellido}
\date{\today}

\begin{document}

\maketitle

\begin{abstract}
  Resumen del trabajo.
\end{abstract}

\tableofcontents

\input{capitulos/introduccion}
\input{capitulos/estado-del-arte}
\input{capitulos/metodologia}
\input{capitulos/resultados}
\input{capitulos/conclusiones}

\printbibliography[heading=bibintoc]

\end{document}
";

const BIB: &str = r"@book{knuth1984,
  author    = {Donald E. Knuth},
  title     = {The {\TeX}book},
  publisher = {Addison-Wesley},
  year      = {1984}
}
";

const NOTES: &str = r"\documentclass[11pt,a4paper]{article}
\usepackage[T1]{fontenc}
\usepackage[spanish,es-tabla]{babel}
\usepackage[margin=2.2cm]{geometry}
\usepackage{amsmath,amssymb,amsthm}
\usepackage{enumitem}
\usepackage[hidelinks]{hyperref}

\theoremstyle{plain}
\newtheorem{theorem}{Teorema}[section]
\newtheorem{lemma}[theorem]{Lema}
\theoremstyle{definition}
\newtheorem{definition}[theorem]{Definición}
\newtheorem{example}[theorem]{Ejemplo}
\theoremstyle{remark}
\newtheorem*{remark}{Observación}

\newcommand{\R}{\mathbb{R}}
\newcommand{\N}{\mathbb{N}}

\title{{{TITULO}}}
\author{}
\date{}

\begin{document}
\maketitle
\tableofcontents

\section{Tema 1}

\begin{definition}
  Un conjunto $A \subseteq \R$ está acotado si existe $M > 0$ con $|a| \le M$ para todo $a \in A$.
\end{definition}

\begin{theorem}
  Todo conjunto no vacío y acotado superiormente de $\R$ tiene supremo.
\end{theorem}

\begin{proof}
  Es el axioma del supremo.
\end{proof}

\end{document}
";

const BEAMER: &str = r"\documentclass[aspectratio=169]{beamer}
\usepackage[T1]{fontenc}
\usepackage[spanish]{babel}
\usetheme{metropolis}

\title{{{TITULO}}}
\subtitle{}
\author{}
\date{\today}

\begin{document}

\maketitle

\begin{frame}{Índice}
  \tableofcontents
\end{frame}

\section{Introducción}

\begin{frame}{Primera diapositiva}
  \begin{itemize}
    \item Una idea
    \item Otra idea
  \end{itemize}
\end{frame}

\end{document}
";

const EMPTY: &str = r"\documentclass{article}
\usepackage[T1]{fontenc}

\begin{document}

Hola.

\end{document}
";

#[cfg(test)]
mod tests {
    use super::*;

    /// Compila cada plantilla con latexmk. Necesita TeX Live: `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn templates_compile() {
        for (i, t) in TEMPLATES.iter().enumerate() {
            let dir = std::env::temp_dir().join(format!("rutex-tpl-{}-{i}", std::process::id()));
            fs::create_dir_all(&dir).unwrap();
            write(t, &dir, "Prueba").unwrap();
            let out = std::process::Command::new("latexmk")
                .args(["-pdf", "-interaction=nonstopmode", "-halt-on-error", "main.tex"])
                .current_dir(&dir)
                .output()
                .unwrap();
            assert!(out.status.success(), "la plantilla «{}» no compila:\n{}", t.name, String::from_utf8_lossy(&out.stdout));
            assert!(dir.join("main.pdf").exists());
            fs::remove_dir_all(&dir).unwrap();
        }
    }
}
