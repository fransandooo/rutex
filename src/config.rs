//! Preferencias en ~/.config/rutex/config, formato `clave=valor` por línea.

use std::fs;
use std::path::PathBuf;

const MAX_RECENT: usize = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemeMode {
    /// Colores del tema de Omarchy si existe; si no, los del sistema.
    Auto,
    System,
    Light,
    Dark,
}

impl ThemeMode {
    pub const ALL: [ThemeMode; 4] = [ThemeMode::Auto, ThemeMode::System, ThemeMode::Light, ThemeMode::Dark];

    pub fn id(self) -> &'static str {
        match self {
            ThemeMode::Auto => "auto",
            ThemeMode::System => "system",
            ThemeMode::Light => "light",
            ThemeMode::Dark => "dark",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ThemeMode::Auto => "Omarchy (automático)",
            ThemeMode::System => "Sistema",
            ThemeMode::Light => "Claro",
            ThemeMode::Dark => "Oscuro",
        }
    }

    fn parse(s: &str) -> ThemeMode {
        ThemeMode::ALL.into_iter().find(|m| m.id() == s).unwrap_or(ThemeMode::Auto)
    }
}

#[derive(Clone)]
pub struct Config {
    pub theme: ThemeMode,
    pub font_size: u32,
    pub vim: bool,
    pub wrap: bool,
    pub autopairs: bool,
    pub autocompile: bool,
    pub compile_delay_ms: u32,
    pub pdf_dark: bool,
    pub recent: Vec<PathBuf>,
    /// Archivo principal elegido a mano: (raíz, relativo).
    pub mains: Vec<(PathBuf, PathBuf)>,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            theme: ThemeMode::Auto,
            font_size: 11,
            vim: false,
            wrap: true,
            autopairs: true,
            autocompile: true,
            compile_delay_ms: 800,
            pdf_dark: false,
            recent: Vec::new(),
            mains: Vec::new(),
        }
    }
}

fn path() -> PathBuf {
    gtk::glib::user_config_dir().join("rutex").join("config")
}

impl Config {
    pub fn load() -> Config {
        let mut cfg = Config::default();
        let Ok(text) = fs::read_to_string(path()) else { return cfg };
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            let flag = v.trim() == "true";
            match k.trim() {
                "theme" => cfg.theme = ThemeMode::parse(v.trim()),
                "font_size" => cfg.font_size = v.trim().parse().unwrap_or(cfg.font_size).clamp(7, 32),
                "vim" => cfg.vim = flag,
                "wrap" => cfg.wrap = flag,
                "autopairs" => cfg.autopairs = flag,
                "autocompile" => cfg.autocompile = flag,
                "compile_delay_ms" => cfg.compile_delay_ms = v.trim().parse().unwrap_or(cfg.compile_delay_ms).clamp(200, 10_000),
                "pdf_dark" => cfg.pdf_dark = flag,
                "recent" => cfg.recent.push(v.into()),
                "main" => {
                    if let Some((root, rel)) = v.split_once('\t') {
                        cfg.mains.push((root.into(), rel.into()));
                    }
                }
                _ => {}
            }
        }
        cfg.recent.retain(|p| p.is_dir());
        cfg
    }

    pub fn save(&self) {
        let mut out = String::new();
        out.push_str(&format!("theme={}\n", self.theme.id()));
        out.push_str(&format!("font_size={}\n", self.font_size));
        out.push_str(&format!("vim={}\n", self.vim));
        out.push_str(&format!("wrap={}\n", self.wrap));
        out.push_str(&format!("autopairs={}\n", self.autopairs));
        out.push_str(&format!("autocompile={}\n", self.autocompile));
        out.push_str(&format!("compile_delay_ms={}\n", self.compile_delay_ms));
        out.push_str(&format!("pdf_dark={}\n", self.pdf_dark));
        for r in &self.recent {
            out.push_str(&format!("recent={}\n", r.display()));
        }
        for (root, rel) in &self.mains {
            out.push_str(&format!("main={}\t{}\n", root.display(), rel.display()));
        }
        let p = path();
        let _ = fs::create_dir_all(p.parent().unwrap());
        let _ = fs::write(p, out);
    }

    pub fn add_recent(&mut self, root: PathBuf) {
        self.recent.retain(|r| *r != root);
        self.recent.insert(0, root);
        self.recent.truncate(MAX_RECENT);
    }

    pub fn main_for(&self, root: &PathBuf) -> Option<PathBuf> {
        self.mains.iter().find(|(r, _)| r == root).map(|(_, m)| m.clone())
    }

    pub fn set_main(&mut self, root: PathBuf, rel: PathBuf) {
        self.mains.retain(|(r, _)| *r != root);
        self.mains.push((root, rel));
    }
}

thread_local! {
    static CONFIG: std::cell::RefCell<Config> = std::cell::RefCell::new(Config::load());
}

pub fn get() -> Config {
    CONFIG.with(|c| c.borrow().clone())
}

/// Modifica la configuración y la guarda en disco.
pub fn update(f: impl FnOnce(&mut Config)) {
    CONFIG.with(|c| {
        let mut c = c.borrow_mut();
        f(&mut c);
        c.save();
    });
}
