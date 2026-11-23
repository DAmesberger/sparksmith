//! The board's physical inventory: every land and every drilled or routed
//! hole that is fabricated, whether or not a net names it. Clearance
//! checks, the router's obstacles and its final validation, the hole
//! rules and the fabrication output read these lists, so what is checked
//! is what is plotted and drilled.
//!
//! - A **land** is copper that is not a track, via or fill: every pad with
//!   copper (an unconnected pin, a fiducial or a mechanical tab included,
//!   with no net) and the copper washer of a plated mounting hole.
//! - A **hole** is every drill and routed slot: board vias, pad holes and
//!   slots (plated and not), pad-owned vias, board mounting holes and
//!   plated board slots. Non-plated board cutouts and slots are milled on
//!   the profile instead.
//!
//! A net answers which conductors are meant to join; a land without one
//! still occupies copper, so copper of any net that reaches it is a
//! clearance finding. Which copper joins is still the copper graph's alone
//! ([`super::connectivity`]); it reads the lands that have a net
//! ([`super::pads`]).
use super::copper::CopperRing;
use super::limits::mm;
use super::placed::PlacedPad;
use crate::diagnostics::{Finding, ObjectRef};
use crate::geometry::{Point, Shape, distance};
use crate::ids::{Endpoint, Id, PadId};
use crate::library::names::capitalised;
use crate::library::{HoleShape, Outline, PadCopper, Segment};
use crate::model::board::{BoardFeatureId, CutoutKind, arc_geometry};
use crate::model::{CopperLayer, Layer, Project};
use crate::rules::RuleKey;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// What a land belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LandOwner {
    /// A placed pad with copper.
    Pad { instance: Id, pad: PadId },
    /// The copper ring of a plated board mounting hole.
    Washer { hole: BoardFeatureId },
}

/// Which copper a land is part of, for clearance: copper of one conductor
/// never keeps a clearance from itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Conductor {
    /// A net's copper.
    Net(Id),
    /// The pads of one terminal not on a net (an unconnected pin).
    Terminal(Endpoint),
    /// Copper of nothing electrical: a fiducial, a tab, a washer. Two such
    /// lands of one placed part are the footprint's own business.
    Alone { instance: Option<Id> },
}

impl Conductor {
    /// Whether copper of `self` and of `other` may touch without a short.
    pub fn same(self, other: Self) -> bool {
        match (self, other) {
            (Self::Net(a), Self::Net(b)) => a == b,
            (Self::Terminal(a), Self::Terminal(b)) => a == b,
            (Self::Alone { instance: Some(a) }, Self::Alone { instance: Some(b) }) => a == b,
            _ => false,
        }
    }

    /// The net, when the copper has one.
    pub fn net(self) -> Option<Id> {
        match self {
            Self::Net(net) => Some(net),
            _ => None,
        }
    }
}

/// One land on the board.
#[derive(Clone, Debug, PartialEq)]
pub struct Land {
    pub owner: LandOwner,
    pub conductor: Conductor,
    /// The copper side of a one-sided land; a through land is on every
    /// copper layer.
    pub layer: Layer,
    pub through: bool,
    /// The board outline of its copper.
    pub outline: Shape,
    pub ring: CopperRing,
    /// How a finding names it: "J1 pad 3", "the plated mounting hole at
    /// (5, 5) mm".
    pub name: String,
}

impl Land {
    /// Whether the land has copper on `layer`.
    pub fn on(&self, layer: impl Into<CopperLayer>) -> bool {
        self.through || CopperLayer::from(self.layer) == layer.into()
    }

    pub fn net(&self) -> Option<Id> {
        self.conductor.net()
    }

    /// The document object a finding names for it: the pad, or the
    /// mounting hole of a washer.
    pub fn object(&self) -> ObjectRef {
        match self.owner {
            LandOwner::Pad { instance, pad } => ObjectRef::Pad { instance, pad },
            LandOwner::Washer { hole } => ObjectRef::BoardFeature { feature: hole.0 },
        }
    }
}

/// How a mounting hole is named in findings.
pub fn mounting_hole_name(plated: bool, at: Point) -> String {
    format!(
        "the {} mounting hole at ({}, {}) mm",
        if plated { "plated" } else { "unplated" },
        super::limits::mm(at[0]),
        super::limits::mm(at[1])
    )
}

/// How a board feature is named in issue titles: "plated mounting hole at
/// (40, 20) mm", "plated board slot at (5, 5) mm", "cutout at …".
pub fn feature_name(project: &Project, id: BoardFeatureId) -> String {
    let board = project.board();
    if let Some(hole) = board.holes.iter().find(|h| h.id == id) {
        let name = mounting_hole_name(hole.plated, hole.at);
        return name.strip_prefix("the ").unwrap_or(&name).to_owned();
    }
    let Some(cutout) = board.cutouts.iter().find(|c| c.id == Some(id)) else {
        return "a board feature".into();
    };
    let what = match cutout.kind {
        CutoutKind::Cutout => "cutout",
        CutoutKind::Slot { plated: true } => "plated board slot",
        CutoutKind::Slot { plated: false } => "board slot",
    };
    match stadium(&cutout.outline).map(|f| f.at()) {
        Some(at) => format!("{what} at ({}, {}) mm", mm(at[0]), mm(at[1])),
        None => format!("a {what}"),
    }
}

/// Every land of `project`: each placed pad with copper, in placement then
/// pad order, then each plated mounting hole's washer.
pub fn lands(project: &Project) -> Vec<Land> {
    lands_of(project, &super::placed_pads(project))
}

/// [`lands`] over `placed`, the board's placed pads.
pub fn lands_of(project: &Project, placed: &[PlacedPad]) -> Vec<Land> {
    let labels = super::pad_labels(project);
    let mut out = Vec::new();
    for pad in placed {
        if pad.pad.kind.copper() == PadCopper::None {
            continue;
        }
        let conductor = match (pad.net, pad.terminal) {
            (Some(net), _) => Conductor::Net(net),
            (None, Some(terminal)) => Conductor::Terminal(terminal),
            (None, None) => Conductor::Alone {
                instance: Some(pad.instance),
            },
        };
        let label = labels
            .get(&(pad.instance, pad.pad.id))
            .cloned()
            .unwrap_or_else(|| pad.pad.id.to_string());
        let designator = project
            .instance(pad.instance)
            .map_or("a part", |i| i.designator.as_str());
        let outline = pad.outline();
        out.push(Land {
            owner: LandOwner::Pad {
                instance: pad.instance,
                pad: pad.pad.id,
            },
            conductor,
            layer: pad.layer,
            through: pad.pad.kind.copper() == PadCopper::Through,
            ring: CopperRing::of_placed_pad(pad),
            outline,
            name: format!("{designator} pad {label}"),
        });
    }
    out.extend(washers(project));
    out
}

/// The copper washer of each plated mounting hole that has one, as a land
/// on every copper layer.
pub fn washers(project: &Project) -> Vec<Land> {
    project
        .board()
        .holes
        .iter()
        .filter_map(|hole| {
            let (true, Some(diameter)) = (hole.plated, hole.pad_diameter) else {
                return None;
            };
            Some(Land {
                owner: LandOwner::Washer { hole: hole.id },
                conductor: Conductor::Alone { instance: None },
                layer: Layer::Top,
                through: true,
                outline: Shape::Circle {
                    center: hole.at,
                    radius: diameter / 2.0,
                },
                ring: CopperRing::of_circle(hole.at, diameter),
                name: mounting_hole_name(true, hole.at),
            })
        })
        .collect()
}

/// A round hit or a routed slot (round ends of `width`, centre line from
/// `from` to `to`), board millimetres.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum HoleFeature {
    Round { at: Point, diameter: f64 },
    Slot { from: Point, to: Point, width: f64 },
}

impl HoleFeature {
    /// The tool diameter: the drill or the slot width.
    pub fn diameter(&self) -> f64 {
        match *self {
            Self::Round { diameter, .. } => diameter,
            Self::Slot { width, .. } => width,
        }
    }

    pub fn is_slot(&self) -> bool {
        matches!(self, Self::Slot { .. })
    }

    /// Its outline as clearance measures it: a disc, or the stadium of a
    /// slot, both exact from their centre lines.
    pub fn ring(&self) -> CopperRing {
        match *self {
            Self::Round { at, diameter } => CopperRing::of_circle(at, diameter),
            Self::Slot { from, to, width } => CopperRing::of_stadium(from, to, width),
        }
    }

    /// Where it is: a hit's centre, a slot's midpoint.
    pub fn at(&self) -> Point {
        match *self {
            Self::Round { at, .. } => at,
            Self::Slot { from, to, .. } => [(from[0] + to[0]) / 2.0, (from[1] + to[1]) / 2.0],
        }
    }
}

/// What a hole is for (the X2 `.AperFunction` of drill files).
#[derive(
    Clone, Copy, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
#[serde(rename_all = "snake_case")]
pub enum HoleFunction {
    /// A board via (`ViaDrill`).
    Via,
    /// A component lead or pad hole, or a pad-owned via (`ComponentDrill`).
    ComponentPin,
    /// A board mounting hole or a footprint's bare hole (`MechanicalDrill`).
    Mechanical,
}

impl HoleFunction {
    pub fn label(self) -> &'static str {
        match self {
            Self::Via => "via",
            Self::ComponentPin => "component",
            Self::Mechanical => "mechanical",
        }
    }
    /// The Gerber X2 aperture function of the hole.
    pub fn aper_function(self) -> &'static str {
        match self {
            Self::Via => "ViaDrill",
            Self::ComponentPin => "ComponentDrill",
            Self::Mechanical => "MechanicalDrill",
        }
    }
}

/// What a hole belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HoleOwner {
    Via(Id),
    /// A placed pad's own hole or slot (`via: None`) or one of its vias.
    Pad {
        instance: Id,
        pad: PadId,
        via: Option<usize>,
    },
    MountingHole(BoardFeatureId),
    /// A plated board slot (a cutout of kind slot).
    BoardSlot(Option<BoardFeatureId>),
}

/// One drilled or routed hole.
#[derive(Clone, Debug, PartialEq)]
pub struct Hole {
    pub feature: HoleFeature,
    pub plated: bool,
    pub function: HoleFunction,
    /// The net of a plated hole, when it has one.
    pub net: Option<Id>,
    pub owner: HoleOwner,
    /// How a finding names it: "J1 pad 5", "a via on GND".
    pub name: String,
}

impl Hole {
    /// The document object a finding names for it: each hole its own, so
    /// a finding (and a waiver) about one hole never names another. A
    /// board slot without a feature id (none is saved without one) is the
    /// board.
    pub fn object(&self) -> ObjectRef {
        match self.owner {
            HoleOwner::Via(id) => ObjectRef::Via { id },
            HoleOwner::Pad {
                instance,
                pad,
                via: None,
            } => ObjectRef::Pad { instance, pad },
            HoleOwner::Pad {
                instance,
                pad,
                via: Some(k),
            } => ObjectRef::PadVia {
                instance,
                pad,
                index: u32::try_from(k).unwrap_or(u32::MAX),
            },
            HoleOwner::MountingHole(id) | HoleOwner::BoardSlot(Some(id)) => {
                ObjectRef::BoardFeature { feature: id.0 }
            }
            HoleOwner::BoardSlot(None) => ObjectRef::Board,
        }
    }

    /// "hole", "slot" or "via-in-pad hole": what it is within its owner.
    pub fn what(&self) -> &'static str {
        match self.owner {
            HoleOwner::Pad { via: Some(_), .. } => "via-in-pad hole",
            _ if self.feature.is_slot() => "slot",
            _ => "hole",
        }
    }
}

/// The holes of one placed pad: its own hole or slot, then its vias.
pub fn pad_holes(pad: &PlacedPad) -> Vec<(HoleFeature, bool)> {
    let t = pad.pad_transform();
    let mut out = Vec::new();
    if let Some(hole) = &pad.pad.hole {
        let feature = match hole.shape {
            HoleShape::Round { diameter_nm } => HoleFeature::Round {
                at: t.apply(hole.offset()),
                diameter: diameter_nm.to_mm_f64(),
            },
            HoleShape::Slot {
                width_nm,
                length_nm,
                angle_mdeg,
            } => {
                let (width, length) = (width_nm.to_mm_f64(), length_nm.to_mm_f64());
                let half = (length.max(width) - width) / 2.0;
                let (c, s) = crate::geometry::cos_sin(angle_mdeg.to_degrees_f64());
                let o = hole.offset();
                HoleFeature::Slot {
                    from: t.apply([o[0] - c * half, o[1] - s * half]),
                    to: t.apply([o[0] + c * half, o[1] + s * half]),
                    width,
                }
            }
        };
        out.push((feature, hole.plated));
    }
    for via in &pad.pad.vias {
        out.push((
            HoleFeature::Round {
                at: t.apply(via.at()),
                diameter: via.drill(),
            },
            true,
        ));
    }
    out
}

/// Every hole of `project` in a fixed order: vias, then pad holes in
/// placement and pad order, then board mounting holes and plated slots.
pub fn holes(project: &Project) -> Vec<Hole> {
    holes_of(project, &super::placed_pads(project))
}

/// A via's drilled hole.
pub(crate) fn via_hole(project: &Project, via: &crate::model::Via) -> Hole {
    Hole {
        feature: HoleFeature::Round {
            at: via.position,
            diameter: via.drill,
        },
        plated: true,
        function: HoleFunction::Via,
        net: Some(via.net),
        owner: HoleOwner::Via(via.id),
        name: format!("a via on {}", project.net_name(via.net)),
    }
}

/// [`holes`] over `placed`, the board's placed pads.
pub fn holes_of(project: &Project, placed: &[PlacedPad]) -> Vec<Hole> {
    let mut out: Vec<Hole> = project
        .vias
        .iter()
        .map(|via| via_hole(project, via))
        .collect();
    out.extend(holes_but_vias(project, placed));
    out
}

/// [`holes_of`] without the via drills: the holes of `placed`, then the
/// board's own holes, in the same order.
pub(crate) fn holes_but_vias(project: &Project, placed: &[PlacedPad]) -> Vec<Hole> {
    let mut out = Vec::new();
    let labels = super::pad_labels(project);
    for pad in placed {
        let label = labels
            .get(&(pad.instance, pad.pad.id))
            .cloned()
            .unwrap_or_else(|| pad.pad.id.to_string());
        let designator = project
            .instance(pad.instance)
            .map_or("a part", |i| i.designator.as_str());
        let own = usize::from(pad.pad.hole.is_some());
        for (k, (feature, plated)) in pad_holes(pad).into_iter().enumerate() {
            let via = k.checked_sub(own);
            out.push(Hole {
                feature,
                plated,
                function: if plated {
                    HoleFunction::ComponentPin
                } else {
                    HoleFunction::Mechanical
                },
                net: pad.net.filter(|_| plated),
                owner: HoleOwner::Pad {
                    instance: pad.instance,
                    pad: pad.pad.id,
                    via,
                },
                name: format!("{designator} pad {label}"),
            });
        }
    }
    out.extend(board_holes(project));
    out
}

/// The holes of the board shape: each mounting hole, then each plated
/// board slot, in the board's order ([`holes`] ends with these).
pub fn board_holes(project: &Project) -> Vec<Hole> {
    let mut out = Vec::new();
    let board = project.board();
    for hole in &board.holes {
        out.push(Hole {
            feature: HoleFeature::Round {
                at: hole.at,
                diameter: hole.drill,
            },
            plated: hole.plated,
            function: HoleFunction::Mechanical,
            net: None,
            owner: HoleOwner::MountingHole(hole.id),
            name: mounting_hole_name(hole.plated, hole.at),
        });
    }
    for cutout in &board.cutouts {
        if cutout.kind == (CutoutKind::Slot { plated: true })
            && let Some(feature) = stadium(&cutout.outline)
        {
            out.push(Hole {
                feature,
                plated: true,
                function: HoleFunction::Mechanical,
                net: None,
                owner: HoleOwner::BoardSlot(cutout.id),
                name: format!(
                    "the plated board slot at ({}, {}) mm",
                    super::limits::mm(feature.at()[0]),
                    super::limits::mm(feature.at()[1])
                ),
            });
        }
    }
    out
}

/// The rules every existing object of the inventory keeps, whatever
/// placed it (a route, an import, a class or profile change):
///
/// - `board.track_width`: each track at least its net's minimum width (its
///   class's, else the effective rule), as routing refuses a narrower one.
/// - `board.via_drill_size` and `board.drill_size`: each hole of
///   [`holes`] (vias; plated and unplated pad holes, slots by their width
///   and pad-owned vias; mounting holes and plated board slots) at least
///   the effective minimum drill, the smallest tool the fabricator drills.
pub fn findings(project: &Project) -> Vec<Finding> {
    let mut out: Vec<Finding> = project
        .tracks
        .iter()
        .filter_map(|track| track_width(project, track))
        .collect();
    let drill_min = drill_min(project);
    out.extend(
        holes(project)
            .iter()
            .filter_map(|hole| drill_size(project, &drill_min, hole)),
    );
    out
}

/// [`findings`] as items: each track's, each via's, then the other holes'
/// together. Made from `last` (an earlier state's items and what changed
/// since) when given: only changed tracks and vias are checked again, and
/// the other holes, which read no track or via, keep their findings.
pub fn items(
    project: &Project,
    last: Option<(&super::Items, &super::CopperChanges)>,
) -> super::Items {
    use super::ItemKey;
    let keys: Vec<ItemKey> = project
        .tracks
        .iter()
        .map(|t| ItemKey::Track(t.id))
        .chain(project.vias.iter().map(|v| ItemKey::Via(v.id)))
        .chain([ItemKey::Rings])
        .collect();
    let drill_min = drill_min(project);
    let again =
        |_: usize, key: &ItemKey| last.is_some_and(|(_, changes)| changes.keys.contains(key));
    let last = last.map(|(items, _)| (items, &again as &dyn Fn(usize, &ItemKey) -> bool));
    let tracks = project.tracks.len();
    let vias = project.vias.len();
    let made = super::Items::made(&keys, last, |i| {
        let found: Vec<Finding> = if let Some(track) = project.tracks.get(i) {
            track_width(project, track).into_iter().collect()
        } else if let Some(via) = i.checked_sub(tracks).and_then(|k| project.vias.get(k)) {
            drill_size(project, &drill_min, &via_hole(project, via))
                .into_iter()
                .collect()
        } else {
            holes(project)
                .get(vias..)
                .into_iter()
                .flatten()
                .filter_map(|hole| drill_size(project, &drill_min, hole))
                .collect()
        };
        Some((found, 0))
    });
    made.unwrap_or_default()
}

/// The finding of a track narrower than its net's minimum width.
fn track_width(project: &Project, track: &crate::model::Track) -> Option<Finding> {
    let min = project
        .rules
        .track_width_min(super::net_class(project, track.net));
    (track.width < min.value - super::EPS).then(|| {
        Finding::new(
            "board.track_width",
            format!(
                "A track on {}: width {} mm is under the minimum; needs {} mm ({}: min track width)",
                project.net_name(track.net),
                mm(track.width),
                mm(min.value),
                min.source.describe()
            ),
            vec![ObjectRef::Track { id: track.id }],
        )
        .requiring(super::limits::at_most(
            &project.rules,
            &min.source,
            RuleKey::TrackWidthMin,
            track.width,
        ))
    })
}

fn drill_min(project: &Project) -> super::limits::Need {
    let rules = &project.rules;
    super::limits::need(rules, &rules.effective(), RuleKey::DrillMin)
}

/// The finding of a hole drilled smaller than the minimum drill.
fn drill_size(project: &Project, drill_min: &super::limits::Need, hole: &Hole) -> Option<Finding> {
    let size = hole.feature.diameter();
    if size >= drill_min.mm - super::EPS {
        return None;
    }
    let (code, text) = match hole.owner {
        HoleOwner::Via(_) => (
            "board.via_drill_size",
            format!(
                "A via on {}: drill {} mm is under the minimum; {}",
                hole.net.map_or("no net", |n| project.net_name(n)),
                mm(size),
                drill_min.text()
            ),
        ),
        _ => (
            "board.drill_size",
            format!(
                "{}: {} {} mm is under the minimum; {}",
                capitalised(&hole.name),
                match hole.what() {
                    "hole" if !matches!(hole.owner, HoleOwner::Pad { .. }) => "drill",
                    what => what,
                },
                mm(size),
                drill_min.text()
            ),
        ),
    };
    Some(Finding::new(code, text, vec![hole.object()]).requiring(drill_min.at_most(size)))
}

/// A closed outline that is a straight slot (two half-turn arcs joined by
/// two lines, or a circle of two half turns) as a routed slot or a round
/// hit; `None` for any other shape.
pub fn stadium(outline: &Outline) -> Option<HoleFeature> {
    let mut from = outline.start;
    let mut arcs = Vec::new();
    let mut lines = 0;
    for s in &outline.segments {
        let to = s.to();
        match *s {
            Segment::Line { .. } if distance(from, to) > 1e-9 => lines += 1,
            Segment::Line { .. } => {}
            Segment::Arc { bulge, .. } => {
                let arc = arc_geometry(from, to, bulge)?;
                if (arc.sweep.abs() - std::f64::consts::PI).abs() > 1e-6 {
                    return None;
                }
                arcs.push(arc);
            }
        }
        from = to;
    }
    let [a, b] = arcs.as_slice() else {
        return None;
    };
    if (a.radius - b.radius).abs() > 1e-6 || !(lines == 2 || lines == 0) {
        return None;
    }
    if distance(a.center, b.center) <= 1e-6 {
        return Some(HoleFeature::Round {
            at: a.center,
            diameter: 2.0 * a.radius,
        });
    }
    (lines == 2).then_some(HoleFeature::Slot {
        from: a.center,
        to: b.center,
        width: 2.0 * a.radius,
    })
}

#[cfg(test)]
pub(crate) mod tests;
