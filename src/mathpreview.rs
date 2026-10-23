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

/// Changes whenever the preamble or a file it \\inputs changes.
fn preamble_fingerprint(root: &Path, preamble: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    preamble.hash(&mut h);
    for line in preamble.lines() {
        for call in crate::index::calls(strip_comment(line)) {
            if matches!(call.name, "input" | "include" | "usepackage")
                && let Some(arg) = call.arg
            {
                for name in arg.split(',') {
                    let p = root.join(name.trim());
                    for p in [p.clone(), p.with_extension("tex"), p.with_extension("sty")] {
                        if let Ok(meta) = fs::metadata(&p) {
                            (p, meta.modified().ok()).hash(&mut h);
                        }
                    }
                }
            }
        }
    }
    h.finish()
}

const MINIMAL: &str = "\\usepackage{amsmath,amssymb,mathtools,bm}\n";

pub struct MathPreview {
    root: PathBuf,
    dir: PathBuf,
    preamble: RefCell<String>,
    program: RefCell<&'static str>,
    cache: RefCell<HashMap<u64, Result<gdk::Texture, String>>>,
    fingerprint: RefCell<u64>,
}

impl MathPreview {
    pub fn new(root: PathBuf, build_dir: &Path) -> Rc<MathPreview> {
        Rc::new(MathPreview {
            root,
            dir: build_dir.join("math"),
            preamble: RefCell::default(),
            program: RefCell::new("pdflatex"),
            cache: RefCell::default(),
            fingerprint: RefCell::default(),
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
        // Precompile the preamble now, so the first hover is already fast.
        let (root, dir, pre, program) = (self.root.clone(), self.dir.clone(), self.preamble.borrow().clone(), *self.program.borrow());
        let fp = preamble_fingerprint(&root, &pre);
        if *self.fingerprint.borrow() != fp {
            self.cache.borrow_mut().clear();
            *self.fingerprint.borrow_mut() = fp;
        }
        std::thread::spawn(move || {
            let _ = fs::create_dir_all(&dir);
            preamble_format(&root, &dir, &pre, program, fp);
        });
    }

    /// Renders a formula, tinted with `color` (the theme's text color).
    /// Results are cached in memory and as PNG files in the build folder.
    pub async fn render(self: &Rc<Self>, formula: String, color: gdk::RGBA) -> Result<gdk::Texture, String> {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (&formula, color.to_string(), *self.fingerprint.borrow(), *self.program.borrow()).hash(&mut h);
        let key = h.finish();
        if let Some(r) = self.cache.borrow().get(&key) {
            return r.clone();
        }
        let png = self.dir.join(format!("{key:016x}.png"));
        let result = if png.is_file() {
            gdk::Texture::from_filename(&png).map_err(|e| e.to_string())
        } else {
            let (root, dir, preamble, program, fp) = (self.root.clone(), self.dir.clone(), self.preamble.borrow().clone(), *self.program.borrow(), *self.fingerprint.borrow());
            let rgb = (color.red() as f64, color.green() as f64, color.blue() as f64);
            let out = png.clone();
            gio::spawn_blocking(move || render_blocking(&root, &dir, &preamble, program, fp, &formula, key, rgb, &out))
                .await
                .unwrap_or_else(|_| Err("render thread failed".into()))
                .and_then(|()| gdk::Texture::from_filename(&png).map_err(|e| e.to_string()))
        };
        self.cache.borrow_mut().insert(key, result.clone());
        result
    }
}

/// Only one thread builds a given preamble format at a time.
static FORMAT_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Runs a TeX program with a time limit; true if it produced `pdf`.
fn run_tex(root: &Path, dir: &Path, program: &str, args: &[String], formats: bool) -> Result<(), String> {
    let mut cmd = crate::compiler::command(program);
    cmd.current_dir(root).env("max_print_line", "1000").args(args).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    if formats {
        // Our precompiled preamble lives in the build folder.
        cmd.env("TEXFORMATS", format!("{}:", dir.display()));
    }
    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return Ok(()),
            Ok(None) if started.elapsed() > Duration::from_secs(30) => {
                let _ = child.kill();
                return Err("timeout".into());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => return Err(e.to_string()),
        }
    }
}

/// Precompiles the preamble into a format (mylatexformat), so each formula
/// costs ~0.3 s instead of reloading every package. pdfLaTeX only; returns
/// the format name, or None to compile normally.
fn preamble_format(root: &Path, dir: &Path, preamble: &str, program: &str, fingerprint: u64) -> Option<String> {
    if program != "pdflatex" {
        return None;
    }
    let name = format!("pre-{fingerprint:016x}");
    let _guard = FORMAT_LOCK.lock().ok()?;
    if dir.join(format!("{name}.fmt")).is_file() {
        return Some(name);
    }
    // Formats are ~15 MB: keep only the current one.
    for e in fs::read_dir(dir).into_iter().flatten().flatten() {
        if e.file_name().to_string_lossy().starts_with("pre-") {
            let _ = fs::remove_file(e.path());
        }
    }
    let tex = dir.join(format!("{name}.tex"));
    fs::write(&tex, format!("\\documentclass[border=1pt,varwidth=40em]{{standalone}}\n{preamble}\n\\begin{{document}}\n\\end{{document}}\n")).ok()?;
    let args = vec!["-ini".into(), "-interaction=nonstopmode".into(), format!("-output-directory={}", dir.display()), format!("-jobname={name}"), "&pdflatex".into(), "mylatexformat.ltx".into(), tex.display().to_string()];
    run_tex(root, dir, "pdflatex", &args, false).ok()?;
    dir.join(format!("{name}.fmt")).is_file().then_some(name)
}

#[allow(clippy::too_many_arguments)]
fn render_blocking(root: &Path, dir: &Path, preamble: &str, program: &str, fingerprint: u64, formula: &str, key: u64, rgb: (f64, f64, f64), png: &Path) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let name = format!("f{key:016x}");
    let tex = dir.join(format!("{name}.tex"));
    let pdf = dir.join(format!("{name}.pdf"));
    let compile = |src: String, formats: bool| -> Result<(), String> {
        fs::write(&tex, src).map_err(|e| e.to_string())?;
        let _ = fs::remove_file(&pdf);
        let args = vec!["-interaction=nonstopmode".into(), "-halt-on-error".into(), format!("-output-directory={}", dir.display()), tex.display().to_string()];
        run_tex(root, dir, program, &args, formats)?;
        if pdf.is_file() {
            Ok(())
        } else {
            let log = fs::read_to_string(dir.join(format!("{name}.log"))).unwrap_or_default();
            Err(log.lines().find(|l| l.starts_with('!')).unwrap_or("LaTeX error").trim_start_matches("! ").to_string())
        }
    };
    let body = format!("\\begin{{document}}\n{formula}\n\\end{{document}}\n");
    let full = |pre: &str| format!("\\documentclass[border=1pt,varwidth=40em]{{standalone}}\n{pre}\n{body}");
    // 1. Precompiled preamble (fast); 2. the preamble as is; 3. a minimal one.
    let mut result = match preamble_format(root, dir, preamble, program, fingerprint) {
        Some(fmt) => compile(format!("%&{fmt}\n{body}"), true),
        None => Err(String::new()),
    };
    if result.is_err() {
        result = compile(full(preamble), false);
    }
    if result.is_err() {
        result = compile(full(&format!("{MINIMAL}{}", definitions(root, preamble))), false);
    }
    result?;

    let data = fs::read(&pdf).map_err(|e| e.to_string())?;
    let doc = poppler::Document::from_bytes(&glib::Bytes::from_owned(data), None).map_err(|e| e.to_string())?;
    let page = doc.page(0).ok_or("empty")?;
    let (pw, ph) = page.size();
    // 2 px per point, shown at half size: crisp on HiDPI screens.
    let scale = 2.0;
    let (w, h) = (((pw * scale).ceil() as i32).max(1), ((ph * scale).ceil() as i32).max(1));
    let mut surface = cairo::ImageSurface::create(cairo::Format::ARgb32, w, h).map_err(|e| e.to_string())?;
    {
        let cr = cairo::Context::new(&surface).map_err(|e| e.to_string())?;
        cr.scale(scale, scale);
        page.render(&cr);
    }
    surface.flush();
    let stride = surface.stride() as usize;
    let (x0, y0, x1, y1) = {
        let data = surface.data().map_err(|e| e.to_string())?;
        // Black ink on transparent → theme text color (premultiplied BGRA),
        // and find the ink's bounding box to crop the empty page around it.
        let mut bbox = (w, h, 0, 0);
        let mut data = data;
        for y in 0..h as usize {
            for x in 0..w as usize {
                let px = &mut data[y * stride + x * 4..y * stride + x * 4 + 4];
                let a = px[3] as f64;
                if px[3] > 8 {
                    bbox = (bbox.0.min(x as i32), bbox.1.min(y as i32), bbox.2.max(x as i32), bbox.3.max(y as i32));
                }
                px[0] = (rgb.2 * a) as u8;
                px[1] = (rgb.1 * a) as u8;
                px[2] = (rgb.0 * a) as u8;
            }
        }
        bbox
    };
    if x1 < x0 {
        return Err("empty formula".into());
    }
    let pad = 4;
    let (cx, cy) = ((x0 - pad).max(0), (y0 - pad).max(0));
    let (cw, ch) = ((x1 + pad + 1).min(w) - cx, (y1 + pad + 1).min(h) - cy);
    let cropped = cairo::ImageSurface::create(cairo::Format::ARgb32, cw, ch).map_err(|e| e.to_string())?;
    {
        let cr = cairo::Context::new(&cropped).map_err(|e| e.to_string())?;
        cr.set_source_surface(&surface, -(cx as f64), -(cy as f64)).map_err(|e| e.to_string())?;
        cr.paint().map_err(|e| e.to_string())?;
    }
    let mut file = fs::File::create(png).map_err(|e| e.to_string())?;
    cropped.write_to_png(&mut file).map_err(|e| e.to_string())?;
    for ext in ["pdf", "aux", "log"] {
        let _ = fs::remove_file(dir.join(format!("{name}.{ext}")));
    }
    Ok(())
}

/// Bubble with the rendered formula, shown at half the rendered size (the
/// render is 2× for HiDPI) and never wider than the editor can comfortably show.
pub fn bubble(texture: &gdk::Texture) -> gtk::Widget {
    let (w, h) = (texture.width() / 2, texture.height() / 2);
    let max_w = 620;
    let (w, h) = if w > max_w { (max_w, h * max_w / w) } else { (w, h) };
    let picture = gtk::Picture::builder().paintable(texture).can_shrink(true).content_fit(gtk::ContentFit::Contain).halign(gtk::Align::Center).build();
    picture.set_size_request(w, h);
    let b = gtk::Box::builder().css_classes(["bubble", "math-bubble"]).halign(gtk::Align::Center).build();
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
        let png = dir.join("out.png");
        for (i, f) in ["$\\norm{x}^2 + \\R$", "$\\int_0^1 f$", "\\begin{align*}a &= b\\\\ c &= d\\end{align*}"].iter().enumerate() {
            let t = Instant::now();
            render_blocking(&root, &dir, &pre, "pdflatex", 7, f, i as u64, (1.0, 1.0, 1.0), &png).unwrap();
            let img = cairo::ImageSurface::create_from_png(&mut fs::File::open(&png).unwrap()).unwrap();
            eprintln!("{f}: {}x{} px in {:?}", img.width(), img.height(), t.elapsed());
            assert!(img.width() < 600, "cropped to the ink, not the 40em page");
        }
        let err = render_blocking(&root, &dir, &pre, "pdflatex", 7, "$\\frac{1}{$", 9, (1.0, 1.0, 1.0), &png).unwrap_err();
        eprintln!("error: {err}");
        fs::remove_dir_all(dir).ok();
    }
}
