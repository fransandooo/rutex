//! Autocompletado LaTeX para GtkSourceView:
//! - `\sec…` → comandos (con huecos para saltar con Tab);
//! - `\begin{` → entornos completos con su `\end`; `\end{` → el entorno abierto;
//! - `\ref{`, `\eqref{`, `\cref{`… → etiquetas del proyecto;
//! - `\cite{`, `\parencite{`… → claves de los `.bib`;
//! - `\input{`, `\include{`, `\includegraphics{`, `\usepackage{` → archivos y paquetes.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gio, glib};
use sourceview5::prelude::*;
use gtk::glib::subclass::prelude::*;
use sourceview5::subclass::prelude::*;

use crate::index::Index;

/// Datos del proyecto que usa el autocompletado (los actualiza la ventana).
#[derive(Default)]
pub struct Data {
    pub index: Index,
    pub tex_files: Vec<PathBuf>,
    pub images: Vec<PathBuf>,
}

pub type SharedData = Rc<RefCell<Data>>;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Command,
    Environment,
    Label,
    Citation,
    File,
    Package,
}

impl Kind {
    fn icon(self) -> &'static str {
        match self {
            Kind::Command => "lang-function-symbolic",
            Kind::Environment => "lang-struct-symbolic",
            Kind::Label => "lang-variable-symbolic",
            Kind::Citation => "x-office-address-book-symbolic",
            Kind::File => "text-x-generic-symbolic",
            Kind::Package => "package-x-generic-symbolic",
        }
    }
}

/// Qué se está escribiendo justo antes del cursor.
#[derive(Debug, PartialEq, Eq)]
pub enum Context {
    /// `\sec` → prefijo "sec".
    Command(String),
    /// `\cite{knu` → comando "cite", prefijo "knu".
    Argument { command: String, prefix: String },
}

impl Context {
    fn prefix(&self) -> &str {
        match self {
            Context::Command(p) => p,
            Context::Argument { prefix, .. } => prefix,
        }
    }
}

pub fn analyze(before: &str) -> Option<Context> {
    let letters: usize = before.chars().rev().take_while(|c| c.is_ascii_alphabetic()).count();
    let rest = &before[..before.len() - letters];
    if rest.ends_with('\\') && !rest.ends_with("\\\\") {
        return Some(Context::Command(before[rest.len()..].to_string()));
    }

    let open = before.rfind('{')?;
    if before[open..].contains('}') {
        return None;
    }
    let prefix = before[open + 1..].rsplit(',').next().unwrap_or("").trim_start().to_string();
    let mut head = &before[..open];
    if head.ends_with(']') {
        head = &head[..head.rfind('[')?];
    }
    let head = head.trim_end_matches('*');
    let name_len = head.chars().rev().take_while(|c| c.is_ascii_alphabetic()).count();
    let command = &head[head.len() - name_len..];
    if command.is_empty() || !head[..head.len() - name_len].ends_with('\\') {
        return None;
    }
    Some(Context::Argument { command: command.to_string(), prefix })
}

// (etiqueta mostrada, texto/snippet a insertar, descripción)
const COMMANDS: &[(&str, &str, &str)] = &[
    ("begin", "\\begin{${1:entorno}}\n\t${2}\n\\end{$1}$0", "Entorno"),
    ("section", "\\section{${1}}$0", "Sección"),
    ("subsection", "\\subsection{${1}}$0", "Subsección"),
    ("subsubsection", "\\subsubsection{${1}}$0", "Subsubsección"),
    ("chapter", "\\chapter{${1}}$0", "Capítulo"),
    ("part", "\\part{${1}}$0", "Parte"),
    ("paragraph", "\\paragraph{${1}}$0", "Párrafo"),
    ("textbf", "\\textbf{${1}}$0", "Negrita"),
    ("textit", "\\textit{${1}}$0", "Cursiva"),
    ("emph", "\\emph{${1}}$0", "Énfasis"),
    ("underline", "\\underline{${1}}$0", "Subrayado"),
    ("texttt", "\\texttt{${1}}$0", "Monoespaciado"),
    ("textsc", "\\textsc{${1}}$0", "Versalitas"),
    ("footnote", "\\footnote{${1}}$0", "Nota al pie"),
    ("label", "\\label{${1}}$0", "Etiqueta"),
    ("ref", "\\ref{${1}}$0", "Referencia"),
    ("eqref", "\\eqref{${1}}$0", "Referencia a ecuación"),
    ("autoref", "\\autoref{${1}}$0", "Referencia con nombre"),
    ("cref", "\\cref{${1}}$0", "Referencia (cleveref)"),
    ("pageref", "\\pageref{${1}}$0", "Página de una etiqueta"),
    ("cite", "\\cite{${1}}$0", "Cita"),
    ("parencite", "\\parencite{${1}}$0", "Cita entre paréntesis"),
    ("textcite", "\\textcite{${1}}$0", "Cita en el texto"),
    ("citep", "\\citep{${1}}$0", "Cita (natbib)"),
    ("citet", "\\citet{${1}}$0", "Cita textual (natbib)"),
    ("url", "\\url{${1}}$0", "URL"),
    ("href", "\\href{${1:url}}{${2:texto}}$0", "Enlace"),
    ("includegraphics", "\\includegraphics[width=${1:0.8}\\linewidth]{${2}}$0", "Imagen"),
    ("caption", "\\caption{${1}}$0", "Pie de figura/tabla"),
    ("centering", "\\centering", "Centrar"),
    ("item", "\\item ${0}", "Elemento de lista"),
    ("input", "\\input{${1}}$0", "Incluir archivo"),
    ("include", "\\include{${1}}$0", "Incluir capítulo"),
    ("usepackage", "\\usepackage{${1}}$0", "Paquete"),
    ("documentclass", "\\documentclass[${1:12pt,a4paper}]{${2:article}}$0", "Clase de documento"),
    ("title", "\\title{${1}}$0", "Título"),
    ("author", "\\author{${1}}$0", "Autor"),
    ("date", "\\date{${1:\\today}}$0", "Fecha"),
    ("maketitle", "\\maketitle", "Portada"),
    ("tableofcontents", "\\tableofcontents", "Índice"),
    ("listoffigures", "\\listoffigures", "Índice de figuras"),
    ("listoftables", "\\listoftables", "Índice de tablas"),
    ("newpage", "\\newpage", "Nueva página"),
    ("clearpage", "\\clearpage", "Nueva página (vacía flotantes)"),
    ("noindent", "\\noindent", "Sin sangría"),
    ("vspace", "\\vspace{${1:1em}}$0", "Espacio vertical"),
    ("hspace", "\\hspace{${1:1em}}$0", "Espacio horizontal"),
    ("newcommand", "\\newcommand{\\${1:nombre}}{${2}}$0", "Nuevo comando"),
    ("bibliography", "\\bibliography{${1:referencias}}$0", "Bibliografía (BibTeX)"),
    ("bibliographystyle", "\\bibliographystyle{${1:plain}}$0", "Estilo bibliográfico"),
    ("printbibliography", "\\printbibliography", "Bibliografía (biblatex)"),
    ("addbibresource", "\\addbibresource{${1:referencias.bib}}$0", "Archivo .bib (biblatex)"),
    ("today", "\\today", "Fecha de hoy"),
    ("hline", "\\hline", "Línea de tabla"),
    ("toprule", "\\toprule", "booktabs"),
    ("midrule", "\\midrule", "booktabs"),
    ("bottomrule", "\\bottomrule", "booktabs"),
    ("multicolumn", "\\multicolumn{${1:2}}{${2:c}}{${3}}$0", "Celda combinada"),
    ("textwidth", "\\textwidth", "Ancho del texto"),
    ("linewidth", "\\linewidth", "Ancho de línea"),
    ("small", "\\small", "Tamaño"),
    ("large", "\\large", "Tamaño"),
    ("Large", "\\Large", "Tamaño"),
    ("footnotesize", "\\footnotesize", "Tamaño"),
    // Matemáticas
    ("frac", "\\frac{${1}}{${2}}$0", "Fracción"),
    ("sqrt", "\\sqrt{${1}}$0", "Raíz"),
    ("sum", "\\sum_{${1:i=1}}^{${2:n}}$0", "Sumatorio"),
    ("prod", "\\prod_{${1:i=1}}^{${2:n}}$0", "Productorio"),
    ("int", "\\int_{${1:a}}^{${2:b}}$0", "Integral"),
    ("lim", "\\lim_{${1:x \\to \\infty}}$0", "Límite"),
    ("text", "\\text{${1}}$0", "Texto en fórmula"),
    ("mathbb", "\\mathbb{${1:R}}$0", "Conjuntos (ℝ, ℕ…)"),
    ("mathcal", "\\mathcal{${1}}$0", "Caligráfica"),
    ("mathbf", "\\mathbf{${1}}$0", "Negrita matemática"),
    ("mathrm", "\\mathrm{${1}}$0", "Romana"),
    ("operatorname", "\\operatorname{${1}}$0", "Operador"),
    ("left(", "\\left( ${1} \\right)$0", "Paréntesis ajustados"),
    ("left[", "\\left[ ${1} \\right]$0", "Corchetes ajustados"),
    ("hat", "\\hat{${1}}$0", "Acento circunflejo"),
    ("bar", "\\bar{${1}}$0", "Barra"),
    ("vec", "\\vec{${1}}$0", "Vector"),
    ("partial", "\\partial", "∂"),
    ("nabla", "\\nabla", "∇"),
    ("infty", "\\infty", "∞"),
    ("cdot", "\\cdot", "·"),
    ("times", "\\times", "×"),
    ("leq", "\\leq", "≤"),
    ("geq", "\\geq", "≥"),
    ("neq", "\\neq", "≠"),
    ("approx", "\\approx", "≈"),
    ("in", "\\in", "∈"),
    ("subset", "\\subset", "⊂"),
    ("rightarrow", "\\rightarrow", "→"),
    ("Rightarrow", "\\Rightarrow", "⇒"),
    ("leftrightarrow", "\\Leftrightarrow", "⇔"),
    ("forall", "\\forall", "∀"),
    ("exists", "\\exists", "∃"),
    ("ldots", "\\ldots", "…"),
    ("cdots", "\\cdots", "⋯"),
    ("quad", "\\quad", "Espacio"),
    ("alpha", "\\alpha", "α"),
    ("beta", "\\beta", "β"),
    ("gamma", "\\gamma", "γ"),
    ("delta", "\\delta", "δ"),
    ("epsilon", "\\epsilon", "ϵ"),
    ("varepsilon", "\\varepsilon", "ε"),
    ("theta", "\\theta", "θ"),
    ("lambda", "\\lambda", "λ"),
    ("mu", "\\mu", "μ"),
    ("pi", "\\pi", "π"),
    ("rho", "\\rho", "ρ"),
    ("sigma", "\\sigma", "σ"),
    ("tau", "\\tau", "τ"),
    ("phi", "\\phi", "ϕ"),
    ("varphi", "\\varphi", "φ"),
    ("omega", "\\omega", "ω"),
    ("Gamma", "\\Gamma", "Γ"),
    ("Delta", "\\Delta", "Δ"),
    ("Theta", "\\Theta", "Θ"),
    ("Lambda", "\\Lambda", "Λ"),
    ("Sigma", "\\Sigma", "Σ"),
    ("Phi", "\\Phi", "Φ"),
    ("Omega", "\\Omega", "Ω"),
];

// (nombre, resto del snippet tras `\begin{nombre`)
const ENVIRONMENTS: &[(&str, &str)] = &[
    ("itemize", "}\n\t\\item ${1}\n\\end{itemize}$0"),
    ("enumerate", "}\n\t\\item ${1}\n\\end{enumerate}$0"),
    ("description", "}\n\t\\item[${1}] ${2}\n\\end{description}$0"),
    ("figure", "}[${1:htbp}]\n\t\\centering\n\t\\includegraphics[width=${2:0.8}\\linewidth]{${3}}\n\t\\caption{${4}}\n\t\\label{fig:${5}}\n\\end{figure}$0"),
    ("table", "}[${1:htbp}]\n\t\\centering\n\t\\caption{${2}}\n\t\\label{tab:${3}}\n\t\\begin{tabular}{${4:lcr}}\n\t\t${5}\n\t\\end{tabular}\n\\end{table}$0"),
    ("tabular", "}{${1:lcr}}\n\t${2}\n\\end{tabular}$0"),
    ("equation", "}\n\t${1}\n\t\\label{eq:${2}}\n\\end{equation}$0"),
    ("equation*", "}\n\t${1}\n\\end{equation*}$0"),
    ("align", "}\n\t${1}\n\\end{align}$0"),
    ("align*", "}\n\t${1}\n\\end{align*}$0"),
    ("gather", "}\n\t${1}\n\\end{gather}$0"),
    ("cases", "}\n\t${1} & ${2} \\\\\n\\end{cases}$0"),
    ("pmatrix", "}\n\t${1}\n\\end{pmatrix}$0"),
    ("bmatrix", "}\n\t${1}\n\\end{bmatrix}$0"),
    ("center", "}\n\t${1}\n\\end{center}$0"),
    ("minipage", "}{${1:0.45}\\linewidth}\n\t${2}\n\\end{minipage}$0"),
    ("abstract", "}\n\t${1}\n\\end{abstract}$0"),
    ("quote", "}\n\t${1}\n\\end{quote}$0"),
    ("verbatim", "}\n${1}\n\\end{verbatim}$0"),
    ("lstlisting", "}[language=${1:Python}]\n${2}\n\\end{lstlisting}$0"),
    ("theorem", "}\n\t${1}\n\\end{theorem}$0"),
    ("lemma", "}\n\t${1}\n\\end{lemma}$0"),
    ("definition", "}\n\t${1}\n\\end{definition}$0"),
    ("proof", "}\n\t${1}\n\\end{proof}$0"),
    ("frame", "}{${1:Título}}\n\t${2}\n\\end{frame}$0"),
    ("subfigure", "}{${1:0.45}\\linewidth}\n\t\\centering\n\t\\includegraphics[width=\\linewidth]{${2}}\n\t\\caption{${3}}\n\\end{subfigure}$0"),
];

const PACKAGES: &[&str] = &[
    "amsmath", "amssymb", "amsthm", "mathtools", "graphicx", "hyperref", "cleveref", "geometry", "babel", "fontenc",
    "inputenc", "xcolor", "tikz", "pgfplots", "booktabs", "siunitx", "biblatex", "natbib", "listings", "minted",
    "caption", "subcaption", "float", "enumitem", "fancyhdr", "setspace", "microtype", "csquotes", "multirow",
    "longtable", "tabularx", "array", "algorithm", "algpseudocode", "physics", "lipsum", "url", "titlesec",
    "tocloft", "appendix", "acronym", "glossaries", "pdfpages", "wrapfig", "fontspec", "unicode-math", "polyglossia",
];

const REF_COMMANDS: &[&str] = &["ref", "eqref", "autoref", "cref", "Cref", "pageref", "nameref", "vref"];
const CITE_COMMANDS: &[&str] = &[
    "cite", "citep", "citet", "parencite", "textcite", "autocite", "footcite", "nocite", "citeauthor", "citeyear",
    "fullcite", "smartcite",
];

mod imp {
    use std::cell::{Cell, RefCell};

    use super::*;

    #[derive(Default)]
    pub struct LatexProposal {
        pub label: RefCell<String>,
        pub insert: RefCell<String>,
        pub detail: RefCell<String>,
        pub snippet: Cell<bool>,
        pub kind: Cell<Option<Kind>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for LatexProposal {
        const NAME: &'static str = "RutexLatexProposal";
        type Type = super::LatexProposal;
        type Interfaces = (sourceview5::CompletionProposal,);
    }
    impl ObjectImpl for LatexProposal {}
    impl CompletionProposalImpl for LatexProposal {}

    #[derive(Default)]
    pub struct LatexProvider {
        pub data: RefCell<Option<SharedData>>,
        pub filter_text: Rc<RefCell<String>>,
        pub filter: RefCell<Option<gtk::CustomFilter>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for LatexProvider {
        const NAME: &'static str = "RutexLatexProvider";
        type Type = super::LatexProvider;
        type Interfaces = (sourceview5::CompletionProvider,);
    }
    impl ObjectImpl for LatexProvider {}

    impl CompletionProviderImpl for LatexProvider {
        fn title(&self) -> Option<glib::GString> {
            Some("LaTeX".into())
        }

        fn priority(&self, _: &sourceview5::CompletionContext) -> i32 {
            100
        }

        fn is_trigger(&self, _: &gtk::TextIter, c: char) -> bool {
            matches!(c, '\\' | '{' | ',')
        }

        fn populate(&self, context: &sourceview5::CompletionContext) -> Result<gio::ListModel, glib::Error> {
            let store = gio::ListStore::new::<super::LatexProposal>();
            let Some(ctx) = context_at(context) else { return Ok(store.upcast()) };
            let data = self.data.borrow().clone();
            for p in proposals(&ctx, data.as_ref(), context) {
                store.append(&p);
            }
            *self.filter_text.borrow_mut() = ctx.prefix().to_lowercase();
            let text = self.filter_text.clone();
            let filter = gtk::CustomFilter::new(move |obj| {
                let p = obj.downcast_ref::<super::LatexProposal>().unwrap();
                let needle = text.borrow();
                needle.is_empty() || p.imp().label.borrow().to_lowercase().contains(needle.as_str())
            });
            *self.filter.borrow_mut() = Some(filter.clone());
            Ok(gtk::FilterListModel::new(Some(store), Some(filter)).upcast())
        }

        fn refilter(&self, context: &sourceview5::CompletionContext, _: &gio::ListModel) {
            let prefix = context_at(context).map(|c| c.prefix().to_lowercase()).unwrap_or_default();
            *self.filter_text.borrow_mut() = prefix;
            if let Some(f) = self.filter.borrow().as_ref() {
                f.changed(gtk::FilterChange::Different);
            }
        }

        fn display(&self, _: &sourceview5::CompletionContext, proposal: &sourceview5::CompletionProposal, cell: &sourceview5::CompletionCell) {
            let Some(p) = proposal.downcast_ref::<super::LatexProposal>() else { return };
            let imp = p.imp();
            match cell.column() {
                sourceview5::CompletionColumn::Icon => {
                    if let Some(kind) = imp.kind.get() {
                        cell.set_icon_name(kind.icon());
                    }
                }
                sourceview5::CompletionColumn::TypedText => cell.set_text(Some(&imp.label.borrow())),
                sourceview5::CompletionColumn::After => cell.set_text(Some(&imp.detail.borrow())),
                _ => cell.set_text(Some("")),
            }
        }

        fn activate(&self, context: &sourceview5::CompletionContext, proposal: &sourceview5::CompletionProposal) {
            let (Some(p), Some(buffer), Some(view)) = (proposal.downcast_ref::<super::LatexProposal>(), context.buffer(), context.view()) else {
                return;
            };
            let Some(ctx) = context_at(context) else { return };
            let imp = p.imp();
            let mut end = buffer.iter_at_mark(&buffer.get_insert());
            let mut start = end;
            let typed = ctx.prefix().chars().count() as i32 + if matches!(ctx, Context::Command(_)) { 1 } else { 0 };
            start.backward_chars(typed);

            buffer.begin_user_action();
            // Entornos: si ya había una `}` (autocierre), la reemplazamos.
            if imp.kind.get() == Some(Kind::Environment) && end.char() == '}' {
                end.forward_char();
            }
            buffer.delete(&mut start, &mut end);
            let insert = imp.insert.borrow().clone();
            if imp.snippet.get() {
                // En los snippets `\` escapa el carácter siguiente: hay que doblarlas.
                let escaped = insert.replace('\\', "\\\\");
                let snippet = sourceview5::Snippet::new_parsed(&escaped).unwrap_or_else(|_| sourceview5::Snippet::new(None, None));
                view.push_snippet(&snippet, Some(&mut start));
            } else {
                buffer.insert(&mut start, &insert);
            }
            buffer.end_user_action();
        }
    }
}

glib::wrapper! {
    pub struct LatexProposal(ObjectSubclass<imp::LatexProposal>) @implements sourceview5::CompletionProposal;
}

glib::wrapper! {
    pub struct LatexProvider(ObjectSubclass<imp::LatexProvider>) @implements sourceview5::CompletionProvider;
}

impl LatexProposal {
    fn new(kind: Kind, label: &str, insert: &str, detail: &str, snippet: bool) -> LatexProposal {
        let p: LatexProposal = glib::Object::new();
        let imp = p.imp();
        imp.kind.set(Some(kind));
        *imp.label.borrow_mut() = label.to_string();
        *imp.insert.borrow_mut() = insert.to_string();
        *imp.detail.borrow_mut() = detail.to_string();
        imp.snippet.set(snippet);
        p
    }
}

impl LatexProvider {
    pub fn new(data: SharedData) -> LatexProvider {
        let p: LatexProvider = glib::Object::new();
        *p.imp().data.borrow_mut() = Some(data);
        p
    }
}

fn context_at(context: &sourceview5::CompletionContext) -> Option<Context> {
    let buffer = context.buffer()?;
    let end = buffer.iter_at_mark(&buffer.get_insert());
    let mut start = end;
    start.set_line_offset(0);
    analyze(&buffer.text(&start, &end, false))
}

fn proposals(ctx: &Context, data: Option<&SharedData>, context: &sourceview5::CompletionContext) -> Vec<LatexProposal> {
    let mut out = Vec::new();
    match ctx {
        Context::Command(_) => {
            for (name, snippet, detail) in COMMANDS {
                out.push(LatexProposal::new(Kind::Command, &format!("\\{name}"), snippet, detail, true));
            }
            if let Some(d) = data {
                for cmd in &d.borrow().index.commands {
                    out.push(LatexProposal::new(Kind::Command, &format!("\\{cmd}"), &format!("\\{cmd}"), "Definido en el proyecto", false));
                }
            }
            // La etiqueta empieza por "\", pero se filtra por lo escrito tras ella.
            for p in &out {
                let label = p.imp().label.borrow().trim_start_matches('\\').to_string();
                *p.imp().label.borrow_mut() = label;
            }
        }
        Context::Argument { command, .. } => match command.as_str() {
            "begin" => {
                for (name, rest) in ENVIRONMENTS {
                    out.push(LatexProposal::new(Kind::Environment, name, &format!("{name}{rest}"), "Entorno", true));
                }
            }
            "end" => {
                if let Some(env) = context.buffer().and_then(|b| open_environment(&b)) {
                    out.push(LatexProposal::new(Kind::Environment, &env, &env, "Cerrar entorno abierto", false));
                }
            }
            c if REF_COMMANDS.contains(&c) => {
                let mut labels: Vec<(String, String)> = data
                    .map(|d| d.borrow().index.labels.iter().map(|l| (l.name.clone(), format!("{}:{}", l.file.display(), l.line))).collect())
                    .unwrap_or_default();
                // Las del buffer actual, aunque aún no estén guardadas.
                if let Some(b) = context.buffer() {
                    let (s, e) = b.bounds();
                    for call in b.text(&s, &e, false).lines().flat_map(|l| crate::index::calls(l).into_iter().map(|c| (c.name.to_string(), c.arg.map(str::to_string))).collect::<Vec<_>>()) {
                        if let ("label", Some(name)) = (call.0.as_str(), call.1) {
                            if !labels.iter().any(|(n, _)| *n == name) {
                                labels.push((name, "este archivo".into()));
                            }
                        }
                    }
                }
                for (name, detail) in labels {
                    out.push(LatexProposal::new(Kind::Label, &name, &name, &detail, false));
                }
            }
            c if CITE_COMMANDS.contains(&c) => {
                if let Some(d) = data {
                    for e in &d.borrow().index.bib {
                        out.push(LatexProposal::new(Kind::Citation, &e.key, &e.key, &e.title, false));
                    }
                }
            }
            "input" | "include" | "subfile" => {
                if let Some(d) = data {
                    for f in &d.borrow().tex_files {
                        let s = f.with_extension("").display().to_string();
                        out.push(LatexProposal::new(Kind::File, &s, &s, "", false));
                    }
                }
            }
            "includegraphics" => {
                if let Some(d) = data {
                    for f in &d.borrow().images {
                        let s = f.display().to_string();
                        out.push(LatexProposal::new(Kind::File, &s, &s, "", false));
                    }
                }
            }
            "usepackage" | "RequirePackage" => {
                for p in PACKAGES {
                    out.push(LatexProposal::new(Kind::Package, p, p, "", false));
                }
            }
            _ => {}
        },
    }
    out
}

/// Último `\begin{x}` sin su `\end{x}` antes del cursor.
fn open_environment(buffer: &sourceview5::Buffer) -> Option<String> {
    let end = buffer.iter_at_mark(&buffer.get_insert());
    let text = buffer.text(&buffer.start_iter(), &end, false);
    let mut stack: Vec<String> = Vec::new();
    for line in text.lines() {
        for call in crate::index::calls(crate::project::strip_comment(line)) {
            match (call.name, call.arg) {
                ("begin", Some(env)) => stack.push(env.to_string()),
                ("end", Some(env)) => {
                    if let Some(pos) = stack.iter().rposition(|e| e == env) {
                        stack.truncate(pos);
                    }
                }
                _ => {}
            }
        }
    }
    stack.pop()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analyzes_contexts() {
        assert_eq!(analyze("texto \\sec"), Some(Context::Command("sec".into())));
        assert_eq!(analyze("\\"), Some(Context::Command(String::new())));
        assert_eq!(analyze("salto \\\\"), None);
        assert_eq!(analyze("ver \\ref{fig:"), Some(Context::Argument { command: "ref".into(), prefix: "fig:".into() }));
        assert_eq!(analyze("\\cite[p.~3]{knuth, lam"), Some(Context::Argument { command: "cite".into(), prefix: "lam".into() }));
        assert_eq!(analyze("\\begin{ite"), Some(Context::Argument { command: "begin".into(), prefix: "ite".into() }));
        assert_eq!(analyze("\\textbf{hola} y"), None);
        assert_eq!(analyze("palabra"), None);
    }
}
