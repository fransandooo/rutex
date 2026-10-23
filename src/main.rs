mod changebar;
mod check;
mod compiler;
mod completion;
mod config;
mod demo;
mod editor;
mod export;
mod filetree;
mod history;
mod historyview;
mod hover;
mod i18n;
mod index;
mod insert;
mod keys;
mod logparse;
mod mathpreview;
mod missing;
mod newproject;
mod pdf;
mod prefs;
mod project;
mod quickopen;
mod refs;
mod search;
mod spell;
mod symbols;
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

/// On laptops with an NVIDIA card next to an integrated one, keep GTK on the
/// integrated GPU: initializing the NVIDIA driver wakes the card (battery) and
/// adds ~45 MB of memory, for an editor that needs none of it. Skipped when the
/// user chose a renderer or driver themselves.
fn prefer_integrated_gpu() {
    let set = |v: &str| std::env::var_os(v).is_some();
    if ["GSK_RENDERER", "VK_LOADER_DRIVERS_SELECT", "VK_LOADER_DRIVERS_DISABLE", "__EGL_VENDOR_LIBRARY_FILENAMES", "__NV_PRIME_RENDER_OFFLOAD", "DRI_PRIME"].iter().any(|v| set(v)) {
        return;
    }
    let drivers: Vec<String> = std::fs::read_dir("/sys/class/drm")
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().strip_prefix("card").is_some_and(|n| n.chars().all(|c| c.is_ascii_digit())))
        .filter_map(|e| std::fs::read_link(e.path().join("device/driver")).ok())
        .filter_map(|d| d.file_name().map(|n| n.to_string_lossy().into_owned()))
        .collect();
    let nvidia = drivers.iter().any(|d| d == "nvidia");
    let other = drivers.iter().any(|d| d != "nvidia" && d != "nouveau");
    if nvidia && other {
        // SAFETY: called first thing in main, before any other thread exists.
        unsafe {
            std::env::set_var("VK_LOADER_DRIVERS_DISABLE", "*nvidia*");
            if std::path::Path::new("/usr/share/glvnd/egl_vendor.d/50_mesa.json").is_file() {
                std::env::set_var("__EGL_VENDOR_LIBRARY_FILENAMES", "/usr/share/glvnd/egl_vendor.d/50_mesa.json");
            }
        }
    }
}

fn main() -> glib::ExitCode {
    prefer_integrated_gpu();
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
