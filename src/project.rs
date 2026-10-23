//! Descubrimiento del proyecto: raíz, archivo principal, archivos editables y
//! carpeta de compilación (fuera del proyecto, en ~/.cache/rutex).

use std::fs;
use std::path::{Component, Path, PathBuf};

const EDITABLE: &[&str] = &[
    "tex", "bib", "sty", "cls", "bst", "bbx", "cbx", "cfg", "def", "ltx", "dtx", "txt", "md",
];
const SKIP_DIRS: &[&str] = &["node_modules", "target", "__pycache__", "_minted"];
const MAX_DEPTH: usize = 8;
const MAX_FILES: usize = 5000;
const PREFERRED_MAIN: &[&str] = &[
    "main.tex", "tfm.tex", "tfg.tex", "tesis.tex", "thesis.tex", "memoria.tex", "paper.tex",
];


#[derive(Clone, Debug)]
pub struct Project {
    pub root: PathBuf,
    /// Archivo principal, relativo a `root`.
    pub main: PathBuf,
    /// Archivos editables, relativos a `root`, ordenados.
    pub files: Vec<PathBuf>,
    pub build_dir: PathBuf,
}

impl Project {
    /// Abre una carpeta o un archivo .tex. Devuelve el proyecto y el archivo a
    /// mostrar primero. Si la carpeta no tiene ningún .tex y `create` es true,
    /// crea `main.tex` con una plantilla; si es false, falla.
    pub fn open(path: &Path, main_override: Option<PathBuf>, create: bool) -> std::io::Result<(Project, PathBuf)> {
        let path = fs::canonicalize(path)?;
        let (root, requested) = if path.is_dir() {
            (path, None)
        } else {
            let root = tex_root_of(&path).unwrap_or_else(|| path.parent().unwrap().to_path_buf());
            let rel = path.strip_prefix(&root).map(Path::to_path_buf).ok();
            (root, rel)
        };

        let mut files = scan(&root);
        if !files.iter().any(|f| has_ext(f, "tex")) {
            if !create {
                return Err(std::io::Error::other(crate::i18n::tr("The folder has no .tex files")));
            }
            let title = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            crate::templates::write(&crate::templates::TEMPLATES[0], &root, &title)?;
            files = scan(&root);
        }

        let main = main_override
            .filter(|m| root.join(m).is_file())
            .or_else(|| requested.clone().filter(|r| is_document(&root.join(r))))
            .or_else(|| find_main(&root, &files))
            .unwrap_or_else(|| files.iter().find(|f| has_ext(f, "tex")).cloned().unwrap());

        let first = requested.unwrap_or_else(|| main.clone());
        let build_dir = build_dir_for(&root);
        Ok((Project { root, main, files, build_dir }, first))
    }

    pub fn rescan(&mut self) {
        self.files = scan(&self.root);
    }

    pub fn name(&self) -> String {
        self.root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    }

    pub fn pdf_path(&self) -> PathBuf {
        self.build_dir.join(self.main.with_extension("pdf").file_name().unwrap())
    }

    pub fn log_path(&self) -> PathBuf {
        self.build_dir.join(self.main.with_extension("log").file_name().unwrap())
    }

    /// Convierte una ruta tal y como la escribe TeX/SyncTeX ("./cap/intro.tex",
    /// "/abs/proyecto/./cap/intro.tex") en una ruta relativa a la raíz.
    pub fn relativize(&self, raw: &str) -> Option<PathBuf> {
        let p = Path::new(raw.trim());
        let joined = if p.is_absolute() { p.to_path_buf() } else { self.root.join(p) };
        let clean = normalize(&joined);
        let rel = clean.strip_prefix(&self.root).ok()?.to_path_buf();
        if rel.as_os_str().is_empty() { None } else { Some(rel) }
    }
}

const TEXT_EXTRA: &[&str] = &[
    "csv", "tsv", "dat", "json", "yaml", "yml", "toml", "ini", "xml", "py", "r", "m", "jl", "lua", "sh", "tikz",
    "pgf", "asy", "gp", "gnuplot", "html", "css", "js", "log", "bbl", "ist", "glo",
];

/// ¿Se puede abrir en el editor? Por extensión; sin extensión, si es UTF-8 y pequeño.
pub fn is_text(abs: &Path) -> bool {
    match abs.extension() {
        Some(_) => EDITABLE.iter().chain(TEXT_EXTRA).any(|e| has_ext(abs, e)),
        None => {
            let Ok(meta) = fs::metadata(abs) else { return false };
            if meta.len() > 1_000_000 {
                return false;
            }
            fs::read(abs).is_ok_and(|b| std::str::from_utf8(&b).is_ok() && !b.contains(&0))
        }
    }
}

const IMAGES: &[&str] = &["png", "jpg", "jpeg", "pdf", "eps", "svg", "gif", "webp"];

/// Imágenes del proyecto (para `\includegraphics`), relativas a la raíz.
pub fn images(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let name = entry.file_name();
            if name.to_string_lossy().starts_with('.') {
                continue;
            }
            let path = entry.path();
            if entry.file_type().is_ok_and(|t| t.is_dir()) {
                if depth < MAX_DEPTH && !SKIP_DIRS.contains(&name.to_string_lossy().as_ref()) {
                    stack.push((path, depth + 1));
                }
            } else if IMAGES.iter().any(|e| has_ext(&path, e))
                && let Ok(rel) = path.strip_prefix(root)
            {
                out.push(rel.to_path_buf());
            }
        }
        if out.len() > MAX_FILES {
            break;
        }
    }
    out.sort();
    out
}

pub fn has_ext(p: &Path, ext: &str) -> bool {
    p.extension().is_some_and(|e| e.eq_ignore_ascii_case(ext))
}

fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

fn scan(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        if out.len() >= MAX_FILES {
            break;
        }
        let Ok(entries) = fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with('.') {
                continue;
            }
            let Ok(ft) = entry.file_type() else { continue };
            let path = entry.path();
            if ft.is_dir() {
                if depth < MAX_DEPTH && !SKIP_DIRS.contains(&name.as_ref()) {
                    stack.push((path, depth + 1));
                }
            } else if EDITABLE.iter().any(|e| has_ext(&path, e)) {
                if let Ok(rel) = path.strip_prefix(root) {
                    out.push(rel.to_path_buf());
                }
                if out.len() >= MAX_FILES {
                    break;
                }
            }
        }
    }
    // Primero los archivos de la raíz, luego por carpetas.
    out.sort_by(|a, b| {
        let da = a.components().count();
        let db = b.components().count();
        (da > 1, a).cmp(&(db > 1, b))
    });
    out
}

/// ¿Contiene `\documentclass` fuera de un comentario?
fn is_document(path: &Path) -> bool {
    let Ok(src) = fs::read_to_string(path) else { return false };
    src.lines().any(|l| strip_comment(l).contains("\\documentclass"))
}

fn find_main(root: &Path, files: &[PathBuf]) -> Option<PathBuf> {
    let docs: Vec<&PathBuf> = files
        .iter()
        .filter(|f| has_ext(f, "tex") && f.components().count() <= 2 && is_document(&root.join(f)))
        .collect();
    for pref in PREFERRED_MAIN {
        if let Some(f) = docs.iter().find(|f| f.as_os_str().eq_ignore_ascii_case(pref)) {
            return Some((*f).clone());
        }
    }
    docs.into_iter().min_by_key(|f| (f.components().count(), (*f).clone())).cloned()
}

/// Sigue un comentario mágico `% !TEX root = ../main.tex` en las primeras líneas.
fn tex_root_of(file: &Path) -> Option<PathBuf> {
    let src = fs::read_to_string(file).ok()?;
    let target = magic_comment(&src, "root")?;
    let main = fs::canonicalize(file.parent()?.join(target)).ok()?;
    main.parent().map(Path::to_path_buf)
}

/// Lee `% !TEX <key> = valor` de las primeras 30 líneas.
pub fn magic_comment(src: &str, key: &str) -> Option<String> {
    src.lines().take(30).find_map(|l| {
        let l = l.trim_start();
        let rest = l.strip_prefix('%')?.trim_start();
        let rest = rest.strip_prefix('!')?.trim_start();
        let (tex, rest) = rest.split_at(rest.find(char::is_whitespace)?);
        if !tex.eq_ignore_ascii_case("tex") {
            return None;
        }
        let (k, v) = rest.split_once('=')?;
        let k = k.trim().to_ascii_lowercase();
        let k = k.strip_prefix("ts-").unwrap_or(&k);
        (k == key).then(|| v.trim().to_string())
    })
}

pub fn strip_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'%' && (i == 0 || bytes[i - 1] != b'\\') {
            return &line[..i];
        }
    }
    line
}

fn build_dir_for(root: &Path) -> PathBuf {
    // FNV-1a: estable entre versiones de Rust (DefaultHasher no lo es).
    let mut h: u64 = 0xcbf29ce484222325;
    for b in root.as_os_str().as_encoded_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    let name: String = root
        .file_name()
        .map(|n| n.to_string_lossy().chars().filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_').collect())
        .unwrap_or_default();
    gtk::glib::user_cache_dir().join("rutex").join(format!("{name}-{h:016x}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn magic_comments() {
        let src = "% !TEX program = xelatex\n%!TeX root=../main.tex\n\\documentclass{article}";
        assert_eq!(magic_comment(src, "program").as_deref(), Some("xelatex"));
        assert_eq!(magic_comment(src, "root").as_deref(), Some("../main.tex"));
        assert_eq!(magic_comment("% !TEX TS-program = lualatex", "program").as_deref(), Some("lualatex"));
        assert_eq!(magic_comment("\\documentclass{article}", "program"), None);
    }

    #[test]
    fn comments_are_stripped() {
        assert_eq!(strip_comment("a % b"), "a ");
        assert_eq!(strip_comment(r"50\% de % nota"), r"50\% de ");
        assert_eq!(strip_comment("% \\documentclass"), "");
    }

    #[test]
    fn relativize_paths() {
        let p = Project {
            root: "/home/u/tfm".into(),
            main: "main.tex".into(),
            files: vec![],
            build_dir: "/tmp".into(),
        };
        assert_eq!(p.relativize("./cap/intro.tex"), Some("cap/intro.tex".into()));
        assert_eq!(p.relativize("/home/u/tfm/./cap/intro.tex"), Some("cap/intro.tex".into()));
        assert_eq!(p.relativize("/usr/share/texmf/tex/latex/base/article.cls"), None);
    }
}
