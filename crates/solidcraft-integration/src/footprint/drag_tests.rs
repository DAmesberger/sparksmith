//! A drag lands where its preview shows it: every point, radius and curve
//! of the sketch after the release is where the last frame had it, within
//! a nanometre. For the board's sketch and a footprint's, which share the
//! edit, and for points the constraints let go everywhere, along a curve
//! or nowhere.
use super::view::{LiveSketch, SketchView, ViewGeometry};
use super::{SketchEvaluator, check, evaluate, params};
use crate::board::SketchBoardEvaluator;
use sparksmith_core::diagnostics::{Error, Result};
use sparksmith_core::geometry::Point;
use sparksmith_core::layout::outline::BoardShapeEvaluator;
use sparksmith_core::library::footprint_source::{
    DimKind, DimValue, Expr, FootprintEvaluator, FootprintParam, FootprintSource, GeomConstraint,
    ParamName, ParamUnit, SketchBlob, SketchCurveId, SketchEdit, SketchPointId, SketchRef,
};

/// A nanometre, in mm.
const NM: f64 = 1e-6;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Of {
    Board,
    Footprint,
}
const BOTH: [Of; 2] = [Of::Board, Of::Footprint];

/// A sketch edited and settled as the engine does: the edit, then the
/// evaluation's solve, whose result is stored.
#[derive(Clone)]
struct Drawing {
    of: Of,
    source: FootprintSource,
}

impl Drawing {
    fn new(of: Of) -> Result<Self> {
        Ok(Self {
            of,
            source: FootprintSource {
                sketch: SketchEvaluator.new_sketch()?,
                ..Default::default()
            },
        })
    }

    fn apply(&mut self, edit: &SketchEdit) -> Result<Vec<SketchRef>> {
        let sketch = &self.source.sketch;
        let out = match self.of {
            Of::Board => SketchBoardEvaluator.apply_sketch(sketch, edit)?,
            Of::Footprint => SketchEvaluator.apply_sketch(sketch, edit)?,
        };
        self.source.sketch = out.sketch;
        Ok(out.created)
    }

    fn settle(&mut self) -> Result<()> {
        let stored = check::check_board_sketch(&self.source.sketch.0)?;
        let values = params::evaluate(&self.source.parameters, &[])?;
        let (solved, _) = evaluate::solve(&stored, &values)?;
        self.source.sketch = SketchBlob(check::canonical(&solved)?);
        Ok(())
    }

    fn edit(&mut self, edit: SketchEdit) -> Result<Vec<SketchRef>> {
        let made = self.apply(&edit)?;
        self.settle()?;
        Ok(made)
    }

    /// A line and its ends.
    fn line(&mut self, a: Point, b: Point) -> Result<(SketchCurveId, [SketchPointId; 2])> {
        let made = self.edit(SketchEdit::AddLine {
            a,
            b,
            construction: false,
        })?;
        let (c, p) = parts(made)?;
        match p.as_slice() {
            [a, b] => Ok((c, [a.clone(), b.clone()])),
            _ => Err(Error::Invalid("a line has two ends".into())),
        }
    }

    fn arc(&mut self, center: Point, start: Point, end: Point) -> Result<[SketchPointId; 3]> {
        let made = self.edit(SketchEdit::AddArc {
            center,
            start,
            end,
            construction: false,
        })?;
        match parts(made)?.1.as_slice() {
            [c, s, e] => Ok([c.clone(), s.clone(), e.clone()]),
            p => Err(Error::Invalid(format!("an arc has three points: {p:?}"))),
        }
    }

    fn circle(&mut self, center: Point, radius: f64) -> Result<(SketchCurveId, SketchPointId)> {
        let made = self.edit(SketchEdit::AddCircle {
            center,
            radius,
            construction: false,
        })?;
        let (c, p) = parts(made)?;
        let center = p.first().cloned();
        Ok((
            c,
            center.ok_or_else(|| Error::Invalid("a circle has a centre".into()))?,
        ))
    }

    fn constrain(&mut self, kind: GeomConstraint, refs: Vec<SketchRef>) -> Result<()> {
        self.edit(SketchEdit::AddConstraint { kind, refs })
            .map(drop)
    }

    fn dimension(&mut self, kind: DimKind, refs: Vec<SketchRef>, value: DimValue) -> Result<()> {
        self.edit(SketchEdit::AddDimension { kind, refs, value })
            .map(drop)
    }

    fn parameter(&mut self, name: &str, expr: &str) {
        self.source.parameters.push(FootprintParam {
            name: ParamName(name.into()),
            expr: Expr(expr.into()),
            unit: ParamUnit::Mm,
            comment: String::new(),
        });
    }

    /// A rectangle of four lines whose ends are joined by constraints, as
    /// Board shape draws a cutout: its lines, and their ends in order.
    fn joined_rect(
        &mut self,
        low: Point,
        high: Point,
    ) -> Result<(Vec<SketchCurveId>, Vec<[SketchPointId; 2]>)> {
        let at = [low, [high[0], low[1]], high, [low[0], high[1]]];
        let mut lines = Vec::new();
        let mut ends: Vec<[SketchPointId; 2]> = Vec::new();
        for k in 0..4 {
            let (l, e) = self.line(at[k], at[(k + 1) % 4])?;
            lines.push(l);
            ends.push(e);
        }
        for k in 0..4 {
            self.constrain(
                GeomConstraint::Coincident,
                vec![p(&ends[k][1]), p(&ends[(k + 1) % 4][0])],
            )?;
        }
        for (k, l) in lines.iter().enumerate() {
            let kind = if k % 2 == 0 {
                GeomConstraint::Horizontal
            } else {
                GeomConstraint::Vertical
            };
            self.constrain(kind, vec![c(l)])?;
        }
        Ok((lines, ends))
    }

    fn view(&self) -> Result<SketchView> {
        Ok(LiveSketch::load(&self.source, &[])?.view())
    }

    fn at(&self, point: &SketchPointId) -> Result<Point> {
        at(&self.view()?, point)
    }

    /// Drag `point` over the pointer's `path` and release at its last
    /// place: what the last frame showed and what the release stores are
    /// the same, also for someone who sends only the release. Returns where
    /// the point landed.
    fn drag(&mut self, what: &str, point: &SketchPointId, path: &[Point]) -> Result<Point> {
        let what = format!("{what} ({:?})", self.of);
        let live = LiveSketch::load(&self.source, &[])?;
        let mut shown = None;
        for to in path {
            shown = Some(live.drag(point, *to)?);
        }
        let shown = shown.ok_or_else(|| Error::Invalid("no path".into()))?;
        let to = *path.last().unwrap_or(&shown.at);
        self.edit(SketchEdit::Drag {
            point: point.clone(),
            to,
        })?;
        same(&what, &shown.sketch.view(), &self.view()?);
        let landed = self.at(point)?;
        assert!(far(landed, shown.at) <= NM, "{what}: {landed:?} {shown:?}");
        Ok(landed)
    }
}

fn parts(made: Vec<SketchRef>) -> Result<(SketchCurveId, Vec<SketchPointId>)> {
    let mut curve = None;
    let mut points = Vec::new();
    for r in made {
        match r {
            SketchRef::Curve(c) => curve = Some(c),
            SketchRef::Point(p) => points.push(p),
            SketchRef::Constraint(_) => {}
        }
    }
    Ok((
        curve.ok_or_else(|| Error::Invalid("no curve made".into()))?,
        points,
    ))
}

fn p(id: &SketchPointId) -> SketchRef {
    SketchRef::Point(id.clone())
}
fn c(id: &SketchCurveId) -> SketchRef {
    SketchRef::Curve(id.clone())
}
fn origin() -> SketchRef {
    SketchRef::Point(SketchPointId("origin".into()))
}

fn at(view: &SketchView, point: &SketchPointId) -> Result<Point> {
    view.point(point)
        .map(|q| q.at)
        .ok_or_else(|| Error::NotFound(point.0.clone()))
}

fn far(a: Point, b: Point) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

fn same(what: &str, shown: &SketchView, landed: &SketchView) {
    assert_eq!(shown.points.len(), landed.points.len(), "{what}");
    for q in &shown.points {
        let l = landed.point(&q.id).map_or([f64::NAN; 2], |l| l.at);
        assert!(
            far(q.at, l) <= NM,
            "{what}: {:?} was shown at {:?} and landed at {l:?}",
            q.id,
            q.at
        );
    }
    for k in &shown.curves {
        if let (
            ViewGeometry::Circle { radius: a, .. },
            Some(ViewGeometry::Circle { radius: b, .. }),
        ) = (&k.geometry, landed.curve(&k.id).map(|l| &l.geometry))
        {
            assert!((a - b).abs() <= NM, "{what}: radius {a} and {b}");
        }
    }
}

/// The steps of a pointer from `a` to `b`.
fn path(a: Point, b: Point, steps: u32) -> Vec<Point> {
    (1..=steps)
        .map(|i| {
            let f = f64::from(i) / f64::from(steps);
            [a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f]
        })
        .collect()
}

/// The finding: a corner of a rectangle whose ends are joined went a
/// quarter of the way. It lands at the pointer, whatever the steps, and the
/// opposite corner stays.
#[test]
fn a_joined_rectangles_corner_lands_at_the_pointer() -> Result<()> {
    for of in BOTH {
        for steps in [1, 4, 30] {
            for to in [[14.9, 8.3], [12.0, 5.5], [30.0, 22.0]] {
                let mut d = Drawing::new(of)?;
                let (_, ends) = d.joined_rect([5.0, 5.0], [10.0, 8.0])?;
                let corner = ends[2][0].clone();
                let landed = d.drag("a corner", &corner, &path([10.0, 8.0], to, steps))?;
                assert!(far(landed, to) <= NM, "{of:?} {steps}: {landed:?}");
                // Its joined end is with it and the corner across stays.
                assert!(far(d.at(&ends[1][1])?, to) <= NM);
                assert!(far(d.at(&ends[0][0])?, [5.0, 5.0]) <= NM);
            }
        }
    }
    Ok(())
}

/// A rectangle drawn as four shared corners (the rectangle edit; the
/// board's outline), free and with some of its dimensions.
#[test]
fn a_shared_corner_lands_at_the_pointer() -> Result<()> {
    for of in BOTH {
        let mut d = Drawing::new(of)?;
        let made = d.edit(SketchEdit::AddRect {
            a: [0.0, 0.0],
            b: [20.0, 10.0],
            construction: false,
        })?;
        let points: Vec<SketchPointId> = made
            .iter()
            .filter_map(|r| match r {
                SketchRef::Point(p) => Some(p.clone()),
                _ => None,
            })
            .collect();
        let lines: Vec<SketchCurveId> = made
            .iter()
            .filter_map(|r| match r {
                SketchRef::Curve(c) => Some(c.clone()),
                _ => None,
            })
            .collect();
        let landed = d.drag("an outline corner", &points[2], &[[23.0, 12.5]])?;
        assert!(far(landed, [23.0, 12.5]) <= NM, "{landed:?}");
        // Its width set: the corner takes the side along.
        d.dimension(DimKind::Distance, vec![c(&lines[0])], DimValue::Fixed(23.0))?;
        let landed = d.drag("a corner of a set width", &points[2], &[[25.0, 14.0]])?;
        assert!(far(landed, [25.0, 14.0]) <= NM, "{landed:?}");
        // Held at the origin and at its width, it only goes up and down.
        d.constrain(GeomConstraint::Coincident, vec![origin(), p(&points[0])])?;
        let before = d.at(&points[2])?;
        let landed = d.drag(
            "a corner that only slides",
            &points[2],
            &path(before, [before[0] + 4.0, before[1] + 3.0], 5),
        )?;
        assert!(
            (landed[0] - before[0]).abs() <= NM && (landed[1] - before[1] - 3.0).abs() <= NM,
            "{before:?} {landed:?}"
        );
        // Wholly set, it stays.
        d.dimension(
            DimKind::Distance,
            vec![c(&lines[1])],
            DimValue::Fixed(before[1] + 3.0),
        )?;
        let before = d.at(&points[2])?;
        let landed = d.drag("a set corner", &points[2], &[[40.0, 40.0]])?;
        assert!(far(before, landed) <= NM, "{before:?} {landed:?}");
    }
    Ok(())
}

/// Driven by a parameter: the dimension holds and the rest follows.
#[test]
fn a_corner_held_by_a_parameter_lands_where_it_is_shown() -> Result<()> {
    for of in BOTH {
        let mut d = Drawing::new(of)?;
        d.parameter("w", "6 mm");
        let (lines, ends) = d.joined_rect([5.0, 5.0], [10.0, 8.0])?;
        d.dimension(
            DimKind::Distance,
            vec![c(&lines[0])],
            DimValue::Param(ParamName("w".into())),
        )?;
        let landed = d.drag("a driven corner", &ends[2][0], &[[14.0, 9.5]])?;
        assert!(far(landed, [14.0, 9.5]) <= NM, "{landed:?}");
        let (a, b) = (d.at(&ends[0][0])?, d.at(&ends[0][1])?);
        assert!((far(a, b) - 6.0).abs() <= NM, "{a:?} {b:?}");
    }
    Ok(())
}

/// An arc's ends and centre, free and joined to a line.
#[test]
fn an_arcs_points_land_where_they_are_shown() -> Result<()> {
    for of in BOTH {
        for (which, to) in [(0, [1.0, 2.0]), (1, [7.0, 1.0]), (2, [1.0, 6.0])] {
            let mut d = Drawing::new(of)?;
            let points = d.arc([0.0, 0.5], [5.0, 0.5], [0.0, 5.5])?;
            let from = d.at(&points[which])?;
            let landed = d.drag("a free arc", &points[which], &path(from, to, 6))?;
            assert!(far(landed, to) <= NM, "{which}: {landed:?}");
            // Tangent to a line joined to its start.
            let mut d = Drawing::new(of)?;
            let points = d.arc([0.0, 0.5], [5.0, 0.5], [0.0, 5.5])?;
            let (line, [_, end]) = d.line([5.0, -6.0], [5.0, 0.5])?;
            d.constrain(GeomConstraint::Coincident, vec![p(&end), p(&points[1])])?;
            let arc = d
                .view()?
                .curves
                .iter()
                .find(|k| matches!(k.geometry, ViewGeometry::Arc { .. }))
                .map(|k| k.id.clone())
                .ok_or_else(|| Error::Invalid("no arc".into()))?;
            d.constrain(GeomConstraint::Tangent, vec![c(&line), c(&arc)])?;
            let from = d.at(&points[which])?;
            d.drag("a joined arc", &points[which], &path(from, to, 6))?;
        }
    }
    Ok(())
}

/// A circle's centre (a hole), free, at a set distance and on a line; its
/// radius keeps.
#[test]
fn a_circles_centre_lands_where_it_is_shown() -> Result<()> {
    for of in BOTH {
        let mut d = Drawing::new(of)?;
        let (circle, center) = d.circle([4.0, 3.0], 1.6)?;
        let landed = d.drag("a hole", &center, &path([4.0, 3.0], [9.0, 7.0], 8))?;
        assert!(far(landed, [9.0, 7.0]) <= NM, "{landed:?}");
        d.dimension(DimKind::Diameter, vec![c(&circle)], DimValue::Fixed(3.2))?;
        let landed = d.drag("a sized hole", &center, &[[6.0, 2.0]])?;
        assert!(far(landed, [6.0, 2.0]) <= NM, "{landed:?}");
        d.dimension(
            DimKind::Distance,
            vec![origin(), p(&center)],
            DimValue::Fixed(40f64.sqrt()),
        )?;
        let landed = d.drag("a hole round the origin", &center, &[[2.0, 9.0]])?;
        assert!(
            (far(landed, [0.0, 0.0]) - 40f64.sqrt()).abs() <= NM,
            "{landed:?}"
        );
        // Towards the pointer, not left where it was.
        assert!(far(landed, [2.0, 9.0]) < far([6.0, 2.0], [2.0, 9.0]) - 1.0);
        let mut d = Drawing::new(of)?;
        let (line, ends) = d.line([0.0, 1.0], [12.0, 5.0])?;
        for e in &ends {
            d.constrain(GeomConstraint::Fixed, vec![p(e)])?;
        }
        let (_, center) = d.circle([6.0, 3.0], 0.5)?;
        d.constrain(GeomConstraint::PointOnCurve, vec![p(&center), c(&line)])?;
        d.drag(
            "a hole on a line",
            &center,
            &path([6.0, 3.0], [9.0, 8.0], 5),
        )?;
    }
    Ok(())
}

/// Moving more than one point: a side of a rectangle takes its joined
/// neighbours along, the same as the preview of the same edit.
#[test]
fn a_moved_side_lands_where_it_is_shown() -> Result<()> {
    for of in BOTH {
        let mut d = Drawing::new(of)?;
        let (lines, ends) = d.joined_rect([5.0, 5.0], [10.0, 8.0])?;
        let edit = SketchEdit::Move {
            ids: vec![c(&lines[2])],
            delta: [0.0, 2.4],
        };
        let shown = LiveSketch::load(&d.source, &[])?.after(std::slice::from_ref(&edit))?;
        d.edit(edit)?;
        same("a side", &shown.view(), &d.view()?);
        for e in &ends[2] {
            assert!((d.at(e)?[1] - 10.4).abs() <= NM, "{:?}", d.at(e)?);
        }
        assert!(far(d.at(&ends[0][0])?, [5.0, 5.0]) <= NM);
    }
    Ok(())
}

/// A point dropped on another and joined to it, and one that is fixed: the
/// preview refuses what the release refuses.
#[test]
fn a_join_and_a_fixed_point_do_what_they_show() -> Result<()> {
    for of in BOTH {
        let mut d = Drawing::new(of)?;
        let (_, ends) = d.joined_rect([5.0, 5.0], [10.0, 8.0])?;
        let (_, [free, _]) = d.line([14.0, 9.0], [18.0, 12.0])?;
        let edits = [
            SketchEdit::Drag {
                point: ends[2][0].clone(),
                to: d.at(&free)?,
            },
            SketchEdit::AddConstraint {
                kind: GeomConstraint::Coincident,
                refs: vec![p(&ends[2][0]), p(&free)],
            },
        ];
        let shown = LiveSketch::load(&d.source, &[])?.after(&edits)?;
        for edit in &edits {
            d.apply(edit)?;
        }
        d.settle()?;
        same("a join", &shown.view(), &d.view()?);
        assert!(far(d.at(&ends[1][1])?, [14.0, 9.0]) <= NM);
        assert!(far(d.at(&free)?, [14.0, 9.0]) <= NM);

        d.constrain(GeomConstraint::Fixed, vec![p(&free)])?;
        let before = d.view()?;
        assert!(
            LiveSketch::load(&d.source, &[])?
                .drag(&free, [1.0, 1.0])
                .is_err()
        );
        let refused = d.edit(SketchEdit::Drag {
            point: free.clone(),
            to: [1.0, 1.0],
        });
        assert!(refused.is_err());
        same("a fixed point", &before, &d.view()?);
    }
    Ok(())
}
