//! Apertura rápida (Ctrl+P): búsqueda difusa de archivos del proyecto y, si
//! la consulta empieza por `@`, de secciones del esquema.

use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, glib};

use crate::index::Section;

const MAX_RESULTS: usize = 60;

pub enum Target {
    File(PathBuf),
    Section(PathBuf, u32),
}

/// Puntuación difusa: todas las letras de `query` en orden dentro de `text`.
/// Premia las coincidencias seguidas, al inicio de palabra y en el nombre del archivo.
pub fn score(query: &str, text: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    let text_l: Vec<char> = text.to_lowercase().chars().collect();
    let name_start = text.rfind('/').map(|i| text[..=i].chars().count()).unwrap_or(0);
    let mut score = 0;
    let mut ti = 0;
    let mut prev: Option<usize> = None;
    for qc in query.to_lowercase().chars().filter(|c| !c.is_whitespace()) {
        let found = (ti..text_l.len()).find(|&i| text_l[i] == qc)?;
        score += 1;
        if prev.is_some_and(|p| p + 1 == found) {
            score += 5;
        }
        if found == 0 || matches!(text_l[found - 1], '/' | '_' | '-' | '.' | ' ') {
            score += 3;
        }
        if found >= name_start {
            score += 2;
        }
        prev = Some(found);
        ti = found + 1;
    }
    // A igualdad, las rutas cortas primero.
    Some(score * 100 - text_l.len() as i32)
}

pub fn show(parent: &impl IsA<gtk::Widget>, files: Vec<PathBuf>, sections: Vec<Section>, on_pick: impl Fn(Target) + 'static) {
    let entry = gtk::SearchEntry::builder().placeholder_text("Buscar archivo…  (@ para secciones)").hexpand(true).build();
    let list = gtk::ListBox::builder().css_classes(["navigation-sidebar", "quick-open"]).build();
    let scroller = gtk::ScrolledWindow::builder().child(&list).min_content_height(360).hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).build();
    let content = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(8).margin_top(12).margin_bottom(12).margin_start(12).margin_end(12).build();
    content.append(&entry);
    content.append(&scroller);
    let dialog = adw::Dialog::builder().title("Abrir").content_width(560).child(&content).build();

    let targets: Rc<std::cell::RefCell<Vec<Target>>> = Rc::default();
    let on_pick = Rc::new(on_pick);

    let fill = {
        let list = list.clone();
        let targets = targets.clone();
        move |query: &str| {
            while let Some(c) = list.first_child() {
                list.remove(&c);
            }
            let mut out = targets.borrow_mut();
            out.clear();
            let mut rows: Vec<(i32, String, String, Target)> = Vec::new();
            if let Some(q) = query.strip_prefix('@') {
                for s in &sections {
                    if let Some(sc) = score(q.trim(), &s.title) {
                        rows.push((sc, s.title.clone(), format!("{}:{}", s.file.display(), s.line), Target::Section(s.file.clone(), s.line)));
                    }
                }
                // Sin texto, en el orden del documento.
                if !q.trim().is_empty() {
                    rows.sort_by_key(|r| -r.0);
                }
            } else {
                for f in &files {
                    let text = f.display().to_string();
                    if let Some(sc) = score(query.trim(), &text) {
                        let name = f.file_name().unwrap_or_default().to_string_lossy().into_owned();
                        let dir = f.parent().map(|p| p.display().to_string()).unwrap_or_default();
                        rows.push((sc, name, dir, Target::File(f.clone())));
                    }
                }
                rows.sort_by_key(|r| -r.0);
            }
            for (_, title, subtitle, target) in rows.into_iter().take(MAX_RESULTS) {
                let row = gtk::Box::builder().orientation(gtk::Orientation::Vertical).margin_top(2).margin_bottom(2).build();
                row.append(&gtk::Label::builder().label(&title).xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).build());
                if !subtitle.is_empty() {
                    row.append(&gtk::Label::builder().label(&subtitle).xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::Start).css_classes(["dim-label", "caption"]).build());
                }
                list.append(&row);
                out.push(target);
            }
            if let Some(first) = list.row_at_index(0) {
                list.select_row(Some(&first));
            }
        }
    };
    fill("");
    let fill = Rc::new(fill);

    let f = fill.clone();
    entry.connect_search_changed(move |e| f(&e.text()));

    let pick = {
        let dialog = dialog.clone();
        let targets = targets.clone();
        let on_pick = on_pick.clone();
        move |index: i32| {
            let target = {
                let mut t = targets.borrow_mut();
                if index < 0 || index as usize >= t.len() {
                    return;
                }
                t.swap_remove(index as usize)
            };
            dialog.close();
            on_pick(target);
        }
    };
    let p = pick.clone();
    list.connect_row_activated(move |_, row| p(row.index()));
    let (p, l) = (pick.clone(), list.clone());
    entry.connect_activate(move |_| p(l.selected_row().map_or(0, |r| r.index())));

    // Flechas para moverse por la lista sin salir del campo de búsqueda.
    let keys = gtk::EventControllerKey::new();
    let l = list.clone();
    keys.connect_key_pressed(move |_, key, _, _| {
        let delta = match key {
            gdk::Key::Down => 1,
            gdk::Key::Up => -1,
            _ => return glib::Propagation::Proceed,
        };
        let current = l.selected_row().map_or(-1, |r| r.index());
        if let Some(row) = l.row_at_index((current + delta).max(0)) {
            l.select_row(Some(&row));
            row.grab_focus();
        }
        glib::Propagation::Stop
    });
    entry.add_controller(keys);
    let e = entry.clone();
    list.connect_row_selected(move |_, _| {
        if !e.has_focus() {
            e.grab_focus();
            e.set_position(-1);
        }
    });

    dialog.present(Some(parent));
    entry.grab_focus();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_scores() {
        assert!(score("intro", "capitulos/intro.tex").is_some());
        assert!(score("cpin", "capitulos/intro.tex").is_some());
        assert!(score("xyz", "capitulos/intro.tex").is_none());
        // Mejor en el nombre que en la carpeta.
        assert!(score("intro", "capitulos/intro.tex") > score("intro", "introduccion/otro.tex"));
        assert!(score("main", "main.tex") > score("main", "anexos/domain/notes.tex"));
    }
}
