mod compiler;
mod config;
mod editor;
mod logparse;
mod pdf;
mod project;
mod synctex;
mod theme;
mod window;

use adw::prelude::*;
use gtk::{gio, glib};

const APP_ID: &str = "dev.fran.Rutex";

const CSS: &str = "
.vim-bar { padding: 2px 8px; font-size: 10pt; }
.pdf-view { background-color: var(--rutex-pdf-bg, shade(@window_bg_color, 0.82)); }
.pdf-view > row { padding: 8px 0; background: none; }
.pdf-view > row:hover, .pdf-view > row:selected { background: none; }
.pdf-page { box-shadow: 0 1px 5px alpha(black, 0.35); }
.pdf-controls { border-radius: 10px; padding: 2px; }
.issues-panel { background-color: @view_bg_color; }
.issues image.error { color: @error_color; }
.issues image.warning { color: @warning_color; }
";

fn main() -> glib::ExitCode {
    let app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::HANDLES_OPEN)
        .build();

    app.connect_startup(|app| {
        let provider = gtk::CssProvider::new();
        provider.load_from_string(CSS);
        gtk::style_context_add_provider_for_display(
            &gtk::gdk::Display::default().expect("sin display"),
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

    app.run()
}
