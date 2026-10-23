//! Rendered preview of the formula under the mouse. The formula is compiled
//! alone with the project's preamble (so the user's own macros work), with a
//! minimal fallback preamble when the real one does not work outside the
//! document class. Results are cached per formula.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk::prelude::*;
use gtk::{cairo, gdk, gio, glib};

use crate::project::strip_comment;

const MATH_ENVS: &[&str] = &[
    "equation", "equation*", "align", "align*", "gather", "gather*", "multline", "multline*", "flalign", "flalign*",
    "alignat", "alignat*", "eqnarray", "eqnarray*", "displaymath", "math",
];

/// The formula around a character offset of `text`, ready to compile
/// (`$…$`, `\[…\]` or `\begin{align*}…\end{align*}`).
pub fn math_at(text: &str, offset: usize) -> Option<String> {
    let byte = text.char_indices().nth(offset).map_or(text.len(), |(i, _)| i);
    environment_at(text, byte).or_else(|| delimited_at(text, byte))
}

/// Inside `\begin{env}…\end{env}` for a math environment (searching a bounded window).
fn environment_at(text: &str, byte: usize) -> Option<String> {
    let window_start = text[..byte].char_indices().rev().nth(4000).map_or(0, |(i, _)| i);
    let before = &text[window_start..byte];
    let begin_rel = before.rfind("\\begin{")?;
    let begin = window_start + begin_rel;
    let name_start = begin + "\\begin{".len();
    let name_end = name_start + text[name_start..].find('}')?;
    let env = &text[name_start..name_end];
    if !MATH_ENVS.contains(&env) {
        return None;
    }
    let end_tag = format!("\\end{{{env}}}");
    // The cursor must not be past an \end of the same environment.
    if before[begin_rel..].contains(&end_tag) {
        return None;
    }
    let end = byte + text[byte..].find(&end_tag)?;
    let body = remove_labels(&text[name_end + 1..end]);
    // Starred version: no equation numbers in the preview.
    let shown = match env {
        "math" | "displaymath" => return Some(format!("\\[{body}\\]")),
        e if e.ends_with('*') || e == "eqnarray" => e.to_string(),
        e => format!("{e}*"),
    };
    Some(format!("\\begin{{{shown}}}{body}\\end{{{shown}}}"))
}

/// Inside `$…$`, `$$…$$`, `\(…\)` or `\[…\]` (display ones may span lines).
fn delimited_at(text: &str, byte: usize) -> Option<String> {
    for (open, close) in [("\\[", "\\]"), ("\\(", "\\)"), ("$$", "$$")] {
        let window_start = text[..byte].char_indices().rev().nth(3000).map_or(0, |(i, _)| i);
        if let Some(rel) = text[window_start..byte].rfind(open) {
            let start = window_start + rel + open.len();
            // No closing delimiter between the opening and the cursor.
            if text[start..byte].contains(close) {
                continue;
            }
            if let Some(len) = text[byte..].find(close) {
                let body = &text[start..byte + len];
                let (o, c) = if open == "\\(" { ("$", "$") } else { ("\\[", "\\]") };
                return Some(format!("{o}{}{c}", remove_labels(body)));
            }
        }
    }
    // Inline $…$ on the current line, counting unescaped dollars.
    let line_start = text[..byte].rfind('\n').map_or(0, |i| i + 1);
    let line_end = byte + text[byte..].find('\n').unwrap_or(text.len() - byte);
    let line = strip_comment(&text[line_start..line_end]);
    let col = byte - line_start;
    if col > line.len() {
        return None;
    }
    let dollars: Vec<usize> = line.char_indices().filter(|(i, c)| *c == '$' && (*i == 0 || line.as_bytes()[i - 1] != b'\\')).map(|(i, _)| i).collect();
    for pair in dollars.chunks(2) {
        if let [a, b] = pair
            && *a < col
            && col <= *b
        {
            return Some(format!("${}$", &line[a + 1..*b]));
        }
    }
    None
}

fn remove_labels(body: &str) -> String {
    let mut out = String::new();
    let mut rest = body;
    while let Some(i) = rest.find("\\label{") {
        out.push_str(&rest[..i]);
        let after = &rest[i + 7..];
        rest = after.find('}').map_or("", |j| &after[j + 1..]);
    }
    out.push_str(rest);
    out
}

/// Preamble of the main file (between \documentclass and \begin{document}),
/// without the \documentclass line.
pub fn preamble_of(main_src: &str) -> String {
    let end = main_src.find("\\begin{document}").unwrap_or(main_src.len());
    main_src[..end].lines().filter(|l| !strip_comment(l).contains("\\documentclass")).collect::<Vec<_>>().join("\n")
}

/// Macro definitions from a preamble, following one level of \input, for the
/// fallback preamble.
fn definitions(root: &Path, preamble: &str) -> String {
    let mut text = preamble.to_string();
    for line in preamble.lines() {
        for call in crate::index::calls(strip_comment(line)) {
            if call.name == "input"
                && let Some(arg) = call.arg
            {
                let p = root.join(arg.trim());
                let p = if p.extension().is_some() { p } else { p.with_extension("tex") };
                if let Ok(src) = fs::read_to_string(p) {
                    text.push('\n');
                    text.push_str(&src);
                }
            }
        }
    }
    text.lines()
        .map(strip_comment)
        .filter(|l| ["\\newcommand", "\\renewcommand", "\\providecommand", "\\DeclareMathOperator", "\\def\\"].iter().any(|k| l.trim_start().starts_with(k)))
        .collect::<Vec<_>>()
        .join("\n")
}

const MINIMAL: &str = "\\usepackage{amsmath,amssymb,mathtools,bm}\n";

pub struct MathPreview {
    root: PathBuf,
    dir: PathBuf,
    preamble: RefCell<String>,
    program: RefCell<&'static str>,
    cache: RefCell<HashMap<u64, Result<gdk::Texture, String>>>,
}

impl MathPreview {
    pub fn new(root: PathBuf, build_dir: &Path) -> Rc<MathPreview> {
        Rc::new(MathPreview {
            root,
            dir: build_dir.join("math"),
            preamble: RefCell::default(),
            program: RefCell::new("pdflatex"),
            cache: RefCell::default(),
        })
    }

    /// Updates the preamble and engine from the main file (after each save).
    pub fn set_main(&self, main_src: &str) {
        let pre = preamble_of(main_src);
        if *self.preamble.borrow() != pre {
            self.cache.borrow_mut().clear();
            *self.preamble.borrow_mut() = pre;
        }
        *self.program.borrow_mut() = match crate::compiler::Engine::detect(&self.root, main_src) {
            crate::compiler::Engine::XeLatex => "xelatex",
            crate::compiler::Engine::LuaLatex => "lualatex",
            _ => "pdflatex",
        };
    }

    /// Renders a formula, tinted with `color` (the theme's text color).
    pub async fn render(self: &Rc<Self>, formula: String, color: gdk::RGBA) -> Result<gdk::Texture, String> {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (&formula, color.to_string(), &*self.preamble.borrow()).hash(&mut h);
        let key = h.finish();
        if let Some(r) = self.cache.borrow().get(&key) {
            return r.clone();
        }
        let (root, dir, preamble, program) = (self.root.clone(), self.dir.clone(), self.preamble.borrow().clone(), *self.program.borrow());
        let rgb = (color.red() as f64, color.green() as f64, color.blue() as f64);
        let pixels = gio::spawn_blocking(move || render_blocking(&root, &dir, &preamble, program, &formula, key, rgb))
            .await
            .unwrap_or_else(|_| Err("render thread failed".into()));
        let result = pixels.map(|(w, h, stride, data)| {
            gdk::MemoryTexture::new(w, h, gdk::MemoryFormat::B8g8r8a8Premultiplied, &glib::Bytes::from_owned(data), stride as usize).upcast()
        });
        self.cache.borrow_mut().insert(key, result.clone());
        result
    }
}

type Pixels = (i32, i32, i32, Vec<u8>);

fn render_blocking(root: &Path, dir: &Path, preamble: &str, program: &str, formula: &str, key: u64, rgb: (f64, f64, f64)) -> Result<Pixels, String> {
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let name = format!("f{key:016x}");
    let attempt = |pre: &str| -> Result<PathBuf, String> {
        let tex = dir.join(format!("{name}.tex"));
        let src = format!("\\documentclass[preview,border=1pt,varwidth=40em]{{standalone}}\n{pre}\n\\begin{{document}}\n{formula}\n\\end{{document}}\n");
        fs::write(&tex, src).map_err(|e| e.to_string())?;
        let mut child = crate::compiler::command(program)
            .current_dir(root)
            .env("max_print_line", "1000")
            .args(["-interaction=nonstopmode", "-halt-on-error"])
            .arg(format!("-output-directory={}", dir.display()))
            .arg(&tex)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| e.to_string())?;
        // A broken preamble must not hang the hover.
        let started = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if started.elapsed() > Duration::from_secs(20) => {
                    let _ = child.kill();
                    return Err("timeout".into());
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(30)),
                Err(e) => return Err(e.to_string()),
            }
        }
        let pdf = dir.join(format!("{name}.pdf"));
        if pdf.is_file() {
            Ok(pdf)
        } else {
            let log = fs::read_to_string(dir.join(format!("{name}.log"))).unwrap_or_default();
            Err(log.lines().find(|l| l.starts_with('!')).unwrap_or("LaTeX error").trim_start_matches("! ").to_string())
        }
    };
    let pdf = attempt(preamble).or_else(|_| attempt(&format!("{MINIMAL}{}", definitions(root, preamble))))?;
    let data = fs::read(&pdf).map_err(|e| e.to_string())?;
    let doc = poppler::Document::from_bytes(&glib::Bytes::from_owned(data), None).map_err(|e| e.to_string())?;
    let page = doc.page(0).ok_or("empty")?;
    let (pw, ph) = page.size();
    // 2.4 px per point: crisp on HiDPI, shown at half size.
    let scale = 2.4;
    let (w, h) = (((pw * scale).ceil() as i32).max(1), ((ph * scale).ceil() as i32).max(1));
    let mut surface = cairo::ImageSurface::create(cairo::Format::ARgb32, w, h).map_err(|e| e.to_string())?;
    {
        let cr = cairo::Context::new(&surface).map_err(|e| e.to_string())?;
        cr.scale(scale, scale);
        page.render(&cr);
    }
    surface.flush();
    let stride = surface.stride();
    let mut data = surface.data().map_err(|e| e.to_string())?.to_vec();
    // Black ink on transparent → theme text color (premultiplied BGRA).
    for px in data.chunks_exact_mut(4) {
        let a = px[3] as f64;
        px[0] = (rgb.2 * a) as u8;
        px[1] = (rgb.1 * a) as u8;
        px[2] = (rgb.0 * a) as u8;
    }
    let _ = fs::remove_file(&pdf);
    Ok((w, h, stride, data))
}

/// Bubble with the rendered formula (shown at half the rendered size).
pub fn bubble(texture: &gdk::Texture) -> gtk::Widget {
    let picture = gtk::Picture::builder().paintable(texture).can_shrink(false).build();
    let (w, h) = (texture.width() / 2, texture.height() / 2);
    picture.set_size_request(w.min(900), h.min(600));
    picture.set_content_fit(gtk::ContentFit::Contain);
    let b = gtk::Box::builder().css_classes(["math-bubble"]).build();
    b.append(&picture);
    b.upcast()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str, marker: &str) -> Option<String> {
        math_at(text, text[..text.find(marker).unwrap()].chars().count())
    }

    #[test]
    fn finds_formulas() {
        assert_eq!(at("Sea $x^2 + y$ un número", "x^2"), Some("$x^2 + y$".into()));
        assert_eq!(at("Sea $x$ y $z$", "z$"), Some("$z$".into()));
        assert_eq!(at("Sea $x$ y luego texto", "luego"), None);
        assert_eq!(at("cuesta 5\\$ y $a$", "5"), None);
        assert_eq!(at("a \\[ \\int f \\] b", "int"), Some("\\[ \\int f \\]".into()));
        assert_eq!(at("a \\( \\alpha \\) b", "alpha"), Some("$ \\alpha $".into()));
        let env = "\\begin{equation}\n  e^{i\\pi} = -1 \\label{eq:e}\n\\end{equation}\ntext";
        assert_eq!(at(env, "pi"), Some("\\begin{equation*}\n  e^{i\\pi} = -1 \n\\end{equation*}".into()));
        assert_eq!(at(env, "text"), None);
        assert_eq!(at("\\begin{itemize}\\item a\\end{itemize}", "item a"), None);
    }

    #[test]
    fn preamble_and_definitions() {
        let main = "\\documentclass{report}\n\\usepackage{amsmath}\n\\newcommand{\\R}{\\mathbb{R}}\n\\begin{document}\nx";
        let pre = preamble_of(main);
        assert!(!pre.contains("documentclass") && pre.contains("\\newcommand{\\R}"));
        assert_eq!(definitions(Path::new("/nonexistent"), &pre), "\\newcommand{\\R}{\\mathbb{R}}");
    }

    /// Renders with the demo's real preamble (needs TeX Live): `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn renders_with_project_preamble() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("demo");
        let dir = std::env::temp_dir().join(format!("rutex-math-{}", std::process::id()));
        let pre = preamble_of(&fs::read_to_string(root.join("main.tex")).unwrap());
        let t = Instant::now();
        let (w, h, _, data) = render_blocking(&root, &dir, &pre, "pdflatex", "$\\norm{x}^2 + \\R$", 1, (1.0, 1.0, 1.0)).unwrap();
        eprintln!("rendered {w}x{h} in {:?}", t.elapsed());
        assert!(w > 20 && h > 10);
        assert!(data.chunks_exact(4).any(|p| p[3] > 200), "there is ink");
        let err = render_blocking(&root, &dir, &pre, "pdflatex", "$\\frac{1}{$", 2, (1.0, 1.0, 1.0)).unwrap_err();
        eprintln!("error: {err}");
        fs::remove_dir_all(dir).ok();
    }
}
