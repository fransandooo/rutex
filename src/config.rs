//! Preferencias en ~/.config/rutex/config, formato `clave=valor` por línea.

use std::fs;
use std::path::PathBuf;

const MAX_RECENT: usize = 12;

#[derive(Default, Clone)]
pub struct Config {
    pub vim: bool,
    pub recent: Vec<PathBuf>,
    /// Archivo principal elegido a mano: (raíz, relativo).
    pub mains: Vec<(PathBuf, PathBuf)>,
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
            match k.trim() {
                "vim" => cfg.vim = v.trim() == "true",
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
        let mut out = format!("vim={}\n", self.vim);
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
