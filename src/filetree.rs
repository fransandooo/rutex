//! Árbol de archivos del proyecto con carpetas desplegables y menú contextual.
//! Las carpetas se listan solo cuando hace falta (al desplegarlas) y el árbol
//! se reconstruye únicamente si algo ha cambiado en disco, conservando las
//! carpetas abiertas.

use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gdk, gio, glib};

use crate::project;

const SKIP_DIRS: &[&str] = &["node_modules", "target", "__pycache__", "_minted"];

type OpenCallback = RefCell<Option<Box<dyn Fn(&Path)>>>;

pub struct FileTree {
    pub widget: gtk::ScrolledWindow,
    root: PathBuf,
    store: gio::ListStore,
    tree: gtk::TreeListModel,
    selection: gtk::SingleSelection,
    list: gtk::ListView,
    main: RefCell<PathBuf>,
    signature: RefCell<Vec<PathBuf>>,
    on_open: OpenCallback,
}

impl FileTree {
    pub fn new(root: PathBuf, main: PathBuf) -> Rc<FileTree> {
        let store = gio::ListStore::new::<gtk::StringObject>();
        let r = root.clone();
        let tree = gtk::TreeListModel::new(store.clone(), false, false, move |item| {
            let rel = item.downcast_ref::<gtk::StringObject>()?.string();
            let abs = r.join(rel.as_str());
            abs.is_dir().then(|| children_model(&r, Path::new(rel.as_str())).upcast())
        });
        let selection = gtk::SingleSelection::builder().model(&tree).autoselect(false).can_unselect(true).build();
        let factory = gtk::SignalListItemFactory::new();
        let list = gtk::ListView::builder()
            .model(&selection)
            .factory(&factory)
            .single_click_activate(true)
            .css_classes(["navigation-sidebar", "file-tree"])
            .build();
        let widget = gtk::ScrolledWindow::builder().child(&list).hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).build();

        let this = Rc::new(FileTree {
            widget,
            root,
            store,
            tree,
            selection,
            list,
            main: RefCell::new(main),
            signature: RefCell::default(),
            on_open: RefCell::default(),
        });
        this.setup_factory(&factory);

        let weak = Rc::downgrade(&this);
        this.list.connect_activate(move |_, pos| {
            let Some(this) = weak.upgrade() else { return };
            let Some(row) = this.tree.row(pos) else { return };
            if row.is_expandable() {
                row.set_expanded(!row.is_expanded());
            } else if let Some(rel) = rel_of(&row) {
                if let Some(cb) = this.on_open.borrow().as_ref() {
                    cb(&rel);
                }
            }
        });

        // Clic derecho en una zona vacía: acciones sobre la raíz del proyecto.
        let click = gtk::GestureClick::builder().button(gdk::BUTTON_SECONDARY).build();
        let weak = Rc::downgrade(&this);
        click.connect_pressed(move |gesture, _, x, y| {
            if let Some(this) = weak.upgrade() {
                gesture.set_state(gtk::EventSequenceState::Claimed);
                this.popup_menu(&this.widget.clone().upcast(), Path::new(""), true, x, y);
            }
        });
        this.widget.add_controller(click);

        this.refresh();
        this
    }

    /// Al activar un archivo (no carpeta). La ruta es relativa a la raíz.
    pub fn connect_open(&self, f: impl Fn(&Path) + 'static) {
        *self.on_open.borrow_mut() = Some(Box::new(f));
    }

    pub fn set_main(&self, main: PathBuf) {
        *self.main.borrow_mut() = main;
        self.rebuild();
    }

    fn setup_factory(self: &Rc<Self>, factory: &gtk::SignalListItemFactory) {
        factory.connect_setup(|_, obj| {
            let item = obj.downcast_ref::<gtk::ListItem>().unwrap();
            let icon = gtk::Image::new();
            let label = gtk::Label::builder().xalign(0.0).hexpand(true).ellipsize(gtk::pango::EllipsizeMode::Middle).build();
            let star = gtk::Image::builder().icon_name("starred-symbolic").tooltip_text("Archivo principal").css_classes(["main-star"]).build();
            let row = gtk::Box::builder().spacing(8).build();
            row.append(&icon);
            row.append(&label);
            row.append(&star);
            let expander = gtk::TreeExpander::builder().child(&row).indent_for_icon(true).build();
            item.set_child(Some(&expander));
        });

        let weak = Rc::downgrade(self);
        factory.connect_bind(move |_, obj| {
            let Some(this) = weak.upgrade() else { return };
            let item = obj.downcast_ref::<gtk::ListItem>().unwrap();
            let Some(row) = item.item().and_downcast::<gtk::TreeListRow>() else { return };
            let expander = item.child().and_downcast::<gtk::TreeExpander>().unwrap();
            expander.set_list_row(Some(&row));
            let Some(rel) = rel_of(&row) else { return };
            let content = expander.child().and_downcast::<gtk::Box>().unwrap();
            let icon = content.first_child().and_downcast::<gtk::Image>().unwrap();
            let label = icon.next_sibling().and_downcast::<gtk::Label>().unwrap();
            let star = label.next_sibling().unwrap();

            let is_dir = row.is_expandable();
            icon.set_icon_name(Some(if is_dir { "folder-symbolic" } else { crate::editor::file_icon(&rel) }));
            label.set_label(&rel.file_name().unwrap_or_default().to_string_lossy());
            label.set_tooltip_text(Some(&rel.display().to_string()));
            star.set_visible(*this.main.borrow() == rel);
            if is_dir {
                content.add_css_class("folder");
            } else {
                content.remove_css_class("folder");
            }
            unsafe { expander.set_data("rutex-rel", (rel, is_dir)) };
        });

        // Menú contextual por fila.
        let weak = Rc::downgrade(self);
        factory.connect_setup(move |_, obj| {
            let item = obj.downcast_ref::<gtk::ListItem>().unwrap();
            let expander = item.child().unwrap();
            let click = gtk::GestureClick::builder().button(gdk::BUTTON_SECONDARY).build();
            let w = weak.clone();
            click.connect_pressed(move |gesture, _, x, y| {
                let Some(this) = w.upgrade() else { return };
                let Some(widget) = gesture.widget() else { return };
                let data = unsafe { widget.data::<(PathBuf, bool)>("rutex-rel").map(|p| p.as_ref().clone()) };
                if let Some((rel, is_dir)) = data {
                    gesture.set_state(gtk::EventSequenceState::Claimed);
                    this.popup_menu(&widget, &rel, is_dir, x, y);
                }
            });
            expander.add_controller(click);
        });
    }

    fn popup_menu(&self, parent: &gtk::Widget, rel: &Path, is_dir: bool, x: f64, y: f64) {
        let target = rel.to_string_lossy().to_string();
        let dir = if is_dir { target.clone() } else { rel.parent().map(|p| p.to_string_lossy().to_string()).unwrap_or_default() };
        let item = |label: &str, action: &str, value: &str| {
            let it = gio::MenuItem::new(Some(label), None);
            it.set_action_and_target_value(Some(action), Some(&value.to_variant()));
            it
        };

        let menu = gio::Menu::new();
        let create = gio::Menu::new();
        create.append_item(&item("Nuevo archivo…", "win.tree-new-file", &dir));
        create.append_item(&item("Nueva carpeta…", "win.tree-new-folder", &dir));
        menu.append_section(None, &create);
        if !target.is_empty() {
            let edit = gio::Menu::new();
            edit.append_item(&item("Renombrar…", "win.tree-rename", &target));
            if !is_dir && project::has_ext(rel, "tex") {
                edit.append_item(&item("Usar como archivo principal", "win.tree-set-main", &target));
            }
            menu.append_section(None, &edit);
        }
        let misc = gio::Menu::new();
        misc.append_item(&item("Copiar ruta", "win.tree-copy-path", &target));
        misc.append_item(&item("Mostrar en el gestor de archivos", "win.tree-reveal", &target));
        menu.append_section(None, &misc);
        if !target.is_empty() {
            let danger = gio::Menu::new();
            danger.append_item(&item("Mover a la papelera", "win.tree-trash", &target));
            menu.append_section(None, &danger);
        }

        let popover = gtk::PopoverMenu::from_model(Some(&menu));
        popover.set_parent(parent);
        popover.set_has_arrow(false);
        popover.set_halign(gtk::Align::Start);
        popover.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        popover.connect_closed(|p| {
            let p = p.clone();
            glib::idle_add_local_once(move || p.unparent());
        });
        popover.popup();
    }

    /// Relee la carpeta; solo reconstruye el árbol si algo cambió.
    pub fn refresh(&self) {
        let sig = signature(&self.root);
        if *self.signature.borrow() == sig {
            return;
        }
        *self.signature.borrow_mut() = sig;
        self.rebuild();
    }

    fn rebuild(&self) {
        let expanded: Vec<PathBuf> = (0..self.tree.n_items())
            .filter_map(|i| self.tree.row(i))
            .filter(|r| r.is_expanded())
            .filter_map(|r| rel_of(&r))
            .collect();
        let selected = self.selection.selected_item().and_downcast::<gtk::TreeListRow>().and_then(|r| rel_of(&r));

        let fresh = children_model(&self.root, Path::new(""));
        let items: Vec<glib::Object> = (0..fresh.n_items()).filter_map(|i| fresh.item(i)).collect();
        self.store.splice(0, self.store.n_items(), &items);

        let mut i = 0;
        while i < self.tree.n_items() {
            if let Some(row) = self.tree.row(i)
                && rel_of(&row).is_some_and(|r| expanded.contains(&r))
            {
                row.set_expanded(true);
            }
            i += 1;
        }
        if let Some(rel) = selected {
            self.select(&rel);
        }
    }

    /// Selecciona (y hace visible) un archivo, desplegando sus carpetas.
    pub fn select(&self, rel: &Path) {
        let mut ancestors: Vec<PathBuf> = rel.ancestors().skip(1).filter(|a| !a.as_os_str().is_empty()).map(Path::to_path_buf).collect();
        ancestors.reverse();
        for dir in ancestors {
            if let Some(row) = self.find_row(&dir) {
                row.set_expanded(true);
            }
        }
        match self.find_position(rel) {
            Some(pos) => {
                self.selection.set_selected(pos);
                self.list.scroll_to(pos, gtk::ListScrollFlags::NONE, None);
            }
            None => self.selection.set_selected(gtk::INVALID_LIST_POSITION),
        }
    }

    fn find_position(&self, rel: &Path) -> Option<u32> {
        (0..self.tree.n_items()).find(|i| self.tree.row(*i).and_then(|r| rel_of(&r)).as_deref() == Some(rel))
    }

    fn find_row(&self, rel: &Path) -> Option<gtk::TreeListRow> {
        self.find_position(rel).and_then(|i| self.tree.row(i))
    }
}

fn rel_of(row: &gtk::TreeListRow) -> Option<PathBuf> {
    row.item().and_downcast::<gtk::StringObject>().map(|s| PathBuf::from(s.string().as_str()))
}

fn visible(name: &str) -> bool {
    !name.starts_with('.') && !SKIP_DIRS.contains(&name)
}

/// Contenido de una carpeta: primero subcarpetas, luego archivos, sin distinguir mayúsculas.
fn children_model(root: &Path, rel: &Path) -> gio::ListStore {
    let store = gio::ListStore::new::<gtk::StringObject>();
    let Ok(entries) = fs::read_dir(root.join(rel)) else { return store };
    let mut items: Vec<(bool, String, PathBuf)> = entries
        .flatten()
        .filter(|e| visible(&e.file_name().to_string_lossy()))
        .map(|e| {
            let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
            let name = e.file_name().to_string_lossy().to_lowercase();
            (!is_dir, name, rel.join(e.file_name()))
        })
        .collect();
    items.sort();
    for (_, _, path) in items {
        store.append(&gtk::StringObject::new(&path.to_string_lossy()));
    }
    store
}

/// Lista de todas las rutas visibles; sirve para saber si hay que reconstruir.
fn signature(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![PathBuf::new()];
    while let Some(rel) = stack.pop() {
        let Ok(entries) = fs::read_dir(root.join(&rel)) else { continue };
        for e in entries.flatten() {
            if !visible(&e.file_name().to_string_lossy()) {
                continue;
            }
            let path = rel.join(e.file_name());
            if e.file_type().is_ok_and(|t| t.is_dir()) && out.len() < 20_000 {
                stack.push(path.clone());
            }
            out.push(path);
        }
    }
    out.sort();
    out
}
