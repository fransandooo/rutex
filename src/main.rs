mod changebar;
mod check;
mod compiler;
mod completion;
mod config;
mod demo;
mod editor;
mod filetree;
mod history;
mod historyview;
mod hover;
mod i18n;
mod index;
mod keys;
mod logparse;
mod mathpreview;
mod missing;
mod newproject;
mod pdf;
mod prefs;
mod project;
mod quickopen;
mod search;
mod spell;
mod synctex;
mod tabs;
mod templates;
mod theme;
mod ui;
mod window;

use adw::prelude::*;
use gtk::{gio, glib};

const APP_ID: &str = "dev.fran.Rutex";

const CSS: &str = include_str!("style.css");

fn main() -> glib::ExitCode {
    // RUTEX_DEV=1 lanza una instancia aparte (útil para probar sin cerrar la instalada).
    let app_id = if std::env::var_os("RUTEX_DEV").is_some() { format!("{APP_ID}.Devel") } else { APP_ID.to_string() };
    let app = adw::Application::builder()
        .application_id(app_id)
        .flags(gio::ApplicationFlags::HANDLES_OPEN)
        .build();

    i18n::set_spanish(config::get().spanish);
    app.connect_startup(|app| {
        let provider = gtk::CssProvider::new();
        provider.load_from_string(CSS);
        gtk::style_context_add_provider_for_display(
            &gtk::gdk::Display::default().expect("no display"),
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
        theme::init();
        window::setup_app_actions(app);
        window::set_accels(app);
    });
    app.connect_activate(window::activate);
    app.connect_open(|app, files, _| {
        for path in files.iter().filter_map(|f| f.path()) {
            window::open_project(app, &path, false);
        }
    });
    // Al salir (también por señal): guardar y no dejar ningún proceso de TeX vivo.
    app.connect_shutdown(|_| {
        window::save_all_projects();
        compiler::kill_all();
        pdf::shutdown_workers();
    });
    for signal in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] {
        let weak = app.downgrade();
        glib_unix::unix_signal_add_local(signal, move || {
            if let Some(app) = weak.upgrade() {
                app.quit();
            }
            glib::ControlFlow::Break
        });
    }

    let code = app.run();
    // Por si alguna ventana se cerró sin pasar por `shutdown`.
    pdf::shutdown_workers();
    code
}
