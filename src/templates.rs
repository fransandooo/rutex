//! Plantillas para proyectos nuevos. Están escritas en inglés con marcas
//! `@@texto@@` que se traducen al crear el proyecto (incluidas rutas como
//! `@@chapters@@/`), así que un proyecto en español sale con `babel spanish`,
//! carpetas `capitulos/` y títulos en español.

use std::fs;
use std::path::Path;

use crate::i18n::{self, tr};

pub struct Template {
    pub name: &'static str,
    pub description: &'static str,
    pub icon: &'static str,
    /// (ruta relativa, contenido). Una ruta que acaba en `/` es una carpeta vacía.
    pub files: &'static [(&'static str, &'static str)],
}

pub const TEMPLATES: &[Template] = &[
    Template {
        name: "Article",
        description: "Simple document with sections, formulas and figures",
        icon: "x-office-document-symbolic",
        files: &[("main.tex", ARTICLE), ("@@figures@@/", "")],
    },
    Template {
        name: "Thesis (TFM / TFG / PhD)",
        description: "Chapters in separate files, biblatex bibliography and figures",
        icon: "accessories-dictionary-symbolic",
        files: &[
            ("main.tex", THESIS),
            ("@@chapters@@/@@introduction@@.tex", "\\chapter{@@Introduction@@}\n\\label{cap:@@introduction@@}\n\n@@Context and motivation of the work@@~\\cite{knuth1984}.\n\n\\section{@@Objectives@@}\n\n\\section{@@Document structure@@}\n"),
            ("@@chapters@@/@@state-of-the-art@@.tex", "\\chapter{@@State of the art@@}\n\\label{cap:@@state-of-the-art@@}\n\n"),
            ("@@chapters@@/@@methodology@@.tex", "\\chapter{@@Methodology@@}\n\\label{cap:@@methodology@@}\n\n"),
            ("@@chapters@@/@@results@@.tex", "\\chapter{@@Results@@}\n\\label{cap:@@results@@}\n\n"),
            ("@@chapters@@/@@conclusions@@.tex", "\\chapter{@@Conclusions@@}\n\\label{cap:@@conclusions@@}\n\n\\section{@@Future work@@}\n"),
            ("@@references@@.bib", BIB),
            ("@@figures@@/", ""),
        ],
    },
    Template {
        name: "Lecture notes",
        description: "Class notes with theorems, definitions and examples",
        icon: "accessories-text-editor-symbolic",
        files: &[("main.tex", NOTES)],
    },
    Template {
        name: "Presentation",
        description: "Beamer slides",
        icon: "x-office-presentation-symbolic",
        files: &[("main.tex", BEAMER), ("@@figures@@/", "")],
    },
    Template { name: "Empty", description: "Just the essentials", icon: "document-new-symbolic", files: &[("main.tex", EMPTY)] },
];

/// Sustituye las marcas `@@…@@`: `TITLE` por el título, `BABEL` por la opción
/// de idioma de babel y el resto por su traducción.
fn localize(text: &'static str, title: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("@@") {
        let Some(len) = rest[start + 2..].find("@@") else { break };
        out.push_str(&rest[..start]);
        let token = &rest[start + 2..start + 2 + len];
        match token {
            "TITLE" => out.push_str(title),
            "BABEL" => out.push_str(if i18n::is_spanish() { "spanish,es-tabla" } else { "english" }),
            t => out.push_str(tr(t)),
        }
        rest = &rest[start + 2 + len + 2..];
    }
    out.push_str(rest);
    out
}

/// Escribe la plantilla en `root` (que debe existir) sin pisar archivos existentes.
pub fn write(template: &Template, root: &Path, title: &str) -> std::io::Result<()> {
    for (rel, content) in template.files {
        let rel = localize(rel, title);
        let path = root.join(&rel);
        if rel.ends_with('/') {
            fs::create_dir_all(&path)?;
            continue;
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        if !path.exists() {
            fs::write(&path, localize(content, title))?;
        }
    }
    Ok(())
}

const ARTICLE: &str = r"\documentclass[11pt,a4paper]{article}
\usepackage[T1]{fontenc}
\usepackage[@@BABEL@@]{babel}
\usepackage[margin=2.5cm]{geometry}
\usepackage{amsmath,amssymb}
\usepackage{graphicx}
\graphicspath{{@@figures@@/}}
\usepackage{booktabs}
\usepackage[hidelinks]{hyperref}

\title{@@TITLE@@}
\author{}
\date{\today}

\begin{document}
\maketitle

\begin{abstract}
  @@Summary of the work.@@
\end{abstract}

\section{@@Introduction@@}
\label{sec:@@introduction@@}

@@Start writing here. Formulas look like this:@@ $e^{i\pi} + 1 = 0$.

\end{document}
";

const THESIS: &str = r"\documentclass[12pt,a4paper,oneside]{report}
\usepackage[T1]{fontenc}
\usepackage[@@BABEL@@]{babel}
\usepackage[margin=2.5cm]{geometry}
\usepackage{amsmath,amssymb}
\usepackage{graphicx}
\graphicspath{{@@figures@@/}}
\usepackage{booktabs}
\usepackage{csquotes}
\usepackage[backend=biber,style=numeric,sorting=none]{biblatex}
\addbibresource{@@references@@.bib}
\usepackage[hidelinks]{hyperref}

\title{@@TITLE@@}
\author{@@Name Surname@@}
\date{\today}

\begin{document}

\maketitle

\begin{abstract}
  @@Summary of the work.@@
\end{abstract}

\tableofcontents

\input{@@chapters@@/@@introduction@@}
\input{@@chapters@@/@@state-of-the-art@@}
\input{@@chapters@@/@@methodology@@}
\input{@@chapters@@/@@results@@}
\input{@@chapters@@/@@conclusions@@}

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
\usepackage[@@BABEL@@]{babel}
\usepackage[margin=2.2cm]{geometry}
\usepackage{amsmath,amssymb,amsthm}
\usepackage{enumitem}
\usepackage[hidelinks]{hyperref}

\theoremstyle{plain}
\newtheorem{theorem}{@@Theorem@@}[section]
\newtheorem{lemma}[theorem]{@@Lemma@@}
\theoremstyle{definition}
\newtheorem{definition}[theorem]{@@Definition@@}
\newtheorem{example}[theorem]{@@Example@@}
\theoremstyle{remark}
\newtheorem*{remark}{@@Remark@@}

\newcommand{\R}{\mathbb{R}}
\newcommand{\N}{\mathbb{N}}

\title{@@TITLE@@}
\author{}
\date{}

\begin{document}
\maketitle
\tableofcontents

\section{@@Topic 1@@}

\begin{definition}
  @@A set $A \subseteq \R$ is bounded if there is $M > 0$ with $|a| \le M$ for every $a \in A$.@@
\end{definition}

\begin{theorem}
  @@Every non-empty subset of $\R$ that is bounded above has a supremum.@@
\end{theorem}

\begin{proof}
  @@This is the completeness axiom.@@
\end{proof}

\end{document}
";

const BEAMER: &str = r"\documentclass[aspectratio=169]{beamer}
\usepackage[T1]{fontenc}
\usepackage[@@BABEL@@]{babel}
\usetheme{metropolis}

\title{@@TITLE@@}
\subtitle{}
\author{}
\date{\today}

\begin{document}

\maketitle

\begin{frame}{@@Contents@@}
  \tableofcontents
\end{frame}

\section{@@Introduction@@}

\begin{frame}{@@First slide@@}
  \begin{itemize}
    \item @@An idea@@
    \item @@Another idea@@
  \end{itemize}
\end{frame}

\end{document}
";

const EMPTY: &str = r"\documentclass{article}
\usepackage[T1]{fontenc}

\begin{document}

@@Hello.@@

\end{document}
";

#[cfg(test)]
mod tests {
    use super::*;

    /// Compila cada plantilla, en inglés y en español, con latexmk.
    /// Necesita TeX Live (y texlive-langspanish): `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn templates_compile() {
        for spanish in [false, true] {
            i18n::set_spanish(spanish);
            for (i, t) in TEMPLATES.iter().enumerate() {
                let dir = std::env::temp_dir().join(format!("rutex-tpl-{}-{i}-{spanish}", std::process::id()));
                fs::create_dir_all(&dir).unwrap();
                write(t, &dir, "Prueba").unwrap();
                let out = std::process::Command::new("latexmk")
                    .args(["-pdf", "-interaction=nonstopmode", "-halt-on-error", "main.tex"])
                    .current_dir(&dir)
                    .output()
                    .unwrap();
                assert!(out.status.success(), "«{}» (español={spanish}) no compila:\n{}", t.name, String::from_utf8_lossy(&out.stdout));
                fs::remove_dir_all(&dir).unwrap();
            }
        }
        i18n::set_spanish(false);
    }

    #[test]
    fn localizes_tokens() {
        let s = localize("\\graphicspath{{@@figures@@/}} @@TITLE@@ [@@BABEL@@]", "Hola");
        assert_eq!(s, "\\graphicspath{{figures/}} Hola [english]");
    }
}
