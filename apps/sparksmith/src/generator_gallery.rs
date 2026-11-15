//! Slice D1: the "New from generator" gallery sheet, which replaces the
//! in-code template gallery. One searchable sheet (the kit search sheet: a
//! field, one chip per shipped generator, the presets on the left and the
//! one generator form on the right) serves two scopes:
//!
//! - **New component** (New component ▾ › From generator…, the start page,
//!   the Place palette): `library.generate` of a complete part, made off
//!   the UI thread with `library.generate_part`'s code
//!   ([`crate::generators::make`]); the created component opens, or with
//!   the Place palette is armed for placement.
//! - **Package** (the Footprint tab's From generator, the 3D tab's Change
//!   package…): `library.package_from_generator`, which makes the package
//!   with its generator and gives the open working set's package and
//!   footprint its content as one undo step, terminals matched by number;
//!   mappings are never guessed.
//!
//! Rows come from `library.generators` (the shelf): each generator's
//! custom values, then its presets, found by name, alias or description.
//! The form is built from the generator's description alone
//! (`generator_form`), with the regenerate rule (G15) shown as it applies:
//! a part the project holds is opened or placed as it is, one an older
//! version made is made again as its next revision, one a newer version
//! made is refused. Each submit waits for the engine's answer: a refusal
//! comes back to the sheet's refusal line, success closes the sheet.
use crate::generator_form::{self, Values};
use crate::generators::{self, Checking, Into, MakeRequest, Making, Offer, Shelf};
use crate::kit::severity::Level;
use crate::kit::toast::{ToastAction, post};
use crate::kit::{self, Tone};
use crate::refusal::{self, Origin};
use egui::{Align, Layout, Ui, pos2, vec2};
use sparksmith_core::catalog::MAX_QUERY;
use sparksmith_core::catalog::generic::generators::{GeneratorCandidate, PresetHit};
use sparksmith_core::library::MAX_LABEL;
use sparksmith_core::library::generated::{GeneratorScope, ParamProblem, Standing};
use sparksmith_core::{ComponentRef, DefinitionId, Engine, Operation};
use sparksmith_ui::theme::Tokens;
use sparksmith_ui::ui_kit::{self, Button};

/// The sheet's id (also its refusal origin).
pub const GALLERY: &str = "generator-gallery";
/// Rows one search lists.
pub const ROWS: usize = 200;
/// Frames a submit waits for the engine's answer before the sheet is
/// usable again (the app may drop an operation while a file is busy).
const ANSWER_FRAMES: u64 = 3;
const KEYS_NEW: &str = "↑↓ move   ⏎ create   esc close";
const KEYS_PLACE: &str = "↑↓ move   ⏎ create and place   esc close";
const KEYS_PACKAGE: &str = "↑↓ move   ⏎ use package   esc close";
/// A search list's group of generator presets, below its real components.
pub const SEARCH_HEADING: &str = "From generator";

/// What the sheet was opened for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Scope {
    /// A new component: `library.generate`.
    NewComponent,
    /// The open working set's package: `library.package_from_generator`.
    Package {
        /// The package draft.
        draft: DefinitionId,
        /// Pads the footprint holds now; a non-empty footprint is replaced
        /// in the same atomic step.
        pads: usize,
    },
}

impl Scope {
    fn generator_scope(&self) -> GeneratorScope {
        match self {
            Self::NewComponent => GeneratorScope::Component,
            Self::Package { .. } => GeneratorScope::Package,
        }
    }
}

/// One row of the list: a generator's custom values, or one of its
/// presets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub generator: String,
    pub preset: Option<String>,
    pub name: String,
    pub detail: String,
}

/// The rows `query` finds among `offers`, restricted to `chip` (a
/// generator id) when one is chosen: each generator's custom row (when the
/// text names the generator or is empty), then the presets it names.
pub fn rows(offers: &[Offer], chip: Option<&str>, query: &str) -> Vec<Row> {
    let words: Vec<String> = query
        .split_whitespace()
        .map(|w| w.to_lowercase().replace('×', "x"))
        .collect();
    let mut out = Vec::new();
    for offer in offers
        .iter()
        .filter(|o| chip.is_none_or(|c| o.description.id == c))
    {
        let title = format!(
            "{} {} {}",
            offer.summary.name,
            offer.description.id,
            offer.summary.families.join(" ")
        )
        .to_lowercase();
        if words.iter().all(|w| title.contains(w.as_str())) {
            out.push(Row {
                generator: offer.description.id.clone(),
                preset: None,
                name: format!("{}: custom values", offer.summary.name),
                detail: format!("Generator {} version {}", offer.name, offer.summary.version),
            });
        }
        for preset in &offer.summary.presets {
            let text = format!(
                "{} {} {} {}",
                preset.name,
                preset.aliases.join(" "),
                preset.description,
                title
            )
            .to_lowercase()
            .replace('×', "x");
            if words.iter().all(|w| text.contains(w.as_str())) {
                out.push(Row {
                    generator: offer.description.id.clone(),
                    preset: Some(preset.key.clone()),
                    name: preset.name.clone(),
                    detail: generator_form::basis_line(preset),
                });
            }
            if out.len() >= ROWS {
                return out;
            }
        }
    }
    out
}

/// What a submit was.
#[derive(Clone, Debug, PartialEq)]
enum Sent {
    Create,
    CreateToPlace,
    Package,
}

/// A submitted operation waiting for the engine's answer.
#[derive(Clone, Debug)]
struct Submitted {
    sent: Sent,
    revision: u64,
    frame: u64,
    request: MakeRequest,
    update: bool,
}

/// The open sheet.
#[derive(Debug)]
struct Open {
    scope: Scope,
    /// The Place palette opened it: the primary continues to placement.
    to_place: bool,
    query: String,
    chip: Option<String>,
    rows: Vec<Row>,
    /// The text and chip the rows are for.
    listed: Option<(String, Option<String>)>,
    highlighted: usize,
    /// The highlighted row's identity, so a new search keeps it.
    kept: Option<(String, Option<String>)>,
    values: Option<Values>,
    name: String,
    named: bool,
    focus: bool,
    scroll: bool,
    making: Option<(Making, Sent, MakeRequest, bool)>,
    submitted: Option<Submitted>,
    refusal: Option<String>,
    /// The module checking the values being edited, off the UI thread.
    checking: Option<Checking>,
    /// The last values the module checked and its problems with them.
    checked: Option<(MakeRequest, Vec<ParamProblem>)>,
}

impl Open {
    fn new(scope: Scope, query: String) -> Self {
        Self {
            scope,
            to_place: false,
            query,
            chip: None,
            rows: Vec::new(),
            listed: None,
            highlighted: 0,
            kept: None,
            values: None,
            name: String::new(),
            named: false,
            focus: true,
            scroll: false,
            making: None,
            submitted: None,
            refusal: None,
            checking: None,
            checked: None,
        }
    }

    fn row(&self) -> Option<&Row> {
        self.rows.get(self.highlighted)
    }

    /// A new highlighted row: its own values and name.
    fn moved(&mut self, offers: &[Offer]) {
        self.refusal = None;
        self.scroll = true;
        let row = self.row().cloned();
        self.kept = row
            .as_ref()
            .map(|r| (r.generator.clone(), r.preset.clone()));
        self.values = row.as_ref().and_then(|r| {
            let offer = offers.iter().find(|o| o.description.id == r.generator)?;
            Some(Values::of(offer, r.preset.as_deref()))
        });
        if !self.named {
            self.name = row
                .filter(|r| r.preset.is_some())
                .map(|r| r.name)
                .unwrap_or_default();
        }
    }
}

/// The sheet's state; closed by default.
#[derive(Debug, Default)]
pub struct GeneratorGallery {
    open: Option<Open>,
    shelf: Shelf,
    /// The component a create made, for the host to open.
    created: Option<DefinitionId>,
    /// The component a create for placement made, with the values each
    /// placed instance takes.
    to_place: Option<ComponentRef>,
}

impl GeneratorGallery {
    /// The shipped generators the sheet offers.
    pub fn set_shelf(&mut self, shelf: &Shelf) {
        self.shelf = shelf.clone();
    }

    /// Open for a new component (New component ▾ › From generator…).
    pub fn open_new(&mut self) {
        self.open = Some(Open::new(Scope::NewComponent, String::new()));
    }

    /// Open on the package draft `draft` of the open working set, whose
    /// footprint holds `pads` pads; `query` prefills the search.
    pub fn open_package(&mut self, draft: DefinitionId, pads: usize, query: String) {
        self.open = Some(Open::new(Scope::Package { draft, pads }, query));
    }

    /// Open for a new component on `request`, a part a search listed. With
    /// `to_place` the primary continues to placement.
    pub fn open_on(&mut self, request: &MakeRequest, query: String, to_place: bool) {
        let mut open = Open::new(Scope::NewComponent, query);
        open.to_place = to_place;
        open.chip = Some(request.generator.clone());
        open.kept = Some((request.generator.clone(), request.preset.clone()));
        open.values = self
            .shelf
            .offer(&request.generator)
            .map(|offer| Values::of_request(&offer, request));
        if let Some(name) = &request.name {
            open.name.clone_from(name);
            open.named = true;
        }
        open.scroll = true;
        // Keep the values handed in when the list first settles.
        open.listed = None;
        self.open = Some(open);
    }

    pub fn close(&mut self) {
        self.open = None;
    }

    #[cfg(test)]
    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// The scope the open sheet serves.
    pub fn scope(&self) -> Option<&Scope> {
        self.open.as_ref().map(|o| &o.scope)
    }

    /// The component a create for placement made, once: the host arms
    /// placement of it.
    pub fn take_to_place(&mut self) -> Option<ComponentRef> {
        self.to_place.take()
    }

    /// The component lineage the last create made or chose, once: the host
    /// opens it in the Component workspace.
    pub fn take_created(&mut self) -> Option<DefinitionId> {
        self.created.take()
    }

    /// List the rows again when the text or the chip moved; keeps the
    /// highlighted row where it still exists.
    fn refresh(&mut self, offers: &[Offer]) {
        let Some(open) = &mut self.open else {
            return;
        };
        let key = (open.query.clone(), open.chip.clone());
        if open.listed.as_ref() == Some(&key) {
            return;
        }
        let first = open.listed.is_none();
        open.rows = rows(offers, open.chip.as_deref(), &open.query);
        open.listed = Some(key);
        let index = open.kept.as_ref().and_then(|(generator, preset)| {
            open.rows
                .iter()
                .position(|r| r.generator == *generator && r.preset == *preset)
        });
        match index {
            Some(i) => {
                open.highlighted = i;
                // Values handed in (`open_on`) stay; a new list starts the
                // highlighted row's own.
                if !(first && open.values.is_some()) {
                    open.moved(offers);
                }
            }
            None => {
                open.highlighted = 0;
                open.moved(offers);
            }
        }
    }

    /// The request the form makes now.
    fn request(&self) -> Option<MakeRequest> {
        let open = self.open.as_ref()?;
        let values = open.values.as_ref()?;
        let name = open.name.trim();
        Some(values.request((!name.is_empty()).then(|| name.to_owned())))
    }

    /// Settle a part being made and a submitted operation.
    fn settle(&mut self, ctx: &egui::Context, engine: &Engine) -> Option<Operation> {
        let id = egui::Id::unique(GALLERY);
        let open = self.open.as_mut()?;
        if let Some((making, _, _, _)) = &mut open.making
            && let Some(done) = making.poll()
        {
            let (_, sent, request, update) = open.making.take()?;
            match done {
                Ok(operation) => {
                    refusal::mark(ctx, Origin::Dialog(id));
                    open.submitted = Some(Submitted {
                        sent,
                        revision: engine.revision(),
                        frame: ctx.cumulative_frame_nr(),
                        request,
                        update,
                    });
                    return Some(operation);
                }
                Err(why) => open.refusal = Some(why),
            }
            return None;
        }
        let sent = open.submitted.as_ref()?;
        if let Some(why) = refusal::claim(ctx, Origin::Dialog(id)) {
            open.submitted = None;
            open.refusal = Some(why);
            return None;
        }
        if engine.revision() != sent.revision {
            let sent = sent.clone();
            let made = generators::made_component(engine, &self.shelf, &sent.request);
            let name = made
                .and_then(|c| engine.project().library.component(c))
                .map_or_else(|| "the part".to_owned(), |c| c.meta.name.clone());
            match sent.sent {
                Sent::Create => {
                    self.created = made.map(|c| c.id);
                    post(
                        ctx,
                        Level::Ok,
                        made_text(&name, sent.update),
                        vec![ToastAction::Undo],
                    );
                }
                Sent::CreateToPlace => self.to_place = made,
                Sent::Package => post(
                    ctx,
                    Level::Ok,
                    "Package, footprint and model taken from the generator",
                    vec![ToastAction::Undo],
                ),
            }
            self.open = None;
            return None;
        }
        if ctx.cumulative_frame_nr() > sent.frame.saturating_add(ANSWER_FRAMES) {
            open.submitted = None;
        }
        None
    }

    /// Show the sheet when open; the operation to apply when a part was
    /// made.
    pub fn show(&mut self, ctx: &egui::Context, engine: &Engine) -> Option<Operation> {
        self.open.as_ref()?;
        if let Some(operation) = self.settle(ctx, engine) {
            return Some(operation);
        }
        let offers = self.shelf.offers().unwrap_or_default();
        self.refresh(&offers);
        self.check(ctx, engine);
        let query_id = egui::Id::unique(GALLERY).with("query");
        // The rows own ↑↓⏎ while the search field (or nothing) has focus; a
        // value being typed keeps its keys.
        let typing = ctx.memory(|m| m.focused().is_some_and(|f| f != query_id));
        let keys = if typing {
            let escape = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
            if escape {
                ctx.memory_mut(|m| m.request_focus(query_id));
            }
            kit::SearchKeys::default()
        } else {
            kit::search_keys(ctx)
        };
        let open = self.open.as_mut()?;
        if keys.escape {
            self.open = None;
            return None;
        }
        let count = open.rows.len();
        if keys.down && count > 0 {
            open.highlighted = (open.highlighted + 1).min(count - 1);
            open.moved(&offers);
        }
        if keys.up && open.highlighted > 0 {
            open.highlighted -= 1;
            open.moved(&offers);
        }
        let mut primary = keys.enter;
        let (closed, chosen) = self.sheet(ctx, engine, &offers, &mut primary);
        if closed {
            self.open = None;
            return None;
        }
        if let Some(i) = chosen
            && let Some(open) = self.open.as_mut()
            && i != open.highlighted
        {
            open.highlighted = i;
            open.moved(&offers);
        }
        if primary {
            self.submit(ctx, engine);
        }
        None
    }

    /// Have the module check the values being edited (D1: its refusal is
    /// marked on the field as the value is edited, not only on Create):
    /// one check at a time, off the UI thread, again whenever the values
    /// moved on.
    fn check(&mut self, ctx: &egui::Context, engine: &Engine) {
        let Some(open) = self.open.as_mut() else {
            return;
        };
        if let Some(checking) = &mut open.checking
            && let Some(problems) = checking.poll()
        {
            open.checked = Some((checking.request.clone(), problems));
            open.checking = None;
        }
        let Some(request) = open.values.as_ref().map(|v| v.request(None)) else {
            return;
        };
        let current = open.checked.as_ref().is_some_and(|(r, _)| *r == request);
        if open.checking.is_none() && !current {
            let scope = open.scope.generator_scope();
            open.checking = generators::check(ctx, engine, request, scope);
        }
    }

    /// The module's problems with the values being edited, once it has
    /// checked exactly these.
    fn refused(&self) -> Option<Vec<ParamProblem>> {
        let open = self.open.as_ref()?;
        let request = open.values.as_ref()?.request(None);
        open.checked
            .as_ref()
            .filter(|(r, problems)| *r == request && !problems.is_empty())
            .map(|(_, problems)| problems.clone())
    }

    /// Where the form's part stands, and why the primary cannot act, if it
    /// cannot. Values the module refused when it checked them stand as
    /// invalid, in its words.
    fn standing(&self, engine: &Engine) -> Option<(Standing, Option<String>)> {
        let open = self.open.as_ref()?;
        let request = self.request()?;
        let offer = self.shelf.offer(&request.generator)?;
        let mut standing =
            generators::standing(engine, &offer, open.scope.generator_scope(), &request);
        if matches!(standing, Standing::New | Standing::Update { .. })
            && let Some(problems) = self.refused()
        {
            standing = Standing::Invalid { problems };
        }
        let blocked = if open.making.is_some() {
            Some("Making the part…".to_owned())
        } else if open.submitted.is_some() {
            Some("Waiting for the engine…".to_owned())
        } else if open.name.len() > MAX_LABEL {
            Some(format!("A name is at most {MAX_LABEL} bytes"))
        } else {
            match (&open.scope, &standing) {
                // A working set takes the package's content whatever the
                // project holds; only values the module refuses block it.
                (Scope::Package { .. }, Standing::Newer { .. }) => None,
                _ => sparksmith_core::catalog::generic::generators::refusal_of(&standing),
            }
        };
        Some((standing, blocked))
    }

    /// The primary: use the part the project holds, or make it.
    fn submit(&mut self, ctx: &egui::Context, engine: &Engine) {
        let Some((standing, blocked)) = self.standing(engine) else {
            return;
        };
        if blocked.is_some() {
            return;
        }
        let Some(request) = self.request() else {
            return;
        };
        let Some(open) = self.open.as_mut() else {
            return;
        };
        open.refusal = None;
        let (sent, into) = match &open.scope {
            Scope::NewComponent => {
                // A part the project holds, made by this version: nothing
                // to make.
                if let Standing::Held { .. } = standing
                    && let Some(component) =
                        sparksmith_core::catalog::generic::generators::component_of(&standing)
                {
                    if open.to_place {
                        self.to_place = Some(component);
                    } else {
                        self.created = Some(component.id);
                    }
                    self.open = None;
                    return;
                }
                let sent = if open.to_place {
                    Sent::CreateToPlace
                } else {
                    Sent::Create
                };
                (sent, Into::Part(GeneratorScope::Component))
            }
            Scope::Package { draft, pads } => (
                Sent::Package,
                Into::WorkingPackage {
                    draft: *draft,
                    replace: *pads > 0,
                },
            ),
        };
        let update = matches!(standing, Standing::Update { .. });
        match generators::make(ctx, engine, request.clone(), into) {
            Some(making) => open.making = Some((making, sent, request, update)),
            None => open.refusal = Some("The part could not be made: no worker thread".into()),
        }
    }

    /// Draw the sheet. Returns whether the user closed it and the row a
    /// click chose; sets `primary` when it was clicked.
    fn sheet(
        &mut self,
        ctx: &egui::Context,
        engine: &Engine,
        offers: &[Offer],
        primary: &mut bool,
    ) -> (bool, Option<usize>) {
        let t = Tokens::get();
        let area = ctx.content_rect();
        let width = (t.form_card_width * 3.0)
            .min(area.width() - t.space_16)
            .max(t.form_card_width);
        let top = pos2(area.center().x - width / 2.0, area.top() + t.space_24 * 2.0);
        let list_height = (area.height() - t.space_24 * 9.0).max(kit::search_row_height() * 4.0);
        let standing = self.standing(engine);
        let status = self.shelf.status();
        let Some(open) = self.open.as_mut() else {
            return (true, None);
        };
        let footer = if open.to_place {
            KEYS_PLACE
        } else if matches!(open.scope, Scope::NewComponent) {
            KEYS_NEW
        } else {
            KEYS_PACKAGE
        };
        let query_id = egui::Id::unique(GALLERY).with("query");
        let mut closed = false;
        let mut picked = None;
        let focus = std::mem::take(&mut open.focus);
        let scroll = std::mem::take(&mut open.scroll);
        kit::search_sheet(ctx, GALLERY, top, width, footer, |ui| {
            kit::search_field(
                ui,
                query_id,
                &mut open.query,
                "Find a part a generator makes: qfn 16, sot-23, header 2x5, 0603",
                false,
                focus,
                MAX_QUERY,
            );
            ui.add_space(t.space_4);
            ui.horizontal_wrapped(|ui| {
                let mut chips: Vec<(Option<String>, String)> = vec![(None, "All".to_owned())];
                chips.extend(
                    offers
                        .iter()
                        .map(|o| (Some(o.description.id.clone()), o.summary.name.clone())),
                );
                for (chip, label) in chips {
                    let style = if chip == open.chip {
                        Button::SmallSelected
                    } else {
                        Button::Small
                    };
                    if ui_kit::button(ui, &label, style).clicked() && chip != open.chip {
                        open.chip = chip;
                    }
                }
            });
            ui.add_space(t.space_4);
            let columns = ui.available_width();
            let left = (columns * 0.42).max(0.0);
            let right = (columns - left - t.space_12).max(0.0);
            ui.allocate_ui_with_layout(
                vec2(columns, list_height),
                Layout::left_to_right(Align::Min),
                |ui| {
                    ui.allocate_ui_with_layout(
                        vec2(left, list_height),
                        Layout::top_down(Align::Min),
                        |ui| {
                            ui.set_width(left);
                            if let Some(status) = &status {
                                kit::search_message(ui, status, false);
                            } else if open.rows.is_empty() {
                                kit::search_message(ui, "No generator makes this", false);
                            } else {
                                kit::search_heading(ui, "Generators", open.rows.len());
                                egui::ScrollArea::vertical()
                                    .id_salt(("generator-gallery-rows", open.chip.clone()))
                                    .max_height(list_height)
                                    .auto_shrink([false, true])
                                    .show(ui, |ui| {
                                        for (i, row) in open.rows.iter().enumerate() {
                                            let response = kit::search_row(
                                                ui,
                                                egui::Id::unique(GALLERY).with(("row", i)),
                                                &kit::SearchRow {
                                                    icon: if row.preset.is_some() {
                                                        "component"
                                                    } else {
                                                        "new"
                                                    },
                                                    label: &row.name,
                                                    query: &open.query,
                                                    detail: &row.detail,
                                                    badges: &[],
                                                    highlighted: i == open.highlighted,
                                                },
                                            );
                                            if scroll && i == open.highlighted {
                                                response.scroll_to_me(None);
                                            }
                                            if response.clicked() {
                                                picked = Some(i);
                                            }
                                            if response.double_clicked() {
                                                *primary = true;
                                            }
                                        }
                                    });
                            }
                        },
                    );
                    ui.add_space(t.space_12);
                    ui.allocate_ui_with_layout(
                        vec2(right, list_height),
                        Layout::top_down(Align::Min),
                        |ui| {
                            ui.set_width(right);
                            egui::ScrollArea::vertical()
                                .id_salt("generator-gallery-form")
                                .max_height(list_height)
                                .auto_shrink([false, true])
                                .show(ui, |ui| {
                                    closed =
                                        form_column(ui, open, offers, standing.as_ref(), primary);
                                });
                        },
                    );
                },
            );
        });
        (closed, picked)
    }
}

/// The toast of a made part.
fn made_text(name: &str, update: bool) -> String {
    if update {
        format!("Made the next revision of {name}; placed parts keep theirs until you update them")
    } else {
        format!("Made {name}: component, symbol, package, footprint and 3D model")
    }
}

/// The form column: the generator's form, the name, where the part
/// stands, and the primary. Returns whether Cancel was clicked.
fn form_column(
    ui: &mut Ui,
    open: &mut Open,
    offers: &[Offer],
    standing: Option<&(Standing, Option<String>)>,
    primary: &mut bool,
) -> bool {
    let Some(generator) = open.values.as_ref().map(|v| v.generator.clone()) else {
        kit::note(ui, "Pick a row to see its values");
        return false;
    };
    let Some(offer) = offers.iter().find(|o| o.description.id == generator) else {
        kit::note(ui, "This generator is not shipped");
        return false;
    };
    kit::status(
        ui,
        &format!("{} · version {}", offer.summary.name, offer.summary.version),
        Tone::Note,
    );
    let problems = match standing {
        Some((Standing::Invalid { problems }, _)) => problems.clone(),
        _ => Vec::new(),
    };
    if matches!(open.scope, Scope::NewComponent) {
        let before = open.name.clone();
        let too_long = open.name.len() > MAX_LABEL;
        kit::checked_text_row(
            ui,
            "Name",
            &mut open.name,
            "the generator's own name",
            too_long,
        );
        if open.name != before {
            open.named = true;
        }
    }
    let (standing, blocked) = match standing {
        Some((s, b)) => (Some(s), b.clone()),
        None => (None, None),
    };
    if let Some(standing) = standing {
        let name = open.name.trim();
        let (line, _) =
            generator_form::standing_line(standing, if name.is_empty() { "It" } else { name });
        match standing {
            Standing::Newer { .. } | Standing::Invalid { .. } => {}
            _ => {
                kit::note(ui, &line);
            }
        }
    }
    if let Scope::Package { pads, .. } = open.scope {
        kit::note(
            ui,
            &if pads > 0 {
                format!(
                    "Replaces the package, the footprint's {} and the model; terminals matched by number; one undo step",
                    kit::counted(pads, "pad", "pads")
                )
            } else {
                "Gives this component the package, its footprint and model; terminals matched by number".to_owned()
            },
        );
    }
    if let Some(why) = blocked.as_ref().or(open.refusal.as_ref()) {
        kit::refusal(ui, why);
    }
    let label = match (&open.scope, standing) {
        (Scope::NewComponent, Some(Standing::Held { .. })) if open.to_place => "Place".to_owned(),
        (Scope::NewComponent, Some(Standing::Held { .. })) => "Open component".to_owned(),
        (Scope::NewComponent, Some(Standing::Update { version, .. })) => {
            format!("Make again (version {version})")
        }
        (Scope::NewComponent, _) if open.to_place => "Create and place".to_owned(),
        (Scope::NewComponent, _) => "Create component".to_owned(),
        (Scope::Package { pads: 0, .. }, _) => "Use package".to_owned(),
        (Scope::Package { .. }, _) => "Replace package".to_owned(),
    };
    let cancel = actions(ui, &label, blocked.as_deref(), primary);
    // The values below the primary, so it stays in view however many the
    // generator has.
    kit::rule(ui);
    let Some(values) = open.values.as_mut() else {
        return cancel;
    };
    if generator_form::rows(ui, offer, values, &problems).changed {
        open.refusal = None;
        if !open.named {
            open.name = values
                .preset
                .as_deref()
                .and_then(|k| offer.preset(k))
                .map(|p| p.name.clone())
                .unwrap_or_default();
        }
    }
    cancel
}

/// The primary and Cancel; whether Cancel was clicked.
fn actions(ui: &mut Ui, label: &str, blocked: Option<&str>, primary: &mut bool) -> bool {
    let mut cancel = false;
    kit::actions(ui, |ui| {
        let response = kit::primary(ui, label, blocked.is_none());
        let response = match blocked {
            Some(why) => response.on_disabled_hover_text(why),
            None => response,
        };
        if response.clicked() {
            *primary = true;
        }
        if ui_kit::button(ui, "Cancel", Button::Normal).clicked() {
            cancel = true;
        }
    });
    cancel
}

/// The chip of a row by where its part stands.
fn standing_chip(standing: &Standing) -> Option<kit::RowChip<'static>> {
    match standing {
        Standing::Held { .. } => None,
        Standing::New => Some(kit::RowChip {
            label: "not yet created",
            warning: false,
        }),
        Standing::Update { .. } => Some(kit::RowChip {
            label: "newer version",
            warning: false,
        }),
        Standing::Newer { .. } | Standing::Invalid { .. } => Some(kit::RowChip {
            label: "cannot be made",
            warning: true,
        }),
    }
}

/// One generic part of a component search list: its name, its detail (or
/// why it cannot be made), badges saying how the text was read, and a chip
/// saying where it stands.
pub fn generic_row(
    ui: &mut Ui,
    id: egui::Id,
    candidate: &GeneratorCandidate,
    query: &str,
    highlighted: bool,
) -> egui::Response {
    let badges: Vec<kit::SearchBadge<'_>> = candidate
        .badges
        .iter()
        .map(|b| kit::SearchBadge {
            label: &b.label,
            present: true,
            tip: &b.tip,
        })
        .collect();
    kit::search_row_marked(
        ui,
        id,
        &kit::SearchRow {
            icon: if candidate.component().is_some() {
                "component"
            } else {
                "new"
            },
            label: &candidate.name,
            query,
            detail: &candidate.detail,
            badges: &badges,
            highlighted,
        },
        kit::RowMarks {
            star: false,
            chip: standing_chip(&candidate.standing),
        },
    )
}

/// One preset of a component search list: its name, its source and where
/// its part stands.
pub fn preset_row(
    ui: &mut Ui,
    id: egui::Id,
    hit: &PresetHit,
    query: &str,
    highlighted: bool,
) -> egui::Response {
    kit::search_row_marked(
        ui,
        id,
        &kit::SearchRow {
            icon: if matches!(hit.standing, Standing::Held { .. }) {
                "component"
            } else {
                "new"
            },
            label: &hit.name,
            query,
            detail: &hit.detail,
            badges: &[],
            highlighted,
        },
        kit::RowMarks {
            star: false,
            chip: standing_chip(&hit.standing),
        },
    )
}

#[cfg(test)]
#[path = "generator_gallery_tests.rs"]
mod tests;
