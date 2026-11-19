//! Part previews for the Place palette: a live, slowly turning render of a
//! part's real body (its part file evaluated by Solidcraft, else its
//! envelope, labelled approximate) above thumbnails of its footprint and
//! symbol.
//!
//! Nothing heavy runs on the UI thread. [`PartPreviews::request`] hands a
//! loader to one worker thread, which reads the records ([`PreviewSource`]),
//! evaluates the body, renders the first frame with a small CPU rasteriser
//! and sends it back; while a pane shows that part the worker renders the
//! next turn angle about 30 times a second, and stops when the pane is no
//! longer drawn. The newest request is served first and at most
//! [`MAX_QUEUED`] wait, so arrowing through a list never queues work for
//! rows left behind. Finished previews are kept per [`PreviewKey`] (a
//! content hash or an exact project revision), most recent first, so a
//! row seen before shows at once.
mod mesh;
mod raster;
mod source;
#[cfg(test)]
mod tests;

pub use source::{FootprintDrawing, PreviewSource};

use egui::{Color32, ColorImage, Rect, Response, Sense, Stroke, TextureHandle, Ui, Vec2, vec2};
use raster::Model;
use sparksmith_core::geometry::Shape;
use sparksmith_core::ids::{ComponentRef, ContentHash};
use sparksmith_core::library::graphics::DrawingLayer;
use sparksmith_core::library::standards::Context;
use sparksmith_core::schematic::PlacedUnit;
use sparksmith_ui::theme::Tokens;
use std::collections::{HashMap, VecDeque};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// Requests waiting for the worker; older ones are dropped.
pub const MAX_QUEUED: usize = 4;
/// Previews kept by the pane (one texture each).
pub const MAX_KEPT: usize = 64;
/// Evaluated bodies the worker keeps for turning.
const MAX_MODELS: usize = 12;
/// Rendered image side, in pixels (the pane scales it).
pub const FRAME_PX: usize = 224;
/// One full turn.
const TURN: Duration = Duration::from_secs(12);
/// Time between turn frames.
const FRAME_STEP: Duration = Duration::from_millis(33);
/// How long the worker keeps turning after the pane was last drawn.
const SHOWN_FOR: Duration = Duration::from_millis(250);

/// What a preview is of: a record by content hash (a folder's or the
/// registry's root revision), or an exact project revision at a document
/// revision.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PreviewKey {
    Record(ContentHash),
    Project {
        component: ComponentRef,
        revision: u64,
    },
}

/// How a finished preview was made.
#[derive(Clone, Debug, PartialEq)]
pub struct Stats {
    /// What the 3D view shows ("3D body", "Envelope (approximate)", …).
    pub label: String,
    pub bodies: usize,
    pub triangles: usize,
    /// From the request to the first frame on the channel, ms.
    pub first_frame_ms: f64,
    /// The body was already evaluated by the worker.
    pub cached: bool,
}

/// Where one preview stands.
#[derive(Clone, Debug, PartialEq)]
pub enum PreviewState {
    /// Never requested, or dropped from the queue or the cache.
    Unknown,
    Pending,
    Ready(Stats),
    /// The records or the body could not be read; the reason. Footprint
    /// and symbol still show when they were read.
    Failed(String),
}

type Loader = Box<dyn FnOnce() -> Result<PreviewSource, String> + Send>;

struct Job {
    key: PreviewKey,
    load: Loader,
    context: Option<Arc<Context>>,
    asked: Instant,
}

#[derive(Default)]
struct Shared {
    jobs: VecDeque<Job>,
    /// The key a pane drew last, and when.
    shown: Option<(PreviewKey, Instant)>,
    motion: bool,
    quit: bool,
}

/// Drawn from the records: what the 2D thumbnails paint.
#[derive(Clone, Debug, Default)]
pub struct Drawings {
    pub name: String,
    pub footprint: Option<FootprintDrawing>,
    pub symbol: Option<PlacedUnit>,
}

enum Message {
    Loaded(PreviewKey, Arc<Drawings>),
    Frame(PreviewKey, ColorImage),
    Ready(PreviewKey, Stats),
    Failed(PreviewKey, String),
}

struct Entry {
    state: PreviewState,
    drawings: Option<Arc<Drawings>>,
    texture: Option<TextureHandle>,
    /// A frame not yet uploaded.
    image: Option<ColorImage>,
}

type Repaint = Arc<Mutex<Option<egui::Context>>>;

/// The previews of one palette: the worker, its results and their textures.
pub struct PartPreviews {
    shared: Arc<(Mutex<Shared>, Condvar)>,
    receiver: Receiver<Message>,
    entries: HashMap<PreviewKey, Entry>,
    /// Most recently used first.
    order: VecDeque<PreviewKey>,
    repaint: Repaint,
}

impl Default for PartPreviews {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for PartPreviews {
    fn drop(&mut self) {
        let (lock, wake) = &*self.shared;
        if let Ok(mut shared) = lock.lock() {
            shared.quit = true;
        }
        wake.notify_all();
    }
}

impl PartPreviews {
    pub fn new() -> Self {
        let shared = Arc::new((
            Mutex::new(Shared {
                motion: true,
                ..Shared::default()
            }),
            Condvar::new(),
        ));
        let (sender, receiver) = mpsc::channel();
        let repaint: Repaint = Arc::default();
        let worker = Worker {
            shared: Arc::clone(&shared),
            sender,
            repaint: Arc::clone(&repaint),
            models: VecDeque::new(),
            turning: None,
        };
        // Without a thread the previews stay pending and say so; nothing
        // else depends on them.
        let _ = std::thread::Builder::new()
            .name("part-preview".into())
            .spawn(move || worker.run());
        Self {
            shared,
            receiver,
            entries: HashMap::new(),
            order: VecDeque::new(),
            repaint,
        }
    }

    /// The context the worker asks to repaint when a frame arrives.
    pub fn set_repaint(&mut self, ctx: &egui::Context) {
        if let Ok(mut slot) = self.repaint.lock()
            && slot.is_none()
        {
            *slot = Some(ctx.clone());
        }
    }

    /// Turn the shown part (the default), or show still frames only
    /// (reduced motion).
    pub fn set_motion(&mut self, motion: bool) {
        if let Ok(mut shared) = self.shared.0.lock() {
            shared.motion = motion;
        }
    }

    /// Ask for the preview of `key`; `load` reads its records on the
    /// worker. Nothing happens when it is kept or already asked for.
    pub fn request(
        &mut self,
        key: PreviewKey,
        load: impl FnOnce() -> Result<PreviewSource, String> + Send + 'static,
    ) {
        if self.entries.contains_key(&key) {
            self.touch(key);
            return;
        }
        self.entries.insert(
            key,
            Entry {
                state: PreviewState::Pending,
                drawings: None,
                texture: None,
                image: None,
            },
        );
        self.touch(key);
        let dropped = {
            let Ok(mut shared) = self.shared.0.lock() else {
                return;
            };
            shared.jobs.push_back(Job {
                key,
                load: Box::new(load),
                context: Context::current().ok(),
                asked: Instant::now(),
            });
            let mut dropped = Vec::new();
            while shared.jobs.len() > MAX_QUEUED {
                if let Some(old) = shared.jobs.pop_front() {
                    dropped.push(old.key);
                }
            }
            dropped
        };
        // A dropped request is asked again when its row is shown again.
        for key in dropped {
            self.entries.remove(&key);
            self.order.retain(|k| *k != key);
        }
        self.shared.1.notify_all();
    }

    fn touch(&mut self, key: PreviewKey) {
        self.order.retain(|k| *k != key);
        self.order.push_front(key);
        while self.order.len() > MAX_KEPT {
            if let Some(old) = self.order.pop_back() {
                self.entries.remove(&old);
            }
        }
    }

    pub fn state(&self, key: &PreviewKey) -> PreviewState {
        self.entries
            .get(key)
            .map_or(PreviewState::Unknown, |e| e.state.clone())
    }

    /// Take what the worker finished. The pane calls this; tests too.
    pub fn receive(&mut self) {
        while let Ok(message) = self.receiver.try_recv() {
            let key = match &message {
                Message::Loaded(k, _)
                | Message::Frame(k, _)
                | Message::Ready(k, _)
                | Message::Failed(k, _) => *k,
            };
            let Some(entry) = self.entries.get_mut(&key) else {
                continue;
            };
            match message {
                Message::Loaded(_, drawings) => entry.drawings = Some(drawings),
                Message::Frame(_, image) => entry.image = Some(image),
                Message::Ready(_, stats) => entry.state = PreviewState::Ready(stats),
                Message::Failed(_, reason) => entry.state = PreviewState::Failed(reason),
            }
        }
    }

    /// Whether `key` has a frame to show (uploaded or waiting to be).
    pub fn has_frame(&self, key: &PreviewKey) -> bool {
        self.entries
            .get(key)
            .is_some_and(|e| e.texture.is_some() || e.image.is_some())
    }

    /// The preview of `key` in `size`: the turning body on top, the
    /// footprint and symbol thumbnails below it side by side, and one line
    /// naming what the 3D view shows. A spinner while the worker reads it.
    pub fn pane(&mut self, ui: &mut Ui, key: &PreviewKey, size: Vec2) -> Response {
        self.set_repaint(ui.ctx());
        self.receive();
        let t = Tokens::get();
        let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
        let caption = t.font_small + t.space_6;
        let gap = t.space_4;
        let thumbs = ((rect.height() - caption - gap) * 0.36).min(rect.width() / 2.0 - gap / 2.0);
        let stage = Rect::from_min_size(
            rect.min,
            vec2(
                rect.width(),
                (rect.height() - caption - thumbs - 2.0 * gap).max(0.0),
            ),
        );
        let row = Rect::from_min_size(
            stage.left_bottom() + vec2(0.0, gap),
            vec2(rect.width(), thumbs.max(0.0)),
        );
        let line = Rect::from_min_size(
            row.left_bottom() + vec2(0.0, gap),
            vec2(rect.width(), caption),
        );
        let painter = ui.painter_at(rect);
        well(&painter, stage);
        let state = self.state(key);
        let ctx = ui.ctx().clone();
        let entry = self.entries.get_mut(key);
        let mut shown = false;
        if let Some(entry) = entry {
            if let Some(image) = entry.image.take() {
                match &mut entry.texture {
                    Some(texture) => texture.set(image, egui::TextureOptions::LINEAR),
                    None => {
                        entry.texture = Some(ctx.load_texture(
                            "part-preview",
                            image,
                            egui::TextureOptions::LINEAR,
                        ));
                    }
                }
            }
            if let Some(texture) = &entry.texture {
                let side = stage.width().min(stage.height());
                let image = Rect::from_center_size(stage.center(), vec2(side, side));
                painter.image(
                    texture.id(),
                    image,
                    Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
                shown = true;
            }
            let half = vec2((row.width() - gap) / 2.0, row.height());
            let left = Rect::from_min_size(row.min, half);
            let right = Rect::from_min_size(row.min + vec2(half.x + gap, 0.0), half);
            well(&painter, left);
            well(&painter, right);
            if let Some(drawings) = &entry.drawings {
                if let Some(footprint) = &drawings.footprint {
                    paint_footprint(ui, left.shrink(t.space_4), footprint);
                }
                if let Some(symbol) = &drawings.symbol {
                    paint_symbol(ui, right.shrink(t.space_4), symbol);
                }
            }
        }
        let text = match &state {
            PreviewState::Ready(stats) => stats.label.clone(),
            PreviewState::Failed(reason) => reason.clone(),
            PreviewState::Pending => "Reading the part…".into(),
            PreviewState::Unknown => String::new(),
        };
        if !shown && matches!(state, PreviewState::Pending) {
            let spin = Rect::from_center_size(stage.center(), Vec2::splat(t.space_16));
            ui.put(spin, egui::Spinner::new().size(t.space_16));
        }
        let ink = if matches!(state, PreviewState::Failed(_)) {
            t.form_error
        } else {
            t.form_dim
        };
        if !text.is_empty() {
            crate::kit::text::line(
                ui,
                egui::Id::unique(("part-preview-caption", *key)),
                line,
                &text,
                "",
                egui::FontId::proportional(t.font_small),
                ink,
            );
        }
        if matches!(state, PreviewState::Ready(_)) {
            if let Ok(mut shared) = self.shared.0.lock() {
                shared.shown = Some((*key, Instant::now()));
            }
            self.shared.1.notify_all();
            ctx.request_repaint_after(FRAME_STEP * 3);
        } else if matches!(state, PreviewState::Pending) {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
        response
    }
}

/// One preview well: the control fill and hairline the kit's fields use.
fn well(painter: &egui::Painter, rect: Rect) {
    let t = Tokens::get();
    painter.rect_filled(rect, t.radius_md, t.control_fill);
    painter.rect_stroke(
        rect,
        t.radius_md,
        Stroke::new(1.0, t.control_border),
        egui::StrokeKind::Inside,
    );
}

/// Fit `bounds` (mm) into `rect`, y up.
fn fitted(bounds: sparksmith_core::geometry::Bounds, rect: Rect) -> crate::symbol_view::View {
    let [w, h] = bounds.size();
    let scale = (rect.width() / w.max(0.1) as f32).min(rect.height() / h.max(0.1) as f32) * 0.9;
    let [cx, cy] = bounds.center();
    crate::symbol_view::View {
        origin: rect.center() + vec2(-(cx as f32) * scale, cy as f32 * scale),
        scale,
    }
}

fn paint_footprint(ui: &Ui, rect: Rect, footprint: &FootprintDrawing) {
    let Some(bounds) = footprint.bounds else {
        return;
    };
    let t = Tokens::get();
    let view = fitted(bounds, rect);
    let painter = ui.painter_at(rect);
    for (shape, width, layer) in &footprint.lines {
        let ink = match layer {
            DrawingLayer::Silkscreen => t.text,
            DrawingLayer::Courtyard | DrawingLayer::Assembly => t.text_dim,
        };
        let stroke = Stroke::new((*width as f32 * view.scale).max(1.0), ink);
        crate::symbol_view::paint_shape(&painter, view, shape, None, Some(stroke));
    }
    // Lands in the footprint canvas's copper colour, holes in the well's.
    let copper = crate::palette::board().copper_top;
    for ring in &footprint.pads {
        let shape = Shape::polygon(ring.clone());
        crate::symbol_view::paint_shape(&painter, view, &shape, Some(copper), None);
    }
    for ring in &footprint.holes {
        let shape = Shape::polygon(ring.clone());
        crate::symbol_view::paint_shape(&painter, view, &shape, Some(t.control_fill), None);
    }
}

fn paint_symbol(ui: &Ui, rect: Rect, unit: &PlacedUnit) {
    let painter = ui.painter_at(rect);
    let Some(bounds) = unit.bounds(crate::symbol_view::metric(&painter)) else {
        return;
    };
    let t = Tokens::get();
    crate::symbol_view::paint(
        &painter,
        fitted(bounds, rect),
        unit,
        crate::symbol_view::Ink {
            stroke: t.text,
            body: t.control_fill,
            text: t.text,
            pin: t.accent,
            pin_text: t.text_dim,
        },
    );
}

struct Worker {
    shared: Arc<(Mutex<Shared>, Condvar)>,
    sender: Sender<Message>,
    repaint: Repaint,
    /// Evaluated bodies, most recent first.
    models: VecDeque<(PreviewKey, Arc<Model>)>,
    /// The part being turned and when its turn began.
    turning: Option<(PreviewKey, Instant)>,
}

enum Work {
    Job(Job),
    Turn(PreviewKey),
    Quit,
}

impl Worker {
    fn run(mut self) {
        loop {
            match self.next() {
                Work::Quit => return,
                Work::Job(job) => self.serve(job),
                Work::Turn(key) => {
                    let started = Instant::now();
                    self.turn(key);
                    let rest = FRAME_STEP.saturating_sub(started.elapsed());
                    let (lock, wake) = &*self.shared;
                    if let Ok(shared) = lock.lock()
                        && shared.jobs.is_empty()
                    {
                        let _ = wake.wait_timeout(shared, rest);
                    }
                }
            }
        }
    }

    fn next(&mut self) -> Work {
        let (lock, wake) = &*self.shared;
        let Ok(mut shared) = lock.lock() else {
            return Work::Quit;
        };
        loop {
            if shared.quit {
                return Work::Quit;
            }
            if let Some(job) = shared.jobs.pop_back() {
                return Work::Job(job);
            }
            let shown = shared
                .shown
                .filter(|(_, at)| at.elapsed() <= SHOWN_FOR)
                .map(|(key, _)| key);
            if shared.motion
                && let Some(key) = shown
                && self.models.iter().any(|(k, _)| *k == key)
            {
                return Work::Turn(key);
            }
            shared = match wake.wait_timeout(shared, SHOWN_FOR) {
                Ok((guard, _)) => guard,
                Err(_) => return Work::Quit,
            };
        }
    }

    fn send(&self, message: Message) {
        let _ = self.sender.send(message);
        if let Ok(slot) = self.repaint.lock()
            && let Some(ctx) = slot.as_ref()
        {
            ctx.request_repaint();
        }
    }

    fn serve(&mut self, job: Job) {
        let _context = job.context.as_ref().map(|c| c.enter());
        let key = job.key;
        let source = match (job.load)() {
            Ok(source) => source,
            Err(reason) => return self.send(Message::Failed(key, reason)),
        };
        self.send(Message::Loaded(
            key,
            Arc::new(Drawings {
                name: source.name.clone(),
                footprint: source.footprint.clone(),
                symbol: source.symbol.clone(),
            }),
        ));
        let label = mesh::label(&source.body);
        let held = self.models.iter().position(|(k, _)| *k == key);
        let cached = held.is_some();
        let model = match held.and_then(|at| self.models.remove(at)) {
            Some((_, model)) => model,
            None => match mesh::model(&source.body) {
                Ok(model) => Arc::new(model),
                Err(reason) => return self.send(Message::Failed(key, reason)),
            },
        };
        self.models.push_front((key, Arc::clone(&model)));
        self.models.truncate(MAX_MODELS);
        self.send(Message::Frame(key, raster::render(&model, 0.0, FRAME_PX)));
        self.send(Message::Ready(
            key,
            Stats {
                label,
                bodies: model.parts.len(),
                triangles: model.triangles(),
                first_frame_ms: job.asked.elapsed().as_secs_f64() * 1000.0,
                cached,
            },
        ));
    }

    fn turn(&mut self, key: PreviewKey) {
        let Some(model) = self
            .models
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, m)| Arc::clone(m))
        else {
            return;
        };
        let begun = match self.turning {
            Some((k, at)) if k == key => at,
            _ => {
                let now = Instant::now();
                self.turning = Some((key, now));
                now
            }
        };
        let angle = begun.elapsed().as_secs_f32() / TURN.as_secs_f32() * std::f32::consts::TAU;
        self.send(Message::Frame(key, raster::render(&model, angle, FRAME_PX)));
    }
}
