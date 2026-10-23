//! "File `x.sty' not found": which package provides it. Uses TeX Live's own
//! package database (texlive.tlpdb, installed with TeX Live), so it works
//! offline and without root. On Arch, every TeX Live collection is a pacman
//! package: collection-latexextra → texlive-latexextra.

use std::path::Path;

const TLPDB: &str = "/usr/share/tlpkg/texlive.tlpdb";

/// Missing files mentioned in log messages (`File `x.sty' not found`,
/// `I can't find file `x.tex'`).
pub fn missing_files<'a>(messages: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for msg in messages {
        let Some(start) = msg.find("File `").map(|i| i + 6).or_else(|| msg.find("find file `").map(|i| i + 11)) else { continue };
        let Some(len) = msg[start..].find('\'') else { continue };
        let name = msg[start..start + len].trim();
        if !name.is_empty() && (msg.contains("not found") || msg.contains("can't find")) && !out.iter().any(|n| n == name) {
            out.push(name.to_string());
        }
    }
    out
}

#[derive(Debug, PartialEq, Eq)]
pub struct Provider {
    /// TeX Live package (e.g. "simplekv").
    pub package: String,
    /// TeX Live collection that includes it (e.g. "collection-plaingeneric").
    pub collection: Option<String>,
}

impl Provider {
    /// Arch package that installs it ("texlive-plaingeneric").
    pub fn arch_package(&self) -> Option<String> {
        self.collection.as_deref()?.strip_prefix("collection-").map(|c| format!("texlive-{c}"))
    }
}

/// Finds the TeX Live package shipping `file` (by file name), or None if no
/// package has it (usually a typo in \usepackage).
pub fn lookup(file: &str) -> Option<Provider> {
    let db = std::fs::read_to_string(TLPDB).ok()?;
    lookup_in(&db, file)
}

pub fn available() -> bool {
    Path::new(TLPDB).is_file()
}

fn lookup_in(db: &str, file: &str) -> Option<Provider> {
    // Without an extension TeX tries .tex.
    let wanted = if file.contains('.') { file.to_string() } else { format!("{file}.tex") };
    let suffix = format!("/{wanted}");
    let mut current = "";
    let mut package = None;
    for line in db.lines() {
        if let Some(name) = line.strip_prefix("name ") {
            current = name;
        } else if line.starts_with(' ') && line.split_whitespace().next().is_some_and(|f| f.ends_with(&suffix)) {
            // Packages named like "foo.x86_64-linux" hold binaries; skip them.
            if !current.contains('.') {
                package = Some(current.to_string());
                break;
            }
        }
    }
    let package = package?;
    let depend = format!("depend {package}");
    let mut collection = None;
    for line in db.lines() {
        if let Some(name) = line.strip_prefix("name ") {
            current = name;
        } else if line == depend && current.starts_with("collection-") {
            collection = Some(current.to_string());
            break;
        }
    }
    Some(Provider { package, collection })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_missing_files_in_log() {
        let msgs = [
            "LaTeX Error: File `noexiste.sty' not found. — \\usepackage",
            "! I can't find file `simplekv.tex'.",
            "LaTeX Warning: Reference `x' undefined",
        ];
        assert_eq!(missing_files(msgs), ["noexiste.sty", "simplekv.tex"]);
    }

    #[test]
    fn looks_up_packages() {
        let db = "name collection-plaingeneric\ndepend simplekv\n\nname simplekv\nrunfiles size=3\n texmf-dist/tex/generic/simplekv/simplekv.tex\n\nname foo.x86_64-linux\nbinfiles\n bin/x86_64-linux/simplekv.tex\n";
        let p = lookup_in(db, "simplekv.tex").unwrap();
        assert_eq!(p, Provider { package: "simplekv".into(), collection: Some("collection-plaingeneric".into()) });
        assert_eq!(p.arch_package().as_deref(), Some("texlive-plaingeneric"));
        assert_eq!(lookup_in(db, "nothere.sty"), None);
    }

    /// Against the real database, when TeX Live is installed.
    #[test]
    fn real_tlpdb() {
        if !available() {
            return;
        }
        let p = lookup("fontawesome5.sty").unwrap();
        assert_eq!(p.arch_package().as_deref(), Some("texlive-fontsextra"));
        assert!(lookup("definitely-not-a-package-xyz.sty").is_none());
    }
}
