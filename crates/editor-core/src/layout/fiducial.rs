//! Fiducial checks: what an assembler's pick-and-place machine needs to
//! find the board, from the fab profile's assembly rules
//! ([`crate::rules::AssemblyRules`]), never from numbers of Sparksmith's.
//!
//! A fiducial is a pad of kind fiducial. Its clear area is the footprint
//! keep-out on its layer that holds its centre (the `fiducial` generator
//! makes one), else its solder mask opening. Copper in the clear area is a
//! keep-out finding and another part's courtyard in it a courtyard
//! finding, as for any keep-out and courtyard; this module adds:
//!
//! - [`TOO_FEW`]: a board side that carries surface-mount parts (of the
//!   active variant) has fewer fiducials than `fiducial_count_min`.
//! - [`SYMMETRIC`]: with `fiducials_asymmetric`, a side's fiducials (two or
//!   more) map onto themselves under a half turn about the centre of the
//!   board outline's box, each within [`SAME_POINT_MM`], so a board turned
//!   round reads as the right way up.
//! - [`SILK`]: silkscreen ink (a drawing or text, any part's) reaches into
//!   a fiducial's clear area on its side.
//! - [`COPPER`]: a track or via reaches into the clear area of a fiducial
//!   that carries no keep-out (an error: the copper would show in the
//!   opening, or touch the dot).
use super::legend::text_strokes;
use super::limits::cite;
use super::placed::{
    BoardDrawing, PlacedItem, opposite, placed_graphics, placed_keepouts, placed_pads,
};
use crate::diagnostics::FindingCode;
use crate::diagnostics::{Finding, Measured, ObjectRef, Unit};
use crate::geometry::{
    Point, distance, polygon_contains, polygons_overlap, polyline_polygon_distance,
};
use crate::ids::Id;
use crate::instances;
use crate::library::{PadKind, PadSide};
use crate::model::{CopperLayer, Layer, Project};
use crate::rules::RuleKey;
use std::collections::BTreeSet;

/// A side carries surface parts but fewer fiducials than the profile asks.
pub const TOO_FEW: &str = crate::diagnostics::FindingCode::BoardFiducialsTooFew.as_str();
/// A side's fiducials are symmetric about the board's centre.
pub const SYMMETRIC: &str = crate::diagnostics::FindingCode::BoardFiducialsSymmetric.as_str();
/// Silkscreen in a fiducial's clear area.
pub const SILK: &str = crate::diagnostics::FindingCode::BoardFiducialSilk.as_str();
/// A track or via in the clear area of a fiducial that has no keep-out of
/// its own (a footprint made elsewhere): its mask opening.
pub const COPPER: &str = crate::diagnostics::FindingCode::BoardFiducialCopper.as_str();

/// Two fiducial centres closer than this, mm, are one point for the
/// symmetry check: 1 µm, the resolution the fabrication files write
/// coordinates to.
pub const SAME_POINT_MM: f64 = 0.001;

/// Chord tolerance for flattening clear areas and silkscreen, mm.
const CHORD_MM: f64 = 0.005;

/// One placed fiducial pad.
#[derive(Clone, Debug, PartialEq)]
pub struct PlacedFiducial {
    pub instance: Id,
    pub role: crate::library::FiducialRole,
    /// The copper layer of its dot.
    pub layer: Layer,
    /// Its centre on the board, mm.
    pub at: Point,
    /// Its clear area on the board, as closed rings.
    pub clear: Vec<Vec<Point>>,
    /// Whether the clear area is a footprint keep-out, whose own check
    /// (`keepout.track`, `keepout.via`) reports copper in it; otherwise it
    /// is the mask opening and [`COPPER`] does.
    pub kept_out: bool,
}

/// Every fiducial pad of every placed part (populated or not: the copper
/// is etched either way), in placement then pad order. Read from the
/// physical pad list the fabrication films draw ([`placed_pads`]), not from
/// the netted pads, so an unnetted fiducial is found as the Gerber writes
/// it.
pub fn placed_fiducials(project: &Project) -> Vec<PlacedFiducial> {
    let keepouts = placed_keepouts(project);
    let board_mask = project.rules.effective().mask.expansion;
    let mut out = Vec::new();
    for pad in placed_pads(project)
        .into_iter()
        .filter(|p| p.pad.kind == PadKind::Fiducial)
    {
        let frame = pad.pad_transform();
        let at = frame.apply([0.0, 0.0]);
        let mut clear: Vec<Vec<Point>> = keepouts
            .iter()
            .filter(|k| k.instance == pad.instance && k.keepout.on(CopperLayer::from(pad.layer)))
            .map(|k| k.keepout.outline.flatten(CHORD_MM))
            .filter(|ring| ring.len() >= 3 && polygon_contains(ring, at))
            .collect();
        let kept_out = !clear.is_empty();
        if !kept_out && let Some(by) = pad.mask_expansion(board_mask) {
            clear.extend(
                pad.pad
                    .shape
                    .offset_pieces(by, CHORD_MM)
                    .into_iter()
                    .map(|p| p.outer.transformed(&frame).flatten(CHORD_MM)),
            );
        }
        out.push(PlacedFiducial {
            instance: pad.instance,
            role: pad
                .pad
                .fiducial_role
                .unwrap_or(crate::library::FiducialRole::Local),
            layer: pad.layer,
            at,
            clear,
            kept_out,
        });
    }
    out
}

/// The board sides that carry a surface-mount land of a part the active
/// variant fits.
fn assembled_sides(project: &Project) -> BTreeSet<bool> {
    let mut sides = BTreeSet::new();
    for placement in &project.placements {
        let Some(instance) = project.instance(placement.instance) else {
            continue;
        };
        let fitted = instances::variants::fitted(project, instance);
        if !fitted.populated() {
            continue;
        }
        let Some(physical) = fitted.physical else {
            continue;
        };
        for pad in &physical.footprint.definition.pads {
            if pad.kind == PadKind::Smd {
                let layer = match pad.side {
                    PadSide::Top => placement.layer,
                    PadSide::Bottom => opposite(placement.layer),
                };
                sides.insert(layer == Layer::Top);
            }
        }
    }
    sides
}

fn side_name(top: bool) -> &'static str {
    if top { "top" } else { "bottom" }
}

fn designator(project: &Project, id: Id) -> String {
    project
        .instance(id)
        .map_or_else(|| "a part".into(), |i| i.designator.clone())
}

/// Every fiducial finding of `project`.
pub fn findings(project: &Project) -> Vec<Finding> {
    let fiducials = placed_fiducials(project);
    let rules = &project.rules;
    let effective = rules.effective();
    let mut out = Vec::new();
    let on = |top: bool| -> Vec<&PlacedFiducial> {
        fiducials
            .iter()
            .filter(|f| {
                f.role == crate::library::FiducialRole::Global && (f.layer == Layer::Top) == top
            })
            .collect()
    };
    if let Some(min) = effective.assembly.fiducial_count_min.filter(|n| *n > 0) {
        for top in assembled_sides(project).into_iter().rev() {
            let n = on(top).len();
            if n < min as usize {
                out.push(Finding::new(
                    FindingCode::BoardFiducialsTooFew,
                    format!(
                        "The {} side has {n} fiducial{} for its surface-mount parts; needs {min} ({})",
                        side_name(top),
                        if n == 1 { "" } else { "s" },
                        cite(rules, RuleKey::FiducialCountMin)
                    ),
                    // Each side is its own finding: a waiver of one side's
                    // count never accepts the other's.
                    vec![ObjectRef::BoardSide { top }],
                )
                .measuring([Measured::at_least(
                    n as f64,
                    f64::from(min),
                    Unit::Count,
                    0.0,
                )]));
            }
        }
    }
    if effective.assembly.fiducials_asymmetric == Some(true) {
        let b = project.board().bounds();
        let centre = [(b.min[0] + b.max[0]) / 2.0, (b.min[1] + b.max[1]) / 2.0];
        for top in [true, false] {
            let marks = on(top);
            if marks.len() < 2 {
                continue;
            }
            let points: Vec<Point> = marks.iter().map(|m| m.at).collect();
            if half_turn_symmetric(&points, centre) {
                let mut objects: Vec<ObjectRef> = Vec::new();
                for m in &marks {
                    let r = ObjectRef::Instance { id: m.instance };
                    if !objects.contains(&r) {
                        objects.push(r);
                    }
                }
                out.push(Finding::new(
                    FindingCode::BoardFiducialsSymmetric,
                    format!(
                        "The {} side's fiducials are symmetric about the board's centre, so a board turned round looks the same; move one ({})",
                        side_name(top),
                        cite(rules, RuleKey::FiducialsAsymmetric)
                    ),
                    objects,
                ));
            }
        }
    }
    out.extend(silk_findings(project, &fiducials));
    out.extend(copper_findings(project, &fiducials));
    out
}

/// [`COPPER`]: tracks and vias on a fiducial's layer that reach into a
/// clear area no keep-out covers.
fn copper_findings(project: &Project, fiducials: &[PlacedFiducial]) -> Vec<Finding> {
    let layers = project.stackup.layers();
    let mut out = Vec::new();
    for f in fiducials.iter().filter(|f| !f.kept_out) {
        let layer = CopperLayer::from(f.layer);
        let fid = designator(project, f.instance);
        let side = side_name(f.layer == Layer::Top);
        for t in project.tracks.iter().filter(|t| t.layer == layer) {
            if f.clear
                .iter()
                .any(|ring| stroke_enters(&[t.start, t.end], t.width, ring))
            {
                out.push(Finding::new(
                    FindingCode::BoardFiducialCopper,
                    format!(
                        "A track on {} crosses fiducial {fid}'s clear area on the {side} side",
                        project.net_name(t.net)
                    ),
                    vec![
                        ObjectRef::Track { id: t.id },
                        ObjectRef::Instance { id: f.instance },
                    ],
                ));
            }
        }
        for v in project.vias.iter().filter(|v| v.on(layer, &layers)) {
            if f.clear
                .iter()
                .any(|ring| stroke_enters(&[v.position], v.diameter, ring))
            {
                out.push(Finding::new(
                    FindingCode::BoardFiducialCopper,
                    format!(
                        "A via on {} lies in fiducial {fid}'s clear area on the {side} side",
                        project.net_name(v.net)
                    ),
                    vec![
                        ObjectRef::Via { id: v.id },
                        ObjectRef::Instance { id: f.instance },
                    ],
                ));
            }
        }
    }
    out
}

/// Whether `points` map onto themselves under a half turn about `centre`,
/// each within [`SAME_POINT_MM`].
fn half_turn_symmetric(points: &[Point], centre: Point) -> bool {
    let turned = |p: Point| [2.0 * centre[0] - p[0], 2.0 * centre[1] - p[1]];
    points.iter().all(|p| {
        points
            .iter()
            .any(|o| distance(*o, turned(*p)) <= SAME_POINT_MM)
    })
}

/// Whether a stroke along `line` of `width` reaches into `ring`.
fn stroke_enters(line: &[Point], width: f64, ring: &[Point]) -> bool {
    !line.is_empty() && polyline_polygon_distance(line, ring) < width / 2.0 - 1e-9
}

/// [`SILK`]: one finding per part whose silkscreen enters a fiducial's
/// clear area.
fn silk_findings(project: &Project, fiducials: &[PlacedFiducial]) -> Vec<Finding> {
    if fiducials.is_empty() {
        return Vec::new();
    }
    let silk = project.rules.effective().silkscreen;
    let mut reported: BTreeSet<(Id, usize)> = BTreeSet::new();
    let mut out = Vec::new();
    for graphic in placed_graphics(project) {
        let BoardDrawing::Silkscreen(layer) = graphic.layer else {
            continue;
        };
        for (i, f) in fiducials.iter().enumerate() {
            if f.layer != layer || reported.contains(&(graphic.instance, i)) {
                continue;
            }
            let enters = f.clear.iter().any(|ring| match &graphic.item {
                PlacedItem::Shape {
                    shape,
                    width,
                    filled,
                } => {
                    let mut line = shape.flatten(CHORD_MM);
                    let closed = shape.is_closed();
                    if closed && let Some(first) = line.first().copied() {
                        line.push(first);
                    }
                    (*filled && closed && polygons_overlap(&line, ring))
                        || stroke_enters(&line, *width, ring)
                }
                PlacedItem::Text(text) => {
                    let strokes = text_strokes(text, &silk);
                    strokes
                        .strokes
                        .iter()
                        .any(|s| stroke_enters(s, strokes.width, ring))
                }
            });
            if enters {
                reported.insert((graphic.instance, i));
                let owner = designator(project, graphic.instance);
                let fid = designator(project, f.instance);
                let whose = if graphic.instance == f.instance {
                    format!("{owner}'s own silkscreen")
                } else {
                    format!("Silkscreen of {owner}")
                };
                let mut objects = vec![ObjectRef::Instance {
                    id: graphic.instance,
                }];
                if graphic.instance != f.instance {
                    objects.push(ObjectRef::Instance { id: f.instance });
                }
                out.push(Finding::new(
                    FindingCode::BoardFiducialSilk,
                    format!(
                        "{whose} lies in fiducial {fid}'s clear area on the {} side",
                        side_name(layer == Layer::Top)
                    ),
                    objects,
                ));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests;
