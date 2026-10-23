//! Folding: chapters, sections and multi-line environments can be collapsed
//! from a small arrow in the margin (or the View menu). Folded lines are
//! hidden with an invisible text tag, so the text itself never changes and
//! saving always writes everything. Editing next to a fold, or moving the
//! cursor into it (search, go to line, the outline), opens it again.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::Duration;

use gtk::glib::subclass::prelude::*;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib, gsk};
use sourceview5::subclass::prelude::*;

use crate::project::strip_comment;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Region {
    /// 0-based lines; `start` stays visible, `start+1..=end` are hidden.
    pub start: u32,
    pub end: u32,
    pub heading: bool,
}

const HEADINGS: &[(&str, u8)] = &[("part", 0), ("chapter", 1), ("section", 2), ("subsection", 3), ("subsubsection", 4), ("paragraph", 5)];
const STOPS: &[&str] = &["\\end{document}", "\\appendix", "\\backmatter", "\\printbibliography", "\\bibliography{"];
const VERBATIM: &[&str] = &["verbatim", "lstlisting", "minted", "comment"];

fn heading_level(line: &str) -> Option<u8> {
    let t = line.trim_start().strip_prefix('\\')?;
    HEADINGS.iter().find_map(|(name, level)| {
        let rest = t.strip_prefix(name)?;
        let rest = rest.strip_prefix('*').unwrap_or(rest);
        (rest.starts_with('{') || rest.starts_with('[')).then_some(*level)
    })
}

/// Environment names opened (`\begin{x}`) or closed on a line, in order.
fn env_events(line: &str) -> Vec<(bool, &str)> {
    let mut out = Vec::new();
    let mut rest = line;
    while let Some(i) = rest.find(['\\']) {
        rest = &rest[i + 1..];
        let open = if rest.starts_with("begin{") {
            true
        } else if rest.starts_with("end{") {
            false
        } else {
            continue;
        };
        let after = &rest[if open { 6 } else { 4 }..];
        if let Some(close) = after.find('}') {
            out.push((open, &after[..close]));
            rest = &after[close..];
        }
    }
    out
}

/// Foldable regions of a LaTeX source: headings up to the next heading of the
/// same or a higher level, and environments spanning several lines.
pub fn regions(text: &str) -> Vec<Region> {
    let lines: Vec<&str> = text.lines().collect();
    let code: Vec<&str> = lines.iter().map(|l| strip_comment(l)).collect();
    let mut out: Vec<Region> = Vec::new();
    let mut stack: Vec<(&str, usize)> = Vec::new();
    let mut verbatim: Option<&str> = None;
    let mut headings: Vec<(usize, u8)> = Vec::new();
    let mut stops: Vec<usize> = Vec::new();
    for (n, line) in code.iter().enumerate() {
        if let Some(v) = verbatim {
            if lines[n].contains(&format!("\\end{{{v}}}")) {
                verbatim = None;
                if let Some(pos) = stack.iter().rposition(|(name, _)| *name == v) {
                    let (_, start) = stack.remove(pos);
                    if n > start {
                        out.push(Region { start: start as u32, end: n as u32, heading: false });
                    }
                }
            }
            continue;
        }
        if let Some(level) = heading_level(line) {
            headings.push((n, level));
        }
        if STOPS.iter().any(|s| line.contains(s)) {
            stops.push(n);
        }
        for (open, name) in env_events(line) {
            if open {
                stack.push((name, n));
                if VERBATIM.contains(&name.trim_end_matches('*')) {
                    verbatim = Some(name);
                    break;
                }
            } else if let Some(pos) = stack.iter().rposition(|(s, _)| *s == name) {
                let (_, start) = stack.remove(pos);
                stack.truncate(pos);
                if n > start && name != "document" {
                    out.push(Region { start: start as u32, end: n as u32, heading: false });
                }
            }
        }
    }
    for (i, &(start, level)) in headings.iter().enumerate() {
        let next = headings[i + 1..].iter().find(|(_, l)| *l <= level).map(|(n, _)| *n);
        let stop = stops.iter().find(|s| **s > start).copied();
        let mut end = match (next, stop) {
            (Some(a), Some(b)) => a.min(b),
            (Some(a), None) => a,
            (None, Some(b)) => b,
            (None, None) => lines.len(),
        }
        .saturating_sub(1);
        while end > start && lines[end].trim().is_empty() {
            end -= 1;
        }
        if end > start {
            out.push(Region { start: start as u32, end: end as u32, heading: true });
        }
    }
    // One region per first line: the biggest one.
    out.sort_by_key(|r| (r.start, std::cmp::Reverse(r.end)));
    out.dedup_by_key(|r| r.start);
    out
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct FoldBar {
        /// First line of each foldable region → whether it is folded.
        pub starts: RefCell<HashMap<u32, bool>>,
        pub on_toggle: RefCell<Option<Box<dyn Fn(u32)>>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for FoldBar {
        const NAME: &'static str = "RutexFoldBar";
        type Type = super::FoldBar;
        type ParentType = sourceview5::GutterRenderer;
    }

    impl ObjectImpl for FoldBar {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().set_width_request(14);
        }
    }
    impl WidgetImpl for FoldBar {}

    impl GutterRendererImpl for FoldBar {
        fn query_activatable(&self, iter: &gtk::TextIter, _area: &gdk::Rectangle) -> bool {
            self.starts.borrow().contains_key(&(iter.line() as u32))
        }

        fn activate(&self, iter: &gtk::TextIter, _area: &gdk::Rectangle, _button: u32, _state: gdk::ModifierType, _n: i32) {
            if let Some(f) = self.on_toggle.borrow().as_ref() {
                f(iter.line() as u32);
            }
        }

        fn snapshot_line(&self, snapshot: &gtk::Snapshot, lines: &sourceview5::GutterLines, line: u32) {
            let Some(&folded) = self.starts.borrow().get(&line) else { return };
            let (y, h) = lines.line_yrange(line, sourceview5::GutterRendererAlignmentMode::First);
            let mut color = self.obj().color();
            color.set_alpha(if folded { 0.95 } else { 0.35 });
            let (cx, cy, r) = (7.0f32, y as f32 + h as f32 / 2.0, 3.5f32);
            let path = gsk::PathBuilder::new();
            if folded {
                // ▸
                path.move_to(cx - r * 0.6, cy - r);
                path.line_to(cx + r * 0.8, cy);
                path.line_to(cx - r * 0.6, cy + r);
            } else {
                // ▾
                path.move_to(cx - r, cy - r * 0.6);
                path.line_to(cx + r, cy - r * 0.6);
                path.line_to(cx, cy + r * 0.8);
            }
            path.close();
            snapshot.append_fill(&path.to_path(), gsk::FillRule::Winding, &color);
        }
    }
}

glib::wrapper! {
    pub struct FoldBar(ObjectSubclass<imp::FoldBar>) @extends sourceview5::GutterRenderer, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

/// The folds of one editor view.
pub struct Folds {
    view: sourceview5::View,
    bar: FoldBar,
    hidden: gtk::TextTag,
    header: gtk::TextTag,
    regions: RefCell<Vec<Region>>,
    /// Marks at the start of each folded first line (they move with edits).
    folded: RefCell<Vec<gtk::TextMark>>,
    /// Lines edited since the last update: folds touching them open.
    touched: RefCell<HashSet<u32>>,
    timer: RefCell<Option<glib::SourceId>>,
    busy: Cell<bool>,
}

impl Folds {
    pub fn attach(view: &sourceview5::View) -> Rc<Folds> {
        let buffer = view.buffer();
        let table = buffer.tag_table();
        let hidden = gtk::TextTag::builder().name("rutex-fold-hidden").invisible(true).build();
        let header = gtk::TextTag::builder().name("rutex-fold-header").paragraph_background_rgba(&gdk::RGBA::new(0.5, 0.5, 0.5, 0.14)).build();
        table.add(&hidden);
        table.add(&header);
        let bar: FoldBar = glib::Object::new();
        sourceview5::prelude::ViewExt::gutter(view, gtk::TextWindowType::Left).insert(&bar, 90);
        let folds = Rc::new(Folds {
            view: view.clone(),
            bar,
            hidden,
            header,
            regions: RefCell::default(),
            folded: RefCell::default(),
            touched: RefCell::default(),
            timer: RefCell::default(),
            busy: Cell::new(false),
        });

        let weak = Rc::downgrade(&folds);
        *folds.bar.imp().on_toggle.borrow_mut() = Some(Box::new(move |line| {
            if let Some(f) = weak.upgrade() {
                f.toggle(line);
            }
        }));
        let weak = Rc::downgrade(&folds);
        buffer.connect_changed(move |b| {
            let Some(f) = weak.upgrade() else { return };
            f.touched.borrow_mut().insert(b.iter_at_mark(&b.get_insert()).line() as u32);
            f.schedule();
        });
        // The cursor went into folded text (search, go to line…): open it.
        let weak = Rc::downgrade(&folds);
        buffer.connect_mark_set(move |b, it, mark| {
            let Some(f) = weak.upgrade() else { return };
            if !f.busy.get() && *mark == b.get_insert() && it.has_tag(&f.hidden) {
                f.reveal(it.line() as u32);
            }
        });
        folds.update();
        folds
    }

    fn buffer(&self) -> gtk::TextBuffer {
        self.view.buffer()
    }

    fn schedule(self: &Rc<Self>) {
        if let Some(id) = self.timer.borrow_mut().take() {
            id.remove();
        }
        let weak = Rc::downgrade(self);
        *self.timer.borrow_mut() = Some(glib::timeout_add_local_once(Duration::from_millis(250), move || {
            if let Some(f) = weak.upgrade() {
                f.timer.borrow_mut().take();
                f.update();
            }
        }));
    }

    /// Recomputes the regions and re-applies the folds that still make sense.
    fn update(&self) {
        let buffer = self.buffer();
        let (s, e) = buffer.bounds();
        *self.regions.borrow_mut() = regions(&buffer.text(&s, &e, true));
        let touched: Vec<u32> = self.touched.borrow_mut().drain().collect();
        let regions = self.regions.borrow().clone();
        self.folded.borrow_mut().retain(|mark| {
            let line = buffer.iter_at_mark(mark).line() as u32;
            let keep = regions.iter().find(|r| r.start == line).is_some_and(|r| !touched.iter().any(|t| *t >= r.start && *t <= r.end + 1));
            if !keep {
                buffer.delete_mark(mark);
            }
            keep
        });
        self.apply();
    }

    /// Hides the folded lines and refreshes the margin arrows.
    fn apply(&self) {
        let buffer = self.buffer();
        let (s, e) = buffer.bounds();
        self.busy.set(true);
        buffer.remove_tag(&self.hidden, &s, &e);
        buffer.remove_tag(&self.header, &s, &e);
        let regions = self.regions.borrow();
        let mut starts: HashMap<u32, bool> = regions.iter().map(|r| (r.start, false)).collect();
        for mark in self.folded.borrow().iter() {
            let line = buffer.iter_at_mark(mark).line() as u32;
            let Some(r) = regions.iter().find(|r| r.start == line) else { continue };
            starts.insert(line, true);
            let Some(mut from) = buffer.iter_at_line(r.start as i32) else { continue };
            let header_start = from;
            if !from.ends_line() {
                from.forward_to_line_end();
            }
            let mut header_end = from;
            header_end.forward_char();
            buffer.apply_tag(&self.header, &header_start, &header_end);
            // Hide from the end of the first line to the end of the last one,
            // so the next visible line follows the folded one directly.
            let mut to = buffer.iter_at_line(r.end as i32).unwrap_or(e);
            if !to.ends_line() {
                to.forward_to_line_end();
            }
            buffer.apply_tag(&self.hidden, &from, &to);
        }
        self.busy.set(false);
        *self.bar.imp().starts.borrow_mut() = starts;
        self.bar.queue_draw();
    }

    fn is_folded(&self, start: u32) -> bool {
        let buffer = self.buffer();
        self.folded.borrow().iter().any(|m| buffer.iter_at_mark(m).line() as u32 == start)
    }

    fn fold(&self, region: Region) {
        if self.is_folded(region.start) {
            return;
        }
        let buffer = self.buffer();
        let Some(at) = buffer.iter_at_line(region.start as i32) else { return };
        // Keep the cursor visible: on the first line if it was inside.
        let cursor = buffer.iter_at_mark(&buffer.get_insert()).line() as u32;
        if cursor > region.start && cursor <= region.end {
            let mut end = at;
            if !end.ends_line() {
                end.forward_to_line_end();
            }
            self.busy.set(true);
            buffer.place_cursor(&end);
            self.busy.set(false);
        }
        self.folded.borrow_mut().push(buffer.create_mark(None, &at, true));
        self.apply();
    }

    fn unfold(&self, start: u32) {
        let buffer = self.buffer();
        self.folded.borrow_mut().retain(|m| {
            let keep = buffer.iter_at_mark(m).line() as u32 != start;
            if !keep {
                buffer.delete_mark(m);
            }
            keep
        });
        self.apply();
    }

    /// Opens every fold that hides `line`.
    fn reveal(&self, line: u32) {
        let hiding: Vec<u32> = self.regions.borrow().iter().filter(|r| line > r.start && line <= r.end).map(|r| r.start).filter(|s| self.is_folded(*s)).collect();
        for s in hiding {
            self.unfold(s);
        }
    }

    pub fn toggle(&self, line: u32) {
        let region = self.regions.borrow().iter().find(|r| r.start == line).copied();
        match region {
            Some(r) if self.is_folded(r.start) => self.unfold(r.start),
            Some(r) => self.fold(r),
            None => {}
        }
    }

    /// Innermost region around the cursor.
    fn around_cursor(&self) -> Option<Region> {
        let buffer = self.buffer();
        let line = buffer.iter_at_mark(&buffer.get_insert()).line() as u32;
        self.regions.borrow().iter().filter(|r| r.start <= line && line <= r.end).min_by_key(|r| r.end - r.start).copied()
    }

    pub fn fold_at_cursor(&self) {
        let buffer = self.buffer();
        let line = buffer.iter_at_mark(&buffer.get_insert()).line() as u32;
        // On an already folded first line, fold the enclosing region instead.
        let candidates: Vec<Region> = self.regions.borrow().iter().filter(|r| r.start <= line && line <= r.end && !self.is_folded(r.start)).copied().collect();
        if let Some(r) = candidates.into_iter().min_by_key(|r| r.end - r.start) {
            self.fold(r);
        }
    }

    pub fn unfold_at_cursor(&self) {
        if let Some(r) = self.around_cursor() {
            self.unfold(r.start);
        }
    }

    pub fn fold_all(&self) {
        let headings: Vec<Region> = self.regions.borrow().iter().filter(|r| r.heading).copied().collect();
        for r in headings {
            self.fold(r);
        }
    }

    pub fn unfold_all(&self) {
        let buffer = self.buffer();
        for m in self.folded.borrow_mut().drain(..) {
            buffer.delete_mark(&m);
        }
        self.apply();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOC: &str = "\\documentclass{report}
\\begin{document}
\\chapter{One}
Intro.
\\section{A}
Text a.
\\begin{figure}
  \\centering
\\end{figure}

\\section{B}
% \\section{not a heading}
Text b.
\\begin{lstlisting}
\\section{inside code}
\\end{lstlisting}
\\chapter{Two}
Last.
\\end{document}
";

    #[test]
    fn finds_regions() {
        let r = regions(DOC);
        let find = |start| r.iter().find(|x| x.start == start).copied();
        assert_eq!(find(2), Some(Region { start: 2, end: 15, heading: true }), "chapter One ends before chapter Two");
        assert_eq!(find(4), Some(Region { start: 4, end: 8, heading: true }), "section A ends before section B, without blank lines");
        assert_eq!(find(6), Some(Region { start: 6, end: 8, heading: false }), "figure environment");
        assert_eq!(find(10), Some(Region { start: 10, end: 15, heading: true }));
        assert_eq!(find(13), Some(Region { start: 13, end: 15, heading: false }), "code listing");
        assert!(find(14).is_none(), "headings inside code are ignored");
        assert_eq!(find(16), Some(Region { start: 16, end: 17, heading: true }), "stops at \\end{{document}}");
        assert!(find(1).is_none(), "the document environment is not folded");
    }

    /// Folding on a real view hides text without changing it.
    #[test]
    fn folds_and_unfolds() {
        crate::testutil::with_gtk(|| {
            let buffer = sourceview5::Buffer::new(None);
            buffer.set_text(DOC);
            let view = sourceview5::View::with_buffer(&buffer);
            let folds = Folds::attach(&view);
            folds.toggle(4);
            let (s, e) = buffer.bounds();
            assert_eq!(buffer.text(&s, &e, true), DOC, "text is unchanged");
            let visible = buffer.text(&s, &e, false).to_string();
            assert!(visible.contains("\\section{A}\n\n\\section{B}"), "{visible}");
            // Moving the cursor into the fold opens it.
            buffer.place_cursor(&buffer.iter_at_line(5).unwrap());
            let (s, e) = buffer.bounds();
            assert!(buffer.text(&s, &e, false).contains("Text a."));
            folds.fold_all();
            let (s, e) = buffer.bounds();
            assert!(!buffer.text(&s, &e, false).contains("Last."));
            folds.unfold_all();
            let (s, e) = buffer.bounds();
            assert_eq!(buffer.text(&s, &e, false), DOC);
        });
    }
}
