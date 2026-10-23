//! "Version history" window: list of versions, what changed compared with the
//! current files, and restore.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use adw::prelude::*;
use gtk::{gio, glib};

use crate::history::{History, Version};
use crate::i18n::{tr, trf};

/// "5 min ago", "yesterday, 14:05"…
pub fn ago(time: i64) -> String {
    let now = glib::DateTime::now_local().map(|d| d.to_unix()).unwrap_or(time);
    let secs = (now - time).max(0);
    if secs < 60 {
        return tr("just now").to_string();
    }
    if secs < 3600 {
        return trf("{} min ago", &[&(secs / 60)]);
    }
    let Ok(dt) = glib::DateTime::from_unix_local(time) else { return String::new() };
    let clock = dt.format("%H:%M").map(|s| s.to_string()).unwrap_or_default();
    if secs < 6 * 3600 {
        return trf("{} h ago", &[&(secs / 3600)]);
    }
    let today = glib::DateTime::now_local().ok().map(|n| n.day_of_year());
    let same_year = glib::DateTime::now_local().ok().map(|n| n.year()) == Some(dt.year());
    match today {
        Some(d) if same_year && d == dt.day_of_year() => trf("today, {}", &[&clock]),
        Some(d) if same_year && d == dt.day_of_year() + 1 => trf("yesterday, {}", &[&clock]),
        _ => dt.format("%e %b %Y, %H:%M").map(|s| s.trim().to_string()).unwrap_or_default(),
    }
}

/// Opens the history window. `on_restored` runs after a restore, so the
/// project window can reload its files.
pub fn show(parent: &impl IsA<gtk::Widget>, history: Arc<History>, on_restored: impl Fn() + 'static) {
    let list = gtk::ListBox::builder().css_classes(["navigation-sidebar", "history-list"]).build();
    list.set_placeholder(Some(&gtk::Label::builder().label(tr("No versions yet.\nThey are saved automatically while you work.")).justify(gtk::Justification::Center).css_classes(["dim-label"]).margin_top(24).build()));
    let left = gtk::ScrolledWindow::builder().child(&list).hscrollbar_policy(gtk::PolicyType::Never).width_request(280).build();

    let title = gtk::Label::builder().xalign(0.0).hexpand(true).ellipsize(gtk::pango::EllipsizeMode::End).css_classes(["heading"]).build();
    let restore = gtk::Button::builder().label(tr("Restore this version")).css_classes(["suggested-action", "pill"]).sensitive(false).build();
    let top = gtk::Box::builder().spacing(12).margin_start(12).margin_end(12).margin_top(8).margin_bottom(8).build();
    let titles = gtk::Box::builder().orientation(gtk::Orientation::Vertical).hexpand(true).build();
    titles.append(&title);
    titles.append(&gtk::Label::builder().label(tr("Red: only in this version · Green: added since then")).xalign(0.0).css_classes(["dim-label", "caption"]).build());
    top.append(&titles);
    top.append(&restore);

    let diff_buf = gtk::TextBuffer::new(None);
    setup_tags(&diff_buf);
    let diff = gtk::TextView::builder().buffer(&diff_buf).editable(false).monospace(true).left_margin(12).top_margin(8).bottom_margin(12).css_classes(["history-diff"]).build();
    let right_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
    right_box.append(&top);
    right_box.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    right_box.append(&gtk::ScrolledWindow::builder().child(&diff).vexpand(true).hexpand(true).build());

    let paned = gtk::Paned::builder().start_child(&left).end_child(&right_box).shrink_start_child(false).resize_start_child(false).build();
    let header = adw::HeaderBar::new();
    let save = gtk::Button::builder().icon_name("document-save-symbolic").tooltip_text(tr("Save a version now")).build();
    header.pack_start(&save);
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&paned));
    let dialog = adw::Dialog::builder().title(tr("Version history")).content_width(1000).content_height(680).child(&toolbar).build();

    let versions: Rc<RefCell<Vec<Version>>> = Rc::default();
    let on_restored = Rc::new(on_restored);

    // Load (or reload) the list in the background.
    let load: Rc<dyn Fn()> = {
        let (list, versions, history) = (list.clone(), versions.clone(), history.clone());
        Rc::new(move || {
            let (list, versions, history) = (list.clone(), versions.clone(), history.clone());
            glib::spawn_future_local(async move {
                let found = gio::spawn_blocking(move || history.versions(300)).await.unwrap_or_default();
                while let Some(c) = list.first_child() {
                    list.remove(&c);
                }
                for v in &found {
                    let row = adw::ActionRow::builder()
                        .title(glib::markup_escape_text(v.name.as_deref().unwrap_or(tr("Automatic"))))
                        .subtitle(format!("{} · {}", ago(v.time), trf("{} files", &[&v.files_changed])))
                        .build();
                    if v.name.is_some() {
                        row.add_prefix(&gtk::Image::from_icon_name("starred-symbolic"));
                    } else {
                        row.add_prefix(&gtk::Image::from_icon_name("document-open-recent-symbolic"));
                    }
                    list.append(&row);
                }
                *versions.borrow_mut() = found;
            });
        })
    };
    load();

    // Selecting a version shows what changed between it and now.
    let (vs, h, buf, t, r) = (versions.clone(), history.clone(), diff_buf.clone(), title.clone(), restore.clone());
    list.connect_row_selected(move |_, row| {
        let Some(v) = row.and_then(|r| vs.borrow().get(r.index() as usize).cloned()) else {
            r.set_sensitive(false);
            return;
        };
        t.set_label(&format!("{} — {}", v.name.as_deref().unwrap_or(tr("Automatic")), ago(v.time)));
        r.set_sensitive(true);
        buf.set_text(tr("Comparing…"));
        let (h, buf) = (h.clone(), buf.clone());
        glib::spawn_future_local(async move {
            let id = v.id.clone();
            let text = gio::spawn_blocking(move || h.diff_with_current(&id)).await.unwrap_or_default();
            fill_diff(&buf, &text);
        });
    });

    // Restore, after confirming.
    let (vs, h, lst, dlg, cb) = (versions.clone(), history.clone(), list.clone(), dialog.clone(), on_restored.clone());
    restore.connect_clicked(move |button| {
        let Some(v) = lst.selected_row().and_then(|r| vs.borrow().get(r.index() as usize).cloned()) else { return };
        let confirm = adw::AlertDialog::builder()
            .heading(tr("Restore this version?"))
            .body(tr("The project files will be replaced by this version. Your current state is saved first, so you can go back to it from this same list."))
            .default_response("cancel")
            .close_response("cancel")
            .build();
        confirm.add_responses(&[("cancel", tr("Cancel")), ("restore", tr("Restore"))]);
        confirm.set_response_appearance("restore", adw::ResponseAppearance::Suggested);
        let (h, dlg, cb) = (h.clone(), dlg.clone(), cb.clone());
        confirm.connect_response(Some("restore"), move |_, _| {
            let (h, dlg, cb) = (h.clone(), dlg.clone(), cb.clone());
            let label = trf("Before restoring “{}”", &[&ago(v.time)]);
            let id = v.id.clone();
            glib::spawn_future_local(async move {
                let done = gio::spawn_blocking(move || h.restore(&id, &label)).await;
                if matches!(done, Ok(Ok(()))) {
                    dlg.close();
                    cb();
                }
            });
        });
        confirm.present(Some(button));
    });

    // Save a named version from here too.
    let (h, reload, dlg) = (history.clone(), load.clone(), dialog.clone());
    save.connect_clicked(move |_| {
        let (h, reload) = (h.clone(), reload.clone());
        ask_name(&dlg, move |name| {
            let (h, reload) = (h.clone(), reload.clone());
            glib::spawn_future_local(async move {
                let _ = gio::spawn_blocking(move || h.snapshot(Some(&name))).await;
                reload();
            });
        });
    });

    dialog.present(Some(parent));
}

/// Asks for a version name ("Before my tutor's review").
pub fn ask_name(parent: &impl IsA<gtk::Widget>, on_ok: impl Fn(String) + 'static) {
    let entry = gtk::Entry::builder().placeholder_text(tr("e.g. Sent to my supervisor")).activates_default(true).build();
    let dialog = adw::AlertDialog::builder().heading(tr("Save a version")).body(tr("Give it a name to find it easily later.")).extra_child(&entry).default_response("save").close_response("cancel").build();
    dialog.add_responses(&[("cancel", tr("Cancel")), ("save", tr("Save"))]);
    dialog.set_response_appearance("save", adw::ResponseAppearance::Suggested);
    let input = entry.clone();
    dialog.connect_response(Some("save"), move |_, _| {
        let name = input.text().trim().to_string();
        on_ok(if name.is_empty() { tr("Unnamed version").to_string() } else { name });
    });
    dialog.present(Some(parent));
    entry.grab_focus();
}

fn setup_tags(buf: &gtk::TextBuffer) {
    let (green, red) = match crate::theme::palette() {
        Some(p) => (p.green, p.red),
        None => ("#2ec27e".to_string(), "#e01b24".to_string()),
    };
    let table = buf.tag_table();
    let bg = |hex: &str| {
        let mut c = gtk::gdk::RGBA::parse(hex).unwrap_or(gtk::gdk::RGBA::BLACK);
        c.set_alpha(0.16);
        c
    };
    table.add(&gtk::TextTag::builder().name("add").paragraph_background_rgba(&bg(&green)).build());
    table.add(&gtk::TextTag::builder().name("del").paragraph_background_rgba(&bg(&red)).build());
    table.add(&gtk::TextTag::builder().name("hunk").foreground("#8f8f8f").build());
    table.add(&gtk::TextTag::builder().name("file").weight(800).scale(1.1).pixels_above_lines(14).pixels_below_lines(4).build());
}

/// Shows a unified diff in a friendlier way: one header per file, hunks dimmed,
/// added lines green and removed lines red. git's own headers are hidden.
fn fill_diff(buf: &gtk::TextBuffer, diff: &str) {
    buf.set_text("");
    if diff.trim().is_empty() {
        buf.set_text(tr("No differences: this version is identical to your current files."));
        return;
    }
    let mut end = buf.end_iter();
    for line in diff.lines() {
        let (text, tag) = if let Some(rest) = line.strip_prefix("diff --git a/") {
            (format!("{}\n", rest.split(" b/").next().unwrap_or(rest)), Some("file"))
        } else if line.starts_with("index ") || line.starts_with("--- ") || line.starts_with("+++ ") || line.starts_with("new file") || line.starts_with("deleted file") || line.starts_with("similarity") || line.starts_with("rename ") {
            continue;
        } else if line.starts_with("@@") {
            (format!("{line}\n"), Some("hunk"))
        } else if line.starts_with('+') {
            (format!("{line}\n"), Some("add"))
        } else if line.starts_with('-') {
            (format!("{line}\n"), Some("del"))
        } else {
            (format!("{line}\n"), None)
        };
        match tag {
            Some(t) => buf.insert_with_tags_by_name(&mut end, &text, &[t]),
            None => buf.insert(&mut end, &text),
        }
    }
}
