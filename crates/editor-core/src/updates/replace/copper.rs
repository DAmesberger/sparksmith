//! Geometry used by replacement review, in board coordinates. Every placed
//! land and drill participates, whether or not the pad has a terminal or net.
use crate::geometry::{Point, Shape};
use crate::ids::Id;
use crate::layout;
use crate::layout::PlacedPad;
use crate::layout::physical::{HoleFeature, pad_holes};
use crate::library::{PadCopper, PadShape, Segment};
use crate::model::{Layer, Project};
use crate::updates::CopperIdentity;
use std::collections::BTreeMap;

struct PhysicalPad {
    label: String,
    land: Option<(LandLayers, Boundary)>,
    holes: Vec<(HoleFeature, bool)>,
}

#[derive(PartialEq, Eq)]
enum LandLayers {
    Side(Layer),
    Through,
}

enum Boundary {
    Circle { center: Point, radius: f64 },
    Contour(Vec<Edge>),
    Other(Shape),
}

/// A contour edge. Custom pads carry an exact arc bulge, primitive lands
/// carry a fillet radius at each end. Starting vertex and winding are free.
#[derive(Clone, Copy)]
struct Edge {
    from: Point,
    to: Point,
    radii: [f64; 2],
    bulge: f64,
}

impl Edge {
    fn reversed(self) -> Self {
        Self {
            from: self.to,
            to: self.from,
            radii: [self.radii[1], self.radii[0]],
            bulge: -self.bulge,
        }
    }

    fn same(self, other: Self) -> bool {
        self.radii == other.radii
            && self.bulge == other.bulge
            && near(self.from, other.from)
            && near(self.to, other.to)
    }
}

// Allow roundoff from the two placement transforms, below one stored nm.
fn near(a: Point, b: Point) -> bool {
    (a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9
}

impl Boundary {
    fn of(pad: &PlacedPad) -> Self {
        if let PadShape::Custom { outline } = &pad.pad.shape {
            let transform = pad.pad_transform();
            let mut from = transform.apply(outline.start);
            let edges = outline.segments.iter().map(|segment| {
                let to = transform.apply(segment.to());
                let bulge = match *segment {
                    Segment::Line { .. } => 0.0,
                    Segment::Arc { bulge, .. } if transform.mirror => -bulge,
                    Segment::Arc { bulge, .. } => bulge,
                };
                let edge = Edge {
                    from,
                    to,
                    radii: [0.0; 2],
                    bulge,
                };
                from = to;
                edge
            });
            return Self::Contour(edges.collect());
        }
        match pad.outline() {
            Shape::Circle { center, radius } => Self::Circle { center, radius },
            Shape::Polygon { points, radii } => {
                let edges = points.iter().enumerate().map(|(i, &from)| {
                    let next = (i + 1) % points.len();
                    Edge {
                        from,
                        to: points[next],
                        radii: [
                            radii.get(i).copied().unwrap_or_default(),
                            radii.get(next).copied().unwrap_or_default(),
                        ],
                        bulge: 0.0,
                    }
                });
                Self::Contour(edges.collect())
            }
            other => Self::Other(other),
        }
    }

    fn same(&self, other: &Self) -> bool {
        match (self, other) {
            (
                Self::Circle {
                    center: a,
                    radius: r,
                },
                Self::Circle {
                    center: b,
                    radius: s,
                },
            ) => r == s && near(*a, *b),
            (Self::Contour(a), Self::Contour(b)) if a.len() == b.len() => {
                let Some(first) = a.first() else { return false };
                b.iter().enumerate().any(|(i, edge)| {
                    (first.same(*edge)
                        && a.iter()
                            .zip(b.iter().cycle().skip(i))
                            .all(|(x, y)| x.same(*y)))
                        || (first.same(edge.reversed())
                            && a.iter()
                                .zip(b.iter().rev().cycle().skip(b.len() - 1 - i))
                                .all(|(x, y)| x.same(y.reversed())))
                })
            }
            (Self::Other(a), Self::Other(b)) => a == b,
            _ => false,
        }
    }
}

fn same_hole(a: HoleFeature, b: HoleFeature) -> bool {
    match (a, b) {
        (HoleFeature::Round { at: a, diameter: d }, HoleFeature::Round { at: b, diameter: e }) => {
            d == e && near(a, b)
        }
        (
            HoleFeature::Slot {
                from: a,
                to: b,
                width: w,
            },
            HoleFeature::Slot {
                from: c,
                to: d,
                width: v,
            },
        ) => w == v && ((near(a, c) && near(b, d)) || (near(a, d) && near(b, c))),
        _ => false,
    }
}

impl PhysicalPad {
    fn of(pad: &PlacedPad) -> Self {
        let land = match pad.pad.kind.copper() {
            PadCopper::None => None,
            PadCopper::Through => Some((LandLayers::Through, Boundary::of(pad))),
            PadCopper::OneSide => Some((LandLayers::Side(pad.layer), Boundary::of(pad))),
        };
        Self {
            label: if pad.pad.number.is_empty() {
                pad.pad.id.to_string()
            } else {
                pad.pad.number.clone()
            },
            land,
            holes: pad_holes(pad),
        }
    }

    fn same(&self, other: &Self) -> bool {
        let land_matches = match (&self.land, &other.land) {
            (None, None) => true,
            (Some((layer_a, a)), Some((layer_b, b))) => layer_a == layer_b && a.same(b),
            _ => false,
        };
        if !land_matches || self.holes.len() != other.holes.len() {
            return false;
        }
        // Drill order and pad-local aperture IDs have no physical meaning.
        let mut holes: Vec<_> = other.holes.iter().collect();
        self.holes.iter().all(|(a, plated)| {
            let Some(i) = holes
                .iter()
                .position(|(b, p)| p == plated && same_hole(*a, *b))
            else {
                return false;
            };
            holes.swap_remove(i);
            true
        })
    }
}

fn inventory(project: &Project, ids: &[Id]) -> BTreeMap<Id, Vec<PhysicalPad>> {
    let mut pads: BTreeMap<_, Vec<_>> = ids.iter().map(|&id| (id, Vec::new())).collect();
    for pad in layout::placed_pads(project) {
        if let Some(parts) = pads.get_mut(&pad.instance) {
            parts.push(PhysicalPad::of(&pad));
        }
    }
    pads
}

pub(in crate::updates) fn copper_identity(
    project: &Project,
    next: &Project,
    ids: &[Id],
) -> Vec<CopperIdentity> {
    let before = inventory(project, ids);
    let mut after = inventory(next, ids);
    ids.iter()
        .filter_map(|id| {
            let designator = next.instance(*id)?.designator.clone();
            let old = before.get(id)?;
            let mut new = after.remove(id)?;
            let (mut old_only, mut renumbered) = (Vec::new(), Vec::new());
            for pad in old {
                if let Some(i) = new.iter().position(|candidate| pad.same(candidate)) {
                    let matched = new.remove(i);
                    if pad.label != matched.label {
                        renumbered.push(format!("{} → {}", pad.label, matched.label));
                    }
                } else {
                    old_only.push(pad.label.clone());
                }
            }
            Some(CopperIdentity {
                instance: *id,
                designator,
                identical: old_only.is_empty() && new.is_empty(),
                old_only,
                new_only: new.into_iter().map(|p| p.label).collect(),
                renumbered,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Result;
    use crate::geometry::Transform2;
    use crate::ids::{PackageTerminalId, PadId};
    use crate::library::{ApertureId, Pad, PadVia};
    use crate::tests::support::{nm, pt};

    #[test]
    fn pad_owned_vias_compare_by_physical_hole_in_any_order() -> Result<()> {
        let mut pad = Pad::smd_rect_mm(PadId(1), PackageTerminalId(1), "EP", [0.0; 2], [3.0; 2])?;
        pad.vias = [[0.5, 0.75], [-0.5, 0.25]]
            .into_iter()
            .zip(1..)
            .map(|(at, id)| PadVia {
                id: ApertureId(id),
                at: pt(at),
                drill_nm: nm(0.3),
            })
            .collect();
        let mut placed = PlacedPad {
            instance: 1,
            pad,
            transform: Transform2 {
                translate: [5.0, 6.0],
                rotation_deg: 90.0,
                mirror: true,
            },
            layer: Layer::Bottom,
            terminal: None,
            net: None,
            fill_override: Default::default(),
        };
        let old = PhysicalPad::of(&placed);
        placed.pad.vias.reverse();
        assert!(old.same(&PhysicalPad::of(&placed)));
        // Turn the square and both via positions about its centre. The
        // square still covers the same copper, but both bores move.
        placed.pad.rotation_mdeg = crate::units::Angle::normalized(90_000);
        assert!(!old.same(&PhysicalPad::of(&placed)));
        Ok(())
    }
}
