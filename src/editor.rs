//! Editor con pestañas: una vista de GtkSourceView por archivo abierto (cada una
//! conserva su historial de deshacer y su posición), autoguardado atómico,
//! búsqueda y modo Vim opcional.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};
use std::time::SystemTime;

use adw::prelude::*;
use gtk::{gio, glib};
use sourceview5::prelude::*;

use crate::completion::{LatexProvider, SharedData};
use crate::config;
use crate::keys;

struct Doc {
    rel: PathBuf,
    buffer: sourceview5::Buffer,
    view: sourceview5::View,
    page: adw::TabPage,
    mtime: Option<SystemTime>,
    vim: Option<(gtk::EventControllerKey, sourceview5::VimIMContext)>,
}

type Callback = RefCell<Option<Box<dyn Fn()>>>;

pub struct Editor {
    pub widget: gtk::Box,
    /// "Ln 12, Col 4" del archivo actual (para la barra de estado).
    pub position_label: gtk::Label,
    /// Línea de comandos del modo Vim (para la barra de estado).
    pub vim_label: gtk::Label,
    tab_view: adw::TabView,
    stack: gtk::Stack,
    root: PathBuf,
    completion: LatexProvider,
    key_options: Rc<keys::Options>,
    /// Problemas de la última compilación por archivo: (línea, es_error, mensaje).
    issues: RefCell<HashMap<PathBuf, Vec<(u32, bool, String)>>>,
    error_attrs: sourceview5::MarkAttributes,
    warning_attrs: sourceview5::MarkAttributes,
    docs: RefCell<Vec<Doc>>,
    on_change: Callback,
    on_save: Callback,
    on_switch: Callback,
    vim_binding: RefCell<Option<glib::Binding>>,
    search_bar: gtk::SearchBar,
    search_entry: gtk::SearchEntry,
    search_settings: sourceview5::SearchSettings,
    search_ctx: RefCell<Option<sourceview5::SearchContext>>,
}

impl Editor {
    pub fn new(root: PathBuf, data: SharedData) -> Rc<Editor> {
        let tab_view = adw::TabView::new();
        tab_view.set_vexpand(true);
        let tab_bar = adw::TabBar::builder().view(&tab_view).autohide(false).expand_tabs(false).css_classes(["rutex-tabs"]).build();

        let empty = adw::StatusPage::builder()
            .icon_name("text-x-generic-symbolic")
            .title("Ningún archivo abierto")
            .description("Elige uno en la barra lateral o pulsa Ctrl+P")
            .css_classes(["compact"])
            .build();
        let stack = gtk::Stack::builder().vexpand(true).build();
        stack.add_named(&tab_view, Some("tabs"));
        stack.add_named(&empty, Some("empty"));

        let search_entry = gtk::SearchEntry::builder().placeholder_text("Buscar en el archivo…").width_chars(32).build();
        let search_bar = gtk::SearchBar::builder().child(&search_entry).show_close_button(true).build();
        search_bar.connect_entry(&search_entry);

        let widget = gtk::Box::new(gtk::Orientation::Vertical, 0);
        widget.append(&tab_bar);
        widget.append(&search_bar);
        widget.append(&stack);

        let search_settings = sourceview5::SearchSettings::builder().wrap_around(true).case_sensitive(false).build();

        let editor = Rc::new(Editor {
            widget,
            position_label: gtk::Label::new(None),
            vim_label: gtk::Label::builder().css_classes(["monospace"]).visible(false).build(),
            tab_view,
            stack,
            root,
            completion: LatexProvider::new(data),
            issues: RefCell::default(),
            error_attrs: sourceview5::MarkAttributes::builder().icon_name("dialog-error-symbolic").build(),
            warning_attrs: sourceview5::MarkAttributes::builder().icon_name("dialog-warning-symbolic").build(),
            key_options: Rc::new(keys::Options { autopairs: Cell::new(config::get().autopairs), vim: Cell::new(config::get().vim) }),
            docs: RefCell::default(),
            on_change: RefCell::default(),
            on_save: RefCell::default(),
            on_switch: RefCell::default(),
            vim_binding: RefCell::default(),
            search_bar,
            search_entry,
            search_settings,
            search_ctx: RefCell::default(),
        });
        editor.setup_marks();
        editor.connect_tabs();
        editor.connect_search();
        editor.update_empty();
        editor
    }

    pub fn connect_change(&self, f: impl Fn() + 'static) {
        *self.on_change.borrow_mut() = Some(Box::new(f));
    }

    pub fn connect_save(&self, f: impl Fn() + 'static) {
        *self.on_save.borrow_mut() = Some(Box::new(f));
    }

    /// Cambio de pestaña (archivo actual distinto).
    pub fn connect_switch(&self, f: impl Fn() + 'static) {
        *self.on_switch.borrow_mut() = Some(Box::new(f));
    }

    fn emit(cb: &Callback) {
        if let Some(cb) = cb.borrow().as_ref() {
            cb();
        }
    }

    fn connect_tabs(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.tab_view.connect_selected_page_notify(move |_| {
            if let Some(this) = weak.upgrade() {
                this.on_selected();
            }
        });

        let weak = Rc::downgrade(self);
        self.tab_view.connect_close_page(move |view, page| {
            if let Some(this) = weak.upgrade() {
                let idx = this.docs.borrow().iter().position(|d| d.page == *page);
                if let Some(idx) = idx {
                    // Autoguardado: al cerrar nunca se pierde nada.
                    this.save_doc_at(idx);
                    this.docs.borrow_mut().remove(idx);
                }
                view.close_page_finish(page, true);
                this.update_empty();
            }
            glib::Propagation::Stop
        });
    }

    fn update_empty(&self) {
        self.stack.set_visible_child_name(if self.tab_view.n_pages() > 0 { "tabs" } else { "empty" });
    }

    fn on_selected(self: &Rc<Self>) {
        let docs = self.docs.borrow();
        let current = self.tab_view.selected_page().and_then(|p| docs.iter().find(|d| d.page == p));
        if let Some(doc) = current {
            let ctx = sourceview5::SearchContext::new(&doc.buffer, Some(&self.search_settings));
            ctx.set_highlight(self.search_bar.is_search_mode());
            *self.search_ctx.borrow_mut() = Some(ctx);
            if let Some(b) = self.vim_binding.borrow_mut().take() {
                b.unbind();
            }
            if let Some((_, im)) = &doc.vim {
                *self.vim_binding.borrow_mut() = Some(im.bind_property("command-bar-text", &self.vim_label, "label").sync_create().build());
            }
            update_position(&doc.buffer, &self.position_label);
        } else {
            self.position_label.set_label("");
        }
        drop(docs);
        Self::emit(&self.on_switch);
    }

    fn current_doc<R>(&self, f: impl FnOnce(&Doc) -> R) -> Option<R> {
        let page = self.tab_view.selected_page()?;
        self.docs.borrow().iter().find(|d| d.page == page).map(f)
    }

    pub fn current(&self) -> Option<PathBuf> {
        self.current_doc(|d| d.rel.clone())
    }

    pub fn cursor_line(&self) -> u32 {
        self.current_doc(|d| d.buffer.iter_at_mark(&d.buffer.get_insert()).line() as u32 + 1).unwrap_or(1)
    }

    pub fn current_view(&self) -> Option<sourceview5::View> {
        self.current_doc(|d| d.view.clone())
    }

    /// Muestra un archivo (relativo a la raíz) en su pestaña, cargándolo si hace falta.
    pub fn open(self: &Rc<Self>, rel: &Path) -> Result<(), String> {
        let existing = self.docs.borrow().iter().find(|d| d.rel == rel).map(|d| d.page.clone());
        let page = match existing {
            Some(page) => page,
            None => self.load(rel)?,
        };
        let changed = self.tab_view.selected_page().as_ref() != Some(&page);
        self.tab_view.set_selected_page(&page);
        if !changed {
            // La primera pestaña se selecciona sola al añadirla, antes de registrarla.
            self.on_selected();
        }
        self.update_empty();
        Ok(())
    }

    fn load(self: &Rc<Self>, rel: &Path) -> Result<adw::TabPage, String> {
        let abs = self.root.join(rel);
        let bytes = fs::read(&abs).map_err(|e| format!("No se pudo abrir {}: {e}", rel.display()))?;
        let text = String::from_utf8(bytes).map_err(|_| format!("{} no está en UTF-8", rel.display()))?;

        let buffer = sourceview5::Buffer::new(None);
        let is_tex_like = ["tex", "sty", "cls", "ltx", "dtx", "bbx", "cbx", "def", "cfg"].iter().any(|e| crate::project::has_ext(rel, e));
        let lang = if crate::project::has_ext(rel, "bib") {
            Some("bibtex")
        } else if is_tex_like {
            Some("latex")
        } else {
            None
        };
        if let Some(id) = lang {
            buffer.set_language(sourceview5::LanguageManager::default().language(id).as_ref());
        }
        buffer.set_highlight_matching_brackets(true);
        apply_scheme(&buffer);

        buffer.begin_irreversible_action();
        buffer.set_text(&text);
        buffer.end_irreversible_action();
        buffer.set_modified(false);
        buffer.place_cursor(&buffer.start_iter());

        let cfg = config::get();
        let view = sourceview5::View::builder()
            .buffer(&buffer)
            .monospace(true)
            .show_line_numbers(true)
            .highlight_current_line(true)
            .auto_indent(true)
            .indent_width(2)
            .tab_width(2)
            .insert_spaces_instead_of_tabs(true)
            .smart_backspace(true)
            .smart_home_end(sourceview5::SmartHomeEndType::Before)
            .wrap_mode(if cfg.wrap { gtk::WrapMode::WordChar } else { gtk::WrapMode::None })
            .top_margin(10)
            .bottom_margin(240)
            .left_margin(8)
            .right_margin(16)
            .css_classes(["rutex-editor"])
            .build();
        keys::install(&view, self.key_options.clone());
        view.set_show_line_marks(true);
        view.set_mark_attributes(MARK_ERROR, &self.error_attrs, 10);
        view.set_mark_attributes(MARK_WARNING, &self.warning_attrs, 5);
        if lang.is_some() {
            let completion = view.completion();
            completion.add_provider(&self.completion);
            completion.set_select_on_show(true);
            completion.set_show_icons(true);
        }
        let scroller = gtk::ScrolledWindow::builder().child(&view).vexpand(true).build();

        let page = self.tab_view.append(&scroller);
        let name = rel.file_name().unwrap_or_default().to_string_lossy().into_owned();
        page.set_title(&name);
        page.set_tooltip(&glib::markup_escape_text(&rel.display().to_string()));
        page.set_icon(Some(&gio::ThemedIcon::new(file_icon(rel))));

        let weak: Weak<Editor> = Rc::downgrade(self);
        buffer.connect_changed(move |_| {
            if let Some(this) = weak.upgrade() {
                Self::emit(&this.on_change);
            }
        });
        let label = self.position_label.clone();
        let weak_view = view.downgrade();
        buffer.connect_cursor_position_notify(move |b| {
            if weak_view.upgrade().is_some_and(|v| v.has_focus()) {
                update_position(b, &label);
            }
        });

        let mut doc = Doc { rel: rel.to_path_buf(), buffer, view, page: page.clone(), mtime: mtime(&abs), vim: None };
        self.apply_marks(&doc);
        if cfg.vim {
            self.attach_vim(&mut doc);
        }
        self.docs.borrow_mut().push(doc);
        Ok(page)
    }

    fn save_doc_at(&self, idx: usize) -> Option<String> {
        let mut docs = self.docs.borrow_mut();
        let doc = docs.get_mut(idx)?;
        if !doc.buffer.is_modified() {
            return None;
        }
        let abs = self.root.join(&doc.rel);
        let mut warning = None;
        if doc.mtime.is_some() && mtime(&abs) != doc.mtime {
            warning = Some(format!("{} había cambiado fuera de rutex; se ha sobrescrito", doc.rel.display()));
        }
        let (start, end) = doc.buffer.bounds();
        let text = doc.buffer.text(&start, &end, true);
        match write_atomic(&abs, text.as_bytes()) {
            Ok(()) => {
                doc.buffer.set_modified(false);
                doc.mtime = mtime(&abs);
                warning
            }
            Err(e) => Some(format!("No se pudo guardar {}: {e}", doc.rel.display())),
        }
    }

    /// Guarda los buffers modificados. Devuelve los mensajes de error/aviso.
    pub fn save_all(&self) -> Vec<String> {
        let n = self.docs.borrow().len();
        (0..n).filter_map(|i| self.save_doc_at(i)).collect()
    }

    /// Recarga los archivos que han cambiado en disco (git pull, otro editor…)
    /// y que no tienen cambios sin guardar aquí. Devuelve si recargó alguno.
    pub fn reload_external(&self) -> bool {
        let mut reloaded = false;
        for doc in self.docs.borrow_mut().iter_mut() {
            let abs = self.root.join(&doc.rel);
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
        let Some(view) = self.current_view() else { return Ok(()) };
        let buffer = view.buffer();
        if let Some(it) = buffer.iter_at_line(line.saturating_sub(1) as i32) {
            buffer.place_cursor(&it);
        }
        // Tras cambiar de pestaña las alturas de línea aún no están calculadas.
        let v = view.clone();
        glib::idle_add_local_once(move || {
            let buffer = v.buffer();
            v.scroll_to_mark(&buffer.get_insert(), 0.0, true, 0.0, 0.35);
        });
        view.grab_focus();
        Ok(())
    }

    pub fn close_current(&self) {
        if let Some(page) = self.tab_view.selected_page() {
            self.tab_view.close_page(&page);
        }
    }

    pub fn focus(&self) {
        if let Some(v) = self.current_view() {
            v.grab_focus();
        }
    }

    /// Vuelve a aplicar el esquema de colores (cambio de tema).
    pub fn refresh_scheme(&self) {
        for doc in self.docs.borrow().iter() {
            apply_scheme(&doc.buffer);
        }
        self.color_marks();
    }

    fn setup_marks(self: &Rc<Self>) {
        for attrs in [&self.error_attrs, &self.warning_attrs] {
            let weak = Rc::downgrade(self);
            attrs.connect_query_tooltip_text(move |_, mark| {
                let Some(this) = weak.upgrade() else { return String::new() };
                let Some(buffer) = mark.buffer() else { return String::new() };
                let line = buffer.iter_at_mark(mark).line() as u32 + 1;
                let docs = this.docs.borrow();
                let Some(doc) = docs.iter().find(|d| d.buffer.upcast_ref::<gtk::TextBuffer>() == &buffer) else { return String::new() };
                let issues = this.issues.borrow();
                issues.get(&doc.rel).into_iter().flatten().filter(|(l, _, _)| *l == line).map(|(_, _, m)| m.as_str()).collect::<Vec<_>>().join("\n")
            });
        }
        self.color_marks();
    }

    /// Fondo suave de las líneas con problemas, con los colores del tema.
    fn color_marks(&self) {
        let (red, yellow) = match crate::theme::palette() {
            Some(p) => (p.red, p.yellow),
            None => ("#e01b24".to_string(), "#e5a50a".to_string()),
        };
        for (attrs, color) in [(&self.error_attrs, red), (&self.warning_attrs, yellow)] {
            if let Ok(mut rgba) = gtk::gdk::RGBA::parse(&color) {
                rgba.set_alpha(0.14);
                attrs.set_background(&rgba);
            }
        }
    }

    /// Marca en el margen las líneas con errores y avisos de la última compilación.
    pub fn set_issues(&self, issues: HashMap<PathBuf, Vec<(u32, bool, String)>>) {
        *self.issues.borrow_mut() = issues;
        for doc in self.docs.borrow().iter() {
            self.apply_marks(doc);
        }
    }

    fn apply_marks(&self, doc: &Doc) {
        let (start, end) = doc.buffer.bounds();
        doc.buffer.remove_source_marks(&start, &end, Some(MARK_ERROR));
        doc.buffer.remove_source_marks(&start, &end, Some(MARK_WARNING));
        let issues = self.issues.borrow();
        for (line, is_error, _) in issues.get(&doc.rel).into_iter().flatten() {
            if let Some(it) = doc.buffer.iter_at_line(line.saturating_sub(1) as i32) {
                doc.buffer.create_source_mark(None, if *is_error { MARK_ERROR } else { MARK_WARNING }, &it);
            }
        }
    }

    pub fn set_wrap(&self, wrap: bool) {
        for doc in self.docs.borrow().iter() {
            doc.view.set_wrap_mode(if wrap { gtk::WrapMode::WordChar } else { gtk::WrapMode::None });
        }
    }

    pub fn set_autopairs(&self, enabled: bool) {
        self.key_options.autopairs.set(enabled);
    }

    pub fn set_vim(self: &Rc<Self>, enabled: bool) {
        self.key_options.vim.set(enabled);
        {
            let mut docs = self.docs.borrow_mut();
            for doc in docs.iter_mut() {
                if let Some((key, im)) = doc.vim.take() {
                    doc.view.remove_controller(&key);
                    im.set_client_widget(None::<&gtk::Widget>);
                }
                if enabled {
                    self.attach_vim(doc);
                }
            }
        }
        self.vim_label.set_visible(enabled);
        self.on_selected();
    }

    fn attach_vim(self: &Rc<Self>, doc: &mut Doc) {
        let im = sourceview5::VimIMContext::new();
        let key = gtk::EventControllerKey::new();
        key.set_im_context(Some(&im));
        key.set_propagation_phase(gtk::PropagationPhase::Capture);
        doc.view.add_controller(key.clone());
        im.set_client_widget(Some(&doc.view));
        let weak = Rc::downgrade(self);
        im.connect_write(move |_, _, _| {
            if let Some(this) = weak.upgrade() {
                Self::emit(&this.on_save);
            }
        });
        self.vim_label.set_visible(true);
        doc.vim = Some((key, im));
    }

    /// Un archivo se ha renombrado/movido en disco: actualiza su pestaña.
    pub fn rename(&self, from: &Path, to: &Path) {
        for doc in self.docs.borrow_mut().iter_mut() {
            if let Ok(rest) = doc.rel.strip_prefix(from) {
                doc.rel = if rest.as_os_str().is_empty() { to.to_path_buf() } else { to.join(rest) };
                doc.page.set_title(&doc.rel.file_name().unwrap_or_default().to_string_lossy());
                doc.page.set_tooltip(&glib::markup_escape_text(&doc.rel.display().to_string()));
                doc.mtime = mtime(&self.root.join(&doc.rel));
            }
        }
        Self::emit(&self.on_switch);
    }

    /// Un archivo (o carpeta) se ha borrado: cierra sus pestañas sin guardar.
    pub fn forget(&self, path: &Path) {
        let pages: Vec<adw::TabPage> = {
            let mut docs = self.docs.borrow_mut();
            let (gone, keep): (Vec<Doc>, Vec<Doc>) = docs.drain(..).partition(|d| d.rel.starts_with(path));
            *docs = keep;
            gone.into_iter().map(|d| d.page).collect()
        };
        for page in pages {
            self.tab_view.close_page(&page);
        }
        self.update_empty();
    }

    pub fn find(&self) {
        if let Some(view) = self.current_view() {
            let buffer = view.buffer();
            if let Some((s, e)) = buffer.selection_bounds()
                && s.line() == e.line()
            {
                self.search_entry.set_text(&buffer.text(&s, &e, false));
            }
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
        let step = |this: &Rc<Self>, forward: bool| {
            let weak = Rc::downgrade(this);
            move |_: &gtk::SearchEntry| {
                if let Some(this) = weak.upgrade() {
                    this.search_step(forward, false);
                }
            }
        };
        self.search_entry.connect_activate(step(self, true));
        self.search_entry.connect_next_match(step(self, true));
        self.search_entry.connect_previous_match(step(self, false));
        let weak = Rc::downgrade(self);
        self.search_bar.connect_search_mode_enabled_notify(move |bar| {
            let Some(this) = weak.upgrade() else { return };
            if let Some(ctx) = this.search_ctx.borrow().as_ref() {
                ctx.set_highlight(bar.is_search_mode());
            }
            if !bar.is_search_mode() {
                this.focus();
            }
        });
    }

    fn search_step(&self, forward: bool, from_start_of_selection: bool) {
        let ctx = self.search_ctx.borrow();
        let Some(ctx) = ctx.as_ref() else { return };
        let Some(view) = self.current_view() else { return };
        let buffer = view.buffer();
        let insert = buffer.iter_at_mark(&buffer.get_insert());
        let (sel_start, sel_end) = buffer.selection_bounds().unwrap_or((insert, insert));
        let found = if forward {
            ctx.forward(if from_start_of_selection { &sel_start } else { &sel_end })
        } else {
            ctx.backward(&sel_start)
        };
        if let Some((mut s, e, _)) = found {
            buffer.select_range(&s, &e);
            view.scroll_to_iter(&mut s, 0.1, false, 0.0, 0.0);
        }
    }
}

const MARK_ERROR: &str = "rutex-error";
const MARK_WARNING: &str = "rutex-warning";

fn update_position(buffer: &sourceview5::Buffer, label: &gtk::Label) {
    let it = buffer.iter_at_mark(&buffer.get_insert());
    label.set_label(&format!("Ln {}, Col {}", it.line() + 1, it.line_offset() + 1));
}

fn apply_scheme(buffer: &sourceview5::Buffer) {
    let id = crate::theme::scheme_id();
    buffer.set_style_scheme(sourceview5::StyleSchemeManager::default().scheme(&id).as_ref());
}

pub fn file_icon(path: &Path) -> &'static str {
    let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "tex" | "ltx" => "text-x-generic-symbolic",
        "bib" => "x-office-address-book-symbolic",
        "sty" | "cls" | "cfg" | "def" | "bst" => "preferences-other-symbolic",
        "png" | "jpg" | "jpeg" | "gif" | "svg" | "webp" | "eps" => "image-x-generic-symbolic",
        "pdf" => "x-office-document-symbolic",
        "md" | "txt" => "text-x-generic-symbolic",
        _ => "text-x-generic-symbolic",
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
