//! Export to Word (.docx) through pandoc. pandoc reads LaTeX well (text,
//! sections, math as real Word equations, citations, lists, code), but drops
//! drawings and leaves cross-references as label names. So the document is
//! prepared first, in a copy in the build folder:
//!
//! - every \input/\include is inlined and comments are removed;
//! - \ref, \cref, \eqref… become the numbers of the last build (from the .aux);
//! - TikZ, circuits, diagrams, Gantt charts, trees and pseudocode are compiled
//!   with the project's preamble and inserted as images;
//! - PDF figures become PNG (Word cannot show PDF);
//! - table column types pandoc does not know (siunitx S, tabularx X, @{}…)
//!   are simplified.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use gtk::glib;

use crate::i18n::{tr, trf};

/// Environments that pandoc cannot convert and that become images.
const PICTURES: &[&str] = &["tikzpicture", "circuitikz", "tikzcd", "forest", "ganttchart", "algorithmic", "algorithmic*", "pgfpicture"];
/// Environments whose content is code, left untouched.
const VERBATIM: &[&str] = &["verbatim", "verbatim*", "Verbatim", "lstlisting", "minted", "comment"];
/// Display math environments that get their equation numbers written in.
const MATH: &[&str] = &["equation", "align", "gather", "multline", "flalign", "alignat", "eqnarray"];

/// pandoc on the PATH (or RUTEX_PANDOC, for tests).
pub fn find_pandoc() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("RUTEX_PANDOC").map(PathBuf::from).filter(|p| p.is_file()) {
        return Some(p);
    }
    std::env::split_paths(&std::env::var_os("PATH")?).map(|d| d.join("pandoc")).find(|p| p.is_file())
}

/// How to install pandoc on this system.
pub fn install_hint() -> &'static str {
    let has = |p: &str| Path::new(p).exists();
    if has("/usr/bin/pacman") {
        "sudo pacman -S pandoc-cli"
    } else if has("/usr/bin/apt") {
        "sudo apt install pandoc"
    } else if has("/usr/bin/dnf") {
        "sudo dnf install pandoc"
    } else if has("/usr/bin/zypper") {
        "sudo zypper install pandoc"
    } else {
        "https://pandoc.org/installing.html"
    }
}

pub struct Export<'a> {
    pub pandoc: &'a Path,
    pub root: &'a Path,
    pub main: &'a Path,
    pub build_dir: &'a Path,
    pub out: &'a Path,
}

/// Exports the document (blocking). Returns notes about what could not be
/// converted exactly.
pub fn export(e: &Export) -> Result<Vec<String>, String> {
    let dir = e.build_dir.join("word");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).map_err(|err| err.to_string())?;
    let main_src = fs::read_to_string(e.root.join(e.main)).map_err(|err| err.to_string())?;
    let spanish = crate::spell::document_language(&main_src).is_some_and(|l| l.starts_with("es"));
    let mut notes = Vec::new();

    let text = flatten(e.root, e.main, 0);
    let labels = read_labels(e.build_dir);
    if labels.is_empty() {
        notes.push(tr("Compile the document once so references get their numbers.").to_string());
    }
    let text = number_equations(&text, &labels);
    let (text, missing) = replace_refs(&text, &labels, spanish);
    if !missing.is_empty() {
        notes.push(trf("References without a number: {}", &[&missing.join(", ")]));
    }

    // Drawings and pseudocode → images.
    let mut bodies = Vec::new();
    replace_envs(&text, PICTURES, &mut |env| {
        bodies.push(env.to_string());
        env.to_string()
    });
    let preamble = crate::mathpreview::preamble_of(&main_src);
    let program = match crate::compiler::Engine::detect(e.root, &main_src) {
        crate::compiler::Engine::XeLatex => "xelatex",
        crate::compiler::Engine::LuaLatex => "lualatex",
        _ => "pdflatex",
    };
    let mut rendered = render_pictures(e.root, &dir, &preamble, program, &bodies).into_iter();
    let mut failed = 0;
    let text = replace_envs(&text, PICTURES, &mut |_| match rendered.next() {
        Some(Ok((png, width))) => format!("\\includegraphics[width={width:.1}pt]{{{}}}", png.display()),
        _ => {
            failed += 1;
            format!("\\textit{{[{}]}}", tr("drawing not converted"))
        }
    });
    if failed > 0 {
        notes.push(trf("{} drawings could not be converted; they are marked in the document.", &[&failed]));
    }

    let text = convert_images(&text, e.root, &dir, &mut notes);
    let text = simplify_tables(&text);
    let flat = dir.join("document.tex");
    fs::write(&flat, &text).map_err(|err| err.to_string())?;

    let mut cmd = crate::compiler::command(&e.pandoc.to_string_lossy());
    cmd.current_dir(e.root)
        .arg(&flat)
        .args(["-f", "latex", "-t", "docx", "--number-sections", "-o"])
        .arg(e.out)
        .arg(format!("--resource-path={}:{}", e.root.display(), dir.display()))
        .arg("-M")
        .arg(if spanish { "lang=es-ES" } else { "lang=en-US" });
    if text.contains("\\tableofcontents") {
        cmd.args(["--toc", "-M"]).arg(if spanish { "toc-title=Índice" } else { "toc-title=Contents" });
    }
    let bibs = bibliographies(&text, e.root);
    if !bibs.is_empty() {
        cmd.args(["--citeproc", "-M", "link-citations=true", "-M"]).arg(if spanish { "reference-section-title=Referencias" } else { "reference-section-title=References" });
        for b in &bibs {
            cmd.arg(format!("--bibliography={}", b.display()));
        }
    }
    let out = cmd.stdout(std::process::Stdio::null()).output().map_err(|err| err.to_string())?;
    let stderr = String::from_utf8_lossy(&out.stderr);
    if !out.status.success() || !e.out.is_file() {
        return Err(stderr.lines().last().unwrap_or("pandoc failed").to_string());
    }
    for line in stderr.lines().filter(|l| l.starts_with("[WARNING]")).take(8) {
        let msg = line.trim_start_matches("[WARNING]").trim().to_string();
        if !notes.contains(&msg) {
            notes.push(msg);
        }
    }
    Ok(notes)
}

/// The document as one text: \input / \include / \subfile inlined, comments
/// removed (except inside code environments).
pub fn flatten(root: &Path, rel: &Path, depth: u32) -> String {
    let path = root.join(rel);
    let path = if path.extension().is_some() { path } else { path.with_extension("tex") };
    let Ok(src) = fs::read_to_string(&path) else { return String::new() };
    let mut out = String::new();
    let mut verbatim: Option<&str> = None;
    for line in src.lines() {
        if let Some(env) = verbatim {
            out.push_str(line);
            out.push('\n');
            if line.contains(&format!("\\end{{{env}}}")) {
                verbatim = None;
            }
            continue;
        }
        let code = crate::project::strip_comment(line);
        if code.trim().is_empty() && code.len() != line.len() {
            // A comment-only line: dropping it keeps the paragraph together.
            continue;
        }
        verbatim = VERBATIM.iter().copied().find(|v| code.contains(&format!("\\begin{{{v}}}")) && !code.contains(&format!("\\end{{{v}}}")));
        let mut rest = code;
        loop {
            let found = ["\\input{", "\\include{", "\\subfile{"].iter().filter_map(|k| rest.find(k).map(|i| (i, *k))).min();
            let Some((i, k)) = found else {
                out.push_str(rest);
                break;
            };
            out.push_str(&rest[..i]);
            let after = &rest[i + k.len()..];
            let Some(close) = after.find('}') else {
                out.push_str(&rest[i..]);
                break;
            };
            if depth < 20 {
                let inner = flatten(root, Path::new(after[..close].trim()), depth + 1);
                // A subfile is a whole document: keep its body.
                let inner = match (inner.find("\\begin{document}"), inner.rfind("\\end{document}")) {
                    (Some(a), Some(b)) if k == "\\subfile{" && a < b => inner[a + 16..b].to_string(),
                    _ => inner,
                };
                // Its last line break would end the paragraph early.
                out.push_str(inner.strip_suffix('\n').unwrap_or(&inner));
            }
            rest = &after[close + 1..];
        }
        out.push('\n');
    }
    out
}

#[derive(Default, Debug)]
pub struct Label {
    pub number: String,
    pub page: String,
    pub title: String,
    pub kind: String,
}

/// Top-level {…} groups of a string.
fn groups(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut depth, mut start) = (0, 0);
    for (i, c) in s.char_indices() {
        match c {
            '{' => {
                if depth == 0 {
                    start = i + 1;
                }
                depth += 1;
            }
            '}' if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    out.push(&s[start..i]);
                }
            }
            _ => {}
        }
    }
    out
}

fn clean(s: &str) -> String {
    s.replace("\\relax", "").replace(['{', '}'], "").trim().to_string()
}

/// Labels from `\newlabel` lines (LaTeX, hyperref and cleveref formats).
pub fn parse_aux(aux: &str, labels: &mut HashMap<String, Label>) {
    for line in aux.lines() {
        let Some(rest) = line.strip_prefix("\\newlabel{") else { continue };
        let Some(end) = rest.find('}') else { continue };
        let key = &rest[..end];
        let Some(data) = groups(&rest[end + 1..]).first().copied() else { continue };
        let g = groups(data);
        if let Some(base) = key.strip_suffix("@cref") {
            // {[figure][1][3]3.1}{…}: the counter's type.
            if let Some(kind) = g.first().and_then(|n| n.strip_prefix('[')).and_then(|n| n.split(']').next()) {
                labels.entry(base.to_string()).or_default().kind = kind.to_string();
            }
            continue;
        }
        let l = labels.entry(key.to_string()).or_default();
        l.number = g.first().map(|n| clean(n)).unwrap_or_default();
        l.page = g.get(1).map(|n| clean(n)).unwrap_or_default();
        l.title = g.get(2).map(|n| clean(n)).unwrap_or_default();
        // hyperref anchors look like "figure.caption.5" or "section.2.1".
        if l.kind.is_empty()
            && let Some(anchor) = g.get(3)
        {
            l.kind = anchor.split('.').next().unwrap_or("").to_string();
        }
    }
}

/// All the labels of the last build (\include'd files have their own .aux).
pub fn read_labels(build_dir: &Path) -> HashMap<String, Label> {
    let mut labels = HashMap::new();
    let mut dirs = vec![(build_dir.to_path_buf(), 0)];
    while let Some((d, depth)) = dirs.pop() {
        for entry in fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if p.is_dir() && depth < 3 && name != "math" && name != "word" {
                dirs.push((p, depth + 1));
            } else if name.ends_with(".aux") {
                parse_aux(&fs::read_to_string(&p).unwrap_or_default(), &mut labels);
            }
        }
    }
    labels
}

fn kind_name(kind: &str, spanish: bool) -> &'static str {
    let (en, es) = match kind {
        "figure" | "subfigure" => ("Figure", "Figura"),
        "table" | "subtable" => ("Table", "Tabla"),
        "equation" | "align" | "gather" => ("Equation", "Ecuación"),
        "chapter" => ("Chapter", "Capítulo"),
        "section" | "subsection" | "subsubsection" => ("Section", "Sección"),
        "appendix" | "subappendix" => ("Appendix", "Apéndice"),
        "part" => ("Part", "Parte"),
        "algorithm" | "ALC@unique" => ("Algorithm", "Algoritmo"),
        "lstlisting" | "listing" | "lstnumber" => ("Listing", "Listado"),
        "theorem" | "thm" => ("Theorem", "Teorema"),
        "lemma" | "lem" => ("Lemma", "Lema"),
        "proposition" | "prop" => ("Proposition", "Proposición"),
        "corollary" | "cor" => ("Corollary", "Corolario"),
        "definition" | "defn" | "def" => ("Definition", "Definición"),
        "remark" | "rem" => ("Remark", "Observación"),
        "example" => ("Example", "Ejemplo"),
        "footnote" => ("Footnote", "Nota"),
        _ => ("", ""),
    };
    if spanish { es } else { en }
}

fn plural(name: &str, spanish: bool) -> String {
    match (spanish, name.strip_suffix("ón")) {
        (true, Some(stem)) => format!("{stem}ones"),
        _ => format!("{name}s"),
    }
}

/// Splits text into code and verbatim pieces and transforms only the code.
fn outside_verbatim(text: &str, f: &mut dyn FnMut(&str) -> String) -> String {
    let mut out = String::new();
    let mut rest = text;
    loop {
        let next = VERBATIM.iter().filter_map(|v| rest.find(&format!("\\begin{{{v}}}")).map(|i| (i, *v))).min();
        let Some((i, v)) = next else {
            out.push_str(&f(rest));
            return out;
        };
        out.push_str(&f(&rest[..i]));
        let end_tag = format!("\\end{{{v}}}");
        let end = rest[i..].find(&end_tag).map_or(rest.len(), |e| i + e + end_tag.len());
        out.push_str(&rest[i..end]);
        rest = &rest[end..];
    }
}

/// Reads `{…}` right after `s` (skipping a `*` and spaces): (content, rest).
fn braced(s: &str) -> Option<(&str, &str)> {
    let s = s.trim_start_matches('*').trim_start();
    let s = s.strip_prefix('{')?;
    let mut depth = 1;
    for (i, c) in s.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some((&s[..i], &s[i + 1..]));
                }
            }
            _ => {}
        }
    }
    None
}

/// \ref and friends → numbers. Returns the text and the keys not found.
pub fn replace_refs(text: &str, labels: &HashMap<String, Label>, spanish: bool) -> (String, Vec<String>) {
    const COMMANDS: &[&str] = &["Autoref", "autoref", "cpageref", "Cref", "cref", "eqref", "nameref", "pageref", "Vref", "vref", "ref"];
    let mut missing: Vec<String> = Vec::new();
    let out = outside_verbatim(text, &mut |code| {
        let mut out = String::new();
        let mut rest = code;
        while let Some(i) = rest.find('\\') {
            out.push_str(&rest[..i]);
            let after = &rest[i + 1..];
            let name_len = after.find(|c: char| !c.is_ascii_alphabetic()).unwrap_or(after.len());
            let name = &after[..name_len];
            let parsed = COMMANDS.contains(&name).then(|| braced(&after[name_len..])).flatten();
            let Some((keys, tail)) = parsed else {
                out.push('\\');
                out.push_str(name);
                rest = &after[name_len..];
                continue;
            };
            let found: Vec<(&str, Option<&Label>)> = keys.split(',').map(str::trim).filter(|k| !k.is_empty()).map(|k| (k, labels.get(k).filter(|l| !l.number.is_empty()))).collect();
            for (k, l) in &found {
                if l.is_none() && !missing.iter().any(|m| m == k) {
                    missing.push(k.to_string());
                }
            }
            let num = |l: Option<&Label>| l.map_or("??".to_string(), |l| if l.kind == "equation" { format!("({})", l.number) } else { l.number.clone() });
            let replacement = match name {
                "ref" => found.iter().map(|(_, l)| l.map_or("??".into(), |l| l.number.clone())).collect::<Vec<_>>().join(", "),
                "eqref" => found.iter().map(|(_, l)| l.map_or("(??)".into(), |l| format!("({})", l.number))).collect::<Vec<_>>().join(", "),
                "pageref" | "cpageref" => found.iter().map(|(_, l)| l.map_or("??".into(), |l| l.page.clone())).collect::<Vec<_>>().join(", "),
                "nameref" => found.iter().map(|(_, l)| l.map_or("??".into(), |l| l.title.clone())).collect::<Vec<_>>().join(", "),
                _ => {
                    // \cref{a,b} → "Figures 3.1 and 3.2"; mixed types name each.
                    let kinds: Vec<&str> = found.iter().map(|(_, l)| l.map_or("", |l| kind_name(&l.kind, spanish))).collect();
                    let numbers: Vec<String> = found.iter().map(|(_, l)| num(*l)).collect();
                    let and = if spanish { " y " } else { " and " };
                    let join = |items: Vec<String>| match items.len() {
                        0 | 1 => items.concat(),
                        n => format!("{}{and}{}", items[..n - 1].join(", "), items[n - 1]),
                    };
                    if kinds.iter().all(|k| *k == kinds[0]) && !kinds[0].is_empty() {
                        let label = if numbers.len() > 1 { plural(kinds[0], spanish) } else { kinds[0].to_string() };
                        format!("{label}~{}", join(numbers))
                    } else {
                        join(kinds.iter().zip(numbers).map(|(k, n)| if k.is_empty() { n } else { format!("{k}~{n}") }).collect())
                    }
                }
            };
            out.push_str(&replacement);
            rest = tail;
        }
        out.push_str(rest);
        out
    });
    (out, missing)
}

/// Finds the outermost occurrences of `envs` (outside code) and replaces each
/// whole `\begin…\end` with `f(it)`.
pub fn replace_envs(text: &str, envs: &[&str], f: &mut dyn FnMut(&str) -> String) -> String {
    outside_verbatim(text, &mut |code| {
        let mut out = String::new();
        let mut rest = code;
        while let Some(i) = rest.find("\\begin{") {
            let after = &rest[i + 7..];
            let Some(close) = after.find('}') else { break };
            let name = &after[..close];
            if !envs.contains(&name) {
                out.push_str(&rest[..i + 7]);
                rest = after;
                continue;
            }
            let (open, end) = (format!("\\begin{{{name}}}"), format!("\\end{{{name}}}"));
            // Matching \end, counting nested environments of the same name.
            let (mut depth, mut pos, mut stop) = (0usize, i, None);
            while pos < rest.len() {
                let (o, e) = (rest[pos..].find(&open), rest[pos..].find(&end));
                match (o, e) {
                    (Some(o), Some(e)) if o < e => {
                        depth += 1;
                        pos += o + open.len();
                    }
                    (_, Some(e)) => {
                        depth -= 1;
                        pos += e + end.len();
                        if depth == 0 {
                            stop = Some(pos);
                            break;
                        }
                    }
                    _ => break,
                }
            }
            let Some(stop) = stop else { break };
            out.push_str(&rest[..i]);
            out.push_str(&f(&rest[i..stop]));
            rest = &rest[stop..];
        }
        out.push_str(rest);
        out
    })
}

/// Word equations have no numbers: write the number of each labelled
/// equation after it, where its \label was.
pub fn number_equations(text: &str, labels: &HashMap<String, Label>) -> String {
    replace_envs(text, MATH, &mut |env| {
        let mut out = String::new();
        let mut rest = env;
        while let Some(i) = rest.find("\\label") {
            out.push_str(&rest[..i]);
            match braced(&rest[i + 6..]) {
                Some((key, tail)) => {
                    if let Some(l) = labels.get(key.trim()).filter(|l| !l.number.is_empty()) {
                        out.push_str(&format!("\\qquad\\text{{({})}}", l.number));
                    }
                    rest = tail;
                }
                None => {
                    out.push_str("\\label");
                    rest = &rest[i + 6..];
                }
            }
        }
        out.push_str(rest);
        out
    })
}

/// Compiles the drawings (all in one document when possible) and renders each
/// to a PNG. Returns (image, width in points) per drawing.
fn render_pictures(root: &Path, dir: &Path, preamble: &str, program: &str, bodies: &[String]) -> Vec<Result<(PathBuf, f64), String>> {
    if bodies.is_empty() {
        return Vec::new();
    }
    let item = |b: &str| {
        let b = if b.starts_with("\\begin{algorithmic") { format!("\\begin{{varwidth}}{{40em}}\n{b}\n\\end{{varwidth}}") } else { b.to_string() };
        format!("\\begin{{rutexpic}}\n{b}\n\\end{{rutexpic}}\n")
    };
    let doc = |items: String| {
        format!("\\documentclass[border=2pt]{{standalone}}\n\\usepackage{{varwidth}}\n{preamble}\n\\newenvironment{{rutexpic}}{{}}{{}}\n\\standaloneenv{{rutexpic}}\n\\begin{{document}}\n{items}\\end{{document}}\n")
    };
    let compile = |name: &str, src: String| -> Option<poppler::Document> {
        let tex = dir.join(format!("{name}.tex"));
        fs::write(&tex, src).ok()?;
        let args = vec!["-interaction=nonstopmode".into(), format!("-output-directory={}", dir.display()), tex.display().to_string()];
        crate::mathpreview::run_tex(root, dir, program, &args, false, Duration::from_secs(120)).ok()?;
        let data = fs::read(dir.join(format!("{name}.pdf"))).ok()?;
        poppler::Document::from_bytes(&glib::Bytes::from_owned(data), None).ok()
    };
    let to_png = |page: Option<poppler::Page>, i: usize| -> Result<(PathBuf, f64), String> {
        let page = page.ok_or("no page")?;
        let png = dir.join(format!("drawing-{}.png", i + 1));
        let width = page_to_png(&page, &png)?;
        Ok((png, width))
    };
    // One compile for everything; if a drawing breaks it, one at a time.
    if let Some(doc_all) = compile("drawings", doc(bodies.iter().map(|b| item(b)).collect()))
        && doc_all.n_pages() as usize == bodies.len()
    {
        return (0..bodies.len()).map(|i| to_png(doc_all.page(i as i32), i)).collect();
    }
    bodies
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let single = compile(&format!("drawing-{}", i + 1), doc(item(b))).ok_or("LaTeX error")?;
            to_png(single.page(0), i)
        })
        .collect()
}

/// Renders a PDF page on white at 200 dpi; returns its width in points.
fn page_to_png(page: &poppler::Page, png: &Path) -> Result<f64, String> {
    let (pw, ph) = page.size();
    let scale = 200.0 / 72.0;
    let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, (pw * scale).ceil() as i32, (ph * scale).ceil() as i32).map_err(|e| e.to_string())?;
    {
        let cr = cairo::Context::new(&surface).map_err(|e| e.to_string())?;
        cr.set_source_rgb(1.0, 1.0, 1.0);
        cr.paint().map_err(|e| e.to_string())?;
        cr.scale(scale, scale);
        page.render(&cr);
    }
    let mut file = fs::File::create(png).map_err(|e| e.to_string())?;
    surface.write_to_png(&mut file).map_err(|e| e.to_string())?;
    Ok(pw)
}

/// \includegraphics paths made absolute; PDF figures converted to PNG.
fn convert_images(text: &str, root: &Path, dir: &Path, notes: &mut Vec<String>) -> String {
    let mut search = vec![root.to_path_buf()];
    if let Some(i) = text.find("\\graphicspath") {
        for g in groups(&text[i..]).first().map(|s| groups(s)).unwrap_or_default() {
            search.push(root.join(g));
        }
    }
    let mut n = 0;
    outside_verbatim(text, &mut |code| {
        let mut out = String::new();
        let mut rest = code;
        while let Some(i) = rest.find("\\includegraphics") {
            out.push_str(&rest[..i]);
            let mut after = &rest[i + 16..];
            let mut opts = String::new();
            if let Some(o) = after.trim_start().strip_prefix('[')
                && let Some(c) = o.find(']')
            {
                opts = o[..c].to_string();
                after = &o[c + 1..];
            }
            let Some((path, tail)) = braced(after) else {
                out.push_str("\\includegraphics");
                rest = &rest[i + 16..];
                continue;
            };
            let found = search.iter().flat_map(|d| ["", ".pdf", ".png", ".jpg", ".jpeg", ".eps"].map(|ext| d.join(format!("{}{ext}", path.trim())))).find(|p| p.is_file());
            let target = match found {
                Some(p) if crate::project::has_ext(&p, "pdf") => {
                    n += 1;
                    let png = dir.join(format!("figure-{n}.png"));
                    let converted = poppler::Document::from_file(&glib::filename_to_uri(&p, None).unwrap_or_default(), None).ok().and_then(|d| d.page(0)).map(|page| page_to_png(&page, &png));
                    match converted {
                        Some(Ok(width)) => {
                            if opts.is_empty() {
                                opts = format!("width={width:.1}pt");
                            }
                            png
                        }
                        _ => p,
                    }
                }
                Some(p) => {
                    if crate::project::has_ext(&p, "eps") {
                        notes.push(trf("EPS images are not supported by Word: {}", &[&path]));
                    }
                    p
                }
                None => {
                    notes.push(trf("Image not found: {}", &[&path]));
                    PathBuf::from(path)
                }
            };
            out.push_str(&format!("\\includegraphics[{opts}]{{{}}}", target.display()));
            rest = tail;
        }
        out.push_str(rest);
        out
    })
}

/// Column types and rules pandoc does not understand → plain ones.
pub fn simplify_tables(text: &str) -> String {
    outside_verbatim(text, &mut |code| {
        let mut s = code.to_string();
        for (from, to) in [("\\begin{tabularx}", "\\begin{tabular}"), ("\\end{tabularx}", "\\end{tabular}"), ("\\begin{tabular*}", "\\begin{tabular}"), ("\\end{tabular*}", "\\end{tabular}")] {
            if from.starts_with("\\begin") {
                // tabularx / tabular* take a width first: drop it.
                while let Some(i) = s.find(from) {
                    let tail = braced(&s[i + from.len()..]).map(|(_, t)| t.to_string()).unwrap_or_default();
                    s = format!("{}{to}{tail}", &s[..i]);
                }
            } else {
                s = s.replace(from, to);
            }
        }
        // Column specs.
        let mut out = String::new();
        let mut rest = s.as_str();
        while let Some(i) = rest.find("\\begin{tabular}").or_else(|| rest.find("\\begin{longtable}")) {
            let head = if rest[i..].starts_with("\\begin{tabular}") { 15 } else { 17 };
            out.push_str(&rest[..i + head]);
            let after = &rest[i + head..];
            let after = after.strip_prefix(|c: char| c == '[').map_or(after, |a| a.find(']').map_or(a, |c| &a[c + 1..]));
            match braced(after) {
                Some((spec, tail)) => {
                    out.push_str(&format!("{{{}}}", simplify_spec(spec)));
                    rest = tail;
                }
                None => rest = after,
            }
        }
        out.push_str(rest);
        // \multirow{n}{w}{text} → text; rules and colours pandoc trips on.
        let mut s = out;
        while let Some(i) = s.find("\\multirow") {
            let mut t = &s[i + 9..];
            let mut parts = Vec::new();
            for _ in 0..3 {
                t = t.trim_start();
                if t.starts_with('[') {
                    t = t.find(']').map_or(t, |c| &t[c + 1..]);
                }
                match braced(t) {
                    Some((p, tail)) => {
                        parts.push(p.to_string());
                        t = tail;
                    }
                    None => break,
                }
            }
            let text = parts.last().cloned().unwrap_or_default();
            s = format!("{}{text}{}", &s[..i], t);
        }
        for cmd in ["\\cmidrule", "\\rowcolor", "\\cellcolor", "\\addlinespace"] {
            while let Some(i) = s.find(cmd) {
                let mut t = &s[i + cmd.len()..];
                if let Some(p) = t.strip_prefix('(') {
                    t = p.find(')').map_or(p, |c| &p[c + 1..]);
                }
                if let Some(p) = t.strip_prefix('[') {
                    t = p.find(']').map_or(p, |c| &p[c + 1..]);
                }
                if cmd != "\\addlinespace"
                    && let Some((_, tail)) = braced(t)
                {
                    t = tail;
                }
                s = format!("{}{}", &s[..i], t);
            }
        }
        s
    })
}

fn simplify_spec(spec: &str) -> String {
    let mut out = String::new();
    let mut chars = spec.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            // @{…}, !{…}, >{…}, <{…}: inter-column material, dropped.
            '@' | '!' | '>' | '<' => skip_group(&mut chars, '{', '}'),
            'S' => {
                if chars.peek() == Some(&'[') {
                    skip_group(&mut chars, '[', ']');
                }
                out.push('r');
            }
            'X' => out.push('l'),
            'p' | 'm' | 'b' => {
                out.push('p');
                out.push('{');
                skip_group(&mut chars, '{', '}');
                out.push_str("3cm}");
            }
            'l' | 'c' | 'r' | '|' => out.push(c),
            _ => {}
        }
    }
    out
}

fn skip_group(chars: &mut std::iter::Peekable<std::str::Chars>, open: char, close: char) {
    if chars.peek() != Some(&open) {
        return;
    }
    let mut depth = 0;
    for c in chars.by_ref() {
        if c == open {
            depth += 1;
        } else if c == close {
            depth -= 1;
            if depth == 0 {
                return;
            }
        }
    }
}

/// .bib files named by \addbibresource or \bibliography.
fn bibliographies(text: &str, root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for line in text.lines() {
        for call in crate::index::calls(line) {
            if let ("addbibresource" | "bibliography", Some(arg)) = (call.name, call.arg) {
                for name in arg.split(',') {
                    let p = root.join(name.trim());
                    let p = if p.extension().is_some() { p } else { p.with_extension("bib") };
                    if p.is_file() && !out.contains(&p) {
                        out.push(p);
                    }
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels() -> HashMap<String, Label> {
        let mut l = HashMap::new();
        parse_aux(
            "\\newlabel{fig:a}{{3.1}{12}{The pipeline}{figure.caption.5}{}}\n\
             \\newlabel{fig:a@cref}{{[figure][1][3]3.1}{[1][12][]12}}\n\
             \\newlabel{fig:b}{{3.2}{13}{Network}{figure.caption.6}{}}\n\
             \\newlabel{eq:gauss}{{2.1}{7}{Equations}{equation.2.1}{}}\n\
             \\newlabel{sec:x}{{2.3}{9}{Units}{section.2.3}{}}\n",
            &mut l,
        );
        l
    }

    #[test]
    fn reads_aux() {
        let l = labels();
        assert_eq!(l["fig:a"].number, "3.1");
        assert_eq!(l["fig:a"].kind, "figure");
        assert_eq!(l["fig:a"].page, "12");
        assert_eq!(l["eq:gauss"].kind, "equation");
    }

    #[test]
    fn replaces_references() {
        let (t, missing) = replace_refs("See \\cref{fig:a,fig:b}, \\ref{sec:x}, \\eqref{eq:gauss} and \\Cref{nope}. \\textbf{x}", &labels(), false);
        assert_eq!(t, "See Figures~3.1 and 3.2, 2.3, (2.1) and ??. \\textbf{x}");
        assert_eq!(missing, ["nope"]);
        let (t, _) = replace_refs("\\cref{eq:gauss} \\autoref{sec:x}", &labels(), true);
        assert_eq!(t, "Ecuación~(2.1) Sección~2.3");
        let (t, _) = replace_refs("\\begin{lstlisting}\n\\ref{fig:a}\n\\end{lstlisting}", &labels(), false);
        assert!(t.contains("\\ref{fig:a}"), "code is left alone");
    }

    #[test]
    fn numbers_equations() {
        let t = number_equations("\\begin{equation}\n  x = 1 \\label{eq:gauss}\n\\end{equation}\n\\label{sec:x}", &labels());
        assert_eq!(t, "\\begin{equation}\n  x = 1 \\qquad\\text{(2.1)}\n\\end{equation}\n\\label{sec:x}");
    }

    #[test]
    fn finds_outermost_environments() {
        let text = "a \\begin{tikzpicture}\\begin{axis}\\end{axis}\\begin{tikzpicture}\\end{tikzpicture}\\end{tikzpicture} b \\begin{forest}[x]\\end{forest}";
        let mut seen = Vec::new();
        let out = replace_envs(text, PICTURES, &mut |e| {
            seen.push(e.to_string());
            "IMG".into()
        });
        assert_eq!(out, "a IMG b IMG");
        assert!(seen[0].ends_with("\\end{tikzpicture}\\end{tikzpicture}"));
    }

    #[test]
    fn simplifies_tables() {
        let t = simplify_tables("\\begin{tabular}{@{}l S[table-format=2.2] >{\\bfseries}c@{}}\n\\multirow{2}{*}{CNN} & 1 \\\\ \\cmidrule(lr){2-3}\n\\end{tabular}\n\\begin{tabularx}{\\linewidth}{lX}\\end{tabularx}");
        assert!(t.starts_with("\\begin{tabular}{lrc}\nCNN & 1 \\\\ \n\\end{tabular}"), "{t}");
        assert!(t.contains("\\begin{tabular}{ll}\\end{tabular}"), "{t}");
    }

    #[test]
    fn flattens_inputs() {
        let dir = std::env::temp_dir().join(format!("rutex-flat-{}", std::process::id()));
        fs::create_dir_all(dir.join("ch")).unwrap();
        fs::write(dir.join("main.tex"), "A\n% \\input{nothing}\n\\input{ch/one} B % note\n\\begin{verbatim}\n% kept\n\\end{verbatim}\n").unwrap();
        fs::write(dir.join("ch/one.tex"), "One\n\\include{ch/two}\n").unwrap();
        fs::write(dir.join("ch/two.tex"), "Two").unwrap();
        let t = flatten(&dir, Path::new("main.tex"), 0);
        assert_eq!(t, "A\nOne\nTwo B \n\\begin{verbatim}\n% kept\n\\end{verbatim}\n");
        fs::remove_dir_all(dir).unwrap();
    }

    /// Whole pipeline on the demo (needs TeX and pandoc:
    /// RUTEX_PANDOC=/path/to/pandoc cargo test -- --ignored exports_demo).
    #[test]
    #[ignore]
    fn exports_demo() {
        let pandoc = find_pandoc().expect("pandoc");
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("demo");
        let build = std::env::temp_dir().join(format!("rutex-export-{}", std::process::id()));
        fs::create_dir_all(&build).unwrap();
        let status = crate::compiler::command("latexmk")
            .current_dir(&root)
            .args(["-pdf", "-interaction=nonstopmode", "-f"])
            .arg(format!("-outdir={}", build.display()))
            .arg("main.tex")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.code().is_some());
        let out = build.join("demo.docx");
        let t = std::time::Instant::now();
        let notes = export(&Export { pandoc: &pandoc, root: &root, main: Path::new("main.tex"), build_dir: &build, out: &out }).unwrap();
        eprintln!("exported in {:?}, notes: {notes:?}", t.elapsed());
        let unzip = std::process::Command::new("unzip").arg("-l").arg(&out).output().unwrap();
        let listing = String::from_utf8_lossy(&unzip.stdout);
        let images = listing.lines().filter(|l| l.contains("media/")).count();
        eprintln!("{images} images");
        assert!(images >= 8, "{listing}");
        let xml = std::process::Command::new("unzip").arg("-p").arg(&out).arg("word/document.xml").output().unwrap();
        let xml = String::from_utf8_lossy(&xml.stdout);
        assert!(xml.contains("Figure 3.1") || xml.contains("Figure\u{a0}3.1"), "references are numbered");
        assert!(!xml.contains("fig:pipeline]"), "no raw label names");
        if let Some(keep) = std::env::var_os("RUTEX_EXPORT_KEEP") {
            fs::copy(&out, keep).unwrap();
        }
        fs::remove_dir_all(build).unwrap();
    }
}
