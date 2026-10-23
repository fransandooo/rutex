//! Bocadillo al pasar el ratón por una línea con errores o avisos.

use std::cell::RefCell;
use std::pin::Pin;
use std::rc::Rc;

use gtk::glib::subclass::prelude::*;
use gtk::prelude::*;
use gtk::{gio, glib};
use sourceview5::subclass::prelude::*;
use crate::i18n::tr;

/// Dado un buffer y una línea (desde 1), devuelve sus problemas: (es_error, mensaje).
pub type Lookup = Rc<dyn Fn(&sourceview5::Buffer, u32) -> Vec<(bool, String)>>;

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
            let issues = match (self.lookup.borrow().as_ref(), context.iter()) {
                (Some(lookup), Some(iter)) => lookup(&context.buffer(), iter.line() as u32 + 1),
                _ => Vec::new(),
            };
            let found = !issues.is_empty();
            if found {
                display.append(&bubble(&issues));
            }
            Box::pin(async move {
                if found { Ok(()) } else { Err(glib::Error::new(gio::IOErrorEnum::NotFound, "no issues")) }
            })
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

fn bubble(issues: &[(bool, String)]) -> gtk::Box {
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
