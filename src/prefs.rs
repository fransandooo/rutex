//! Diálogo de preferencias y ventana de atajos de teclado.

use adw::prelude::*;

use crate::config::{self, ThemeMode};
use crate::i18n::tr;

/// Qué ha cambiado, para que la ventana lo aplique en caliente.
pub enum Change {
    Language,
    Spelling,
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

    // --- Idioma (los nombres van sin traducir para que siempre se encuentren) ---
    let language = adw::ComboRow::builder()
        .title("Language · Idioma")
        .model(&gtk::StringList::new(&["English", "Español"]))
        .selected(if cfg.spanish { 1 } else { 0 })
        .build();

    // --- Apariencia ---
    let theme_names: Vec<&str> = ThemeMode::ALL.iter().map(|m| m.label()).collect();
    let theme = adw::ComboRow::builder()
        .title(tr("Theme"))
        .subtitle(tr("“Omarchy” follows your system theme and changes as soon as you change it"))
        .model(&gtk::StringList::new(&theme_names))
        .selected(ThemeMode::ALL.iter().position(|m| *m == cfg.theme).unwrap_or(0) as u32)
        .build();
    let font = adw::SpinRow::builder()
        .title(tr("Editor font size"))
        .adjustment(&gtk::Adjustment::new(cfg.font_size as f64, 7.0, 32.0, 1.0, 2.0, 0.0))
        .build();
    let pdf_dark = adw::SwitchRow::builder()
        .title(tr("Dark PDF"))
        .subtitle(tr("Paints pages with the theme colors (on screen only)"))
        .active(cfg.pdf_dark)
        .build();
    let appearance = adw::PreferencesGroup::builder().title(tr("Appearance")).build();
    appearance.add(&language);
    appearance.add(&theme);
    appearance.add(&font);
    appearance.add(&pdf_dark);

    // --- Editor ---
    let wrap = adw::SwitchRow::builder().title(tr("Wrap long lines")).active(cfg.wrap).build();
    let autopairs = adw::SwitchRow::builder()
        .title(tr("Auto-close brackets"))
        .subtitle(tr("{ [ ( $ and \\end{…} when pressing Enter after \\begin{…}"))
        .active(cfg.autopairs)
        .build();
    let spell = adw::SwitchRow::builder()
        .title(tr("Check spelling"))
        .subtitle(tr("Right-click a word for suggestions"))
        .active(cfg.spellcheck)
        .build();
    let spell_names = [tr("Same as the document (babel)"), "English (US)", "English (UK)", "Español"];
    let spell_codes = ["auto", "en_US", "en_GB", "es_ES"];
    let spell_lang = adw::ComboRow::builder()
        .title(tr("Spelling language"))
        .model(&gtk::StringList::new(&spell_names))
        .selected(spell_codes.iter().position(|c| *c == cfg.spell_lang).unwrap_or(0) as u32)
        .build();
    spell.bind_property("active", &spell_lang, "sensitive").sync_create().build();
    let vim = adw::SwitchRow::builder().title(tr("Vim mode")).active(cfg.vim).build();
    let editor = adw::PreferencesGroup::builder().title("Editor").build();
    editor.add(&wrap);
    editor.add(&autopairs);
    editor.add(&spell);
    editor.add(&spell_lang);
    editor.add(&vim);

    // --- Compilación ---
    let autocompile = adw::SwitchRow::builder()
        .title(tr("Compile automatically"))
        .subtitle(tr("When off, files are still saved but only compile with Ctrl+S or Ctrl+Enter"))
        .active(cfg.autocompile)
        .build();
    let delay = adw::SpinRow::builder()
        .title(tr("Delay after you stop typing (ms)"))
        .adjustment(&gtk::Adjustment::new(cfg.compile_delay_ms as f64, 200.0, 10_000.0, 100.0, 500.0, 0.0))
        .build();
    autocompile.bind_property("active", &delay, "sensitive").sync_create().build();
    let build = adw::PreferencesGroup::builder().title(tr("Compilation")).build();
    build.add(&autocompile);
    build.add(&delay);

    // Optional AI explanations: off by default, the user's own key.
    let ai_switch = adw::SwitchRow::builder()
        .title(tr("Explain errors with Claude"))
        .subtitle(tr("Adds an “Ask Claude” button to each problem. Only when you press it, the error and the lines around it are sent to Anthropic."))
        .active(cfg.ai_help)
        .build();
    let ai_key = adw::PasswordEntryRow::builder().title(tr("Anthropic API key")).show_apply_button(true).build();
    ai_key.set_text(&crate::ai::saved_key());
    let ai = adw::PreferencesGroup::builder()
        .title(tr("AI help (optional)"))
        .description(tr("Uses your own API key from console.anthropic.com; each answer costs a fraction of a cent. The key is stored only on this computer."))
        .build();
    ai.add(&ai_switch);
    if crate::ai::key_from_env() {
        ai.add(&adw::ActionRow::builder().title(tr("Using ANTHROPIC_API_KEY from the environment")).css_classes(["dim-label"]).build());
    } else {
        ai.add(&ai_key);
    }

    let page = adw::PreferencesPage::new();
    page.add(&appearance);
    page.add(&editor);
    page.add(&build);
    page.add(&ai);
    let dialog = adw::PreferencesDialog::builder().title(tr("Preferences")).build();
    dialog.add(&page);

    let cb = on_change.clone();
    language.connect_selected_notify(move |row| {
        let spanish = row.selected() == 1;
        config::update(|c| c.spanish = spanish);
        cb(Change::Language);
    });
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
    spell.connect_active_notify(move |row| {
        let on = row.is_active();
        config::update(|c| c.spellcheck = on);
        cb(Change::Spelling);
    });
    let cb = on_change.clone();
    spell_lang.connect_selected_notify(move |row| {
        let code = spell_codes[row.selected() as usize % spell_codes.len()];
        config::update(|c| c.spell_lang = code.to_string());
        cb(Change::Spelling);
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

    ai_switch.connect_active_notify(|row| {
        let on = row.is_active();
        config::update(|c| c.ai_help = on);
    });
    let toast_target = dialog.clone();
    ai_key.connect_apply(move |row| {
        let msg = match crate::ai::save_key(&row.text()) {
            Ok(()) if row.text().trim().is_empty() => tr("API key removed"),
            Ok(()) => tr("API key saved"),
            Err(_) => tr("Could not save the API key"),
        };
        toast_target.add_toast(adw::Toast::new(msg));
    });

    dialog.present(Some(parent));
}

pub fn shortcuts(parent: &impl IsA<gtk::Widget>) {
    let sections: &[(&str, &[(&str, &str)])] = &[
        (
            tr("Project"),
            &[
                (tr("Save and compile"), "<Control>s"),
                (tr("Compile now"), "<Control>Return"),
                (tr("Quick open"), "<Control>p"),
                (tr("Version history"), "<Control><Shift>h"),
                (tr("Check before submitting"), "<Control><Shift>r"),
                (tr("Math symbols"), "<Control><Shift>m"),
                (tr("Fold section"), "<Alt>minus"),
                (tr("Unfold section"), "<Alt>plus"),
                (tr("Fold all sections"), "<Control><Alt>minus"),
                (tr("Save a version"), "<Control><Shift>s"),
                (tr("New file"), "<Control>n"),
                (tr("Open project"), "<Control>o"),
                (tr("New window"), "<Control><Alt>n"),
                (tr("Show files"), "<Control><Shift>e"),
                (tr("Show outline"), "<Control><Shift>o"),
                (tr("Toggle sidebar"), "F9"),
                (tr("Problems panel"), "<Control>e"),
                (tr("Preferences"), "<Control>comma"),
            ],
        ),
        (
            tr("Editing"),
            &[
                (tr("Undo"), "<Control>z"),
                (tr("Redo"), "<Control><Shift>z"),
                (tr("Find"), "<Control>f"),
                (tr("Find in project"), "<Control><Shift>f"),
                (tr("Go to line"), "<Control>g"),
                (tr("Comment or uncomment"), "<Control>slash"),
                (tr("Bold (\\textbf)"), "<Control>b"),
                (tr("Italic (\\textit)"), "<Control>i"),
                (tr("Duplicate line"), "<Control>d"),
                (tr("Delete line"), "<Control><Shift>k"),
                (tr("Move line"), "<Alt>Up <Alt>Down"),
                (tr("Show suggestions"), "<Control>space"),
                (tr("Close tab"), "<Control>w"),
            ],
        ),
        (
            "PDF",
            &[
                (tr("Jump from source to PDF"), "<Control>j"),
                (tr("Zoom in"), "<Control>plus"),
                (tr("Zoom out"), "<Control>minus"),
                (tr("Fit width"), "<Control>0"),
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
