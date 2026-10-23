//! Editor de texto: un `sourceview5::View` y un buffer por archivo abierto
//! (se conserva el historial de deshacer al cambiar de archivo), autoguardado
//! atómico, búsqueda y modo Vim opcional.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};
use std::time::SystemTime;

use gtk::prelude::*;
use gtk::glib;
use sourceview5::prelude::*;

struct Doc {
    buffer: sourceview5::Buffer,
    mtime: Option<SystemTime>,
}

type Callback = RefCell<Option<Box<dyn Fn()>>>;

pub struct Editor {
    pub widget: gtk::Box,
    view: sourceview5::View,
    root: RefCell<PathBuf>,
    docs: RefCell<HashMap<PathBuf, Doc>>,
    current: RefCell<Option<PathBuf>>,
    on_change: Callback,
    on_save: Callback,
    vim: RefCell<Option<(gtk::EventControllerKey, sourceview5::VimIMContext)>>,
    vim_label: gtk::Label,
    search_bar: gtk::SearchBar,
    search_entry: gtk::SearchEntry,
    search_settings: sourceview5::SearchSettings,
    search_ctx: RefCell<Option<sourceview5::SearchContext>>,
}

impl Editor {
    pub fn new(root: PathBuf) -> Rc<Editor> {
        let view = sourceview5::View::builder()
            .monospace(true)
            .show_line_numbers(true)
            .highlight_current_line(true)
            .auto_indent(true)
            .indent_width(2)
            .tab_width(2)
            .insert_spaces_instead_of_tabs(true)
            .smart_backspace(true)
            .wrap_mode(gtk::WrapMode::WordChar)
            .top_margin(8)
            .bottom_margin(200)
            .left_margin(6)
            .right_margin(12)
            .vexpand(true)
            .build();
        view.add_css_class("rutex-editor");

        let scroller = gtk::ScrolledWindow::builder().child(&view).vexpand(true).build();

        let search_entry = gtk::SearchEntry::builder().placeholder_text("Buscar…").width_chars(32).build();
        let search_bar = gtk::SearchBar::builder().child(&search_entry).show_close_button(true).build();
        search_bar.connect_entry(&search_entry);

        let vim_label = gtk::Label::builder().xalign(0.0).visible(false).css_classes(["vim-bar", "monospace"]).build();

        let widget = gtk::Box::new(gtk::Orientation::Vertical, 0);
        widget.append(&search_bar);
        widget.append(&scroller);
        widget.append(&vim_label);

        let search_settings = sourceview5::SearchSettings::builder().wrap_around(true).case_sensitive(false).build();

        let editor = Rc::new(Editor {
            widget,
            view,
            root: RefCell::new(root),
            docs: RefCell::default(),
            current: RefCell::default(),
            on_change: RefCell::default(),
            on_save: RefCell::default(),
            vim: RefCell::default(),
            vim_label,
            search_bar,
            search_entry,
            search_settings,
            search_ctx: RefCell::default(),
        });
        editor.connect_search();
        editor
    }

    pub fn connect_change(&self, f: impl Fn() + 'static) {
        *self.on_change.borrow_mut() = Some(Box::new(f));
    }

    pub fn connect_save(&self, f: impl Fn() + 'static) {
        *self.on_save.borrow_mut() = Some(Box::new(f));
    }

    pub fn current(&self) -> Option<PathBuf> {
        self.current.borrow().clone()
    }

    pub fn cursor_line(&self) -> u32 {
        let buffer = self.view.buffer();
        buffer.iter_at_mark(&buffer.get_insert()).line() as u32 + 1
    }

    /// Muestra un archivo (relativo a la raíz), cargándolo si hace falta.
    pub fn open(self: &Rc<Self>, rel: &Path) -> Result<(), String> {
        if self.current.borrow().as_deref() == Some(rel) {
            return Ok(());
        }
        if !self.docs.borrow().contains_key(rel) {
            let doc = self.load(rel)?;
            self.docs.borrow_mut().insert(rel.to_path_buf(), doc);
        }
        let buffer = self.docs.borrow()[rel].buffer.clone();
        self.view.set_buffer(Some(&buffer));
        *self.current.borrow_mut() = Some(rel.to_path_buf());

        let ctx = sourceview5::SearchContext::new(&buffer, Some(&self.search_settings));
        ctx.set_highlight(self.search_bar.is_search_mode());
        *self.search_ctx.borrow_mut() = Some(ctx);
        Ok(())
    }

    fn load(self: &Rc<Self>, rel: &Path) -> Result<Doc, String> {
        let abs = self.root.borrow().join(rel);
        let bytes = fs::read(&abs).map_err(|e| format!("No se pudo abrir {}: {e}", rel.display()))?;
        let text = String::from_utf8(bytes).map_err(|_| format!("{} no está en UTF-8", rel.display()))?;

        let buffer = sourceview5::Buffer::new(None);
        let lang_id = if crate::project::has_ext(rel, "bib") { "bibtex" } else { "latex" };
        let is_tex_like = ["tex", "bib", "sty", "cls", "ltx", "dtx", "bbx", "cbx", "def", "cfg"]
            .iter()
            .any(|e| crate::project::has_ext(rel, e));
        if is_tex_like {
            buffer.set_language(sourceview5::LanguageManager::default().language(lang_id).as_ref());
        }
        buffer.set_highlight_matching_brackets(true);
        self.apply_scheme(&buffer);

        buffer.begin_irreversible_action();
        buffer.set_text(&text);
        buffer.end_irreversible_action();
        buffer.set_modified(false);
        buffer.place_cursor(&buffer.start_iter());

        let weak: Weak<Editor> = Rc::downgrade(self);
        buffer.connect_changed(move |_| {
            if let Some(this) = weak.upgrade()
                && let Some(cb) = this.on_change.borrow().as_ref() {
                    cb();
                }
        });
        Ok(Doc { buffer, mtime: mtime(&abs) })
    }

    /// Guarda los buffers modificados. Devuelve los mensajes de error/aviso.
    pub fn save_all(&self) -> Vec<String> {
        let root = self.root.borrow().clone();
        let mut msgs = Vec::new();
        for (rel, doc) in self.docs.borrow_mut().iter_mut() {
            if !doc.buffer.is_modified() {
                continue;
            }
            let abs = root.join(rel);
            if doc.mtime.is_some() && mtime(&abs) != doc.mtime {
                msgs.push(format!("{} había cambiado fuera de rutex; se ha sobrescrito", rel.display()));
            }
            let (start, end) = doc.buffer.bounds();
            let text = doc.buffer.text(&start, &end, true);
            match write_atomic(&abs, text.as_bytes()) {
                Ok(()) => {
                    doc.buffer.set_modified(false);
                    doc.mtime = mtime(&abs);
                }
                Err(e) => msgs.push(format!("No se pudo guardar {}: {e}", rel.display())),
            }
        }
        msgs
    }

    /// Recarga los archivos que han cambiado en disco (git pull, otro editor…)
    /// y que no tienen cambios sin guardar aquí. Devuelve si recargó alguno.
    pub fn reload_external(&self) -> bool {
        let root = self.root.borrow().clone();
        let mut reloaded = false;
        for (rel, doc) in self.docs.borrow_mut().iter_mut() {
            let abs = root.join(rel);
            let disk = mtime(&abs);
            if disk.is_none() || disk == doc.mtime || doc.buffer.is_modified() {
                continue;
            }
            let Ok(text) = fs::read_to_string(&abs) else { continue };
            let line = doc.buffer.iter_at_mark(&doc.buffer.get_insert()).line();
            doc.buffer.begin_irreversible_action();
            doc.buffer.set_text(&text);
            doc.buffer.end_irreversible_action();
            doc.buffer.set_modified(false);
            if let Some(it) = doc.buffer.iter_at_line(line) {
                doc.buffer.place_cursor(&it);
            }
            doc.mtime = disk;
            reloaded = true;
        }
        reloaded
    }

    /// Abre `rel` y coloca el cursor en `line` (desde 1).
    pub fn goto(self: &Rc<Self>, rel: &Path, line: u32) -> Result<(), String> {
        self.open(rel)?;
        let buffer = self.view.buffer();
        if let Some(it) = buffer.iter_at_line(line.saturating_sub(1) as i32) {
            buffer.place_cursor(&it);
        }
        let view = self.view.clone();
        // Tras cambiar de buffer las alturas de línea aún no están calculadas.
        glib::idle_add_local_once(move || {
            let buffer = view.buffer();
            view.scroll_to_mark(&buffer.get_insert(), 0.0, true, 0.0, 0.35);
        });
        self.view.grab_focus();
        Ok(())
    }

    /// Vuelve a aplicar el esquema de colores (cambio de tema).
    pub fn refresh_scheme(&self) {
        for doc in self.docs.borrow().values() {
            self.apply_scheme(&doc.buffer);
        }
    }

    fn apply_scheme(&self, buffer: &sourceview5::Buffer) {
        let id = crate::theme::scheme_id();
        buffer.set_style_scheme(sourceview5::StyleSchemeManager::default().scheme(&id).as_ref());
    }

    pub fn set_vim(self: &Rc<Self>, enabled: bool) {
        if let Some((key, im)) = self.vim.borrow_mut().take() {
            self.view.remove_controller(&key);
            im.set_client_widget(None::<&gtk::Widget>);
        }
        self.vim_label.set_visible(enabled);
        if !enabled {
            return;
        }
        let im = sourceview5::VimIMContext::new();
        let key = gtk::EventControllerKey::new();
        key.set_im_context(Some(&im));
        key.set_propagation_phase(gtk::PropagationPhase::Capture);
        self.view.add_controller(key.clone());
        im.set_client_widget(Some(&self.view));
        im.bind_property("command-bar-text", &self.vim_label, "label").sync_create().build();
        let weak = Rc::downgrade(self);
        im.connect_write(move |_, _, _| {
            if let Some(this) = weak.upgrade()
                && let Some(cb) = this.on_save.borrow().as_ref() {
                    cb();
                }
        });
        *self.vim.borrow_mut() = Some((key, im));
    }

    pub fn find(&self) {
        let buffer = self.view.buffer();
        if let Some((s, e)) = buffer.selection_bounds()
            && s.line() == e.line() {
                self.search_entry.set_text(&buffer.text(&s, &e, false));
            }
        self.search_bar.set_search_mode(true);
        self.search_entry.grab_focus();
    }

    fn connect_search(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.search_entry.connect_search_changed(move |entry| {
            let Some(this) = weak.upgrade() else { return };
            let text = entry.text();
            this.search_settings.set_search_text((!text.is_empty()).then_some(text.as_str()));
            this.search_step(true, true);
        });
        let weak = Rc::downgrade(self);
        self.search_entry.connect_activate(move |_| {
            if let Some(this) = weak.upgrade() {
                this.search_step(true, false);
            }
        });
        let weak = Rc::downgrade(self);
        self.search_entry.connect_next_match(move |_| {
            if let Some(this) = weak.upgrade() {
                this.search_step(true, false);
            }
        });
        let weak = Rc::downgrade(self);
        self.search_entry.connect_previous_match(move |_| {
            if let Some(this) = weak.upgrade() {
                this.search_step(false, false);
            }
        });
        let weak = Rc::downgrade(self);
        self.search_bar.connect_search_mode_enabled_notify(move |bar| {
            let Some(this) = weak.upgrade() else { return };
            if let Some(ctx) = this.search_ctx.borrow().as_ref() {
                ctx.set_highlight(bar.is_search_mode());
            }
            if !bar.is_search_mode() {
                this.view.grab_focus();
            }
        });
    }

    fn search_step(&self, forward: bool, from_start_of_selection: bool) {
        let ctx = self.search_ctx.borrow();
        let Some(ctx) = ctx.as_ref() else { return };
        let buffer = self.view.buffer();
        let insert = buffer.iter_at_mark(&buffer.get_insert());
        let (sel_start, sel_end) = buffer.selection_bounds().unwrap_or((insert, insert));
        let found = if forward {
            ctx.forward(if from_start_of_selection { &sel_start } else { &sel_end })
        } else {
            ctx.backward(&sel_start)
        };
        if let Some((mut s, e, _)) = found {
            buffer.select_range(&s, &e);
            self.view.scroll_to_iter(&mut s, 0.1, false, 0.0, 0.0);
        }
    }
}

fn mtime(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Escribe en un temporal y lo renombra: nunca queda un archivo a medias.
fn write_atomic(path: &Path, data: &[u8]) -> std::io::Result<()> {
    if fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
        return fs::write(path, data);
    }
    let name = path.file_name().unwrap().to_string_lossy();
    let tmp = path.with_file_name(format!(".{name}.rutex~"));
    fs::write(&tmp, data)?;
    if let Ok(meta) = fs::metadata(path) {
        let _ = fs::set_permissions(&tmp, meta.permissions());
    }
    fs::rename(&tmp, path)
}
