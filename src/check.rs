//! "Check before submitting": everything worth fixing before handing in a
//! thesis, in one list. Undefined references come from LaTeX's own log (it
//! knows every label, including ones set through options like `label=`), and
//! unused labels / bibliography entries come from the project index.

use std::collections::HashSet;
use std::path::PathBuf;

use crate::index::Index;

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub text: String,
    pub file: Option<PathBuf>,
    pub line: Option<u32>,
}

#[derive(Debug)]
pub struct Group {
    /// English title (translated by the UI).
    pub title: &'static str,
    pub icon: &'static str,
    pub items: Vec<Item>,
}

/// A log message with its location: (message, file, line).
pub type Logged<'a> = (&'a str, Option<PathBuf>, Option<u32>);

/// Key inside "Reference `fig:x' on page 1 undefined…" / "Citation 'knuth' …".
fn undefined_key(message: &str) -> Option<String> {
    if !message.contains("undefined") || !(message.contains("Reference") || message.contains("Citation")) {
        return None;
    }
    let start = message.find(['`', '\''])? + 1;
    let len = message[start..].find('\'')?;
    Some(message[start..start + len].to_string())
}

pub fn build(index: &Index, warnings: &[Logged], badboxes: &[Logged]) -> Vec<Group> {
    let mut groups = Vec::new();

    // Undefined \ref / \cite: they print as "??" or [?] in the PDF.
    let mut seen = HashSet::new();
    let undefined: Vec<Item> = warnings
        .iter()
        .filter_map(|(msg, file, line)| {
            let key = undefined_key(msg)?;
            seen.insert((key.clone(), *line)).then(|| Item { text: key, file: file.clone(), line: *line })
        })
        .collect();
    groups.push(Group { title: "Undefined references and citations", icon: "dialog-error-symbolic", items: undefined });

    let used_refs: HashSet<&str> = index.refs_used.iter().map(|r| r.text.as_str()).collect();
    let unused_labels = index
        .labels
        .iter()
        .filter(|l| !used_refs.contains(l.name.as_str()))
        .map(|l| Item { text: l.name.clone(), file: Some(l.file.clone()), line: Some(l.line) })
        .collect();
    groups.push(Group { title: "Labels never referenced", icon: "mail-attachment-symbolic", items: unused_labels });

    let dup = index.duplicate_labels.iter().map(|l| Item { text: l.name.clone(), file: Some(l.file.clone()), line: Some(l.line) }).collect();
    groups.push(Group { title: "Labels defined more than once", icon: "edit-copy-symbolic", items: dup });

    if !index.nocite_all {
        let cited: HashSet<&str> = index.cites_used.iter().map(|c| c.text.as_str()).collect();
        let unused = index
            .bib
            .iter()
            .filter(|e| !cited.contains(e.key.as_str()))
            .map(|e| Item { text: if e.title.is_empty() { e.key.clone() } else { format!("{} — {}", e.key, e.title) }, file: Some(e.file.clone()), line: Some(e.line) })
            .collect();
        groups.push(Group { title: "Bibliography entries never cited", icon: "x-office-address-book-symbolic", items: unused });
    }

    // Worst first: "Overfull \hbox (12.5pt)".
    let mut boxes: Vec<(f64, Item)> = badboxes
        .iter()
        .map(|(msg, file, line)| {
            let pts = msg.split('(').nth(1).and_then(|p| p.split("pt").next()).and_then(|p| p.parse().ok()).unwrap_or(0.0);
            (pts, Item { text: msg.to_string(), file: file.clone(), line: *line })
        })
        .collect();
    boxes.sort_by(|a, b| b.0.total_cmp(&a.0));
    groups.push(Group { title: "Text sticking into the margin", icon: "format-justify-fill-symbolic", items: boxes.into_iter().map(|(_, i)| i).collect() });

    let todos = index.todos.iter().map(|t| Item { text: t.text.clone(), file: Some(t.file.clone()), line: Some(t.line) }).collect();
    groups.push(Group { title: "To-do notes", icon: "task-due-symbolic", items: todos });

    groups
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::{BibEntry, Label, Located};

    #[test]
    fn builds_report() {
        let p = |s: &str| PathBuf::from(s);
        let index = Index {
            labels: vec![
                Label { name: "fig:a".into(), file: p("a.tex"), line: 3 },
                Label { name: "fig:unused".into(), file: p("a.tex"), line: 9 },
            ],
            refs_used: vec![Located { text: "fig:a".into(), file: p("a.tex"), line: 5 }],
            cites_used: vec![Located { text: "knuth".into(), file: p("a.tex"), line: 6 }],
            bib: vec![
                BibEntry { key: "knuth".into(), title: "TeX".into(), author: String::new(), year: String::new(), file: p("r.bib"), line: 1 },
                BibEntry { key: "lamport".into(), title: "LaTeX".into(), author: String::new(), year: String::new(), file: p("r.bib"), line: 7 },
            ],
            todos: vec![Located { text: "TODO: cite more".into(), file: p("a.tex"), line: 2 }],
            ..Default::default()
        };
        let warnings = [
            ("LaTeX Warning: Reference `fig:missing' on page 1 undefined on input line 12.", Some(p("a.tex")), Some(12)),
            ("LaTeX Warning: There were undefined references.", None, None),
        ];
        let badboxes = [("Overfull \\hbox (3.0pt)", Some(p("a.tex")), Some(4)), ("Overfull \\hbox (25.1pt)", Some(p("a.tex")), Some(8))];
        let groups = build(&index, &warnings, &badboxes);
        let get = |t: &str| groups.iter().find(|g| g.title == t).unwrap();
        assert_eq!(get("Undefined references and citations").items.iter().map(|i| i.text.as_str()).collect::<Vec<_>>(), ["fig:missing"]);
        assert_eq!(get("Labels never referenced").items[0].text, "fig:unused");
        assert_eq!(get("Bibliography entries never cited").items[0].text, "lamport — LaTeX");
        assert_eq!(get("Text sticking into the margin").items[0].line, Some(8), "worst first");
        assert_eq!(get("To-do notes").items.len(), 1);
    }

    #[test]
    fn nocite_star_hides_unused_entries() {
        let index = Index { nocite_all: true, ..Default::default() };
        assert!(build(&index, &[], &[]).iter().all(|g| g.title != "Bibliography entries never cited"));
    }
}
