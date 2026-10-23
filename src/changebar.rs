//! Thin bars in the editor margin showing what changed compared with a
//! version from the history: green = added, orange = modified, red = removed
//! (a short mark where lines were deleted). Like git gutters, without touching
//! the text itself.

use std::cell::RefCell;
use std::collections::HashMap;

use gtk::glib::subclass::prelude::*;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib, graphene};
use sourceview5::subclass::prelude::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    Added,
    Modified,
    /// Lines were removed just before this one.
    Deleted,
}

/// Per-line changes (0-based line → change) between two texts.
pub fn line_changes(old: &str, new: &str) -> HashMap<u32, Change> {
    let mut out = HashMap::new();
    let diff = similar::TextDiff::from_lines(old, new);
    for op in diff.ops() {
        let (tag, _old, new_range) = op.as_tag_tuple();
        match tag {
            similar::DiffTag::Equal => {}
            similar::DiffTag::Insert => {
                for l in new_range {
                    out.insert(l as u32, Change::Added);
                }
            }
            similar::DiffTag::Replace => {
                for l in new_range {
                    out.insert(l as u32, Change::Modified);
                }
            }
            similar::DiffTag::Delete => {
                // Mark the line that now sits where the deleted ones were.
                let at = new_range.start as u32;
                out.entry(at).or_insert(Change::Deleted);
            }
        }
    }
    out
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct ChangeBar {
        pub changes: RefCell<HashMap<u32, Change>>,
        pub colors: RefCell<Option<[gdk::RGBA; 3]>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ChangeBar {
        const NAME: &'static str = "RutexChangeBar";
        type Type = super::ChangeBar;
        type ParentType = sourceview5::GutterRenderer;
    }

    impl ObjectImpl for ChangeBar {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().set_width_request(5);
        }
    }
    impl WidgetImpl for ChangeBar {}

    impl GutterRendererImpl for ChangeBar {
        fn snapshot_line(&self, snapshot: &gtk::Snapshot, lines: &sourceview5::GutterLines, line: u32) {
            let Some(change) = self.changes.borrow().get(&line).copied() else { return };
            let colors = self.colors.borrow().unwrap_or_else(super::default_colors);
            let (y, h) = lines.line_yrange(line, sourceview5::GutterRendererAlignmentMode::Cell);
            let (color, rect) = match change {
                Change::Added => (colors[0], graphene::Rect::new(1.0, y as f32, 3.0, h as f32)),
                Change::Modified => (colors[1], graphene::Rect::new(1.0, y as f32, 3.0, h as f32)),
                Change::Deleted => (colors[2], graphene::Rect::new(0.0, y as f32 - 2.0, 5.0, 4.0)),
            };
            snapshot.append_color(&color, &rect);
        }
    }
}

glib::wrapper! {
    pub struct ChangeBar(ObjectSubclass<imp::ChangeBar>) @extends sourceview5::GutterRenderer, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for ChangeBar {
    fn default() -> Self {
        glib::Object::new()
    }
}

fn default_colors() -> [gdk::RGBA; 3] {
    let p = |s| gdk::RGBA::parse(s).unwrap();
    [p("#2ec27e"), p("#e5a50a"), p("#e01b24")]
}

impl ChangeBar {
    pub fn set_changes(&self, changes: HashMap<u32, Change>) {
        *self.imp().changes.borrow_mut() = changes;
        self.queue_draw();
    }

    /// Theme colors (green, yellow, red) when the palette has them.
    pub fn refresh_colors(&self) {
        let colors = crate::theme::palette().and_then(|p| {
            let c = |s: &str| gdk::RGBA::parse(s).ok();
            Some([c(&p.green)?, c(&p.orange)?, c(&p.red)?])
        });
        *self.imp().colors.borrow_mut() = colors;
        self.queue_draw();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_lines() {
        let old = "a\nb\nc\nd\n";
        let new = "a\nB\nc\nnew\nd\n";
        let ch = line_changes(old, new);
        assert_eq!(ch.get(&1), Some(&Change::Modified));
        assert_eq!(ch.get(&3), Some(&Change::Added));
        assert_eq!(ch.get(&0), None);
        let ch = line_changes("a\nb\nc\n", "a\nc\n");
        assert_eq!(ch.get(&1), Some(&Change::Deleted));
    }
}
