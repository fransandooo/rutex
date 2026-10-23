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

const CACHE_LIMIT: usize = 128 * 1024 * 1024;
const MARGIN: f64 = 16.0;
const MAX_RENDER_WIDTH: i32 = 6000;

enum Req {
    Load { generation: u64, path: PathBuf },
    Render { generation: u64, page: usize, width_px: i32 },
}

enum Resp {
    Loaded { generation: u64, sizes: Vec<(f64, f64)> },
    Failed { generation: u64, error: String },
    Rendered { generation: u64, page: usize, width: i32, height: i32, stride: i32, data: Vec<u8> },
}

type InverseCallback = RefCell<Option<Box<dyn Fn(PdfPos)>>>;

struct Cached {
    generation: u64,
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
    marker: Option<(usize, f64)>,
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
    state: RefCell<State>,
    tx: async_channel::Sender<Req>,
    on_inverse: InverseCallback,
    relayout_queued: std::cell::Cell<bool>,
}

impl PdfView {
    pub fn new() -> Rc<PdfView> {
        let (req_tx, req_rx) = async_channel::unbounded();
        let (resp_tx, resp_rx) = async_channel::unbounded();
        std::thread::Builder::new()
            .name("rutex-pdf".into())
            .spawn(move || worker(req_rx, resp_tx))
            .expect("no se pudo crear el hilo del PDF");

        let model = gtk::StringList::new(&[]);
        let factory = gtk::SignalListItemFactory::new();
        let list = gtk::ListView::new(Some(gtk::NoSelection::new(Some(model.clone()))), Some(factory.clone()));
        list.add_css_class("pdf-view");

        let scrolled = gtk::ScrolledWindow::builder().child(&list).hexpand(true).vexpand(true).build();

        let status = adw::StatusPage::builder()
            .icon_name("x-office-document-symbolic")
            .title("Sin PDF todavía")
            .description("Se compilará automáticamente al escribir.\nCtrl+Intro para compilar ahora.")
            .build();
        status.add_css_class("compact");

        let stack = gtk::Stack::new();
        stack.add_named(&status, Some("empty"));
        stack.add_named(&scrolled, Some("pdf"));
        stack.set_visible_child_name("empty");

        let zoom_label = gtk::Label::builder().label("100%").width_chars(5).build();
        let zoom_out = gtk::Button::builder().icon_name("zoom-out-symbolic").tooltip_text("Alejar (Ctrl+-)").build();
        let zoom_in = gtk::Button::builder().icon_name("zoom-in-symbolic").tooltip_text("Acercar (Ctrl++)").build();
        let fit = gtk::Button::builder().icon_name("zoom-fit-best-symbolic").tooltip_text("Ajustar al ancho (Ctrl+0)").build();
        let controls = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(2)
            .halign(gtk::Align::End)
            .valign(gtk::Align::End)
            .margin_end(18)
            .margin_bottom(18)
            .css_classes(["osd", "toolbar", "pdf-controls"])
            .build();
        controls.append(&zoom_out);
        controls.append(&zoom_label);
        controls.append(&zoom_in);
        controls.append(&fit);

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
            state: RefCell::new(State { zoom: 1.0, ..Default::default() }),
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
    pub fn connect_inverse(&self, f: impl Fn(PdfPos) + 'static) {
        *self.on_inverse.borrow_mut() = Some(Box::new(f));
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
        if pos.page >= self.model.n_items() as usize {
            return;
        }
        {
            let mut st = self.state.borrow_mut();
            st.marker = Some((pos.page, pos.y));
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
                    vadj.set_value(vadj.value() + p.y() as f64 - vadj.page_size() * 0.35);
                }
                da.queue_draw();
            }
        });

        let weak = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(Duration::from_millis(1600), move || {
            let Some(this) = weak.upgrade() else { return };
            let mut st = this.state.borrow_mut();
            st.marker_timeout = None;
            if let Some((page, _)) = st.marker.take()
                && let Some(da) = st.bound.get(&page).and_then(|w| w.upgrade()) {
                    da.queue_draw();
                }
        });
        self.state.borrow_mut().marker_timeout = Some(id);
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

            let click = gtk::GestureClick::new();
            let w = weak.clone();
            click.connect_pressed(move |gesture, n_press, x, y| {
                if n_press != 2 {
                    return;
                }
                let Some(this) = w.upgrade() else { return };
                let Some(page) = gesture.widget().and_then(|da| page_of(&da)) else { return };
                let scale = this.state.borrow().scale();
                if let Some(cb) = this.on_inverse.borrow().as_ref() {
                    cb(PdfPos { page, x: x / scale, y: y / scale });
                }
            });
            da.add_controller(click);
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
        cr.set_source_rgb(1.0, 1.0, 1.0);
        cr.paint().ok();

        let device_scale = da.native().and_then(|n| n.surface()).map_or(da.scale_factor() as f64, |s| s.scale());
        let target = ((w * device_scale).round() as i32).clamp(1, MAX_RENDER_WIDTH);

        let mut st = self.state.borrow_mut();
        st.tick += 1;
        let tick = st.tick;
        let shown = st.shown;
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
            needs_render = c.generation != shown || (c.width - target).abs() > 1;
        }

        if let Some((mpage, my)) = st.marker
            && mpage == page {
                let y = my * st.scale();
                cr.set_source_rgba(0.95, 0.3, 0.2, 0.35);
                cr.rectangle(0.0, (y - 14.0).max(0.0), w, 18.0);
                cr.fill().ok();
            }

        if needs_render && shown > 0 && st.requested.insert((page, target, shown)) {
            let _ = self.tx.try_send(Req::Render { generation: shown, page, width_px: target });
        }
    }

    fn handle(&self, resp: Resp) {
        match resp {
            Resp::Loaded { generation, sizes } => {
                let n_old = self.model.n_items();
                {
                    let mut st = self.state.borrow_mut();
                    if generation != st.generation {
                        return;
                    }
                    st.shown = generation;
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
            }
            Resp::Failed { generation, error } => {
                if generation == self.state.borrow().generation && !self.has_document() {
                    self.status.set_description(Some(&format!("No se pudo abrir el PDF:\n{error}")));
                    self.stack.set_visible_child_name("empty");
                }
            }
            Resp::Rendered { generation, page, width, height, stride, data } => {
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
                if let Some(old) = st.cache.insert(page, Cached { generation, width, height, surface, bytes, used }) {
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
                    let sizes = (0..d.n_pages()).filter_map(|i| d.page(i)).map(|p| p.size()).collect();
                    doc = Some((generation, d));
                    if tx.send_blocking(Resp::Loaded { generation, sizes }).is_err() {
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

        let Some(Req::Render { generation, page, width_px }) = pending.pop() else { continue };
        pending.retain(|r| !matches!(r, Req::Render { page: p, .. } if *p == page));
        let Some((doc_gen, d)) = doc.as_ref() else { continue };
        if *doc_gen != generation {
            continue;
        }
        if let Some((width, height, stride, data)) = render(d, page, width_px) {
            let resp = Resp::Rendered { generation, page, width, height, stride, data };
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
