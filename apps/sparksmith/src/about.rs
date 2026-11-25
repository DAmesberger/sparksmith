//! What a thing on the board or the sheet is, in the words of its card
//! ([`crate::kit::hover_card`]): the one function from an object to what its
//! card says ([`about`]), and the one state every canvas shows it through
//! ([`Card`]). Schematic, Layout and PCB 3D ask the same function, so a part
//! reads the same in all three.
//!
//! A value nothing states is left out. The card holds no control, takes no
//! key and no focus; acting is in Properties.
use crate::kit::hover_card::{self, Content, Over};
use egui::{Pos2, Rect, Ui};
use sparksmith_core::instances::{self, variants};
use sparksmith_core::{
    ComponentInstance, Endpoint, Id, Layer, PadId, Project, TerminalId, ViaSpan,
};

/// What a card is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Subject {
    Part(Id),
    Pad {
        instance: Id,
        pad: PadId,
    },
    Pin(Endpoint),
    Track(Id),
    Via(Id),
    /// A schematic wire, on its net once it has one.
    Wire {
        wire: Id,
        net: Option<Id>,
    },
    /// A drawn dimension: its editor says what it measures.
    Dimension(Id),
}

fn mm(v: f64) -> String {
    format!(
        "{} mm",
        crate::kit::number_text((v * 1000.0).round() / 1000.0)
    )
}

/// `text` without the space around it; nothing when nothing is left.
fn said(text: &str) -> Option<String> {
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

/// What `subject` is, or nothing when the document no longer has it.
pub fn about(project: &Project, subject: Subject) -> Option<Content> {
    match subject {
        Subject::Part(id) => Some(part(project, instance(project, id)?)),
        Subject::Pad { instance: id, pad } => pad_of(project, instance(project, id)?, pad),
        Subject::Pin(pin) => Some(pin_of(
            project,
            instance(project, pin.instance)?,
            pin.terminal,
        )),
        Subject::Track(id) => {
            let track = project.tracks.iter().find(|t| t.id == id)?;
            let mut rows = vec![("Layer", track.layer.name()), ("Width", mm(track.width))];
            rows.extend(class(project, track.net));
            rows.extend(locked(track.locked));
            Some(Content::new(
                format!("Track · {}", project.net_name(track.net)),
                rows,
            ))
        }
        Subject::Via(id) => {
            let via = project.via(id)?;
            let span = match via.span {
                ViaSpan::Through => "Through".to_owned(),
                ViaSpan::Between { from, to } => format!("{} to {}", from.name(), to.name()),
            };
            let mut rows = vec![
                ("Span", span),
                ("Drill", mm(via.drill)),
                ("Pad", mm(via.diameter)),
            ];
            if !via.fill.is_none() {
                rows.push(("Fill", via.fill.label().to_owned()));
            }
            rows.extend(class(project, via.net));
            rows.extend(locked(via.locked));
            Some(Content::new(
                format!("Via · {}", project.net_name(via.net)),
                rows,
            ))
        }
        Subject::Wire { wire, net } => {
            project.wire(wire)?;
            let Some(net) = net.and_then(|id| project.net(id)) else {
                return Some(Content::new("Wire".to_owned(), Vec::new()));
            };
            let mut rows = Vec::new();
            rows.extend(class(project, net.id));
            rows.push(("Pins", net.terminals.len().to_string()));
            Some(Content::new(format!("Net · {}", net.name), rows))
        }
        Subject::Dimension(_) => None,
    }
}

fn instance(project: &Project, id: Id) -> Option<&ComponentInstance> {
    project
        .instance(id)
        .or_else(|| project.drawing_instance(id))
}

fn class(project: &Project, net: Id) -> Option<(&'static str, String)> {
    let class = project.rules.class_of(net, project.net_name(net))?;
    Some(("Class", project.rules.class(class)?.name.clone()))
}

fn locked(locked: bool) -> Option<(&'static str, String)> {
    locked.then(|| ("Locked", "Yes".to_owned()))
}

/// What a part is, after its designator: the number of the Part fitted, else
/// its value; then its package.
fn named(project: &Project, instance: &ComponentInstance) -> Vec<String> {
    let fitted = variants::fitted(project, instance);
    let number = fitted
        .part
        .and_then(|r| project.library.part(r))
        .and_then(|p| said(&p.definition.mpn));
    let value = || {
        variants::fitted_value(project, instance, &fitted)
            .ok()
            .flatten()
            .and_then(|v| said(&v))
    };
    number
        .or_else(value)
        .into_iter()
        .chain(sparksmith_core::fabrication::assembly::package_name(
            project, instance,
        ))
        .collect()
}

/// A part: "C21 · 10 µF 25 V · 0805", what it is in words, and what the
/// library, the board and the assembly variant say of it. A block and the
/// part's role in it are further rows here.
fn part(project: &Project, instance: &ComponentInstance) -> Content {
    let fitted = variants::fitted(project, instance);
    let named = named(project, instance);
    let title = std::iter::once(instance.designator.clone())
        .chain(named.iter().cloned())
        .collect::<Vec<_>>()
        .join(" · ");
    let mut content = Content::new(title, Vec::new());
    content.words = project
        .library
        .component(fitted.component)
        .and_then(|c| said(&c.meta.description).or_else(|| said(&c.meta.name)))
        .filter(|words| !named.contains(words));
    let rows = &mut content.rows;
    if let Some(part) = fitted.part.and_then(|r| project.library.part(r)) {
        // The title has its number.
        if said(&part.definition.mpn).is_some()
            && let Ok(Some(value)) = variants::fitted_value(project, instance, &fitted)
            && let Some(value) = said(&value).filter(|v| !named.contains(v))
        {
            rows.push(("Value", value));
        }
        rows.extend(said(&part.definition.manufacturer).map(|m| ("Manufacturer", m)));
    }
    let height = fitted
        .physical
        .and_then(|p| p.package.definition.envelope_nm.get(2).copied())
        .map(|h| h.to_mm_f64())
        .filter(|h| *h > 0.0);
    rows.extend(height.map(|h| ("Height", mm(h))));
    match project.placement_of(instance.id) {
        Some(placement) => {
            let side = match placement.layer {
                Layer::Top => "Top",
                Layer::Bottom => "Bottom",
            };
            rows.push(("Side", side.to_owned()));
            rows.extend(locked(placement.locked));
        }
        None if fitted.physical.is_some() => rows.push(("Board", "Not placed".to_owned())),
        None => {}
    }
    let variant = project
        .variants
        .active()
        .map_or_else(String::new, |v| format!(" in {}", v.name));
    if !fitted.populated() {
        rows.push(("Assembly", format!("Not fitted{variant}")));
    } else if fitted.substituted {
        rows.push(("Assembly", format!("Substitute{variant}")));
    }
    content
}

/// A pad of a placed part: its number, net and what its pin does.
fn pad_of(project: &Project, instance: &ComponentInstance, pad: PadId) -> Option<Content> {
    let physical = instances::resolve(&project.library, instance)
        .ok()?
        .physical?;
    let pad = physical
        .footprint
        .definition
        .pads
        .iter()
        .find(|p| p.id == pad)?;
    let terminal = pad
        .terminal
        .and_then(|t| physical.binding.definition.terminal_of(t));
    let title = match said(&pad.number) {
        Some(number) => format!("{} pad {number}", instance.designator),
        None => format!("{} pad", instance.designator),
    };
    let rows = terminal.map_or_else(Vec::new, |t| terminal_rows(project, instance, t));
    Some(on(project, instance, title, rows))
}

/// A symbol's pin: as its pad, by the numbers its package gives it.
fn pin_of(project: &Project, instance: &ComponentInstance, terminal: TerminalId) -> Content {
    let numbers = instances::resolve(&project.library, instance)
        .ok()
        .and_then(|r| r.physical)
        .and_then(|p| {
            sparksmith_core::schematic::pin_numbers(
                &p.binding.definition,
                &p.package.definition,
                terminal,
            )
        });
    let title = match numbers {
        Some(numbers) => format!("{} pin {numbers}", instance.designator),
        None => format!("{} pin", instance.designator),
    };
    on(
        project,
        instance,
        title,
        terminal_rows(project, instance, terminal),
    )
}

/// A card about something on a part, which says what the part is.
fn on(
    project: &Project,
    instance: &ComponentInstance,
    title: String,
    rows: Vec<(&'static str, String)>,
) -> Content {
    let mut content = Content::new(title, rows);
    content.words = said(&named(project, instance).join(" · "));
    content
}

/// A terminal's net, name, and the function it has here.
fn terminal_rows(
    project: &Project,
    instance: &ComponentInstance,
    terminal: TerminalId,
) -> Vec<(&'static str, String)> {
    let mut rows = Vec::new();
    let endpoint = Endpoint {
        instance: instance.id,
        terminal,
    };
    rows.extend(project.net_of(endpoint).map(|n| ("Net", n.name.clone())));
    let declared = project
        .library
        .component(instance.component)
        .and_then(|c| c.definition.terminal(terminal));
    let name = declared.and_then(|t| said(&t.name));
    rows.extend(name.clone().map(|n| ("Pin", n)));
    if declared.is_some_and(|t| t.functions().is_some())
        && let Ok(resolution) = instances::function_resolution(&project.library, instance)
        && let Some(active) = resolution.active(terminal)
        && let Some(label) = said(&active.label)
    {
        let direction = active.direction.label();
        let mut words = Vec::new();
        if Some(&label) != name.as_ref() && label != direction {
            words.push(label);
        }
        words.push(direction.to_owned());
        rows.push(("Function", words.join(", ")));
    }
    rows
}

/// The thing under the pointer or the keyboard focus, and where it is.
pub struct Target {
    pub subject: Subject,
    pub over: Over,
    /// What the card says, for a subject only its editor can describe.
    pub content: Option<Content>,
}

impl Target {
    pub fn new(subject: Subject, over: Over) -> Self {
        Self {
            subject,
            over,
            content: None,
        }
    }
}

/// The one card of a canvas. What it says is made once per subject and
/// document revision; a frame with nothing under the pointer does nothing.
#[derive(Default)]
pub struct Card {
    shown: Option<(Subject, u64, Option<Content>)>,
    /// Esc closed this subject's card, until something else is hovered.
    closed: Option<Subject>,
    escape: bool,
}

impl Card {
    /// The canvas's start: whether Esc was pressed, read before a command
    /// takes the key and left for it.
    pub fn begin(&mut self, ctx: &egui::Context) {
        self.escape = ctx.input(|i| i.key_pressed(egui::Key::Escape));
    }

    /// Whether the pointer is at rest on a canvas: nothing is pressed or
    /// dragged and no menu is open.
    pub fn rests(ctx: &egui::Context) -> bool {
        !ctx.input(|i| i.pointer.any_down())
            && !ctx.any_popup_open()
            && !crate::context_menu::is_open(ctx)
    }

    /// Show `target`'s card beside it in `room`; with none, close it.
    pub fn show(
        &mut self,
        ui: &Ui,
        project: &Project,
        revision: u64,
        target: Option<Target>,
        pointer: Option<Pos2>,
        room: Rect,
    ) -> Option<Rect> {
        let Some(target) = target else {
            self.shown = None;
            self.closed = None;
            return None;
        };
        let subject = target.subject;
        if self.escape {
            self.closed = Some(subject);
        }
        if self.closed == Some(subject) {
            return None;
        }
        self.closed = None;
        if target.content.is_some()
            || !matches!(&self.shown, Some((s, r, _)) if *s == subject && *r == revision)
        {
            let content = target.content.or_else(|| about(project, subject));
            self.shown = Some((subject, revision, content));
        }
        let content = self.shown.as_ref().and_then(|(_, _, c)| c.as_ref())?;
        Some(hover_card::show(ui, content, &target.over, pointer, room))
    }
}

#[cfg(test)]
#[path = "about_tests.rs"]
mod tests;
