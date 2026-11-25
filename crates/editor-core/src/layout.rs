//! Placement and manual routing over the electrical document. Geometry here is
//! electronics checking geometry; native 3D remains in the Solidcraft adapter.
use crate::diagnostics::{Error, Finding, ObjectRef, Result};
use crate::geometry::{Point, Shape, Transform2, point_valid};
use crate::ids::{Endpoint, Id, PadId};
use crate::instances;
use crate::library::{PAD_ARC_TOLERANCE_MM, PadCopper, PadKind, PadShape};
use crate::model::{
    CopperLayer, EditPolicy, Fabrication, Layer, PhysicalChoice, Placement, Project, Stackup, Track,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

/// Initial board rule, in mm. A future rules document replaces this explicit
/// prototype default; it must not be mistaken for a manufacturer qualification.
pub const CLEARANCE_MM: f64 = 0.2;
const EPS: f64 = 1e-8;
/// Where a part's datum may lie, mm, on both axes: the working area. A
/// part may sit off the board (the board's surroundings are its staging
/// area); DRC reports its copper past the edge. Every board lies within
/// 0–1000 mm, so the area leaves at least one largest board's width free
/// on every side, and no board size or outline edit can strand a part.
pub const WORKING_AREA_MM: [f64; 2] = [-1000.0, 2000.0];

pub mod arrange;
pub mod blocks;
pub mod board_edge;
pub mod board_text;
pub mod bodies;
pub mod connectivity;
mod copper;
pub mod courtyard;
pub mod dimensions;
pub mod etch;
pub mod fab_stackup;
pub mod fiducial;
pub mod fill;
pub mod grid;
pub mod impedance;
pub mod legend;
pub mod length;
pub mod limits;
pub mod mask_colour;
pub mod mask_web;
pub mod meander;
pub mod obstacles;
pub mod outline;
mod pad_hole;
pub mod pair;
mod part;
pub mod physical;
mod placed;
pub mod posture;
pub mod profile_fit;
pub mod restack;
pub mod router;
pub mod several;
pub mod showcase;
pub mod staging;
pub mod via;
pub mod via_class;
pub mod via_fill;
pub use crate::rules::signal::CheckSettings;
pub use arrange::part_envelope;
pub use connectivity::{CopperGraph, NetCounts};
#[cfg(test)]
pub(crate) use copper::limit_finding as copper_limit_finding;
pub use copper::{BoardCopper, CURVE_SLACK_MM, CopperChanges, CopperRing, ItemKey, Items};
#[cfg(test)]
pub(crate) use courtyard::limit_finding as courtyard_limit_finding;
pub use courtyard::{PlacedCourtyard, placed_courtyards};
pub use pad_hole::{PadHoleIndex, items as pad_hole_items};
pub use placed::{
    BoardDrawing, GraphicOwner, PlacedBridge, PlacedGraphic, PlacedHole, PlacedItem, PlacedKeepout,
    PlacedPad, Whose, board_text_graphic, model_to_board, pick_graphic, pick_pad, placed_bridges,
    placed_graphics, placed_keepouts, placed_pads, placement_transform, seat_height,
};
pub use via::{LayerChangeRequest, ViaRequest, via_pad_clearance_conflict};
/// Nearby copper pairs one check may compare per shape on average, and
/// at least [`MIN_CHECK_PAIRS`] in all. Pairs are only shapes whose bounds
/// come within [`copper::check_reach`] (the bounds grids never offer
/// farther ones), so a routed board of any size stays far below it: a
/// shape has a handful of neighbours, not hundreds. Only copper piled on
/// copper (a file with thousands of shapes stacked on one spot, which
/// would make a check quadratic) reaches it. It bounds density, never how
/// many objects a design has.
const CHECK_PAIRS_PER_SHAPE: usize = 256;
/// The pair budget of a check over few shapes.
const MIN_CHECK_PAIRS: usize = 1_000_000;

/// The most nearby pairs a check over `shapes` copper shapes compares
/// ([`CHECK_PAIRS_PER_SHAPE`]).
pub(crate) fn check_pair_budget(shapes: usize) -> usize {
    shapes
        .saturating_mul(CHECK_PAIRS_PER_SHAPE)
        .max(MIN_CHECK_PAIRS)
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "command", content = "params", deny_unknown_fields)]
pub enum LayoutCommand {
    /// Stage every board-placeable instance without a placement on the top
    /// side, as one undo step. Existing placements and routing are never
    /// replaced; refused when they do not fit.
    #[serde(rename = "layout.sync")]
    Sync,
    /// Place an instance's footprint on the board at `position`, `rotation_deg`
    /// and `layer`.
    #[serde(rename = "layout.place")]
    Place {
        instance: Id,
        position: Point,
        rotation_deg: f64,
        layer: Layer,
    },
    /// Move a placed instance to a new `position`, `rotation_deg` and `layer`.
    /// The unlocked tracks and vias attached to its pads come with it in the
    /// same undo step, as for a part its dimensions move.
    #[serde(rename = "layout.move")]
    Move {
        instance: Id,
        position: Point,
        rotation_deg: f64,
        layer: Layer,
    },
    /// Lock or unlock an instance's placement.
    #[serde(rename = "layout.placement_lock")]
    SetPlacementLock { instance: Id, locked: bool },
    /// Route one track of a net from `start` to `end` at `width` on one copper
    /// layer, checked against other nets' copper.
    #[serde(rename = "layout.route")]
    Route { request: RouteRequest },
    /// Apply a route edit set (tracks and vias to add, replace and remove) as
    /// one transaction, checked as a whole.
    #[serde(rename = "layout.route_apply")]
    ApplyRoute { edits: RouteEditSet },
    /// Delete a track.
    #[serde(rename = "layout.track_delete")]
    DeleteTrack { id: Id },
    /// Lock or unlock a track.
    #[serde(rename = "layout.track_lock")]
    SetTrackLock { id: Id, locked: bool },
    /// Change one track's width, copper layer or both, as one undo step.
    /// The track keeps its identity, net and ends. The edit goes through
    /// the same checks as `layout.route_apply` (a locked track, a width
    /// outside 0.05–5 mm, copper outside the board or a clearance
    /// violation is refused and nothing changes); omitting both is refused.
    #[serde(rename = "layout.track_set")]
    SetTrack {
        id: Id,
        #[serde(default)]
        width: Option<f64>,
        #[serde(default)]
        layer: Option<CopperLayer>,
    },
    /// Replace the board's fabrication attributes (mask colour and finish,
    /// pad finish, silkscreen colour) as one undo step.
    #[serde(rename = "layout.set_fabrication")]
    SetFabrication(Fabrication),
    /// Replace the board's stackup as one undo step; copper layers keep
    /// their names. Copper (tracks, fills, keepouts, blind vias) on a layer
    /// the new stackup lacks makes it fail (`layout.edit_stackup` moves it).
    #[serde(rename = "layout.set_stackup")]
    SetStackup(Stackup),
    /// Replace the board's stackup and move copper between layers, as one undo
    /// step ([`restack::StackupEdit`]). Every track, fill, keepout and
    /// blind-via end on a moved layer moves with it, all at once. Copper left
    /// on a layer the new stackup lacks refuses the edit.
    #[serde(rename = "layout.edit_stackup")]
    EditStackup(restack::StackupEdit),
    /// Take a fabricator's stackup (an id `layout.stackups` lists) as one undo
    /// step. `layout.stackups {id}` says first what it changes, what refuses
    /// it and what the board's fab profile says of it. Copper layers keep
    /// their place from the top and the bottom stays the bottom; `moves`
    /// sends the copper of a layer the stackup lacks to one it has. A layer
    /// whose sheets state several Dk takes their average by thickness, with
    /// the sheets it is calculated from. The board keeps the stackup's id and
    /// name until a layer is edited. A host with the stackup library carries this out
    /// as `layout.edit_stackup`.
    #[serde(rename = "layout.use_stackup")]
    UseStackup {
        stackup: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        moves: Vec<restack::LayerMove>,
    },
    /// Add a board via; the outcome names it (`Outcome::ViaAdded`).
    #[serde(rename = "layout.add_via")]
    AddVia { via: ViaRequest },
    /// Move a via to `position`.
    #[serde(rename = "layout.via_move")]
    MoveVia { id: Id, position: Point },
    /// Delete a via.
    #[serde(rename = "layout.via_delete")]
    DeleteVia { id: Id },
    /// Cover a via with solder mask (`tented`) or leave it open.
    #[serde(rename = "layout.via_tent")]
    SetViaTented { id: Id, tented: bool },
    /// Set what closes the holes of vias: `none` leaves them open, `capped`
    /// fills and plates them over. One undo step, refused whole.
    #[serde(rename = "layout.via_set")]
    SetVias {
        ids: Vec<Id>,
        fill: crate::model::ViaFill,
    },
    /// Lock or unlock a via.
    #[serde(rename = "layout.via_lock")]
    SetViaLock { id: Id, locked: bool },
    /// Route to a via, change layer there and route on, as one undo step;
    /// the outcome names the via (`Outcome::ViaAdded`).
    #[serde(rename = "layout.route_layer_change")]
    RouteLayerChange { request: LayerChangeRequest },
    /// Resize the rectangular board outline (mm, origin at the lower-left
    /// corner). Placements, tracks and vias must stay inside it.
    #[serde(rename = "layout.set_board_size")]
    SetBoardSize { size: Point },
    /// Route both halves of a differential pair along a centre line, as one
    /// undo step. 45° fan-out from the start pads, both halves coupled at the
    /// pair gap with mitred corners, a via pair where consecutive legs change
    /// layer, fan-in to the end pads. Width and gap come from the pair's class
    /// (derived from its impedance target per layer) unless given. Checked like
    /// `layout.route_apply`.
    #[serde(rename = "layout.route_pair")]
    RoutePair { request: pair::PairRouteRequest },
    /// Meander one straight track segment to add an exact length, or to bring
    /// its net to a target length, as one undo step. The fewest bumps within
    /// the amplitude, on the side with room, legs at least the track clearance
    /// apart; refused when no meander fits.
    #[serde(rename = "layout.tune_length")]
    TuneLength { request: meander::TuneRequest },
    /// One interactive route step in a routing mode: ignore, walkaround or
    /// shove (the default). In `layout::router`, `ignore` routes straight
    /// (refused on a clearance violation), `walkaround` goes round other nets'
    /// copper at the clearance, and `shove` pushes their tracks and vias aside.
    /// One undo step with every shoved track and via; the head stops at the
    /// last legal point when no way exists, and a step that cannot move it at
    /// all is refused. `max_detour` bounds the path against the straight line;
    /// the outcome reports where the head ended and its `detour_factor`
    /// (`Outcome::RouteStepped`). `layout.route_preview` returns the same plan
    /// unapplied.
    #[serde(rename = "layout.route_step")]
    RouteStep { request: router::RouteStepRequest },
    /// Drag a track segment along its normal by `offset` mm, shoving other
    /// nets' copper aside. In `layout::router::drag`, end-to-end neighbours on
    /// its net stretch, free ends follow, held ends jog, and other nets'
    /// tracks and vias are shoved in `mode`. One undo step with every moved track and via; the
    /// segment stops at the last legal offset when no room exists, and a
    /// drag that cannot move at all is refused. The outcome gives the
    /// offset reached (`Outcome::TrackDragged`); `layout.track_drag_preview`
    /// returns the plan unapplied.
    #[serde(rename = "layout.track_drag")]
    TrackDrag {
        request: router::drag::TrackDragRequest,
    },
    /// Place a new instance of a mechanical part spec, finding or making its
    /// variant, as one undo step (`Outcome::PartPlaced`). The variant is its
    /// component, mechanical symbol, package using the part file, footprint and
    /// binding (docs/mechanical-parts.md); its unit stays off-sheet.
    #[serde(rename = "layout.place_part")]
    PlacePart {
        part: Box<crate::library::mechanical::PartSpec>,
        position: Point,
        #[serde(default)]
        rotation_deg: f64,
        layer: Layer,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        designator: Option<String>,
    },
    /// Move a placed mechanical part to the variant of another spec, found or
    /// made, as one undo step. It keeps its identity, designator, placement and
    /// marking (`Outcome::PartPlaced`).
    #[serde(rename = "layout.set_part")]
    SetPart {
        instance: Id,
        part: Box<crate::library::mechanical::PartSpec>,
    },
    /// Remove a mechanical part with no sheet occurrence and no connected
    /// terminal, with its placement, as one undo step.
    #[serde(rename = "layout.remove_part")]
    RemovePart { instance: Id },
    /// Move several placed parts to new poses as one undo step (a group
    /// drag or flip). The unlocked copper attached to any of them is
    /// carried at once, so a track between two parts that move alike moves
    /// with both. A locked part refuses the whole move. `copper` names
    /// selected tracks and vias that slide with the parts (those attached
    /// to a moved part are carried with it); they are re-routed with the
    /// carried copper, and a locked one refuses the whole move.
    #[serde(rename = "layout.move_parts")]
    MoveParts {
        moves: Vec<arrange::PartMove>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        copper: Option<arrange::CopperMove>,
    },
    /// Line up two or more parts' outlines on one edge (left, right, top,
    /// bottom) or centre line, as one undo step with their copper carried.
    /// A part that would move and is locked, or whose dimensions hold it
    /// along that axis, refuses the command and is named.
    #[serde(rename = "layout.align")]
    Align {
        instances: Vec<Id>,
        to: arrange::AlignTo,
    },
    /// Space three or more parts evenly along an axis: the outermost stay
    /// and the gaps between neighbouring outlines become equal. One undo
    /// step, refused like `layout.align`.
    #[serde(rename = "layout.distribute")]
    Distribute {
        instances: Vec<Id>,
        axis: arrange::Axis,
    },
    /// Delete several tracks, vias and fills as one undo step (a
    /// multi-selection's Delete, or a dangling stub's tracks). A locked
    /// track or via refuses the whole command. Named parts are kept, as
    /// they come from the schematic: the outcome (`parts_kept`) names
    /// them. Naming only parts is refused.
    #[serde(rename = "layout.delete_objects")]
    DeleteObjects { objects: several::Objects },
    /// Lock or unlock several parts, tracks, vias and fills as one undo
    /// step.
    #[serde(rename = "layout.lock_objects")]
    LockObjects {
        objects: several::Objects,
        locked: bool,
    },
    /// Add text to the board's silkscreen; the outcome names it
    /// (`Outcome::TextAdded`). Example:
    /// `{"text":{"text":"Sensor node rev A","position":{"x_nm":20000000,"y_nm":5000000},"height_nm":1000000,"side":"Top"}}`.
    #[serde(rename = "layout.text_add")]
    AddText {
        text: Box<crate::model::BoardTextSpec>,
    },
    /// Replace a board text with an edited one of the same id: its words,
    /// place, height, turn or side.
    #[serde(rename = "layout.text_edit")]
    EditText { text: Box<crate::model::BoardText> },
    /// Delete a board text.
    #[serde(rename = "layout.text_delete")]
    DeleteText { id: Id },
    /// Record the track pieces an import refused, replacing those
    /// recorded: copper left ending where one ran says why
    /// (`board.track_dangling`).
    #[serde(rename = "layout.set_refused_import_tracks")]
    SetRefusedImportTracks {
        tracks: Vec<crate::model::import_review::RefusedTrack>,
    },
}

// A new command chooses its permission class here.
declare_entries!(LayoutCommand in layout {
    Sync "layout.sync" Edit "Sync the board with the schematic",
    Place "layout.place" Edit "Place a part on the board",
    Move "layout.move" Edit "Move a placed part",
    SetPlacementLock "layout.placement_lock" Edit "Lock or unlock a placement",
    Route "layout.route" Edit "Route a track",
    ApplyRoute "layout.route_apply" Edit "Apply a routed path",
    DeleteTrack "layout.track_delete" Edit "Delete a track",
    SetTrackLock "layout.track_lock" Edit "Lock or unlock a track",
    SetTrack "layout.track_set" Edit "Change a track",
    SetFabrication "layout.set_fabrication" Edit "Set fabrication settings",
    SetStackup "layout.set_stackup" Edit "Set the layer stackup" [since(2, 10)],
    EditStackup "layout.edit_stackup" Edit "Edit the layer stackup" [since(2, 10)],
    UseStackup "layout.use_stackup" Edit "Use a fabricator's stackup" [since(3, 0)],
    AddVia "layout.add_via" Edit "Add a via" [returns(via_added)],
    MoveVia "layout.via_move" Edit "Move a via",
    DeleteVia "layout.via_delete" Edit "Delete a via",
    SetViaTented "layout.via_tent" Edit "Tent or untent a via",
    SetVias "layout.via_set" Edit "Fill vias or leave them open" [since(3, 0)],
    SetViaLock "layout.via_lock" Edit "Lock or unlock a via",
    RouteLayerChange "layout.route_layer_change" Edit "Change layer while routing" [returns(via_added)],
    SetBoardSize "layout.set_board_size" Edit "Set the board size",
    RoutePair "layout.route_pair" Edit "Route a differential pair",
    TuneLength "layout.tune_length" Edit "Tune a track's length",
    RouteStep "layout.route_step" Edit "Route one step" [returns(route_stepped)],
    TrackDrag "layout.track_drag" Edit "Drag a track with shove" [returns(track_dragged)],
    PlacePart "layout.place_part" Edit "Place a mechanical part" [returns(part_placed)],
    SetPart "layout.set_part" Edit "Change a mechanical part" [returns(part_placed)],
    RemovePart "layout.remove_part" Edit "Remove a mechanical part",
    MoveParts "layout.move_parts" Edit "Move several parts together",
    Align "layout.align" Edit "Align parts",
    Distribute "layout.distribute" Edit "Distribute parts evenly",
    DeleteObjects "layout.delete_objects" Edit "Delete several board objects" [returns(none, parts_kept)],
    LockObjects "layout.lock_objects" Edit "Lock or unlock several board objects",
    SetRefusedImportTracks "layout.set_refused_import_tracks" Edit "Record tracks an import refused" [since(0, 3)],
    AddText "layout.text_add" Edit "Add text to the silkscreen" [since(4, 2), returns(text_added)],
    EditText "layout.text_edit" Edit "Edit a board text" [since(4, 2)],
    DeleteText "layout.text_delete" Edit "Delete a board text" [since(4, 2)],
});

impl LayoutCommand {
    /// Whether the command places or re-points a mechanical part (its
    /// outcome is `Outcome::PartPlaced`).
    pub fn places_part(&self) -> bool {
        matches!(self, Self::PlacePart { .. } | Self::SetPart { .. })
    }

    /// Whether the command adds a board text (its outcome is
    /// `Outcome::TextAdded`).
    pub fn adds_text(&self) -> bool {
        matches!(self, Self::AddText { .. })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RouteRequest {
    pub net: Id,
    pub start: Point,
    pub end: Point,
    pub width: f64,
    pub layer: CopperLayer,
}

/// A speculative edit set. A shove (`router`) replaces, adds and removes
/// other nets' tracks and moves their vias in the same transaction. Nothing is committed until references, locks and
/// obstacle checks pass for the entire candidate.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RouteEditSet {
    pub add: Vec<RouteRequest>,
    pub replace: Vec<Track>,
    pub remove: Vec<Id>,
    /// Vias to add with the tracks (a route step's layer change). Each is
    /// checked against other nets' copper once the tracks are in place.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub vias: Vec<ViaRequest>,
    /// Vias to remove with the tracks (unwinding a route step); a locked
    /// via refuses the whole edit.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remove_vias: Vec<Id>,
    /// Tracks put back with their former identities (undoing a shove that
    /// merged a chain's segments): each identity must be free and issued
    /// before.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub restore: Vec<Track>,
    /// Vias moved with the tracks (a shove); a locked via refuses the
    /// whole edit. Each is checked against other nets' copper once the
    /// tracks are in place.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub move_vias: Vec<ViaMove>,
}

/// A via's new position in a routing edit.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ViaMove {
    pub id: Id,
    pub position: Point,
}

pub trait RoutingStrategy {
    fn propose(&self, project: &Project, request: RouteRequest) -> Result<RouteEditSet>;
}

pub struct ManualRouter;
impl RoutingStrategy for ManualRouter {
    fn propose(&self, project: &Project, request: RouteRequest) -> Result<RouteEditSet> {
        check_request(project, &request)?;
        Ok(RouteEditSet {
            add: vec![request],
            ..Default::default()
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct WorldPad {
    pub instance: Id,
    pub pad: PadId,
    pub terminal: Endpoint,
    pub net: Id,
    pub position: Point,
    /// The pad shape's extent (pad frame), for summaries; geometry reads
    /// `shape` through [`WorldPad::outline`].
    pub size: Point,
    /// The exact pad shape (pad frame).
    pub shape: PadShape,
    pub rotation_deg: f64,
    /// The copper layer of a surface pad, after its side flip. A plated
    /// through-hole pad records the placement layer and spans every layer.
    pub layer: Layer,
    /// Copper on every layer around a plated barrel.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub through: bool,
    /// Placed on the bottom: the pad frame is reflected across its x-axis
    /// before `rotation_deg` turns it, as the placement transform does.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub mirrored: bool,
}

/// Whether a pad of `kind` whose copper side is `side` has copper on
/// `layer`: a plated through pad on every layer, a surface pad on its own
/// side, a bare hole on none. The one answer to which copper layers a pad
/// has; every pad type below asks it.
pub fn pad_copper_on(kind: PadKind, side: Layer, layer: CopperLayer) -> bool {
    match kind.copper() {
        PadCopper::None => false,
        PadCopper::Through => true,
        PadCopper::OneSide => CopperLayer::from(side) == layer,
    }
}

impl WorldPad {
    /// Whether the pad has copper on `layer` ([`pad_copper_on`]).
    pub fn on(&self, layer: impl Into<CopperLayer>) -> bool {
        let kind = if self.through {
            PadKind::PlatedThrough
        } else {
            PadKind::Smd
        };
        pad_copper_on(kind, self.layer, layer.into())
    }

    /// Pad frame to board coordinates (reflected first on the bottom).
    pub fn frame(&self) -> Transform2 {
        Transform2 {
            translate: self.position,
            rotation_deg: self.rotation_deg,
            mirror: self.mirrored,
        }
    }

    /// The exact copper outline on the board, custom-pad arcs flattened
    /// within [`PAD_ARC_TOLERANCE_MM`]. Board checks, airwires and routing
    /// clearance measure it (through [`CopperRing`]).
    pub fn outline(&self) -> Shape {
        self.shape
            .outline(PAD_ARC_TOLERANCE_MM)
            .transformed(&self.frame())
    }
}

/// Package datum is the placement origin. The footprint records that datum's
/// location relative to its own origin. Bottom is rigid Rx(pi), then Rz(theta).
pub fn placed_point(placement: &Placement, point: Point, datum: Point) -> Point {
    let x = point[0] - datum[0];
    let y = (point[1] - datum[1])
        * if placement.layer == Layer::Bottom {
            -1.0
        } else {
            1.0
        };
    let angle = placement.rotation_deg.to_radians();
    let (s, c) = angle.sin_cos();
    [
        placement.position[0] + c * x - s * y,
        placement.position[1] + s * x + c * y,
    ]
}

/// One connected pad where a route starts or ends: `layout.pads`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct PadRow {
    pub designator: String,
    /// The pad's number on its footprint ("1", "A1", "EP").
    pub pad: String,
    pub net: Id,
    pub net_name: String,
    /// The pad's centre on the board, in mm.
    pub position: Point,
    pub layer: Layer,
    /// Plated through: on every copper layer.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub through: bool,
}

/// The most designators one `layout.pads` names.
pub const MAX_PAD_FILTER: usize = 256;

/// The placed parts' connected pads, by designator and number, with their
/// net and board position: what a route between two pads starts and ends
/// at. `designators` (none: every part) and `net` narrow the list; a
/// designator no instance has is refused, naming it.
pub fn pad_rows(project: &Project, designators: &[String], net: Option<Id>) -> Result<Vec<PadRow>> {
    if designators.len() > MAX_PAD_FILTER {
        return Err(Error::Invalid(format!(
            "it names {} designators; name at most {MAX_PAD_FILTER}",
            designators.len()
        )));
    }
    let mut wanted = BTreeSet::new();
    for designator in designators {
        let instance = project
            .instance_by_designator(designator)
            .ok_or_else(|| Error::NotFound(format!("No part has the designator {designator}.")))?;
        wanted.insert(instance.id);
    }
    if let Some(net) = net
        && project.net(net).is_none()
    {
        return Err(Error::NotFound(format!("The project has no net {net}.")));
    }
    let mut rows = Vec::new();
    for pad in pads(project) {
        if (!wanted.is_empty() && !wanted.contains(&pad.instance))
            || net.is_some_and(|n| n != pad.net)
        {
            continue;
        }
        let Some(instance) = project.instance(pad.instance) else {
            continue;
        };
        let number = instances::resolve(&project.library, instance)
            .ok()
            .and_then(|r| r.physical)
            .and_then(|p| {
                p.footprint
                    .definition
                    .pads
                    .iter()
                    .find(|q| q.id == pad.pad)
                    .map(|q| q.number.clone())
            })
            .unwrap_or_default();
        rows.push(PadRow {
            designator: instance.designator.clone(),
            pad: number,
            net: pad.net,
            net_name: project
                .net(pad.net)
                .map(|n| n.name.clone())
                .unwrap_or_default(),
            position: pad.position,
            layer: pad.layer,
            through: pad.through,
        });
    }
    Ok(rows)
}

/// Connected copper pads with their exact shapes: the placed pads
/// ([`placed_pads`]) with copper and a net. The copper graph joins these;
/// clearance reads every land, netted or not ([`physical::lands`]).
pub fn pads(project: &Project) -> Vec<WorldPad> {
    placed_pads(project)
        .iter()
        .filter_map(WorldPad::of_placed)
        .collect()
}

impl WorldPad {
    /// `pad` as a connected pad: `None` for a bare hole or a pad without a
    /// net.
    pub fn of_placed(pad: &PlacedPad) -> Option<Self> {
        if pad.pad.kind.copper() == PadCopper::None {
            return None;
        }
        let (Some(terminal), Some(net)) = (pad.terminal, pad.net) else {
            return None;
        };
        let t = pad.pad_transform();
        Some(Self {
            instance: pad.instance,
            pad: pad.pad.id,
            terminal,
            net,
            position: t.translate,
            size: pad.pad.shape.extent(),
            shape: pad.pad.shape.clone(),
            rotation_deg: t.rotation_deg,
            layer: pad.layer,
            through: pad.pad.kind.copper() == PadCopper::Through,
            mirrored: t.mirror,
        })
    }
}

/// One copper pad of a placed part: its component terminal, its label
/// (number, else ID) and its centre on the board, whatever its net.
#[derive(Clone, Debug, PartialEq)]
pub struct PlacedPadCentre {
    pub terminal: crate::ids::TerminalId,
    pub label: String,
    pub position: Point,
}

/// The copper pads of `instance` on the board that belong to a terminal,
/// connected or not; empty when it is not placed or has no footprint.
pub fn placed_pad_centres(project: &Project, instance: Id) -> Vec<PlacedPadCentre> {
    let Some(placement) = project.placements.iter().find(|p| p.instance == instance) else {
        return Vec::new();
    };
    let Some(physical) = project
        .instance(instance)
        .and_then(|i| instances::resolve(&project.library, i).ok())
        .and_then(|r| r.physical)
    else {
        return Vec::new();
    };
    let footprint = &physical.footprint.definition;
    let datum = &footprint.datum;
    footprint
        .pads
        .iter()
        .filter(|pad| pad.kind.copper() != PadCopper::None)
        .filter_map(|pad| {
            let terminal = pad
                .terminal
                .and_then(|t| physical.binding.definition.terminal_of(t))?;
            Some(PlacedPadCentre {
                terminal,
                label: if pad.number.is_empty() {
                    pad.id.to_string()
                } else {
                    pad.number.clone()
                },
                position: placed_point(
                    placement,
                    placed::undo_datum_rotation(pad.position(), datum),
                    datum.offset(),
                ),
            })
        })
        .collect()
}

/// The user's label of a placed pad: its number, or its ID when the number
/// is empty. Keyed by instance and pad ID; one resolution per placement.
pub fn pad_labels(project: &Project) -> BTreeMap<(Id, PadId), String> {
    let mut labels = BTreeMap::new();
    for placement in &project.placements {
        let Some(physical) = project
            .instance(placement.instance)
            .and_then(|i| instances::resolve(&project.library, i).ok())
            .and_then(|r| r.physical)
        else {
            continue;
        };
        for pad in &physical.footprint.definition.pads {
            let label = if pad.number.is_empty() {
                pad.id.to_string()
            } else {
                pad.number.clone()
            };
            labels.insert((placement.instance, pad.id), label);
        }
    }
    labels
}

/// Whether an instance has a resolvable physical choice.
pub fn board_placeable(project: &Project, instance: Id) -> bool {
    project
        .instance(instance)
        .and_then(|i| instances::resolve(&project.library, i).ok())
        .is_some_and(|r| r.physical.is_some())
}

/// IDs shown in the update preview: board-placeable instances without a
/// placement. Existing placement/routing is never replaced.
pub fn sync_preview(project: &Project) -> Vec<Id> {
    project
        .instances
        .iter()
        .filter(|instance| {
            project.placement_of(instance.id).is_none() && board_placeable(project, instance.id)
        })
        .map(|i| i.id)
        .collect()
}

/// Change an instance's physical choice. Direct only without a board
/// placement; a consequential change keeps the placement, or removes it when
/// the choice is cleared.
pub fn set_physical(
    project: &mut Project,
    instance: Id,
    choice: Option<PhysicalChoice>,
    policy: EditPolicy,
) -> Result<()> {
    let placed = project.placement_of(instance).is_some();
    if placed && policy == EditPolicy::Direct {
        return Err(Error::Conflict(
            "This instance is on the board. Review the change first: the review shows what it does to the copper.".into(),
        ));
    }
    let target = instances::instance_mut(project, instance)?;
    let old_footprint = target.physical.map(|p| p.footprint);
    target.physical = choice;
    instances::text::follow_footprint(project, instance, old_footprint);
    if choice.is_none() {
        project.placements.retain(|p| p.instance != instance);
    }
    instances::check_instance(project, instance)
}

/// Remove an instance's board placement. Direct only without one.
pub fn detach_instance(project: &mut Project, instance: Id, policy: EditPolicy) -> Result<()> {
    if project.placement_of(instance).is_some() {
        if policy == EditPolicy::Direct {
            return Err(Error::Conflict(
                "Remove the PCB placement first, or review the removal.".into(),
            ));
        }
        project.placements.retain(|p| p.instance != instance);
    }
    Ok(())
}

/// Atomic when invoked directly: a failure leaves `project` unchanged.
pub fn apply(project: &mut Project, command: LayoutCommand) -> Result<()> {
    let mut candidate = project.clone();
    apply_staged(&mut candidate, command)?;
    *project = candidate;
    Ok(())
}

/// The refusal for a locked part: "J2 is locked; unlock it to move it".
/// `verb` is what was refused ("move", "change", "remove").
pub(crate) fn locked(project: &Project, instance: Id, verb: &str) -> Error {
    Error::Conflict(format!(
        "{} is locked; unlock it to {verb} it",
        outline::part_name(project, instance)
    ))
}

/// The refusal for a command on a part that is not on the board: names
/// the part by designator when the instance exists, else by its ID.
fn unplaced(project: &Project, instance: Id) -> Error {
    Error::NotFound(match project.instance(instance) {
        Some(i) => format!("{} is not placed on the board", i.designator),
        None => format!("No component instance has ID {instance}"),
    })
}

/// Why a hand move of the placed part `instance` is refused, in the words
/// the engine refuses `layout.move` with: it is locked, or its dimensions
/// hold its position (see [`dimensions::held`]). `None` for a part that can
/// move (a dimensioned part may only slide), so a canvas can refuse a drag
/// before it starts and say why.
pub fn move_refusal(
    project: &Project,
    instance: Id,
    evaluator: Option<&dyn outline::BoardShapeEvaluator>,
) -> Option<Error> {
    let placement = project.placement_of(instance)?;
    if placement.locked {
        return Some(locked(project, instance, "move"));
    }
    if let Some(error) = blocks::lock_refusal(project, instance, "move") {
        return Some(error);
    }
    dimensions::held(project, instance, evaluator)
}

/// Apply to a candidate the caller discards on failure. Returns the via a
/// `layout.add_via` created.
pub(crate) fn apply_staged(candidate: &mut Project, command: LayoutCommand) -> Result<Option<Id>> {
    let created = apply_inner(candidate, command)?;
    validate(candidate)?;
    Ok(created)
}

fn apply_inner(project: &mut Project, command: LayoutCommand) -> Result<Option<Id>> {
    match command {
        LayoutCommand::Sync => {
            let missing = sync_preview(project);
            staging::stage(project, missing)?;
        }
        LayoutCommand::Place {
            instance,
            position,
            rotation_deg,
            layer,
        } => {
            if project.instance(instance).is_none() {
                return Err(Error::NotFound("unknown component instance".into()));
            }
            if !board_placeable(project, instance) {
                return Err(Error::Conflict(
                    "choose a physical implementation before placing this instance".into(),
                ));
            }
            if project.placements.iter().any(|p| p.instance == instance) {
                return Err(Error::Conflict("component is already placed".into()));
            }
            // Placements stay in instance order, the order they are saved in.
            let at = project
                .placements
                .partition_point(|p| p.instance < instance);
            project.placements.insert(
                at,
                Placement {
                    instance,
                    position,
                    rotation_deg,
                    layer,
                    locked: false,
                    fill_override: Default::default(),
                    pad_fill_overrides: Vec::new(),
                },
            );
        }
        LayoutCommand::Move {
            instance,
            position,
            rotation_deg,
            layer,
        } => {
            if project.placement_of(instance).is_some_and(|p| p.locked) {
                return Err(locked(project, instance, "move"));
            }
            if let Some(error) = blocks::lock_refusal(project, instance, "move") {
                return Err(error);
            }
            let missing = unplaced(project, instance);
            let placement = project
                .placements
                .iter_mut()
                .find(|p| p.instance == instance)
                .ok_or(missing)?;
            placement.position = position;
            placement.rotation_deg = rotation_deg;
            placement.layer = layer;
            // The engine carries the attached copper (`dimensions::carry`).
        }
        LayoutCommand::SetPlacementLock { instance, locked } => {
            let missing = unplaced(project, instance);
            project
                .placements
                .iter_mut()
                .find(|p| p.instance == instance)
                .ok_or(missing)?
                .locked = locked;
        }
        LayoutCommand::Route { request } => {
            let edits = ManualRouter.propose(project, request)?;
            apply_routes(project, edits)?;
        }
        LayoutCommand::ApplyRoute { edits } => apply_routes(project, edits)?,
        LayoutCommand::DeleteTrack { id } => {
            apply_routes(
                project,
                RouteEditSet {
                    remove: vec![id],
                    ..Default::default()
                },
            )?;
        }
        LayoutCommand::SetTrackLock { id, locked } => {
            project
                .tracks
                .iter_mut()
                .find(|t| t.id == id)
                .ok_or_else(|| Error::NotFound("unknown track".into()))?
                .locked = locked;
        }
        LayoutCommand::SetTrack { id, width, layer } => {
            if width.is_none() && layer.is_none() {
                return invalid("layout.track_set needs a width, a layer or both");
            }
            let mut track = project
                .tracks
                .iter()
                .find(|t| t.id == id)
                .cloned()
                .ok_or_else(|| Error::NotFound("unknown track".into()))?;
            if let Some(width) = width {
                track.width = width;
            }
            if let Some(layer) = layer {
                track.layer = layer;
            }
            apply_routes(
                project,
                RouteEditSet {
                    replace: vec![track],
                    ..Default::default()
                },
            )?;
        }
        LayoutCommand::SetFabrication(fabrication) => {
            project.fabrication = fabrication;
            // The fab profile's colour option follows, in the same step.
            mask_colour::follow(&mut project.rules, fabrication.mask_colour)?;
        }
        LayoutCommand::SetStackup(stackup) => restack::apply(
            project,
            restack::StackupEdit {
                stackup,
                moves: Vec::new(),
                removed: Vec::new(),
            },
        )?,
        LayoutCommand::EditStackup(edit) => restack::apply(project, edit)?,
        LayoutCommand::UseStackup { stackup, .. } => {
            return Err(Error::NotFound(format!(
                "layout.stackup_unknown: no stackup {stackup} here; a host with the stackup \
                 library takes it, or send its layers with layout.edit_stackup"
            )));
        }
        LayoutCommand::AddVia { via } => return via::add(project, via).map(Some),
        LayoutCommand::MoveVia { id, position } => via::move_to(project, id, position)?,
        LayoutCommand::DeleteVia { id } => via::delete(project, id)?,
        LayoutCommand::SetViaTented { id, tented } => via::set_tented(project, id, tented)?,
        LayoutCommand::SetVias { ids, fill } => via::set_fill(project, &ids, fill)?,
        LayoutCommand::SetViaLock { id, locked } => via::set_lock(project, id, locked)?,
        LayoutCommand::RouteLayerChange { request } => {
            return via::route_layer_change(project, request).map(Some);
        }
        LayoutCommand::RoutePair { request } => {
            let edits = pair::route(project, &request)?;
            apply_routes(project, edits)?;
        }
        LayoutCommand::RouteStep { request } => {
            route_step(project, &request)?;
        }
        LayoutCommand::TrackDrag { request } => {
            track_drag(project, &request)?;
        }
        LayoutCommand::TuneLength { request } => {
            let obstacles = obstacles::Obstacles::of(project);
            let graph = CopperGraph::of(project);
            let plan = meander::plan(project, &obstacles, &graph, &request)?;
            apply_routes(project, plan.edits)?;
        }
        LayoutCommand::SetBoardSize { size } => {
            if !point_valid(size) || !size.iter().all(|v| (5.0..=1000.0).contains(v)) {
                return invalid("board dimensions must be 5–1000 mm");
            }
            project.board_size = size;
        }
        LayoutCommand::PlacePart {
            part,
            position,
            rotation_deg,
            layer,
            designator,
        } => {
            return part::place(project, &part, position, rotation_deg, layer, designator)
                .map(Some);
        }
        LayoutCommand::SetPart { instance, part } => {
            return part::set(project, instance, &part).map(Some);
        }
        LayoutCommand::RemovePart { instance } => part::remove(project, instance)?,
        LayoutCommand::MoveParts { moves, copper } => {
            arrange::move_parts(project, &moves, copper.as_ref())?;
        }
        // Without the host's board evaluator a dimensioned part is refused
        // (the engine passes its evaluator: `Engine::apply_hosted`).
        LayoutCommand::Align { instances, to } => arrange::align(project, &instances, to, None)?,
        LayoutCommand::Distribute { instances, axis } => {
            arrange::distribute(project, &instances, axis, None)?;
        }
        LayoutCommand::DeleteObjects { objects } => {
            several::delete(project, &objects)?;
        }
        LayoutCommand::LockObjects { objects, locked } => {
            several::lock(project, &objects, locked)?;
        }
        LayoutCommand::AddText { text } => return board_text::add(project, *text).map(Some),
        LayoutCommand::EditText { text } => board_text::edit(project, *text)?,
        LayoutCommand::DeleteText { id } => board_text::delete(project, id)?,
        LayoutCommand::SetRefusedImportTracks { tracks } => {
            crate::model::import_review::check_refused(&tracks)?;
            project.refused_import_tracks = tracks;
        }
    }
    Ok(None)
}

/// `layout.route_step` on a candidate: applies the plan and returns it,
/// its edit set spent. Refused when the head cannot move or the path
/// exceeds the request's `max_detour`.
pub(crate) fn route_step(
    project: &mut Project,
    request: &router::RouteStepRequest,
) -> Result<router::RoutePlan> {
    request.check()?;
    let index = router::RouteIndex::of(project);
    let mut plan = router::plan(project, &index, request);
    if plan.edits.add.is_empty() && plan.edits.vias.is_empty() {
        return Err(Error::Conflict(
            plan.blocked.unwrap_or_else(|| "Nothing to route".into()),
        ));
    }
    apply_routes(project, std::mem::take(&mut plan.edits))?;
    Ok(plan)
}

/// `layout.track_drag` on a candidate: applies the plan and returns it,
/// its edit set spent. Refused when the segment cannot move.
pub(crate) fn track_drag(
    project: &mut Project,
    request: &router::drag::TrackDragRequest,
) -> Result<router::RoutePlan> {
    request.check()?;
    let index = router::RouteIndex::of(project);
    let mut plan = router::drag::drag_plan(project, &index, request);
    if plan.edits.replace.is_empty() {
        return Err(Error::Conflict(
            plan.blocked
                .unwrap_or_else(|| "The track cannot move there".into()),
        ));
    }
    apply_routes(project, std::mem::take(&mut plan.edits))?;
    Ok(plan)
}

fn invalid<T>(message: &str) -> Result<T> {
    Err(Error::Invalid(message.into()))
}

fn apply_routes(project: &mut Project, edits: RouteEditSet) -> Result<()> {
    // Tracks and vias by identity, so an edit costs its own size plus one
    // pass over the board, never edits × board.
    let track_at: HashMap<Id, usize> = project
        .tracks
        .iter()
        .enumerate()
        .map(|(k, t)| (t.id, k))
        .collect();
    let via_at: HashMap<Id, usize> = project
        .vias
        .iter()
        .enumerate()
        .map(|(k, v)| (v.id, k))
        .collect();
    let mut touched = BTreeSet::new();
    for id in edits
        .remove
        .iter()
        .copied()
        .chain(edits.replace.iter().map(|t| t.id))
    {
        if !touched.insert(id) {
            return invalid("track appears more than once in routing edit");
        }
        let previous = track_at
            .get(&id)
            .and_then(|k| project.tracks.get(*k))
            .ok_or_else(|| Error::NotFound("routing edit references unknown track".into()))?;
        if previous.locked {
            return Err(Error::Conflict(
                "routing edit would change a locked track".into(),
            ));
        }
    }
    let nets = project
        .nets
        .iter()
        .map(|net| net.id)
        .collect::<BTreeSet<_>>();
    for track in &edits.replace {
        if track_at
            .get(&track.id)
            .and_then(|k| project.tracks.get(*k))
            .is_some_and(|old| old.net != track.net)
        {
            return Err(Error::Conflict(
                "routing edits cannot silently change a track's net".into(),
            ));
        }
        check_request_with_nets(project, &request_from(track), &nets)?;
        // A track that stays on its layer is left there: the board check
        // tells what its class does not allow.
        if track_at
            .get(&track.id)
            .and_then(|k| project.tracks.get(*k))
            .is_some_and(|old| old.layer != track.layer)
        {
            via_class::check_layer(project, track.net, track.layer)?;
        }
    }
    for request in &edits.add {
        check_request_with_nets(project, request, &nets)?;
        via_class::check_layer(project, request.net, request.layer)?;
    }
    let mut vias_before: BTreeMap<Id, usize> = BTreeMap::new();
    for request in &edits.vias {
        let count = || project.vias.iter().filter(|v| v.net == request.net).count();
        vias_before.entry(request.net).or_insert_with(count);
    }
    for track in &edits.restore {
        if !touched.insert(track.id) {
            return invalid("track appears more than once in routing edit");
        }
        if track.id == 0
            || track.id >= project.next_id
            || track_at.contains_key(&track.id)
            || via_at.contains_key(&track.id)
        {
            return invalid("a restored track needs a free identity issued before");
        }
        check_request_with_nets(project, &request_from(track), &nets)?;
    }
    let mut moved_vias = BTreeSet::new();
    for moved in &edits.move_vias {
        if !moved_vias.insert(moved.id) || edits.remove_vias.contains(&moved.id) {
            return invalid("via appears more than once in routing edit");
        }
        if !point_valid(moved.position) {
            return invalid("via coordinates must be finite");
        }
        via::unlocked_at(project, via_at.get(&moved.id).copied())?.position = moved.position;
    }
    let mut removed_vias = HashSet::new();
    for id in edits.remove_vias.iter().copied() {
        if !removed_vias.insert(id) {
            return invalid("via appears more than once in routing edit");
        }
        via::unlocked_at(project, via_at.get(&id).copied())?;
    }
    if !removed_vias.is_empty() {
        project.vias.retain(|v| !removed_vias.contains(&v.id));
    }
    let mut added_vias = Vec::with_capacity(edits.vias.len());
    for request in edits.vias {
        if !nets.contains(&request.net) {
            return Err(Error::NotFound("unknown electrical net".into()));
        }
        added_vias.push(via::insert(project, request)?);
    }
    for (net, before) in vias_before {
        via_class::check_vias(project, net, before)?;
    }
    let mut changed = BTreeSet::new();
    if !edits.replace.is_empty() {
        let tracks = &mut *project.tracks;
        for track in edits.replace {
            changed.insert(track.id);
            let target = track_at
                .get(&track.id)
                .and_then(|k| tracks.get_mut(*k))
                .ok_or_else(|| Error::NotFound("missing replacement target".into()))?;
            *target = track;
        }
    }
    if !edits.remove.is_empty() {
        let removed: HashSet<Id> = edits.remove.iter().copied().collect();
        project.tracks.retain(|t| !removed.contains(&t.id));
    }
    for track in edits.restore {
        changed.insert(track.id);
        project.tracks.push(track);
    }
    for request in edits.add {
        let id = project.allocate()?;
        changed.insert(id);
        project.tracks.push(Track {
            id,
            net: request.net,
            start: request.start,
            end: request.end,
            width: request.width,
            layer: request.layer,
            locked: false,
        });
    }
    validate(project)?;
    let vias: BTreeSet<Id> = added_vias.into_iter().chain(moved_vias).collect();
    for via in project.vias.iter().filter(|v| vias.contains(&v.id)) {
        via::check_inside(project, via)?;
    }
    router::check_committed(project, &changed, &vias)
}

fn request_from(track: &Track) -> RouteRequest {
    RouteRequest {
        net: track.net,
        start: track.start,
        end: track.end,
        width: track.width,
        layer: track.layer,
    }
}

fn check_request(project: &Project, request: &RouteRequest) -> Result<()> {
    let nets = project
        .nets
        .iter()
        .map(|net| net.id)
        .collect::<BTreeSet<_>>();
    check_request_with_nets(project, request, &nets)
}

/// Copper a host brings in as it is ([`import_copper`]): how much the board
/// took, and what it cannot hold, each with the reason.
#[derive(Debug, Default)]
pub struct ImportedCopper {
    pub tracks: usize,
    pub vias: usize,
    pub refused_tracks: Vec<(RouteRequest, Error)>,
    pub refused_vias: Vec<(ViaRequest, Error)>,
}

/// Add an imported board's tracks and vias as its file has them. Only what
/// a document cannot hold is left out ([`validate`]: a net or copper layer
/// the board lacks, a size out of range, a via without a ring or not
/// through, copper off the working area). No rule is read: what breaks the
/// fab profile or lies off the board is the board checks' to report, as in
/// an opened file. Not an
/// operation: an importer adds it to the document it builds, which
/// [`crate::Engine::replace_document`] validates.
pub fn import_copper(
    project: &mut Project,
    tracks: Vec<RouteRequest>,
    vias: Vec<ViaRequest>,
) -> Result<ImportedCopper> {
    let nets: BTreeSet<Id> = project.nets.iter().map(|net| net.id).collect();
    let mut out = ImportedCopper::default();
    for request in tracks {
        if let Err(why) = check_held(project, &request, &nets) {
            out.refused_tracks.push((request, why));
            continue;
        }
        let id = project.allocate()?;
        project.tracks.push(Track {
            id,
            net: request.net,
            start: request.start,
            end: request.end,
            width: request.width,
            layer: request.layer,
            locked: false,
        });
        out.tracks += 1;
    }
    for request in vias {
        if let Err(why) = via::check_held(&request, &nets) {
            out.refused_vias.push((request, why));
            continue;
        }
        via::insert(project, request)?;
        out.vias += 1;
    }
    Ok(out)
}

/// Whether the document can hold the track `request` asks for, whatever
/// the rules say of it.
fn check_held(project: &Project, request: &RouteRequest, nets: &BTreeSet<Id>) -> Result<()> {
    check_track_shape(request)?;
    if !nets.contains(&request.net) {
        return Err(Error::NotFound("unknown electrical net".into()));
    }
    if !project.stackup.has(request.layer) {
        return invalid("track lies on a copper layer the stackup lacks");
    }
    if ![request.start, request.end]
        .into_iter()
        .all(in_working_area)
    {
        return invalid("track copper must lie inside the working area");
    }
    Ok(())
}

fn check_track_shape(request: &RouteRequest) -> Result<()> {
    if !point_valid(request.start)
        || !point_valid(request.end)
        || !request.width.is_finite()
        || !(0.05..=5.0).contains(&request.width)
    {
        return invalid("track coordinates must be finite; width must be 0.05–5 mm");
    }
    if distance(request.start, request.end) < 0.01 {
        return invalid("track must be at least 0.01 mm long");
    }
    Ok(())
}

fn check_track_inside(project: &Project, request: &RouteRequest) -> Result<()> {
    let track = Track {
        id: 0,
        net: request.net,
        start: request.start,
        end: request.end,
        width: request.width,
        layer: request.layer,
        locked: false,
    };
    if !track_inside_board(project, &track) {
        return invalid("track copper must lie inside the board outline");
    }
    Ok(())
}

fn check_request_with_nets(
    project: &Project,
    request: &RouteRequest,
    nets: &BTreeSet<Id>,
) -> Result<()> {
    check_track_shape(request)?;
    if !nets.contains(&request.net) {
        return Err(Error::NotFound("unknown electrical net".into()));
    }
    check_class_width(project, request.net, request.width)?;
    check_track_inside(project, request)
}

/// The net class of `net`: its explicit assignment, else the first
/// matching name pattern (never inferred from anything else).
pub fn net_class(project: &Project, net: Id) -> Option<crate::rules::NetClassId> {
    project.rules.class_of(net, project.net_name(net))
}

/// The Route tool's width for `net`: its class's width, else the
/// effective default.
pub fn route_width(project: &Project, net: Id) -> crate::rules::Resolved<f64> {
    project.rules.track_width(net_class(project, net))
}

/// A through via on `net` at `position` with its class's drill and pad
/// (else the effective defaults), tented as the rules' `tent vias` says.
pub fn class_via(project: &Project, net: Id, position: Point) -> ViaRequest {
    let (drill, diameter) = project.rules.via_size(net_class(project, net)).value;
    ViaRequest {
        drill,
        diameter,
        ..ViaRequest::through(&project.rules.effective(), net, position)
    }
}

/// The track widths the Route tool steps through for `net`, ascending: the
/// net's own width ([`route_width`]), the rule default and every net
/// class's width, none below the net's minimum.
pub fn route_widths(project: &Project, net: Id) -> Vec<f64> {
    let rules = &project.rules;
    let min = rules.track_width_min(net_class(project, net)).value;
    let mut widths: Vec<f64> = std::iter::once(route_width(project, net).value)
        .chain(std::iter::once(rules.track_width(None).value))
        .chain(rules.net_classes.iter().filter_map(|c| c.track_width))
        .filter(|w| w.is_finite() && *w + 1e-9 >= min && (0.05..=5.0).contains(w))
        .collect();
    widths.sort_by(f64::total_cmp);
    widths.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
    widths
}

/// The via (drill, pad) sizes the Route tool steps through for `net`,
/// ascending by pad: the net's own via ([`class_via`]), the rule default
/// and every net class's via, each with at least the rules' drill and
/// annular ring.
pub fn via_sizes(project: &Project, net: Id) -> Vec<(f64, f64)> {
    let rules = &project.rules;
    let holes = rules.effective().holes;
    let base = (holes.via_drill_default, holes.via_diameter_default);
    let mut sizes: Vec<(f64, f64)> = [
        rules.via_size(net_class(project, net)).value,
        rules.via_size(None).value,
    ]
    .into_iter()
    .chain(
        rules
            .net_classes
            .iter()
            .filter(|c| c.via_drill.is_some() || c.via_diameter.is_some())
            .map(|c| {
                (
                    c.via_drill.unwrap_or(base.0),
                    c.via_diameter.unwrap_or(base.1),
                )
            }),
    )
    .filter(|(drill, pad)| via_size_fits(project, *drill, *pad).is_ok())
    .collect();
    sizes.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.total_cmp(&b.0)));
    sizes.dedup_by(|a, b| (a.0 - b.0).abs() < 1e-9 && (a.1 - b.1).abs() < 1e-9);
    sizes
}

/// Whether a via of `drill` and `pad` (mm) meets the rules' smallest drill
/// and via annular ring; the error names the rule it misses.
pub fn via_size_fits(project: &Project, drill: f64, pad: f64) -> Result<()> {
    let holes = project.rules.effective().holes;
    if !drill.is_finite() || !pad.is_finite() || drill + 1e-9 < holes.drill_min {
        return Err(Error::Invalid(format!(
            "Via drill needs at least {} mm (min drill)",
            trim_mm(holes.drill_min)
        )));
    }
    if (pad - drill) / 2.0 + 1e-9 < holes.via_annular_ring_min {
        return Err(Error::Invalid(format!(
            "Via pad needs at least {} mm (min via annular ring)",
            trim_mm(drill + 2.0 * holes.via_annular_ring_min)
        )));
    }
    Ok(())
}

/// Routing refuses a track narrower than its net's class minimum or the
/// effective minimum track width.
fn check_class_width(project: &Project, net: Id, width: f64) -> Result<()> {
    let min = project.rules.track_width_min(net_class(project, net));
    if width + 1e-9 < min.value {
        return Err(Error::Invalid(format!(
            "Track on {} needs at least {} mm ({}: min track width)",
            project.net_name(net),
            trim_mm(min.value),
            min.source.describe()
        )));
    }
    Ok(())
}

fn trim_mm(mm: f64) -> String {
    let text = format!("{mm:.3}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

pub fn validate(project: &Project) -> Result<()> {
    mask_colour::check(project)?;
    profile_fit::check(project)?;
    if !project
        .board_size
        .iter()
        .all(|v| v.is_finite() && (5.0..=1000.0).contains(v))
    {
        return invalid("board dimensions must be 5–1000 mm");
    }
    instances::text::check(project)?;
    let mut placed = BTreeSet::new();
    for placement in &project.placements {
        let Some(instance) = project
            .instance(placement.instance)
            .filter(|_| placed.insert(placement.instance))
        else {
            return invalid("duplicate or unknown placement instance");
        };
        if instances::resolve(&project.library, instance)
            .ok()
            .and_then(|r| r.physical)
            .is_none()
        {
            return invalid("a placement requires a resolvable physical choice");
        }
        if !in_working_area(placement.position) {
            return Err(Error::Invalid(format!(
                "{} must stay within {} to {} mm on each axis",
                outline::part_name(project, placement.instance),
                limits::mm(WORKING_AREA_MM[0]),
                limits::mm(WORKING_AREA_MM[1])
            )));
        }
        if !placement.rotation_deg.is_finite() || placement.rotation_deg.abs() > 360_000.0 {
            return invalid("placement rotation must be finite");
        }
    }
    project.stackup.check()?;
    fill::validate(project)?;
    outline::validate(project)?;
    dimensions::validate(project)?;
    let ids = project.nets.iter().map(|n| n.id).collect::<BTreeSet<_>>();
    via::validate(project, &ids)?;
    length::validate(project)?;
    let mut track_ids = BTreeSet::new();
    for track in &project.tracks {
        if track.id == 0 || track.id >= project.next_id || !track_ids.insert(track.id) {
            return invalid("invalid or duplicate track identity");
        }
        if !ids.contains(&track.net) {
            return invalid(
                "track references an unknown net; repair the electrical update explicitly",
            );
        }
        if !point_valid(track.start)
            || !point_valid(track.end)
            || !track.width.is_finite()
            || !(0.05..=5.0).contains(&track.width)
            || distance(track.start, track.end) < 0.01
        {
            return invalid("invalid track geometry");
        }
        if !project.stackup.has(track.layer) {
            return invalid("track lies on a copper layer the stackup lacks");
        }
        // Copper off the board is kept (an import's is never dropped); the
        // board check tells it, and no edit puts copper there
        // ([`copper_stays`]).
        if ![track.start, track.end].into_iter().all(in_working_area) {
            return invalid("track copper lies outside the working area");
        }
    }
    Ok(())
}

/// Whether `point` lies in the [`WORKING_AREA_MM`] square.
pub fn in_working_area(point: Point) -> bool {
    let [lo, hi] = WORKING_AREA_MM;
    point.iter().all(|v| v.is_finite() && (lo..=hi).contains(v))
}

/// A track's copper: the stadium a round aperture of its width sweeps from
/// start to end. Its round end caps join seamlessly at any angle, as
/// fabricated. Exact (the caps are true half circles); the checks, the
/// Layout view and the Solidcraft adapter all use this outline.
pub fn track_shape(track: &Track) -> Shape {
    stadium_shape(track.start, track.end, track.width)
}

/// The points within `width / 2` of the segment `start`–`end`: a track's
/// copper, a slot's drill.
pub fn stadium_shape(start: Point, end: Point, width: f64) -> Shape {
    let half = width * 0.5;
    let dx = end[0] - start[0];
    let dy = end[1] - start[1];
    let length = dx.hypot(dy).max(EPS);
    let along = [dx / length * half, dy / length * half];
    let normal = [-along[1], along[0]];
    let (a, b) = (sub(start, along), add(end, along));
    Shape::Polygon {
        points: vec![
            sub(a, normal),
            sub(b, normal),
            add(b, normal),
            add(a, normal),
        ],
        radii: vec![half; 4],
    }
}

/// [`track_shape`] flattened within [`CURVE_SLACK_MM`], its vertices on the
/// true outline, counter-clockwise.
pub fn track_outline(track: &Track) -> Vec<Point> {
    track_shape(track).flatten(CURVE_SLACK_MM)
}

/// Whether `point` lies on a track's copper, or within `tolerance` of it.
pub fn track_contains(track: &Track, point: Point, tolerance: f64) -> bool {
    crate::geometry::segment_distance(point, track.start, track.end)
        <= track.width * 0.5 + tolerance
}

/// Whether a track's whole copper, round caps included, lies on the board.
fn track_inside_board(project: &Project, track: &Track) -> bool {
    track_within(track, project.board_size)
}

/// Whether a track's whole copper lies on a board of `size`.
fn track_within(track: &Track, size: Point) -> bool {
    let half = track.width * 0.5;
    [track.start, track.end].iter().all(|p| {
        p[0] - half >= -EPS
            && p[1] - half >= -EPS
            && p[0] + half <= size[0] + EPS
            && p[1] + half <= size[1] + EPS
    })
}

/// The tracks and vias whose copper is not wholly on the board's box.
pub fn outside_board(project: &Project) -> BTreeSet<Id> {
    let size = project.board_size;
    let tracks = project
        .tracks
        .iter()
        .filter(|t| !track_within(t, size))
        .map(|t| t.id);
    let vias = project
        .vias
        .iter()
        .filter(|v| !via::within(v, size))
        .map(|v| v.id);
    tracks.chain(vias).collect()
}

/// Refuse an edit that leaves copper off the board: a track or via outside
/// the box now that was not outside it in `base`. What was outside already
/// (an imported board's) stays as it is.
pub(crate) fn copper_stays(base: &Project, project: &Project) -> Result<()> {
    let now = outside_board(project);
    if now.is_empty() {
        return Ok(());
    }
    let before = outside_board(base);
    if project.board_size != base.board_size {
        return copper_fits(project, project.board_size, &before);
    }
    for track in &project.tracks {
        if now.contains(&track.id) && !before.contains(&track.id) {
            return invalid("track copper must lie inside the board outline");
        }
    }
    for via in &project.vias {
        if now.contains(&via.id) && !before.contains(&via.id) {
            return via::check_inside(project, via);
        }
    }
    Ok(())
}

/// Refuse a board `size` that would leave tracks or vias off the board,
/// naming them by count and net ("2 tracks on GND and CC2 would lie
/// outside 38 × 64 mm"), so a designer knows what to move or remove.
/// A resized board is judged once its dimensioned parts have carried their
/// copper (`updates::finalize`): copper that follows the edges is no
/// refusal.
///
/// Copper among `outside` was off the board before and stays there.
pub(crate) fn copper_fits(project: &Project, size: Point, outside: &BTreeSet<Id>) -> Result<()> {
    let tracks: Vec<Id> = project
        .tracks
        .iter()
        .filter(|t| !track_within(t, size) && !outside.contains(&t.id))
        .map(|t| t.net)
        .collect();
    let vias: Vec<Id> = project
        .vias
        .iter()
        .filter(|v| !via::within(v, size) && !outside.contains(&v.id))
        .map(|v| v.net)
        .collect();
    let counted = |nets: &[Id], one: &str, many: &str| -> Option<String> {
        let mut names: Vec<&str> = Vec::new();
        for net in nets {
            let name = project.net_name(*net);
            if !names.contains(&name) {
                names.push(name);
            }
        }
        let shown = match names.as_slice() {
            [] => return None,
            [a] => (*a).to_owned(),
            [a, b] => format!("{a} and {b}"),
            [a, b, rest @ ..] => format!("{a}, {b} and {} more nets", rest.len()),
        };
        let kind = if nets.len() == 1 { one } else { many };
        Some(format!("{} {kind} on {shown}", nets.len()))
    };
    let parts: Vec<String> = [
        counted(&tracks, "track", "tracks"),
        counted(&vias, "via", "vias"),
    ]
    .into_iter()
    .flatten()
    .collect();
    if parts.is_empty() {
        return Ok(());
    }
    Err(Error::Invalid(format!(
        "{} would lie outside {} × {} mm",
        parts.join(" and "),
        limits::mm(size[0]),
        limits::mm(size[1])
    )))
}

fn add(a: Point, b: Point) -> Point {
    [a[0] + b[0], a[1] + b[1]]
}
fn sub(a: Point, b: Point) -> Point {
    [a[0] - b[0], a[1] - b[1]]
}
fn distance(a: Point, b: Point) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

pub fn point_in_polygon(point: Point, polygon: &[Point; 4]) -> bool {
    let mut positive = false;
    let mut negative = false;
    for i in 0..4 {
        let a = polygon[i];
        let b = polygon[(i + 1) % 4];
        let cross = (b[0] - a[0]) * (point[1] - a[1]) - (b[1] - a[1]) * (point[0] - a[0]);
        positive |= cross > EPS;
        negative |= cross < -EPS;
    }
    !(positive && negative)
}

/// Same geometry and clearance rule as routing and the board checks
/// (the project's effective rules, net classes and class pairs), used by
/// revision impact plans.
pub fn track_pad_clearance_conflict(
    track: &Track,
    pad: &WorldPad,
    rules: &limits::BoardRules<'_>,
) -> bool {
    use crate::rules::CopperObject;
    pad.on(track.layer)
        && track.net != pad.net
        && CopperRing::of_track(track).closer_than(
            &CopperRing::of_pad(pad),
            rules.clearance(
                CopperObject::Track,
                Some(track.net),
                CopperObject::Pad,
                Some(pad.net),
            ),
        )
}

/// A track or via by ID, with the index of a pad it comes too close to.
pub type PadHit = (Id, usize);

/// The tracks and vias of `project` closer than the clearance rule to a
/// pad of another net among `pads` ([`track_pad_clearance_conflict`],
/// [`via_pad_clearance_conflict`]), each with the index of the first such
/// pad. Only copper near each pad is compared.
pub fn pad_clearance_conflicts(
    project: &Project,
    pads: &[WorldPad],
    rules: &limits::BoardRules<'_>,
) -> (Vec<PadHit>, Vec<PadHit>) {
    let layers = project.stackup.layers();
    let reach = rules.max_clearance() + 2.0 * copper::CURVE_SLACK_MM;
    let tracks = grid::BoundsGrid::new(
        project
            .tracks
            .iter()
            .map(|t| CopperRing::of_track(t).bounds),
        reach,
    );
    let vias = grid::BoundsGrid::new(
        project.vias.iter().map(|v| CopperRing::of_via(v).bounds),
        reach,
    );
    let (mut track_hits, mut via_hits) = (BTreeMap::new(), BTreeMap::new());
    let mut near = Vec::new();
    for (index, pad) in pads.iter().enumerate() {
        let Some(bounds) = CopperRing::of_pad(pad).bounds else {
            continue;
        };
        tracks.near(&bounds, reach, &mut near);
        for track in near.iter().filter_map(|i| project.tracks.get(*i)) {
            if track_pad_clearance_conflict(track, pad, rules) {
                track_hits.entry(track.id).or_insert(index);
            }
        }
        vias.near(&bounds, reach, &mut near);
        for via in near.iter().filter_map(|i| project.vias.get(*i)) {
            if via_pad_clearance_conflict(via, pad, &layers, rules) {
                via_hits.entry(via.id).or_insert(index);
            }
        }
    }
    (
        track_hits.into_iter().collect(),
        via_hits.into_iter().collect(),
    )
}

/// Every board check: clearance and edges over the exact outlines
/// ([`BoardCheck`]), connectivity from the copper graph, fills,
/// outline, lengths and etch hazards ([`etch`]). Builds the copper graph; callers holding the
/// revision's graph ([`crate::Engine::copper_graph`]) call
/// [`checks_with`]. `settings` are the host's values for limits the
/// project leaves open ([`crate::Engine::check_settings`]).
pub fn checks(project: &Project, settings: &CheckSettings) -> Vec<Finding> {
    checks_with(project, &CopperGraph::of(project), settings)
}

/// [`checks`] with `graph`, the copper graph of `project`.
pub fn checks_with(
    project: &Project,
    graph: &CopperGraph,
    settings: &CheckSettings,
) -> Vec<Finding> {
    checks_on(project, &BoardCopper::of(project), graph, settings)
}

/// [`checks_with`] on `copper`, the revision's [`BoardCopper`], for a
/// host that already holds it.
pub fn checks_on(
    project: &Project,
    copper: &BoardCopper,
    graph: &CopperGraph,
    settings: &CheckSettings,
) -> Vec<Finding> {
    let graph = GraphSource::given(graph);
    let (families, _) = check_families(project, copper, &graph, settings, &mut |_| None);
    families.into_iter().flat_map(|(_, f)| f).collect()
}

/// The copper graph a board check reads: one the caller holds, or one
/// made when a check first asks for it, so checks whose rules need no
/// connectivity never build it.
pub struct GraphSource<'a> {
    given: Option<&'a CopperGraph>,
    make: Option<&'a dyn Fn() -> std::sync::Arc<CopperGraph>>,
    made: std::cell::OnceCell<std::sync::Arc<CopperGraph>>,
}

impl<'a> GraphSource<'a> {
    /// `graph`, already made.
    pub fn given(graph: &'a CopperGraph) -> Self {
        Self {
            given: Some(graph),
            make: None,
            made: std::cell::OnceCell::new(),
        }
    }

    /// The graph `make` returns, asked for once, on first use.
    pub fn lazy(make: &'a dyn Fn() -> std::sync::Arc<CopperGraph>) -> Self {
        Self {
            given: None,
            make: Some(make),
            made: std::cell::OnceCell::new(),
        }
    }

    /// The graph made, when [`Self::lazy`] made it.
    pub fn made(&self) -> Option<std::sync::Arc<CopperGraph>> {
        self.made.get().cloned()
    }

    /// The graph.
    pub fn get(&self) -> &CopperGraph {
        if let Some(graph) = self.given {
            return graph;
        }
        self.made.get_or_init(|| match self.make {
            Some(make) => make(),
            None => std::sync::Arc::new(CopperGraph::default()),
        })
    }
}

/// One family of board checks. [`checks_on`] runs them in
/// [`BoardCheck::ALL`]'s order, which is the order their findings are
/// listed in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoardCheck {
    /// Parts the schematic has that the board does not place.
    Unplaced,
    /// Copper to copper and copper to edge ([`BoardCopper`]).
    Clearance,
    /// The copper graph's findings: unrouted, dangling, unjoined islands.
    Connectivity,
    /// Via drills, rings and holes ([`via`]).
    Vias,
    /// Pad holes and their copper ([`pad_hole`]).
    PadHoles,
    Courtyards,
    /// Track widths, drill sizes and board text heights ([`physical`]).
    Physical,
    /// Fills and keepouts.
    Fills,
    Outline,
    Dimensions,
    Lengths,
    Etch,
    Bodies,
    MaskColour,
    ProfileFit,
    Fiducials,
    /// Mask webs between the assembled board's openings ([`mask_web`]).
    MaskWeb,
    /// Filled vias against the fab profile ([`via_fill`]).
    ViaFills,
    /// Vias and layers a net class does not allow ([`via_class`]).
    ViaClass,
}

impl BoardCheck {
    pub const ALL: [BoardCheck; 19] = [
        Self::Unplaced,
        Self::Clearance,
        Self::Connectivity,
        Self::Vias,
        Self::PadHoles,
        Self::Courtyards,
        Self::Physical,
        Self::Fills,
        Self::Outline,
        Self::Dimensions,
        Self::Lengths,
        Self::Etch,
        Self::Bodies,
        Self::MaskColour,
        Self::ProfileFit,
        Self::Fiducials,
        Self::MaskWeb,
        Self::ViaFills,
        Self::ViaClass,
    ];

    /// Whether the family reads the board's tracks or vias. A change to
    /// tracks and vias alone leaves every other family's findings as they
    /// were.
    pub fn reads_copper(self) -> bool {
        match self {
            Self::Clearance
            | Self::Connectivity
            | Self::Vias
            | Self::PadHoles
            | Self::Physical
            | Self::Fills
            | Self::Lengths
            | Self::Etch
            | Self::Fiducials
            | Self::ViaFills
            | Self::ViaClass => true,
            Self::Unplaced
            | Self::Courtyards
            | Self::Outline
            | Self::Dimensions
            | Self::Bodies
            | Self::MaskColour
            | Self::ProfileFit
            | Self::MaskWeb => false,
        }
    }

    /// The findings of a family that checks item by item, as its items
    /// ([`copper::Items`]), made from `last` (an earlier state's items and
    /// what changed since) when given. `None` for the other families, and
    /// when the check would stop at its pair limit, which only
    /// [`check_families`] reports.
    pub fn items(
        self,
        project: &Project,
        copper: &BoardCopper,
        last: Option<(&copper::Items, &copper::CopperChanges)>,
    ) -> Option<copper::Items> {
        match self {
            Self::Clearance => copper.clearance_items(project, last),
            Self::Vias => via::items(copper, project, last),
            Self::Physical => Some(physical::items(project, last)),
            _ => None,
        }
    }

    /// Whether the family is skipped once the clearance check stopped at
    /// its pair limit: the copper after it is unchecked, which that
    /// check's finding says.
    fn after_copper_limit(self) -> bool {
        matches!(
            self,
            Self::Connectivity | Self::Vias | Self::PadHoles | Self::Courtyards
        )
    }

    fn run(
        self,
        project: &Project,
        copper: &BoardCopper,
        graph: &GraphSource<'_>,
        settings: &CheckSettings,
    ) -> (Vec<Finding>, bool) {
        let findings = match self {
            Self::Unplaced => copper::unplaced(project),
            Self::Clearance => return copper.clearance(project),
            Self::Connectivity => graph.get().findings(project),
            Self::Vias => via::findings(copper, project),
            Self::PadHoles => pad_hole::findings(copper, project, graph),
            Self::Courtyards => courtyard::findings(project),
            Self::Physical => physical::findings(project),
            Self::Fills => fill::findings(project),
            Self::Outline => outline::checks(project),
            Self::Dimensions => dimensions::checks(project),
            Self::Lengths => length::checks_from(project, graph, settings),
            Self::Etch => etch::checks_from(project, graph),
            Self::Bodies => bodies::checks(project),
            Self::MaskColour => mask_colour::findings(project),
            Self::ProfileFit => profile_fit::findings(project),
            Self::Fiducials => fiducial::findings(project),
            Self::MaskWeb => mask_web::findings(project),
            Self::ViaFills => via_fill::findings(project),
            Self::ViaClass => via_class::findings(project),
        };
        (findings, false)
    }
}

/// Where the copper graph lists each connectivity finding
/// ([`CopperGraph::findings`]): unrouted pads in pad order, dangling vias
/// in via order, dangling tracks by their first track, then fills'
/// unjoined islands in fill order. Findings of different nets made by
/// graphs of some nets ([`CopperGraph::of_nets`]) merge into the whole
/// board's list by it.
pub struct ConnectivityOrder<'a> {
    project: &'a Project,
    pads: &'a std::collections::HashMap<(Id, PadId), usize>,
}

impl<'a> ConnectivityOrder<'a> {
    /// The order on `project`, whose connected pads ([`pads`]) are at
    /// `pads`' indices.
    pub fn new(
        project: &'a Project,
        pads: &'a std::collections::HashMap<(Id, PadId), usize>,
    ) -> Self {
        Self { project, pads }
    }

    /// The index of each connected pad of `project` ([`pads`]).
    pub fn pad_index(project: &Project) -> std::collections::HashMap<(Id, PadId), usize> {
        pads(project)
            .iter()
            .enumerate()
            .map(|(k, p)| ((p.instance, p.pad), k))
            .collect()
    }

    /// The finding's place in the list.
    pub fn key(&self, finding: &Finding) -> (u8, usize) {
        let first = |index: &dyn Fn(&ObjectRef) -> Option<usize>| {
            finding
                .objects
                .iter()
                .filter_map(index)
                .min()
                .unwrap_or(usize::MAX)
        };
        let project = self.project;
        match finding.code.as_str() {
            "board.unrouted" => (
                0,
                first(&|o| match *o {
                    ObjectRef::Pad { instance, pad } => self.pads.get(&(instance, pad)).copied(),
                    _ => None,
                }),
            ),
            "board.via_dangling" => (
                1,
                first(&|o| match *o {
                    ObjectRef::Via { id } => at(&project.vias, id, |v| v.id),
                    _ => None,
                }),
            ),
            "board.track_dangling" => (
                2,
                first(&|o| match *o {
                    ObjectRef::Track { id } => at(&project.tracks, id, |t| t.id),
                    _ => None,
                }),
            ),
            _ => (
                3,
                first(&|o| match *o {
                    ObjectRef::Fill { id } => at(&project.fills, id, |f| f.id),
                    _ => None,
                }),
            ),
        }
    }

    /// The net a connectivity finding is about: its net, or its fill's.
    pub fn net(&self, finding: &Finding) -> Option<Id> {
        finding.objects.iter().find_map(|o| match *o {
            ObjectRef::Net { id } => Some(id),
            ObjectRef::Fill { id } => self.project.fill(id).and_then(|f| f.net),
            _ => None,
        })
    }
}

/// The index of the item of `items` with identity `id`: a binary search
/// (collections are kept in identity order but for undone deletions), with
/// a scan when it misses.
fn at<T>(items: &[T], id: Id, key: impl Fn(&T) -> Id) -> Option<usize> {
    match items.binary_search_by_key(&id, &key) {
        Ok(k) => Some(k),
        Err(_) => items.iter().position(|item| key(item) == id),
    }
}

/// Every board check family's findings, in [`BoardCheck::ALL`]'s order,
/// and whether the clearance check stopped at its limit (the families
/// after it in [`BoardCheck::after_copper_limit`] are then left out).
/// `kept` may give a family's findings from an earlier state that the
/// caller knows are unchanged; the family is then not run.
pub fn check_families(
    project: &Project,
    copper: &BoardCopper,
    graph: &GraphSource<'_>,
    settings: &CheckSettings,
    kept: &mut dyn FnMut(BoardCheck) -> Option<Vec<Finding>>,
) -> (Vec<(BoardCheck, Vec<Finding>)>, bool) {
    let mut out = Vec::with_capacity(BoardCheck::ALL.len());
    let mut stopped = false;
    for check in BoardCheck::ALL {
        if stopped && check.after_copper_limit() {
            continue;
        }
        let mut findings = match kept(check) {
            Some(findings) => findings,
            None => {
                let (findings, stop) = check.run(project, copper, graph, settings);
                stopped |= stop;
                findings
            }
        };
        if matches!(check, BoardCheck::Clearance | BoardCheck::Fills)
            && project.instances.iter().any(|i| !i.ties.is_empty())
        {
            findings.retain(|f| {
                !matches!(
                    f.code.as_str(),
                    "board.clearance" | "board.overlap" | "fill.clearance" | "fill.overlap"
                ) || !graph.get().accounts_for(&f.objects)
            });
        }
        out.push((check, findings));
    }
    (out, stopped)
}

/// Airwires between physically disconnected copper islands; see
/// [`CopperGraph::airwires`].
pub fn airwires(project: &Project) -> Vec<(WorldPad, WorldPad)> {
    CopperGraph::of(project).airwires()
}

#[cfg(test)]
mod tests;
