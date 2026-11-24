//! A layer's image against the design's material, as exact polygons: what
//! is dark in one only, shrunk by half [`TOLERANCE_NM`], must be nothing.
//! The design's polygons are the oracle's edges flattened here; the stored
//! fill regions are taken as they are. The boolean operations are
//! `i_overlay`'s, which the fill computation and the writer's union of pours
//! use too.
use super::Fallible;
use super::oracle::{Area, Edge, Frame, Ink, Prim, Wrong};
use crate::geometry::Point;
use crate::model::fill::FilledRegion;
use i_overlay::core::fill_rule::FillRule;
use i_overlay::core::overlay_rule::OverlayRule;
use sparksmith_fabcheck::polygons::{self, Ring, Shapes};

/// A difference narrower than this is the format's and the chords', nm.
pub(super) const TOLERANCE_NM: i32 = 5_000;
/// Chord tolerance of the design's curves, mm.
const CHORD_MM: f64 = 0.000_25;

fn ring(points: impl Iterator<Item = Point>) -> Fallible<Ring> {
    points.map(polygons::nm).collect()
}

/// The inside of one closed loop of edges in a frame.
fn area(edges: &[Edge], frame: Frame) -> Fallible<Shapes> {
    let points = edges
        .iter()
        .flat_map(|e| e.flattened(CHORD_MM))
        .map(|p| frame.to_board(p));
    Ok(polygons::normalise(&[ring(points)?], FillRule::EvenOdd))
}

/// Material that is not grown.
pub(super) fn solid(prim: &Prim) -> Fallible<Shapes> {
    area(&prim.area.edges, prim.frame)
}

/// A stored fill region: its outer ring less its holes.
pub(super) fn region(region: &FilledRegion) -> Fallible<Shapes> {
    let rings = std::iter::once(&region.outer)
        .chain(&region.holes)
        .map(|r| ring(r.iter().copied()))
        .collect::<Fallible<Vec<_>>>()?;
    Ok(polygons::normalise(&rings, FillRule::EvenOdd))
}

/// What a piece of silkscreen prints.
pub(super) fn ink(ink: &Ink) -> Fallible<Shapes> {
    let mut out = Shapes::new();
    if ink.filled {
        out.extend(area(&ink.edges, ink.frame)?);
    }
    if ink.half <= 0.0 {
        return Ok(out);
    }
    for edge in &ink.edges {
        let mut points = edge.flattened(CHORD_MM);
        points.push(edge.ends().1);
        for w in points.windows(2) {
            let pen = Area::capsule(w[0], w[1], ink.half);
            out.extend(area(&pen.edges, ink.frame)?);
        }
    }
    Ok(out)
}

/// The pieces dark in `file` or in `design` only, at most `most` of each.
pub(super) fn compare(
    file: &Shapes,
    design: &Shapes,
    what: impl Fn(Point) -> String,
    most: usize,
) -> Fallible<Vec<Wrong>> {
    let mut out = Vec::new();
    for (a, b, missing) in [(design, file, true), (file, design, false)] {
        let d = polygons::boolean(a, b, OverlayRule::Difference);
        let wide = polygons::offset(&d, -(TOLERANCE_NM / 2))?;
        for shape in wide
            .iter()
            .filter(|s| s.iter().map(|r| polygons::area2(r)).sum::<i128>() > 0)
            .take(most)
        {
            let Some(p) = shape.first().and_then(|r| r.first()) else {
                continue;
            };
            let at = [f64::from(p.x) / 1e6, f64::from(p.y) / 1e6];
            out.push(Wrong {
                at,
                missing,
                what: what(at),
            });
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A disc against itself moved: 4 µm is within the tolerance, 8 µm is
    /// told on both sides.
    #[test]
    fn a_moved_disc_is_told_past_the_tolerance() -> Fallible<()> {
        let disc = |x: f64| {
            let area = Area::disc([0.0; 2], 0.3);
            solid(&Prim::new(area, Frame::still([x, 0.0]), 0.0, "a disc"))
        };
        let what = |_: Point| "a disc".to_owned();
        assert!(compare(&disc(5.0)?, &disc(5.0)?, what, 9)?.is_empty());
        assert!(compare(&disc(5.004)?, &disc(5.0)?, what, 9)?.is_empty());
        let moved = compare(&disc(5.008)?, &disc(5.0)?, what, 9)?;
        assert!(moved.iter().any(|w| w.missing) && moved.iter().any(|w| !w.missing));
        Ok(())
    }
}
