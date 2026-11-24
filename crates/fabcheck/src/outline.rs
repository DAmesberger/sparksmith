//! A board profile read back: its drawn lines and arcs chained into closed
//! contours, and two profiles compared both ways. Each contour of one must
//! be a contour of the other and the reverse, so an extra cutout, a missing
//! one and a missing file are all differences.
use crate::gerber::{GerberFile, Kind};
use crate::{Fallible, Point};

/// Most points sampled on one line or arc.
const MAX_SAMPLES: f64 = 4096.0;
/// Sample spacing along a contour, mm.
const SAMPLE_MM: f64 = 0.05;
/// Ends this near meet, mm (two steps of the 1 nm format, rounded apart).
const JOIN_MM: f64 = 2e-6;

/// One drawn piece, from `from` to `to`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Edge {
    Line {
        from: Point,
        to: Point,
    },
    /// Signed sweep about `center`, radians (positive counter-clockwise).
    Arc {
        from: Point,
        to: Point,
        center: Point,
        sweep: f64,
    },
}

/// A closed contour.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Contour {
    pub edges: Vec<Edge>,
}

fn distance(a: Point, b: Point) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

impl Edge {
    /// The edge of a drawn arc; one that ends where it starts is a circle.
    fn arc(from: Point, to: Point, center: Point, clockwise: bool) -> Self {
        let at = |p: Point| (p[1] - center[1]).atan2(p[0] - center[0]);
        let tau = std::f64::consts::TAU;
        let turn = if clockwise {
            at(from) - at(to)
        } else {
            at(to) - at(from)
        }
        .rem_euclid(tau);
        let turn = if distance(from, to) < 1e-9 { tau } else { turn };
        Self::Arc {
            from,
            to,
            center,
            sweep: if clockwise { -turn } else { turn },
        }
    }

    pub fn from(&self) -> Point {
        match *self {
            Self::Line { from, .. } | Self::Arc { from, .. } => from,
        }
    }

    pub fn to(&self) -> Point {
        match *self {
            Self::Line { to, .. } | Self::Arc { to, .. } => to,
        }
    }

    pub fn length(&self) -> f64 {
        match *self {
            Self::Line { from, to } => distance(from, to),
            Self::Arc {
                from,
                center,
                sweep,
                ..
            } => distance(from, center) * sweep.abs(),
        }
    }

    /// The point a share `t` of the way along.
    pub fn at(&self, t: f64) -> Point {
        match *self {
            Self::Line { from, to } => [
                from[0] + (to[0] - from[0]) * t,
                from[1] + (to[1] - from[1]) * t,
            ],
            Self::Arc {
                from,
                center,
                sweep,
                ..
            } => {
                let r = distance(from, center);
                let a = (from[1] - center[1]).atan2(from[0] - center[0]) + sweep * t;
                [center[0] + r * a.cos(), center[1] + r * a.sin()]
            }
        }
    }

    /// The distance of `p` from the edge, exact for lines and arcs.
    pub fn distance(&self, p: Point) -> f64 {
        match *self {
            Self::Line { from, to } => {
                let d = [to[0] - from[0], to[1] - from[1]];
                let l2 = d[0] * d[0] + d[1] * d[1];
                let t = if l2 > 0.0 {
                    (((p[0] - from[0]) * d[0] + (p[1] - from[1]) * d[1]) / l2).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                distance(p, [from[0] + d[0] * t, from[1] + d[1] * t])
            }
            Self::Arc {
                from,
                to,
                center,
                sweep,
            } => {
                let a0 = (from[1] - center[1]).atan2(from[0] - center[0]);
                let a = (p[1] - center[1]).atan2(p[0] - center[0]);
                let tau = std::f64::consts::TAU;
                let turn = (if sweep < 0.0 { a0 - a } else { a - a0 }).rem_euclid(tau);
                if turn <= sweep.abs() {
                    (distance(p, center) - distance(from, center)).abs()
                } else {
                    distance(p, from).min(distance(p, to))
                }
            }
        }
    }
}

impl Contour {
    pub fn length(&self) -> f64 {
        self.edges.iter().map(Edge::length).sum()
    }

    /// The distance of `p` from the contour's line.
    pub fn distance(&self, p: Point) -> f64 {
        self.edges
            .iter()
            .map(|e| e.distance(p))
            .fold(f64::INFINITY, f64::min)
    }

    /// The farthest a sampled point of `self` lies from `other`.
    fn farthest_from(&self, other: &Self) -> f64 {
        self.edges
            .iter()
            .flat_map(|e| {
                let n = (e.length() / SAMPLE_MM).ceil().clamp(1.0, MAX_SAMPLES) as usize;
                (0..=n).map(move |k| e.at(k as f64 / n as f64))
            })
            .map(|p| other.distance(p))
            .fold(0.0, f64::max)
    }

    /// How far apart two contours lie, mm, at worst, both ways.
    pub fn apart(&self, other: &Self) -> f64 {
        self.farthest_from(other).max(other.farthest_from(self))
    }

    /// Moved by `by` mm.
    pub fn translated(&self, by: Point) -> Self {
        let m = |p: Point| [p[0] + by[0], p[1] + by[1]];
        Self {
            edges: self
                .edges
                .iter()
                .map(|e| match *e {
                    Edge::Line { from, to } => Edge::Line {
                        from: m(from),
                        to: m(to),
                    },
                    Edge::Arc {
                        from,
                        to,
                        center,
                        sweep,
                    } => Edge::Arc {
                        from: m(from),
                        to: m(to),
                        center: m(center),
                        sweep,
                    },
                })
                .collect(),
        }
    }
}

/// The closed contours a profile layer draws, whatever order and direction
/// its pieces are drawn in. Anything but a dark draw, and a piece that
/// closes no contour, is an error.
pub fn contours(file: &GerberFile) -> Fallible<Vec<Contour>> {
    let mut open: Vec<Edge> = Vec::new();
    for object in &file.objects {
        if !object.dark {
            return Err("outline: the profile clears".into());
        }
        open.push(match &object.kind {
            Kind::Draw { from, to } if distance(*from, *to) < 1e-9 => continue,
            Kind::Draw { from, to } => Edge::Line {
                from: *from,
                to: *to,
            },
            Kind::Arc {
                from,
                to,
                center,
                clockwise,
            } => Edge::arc(*from, *to, *center, *clockwise),
            _ => return Err("outline: the profile holds only draws".into()),
        });
    }
    let mut out = Vec::new();
    while !open.is_empty() {
        let first = open.remove(0);
        let mut contour = Contour { edges: vec![first] };
        let mut end = first.to();
        while distance(end, first.from()) > JOIN_MM {
            let Some(k) = open
                .iter()
                .position(|e| distance(e.from(), end).min(distance(e.to(), end)) <= JOIN_MM)
            else {
                return Err(format!("outline: an open contour ends at {end:?}").into());
            };
            let next = open.remove(k);
            let next = if distance(next.from(), end) <= JOIN_MM {
                next
            } else {
                reversed(next)
            };
            end = next.to();
            contour.edges.push(next);
        }
        out.push(contour);
    }
    Ok(out)
}

fn reversed(edge: Edge) -> Edge {
    match edge {
        Edge::Line { from, to } => Edge::Line { from: to, to: from },
        Edge::Arc {
            from,
            to,
            center,
            sweep,
        } => Edge::Arc {
            from: to,
            to: from,
            center,
            sweep: -sweep,
        },
    }
}

/// Two profiles compared, contour by contour.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OutlineDiff {
    pub matched: usize,
    /// The farthest a matched pair lies apart, mm.
    pub worst_mm: f64,
    /// Contours of A, then of B, with no partner within the tolerance.
    pub only_a: Vec<Contour>,
    pub only_b: Vec<Contour>,
}

impl OutlineDiff {
    /// Every contour of each is one of the other, and there is an outline.
    pub fn passes(&self) -> bool {
        self.matched > 0 && self.only_a.is_empty() && self.only_b.is_empty()
    }
}

/// Pair the contours of `a` with those of `b` lying within `tolerance_mm`
/// of each other everywhere, one to one.
pub fn compare(a: &[Contour], b: &[Contour], tolerance_mm: f64) -> OutlineDiff {
    let mut left: Vec<&Contour> = b.iter().collect();
    let mut diff = OutlineDiff::default();
    for contour in a {
        let best = left
            .iter()
            .enumerate()
            .map(|(k, other)| (k, contour.apart(other)))
            .min_by(|x, y| x.1.total_cmp(&y.1))
            .filter(|(_, d)| *d <= tolerance_mm);
        match best {
            Some((k, d)) => {
                left.remove(k);
                diff.matched += 1;
                diff.worst_mm = diff.worst_mm.max(d);
            }
            None => diff.only_a.push(contour.clone()),
        }
    }
    diff.only_b = left.into_iter().cloned().collect();
    diff
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(at: Point, side: f64) -> Contour {
        let p = [
            at,
            [at[0] + side, at[1]],
            [at[0] + side, at[1] + side],
            [at[0], at[1] + side],
        ];
        Contour {
            edges: (0..4)
                .map(|k| Edge::Line {
                    from: p[k],
                    to: p[(k + 1) % 4],
                })
                .collect(),
        }
    }

    #[test]
    fn an_extra_a_missing_and_a_moved_contour_are_differences() {
        let board = square([0.0, 0.0], 10.0);
        let cutout = square([4.0, 4.0], 1.0);
        let both = [board.clone(), cutout.clone()];
        assert!(compare(&both, &both, 0.001).passes());
        let one = [board.clone()];
        assert!(!compare(&both, &one, 0.001).passes(), "extra cutout");
        assert!(!compare(&one, &both, 0.001).passes(), "missing cutout");
        assert!(!compare(&one, &[], 0.001).passes(), "missing reference");
        assert!(!compare(&[], &[], 0.001).passes(), "nothing compared");
        let moved = [board, square([4.0, 4.002], 1.0)];
        assert!(!compare(&both, &moved, 0.001).passes(), "moved");
        // A cutout drawn twice is not one cutout.
        let twice = [both[0].clone(), cutout.clone(), cutout];
        assert!(!compare(&twice, &both, 0.001).passes(), "drawn twice");
    }
}
