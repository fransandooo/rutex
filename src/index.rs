//! Índice del proyecto: esquema de secciones (siguiendo `\input`/`\include` en
//! orden desde el archivo principal), etiquetas `\label` y entradas de los
//! `.bib`. Se recalcula en segundo plano tras cada guardado.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::project::{self, strip_comment};

const SECTIONS: &[(&str, u8)] = &[
    ("part", 0),
    ("chapter", 1),
    ("section", 2),
    ("subsection", 3),
    ("subsubsection", 4),
    ("paragraph", 5),
];
const INCLUDES: &[&str] = &["input", "include", "subfile", "InputIfFileExists"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Section {
    pub level: u8,
    pub title: String,
    pub file: PathBuf,
    pub line: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Label {
    pub name: String,
    pub file: PathBuf,
    pub line: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BibEntry {
    pub key: String,
    pub title: String,
    pub author: String,
    pub year: String,
    /// .bib file (relative to the root) and line of the entry.
    pub file: PathBuf,
    pub line: u32,
}

#[derive(Clone, Debug, Default)]
pub struct Index {
    pub outline: Vec<Section>,
    pub labels: Vec<Label>,
    pub bib: Vec<BibEntry>,
    /// Comandos definidos por el usuario (`\newcommand{\R}`…), sin la barra.
    pub commands: Vec<String>,
    /// Keys used in \ref-like commands, with where.
    pub refs_used: Vec<Located>,
    /// Keys used in \cite-like commands, with where.
    pub cites_used: Vec<Located>,
    /// `\nocite{*}` cites the whole bibliography.
    pub nocite_all: bool,
    /// Labels defined more than once (every definition).
    pub duplicate_labels: Vec<Label>,
    /// TODO / FIXME comments and \todo{} notes.
    pub todos: Vec<Located>,
}

/// A key or note and where it appears.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Located {
    pub text: String,
    pub file: PathBuf,
    pub line: u32,
}

/// Una llamada a comando encontrada en una línea: `\nombre*[opt]{arg}`.
#[derive(Debug, PartialEq, Eq)]
pub struct Call<'a> {
    pub name: &'a str,
    pub arg: Option<&'a str>,
}

/// Encuentra los comandos de una línea (ya sin comentarios) con su primer argumento.
pub fn calls(line: &str) -> Vec<Call<'_>> {
    let bytes = line.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'\\' {
            i += 1;
            continue;
        }
        let start = i + 1;
        let mut j = start;
        while j < bytes.len() && bytes[j].is_ascii_alphabetic() {
            j += 1;
        }
        if j == start {
            i += 2;
            continue;
        }
        let name = &line[start..j];
        if j < bytes.len() && bytes[j] == b'*' {
            j += 1;
        }
        while j < bytes.len() && bytes[j] == b' ' {
            j += 1;
        }
        if j < bytes.len() && bytes[j] == b'[' {
            if let Some(end) = matching(bytes, j, b'[', b']') {
                j = end + 1;
            }
        }
        let arg = if j < bytes.len() && bytes[j] == b'{' {
            matching(bytes, j, b'{', b'}').map(|end| {
                let a = &line[j + 1..end];
                j = end + 1;
                a
            })
        } else {
            None
        };
        out.push(Call { name, arg });
        i = j.max(i + 1);
    }
    out
}

fn matching(bytes: &[u8], open_at: usize, open: u8, close: u8) -> Option<usize> {
    let mut depth = 0;
    for (k, &b) in bytes.iter().enumerate().skip(open_at) {
        if b == open {
            depth += 1;
        } else if b == close {
            depth -= 1;
            if depth == 0 {
                return Some(k);
            }
        }
    }
    None
}

/// Título legible: quita comandos y llaves (`Uso de \textbf{GPU}` → `Uso de GPU`).
pub fn clean_title(raw: &str) -> String {
    let mut out = String::new();
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                let mut name = String::new();
                while let Some(&n) = chars.peek() {
                    if n.is_ascii_alphabetic() {
                        name.push(n);
                        chars.next();
                    } else {
                        break;
                    }
                }
                if name.is_empty() {
                    // \% \& \_ … → el carácter literal.
                    if let Some(n) = chars.next() {
                        out.push(n);
                    }
                }
            }
            '{' | '}' | '$' => {}
            '~' => out.push(' '),
            c => out.push(c),
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn build(root: &Path, main: &Path, all_files: &[PathBuf]) -> Index {
    let mut index = Index::default();
    let mut visited = HashSet::new();
    walk(root, main, 0, &mut visited, &mut index);

    // Etiquetas y comandos de archivos .tex no incluidos desde el principal.
    for f in all_files.iter().filter(|f| project::has_ext(f, "tex") && !visited.contains(*f)) {
        if let Ok(src) = fs::read_to_string(root.join(f)) {
            scan(&src, f, &mut index, false);
        }
    }

    let mut seen = HashSet::new();
    for f in all_files.iter().filter(|f| project::has_ext(f, "bib")) {
        if let Ok(src) = fs::read_to_string(root.join(f)) {
            for mut e in parse_bib(&src) {
                if seen.insert(e.key.clone()) {
                    e.file = f.clone();
                    index.bib.push(e);
                }
            }
        }
    }
    index.labels.sort_by(|a, b| a.name.cmp(&b.name));
    index.duplicate_labels = index
        .labels
        .iter()
        .filter(|l| index.labels.iter().filter(|o| o.name == l.name).count() > 1)
        .cloned()
        .collect();
    index.labels.dedup_by(|a, b| a.name == b.name);
    index.commands.sort();
    index.commands.dedup();
    index
}

fn walk(root: &Path, rel: &Path, depth: usize, visited: &mut HashSet<PathBuf>, index: &mut Index) {
    if depth > 16 || !visited.insert(rel.to_path_buf()) {
        return;
    }
    let Ok(src) = fs::read_to_string(root.join(rel)) else { return };
    for (n, line) in src.lines().enumerate() {
        find_todo(line, rel, n as u32 + 1, index);
        option_labels(line, rel, n as u32 + 1, index);
        let line = strip_comment(line);
        for call in calls(line) {
            scan_call(&call, rel, n as u32 + 1, index, true);
            if INCLUDES.contains(&call.name) {
                if let Some(target) = call.arg.and_then(|a| resolve(root, a.trim())) {
                    walk(root, &target, depth + 1, visited, index);
                }
            }
        }
    }
}

fn scan(src: &str, rel: &Path, index: &mut Index, outline: bool) {
    for (n, line) in src.lines().enumerate() {
        find_todo(line, rel, n as u32 + 1, index);
        option_labels(line, rel, n as u32 + 1, index);
        for call in calls(strip_comment(line)) {
            scan_call(&call, rel, n as u32 + 1, index, outline);
        }
    }
}

/// Labels given as options: `\begin{lstlisting}[label={lst:rust}]`, `label=x`.
fn option_labels(line: &str, rel: &Path, n: u32, index: &mut Index) {
    let mut rest = strip_comment(line);
    while let Some(i) = rest.find("label=").or_else(|| rest.find("label =")) {
        // `\label{…}` is handled elsewhere; only key=value options here.
        let after = rest[i..].split_once('=').map_or("", |x| x.1).trim_start();
        let name: String = if let Some(braced) = after.strip_prefix('{') {
            braced.chars().take_while(|c| *c != '}').collect()
        } else {
            after.chars().take_while(|c| !matches!(c, ',' | ']' | '}' | ' ')).collect()
        };
        let name = name.trim();
        if !name.is_empty() && !index.labels.iter().any(|l| l.name == name) {
            index.labels.push(Label { name: name.to_string(), file: rel.to_path_buf(), line: n });
        }
        rest = &rest[i + 6..];
    }
}

/// `% TODO: …`, `% FIXME …` and `\todo{…}`.
fn find_todo(line: &str, rel: &Path, n: u32, index: &mut Index) {
    let code = strip_comment(line);
    let comment = &line[code.len()..];
    let note = if let Some(i) = comment.find("TODO").or_else(|| comment.find("FIXME")) {
        Some(comment[i..].trim().to_string())
    } else {
        calls(code).into_iter().find(|c| c.name == "todo").and_then(|c| c.arg).map(|a| format!("\\todo: {}", clean_title(a)))
    };
    if let Some(text) = note {
        index.todos.push(Located { text, file: rel.to_path_buf(), line: n });
    }
}

fn scan_call(call: &Call, rel: &Path, line: u32, index: &mut Index, outline: bool) {
    let Some(arg) = call.arg else { return };
    if outline && let Some((_, level)) = SECTIONS.iter().find(|(n, _)| *n == call.name) {
        index.outline.push(Section { level: *level, title: clean_title(arg), file: rel.to_path_buf(), line });
    } else if crate::completion::REF_COMMANDS.contains(&call.name) {
        for key in arg.split(',').map(str::trim).filter(|k| !k.is_empty()) {
            index.refs_used.push(Located { text: key.to_string(), file: rel.to_path_buf(), line });
        }
    } else if crate::completion::CITE_COMMANDS.contains(&call.name) {
        for key in arg.split(',').map(str::trim).filter(|k| !k.is_empty()) {
            if key == "*" {
                index.nocite_all = true;
            } else {
                index.cites_used.push(Located { text: key.to_string(), file: rel.to_path_buf(), line });
            }
        }
    } else if call.name == "label" {
        index.labels.push(Label { name: arg.trim().to_string(), file: rel.to_path_buf(), line });
    } else if matches!(call.name, "newcommand" | "renewcommand" | "DeclareMathOperator" | "providecommand") {
        let name = arg.trim().trim_start_matches('\\');
        if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphabetic()) {
            index.commands.push(name.to_string());
        }
    }
}

/// `capitulos/intro` → `capitulos/intro.tex` si existe.
fn resolve(root: &Path, arg: &str) -> Option<PathBuf> {
    let p = PathBuf::from(arg);
    let candidates = [p.clone(), p.with_extension("tex"), PathBuf::from(format!("{arg}.tex"))];
    candidates.into_iter().find(|c| root.join(c).is_file())
}

pub fn parse_bib(src: &str) -> Vec<BibEntry> {
    let mut out = Vec::new();
    let mut rest = src;
    while let Some(at) = rest.find('@') {
        let line = src[..src.len() - rest.len() + at].matches('\n').count() as u32 + 1;
        rest = &rest[at + 1..];
        let kind: String = rest.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
        let lower = kind.to_ascii_lowercase();
        if kind.is_empty() || matches!(lower.as_str(), "comment" | "string" | "preamble") {
            continue;
        }
        let after = rest[kind.len()..].trim_start();
        let Some(body) = after.strip_prefix('{').or_else(|| after.strip_prefix('(')) else { continue };
        let Some(comma) = body.find(',') else { continue };
        let key = body[..comma].trim().to_string();
        if key.is_empty() || key.contains(char::is_whitespace) {
            continue;
        }
        let entry_end = body.find("\n@").unwrap_or(body.len());
        let entry = &body[..entry_end];
        let title = field(entry, "title").map(|t| clean_title(&t)).unwrap_or_default();
        let author = field(entry, "author").map(|a| short_authors(&clean_title(&a))).unwrap_or_default();
        let year = field(entry, "year").or_else(|| field(entry, "date").map(|d| d.chars().take(4).collect())).unwrap_or_default();
        out.push(BibEntry { key, title, author, year: year.trim().to_string(), file: PathBuf::new(), line });
    }
    out
}

/// "Knuth, Donald and Lamport, Leslie" → "Knuth and Lamport"; 3+ → "Knuth et al."
fn short_authors(raw: &str) -> String {
    let surname = |a: &str| -> String {
        let a = a.trim();
        match a.split_once(',') {
            Some((last, _)) => last.trim().to_string(),
            None => a.split_whitespace().last().unwrap_or(a).to_string(),
        }
    };
    let authors: Vec<&str> = raw.split(" and ").collect();
    match authors.len() {
        0 => String::new(),
        1 => surname(authors[0]),
        2 => format!("{} & {}", surname(authors[0]), surname(authors[1])),
        _ => format!("{} et al.", surname(authors[0])),
    }
}

fn field(entry: &str, name: &str) -> Option<String> {
    let lower = entry.to_ascii_lowercase();
    let mut from = 0;
    while let Some(pos) = lower[from..].find(name) {
        let start = from + pos;
        from = start + name.len();
        let before_ok = start == 0 || !lower.as_bytes()[start - 1].is_ascii_alphanumeric();
        let after = lower[from..].trim_start();
        if !before_ok || !after.starts_with('=') {
            continue;
        }
        let value_start = entry.len() - after.len() + 1;
        let value = entry[value_start..].trim_start();
        let bytes = value.as_bytes();
        return match bytes.first()? {
            b'{' => matching(bytes, 0, b'{', b'}').map(|end| value[1..end].to_string()),
            b'"' => value[1..].find('"').map(|end| value[1..end + 1].to_string()),
            _ => None,
        };
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_calls() {
        let c = calls(r"\section*[corto]{Intro \textbf{GPU}} y \label{sec:intro}");
        assert_eq!(c[0], Call { name: "section", arg: Some(r"Intro \textbf{GPU}") });
        assert!(c.iter().any(|c| c.name == "label" && c.arg == Some("sec:intro")));
        assert!(calls(r"50\% y \\ salto").is_empty());
    }

    #[test]
    fn cleans_titles() {
        assert_eq!(clean_title(r"Uso de \textbf{GPU} en $\mathbb{R}^n$"), "Uso de GPU en R^n");
        assert_eq!(clean_title(r"Ruido~y 50\%"), "Ruido y 50%");
    }

    #[test]
    fn parses_bib() {
        let bib = "@article{knuth84,\n  author = {Donald Knuth},\n  title = {Literate {P}rogramming},\n}\n@comment{x}\n@book{lamport,\n title=\"LaTeX\"}";
        let e = parse_bib(bib);
        assert_eq!(e.len(), 2);
        assert_eq!(e[0].key, "knuth84");
        assert_eq!(e[0].title, "Literate Programming");
        assert_eq!(e[0].author, "Knuth");
        assert_eq!(e[0].line, 1);
        assert_eq!(e[1].line, 6);
        assert_eq!(short_authors("Vaswani, Ashish and Shazeer, Noam and Parmar, Niki"), "Vaswani et al.");
        assert_eq!(short_authors("Leslie Lamport and Donald Knuth"), "Lamport & Knuth");
        assert_eq!(e[1].title, "LaTeX");
    }

    #[test]
    fn walks_includes_in_order() {
        let dir = std::env::temp_dir().join(format!("rutex-index-{}", std::process::id()));
        fs::create_dir_all(dir.join("cap")).unwrap();
        fs::write(dir.join("main.tex"), "\\chapter{Uno}\n\\input{cap/a}\n\\chapter{Tres}\n\\newcommand{\\R}{x}").unwrap();
        fs::write(dir.join("cap/a.tex"), "\\section{Dos}\\label{sec:dos}").unwrap();
        let idx = build(&dir, Path::new("main.tex"), &[PathBuf::from("main.tex"), PathBuf::from("cap/a.tex")]);
        let titles: Vec<_> = idx.outline.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(titles, ["Uno", "Dos", "Tres"]);
        assert_eq!(idx.outline[1].file, PathBuf::from("cap/a.tex"));
        assert_eq!(idx.labels[0].name, "sec:dos");
        assert_eq!(idx.commands, ["R"]);
        let mut li = Index::default();
        option_labels("\\begin{lstlisting}[language=Rust, label={lst:rust}]", Path::new("a.tex"), 3, &mut li);
        option_labels("\\begin{tcolorbox}[label=box:one]", Path::new("a.tex"), 4, &mut li);
        assert_eq!(li.labels.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(), ["lst:rust", "box:one"]);
        fs::remove_dir_all(dir).unwrap();
    }
}
