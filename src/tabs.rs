//! Barra de pestañas propia sobre un `adw::TabView`: más compacta y plana que
//! la de libadwaita, con el botón de cerrar solo al pasar el ratón (o en la
//! activa), subrayado de acento y clic central para cerrar.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, glib};

struct Tab {
    page: adw::TabPage,
    root: gtk::Box,
}

pub struct TabStrip {
    pub widget: gtk::ScrolledWindow,
    row: gtk::Box,
    view: adw::TabView,
    tabs: RefCell<Vec<Tab>>,
}

impl TabStrip {
    pub fn new(view: &adw::TabView) -> Rc<TabStrip> {
        let row = gtk::Box::builder().spacing(2).css_classes(["rtabs-row"]).build();
        let widget = gtk::ScrolledWindow::builder()
            .child(&row)
            .vscrollbar_policy(gtk::PolicyType::Never)
            .hscrollbar_policy(gtk::PolicyType::External)
            .css_classes(["rtabs"])
            .build();
        let this = Rc::new(TabStrip { widget, row, view: view.clone(), tabs: RefCell::default() });

        let weak = Rc::downgrade(&this);
        view.connect_page_attached(move |_, page, pos| {
            if let Some(this) = weak.upgrade() {
                this.add(page, pos);
            }
        });
        let weak = Rc::downgrade(&this);
        view.connect_page_detached(move |_, page, _| {
            if let Some(this) = weak.upgrade() {
                this.remove(page);
            }
        });
        let weak = Rc::downgrade(&this);
        view.connect_selected_page_notify(move |_| {
            if let Some(this) = weak.upgrade() {
                this.sync_selection();
            }
        });

        // La rueda del ratón desplaza las pestañas en horizontal.
        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
        let w = this.widget.downgrade();
        scroll.connect_scroll(move |_, dx, dy| {
            if let Some(sw) = w.upgrade() {
                let adj = sw.hadjustment();
                adj.set_value(adj.value() + (dx + dy) * 40.0);
            }
            glib::Propagation::Stop
        });
        this.widget.add_controller(scroll);
        this
    }

    fn add(self: &Rc<Self>, page: &adw::TabPage, pos: i32) {
        let icon = gtk::Image::new();
        if let Some(gicon) = page.icon() {
            icon.set_from_gicon(&gicon);
        }
        let label = gtk::Label::builder().label(page.title()).css_classes(["rtab-title"]).build();
        let close = gtk::Button::builder()
            .icon_name("window-close-symbolic")
            .tooltip_text("Cerrar (Ctrl+W)")
            .css_classes(["flat", "rtab-close"])
            .valign(gtk::Align::Center)
            .build();
        let root = gtk::Box::builder().spacing(6).css_classes(["rtab"]).build();
        root.append(&icon);
        root.append(&label);
        root.append(&close);
        root.set_tooltip_markup(page.tooltip().as_deref());

        page.bind_property("title", &label, "label").build();
        // El icono se pone después de añadir la página.
        let img = icon.clone();
        page.connect_icon_notify(move |p| {
            if let Some(gicon) = p.icon() {
                img.set_from_gicon(&gicon);
            }
        });
        page.bind_property("tooltip", &root, "tooltip-markup").build();

        let (view, p) = (self.view.clone(), page.clone());
        close.connect_clicked(move |_| view.close_page(&p));

        let click = gtk::GestureClick::builder().button(0).build();
        let (view, p) = (self.view.clone(), page.clone());
        click.connect_pressed(move |g, _, _, _| {
            if g.current_button() == gdk::BUTTON_MIDDLE {
                view.close_page(&p);
            } else {
                view.set_selected_page(&p);
            }
        });
        root.add_controller(click);

        // Insertar en su posición.
        let tabs = self.tabs.borrow();
        let before = if pos > 0 { tabs.get(pos as usize - 1).map(|t| t.root.clone()) } else { None };
        drop(tabs);
        self.row.insert_child_after(&root, before.as_ref());
        let len = self.tabs.borrow().len();
        self.tabs.borrow_mut().insert((pos as usize).min(len), Tab { page: page.clone(), root });
        self.sync_selection();
    }

    fn remove(&self, page: &adw::TabPage) {
        let mut tabs = self.tabs.borrow_mut();
        if let Some(i) = tabs.iter().position(|t| t.page == *page) {
            let tab = tabs.remove(i);
            self.row.remove(&tab.root);
        }
    }

    fn sync_selection(self: &Rc<Self>) {
        let selected = self.view.selected_page();
        let mut target = None;
        for t in self.tabs.borrow().iter() {
            if Some(&t.page) == selected.as_ref() {
                t.root.add_css_class("selected");
                target = Some(t.root.clone());
            } else {
                t.root.remove_css_class("selected");
            }
        }
        // Hacer visible la pestaña activa cuando ya tiene tamaño.
        if let Some(tab) = target {
            let sw = self.widget.clone();
            let row = self.row.clone();
            glib::idle_add_local_once(move || {
                let Some(b) = tab.compute_bounds(&row) else { return };
                let adj = sw.hadjustment();
                let (x, w) = (b.x() as f64, b.width() as f64);
                if x < adj.value() {
                    adj.set_value(x);
                } else if x + w > adj.value() + adj.page_size() {
                    adj.set_value(x + w - adj.page_size());
                }
            });
        }
    }
}
