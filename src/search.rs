//! Find and replace across the project's text files (plain text, optionally
//! case-sensitive and whole-word). Blocking; runs in a worker thread.

use std::fs;
use std::path::{Path, PathBuf};

const MAX_HITS: usize = 2000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    pub file: PathBuf,
    /// 1-based line.
    pub line: u32,
    /// Match position and length in characters within `text`.
    pub col: usize,
    pub len: usize,
    pub text: String,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Options {
    pub case_sensitive: bool,
    pub whole_word: bool,
}

fn fold(c: char, opts: Options) -> char {
    if opts.case_sensitive { c } else { c.to_lowercase().next().unwrap_or(c) }
}

/// Character positions where `needle` occurs in `hay`.
pub fn find_in(hay: &[char], needle: &[char], opts: Options) -> Vec<usize> {
    if needle.is_empty() || needle.len() > hay.len() {
        return Vec::new();
    }
    let word = |c: Option<&char>| c.is_some_and(|c| c.is_alphanumeric() || *c == '_');
    let mut out = Vec::new();
    let mut i = 0;
    while i + needle.len() <= hay.len() {
        let matches = hay[i..i + needle.len()].iter().zip(needle).all(|(a, b)| fold(*a, opts) == fold(*b, opts));
        let bounded = !opts.whole_word || (!word(i.checked_sub(1).and_then(|j| hay.get(j))) && !word(hay.get(i + needle.len())));
        if matches && bounded {
            out.push(i);
            i += needle.len();
        } else {
            i += 1;
        }
    }
    out
}

pub fn search(root: &Path, files: &[PathBuf], query: &str, opts: Options) -> Vec<Hit> {
    let needle: Vec<char> = query.chars().collect();
    let mut hits = Vec::new();
    for rel in files {
        let Ok(src) = fs::read_to_string(root.join(rel)) else { continue };
        for (n, line) in src.lines().enumerate() {
            let chars: Vec<char> = line.chars().collect();
            for col in find_in(&chars, &needle, opts) {
                // Keep long lines readable: show a window around the match.
                let start = col.saturating_sub(40);
                let text: String = chars[start..(col + needle.len() + 80).min(chars.len())].iter().collect();
                hits.push(Hit { file: rel.clone(), line: n as u32 + 1, col: col - start, len: needle.len(), text });
                if hits.len() >= MAX_HITS {
                    return hits;
                }
            }
        }
    }
    hits
}

/// Replaces every occurrence in a text; returns the new text and how many.
pub fn replace_in(src: &str, query: &str, replacement: &str, opts: Options) -> (String, usize) {
    let needle: Vec<char> = query.chars().collect();
    let mut out = String::with_capacity(src.len());
    let mut count = 0;
    for (i, line) in src.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let chars: Vec<char> = line.chars().collect();
        let mut last = 0;
        for col in find_in(&chars, &needle, opts) {
            out.extend(&chars[last..col]);
            out.push_str(replacement);
            last = col + needle.len();
            count += 1;
        }
        out.extend(&chars[last..]);
    }
    (out, count)
}

/// Replaces in every file; returns (occurrences, files changed).
pub fn replace_all(root: &Path, files: &[PathBuf], query: &str, replacement: &str, opts: Options) -> std::io::Result<(usize, usize)> {
    let (mut total, mut changed) = (0, 0);
    for rel in files {
        let path = root.join(rel);
        let Ok(src) = fs::read_to_string(&path) else { continue };
        let (new, n) = replace_in(&src, query, replacement, opts);
        if n > 0 {
            fs::write(&path, new)?;
            total += n;
            changed += 1;
        }
    }
    Ok((total, changed))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos(h: &str, n: &str, o: Options) -> Vec<usize> {
        find_in(&h.chars().collect::<Vec<_>>(), &n.chars().collect::<Vec<_>>(), o)
    }

    #[test]
    fn finds_with_options() {
        let any = Options::default();
        assert_eq!(pos("Figura y figura", "figura", any), vec![0, 9]);
        assert_eq!(pos("Figura y figura", "figura", Options { case_sensitive: true, ..any }), vec![9]);
        assert_eq!(pos("red reddish red", "red", Options { whole_word: true, ..any }), vec![0, 12]);
        assert_eq!(pos("ñandú y Ñandú", "ñandú", any), vec![0, 8]);
        assert!(pos("abc", "", any).is_empty());
    }

    #[test]
    fn replaces_preserving_lines() {
        let (s, n) = replace_in("uno dos\nDos tres\n", "dos", "2", Options::default());
        assert_eq!((s.as_str(), n), ("uno 2\n2 tres\n", 2));
        let (s, n) = replace_in("cat category cat", "cat", "dog", Options { whole_word: true, ..Options::default() });
        assert_eq!((s.as_str(), n), ("dog category dog", 2));
    }
}
