//! Ventanas: pantalla de bienvenida y una ventana por proyecto.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use adw::prelude::*;
use gtk::{gio, glib};

use crate::compiler::{self, Outcome};
use crate::config::{self, ThemeMode};
use crate::demo;
use crate::editor::Editor;
use crate::filetree::FileTree;
use crate::history::History;
use crate::completion::{Data, SharedData};
use crate::index::{self, Section};
use crate::logparse::Severity;
use crate::newproject;
use crate::pdf::PdfView;
use crate::prefs::{self, Change};
use crate::quickopen;
use crate::project::Project;
use crate::synctex;
use crate::i18n::{tr, trf};

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
        ("win.history", &["<Ctrl><Shift>h"]),
        ("win.check", &["<Ctrl><Shift>r"]),
        ("win.symbols", &["<Ctrl><Shift>m"]),
        ("win.fold", &["<Alt>minus", "<Alt>KP_Subtract"]),
        ("win.unfold", &["<Alt>plus", "<Alt>equal", "<Alt>KP_Add"]),
        ("win.fold-all", &["<Ctrl><Alt>minus"]),
        ("win.unfold-all", &["<Ctrl><Alt>plus", "<Ctrl><Alt>equal"]),
        ("win.save-version", &["<Ctrl><Shift>s"]),
        ("win.zoom-in", &["<Ctrl>plus", "<Ctrl>equal", "<Ctrl>KP_Add"]),
        ("win.zoom-out", &["<Ctrl>minus", "<Ctrl>KP_Subtract"]),
        ("win.zoom-fit", &["<Ctrl>0"]),
        ("win.toggle-sidebar", &["F9"]),
        ("win.pdf-fullscreen", &["F11"]),
        ("win.toggle-issues", &["<Ctrl>e"]),
        ("win.show-files", &["<Ctrl><Shift>e"]),
        ("win.show-outline", &["<Ctrl><Shift>o"]),
        ("win.find-in-project", &["<Ctrl><Shift>f"]),
        ("win.close-tab", &["<Ctrl>w"]),
        ("window.close", &["<Ctrl><Shift>w"]),
        ("app.quit", &["<Ctrl>q"]),
        ("app.preferences", &["<Ctrl>comma"]),
        ("app.new-project", &["<Ctrl><Shift>n"]),
        ("app.new-window", &["<Ctrl><Alt>n"]),
        ("app.shortcuts", &["<Ctrl>question", "<Ctrl><Shift>question"]),
    ];
    for (action, keys) in accels {
        app.set_accels_for_action(action, keys);
    }
}

pub fn setup_app_actions(app: &adw::Application) {
    let new_window = gio::SimpleAction::new("new-window", None);
    let weak = app.downgrade();
    new_window.connect_activate(move |_, _| {
        if let Some(app) = weak.upgrade() {
            show_welcome(&app);
        }
    });
    app.add_action(&new_window);

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

    let spellcheck = gio::SimpleAction::new_stateful("spellcheck", None, &config::get().spellcheck.to_variant());
    let weak = app.downgrade();
    spellcheck.connect_activate(move |action, _| {
        let on = !action.state().and_then(|s| s.get::<bool>()).unwrap_or(true);
        config::update(|c| c.spellcheck = on);
        if let Some(app) = weak.upgrade() {
            apply_change(&app, Change::Spelling);
        }
    });
    app.add_action(&spellcheck);

    let spell_lang = gio::SimpleAction::new_stateful("spell-lang", Some(glib::VariantTy::STRING), &config::get().spell_lang.to_variant());
    let weak = app.downgrade();
    spell_lang.connect_activate(move |_, param| {
        let Some(code) = param.and_then(|p| p.get::<String>()) else { return };
        config::update(|c| c.spell_lang = code);
        if let Some(app) = weak.upgrade() {
            apply_change(&app, Change::Spelling);
        }
    });
    app.add_action(&spell_lang);

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
            Err(e) => report_error(&app, &trf("Could not prepare the demo: {}", &[&e])),
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
        Change::Language => {
            crate::i18n::set_spanish(config::get().spanish);
            // Fuera del manejador del diálogo, que se destruye con su ventana.
            let app = app.clone();
            glib::idle_add_local_once(move || rebuild_windows(&app));
        }
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
        Change::Spelling => for_each_project(|pw| {
            let cfg = config::get();
            for (name, value) in [("spellcheck", cfg.spellcheck.to_variant()), ("spell-lang", cfg.spell_lang.to_variant())] {
                if let Some(a) = app.lookup_action(name).and_downcast::<gio::SimpleAction>() {
                    a.set_state(&value);
                }
            }
            pw.editor.set_spellcheck(config::get().spellcheck);
            pw.update_spell_language();
        }),
    }
}

/// Vuelve a crear todas las ventanas (p. ej. al cambiar de idioma) con los
/// mismos proyectos abiertos. Todo se guarda al cerrar.
fn rebuild_windows(app: &adw::Application) {
    let _hold = app.hold(); // que la app no se cierre al quedarse sin ventanas
    let roots: Vec<PathBuf> = PROJECTS.with(|p| p.borrow().iter().map(|pw| pw.project.borrow().root.clone()).collect());
    let projects = PROJECTS.with(|p| p.borrow().clone());
    for pw in projects {
        pw.window.close();
    }
    if let Some(w) = WELCOME.with(|w| w.borrow_mut().take()).and_then(|w| w.upgrade()) {
        w.close();
    }
    if roots.is_empty() {
        show_welcome(app);
    }
    for root in roots {
        open_project(app, &root, false);
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

/// Launching rutex again (from the app launcher, say) opens another window
/// in the same process, where another project can be opened: cheaper than a
/// second process, which would load GTK, the theme and the dictionaries again.
pub fn activate(app: &adw::Application) {
    show_welcome(app);
}

/// Abre un proyecto (carpeta o archivo .tex). Si ya está abierto, lo enfoca.
pub fn open_project(app: &adw::Application, path: &Path, create: bool) {
    let Ok(canon) = fs::canonicalize(path) else {
        report_error(app, &trf("Does not exist: {}", &[&path.display()]));
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
        Err(e) => report_error(app, &trf("Could not open {}:\n{}", &[&canon.display(), &e])),
    }
}

fn report_error(app: &adw::Application, msg: &str) {
    let dialog = adw::AlertDialog::new(Some("rutex"), Some(msg));
    dialog.add_response("ok", tr("OK"));
    match app.active_window() {
        Some(w) => dialog.present(Some(&w)),
        None => {
            show_welcome(app);
            dialog.present(app.active_window().as_ref());
        }
    }
}

fn pick_folder(app: &adw::Application, parent: Option<&gtk::Window>, create: bool) {
    let title = if create { tr("New project folder") } else { tr("Open project") };
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
    let subtitle = gtk::Label::builder().label(tr("LaTeX editor with live compilation")).xalign(0.0).css_classes(["dim-label"]).build();
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
    let new = action_card("document-new-symbolic", tr("New project"), tr("From a template: article, thesis, notes, slides…"));
    new.set_action_name(Some("app.new-project"));
    let open = action_card("folder-open-symbolic", tr("Open folder"), tr("A LaTeX project you already have"));
    let demo = action_card("starred-symbolic", tr("Try the demo"), tr("A complete project with figures, plots and a bibliography"));
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
        let group = adw::PreferencesGroup::builder().title(tr("Recent")).build();
        for path in &recent {
            let row = adw::ActionRow::builder()
                .title(glib::markup_escape_text(&path.file_name().unwrap_or_default().to_string_lossy()))
                .subtitle(glib::markup_escape_text(&tilde(path)))
                .activatable(true)
                .build();
            row.add_prefix(&gtk::Image::from_icon_name("folder-symbolic"));
            let forget = gtk::Button::builder().icon_name("window-close-symbolic").valign(gtk::Align::Center).tooltip_text(tr("Remove from recent")).css_classes(["flat", "circular"]).build();
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
    content.append(&gtk::Label::builder().label(tr("Ctrl+, preferences  ·  Ctrl+? keyboard shortcuts")).css_classes(["dim-label", "caption"]).build());

    let clamp = adw::Clamp::builder().maximum_size(640).child(&content).build();
    let scroller = gtk::ScrolledWindow::builder().child(&clamp).hscrollbar_policy(gtk::PolicyType::Never).build();
    let header = adw::HeaderBar::builder().show_title(false).build();
    let menu = gio::Menu::new();
    menu.append_submenu(Some(tr("Theme")), &theme_menu());
    menu.append(Some(tr("Preferences")), Some("app.preferences"));
    menu.append(Some(tr("Keyboard shortcuts")), Some("app.shortcuts"));
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
            out.push(crate::i18n::thousands_sep());
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

/// Widgets of the "Search" sidebar page.
struct SearchUi {
    entry: gtk::SearchEntry,
    case: gtk::ToggleButton,
    word: gtk::ToggleButton,
    replace: gtk::Entry,
    replace_btn: gtk::Button,
    summary: gtk::Label,
    results: gtk::ListBox,
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
    sync_btn: gtk::Button,
    engine_label: gtk::Label,
    words_label: gtk::Label,
    words_popover: gtk::Popover,
    word_count: RefCell<Option<compiler::WordCount>>,
    spell_label: gtk::Label,
    title_label: gtk::Label,
    issues_revealer: gtk::Revealer,
    issues_list: gtk::ListBox,
    issues_stack: gtk::Stack,
    output: gtk::TextView,
    issues: RefCell<Vec<ShownIssue>>,
    badboxes: RefCell<Vec<ShownIssue>>,
    missing_banner: adw::Banner,
    install_command: RefCell<String>,
    compile: RefCell<CompileState>,
    autosave: RefCell<Option<glib::SourceId>>,
    pdf_mtime: Cell<Option<SystemTime>>,
    data: SharedData,
    index_generation: Cell<u64>,
    outline_list: gtk::ListBox,
    search: SearchUi,
    search_hits: RefCell<Vec<Option<crate::search::Hit>>>,
    search_generation: Cell<u64>,
    sections: adw::ToggleGroup,
    split: adw::OverlaySplitView,
    toolbar: adw::ToolbarView,
    editor_col: gtk::Box,
    /// Modo «PDF en grande»: guarda si la barra lateral estaba visible.
    pdf_full: Cell<Option<bool>>,
    /// Local version history (None if git is not installed).
    history: Option<Arc<History>>,
    math: Rc<crate::mathpreview::MathPreview>,
    /// History version the margin change bars compare against.
    baseline: RefCell<Option<String>>,
}

impl ProjectWindow {
    fn new(app: &adw::Application, project: Project, first: &Path) -> Rc<ProjectWindow> {
        let data = SharedData::default();
        let editor = Editor::new(project.root.clone(), data.clone());
        let pdf = PdfView::new();

        // --- Barra superior ---
        // «Centro de mando»: proyecto y archivo actual; al pulsarlo, apertura rápida.
        let title_label = gtk::Label::builder().label(project.name()).ellipsize(gtk::pango::EllipsizeMode::Middle).hexpand(true).build();
        let title_box = gtk::Box::builder().spacing(8).build();
        title_box.append(&gtk::Image::from_icon_name("system-search-symbolic"));
        title_box.append(&title_label);
        title_box.append(&gtk::Label::builder().label("Ctrl P").css_classes(["kbd"]).build());
        let title = gtk::Button::builder()
            .child(&title_box)
            .action_name("win.quick-open")
            .tooltip_text(tilde(&project.root))
            .css_classes(["command-center"])
            .build();
        let header = adw::HeaderBar::new();
        header.set_title_widget(Some(&title));

        let sidebar_btn = gtk::ToggleButton::builder().icon_name("sidebar-show-symbolic").active(true).tooltip_text(tr("Sidebar (F9)")).build();
        header.pack_start(&sidebar_btn);

        // Every feature is reachable from here; GTK shows each shortcut next to it.
        let menu = gio::Menu::new();
        let section = |title: &str, items: &[(&str, &str)]| {
            let s = gio::Menu::new();
            for (label, action) in items {
                s.append(Some(label), Some(action));
            }
            menu.append_section(Some(title), &s);
        };
        section(tr("Project"), &[
            (tr("New file…"), "win.new-file"),
            (tr("New window"), "app.new-window"),
            (tr("New project…"), "app.new-project"),
            (tr("Open project…"), "win.open"),
            (tr("Open the demo"), "app.open-demo"),
            (tr("Use this file as main"), "win.set-main"),
        ]);
        section(tr("Tools"), &[
            (tr("Find in project"), "win.find-in-project"),
            (tr("Quick open"), "win.quick-open"),
            (tr("Check before submitting…"), "win.check"),
            (tr("Version history…"), "win.history"),
            (tr("Sync with Git"), "win.sync"),
            (tr("Save a version…"), "win.save-version"),
        ]);
        section(tr("PDF"), &[
            (tr("Fullscreen PDF"), "win.pdf-fullscreen"),
            (tr("Save PDF to the project folder"), "win.export-pdf"),
            (tr("Export to Word (.docx)…"), "win.export-docx"),
            (tr("Open PDF in external viewer"), "win.open-pdf"),
            (tr("Clean auxiliary files"), "win.clean"),
        ]);
        let view = gio::Menu::new();
        view.append_submenu(Some(tr("Theme")), &theme_menu());
        view.append(Some(tr("Fold section")), Some("win.fold"));
        view.append(Some(tr("Unfold section")), Some("win.unfold"));
        view.append(Some(tr("Fold all sections")), Some("win.fold-all"));
        view.append(Some(tr("Unfold everything")), Some("win.unfold-all"));
        view.append(Some(tr("Check spelling")), Some("app.spellcheck"));
        view.append(Some(tr("Vim mode")), Some("app.vim"));
        menu.append_section(Some(tr("View")), &view);
        let s4 = gio::Menu::new();
        s4.append(Some(tr("Preferences")), Some("app.preferences"));
        s4.append(Some(tr("Keyboard shortcuts")), Some("app.shortcuts"));
        menu.append_section(None, &s4);
        let menu_btn = gtk::MenuButton::builder().icon_name("open-menu-symbolic").menu_model(&menu).primary(true).tooltip_text(tr("Menu")).build();

        let compile_btn = gtk::Button::builder()
            .icon_name("media-playback-start-symbolic")
            .tooltip_text(tr("Compile (Ctrl+Enter)"))
            .action_name("win.compile")
            .css_classes(["compile-button"])
            .build();

        // Estado de la compilación: también abre el panel de problemas.
        let spinner = adw::Spinner::builder().visible(false).build();
        let status_icon = gtk::Image::from_icon_name("emblem-ok-symbolic");
        let status = gtk::Label::new(Some(tr("Ready")));
        let status_box = gtk::Box::builder().spacing(6).build();
        status_box.append(&spinner);
        status_box.append(&status_icon);
        status_box.append(&status);
        let issues_btn = gtk::ToggleButton::builder().child(&status_box).tooltip_text(tr("Problems and output (Ctrl+E)")).css_classes(["flat", "compile-status"]).build();

        let history_btn = gtk::Button::builder().icon_name("document-open-recent-symbolic").tooltip_text(tr("Version history (Ctrl+Shift+H)")).action_name("win.history").build();
        // Only shown for projects in a Git repository with a remote.
        let sync_btn = gtk::Button::builder().icon_name("emblem-synchronizing-symbolic").tooltip_text(tr("Sync with Git")).action_name("win.sync").visible(false).build();
        header.pack_end(&menu_btn);
        // Its own button: images, tables, symbols and references are easy to find.
        let insert_btn = gtk::MenuButton::builder()
            .child(&adw::ButtonContent::builder().icon_name("list-add-symbolic").label(tr("Insert")).build())
            .menu_model(&crate::insert::menu())
            .tooltip_text(tr("Insert an image, a table, a symbol or a reference"))
            .css_classes(["flat"])
            .build();
        header.pack_start(&insert_btn);
        header.pack_end(&history_btn);
        header.pack_end(&sync_btn);
        header.pack_end(&compile_btn);
        header.pack_end(&issues_btn);

        // --- Barra lateral ---
        let tree = FileTree::new(project.root.clone(), project.main.clone());
        let sections = adw::ToggleGroup::builder().css_classes(["round", "sidebar-switcher"]).homogeneous(true).hexpand(true).build();
        sections.add(adw::Toggle::builder().name("files").label(tr("Files")).build());
        sections.add(adw::Toggle::builder().name("outline").label(tr("Outline")).build());
        sections.add(adw::Toggle::builder().name("search").label(tr("Search")).build());
        sections.set_active_name(Some("files"));
        let new_file_btn = gtk::Button::builder().icon_name("document-new-symbolic").tooltip_text(tr("New file (Ctrl+N)")).action_name("win.new-file").css_classes(["flat"]).build();
        let new_folder_btn = gtk::Button::builder()
            .icon_name("folder-new-symbolic")
            .tooltip_text(tr("New folder"))
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
            &gtk::Label::builder().label(tr("No sections yet.\nUse \\section{…}")).justify(gtk::Justification::Center).css_classes(["dim-label"]).margin_top(24).build(),
        ));
        let outline_scroller = gtk::ScrolledWindow::builder().child(&outline_list).hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).build();

        let sidebar_stack = gtk::Stack::builder().transition_type(gtk::StackTransitionType::Crossfade).vexpand(true).build();
        sidebar_stack.add_named(&files_pane, Some("files"));
        sidebar_stack.add_named(&outline_scroller, Some("outline"));

        // --- Search in the project ---
        let search_entry = gtk::SearchEntry::builder().placeholder_text(tr("Search in project…")).hexpand(true).build();
        let case_btn = gtk::ToggleButton::builder().label("Aa").tooltip_text(tr("Match case")).css_classes(["flat"]).build();
        let word_btn = gtk::ToggleButton::builder().label("W").tooltip_text(tr("Whole word")).css_classes(["flat"]).build();
        let search_row = gtk::Box::builder().spacing(2).build();
        search_row.append(&search_entry);
        search_row.append(&case_btn);
        search_row.append(&word_btn);
        let replace_entry = gtk::Entry::builder().placeholder_text(tr("Replace with…")).hexpand(true).build();
        let replace_btn = gtk::Button::builder().icon_name("edit-find-replace-symbolic").tooltip_text(tr("Replace all (a version is saved first)")).css_classes(["flat"]).sensitive(false).build();
        let replace_row = gtk::Box::builder().spacing(2).build();
        replace_row.append(&replace_entry);
        replace_row.append(&replace_btn);
        let search_summary = gtk::Label::builder().xalign(0.0).css_classes(["dim-label", "caption"]).build();
        let search_results = gtk::ListBox::builder().css_classes(["navigation-sidebar", "search-results"]).build();
        let search_controls = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).margin_start(10).margin_end(10).margin_top(4).margin_bottom(4).build();
        search_controls.append(&search_row);
        search_controls.append(&replace_row);
        search_controls.append(&search_summary);
        let search_pane = gtk::Box::new(gtk::Orientation::Vertical, 0);
        search_pane.append(&search_controls);
        search_pane.append(&gtk::ScrolledWindow::builder().child(&search_results).hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).build());
        sidebar_stack.add_named(&search_pane, Some("search"));
        sections.bind_property("active-name", &sidebar_stack, "visible-child-name").sync_create().build();

        let sidebar = gtk::Box::builder().orientation(gtk::Orientation::Vertical).css_classes(["rutex-sidebar"]).build();
        sidebar.append(&sidebar_header);
        sidebar.append(&sidebar_stack);

        // --- Panel de problemas ---
        let issues_list = gtk::ListBox::builder().css_classes(["issues"]).build();
        let output = gtk::TextView::builder().editable(false).monospace(true).wrap_mode(gtk::WrapMode::WordChar).left_margin(10).top_margin(6).build();
        let issues_stack = gtk::Stack::new();
        issues_stack.add_titled(&gtk::ScrolledWindow::builder().child(&issues_list).build(), Some("issues"), tr("Problems"));
        issues_stack.add_titled(&gtk::ScrolledWindow::builder().child(&output).build(), Some("output"), tr("latexmk output"));
        issues_stack.set_vexpand(true);
        let switcher = gtk::StackSwitcher::builder().stack(&issues_stack).halign(gtk::Align::Start).build();
        let close_issues = gtk::Button::builder().icon_name("window-close-symbolic").css_classes(["flat", "circular"]).halign(gtk::Align::End).hexpand(true).action_name("win.toggle-issues").build();
        let panel_header = gtk::Box::builder().css_classes(["panel-header"]).build();
        panel_header.append(&switcher);
        panel_header.append(&close_issues);
        let panel = gtk::Box::builder().orientation(gtk::Orientation::Vertical).height_request(200).css_classes(["issues-panel"]).build();
        panel.append(&panel_header);
        panel.append(&issues_stack);
        // vexpand(false) explícito: si no, el revealer hereda el vexpand de su
        // contenido incluso oculto y le roba medio alto al editor.
        let issues_revealer = gtk::Revealer::builder().child(&panel).transition_type(gtk::RevealerTransitionType::SlideUp).vexpand(false).build();

        // --- Barra de estado ---
        let engine_label = gtk::Label::new(None);
        let words_label = gtk::Label::new(None);
        let words_popover = gtk::Popover::builder().css_classes(["words-popover"]).build();
        let words_btn = gtk::MenuButton::builder().child(&words_label).popover(&words_popover).css_classes(["flat", "words-button"]).tooltip_text(tr("Words per chapter and goal")).build();
        let spell_label = gtk::Label::new(None);
        let spell_box = gtk::Box::builder().spacing(5).build();
        spell_box.append(&gtk::Image::from_icon_name("tools-check-spelling-symbolic"));
        spell_box.append(&spell_label);
        let spell_menu = gio::Menu::new();
        spell_menu.append(Some(tr("Check spelling")), Some("app.spellcheck"));
        let langs = gio::Menu::new();
        for (code, name) in [("auto", tr("Same as the document (babel)")), ("en_US", "English (US)"), ("en_GB", "English (UK)"), ("es_ES", "Español")] {
            let item = gio::MenuItem::new(Some(name), None);
            item.set_action_and_target_value(Some("app.spell-lang"), Some(&code.to_variant()));
            langs.append_item(&item);
        }
        spell_menu.append_section(Some(tr("Spelling language")), &langs);
        let spell_btn = gtk::MenuButton::builder().child(&spell_box).menu_model(&spell_menu).css_classes(["flat", "words-button"]).tooltip_text(tr("Spelling: click to change the language or turn it off")).build();
        let statusbar = gtk::Box::builder().spacing(16).css_classes(["statusbar"]).build();
        statusbar.append(&editor.position_label);
        statusbar.append(&editor.vim_label);
        statusbar.append(&gtk::Box::builder().hexpand(true).build());
        statusbar.append(&spell_btn);
        statusbar.append(&words_btn);
        statusbar.append(&engine_label);

        let editor_col = gtk::Box::builder().orientation(gtk::Orientation::Vertical).css_classes(["editor-column"]).build();
        let missing_banner = adw::Banner::builder().revealed(false).build();
        editor_col.append(&missing_banner);
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
        // The ratio is over the whole width (max_position excludes the PDF's minimum).
        let total = |paned: &gtk::Paned| {
            let pdf_min = paned.end_child().map_or(0, |c| c.measure(gtk::Orientation::Horizontal, -1).0);
            (paned.max_position() + pdf_min) as f64
        };
        main.connect_max_position_notify(move |paned| {
            if paned.max_position() > 300 {
                a.set(true);
                paned.set_position((total(paned) * r.get()) as i32);
                a.set(false);
            }
        });
        main.connect_position_notify(move |paned| {
            if !adjusting.get() && paned.max_position() > 300 {
                ratio.set((paned.position() as f64 / total(paned)).clamp(0.2, 0.8));
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

        let math = crate::mathpreview::MathPreview::new(project.root.clone(), &project.build_dir);
        editor.set_math_preview(math.clone());
        editor.set_main(&project.main);
        let history = crate::history::git_available().then(|| Arc::new(History::for_project(&project.root, &project.build_dir)));
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
            sync_btn,
            engine_label,
            words_label,
            words_popover,
            word_count: RefCell::default(),
            spell_label,
            title_label,
            issues_revealer,
            issues_list,
            issues_stack,
            output,
            issues: RefCell::default(),
            badboxes: RefCell::default(),
            missing_banner,
            install_command: RefCell::default(),
            compile: RefCell::default(),
            autosave: RefCell::default(),
            pdf_mtime: Cell::new(None),
            data,
            index_generation: Cell::new(0),
            outline_list,
            search: SearchUi { entry: search_entry, case: case_btn, word: word_btn, replace: replace_entry, replace_btn, summary: search_summary, results: search_results },
            search_hits: RefCell::default(),
            search_generation: Cell::new(0),
            sections,
            split,
            toolbar,
            editor_col,
            pdf_full: Cell::new(None),
            history,
            math,
            baseline: RefCell::default(),
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
        this.refresh_baseline();
        this.refresh_sync_status();
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
        self.missing_banner.connect_button_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                let cmd = this.install_command.borrow().clone();
                this.window.clipboard().set_text(&cmd);
                this.toast(&trf("Copied: {} — paste it in a terminal, then compile again", &[&cmd]));
            }
        });

        let weak = Rc::downgrade(self);
        self.editor.connect_message(move |msg| {
            if let Some(this) = weak.upgrade() {
                this.toast(msg);
            }
        });

        let weak = Rc::downgrade(self);
        self.editor.connect_loaded(move |rel| {
            if let Some(this) = weak.upgrade() {
                this.load_baseline_for(rel);
            }
        });

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
                let name = this.project.borrow().name();
                match this.editor.current() {
                    Some(rel) => this.title_label.set_label(&format!("{name}  ·  {}", rel.display())),
                    None => this.title_label.set_label(&name),
                }
                if let Some(rel) = this.editor.current() {
                    this.tree.select(&rel);
                }
            }
        });

        self.connect_search();

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
        self.pdf.connect_inverse(move |pos, fallback| {
            let Some(this) = weak.upgrade() else { return };
            let p = this.project.borrow().clone();
            let weak = Rc::downgrade(&this);
            glib::spawn_future_local(async move {
                // SyncTeX apunta a archivos generados (main.toc, main.bbl…) cuando
                // se pulsa el índice o la bibliografía: esos no son del proyecto.
                let lookup = |pos| {
                    let p = p.clone();
                    async move {
                        let (input, line) = synctex::edit(p.root.clone(), p.build_dir.clone(), p.pdf_path(), pos).await?;
                        p.relativize(&input).filter(|rel| p.root.join(rel).is_file()).map(|rel| (rel, line))
                    }
                };
                let mut found = lookup(pos).await;
                if found.is_none() && let Some(target) = fallback {
                    found = lookup(target).await;
                }
                let Some(this) = weak.upgrade() else { return };
                match found {
                    Some((rel, line)) => this.goto(&rel, line),
                    None => this.toast(tr("That part is generated by LaTeX (table of contents, bibliography…) and has no source line")),
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
            this.refresh_sync_status();
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
                // A last version when closing (only if something changed; quick).
                if let Some(h) = &this.history {
                    let _ = h.snapshot(None);
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
        add("history", Box::new(|t| t.show_history()));
        add("check", Box::new(|t| t.show_check()));
        add("insert-image", Box::new(|t| t.pick_and_insert(true)));
        add("insert-csv", Box::new(|t| t.pick_and_insert(false)));
        add("add-reference", Box::new(|t| t.add_reference_dialog()));
        add("symbols", Box::new(|t| {
            if t.editor.current_view().is_none() {
                t.toast(tr("Open a .tex file first"));
                return;
            }
            let weak = Rc::downgrade(t);
            crate::symbols::show(&t.window, move |cmd| {
                if let Some(this) = weak.upgrade() {
                    this.editor.insert_symbol(cmd);
                }
            });
        }));
        add("zotero-cite", Box::new(|t| t.zotero_cite()));
        add("save-version", Box::new(|t| t.save_version()));
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
        add("export-docx", Box::new(|t| t.export_docx()));
        add("sync", Box::new(|t| t.sync()));
        add("fold", Box::new(|t| t.editor.folds().iter().for_each(|f| f.fold_at_cursor())));
        add("unfold", Box::new(|t| t.editor.folds().iter().for_each(|f| f.unfold_at_cursor())));
        add("fold-all", Box::new(|t| t.editor.folds().iter().for_each(|f| f.fold_all())));
        add("unfold-all", Box::new(|t| t.editor.folds().iter().for_each(|f| f.unfold_all())));
        add("open-pdf", Box::new(|t| t.open_pdf_external()));
        add("clean", Box::new(|t| t.clean()));
        add("show-files", Box::new(|t| t.show_sidebar("files")));
        add("show-outline", Box::new(|t| t.show_sidebar("outline")));
        add("find-in-project", Box::new(|t| t.find_in_project()));
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
            t.toast(tr("Path copied"));
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

    /// Dictionary from the preferences or the document's babel language.
    fn update_spell_language(&self) {
        let cfg = config::get();
        let p = self.project.borrow();
        let main_src = fs::read_to_string(p.root.join(&p.main)).unwrap_or_default();
        self.math.set_main(&main_src);
        let wanted = crate::spell::resolve(&cfg.spell_lang, &main_src);
        let used = self.editor.set_spell_language(&wanted);
        match (cfg.spellcheck, used) {
            (false, _) => self.spell_label.set_label(tr("Spelling off")),
            (true, Some(code)) => self.spell_label.set_label(&crate::spell::display_name(&code)),
            (true, None) => self.spell_label.set_label(&trf("No dictionary for {} (install hunspell-{})", &[&wanted, &wanted.split('_').next().unwrap_or("")])),
        }
    }

    /// Recalcula esquema, etiquetas y bibliografía en un hilo aparte.
    fn refresh_index(self: &Rc<Self>) {
        self.update_spell_language();
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

    fn find_in_project(self: &Rc<Self>) {
        self.show_sidebar("search");
        if let Some(view) = self.editor.current_view() {
            let buffer = view.buffer();
            if let Some((s, e)) = buffer.selection_bounds()
                && s.line() == e.line()
            {
                self.search.entry.set_text(&buffer.text(&s, &e, true));
            }
        }
        self.search.entry.grab_focus();
    }

    fn connect_search(self: &Rc<Self>) {
        let rerun = |this: &Rc<Self>| {
            let weak = Rc::downgrade(this);
            move || {
                if let Some(this) = weak.upgrade() {
                    this.run_search();
                }
            }
        };
        let f = rerun(self);
        self.search.entry.connect_search_changed(move |_| f());
        let f = rerun(self);
        self.search.case.connect_toggled(move |_| f());
        let f = rerun(self);
        self.search.word.connect_toggled(move |_| f());

        let weak = Rc::downgrade(self);
        self.search.results.connect_row_activated(move |_, row| {
            let Some(this) = weak.upgrade() else { return };
            let hit = this.search_hits.borrow().get(row.index() as usize).cloned().flatten();
            if let Some(hit) = hit {
                this.goto(&hit.file, hit.line);
                if let Some(view) = this.editor.current_view() {
                    let buffer = view.buffer();
                    if let Some(start) = buffer.iter_at_line(hit.line as i32 - 1) {
                        // Select the match (the shown text may start later in the line).
                        let line_text = {
                            let mut end = start;
                            end.forward_to_line_end();
                            buffer.text(&start, &end, true).to_string()
                        };
                        let shown: String = hit.text.chars().take(hit.col + hit.len).collect();
                        let offset = line_text.find(&shown).map(|b| line_text[..b].chars().count()).unwrap_or(0) + hit.col;
                        let a = buffer.iter_at_offset(start.offset() + offset as i32);
                        let b = buffer.iter_at_offset(start.offset() + (offset + hit.len) as i32);
                        buffer.select_range(&a, &b);
                    }
                }
            }
        });

        let weak = Rc::downgrade(self);
        self.search.replace_btn.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                this.replace_in_project();
            }
        });
    }

    fn search_options(&self) -> crate::search::Options {
        crate::search::Options { case_sensitive: self.search.case.is_active(), whole_word: self.search.word.is_active() }
    }

    fn run_search(self: &Rc<Self>) {
        let query = self.search.entry.text().to_string();
        let generation = self.search_generation.get() + 1;
        self.search_generation.set(generation);
        self.search.replace_btn.set_sensitive(!query.is_empty());
        if query.is_empty() {
            self.fill_search(&[]);
            self.search.summary.set_label("");
            return;
        }
        self.save();
        let p = self.project.borrow().clone();
        let opts = self.search_options();
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let hits = gio::spawn_blocking(move || crate::search::search(&p.root, &p.files, &query, opts)).await.unwrap_or_default();
            let Some(this) = weak.upgrade() else { return };
            if this.search_generation.get() == generation {
                this.fill_search(&hits);
            }
        });
    }

    fn fill_search(&self, hits: &[crate::search::Hit]) {
        while let Some(c) = self.search.results.first_child() {
            self.search.results.remove(&c);
        }
        let accent = crate::theme::palette().map(|p| p.accent).unwrap_or_else(|| "#3584e4".into());
        let mut rows: Vec<Option<crate::search::Hit>> = Vec::new();
        let mut last_file: Option<&Path> = None;
        let files = hits.iter().map(|h| &h.file).collect::<std::collections::HashSet<_>>().len();
        for hit in hits {
            if last_file != Some(hit.file.as_path()) {
                last_file = Some(hit.file.as_path());
                let header = gtk::Box::builder().spacing(6).margin_top(6).build();
                header.append(&gtk::Image::from_icon_name(crate::editor::file_icon(&hit.file)));
                header.append(&gtk::Label::builder().label(hit.file.display().to_string()).xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::Start).css_classes(["heading"]).build());
                let row = gtk::ListBoxRow::builder().child(&header).activatable(false).selectable(false).build();
                self.search.results.append(&row);
                rows.push(None);
            }
            let chars: Vec<char> = hit.text.chars().collect();
            let esc = |s: &[char]| glib::markup_escape_text(&s.iter().collect::<String>()).to_string();
            let end = (hit.col + hit.len).min(chars.len());
            let lead = chars[..hit.col].iter().take_while(|c| c.is_whitespace()).count();
            let markup = format!(
                "<span alpha=\"60%\">{}</span>  {}<span weight=\"bold\" foreground=\"{accent}\">{}</span>{}",
                hit.line,
                esc(&chars[lead..hit.col]),
                esc(&chars[hit.col..end]),
                esc(&chars[end..]),
            );
            let label = gtk::Label::builder().label(markup).use_markup(true).xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).build();
            self.search.results.append(&label);
            rows.push(Some(hit.clone()));
        }
        *self.search_hits.borrow_mut() = rows;
        self.search.summary.set_label(&if hits.is_empty() {
            if self.search.entry.text().is_empty() { String::new() } else { tr("No results").to_string() }
        } else {
            trf("{} results in {} files", &[&hits.len(), &files])
        });
    }

    fn replace_in_project(self: &Rc<Self>) {
        let query = self.search.entry.text().to_string();
        if query.is_empty() {
            return;
        }
        let replacement = self.search.replace.text().to_string();
        self.save();
        let p = self.project.borrow().clone();
        let opts = self.search_options();
        let history = self.history.clone();
        let label = trf("Before replacing “{}”", &[&query]);
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = gio::spawn_blocking(move || {
                if let Some(h) = &history {
                    h.snapshot(Some(&label))?;
                }
                crate::search::replace_all(&p.root, &p.files, &query, &replacement, opts)
            })
            .await;
            let Some(this) = weak.upgrade() else { return };
            match result {
                Ok(Ok((n, files))) => {
                    this.editor.reload_external();
                    this.toast(&trf("Replaced {} occurrences in {} files", &[&n, &files]));
                    this.run_search();
                    this.compile();
                }
                Ok(Err(e)) => this.toast(&trf("Could not replace: {}", &[&e])),
                Err(_) => {}
            }
        });
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
        dialog.add_responses(&[("cancel", tr("Cancel")), ("ok", ok_label)]);
        dialog.set_response_appearance("ok", adw::ResponseAppearance::Suggested);
        let weak = Rc::downgrade(self);
        let input = entry.clone();
        dialog.connect_response(Some("ok"), move |_, _| {
            let Some(this) = weak.upgrade() else { return };
            let text = input.text().trim().trim_matches('/').to_string();
            if text.is_empty() || text.split('/').any(|c| c == ".." || c.is_empty()) {
                this.toast(tr("Invalid name"));
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

    /// Shows the Sync button when the project is in a Git repository with a
    /// remote, with what is pending in its tooltip (local only, no network).
    fn refresh_sync_status(self: &Rc<Self>) {
        let root = self.project.borrow().root.clone();
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let st = gio::spawn_blocking(move || crate::sync::status(&root)).await.ok().flatten();
            let Some(this) = weak.upgrade() else { return };
            let Some(st) = st.filter(|s| s.has_remote) else {
                this.sync_btn.set_visible(false);
                return;
            };
            let mut parts = vec![tr("Sync with Git").to_string()];
            if !st.conflicts.is_empty() {
                parts.push(trf("{} files with conflicts to resolve", &[&st.conflicts.len()]));
            }
            if st.changed > 0 {
                parts.push(trf("{} changed files to send", &[&st.changed]));
            }
            if st.behind > 0 {
                parts.push(trf("{} updates to receive", &[&st.behind]));
            }
            this.sync_btn.set_tooltip_text(Some(&parts.join("\n")));
            this.sync_btn.set_visible(true);
        });
    }

    /// Commit, receive the others' changes and send yours (see sync.rs).
    fn sync(self: &Rc<Self>) {
        self.save();
        let root = self.project.borrow().root.clone();
        if crate::sync::status(&root).is_none() {
            let dialog = adw::AlertDialog::builder()
                .heading(tr("This project is not in Git"))
                .body(tr("To work with others, put the project folder in a Git repository with a remote (for example, create a repository on GitHub and clone it), and open that folder in rutex. Then this button sends and receives changes."))
                .close_response("close")
                .build();
            dialog.add_response("close", tr("Close"));
            dialog.present(Some(&self.window));
            return;
        }
        self.sync_btn.set_sensitive(false);
        self.toast(tr("Syncing…"));
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let message = crate::sync::message();
            let r = root.clone();
            let result = gio::spawn_blocking(move || crate::sync::sync(&r, &message)).await.unwrap_or_else(|_| Err("thread".into()));
            let Some(this) = weak.upgrade() else { return };
            this.sync_btn.set_sensitive(true);
            // Files may have changed on disk: show them and rebuild.
            this.refresh_files();
            if this.editor.reload_external() {
                this.compile();
            }
            this.refresh_sync_status();
            match result {
                Ok(crate::sync::Outcome::Synced { received: 0, sent: 0 }) => this.toast(tr("Already up to date")),
                Ok(crate::sync::Outcome::Synced { received, sent }) => this.toast(&trf("Synced: {} received, {} sent", &[&received, &sent])),
                Ok(crate::sync::Outcome::Conflicts(files)) => {
                    let names: Vec<String> = files.iter().map(|f| f.strip_prefix(&root).unwrap_or(f).display().to_string()).collect();
                    let dialog = adw::AlertDialog::builder()
                        .heading(tr("Someone else changed the same lines"))
                        .body(trf("Both versions are now in {}, between the lines <<<<<<<, ======= and >>>>>>>. Keep the text you want, delete those marker lines, and press Sync again.", &[&names.join(", ")]))
                        .close_response("close")
                        .build();
                    dialog.add_responses(&[("close", tr("Close")), ("open", tr("Show me"))]);
                    dialog.set_response_appearance("open", adw::ResponseAppearance::Suggested);
                    let (weak, first) = (Rc::downgrade(&this), files.first().cloned());
                    dialog.connect_response(Some("open"), move |_, _| {
                        let (Some(this), Some(file)) = (weak.upgrade(), first.clone()) else { return };
                        let Ok(rel) = file.strip_prefix(&this.project.borrow().root.clone()).map(Path::to_path_buf) else { return };
                        let line = fs::read_to_string(&file).ok().and_then(|t| t.lines().position(|l| l.starts_with("<<<<<<< "))).unwrap_or(0);
                        this.goto(&rel, line as u32 + 1);
                    });
                    dialog.present(Some(&this.window));
                }
                Err(e) => {
                    let dialog = adw::AlertDialog::builder().heading(tr("Could not sync")).body(e).close_response("close").build();
                    dialog.add_response("close", tr("Close"));
                    dialog.present(Some(&this.window));
                }
            }
        });
    }

    /// Word copy of the document for people who review in Word (see export.rs).
    fn export_docx(self: &Rc<Self>) {
        let Some(pandoc) = crate::export::find_pandoc() else {
            let hint = crate::export::install_hint();
            let dialog = adw::AlertDialog::builder()
                .heading(tr("Exporting to Word needs pandoc"))
                .body(trf("pandoc is a free document converter. Install it with:\n\n{}\n\nand try again.", &[&hint]))
                .close_response("close")
                .build();
            dialog.add_responses(&[("close", tr("Close")), ("copy", tr("Copy command"))]);
            let window = self.window.clone();
            dialog.connect_response(Some("copy"), move |_, _| window.clipboard().set_text(hint));
            dialog.present(Some(&self.window));
            return;
        };
        self.save();
        let p = self.project.borrow().clone();
        let stem = p.main.file_stem().unwrap_or_default().to_string_lossy().into_owned();
        let chooser = gtk::FileDialog::builder()
            .title(tr("Export to Word"))
            .initial_name(format!("{stem}.docx"))
            .initial_folder(&gio::File::for_path(&p.root))
            .modal(true)
            .build();
        let weak = Rc::downgrade(self);
        chooser.save(Some(&self.window), gio::Cancellable::NONE, move |res| {
            let (Some(this), Ok(file)) = (weak.upgrade(), res) else { return };
            let Some(out) = file.path() else { return };
            this.toast(tr("Exporting to Word…"));
            let weak = Rc::downgrade(&this);
            glib::spawn_future_local(async move {
                let target = out.clone();
                let result = gio::spawn_blocking(move || {
                    crate::export::export(&crate::export::Export { pandoc: &pandoc, root: &p.root, main: &p.main, build_dir: &p.build_dir, out: &target })
                })
                .await
                .unwrap_or_else(|_| Err("thread".into()));
                let Some(this) = weak.upgrade() else { return };
                let open = {
                    let (window, out) = (this.window.clone(), out.clone());
                    move || gtk::FileLauncher::new(Some(&gio::File::for_path(&out))).launch(Some(&window), gio::Cancellable::NONE, |_| {})
                };
                let name = out.file_name().unwrap_or_default().to_string_lossy().into_owned();
                match result {
                    Ok(notes) if notes.is_empty() => {
                        let toast = adw::Toast::builder().title(glib::markup_escape_text(&trf("Exported {}", &[&name]))).button_label(tr("Open")).timeout(6).build();
                        toast.connect_button_clicked(move |_| open());
                        this.toasts.add_toast(toast);
                    }
                    Ok(notes) => {
                        let body = format!("{}\n\n• {}", tr("Word cannot show everything LaTeX does. Worth a look:"), notes.join("\n• "));
                        let dialog = adw::AlertDialog::builder().heading(trf("Exported {}", &[&name])).body(body).close_response("close").build();
                        dialog.add_responses(&[("close", tr("Close")), ("open", tr("Open"))]);
                        dialog.set_response_appearance("open", adw::ResponseAppearance::Suggested);
                        dialog.connect_response(Some("open"), move |_, _| open());
                        dialog.present(Some(&this.window));
                    }
                    Err(e) => {
                        let dialog = adw::AlertDialog::builder().heading(tr("Could not export to Word")).body(e).close_response("close").build();
                        dialog.add_response("close", tr("Close"));
                        dialog.present(Some(&this.window));
                    }
                }
            });
        });
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
                self.set_status(Status::Busy, tr("Compiling…"));
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
                    format!("{}\n\nsudo pacman -S --needed texlive-basic texlive-latex texlive-latexrecommended \\\n  texlive-latexextra texlive-fontsrecommended texlive-bibtexextra texlive-mathscience \\\n  texlive-binextra texlive-xetex texlive-luatex biber\n\n{}", tr("latexmk was not found. Install TeX Live, for example on Arch:"), tr("On Debian/Ubuntu: texlive-full or texlive-latex-extra + latexmk. On Fedora: texlive-scheme-medium + latexmk."))
                } else {
                    trf("Could not run latexmk: {}", &[&e])
                };
                self.set_status(Status::Error, tr("No TeX"));
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
        let (badboxes, shown): (Vec<ShownIssue>, Vec<ShownIssue>) = shown.into_iter().partition(|i| i.severity == Severity::Badbox);
        *self.badboxes.borrow_mut() = badboxes;
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
            if errors == 0 {
                self.auto_snapshot();
            }
        }
        self.suggest_packages(&shown);
        *self.issues.borrow_mut() = shown;
        self.output.buffer().set_text(&outcome.output);

        let secs = crate::i18n::decimal(outcome.elapsed.as_secs_f64());
        match (errors, warnings) {
            (0, _) if !outcome.ok && outcome.pdf.is_none() => self.set_status(Status::Error, tr("Build failed")),
            (0, 0) => self.set_status(Status::Ok, &format!("{secs} s")),
            (0, w) => self.set_status(Status::Warning, &if w == 1 { tr("1 warning").to_string() } else { trf("{} warnings", &[&w]) }),
            (1, _) => self.set_status(Status::Error, tr(tr("1 error"))),
            (e, _) => self.set_status(Status::Error, &trf("{} errors", &[&e])),
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
            if let (Some(this), Some(wc)) = (weak.upgrade(), words) {
                *this.word_count.borrow_mut() = Some(wc);
                this.show_word_count();
            }
        });
    }

    fn word_goal(&self) -> u64 {
        config::get().goal_for(&self.project.borrow().root)
    }

    /// Status bar label ("877 / 15,000 words") and the per-chapter popover.
    fn show_word_count(self: &Rc<Self>) {
        let Some(wc) = self.word_count.borrow().clone() else { return };
        let goal = self.word_goal();
        self.words_label.set_label(&if goal > 0 {
            trf("{} / {} words", &[&thousands(wc.total), &thousands(goal)])
        } else {
            trf("{} words", &[&thousands(wc.total)])
        });

        let body = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(8).margin_start(6).margin_end(6).margin_top(6).margin_bottom(6).width_request(320).build();
        body.append(&gtk::Label::builder().label(tr("Words")).xalign(0.0).css_classes(["heading"]).build());
        if goal > 0 {
            let fraction = (wc.total as f64 / goal as f64).min(1.0);
            body.append(&gtk::ProgressBar::builder().fraction(fraction).build());
            body.append(&gtk::Label::builder().label(trf("{} of {} ({}%)", &[&thousands(wc.total), &thousands(goal), &((fraction * 100.0).round() as u32)])).xalign(0.0).css_classes(["dim-label", "caption"]).build());
        }
        let max = wc.files.iter().map(|f| f.1).max().unwrap_or(1).max(1);
        let grid = gtk::Grid::builder().column_spacing(10).row_spacing(4).margin_top(4).build();
        for (i, (file, n)) in wc.files.iter().enumerate() {
            let name = Path::new(file).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            grid.attach(&gtk::Label::builder().label(&name).xalign(0.0).tooltip_text(file).ellipsize(gtk::pango::EllipsizeMode::End).max_width_chars(22).build(), 0, i as i32, 1, 1);
            grid.attach(&gtk::LevelBar::builder().value(*n as f64 / max as f64).hexpand(true).valign(gtk::Align::Center).css_classes(["words-bar"]).build(), 1, i as i32, 1, 1);
            grid.attach(&gtk::Label::builder().label(thousands(*n)).xalign(1.0).css_classes(["numeric"]).build(), 2, i as i32, 1, 1);
        }
        body.append(&grid);

        let goal_spin = gtk::SpinButton::with_range(0.0, 500_000.0, 500.0);
        goal_spin.set_value(goal as f64);
        goal_spin.set_tooltip_text(Some(tr("0 = no goal")));
        let goal_row = gtk::Box::builder().spacing(8).margin_top(6).build();
        goal_row.append(&gtk::Label::builder().label(tr("Goal")).xalign(0.0).hexpand(true).build());
        goal_row.append(&goal_spin);
        body.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        body.append(&goal_row);
        let weak = Rc::downgrade(self);
        goal_spin.connect_value_changed(move |s| {
            let Some(this) = weak.upgrade() else { return };
            let root = this.project.borrow().root.clone();
            config::update(|c| c.set_goal(root, s.value() as u64));
            let wc = this.word_count.borrow().clone();
            if let Some(wc) = wc {
                let goal = this.word_goal();
                this.words_label.set_label(&if goal > 0 {
                    trf("{} / {} words", &[&thousands(wc.total), &thousands(goal)])
                } else {
                    trf("{} words", &[&thousands(wc.total)])
                });
            }
        });
        self.words_popover.set_child(Some(&body));
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
                Severity::Warning | Severity::Badbox => ("dialog-warning-symbolic", "warning"),
            };
            let row = gtk::Box::builder().spacing(8).margin_start(6).margin_end(6).build();
            row.append(&gtk::Image::builder().icon_name(icon).css_classes([class]).build());
            let location = match (&issue.file, issue.line) {
                (Some(f), Some(l)) => format!("{}:{l}", f.display()),
                (Some(f), None) => f.display().to_string(),
                (None, Some(l)) => trf("line {}", &[&l]),
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
            row.append(&self.issue_help_button(issue));
            self.issues_list.append(&row);
        }
        self.issues_stack.set_visible_child_name("issues");
    }

    /// "?" next to a problem: what it means, how to fix it, and (if turned on
    /// in Preferences) Claude's explanation of this particular case.
    fn issue_help_button(&self, issue: &ShownIssue) -> gtk::MenuButton {
        let content = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(10).width_request(380).css_classes(["issue-help"]).build();
        let label = |text: &str, classes: &[&str]| gtk::Label::builder().label(text).xalign(0.0).wrap(true).max_width_chars(56).selectable(true).css_classes(classes.to_vec()).build();
        content.append(&label(&issue.message, &["heading"]));
        match crate::explain::explain(&issue.message) {
            Some((what, fix)) => {
                content.append(&label(what, &[]));
                content.append(&label(&trf("How to fix: {}", &[&fix]), &["bubble-fix"]));
            }
            None => content.append(&label(tr("rutex has no built-in explanation for this message."), &["dim-label"])),
        }
        let cfg = config::get();
        if !cfg.ai_help {
            content.append(&label(tr("Tip: you can turn on explanations by Claude in Preferences → AI help."), &["dim-label", "caption"]));
        } else if crate::ai::api_key().is_none() {
            content.append(&label(tr("Add your Anthropic API key in Preferences → AI help to ask Claude."), &["dim-label", "caption"]));
        } else {
            let ask = gtk::Button::builder().label(tr("Ask Claude")).halign(gtk::Align::Start).css_classes(["suggested-action", "pill"]).build();
            let answer = label("", &["ai-answer"]);
            answer.set_visible(false);
            content.append(&ask);
            content.append(&answer);
            let p = self.project.borrow().clone();
            let (message, file, line) = (issue.message.clone(), issue.file.clone(), issue.line.unwrap_or(1));
            ask.connect_clicked(move |btn| {
                let Some(key) = crate::ai::api_key() else { return };
                btn.set_sensitive(false);
                btn.set_label(tr("Asking Claude…"));
                let (btn, answer, p, message, file) = (btn.clone(), answer.clone(), p.clone(), message.clone(), file.clone());
                glib::spawn_future_local(async move {
                    let spanish = crate::i18n::is_spanish();
                    let result = gio::spawn_blocking(move || {
                        let name = file.as_ref().map(|f| f.display().to_string()).unwrap_or_default();
                        let src = file.as_ref().and_then(|f| fs::read_to_string(p.root.join(f)).ok()).unwrap_or_default();
                        let excerpt = crate::ai::excerpt(&src, line, 12);
                        crate::ai::explain(&key, &message, &name, line, &excerpt, spanish, &p.build_dir)
                    })
                    .await
                    .unwrap_or_else(|_| Err("thread".into()));
                    answer.set_visible(true);
                    match result {
                        Ok(text) => {
                            answer.set_label(&text);
                            btn.set_visible(false);
                        }
                        Err(e) => {
                            answer.set_label(&e);
                            btn.set_sensitive(true);
                            btn.set_label(tr("Ask Claude"));
                        }
                    }
                });
            });
        }
        let scroller = gtk::ScrolledWindow::builder().child(&content).propagate_natural_height(true).max_content_height(460).hscrollbar_policy(gtk::PolicyType::Never).build();
        gtk::MenuButton::builder()
            .icon_name("dialog-question-symbolic")
            .tooltip_text(tr("What does this mean?"))
            .popover(&gtk::Popover::builder().child(&scroller).build())
            .css_classes(["flat", "circular"])
            .valign(gtk::Align::Center)
            .build()
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
                None => this.toast(tr("SyncTeX could not find that line in the PDF")),
            }
        });
    }

    fn add_reference_dialog(self: &Rc<Self>) {
        let entry = gtk::Entry::builder().placeholder_text("10.1145/361604.361612 · arXiv:1706.03762").activates_default(true).build();
        let dialog = adw::AlertDialog::builder()
            .heading(tr("Add a reference"))
            .body(tr("Paste a DOI, an arXiv id or a link to either. The entry is added to your .bib and cited at the cursor."))
            .extra_child(&entry)
            .default_response("add")
            .close_response("cancel")
            .build();
        dialog.add_responses(&[("cancel", tr("Cancel")), ("add", tr("Add"))]);
        dialog.set_response_appearance("add", adw::ResponseAppearance::Suggested);
        let weak = Rc::downgrade(self);
        let input = entry.clone();
        dialog.connect_response(Some("add"), move |_, _| {
            let Some(this) = weak.upgrade() else { return };
            let Some(ident) = crate::refs::parse_identifier(&input.text()) else {
                this.toast(tr("That does not look like a DOI or an arXiv id"));
                return;
            };
            this.toast(tr("Looking up the reference…"));
            let weak = Rc::downgrade(&this);
            glib::spawn_future_local(async move {
                let bib = gio::spawn_blocking(move || crate::refs::fetch(&ident)).await.unwrap_or_else(|_| Err("thread".into()));
                let Some(this) = weak.upgrade() else { return };
                match bib {
                    Ok(bib) => this.add_bibtex(&bib),
                    Err(e) => this.toast(&trf("Could not get the reference: {}", &[&e])),
                }
            });
        });
        dialog.present(Some(&self.window));
        entry.grab_focus();
    }

    fn zotero_cite(self: &Rc<Self>) {
        self.toast(tr("Pick the references in Zotero…"));
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let bib = gio::spawn_blocking(crate::refs::zotero_cayw).await.unwrap_or_else(|_| Err("thread".into()));
            let Some(this) = weak.upgrade() else { return };
            match bib {
                Ok(bib) if bib.trim().is_empty() => {}
                Ok(bib) => this.add_bibtex(&bib),
                Err(_) => this.toast(tr("Could not reach Zotero: open it and install the Better BibTeX add-on")),
            }
        });
    }

    /// Adds BibTeX entries to the project's .bib and cites them at the cursor.
    fn add_bibtex(self: &Rc<Self>, bibtex: &str) {
        let p = self.project.borrow().clone();
        let main_src = fs::read_to_string(p.root.join(&p.main)).unwrap_or_default();
        let (bib, created) = crate::refs::target_bib(&p.root, &main_src, &p.files);
        match crate::refs::add_to_bib(&p.root, &bib, bibtex) {
            Ok(keys) if !keys.is_empty() => {
                let cite = format!("\\cite{{{}}}", keys.join(", "));
                let inserted = self.editor.insert_at_cursor(&cite);
                self.refresh_files();
                self.refresh_index();
                let mut msg = trf("Added to {}: {}", &[&bib.display(), &keys.join(", ")]);
                if created {
                    msg.push_str(&trf(" — add \\addbibresource{{}} to your preamble", &[&bib.display()]));
                } else if !inserted {
                    msg.push_str(&trf(" — cite it with {}", &[&cite]));
                }
                self.toast(&msg);
            }
            Ok(_) => self.toast(tr("No entries found")),
            Err(e) => self.toast(&trf("Could not write the .bib file: {}", &[&e])),
        }
    }

    /// File chooser for "Insert image…" / "Insert table from CSV…".
    fn pick_and_insert(self: &Rc<Self>, image: bool) {
        if self.editor.current_view().is_none() {
            self.toast(tr("Open a .tex file first"));
            return;
        }
        let filter = gtk::FileFilter::new();
        if image {
            filter.set_name(Some(tr("Images")));
            for ext in crate::insert::IMAGE_EXTS {
                filter.add_suffix(ext);
            }
        } else {
            filter.set_name(Some("CSV"));
            for ext in ["csv", "tsv", "txt"] {
                filter.add_suffix(ext);
            }
        }
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&filter);
        let dialog = gtk::FileDialog::builder().title(if image { tr("Insert image") } else { tr("Insert table from CSV") }).filters(&filters).modal(true).build();
        let weak = Rc::downgrade(self);
        dialog.open(Some(&self.window), gio::Cancellable::NONE, move |res| {
            let (Some(this), Some(path)) = (weak.upgrade(), res.ok().and_then(|f| f.path())) else { return };
            if image {
                if let Some(view) = this.editor.current_view() {
                    this.editor.insert_files(&view, &[path], None);
                }
            } else {
                let ok = fs::read_to_string(&path).map(|csv| this.editor.insert_csv(&csv)).unwrap_or(false);
                if !ok {
                    this.toast(tr("That file does not look like a table (same number of columns in every row)"));
                }
            }
        });
    }

    /// "Check before submitting" window, from the last build and the index.
    fn show_check(self: &Rc<Self>) {
        let groups = {
            let issues = self.issues.borrow();
            let badboxes = self.badboxes.borrow();
            let warnings: Vec<crate::check::Logged> = issues.iter().map(|i| (i.message.as_str(), i.file.clone(), i.line)).collect();
            let boxes: Vec<crate::check::Logged> = badboxes.iter().map(|i| (i.message.as_str(), i.file.clone(), i.line)).collect();
            crate::check::build(&self.data.borrow().index, &warnings, &boxes)
        };
        let total: usize = groups.iter().map(|g| g.items.len()).sum();

        let page = adw::PreferencesPage::new();
        let dialog = adw::Dialog::builder().title(tr("Check before submitting")).content_width(640).content_height(640).build();
        if total == 0 {
            let ok = adw::StatusPage::builder()
                .icon_name("emblem-ok-symbolic")
                .title(tr("Ready to submit"))
                .description(tr("No undefined references, unused labels or citations, margin overflows or to-do notes."))
                .build();
            let group = adw::PreferencesGroup::new();
            group.add(&ok);
            page.add(&group);
        } else {
            let intro = adw::PreferencesGroup::builder().description(tr("Based on the last build. Click an item to go to it.")).build();
            page.add(&intro);
        }
        for g in groups.iter().filter(|g| !g.items.is_empty()) {
            let group = adw::PreferencesGroup::builder().title(format!("{} ({})", tr(g.title), g.items.len())).build();
            for item in g.items.iter().take(200) {
                let location = match (&item.file, item.line) {
                    (Some(f), Some(l)) => format!("{}:{l}", f.display()),
                    (Some(f), None) => f.display().to_string(),
                    _ => String::new(),
                };
                let row = adw::ActionRow::builder()
                    .title(glib::markup_escape_text(&item.text))
                    .subtitle(glib::markup_escape_text(&location))
                    .activatable(item.file.is_some())
                    .build();
                row.add_prefix(&gtk::Image::from_icon_name(g.icon));
                if let Some(file) = item.file.clone() {
                    let line = item.line.unwrap_or(1);
                    let (weak, dlg) = (Rc::downgrade(self), dialog.clone());
                    row.connect_activated(move |_| {
                        dlg.close();
                        if let Some(this) = weak.upgrade() {
                            this.goto(&file, line);
                        }
                    });
                }
                group.add(&row);
            }
            page.add(&group);
        }
        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&adw::HeaderBar::new());
        toolbar.set_content(Some(&page));
        dialog.set_child(Some(&toolbar));
        dialog.present(Some(&self.window));
    }

    /// "File `x.sty' not found": banner with the package that provides it.
    fn suggest_packages(self: &Rc<Self>, issues: &[ShownIssue]) {
        let files = crate::missing::missing_files(issues.iter().filter(|i| i.severity == Severity::Error).map(|i| i.message.as_str()));
        if files.is_empty() || !crate::missing::available() {
            self.missing_banner.set_revealed(false);
            return;
        }
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let found = gio::spawn_blocking(move || files.into_iter().map(|f| {
                let p = crate::missing::lookup(&f);
                (f, p)
            }).collect::<Vec<_>>()).await.unwrap_or_default();
            let Some(this) = weak.upgrade() else { return };
            let packages: Vec<String> = found.iter().filter_map(|(_, p)| p.as_ref()?.arch_package()).collect::<std::collections::BTreeSet<_>>().into_iter().collect();
            let unknown: Vec<&str> = found.iter().filter(|(_, p)| p.is_none()).map(|(f, _)| f.as_str()).collect();
            let files: Vec<&str> = found.iter().filter(|(_, p)| p.is_some()).map(|(f, _)| f.as_str()).collect();
            let has_pacman = gtk::glib::find_program_in_path("pacman").is_some();
            let mut text = String::new();
            if !packages.is_empty() {
                text = if has_pacman {
                    trf("Missing {}: install {}", &[&files.join(", "), &packages.join(" ")])
                } else {
                    let tl: Vec<String> = found.iter().filter_map(|(_, p)| p.as_ref().map(|p| p.package.clone())).collect();
                    trf("Missing {}: it is in the TeX Live package {}", &[&files.join(", "), &tl.join(", ")])
                };
            }
            if !unknown.is_empty() {
                if !text.is_empty() {
                    text.push_str(" · ");
                }
                text.push_str(&trf("{} is not in TeX Live: check the name", &[&unknown.join(", ")]));
            }
            *this.install_command.borrow_mut() = format!("sudo pacman -S --needed {}", packages.join(" "));
            this.missing_banner.set_title(&glib::markup_escape_text(&text));
            this.missing_banner.set_button_label(if has_pacman && !packages.is_empty() { Some(tr("Copy install command")) } else { None });
            this.missing_banner.set_revealed(true);
        });
    }

    /// Automatic version after a good build, at most every 5 minutes.
    fn auto_snapshot(self: &Rc<Self>) {
        let Some(h) = self.history.clone() else { return };
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let _ = gio::spawn_blocking(move || {
                if h.age_of_last().is_none_or(|age| age >= 300) {
                    let _ = h.snapshot(None);
                }
            })
            .await;
            // A new project gets its first baseline from its first version.
            if let Some(this) = weak.upgrade()
                && this.baseline.borrow().is_none()
            {
                this.refresh_baseline();
            }
        });
    }

    /// Picks the baseline (newest named version, or newest) and reloads the bars.
    fn refresh_baseline(self: &Rc<Self>) {
        let Some(h) = self.history.clone() else { return };
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let id = gio::spawn_blocking(move || h.baseline()).await.ok().flatten();
            let Some(this) = weak.upgrade() else { return };
            *this.baseline.borrow_mut() = id;
            for rel in this.editor.open_files() {
                this.load_baseline_for(&rel);
            }
        });
    }

    fn load_baseline_for(self: &Rc<Self>, rel: &Path) {
        let (Some(h), Some(id)) = (self.history.clone(), self.baseline.borrow().clone()) else { return };
        let rel = rel.to_path_buf();
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let r = rel.clone();
            // A file that did not exist in the baseline is all new.
            let text = gio::spawn_blocking(move || h.file_at(&id, &r).unwrap_or_default()).await.ok();
            if let Some(this) = weak.upgrade() {
                this.editor.set_baseline(&rel, text);
            }
        });
    }

    fn show_history(self: &Rc<Self>) {
        let Some(h) = self.history.clone() else {
            self.toast(tr("Install git to use the version history"));
            return;
        };
        self.save();
        let weak = Rc::downgrade(self);
        crate::historyview::show(&self.window, h, move || {
            let Some(this) = weak.upgrade() else { return };
            this.editor.reload_external();
            this.editor.forget_missing();
            this.refresh_files();
            this.compile();
            this.toast(tr("Version restored. Your previous state is saved in the history."));
            this.refresh_baseline();
        });
    }

    fn save_version(self: &Rc<Self>) {
        let Some(h) = self.history.clone() else {
            self.toast(tr("Install git to use the version history"));
            return;
        };
        self.save();
        let weak = Rc::downgrade(self);
        crate::historyview::ask_name(&self.window, move |name| {
            let h = h.clone();
            let weak = weak.clone();
            glib::spawn_future_local(async move {
                let saved = gio::spawn_blocking(move || h.snapshot(Some(&name))).await;
                if let (Some(this), Ok(Ok(_))) = (weak.upgrade(), saved) {
                    this.toast(tr("Version saved"));
                    this.refresh_baseline();
                }
            });
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
        let entry = gtk::Entry::builder().input_purpose(gtk::InputPurpose::Digits).activates_default(true).placeholder_text(tr("Line number")).build();
        let dialog = adw::AlertDialog::builder().heading(tr("Go to line")).extra_child(&entry).default_response("go").close_response("cancel").build();
        dialog.add_responses(&[("cancel", tr("Cancel")), ("go", tr("Go"))]);
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
        self.ask_path(tr("New file"), tr("Path inside the project. Without an extension it will be .tex."), &initial, tr("Create"), |this, mut rel| {
            if rel.extension().is_none() {
                rel.set_extension("tex");
            }
            let abs = this.project.borrow().root.join(&rel);
            if !abs.exists() {
                let created = abs.parent().map_or(Ok(()), fs::create_dir_all).and_then(|_| fs::write(&abs, ""));
                if let Err(e) = created {
                    this.toast(&trf("Could not create it: {}", &[&e]));
                    return;
                }
                if crate::project::has_ext(&rel, "tex") {
                    this.toast(&trf("Include it with \\input{{}}", &[&rel.with_extension("").display()]));
                }
            }
            this.refresh_files();
            this.open_file(&rel);
        });
    }

    fn new_folder_dialog(self: &Rc<Self>, dir: &Path) {
        let initial = if dir.as_os_str().is_empty() { String::new() } else { format!("{}/", dir.display()) };
        self.ask_path(tr("New folder"), tr("Path inside the project"), &initial, tr("Create"), |this, rel| {
            let abs = this.project.borrow().root.join(&rel);
            if let Err(e) = fs::create_dir_all(&abs) {
                this.toast(&trf("Could not create it: {}", &[&e]));
                return;
            }
            this.refresh_files();
            this.tree.select(&rel);
        });
    }

    fn rename_dialog(self: &Rc<Self>, rel: &Path) {
        let from = rel.to_path_buf();
        let body = trf("New name or path for “{}”", &[&rel.display()]);
        self.ask_path(tr("Rename"), &body, &rel.display().to_string(), tr("Rename"), move |this, to| {
            if to == from {
                return;
            }
            let root = this.project.borrow().root.clone();
            if root.join(&to).exists() {
                this.toast(&trf("{} already exists", &[&to.display()]));
                return;
            }
            let moved = to.parent().map_or(Ok(()), |p| fs::create_dir_all(root.join(p))).and_then(|_| fs::rename(root.join(&from), root.join(&to)));
            if let Err(e) = moved {
                this.toast(&trf("Could not rename: {}", &[&e]));
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
            .heading(tr("Move to trash?"))
            .body(trf("“{}” will be moved to the trash. You can restore it from your file manager.", &[&rel.display()]))
            .default_response("cancel")
            .close_response("cancel")
            .build();
        dialog.add_responses(&[("cancel", tr("Cancel")), ("trash", tr("Move to trash"))]);
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
                        this.toast(tr("You removed the main file: pick another with right click → Use as main file"));
                    }
                }
                Err(e) => this.toast(&trf("Could not move it to the trash: {}", &[&e])),
            }
        });
        dialog.present(Some(&self.window));
    }

    fn set_main(self: &Rc<Self>, rel: &Path) {
        let rel = rel.to_path_buf();
        if !crate::project::has_ext(&rel, "tex") {
            self.toast(tr("The main file must be a .tex file"));
            return;
        }
        let root = {
            let mut p = self.project.borrow_mut();
            p.main = rel.clone();
            p.root.clone()
        };
        config::update(|c| c.set_main(root, rel.clone()));
        self.editor.set_main(&rel);
        self.pdf_mtime.set(None);
        self.tree.set_main(rel.clone());
        self.toast(&trf("Main file: {}", &[&rel.display()]));
        self.compile();
    }

    fn export_pdf(&self) {
        let p = self.project.borrow();
        let src = p.pdf_path();
        let dst = p.root.join(src.file_name().unwrap());
        match fs::copy(&src, &dst) {
            Ok(_) => self.toast(&trf("PDF saved to {}", &[&dst.display()])),
            Err(_) => self.toast(tr("There is no PDF yet")),
        }
    }

    fn open_pdf_external(&self) {
        let path = self.project.borrow().pdf_path();
        if !path.exists() {
            self.toast(tr("There is no PDF yet"));
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
        self.toast(tr("Auxiliary files removed"));
        // Si había una compilación en marcha, su final relanzará otra.
        if self.compile.borrow().running {
            self.compile.borrow_mut().pending = true;
        } else {
            self.compile();
        }
    }
}
