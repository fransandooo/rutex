//! Diálogo de preferencias y ventana de atajos de teclado.

use adw::prelude::*;

use crate::config::{self, ThemeMode};

/// Qué ha cambiado, para que la ventana lo aplique en caliente.
pub enum Change {
    Theme,
    FontSize(u32),
    Wrap(bool),
    Autopairs(bool),
    Vim(bool),
    PdfDark,
    Autocompile,
}

pub fn show(parent: &impl IsA<gtk::Widget>, on_change: impl Fn(Change) + 'static) {
    let cfg = config::get();
    let on_change = std::rc::Rc::new(on_change);

    // --- Apariencia ---
    let theme_names: Vec<&str> = ThemeMode::ALL.iter().map(|m| m.label()).collect();
    let theme = adw::ComboRow::builder()
        .title("Tema")
        .subtitle("«Omarchy» sigue el tema del sistema y cambia en cuanto lo cambias tú")
        .model(&gtk::StringList::new(&theme_names))
        .selected(ThemeMode::ALL.iter().position(|m| *m == cfg.theme).unwrap_or(0) as u32)
        .build();
    let font = adw::SpinRow::builder()
        .title("Tamaño de letra del editor")
        .adjustment(&gtk::Adjustment::new(cfg.font_size as f64, 7.0, 32.0, 1.0, 2.0, 0.0))
        .build();
    let pdf_dark = adw::SwitchRow::builder()
        .title("PDF en modo oscuro")
        .subtitle("Pinta las páginas con los colores del tema (solo en pantalla)")
        .active(cfg.pdf_dark)
        .build();
    let appearance = adw::PreferencesGroup::builder().title("Apariencia").build();
    appearance.add(&theme);
    appearance.add(&font);
    appearance.add(&pdf_dark);

    // --- Editor ---
    let wrap = adw::SwitchRow::builder().title("Ajustar líneas largas").active(cfg.wrap).build();
    let autopairs = adw::SwitchRow::builder()
        .title("Cerrar llaves automáticamente")
        .subtitle("{ [ ( $ y \\end{…} al pulsar Intro tras \\begin{…}")
        .active(cfg.autopairs)
        .build();
    let vim = adw::SwitchRow::builder().title("Modo Vim").active(cfg.vim).build();
    let editor = adw::PreferencesGroup::builder().title("Editor").build();
    editor.add(&wrap);
    editor.add(&autopairs);
    editor.add(&vim);

    // --- Compilación ---
    let autocompile = adw::SwitchRow::builder()
        .title("Compilar automáticamente")
        .subtitle("Si lo desactivas, se guarda igual pero solo compila con Ctrl+S o Ctrl+Intro")
        .active(cfg.autocompile)
        .build();
    let delay = adw::SpinRow::builder()
        .title("Espera tras dejar de escribir (ms)")
        .adjustment(&gtk::Adjustment::new(cfg.compile_delay_ms as f64, 200.0, 10_000.0, 100.0, 500.0, 0.0))
        .build();
    autocompile.bind_property("active", &delay, "sensitive").sync_create().build();
    let build = adw::PreferencesGroup::builder().title("Compilación").build();
    build.add(&autocompile);
    build.add(&delay);

    let page = adw::PreferencesPage::new();
    page.add(&appearance);
    page.add(&editor);
    page.add(&build);
    let dialog = adw::PreferencesDialog::builder().title("Preferencias").build();
    dialog.add(&page);

    let cb = on_change.clone();
    theme.connect_selected_notify(move |row| {
        let mode = ThemeMode::ALL[row.selected() as usize % ThemeMode::ALL.len()];
        config::update(|c| c.theme = mode);
        cb(Change::Theme);
    });
    let cb = on_change.clone();
    font.connect_value_notify(move |row| {
        let pt = row.value() as u32;
        config::update(|c| c.font_size = pt);
        cb(Change::FontSize(pt));
    });
    let cb = on_change.clone();
    pdf_dark.connect_active_notify(move |row| {
        let on = row.is_active();
        config::update(|c| c.pdf_dark = on);
        cb(Change::PdfDark);
    });
    let cb = on_change.clone();
    wrap.connect_active_notify(move |row| {
        let on = row.is_active();
        config::update(|c| c.wrap = on);
        cb(Change::Wrap(on));
    });
    let cb = on_change.clone();
    autopairs.connect_active_notify(move |row| {
        let on = row.is_active();
        config::update(|c| c.autopairs = on);
        cb(Change::Autopairs(on));
    });
    let cb = on_change.clone();
    vim.connect_active_notify(move |row| {
        let on = row.is_active();
        config::update(|c| c.vim = on);
        cb(Change::Vim(on));
    });
    let cb = on_change.clone();
    autocompile.connect_active_notify(move |row| {
        let on = row.is_active();
        config::update(|c| c.autocompile = on);
        cb(Change::Autocompile);
    });
    let cb = on_change.clone();
    delay.connect_value_notify(move |row| {
        let ms = row.value() as u32;
        config::update(|c| c.compile_delay_ms = ms);
        cb(Change::Autocompile);
    });

    dialog.present(Some(parent));
}

pub fn shortcuts(parent: &impl IsA<gtk::Widget>) {
    let sections: &[(&str, &[(&str, &str)])] = &[
        (
            "Proyecto",
            &[
                ("Guardar y compilar", "<Control>s"),
                ("Compilar ahora", "<Control>Return"),
                ("Apertura rápida de archivos", "<Control>p"),
                ("Nuevo archivo", "<Control>n"),
                ("Abrir proyecto", "<Control>o"),
                ("Mostrar archivos", "<Control><Shift>e"),
                ("Mostrar esquema", "<Control><Shift>o"),
                ("Mostrar u ocultar la barra lateral", "F9"),
                ("Panel de problemas", "<Control>e"),
                ("Preferencias", "<Control>comma"),
            ],
        ),
        (
            "Edición",
            &[
                ("Deshacer", "<Control>z"),
                ("Rehacer", "<Control><Shift>z"),
                ("Buscar", "<Control>f"),
                ("Ir a la línea", "<Control>g"),
                ("Comentar o descomentar", "<Control>slash"),
                ("Negrita (\\textbf)", "<Control>b"),
                ("Cursiva (\\textit)", "<Control>i"),
                ("Duplicar línea", "<Control>d"),
                ("Borrar línea", "<Control><Shift>k"),
                ("Mover línea", "<Alt>Up <Alt>Down"),
                ("Mostrar sugerencias", "<Control>space"),
                ("Cerrar pestaña", "<Control>w"),
            ],
        ),
        (
            "PDF",
            &[
                ("Ir del código al PDF", "<Control>j"),
                ("Acercar", "<Control>plus"),
                ("Alejar", "<Control>minus"),
                ("Ajustar al ancho", "<Control>0"),
            ],
        ),
    ];
    let dialog = adw::ShortcutsDialog::new();
    for (title, items) in sections {
        let section = adw::ShortcutsSection::new(Some(title));
        for (name, accel) in *items {
            section.add(adw::ShortcutsItem::new(name, accel));
        }
        dialog.add(section);
    }
    dialog.present(Some(parent));
}
