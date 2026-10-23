//! Ventanas: pantalla de bienvenida y una ventana por proyecto.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, SystemTime};

use adw::prelude::*;
use gtk::{gio, glib};

use crate::compiler::{self, Outcome};
use crate::config::{self, ThemeMode};
use crate::demo;
use crate::editor::Editor;
use crate::filetree::FileTree;
use crate::completion::{Data, SharedData};
use crate::index::{self, Section};
use crate::logparse::Severity;
use crate::newproject;
use crate::pdf::PdfView;
use crate::prefs::{self, Change};
use crate::quickopen;
use crate::project::Project;
use crate::synctex;

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
        ("win.goto-line", &["<Ctrl>g"]),
        ("win.quick-open", &["<Ctrl>p"]),
        ("win.zoom-in", &["<Ctrl>plus", "<Ctrl>equal", "<Ctrl>KP_Add"]),
        ("win.zoom-out", &["<Ctrl>minus", "<Ctrl>KP_Subtract"]),
        ("win.zoom-fit", &["<Ctrl>0"]),
        ("win.toggle-sidebar", &["F9"]),
        ("win.pdf-fullscreen", &["F11"]),
        ("win.toggle-issues", &["<Ctrl>e"]),
        ("win.show-files", &["<Ctrl><Shift>e"]),
        ("win.show-outline", &["<Ctrl><Shift>o"]),
        ("win.close-tab", &["<Ctrl>w"]),
        ("window.close", &["<Ctrl><Shift>w"]),
        ("app.quit", &["<Ctrl>q"]),
        ("app.preferences", &["<Ctrl>comma"]),
        ("app.new-project", &["<Ctrl><Shift>n"]),
        ("app.shortcuts", &["<Ctrl>question", "<Ctrl><Shift>question"]),
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

    let theme = gio::SimpleAction::new_stateful("theme", Some(glib::VariantTy::STRING), &config::get().theme.id().to_variant());
    theme.connect_activate(|action, param| {
        let Some(id) = param.and_then(|p| p.get::<String>()) else { return };
        let Some(mode) = ThemeMode::ALL.into_iter().find(|m| m.id() == id) else { return };
        action.set_state(&id.to_variant());
        config::update(|c| c.theme = mode);
        crate::theme::apply();
    });
    app.add_action(&theme);

    crate::theme::connect_changed(|| {
        for_each_project(|pw| {
            pw.editor.refresh_scheme();
            pw.pdf.refresh_colors();
        })
    });

    let preferences = gio::SimpleAction::new("preferences", None);
    let weak = app.downgrade();
    preferences.connect_activate(move |_, _| {
        let Some(app) = weak.upgrade() else { return };
        let Some(win) = app.active_window() else { return };
        let app2 = app.clone();
        prefs::show(&win, move |change| apply_change(&app2, change));
    });
    app.add_action(&preferences);

    let new_project = gio::SimpleAction::new("new-project", None);
    let weak = app.downgrade();
    new_project.connect_activate(move |_, _| {
        let Some(app) = weak.upgrade() else { return };
        let Some(win) = app.active_window() else { return };
        let a = app.clone();
        newproject::show(&win, move |root| open_project(&a, &root, false));
    });
    app.add_action(&new_project);

    let open_demo = gio::SimpleAction::new("open-demo", None);
    let weak = app.downgrade();
    open_demo.connect_activate(move |_, _| {
        let Some(app) = weak.upgrade() else { return };
        match demo::install() {
            Ok(root) => open_project(&app, &root, false),
            Err(e) => report_error(&app, &format!("No se pudo preparar la demo: {e}")),
        }
    });
    app.add_action(&open_demo);

    let shortcuts = gio::SimpleAction::new("shortcuts", None);
    let weak = app.downgrade();
    shortcuts.connect_activate(move |_, _| {
        if let Some(win) = weak.upgrade().and_then(|a| a.active_window()) {
            prefs::shortcuts(&win);
        }
    });
    app.add_action(&shortcuts);
}

/// Aplica en caliente un cambio de las preferencias a todas las ventanas.
fn apply_change(app: &adw::Application, change: Change) {
    match change {
        Change::Theme => {
            if let Some(action) = app.lookup_action("theme").and_downcast::<gio::SimpleAction>() {
                action.set_state(&config::get().theme.id().to_variant());
            }
            crate::theme::apply();
        }
        Change::FontSize(pt) => crate::theme::set_font_size(pt),
        Change::Wrap(on) => for_each_project(|pw| pw.editor.set_wrap(on)),
        Change::Autopairs(on) => for_each_project(|pw| pw.editor.set_autopairs(on)),
        Change::Vim(on) => {
            if let Some(action) = app.lookup_action("vim").and_downcast::<gio::SimpleAction>() {
                action.set_state(&on.to_variant());
            }
            for_each_project(|pw| pw.editor.set_vim(on));
        }
        Change::PdfDark => for_each_project(|pw| {
            pw.pdf.set_dark_button(config::get().pdf_dark);
            pw.pdf.refresh_colors();
        }),
        Change::Autocompile => {}
    }
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
    let window = adw::ApplicationWindow::builder().application(app).title("rutex").default_width(720).default_height(680).build();

    // Cabecera con el logo.
    let logo = gtk::Image::builder().icon_name("dev.fran.Rutex").pixel_size(72).build();
    let title = gtk::Label::builder().label("rutex").xalign(0.0).css_classes(["title-1"]).build();
    let subtitle = gtk::Label::builder().label("Editor LaTeX con compilación automática").xalign(0.0).css_classes(["dim-label"]).build();
    let titles = gtk::Box::builder().orientation(gtk::Orientation::Vertical).valign(gtk::Align::Center).spacing(2).build();
    titles.append(&title);
    titles.append(&subtitle);
    let hero = gtk::Box::builder().spacing(18).build();
    hero.append(&logo);
    hero.append(&titles);

    // Dos acciones grandes.
    let action_card = |icon: &str, heading: &str, detail: &str| {
        let b = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).margin_top(14).margin_bottom(14).margin_start(14).margin_end(14).build();
        b.append(&gtk::Image::builder().icon_name(icon).pixel_size(28).halign(gtk::Align::Start).css_classes(["accent"]).build());
        b.append(&gtk::Label::builder().label(heading).xalign(0.0).css_classes(["heading"]).build());
        b.append(&gtk::Label::builder().label(detail).xalign(0.0).wrap(true).css_classes(["dim-label", "caption"]).build());
        gtk::Button::builder().child(&b).hexpand(true).css_classes(["card", "welcome-action"]).build()
    };
    let new = action_card("document-new-symbolic", "Nuevo proyecto", "Desde una plantilla: artículo, TFM, apuntes, presentación…");
    new.set_action_name(Some("app.new-project"));
    let open = action_card("folder-open-symbolic", "Abrir carpeta", "Un proyecto LaTeX que ya tengas");
    let demo = action_card("starred-symbolic", "Ver la demo", "Un proyecto completo con figuras, gráficas y bibliografía");
    demo.set_action_name(Some("app.open-demo"));
    let actions = gtk::Box::builder().spacing(12).homogeneous(true).build();
    actions.append(&new);
    actions.append(&open);
    actions.append(&demo);

    let content = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(28).margin_top(24).margin_bottom(32).margin_start(24).margin_end(24).build();
    content.append(&hero);
    content.append(&actions);

    let recent = config::get().recent;
    if !recent.is_empty() {
        let group = adw::PreferencesGroup::builder().title("Recientes").build();
        for path in &recent {
            let row = adw::ActionRow::builder()
                .title(glib::markup_escape_text(&path.file_name().unwrap_or_default().to_string_lossy()))
                .subtitle(glib::markup_escape_text(&tilde(path)))
                .activatable(true)
                .build();
            row.add_prefix(&gtk::Image::from_icon_name("folder-symbolic"));
            let forget = gtk::Button::builder().icon_name("window-close-symbolic").valign(gtk::Align::Center).tooltip_text("Quitar de recientes").css_classes(["flat", "circular"]).build();
            row.add_suffix(&forget);
            let (a, p) = (app.clone(), path.clone());
            row.connect_activated(move |_| open_project(&a, &p, false));
            let (p, r, g) = (path.clone(), row.clone(), group.clone());
            forget.connect_clicked(move |_| {
                config::update(|c| c.recent.retain(|x| *x != p));
                g.remove(&r);
            });
            group.add(&row);
        }
        content.append(&group);
    }
    content.append(&gtk::Label::builder().label("Ctrl+, preferencias  ·  Ctrl+? atajos de teclado").css_classes(["dim-label", "caption"]).build());

    let clamp = adw::Clamp::builder().maximum_size(640).child(&content).build();
    let scroller = gtk::ScrolledWindow::builder().child(&clamp).hscrollbar_policy(gtk::PolicyType::Never).build();
    let header = adw::HeaderBar::builder().show_title(false).build();
    let menu = gio::Menu::new();
    menu.append_submenu(Some("Tema"), &theme_menu());
    menu.append(Some("Preferencias"), Some("app.preferences"));
    menu.append(Some("Atajos de teclado"), Some("app.shortcuts"));
    header.pack_end(&gtk::MenuButton::builder().icon_name("open-menu-symbolic").menu_model(&menu).build());
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&scroller));
    window.set_content(Some(&toolbar));

    let (a, w) = (app.clone(), window.downgrade());
    open.connect_clicked(move |_| pick_folder(&a, w.upgrade().map(|w| w.upcast()).as_ref(), false));

    WELCOME.with(|w| *w.borrow_mut() = Some(window.downgrade()));
    window.present();
}

fn theme_menu() -> gio::Menu {
    let themes = gio::Menu::new();
    for mode in ThemeMode::ALL {
        let item = gio::MenuItem::new(Some(mode.label()), None);
        item.set_action_and_target_value(Some("app.theme"), Some(&mode.id().to_variant()));
        themes.append_item(&item);
    }
    themes
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Status {
    Busy,
    Ok,
    Warning,
    Error,
}

/// 12345 → "12.345".
fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push('.');
        }
        out.push(c);
    }
    out
}

/// `/home/u/tfm` → `~/tfm`.
fn tilde(path: &Path) -> String {
    let home = glib::home_dir();
    match path.strip_prefix(&home) {
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
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
    toasts: adw::ToastOverlay,
    tree: Rc<FileTree>,
    spinner: adw::Spinner,
    status_icon: gtk::Image,
    status: gtk::Label,
    issues_btn: gtk::ToggleButton,
    engine_label: gtk::Label,
    words_label: gtk::Label,
    issues_revealer: gtk::Revealer,
    issues_list: gtk::ListBox,
    issues_stack: gtk::Stack,
    output: gtk::TextView,
    issues: RefCell<Vec<ShownIssue>>,
    compile: RefCell<CompileState>,
    autosave: RefCell<Option<glib::SourceId>>,
    pdf_mtime: Cell<Option<SystemTime>>,
    data: SharedData,
    index_generation: Cell<u64>,
    outline_list: gtk::ListBox,
    sections: adw::ToggleGroup,
    split: adw::OverlaySplitView,
    toolbar: adw::ToolbarView,
    editor_col: gtk::Box,
    /// Modo «PDF en grande»: guarda si la barra lateral estaba visible.
    pdf_full: Cell<Option<bool>>,
}

impl ProjectWindow {
    fn new(app: &adw::Application, project: Project, first: &Path) -> Rc<ProjectWindow> {
        let data = SharedData::default();
        let editor = Editor::new(project.root.clone(), data.clone());
        let pdf = PdfView::new();

        // --- Barra superior ---
        let title = adw::WindowTitle::new(&project.name(), &tilde(&project.root));
        let header = adw::HeaderBar::new();
        header.set_title_widget(Some(&title));

        let sidebar_btn = gtk::ToggleButton::builder().icon_name("sidebar-show-symbolic").active(true).tooltip_text("Barra lateral (F9)").build();
        header.pack_start(&sidebar_btn);

        let menu = gio::Menu::new();
        let s1 = gio::Menu::new();
        s1.append(Some("Nuevo proyecto…"), Some("app.new-project"));
        s1.append(Some("Abrir proyecto…"), Some("win.open"));
        s1.append(Some("Abrir la demo"), Some("app.open-demo"));
        s1.append(Some("Nuevo archivo…"), Some("win.new-file"));
        s1.append(Some("Usar este archivo como principal"), Some("win.set-main"));
        menu.append_section(None, &s1);
        let s2 = gio::Menu::new();
        s2.append(Some("Guardar PDF en la carpeta del proyecto"), Some("win.export-pdf"));
        s2.append(Some("Abrir PDF en visor externo"), Some("win.open-pdf"));
        s2.append(Some("Limpiar archivos auxiliares"), Some("win.clean"));
        menu.append_section(None, &s2);
        let s3 = gio::Menu::new();
        s3.append_submenu(Some("Tema"), &theme_menu());
        s3.append(Some("Modo Vim"), Some("app.vim"));
        menu.append_section(None, &s3);
        let s4 = gio::Menu::new();
        s4.append(Some("Preferencias"), Some("app.preferences"));
        s4.append(Some("Atajos de teclado"), Some("app.shortcuts"));
        menu.append_section(None, &s4);
        let menu_btn = gtk::MenuButton::builder().icon_name("open-menu-symbolic").menu_model(&menu).primary(true).tooltip_text("Menú").build();

        let compile_btn = gtk::Button::builder()
            .icon_name("media-playback-start-symbolic")
            .tooltip_text("Compilar (Ctrl+Intro)")
            .action_name("win.compile")
            .css_classes(["compile-button"])
            .build();

        // Estado de la compilación: también abre el panel de problemas.
        let spinner = adw::Spinner::builder().visible(false).build();
        let status_icon = gtk::Image::from_icon_name("emblem-ok-symbolic");
        let status = gtk::Label::new(Some("Listo"));
        let status_box = gtk::Box::builder().spacing(6).build();
        status_box.append(&spinner);
        status_box.append(&status_icon);
        status_box.append(&status);
        let issues_btn = gtk::ToggleButton::builder().child(&status_box).tooltip_text("Problemas y salida (Ctrl+E)").css_classes(["flat", "compile-status"]).build();

        header.pack_end(&menu_btn);
        header.pack_end(&compile_btn);
        header.pack_end(&issues_btn);

        // --- Barra lateral ---
        let tree = FileTree::new(project.root.clone(), project.main.clone());
        let sections = adw::ToggleGroup::builder().css_classes(["round", "sidebar-switcher"]).homogeneous(true).hexpand(true).build();
        sections.add(adw::Toggle::builder().name("files").label("Archivos").build());
        sections.add(adw::Toggle::builder().name("outline").label("Esquema").build());
        sections.set_active_name(Some("files"));
        let new_file_btn = gtk::Button::builder().icon_name("document-new-symbolic").tooltip_text("Nuevo archivo (Ctrl+N)").action_name("win.new-file").css_classes(["flat"]).build();
        let new_folder_btn = gtk::Button::builder()
            .icon_name("folder-new-symbolic")
            .tooltip_text("Nueva carpeta")
            .action_name("win.tree-new-folder")
            .action_target(&"".to_variant())
            .css_classes(["flat"])
            .build();
        let sidebar_header = gtk::Box::builder().spacing(2).css_classes(["sidebar-header"]).build();
        sidebar_header.append(&sections);

        let files_title = gtk::Label::builder().label(project.name().to_uppercase()).xalign(0.0).hexpand(true).ellipsize(gtk::pango::EllipsizeMode::End).css_classes(["sidebar-title"]).build();
        let files_header = gtk::Box::builder().spacing(2).css_classes(["pane-header"]).build();
        files_header.append(&files_title);
        files_header.append(&new_file_btn);
        files_header.append(&new_folder_btn);
        let files_pane = gtk::Box::new(gtk::Orientation::Vertical, 0);
        files_pane.append(&files_header);
        files_pane.append(&tree.widget);

        let outline_list = gtk::ListBox::builder().css_classes(["navigation-sidebar", "outline"]).build();
        outline_list.set_placeholder(Some(
            &gtk::Label::builder().label("Sin secciones todavía.\nUsa \\section{…}").justify(gtk::Justification::Center).css_classes(["dim-label"]).margin_top(24).build(),
        ));
        let outline_scroller = gtk::ScrolledWindow::builder().child(&outline_list).hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).build();

        let sidebar_stack = gtk::Stack::builder().transition_type(gtk::StackTransitionType::Crossfade).vexpand(true).build();
        sidebar_stack.add_named(&files_pane, Some("files"));
        sidebar_stack.add_named(&outline_scroller, Some("outline"));
        sections.bind_property("active-name", &sidebar_stack, "visible-child-name").sync_create().build();

        let sidebar = gtk::Box::builder().orientation(gtk::Orientation::Vertical).css_classes(["rutex-sidebar"]).build();
        sidebar.append(&sidebar_header);
        sidebar.append(&sidebar_stack);

        // --- Panel de problemas ---
        let issues_list = gtk::ListBox::builder().css_classes(["issues"]).build();
        let output = gtk::TextView::builder().editable(false).monospace(true).wrap_mode(gtk::WrapMode::WordChar).left_margin(10).top_margin(6).build();
        let issues_stack = gtk::Stack::new();
        issues_stack.add_titled(&gtk::ScrolledWindow::builder().child(&issues_list).build(), Some("issues"), "Problemas");
        issues_stack.add_titled(&gtk::ScrolledWindow::builder().child(&output).build(), Some("output"), "Salida de latexmk");
        issues_stack.set_vexpand(true);
        let switcher = gtk::StackSwitcher::builder().stack(&issues_stack).halign(gtk::Align::Start).build();
        let close_issues = gtk::Button::builder().icon_name("window-close-symbolic").css_classes(["flat", "circular"]).halign(gtk::Align::End).hexpand(true).action_name("win.toggle-issues").build();
        let panel_header = gtk::Box::builder().css_classes(["panel-header"]).build();
        panel_header.append(&switcher);
        panel_header.append(&close_issues);
        let panel = gtk::Box::builder().orientation(gtk::Orientation::Vertical).height_request(200).css_classes(["issues-panel"]).build();
        panel.append(&panel_header);
        panel.append(&issues_stack);
        let issues_revealer = gtk::Revealer::builder().child(&panel).transition_type(gtk::RevealerTransitionType::SlideUp).build();

        // --- Barra de estado ---
        let engine_label = gtk::Label::new(None);
        let words_label = gtk::Label::new(None);
        let statusbar = gtk::Box::builder().spacing(16).css_classes(["statusbar"]).build();
        statusbar.append(&editor.position_label);
        statusbar.append(&editor.vim_label);
        statusbar.append(&gtk::Box::builder().hexpand(true).build());
        statusbar.append(&words_label);
        statusbar.append(&engine_label);

        let editor_col = gtk::Box::builder().orientation(gtk::Orientation::Vertical).css_classes(["editor-column"]).build();
        editor_col.append(&editor.widget);
        editor_col.append(&issues_revealer);
        editor_col.append(&statusbar);

        let main = gtk::Paned::builder().start_child(&editor_col).end_child(&pdf.widget).shrink_start_child(false).shrink_end_child(false).build();
        // Con Hyprland el tamaño de la ventana lo decide el gestor: repartimos
        // el espacio cuando sabemos cuánto hay…
        // y mantenemos la proporción cuando Hyprland cambia el tamaño de la ventana
        // (si arrastras el separador, se recuerda tu proporción).
        let ratio = Rc::new(Cell::new(0.52));
        let adjusting = Rc::new(Cell::new(false));
        let (r, a) = (ratio.clone(), adjusting.clone());
        main.connect_max_position_notify(move |paned| {
            if paned.max_position() > 400 {
                a.set(true);
                paned.set_position((paned.max_position() as f64 * r.get()) as i32);
                a.set(false);
            }
        });
        main.connect_position_notify(move |paned| {
            if !adjusting.get() && paned.max_position() > 400 {
                ratio.set((paned.position() as f64 / paned.max_position() as f64).clamp(0.2, 0.8));
            }
        });

        let split = adw::OverlaySplitView::builder()
            .sidebar(&sidebar)
            .content(&main)
            .min_sidebar_width(230.0)
            .max_sidebar_width(320.0)
            .sidebar_width_fraction(0.16)
            .build();
        sidebar_btn.bind_property("active", &split, "show-sidebar").bidirectional().sync_create().build();

        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&split));

        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&header);
        toolbar.set_content(Some(&toasts));
        toolbar.set_top_bar_style(adw::ToolbarStyle::Flat);

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
            toasts,
            tree,
            spinner,
            status_icon,
            status,
            issues_btn,
            engine_label,
            words_label,
            issues_revealer,
            issues_list,
            issues_stack,
            output,
            issues: RefCell::default(),
            compile: RefCell::default(),
            autosave: RefCell::default(),
            pdf_mtime: Cell::new(None),
            data,
            index_generation: Cell::new(0),
            outline_list,
            sections,
            split,
            toolbar,
            editor_col,
            pdf_full: Cell::new(None),
        });

        this.issues_btn.bind_property("active", &this.issues_revealer, "reveal-child").sync_create().build();
        this.editor.set_vim(config::get().vim);
        this.connect_signals();
        this.add_actions(&sidebar_btn);
        this.open_file(first);
        this.editor.focus();

        // PDF de la sesión anterior: vista previa instantánea mientras se recompila.
        let pdf_path = this.project.borrow().pdf_path();
        if let Ok(mtime) = fs::metadata(&pdf_path).and_then(|m| m.modified()) {
            this.pdf_mtime.set(Some(mtime));
            this.pdf.load(pdf_path);
        }
        this.compile();
        this.refresh_index();
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
        self.pdf.connect_dark_toggled(move |on| {
            if weak.upgrade().is_some() {
                config::update(|c| c.pdf_dark = on);
                for_each_project(|pw| {
                    pw.pdf.set_dark_button(on);
                    pw.pdf.refresh_colors();
                });
            }
        });
        let weak = Rc::downgrade(self);
        self.pdf.connect_fullscreen(move |_| {
            if let Some(this) = weak.upgrade() {
                this.toggle_pdf_fullscreen();
            }
        });
        // Esc sale del modo «PDF en grande» (en captura: el foco puede estar en
        // un widget oculto que se tragaría la tecla).
        let esc = gtk::EventControllerKey::new();
        esc.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        esc.connect_key_pressed(move |_, key, _, _| {
            if let Some(this) = weak.upgrade()
                && key == gtk::gdk::Key::Escape
                && this.pdf_full.get().is_some()
            {
                this.toggle_pdf_fullscreen();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        self.window.add_controller(esc);

        let weak = Rc::downgrade(self);
        self.editor.connect_issue_click(move |rel, line| {
            if let Some(this) = weak.upgrade() {
                this.show_issue(rel, line);
            }
        });
        let weak = Rc::downgrade(self);
        self.editor.connect_save(move || {
            if let Some(this) = weak.upgrade() {
                this.save_and_compile();
            }
        });

        let weak = Rc::downgrade(self);
        self.editor.connect_switch(move || {
            if let Some(this) = weak.upgrade() {
                if let Some(rel) = this.editor.current() {
                    this.tree.select(&rel);
                }
            }
        });

        let weak = Rc::downgrade(self);
        self.outline_list.connect_row_activated(move |_, row| {
            let Some(this) = weak.upgrade() else { return };
            let target = this.data.borrow().index.outline.get(row.index() as usize).map(|s| (s.file.clone(), s.line));
            if let Some((file, line)) = target {
                this.goto(&file, line);
            }
        });

        let weak = Rc::downgrade(self);
        self.tree.connect_open(move |rel| {
            if let Some(this) = weak.upgrade() {
                this.open_path(rel);
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
        add("goto-line", Box::new(|t| t.goto_line_dialog()));
        add("quick-open", Box::new(|t| t.quick_open()));
        add("zoom-in", Box::new(|t| t.pdf.zoom(1)));
        add("zoom-out", Box::new(|t| t.pdf.zoom(-1)));
        add("zoom-fit", Box::new(|t| t.pdf.zoom(0)));
        add("new-file", Box::new(|t| {
            let dir = t.editor.current().and_then(|c| c.parent().map(Path::to_path_buf)).unwrap_or_default();
            t.new_file_dialog(&dir);
        }));
        add("set-main", Box::new(|t| {
            if let Some(rel) = t.editor.current() {
                t.set_main(&rel);
            }
        }));
        add("export-pdf", Box::new(|t| t.export_pdf()));
        add("open-pdf", Box::new(|t| t.open_pdf_external()));
        add("clean", Box::new(|t| t.clean()));
        add("show-files", Box::new(|t| t.show_sidebar("files")));
        add("show-outline", Box::new(|t| t.show_sidebar("outline")));
        add("pdf-fullscreen", Box::new(|t| t.toggle_pdf_fullscreen()));
        add("close-tab", Box::new(|t| t.editor.close_current()));
        add("toggle-issues", Box::new(|t| t.issues_btn.set_active(!t.issues_revealer.reveals_child())));
        let btn = sidebar_btn.clone();
        add("toggle-sidebar", Box::new(move |_| btn.set_active(!btn.is_active())));

        // Acciones del árbol de archivos: reciben la ruta relativa como parámetro.
        type PathHandler = Box<dyn Fn(&Rc<ProjectWindow>, PathBuf)>;
        let add_path = |name: &str, f: PathHandler| {
            let action = gio::SimpleAction::new(name, Some(glib::VariantTy::STRING));
            let weak = Rc::downgrade(self);
            action.connect_activate(move |_, param| {
                if let (Some(this), Some(rel)) = (weak.upgrade(), param.and_then(|p| p.get::<String>())) {
                    f(&this, PathBuf::from(rel));
                }
            });
            self.window.add_action(&action);
        };
        add_path("tree-new-file", Box::new(|t, dir| t.new_file_dialog(&dir)));
        add_path("tree-new-folder", Box::new(|t, dir| t.new_folder_dialog(&dir)));
        add_path("tree-rename", Box::new(|t, rel| t.rename_dialog(&rel)));
        add_path("tree-trash", Box::new(|t, rel| t.trash_dialog(&rel)));
        add_path("tree-set-main", Box::new(|t, rel| t.set_main(&rel)));
        add_path("tree-copy-path", Box::new(|t, rel| {
            let abs = t.project.borrow().root.join(&rel);
            t.window.clipboard().set_text(&abs.to_string_lossy());
            t.toast("Ruta copiada");
        }));
        add_path("tree-reveal", Box::new(|t, rel| {
            let abs = t.project.borrow().root.join(&rel);
            let file = gio::File::for_path(&abs);
            let launcher = gtk::FileLauncher::new(Some(&file));
            if abs.is_dir() {
                launcher.launch(Some(&t.window), gio::Cancellable::NONE, |_| {});
            } else {
                launcher.open_containing_folder(Some(&t.window), gio::Cancellable::NONE, |_| {});
            }
        }));
    }

    fn toast(&self, msg: &str) {
        self.toasts.add_toast(adw::Toast::builder().title(glib::markup_escape_text(msg)).timeout(3).build());
    }

    fn open_file(self: &Rc<Self>, rel: &Path) {
        if let Err(e) = self.editor.open(rel) {
            self.toast(&e);
        }
    }

    fn goto(self: &Rc<Self>, rel: &Path, line: u32) {
        if let Err(e) = self.editor.goto(rel, line) {
            self.toast(&e);
        }
    }

    fn refresh_files(self: &Rc<Self>) {
        let before = self.project.borrow().files.clone();
        self.project.borrow_mut().rescan();
        self.tree.refresh();
        if self.project.borrow().files != before {
            self.refresh_index();
        }
    }

    /// Recalcula esquema, etiquetas y bibliografía en un hilo aparte.
    fn refresh_index(self: &Rc<Self>) {
        let generation = self.index_generation.get() + 1;
        self.index_generation.set(generation);
        let p = self.project.borrow().clone();
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let built = gio::spawn_blocking(move || {
                let idx = index::build(&p.root, &p.main, &p.files);
                let tex: Vec<PathBuf> = p.files.iter().filter(|f| crate::project::has_ext(f, "tex")).cloned().collect();
                (idx, tex, crate::project::images(&p.root))
            })
            .await;
            let Some(this) = weak.upgrade() else { return };
            if this.index_generation.get() != generation {
                return;
            }
            if let Ok((idx, tex_files, images)) = built {
                this.fill_outline(&idx.outline);
                *this.data.borrow_mut() = Data { index: idx, tex_files, images };
            }
        });
    }

    /// Pantalla completa solo con el PDF (F11 / Esc).
    fn toggle_pdf_fullscreen(&self) {
        match self.pdf_full.take() {
            Some(sidebar) => {
                self.window.unfullscreen();
                self.toolbar.set_reveal_top_bars(true);
                self.editor_col.set_visible(true);
                self.split.set_show_sidebar(sidebar);
                self.pdf.set_fullscreen_button(false);
                self.editor.focus();
            }
            None => {
                self.pdf_full.set(Some(self.split.shows_sidebar()));
                self.split.set_show_sidebar(false);
                self.editor_col.set_visible(false);
                self.toolbar.set_reveal_top_bars(false);
                self.window.fullscreen();
                self.pdf.set_fullscreen_button(true);
            }
        }
    }

    fn show_sidebar(&self, name: &str) {
        self.split.set_show_sidebar(true);
        self.sections.set_active_name(Some(name));
    }

    fn fill_outline(&self, outline: &[Section]) {
        while let Some(child) = self.outline_list.first_child() {
            self.outline_list.remove(&child);
        }
        let min = outline.iter().map(|s| s.level).min().unwrap_or(0);
        for section in outline {
            let depth = (section.level - min) as i32;
            let label = gtk::Label::builder()
                .label(&section.title)
                .xalign(0.0)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .margin_start(depth * 14)
                .tooltip_text(format!("{}:{}", section.file.display(), section.line))
                .css_classes([format!("outline-depth-{}", depth.min(3))])
                .build();
            self.outline_list.append(&label);
        }
    }

    /// Abre un archivo del árbol: los de texto en el editor y el resto (imágenes,
    /// PDF…) con la aplicación del sistema.
    fn open_path(self: &Rc<Self>, rel: &Path) {
        let abs = self.project.borrow().root.join(rel);
        if crate::project::is_text(&abs) {
            self.open_file(rel);
        } else {
            gtk::FileLauncher::new(Some(&gio::File::for_path(abs))).launch(Some(&self.window), gio::Cancellable::NONE, |_| {});
        }
    }

    /// Diálogo con un campo de texto. `on_ok` recibe el texto ya validado como
    /// ruta relativa segura (sin `..` ni `/` inicial).
    fn ask_path(self: &Rc<Self>, heading: &str, body: &str, initial: &str, ok_label: &str, on_ok: impl Fn(&Rc<Self>, PathBuf) + 'static) {
        let entry = gtk::Entry::builder().text(initial).activates_default(true).build();
        let dialog = adw::AlertDialog::builder()
            .heading(heading)
            .body(body)
            .extra_child(&entry)
            .default_response("ok")
            .close_response("cancel")
            .build();
        dialog.add_responses(&[("cancel", "Cancelar"), ("ok", ok_label)]);
        dialog.set_response_appearance("ok", adw::ResponseAppearance::Suggested);
        let weak = Rc::downgrade(self);
        let input = entry.clone();
        dialog.connect_response(Some("ok"), move |_, _| {
            let Some(this) = weak.upgrade() else { return };
            let text = input.text().trim().trim_matches('/').to_string();
            if text.is_empty() || text.split('/').any(|c| c == ".." || c.is_empty()) {
                this.toast("Nombre no válido");
                return;
            }
            on_ok(&this, PathBuf::from(text));
        });
        dialog.present(Some(&self.window));
        entry.grab_focus();
        entry.set_position(-1);
    }

    fn schedule_autosave(self: &Rc<Self>) {
        if let Some(id) = self.autosave.borrow_mut().take() {
            id.remove();
        }
        let weak = Rc::downgrade(self);
        let cfg = config::get();
        let id = glib::timeout_add_local_once(Duration::from_millis(cfg.compile_delay_ms as u64), move || {
            if let Some(this) = weak.upgrade() {
                this.autosave.borrow_mut().take();
                if cfg.autocompile {
                    this.save_and_compile();
                } else {
                    this.save();
                    this.refresh_index();
                }
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
        self.refresh_index();
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
                self.set_status(Status::Busy, "Compilando…");
                self.engine_label.set_label(running.engine.label());
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
                self.set_status(Status::Error, "Sin TeX");
                self.output.buffer().set_text(&msg);
                self.issues_stack.set_visible_child_name("output");
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
        let mut marks: HashMap<PathBuf, Vec<(u32, bool, String)>> = HashMap::new();
        for i in &shown {
            if let (Some(f), Some(l)) = (&i.file, i.line) {
                marks.entry(f.clone()).or_default().push((l, i.severity == Severity::Error, i.message.clone()));
            }
        }
        self.editor.set_issues(marks);
        if outcome.pdf.is_some() {
            self.update_word_count();
        }
        *self.issues.borrow_mut() = shown;
        self.output.buffer().set_text(&outcome.output);

        let secs = format!("{:.1}", outcome.elapsed.as_secs_f64()).replace('.', ",");
        match (errors, warnings) {
            (0, _) if !outcome.ok && outcome.pdf.is_none() => self.set_status(Status::Error, "Falló la compilación"),
            (0, 0) => self.set_status(Status::Ok, &format!("{secs} s")),
            (0, w) => self.set_status(Status::Warning, &format!("{w} {}", if w == 1 { "aviso" } else { "avisos" })),
            (1, _) => self.set_status(Status::Error, "1 error"),
            (e, _) => self.set_status(Status::Error, &format!("{e} errores")),
        }

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

    fn update_word_count(self: &Rc<Self>) {
        let p = self.project.borrow().clone();
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let words = compiler::word_count(p.root, p.main).await;
            if let (Some(this), Some(n)) = (weak.upgrade(), words) {
                this.words_label.set_label(&format!("{} palabras", thousands(n)));
            }
        });
    }

    /// Abre el panel de problemas con el de esa línea seleccionado.
    fn show_issue(&self, rel: &Path, line: u32) {
        self.issues_stack.set_visible_child_name("issues");
        self.issues_btn.set_active(true);
        let idx = self.issues.borrow().iter().position(|i| i.file.as_deref() == Some(rel) && i.line == Some(line));
        if let Some(row) = idx.and_then(|i| self.issues_list.row_at_index(i as i32)) {
            self.issues_list.select_row(Some(&row));
            row.grab_focus();
        }
    }

    fn set_status(&self, status: Status, text: &str) {
        let busy = status == Status::Busy;
        self.spinner.set_visible(busy);
        self.status_icon.set_visible(!busy);
        self.status_icon.set_icon_name(Some(match status {
            Status::Ok | Status::Busy => "emblem-ok-symbolic",
            Status::Warning => "dialog-warning-symbolic",
            Status::Error => "dialog-error-symbolic",
        }));
        for class in ["ok", "warning", "error", "busy"] {
            self.issues_btn.remove_css_class(class);
        }
        self.issues_btn.add_css_class(match status {
            Status::Ok => "ok",
            Status::Warning => "warning",
            Status::Error => "error",
            Status::Busy => "busy",
        });
        self.status.set_label(text);
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

    fn quick_open(self: &Rc<Self>) {
        let files = self.project.borrow().files.clone();
        let sections = self.data.borrow().index.outline.clone();
        let weak = Rc::downgrade(self);
        quickopen::show(&self.window, files, sections, move |target| {
            let Some(this) = weak.upgrade() else { return };
            match target {
                quickopen::Target::File(rel) => this.open_path(&rel),
                quickopen::Target::Section(rel, line) => this.goto(&rel, line),
            }
        });
    }

    fn goto_line_dialog(self: &Rc<Self>) {
        let Some(rel) = self.editor.current() else { return };
        let entry = gtk::Entry::builder().input_purpose(gtk::InputPurpose::Digits).activates_default(true).placeholder_text("Número de línea").build();
        let dialog = adw::AlertDialog::builder().heading("Ir a la línea").extra_child(&entry).default_response("go").close_response("cancel").build();
        dialog.add_responses(&[("cancel", "Cancelar"), ("go", "Ir")]);
        dialog.set_response_appearance("go", adw::ResponseAppearance::Suggested);
        let weak = Rc::downgrade(self);
        let input = entry.clone();
        dialog.connect_response(Some("go"), move |_, _| {
            if let (Some(this), Ok(line)) = (weak.upgrade(), input.text().trim().parse::<u32>()) {
                this.goto(&rel, line.max(1));
            }
        });
        dialog.present(Some(&self.window));
        entry.grab_focus();
    }

    fn new_file_dialog(self: &Rc<Self>, dir: &Path) {
        let initial = if dir.as_os_str().is_empty() { String::new() } else { format!("{}/", dir.display()) };
        self.ask_path("Nuevo archivo", "Ruta dentro del proyecto. Si no pones extensión, será .tex.", &initial, "Crear", |this, mut rel| {
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
                    this.toast(&format!("Inclúyelo con \\input{{{}}}", rel.with_extension("").display()));
                }
            }
            this.refresh_files();
            this.open_file(&rel);
        });
    }

    fn new_folder_dialog(self: &Rc<Self>, dir: &Path) {
        let initial = if dir.as_os_str().is_empty() { String::new() } else { format!("{}/", dir.display()) };
        self.ask_path("Nueva carpeta", "Ruta dentro del proyecto", &initial, "Crear", |this, rel| {
            let abs = this.project.borrow().root.join(&rel);
            if let Err(e) = fs::create_dir_all(&abs) {
                this.toast(&format!("No se pudo crear: {e}"));
                return;
            }
            this.refresh_files();
            this.tree.select(&rel);
        });
    }

    fn rename_dialog(self: &Rc<Self>, rel: &Path) {
        let from = rel.to_path_buf();
        let body = format!("Nuevo nombre o ruta para «{}»", rel.display());
        self.ask_path("Renombrar", &body, &rel.display().to_string(), "Renombrar", move |this, to| {
            if to == from {
                return;
            }
            let root = this.project.borrow().root.clone();
            if root.join(&to).exists() {
                this.toast(&format!("Ya existe {}", to.display()));
                return;
            }
            let moved = to.parent().map_or(Ok(()), |p| fs::create_dir_all(root.join(p))).and_then(|_| fs::rename(root.join(&from), root.join(&to)));
            if let Err(e) = moved {
                this.toast(&format!("No se pudo renombrar: {e}"));
                return;
            }
            this.editor.rename(&from, &to);
            let main = this.project.borrow().main.clone();
            if let Ok(rest) = main.strip_prefix(&from) {
                let new_main = if rest.as_os_str().is_empty() { to.clone() } else { to.join(rest) };
                this.set_main(&new_main);
            }
            this.refresh_files();
        });
    }

    fn trash_dialog(self: &Rc<Self>, rel: &Path) {
        let dialog = adw::AlertDialog::builder()
            .heading("¿Mover a la papelera?")
            .body(format!("«{}» se moverá a la papelera. Podrás recuperarlo desde tu gestor de archivos.", rel.display()))
            .default_response("cancel")
            .close_response("cancel")
            .build();
        dialog.add_responses(&[("cancel", "Cancelar"), ("trash", "Mover a la papelera")]);
        dialog.set_response_appearance("trash", adw::ResponseAppearance::Destructive);
        let weak = Rc::downgrade(self);
        let rel = rel.to_path_buf();
        dialog.connect_response(Some("trash"), move |_, _| {
            let Some(this) = weak.upgrade() else { return };
            let abs = this.project.borrow().root.join(&rel);
            match gio::File::for_path(&abs).trash(gio::Cancellable::NONE) {
                Ok(()) => {
                    this.editor.forget(&rel);
                    this.refresh_files();
                    if this.project.borrow().main.starts_with(&rel) {
                        this.toast("Has borrado el archivo principal: elige otro con clic derecho → Usar como archivo principal");
                    }
                }
                Err(e) => this.toast(&format!("No se pudo mover a la papelera: {e}")),
            }
        });
        dialog.present(Some(&self.window));
    }

    fn set_main(self: &Rc<Self>, rel: &Path) {
        let rel = rel.to_path_buf();
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
        self.tree.set_main(rel.clone());
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
