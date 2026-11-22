//! Copy and paste on a sheet. `schematic.copy` reads sheet objects into a
//! self-contained [`SheetClip`], so the objects may be gone (cut) by the
//! time `schematic.paste` recreates them as one undo step. Nets stay
//! separate: wire ends in the clip refer to clip objects or are free, and a
//! label joins by its name as any label does.
//!
//! A symbol names library revisions by the ids of the project it was copied
//! from, and another project can hold different revisions under the same
//! ids (two projects from one template). So a clip records each revision
//! its symbols reach with a digest of its content ([`ClipLibrary`]), and
//! each symbol's pads by terminal. A paste uses the destination's revisions
//! only when every digest matches; otherwise it imports the bundles a
//! portable copy carries, with fresh ids, or refuses. Every pasted symbol
//! must join the same pads to the same terminals as its source.
use super::{Result, invalid, labels, not_found, sheets, wiring};
use crate::diagnostics::Error;
use crate::geometry::Point;
use crate::ids::{
    ContentHash, DefinitionId, DefinitionKind, DefinitionRef, Endpoint, Id, SheetId, TerminalId,
};
use crate::instances::{self, NewInstance};
use crate::library::bundle::{self, Bundle, RefMap, SourceRevision};
use crate::library::{Library, import};
use crate::model::{
    ComponentInstance, Corner, FreeEnd, Label, LabelRef, Project, SheetItem, SymbolOccurrence,
    Wire, WireEnd,
};
use crate::units::PointNm;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Objects one `schematic.copy` may name.
pub const MAX_CLIP_OBJECTS: usize = 4096;

/// A sheet object to copy.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ClipObject {
    /// A symbol: the instance with its units on the sheet.
    Instance {
        id: Id,
    },
    Label {
        id: Id,
    },
    Drawing {
        id: Id,
    },
    Wire {
        id: Id,
    },
}

/// One copied symbol.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ClipSymbol {
    pub instance: ComponentInstance,
    /// Its units on the copied sheet; at least one.
    pub occurrences: Vec<SymbolOccurrence>,
    /// Which pads each terminal joins in the source, by pad number: what a
    /// paste must keep ([`terminal_pads`]). Empty for a part with no
    /// physical implementation.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pads: Vec<TerminalPads>,
}

/// The pads one terminal joins, by pad number.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TerminalPads {
    pub terminal: TerminalId,
    pub pads: Vec<String>,
}

/// The library revisions a clip's symbols reach, as the project they were
/// copied from holds them.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ClipLibrary {
    /// Every revision the symbols reach, with a digest of its content and
    /// its portable identity (`library::import::project_lineage_id`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub revisions: Vec<ClipRevision>,
    /// With a portable copy (`schematic.copy` with `portable`), the bundles
    /// that carry those revisions, so a project that holds other content
    /// under their ids imports them with fresh ids.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bundles: Vec<Bundle>,
}

impl ClipLibrary {
    pub fn is_empty(&self) -> bool {
        self.revisions.is_empty() && self.bundles.is_empty()
    }
}

/// One library revision a clip names.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ClipRevision {
    pub at: DefinitionRef,
    /// SHA-256 of its stored name, metadata and body, and a component's
    /// pending pin review ([`revision_digest`]).
    pub digest: ContentHash,
    pub source: SourceRevision,
}

/// What `schematic.paste` recreates. A wire's terminal and label ends name
/// objects of this clip; every other end is free.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SheetClip {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub symbols: Vec<ClipSymbol>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<Label>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub drawings: Vec<SheetItem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub wires: Vec<Wire>,
    /// What the symbols name in their library, so a paste never reads
    /// another project's revisions under the same ids.
    #[serde(default, skip_serializing_if = "ClipLibrary::is_empty")]
    pub library: ClipLibrary,
}

impl SheetClip {
    /// The point of its first object, the one a paste at a point puts there.
    pub fn anchor(&self) -> Option<Point> {
        let symbol = || {
            self.symbols
                .first()
                .and_then(|s| s.occurrences.first())
                .map(|o| o.position.to_mm_f64())
        };
        let label = || self.labels.first().map(|l| l.position.to_mm_f64());
        let drawing = || {
            self.drawings.first().map(|d| match d {
                SheetItem::Text { at, .. } => at.to_mm_f64(),
                SheetItem::Line { from, .. } => from.to_mm_f64(),
                SheetItem::Rect { min, .. } => min.to_mm_f64(),
            })
        };
        let wire = || {
            let wire = self.wires.first()?;
            wire.from
                .free_point()
                .or_else(|| wire.corners.first().map(Corner::at_mm))
                .or_else(|| wire.to.free_point())
        };
        symbol().or_else(label).or_else(drawing).or_else(wire)
    }
}

/// A symbol `schematic.paste` created.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PastedSymbol {
    pub instance: Id,
    /// Its first occurrence.
    pub occurrence: Id,
    pub designator: String,
}

/// What `schematic.paste` created. A wire listed here may be gone once the
/// engine's tidy joined it to another.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Pasted {
    pub symbols: Vec<PastedSymbol>,
    pub labels: Vec<Id>,
    pub drawings: Vec<Id>,
    pub wires: Vec<Id>,
}

/// Read `objects` of `sheet` into a clip. A wire comes along when it is
/// named, or when both its ends are on named symbols or labels; an end on
/// anything else becomes a free end where it is. The clip records the
/// library revisions its symbols reach; `portable` adds the bundles that
/// carry them, for a paste into another project.
pub fn copy(
    project: &Project,
    sheet: SheetId,
    objects: &[ClipObject],
    portable: bool,
) -> Result<SheetClip> {
    if objects.is_empty() {
        return Err(invalid("Name at least one object to copy."));
    }
    if objects.len() > MAX_CLIP_OBJECTS {
        return Err(Error::Limit("Copy at most 4096 objects at a time.".into()));
    }
    labels::check_sheet(project, sheet)?;
    let mut clip = SheetClip::default();
    let mut symbols = BTreeSet::new();
    let mut names = BTreeSet::new();
    let mut drawings = BTreeSet::new();
    let mut named_wires = BTreeSet::new();
    for object in objects {
        match *object {
            ClipObject::Instance { id } => {
                let instance = project
                    .instance(id)
                    .ok_or_else(|| not_found("Unknown component instance."))?;
                if !symbols.insert(id) {
                    continue;
                }
                let occurrences: Vec<SymbolOccurrence> = project
                    .occurrences_of(id)
                    .filter(|o| o.sheet == sheet)
                    .cloned()
                    .collect();
                if occurrences.is_empty() {
                    return Err(invalid("Only a symbol on this sheet can be copied."));
                }
                clip.symbols.push(ClipSymbol {
                    instance: instance.clone(),
                    occurrences,
                    pads: terminal_pads(&project.library, instance)?,
                });
            }
            ClipObject::Label { id } => {
                let label = project
                    .label(id)
                    .filter(|l| l.sheet == sheet)
                    .ok_or_else(|| not_found("No such label on this sheet."))?;
                if names.insert(id) {
                    let mut copied = label.clone();
                    // Where it is drawn, which a label placed by its pin
                    // does not store.
                    copied.position = super::sheet_point(labels::anchor(project, label))?;
                    copied.rotation = labels::current_rotation(project, id)?;
                    clip.labels.push(copied);
                }
            }
            ClipObject::Drawing { id } => {
                let drawing = project
                    .drawing(id)
                    .filter(|d| d.sheet == sheet)
                    .ok_or_else(|| not_found("No such drawing on this sheet."))?;
                if drawings.insert(id) {
                    clip.drawings.push(drawing.item.clone());
                }
            }
            ClipObject::Wire { id } => {
                project
                    .wire(id)
                    .filter(|w| w.sheet == sheet)
                    .ok_or_else(|| not_found("No such wire on this sheet."))?;
                named_wires.insert(id);
            }
        }
    }
    let kept = |end: WireEnd| match end {
        WireEnd::Terminal(endpoint) => symbols.contains(&endpoint.instance),
        WireEnd::Label(LabelRef { label }) => names.contains(&label),
        WireEnd::Junction(_) | WireEnd::SheetPin(_) | WireEnd::Free(_) => false,
    };
    let clipped = |end: WireEnd| {
        if kept(end) {
            return Ok(end);
        }
        wiring::end_position(project, end)
            .and_then(WireEnd::free)
            .ok_or_else(|| invalid("A wire end has no position on this sheet."))
    };
    for wire in project.wires.iter().filter(|w| w.sheet == sheet) {
        if named_wires.contains(&wire.id) || (kept(wire.from) && kept(wire.to)) {
            clip.wires.push(Wire {
                from: clipped(wire.from)?,
                to: clipped(wire.to)?,
                ..wire.clone()
            });
        }
    }
    // A label placed by its pin keeps doing so only with that pin's wire;
    // alone it stays where it is pasted.
    let wires = &clip.wires;
    for label in &mut clip.labels {
        let end = WireEnd::label(label.id);
        label.manual = label.manual || !wires.iter().any(|w| w.from == end || w.to == end);
    }
    clip.library = clip_library(&project.library, &clip.symbols, portable)?;
    Ok(clip)
}

/// The pads each terminal of `instance` joins, by pad number, through its
/// binding and footprint; empty without a physical implementation.
pub fn terminal_pads(library: &Library, instance: &ComponentInstance) -> Result<Vec<TerminalPads>> {
    let Some(physical) = instances::resolve(library, instance)?.physical else {
        return Ok(Vec::new());
    };
    let mut by_terminal: BTreeMap<TerminalId, Vec<String>> = BTreeMap::new();
    for pad in &physical.footprint.definition.pads {
        if let Some(terminal) = pad
            .terminal
            .and_then(|t| physical.binding.definition.terminal_of(t))
        {
            by_terminal
                .entry(terminal)
                .or_default()
                .push(pad.number.clone());
        }
    }
    Ok(by_terminal
        .into_iter()
        .map(|(terminal, mut pads)| {
            pads.sort();
            TerminalPads { terminal, pads }
        })
        .collect())
}

/// SHA-256 of a revision's stored metadata and body, and for a component
/// the pin review it was published with (`Library::reviews`): equal
/// digests under one id are the same revision with the same pins still to
/// review in any project, so a paste that reuses the destination's copy
/// never drops a pending review (review §11).
pub fn revision_digest(library: &Library, at: DefinitionRef) -> Result<ContentHash> {
    use sha2::{Digest, Sha256};
    let (meta, body) = library
        .definition(at)
        .ok_or_else(|| not_found("A copied part names a library revision that is missing."))?;
    let review = (at.kind == DefinitionKind::Component)
        .then(|| library.review_of(crate::ids::ComponentRef::new(at.id, at.revision)))
        .flatten()
        .filter(|r| !r.is_empty());
    let bytes = serde_json::to_vec(&(at, meta, body, review))
        .map_err(|e| Error::Invalid(format!("a definition could not be written: {e}")))?;
    let mut hasher = Sha256::new();
    hasher.update(b"sparksmith.clip-revision\0");
    hasher.update(&bytes);
    Ok(ContentHash(hasher.finalize().into()))
}

/// The roots one symbol reaches: its Part, else its component, and the
/// latest revision of each field lineage its properties set.
fn roots(library: &Library, instance: &ComponentInstance) -> Vec<DefinitionRef> {
    let mut out = vec![
        instance
            .part
            .map_or_else(|| instance.component.erased(), |p| p.erased()),
    ];
    for field in instance.properties.keys() {
        if let Some((DefinitionKind::Field, revision)) = library.latest(*field) {
            out.push(DefinitionRef {
                kind: DefinitionKind::Field,
                id: *field,
                revision,
            });
        }
    }
    out
}

fn clip_library(library: &Library, symbols: &[ClipSymbol], portable: bool) -> Result<ClipLibrary> {
    let mut roots_: Vec<DefinitionRef> = Vec::new();
    for symbol in symbols {
        for root in roots(library, &symbol.instance) {
            if !roots_.contains(&root) {
                roots_.push(root);
            }
        }
    }
    let mut members = BTreeSet::new();
    for root in &roots_ {
        members.extend(bundle::closure(library, *root)?);
    }
    let mut out = ClipLibrary::default();
    for at in members {
        out.revisions.push(ClipRevision {
            at,
            digest: revision_digest(library, at)?,
            source: SourceRevision {
                lineage: import::project_lineage_id(library, at.kind, at.id)?,
                revision: at.revision,
            },
        });
    }
    // A closure that may not travel (a 3D model whose licence forbids
    // sharing it) leaves the clip without bundles: it still pastes in this
    // project, and another project refuses it, saying why.
    if portable {
        let bundles: Result<Vec<Bundle>> = roots_
            .into_iter()
            .map(|root| import::project_bundle(library, root, "Copied parts"))
            .collect();
        out.bundles = bundles.unwrap_or_default();
    }
    Ok(out)
}

/// How a clip's library revisions read in the destination: as they are,
/// when the destination holds every one with the same content, or as the
/// copies the clip's bundles import. Refused when neither holds.
struct Mapped {
    revisions: BTreeMap<DefinitionRef, DefinitionRef>,
    lineages: BTreeMap<DefinitionId, DefinitionId>,
}

impl RefMap for Mapped {
    fn exact(&mut self, target: DefinitionRef) -> Result<DefinitionRef> {
        self.revisions
            .get(&target)
            .copied()
            .ok_or_else(|| invalid("A copied part names a revision the clip does not carry."))
    }
    fn lineage(&mut self, _: DefinitionKind, id: DefinitionId) -> Result<DefinitionId> {
        self.lineages
            .get(&id)
            .copied()
            .ok_or_else(|| invalid("A copied part names a lineage the clip does not carry."))
    }
}

/// `None` when the destination holds every revision of `clip` as it is.
fn map_library(project: &mut Project, clip: &ClipLibrary) -> Result<Option<Mapped>> {
    if clip.revisions.is_empty() {
        return Err(Error::Conflict(
            "This clip does not say which library revisions its parts are, so pasting it could change their pin mapping. Copy the parts again.".into(),
        ));
    }
    let same = clip
        .revisions
        .iter()
        .all(|r| revision_digest(&project.library, r.at).is_ok_and(|digest| digest == r.digest));
    if same {
        return Ok(None);
    }
    if clip.bundles.is_empty() {
        return Err(Error::Conflict(
            "These parts come from a project that holds other library revisions under the same ids, so pasting them here would change their pin mapping. Copy them with their library (a portable copy), or import the parts first.".into(),
        ));
    }
    let mut by_source: BTreeMap<SourceRevision, DefinitionRef> = BTreeMap::new();
    for bundle in &clip.bundles {
        let mut library = std::mem::take(&mut project.library);
        let mut allocate = || project.allocate().map(DefinitionId);
        let report = import::import(&mut library, bundle, &mut allocate);
        project.library = library;
        for r in report?.revisions {
            by_source.insert(
                SourceRevision {
                    lineage: r.record.origin.lineage,
                    revision: r.record.origin.revision,
                },
                r.record.local,
            );
        }
    }
    let mut mapped = Mapped {
        revisions: BTreeMap::new(),
        lineages: BTreeMap::new(),
    };
    for r in &clip.revisions {
        let local = by_source
            .get(&r.source)
            .copied()
            .ok_or_else(|| invalid("A copied part names a revision its bundles do not carry."))?;
        mapped.revisions.insert(r.at, local);
        mapped.lineages.insert(r.at.id, local.id);
    }
    Ok(Some(mapped))
}

/// `instance` with every library reference read through `map`.
fn remap(instance: &mut ComponentInstance, map: &mut Mapped) -> Result<()> {
    let component = map.exact(instance.component.erased())?;
    instance.component = crate::ids::ComponentRef::new(component.id, component.revision);
    if let Some(part) = instance.part {
        let r = map.exact(part.erased())?;
        instance.part = Some(crate::ids::PartRef::new(r.id, r.revision));
    }
    instance.representation = map.lineage(DefinitionKind::Symbol, instance.representation)?;
    if let Some(physical) = &mut instance.physical {
        physical.binding = map.lineage(DefinitionKind::Binding, physical.binding)?;
        physical.footprint = map.lineage(DefinitionKind::Footprint, physical.footprint)?;
    }
    let properties = std::mem::take(&mut instance.properties);
    for (field, value) in properties {
        instance
            .properties
            .insert(map.lineage(DefinitionKind::Field, field)?, value);
    }
    if let Some(spec) = &mut instance.marking.replace {
        bundle::marking_refs(spec, map)?;
    }
    Ok(())
}

/// Create `clip` on `sheet`, shifted by `offset`. The caller discards the
/// candidate on failure.
pub fn paste(
    project: &mut Project,
    sheet: SheetId,
    clip: SheetClip,
    offset: PointNm,
) -> Result<Pasted> {
    if clip == SheetClip::default() {
        return Err(invalid("There is nothing to paste."));
    }
    labels::check_sheet(project, sheet)?;
    let mut map = if clip.symbols.is_empty() {
        None
    } else {
        map_library(project, &clip.library)?
    };
    let shift = |p: PointNm| shifted_point(p, offset);
    let mut pasted = Pasted::default();
    let mut new_instances = BTreeMap::new();
    let mut new_labels = BTreeMap::new();
    for mut symbol in clip.symbols {
        let source = symbol.instance.id;
        if let Some(map) = &mut map {
            remap(&mut symbol.instance, map)?;
        }
        let pads = std::mem::take(&mut symbol.pads);
        let (id, occurrence) = paste_symbol(project, sheet, symbol, offset)?;
        // The copy joins the same pads to the same terminals, or nothing
        // is pasted.
        let placed = instances::instance_mut(project, id)?.clone();
        if terminal_pads(&project.library, &placed)? != pads {
            return Err(Error::Conflict(format!(
                "Pasting {} would join its pads to other terminals than the part it was copied from, so nothing was pasted.",
                placed.designator
            )));
        }
        new_instances.insert(source, id);
        pasted.symbols.push(PastedSymbol {
            instance: id,
            occurrence,
            designator: project
                .instance(id)
                .map(|i| i.designator.clone())
                .unwrap_or_default(),
        });
    }
    for label in clip.labels {
        let id = labels::place(
            project,
            labels::NewLabel {
                sheet,
                name: label.name,
                kind: label.kind,
                position: shift(label.position)?,
                rotation: label.rotation,
                attach: None,
            },
        )?;
        if !label.manual
            && let Some(placed) = project.labels.iter_mut().find(|l| l.id == id)
        {
            placed.manual = false;
        }
        new_labels.insert(label.id, id);
        pasted.labels.push(id);
    }
    for item in clip.drawings {
        pasted
            .drawings
            .push(sheets::add_drawing(project, sheet, shifted(item, offset)?)?);
    }
    let end = |end: WireEnd| -> Result<WireEnd> {
        let unknown = || invalid("A pasted wire ends on an object the clip does not hold.");
        match end {
            WireEnd::Terminal(endpoint) => Ok(WireEnd::Terminal(Endpoint {
                instance: *new_instances.get(&endpoint.instance).ok_or_else(unknown)?,
                terminal: endpoint.terminal,
            })),
            WireEnd::Label(LabelRef { label }) => {
                Ok(WireEnd::label(*new_labels.get(&label).ok_or_else(unknown)?))
            }
            WireEnd::Free(free) => shift(free.free)
                .map(|free| WireEnd::Free(FreeEnd { free }))
                .map_err(|_| invalid("A pasted wire end is off the sheet's range.")),
            WireEnd::Junction(_) | WireEnd::SheetPin(_) => Err(unknown()),
        }
    };
    for wire in clip.wires {
        pasted.wires.push(super::draw_wire(
            project,
            Some(sheet),
            end(wire.from)?.into(),
            wire.corners
                .iter()
                .map(|c| {
                    Ok(Corner {
                        at: shift(c.at)?,
                        ..*c
                    })
                })
                .collect::<Result<_>>()?,
            end(wire.to)?.into(),
        )?);
    }
    Ok(pasted)
}

/// A new instance of the symbol's component, with its next free designator,
/// everything else the instance holds, and its units in place.
fn paste_symbol(
    project: &mut Project,
    sheet: SheetId,
    symbol: ClipSymbol,
    offset: PointNm,
) -> Result<(Id, Id)> {
    let source = symbol.instance;
    let shift = |p: PointNm| shifted_point(p, offset);
    let mut units = symbol.occurrences.into_iter();
    let first = units
        .next()
        .ok_or_else(|| invalid("A pasted symbol needs a placed unit."))?;
    let (id, occurrence) = instances::place(
        project,
        NewInstance {
            component: source.component,
            designator: None,
            properties: Vec::new(),
            representation: Some(source.representation),
            unit: Some(first.unit),
            position: Some(shift(first.position)?),
            physical: source.physical,
            sheet: Some(sheet),
        },
    )?;
    orient(project, occurrence, &first);
    for unit in units {
        let added =
            instances::add_occurrence(project, id, unit.unit, shift(unit.position)?, sheet)?;
        orient(project, added, &unit);
    }
    // A copy is the same part ([`ComponentInstance::duplicate`]), on the
    // sheet: its units are drawn here, so it is not board-only.
    let instance = instances::instance_mut(project, id)?;
    let designator = std::mem::take(&mut instance.designator);
    *instance = ComponentInstance {
        board_only: false,
        ..source.duplicate(id, designator)
    };
    instances::check_instance(project, id)?;
    Ok((id, occurrence))
}

fn orient(project: &mut Project, occurrence: Id, like: &SymbolOccurrence) {
    if let Some(o) = project.occurrences.iter_mut().find(|o| o.id == occurrence) {
        o.rotation = like.rotation;
        o.mirror = like.mirror;
    }
}

/// `p` moved by `offset`; refused off the sheet's range.
fn shifted_point(p: PointNm, offset: PointNm) -> Result<PointNm> {
    let off = |_| invalid(super::POINT_RANGE);
    Ok(PointNm::new(
        p.x_nm.checked_add(offset.x_nm).map_err(off)?,
        p.y_nm.checked_add(offset.y_nm).map_err(off)?,
    ))
}

fn shifted(item: SheetItem, offset: PointNm) -> Result<SheetItem> {
    let shift = |p: PointNm| shifted_point(p, offset);
    Ok(match item {
        SheetItem::Text {
            at,
            text,
            height_nm,
            rotation,
        } => SheetItem::Text {
            at: shift(at)?,
            text,
            height_nm,
            rotation,
        },
        SheetItem::Line { from, to } => SheetItem::Line {
            from: shift(from)?,
            to: shift(to)?,
        },
        SheetItem::Rect { min, max } => SheetItem::Rect {
            min: shift(min)?,
            max: shift(max)?,
        },
    })
}
