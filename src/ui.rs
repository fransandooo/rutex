//! Piezas de interfaz reutilizables.

use gtk::prelude::*;
use gtk::{gdk, glib};

pub enum MenuEntry {
    Item { icon: &'static str, label: String, action: &'static str, target: glib::Variant, destructive: bool },
    Separator,
}

impl MenuEntry {
    pub fn item(icon: &'static str, label: impl Into<String>, action: &'static str, target: &str) -> MenuEntry {
        MenuEntry::Item { icon, label: label.into(), action, target: target.to_variant(), destructive: false }
    }

    pub fn danger(icon: &'static str, label: impl Into<String>, action: &'static str, target: &str) -> MenuEntry {
        MenuEntry::Item { icon, label: label.into(), action, target: target.to_variant(), destructive: true }
    }
}

/// Menú contextual propio: botones planos con icono, sin desplazamiento y con
/// la estética de la aplicación (en vez del `PopoverMenu` por defecto).
pub fn context_menu(parent: &impl IsA<gtk::Widget>, x: f64, y: f64, entries: Vec<MenuEntry>) {
    let parent = parent.as_ref();
    let popover = gtk::Popover::builder().has_arrow(false).halign(gtk::Align::Start).css_classes(["context-menu"]).build();
    let list = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(1).build();
    for entry in entries {
        match entry {
            MenuEntry::Separator => list.append(&gtk::Separator::new(gtk::Orientation::Horizontal)),
            MenuEntry::Item { icon, label, action, target, destructive } => {
                let row = gtk::Box::builder().spacing(10).build();
                row.append(&gtk::Image::from_icon_name(icon));
                row.append(&gtk::Label::builder().label(label).xalign(0.0).hexpand(true).build());
                let button = gtk::Button::builder().child(&row).css_classes(["flat", "menu-item"]).build();
                if destructive {
                    button.add_css_class("destructive");
                }
                let (pop, p) = (popover.downgrade(), parent.downgrade());
                button.connect_clicked(move |_| {
                    if let Some(pop) = pop.upgrade() {
                        pop.popdown();
                    }
                    if let Some(p) = p.upgrade() {
                        let _ = p.activate_action(action, Some(&target));
                    }
                });
                list.append(&button);
            }
        }
    }
    popover.set_child(Some(&list));
    popover.set_parent(parent);
    popover.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
    popover.connect_closed(|p| {
        let p = p.clone();
        glib::idle_add_local_once(move || p.unparent());
    });
    popover.popup();
}
