//! Diálogo «Nuevo proyecto»: nombre, ubicación y plantilla.

use std::cell::{Cell, RefCell};
use std::fs;
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};

use crate::templates::{self, TEMPLATES};

fn default_location() -> PathBuf {
    glib::user_special_dir(glib::UserDirectory::Documents).unwrap_or_else(glib::home_dir)
}

fn tilde(path: &std::path::Path) -> String {
    match path.strip_prefix(glib::home_dir()) {
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

pub fn show(parent: &impl IsA<gtk::Widget>, on_create: impl Fn(PathBuf) + 'static) {
    let location = Rc::new(RefCell::new(default_location()));
    let chosen = Rc::new(Cell::new(1usize)); // TFM por defecto

    let name = adw::EntryRow::builder().title("Nombre de la carpeta").text("mi-proyecto").build();
    let place = adw::ActionRow::builder().title("Ubicación").subtitle(tilde(&location.borrow())).build();
    let change = gtk::Button::builder().label("Cambiar…").valign(gtk::Align::Center).css_classes(["flat"]).build();
    place.add_suffix(&change);
    place.set_activatable_widget(Some(&change));
    let where_group = adw::PreferencesGroup::new();
    where_group.add(&name);
    where_group.add(&place);

    let tpl_group = adw::PreferencesGroup::builder().title("Plantilla").build();
    let mut first_check: Option<gtk::CheckButton> = None;
    for (i, t) in TEMPLATES.iter().enumerate() {
        let check = gtk::CheckButton::builder().valign(gtk::Align::Center).active(i == chosen.get()).build();
        if let Some(group) = &first_check {
            check.set_group(Some(group));
        } else {
            first_check = Some(check.clone());
        }
        let c = chosen.clone();
        check.connect_toggled(move |b| {
            if b.is_active() {
                c.set(i);
            }
        });
        let row = adw::ActionRow::builder().title(t.name).subtitle(t.description).activatable_widget(&check).build();
        row.add_prefix(&check);
        row.add_suffix(&gtk::Image::from_icon_name(t.icon));
        tpl_group.add(&row);
    }

    let error = gtk::Label::builder().css_classes(["error"]).wrap(true).visible(false).build();
    let create = gtk::Button::builder().label("Crear proyecto").halign(gtk::Align::Center).css_classes(["pill", "suggested-action"]).build();

    let content = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(18).margin_top(12).margin_bottom(24).margin_start(18).margin_end(18).build();
    content.append(&where_group);
    content.append(&tpl_group);
    content.append(&error);
    content.append(&create);

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    toolbar.set_content(Some(&gtk::ScrolledWindow::builder().child(&content).propagate_natural_height(true).build()));
    let dialog = adw::Dialog::builder().title("Nuevo proyecto").content_width(560).child(&toolbar).build();

    let (loc, sub, dlg) = (location.clone(), place.clone(), dialog.clone());
    change.connect_clicked(move |_| {
        let fd = gtk::FileDialog::builder().title("Ubicación del proyecto").initial_folder(&gio::File::for_path(&*loc.borrow())).build();
        let (loc, sub) = (loc.clone(), sub.clone());
        let parent = dlg.root().and_downcast::<gtk::Window>();
        fd.select_folder(parent.as_ref(), gio::Cancellable::NONE, move |res| {
            if let Some(path) = res.ok().and_then(|f| f.path()) {
                sub.set_subtitle(&tilde(&path));
                *loc.borrow_mut() = path;
            }
        });
    });

    let on_create = Rc::new(on_create);
    let name_row = name.clone();
    let dlg = dialog.clone();
    let do_create = move || {
        let folder = name.text().trim().to_string();
        let fail = |msg: &str| {
            error.set_label(msg);
            error.set_visible(true);
        };
        if folder.is_empty() || folder.contains('/') || folder == "." || folder == ".." {
            fail("Escribe un nombre de carpeta válido (sin «/»).");
            return;
        }
        let root = location.borrow().join(&folder);
        if root.exists() && fs::read_dir(&root).map(|mut d| d.next().is_some()).unwrap_or(true) {
            fail("Esa carpeta ya existe y no está vacía. Elige otro nombre o usa «Abrir carpeta».");
            return;
        }
        let title = folder.replace(['-', '_'], " ");
        let made = fs::create_dir_all(&root).and_then(|_| templates::write(&TEMPLATES[chosen.get()], &root, &title));
        match made {
            Ok(()) => {
                dlg.close();
                on_create(root);
            }
            Err(e) => fail(&format!("No se pudo crear el proyecto: {e}")),
        }
    };
    let do_create = Rc::new(do_create);
    let f = do_create.clone();
    create.connect_clicked(move |_| f());
    // Intro en el nombre también crea.
    name_row.connect_entry_activated(move |_| do_create());

    dialog.present(Some(parent));
}
