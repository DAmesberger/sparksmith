//! Footprint content in board coordinates: the one placement transform and
//! the placed pad and graphic queries every view and check reads.
use crate::diagnostics::{Error, Result};
use crate::geometry::{Point, Shape, Transform2, Transform3, cos_sin};
use crate::ids::{Endpoint, GraphicId, Id};
use crate::instances;
use crate::library::graphics::{
    DrawingLayer, Fill, Item, PlacedText, Side, TextContent, TextMetric,
};
use crate::library::{
    Datum, FiducialRole, PAD_ARC_TOLERANCE_MM, PackageDefinition, Pad, PadKind, PadSide,
};
use crate::model::fill::PadFillOverride;
use crate::model::{CopperLayer, Layer, Placement, Project, Stackup};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Footprint-local to board coordinates for one placement: move the package
/// datum to the origin and undo its rotation, reflect across the x-axis on
/// the bottom (rigid Rx(pi)), rotate by the placement, then move to the
/// placement position.
pub fn placement_transform(placement: &Placement, datum: &Datum) -> Transform2 {
    let to_datum =
        Transform2::translation([-datum.offset()[0], -datum.offset()[1]]).then(&Transform2 {
            translate: [0.0, 0.0],
            rotation_deg: -datum.rotation_deg(),
            mirror: false,
        });
    to_datum.then(&Transform2 {
        translate: placement.position,
        rotation_deg: placement.rotation_deg,
        mirror: placement.layer == Layer::Bottom,
    })
}

/// Model frame → board frame of one placement, mm.
///
/// Equal to the footprint-frame chain the 2D drawing and the 3D board use
/// (`library::model_frame` then `layout::placement_transform`, the seating
/// plane at its side's land copper): the footprint datum that chain applies
/// and undoes cancels, leaving the model placement in the datum frame.
/// Refused when the stackup has no copper on the placement's side.
pub fn model_to_board(
    placement: &Placement,
    package: &PackageDefinition,
    stackup: &Stackup,
) -> Result<Transform3> {
    let z = seat_height(placement, stackup)?;
    let side = match placement.layer {
        Layer::Top => Transform3::IDENTITY,
        Layer::Bottom => Transform3::rot_x(180.0),
    };
    let [x, y] = placement.position;
    Ok(Transform3::translation([x, y, z])
        .then(&Transform3::rot_z(placement.rotation_deg))
        .then(&side)
        .then(&package.placement.transform()))
}

/// The board height of a placement's seating plane, mm: the outer face of
/// its side's land copper. Refused when the stackup has no copper there.
pub fn seat_height(placement: &Placement, stackup: &Stackup) -> Result<f64> {
    let layer = match placement.layer {
        Layer::Top => CopperLayer::Top,
        Layer::Bottom => CopperLayer::Bottom,
    };
    let span = stackup.copper_span(layer).ok_or_else(|| {
        Error::Invalid(format!(
            "interface.datum: the stackup has no {} copper to seat a part on",
            layer.name()
        ))
    })?;
    Ok(match placement.layer {
        Layer::Top => span.1,
        Layer::Bottom => span.0,
    })
}

/// Undo the datum rotation about the datum (identity at 0°).
pub(super) fn undo_datum_rotation(p: Point, datum: &Datum) -> Point {
    if datum.rotation_deg() == 0.0 {
        return p;
    }
    let (c, s) = cos_sin(-datum.rotation_deg());
    let d = [p[0] - datum.offset()[0], p[1] - datum.offset()[1]];
    [
        datum.offset()[0] + c * d[0] - s * d[1],
        datum.offset()[1] + s * d[0] + c * d[1],
    ]
}

/// One placed pad: the footprint-local pad and its transform to the board.
/// Unconnected and terminal-less pads are included.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct PlacedPad {
    pub instance: Id,
    /// The pad as stored, footprint-local.
    pub pad: Pad,
    /// Footprint-local to board coordinates.
    pub transform: Transform2,
    /// The copper layer of a surface pad after the side flip; a
    /// through-hole pad records the placement layer and spans all layers.
    pub layer: Layer,
    pub terminal: Option<Endpoint>,
    pub net: Option<Id>,
    /// How fills join this pad, over each fill's settings: the pad's own
    /// override over its part's ([`Placement::fill_override_of`]).
    #[serde(default, skip_serializing_if = "PadFillOverride::is_empty")]
    pub fill_override: PadFillOverride,
}

/// Every pad of every placed part, in placement then pad order.
pub fn placed_pads(project: &Project) -> Vec<PlacedPad> {
    let nets = net_by_terminal(project);
    let mut result = Vec::new();
    for placement in &project.placements {
        let Some(instance) = project.instance(placement.instance) else {
            continue;
        };
        let Some(physical) = instances::resolve(&project.library, instance)
            .ok()
            .and_then(|r| r.physical)
        else {
            continue;
        };
        let footprint = &physical.footprint.definition;
        let transform = placement_transform(placement, &footprint.datum);
        let marks_only = footprint
            .pads
            .iter()
            .all(|pad| pad.kind == PadKind::Fiducial);
        for pad in &footprint.pads {
            let terminal = pad
                .terminal
                .and_then(|t| physical.binding.definition.terminal_of(t))
                .map(|terminal| Endpoint {
                    instance: instance.id,
                    terminal,
                });
            let layer = match pad.side {
                PadSide::Top => placement.layer,
                PadSide::Bottom => opposite(placement.layer),
            };
            // Older footprints recorded an unqualified mark. A mark on a
            // package aligns that package. A footprint of only marks aligns the board.
            let mut pad = pad.clone();
            if pad.kind == PadKind::Fiducial {
                pad.fiducial_role.get_or_insert(if marks_only {
                    FiducialRole::Global
                } else {
                    FiducialRole::Local
                });
            }
            result.push(PlacedPad {
                instance: instance.id,
                transform,
                layer,
                net: terminal.and_then(|t| nets.get(&t).copied()),
                terminal,
                fill_override: placement.fill_override_of(pad.id),
                pad,
            });
        }
    }
    result
}

/// The board layer a footprint graphic is drawn on.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BoardDrawing {
    Silkscreen(Layer),
    Courtyard(Layer),
    Assembly(Layer),
}

/// A footprint graphic's content in board coordinates.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PlacedItem {
    Shape {
        shape: Shape,
        width: f64,
        filled: bool,
    },
    Text(PlacedText),
}

/// One footprint graphic of one placed part.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct PlacedGraphic {
    pub instance: Id,
    pub graphic: GraphicId,
    pub layer: BoardDrawing,
    pub item: PlacedItem,
}

/// Every footprint graphic of every placed part, including parts without
/// nets. `Side::Component` maps to the placement layer; on the bottom, text
/// is marked mirrored and arcs reverse (through `Shape::transformed`).
/// Designator text shows the instance designator and literal text itself;
/// other text content resolves to empty text for now. Hidden text is left
/// out. A text the instance moved (`text_placements`) is placed there.
pub fn placed_graphics(project: &Project) -> Vec<PlacedGraphic> {
    let mut result = Vec::new();
    for placement in &project.placements {
        let Some(instance) = project.instance(placement.instance) else {
            continue;
        };
        let Some(physical) = instances::resolve(&project.library, instance)
            .ok()
            .and_then(|r| r.physical)
        else {
            continue;
        };
        let footprint = &physical.footprint.definition;
        let t = placement_transform(placement, &footprint.datum);
        for graphic in &footprint.graphics {
            let side = match graphic.side {
                Side::Component => placement.layer,
                Side::Opposite => opposite(placement.layer),
            };
            let layer = match graphic.layer {
                DrawingLayer::Silkscreen => BoardDrawing::Silkscreen(side),
                DrawingLayer::Courtyard => BoardDrawing::Courtyard(side),
                DrawingLayer::Assembly => BoardDrawing::Assembly(side),
            };
            let item = match &graphic.item {
                Item::Shape {
                    shape,
                    width_nm,
                    fill,
                } => PlacedItem::Shape {
                    shape: shape.to_mm().transformed(&t),
                    width: width_nm.to_mm_f64(),
                    filled: *fill != Fill::None,
                },
                Item::Text(text) => {
                    if !text.visible {
                        continue;
                    }
                    let content = match &text.content {
                        TextContent::Literal(s) => s.clone(),
                        TextContent::Designator => instance.designator.clone(),
                        _ => String::new(),
                    };
                    // Where this instance prints it, in the footprint's axes.
                    let moved = instance
                        .text_placements
                        .get(&graphic.id)
                        .copied()
                        .unwrap_or_default();
                    let a = text.at.to_mm_f64();
                    let at = [a[0] + moved.offset[0], a[1] + moved.offset[1]];
                    let rotation = text.rotation.degrees() + moved.turn.degrees();
                    PlacedItem::Text(PlacedText {
                        graphic: Some(graphic.id),
                        text: content,
                        at: t.apply(at),
                        height: text.height_nm.to_mm_f64(),
                        rotation_deg: t.apply_angle(rotation).rem_euclid(360.0),
                        justify: text.justify,
                        mirrored: t.mirror,
                        line_spacing: text.line_spacing,
                    })
                }
            };
            result.push(PlacedGraphic {
                instance: instance.id,
                graphic: graphic.id,
                layer,
                item,
            });
        }
    }
    result
}

/// A footprint's copper keep-out placed on the board, as a board keepout
/// in board coordinates, with the instance that carries it.
#[derive(Clone, Debug, PartialEq)]
pub struct PlacedKeepout {
    pub instance: Id,
    /// `id` is the instance's: a placed keep-out has no identity of its
    /// own and is edited only through its footprint.
    pub keepout: crate::model::Keepout,
}

/// Every footprint keep-out of every placed part (an antenna clearance):
/// its outline through the placement transform (arcs reverse on the
/// bottom), on every copper layer or the placement's outer layer, forbidding
/// fill copper, tracks and vias, and pads when the footprint says so.
pub fn placed_keepouts(project: &Project) -> Vec<PlacedKeepout> {
    use crate::library::KeepoutScope;
    use crate::model::fill::{KeepoutLayers, KeepoutRules};
    let mut result = Vec::new();
    for placement in &project.placements {
        let Some(instance) = project.instance(placement.instance) else {
            continue;
        };
        let Some(physical) = instances::resolve(&project.library, instance)
            .ok()
            .and_then(|r| r.physical)
        else {
            continue;
        };
        let footprint = &physical.footprint.definition;
        if footprint.keepouts.is_empty() {
            continue;
        }
        let t = placement_transform(placement, &footprint.datum);
        for k in &footprint.keepouts {
            let outline = transform_outline(&k.outline, &t);
            let layers = match k.scope {
                KeepoutScope::AllCopper => KeepoutLayers::All,
                KeepoutScope::ComponentSide => {
                    KeepoutLayers::Only(vec![crate::model::CopperLayer::from(placement.layer)])
                }
                KeepoutScope::OppositeSide => {
                    KeepoutLayers::Only(vec![crate::model::CopperLayer::from(opposite(
                        placement.layer,
                    ))])
                }
            };
            result.push(PlacedKeepout {
                instance: instance.id,
                keepout: crate::model::Keepout {
                    id: instance.id,
                    name: if k.name.is_empty() {
                        instance.designator.clone()
                    } else {
                        format!("{} {}", instance.designator, k.name)
                    },
                    outline,
                    layers,
                    forbid: KeepoutRules {
                        fills: true,
                        tracks: true,
                        vias: true,
                        pads: k.forbid_pads,
                    },
                },
            });
        }
    }
    result
}

pub(crate) fn opposite(layer: Layer) -> Layer {
    match layer {
        Layer::Top => Layer::Bottom,
        Layer::Bottom => Layer::Top,
    }
}

fn net_by_terminal(project: &Project) -> BTreeMap<Endpoint, Id> {
    project
        .nets
        .iter()
        .flat_map(|net| {
            net.terminals
                .iter()
                .map(move |terminal| (*terminal, net.id))
        })
        .collect()
}

/// A drilled hole of a placed pad in board coordinates.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct PlacedHole {
    /// The drill outline: a circle, or a stadium for a slot.
    pub shape: Shape,
    pub plated: bool,
}

impl PlacedPad {
    /// The pad origin (shape centre) on the board.
    pub fn center(&self) -> Point {
        self.transform.apply(self.pad.position())
    }

    /// How far the pad's solder mask opening grows past its land, mm:
    /// the pad's own expansion, else the board's (`board_expansion`, the
    /// effective `mask_expansion` rule); `None` when the pad is tented. The
    /// one reading the mask film (`fabrication::artwork`) and the checks
    /// that need a pad's opening share.
    pub fn mask_expansion(&self, board_expansion: f64) -> Option<f64> {
        match self.pad.mask {
            crate::library::MaskRule::Inherit => Some(board_expansion),
            crate::library::MaskRule::Tented => None,
            crate::library::MaskRule::Expansion(x) => Some(x.to_mm_f64()),
        }
    }

    /// Pad frame to board coordinates.
    pub fn pad_transform(&self) -> Transform2 {
        self.pad.frame().then(&self.transform)
    }

    /// Whether the pad has copper on `layer` ([`super::pad_copper_on`]):
    /// plated through-hole pads on every layer, surface kinds on their own,
    /// a bare hole on none.
    pub fn on_copper(&self, layer: impl Into<CopperLayer>) -> bool {
        super::pad_copper_on(self.pad.kind, self.layer, layer.into())
    }

    /// The copper (or, for a bare hole, keep-out) outline on the board.
    pub fn outline(&self) -> Shape {
        self.pad
            .shape
            .outline(PAD_ARC_TOLERANCE_MM)
            .transformed(&self.pad_transform())
    }

    /// The pad's drilled holes on the board: its own hole, then its vias.
    /// The bounds of everything the pad puts on the board: its outline
    /// and its holes.
    pub fn extent(&self) -> crate::geometry::Bounds {
        self.holes()
            .iter()
            .map(|h| h.shape.bounds(0.0))
            .fold(self.outline().bounds(0.0), crate::geometry::Bounds::union)
    }

    pub fn holes(&self) -> Vec<PlacedHole> {
        let t = self.pad_transform();
        self.pad
            .hole
            .iter()
            .map(|hole| PlacedHole {
                shape: hole.shape().transformed(&t),
                plated: hole.plated,
            })
            .chain(self.pad.vias.iter().map(|via| PlacedHole {
                shape: Shape::Circle {
                    center: t.apply(via.at()),
                    radius: via.drill() / 2.0,
                },
                plated: true,
            }))
            .collect()
    }

    /// Whether `p` (board coordinates) lies on the pad's outline area.
    pub fn contains(&self, p: Point) -> bool {
        self.outline().distance(p, true) <= 0.0
    }
}

/// The topmost (last drawn) pad under `p` among the pads `visible` accepts.
pub fn pick_pad(
    pads: &[PlacedPad],
    p: Point,
    visible: impl Fn(&PlacedPad) -> bool,
) -> Option<&PlacedPad> {
    pads.iter()
        .rev()
        .find(|pad| visible(pad) && pad.contains(p))
}

/// The topmost (last drawn) graphic within `tolerance` mm of `p` among the
/// graphics `visible` accepts. Strokes pick within half their width, filled
/// shapes inside, text inside its box under `metric`.
pub fn pick_graphic(
    graphics: &[PlacedGraphic],
    p: Point,
    tolerance: f64,
    metric: TextMetric,
    visible: impl Fn(&PlacedGraphic) -> bool,
) -> Option<&PlacedGraphic> {
    graphics.iter().rev().find(|g| {
        visible(g)
            && match &g.item {
                PlacedItem::Shape {
                    shape,
                    width,
                    filled,
                } => shape.distance(p, *filled) <= width / 2.0 + tolerance,
                PlacedItem::Text(text) => text.bounds(metric).inflated(tolerance).contains(p),
            }
    })
}

#[cfg(test)]
mod tests;

/// One fabricated bridge, including its geometry and explicit pad and net identities.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct PlacedBridge {
    pub group: crate::ids::TieId,
    pub instance: Id,
    pub bridge: crate::ids::BridgeId,
    pub layer: CopperLayer,
    pub nets: Vec<Id>,
    pub pads: Vec<crate::ids::PadId>,
    pub outline: crate::library::Outline,
}

/// Fabricated copper is independent of which assembly variant is selected.
pub fn placed_bridges(project: &Project) -> Vec<PlacedBridge> {
    if !project.instances.iter().any(|i| !i.ties.is_empty()) {
        return Vec::new();
    }
    let nets = net_by_terminal(project);
    let mut bridges = Vec::new();
    for placement in &project.placements {
        let Some(i) = project
            .instance(placement.instance)
            .filter(|i| !i.ties.is_empty())
        else {
            continue;
        };
        let Some(view) = instances::resolve(&project.library, i)
            .ok()
            .and_then(|r| r.physical)
        else {
            continue;
        };
        let fp = &view.footprint.definition;
        let Some(component) = project.library.component(i.component) else {
            continue;
        };
        if fp.bridges.is_empty() {
            continue;
        }
        let pads =
            crate::library::tie::TiePads::of(fp, &view.binding.definition, &component.definition);
        let transform = placement_transform(placement, &fp.datum);
        for bridge in &fp.bridges {
            let Some(group) = pads.owner(bridge) else {
                continue;
            };
            let mut member_nets = bridge
                .pads
                .iter()
                .filter_map(|p| {
                    let terminal = pads.terminal(*p)?;
                    nets.get(&Endpoint {
                        instance: i.id,
                        terminal,
                    })
                    .copied()
                })
                .collect::<Vec<_>>();
            member_nets.sort_unstable();
            member_nets.dedup();
            bridges.push(PlacedBridge {
                group,
                instance: i.id,
                bridge: bridge.id,
                layer: match bridge.side {
                    PadSide::Top => placement.layer,
                    PadSide::Bottom => opposite(placement.layer),
                }
                .into(),
                nets: member_nets,
                pads: bridge.pads.clone(),
                outline: transform_outline(&bridge.outline, &transform),
            });
        }
    }
    bridges
}

fn transform_outline(src: &crate::library::Outline, t: &Transform2) -> crate::library::Outline {
    use crate::library::{Outline, Segment};
    Outline {
        start: t.apply(src.start),
        segments: src
            .segments
            .iter()
            .map(|s| match *s {
                Segment::Line { to } => Segment::Line { to: t.apply(to) },
                Segment::Arc { to, bulge } => Segment::Arc {
                    to: t.apply(to),
                    bulge: if t.mirror { -bulge } else { bulge },
                },
            })
            .collect(),
    }
}
