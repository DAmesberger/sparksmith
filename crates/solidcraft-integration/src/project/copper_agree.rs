//! How far the board plan's copper on each layer lies from the core's:
//! the copper the copper graph joins, the checks measure and the
//! fabrication output plots (every pad on each layer
//! [`sparksmith_core::layout::pad_copper_on`] gives it, by its exact
//! outline; each track's stadium, [`sparksmith_core::layout::track_shape`];
//! each via's pad on the layers it joins; each fill's computed copper).
//!
//! Layout and the 3D view both show the plan's copper
//! ([`super::plan::BoardPlan::layer_copper`]), so this is the one
//! measure of whether what they show is the document's copper. On each
//! layer, pad, track and via copper and fill copper are compared apart,
//! each by the area of the two unions' symmetric difference, against what
//! flattening allows: the plan's track caps are chords, its fills are
//! simplified to [`super::fill::MODEL_CHORD_MM`] and curved outlines on
//! both sides are flattened.
//!
//! A pad's land is compared by its outline: a drilled pad's bore is a hole
//! in the plan's land and none in the core's outline (the copper graph
//! joins copper by the outline).
use super::fill::MODEL_CHORD_MM;
use super::plan::BoardPlan;
use crate::material::BodyKind;
use crate::pad_outline::FLATTEN_TOLERANCE_MM;
use crate::{Error, Result};
use i_overlay::core::fill_rule::FillRule;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::float::single::SingleFloatOverlay;
use sparksmith_core::geometry::signed_area2;
use sparksmith_core::layout;
use sparksmith_core::library::PAD_ARC_TOLERANCE_MM;
use sparksmith_core::{CopperLayer, Point, Project};

/// One layer's agreement.
#[derive(Clone, Debug, PartialEq)]
pub struct LayerAgreement {
    pub layer: CopperLayer,
    /// The plan's copper on the layer, mm².
    pub plan_mm2: f64,
    /// The core's copper on the layer, mm².
    pub core_mm2: f64,
    /// The area where exactly one of them has pad, track or via copper,
    /// mm², and what flattening allows it to be.
    pub objects_differ_mm2: f64,
    pub objects_allowed_mm2: f64,
    /// The same for fill copper (the plan's tiles, the computed regions).
    pub fills_differ_mm2: f64,
    pub fills_allowed_mm2: f64,
}

impl LayerAgreement {
    pub fn agrees(&self) -> bool {
        self.objects_differ_mm2 <= self.objects_allowed_mm2
            && self.fills_differ_mm2 <= self.fills_allowed_mm2
    }

    /// Where exactly one of them has copper of a kind, mm².
    pub fn differ_mm2(&self) -> f64 {
        self.objects_differ_mm2 + self.fills_differ_mm2
    }
}

/// A shape: its outer loop counter-clockwise, then its holes clockwise.
type Shape = Vec<Vec<Point>>;

fn oriented(mut ring: Vec<Point>, ccw: bool) -> Vec<Point> {
    if (signed_area2(&ring) > 0.0) != ccw {
        ring.reverse();
    }
    ring
}

fn perimeter(ring: &[Point]) -> f64 {
    let n = ring.len();
    (0..n)
        .map(|i| sparksmith_core::geometry::distance(ring[i], ring[(i + 1) % n]))
        .sum()
}

/// The area of shapes as an overlay gives them (outer loops
/// counter-clockwise, holes clockwise).
fn area(shapes: &[Shape]) -> f64 {
    shapes.iter().flatten().map(|r| signed_area2(r) / 2.0).sum()
}

fn union(shapes: Vec<Shape>) -> Vec<Shape> {
    if shapes.is_empty() {
        return Vec::new();
    }
    shapes.overlay(
        &Vec::<Shape>::new(),
        OverlayRule::Subject,
        FillRule::NonZero,
    )
}

/// Both unions' areas and the area where exactly one has copper, mm².
fn compare(a: Vec<Shape>, b: Vec<Shape>) -> (f64, f64, f64) {
    let (a, b) = (union(a), union(b));
    let xor: Vec<Shape> = if a.is_empty() && b.is_empty() {
        Vec::new()
    } else {
        a.overlay(&b, OverlayRule::Xor, FillRule::NonZero)
    };
    (area(&a), area(&b), area(&xor).abs())
}

/// One side's copper on a layer: pads, tracks and vias; fills.
#[derive(Default)]
struct Copper {
    objects: Vec<Shape>,
    fills: Vec<Shape>,
}

/// Each copper layer's agreement between `plan` (the plan of `project`'s
/// board, its copper at least) and the core's copper of `project`.
pub fn agreement(project: &Project, plan: &BoardPlan) -> Result<Vec<LayerAgreement>> {
    let layers = project.stackup.layers();
    let pads = layout::placed_pads(project);
    let mut out = Vec::with_capacity(layers.len());
    for layer in layers.iter().copied() {
        let mut allowed = 1e-9;
        let mut fill_allowed = 1e-9;
        // The plan's copper: each body's outer loop (a fill tile with its
        // holes, which are clearances, not drills).
        let mut planned = Copper::default();
        for body in plan.layer_copper(layer) {
            let fill = body.kind == BodyKind::FillCopper;
            let loops = body.board_loops();
            let keep = if fill { loops.len() } else { 1 };
            let mut shape = Shape::new();
            for (i, contour) in loops.iter().take(keep).enumerate() {
                let ring = contour
                    .flatten(FLATTEN_TOLERANCE_MM)
                    .map_err(|e| Error::Verification(e.to_string()))?;
                if ring.len() >= 3 {
                    shape.push(oriented(ring, i == 0));
                }
            }
            match (shape.is_empty(), fill) {
                (true, _) => {}
                (false, true) => planned.fills.push(shape),
                (false, false) => planned.objects.push(shape),
            }
        }
        // The core's copper.
        let mut core = Copper::default();
        for pad in pads.iter().filter(|p| p.on_copper(layer)) {
            let ring = pad.outline().flatten(PAD_ARC_TOLERANCE_MM);
            if ring.len() >= 3 {
                allowed += perimeter(&ring) * (PAD_ARC_TOLERANCE_MM + FLATTEN_TOLERANCE_MM);
                core.objects.push(vec![oriented(ring, true)]);
            }
        }
        for track in project.tracks.iter().filter(|t| t.layer == layer) {
            let ring = layout::track_shape(track).flatten(PAD_ARC_TOLERANCE_MM);
            if ring.len() >= 3 {
                // The plan's caps are chords, eight to a half circle: each
                // cap's sagitta along its arc.
                let r = track.width / 2.0;
                let sagitta = r * (1.0 - (std::f64::consts::PI / 16.0).cos());
                allowed += std::f64::consts::TAU * r * (sagitta + PAD_ARC_TOLERANCE_MM);
                core.objects.push(vec![oriented(ring, true)]);
            }
        }
        for via in project.vias.iter().filter(|v| v.on(layer, &layers)) {
            let ring = sparksmith_core::geometry::Shape::Circle {
                center: via.position,
                radius: via.diameter / 2.0,
            }
            .flatten(PAD_ARC_TOLERANCE_MM);
            if ring.len() >= 3 {
                allowed += perimeter(&ring) * (PAD_ARC_TOLERANCE_MM + FLATTEN_TOLERANCE_MM);
                core.objects.push(vec![oriented(ring, true)]);
            }
        }
        for fill in project.fills.iter().filter(|f| f.layer == layer) {
            for region in fill.computed.iter().flat_map(|c| &c.regions) {
                if region.outer.len() < 3 {
                    continue;
                }
                let mut shape = vec![oriented(region.outer.clone(), true)];
                fill_allowed += perimeter(&region.outer) * MODEL_CHORD_MM;
                for hole in region.holes.iter().filter(|h| h.len() >= 3) {
                    fill_allowed += perimeter(hole) * MODEL_CHORD_MM;
                    shape.push(oriented(hole.clone(), false));
                }
                core.fills.push(shape);
            }
        }
        let all = |c: &Copper| c.objects.iter().chain(&c.fills).cloned().collect();
        let (plan_mm2, core_mm2, _) = compare(all(&planned), all(&core));
        let (_, _, objects_differ_mm2) = compare(planned.objects, core.objects);
        let (_, _, fills_differ_mm2) = compare(planned.fills, core.fills);
        out.push(LayerAgreement {
            layer,
            plan_mm2,
            core_mm2,
            objects_differ_mm2,
            objects_allowed_mm2: allowed,
            fills_differ_mm2,
            fills_allowed_mm2: fill_allowed,
        });
    }
    Ok(out)
}
