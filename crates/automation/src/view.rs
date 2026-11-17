//! The window's module, `view` (`docs/extension-surface.md`, slice S5a):
//! what the window shows (`view.context`), selecting in it (`view.select`)
//! and opening a workspace, sheet or PCB view in it (`view.open`).
//!
//! The entries are declared beside [`crate::Request`] like every other, so
//! the catalog and `surface.lock` hold them; the window runs them through
//! the [`WindowHost`] it hands the front door ([`crate::CallCx::window`]).
//! A host without a window (the CLI, headless MCP) answers
//! `host.unavailable`. `view.select` and `view.open` are of the Show class:
//! they change what the person sees, never the document, take no revision
//! and no undo step, and rebuild no geometry.
use crate::front::Error;
use crate::gate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sparksmith_core::{DefinitionId, Engine, Id, ObjectRef, Project, SheetId};

/// The most objects one `view.select` names.
pub const MAX_SELECT: usize = 1024;

/// `view.context` takes nothing.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ContextParams {}

/// Select objects in the window, replacing the selection, and frame them,
/// in the shown workspace when it shows them, else in the one that does.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(try_from = "RawSelect", deny_unknown_fields)]
pub struct SelectParams {
    /// Project object ids: parts (the instance, or the symbol occurrence
    /// `component.place` returns), tracks, vias, fills, keepouts, wires,
    /// labels and sheet symbols. A designator given here ("U1") is read as
    /// one of `designators`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ids: Vec<Id>,
    /// Parts by designator ("U1", "r3"), compared case-insensitively.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub designators: Vec<String>,
    /// Objects by kind, as findings name them (a net, a pad, a sheet).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub objects: Vec<ObjectRef>,
}

/// Select objects in the window, replacing the selection, and frame them,
/// in the shown workspace when it shows them, else in the one that does.
/// (`view.select`'s parameters as sent, and its schema: an id may be a
/// designator.)
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RawSelect {
    /// Project object ids: parts (the instance, or the symbol occurrence
    /// `component.place` returns), tracks, vias, fills, keepouts, wires,
    /// labels and sheet symbols. A designator given here ("U1") is read as
    /// one of `designators`.
    #[serde(default)]
    ids: Vec<IdOrName>,
    /// Parts by designator ("U1", "r3"), compared case-insensitively.
    #[serde(default)]
    designators: Vec<String>,
    /// Objects by kind, as findings name them (a net, a pad, a sheet).
    #[serde(default)]
    objects: Vec<ObjectRef>,
}

/// An object id, or a part's designator.
#[derive(Deserialize, JsonSchema)]
#[serde(untagged)]
enum IdOrName {
    Id(Id),
    Name(String),
}

impl TryFrom<RawSelect> for SelectParams {
    type Error = String;

    fn try_from(raw: RawSelect) -> Result<Self, String> {
        let mut params = Self {
            ids: Vec::with_capacity(raw.ids.len()),
            designators: raw.designators,
            objects: raw.objects,
        };
        for id in raw.ids {
            match id {
                IdOrName::Id(id) => params.ids.push(id),
                IdOrName::Name(name) => match name.trim().parse::<Id>() {
                    Ok(id) => params.ids.push(id),
                    Err(_) => params.designators.push(name),
                },
            }
        }
        Ok(params)
    }
}

/// Show a workspace: the PCB in its Layout or 3D view, a schematic sheet,
/// or the Component editor on a draft.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OpenParams {
    pub workspace: Workspace,
    /// PCB only: Layout (2D) or 3D. The view shown stays when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<PcbView>,
    /// Schematic only: the sheet to show.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sheet: Option<SheetId>,
    /// Component only: a component draft's lineage to open.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub definition: Option<DefinitionId>,
}

/// The window's workspaces.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Workspace {
    Component,
    Schematic,
    Pcb,
}

/// The PCB workspace's two views of one design.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
pub enum PcbView {
    #[serde(rename = "layout")]
    Layout,
    #[serde(rename = "3d")]
    ThreeD,
}

/// What the window shows: `view.context`'s answer, and `view.open`'s.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Context {
    /// The project's revision now.
    pub revision: u64,
    pub workspace: Workspace,
    /// The PCB workspace's view.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub view: Option<PcbView>,
    /// The schematic sheet shown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sheet: Option<Sheet>,
    /// The shown workspace's selection.
    pub selection: Vec<Selected>,
}

/// A sheet, with its name.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Sheet {
    pub id: SheetId,
    pub name: String,
}

/// One selected object, with the name a person reads (a designator, a net
/// name) where it has one.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Selected {
    #[serde(flatten)]
    pub object: ObjectRef,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl Context {
    /// The context of a window showing `workspace` (with `view` and
    /// `sheet`) and selecting `selection` in `engine`'s project.
    pub fn new(
        engine: &Engine,
        workspace: Workspace,
        view: Option<PcbView>,
        sheet: Option<SheetId>,
        selection: Vec<ObjectRef>,
    ) -> Self {
        let project = engine.project();
        Self {
            revision: engine.revision(),
            workspace,
            view,
            sheet: sheet.and_then(|id| {
                project.sheet(id).map(|sheet| Sheet {
                    id,
                    name: sheet.name.clone(),
                })
            }),
            selection: selection
                .into_iter()
                .map(|object| Selected {
                    name: name_of(project, &object),
                    object,
                })
                .collect(),
        }
    }
}

fn name_of(project: &Project, object: &ObjectRef) -> Option<String> {
    match *object {
        ObjectRef::Instance { id } => project.instance(id).map(|i| i.designator.clone()),
        ObjectRef::Net { id } => project.net(id).map(|n| n.name.clone()),
        _ => None,
    }
}

/// What `view.select` selected.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Selection {
    /// Objects named.
    pub selected: usize,
    /// The workspace that shows them.
    pub workspace: Workspace,
}

/// The window, as the front door reaches it. The desktop app gives one for
/// each call it runs; a host without a window gives none. Each call gets
/// the engine the call runs on, which is the window's.
pub trait WindowHost {
    /// Why the person holds the window now (typing, dragging, a dialog),
    /// when they do: anything but a read waits.
    fn busy(&self) -> Option<String>;
    /// What the window shows.
    fn context(&self, engine: &Engine) -> Context;
    /// Select and frame `objects`, every one of which exists.
    fn select(&mut self, engine: &Engine, objects: &[ObjectRef]) -> Result<Selection, Box<Error>>;
    /// Show what `params` names, already checked against `engine`.
    fn open(&mut self, engine: &Engine, params: &OpenParams) -> Result<Context, Box<Error>>;
    /// Show a workflow's card in the chat (`assist.offer`), already checked
    /// against the workflows lent to the session; answer the card's id. A
    /// window without a chat answers `host.unavailable`.
    fn offer(&mut self, offer: &crate::workflow::Offer) -> Result<String, Box<Error>> {
        let _ = offer;
        Err(unavailable("assist.offer"))
    }
    /// Show a question card in the chat (`assist.ask`), already checked;
    /// answer the card's id. The chat holds the call until the person
    /// answers and replies with their answers. A window without a chat
    /// answers `host.unavailable`.
    fn ask(&mut self, ask: &crate::workflow::ask::AskParams) -> Result<String, Box<Error>> {
        let _ = ask;
        Err(unavailable("assist.ask"))
    }
}

/// A `view` request, as the front door runs it.
#[derive(Clone, Debug)]
pub enum Request {
    Context,
    Select(SelectParams),
    Open(OpenParams),
    Offer(crate::workflow::entries::OfferParams),
    Ask(crate::workflow::ask::AskParams),
}

impl Request {
    /// The `view` request `request` is, if it is one.
    pub fn of(request: &crate::Request) -> Option<Self> {
        match request {
            crate::Request::ViewContext(_) => Some(Self::Context),
            crate::Request::ViewSelect(params) => Some(Self::Select(params.clone())),
            crate::Request::ViewOpen(params) => Some(Self::Open(params.clone())),
            crate::Request::Offer(params) => Some(Self::Offer(params.clone())),
            crate::Request::AskPerson(params) => Some(Self::Ask(params.clone())),
            _ => None,
        }
    }
}

/// Run `request` in `window`, or answer `host.unavailable` without one.
/// `workflows` are those lent to the session, which `assist.offer` checks
/// its workflow and inputs against.
pub fn run(
    window: Option<&mut dyn WindowHost>,
    engine: &Engine,
    workflows: Option<&[crate::workflow::Listed]>,
    entry: &str,
    request: Request,
) -> Result<serde_json::Value, Box<Error>> {
    let window = window.ok_or_else(|| unavailable(entry))?;
    let value = match request {
        Request::Context => serde_json::to_value(window.context(engine)),
        Request::Select(params) => {
            let objects = objects(engine.project(), &params)?;
            serde_json::to_value(window.select(engine, &objects)?)
        }
        Request::Open(params) => {
            check_open(engine, &params)?;
            serde_json::to_value(window.open(engine, &params)?)
        }
        Request::Offer(params) => {
            let offer = check_offer(workflows, &params)?;
            let card = window.offer(&offer)?;
            serde_json::to_value(crate::workflow::entries::OfferReply {
                card,
                missing: offer.missing,
            })
        }
        Request::Ask(params) => {
            params.check().map_err(|why| {
                refused(
                    "assist.ask",
                    entry,
                    &format!("{why}. Nothing was shown."),
                    "Fix the card and ask again.",
                )
            })?;
            let card = window.ask(&params)?;
            serde_json::to_value(crate::workflow::ask::AskReply {
                card,
                ..Default::default()
            })
        }
    };
    value.map_err(|e| refused("malformed_request", entry, &e.to_string(), "Report it."))
}

/// `assist.offer`'s card, checked against the lent workflows.
fn check_offer(
    workflows: Option<&[crate::workflow::Listed]>,
    params: &crate::workflow::entries::OfferParams,
) -> Result<crate::workflow::Offer, Box<Error>> {
    const ENTRY: &str = "assist.offer";
    let listed = workflows.ok_or_else(|| {
        refused(
            "assist.no_workflows",
            ENTRY,
            "this host lent no workflows to this session. Nothing was shown.",
            "Carry on in the conversation.",
        )
    })?;
    crate::workflow::Offer::from_params(listed, params).map_err(|why| {
        refused(
            "assist.offer",
            ENTRY,
            &format!("{why}. Nothing was shown."),
            "List the workflows with query assist.workflows and offer one by its id, with inputs it has.",
        )
    })
}

/// `view.*` without a window.
pub fn unavailable(entry: &str) -> Box<Error> {
    refused(
        "host.unavailable",
        entry,
        concat!(
            "this host has no window: it steers the ",
            sparksmith_core::product_name!(),
            " window, which mcp --live reaches. Nothing was changed."
        ),
        concat!(
            "Connect to the open window (",
            sparksmith_core::product_short_name!(),
            " mcp --live), or carry on without it."
        ),
    )
}

/// The objects `params` names, each checked to exist: ids resolved to
/// their kind.
pub fn objects(project: &Project, params: &SelectParams) -> Result<Vec<ObjectRef>, Box<Error>> {
    const ENTRY: &str = "view.select";
    let count = params
        .ids
        .len()
        .saturating_add(params.designators.len())
        .saturating_add(params.objects.len());
    if count == 0 || count > MAX_SELECT {
        return Err(refused(
            "malformed_request",
            ENTRY,
            &format!("it names {count} objects; name 1 to {MAX_SELECT}. Nothing was changed."),
            "Send ids, designators or objects.",
        ));
    }
    let mut out = Vec::with_capacity(count);
    for &id in &params.ids {
        let object = object_of(project, id).ok_or_else(|| {
            refused(
                "view.unknown_object",
                ENTRY,
                &format!(
                    "no part, symbol, track, via, fill, keepout, wire, label or sheet symbol has id {id}. Nothing was selected."
                ),
                &format!(
                    "Name parts by designator ({}), or read the ids with query project.inspect; name a net, pad or sheet in objects.",
                    known_designators(project)
                ),
            )
        })?;
        out.push(object);
    }
    for designator in &params.designators {
        let instance = project
            .instance_by_designator(designator.trim())
            .ok_or_else(|| {
                refused(
                    "view.unknown_object",
                    ENTRY,
                    &format!(
                        "no part has the designator {:?}. Nothing was selected.",
                        designator.chars().take(64).collect::<String>()
                    ),
                    &format!("Name one the project has: {}.", known_designators(project)),
                )
            })?;
        out.push(ObjectRef::Instance { id: instance.id });
    }
    for object in &params.objects {
        if !exists(project, object) {
            return Err(refused(
                "view.unknown_object",
                ENTRY,
                &format!(
                    "{} names nothing in the project. Nothing was selected.",
                    serde_json::to_string(object).unwrap_or_default()
                ),
                "Read the project again; the object may have been deleted.",
            ));
        }
        out.push(*object);
    }
    Ok(out)
}

/// Designators a refusal lists at most.
const LISTED_DESIGNATORS: usize = 10;

/// The project's designators, for a refusal: up to ten, then how many more.
fn known_designators(project: &Project) -> String {
    let all: Vec<&str> = project
        .instances
        .iter()
        .map(|i| i.designator.as_str())
        .collect();
    if all.is_empty() {
        return "the project has no parts yet".into();
    }
    let mut listed = all[..all.len().min(LISTED_DESIGNATORS)].join(", ");
    if all.len() > LISTED_DESIGNATORS {
        listed.push_str(&format!(" and {} more", all.len() - LISTED_DESIGNATORS));
    }
    listed
}

/// The object a project object id names. A symbol occurrence's id (what
/// `component.place` returns beside the instance) names its part.
pub fn object_of(project: &Project, id: Id) -> Option<ObjectRef> {
    if project.instance(id).is_some() {
        Some(ObjectRef::Instance { id })
    } else if let Some(occurrence) = project.occurrence(id) {
        Some(ObjectRef::Instance {
            id: occurrence.instance,
        })
    } else if project.tracks.iter().any(|t| t.id == id) {
        Some(ObjectRef::Track { id })
    } else if project.vias.iter().any(|v| v.id == id) {
        Some(ObjectRef::Via { id })
    } else if project.fills.iter().any(|f| f.id == id) {
        Some(ObjectRef::Fill { id })
    } else if project.keepouts.iter().any(|k| k.id == id) {
        Some(ObjectRef::Keepout { id })
    } else if project.wire(id).is_some() {
        Some(ObjectRef::Wire { id })
    } else if project.label(id).is_some() {
        Some(ObjectRef::Label { id })
    } else if project.sheet_symbol(id).is_some() {
        Some(ObjectRef::SheetSymbol { id })
    } else {
        None
    }
}

/// Whether `object` names something in `project`. Kinds a window cannot
/// look up here (library objects, terminals) are left to its editors.
fn exists(project: &Project, object: &ObjectRef) -> bool {
    match *object {
        ObjectRef::Instance { id }
        | ObjectRef::Pad { instance: id, .. }
        | ObjectRef::Unit { instance: id, .. }
        | ObjectRef::Property { instance: id, .. } => project.instance(id).is_some(),
        ObjectRef::Net { id } => project.net(id).is_some(),
        ObjectRef::Sheet { sheet } => project.sheet(sheet).is_some(),
        ObjectRef::Label { id } => project.label(id).is_some(),
        ObjectRef::SheetSymbol { id } => project.sheet_symbol(id).is_some(),
        ObjectRef::Track { id }
        | ObjectRef::Via { id }
        | ObjectRef::Fill { id }
        | ObjectRef::Keepout { id }
        | ObjectRef::Wire { id } => object_of(project, id) == Some(*object),
        _ => true,
    }
}

/// `view.open`'s parameters fit its workspace and name what exists.
fn check_open(engine: &Engine, params: &OpenParams) -> Result<(), Box<Error>> {
    const ENTRY: &str = "view.open";
    let misplaced = |field: &str, workspace: &str| {
        refused(
            "malformed_request",
            ENTRY,
            &format!("{field} belongs to the {workspace} workspace. Nothing was changed."),
            &format!("Send workspace: {workspace} with it, or leave {field} out."),
        )
    };
    if params.view.is_some() && params.workspace != Workspace::Pcb {
        return Err(misplaced("view", "pcb"));
    }
    if params.sheet.is_some() && params.workspace != Workspace::Schematic {
        return Err(misplaced("sheet", "schematic"));
    }
    if params.definition.is_some() && params.workspace != Workspace::Component {
        return Err(misplaced("definition", "component"));
    }
    if let Some(sheet) = params.sheet
        && engine.project().sheet(sheet).is_none()
    {
        return Err(refused(
            "view.unknown_object",
            ENTRY,
            &format!("the project has no sheet {sheet}. Nothing was changed."),
            "Read the sheets with query project.inspect.",
        ));
    }
    if let Some(definition) = params.definition
        && engine.drafts().get(definition).is_none()
    {
        return Err(refused(
            "view.not_a_draft",
            ENTRY,
            &format!(
                "{definition} is not a component draft of this project; opening a published one would draft it, which changes the project. Nothing was changed."
            ),
            "Draft it with apply first, or open one of the drafts project.inspect lists.",
        ));
    }
    Ok(())
}

fn refused(code: &str, entry: &str, why: &str, instead: &str) -> Box<Error> {
    Box::new(gate::refusal(code, entry, why, instead))
}

/// A window for tests: it keeps what it was asked to show.
#[cfg(test)]
#[derive(Clone, Debug)]
pub(crate) struct Fake {
    pub busy: Option<String>,
    pub workspace: Workspace,
    pub view: Option<PcbView>,
    pub sheet: Option<SheetId>,
    pub selection: Vec<ObjectRef>,
    /// The workflow cards shown, oldest first.
    pub offers: Vec<crate::workflow::Offer>,
    /// The question cards shown, oldest first.
    pub asks: Vec<crate::workflow::ask::AskParams>,
}

#[cfg(test)]
impl Fake {
    /// The PCB layout, nothing selected, the person not busy.
    pub fn idle() -> Self {
        Self {
            busy: None,
            workspace: Workspace::Pcb,
            view: Some(PcbView::Layout),
            sheet: None,
            selection: Vec::new(),
            offers: Vec::new(),
            asks: Vec::new(),
        }
    }
    /// Held by the person for `reason`, or idle.
    pub fn held(reason: Option<&str>) -> Self {
        Self {
            busy: reason.map(str::to_owned),
            ..Self::idle()
        }
    }
}

#[cfg(test)]
impl WindowHost for Fake {
    fn busy(&self) -> Option<String> {
        self.busy.clone()
    }
    fn context(&self, engine: &Engine) -> Context {
        Context::new(
            engine,
            self.workspace,
            self.view,
            self.sheet,
            self.selection.clone(),
        )
    }
    fn select(&mut self, _: &Engine, objects: &[ObjectRef]) -> Result<Selection, Box<Error>> {
        self.selection = objects.to_vec();
        Ok(Selection {
            selected: objects.len(),
            workspace: self.workspace,
        })
    }
    fn offer(&mut self, offer: &crate::workflow::Offer) -> Result<String, Box<Error>> {
        self.offers.push(offer.clone());
        Ok(format!("card-{}", self.offers.len()))
    }
    fn ask(&mut self, ask: &crate::workflow::ask::AskParams) -> Result<String, Box<Error>> {
        self.asks.push(ask.clone());
        Ok(format!("question-{}", self.asks.len()))
    }
    fn open(&mut self, engine: &Engine, params: &OpenParams) -> Result<Context, Box<Error>> {
        self.workspace = params.workspace;
        self.view = (params.workspace == Workspace::Pcb)
            .then(|| params.view.or(self.view).unwrap_or(PcbView::Layout));
        self.sheet = params
            .sheet
            .filter(|_| params.workspace == Workspace::Schematic);
        Ok(self.context(engine))
    }
}

#[cfg(test)]
mod tests;
