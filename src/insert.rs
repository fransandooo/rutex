//! Inserting figures and tables without writing the code by hand: an image
//! file becomes a `figure` environment (copied into the project's figures
//! folder), and spreadsheet cells or a CSV file become a `table` with booktabs.

use std::fs;
use std::path::{Path, PathBuf};

use gtk::gio;

use crate::i18n::tr;

/// The Insert menu, shared by its header button and the editor's right-click menu.
pub fn menu() -> gio::Menu {
    let menu = gio::Menu::new();
    let add = |m: &gio::Menu, items: &[(&str, &str)]| {
        let s = gio::Menu::new();
        for (label, action) in items {
            s.append(Some(label), Some(action));
        }
        m.append_section(None, &s);
    };
    add(&menu, &[(tr("Image…"), "win.insert-image"), (tr("Table from CSV…"), "win.insert-csv"), (tr("Math symbol…"), "win.symbols")]);
    add(&menu, &[(tr("Reference from DOI or arXiv…"), "win.add-reference"), (tr("Citation from Zotero"), "win.zotero-cite")]);
    menu
}

pub const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "pdf", "eps", "svg", "gif", "webp"];

pub fn is_image(p: &Path) -> bool {
    IMAGE_EXTS.iter().any(|e| crate::project::has_ext(p, e))
}

/// Folder for figures: an existing usual one, the first \graphicspath entry,
/// or a new `figures/` (translated with the UI).
pub fn figures_dir(root: &Path, main_src: &str) -> PathBuf {
    for name in ["figures", "figuras", "images", "imagenes", "img", "figs", "fig"] {
        if root.join(name).is_dir() {
            return PathBuf::from(name);
        }
    }
    if let Some(i) = main_src.find("\\graphicspath{{") {
        let rest = &main_src[i + 15..];
        if let Some(end) = rest.find('}') {
            let dir = rest[..end].trim_end_matches('/');
            if !dir.is_empty() {
                return PathBuf::from(dir);
            }
        }
    }
    PathBuf::from(tr("figures"))
}

/// Copies an image into the project (unless it is already inside) and returns
/// its path relative to the root.
pub fn import_image(root: &Path, main_src: &str, file: &Path) -> std::io::Result<PathBuf> {
    if let Ok(rel) = file.strip_prefix(root) {
        return Ok(rel.to_path_buf());
    }
    let dir = figures_dir(root, main_src);
    fs::create_dir_all(root.join(&dir))?;
    let stem = sanitize(&file.file_stem().unwrap_or_default().to_string_lossy());
    let ext = file.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let mut rel = dir.join(format!("{stem}.{ext}"));
    let mut n = 1;
    // Same name, different file: add a number instead of overwriting.
    while root.join(&rel).exists() && fs::read(root.join(&rel))? != fs::read(file)? {
        rel = dir.join(format!("{stem}-{n}.{ext}"));
        n += 1;
    }
    if !root.join(&rel).exists() {
        fs::copy(file, root.join(&rel))?;
    }
    Ok(rel)
}

/// File names that work everywhere in LaTeX: no spaces or accents.
pub fn sanitize(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| match c {
            'á' | 'à' | 'ä' | 'â' => 'a',
            'é' | 'è' | 'ë' | 'ê' => 'e',
            'í' | 'ì' | 'ï' | 'î' => 'i',
            'ó' | 'ò' | 'ö' | 'ô' => 'o',
            'ú' | 'ù' | 'ü' | 'û' => 'u',
            'ñ' => 'n',
            'ç' => 'c',
            c if c.is_ascii_alphanumeric() || c == '-' || c == '_' => c.to_ascii_lowercase(),
            _ => '-',
        })
        .collect();
    let s = s.split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-");
    if s.is_empty() { "image".into() } else { s }
}

/// Whether the project loads a package (main file plus \input'd preamble files).
pub fn uses_package(root: &Path, main_src: &str, package: &str) -> bool {
    let preamble = crate::mathpreview::preamble_of(main_src);
    let mut text = preamble.clone();
    for line in preamble.lines() {
        for call in crate::index::calls(crate::project::strip_comment(line)) {
            if call.name == "input"
                && let Some(arg) = call.arg
            {
                let p = root.join(arg.trim());
                let p = if p.extension().is_some() { p } else { p.with_extension("tex") };
                text.push_str(&fs::read_to_string(p).unwrap_or_default());
            }
        }
    }
    text.lines().map(crate::project::strip_comment).any(|l| {
        crate::index::calls(l).iter().any(|c| c.name == "usepackage" && c.arg.is_some_and(|a| a.split(',').any(|p| p.trim() == package)))
    })
}

/// `figure` environment for an image. Returns the text and the caption
/// placeholder so the editor can select it.
pub fn figure(rel: &Path, indent: &str) -> (String, &'static str) {
    let path = rel.with_extension("").to_string_lossy().replace('\\', "/");
    let label = sanitize(&rel.file_stem().unwrap_or_default().to_string_lossy());
    let caption = tr("Caption");
    let text = format!(
        "\\begin{{figure}}[htbp]\n{indent}  \\centering\n{indent}  \\includegraphics[width=0.8\\linewidth]{{{path}}}\n{indent}  \\caption{{{caption}}}\n{indent}  \\label{{fig:{label}}}\n{indent}\\end{{figure}}"
    );
    (text, caption)
}

/// Rows from spreadsheet cells (tab-separated) or CSV (`,` or `;`), with
/// quoted fields. None if it does not look like a table.
pub fn parse_table(text: &str) -> Option<Vec<Vec<String>>> {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.len() < 2 {
        return None;
    }
    let delim = [('\t', true), (';', false), (',', false)]
        .into_iter()
        .find(|(d, _)| {
            let counts: Vec<usize> = lines.iter().map(|l| split_row(l, *d).len()).collect();
            counts[0] >= 2 && counts.iter().all(|c| *c == counts[0])
        })
        .map(|(d, _)| d)?;
    Some(lines.iter().map(|l| split_row(l, delim)).collect())
}

fn split_row(line: &str, delim: char) -> Vec<String> {
    let mut out = Vec::new();
    let mut cell = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                cell.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            c if c == delim && !quoted => out.push(std::mem::take(&mut cell).trim().to_string()),
            c => cell.push(c),
        }
    }
    out.push(cell.trim().to_string());
    out
}

fn escape(cell: &str) -> String {
    let mut out = String::new();
    for c in cell.chars() {
        match c {
            '&' | '%' | '$' | '#' | '_' | '{' | '}' => {
                out.push('\\');
                out.push(c);
            }
            '~' => out.push_str("\\textasciitilde{}"),
            '^' => out.push_str("\\textasciicircum{}"),
            '\\' => out.push_str("\\textbackslash{}"),
            c => out.push(c),
        }
    }
    out
}

fn is_number(cell: &str) -> bool {
    let c = cell.trim().trim_end_matches('%');
    !c.is_empty() && c.replace([',', '.', '-', '+', ' '], "").chars().all(|ch| ch.is_ascii_digit())
}

/// `table` environment; booktabs rules if the preamble loads booktabs.
/// Numeric columns are right-aligned.
pub fn table(rows: &[Vec<String>], booktabs: bool, indent: &str) -> (String, &'static str) {
    let cols = rows.iter().map(Vec::len).max().unwrap_or(0);
    let spec: String = (0..cols)
        .map(|c| if rows.iter().skip(1).all(|r| r.get(c).is_some_and(|v| is_number(v) || v.is_empty())) && rows.len() > 1 { 'r' } else { 'l' })
        .collect();
    let cells: Vec<Vec<String>> = rows.iter().map(|r| (0..cols).map(|c| escape(r.get(c).map_or("", String::as_str))).collect()).collect();
    let widths: Vec<usize> = (0..cols).map(|c| cells.iter().map(|r| r[c].chars().count()).max().unwrap_or(0)).collect();
    let row_text = |r: &Vec<String>| {
        let padded: Vec<String> = r.iter().enumerate().map(|(i, v)| format!("{v:<w$}", w = widths[i])).collect();
        format!("{indent}    {} \\\\", padded.join(" & ").trim_end())
    };
    let (top, mid, bottom) = if booktabs { ("\\toprule", "\\midrule", "\\bottomrule") } else { ("\\hline", "\\hline", "\\hline") };
    let caption = tr("Caption");
    let mut t = format!("\\begin{{table}}[htbp]\n{indent}  \\centering\n{indent}  \\caption{{{caption}}}\n{indent}  \\label{{tab:}}\n{indent}  \\begin{{tabular}}{{{spec}}}\n{indent}    {top}\n");
    if let Some(header) = cells.first() {
        t.push_str(&row_text(header));
        t.push_str(&format!("\n{indent}    {mid}\n"));
    }
    for r in cells.iter().skip(1) {
        t.push_str(&row_text(r));
        t.push('\n');
    }
    t.push_str(&format!("{indent}    {bottom}\n{indent}  \\end{{tabular}}\n{indent}\\end{{table}}"));
    (t, caption)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_spreadsheet_and_csv() {
        let tsv = "Model\tAcc\nBase\t81.2\nOurs\t92.5\n";
        assert_eq!(parse_table(tsv).unwrap(), vec![vec!["Model", "Acc"], vec!["Base", "81.2"], vec!["Ours", "92.5"]]);
        let csv = "name,comment\n\"Smith, J.\",\"said \"\"hi\"\"\"\n";
        assert_eq!(parse_table(csv).unwrap()[1], vec!["Smith, J.", "said \"hi\""]);
        assert_eq!(parse_table("a;b\n1;2").unwrap()[1], vec!["1", "2"]);
        assert!(parse_table("just one line\twith tab").is_none());
        assert!(parse_table("a,b\n1,2,3").is_none(), "inconsistent columns are not a table");
    }

    #[test]
    fn builds_table() {
        let rows = parse_table("Model\tAcc (%)\nBase\t81.2\nOurs_v2\t92.5").unwrap();
        let (t, _) = table(&rows, true, "");
        assert!(t.contains("\\begin{tabular}{lr}"), "{t}");
        assert!(t.contains("\\toprule") && t.contains("\\midrule"));
        assert!(t.contains("Ours\\_v2 & 92.5 \\\\"), "{t}");
        assert!(t.contains("Acc (\\%)"));
    }

    #[test]
    fn figure_and_names() {
        assert_eq!(sanitize("Gráfica Final (v2)"), "grafica-final-v2");
        let (f, cap) = figure(Path::new("figures/grafica-final.png"), "");
        assert!(f.contains("\\includegraphics[width=0.8\\linewidth]{figures/grafica-final}"));
        assert!(f.contains("\\label{fig:grafica-final}") && f.contains(cap));
    }
}
