//! Atajos y comportamientos de edición propios de un editor de LaTeX cómodo:
//! comentar líneas, negrita/cursiva, duplicar/borrar línea, autocierre de
//! llaves y `$`, y `\end{…}` automático al pulsar Intro tras `\begin{…}`.
//! Se instalan en fase de captura para adelantarse a los atajos de GtkTextView
//! (por ejemplo Ctrl+/ que por defecto selecciona todo).

use std::cell::Cell;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gdk, glib};
use sourceview5::prelude::ViewManualExt;

/// Opciones que se pueden cambiar en caliente desde las preferencias.
#[derive(Default)]
pub struct Options {
    pub autopairs: Cell<bool>,
    pub vim: Cell<bool>,
}

pub fn install(view: &sourceview5::View, options: Rc<Options>) {
    VIEWS.with(|v| v.borrow_mut().push(view.downgrade()));
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak = view.downgrade();
    keys.connect_key_pressed(move |_, keyval, _, state| {
        let Some(view) = weak.upgrade() else { return glib::Propagation::Proceed };
        let buffer = view.buffer();
        let ctrl = state.contains(gdk::ModifierType::CONTROL_MASK);
        let shift = state.contains(gdk::ModifierType::SHIFT_MASK);
        let alt = state.contains(gdk::ModifierType::ALT_MASK);
        let key = keyval.to_lower();

        let handled = if ctrl && !alt {
            match key {
                gdk::Key::slash | gdk::Key::KP_Divide => toggle_comment(&buffer),
                gdk::Key::b if !shift => wrap(&buffer, "\\textbf{", "}"),
                gdk::Key::i if !shift => wrap(&buffer, "\\textit{", "}"),
                gdk::Key::d if !shift => duplicate(&buffer),
                gdk::Key::k if shift => delete_lines(&buffer),
                // Atajos de la ventana que GtkTextView podría quedarse.
                gdk::Key::g if !shift => view.activate_action("win.goto-line", None).is_ok(),
                gdk::Key::j if !shift => view.activate_action("win.forward-sync", None).is_ok(),
                gdk::Key::p if !shift => view.activate_action("win.quick-open", None).is_ok(),
                _ => false,
            }
        } else if !ctrl && !alt && !options.vim.get() {
            match keyval {
                gdk::Key::Return | gdk::Key::KP_Enter if !shift => close_environment(&buffer),
                gdk::Key::Tab if !shift => expand_trigger(&view),
                _ if at_snippet(&buffer, keyval) => true,
                _ if options.autopairs.get() => autopair(&buffer, keyval),
                _ => false,
            }
        } else {
            false
        };
        if handled {
            view.scroll_mark_onscreen(&buffer.get_insert());
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    view.add_controller(keys);
}

fn cursor(buffer: &gtk::TextBuffer) -> gtk::TextIter {
    buffer.iter_at_mark(&buffer.get_insert())
}

/// Envuelve la selección (o inserta y deja el cursor dentro).
fn wrap(buffer: &gtk::TextBuffer, prefix: &str, suffix: &str) -> bool {
    buffer.begin_user_action();
    let pre = prefix.chars().count() as i32;
    if let Some((mut s, mut e)) = buffer.selection_bounds() {
        let text = buffer.text(&s, &e, false).to_string();
        let start = s.offset();
        buffer.delete(&mut s, &mut e);
        buffer.insert(&mut s, &format!("{prefix}{text}{suffix}"));
        let a = buffer.iter_at_offset(start + pre);
        let b = buffer.iter_at_offset(start + pre + text.chars().count() as i32);
        buffer.select_range(&a, &b);
    } else {
        let mut it = cursor(buffer);
        let start = it.offset();
        buffer.insert(&mut it, &format!("{prefix}{suffix}"));
        buffer.place_cursor(&buffer.iter_at_offset(start + pre));
    }
    buffer.end_user_action();
    true
}

/// Líneas afectadas por la selección (o la del cursor).
fn line_range(buffer: &gtk::TextBuffer) -> (i32, i32) {
    match buffer.selection_bounds() {
        Some((s, e)) => {
            let last = if e.starts_line() && e.line() > s.line() { e.line() - 1 } else { e.line() };
            (s.line(), last)
        }
        None => {
            let l = cursor(buffer).line();
            (l, l)
        }
    }
}

fn line_text(buffer: &gtk::TextBuffer, line: i32) -> String {
    let Some(start) = buffer.iter_at_line(line) else { return String::new() };
    let mut end = start;
    if !end.ends_line() {
        end.forward_to_line_end();
    }
    buffer.text(&start, &end, false).to_string()
}

/// Comenta o descomenta (`% `) las líneas seleccionadas.
fn toggle_comment(buffer: &gtk::TextBuffer) -> bool {
    let (first, last) = line_range(buffer);
    let lines: Vec<String> = (first..=last).map(|l| line_text(buffer, l)).collect();
    let uncomment = lines.iter().filter(|l| !l.trim().is_empty()).all(|l| l.trim_start().starts_with('%'));
    buffer.begin_user_action();
    for (i, text) in lines.iter().enumerate().rev() {
        let line = first + i as i32;
        if text.trim().is_empty() {
            continue;
        }
        let indent = (text.len() - text.trim_start().len()) as i32;
        let indent_chars = text[..indent as usize].chars().count() as i32;
        let Some(mut at) = buffer.iter_at_line_offset(line, indent_chars) else { continue };
        if uncomment {
            let rest = &text[indent as usize..];
            let n = if rest.starts_with("% ") { 2 } else { 1 };
            let mut end = at;
            end.forward_chars(n);
            buffer.delete(&mut at, &mut end);
        } else {
            buffer.insert(&mut at, "% ");
        }
    }
    buffer.end_user_action();
    true
}

/// Duplica la selección, o la línea actual si no hay selección.
fn duplicate(buffer: &gtk::TextBuffer) -> bool {
    buffer.begin_user_action();
    if let Some((s, mut e)) = buffer.selection_bounds() {
        let text = buffer.text(&s, &e, false).to_string();
        buffer.insert(&mut e, &text);
    } else {
        let line = cursor(buffer).line();
        let offset = cursor(buffer).line_offset();
        let text = line_text(buffer, line);
        let Some(mut start) = buffer.iter_at_line(line) else { return false };
        buffer.insert(&mut start, &format!("{text}\n"));
        if let Some(it) = buffer.iter_at_line_offset(line + 1, offset) {
            buffer.place_cursor(&it);
        }
    }
    buffer.end_user_action();
    true
}

/// Borra las líneas seleccionadas (o la actual).
fn delete_lines(buffer: &gtk::TextBuffer) -> bool {
    let (first, last) = line_range(buffer);
    let Some(mut start) = buffer.iter_at_line(first) else { return false };
    let mut end = buffer.iter_at_line(last + 1).unwrap_or_else(|| buffer.end_iter());
    if end.line() == last {
        // Última línea del documento: borramos también el salto anterior.
        end = buffer.end_iter();
        start.backward_char();
    }
    buffer.begin_user_action();
    buffer.delete(&mut start, &mut end);
    buffer.end_user_action();
    true
}

/// Intro justo después de `\begin{entorno}` → añade la línea interior y `\end{entorno}`.
fn close_environment(buffer: &gtk::TextBuffer) -> bool {
    if buffer.has_selection() {
        return false;
    }
    let it = cursor(buffer);
    let mut line_start = it;
    line_start.set_line_offset(0);
    let mut line_end = it;
    if !line_end.ends_line() {
        line_end.forward_to_line_end();
    }
    if !buffer.text(&it, &line_end, false).trim().is_empty() {
        return false;
    }
    let before = buffer.text(&line_start, &it, false).to_string();
    let Some(env) = begun_environment(&before) else { return false };

    // Solo si en el documento hay más \begin{env} que \end{env}.
    let (s, e) = buffer.bounds();
    let all = buffer.text(&s, &e, false);
    let opens = all.matches(&format!("\\begin{{{env}}}")).count();
    let closes = all.matches(&format!("\\end{{{env}}}")).count();
    if closes >= opens {
        return false;
    }
    let indent: String = before.chars().take_while(|c| c.is_whitespace()).collect();
    buffer.begin_user_action();
    let pos = it.offset();
    let mut at = it;
    let inner = format!("\n{indent}  ");
    buffer.insert(&mut at, &format!("{inner}\n{indent}\\end{{{env}}}"));
    let target = buffer.iter_at_offset(pos + inner.chars().count() as i32);
    buffer.place_cursor(&target);
    buffer.end_user_action();
    true
}

/// `@` + key inside a formula → command (LaTeX Workshop style).
pub const AT_SNIPPETS: &[(char, &str)] = &[
    ('a', "\\alpha"), ('b', "\\beta"), ('g', "\\gamma"), ('d', "\\delta"), ('e', "\\epsilon"), ('z', "\\zeta"),
    ('h', "\\eta"), ('q', "\\theta"), ('i', "\\iota"), ('k', "\\kappa"), ('l', "\\lambda"), ('m', "\\mu"),
    ('n', "\\nu"), ('x', "\\xi"), ('p', "\\pi"), ('r', "\\rho"), ('s', "\\sigma"), ('t', "\\tau"),
    ('u', "\\upsilon"), ('f', "\\phi"), ('c', "\\chi"), ('y', "\\psi"), ('w', "\\omega"), ('G', "\\Gamma"),
    ('D', "\\Delta"), ('Q', "\\Theta"), ('L', "\\Lambda"), ('X', "\\Xi"), ('P', "\\Pi"), ('S', "\\Sigma"),
    ('U', "\\Upsilon"), ('F', "\\Phi"), ('Y', "\\Psi"), ('W', "\\Omega"), ('8', "\\infty"), ('6', "\\partial"),
    ('<', "\\leq"), ('>', "\\geq"), ('=', "\\equiv"), ('+', "\\sum"), ('I', "\\int"), ('.', "\\cdot"),
    ('*', "\\times"), ('-', "\\setminus"), ('0', "\\emptyset"), ('/', "\\frac{${1}}{${2}}$0"), ('^', "\\hat{${1}}$0"),
    ('_', "\\bar{${1}}$0"), ('(', "\\left( ${1} \\right)$0"), ('[', "\\left[ ${1} \\right]$0"),
];

/// Word + Tab → environment.
pub const TAB_TRIGGERS: &[(&str, &str)] = &[
    ("BEQ", "\\begin{equation}\n\t${1}\n\t\\label{eq:${2}}\n\\end{equation}$0"),
    ("BSEQ", "\\begin{equation*}\n\t${1}\n\\end{equation*}$0"),
    ("BAL", "\\begin{align}\n\t${1}\n\\end{align}$0"),
    ("BSAL", "\\begin{align*}\n\t${1}\n\\end{align*}$0"),
    ("BGA", "\\begin{gather}\n\t${1}\n\\end{gather}$0"),
    ("BCAS", "\\begin{cases}\n\t${1} & ${2} \\\\\n\\end{cases}$0"),
    ("BIT", "\\begin{itemize}\n\t\\item ${1}\n\\end{itemize}$0"),
    ("BEN", "\\begin{enumerate}\n\t\\item ${1}\n\\end{enumerate}$0"),
    ("BDE", "\\begin{description}\n\t\\item[${1}] ${2}\n\\end{description}$0"),
    ("BFI", "\\begin{figure}[htbp]\n\t\\centering\n\t\\includegraphics[width=0.8\\linewidth]{${1}}\n\t\\caption{${2}}\n\t\\label{fig:${3}}\n\\end{figure}$0"),
    ("BTA", "\\begin{table}[htbp]\n\t\\centering\n\t\\caption{${1}}\n\t\\label{tab:${2}}\n\t\\begin{tabular}{${3:lcr}}\n\t\t${4}\n\t\\end{tabular}\n\\end{table}$0"),
    ("BTH", "\\begin{theorem}\n\t${1}\n\\end{theorem}$0"),
    ("BPR", "\\begin{proof}\n\t${1}\n\\end{proof}$0"),
    ("BFR", "\\begin{frame}{${1}}\n\t${2}\n\\end{frame}$0"),
];

/// Expands a snippet replacing `len` characters before the cursor.
fn push_snippet(view: &sourceview5::View, len: i32, snippet: &str) {
    let buffer = view.buffer();
    let mut end = cursor(&buffer);
    let mut start = end;
    start.backward_chars(len);
    buffer.begin_user_action();
    buffer.delete(&mut start, &mut end);
    // In snippets a backslash escapes the next character: double them.
    let parsed = sourceview5::Snippet::new_parsed(&snippet.replace('\\', "\\\\")).unwrap_or_else(|_| sourceview5::Snippet::new(None, None));
    view.push_snippet(&parsed, Some(&mut start));
    buffer.end_user_action();
}

/// `BEQ` + Tab → equation environment (only when the word is a trigger).
fn expand_trigger(view: &sourceview5::View) -> bool {
    let buffer = view.buffer();
    let it = cursor(&buffer);
    let mut start = it;
    start.set_line_offset(0);
    let before = buffer.text(&start, &it, false).to_string();
    let word: String = before.chars().rev().take_while(|c| c.is_ascii_uppercase()).collect::<Vec<_>>().into_iter().rev().collect();
    let rest = &before[..before.len() - word.len()];
    if !(rest.is_empty() || rest.ends_with(char::is_whitespace)) {
        return false;
    }
    let Some((_, snippet)) = TAB_TRIGGERS.iter().find(|(t, _)| *t == word) else { return false };
    push_snippet(view, word.chars().count() as i32, snippet);
    true
}

/// `@a` inside a formula → `\\alpha` as soon as the second key is typed.
fn at_snippet(buffer: &gtk::TextBuffer, keyval: gdk::Key) -> bool {
    let Some(typed) = keyval.to_unicode() else { return false };
    let Some((_, cmd)) = AT_SNIPPETS.iter().find(|(c, _)| *c == typed) else { return false };
    let it = cursor(buffer);
    let mut prev = it;
    if !prev.backward_char() || prev.char() != '@' {
        return false;
    }
    let (s, e) = buffer.bounds();
    if crate::mathpreview::math_at(&buffer.text(&s, &e, false), it.offset() as usize).is_none() {
        return false;
    }
    let Some(view) = buffer_view(buffer) else { return false };
    let snippet = if cmd.contains('$') { cmd.to_string() } else { format!("{cmd} ") };
    push_snippet(&view, 1, &snippet);
    true
}

thread_local! {
    static VIEWS: std::cell::RefCell<Vec<glib::WeakRef<sourceview5::View>>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// The view showing a buffer (each buffer has exactly one view here).
fn buffer_view(buffer: &gtk::TextBuffer) -> Option<sourceview5::View> {
    VIEWS.with(|v| {
        let mut v = v.borrow_mut();
        v.retain(|w| w.upgrade().is_some());
        v.iter().filter_map(|w| w.upgrade()).find(|view| view.buffer() == *buffer)
    })
}

/// `  \begin{figure}[htbp]` → `figure`.
pub fn begun_environment(before: &str) -> Option<String> {
    let start = before.rfind("\\begin{")? + "\\begin{".len();
    let close = before[start..].find('}')? + start;
    let env = &before[start..close];
    if env.is_empty() || before[close..].contains("\\end") || env.chars().any(|c| !(c.is_alphanumeric() || c == '*')) {
        return None;
    }
    Some(env.to_string())
}

fn closer(open: gdk::Key) -> Option<char> {
    match open {
        gdk::Key::braceleft => Some('}'),
        gdk::Key::bracketleft => Some(']'),
        gdk::Key::parenleft => Some(')'),
        gdk::Key::dollar => Some('$'),
        _ => None,
    }
}

/// Autocierre de `{ [ ( $`, saltar el cierre si ya está y borrar parejas vacías.
fn autopair(buffer: &gtk::TextBuffer, keyval: gdk::Key) -> bool {
    let it = cursor(buffer);
    let next = it.char();
    let mut prev_it = it;
    let prev = if prev_it.backward_char() { prev_it.char() } else { '\n' };

    if keyval == gdk::Key::BackSpace {
        let pairs = [('{', '}'), ('[', ']'), ('(', ')'), ('$', '$')];
        if !buffer.has_selection() && pairs.contains(&(prev, next)) {
            let mut s = prev_it;
            let mut e = it;
            e.forward_char();
            buffer.begin_user_action();
            buffer.delete(&mut s, &mut e);
            buffer.end_user_action();
            return true;
        }
        return false;
    }

    let Some(typed) = keyval.to_unicode() else { return false };
    // Saltar sobre un cierre que ya está ahí.
    if matches!(typed, '}' | ']' | ')' | '$') && next == typed && !buffer.has_selection() && prev != '\\' {
        let mut fwd = it;
        fwd.forward_char();
        buffer.place_cursor(&fwd);
        return true;
    }
    let Some(close) = closer(keyval) else { return false };
    if prev == '\\' {
        return false; // \{ \$ … son caracteres literales
    }
    if buffer.has_selection() {
        return wrap(buffer, &typed.to_string(), &close.to_string());
    }
    // Solo si después no hay texto pegado (no molestar al editar en mitad de una palabra).
    if !(next.is_whitespace() || next == '\0' || matches!(next, '}' | ']' | ')' | ',' | '.')) {
        return false;
    }
    if typed == '$' && prev.is_alphanumeric() {
        return false;
    }
    wrap(buffer, &typed.to_string(), &close.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_begun_environment() {
        assert_eq!(begun_environment("  \\begin{itemize}").as_deref(), Some("itemize"));
        assert_eq!(begun_environment("\\begin{figure}[htbp]").as_deref(), Some("figure"));
        assert_eq!(begun_environment("\\begin{align*}").as_deref(), Some("align*"));
        assert_eq!(begun_environment("\\begin{x}hola\\end{x}"), None);
        assert_eq!(begun_environment("texto"), None);
    }

    /// Prueba las ediciones sobre un GtkTextBuffer real (se salta si no hay display).
    #[test]
    fn buffer_edits() {
        if gtk::init().is_err() {
            return;
        }
        let buf = gtk::TextBuffer::new(None);
        let set = |text: &str, cursor: i32| {
            buf.set_text(text);
            buf.place_cursor(&buf.iter_at_offset(cursor));
        };
        let text = || {
            let (s, e) = buf.bounds();
            buf.text(&s, &e, false).to_string()
        };

        set("a\nb", 0);
        toggle_comment(&buf);
        assert_eq!(text(), "% a\nb");
        toggle_comment(&buf);
        assert_eq!(text(), "a\nb");

        set("hola", 0);
        buf.select_range(&buf.iter_at_offset(0), &buf.iter_at_offset(4));
        wrap(&buf, "\\textbf{", "}");
        assert_eq!(text(), "\\textbf{hola}");

        set("uno\ndos", 1);
        duplicate(&buf);
        assert_eq!(text(), "uno\nuno\ndos");
        delete_lines(&buf);
        assert_eq!(text(), "uno\ndos");

        set("  \\begin{itemize}", 17);
        assert!(close_environment(&buf));
        assert_eq!(text(), "  \\begin{itemize}\n    \n  \\end{itemize}");
        assert_eq!(cursor(&buf).line(), 1);

        set("x ", 2);
        autopair(&buf, gdk::Key::braceleft);
        assert_eq!(text(), "x {}");
        autopair(&buf, gdk::Key::braceright);
        assert_eq!(cursor(&buf).offset(), 4);
        set("x {}", 3);
        autopair(&buf, gdk::Key::BackSpace);
        assert_eq!(text(), "x ");
        set("50", 2);
        assert!(!autopair(&buf, gdk::Key::dollar));
    }

    /// Snippets on a real view: `BEQ` + Tab and `@a` inside a formula.
    #[test]
    fn snippets() {
        if gtk::init().is_err() {
            return;
        }
        let buffer = sourceview5::Buffer::new(None);
        let view = sourceview5::View::with_buffer(&buffer);
        install(&view, Rc::new(Options::default()));
        let text = || {
            let (s, e) = buffer.bounds();
            buffer.text(&s, &e, false).to_string()
        };

        buffer.set_text("BEQ");
        buffer.place_cursor(&buffer.end_iter());
        assert!(expand_trigger(&view));
        assert!(text().starts_with("\\begin{equation}\n"), "{}", text());

        buffer.set_text("so BITE");
        buffer.place_cursor(&buffer.end_iter());
        assert!(!expand_trigger(&view), "only exact triggers");

        buffer.set_text("$x = @$");
        buffer.place_cursor(&buffer.iter_at_offset(6));
        assert!(at_snippet(buffer.upcast_ref(), gdk::Key::a));
        assert_eq!(text(), "$x = \\alpha $");

        buffer.set_text("mail me@a");
        buffer.place_cursor(&buffer.end_iter());
        assert!(!at_snippet(buffer.upcast_ref(), gdk::Key::b), "not outside formulas");
    }
}
