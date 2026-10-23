//! Tema: sigue la paleta de Omarchy (`colors.toml`) y la aplica en caliente a la
//! interfaz (variables CSS de libadwaita) y al editor (esquema de GtkSourceView
//! generado). Vigila la carpeta del tema con inotify: cuando ejecutas
//! `omarchy-theme-set`, rutex cambia de colores sin reiniciarse y sin sondeos.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use adw::prelude::*;
use gtk::{gio, glib};

use crate::config::{self, ThemeMode};

const SCHEME_ID: &str = "rutex-omarchy";

#[derive(Clone, Debug)]
pub struct Palette {
    pub dark: bool,
    pub accent: String,
    pub bg: String,
    pub dark_bg: String,
    pub darker_bg: String,
    pub lighter_bg: String,
    pub fg: String,
    pub muted: String,
    pub selection: String,
    pub red: String,
    pub orange: String,
    pub yellow: String,
    pub green: String,
    pub cyan: String,
    pub blue: String,
    pub magenta: String,
}

struct State {
    css: gtk::CssProvider,
    font_css: gtk::CssProvider,
    palette: Option<Palette>,
    monitors: Vec<gio::FileMonitor>,
    pending: Option<glib::SourceId>,
    listeners: Vec<Box<dyn Fn()>>,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

/// Carpetas donde Omarchy deja el tema actual (versiones nuevas y antiguas).
fn omarchy_current_dirs() -> Vec<PathBuf> {
    let home = glib::home_dir();
    vec![home.join(".local/state/omarchy/current"), home.join(".config/omarchy/current")]
}

fn colors_file() -> Option<PathBuf> {
    omarchy_current_dirs().into_iter().map(|d| d.join("theme/colors.toml")).find(|p| p.is_file())
}

/// Lee un `colors.toml` plano (`clave = "valor"`).
pub fn parse_palette(text: &str) -> Option<Palette> {
    let mut map: HashMap<&str, String> = HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else { continue };
        let v = v.trim().trim_matches('"').trim_matches('\'').to_string();
        map.insert(k.trim(), v);
    }
    let hex = |k: &str| map.get(k).filter(|v| v.starts_with('#') && (v.len() == 7 || v.len() == 9)).map(|v| v[..7].to_string());
    let bg = hex("background")?;
    let fg = hex("foreground")?;
    let accent = hex("accent").or_else(|| hex("blue")).unwrap_or_else(|| fg.clone());
    let dark = match map.get("mode").map(String::as_str) {
        Some("light") => false,
        Some("dark") => true,
        _ => luminance(&bg) < 0.5,
    };
    let pick = |k: &str, fallback: &str| hex(k).unwrap_or_else(|| fallback.to_string());
    Some(Palette {
        dark,
        dark_bg: hex("dark_background").unwrap_or_else(|| mix(&bg, if dark { "#000000" } else { "#ffffff" }, 0.25)),
        darker_bg: hex("darker_background").unwrap_or_else(|| mix(&bg, if dark { "#000000" } else { "#ffffff" }, 0.45)),
        lighter_bg: hex("lighter_background").unwrap_or_else(|| mix(&bg, &fg, 0.08)),
        muted: hex("muted").unwrap_or_else(|| mix(&fg, &bg, 0.45)),
        selection: hex("selection").unwrap_or_else(|| mix(&bg, &accent, 0.3)),
        red: pick("red", "#e06c75"),
        orange: hex("orange").unwrap_or_else(|| pick("yellow", "#d19a66")),
        yellow: pick("yellow", "#e5c07b"),
        green: pick("green", "#98c379"),
        cyan: pick("cyan", "#56b6c2"),
        blue: pick("blue", "#61afef"),
        magenta: pick("magenta", "#c678dd"),
        accent,
        bg,
        fg,
    })
}

fn rgb(hex: &str) -> (f64, f64, f64) {
    let p = |i: usize| u8::from_str_radix(hex.get(i..i + 2).unwrap_or("00"), 16).unwrap_or(0) as f64 / 255.0;
    (p(1), p(3), p(5))
}

fn luminance(hex: &str) -> f64 {
    let (r, g, b) = rgb(hex);
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

/// Mezcla `a` con `b` (t = 0 → a, t = 1 → b).
fn mix(a: &str, b: &str, t: f64) -> String {
    let (ar, ag, ab) = rgb(a);
    let (br, bg, bb) = rgb(b);
    let c = |x: f64, y: f64| ((x + (y - x) * t) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", c(ar, br), c(ag, bg), c(ab, bb))
}

fn css_for(p: &Palette) -> String {
    let accent_fg = if luminance(&p.accent) > 0.55 { "#000000" } else { "#ffffff" };
    let border = mix(&p.bg, &p.fg, 0.12);
    format!(
        ":root {{
  --accent-bg-color: {accent};
  --accent-fg-color: {accent_fg};
  --accent-color: {accent};
  --window-bg-color: {bg};
  --window-fg-color: {fg};
  --view-bg-color: {bg};
  --view-fg-color: {fg};
  --headerbar-bg-color: {dark_bg};
  --headerbar-fg-color: {fg};
  --headerbar-backdrop-color: {dark_bg};
  --headerbar-border-color: {border};
  --sidebar-bg-color: {dark_bg};
  --sidebar-fg-color: {fg};
  --sidebar-backdrop-color: {dark_bg};
  --sidebar-border-color: {border};
  --secondary-sidebar-bg-color: {dark_bg};
  --secondary-sidebar-fg-color: {fg};
  --card-bg-color: {lighter_bg};
  --card-fg-color: {fg};
  --dialog-bg-color: {dark_bg};
  --dialog-fg-color: {fg};
  --popover-bg-color: {lighter_bg};
  --popover-fg-color: {fg};
  --thumbnail-bg-color: {lighter_bg};
  --error-color: {red};
  --error-bg-color: {red};
  --destructive-color: {red};
  --destructive-bg-color: {red};
  --warning-color: {yellow};
  --success-color: {green};
  --border-color: {border};
  --rutex-pdf-bg: {darker_bg};
  --rutex-muted: {muted};
}}
",
        accent = p.accent,
        bg = p.bg,
        fg = p.fg,
        dark_bg = p.dark_bg,
        lighter_bg = p.lighter_bg,
        red = p.red,
        yellow = p.yellow,
        green = p.green,
        muted = p.muted,
        darker_bg = p.darker_bg,
    )
}

fn scheme_for(p: &Palette) -> String {
    let current = mix(&p.bg, &p.fg, 0.05);
    let search = mix(&p.bg, &p.yellow, 0.35);
    let styles = [
        format!(r#"<style name="text" foreground="{}" background="{}"/>"#, p.fg, p.bg),
        format!(r#"<style name="selection" background="{}"/>"#, p.selection),
        format!(r#"<style name="selection-unfocused" background="{}"/>"#, p.selection),
        format!(r#"<style name="cursor" foreground="{}"/>"#, p.accent),
        format!(r#"<style name="current-line" background="{current}"/>"#),
        format!(r#"<style name="line-numbers" foreground="{}" background="{}"/>"#, p.muted, p.bg),
        format!(r#"<style name="current-line-number" foreground="{}" background="{current}" bold="true"/>"#, p.fg),
        format!(r#"<style name="bracket-match" foreground="{}" bold="true" underline="single"/>"#, p.accent),
        format!(r#"<style name="bracket-mismatch" foreground="{}" underline="error"/>"#, p.red),
        format!(r#"<style name="search-match" background="{search}"/>"#),
        format!(r#"<style name="draw-spaces" foreground="{}"/>"#, p.muted),
        format!(r#"<style name="right-margin" foreground="{}" background="{}"/>"#, p.muted, p.dark_bg),
        format!(r#"<style name="def:comment" foreground="{}" italic="true"/>"#, p.muted),
        format!(r#"<style name="def:keyword" foreground="{}"/>"#, p.accent),
        format!(r#"<style name="def:statement" foreground="{}"/>"#, p.accent),
        format!(r#"<style name="def:string" foreground="{}"/>"#, p.green),
        format!(r#"<style name="def:special-char" foreground="{}"/>"#, p.orange),
        format!(r#"<style name="def:preprocessor" foreground="{}"/>"#, p.magenta),
        format!(r#"<style name="def:number" foreground="{}"/>"#, p.orange),
        format!(r#"<style name="def:constant" foreground="{}"/>"#, p.orange),
        format!(r#"<style name="def:type" foreground="{}"/>"#, p.yellow),
        format!(r#"<style name="def:function" foreground="{}"/>"#, p.blue),
        format!(r#"<style name="def:identifier" foreground="{}"/>"#, p.cyan),
        format!(r#"<style name="def:operator" foreground="{}"/>"#, p.fg),
        format!(r#"<style name="def:error" underline="error" foreground="{}"/>"#, p.red),
        format!(r#"<style name="def:heading0" foreground="{}" bold="true"/>"#, p.yellow),
        format!(r#"<style name="def:heading1" foreground="{}" bold="true"/>"#, p.yellow),
        format!(r#"<style name="def:heading2" foreground="{}" bold="true"/>"#, p.yellow),
        format!(r#"<style name="def:heading3" foreground="{}" bold="true"/>"#, p.yellow),
        format!(r#"<style name="def:heading4" foreground="{}"/>"#, p.yellow),
        format!(r#"<style name="def:heading5" foreground="{}"/>"#, p.yellow),
        format!(r#"<style name="def:heading6" foreground="{}"/>"#, p.yellow),
        format!(r#"<style name="latex:display-math" foreground="{}"/>"#, p.green),
        format!(r#"<style name="latex:included-file" foreground="{}"/>"#, p.blue),
        format!(r#"<style name="latex:include" foreground="{}"/>"#, p.magenta),
    ];
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<style-scheme id=\"{SCHEME_ID}\" name=\"Omarchy\" version=\"1.0\">\n  {}\n</style-scheme>\n",
        styles.join("\n  ")
    )
}

fn styles_dir() -> PathBuf {
    glib::user_cache_dir().join("rutex").join("styles")
}

/// Inicializa el tema. Llamar una vez tras crear el display.
pub fn init() {
    let display = gtk::gdk::Display::default().expect("no display");
    let css = gtk::CssProvider::new();
    let font_css = gtk::CssProvider::new();
    // Por encima de libadwaita pero por debajo de la hoja de estilos propia.
    gtk::style_context_add_provider_for_display(&display, &css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION - 1);
    gtk::style_context_add_provider_for_display(&display, &font_css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1);

    let dir = styles_dir();
    let _ = fs::create_dir_all(&dir);
    let manager = sourceview5::StyleSchemeManager::default();
    manager.prepend_search_path(&dir.to_string_lossy());

    STATE.with(|s| {
        *s.borrow_mut() = Some(State { css, font_css, palette: None, monitors: Vec::new(), pending: None, listeners: Vec::new() })
    });

    adw::StyleManager::default().connect_dark_notify(|_| notify());
    apply();
    set_font_size(config::get().font_size);
}

/// Avisa cuando cambian los colores (para repintar editor y PDF).
pub fn connect_changed(f: impl Fn() + 'static) {
    STATE.with(|s| s.borrow_mut().as_mut().unwrap().listeners.push(Box::new(f)));
}

fn notify() {
    // Los oyentes pueden registrar otros: se llaman fuera del préstamo.
    let listeners = STATE.with(|s| std::mem::take(&mut s.borrow_mut().as_mut().unwrap().listeners));
    for l in &listeners {
        l();
    }
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        let st = st.as_mut().unwrap();
        let added = std::mem::take(&mut st.listeners);
        st.listeners = listeners;
        st.listeners.extend(added);
    });
}

pub fn palette() -> Option<Palette> {
    STATE.with(|s| s.borrow().as_ref().and_then(|st| st.palette.clone()))
}

pub fn set_font_size(pt: u32) {
    STATE.with(|s| {
        if let Some(st) = s.borrow().as_ref() {
            st.font_css.load_from_string(&format!(".rutex-editor {{ font-size: {pt}pt; }}"));
        }
    });
}

/// Id del esquema de GtkSourceView a usar ahora mismo.
pub fn scheme_id() -> String {
    if palette().is_some() {
        return SCHEME_ID.to_string();
    }
    if adw::StyleManager::default().is_dark() { "Adwaita-dark".into() } else { "Adwaita".into() }
}

/// Recalcula el tema según la configuración y la paleta actual de Omarchy.
pub fn apply() {
    let mode = config::get().theme;
    let style = adw::StyleManager::default();
    let palette = if mode == ThemeMode::Auto {
        colors_file().and_then(|f| fs::read_to_string(f).ok()).and_then(|t| parse_palette(&t))
    } else {
        None
    };

    STATE.with(|s| {
        let mut st = s.borrow_mut();
        let st = st.as_mut().unwrap();
        match &palette {
            Some(p) => {
                st.css.load_from_string(&css_for(p));
                let _ = fs::write(styles_dir().join(format!("{SCHEME_ID}.xml")), scheme_for(p));
                sourceview5::StyleSchemeManager::default().force_rescan();
            }
            None => st.css.load_from_string(""),
        }
        st.palette = palette.clone();
    });

    style.set_color_scheme(match (mode, &palette) {
        (_, Some(p)) if p.dark => adw::ColorScheme::ForceDark,
        (_, Some(_)) => adw::ColorScheme::ForceLight,
        (ThemeMode::Light, _) => adw::ColorScheme::ForceLight,
        (ThemeMode::Dark, _) => adw::ColorScheme::ForceDark,
        _ => adw::ColorScheme::Default,
    });

    watch();
    notify();
}

/// Vigila la carpeta `current` (Omarchy sustituye `theme` entera al cambiar de
/// tema) y el propio `theme` (por si se edita `colors.toml` a mano).
fn watch() {
    let mut monitors = Vec::new();
    if config::get().theme == ThemeMode::Auto {
        for dir in omarchy_current_dirs().into_iter().filter(|d| d.is_dir()) {
            for target in [dir.clone(), dir.join("theme")] {
                let file = gio::File::for_path(&target);
                if let Ok(m) = file.monitor_directory(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE) {
                    m.connect_changed(|_, _, _, _| schedule_apply());
                    monitors.push(m);
                }
            }
        }
    }
    STATE.with(|s| {
        if let Some(st) = s.borrow_mut().as_mut() {
            for m in st.monitors.drain(..) {
                m.cancel();
            }
            st.monitors = monitors;
        }
    });
}

/// `omarchy-theme-set` genera muchos eventos seguidos: esperamos a que acabe.
fn schedule_apply() {
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        let st = st.as_mut().unwrap();
        if let Some(id) = st.pending.take() {
            id.remove();
        }
        st.pending = Some(glib::timeout_add_local_once(Duration::from_millis(300), || {
            STATE.with(|s| s.borrow_mut().as_mut().unwrap().pending = None);
            apply();
        }));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_omarchy_colors() {
        let text = "# Tema\nmode = \"dark\"\naccent = \"#d6634d\"\nbackground = \"#191820\"\nforeground = \"#eecca2\"\nhyprland_active_border = \"rgb(d6634d)\"\n";
        let p = parse_palette(text).unwrap();
        assert!(p.dark);
        assert_eq!(p.accent, "#d6634d");
        assert_eq!(p.bg, "#191820");
        assert_eq!(p.red, "#e06c75");
        assert!(parse_palette("accent = \"#fff\"").is_none());
    }

    #[test]
    fn light_mode_from_luminance() {
        let p = parse_palette("background = \"#faf4ed\"\nforeground = \"#575279\"").unwrap();
        assert!(!p.dark);
    }

    #[test]
    fn mixing() {
        assert_eq!(mix("#000000", "#ffffff", 0.5), "#808080");
    }
}
