//! Parser del .log de LaTeX. Compilamos con `-file-line-error` y
//! `max_print_line` alto, así que los errores llegan como `archivo:línea: mensaje`
//! en una sola línea. Para los avisos seguimos la pila de archivos abiertos
//! (los paréntesis del log), igual que hace Overleaf.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issue {
    pub severity: Severity,
    /// Ruta tal y como aparece en el log (sin normalizar).
    pub file: Option<String>,
    pub line: Option<u32>,
    pub message: String,
}

pub fn parse(log: &str) -> Vec<Issue> {
    let lines: Vec<&str> = log.lines().collect();
    let mut issues: Vec<Issue> = Vec::new();
    let mut stack: Vec<Option<String>> = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];

        if let Some((file, ln, msg)) = file_line_error(line) {
            let mut message = msg.to_string();
            if let Some(ctx) = context(&lines[i + 1..]) {
                message.push_str(" — ");
                message.push_str(&ctx);
            }
            push(&mut issues, Issue { severity: Severity::Error, file: Some(file.into()), line: Some(ln), message });
        } else if let Some(msg) = line.strip_prefix("! ") {
            let (ln, ctx) = l_number(&lines[i + 1..]);
            let mut message = msg.trim().to_string();
            if let Some(ctx) = ctx {
                message.push_str(" — ");
                message.push_str(&ctx);
            }
            let file = stack.iter().rev().find_map(|f| f.clone());
            push(&mut issues, Issue { severity: Severity::Error, file, line: ln, message });
        } else if let Some(start) = warning_start(line) {
            let mut message = line[start..].trim().to_string();
            // Continuaciones: "(paquete)    texto…"
            let mut j = i + 1;
            while j < lines.len() && is_continuation(lines[j]) {
                message.push(' ');
                message.push_str(lines[j].split_once(')').map_or("", |x| x.1).trim());
                j += 1;
            }
            let ln = input_line(&message);
            let file = stack.iter().rev().find_map(|f| f.clone());
            push(&mut issues, Issue { severity: Severity::Warning, file, line: ln, message });
        }

        track_files(line, &mut stack);
        i += 1;
    }
    issues
}

fn push(issues: &mut Vec<Issue>, issue: Issue) {
    if !issues.contains(&issue) {
        issues.push(issue);
    }
}

/// `./cap/intro.tex:12: Undefined control sequence.`
fn file_line_error(line: &str) -> Option<(&str, u32, &str)> {
    let bytes = line.as_bytes();
    let mut search = 0;
    while let Some(off) = line[search..].find(':') {
        let colon = search + off;
        let digits_end = bytes[colon + 1..].iter().take_while(|b| b.is_ascii_digit()).count() + colon + 1;
        if digits_end > colon + 1 && bytes.get(digits_end) == Some(&b':') {
            let file = &line[..colon];
            let looks_like_file = file.starts_with('.') || file.starts_with('/') || has_tex_ext(file);
            if !file.is_empty() && looks_like_file {
                let ln = line[colon + 1..digits_end].parse().ok()?;
                return Some((file, ln, line[digits_end + 1..].trim()));
            }
            return None;
        }
        search = colon + 1;
    }
    None
}

fn has_tex_ext(file: &str) -> bool {
    [".tex", ".sty", ".cls", ".bib", ".bbl", ".cfg", ".def", ".ltx"].iter().any(|e| file.ends_with(e))
}

/// Busca `l.12 \comando` en las líneas siguientes a un error.
fn l_number(rest: &[&str]) -> (Option<u32>, Option<String>) {
    for l in rest.iter().take(12) {
        if let Some(after) = l.strip_prefix("l.") {
            let digits: String = after.chars().take_while(char::is_ascii_digit).collect();
            let ctx = after[digits.len()..].trim();
            let ctx = (!ctx.is_empty()).then(|| shorten(ctx));
            return (digits.parse().ok(), ctx);
        }
    }
    (None, None)
}

fn context(rest: &[&str]) -> Option<String> {
    l_number(rest).1
}

fn shorten(s: &str) -> String {
    const MAX: usize = 80;
    if s.chars().count() <= MAX {
        s.to_string()
    } else {
        let tail: String = s.chars().rev().take(MAX).collect::<Vec<_>>().into_iter().rev().collect();
        format!("…{tail}")
    }
}

fn warning_start(line: &str) -> Option<usize> {
    let head = &line[..line.find(" Warning: ")?];
    ["LaTeX", "Package ", "Class ", "pdfTeX", "biblatex"].iter().filter_map(|p| head.rfind(p)).min()
}

fn is_continuation(line: &str) -> bool {
    let Some(rest) = line.strip_prefix('(') else { return false };
    let Some((name, after)) = rest.split_once(')') else { return false };
    !name.is_empty() && !name.contains(char::is_whitespace) && !name.contains('/') && after.starts_with("  ")
}

fn input_line(message: &str) -> Option<u32> {
    let idx = message.find("input line ")?;
    message[idx + 11..].chars().take_while(char::is_ascii_digit).collect::<String>().parse().ok()
}

/// Mantiene la pila de archivos abiertos según los paréntesis del log.
fn track_files(line: &str, stack: &mut Vec<Option<String>>) {
    let chars = line.char_indices();
    for (i, c) in chars {
        match c {
            '(' => {
                let rest = &line[i + 1..];
                let end = rest.find(|c: char| c.is_whitespace() || c == '(' || c == ')').unwrap_or(rest.len());
                let token = &rest[..end];
                let is_file = token.len() > 2
                    && (token.starts_with('/') || token.starts_with("./") || token.starts_with("../"))
                    && token.rsplit_once('.').is_some_and(|(_, ext)| (1..=4).contains(&ext.len()) && ext.chars().all(|c| c.is_ascii_alphanumeric()));
                stack.push(is_file.then(|| token.to_string()));
            }
            ')' => {
                stack.pop();
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOG: &str = r"This is pdfTeX, Version 3.141592653-2.6-1.40.27 (TeX Live 2026/Arch Linux) (preloaded format=pdflatex)
(./main.tex
LaTeX2e <2025-11-01>
(/usr/share/texmf-dist/tex/latex/base/article.cls
Document Class: article 2024/06/29 v1.4n Standard LaTeX document class
(/usr/share/texmf-dist/tex/latex/base/size12.clo))
(./capitulos/intro.tex
./capitulos/intro.tex:7: Undefined control sequence.
l.7 Esto es \foo
                 bar
LaTeX Warning: Reference `fig:x' on page 1 undefined on input line 9.

)
Package natbib Warning: Citation `knuth' on page 2 undefined on input line 30.

./main.tex:20: LaTeX Error: File `noexiste.sty' not found.
Type X to quit or <RETURN> to proceed,
l.20 \usepackage{noexiste}
                          ^^M
Package hyperref Warning: Token not allowed in a PDF string (Unicode):
(hyperref)                removing `math shift' on input line 12.

)
! Emergency stop.
<*> main.tex
";

    #[test]
    fn parses_errors_and_warnings() {
        let issues = parse(LOG);
        let errors: Vec<_> = issues.iter().filter(|i| i.severity == Severity::Error).collect();
        let warnings: Vec<_> = issues.iter().filter(|i| i.severity == Severity::Warning).collect();

        assert_eq!(errors[0].file.as_deref(), Some("./capitulos/intro.tex"));
        assert_eq!(errors[0].line, Some(7));
        assert!(errors[0].message.starts_with("Undefined control sequence."));
        assert!(errors[0].message.contains("\\foo"));

        assert_eq!(errors[1].file.as_deref(), Some("./main.tex"));
        assert_eq!(errors[1].line, Some(20));
        assert!(errors[1].message.contains("noexiste.sty"));

        assert_eq!(errors[2].message, "Emergency stop.");

        assert_eq!(warnings[0].file.as_deref(), Some("./capitulos/intro.tex"));
        assert_eq!(warnings[0].line, Some(9));
        assert!(warnings[0].message.starts_with("LaTeX Warning: Reference"));

        assert_eq!(warnings[1].file.as_deref(), Some("./main.tex"));
        assert_eq!(warnings[1].line, Some(30));
        assert!(warnings[1].message.starts_with("Package natbib Warning"));

        assert_eq!(warnings[2].line, Some(12));
        assert!(warnings[2].message.contains("removing `math shift'"));
    }

    #[test]
    fn file_line_error_requires_a_path() {
        assert_eq!(file_line_error("./a.tex:3: Boom"), Some(("./a.tex", 3, "Boom")));
        assert_eq!(file_line_error("Note: 12: no"), None);
        assert_eq!(file_line_error("plain text"), None);
    }
}
