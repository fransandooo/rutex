//! Spell checking (libspelling over enchant/hunspell). The language is taken
//! from the document (babel / polyglossia), so a Spanish thesis is checked in
//! Spanish and an English paper in English, whatever the UI language is.

use crate::project::strip_comment;

/// babel/polyglossia language names → dictionary codes.
const LANGUAGES: &[(&str, &str)] = &[
    ("english", "en_US"),
    ("american", "en_US"),
    ("USenglish", "en_US"),
    ("british", "en_GB"),
    ("UKenglish", "en_GB"),
    ("spanish", "es_ES"),
    ("catalan", "ca_ES"),
    ("galician", "gl_ES"),
    ("basque", "eu_ES"),
    ("french", "fr_FR"),
    ("german", "de_DE"),
    ("ngerman", "de_DE"),
    ("italian", "it_IT"),
    ("portuguese", "pt_PT"),
    ("brazilian", "pt_BR"),
    ("dutch", "nl_NL"),
];

/// Dictionary code for the document's main language, if it declares one.
/// With babel the main language is the last option; polyglossia uses
/// `\setmainlanguage` / `\setdefaultlanguage`.
pub fn document_language(main_src: &str) -> Option<&'static str> {
    let mut found = None;
    for line in main_src.lines().map(strip_comment) {
        for call in crate::index::calls(line) {
            match (call.name, call.arg) {
                ("setmainlanguage" | "setdefaultlanguage", Some(lang)) => {
                    return code_for(lang.trim());
                }
                ("usepackage", Some(pkg)) if pkg.trim() == "babel" => {
                    if let Some(opts) = babel_options(line) {
                        // `main=X` wins; otherwise the last language option is the main one.
                        let main = opts.split(',').find_map(|o| o.trim().strip_prefix("main=").and_then(|l| code_for(l.trim())));
                        found = main.or_else(|| opts.split(',').rev().find_map(|o| code_for(o.trim()))).or(found);
                    }
                }
                ("documentclass", _) => {
                    // \documentclass[spanish]{article} also selects babel's language.
                    if let Some(opts) = bracket(line, "\\documentclass") {
                        found = opts.split(',').rev().find_map(|o| code_for(o.trim())).or(found);
                    }
                }
                _ => {}
            }
        }
    }
    found
}

fn code_for(name: &str) -> Option<&'static str> {
    LANGUAGES.iter().find(|(n, _)| *n == name).map(|(_, c)| *c)
}

/// Options of `\usepackage[...]{babel}` on a line.
fn babel_options(line: &str) -> Option<&str> {
    let at = line.find("{babel}")?;
    let head = &line[..at];
    let open = head.rfind('[')?;
    let close = head.rfind(']')?;
    (close > open).then(|| &head[open + 1..close])
}

fn bracket<'a>(line: &'a str, command: &str) -> Option<&'a str> {
    let rest = &line[line.find(command)? + command.len()..];
    let rest = rest.trim_start().strip_prefix('[')?;
    Some(&rest[..rest.find(']')?])
}

/// Language to use: the user's choice, or the document's, or the UI's.
pub fn resolve(setting: &str, main_src: &str) -> String {
    if setting != "auto" {
        return setting.to_string();
    }
    document_language(main_src)
        .unwrap_or(if crate::i18n::is_spanish() { "es_ES" } else { "en_US" })
        .to_string()
}

/// Uses the closest dictionary that is installed (es_ES → es, en_GB → en_US…).
pub fn installed(checker: &libspelling::Checker, wanted: &str) -> Option<String> {
    let provider = checker.provider();
    let base = wanted.split('_').next().unwrap_or(wanted);
    [wanted.to_string(), base.to_string(), format!("{base}_US"), format!("{base}_ES")]
        .into_iter()
        .find(|code| provider.supports_language(code))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_document_language() {
        assert_eq!(document_language("\\usepackage[spanish,es-tabla]{babel}"), Some("es_ES"));
        assert_eq!(document_language("\\usepackage[english,spanish]{babel}"), Some("es_ES"));
        assert_eq!(document_language("\\usepackage[main=english,spanish]{babel}"), Some("en_US"));
        assert_eq!(document_language("\\documentclass[12pt,british]{article}\n\\usepackage{babel}"), Some("en_GB"));
        assert_eq!(document_language("\\usepackage{polyglossia}\n\\setmainlanguage{french}"), Some("fr_FR"));
        assert_eq!(document_language("% \\usepackage[spanish]{babel}\n\\usepackage[T1]{fontenc}"), None);
    }

    #[test]
    fn manual_setting_wins() {
        assert_eq!(resolve("en_GB", "\\usepackage[spanish]{babel}"), "en_GB");
        assert_eq!(resolve("auto", "\\usepackage[spanish]{babel}"), "es_ES");
    }
}
