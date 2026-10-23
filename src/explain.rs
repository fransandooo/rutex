//! Plain-language explanations of LaTeX's most common errors and warnings,
//! with how to fix them. Offline: shown in the error bubble and the problems
//! list.

use crate::i18n::tr;

/// (pattern found in the message, what it means, how to fix it)
const KNOWN: &[(&str, &str, &str)] = &[
    (
        "Undefined control sequence",
        "A command on this line does not exist: a typo, or it comes from a package that is not loaded.",
        "Check the spelling of the command. If it belongs to a package (\\mathbb needs amssymb, \\SI needs siunitx…), add \\usepackage{…} to the preamble.",
    ),
    (
        "Missing $ inserted",
        "Something that only works in math (^, _, \\alpha, \\frac…) is outside a formula, or a formula is not closed.",
        "Put the math between $…$. For a literal underscore or caret in text write \\_ or \\^{}.",
    ),
    (
        "Missing } inserted",
        "Braces do not match: a { was never closed.",
        "Count the { and } on this line and the ones above; every { needs its }.",
    ),
    (
        "Missing { inserted",
        "Braces do not match: LaTeX expected a { here.",
        "Check the braces around this point, especially in \\frac{}{}, ^{} and _{}.",
    ),
    (
        "Extra }, or forgotten",
        "There is a } without its {, or an environment or $ closed in the wrong place.",
        "Look for a stray } on this line, or a \\begin{…} / $ that is closed before its contents end.",
    ),
    (
        "Too many }'s",
        "There is a } with no matching {.",
        "Remove the extra } or add the missing { before it.",
    ),
    (
        "Runaway argument",
        "An argument was never closed, so LaTeX kept reading until a blank line or the end of the file.",
        "A } is missing, usually a few lines above the reported one. Check the last \\textbf{, \\section{, \\caption{… you opened.",
    ),
    (
        "Paragraph ended before",
        "A command's argument contains a blank line, which is not allowed there; often a missing }.",
        "Close the argument with } before the blank line, or remove the blank line inside it.",
    ),
    (
        "File ended while scanning",
        "The file ended while LaTeX was still reading an argument: a } is missing somewhere.",
        "Look for an unclosed { in the last command you edited.",
    ),
    (
        "Environment",
        "The environment in \\begin{…} does not exist: a typo, or its package is missing.",
        "Check the name. Some need a package: align needs amsmath, subfigure needs subcaption, lstlisting needs listings.",
    ),
    (
        "ended by \\end{",
        "Environments are closed in the wrong order, or one is missing its \\end.",
        "Every \\begin{x} needs its \\end{x}, closed in reverse order of opening.",
    ),
    (
        "not found",
        "A file is missing: a package that is not installed, or one of your files with a wrong path.",
        "If it is a package, install it (rutex suggests which one above the editor). If it is your file, check the path, relative to the main file.",
    ),
    (
        "I can't find file",
        "A file is missing: a package that is not installed, or one of your files with a wrong path.",
        "If it is a package, install it (rutex suggests which one above the editor). If it is your file, check the path, relative to the main file.",
    ),
    (
        "Misplaced alignment tab character &",
        "& only works in tables and aligned equations.",
        "For an ampersand in text write \\&.",
    ),
    (
        "There's no line here to end",
        "\\\\ was used where there is no line to break, such as after a blank line or right after a heading.",
        "Remove the \\\\. For space between paragraphs use a blank line or \\vspace{…}.",
    ),
    (
        "Double superscript",
        "Two ^ in a row (x^a^b) are ambiguous.",
        "Group them with braces: x^{ab} or {x^a}^b.",
    ),
    (
        "Double subscript",
        "Two _ in a row (x_a_b) are ambiguous.",
        "Group them with braces: x_{ab} or {x_a}_b.",
    ),
    (
        "Illegal unit of measure",
        "A length is missing its unit.",
        "Write the unit: 2cm, 10pt, 0.5\\linewidth.",
    ),
    (
        "Missing number, treated as zero",
        "LaTeX expected a number or a length. Often a line after \\\\ starts with [ and is read as an option.",
        "If the next line starts with [, write \\\\ \\relax or put the bracket in braces: {[}.",
    ),
    (
        "Unicode character",
        "A character that pdfLaTeX's fonts cannot show (an emoji, Chinese, some symbols).",
        "Replace it with a LaTeX command, or compile with XeLaTeX or LuaLaTeX by adding % !TEX program = xelatex at the top of the main file.",
    ),
    (
        "Option clash for package",
        "A package is loaded twice with different options (sometimes once by the class or another package).",
        "Load it once with all its options, or use \\PassOptionsToPackage{…}{…} before \\documentclass.",
    ),
    (
        "Can be used only in preamble",
        "\\usepackage or a similar command appears after \\begin{document}.",
        "Move it above \\begin{document}.",
    ),
    (
        "Not in outer par mode",
        "A figure or table is inside something that cannot hold floats (a minipage, a box, a footnote, another float).",
        "Move it out, or inside a minipage use \\captionof{figure}{…} from the caption package instead of a figure environment.",
    ),
    (
        "Float(s) lost",
        "A figure or table is inside something that cannot hold floats.",
        "Move the figure or table environment out of the minipage, box or footnote.",
    ),
    (
        "Too many unprocessed floats",
        "Too many figures and tables are waiting for a place on a page.",
        "Add \\clearpage after a few figures, or use more flexible placements like [htbp].",
    ),
    (
        "illegal in command argument",
        "\\verb cannot be used inside another command's argument (a caption, a footnote…).",
        "Use \\texttt{…} there, escaping special characters (\\_, \\#).",
    ),
    (
        "already defined",
        "\\newcommand or \\newenvironment uses a name that already exists.",
        "Choose another name, or use \\renewcommand if you really want to replace it.",
    ),
    (
        "Lonely \\item",
        "\\item is outside a list.",
        "Put the items inside \\begin{itemize} … \\end{itemize} (or enumerate).",
    ),
    (
        "perhaps a missing \\item",
        "Text appears inside a list before the first \\item, or a list is empty.",
        "Start every list with \\item, and do not leave a list environment empty.",
    ),
    (
        "Display math should end with $$",
        "A $$ formula is not closed properly.",
        "Use \\[ … \\] instead of $$ … $$ and check that it is closed.",
    ),
    (
        "Bad math environment delimiter",
        "\\[ or \\( is opened inside a formula, or closed without being opened.",
        "Check that every \\[ has its \\] and every \\( its \\), and that they are not nested.",
    ),
    (
        "Missing \\begin{document}",
        "There is text before \\begin{document}, or a line in the preamble is broken.",
        "Look for stray text or a typo in the preamble, above \\begin{document}.",
    ),
    (
        "has an extra }",
        "A fragile command (\\footnote, \\verb…) is inside a moving argument such as a section title or caption, or braces do not match.",
        "Put \\protect before the fragile command, or move it out of the title.",
    ),
    (
        "Undefined color",
        "The colour name is not defined.",
        "Use a known name (red, blue, gray…) or define it with \\definecolor{name}{HTML}{1E90FF}.",
    ),
    (
        "Citation",
        "This \\cite key is not in the bibliography yet, or the bibliography has not been processed.",
        "Check the key in your .bib file (Ctrl+click shows it). rutex runs biber/bibtex automatically; if it persists, the key is wrong.",
    ),
    (
        "Reference",
        "This \\ref points to a \\label that does not exist (yet).",
        "Check the label name. Right after adding a label, the next build resolves it.",
    ),
    (
        "Label(s) may have changed",
        "References changed and need one more build.",
        "Nothing to do: rutex rebuilds until references settle.",
    ),
    (
        "Overfull \\hbox",
        "A line sticks out into the margin.",
        "Rephrase the sentence, allow hyphenation of a long word with \\-, wrap URLs in \\url{…}, or make a wide table smaller.",
    ),
    (
        "Underfull \\hbox",
        "A line is too loose, often because of \\\\ at the end of a paragraph.",
        "Usually harmless. Remove \\\\ at paragraph ends; use a blank line instead.",
    ),
    (
        "Font shape",
        "A font variant (bold italic, small caps…) does not exist and another one is used instead.",
        "Usually harmless. Load a font with that variant if it matters, e.g. \\usepackage{lmodern}.",
    ),
    (
        "Emergency stop",
        "LaTeX stopped early because of an earlier problem.",
        "Fix the first error in the list; this one usually disappears with it.",
    ),
];

/// What a message means and how to fix it, if it is a known one.
pub fn explain(message: &str) -> Option<(&'static str, &'static str)> {
    KNOWN.iter().find(|(pattern, _, _)| message.contains(pattern)).map(|(_, what, fix)| (tr(what), tr(fix)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explains_common_errors() {
        assert!(explain("Undefined control sequence.").unwrap().1.contains("\\usepackage"));
        assert!(explain("LaTeX Error: Environment alig undefined.").unwrap().0.contains("environment"));
        assert!(explain("LaTeX Error: File `tikzcd.sty' not found.").is_some());
        assert!(explain("Missing $ inserted.").is_some());
        assert!(explain("Overfull \\hbox (12.3pt too wide) in paragraph").is_some());
        assert!(explain("Something nobody has seen").is_none());
    }

    /// Every explanation has a Spanish translation.
    #[test]
    fn all_translated() {
        for (_, what, fix) in KNOWN {
            assert_ne!(crate::i18n::lookup(what, true), *what, "missing translation: {what}");
            assert_ne!(crate::i18n::lookup(fix, true), *fix, "missing translation: {fix}");
        }
    }
}
