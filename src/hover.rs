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

/// A bubble card: a small header (icon + kind), a title, and an optional
/// footer. Returns (card, body) so callers can add content between them.
pub fn card(icon: &str, kind: &str, title: Option<&str>) -> (gtk::Box, gtk::Box) {
    let card = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).css_classes(["bubble"]).build();
    let header = gtk::Box::builder().spacing(6).css_classes(["bubble-kind"]).build();
    header.append(&gtk::Image::from_icon_name(icon));
    header.append(&gtk::Label::builder().label(kind).xalign(0.0).build());
    card.append(&header);
    if let Some(t) = title {
        card.append(&gtk::Label::builder().label(t).xalign(0.0).wrap(true).wrap_mode(gtk::pango::WrapMode::WordChar).max_width_chars(52).css_classes(["bubble-title"]).build());
    }
    let body = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).build();
    card.append(&body);
    (card, body)
}

pub fn footer(text: &str) -> gtk::Label {
    gtk::Label::builder().label(text).xalign(0.0).wrap(true).max_width_chars(60).css_classes(["bubble-foot"]).build()
}

fn text(t: &str) -> gtk::Label {
    gtk::Label::builder().label(t).xalign(0.0).wrap(true).wrap_mode(gtk::pango::WrapMode::WordChar).max_width_chars(52).selectable(true).css_classes(["bubble-text"]).build()
}

/// Errors and warnings of a line.
pub fn issues_bubble(issues: &[(bool, String)]) -> gtk::Box {
    let errors = issues.iter().filter(|i| i.0).count();
    let (icon, kind) = if errors > 0 { ("dialog-error-symbolic", tr("Error")) } else { ("dialog-warning-symbolic", tr("Warning")) };
    let (card, body) = card(icon, kind, None);
    card.add_css_class(if errors > 0 { "error" } else { "warning" });
    for (is_error, msg) in issues {
        let row = gtk::Box::builder().spacing(8).build();
        if issues.len() > 1 {
            row.append(&gtk::Image::builder().icon_name(if *is_error { "dialog-error-symbolic" } else { "dialog-warning-symbolic" }).valign(gtk::Align::Start).build());
        }
        let col = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(4).build();
        col.append(&text(msg));
        if let Some((what, fix)) = crate::explain::explain(msg) {
            col.append(&gtk::Label::builder().label(what).xalign(0.0).wrap(true).max_width_chars(52).css_classes(["bubble-explain"]).build());
            col.append(&gtk::Label::builder().label(crate::i18n::trf("How to fix: {}", &[&fix])).xalign(0.0).wrap(true).max_width_chars(52).css_classes(["bubble-fix"]).build());
        }
        row.append(&col);
        body.append(&row);
    }
    card.append(&footer(tr("Click the margin icon to see the problems list")));
    card
}

/// What a `\\ref` points to. `found` = (kind, description, location).
pub fn label_bubble(key: &str, found: Option<(&str, String, String)>) -> gtk::Box {
    match found {
        Some((kind, description, location)) => {
            let (card, _) = card("mail-attachment-symbolic", kind, Some(&description));
            card.append(&footer(&format!("{key} · {location} · {}", tr("Ctrl+click to go there"))));
            card
        }
        None => {
            let (card, body) = card("dialog-warning-symbolic", tr("Reference"), Some(key));
            card.add_css_class("warning");
            body.append(&text(&crate::i18n::trf("No \\label{{}} in the project", &[&key])));
            card
        }
    }
}

/// A bibliography entry: title, authors and year.
pub fn cite_bubble(key: &str, entry: Option<&crate::index::BibEntry>) -> gtk::Box {
    match entry {
        Some(e) => {
            let title = if e.title.is_empty() { key.to_string() } else { e.title.clone() };
            let (card, body) = card("x-office-address-book-symbolic", tr("Citation"), Some(&title));
            let who = match (e.author.is_empty(), e.year.is_empty()) {
                (false, false) => format!("{} ({})", e.author, e.year),
                (false, true) => e.author.clone(),
                (true, false) => e.year.clone(),
                (true, true) => String::new(),
            };
            if !who.is_empty() {
                body.append(&text(&who));
            }
            card.append(&footer(&format!("{key} · {}:{} · {}", e.file.display(), e.line, tr("Ctrl+click to go there"))));
            card
        }
        None => {
            let (card, body) = card("dialog-warning-symbolic", tr("Citation"), Some(key));
            card.add_css_class("warning");
            body.append(&text(&crate::i18n::trf("{} is not in any .bib file", &[&key])));
            card
        }
    }
}
