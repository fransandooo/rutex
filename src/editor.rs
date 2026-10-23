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
use crate::hover::IssueHover;
use crate::keys;
use crate::tabs::TabStrip;
use crate::i18n::{tr, trf};

struct Doc {
    rel: PathBuf,
    buffer: sourceview5::Buffer,
    view: sourceview5::View,
    page: adw::TabPage,
    mtime: Option<SystemTime>,
    vim: Option<(gtk::EventControllerKey, sourceview5::VimIMContext)>,
    spell: Option<libspelling::TextBufferAdapter>,
    bar: crate::changebar::ChangeBar,
    /// Text in the history version the margin bars compare against.
    baseline: Option<String>,
    diff_timer: Option<glib::SourceId>,
}

type Callback = RefCell<Option<Box<dyn Fn()>>>;

pub struct Editor {
    pub widget: gtk::Box,
    /// "Ln 12, Col 4" del archivo actual (para la barra de estado).
    pub position_label: gtk::Label,
    /// Línea de comandos del modo Vim (para la barra de estado).
    pub vim_label: gtk::Label,
    tab_view: adw::TabView,
    _tab_strip: Rc<TabStrip>,
    stack: gtk::Stack,
    root: PathBuf,
    completion: LatexProvider,
    data: SharedData,
    key_options: Rc<keys::Options>,
    checker: libspelling::Checker,
    math: RefCell<Option<Rc<crate::mathpreview::MathPreview>>>,
    /// Problemas de la última compilación por archivo: (línea, es_error, mensaje).
    issues: RefCell<HashMap<PathBuf, Vec<(u32, bool, String)>>>,
    error_attrs: sourceview5::MarkAttributes,
    hover: RefCell<Option<IssueHover>>,
    on_issue_click: RefCell<Option<Box<dyn Fn(&Path, u32)>>>,
    warning_attrs: sourceview5::MarkAttributes,
    docs: RefCell<Vec<Doc>>,
    on_change: Callback,
    on_save: Callback,
    on_switch: Callback,
    on_loaded: RefCell<Option<Box<dyn Fn(&Path)>>>,
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
        let tab_strip = TabStrip::new(&tab_view);

        let empty = adw::StatusPage::builder()
            .icon_name("text-x-generic-symbolic")
            .title(tr("No open files"))
            .description(tr("Pick one in the sidebar or press Ctrl+P"))
            .css_classes(["compact"])
            .build();
        let stack = gtk::Stack::builder().vexpand(true).build();
        stack.add_named(&tab_view, Some("tabs"));
        stack.add_named(&empty, Some("empty"));

        let search_entry = gtk::SearchEntry::builder().placeholder_text(tr("Find in file…")).width_chars(12).hexpand(true).build();
        let search_bar = gtk::SearchBar::builder().child(&search_entry).show_close_button(true).build();
        search_bar.connect_entry(&search_entry);

        let widget = gtk::Box::new(gtk::Orientation::Vertical, 0);
        widget.append(&tab_strip.widget);
        widget.append(&search_bar);
        widget.append(&stack);

        let search_settings = sourceview5::SearchSettings::builder().wrap_around(true).case_sensitive(false).build();

        let editor = Rc::new(Editor {
            widget,
            position_label: gtk::Label::new(None),
            vim_label: gtk::Label::builder().css_classes(["monospace"]).visible(false).build(),
            tab_view,
            _tab_strip: tab_strip,
            stack,
            root,
            completion: LatexProvider::new(data.clone()),
            data,
            issues: RefCell::default(),
            hover: RefCell::default(),
            on_issue_click: RefCell::default(),
            error_attrs: sourceview5::MarkAttributes::builder().icon_name("dialog-error-symbolic").build(),
            warning_attrs: sourceview5::MarkAttributes::builder().icon_name("dialog-warning-symbolic").build(),
            checker: libspelling::Checker::default(),
            math: RefCell::default(),
            key_options: Rc::new(keys::Options { autopairs: Cell::new(config::get().autopairs), vim: Cell::new(config::get().vim) }),
            docs: RefCell::default(),
            on_change: RefCell::default(),
            on_save: RefCell::default(),
            on_switch: RefCell::default(),
            on_loaded: RefCell::default(),
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
        let bytes = fs::read(&abs).map_err(|e| trf("Could not open {}: {}", &[&rel.display(), &e]))?;
        let text = String::from_utf8(bytes).map_err(|_| trf("{} is not UTF-8", &[&rel.display()]))?;

        let buffer = sourceview5::Buffer::new(None);
        let is_tex_like = ["tex", "sty", "cls", "ltx", "dtx", "bbx", "cbx", "def", "cfg"].iter().any(|e| crate::project::has_ext(rel, e));
        let lang = if crate::project::has_ext(rel, "bib") {
            Some("bibtex")
        } else if is_tex_like {
            Some("rutex-latex")
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
            .bottom_margin(160)
            .left_margin(8)
            .right_margin(16)
            .css_classes(["rutex-editor"])
            .build();
        keys::install(&view, self.key_options.clone());
        view.set_show_line_marks(true);
        view.set_mark_attributes(MARK_ERROR, &self.error_attrs, 10);
        view.set_mark_attributes(MARK_WARNING, &self.warning_attrs, 5);
        if let Some(h) = self.hover.borrow().as_ref() {
            view.hover().add_provider(h);
        }
        // Ctrl+click on a \\ref or \\cite key: go to its \\label or .bib entry.
        let goto_def = gtk::GestureClick::new();
        goto_def.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        goto_def.connect_pressed(move |gesture, _, x, y| {
            if !gesture.current_event_state().contains(gtk::gdk::ModifierType::CONTROL_MASK) {
                return;
            }
            let Some(this) = weak.upgrade() else { return };
            let Some(view) = gesture.widget().and_downcast::<sourceview5::View>() else { return };
            let (bx, by) = view.window_to_buffer_coords(gtk::TextWindowType::Widget, x as i32, y as i32);
            let Some(iter) = view.iter_at_location(bx, by) else { return };
            let Some(key) = Self::key_at_iter(&view.buffer(), &iter) else { return };
            if let Some((rel, line)) = this.definition(&key) {
                gesture.set_state(gtk::EventSequenceState::Claimed);
                let this2 = this.clone();
                glib::idle_add_local_once(move || {
                    let _ = this2.goto(&rel, line);
                });
            }
        });
        view.add_controller(goto_def);

        let weak = Rc::downgrade(self);
        view.connect_line_mark_activated(move |view, iter, _, _, _| {
            let Some(this) = weak.upgrade() else { return };
            let Some(buffer) = view.buffer().downcast::<sourceview5::Buffer>().ok() else { return };
            let line = iter.line() as u32 + 1;
            if let Some((rel, _)) = this.issues_at(&buffer, line)
                && let Some(cb) = this.on_issue_click.borrow().as_ref()
            {
                cb(&rel, line);
            }
        });
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

        // Spell checking for prose files (not .bib). The LaTeX highlighting marks
        // commands and math as no-spell-check, so only real text is checked.
        let spell = (lang == Some("rutex-latex") || lang.is_none()).then(|| {
            let adapter = libspelling::TextBufferAdapter::new(&buffer, &self.checker);
            view.set_extra_menu(Some(&adapter.menu_model()));
            view.insert_action_group("spelling", Some(&adapter));
            adapter.set_enabled(cfg.spellcheck);
            adapter
        });
        // Change bars in the margin, right after the line numbers.
        let bar = crate::changebar::ChangeBar::default();
        bar.refresh_colors();
        sourceview5::prelude::ViewExt::gutter(&view, gtk::TextWindowType::Left).insert(&bar, 100);
        let weak = Rc::downgrade(self);
        let changed_rel = rel.to_path_buf();
        buffer.connect_changed(move |_| {
            if let Some(this) = weak.upgrade() {
                this.schedule_diff(&changed_rel);
            }
        });
        let mut doc = Doc { rel: rel.to_path_buf(), buffer, view, page: page.clone(), mtime: mtime(&abs), vim: None, spell, bar, baseline: None, diff_timer: None };
        self.apply_marks(&doc);
        if cfg.vim {
            self.attach_vim(&mut doc);
        }
        self.docs.borrow_mut().push(doc);
        if let Some(cb) = self.on_loaded.borrow().as_ref() {
            cb(rel);
        }
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
            warning = Some(trf("{} had changed outside rutex; it was overwritten", &[&doc.rel.display()]));
        }
        let (start, end) = doc.buffer.bounds();
        let text = doc.buffer.text(&start, &end, true);
        match write_atomic(&abs, text.as_bytes()) {
            Ok(()) => {
                doc.buffer.set_modified(false);
                doc.mtime = mtime(&abs);
                warning
            }
            Err(e) => Some(trf("Could not save {}: {}", &[&doc.rel.display(), &e])),
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
            doc.bar.refresh_colors();
        }
        self.color_marks();
    }

    pub fn set_math_preview(&self, math: Rc<crate::mathpreview::MathPreview>) {
        *self.math.borrow_mut() = Some(math);
    }

    /// Rendered formula under the mouse (compiled in the background).
    #[allow(clippy::type_complexity)]
    fn math_bubble(&self, buffer: &sourceview5::Buffer, iter: &gtk::TextIter) -> Option<std::pin::Pin<Box<dyn std::future::Future<Output = Option<gtk::Widget>>>>> {
        let math = self.math.borrow().clone()?;
        if buffer.language().is_none_or(|l| l.id() != "rutex-latex") {
            return None;
        }
        let (s, e) = buffer.bounds();
        let formula = crate::mathpreview::math_at(&buffer.text(&s, &e, false), iter.offset() as usize)?;
        let color = match crate::theme::palette() {
            Some(p) => gtk::gdk::RGBA::parse(&p.fg).unwrap_or(gtk::gdk::RGBA::WHITE),
            None if adw::StyleManager::default().is_dark() => gtk::gdk::RGBA::WHITE,
            None => gtk::gdk::RGBA::BLACK,
        };
        Some(Box::pin(async move {
            Some(match math.render(formula, color).await {
                Ok(texture) => crate::mathpreview::bubble(&texture),
                Err(msg) => gtk::Label::builder().label(crate::i18n::trf("Could not render the formula: {}", &[&msg])).wrap(true).max_width_chars(60).css_classes(["dim-label", "caption"]).build().upcast(),
            })
        }))
    }

    /// Misspelled word under the mouse: suggestions to click, add or ignore.
    fn spell_bubble(&self, buffer: &sourceview5::Buffer, iter: &gtk::TextIter) -> Option<gtk::Widget> {
        let docs = self.docs.borrow();
        let adapter = docs.iter().find(|d| d.buffer == *buffer)?.spell.clone()?;
        drop(docs);
        let tag = adapter.tag()?;
        if !adapter.is_enabled() || !iter.has_tag(&tag) {
            return None;
        }
        let mut start = *iter;
        if !start.starts_tag(Some(&tag)) {
            start.backward_to_tag_toggle(Some(&tag));
        }
        let mut end = *iter;
        end.forward_to_tag_toggle(Some(&tag));
        let word = buffer.text(&start, &end, false).to_string();
        if word.trim().is_empty() {
            return None;
        }
        let language = self.checker.language().map(|l| crate::spell::display_name(&l)).unwrap_or_default();
        let corrections: Vec<String> = self.checker.list_corrections(&word).into_iter().take(6).map(|c| c.to_string()).collect();

        let b = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(8).css_classes(["spell-bubble"]).build();
        b.append(&gtk::Label::builder().label(crate::i18n::trf("“{}” is not in the {} dictionary", &[&word, &language])).xalign(0.0).wrap(true).css_classes(["dim-label", "caption"]).build());
        // Marks survive edits, iters do not.
        let m_start = buffer.create_mark(None, &start, true);
        let m_end = buffer.create_mark(None, &end, false);
        let fixes = gtk::FlowBox::builder().selection_mode(gtk::SelectionMode::None).max_children_per_line(6).column_spacing(4).row_spacing(4).build();
        for c in &corrections {
            let btn = gtk::Button::builder().label(c).css_classes(["suggestion"]).build();
            let (buf, ms, me, text) = (buffer.clone(), m_start.clone(), m_end.clone(), c.clone());
            btn.connect_clicked(move |_| {
                let mut s = buf.iter_at_mark(&ms);
                let mut e = buf.iter_at_mark(&me);
                buf.begin_user_action();
                buf.delete(&mut s, &mut e);
                buf.insert(&mut s, &text);
                buf.end_user_action();
            });
            fixes.insert(&btn, -1);
        }
        if corrections.is_empty() {
            b.append(&gtk::Label::builder().label(tr("No suggestions")).xalign(0.0).build());
        } else {
            b.append(&fixes);
        }
        let actions = gtk::Box::builder().spacing(6).build();
        let add = gtk::Button::builder().label(tr("Add to dictionary")).css_classes(["flat"]).build();
        let ignore = gtk::Button::builder().label(tr("Ignore")).css_classes(["flat"]).build();
        let (checker, ad, w) = (self.checker.clone(), adapter.clone(), word.clone());
        add.connect_clicked(move |_| {
            checker.add_word(&w);
            ad.invalidate_all();
        });
        let (checker, ad, w) = (self.checker.clone(), adapter, word);
        ignore.connect_clicked(move |_| {
            checker.ignore_word(&w);
            ad.invalidate_all();
        });
        actions.append(&add);
        actions.append(&ignore);
        b.append(&actions);
        Some(b.upcast())
    }

    /// \\ref / \\cite key at an iter.
    fn key_at_iter(buffer: &impl IsA<gtk::TextBuffer>, iter: &gtk::TextIter) -> Option<crate::completion::KeyAt> {
        let buffer = buffer.as_ref();
        let mut start = *iter;
        start.set_line_offset(0);
        let mut end = *iter;
        if !end.ends_line() {
            end.forward_to_line_end();
        }
        let line = buffer.text(&start, &end, false);
        crate::completion::key_at(&line, iter.line_offset() as usize)
    }

    /// Where a key is defined: (file, line).
    fn definition(&self, key: &crate::completion::KeyAt) -> Option<(PathBuf, u32)> {
        let data = self.data.borrow();
        match key {
            crate::completion::KeyAt::Label(k) => data.index.labels.iter().find(|l| l.name == *k).map(|l| (l.file.clone(), l.line)),
            crate::completion::KeyAt::Cite(k) => data.index.bib.iter().find(|e| e.key == *k).map(|e| (e.file.clone(), e.line)),
        }
    }

    fn reference_bubble(&self, buffer: &sourceview5::Buffer, iter: &gtk::TextIter) -> Option<gtk::Widget> {
        let key = Self::key_at_iter(buffer, iter)?;
        let data = self.data.borrow();
        Some(match &key {
            crate::completion::KeyAt::Cite(k) => crate::hover::cite_bubble(k, data.index.bib.iter().find(|e| e.key == *k)).upcast(),
            crate::completion::KeyAt::Label(k) => {
                let found = data.index.labels.iter().find(|l| l.name == *k).map(|l| {
                    let description = describe_label(&self.root, &data.index, &l.file, l.line).unwrap_or_else(|| k.clone());
                    (description, format!("{}:{}", l.file.display(), l.line))
                });
                crate::hover::label_bubble(k, found).upcast()
            }
        })
    }

    /// Clic en el icono de error/aviso del margen.
    pub fn connect_issue_click(&self, f: impl Fn(&Path, u32) + 'static) {
        *self.on_issue_click.borrow_mut() = Some(Box::new(f));
    }

    fn issues_at(&self, buffer: &sourceview5::Buffer, line: u32) -> Option<(PathBuf, Vec<(bool, String)>)> {
        let docs = self.docs.borrow();
        let doc = docs.iter().find(|d| d.buffer == *buffer)?;
        let issues = self.issues.borrow();
        let found: Vec<(bool, String)> = issues.get(&doc.rel)?.iter().filter(|(l, _, _)| *l == line).map(|(_, e, m)| (*e, m.clone())).collect();
        (!found.is_empty()).then(|| (doc.rel.clone(), found))
    }

    fn setup_marks(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        let lookup: crate::hover::Lookup = Rc::new(move |buffer, iter| {
            let this = weak.upgrade()?;
            let line = iter.line() as u32 + 1;
            let issues = this.issues_at(buffer, line).map(|(_, v)| v);
            let reference = this.spell_bubble(buffer, iter).or_else(|| this.reference_bubble(buffer, iter));
            use crate::hover::Content;
            match (issues, reference) {
                (None, None) => this.math_bubble(buffer, iter).map(Content::Later),
                (Some(i), None) => Some(Content::Ready(crate::hover::issues_bubble(&i).upcast())),
                (None, Some(r)) => Some(Content::Ready(r)),
                (Some(i), Some(r)) => {
                    let b = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(10).build();
                    b.append(&r);
                    b.append(&crate::hover::issues_bubble(&i));
                    Some(Content::Ready(b.upcast()))
                }
            }
        });
        *self.hover.borrow_mut() = Some(IssueHover::new(lookup));

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

    /// Called after a file is opened (the window then fetches its baseline).
    pub fn connect_loaded(&self, f: impl Fn(&Path) + 'static) {
        *self.on_loaded.borrow_mut() = Some(Box::new(f));
    }

    pub fn open_files(&self) -> Vec<PathBuf> {
        self.docs.borrow().iter().map(|d| d.rel.clone()).collect()
    }

    /// Sets the text the margin bars compare against (None hides the bars).
    pub fn set_baseline(&self, rel: &Path, text: Option<String>) {
        if let Some(doc) = self.docs.borrow_mut().iter_mut().find(|d| d.rel == rel) {
            doc.baseline = text;
        }
        self.update_diff(rel);
    }

    fn schedule_diff(self: &Rc<Self>, rel: &Path) {
        let mut docs = self.docs.borrow_mut();
        let Some(doc) = docs.iter_mut().find(|d| d.rel == rel) else { return };
        if doc.baseline.is_none() {
            return;
        }
        if let Some(id) = doc.diff_timer.take() {
            id.remove();
        }
        let weak = Rc::downgrade(self);
        let rel = rel.to_path_buf();
        doc.diff_timer = Some(glib::timeout_add_local_once(std::time::Duration::from_millis(400), move || {
            if let Some(this) = weak.upgrade() {
                if let Some(d) = this.docs.borrow_mut().iter_mut().find(|d| d.rel == rel) {
                    d.diff_timer = None;
                }
                this.update_diff(&rel);
            }
        }));
    }

    fn update_diff(&self, rel: &Path) {
        let docs = self.docs.borrow();
        let Some(doc) = docs.iter().find(|d| d.rel == rel) else { return };
        let changes = match &doc.baseline {
            Some(base) => {
                let (s, e) = doc.buffer.bounds();
                crate::changebar::line_changes(base, &doc.buffer.text(&s, &e, true))
            }
            None => Default::default(),
        };
        doc.bar.set_changes(changes);
    }

    pub fn set_spellcheck(&self, enabled: bool) {
        for doc in self.docs.borrow().iter() {
            if let Some(a) = &doc.spell {
                a.set_enabled(enabled);
            }
        }
    }

    /// Changes the dictionary (falls back to the closest installed one).
    /// Returns the code actually used, or None if there is no dictionary.
    pub fn set_spell_language(&self, wanted: &str) -> Option<String> {
        let code = crate::spell::installed(&self.checker, wanted)?;
        if self.checker.language().as_deref() != Some(code.as_str()) {
            self.checker.set_language(&code);
            for doc in self.docs.borrow().iter() {
                if let Some(a) = &doc.spell {
                    a.invalidate_all();
                }
            }
        }
        Some(code)
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

    /// Closes the tabs of files that no longer exist (e.g. after restoring a version).
    pub fn forget_missing(&self) {
        let gone: Vec<PathBuf> = self.docs.borrow().iter().filter(|d| !self.root.join(&d.rel).exists()).map(|d| d.rel.clone()).collect();
        for rel in gone {
            self.forget(&rel);
        }
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

/// Human description of a label: the section title or the nearby caption.
fn describe_label(root: &Path, index: &crate::index::Index, file: &Path, line: u32) -> Option<String> {
    if let Some(s) = index.outline.iter().find(|s| s.file == file && s.line.abs_diff(line) <= 1) {
        return Some(s.title.clone());
    }
    let src = fs::read_to_string(root.join(file)).ok()?;
    let lines: Vec<&str> = src.lines().collect();
    let from = (line as usize).saturating_sub(10);
    let to = (line as usize + 3).min(lines.len());
    lines[from..to].iter().rev().find_map(|l| {
        crate::index::calls(crate::project::strip_comment(l))
            .into_iter()
            .find(|c| c.name == "caption")
            .and_then(|c| c.arg.map(crate::index::clean_title))
    })
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
