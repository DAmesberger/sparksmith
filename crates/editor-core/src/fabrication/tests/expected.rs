//! What each layer must show, rasterised pixel by pixel from the point oracle
//! ([`super::oracle`]): exact distances to the document's own primitives,
//! with none of the production geometry and no polygon library. Copper is
//! every pad with copper on the layer, every track, via and plated mounting
//! hole ring and every computed fill region (outer ring and holes,
//! even-odd). Mask openings and paste follow the resolution rule stated in
//! `fabrication`: an inherited mask expansion or paste reduction is the
//! effective design rule's; explicit pad rules are as authored; a via opens
//! only when untented. The profile is the outer outline less every cutout
//! and slot but the plated ones, which the plated drill file routes.
use super::Fallible;
use super::oracle::{self, Area, Prim};
use super::raster::{Bitmap, Shape};
use crate::geometry::Shape as CoreShape;
use crate::model::board::CutoutKind;
use crate::model::{CopperLayer, Layer, Project};

/// Chord tolerance of flattened curves here, mm.
pub(crate) const TOLERANCE_MM: f64 = 0.0005;

/// A bitmap over the board's bounds grown by `margin` mm, at `pixel` mm.
pub(crate) fn board_bitmap(project: &Project, margin: f64, pixel: f64) -> Fallible<Bitmap> {
    let b = project.board().bounds();
    Bitmap::new(
        [b.min[0] - margin, b.min[1] - margin],
        [b.max[0] + margin, b.max[1] + margin],
        pixel,
    )
}

/// A core shape as a raster shape: circles stay exact discs, everything
/// else is flattened within [`TOLERANCE_MM`].
pub(crate) fn raster_shape(shape: &CoreShape) -> Shape {
    match shape {
        CoreShape::Circle { center, radius } => Shape::Disc {
            c: *center,
            r: *radius,
        },
        other => Shape::Poly {
            contours: vec![other.flatten(TOLERANCE_MM)],
        },
    }
}

/// Every pixel within the material of `prims`, dark.
fn painted(prims: &[Prim], bitmap: &mut Bitmap) {
    for prim in prims {
        let [min, max] = prim.bounds();
        bitmap.paint(min, max, |p| prim.excess(p) <= 0.0);
    }
}

/// The copper of `layer`, dark on `bitmap`.
pub(crate) fn copper(project: &Project, layer: CopperLayer, bitmap: &mut Bitmap) -> Fallible<()> {
    // Fill regions are polygons of many points: filled as they are stored.
    for region in project
        .fills
        .iter()
        .filter(|f| f.layer == layer)
        .flat_map(|f| f.computed.iter().flat_map(|c| &c.regions))
    {
        let mut contours = vec![region.outer.clone()];
        contours.extend(region.holes.iter().cloned());
        bitmap.render(&Shape::Poly { contours }, true);
    }
    painted(&oracle::copper_but_fills(project, layer), bitmap);
    Ok(())
}

/// The solder-mask openings of `side`, dark on `bitmap`.
pub(crate) fn mask(project: &Project, side: Layer, bitmap: &mut Bitmap) -> Fallible<()> {
    painted(&oracle::mask(project, side), bitmap);
    Ok(())
}

/// The solder-paste openings of `side`, dark on `bitmap`.
pub(crate) fn paste(project: &Project, side: Layer, bitmap: &mut Bitmap) -> Fallible<()> {
    painted(&oracle::paste(project, side)?, bitmap);
    Ok(())
}

/// What the profile encloses: the outer outline less every cutout and
/// non-plated slot, dark.
pub(crate) fn outline(project: &Project, bitmap: &mut Bitmap) -> Fallible<()> {
    let board = project.board();
    let outer = Area::of_outline(&board.outer);
    let cutouts: Vec<Area> = board
        .cutouts
        .iter()
        .filter(|c| c.kind != (CutoutKind::Slot { plated: true }))
        .map(|c| Area::of_outline(&c.outline))
        .collect();
    let b = board.bounds();
    bitmap.paint(b.min, b.max, |p| {
        outer.inside(p) && !cutouts.iter().any(|c| c.inside(p))
    });
    Ok(())
}

/// Every drilled hole that is (or is not) plated, dark: vias, pad holes and
/// pad vias, board mounting holes and, when plated, board slots.
pub(crate) fn drills(project: &Project, plated: bool, bitmap: &mut Bitmap) -> Fallible<()> {
    for hole in oracle::holes(project)?
        .iter()
        .filter(|h| h.plated == plated)
    {
        let area = Area::capsule(hole.from, hole.to, hole.width / 2.0);
        let r = hole.width / 2.0;
        let min = [
            hole.from[0].min(hole.to[0]) - r,
            hole.from[1].min(hole.to[1]) - r,
        ];
        let max = [
            hole.from[0].max(hole.to[0]) + r,
            hole.from[1].max(hole.to[1]) + r,
        ];
        bitmap.paint(min, max, |p| area.signed(p) <= 0.0);
    }
    Ok(())
}
