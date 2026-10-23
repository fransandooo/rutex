//! Adding references without typing BibTeX: from a DOI or an arXiv id (via
//! doi.org content negotiation and arxiv.org/bibtex, using curl), or picked in
//! Zotero through Better BibTeX's "cite as you write" endpoint.

use std::fs;
use std::path::{Path, PathBuf};

use crate::compiler;

#[derive(Debug, PartialEq, Eq)]
pub enum Ident {
    Doi(String),
    Arxiv(String),
}

/// Accepts "10.1145/…", "doi:10…", "https://doi.org/10…", "1706.03762",
/// "arXiv:1706.03762v2", "https://arxiv.org/abs/1706.03762".
pub fn parse_identifier(input: &str) -> Option<Ident> {
    let s = input.trim().trim_end_matches('/');
    let lower = s.to_ascii_lowercase();
    if let Some(i) = lower.find("10.")
        && lower[i..].contains('/')
        && !lower.contains("arxiv.org")
    {
        return Some(Ident::Doi(s[i..].to_string()));
    }
    let id = lower
        .rsplit(['/', ':'])
        .next()
        .unwrap_or("")
        .trim_end_matches(".pdf")
        .to_string();
    let base = id.split('v').next().unwrap_or("");
    let modern = base.split('.').collect::<Vec<_>>();
    if modern.len() == 2 && modern[0].len() == 4 && modern[0].chars().all(|c| c.is_ascii_digit()) && modern[1].chars().all(|c| c.is_ascii_digit()) {
        return Some(Ident::Arxiv(id));
    }
    None
}

/// Downloads the BibTeX of an identifier (blocking).
pub fn fetch(ident: &Ident) -> Result<String, String> {
    let (url, accept) = match ident {
        Ident::Doi(d) => (format!("https://doi.org/{d}"), "application/x-bibtex"),
        Ident::Arxiv(a) => (format!("https://arxiv.org/bibtex/{a}"), "text/plain"),
    };
    let out = compiler::command("curl")
        .args(["-sSL", "--fail", "--max-time", "20", "-H"])
        .arg(format!("Accept: {accept}"))
        .arg(&url)
        .output()
        .map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !out.status.success() || !text.starts_with('@') {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(if err.is_empty() { "not found".into() } else { err });
    }
    Ok(text)
}

/// Asks Zotero (with Better BibTeX) for references: Zotero shows its quick
/// search, and the selected items come back as BibLaTeX.
pub fn zotero_cayw() -> Result<String, String> {
    let out = compiler::command("curl")
        .args(["-sS", "--fail", "--max-time", "300"])
        .arg("http://127.0.0.1:23119/better-bibtex/cayw?format=translate&translator=Better%20BibLaTeX")
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Splits a .bib text into entries (each starting with '@').
pub fn split_entries(bib: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut current = String::new();
    for c in bib.chars() {
        if c == '@' && depth == 0 && !current.trim().is_empty() {
            out.push(std::mem::take(&mut current).trim().to_string());
        }
        if c == '@' && depth == 0 {
            current.clear();
        }
        match c {
            '{' | '(' => depth += 1,
            '}' | ')' => depth -= 1,
            _ => {}
        }
        current.push(c);
    }
    if current.trim().starts_with('@') {
        out.push(current.trim().to_string());
    }
    out
}

/// Top-level fields of one entry: (type, key, [(name, value)]).
fn fields(entry: &str) -> Option<(String, String, Vec<(String, String)>)> {
    let entry = entry.trim();
    let open = entry.find(['{', '('])?;
    let kind = entry[1..open].trim().to_string();
    let body = &entry[open + 1..entry.len().saturating_sub(1)];
    let comma = body.find(',')?;
    let key = body[..comma].trim().to_string();
    let mut out = Vec::new();
    let (mut depth, mut quote, mut start) = (0, false, comma + 1);
    let bytes = body.as_bytes();
    let mut push = |s: &str| {
        if let Some((n, v)) = s.split_once('=') {
            let n = n.trim().to_ascii_lowercase();
            if !n.is_empty() {
                out.push((n, v.trim().to_string()));
            }
        }
    };
    for (i, &b) in bytes.iter().enumerate().skip(comma + 1) {
        match b {
            b'{' => depth += 1,
            b'}' => depth -= 1,
            b'"' if depth == 0 => quote = !quote,
            b',' if depth == 0 && !quote => {
                push(&body[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    push(&body[start..]);
    Some((kind, key, out))
}

fn unbrace(v: &str) -> String {
    v.trim().trim_matches(['{', '}', '"']).to_string()
}

/// Better-BibTeX-like key: first author's surname + year + first title word.
pub fn make_key(fields: &[(String, String)]) -> String {
    let get = |n: &str| fields.iter().find(|(k, _)| k == n).map(|(_, v)| unbrace(v)).unwrap_or_default();
    let author = get("author");
    let first = author.split(" and ").next().unwrap_or("");
    let surname = match first.split_once(',') {
        Some((last, _)) => last.trim().to_string(),
        None => first.split_whitespace().last().unwrap_or("").to_string(),
    };
    let year: String = get("year").chars().filter(char::is_ascii_digit).take(4).collect();
    let word = get("title")
        .split(|c: char| !c.is_alphanumeric())
        .map(str::to_lowercase)
        .find(|w| w.len() > 2 && !["the", "and", "for", "with", "from", "una", "los", "las"].contains(&w.as_str()))
        .unwrap_or_default();
    let key = format!("{}{year}{word}", crate::insert::sanitize(&surname).replace('-', ""));
    if key.is_empty() { "ref".into() } else { key }
}

/// Reformats one entry (one field per line) with a readable key that does not
/// clash with `existing`. Returns (key, text, doi).
pub fn prepare(entry: &str, existing: &[String]) -> Option<(String, String, Option<String>)> {
    let (kind, _, fields) = fields(entry)?;
    let base = make_key(&fields);
    let mut key = base.clone();
    let mut suffix = b'b';
    while existing.contains(&key) {
        key = format!("{base}{}", suffix as char);
        suffix += 1;
    }
    let width = fields.iter().map(|(n, _)| n.len()).max().unwrap_or(0);
    let mut text = format!("@{}{{{key},\n", kind.to_ascii_lowercase());
    for (n, v) in &fields {
        let v = if v.starts_with('{') || v.starts_with('"') || v.chars().all(|c| c.is_ascii_digit()) { v.clone() } else { format!("{{{v}}}") };
        text.push_str(&format!("  {n:<width$} = {v},\n"));
    }
    text.push('}');
    let doi = fields.iter().find(|(n, _)| n == "doi").map(|(_, v)| unbrace(v).to_ascii_lowercase());
    Some((key, text, doi))
}

/// Key of an existing entry with this DOI, so it is not added twice.
pub fn existing_by_doi(bib: &str, doi: &str) -> Option<String> {
    split_entries(bib).into_iter().find_map(|e| {
        let (_, key, f) = fields(&e)?;
        f.iter().any(|(n, v)| n == "doi" && unbrace(v).eq_ignore_ascii_case(doi)).then_some(key)
    })
}

/// The .bib the document uses (\addbibresource / \bibliography), else the
/// first one in the project, else a new references.bib.
pub fn target_bib(root: &Path, main_src: &str, files: &[PathBuf]) -> (PathBuf, bool) {
    for line in main_src.lines().map(crate::project::strip_comment) {
        for call in crate::index::calls(line) {
            if let ("addbibresource" | "bibliography", Some(arg)) = (call.name, call.arg) {
                let first = arg.split(',').next().unwrap_or("").trim();
                if !first.is_empty() {
                    let p = PathBuf::from(first);
                    let p = if p.extension().is_some() { p } else { p.with_extension("bib") };
                    return (p, false);
                }
            }
        }
    }
    if let Some(f) = files.iter().find(|f| crate::project::has_ext(f, "bib")) {
        return (f.clone(), false);
    }
    (PathBuf::from(format!("{}.bib", crate::i18n::tr("references"))), !root.join("references.bib").exists())
}

/// Appends entries to the .bib; returns the keys to cite (existing ones reused).
pub fn add_to_bib(root: &Path, bib_rel: &Path, bibtex: &str) -> std::io::Result<Vec<String>> {
    let path = root.join(bib_rel);
    let mut current = fs::read_to_string(&path).unwrap_or_default();
    let mut existing: Vec<String> = crate::index::parse_bib(&current).into_iter().map(|e| e.key).collect();
    let mut keys = Vec::new();
    for entry in split_entries(bibtex) {
        let Some((key, text, doi)) = prepare(&entry, &existing) else { continue };
        if let Some(old) = doi.as_deref().and_then(|d| existing_by_doi(&current, d)) {
            keys.push(old);
            continue;
        }
        if !current.is_empty() && !current.ends_with("\n\n") {
            current.push_str(if current.ends_with('\n') { "\n" } else { "\n\n" });
        }
        current.push_str(&text);
        current.push('\n');
        existing.push(key.clone());
        keys.push(key);
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, current)?;
    Ok(keys)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_identifiers() {
        assert_eq!(parse_identifier("10.1145/361604.361612"), Some(Ident::Doi("10.1145/361604.361612".into())));
        assert_eq!(parse_identifier("https://doi.org/10.1145/361604.361612"), Some(Ident::Doi("10.1145/361604.361612".into())));
        assert_eq!(parse_identifier("doi:10.1000/xyz"), Some(Ident::Doi("10.1000/xyz".into())));
        assert_eq!(parse_identifier("arXiv:1706.03762v5"), Some(Ident::Arxiv("1706.03762v5".into())));
        assert_eq!(parse_identifier("https://arxiv.org/abs/1706.03762"), Some(Ident::Arxiv("1706.03762".into())));
        assert_eq!(parse_identifier("https://arxiv.org/pdf/2301.00001.pdf"), Some(Ident::Arxiv("2301.00001".into())));
        assert_eq!(parse_identifier("hello"), None);
    }

    #[test]
    fn prepares_entries() {
        let doi = " @article{Knuth_1974, title={Computer programming as an art}, volume={17}, DOI={10.1145/361604.361612}, author={Knuth, Donald E.}, year={1974}, month=Dec, pages={667–673} }";
        let (key, text, d) = prepare(doi, &[]).unwrap();
        assert_eq!(key, "knuth1974computer");
        assert!(text.starts_with("@article{knuth1974computer,\n"), "{text}");
        assert!(text.contains("  title  = {Computer programming as an art},") && text.contains("month  = {Dec}"), "{text}");
        assert_eq!(d.as_deref(), Some("10.1145/361604.361612"));
        let (key2, _, _) = prepare(doi, &["knuth1974computer".into()]).unwrap();
        assert_eq!(key2, "knuth1974computerb");
        let arxiv = "@misc{vaswani2023attentionneed,\n title={Attention Is All You Need},\n author={Ashish Vaswani and Noam Shazeer},\n year={2023},\n}";
        assert_eq!(prepare(arxiv, &[]).unwrap().0, "vaswani2023attention");
    }

    #[test]
    fn adds_once_per_doi() {
        let dir = std::env::temp_dir().join(format!("rutex-refs-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let e = "@article{X, title={Computer programming as an art}, DOI={10.1145/361604.361612}, author={Knuth, Donald E.}, year={1974}}";
        assert_eq!(add_to_bib(&dir, Path::new("refs.bib"), e).unwrap(), ["knuth1974computer"]);
        assert_eq!(add_to_bib(&dir, Path::new("refs.bib"), e).unwrap(), ["knuth1974computer"], "same DOI: reuse the key");
        assert_eq!(crate::index::parse_bib(&fs::read_to_string(dir.join("refs.bib")).unwrap()).len(), 1);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn finds_target_bib() {
        let (p, _) = target_bib(Path::new("/x"), "\\addbibresource{refs/main.bib}", &[]);
        assert_eq!(p, PathBuf::from("refs/main.bib"));
        let (p, _) = target_bib(Path::new("/x"), "\\bibliography{tesis}", &[]);
        assert_eq!(p, PathBuf::from("tesis.bib"));
    }

    /// Real requests (needs internet): `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn fetches_real_references() {
        let doi = fetch(&parse_identifier("https://doi.org/10.1145/361604.361612").unwrap()).unwrap();
        assert_eq!(prepare(&doi, &[]).unwrap().0, "knuth1974computer");
        let arxiv = fetch(&parse_identifier("arXiv:1706.03762").unwrap()).unwrap();
        let (key, text, _) = prepare(&arxiv, &[]).unwrap();
        eprintln!("{text}");
        assert_eq!(key, "vaswani2023attention");
    }
}
