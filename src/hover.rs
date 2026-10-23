//! Bocadillo al pasar el ratón por una línea con errores o avisos.

use std::cell::RefCell;
use std::pin::Pin;
use std::rc::Rc;

use gtk::glib::subclass::prelude::*;
use gtk::prelude::*;
use gtk::{gio, glib};
use sourceview5::subclass::prelude::*;
use crate::i18n::tr;

/// What to show for a position: a widget now, or one that is still being
/// made (a formula being compiled).
pub enum Content {
    Ready(gtk::Widget),
    Later(Pin<Box<dyn std::future::Future<Output = Option<gtk::Widget>>>>),
}

/// Builds the bubble for a position in a buffer, or None if there is nothing to show.
pub type Lookup = Rc<dyn Fn(&sourceview5::Buffer, &gtk::TextIter) -> Option<Content>>;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct IssueHover {
        pub lookup: RefCell<Option<Lookup>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for IssueHover {
        const NAME: &'static str = "RutexIssueHover";
        type Type = super::IssueHover;
        type Interfaces = (sourceview5::HoverProvider,);
    }
    impl ObjectImpl for IssueHover {}

    impl HoverProviderImpl for IssueHover {
        fn populate_future(
            &self,
            context: &sourceview5::HoverContext,
            display: &sourceview5::HoverDisplay,
        ) -> Pin<Box<dyn std::future::Future<Output = Result<(), glib::Error>> + 'static>> {
            let content = match (self.lookup.borrow().as_ref(), context.iter()) {
                (Some(lookup), Some(iter)) => lookup(&context.buffer(), &iter),
                _ => None,
            };
            let nothing = || glib::Error::new(gio::IOErrorEnum::NotFound, "nothing to show");
            match content {
                Some(Content::Ready(w)) => {
                    display.append(&w);
                    Box::pin(async { Ok(()) })
                }
                Some(Content::Later(future)) => {
                    let display = display.clone();
                    Box::pin(async move {
                        match future.await {
                            Some(w) => {
                                display.append(&w);
                                Ok(())
                            }
                            None => Err(nothing()),
                        }
                    })
                }
                None => Box::pin(async move { Err(nothing()) }),
            }
        }
    }
}

glib::wrapper! {
    pub struct IssueHover(ObjectSubclass<imp::IssueHover>) @implements sourceview5::HoverProvider;
}

impl IssueHover {
    pub fn new(lookup: Lookup) -> IssueHover {
        let h: IssueHover = glib::Object::new();
        *h.imp().lookup.borrow_mut() = Some(lookup);
        h
    }
}

/// Errors and warnings of a line.
pub fn issues_bubble(issues: &[(bool, String)]) -> gtk::Box {
    let b = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).css_classes(["issue-bubble"]).build();
    for (is_error, msg) in issues {
        let row = gtk::Box::builder().spacing(8).build();
        let icon = if *is_error { "dialog-error-symbolic" } else { "dialog-warning-symbolic" };
        row.append(&gtk::Image::builder().icon_name(icon).valign(gtk::Align::Start).css_classes([if *is_error { "error" } else { "warning" }]).build());
        row.append(&gtk::Label::builder().label(msg).xalign(0.0).wrap(true).max_width_chars(70).selectable(true).build());
        b.append(&row);
    }
    b.append(&gtk::Label::builder().label(tr("Click the margin icon to see the problems list")).xalign(0.0).css_classes(["dim-label", "caption"]).build());
    b
}

/// What a `\\ref` points to: a title or caption, and where it is defined.
pub fn label_bubble(key: &str, found: Option<(String, String)>) -> gtk::Box {
    let b = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(4).css_classes(["ref-bubble"]).build();
    match found {
        Some((description, location)) => {
            b.append(&gtk::Label::builder().label(&description).xalign(0.0).wrap(true).max_width_chars(70).css_classes(["heading"]).build());
            b.append(&gtk::Label::builder().label(format!("{key} · {location}")).xalign(0.0).css_classes(["dim-label", "caption", "monospace"]).build());
            b.append(&gtk::Label::builder().label(tr("Ctrl+click to go there")).xalign(0.0).css_classes(["dim-label", "caption"]).build());
        }
        None => {
            b.append(&gtk::Label::builder().label(crate::i18n::trf("No \\label{{}} in the project", &[&key])).xalign(0.0).css_classes(["warning"]).build());
        }
    }
    b
}

/// A bibliography entry: authors, year and title.
pub fn cite_bubble(key: &str, entry: Option<&crate::index::BibEntry>) -> gtk::Box {
    let b = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(4).css_classes(["ref-bubble"]).build();
    match entry {
        Some(e) => {
            let who = match (e.author.is_empty(), e.year.is_empty()) {
                (false, false) => format!("{} ({})", e.author, e.year),
                (false, true) => e.author.clone(),
                (true, false) => e.year.clone(),
                (true, true) => key.to_string(),
            };
            b.append(&gtk::Label::builder().label(&who).xalign(0.0).css_classes(["heading"]).build());
            if !e.title.is_empty() {
                b.append(&gtk::Label::builder().label(&e.title).xalign(0.0).wrap(true).max_width_chars(70).build());
            }
            b.append(&gtk::Label::builder().label(format!("{key} · {}:{}", e.file.display(), e.line)).xalign(0.0).css_classes(["dim-label", "caption", "monospace"]).build());
            b.append(&gtk::Label::builder().label(tr("Ctrl+click to go there")).xalign(0.0).css_classes(["dim-label", "caption"]).build());
        }
        None => {
            b.append(&gtk::Label::builder().label(crate::i18n::trf("{} is not in any .bib file", &[&key])).xalign(0.0).css_classes(["warning"]).build());
        }
    }
    b
}
