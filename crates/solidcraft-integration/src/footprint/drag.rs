//! Dragging sketch points. What a drag shows while it is held
//! ([`super::view::LiveSketch::drag`]) and what its release stores
//! ([`super::edit::apply`]) are this one computation on the same sketch, so
//! the point lands where it was shown. It depends on the sketch and the
//! target only, not on the way the pointer took, and its work is counted,
//! not timed.
use super::check::MAX_COORD_MM;
use crate::geom::Vec2;
use crate::geom::prelude::*;
use solidcraft_engine::sketch::{Analysis, Budget, Hold, Sketch, move_point_towards, solve_with};

/// Frames' work (`Budget::frame`) following a target may take.
const FOLLOW_FRAMES: usize = 4;
/// A point that moved less, mm, did not move.
const STILL_MM: f64 = 1e-9;

pub(crate) struct Dragged {
    pub(crate) sketch: Sketch,
    /// The point followed (exactly, or as close as the constraints allow).
    pub(crate) moved: bool,
    /// The work ran out before the solve finished.
    pub(crate) exhausted: bool,
}

fn in_range(sk: &Sketch) -> bool {
    sk.points.iter().all(|q| {
        q.pos.is_finite() && q.pos.x.abs() <= MAX_COORD_MM && q.pos.y.abs() <= MAX_COORD_MM
    })
}

/// `start` with the points at their targets and held there, and the rest
/// solved round them; none when the constraints do not allow that.
pub(crate) fn held(
    start: &Sketch,
    targets: &[(usize, Vec2)],
    budget: Option<&mut Budget>,
) -> Option<Sketch> {
    let reference: Vec<Vec2> = start.points.iter().map(|q| q.pos).collect();
    let mut sketch = start.clone();
    for (p, to) in targets {
        sketch.points.get_mut(*p)?.pos = *to;
    }
    let hold = Hold {
        points: targets.iter().map(|(p, _)| *p).collect(),
        radii: Vec::new(),
    };
    let report = solve_with(
        &mut sketch,
        &hold,
        Some(&reference),
        Analysis::StatusOnly,
        budget,
    );
    (report.ok() && in_range(&sketch)).then_some(sketch)
}

/// Point `p` of `start` dragged to `target`: held there and the rest solved;
/// where the constraints do not allow that, as close as they allow;
/// otherwise the sketch as it was.
pub(crate) fn point(start: &Sketch, p: usize, target: Vec2) -> Dragged {
    let mut budget = Budget::frame();
    let spent = |b: &Budget| b.exhausted() || b.numeric_exhausted();
    if let Some(sketch) = held(start, &[(p, target)], Some(&mut budget)) {
        return Dragged {
            sketch,
            moved: true,
            exhausted: spent(&budget),
        };
    }
    let mut sketch = start.clone();
    let mut exhausted = false;
    for _ in 0..FOLLOW_FRAMES {
        let Some(from) = sketch.point(p) else { break };
        let mut next = sketch.clone();
        let followed = move_point_towards(
            &mut next,
            p,
            target,
            (target - from).len().max(1e-3),
            Some(&mut budget),
            |_, _| true,
        );
        exhausted = spent(&budget);
        let step = next.point(p).map_or(0.0, |at| (at - from).len());
        if followed && in_range(&next) {
            sketch = next;
        }
        // Done, unless the held solve or this frame's steps used its work up.
        if !exhausted || (followed && step <= STILL_MM) {
            break;
        }
        budget = Budget::frame();
    }
    let moved = start
        .point(p)
        .zip(sketch.point(p))
        .is_some_and(|(a, b)| (b - a).len() > STILL_MM);
    Dragged {
        sketch,
        moved,
        exhausted,
    }
}
