//! The boards' sketches, and what stands in for the sketch engine (the host's,
//! which the core does not have): a sketch is the solved board as text, and
//! the edits are the ones a person's come to. What the board makes of a solve
//! is the product's own.
use super::Fallible;
use super::plan::{BOARD, Edit};
use crate::diagnostics::{Error, Result};
use crate::geometry::Point;
use crate::layout::LayoutCommand;
use crate::layout::outline::{BoardCommand, BoardEdit, BoardEvaluation, BoardShapeEvaluator};
use crate::library::footprint_source::{
    EvaluatorRef, SketchBlob, SketchCurveId, SketchEdit, SketchOutcome, SolveSummary,
};
use crate::library::{Outline, Segment};
use crate::model::board::{
    BoardSketch, CutoutKind, CutoutRole, ShapeFault, ShapeFaultKind, SolvedBoard, SolvedCutout,
};
use crate::model::{BoardShape, Project};
use crate::{Evaluators, Operation};
use serde::{Deserialize, Serialize};

/// What a sketch says: the board, and whether its outline is closed.
#[derive(Serialize, Deserialize)]
struct Drawn {
    board: SolvedBoard,
    open: bool,
}

impl Drawn {
    fn of(sketch: &SketchBlob) -> Result<Self> {
        Ok(serde_json::from_str(&sketch.0)?)
    }

    /// With sorted keys, as a sketch engine gives it ([`SketchBlob`]).
    fn blob(&self) -> Result<SketchBlob> {
        let sorted = serde_json::to_value(self)?;
        Ok(SketchBlob(serde_json::to_string(&sorted)?))
    }
}

fn evaluator() -> EvaluatorRef {
    EvaluatorRef {
        engine: "random".into(),
        rev: "1".into(),
    }
}

fn summary() -> SolveSummary {
    SolveSummary {
        evaluator: evaluator(),
        status: Default::default(),
        dof: 0,
        result_hash: String::new(),
    }
}

/// The curves behind a cutout's outline, one a segment.
pub(super) fn curves(id: u32, outline: &Outline) -> Vec<SketchCurveId> {
    (0..outline.segments.len())
        .map(|k| SketchCurveId(format!("c{id}-{k}")))
        .collect()
}

/// The sketch of a board.
pub(super) fn sketch(board: SolvedBoard, outline_fit: f64) -> Fallible<BoardSketch> {
    let roles = board
        .cutouts
        .iter()
        .filter_map(|c| {
            Some(CutoutRole {
                id: c.id?,
                kind: c.kind,
                curves: c.curves.clone(),
            })
        })
        .collect();
    let drawn = Drawn { board, open: false };
    Ok(BoardSketch {
        evaluator: evaluator(),
        sketch: drawn.blob()?,
        parameters: Vec::new(),
        cutouts: roles,
        holes: Vec::new(),
        edges: Vec::new(),
        next_feature: 20,
        solve: summary(),
        solved: drawn.board,
        current: true,
        outline_fit,
        fault: None,
        idle: Vec::new(),
    })
}

/// Everything right of the middle and above it further out by `by`.
fn stretched(board: &mut SolvedBoard, by: Point) {
    let moved = |p: &mut Point| {
        for k in 0..2 {
            if p[k] > BOARD[k] / 2.0 {
                p[k] += by[k];
            }
        }
    };
    let outline = |o: &mut Outline| {
        moved(&mut o.start);
        for s in &mut o.segments {
            match s {
                Segment::Line { to } | Segment::Arc { to, .. } => moved(to),
            }
        }
    };
    outline(&mut board.outer);
    for c in &mut board.cutouts {
        // Whole, by its middle: a slot keeps its ends.
        let n = c.outline.segments.len().max(1) as f64;
        let mut middle = [0, 1].map(|k| {
            let sum: f64 = c.outline.segments.iter().map(|s| s.to()[k]).sum();
            sum / n
        });
        let before = middle;
        moved(&mut middle);
        let shift = |p: &mut Point| {
            for k in 0..2 {
                p[k] += middle[k] - before[k];
            }
        };
        shift(&mut c.outline.start);
        for s in &mut c.outline.segments {
            match s {
                Segment::Line { to } | Segment::Arc { to, .. } => shift(to),
            }
        }
    }
    for h in &mut board.holes {
        moved(&mut h.at);
    }
}

/// The edits' evaluator.
pub(super) struct Sketches;

impl BoardShapeEvaluator for Sketches {
    fn evaluator(&self) -> EvaluatorRef {
        evaluator()
    }

    fn rectangle(&self, _: Point, _: f64) -> Result<BoardSketch> {
        Err(Error::Unsupported(
            "the random boards start no sketch".into(),
        ))
    }

    /// A point is a stretch by it, a circle a window, a line opens or closes
    /// the outline.
    fn apply_sketch(&self, sketch: &SketchBlob, edit: &SketchEdit) -> Result<SketchOutcome> {
        let mut drawn = Drawn::of(sketch)?;
        match edit {
            SketchEdit::AddPoint { at } => stretched(&mut drawn.board, *at),
            SketchEdit::AddCircle { center, radius, .. } => {
                let ([x, y], r) = (*center, *radius);
                let outline = Outline {
                    start: [x + r, y],
                    segments: [[x - r, y], [x + r, y]]
                        .into_iter()
                        .map(|to| Segment::Arc { to, bulge: 1.0 })
                        .collect(),
                };
                let n = 100 + drawn.board.cutouts.len() as u32;
                drawn.board.cutouts.push(SolvedCutout {
                    id: None,
                    kind: CutoutKind::Cutout,
                    curves: curves(n, &outline),
                    outline,
                });
            }
            SketchEdit::AddLine { .. } => drawn.open = !drawn.open,
            other => return Err(Error::Unsupported(format!("no {other:?}"))),
        }
        Ok(SketchOutcome {
            sketch: drawn.blob()?,
            created: Vec::new(),
            deleted: Vec::new(),
        })
    }

    fn evaluate(&self, board: &BoardSketch) -> Result<BoardEvaluation> {
        let mut drawn = Drawn::of(&board.sketch)?;
        // Each loop is its role's, by its curves.
        for c in &mut drawn.board.cutouts {
            let role = board.cutouts.iter().find(|r| r.curves == c.curves);
            c.id = role.map(|r| r.id);
            c.kind = role.map_or(c.kind, |r| r.kind);
        }
        let fault = drawn.open.then(|| ShapeFault {
            kind: ShapeFaultKind::Open,
            message: "The outline is open".into(),
            at: Vec::new(),
        });
        Ok(BoardEvaluation {
            solved: (!drawn.open).then_some(drawn.board),
            solve: summary(),
            findings: Vec::new(),
            fault,
            sketch_out: None,
            idle: Vec::new(),
        })
    }
}

/// The evaluators an edit is made with.
pub(super) fn evaluators() -> Evaluators<'static> {
    Evaluators {
        footprint: None,
        board: Some(&Sketches),
    }
}

/// An edit of the board's shape as a command, on a sketch or a rectangle.
pub(super) fn operation(project: &Project, edit: &Edit) -> Option<Operation> {
    let sketch = matches!(project.board_shape, BoardShape::Sketch(_));
    let edits = |edit: SketchEdit| {
        Operation::Board(BoardCommand::Edit {
            edits: vec![BoardEdit::Sketch { edit }],
        })
    };
    Some(match edit {
        Edit::Stretch { by } if sketch => edits(SketchEdit::AddPoint { at: *by }),
        Edit::Stretch { by } => Operation::Layout(LayoutCommand::SetBoardSize {
            size: [project.board_size[0] + by[0], project.board_size[1] + by[1]],
        }),
        Edit::Window { at, radius } if sketch => edits(SketchEdit::AddCircle {
            center: *at,
            radius: *radius,
            construction: false,
        }),
        Edit::Open if sketch => edits(SketchEdit::AddLine {
            a: [0.0; 2],
            b: [1.0, 0.0],
            construction: false,
        }),
        _ => return None,
    })
}
