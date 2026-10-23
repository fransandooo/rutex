//! Ventanas: pantalla de bienvenida y una ventana por proyecto.

use std::cell::{Cell, RefCell};
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, SystemTime};

use adw::prelude::*;
use gtk::{gio, glib};

use crate::compiler::{self, Outcome};
use crate::config;
use crate::editor::Editor;
use crate::logparse::Severity;
use crate::pdf::PdfView;
use crate::project::Project;
use crate::synctex;

const AUTOSAVE_DELAY: Duration = Duration::from_millis(800);

thread_local! {
    static PROJECTS: RefCell<Vec<Rc<ProjectWindow>>> = const { RefCell::new(Vec::new()) };
    static WELCOME: RefCell<Option<glib::WeakRef<adw::ApplicationWindow>>> = const { RefCell::new(None) };
}

pub fn set_accels(app: &adw::Application) {
    let accels: &[(&str, &[&str])] = &[
        ("win.compile", &["<Ctrl>Return"]),
        ("win.save", &["<Ctrl>s"]),
        ("win.open", &["<Ctrl>o"]),
        ("win.new-file", &["<Ctrl>n"]),
        ("win.find", &["<Ctrl>f"]),
        ("win.forward-sync", &["<Ctrl>j"]),
        ("win.zoom-in", &["<Ctrl>plus", "<Ctrl>equal", "<Ctrl>KP_Add"]),
        ("win.zoom-out", &["<Ctrl>minus", "<Ctrl>KP_Subtract"]),
        ("win.zoom-fit", &["<Ctrl>0"]),
        ("win.toggle-sidebar", &["F9"]),
        ("win.toggle-issues", &["<Ctrl>e"]),
        ("window.close", &["<Ctrl>w"]),
        ("app.quit", &["<Ctrl>q"]),
    ];
    for (action, keys) in accels {
        app.set_accels_for_action(action, keys);
    }
}

pub fn setup_app_actions(app: &adw::Application) {
    let quit = gio::SimpleAction::new("quit", None);
    let weak = app.downgrade();
    quit.connect_activate(move |_, _| {
        if let Some(app) = weak.upgrade() {
            for w in app.windows() {
                w.close();
            }
        }
    });
    app.add_action(&quit);

    let vim = gio::SimpleAction::new_stateful("vim", None, &config::get().vim.to_variant());
    vim.connect_activate(|action, _| {
        let enabled = !action.state().and_then(|s| s.get::<bool>()).unwrap_or(false);
        action.set_state(&enabled.to_variant());
        config::update(|c| c.vim = enabled);
        for_each_project(|pw| pw.editor.set_vim(enabled));
    });
    app.add_action(&vim);

    adw::StyleManager::default().connect_dark_notify(|sm| {
        let dark = sm.is_dark();
        for_each_project(|pw| pw.editor.set_dark(dark));
    });
}

fn for_each_project(f: impl Fn(&Rc<ProjectWindow>)) {
    let projects = PROJECTS.with(|p| p.borrow().clone());
    for pw in &projects {
        f(pw);
    }
}

pub fn save_all_projects() {
    for_each_project(|pw| {
        pw.editor.save_all();
    });
}

pub fn activate(app: &adw::Application) {
    match app.active_window() {
        Some(w) => w.present(),
        None => show_welcome(app),
    }
}

/// Abre un proyecto (carpeta o archivo .tex). Si ya está abierto, lo enfoca.
pub fn open_project(app: &adw::Application, path: &Path, create: bool) {
    let Ok(canon) = fs::canonicalize(path) else {
        report_error(app, &format!("No existe: {}", path.display()));
        return;
    };
    let existing = PROJECTS.with(|p| p.borrow().iter().find(|pw| canon.starts_with(&pw.project.borrow().root)).cloned());
    if let Some(pw) = existing {
        if canon.is_file() {
            let rel = canon.strip_prefix(&pw.project.borrow().root).unwrap().to_path_buf();
            pw.open_file(&rel);
        }
        pw.window.present();
        return;
    }

    let guess_root = if canon.is_dir() { canon.clone() } else { canon.parent().unwrap().to_path_buf() };
    match Project::open(&canon, config::get().main_for(&guess_root), create) {
        Ok((project, first)) => {
            config::update(|c| c.add_recent(project.root.clone()));
            let pw = ProjectWindow::new(app, project, &first);
            PROJECTS.with(|p| p.borrow_mut().push(pw.clone()));
            pw.window.present();
            if let Some(w) = WELCOME.with(|w| w.borrow_mut().take()).and_then(|w| w.upgrade()) {
                w.close();
            }
        }
        Err(e) => report_error(app, &format!("No se pudo abrir {}:\n{e}", canon.display())),
    }
}

fn report_error(app: &adw::Application, msg: &str) {
    let dialog = adw::AlertDialog::new(Some("rutex"), Some(msg));
    dialog.add_response("ok", "Aceptar");
    match app.active_window() {
        Some(w) => dialog.present(Some(&w)),
        None => {
            show_welcome(app);
            dialog.present(app.active_window().as_ref());
        }
    }
}

fn pick_folder(app: &adw::Application, parent: Option<&gtk::Window>, create: bool) {
    let title = if create { "Carpeta del nuevo proyecto" } else { "Abrir proyecto" };
    let dialog = gtk::FileDialog::builder().title(title).modal(true).build();
    let app = app.clone();
    dialog.select_folder(parent, gio::Cancellable::NONE, move |res| {
        if let Some(path) = res.ok().and_then(|f| f.path()) {
            open_project(&app, &path, create);
        }
    });
}

fn show_welcome(app: &adw::Application) {
    if let Some(w) = WELCOME.with(|w| w.borrow().clone()).and_then(|w| w.upgrade()) {
        w.present();
        return;
    }
    let window = adw::ApplicationWindow::builder().application(app).title("rutex").default_width(640).default_height(620).build();

    let open = gtk::Button::builder().label("Abrir proyecto…").css_classes(["pill", "suggested-action"]).build();
    let new = gtk::Button::builder().label("Nuevo proyecto…").css_classes(["pill"]).build();
    let buttons = gtk::Box::builder().spacing(12).halign(gtk::Align::Center).build();
    buttons.append(&open);
    buttons.append(&new);

    let content = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(24).build();
    content.append(&buttons);

    let recent = config::get().recent;
    if !recent.is_empty() {
        let list = gtk::ListBox::builder().css_classes(["boxed-list"]).selection_mode(gtk::SelectionMode::None).build();
        for path in &recent {
            let row = adw::ActionRow::builder()
                .title(glib::markup_escape_text(&path.file_name().unwrap_or_default().to_string_lossy()))
                .subtitle(glib::markup_escape_text(&path.display().to_string()))
                .activatable(true)
                .build();
            row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
            let app = app.clone();
            let path = path.clone();
            row.connect_activated(move |_| open_project(&app, &path, false));
            list.append(&row);
        }
        let clamp = adw::Clamp::builder().maximum_size(480).child(&list).build();
        content.append(&clamp);
    }

    let status = adw::StatusPage::builder()
        .icon_name("accessories-text-editor-symbolic")
        .title("rutex")
        .description("Editor LaTeX con compilación automática")
        .child(&content)
        .build();
    let scroller = gtk::ScrolledWindow::builder().child(&status).build();
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    toolbar.set_content(Some(&scroller));
    window.set_content(Some(&toolbar));

    let (a, w) = (app.clone(), window.downgrade());
    open.connect_clicked(move |_| pick_folder(&a, w.upgrade().map(|w| w.upcast()).as_ref(), false));
    let (a, w) = (app.clone(), window.downgrade());
    new.connect_clicked(move |_| pick_folder(&a, w.upgrade().map(|w| w.upcast()).as_ref(), true));

    WELCOME.with(|w| *w.borrow_mut() = Some(window.downgrade()));
    window.present();
}

#[derive(Default)]
struct CompileState {
    running: bool,
    pending: bool,
    pid: Option<i32>,
}

/// Problema ya resuelto a una ruta del proyecto.
struct ShownIssue {
    severity: Severity,
    file: Option<PathBuf>,
    line: Option<u32>,
    message: String,
}

pub struct ProjectWindow {
    pub window: adw::ApplicationWindow,
    app: adw::Application,
    project: RefCell<Project>,
    editor: Rc<Editor>,
    pdf: Rc<PdfView>,
    title: adw::WindowTitle,
    toasts: adw::ToastOverlay,
    files_list: gtk::ListBox,
    sidebar: gtk::ScrolledWindow,
    spinner: gtk::Spinner,
    status: gtk::Label,
    issues_btn: gtk::ToggleButton,
    issues_count: gtk::Label,
    issues_revealer: gtk::Revealer,
    issues_list: gtk::ListBox,
    issues_stack: gtk::Stack,
    output: gtk::TextView,
    issues: RefCell<Vec<ShownIssue>>,
    compile: RefCell<CompileState>,
    autosave: RefCell<Option<glib::SourceId>>,
    pdf_mtime: Cell<Option<SystemTime>>,
}

impl ProjectWindow {
    fn new(app: &adw::Application, project: Project, first: &Path) -> Rc<ProjectWindow> {
        let editor = Editor::new(project.root.clone());
        let pdf = PdfView::new();

        // --- Barra superior ---
        let title = adw::WindowTitle::new(&project.name(), "");
        let header = adw::HeaderBar::new();
        header.set_title_widget(Some(&title));

        let sidebar_btn = gtk::ToggleButton::builder().icon_name("sidebar-show-symbolic").active(true).tooltip_text("Archivos (F9)").build();
        let open_btn = gtk::Button::builder().icon_name("folder-open-symbolic").tooltip_text("Abrir proyecto (Ctrl+O)").action_name("win.open").build();
        let new_btn = gtk::Button::builder().icon_name("document-new-symbolic").tooltip_text("Nuevo archivo (Ctrl+N)").action_name("win.new-file").build();
        header.pack_start(&sidebar_btn);
        header.pack_start(&open_btn);
        header.pack_start(&new_btn);

        let menu = gio::Menu::new();
        let s1 = gio::Menu::new();
        s1.append(Some("Nuevo archivo…"), Some("win.new-file"));
        s1.append(Some("Usar este archivo como principal"), Some("win.set-main"));
        menu.append_section(None, &s1);
        let s2 = gio::Menu::new();
        s2.append(Some("Guardar PDF en la carpeta del proyecto"), Some("win.export-pdf"));
        s2.append(Some("Abrir PDF en visor externo"), Some("win.open-pdf"));
        s2.append(Some("Limpiar archivos auxiliares"), Some("win.clean"));
        menu.append_section(None, &s2);
        let s3 = gio::Menu::new();
        s3.append(Some("Modo Vim"), Some("app.vim"));
        menu.append_section(None, &s3);
        let menu_btn = gtk::MenuButton::builder().icon_name("open-menu-symbolic").menu_model(&menu).primary(true).build();

        let compile_btn = gtk::Button::builder().icon_name("media-playback-start-symbolic").tooltip_text("Compilar (Ctrl+Intro)").action_name("win.compile").build();
        let spinner = gtk::Spinner::new();
        let status = gtk::Label::builder().css_classes(["dim-label", "caption"]).build();
        let issues_count = gtk::Label::new(None);
        let issues_box = gtk::Box::builder().spacing(4).build();
        issues_box.append(&gtk::Image::from_icon_name("dialog-warning-symbolic"));
        issues_box.append(&issues_count);
        let issues_btn = gtk::ToggleButton::builder().child(&issues_box).tooltip_text("Problemas (Ctrl+E)").visible(false).build();

        header.pack_end(&menu_btn);
        header.pack_end(&compile_btn);
        header.pack_end(&issues_btn);
        header.pack_end(&status);
        header.pack_end(&spinner);

        // --- Barra lateral ---
        let files_list = gtk::ListBox::builder().css_classes(["navigation-sidebar"]).build();
        let sidebar = gtk::ScrolledWindow::builder()
            .child(&files_list)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .width_request(180)
            .build();

        // --- Panel de problemas ---
        let issues_list = gtk::ListBox::builder().css_classes(["issues"]).build();
        let output = gtk::TextView::builder().editable(false).monospace(true).wrap_mode(gtk::WrapMode::WordChar).left_margin(8).top_margin(6).build();
        let issues_stack = gtk::Stack::new();
        issues_stack.add_titled(&gtk::ScrolledWindow::builder().child(&issues_list).build(), Some("issues"), "Problemas");
        issues_stack.add_titled(&gtk::ScrolledWindow::builder().child(&output).build(), Some("output"), "Salida de latexmk");
        let switcher = gtk::StackSwitcher::builder().stack(&issues_stack).halign(gtk::Align::Start).margin_start(6).margin_top(4).margin_bottom(4).build();
        let panel = gtk::Box::builder().orientation(gtk::Orientation::Vertical).height_request(190).css_classes(["issues-panel"]).build();
        panel.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        panel.append(&switcher);
        issues_stack.set_vexpand(true);
        panel.append(&issues_stack);
        let issues_revealer = gtk::Revealer::builder().child(&panel).transition_type(gtk::RevealerTransitionType::SlideUp).build();

        let editor_col = gtk::Box::new(gtk::Orientation::Vertical, 0);
        editor_col.append(&editor.widget);
        editor_col.append(&issues_revealer);

        let left = gtk::Paned::builder()
            .start_child(&sidebar)
            .end_child(&editor_col)
            .position(220)
            .resize_start_child(false)
            .shrink_start_child(false)
            .shrink_end_child(false)
            .build();
        let main = gtk::Paned::builder().start_child(&left).end_child(&pdf.widget).shrink_start_child(false).shrink_end_child(false).build();
        // Con Hyprland el tamaño de la ventana lo decide el gestor: repartimos
        // el espacio la primera vez que sabemos cuánto hay (≈55 % editor, 45 % PDF).
        let placed = Cell::new(false);
        main.connect_max_position_notify(move |paned| {
            if paned.max_position() > 400 && !placed.replace(true) {
                paned.set_position((paned.max_position() as f64 * 0.56) as i32);
            }
        });
        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&main));

        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&header);
        toolbar.set_content(Some(&toasts));

        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title(format!("{} — rutex", project.name()))
            .default_width(1500)
            .default_height(920)
            .content(&toolbar)
            .build();

        let this = Rc::new(ProjectWindow {
            window,
            app: app.clone(),
            project: RefCell::new(project),
            editor,
            pdf,
            title,
            toasts,
            files_list,
            sidebar,
            spinner,
            status,
            issues_btn,
            issues_count,
            issues_revealer,
            issues_list,
            issues_stack,
            output,
            issues: RefCell::default(),
            compile: RefCell::default(),
            autosave: RefCell::default(),
            pdf_mtime: Cell::new(None),
        });

        sidebar_btn.bind_property("active", &this.sidebar, "visible").sync_create().build();
        this.issues_btn.bind_property("active", &this.issues_revealer, "reveal-child").sync_create().build();
        this.editor.set_vim(config::get().vim);
        this.connect_signals();
        this.add_actions(&sidebar_btn);
        this.refresh_files();
        this.open_file(first);

        // PDF de la sesión anterior: vista previa instantánea mientras se recompila.
        let pdf_path = this.project.borrow().pdf_path();
        if let Ok(mtime) = fs::metadata(&pdf_path).and_then(|m| m.modified()) {
            this.pdf_mtime.set(Some(mtime));
            this.pdf.load(pdf_path);
        }
        this.compile();
        this
    }

    fn connect_signals(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.editor.connect_change(move || {
            if let Some(this) = weak.upgrade() {
                this.schedule_autosave();
            }
        });
        let weak = Rc::downgrade(self);
        self.editor.connect_save(move || {
            if let Some(this) = weak.upgrade() {
                this.save_and_compile();
            }
        });

        let weak = Rc::downgrade(self);
        self.files_list.connect_row_activated(move |_, row| {
            let Some(this) = weak.upgrade() else { return };
            let rel = this.project.borrow().files.get(row.index() as usize).cloned();
            if let Some(rel) = rel {
                this.open_file(&rel);
            }
        });

        let weak = Rc::downgrade(self);
        self.issues_list.connect_row_activated(move |_, row| {
            let Some(this) = weak.upgrade() else { return };
            let target = {
                let issues = this.issues.borrow();
                issues.get(row.index() as usize).map(|i| (i.file.clone(), i.line))
            };
            if let Some((file, line)) = target {
                let file = file.unwrap_or_else(|| this.project.borrow().main.clone());
                this.goto(&file, line.unwrap_or(1));
            }
        });

        let weak = Rc::downgrade(self);
        self.pdf.connect_inverse(move |pos| {
            let Some(this) = weak.upgrade() else { return };
            let p = this.project.borrow().clone();
            let weak = Rc::downgrade(&this);
            glib::spawn_future_local(async move {
                let found = synctex::edit(p.root.clone(), p.build_dir.clone(), p.pdf_path(), pos).await;
                let Some(this) = weak.upgrade() else { return };
                match found.and_then(|(input, line)| p.relativize(&input).map(|rel| (rel, line))) {
                    Some((rel, line)) => this.goto(&rel, line),
                    None => this.toast("SyncTeX no encontró esa posición"),
                }
            });
        });

        let weak = Rc::downgrade(self);
        self.window.connect_is_active_notify(move |w| {
            if !w.is_active() {
                return;
            }
            let Some(this) = weak.upgrade() else { return };
            this.project.borrow_mut().rescan();
            this.refresh_files();
            if this.editor.reload_external() {
                this.compile();
            }
        });

        let weak = Rc::downgrade(self);
        self.window.connect_close_request(move |_| {
            if let Some(this) = weak.upgrade() {
                this.save();
                if let Some(id) = this.autosave.borrow_mut().take() {
                    id.remove();
                }
                if let Some(pid) = this.compile.borrow_mut().pid.take() {
                    compiler::kill(pid);
                }
                PROJECTS.with(|p| p.borrow_mut().retain(|pw| !Rc::ptr_eq(pw, &this)));
            }
            glib::Propagation::Proceed
        });
    }

    fn add_actions(self: &Rc<Self>, sidebar_btn: &gtk::ToggleButton) {
        type Handler = Box<dyn Fn(&Rc<ProjectWindow>)>;
        let add = |name: &str, f: Handler| {
            let action = gio::SimpleAction::new(name, None);
            let weak = Rc::downgrade(self);
            action.connect_activate(move |_, _| {
                if let Some(this) = weak.upgrade() {
                    f(&this);
                }
            });
            self.window.add_action(&action);
        };

        add("compile", Box::new(|t| t.save_and_compile()));
        add("save", Box::new(|t| t.save_and_compile()));
        add("open", Box::new(|t| pick_folder(&t.app, Some(t.window.upcast_ref()), false)));
        add("find", Box::new(|t| t.editor.find()));
        add("forward-sync", Box::new(|t| t.forward_sync()));
        add("zoom-in", Box::new(|t| t.pdf.zoom(1)));
        add("zoom-out", Box::new(|t| t.pdf.zoom(-1)));
        add("zoom-fit", Box::new(|t| t.pdf.zoom(0)));
        add("new-file", Box::new(|t| t.new_file_dialog()));
        add("set-main", Box::new(|t| t.set_main()));
        add("export-pdf", Box::new(|t| t.export_pdf()));
        add("open-pdf", Box::new(|t| t.open_pdf_external()));
        add("clean", Box::new(|t| t.clean()));
        add("toggle-issues", Box::new(|t| t.issues_btn.set_active(!t.issues_revealer.reveals_child())));
        let btn = sidebar_btn.clone();
        add("toggle-sidebar", Box::new(move |_| btn.set_active(!btn.is_active())));
    }

    fn toast(&self, msg: &str) {
        self.toasts.add_toast(adw::Toast::builder().title(glib::markup_escape_text(msg)).timeout(3).build());
    }

    fn open_file(self: &Rc<Self>, rel: &Path) {
        match self.editor.open(rel) {
            Ok(()) => {
                self.title.set_subtitle(&rel.display().to_string());
                self.select_current_file();
            }
            Err(e) => self.toast(&e),
        }
    }

    fn goto(self: &Rc<Self>, rel: &Path, line: u32) {
        match self.editor.goto(rel, line) {
            Ok(()) => {
                self.title.set_subtitle(&rel.display().to_string());
                self.select_current_file();
            }
            Err(e) => self.toast(&e),
        }
    }

    fn refresh_files(&self) {
        while let Some(child) = self.files_list.first_child() {
            self.files_list.remove(&child);
        }
        let project = self.project.borrow();
        for rel in &project.files {
            let name = rel.file_name().unwrap_or_default().to_string_lossy();
            let dir = rel.parent().map(|p| p.display().to_string()).unwrap_or_default();
            let markup = if dir.is_empty() {
                glib::markup_escape_text(&name).to_string()
            } else {
                format!("<span alpha=\"55%\">{}/</span>{}", glib::markup_escape_text(&dir), glib::markup_escape_text(&name))
            };
            let label = gtk::Label::builder().label(markup).use_markup(true).xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::Start).hexpand(true).build();
            let row_box = gtk::Box::builder().spacing(6).build();
            row_box.append(&label);
            if *rel == project.main {
                row_box.append(&gtk::Image::builder().icon_name("starred-symbolic").tooltip_text("Archivo principal").build());
            }
            self.files_list.append(&row_box);
        }
        drop(project);
        self.select_current_file();
    }

    fn select_current_file(&self) {
        let current = self.editor.current();
        let idx = current.and_then(|c| self.project.borrow().files.iter().position(|f| *f == c));
        match idx.and_then(|i| self.files_list.row_at_index(i as i32)) {
            Some(row) => self.files_list.select_row(Some(&row)),
            None => self.files_list.unselect_all(),
        }
    }

    fn schedule_autosave(self: &Rc<Self>) {
        if let Some(id) = self.autosave.borrow_mut().take() {
            id.remove();
        }
        let weak = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(AUTOSAVE_DELAY, move || {
            if let Some(this) = weak.upgrade() {
                this.autosave.borrow_mut().take();
                this.save_and_compile();
            }
        });
        *self.autosave.borrow_mut() = Some(id);
    }

    fn save(&self) {
        for msg in self.editor.save_all() {
            self.toast(&msg);
        }
    }

    fn save_and_compile(self: &Rc<Self>) {
        if let Some(id) = self.autosave.borrow_mut().take() {
            id.remove();
        }
        self.save();
        self.compile();
    }

    /// Si ya hay una compilación en marcha no se interrumpe (matar pdflatex a
    /// medias deja .aux corruptos): se encola una más para cuando termine.
    fn compile(self: &Rc<Self>) {
        {
            let mut st = self.compile.borrow_mut();
            if st.running {
                st.pending = true;
                return;
            }
            st.running = true;
        }
        let project = self.project.borrow().clone();
        match compiler::spawn(&project) {
            Ok(running) => {
                self.compile.borrow_mut().pid = Some(running.pid);
                self.spinner.start();
                self.status.set_label(&format!("Compilando ({})…", running.engine.label()));
                let weak = Rc::downgrade(self);
                glib::spawn_future_local(async move {
                    let outcome = running.wait().await;
                    if let Some(this) = weak.upgrade() {
                        this.finish_compile(outcome);
                    }
                });
            }
            Err(e) => {
                self.compile.replace(CompileState::default());
                let msg = if e.kind() == std::io::ErrorKind::NotFound {
                    "No se encontró latexmk. Instala TeX Live:\n\nsudo pacman -S --needed texlive-basic texlive-latex texlive-latexrecommended \\\n  texlive-latexextra texlive-fontsrecommended texlive-bibtexextra texlive-mathscience \\\n  texlive-langspanish texlive-binextra texlive-xetex texlive-luatex biber".to_string()
                } else {
                    format!("No se pudo lanzar latexmk: {e}")
                };
                self.status.set_label("Sin TeX");
                self.output.buffer().set_text(&msg);
                self.issues_stack.set_visible_child_name("output");
                self.issues_count.set_label("!");
                self.issues_btn.set_visible(true);
                self.issues_btn.set_active(true);
            }
        }
    }

    fn finish_compile(self: &Rc<Self>, outcome: Outcome) {
        let pending = {
            let mut st = self.compile.borrow_mut();
            let pending = st.pending;
            *st = CompileState::default();
            pending
        };
        self.spinner.stop();

        let project = self.project.borrow().clone();
        let shown: Vec<ShownIssue> = outcome
            .issues
            .into_iter()
            .map(|i| ShownIssue {
                severity: i.severity,
                file: i.file.as_deref().and_then(|f| project.relativize(f)).filter(|f| project.root.join(f).is_file()),
                line: i.line,
                message: i.message,
            })
            .collect();
        let errors = shown.iter().filter(|i| i.severity == Severity::Error).count();
        let warnings = shown.len() - errors;
        self.fill_issues(&shown);
        *self.issues.borrow_mut() = shown;
        self.output.buffer().set_text(&outcome.output);

        let secs = format!("{:.1}", outcome.elapsed.as_secs_f64()).replace('.', ",");
        let label = match (errors, outcome.pdf.is_some()) {
            (0, true) if outcome.ok => format!("✓ {secs} s"),
            (0, _) => "✗ Falló la compilación".to_string(),
            (1, _) => "✗ 1 error".to_string(),
            (n, _) => format!("✗ {n} errores"),
        };
        self.status.set_label(&label);
        self.issues_count.set_label(&if errors > 0 { format!("{errors}·{warnings}") } else { warnings.to_string() });
        self.issues_btn.set_visible(errors + warnings > 0 || !outcome.ok);

        if let Some((path, mtime)) = outcome.pdf {
            if self.pdf_mtime.get() != Some(mtime) {
                self.pdf_mtime.set(Some(mtime));
                self.pdf.load(path);
            }
        } else if errors == 0 {
            // Fallo antes de llegar a TeX (paquete de sistema que falta, biber…).
            self.issues_stack.set_visible_child_name("output");
            self.issues_btn.set_active(true);
        }

        if pending {
            self.compile();
        }
    }

    fn fill_issues(&self, issues: &[ShownIssue]) {
        while let Some(child) = self.issues_list.first_child() {
            self.issues_list.remove(&child);
        }
        for issue in issues {
            let (icon, class) = match issue.severity {
                Severity::Error => ("dialog-error-symbolic", "error"),
                Severity::Warning => ("dialog-warning-symbolic", "warning"),
            };
            let row = gtk::Box::builder().spacing(8).margin_start(6).margin_end(6).build();
            row.append(&gtk::Image::builder().icon_name(icon).css_classes([class]).build());
            let location = match (&issue.file, issue.line) {
                (Some(f), Some(l)) => format!("{}:{l}", f.display()),
                (Some(f), None) => f.display().to_string(),
                (None, Some(l)) => format!("línea {l}"),
                (None, None) => String::new(),
            };
            if !location.is_empty() {
                row.append(&gtk::Label::builder().label(location).css_classes(["dim-label", "monospace"]).build());
            }
            row.append(
                &gtk::Label::builder()
                    .label(&issue.message)
                    .xalign(0.0)
                    .hexpand(true)
                    .ellipsize(gtk::pango::EllipsizeMode::End)
                    .tooltip_text(&issue.message)
                    .build(),
            );
            self.issues_list.append(&row);
        }
        self.issues_stack.set_visible_child_name("issues");
    }

    fn forward_sync(self: &Rc<Self>) {
        let Some(rel) = self.editor.current() else { return };
        if !self.pdf.has_document() {
            return;
        }
        let line = self.editor.cursor_line();
        let p = self.project.borrow().clone();
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let pos = synctex::view(p.root.clone(), p.build_dir.clone(), p.pdf_path(), p.root.join(&rel), line).await;
            let Some(this) = weak.upgrade() else { return };
            match pos {
                Some(pos) => this.pdf.reveal(pos),
                None => this.toast("SyncTeX no encontró esa línea en el PDF"),
            }
        });
    }

    fn new_file_dialog(self: &Rc<Self>) {
        let entry = gtk::Entry::builder().placeholder_text("capitulos/introduccion.tex").activates_default(true).build();
        let dialog = adw::AlertDialog::builder()
            .heading("Nuevo archivo")
            .body("Ruta relativa a la carpeta del proyecto")
            .extra_child(&entry)
            .default_response("create")
            .close_response("cancel")
            .build();
        dialog.add_responses(&[("cancel", "Cancelar"), ("create", "Crear")]);
        dialog.set_response_appearance("create", adw::ResponseAppearance::Suggested);
        let weak = Rc::downgrade(self);
        let input = entry.clone();
        dialog.connect_response(Some("create"), move |_, _| {
            let Some(this) = weak.upgrade() else { return };
            let text = input.text().trim().trim_start_matches('/').to_string();
            if text.is_empty() || text.split('/').any(|c| c == "..") {
                return;
            }
            let mut rel = PathBuf::from(&text);
            if rel.extension().is_none() {
                rel.set_extension("tex");
            }
            let abs = this.project.borrow().root.join(&rel);
            if !abs.exists() {
                let created = abs.parent().map_or(Ok(()), fs::create_dir_all).and_then(|_| fs::write(&abs, ""));
                if let Err(e) = created {
                    this.toast(&format!("No se pudo crear: {e}"));
                    return;
                }
                if crate::project::has_ext(&rel, "tex") {
                    let stem = rel.with_extension("");
                    this.toast(&format!("Inclúyelo con \\input{{{}}}", stem.display()));
                }
            }
            this.project.borrow_mut().rescan();
            this.refresh_files();
            this.open_file(&rel);
        });
        dialog.present(Some(&self.window));
        entry.grab_focus();
    }

    fn set_main(self: &Rc<Self>) {
        let Some(rel) = self.editor.current() else { return };
        if !crate::project::has_ext(&rel, "tex") {
            self.toast("El archivo principal debe ser un .tex");
            return;
        }
        let root = {
            let mut p = self.project.borrow_mut();
            p.main = rel.clone();
            p.root.clone()
        };
        config::update(|c| c.set_main(root, rel.clone()));
        self.pdf_mtime.set(None);
        self.refresh_files();
        self.toast(&format!("Archivo principal: {}", rel.display()));
        self.compile();
    }

    fn export_pdf(&self) {
        let p = self.project.borrow();
        let src = p.pdf_path();
        let dst = p.root.join(src.file_name().unwrap());
        match fs::copy(&src, &dst) {
            Ok(_) => self.toast(&format!("PDF guardado en {}", dst.display())),
            Err(_) => self.toast("Todavía no hay PDF"),
        }
    }

    fn open_pdf_external(&self) {
        let path = self.project.borrow().pdf_path();
        if !path.exists() {
            self.toast("Todavía no hay PDF");
            return;
        }
        gtk::FileLauncher::new(Some(&gio::File::for_path(path))).launch(Some(&self.window), gio::Cancellable::NONE, |_| {});
    }

    fn clean(self: &Rc<Self>) {
        if let Some(pid) = self.compile.borrow_mut().pid.take() {
            compiler::kill(pid);
        }
        let dir = self.project.borrow().build_dir.clone();
        let _ = fs::remove_dir_all(&dir);
        self.pdf_mtime.set(None);
        self.toast("Archivos auxiliares eliminados");
        // Si había una compilación en marcha, su final relanzará otra.
        if self.compile.borrow().running {
            self.compile.borrow_mut().pending = true;
        } else {
            self.compile();
        }
    }
}
