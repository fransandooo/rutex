//! Visor PDF.
//! - Poppler vive en un hilo propio: la interfaz nunca se bloquea renderizando.
//! - `gtk::ListView` virtualiza las páginas: solo existen widgets para las visibles
//!   y solo esas se renderizan.
//! - Caché de páginas renderizadas con límite de memoria (LRU).
//! - Al recompilar se sigue mostrando la versión anterior hasta que llega la nueva,
//!   así que no hay parpadeo y se conserva el scroll.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use gtk::prelude::*;
use gtk::{cairo, glib, graphene};

use crate::synctex::PdfPos;
use crate::i18n::{tr, trf};

/// Hilos de Poppler vivos. Hay que cerrarlos antes de salir: si el proceso
/// termina con un hilo aún liberando su documento, los destructores globales de
/// Poppler se ejecutan a la vez y el programa revienta (SIGSEGV en GlobalParams).
static WORKERS: std::sync::Mutex<Vec<(async_channel::Sender<Req>, std::thread::JoinHandle<()>)>> = std::sync::Mutex::new(Vec::new());

/// Cierra todos los hilos de PDF y espera a que suelten sus documentos.
pub fn shutdown_workers() {
    let workers = std::mem::take(&mut *WORKERS.lock().unwrap());
    for (tx, handle) in workers {
        tx.close();
        let _ = handle.join();
    }
}

const CACHE_LIMIT: usize = 128 * 1024 * 1024;
const MARGIN: f64 = 16.0;
/// Relleno vertical de cada fila de página (ver `.pdf-view > row` en style.css).
const ROW_PADDING: f64 = 20.0;
const MAX_RENDER_WIDTH: i32 = 6000;

enum Req {
    Load { generation: u64, path: PathBuf },
    Render { generation: u64, page: usize, width_px: i32, colors: Colors },
    /// Líneas de texto de una página (para marcar con precisión un destino).
    Lines { generation: u64, page: usize },
}

enum Resp {
    Loaded { generation: u64, sizes: Vec<(f64, f64)>, links: Vec<Vec<Link>> },
    Failed { generation: u64, error: String },
    Rendered { generation: u64, page: usize, width: i32, height: i32, stride: i32, data: Vec<u8>, colors: Colors },
    Lines { generation: u64, page: usize, lines: Vec<(f64, f64)> },
}

/// Qué indica la `y` de la marca roja: la línea base de un texto (SyncTeX) o
/// el ancla de un enlace, que hyperref coloca justo encima de la línea.
#[derive(Clone, Copy, Debug, PartialEq)]
enum MarkKind {
    Baseline,
    Anchor,
}

/// Búsqueda inversa: posición del clic y, si se pulsó un enlace interno, su
/// destino (para cuando el clic cae en texto generado, como el índice).
type InverseCallback = RefCell<Option<Box<dyn Fn(PdfPos, Option<PdfPos>)>>>;
type Callback<T> = RefCell<Option<Box<dyn Fn(T)>>>;

/// Enlace del PDF (índice, \\ref, \\cite, URL…), en puntos desde arriba a la izquierda.
#[derive(Clone, Debug)]
pub struct Link {
    x1: f64,
    y1: f64,
    x2: f64,
    y2: f64,
    target: LinkTarget,
}

#[derive(Clone, Debug)]
enum LinkTarget {
    /// Página (desde 0) y, si se conoce, la altura desde arriba en puntos.
    Page(usize, Option<f64>),
    Uri(String),
}

/// Colores del modo oscuro (fondo, texto) en RGB, o `None` para el PDF original.
type Colors = Option<[u8; 6]>;

struct Cached {
    generation: u64,
    colors: Colors,
    width: i32,
    height: i32,
    surface: cairo::ImageSurface,
    bytes: usize,
    used: u64,
}

#[derive(Default)]
struct State {
    /// Última carga pedida y la que se está mostrando.
    generation: u64,
    shown: u64,
    sizes: Vec<(f64, f64)>,
    max_w: f64,
    /// 1.0 = ajustar al ancho.
    zoom: f64,
    avail: f64,
    cache: HashMap<usize, Cached>,
    cache_bytes: usize,
    tick: u64,
    bound: HashMap<usize, glib::WeakRef<gtk::DrawingArea>>,
    requested: HashSet<(usize, i32, u64)>,
    marker: Option<(usize, f64, MarkKind)>,
    /// Líneas de texto (arriba, abajo) por página, pedidas bajo demanda.
    lines: HashMap<usize, Vec<(f64, f64)>>,
    links: Vec<Vec<Link>>,
    /// Posiciones de scroll a las que volver tras seguir un enlace.
    back: Vec<f64>,
    colors: Colors,
    marker_timeout: Option<glib::SourceId>,
}

impl State {
    fn scale(&self) -> f64 {
        if self.max_w <= 0.0 {
            return 1.0;
        }
        (self.avail - 2.0 * MARGIN).max(120.0) / self.max_w * self.zoom
    }

    fn evict(&mut self) {
        while self.cache_bytes > CACHE_LIMIT {
            let victim = self
                .cache
                .iter()
                .filter(|(p, _)| !self.bound.contains_key(p))
                .min_by_key(|(_, c)| c.used)
                .map(|(p, _)| *p);
            let Some(page) = victim else { break };
            if let Some(c) = self.cache.remove(&page) {
                self.cache_bytes -= c.bytes;
            }
        }
    }
}

pub struct PdfView {
    pub widget: gtk::Overlay,
    stack: gtk::Stack,
    status: adw::StatusPage,
    scrolled: gtk::ScrolledWindow,
    list: gtk::ListView,
    model: gtk::StringList,
    zoom_label: gtk::Label,
    page_label: gtk::Label,
    dark_btn: gtk::ToggleButton,
    full_btn: gtk::Button,
    back_btn: gtk::Button,
    on_dark: Callback<bool>,
    on_fullscreen: Callback<()>,
    state: RefCell<State>,
    tx: async_channel::Sender<Req>,
    on_inverse: InverseCallback,
    relayout_queued: std::cell::Cell<bool>,
}

impl PdfView {
    pub fn new() -> Rc<PdfView> {
        let (req_tx, req_rx) = async_channel::unbounded();
        let (resp_tx, resp_rx) = async_channel::unbounded();
        let handle = std::thread::Builder::new()
            .name("rutex-pdf".into())
            .spawn(move || worker(req_rx, resp_tx))
            .expect("could not start the PDF thread");
        WORKERS.lock().unwrap().push((req_tx.clone(), handle));

        let model = gtk::StringList::new(&[]);
        let factory = gtk::SignalListItemFactory::new();
        let list = gtk::ListView::new(Some(gtk::NoSelection::new(Some(model.clone()))), Some(factory.clone()));
        list.add_css_class("pdf-view");

        let scrolled = gtk::ScrolledWindow::builder().child(&list).hexpand(true).vexpand(true).build();

        let status = adw::StatusPage::builder()
            .icon_name("x-office-document-symbolic")
            .title(tr("No PDF yet"))
            .description(tr("It compiles automatically while you write.\nCtrl+Enter compiles now."))
            .build();
        status.add_css_class("compact");

        let stack = gtk::Stack::new();
        stack.add_named(&status, Some("empty"));
        stack.add_named(&scrolled, Some("pdf"));
        stack.set_visible_child_name("empty");

        let zoom_label = gtk::Label::builder().label("100%").width_chars(5).build();
        let page_label = gtk::Label::builder().label("–").width_chars(7).css_classes(["numeric"]).build();
        let zoom_out = gtk::Button::builder().icon_name("zoom-out-symbolic").tooltip_text(tr("Zoom out (Ctrl+-)")).build();
        let zoom_in = gtk::Button::builder().icon_name("zoom-in-symbolic").tooltip_text(tr("Zoom in (Ctrl++)")).build();
        let fit = gtk::Button::builder().icon_name("zoom-fit-best-symbolic").tooltip_text(tr("Fit width (Ctrl+0)")).build();
        let controls = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(2)
            .halign(gtk::Align::End)
            .valign(gtk::Align::End)
            .margin_end(18)
            .margin_bottom(18)
            .css_classes(["osd", "toolbar", "pdf-controls"])
            .build();
        controls.append(&page_label);
        controls.append(&gtk::Separator::new(gtk::Orientation::Vertical));
        controls.append(&zoom_out);
        controls.append(&zoom_label);
        controls.append(&zoom_in);
        controls.append(&fit);
        controls.append(&gtk::Separator::new(gtk::Orientation::Vertical));
        let dark_btn = gtk::ToggleButton::builder()
            .icon_name("weather-clear-night-symbolic")
            .tooltip_text(tr("Dark PDF"))
            .active(crate::config::get().pdf_dark)
            .build();
        let full_btn = gtk::Button::builder().icon_name("view-fullscreen-symbolic").tooltip_text(tr("Fullscreen PDF (F11)")).build();
        controls.append(&dark_btn);
        let back_btn = gtk::Button::builder()
            .icon_name("go-previous-symbolic")
            .tooltip_text(tr("Back to where you were"))
            .visible(false)
            .build();
        controls.prepend(&back_btn);
        controls.append(&full_btn);

        let widget = gtk::Overlay::builder().child(&stack).build();
        widget.add_overlay(&controls);

        let this = Rc::new(PdfView {
            widget,
            stack,
            status,
            scrolled,
            list,
            model,
            zoom_label,
            page_label,
            dark_btn,
            full_btn,
            back_btn,
            on_dark: RefCell::default(),
            on_fullscreen: RefCell::default(),
            state: RefCell::new(State { zoom: 1.0, colors: current_colors(), ..Default::default() }),
            tx: req_tx,
            on_inverse: RefCell::default(),
            relayout_queued: std::cell::Cell::new(false),
        });

        this.setup_factory(&factory);

        let weak = Rc::downgrade(&this);
        glib::spawn_future_local(async move {
            while let Ok(resp) = resp_rx.recv().await {
                let Some(this) = weak.upgrade() else { break };
                this.handle(resp);
            }
        });

        let weak = Rc::downgrade(&this);
        // Cambiar tamaños durante la asignación de GTK provoca avisos y saltos:
        // el reajuste se hace en el siguiente ciclo libre.
        this.scrolled.hadjustment().connect_page_size_notify(move |_| {
            let Some(this) = weak.upgrade() else { return };
            if this.relayout_queued.replace(true) {
                return;
            }
            let weak = Rc::downgrade(&this);
            glib::idle_add_local_once(move || {
                let Some(this) = weak.upgrade() else { return };
                this.relayout_queued.set(false);
                let width = this.scrolled.hadjustment().page_size();
                if (this.state.borrow().avail - width).abs() > 0.5 {
                    this.state.borrow_mut().avail = width;
                    this.relayout();
                }
            });
        });

        let weak = Rc::downgrade(&this);
        this.back_btn.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                this.go_back();
            }
        });
        // Botón «atrás» del ratón.
        let mouse_back = gtk::GestureClick::builder().button(8).build();
        let weak = Rc::downgrade(&this);
        mouse_back.connect_pressed(move |_, _, _, _| {
            if let Some(this) = weak.upgrade() {
                this.go_back();
            }
        });
        this.scrolled.add_controller(mouse_back);

        let weak = Rc::downgrade(&this);
        this.dark_btn.connect_toggled(move |b| {
            if let Some(this) = weak.upgrade()
                && let Some(cb) = this.on_dark.borrow().as_ref()
            {
                cb(b.is_active());
            }
        });
        let weak = Rc::downgrade(&this);
        this.full_btn.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade()
                && let Some(cb) = this.on_fullscreen.borrow().as_ref()
            {
                cb(());
            }
        });

        let weak = Rc::downgrade(&this);
        this.scrolled.vadjustment().connect_value_changed(move |_| {
            if let Some(this) = weak.upgrade() {
                this.update_page_label();
            }
        });

        for (button, step) in [(&zoom_in, 1), (&zoom_out, -1), (&fit, 0)] {
            let weak = Rc::downgrade(&this);
            button.connect_clicked(move |_| {
                if let Some(this) = weak.upgrade() {
                    this.zoom(step);
                }
            });
        }

        // Ctrl + rueda = zoom.
        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        scroll.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(&this);
        scroll.connect_scroll(move |ctrl, _, dy| {
            let Some(this) = weak.upgrade() else { return glib::Propagation::Proceed };
            if !ctrl.current_event_state().contains(gtk::gdk::ModifierType::CONTROL_MASK) {
                return glib::Propagation::Proceed;
            }
            this.zoom(if dy < 0.0 { 1 } else { -1 });
            glib::Propagation::Stop
        });
        this.scrolled.add_controller(scroll);

        this
    }

    /// Llamado al hacer doble clic en el PDF (búsqueda inversa).
    pub fn connect_inverse(&self, f: impl Fn(PdfPos, Option<PdfPos>) + 'static) {
        *self.on_inverse.borrow_mut() = Some(Box::new(f));
    }

    pub fn connect_dark_toggled(&self, f: impl Fn(bool) + 'static) {
        *self.on_dark.borrow_mut() = Some(Box::new(f));
    }

    pub fn connect_fullscreen(&self, f: impl Fn(()) + 'static) {
        *self.on_fullscreen.borrow_mut() = Some(Box::new(f));
    }

    /// Sincroniza el botón de modo oscuro sin volver a avisar.
    pub fn set_dark_button(&self, on: bool) {
        if self.dark_btn.is_active() != on {
            let cb = self.on_dark.borrow_mut().take();
            self.dark_btn.set_active(on);
            *self.on_dark.borrow_mut() = cb;
        }
    }

    pub fn set_fullscreen_button(&self, on: bool) {
        self.full_btn.set_icon_name(if on { "view-restore-symbolic" } else { "view-fullscreen-symbolic" });
        self.full_btn.set_tooltip_text(Some(if on { tr("Back to the editor (Esc)") } else { tr("Fullscreen PDF (F11)") }));
    }

    pub fn has_document(&self) -> bool {
        self.state.borrow().shown > 0
    }

    /// Carga (o recarga) el PDF. La versión actual sigue visible mientras tanto.
    pub fn load(&self, path: PathBuf) {
        let generation = {
            let mut st = self.state.borrow_mut();
            st.generation += 1;
            st.generation
        };
        let _ = self.tx.try_send(Req::Load { generation, path });
    }

    /// Repinta las páginas (cambio de tema o del modo oscuro del PDF).
    pub fn refresh_colors(&self) {
        let mut st = self.state.borrow_mut();
        st.colors = current_colors();
        st.requested.clear();
        for weak in st.bound.values() {
            if let Some(da) = weak.upgrade() {
                da.queue_draw();
            }
        }
    }

    /// "3 / 12": la página que ocupa el tercio superior de la vista, calculada
    /// con las alturas conocidas de las páginas (los widgets de la lista se
    /// reciclan y su posición no es fiable fuera de pantalla).
    fn update_page_label(&self) {
        let st = self.state.borrow();
        let total = st.sizes.len();
        if total == 0 {
            self.page_label.set_label("–");
            return;
        }
        let vadj = self.scrolled.vadjustment();
        let probe = vadj.value() + vadj.page_size() * 0.33;
        let scale = st.scale();
        let mut bottom = 0.0;
        let mut current = total - 1;
        for (i, (_, h)) in st.sizes.iter().enumerate() {
            bottom += h * scale + ROW_PADDING;
            if bottom > probe {
                current = i;
                break;
            }
        }
        self.page_label.set_label(&format!("{} / {}", current + 1, total));
    }

    /// 1 = acercar, -1 = alejar, 0 = ajustar al ancho.
    pub fn zoom(&self, step: i32) {
        let vadj = self.scrolled.vadjustment();
        let ratio = if vadj.upper() > 0.0 { (vadj.value() + vadj.page_size() / 2.0) / vadj.upper() } else { 0.0 };
        {
            let mut st = self.state.borrow_mut();
            st.zoom = match step {
                0 => 1.0,
                s if s > 0 => (st.zoom * 1.15).min(5.0),
                _ => (st.zoom / 1.15).max(0.25),
            };
        }
        self.relayout();
        let scrolled = self.scrolled.clone();
        glib::idle_add_local_once(move || {
            let vadj = scrolled.vadjustment();
            vadj.set_value(ratio * vadj.upper() - vadj.page_size() / 2.0);
        });
    }

    /// Lleva la vista a una posición (búsqueda directa) y la marca un momento.
    pub fn reveal(self: &Rc<Self>, pos: PdfPos) {
        self.reveal_at(pos, 0.35, MarkKind::Baseline);
    }

    /// Como `reveal`, dejando la posición a `frac` de la altura de la vista.
    fn reveal_at(self: &Rc<Self>, pos: PdfPos, frac: f64, kind: MarkKind) {
        if pos.page >= self.model.n_items() as usize {
            return;
        }
        {
            let mut st = self.state.borrow_mut();
            st.marker = Some((pos.page, pos.y, kind));
            if !st.lines.contains_key(&pos.page) {
                let _ = self.tx.try_send(Req::Lines { generation: st.shown, page: pos.page });
            }
            if let Some(id) = st.marker_timeout.take() {
                id.remove();
            }
        }
        self.list.scroll_to(pos.page as u32, gtk::ListScrollFlags::NONE, None);

        let weak = Rc::downgrade(self);
        glib::timeout_add_local_once(Duration::from_millis(60), move || {
            let Some(this) = weak.upgrade() else { return };
            let (da, scale) = {
                let st = this.state.borrow();
                (st.bound.get(&pos.page).and_then(|w| w.upgrade()), st.scale())
            };
            if let Some(da) = da {
                let y = (pos.y * scale) as f32;
                if let Some(p) = da.compute_point(&this.scrolled, &graphene::Point::new(0.0, y)) {
                    let vadj = this.scrolled.vadjustment();
                    vadj.set_value(vadj.value() + p.y() as f64 - vadj.page_size() * frac);
                }
                da.queue_draw();
            }
        });

        let weak = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(Duration::from_millis(1600), move || {
            let Some(this) = weak.upgrade() else { return };
            let mut st = this.state.borrow_mut();
            st.marker_timeout = None;
            if let Some((page, _, _)) = st.marker.take()
                && let Some(da) = st.bound.get(&page).and_then(|w| w.upgrade()) {
                    da.queue_draw();
                }
        });
        self.state.borrow_mut().marker_timeout = Some(id);
    }

    fn link_at(&self, page: usize, x: f64, y: f64) -> Option<LinkTarget> {
        let st = self.state.borrow();
        st.links.get(page)?.iter().find(|l| x >= l.x1 && x <= l.x2 && y >= l.y1 && y <= l.y2).map(|l| l.target.clone())
    }

    fn follow(self: &Rc<Self>, target: LinkTarget, widget: &gtk::Widget) {
        match target {
            LinkTarget::Page(page, y) => {
                let value = self.scrolled.vadjustment().value();
                {
                    let mut st = self.state.borrow_mut();
                    st.back.push(value);
                    if st.back.len() > 50 {
                        st.back.remove(0);
                    }
                }
                self.back_btn.set_visible(true);
                self.reveal_at(PdfPos { page, x: 0.0, y: y.unwrap_or(0.0) }, 0.08, MarkKind::Anchor);
            }
            LinkTarget::Uri(uri) => {
                let window = widget.root().and_downcast::<gtk::Window>();
                gtk::UriLauncher::new(&uri).launch(window.as_ref(), gtk::gio::Cancellable::NONE, |_| {});
            }
        }
    }

    fn go_back(&self) {
        let value = self.state.borrow_mut().back.pop();
        if let Some(v) = value {
            self.scrolled.vadjustment().set_value(v);
        }
        self.back_btn.set_visible(!self.state.borrow().back.is_empty());
    }

    fn setup_factory(self: &Rc<Self>, factory: &gtk::SignalListItemFactory) {
        let weak = Rc::downgrade(self);
        factory.connect_setup(move |_, obj| {
            let item = obj.downcast_ref::<gtk::ListItem>().unwrap();
            item.set_activatable(false);
            item.set_focusable(false);
            let da = gtk::DrawingArea::builder().halign(gtk::Align::Center).css_classes(["pdf-page"]).build();

            let w = weak.clone();
            da.set_draw_func(move |da, cr, width, height| {
                if let (Some(this), Some(page)) = (w.upgrade(), page_of(da)) {
                    this.draw_page(da, page, cr, width, height);
                }
            });

            // Clic en un enlace: seguirlo. Doble clic en texto o Ctrl+clic en
            // cualquier sitio: ir al código (SyncTeX).
            let click = gtk::GestureClick::new();
            let w = weak.clone();
            click.connect_pressed(move |gesture, n_press, x, y| {
                let Some(this) = w.upgrade() else { return };
                let Some(da) = gesture.widget() else { return };
                let Some(page) = page_of(&da) else { return };
                let scale = this.state.borrow().scale();
                let (px, py) = (x / scale, y / scale);
                let ctrl = gesture.current_event_state().contains(gtk::gdk::ModifierType::CONTROL_MASK);
                let link = this.link_at(page, px, py);
                if !ctrl && n_press == 1 && let Some(target) = link {
                    this.follow(target, &da);
                } else if ctrl || (n_press == 2 && link.is_none()) {
                    // Un poco por debajo del destino, para caer en la línea del título.
                    let fallback = match link {
                        Some(LinkTarget::Page(p, y)) => Some(PdfPos { page: p, x: 150.0, y: y.unwrap_or(0.0) + 12.0 }),
                        _ => None,
                    };
                    if let Some(cb) = this.on_inverse.borrow().as_ref() {
                        cb(PdfPos { page, x: px, y: py }, fallback);
                    }
                }
            });
            da.add_controller(click);

            // Mano sobre los enlaces y la URL como tooltip.
            let motion = gtk::EventControllerMotion::new();
            let w = weak.clone();
            motion.connect_motion(move |ctrl, x, y| {
                let Some(this) = w.upgrade() else { return };
                let Some(da) = ctrl.widget() else { return };
                let Some(page) = page_of(&da) else { return };
                let scale = this.state.borrow().scale();
                match this.link_at(page, x / scale, y / scale) {
                    Some(target) => {
                        da.set_cursor_from_name(Some("pointer"));
                        da.set_tooltip_text(match &target {
                            LinkTarget::Uri(u) => Some(u.as_str()),
                            LinkTarget::Page(..) => None,
                        });
                    }
                    None => {
                        da.set_cursor_from_name(None);
                        da.set_tooltip_text(None);
                    }
                }
            });
            da.add_controller(motion);
            item.set_child(Some(&da));
        });

        let weak = Rc::downgrade(self);
        factory.connect_bind(move |_, obj| {
            let Some(this) = weak.upgrade() else { return };
            let item = obj.downcast_ref::<gtk::ListItem>().unwrap();
            let da = item.child().and_downcast::<gtk::DrawingArea>().unwrap();
            let page = item.position() as usize;
            unsafe { da.set_data::<usize>("rutex-page", page) };
            let mut st = this.state.borrow_mut();
            size_widget(&st, &da, page);
            st.bound.insert(page, da.downgrade());
            da.queue_draw();
        });

        let weak = Rc::downgrade(self);
        factory.connect_unbind(move |_, obj| {
            let Some(this) = weak.upgrade() else { return };
            let item = obj.downcast_ref::<gtk::ListItem>().unwrap();
            if let Some(da) = item.child().and_downcast::<gtk::DrawingArea>()
                && let Some(page) = page_of(&da) {
                    let mut st = this.state.borrow_mut();
                    if st.bound.get(&page).and_then(|w| w.upgrade()).as_ref() == Some(&da) {
                        st.bound.remove(&page);
                    }
                }
        });
    }

    fn relayout(&self) {
        let st = self.state.borrow();
        for (page, weak) in &st.bound {
            if let Some(da) = weak.upgrade() {
                size_widget(&st, &da, *page);
                da.queue_draw();
            }
        }
        self.zoom_label.set_label(&format!("{:.0}%", st.zoom * 100.0));
    }

    fn draw_page(&self, da: &gtk::DrawingArea, page: usize, cr: &cairo::Context, width: i32, height: i32) {
        let (w, h) = (width as f64, height as f64);
        match self.state.borrow().colors {
            Some(c) => cr.set_source_rgb(c[0] as f64 / 255.0, c[1] as f64 / 255.0, c[2] as f64 / 255.0),
            None => cr.set_source_rgb(1.0, 1.0, 1.0),
        }
        cr.paint().ok();

        let device_scale = da.native().and_then(|n| n.surface()).map_or(da.scale_factor() as f64, |s| s.scale());
        let target = ((w * device_scale).round() as i32).clamp(1, MAX_RENDER_WIDTH);

        let mut st = self.state.borrow_mut();
        st.tick += 1;
        let tick = st.tick;
        let shown = st.shown;
        let colors = st.colors;
        let mut needs_render = true;
        if let Some(c) = st.cache.get_mut(&page) {
            c.used = tick;
            cr.save().ok();
            cr.scale(w / c.width as f64, h / c.height as f64);
            if cr.set_source_surface(&c.surface, 0.0, 0.0).is_ok() {
                cr.source().set_filter(cairo::Filter::Good);
                cr.paint().ok();
            }
            cr.restore().ok();
            needs_render = c.generation != shown || (c.width - target).abs() > 1 || c.colors != colors;
        }

        if let Some((mpage, my, kind)) = st.marker
            && mpage == page
        {
            let (top, bottom) = marker_band(st.lines.get(&page).map(Vec::as_slice).unwrap_or(&[]), my, kind);
            let s = st.scale();
            cr.set_source_rgba(0.95, 0.3, 0.2, 0.3);
            cr.rectangle(0.0, (top - 2.0) * s, w, (bottom - top + 4.0) * s);
            cr.fill().ok();
        }

        if needs_render && shown > 0 && st.requested.insert((page, target, shown)) {
            let _ = self.tx.try_send(Req::Render { generation: shown, page, width_px: target, colors: st.colors });
        }
    }

    fn handle(&self, resp: Resp) {
        match resp {
            Resp::Loaded { generation, sizes, links } => {
                let n_old = self.model.n_items();
                {
                    let mut st = self.state.borrow_mut();
                    if generation != st.generation {
                        return;
                    }
                    st.shown = generation;
                    st.links = links;
                    st.lines.clear();
                    st.max_w = sizes.iter().map(|s| s.0).fold(0.0, f64::max);
                    st.requested.clear();
                    let n = sizes.len();
                    st.sizes = sizes;
                    let stale: Vec<usize> = st.cache.keys().copied().filter(|p| *p >= n).collect();
                    for p in stale {
                        let c = st.cache.remove(&p).unwrap();
                        st.cache_bytes -= c.bytes;
                    }
                    if st.avail <= 0.0 {
                        st.avail = self.scrolled.width() as f64;
                    }
                }
                let n = self.state.borrow().sizes.len() as u32;
                if n > n_old {
                    let empty: Vec<&str> = vec![""; (n - n_old) as usize];
                    self.model.splice(n_old, 0, &empty);
                } else if n < n_old {
                    self.model.splice(n, n_old - n, &[]);
                }
                self.relayout();
                self.stack.set_visible_child_name(if n > 0 { "pdf" } else { "empty" });
                self.update_page_label();
            }
            Resp::Failed { generation, error } => {
                if generation == self.state.borrow().generation && !self.has_document() {
                    self.status.set_description(Some(&trf("Could not open the PDF:\n{}", &[&error])));
                    self.stack.set_visible_child_name("empty");
                }
            }
            Resp::Lines { generation, page, lines } => {
                let mut st = self.state.borrow_mut();
                if generation != st.shown {
                    return;
                }
                st.lines.insert(page, lines);
                if let Some(da) = st.bound.get(&page).and_then(|w| w.upgrade()) {
                    da.queue_draw();
                }
            }
            Resp::Rendered { generation, page, width, height, stride, data, colors } => {
                let mut st = self.state.borrow_mut();
                st.requested.retain(|r| !(r.0 == page && r.2 == generation));
                if generation != st.shown {
                    return;
                }
                let bytes = data.len();
                let Ok(surface) = cairo::ImageSurface::create_for_data(data, cairo::Format::ARgb32, width, height, stride) else {
                    return;
                };
                st.tick += 1;
                let used = st.tick;
                if let Some(old) = st.cache.insert(page, Cached { generation, colors, width, height, surface, bytes, used }) {
                    st.cache_bytes -= old.bytes;
                }
                st.cache_bytes += bytes;
                st.evict();
                if let Some(da) = st.bound.get(&page).and_then(|w| w.upgrade()) {
                    da.queue_draw();
                }
            }
        }
    }
}

fn page_of(widget: &impl IsA<gtk::Widget>) -> Option<usize> {
    unsafe { widget.as_ref().data::<usize>("rutex-page").map(|p| *p.as_ref()) }
}

fn size_widget(st: &State, da: &gtk::DrawingArea, page: usize) {
    let (w, h) = st.sizes.get(page).copied().unwrap_or((595.0, 842.0));
    let s = st.scale();
    da.set_content_width((w * s).round() as i32);
    da.set_content_height((h * s).round() as i32);
}

/// Hilo de Poppler. Atiende primero la carga más reciente y después los
/// renders, empezando por el último pedido (lo que el usuario está mirando ahora).
fn worker(rx: async_channel::Receiver<Req>, tx: async_channel::Sender<Resp>) {
    let mut doc: Option<(u64, poppler::Document)> = None;
    let mut pending: Vec<Req> = Vec::new();
    loop {
        if pending.is_empty() {
            match rx.recv_blocking() {
                Ok(r) => pending.push(r),
                Err(_) => return,
            }
        }
        while let Ok(r) = rx.try_recv() {
            pending.push(r);
        }

        if let Some(i) = pending.iter().rposition(|r| matches!(r, Req::Load { .. })) {
            let Req::Load { generation, path } = pending.remove(i) else { unreachable!() };
            pending.retain(|r| matches!(r, Req::Render { generation: g, .. } if *g >= generation));
            match open(&path) {
                Ok(d) => {
                    let sizes: Vec<(f64, f64)> = (0..d.n_pages()).filter_map(|i| d.page(i)).map(|p| p.size()).collect();
                    let links = (0..d.n_pages()).map(|i| page_links(&d, i, &sizes)).collect();
                    doc = Some((generation, d));
                    if tx.send_blocking(Resp::Loaded { generation, sizes, links }).is_err() {
                        return;
                    }
                }
                Err(error) => {
                    if tx.send_blocking(Resp::Failed { generation, error }).is_err() {
                        return;
                    }
                }
            }
            continue;
        }

        // Las peticiones de líneas son baratas y la marca roja las espera: primero.
        if let Some(i) = pending.iter().position(|r| matches!(r, Req::Lines { .. })) {
            let Req::Lines { generation, page } = pending.remove(i) else { unreachable!() };
            if let Some((doc_gen, d)) = doc.as_ref()
                && *doc_gen == generation
                && let Some(p) = d.page(page as i32)
            {
                let resp = Resp::Lines { generation, page, lines: text_lines(&p) };
                if tx.send_blocking(resp).is_err() {
                    return;
                }
            }
            continue;
        }
        let Some(Req::Render { generation, page, width_px, colors }) = pending.pop() else { continue };
        pending.retain(|r| !matches!(r, Req::Render { page: p, .. } if *p == page));
        let Some((doc_gen, d)) = doc.as_ref() else { continue };
        if *doc_gen != generation {
            continue;
        }
        if let Some((width, height, stride, mut data)) = render(d, page, width_px) {
            if let Some(c) = colors {
                recolor(&mut data, c);
            }
            let resp = Resp::Rendered { generation, page, width, height, stride, data, colors };
            if tx.send_blocking(resp).is_err() {
                return;
            }
        }
    }
}

fn open(path: &PathBuf) -> Result<poppler::Document, String> {
    let data = std::fs::read(path).map_err(|e| e.to_string())?;
    poppler::Document::from_bytes(&glib::Bytes::from_owned(data), None).map_err(|e| e.to_string())
}

fn render(doc: &poppler::Document, index: usize, width: i32) -> Option<(i32, i32, i32, Vec<u8>)> {
    let page = doc.page(index as i32)?;
    let (pw, ph) = page.size();
    let scale = width as f64 / pw;
    let height = ((ph * scale).round() as i32).max(1);
    let mut surface = cairo::ImageSurface::create(cairo::Format::ARgb32, width, height).ok()?;
    {
        let cr = cairo::Context::new(&surface).ok()?;
        cr.set_source_rgb(1.0, 1.0, 1.0);
        cr.paint().ok()?;
        cr.scale(scale, scale);
        page.render(&cr);
    }
    surface.flush();
    let stride = surface.stride();
    let data = surface.data().ok()?.to_vec();
    Some((width, height, stride, data))
}

/// Modo oscuro: blanco → fondo del tema, negro → texto del tema (como zathura).
fn recolor(data: &mut [u8], c: [u8; 6]) {
    let (bg, fg) = ([c[0] as f32, c[1] as f32, c[2] as f32], [c[3] as f32, c[4] as f32, c[5] as f32]);
    // ARGB32 en memoria little-endian: B, G, R, A (opaco: pintamos fondo blanco).
    for px in data.chunks_exact_mut(4) {
        let lum = (0.114 * px[0] as f32 + 0.587 * px[1] as f32 + 0.299 * px[2] as f32) / 255.0;
        px[2] = (fg[0] + (bg[0] - fg[0]) * lum) as u8;
        px[1] = (fg[1] + (bg[1] - fg[1]) * lum) as u8;
        px[0] = (fg[2] + (bg[2] - fg[2]) * lum) as u8;
    }
}

/// Colores para el modo oscuro según preferencias y tema actual.
fn current_colors() -> Colors {
    if !crate::config::get().pdf_dark {
        return None;
    }
    let (bg, fg) = match crate::theme::palette() {
        Some(p) => (p.bg, p.fg),
        None => ("#1e1e1e".to_string(), "#deddda".to_string()),
    };
    let parse = |hex: &str, i: usize| u8::from_str_radix(&hex[1 + i * 2..3 + i * 2], 16).unwrap_or(0);
    Some([parse(&bg, 0), parse(&bg, 1), parse(&bg, 2), parse(&fg, 0), parse(&fg, 1), parse(&fg, 2)])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Compila un PDF con hyperref y comprueba que se leen sus enlaces.
    /// Necesita TeX Live: `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn reads_pdf_links() {
        let dir = std::env::temp_dir().join(format!("rutex-links-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("main.tex"),
            "\\documentclass{article}\\usepackage{hyperref}\\begin{document}\\tableofcontents\\newpage\\section{Uno}\\label{s}See \\url{https://example.org}.\\newpage\\section{Dos}Back to \\ref{s}.\\end{document}",
        )
        .unwrap();
        let ok = std::process::Command::new("latexmk").args(["-pdf", "-interaction=nonstopmode", "main.tex"]).current_dir(&dir).output().unwrap();
        assert!(ok.status.success());
        let doc = open(&dir.join("main.pdf")).unwrap();
        let sizes: Vec<(f64, f64)> = (0..doc.n_pages()).filter_map(|i| doc.page(i)).map(|p| p.size()).collect();
        let toc = page_links(&doc, 0, &sizes);
        // El índice enlaza a la sección 1 (página 2) y a la 2 (página 3), cerca de arriba.
        let mut pages: Vec<usize> = toc.iter().filter_map(|l| match l.target { LinkTarget::Page(p, _) => Some(p), _ => None }).collect();
        pages.sort();
        assert_eq!(pages, vec![1, 2]);
        assert!(toc.iter().all(|l| l.y1 < l.y2 && l.y2 < sizes[0].1 / 2.0), "los enlaces del índice están arriba: {toc:?}");
        if let LinkTarget::Page(_, Some(y)) = toc[0].target {
            assert!(y < 200.0, "el destino de una sección está en la parte alta de la página: {y}");
        }
        let uri = page_links(&doc, 1, &sizes);
        assert!(uri.iter().any(|l| matches!(&l.target, LinkTarget::Uri(u) if u == "https://example.org")));
        // Líneas de texto de la página de la sección 1: título y párrafo, ordenadas.
        let lines = text_lines(&doc.page(1).unwrap());
        assert!(lines.len() >= 2 && lines.windows(2).all(|w| w[0].0 < w[1].0), "{lines:?}");
        let back = page_links(&doc, 2, &sizes);
        assert!(back.iter().any(|l| matches!(l.target, LinkTarget::Page(1, _))));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn marks_the_right_line() {
        // Líneas reales de la bibliografía de la demo: [3] y [4].
        let lines = merge_lines(vec![(240.6, 254.8), (243.5, 253.1), (254.8, 254.8), (258.6, 272.7), (261.4, 271.0), (272.6, 286.7)]);
        assert_eq!(lines, vec![(240.6, 254.8), (258.6, 272.7), (272.6, 286.7)]);
        // El ancla de [4] está en 258.2, justo encima: se marca [4], no [3].
        assert_eq!(marker_band(&lines, 258.2, MarkKind::Anchor), (258.6, 272.7));
        // Una línea base dentro de [3] marca [3].
        assert_eq!(marker_band(&lines, 252.0, MarkKind::Baseline), (240.6, 254.8));
    }

    #[test]
    fn recolors_white_to_background() {
        // Un píxel blanco y uno negro (B, G, R, A).
        let mut data = vec![255, 255, 255, 255, 0, 0, 0, 255];
        recolor(&mut data, [0x19, 0x18, 0x20, 0xee, 0xcc, 0xa2]);
        assert_eq!(&data[..3], &[0x20, 0x18, 0x19]);
        assert_eq!(&data[4..7], &[0xa2, 0xcc, 0xee]);
    }
}

/// Enlaces de una página. Las bindings de Rust no exponen los campos de
/// `PopplerLinkMapping`/`PopplerDest`, así que se leen de la API en C. Poppler da
/// las áreas con el origen abajo a la izquierda; aquí se pasan a desde arriba.
fn page_links(doc: &poppler::Document, index: i32, sizes: &[(f64, f64)]) -> Vec<Link> {
    use gtk::glib::translate::ToGlibPtr;
    use poppler::ffi;
    use std::ffi::CStr;

    let Some(page) = doc.page(index) else { return Vec::new() };
    let height = sizes.get(index as usize).map_or(0.0, |s| s.1);
    let mut out = Vec::new();
    for mapping in page.link_mapping() {
        let raw: *const ffi::PopplerLinkMapping = mapping.to_glib_none().0;
        // SAFETY: `mapping` vive durante todo el cuerpo del bucle y poppler
        // garantiza que `action` apunta a una acción válida o es nulo.
        let target = unsafe {
            let action = (*raw).action;
            if action.is_null() {
                continue;
            }
            match (*action).type_ {
                ffi::POPPLER_ACTION_GOTO_DEST => resolve_dest(doc, (*action).goto_dest.dest, sizes),
                ffi::POPPLER_ACTION_URI if !(*action).uri.uri.is_null() => {
                    Some(LinkTarget::Uri(CStr::from_ptr((*action).uri.uri).to_string_lossy().into_owned()))
                }
                _ => None,
            }
        };
        let Some(target) = target else { continue };
        let area = unsafe { (*raw).area };
        out.push(Link { x1: area.x1, x2: area.x2, y1: height - area.y2, y2: height - area.y1, target });
    }
    out
}

/// Líneas de texto de la página como (arriba, abajo) en puntos desde arriba,
/// a partir de los rectángulos de cada carácter (`poppler_page_get_text_layout`,
/// que poppler-rs no expone).
fn text_lines(page: &poppler::Page) -> Vec<(f64, f64)> {
    use gtk::glib::translate::ToGlibPtr;
    use poppler::ffi;

    let mut rects: *mut ffi::PopplerRectangle = std::ptr::null_mut();
    let mut n: std::os::raw::c_uint = 0;
    // SAFETY: poppler reserva `rects` con g_malloc y nos pasa su propiedad.
    let spans: Vec<(f64, f64)> = unsafe {
        if ffi::poppler_page_get_text_layout(page.to_glib_none().0, &mut rects, &mut n) == 0 || rects.is_null() {
            return Vec::new();
        }
        let spans = std::slice::from_raw_parts(rects, n as usize).iter().map(|r| (r.y1, r.y2)).collect();
        glib::ffi::g_free(rects.cast());
        spans
    };
    merge_lines(spans)
}

/// Junta los rectángulos de caracteres en líneas (se solapan en vertical).
fn merge_lines(mut spans: Vec<(f64, f64)>) -> Vec<(f64, f64)> {
    spans.retain(|(t, b)| b - t > 0.5);
    spans.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut lines: Vec<(f64, f64)> = Vec::new();
    for (t, b) in spans {
        if let Some(last) = lines.last_mut() {
            let overlap = last.1.min(b) - last.0.max(t);
            if overlap > 0.5 * (b - t).min(last.1 - last.0) {
                last.0 = last.0.min(t);
                last.1 = last.1.max(b);
                continue;
            }
        }
        lines.push((t, b));
    }
    lines
}

/// Franja (arriba, abajo) a resaltar. Un ancla de enlace está justo encima de
/// su línea: se marca la primera línea que empieza a su altura o debajo. Una
/// línea base (SyncTeX) cae dentro de su línea. Sin datos, se estima.
fn marker_band(lines: &[(f64, f64)], y: f64, kind: MarkKind) -> (f64, f64) {
    let found = match kind {
        MarkKind::Anchor => lines.iter().find(|(t, _)| *t >= y - 3.0),
        MarkKind::Baseline => lines.iter().filter(|(t, b)| *t - 1.0 <= y && y <= *b + 2.0).min_by(|a, b| (a.1 - y).abs().total_cmp(&(b.1 - y).abs())),
    };
    match (found, kind) {
        (Some(&(t, b)), _) => (t, b),
        (None, MarkKind::Anchor) => (y, y + 12.0),
        (None, MarkKind::Baseline) => (y - 10.0, y + 2.0),
    }
}

/// Destino de un enlace interno: página (desde 0) y altura desde arriba.
unsafe fn resolve_dest(doc: &poppler::Document, dest: *mut poppler::ffi::PopplerDest, sizes: &[(f64, f64)]) -> Option<LinkTarget> {
    use gtk::glib::translate::ToGlibPtr;
    use poppler::ffi;
    use std::ffi::CStr;

    if dest.is_null() {
        return None;
    }
    // Los destinos con nombre (los de hyperref) se buscan en el documento.
    let named;
    let d: *const ffi::PopplerDest = unsafe {
        if (*dest).type_ == ffi::POPPLER_DEST_NAMED {
            if (*dest).named_dest.is_null() {
                return None;
            }
            let name = CStr::from_ptr((*dest).named_dest).to_string_lossy().into_owned();
            named = doc.find_dest(&name)?;
            named.to_glib_none().0
        } else {
            dest
        }
    };
    let (page_num, top) = unsafe { ((*d).page_num, (*d).top) };
    let page = usize::try_from(page_num - 1).ok()?;
    let height = sizes.get(page)?.1;
    let y = (top > 0.0 && top <= height).then_some(height - top);
    Some(LinkTarget::Page(page, y))
}
