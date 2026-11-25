//! Board shape mode: the gesture builders on SolidCraft's real sketch
//! engine, the canvas and card in headless egui frames, and the expressions
//! a part dimension's value reads.
use super::*;
use crate::pcb_editor::edge_card;
use crate::pcb_editor::{PcbEditor, board_view};
use crate::refusal;
use egui::{Pos2, Shape};
use sparksmith_core::layout::outline::{BoardCommand, BoardEvaluation};
use sparksmith_core::library::footprint_source::SolveStatus;
use sparksmith_core::library::footprint_source::{EvaluatorRef, FootprintEvaluator, SketchOutcome};
use sparksmith_core::model::board::{NamedEdge, ParameterValue, ShapeFaultKind, SolvedBoard};
use sparksmith_core::{BoardShape, Engine, Operation};
use sparksmith_solidcraft::footprint::SketchEvaluator;
use sparksmith_solidcraft::footprint::view::LiveSketch;
use sparksmith_ui::fonts;

type Fallible<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

/// A stand-in for the integration crate's board evaluator: SolidCraft's
/// sketch edits (as the footprint evaluator applies them), no solve.
struct Host;

impl BoardShapeEvaluator for Host {
    fn evaluator(&self) -> EvaluatorRef {
        sparksmith_solidcraft::footprint::evaluator_ref()
    }
    fn rectangle(&self, size: Point, _outline_fit: f64) -> Result<BoardSketch> {
        board_of(size, Vec::new())
    }
    fn apply_sketch(&self, sketch: &SketchBlob, edit: &SketchEdit) -> Result<SketchOutcome> {
        SketchEvaluator.apply_sketch(sketch, edit)
    }
    fn evaluate(&self, _: &BoardSketch) -> Result<BoardEvaluation> {
        Err(Error::Unsupported("test host".into()))
    }
}

static HOST: Host = Host;

fn blank() -> Result<SketchBlob> {
    SketchEvaluator.new_sketch()
}

/// Apply a commit's sketch edits to `sketch`, as the engine would.
fn applied(sketch: &SketchBlob, edits: &[BoardEdit]) -> Result<SketchBlob> {
    let mut blob = sketch.clone();
    for edit in edits {
        if let BoardEdit::Sketch { edit } = edit {
            blob = SketchEvaluator.apply_sketch(&blob, edit)?.sketch;
        }
    }
    Ok(blob)
}

fn parameters_of(edits: &[BoardEdit]) -> Vec<FootprintParam> {
    edits
        .iter()
        .filter_map(|e| match e {
            BoardEdit::SetParameter { param } => Some(param.clone()),
            _ => None,
        })
        .collect()
}

fn status(sketch: &SketchBlob, parameters: Vec<FootprintParam>) -> Result<SolveStatus> {
    let live = LiveSketch::load(&source_with(sketch.clone(), parameters), &[])?;
    Ok(live.report().status)
}

/// A board sketch of a `size` rectangle drawn by the Rectangle tool's
/// builder, its sides named as a rectangle's.
fn board_of(size: Point, parameters: Vec<FootprintParam>) -> Result<BoardSketch> {
    let mut stage = Stage::new(&HOST, &blank()?);
    let curves = rectangle(&mut stage, [0.0, 0.0], size, 0.0, None, false)?;
    let edges = curves
        .iter()
        .zip(sparksmith_core::model::board::RECT_EDGES)
        .map(|(c, name)| NamedEdge {
            name: EdgeName(name.into()),
            curve: c.clone(),
        })
        .collect();
    let mut board = BoardSketch {
        evaluator: HOST.evaluator(),
        sketch: stage.staged().clone(),
        parameters,
        cutouts: Vec::new(),
        holes: Vec::new(),
        edges,
        next_feature: 1,
        solve: Default::default(),
        solved: SolvedBoard::rectangle(size),
        current: true,
        outline_fit: 0.01,
        fault: None,
        idle: Vec::new(),
    };
    board.solved.values = view::parameter_values(&source_of(&board), &[])?
        .into_iter()
        .map(|p| {
            let value = p.value.map_err(Error::Invalid)?;
            Ok(ParameterValue {
                name: p.name,
                value,
            })
        })
        .collect::<Result<_>>()?;
    Ok(board)
}

fn scene_view(board: &BoardSketch) -> Result<SketchView> {
    view::sketch_view(&source_of(board))
}

#[test]
fn a_rounded_rectangle_binds_its_corners_to_one_parameter_and_solves() -> Fallible {
    let mut stage = Stage::new(&HOST, &blank()?);
    let name = fresh_name(&[], "corner_r");
    let curves = rectangle(
        &mut stage,
        [0.0, 0.0],
        [50.0, 30.0],
        3.0,
        Some((name.clone(), true)),
        false,
    )?;
    // Four sides and four corner arcs, in loop order.
    assert_eq!(curves.len(), 8);
    let params = parameters_of(&stage.edits);
    assert_eq!(params.len(), 1);
    assert_eq!(params[0].name, name);
    assert_eq!(params[0].expr.0, "3 mm");
    let bound = stage
        .edits
        .iter()
        .filter(|e| {
            matches!(e, BoardEdit::Sketch { edit: SketchEdit::AddDimension {
                kind: DimKind::Radius, value: DimValue::Param(p), ..
            } } if *p == name)
        })
        .count();
    assert_eq!(bound, 4, "every corner reads the one radius parameter");
    let sketch = stage.staged().clone();
    let solved = status(&sketch, params.clone())?;
    assert!(
        matches!(solved, SolveStatus::Solved | SolveStatus::UnderConstrained),
        "{solved:?}"
    );
    // A second rounded rectangle gets its own parameter.
    assert_eq!(fresh_name(&params, "corner_r").0, "corner_r2");
    // Too large a radius is refused before anything is drawn.
    let mut stage = Stage::new(&HOST, &blank()?);
    assert!(rectangle(&mut stage, [0.0, 0.0], [10.0, 4.0], 2.5, None, false).is_err());
    assert!(stage.edits.is_empty());
    Ok(())
}

#[test]
fn a_fillet_rounds_a_drawn_corner_and_refuses_a_shared_one() -> Fallible {
    let board = board_of([40.0, 20.0], Vec::new())?;
    let view = scene_view(&board)?;
    let mut stage = Stage::new(&HOST, &board.sketch);
    fillet(&mut stage, &view, [0.1, 0.0], 0.5, 4.0)?;
    let after = view_of(stage.staged())?;
    let arcs: Vec<_> = after
        .curves
        .iter()
        .filter_map(|c| match c.geometry {
            ViewGeometry::Arc { center, start, end } => Some((center, start, end)),
            _ => None,
        })
        .collect();
    assert_eq!(arcs.len(), 1);
    let (center, start, end) = arcs[0];
    assert!(dist(center, [4.0, 4.0]) < 1e-6, "{center:?}");
    // The corner's ends moved to the tangent points.
    let ends = [start, end];
    assert!(ends.iter().any(|p| dist(*p, [4.0, 0.0]) < 1e-6));
    assert!(ends.iter().any(|p| dist(*p, [0.0, 4.0]) < 1e-6));
    assert!(matches!(
        status(stage.staged(), Vec::new())?,
        SolveStatus::Solved | SolveStatus::UnderConstrained
    ));
    // Too large for the 20 mm side.
    let mut stage = Stage::new(&HOST, &board.sketch);
    let error = fillet(&mut stage, &view, [0.0, 0.0], 0.5, 25.0)
        .err()
        .ok_or("too large a fillet was accepted")?;
    assert!(error.to_string().contains("too large"), "{error}");
    // A corner drawn as one shared point is the evaluator's fillet.
    let shared = SketchEvaluator
        .apply_sketch(
            &blank()?,
            &SketchEdit::AddRect {
                a: [0.0, 0.0],
                b: [10.0, 10.0],
                construction: false,
            },
        )?
        .sketch;
    let view = view_of(&shared)?;
    let mut stage = Stage::new(&HOST, &shared);
    fillet(&mut stage, &view, [10.0, 10.0], 0.5, 1.0)?;
    let [BoardEdit::Fillet { corners, radius }] = stage.edits.as_slice() else {
        return Err(format!("not one fillet: {:?}", stage.edits).into());
    };
    let corner = view
        .points
        .iter()
        .find(|p| dist(p.at, [10.0, 10.0]) < 1e-9)
        .ok_or("no shared corner")?;
    assert_eq!(corners, &vec![corner.id.clone()]);
    assert_eq!(radius, &DimValue::Fixed(1.0));
    // Away from any corner, the click is refused.
    let mut stage = Stage::new(&HOST, &shared);
    let error = fillet(&mut stage, &view, [5.0, 5.0], 0.5, 1.0)
        .err()
        .ok_or("a fillet away from a corner was accepted")?;
    assert!(error.to_string().contains("corner"), "{error}");
    Ok(())
}

/// The demo run's first gesture: Fillet on a corner of the sketch Shape
/// mode generated for a plain rectangle board rounds it in one step.
#[test]
fn a_fillet_rounds_a_corner_of_the_generated_board_sketch() -> Fallible {
    let mut d = Drawn::new()?;
    d.editor.shape.tool = ShapeTool::Fillet;
    let corner = d.at(1.0, 1.0);
    let operation = d.click(corner);
    let edits = d.apply(operation)?;
    assert!(
        matches!(edits.as_slice(), [BoardEdit::Fillet { corners, .. }] if corners.len() == 1),
        "{edits:?}"
    );
    let board = d.board()?;
    assert!(board.fault.is_none(), "{:?}", board.fault);
    let arcs = |b: &BoardSketch| {
        b.solved
            .outer
            .segments
            .iter()
            .filter(|s| matches!(s, sparksmith_core::library::Segment::Arc { .. }))
            .count()
    };
    assert_eq!(arcs(&board), 1, "{:?}", board.solved.outer);
    d.engine.undo()?;
    assert_eq!(arcs(&d.board()?), 0);
    Ok(())
}

#[test]
fn cutouts_slots_and_holes_name_the_curves_and_points_they_draw() -> Fallible {
    let board = board_of([60.0, 40.0], Vec::new())?;
    let view = scene_view(&board)?;
    let scene = Scene {
        board: &board,
        view: &view,
        evaluator: &HOST,
        tolerance: 0.3,
    };
    let options = ShapeOptions::default();
    for (tool, clicks) in [
        (ShapeTool::Cutout, [[10.0, 10.0], [20.0, 18.0]]),
        (ShapeTool::Slot, [[30.0, 20.0], [45.0, 20.0]]),
    ] {
        let mut gesture = Gesture::default();
        assert!(matches!(
            press(tool, &options, &mut gesture, clicks[0], &scene)?,
            Step::More
        ));
        let Step::Commit(commit) = press(tool, &options, &mut gesture, clicks[1], &scene)? else {
            return Err(format!("{tool:?} did not commit").into());
        };
        let drawn = view_of(&applied(&board.sketch, &commit.edits)?)?;
        let roles: Vec<_> = commit
            .edits
            .iter()
            .filter_map(|e| match e {
                BoardEdit::SetCutoutKind { curves, kind } => Some((curves, kind)),
                _ => None,
            })
            .collect();
        if tool == ShapeTool::Slot {
            // The slot's four curves get a slot role.
            let [(curves, kind)] = roles.as_slice() else {
                return Err("the slot named no slot role".into());
            };
            assert_eq!(**kind, CutoutKind::Slot { plated: false });
            assert_eq!(curves.len(), 4);
            assert!(curves.iter().all(|c| drawn.curve(c).is_some()));
        } else {
            // A closed loop inside the outline is a cutout without a role.
            assert!(roles.is_empty());
            assert_eq!(drawn.curves.len(), view.curves.len() + 4);
        }
        assert!(matches!(
            status(&applied(&board.sketch, &commit.edits)?, Vec::new())?,
            SolveStatus::Solved | SolveStatus::UnderConstrained
        ));
    }
    let mut gesture = Gesture::default();
    let Step::Commit(commit) = press(ShapeTool::Hole, &options, &mut gesture, [5.0, 5.0], &scene)?
    else {
        return Err("the hole did not commit".into());
    };
    let Some(BoardEdit::AddMountingHole { center, drill, .. }) = commit.edits.last() else {
        return Err("no mounting hole".into());
    };
    assert_eq!(drill.0, "3.2 mm");
    let drawn = view_of(&applied(&board.sketch, &commit.edits)?)?;
    assert_eq!(drawn.point(center).map(|p| p.at), Some([5.0, 5.0]));
    Ok(())
}

#[test]
fn lines_chain_and_tie_to_the_points_they_snap_to() -> Fallible {
    let board = board_of([60.0, 40.0], Vec::new())?;
    let view = scene_view(&board)?;
    let scene = Scene {
        board: &board,
        view: &view,
        evaluator: &HOST,
        tolerance: 0.5,
    };
    let mut gesture = Gesture::default();
    let options = ShapeOptions::default();
    press(
        ShapeTool::Line,
        &options,
        &mut gesture,
        [60.2, 39.8],
        &scene,
    )?;
    let Step::Commit(commit) = press(
        ShapeTool::Line,
        &options,
        &mut gesture,
        [70.0, 30.0],
        &scene,
    )?
    else {
        return Err("the line did not commit".into());
    };
    // Snapped to the board's corner and tied there.
    assert!(commit.edits.iter().any(|e| matches!(
        e,
        BoardEdit::Sketch { edit: SketchEdit::AddLine { a, .. } } if *a == [60.0, 40.0]
    )));
    assert!(commit.edits.iter().any(|e| matches!(
        e,
        BoardEdit::Sketch {
            edit: SketchEdit::AddConstraint {
                kind: GeomConstraint::Coincident,
                ..
            }
        }
    )));
    // The chain continues from the line's end.
    assert_eq!(gesture.pending.len(), 1);
    assert_eq!(gesture.pending[0].at, [70.0, 30.0]);
    Ok(())
}

#[test]
fn a_dimension_is_placed_then_bound_to_a_board_parameter() -> Fallible {
    let board = board_of([60.0, 40.0], Vec::new())?;
    let view = scene_view(&board)?;
    let scene = Scene {
        board: &board,
        view: &view,
        evaluator: &HOST,
        tolerance: 0.5,
    };
    let mut gesture = Gesture::default();
    let options = ShapeOptions::default();
    // The bottom side, then a click below it places its length.
    press(
        ShapeTool::Dimension,
        &options,
        &mut gesture,
        [30.0, 0.0],
        &scene,
    )?;
    let Step::Dimension(new) = press(
        ShapeTool::Dimension,
        &options,
        &mut gesture,
        [30.0, -5.0],
        &scene,
    )?
    else {
        return Err("the dimension was not placed".into());
    };
    assert_eq!(new.value, 60.0);
    // The value defines the parameter and adds the dimension bound to it.
    let edits = new_dimension_edits(&board, &new, "width = 80 mm")?;
    assert!(matches!(
        edits.first(),
        Some(BoardEdit::SetParameter { param }) if param.name.0 == "width" && param.expr.0 == "80 mm"
    ));
    assert!(matches!(
        edits.get(1),
        Some(BoardEdit::Sketch { edit: SketchEdit::AddDimension { value: DimValue::Param(p), .. } })
            if p.0 == "width"
    ));
    assert!(new_dimension_edits(&board, &new, "nowhere").is_err());
    // Untouched, it is taken as measured.
    let edits = new_dimension_edits(&board, &new, "60")?;
    let id = new.id.clone();
    let dimensioned = BoardSketch {
        sketch: applied(&board.sketch, &edits)?,
        ..board.clone()
    };
    let view = scene_view(&dimensioned)?;
    let shown = view
        .constraints
        .iter()
        .find(|k| k.id == id)
        .ok_or("the dimension is missing")?;
    assert_eq!(shown.value.map(|v| (v * 1e6).round() / 1e6), Some(60.0));
    let edits = dimension_value_edits(&dimensioned, &view, &id, "width = 80 mm")?;
    assert!(matches!(
        edits.first(),
        Some(BoardEdit::SetParameter { param }) if param.name.0 == "width" && param.expr.0 == "80 mm"
    ));
    assert!(matches!(
        edits.get(1),
        Some(BoardEdit::Sketch { edit: SketchEdit::SetDimension { value: DimValue::Param(p), .. } })
            if p.0 == "width"
    ));
    // A bare name must exist.
    assert!(dimension_value_edits(&dimensioned, &view, &id, "nowhere").is_err());
    Ok(())
}

#[test]
fn deleting_a_named_curve_unnames_it_first() -> Fallible {
    let board = board_of([60.0, 40.0], Vec::new())?;
    let bottom = board.edges[0].curve.clone();
    let edits = delete_edits(&board, &[SketchRef::Curve(bottom)]);
    assert!(matches!(
        edits.first(),
        Some(BoardEdit::UnnameEdge { name }) if name.0 == "bottom"
    ));
    assert!(matches!(
        edits.last(),
        Some(BoardEdit::Sketch {
            edit: SketchEdit::Delete { .. }
        })
    ));
    assert!(delete_edits(&board, &[]).is_empty());
    Ok(())
}

fn sketch_engine(parameters: Vec<FootprintParam>) -> Fallible<Engine> {
    let mut file = Engine::demo()?.to_file();
    let size = file.project.board_size;
    file.project.board_shape = BoardShape::Sketch(Box::new(board_of(size, parameters)?));
    Ok(Engine::from_file(file)?)
}

#[test]
fn edge_values_evaluate_board_parameters() -> Fallible {
    let width = FootprintParam {
        name: ParamName("width".into()),
        expr: Expr("80 mm".into()),
        unit: ParamUnit::Mm,
        comment: String::new(),
    };
    let engine = sketch_engine(vec![width])?;
    let project = engine.project();
    let v = edge_card::evaluate(project, &Expr("width / 2 - 3 mm".into()))
        .ok_or("the expression did not evaluate")?;
    assert!((v - 37.0).abs() < 1e-9, "{v}");
    assert_eq!(
        edge_card::evaluate(project, &Expr("2.5 mm".into())),
        Some(2.5)
    );
    // As the guide on units says.
    for (text, mm) in [("1 in", 25.4), ("100 mil", 2.54)] {
        let v = edge_card::evaluate(project, &Expr(text.into())).ok_or(text)?;
        assert!((v - mm).abs() < 1e-9, "{text}: {v}");
    }
    assert!(edge_card::expression_error(project, "width / 2").is_none());
    assert!(edge_card::expression_error(project, "depth").is_some());
    // A plain rectangle board reads `width` and `height` as the dimension
    // solve does, before Shape mode ever made it a sketch.
    let demo = Engine::demo()?;
    assert!(matches!(demo.project().board_shape, BoardShape::Rectangle));
    let [_, h] = demo.project().board_size;
    assert!(edge_card::expression_error(demo.project(), "height / 2").is_none());
    let v = edge_card::evaluate(demo.project(), &Expr("height / 2".into()))
        .ok_or("height / 2 did not evaluate on a rectangle board")?;
    assert!((v - h / 2.0).abs() < 1e-9, "{v} vs {h}");
    assert!(edge_card::expression_error(demo.project(), "depth").is_some());
    assert!(edge_card::expression_error(demo.project(), "4 mm").is_none());
    Ok(())
}

#[test]
fn ear_clipping_fills_a_concave_board() {
    // An L: 6 corners, 4 triangles, area 3 (of unit squares).
    let ring = [
        Pos2::new(0.0, 0.0),
        Pos2::new(2.0, 0.0),
        Pos2::new(2.0, 1.0),
        Pos2::new(1.0, 1.0),
        Pos2::new(1.0, 2.0),
        Pos2::new(0.0, 2.0),
    ];
    let triangles = crate::pcb_editor::board_shape_ui::triangulate(&ring);
    assert_eq!(triangles.len(), 4);
    let area: f32 = triangles
        .iter()
        .map(|[a, b, c]| {
            let (a, b, c) = (ring[*a as usize], ring[*b as usize], ring[*c as usize]);
            ((b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)).abs() / 2.0
        })
        .sum();
    assert!((area - 3.0).abs() < 1e-5, "{area}");
}

// --- Headless frames ------------------------------------------------------

const SCREEN: egui::Vec2 = egui::vec2(900.0, 700.0);

fn canvas(
    editor: &mut PcbEditor,
    engine: &Engine,
    ctx: &egui::Context,
    events: Vec<egui::Event>,
) -> (egui::FullOutput, Option<Operation>, egui::Rect) {
    let mut out = None;
    let mut rect = egui::Rect::NOTHING;
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, SCREEN)),
            events,
            ..Default::default()
        },
        |ui| {
            rect = ui.available_rect_before_wrap();
            out = editor.canvas(ui, engine);
        },
    );
    output.textures_delta.clear();
    (output, out, rect)
}

fn slot(
    editor: &mut PcbEditor,
    engine: &Engine,
    ctx: &egui::Context,
    events: Vec<egui::Event>,
) -> (egui::FullOutput, Option<Operation>) {
    let mut out = None;
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                Pos2::ZERO,
                egui::vec2(700.0, 900.0),
            )),
            events,
            ..Default::default()
        },
        |ui| {
            out = editor.subject_slot(ui, engine).output;
            let _ = refusal::take_mark(ui.ctx());
        },
    );
    output.textures_delta.clear();
    (output, out)
}

fn texts(shape: &Shape, out: &mut Vec<(String, Pos2)>) {
    match shape {
        Shape::Text(text) => out.push((
            text.galley.job.text.clone(),
            text.pos + text.galley.size() * 0.5,
        )),
        Shape::Vec(shapes) => shapes.iter().for_each(|s| texts(s, out)),
        _ => {}
    }
}

fn painted(output: &egui::FullOutput) -> Vec<(String, Pos2)> {
    let mut out = Vec::new();
    for shape in &output.shapes {
        texts(&shape.shape, &mut out);
    }
    out
}

fn press_at(pos: Pos2) -> Vec<egui::Event> {
    vec![
        egui::Event::PointerMoved(pos),
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        },
    ]
}

fn release_at(pos: Pos2) -> Vec<egui::Event> {
    vec![egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    }]
}

fn key(key: egui::Key) -> Vec<egui::Event> {
    vec![egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }]
}

/// Click a card button by its text.
fn click_text(
    editor: &mut PcbEditor,
    engine: &Engine,
    ctx: &egui::Context,
    label: &str,
) -> Fallible<Option<Operation>> {
    let (output, _) = slot(editor, engine, ctx, vec![]);
    let at = painted(&output)
        .into_iter()
        .find(|(t, _)| t == label)
        .map(|(_, p)| p)
        .ok_or_else(|| format!("no {label} on the card"))?;
    slot(editor, engine, ctx, press_at(at));
    Ok(slot(editor, engine, ctx, release_at(at)).1)
}

#[test]
fn the_shape_mode_draws_the_sketch_and_a_rectangle_commits_one_board_edit() -> Fallible {
    let engine = sketch_engine(Vec::new())?;
    let ctx = egui::Context::default();
    fonts::setup_bundled_fonts(&ctx);
    let mut editor = PcbEditor::default();
    editor.shape.evaluator = Some(&HOST);
    assert!(
        editor.on_tool("board-shape", &engine).is_none(),
        "already a sketch"
    );
    assert!(editor.shape.active);
    let ribbon = editor.ribbon(&engine);
    let ids: Vec<&str> = ribbon
        .groups
        .iter()
        .flat_map(|g| &g.tools)
        .flat_map(|t| std::iter::once(t.id).chain(t.menu.iter().map(|v| v.id)))
        .collect();
    for tool in ShapeTool::ALL {
        assert!(ids.contains(&tool.id()), "{}", tool.id());
    }
    assert_eq!(editor.subject(), crate::kit::card::Subject::Tool);
    // The sketch is drawn with its named edges.
    let (output, _, rect) = canvas(&mut editor, &engine, &ctx, vec![]);
    let shown = painted(&output);
    for name in ["bottom", "right", "top", "left"] {
        assert!(shown.iter().any(|(t, _)| t == name), "{name} is not drawn");
    }
    // R arms Rectangle; two clicks draw a cutout-free rectangle.
    let view = board_view(
        rect,
        engine.project().board_size,
        1.0,
        egui::Vec2::ZERO,
        false,
    );
    let hover = view.screen([5.0, 5.0]);
    canvas(
        &mut editor,
        &engine,
        &ctx,
        vec![egui::Event::PointerMoved(hover)],
    );
    canvas(&mut editor, &engine, &ctx, key(egui::Key::R));
    assert_eq!(editor.shape.tool, ShapeTool::Rect);
    let mut operation = None;
    for at in [[5.0, 5.0], [12.0, 9.0]] {
        let pos = view.screen(at);
        canvas(&mut editor, &engine, &ctx, press_at(pos));
        operation = canvas(&mut editor, &engine, &ctx, release_at(pos))
            .1
            .or(operation);
    }
    let Some(Operation::Board(BoardCommand::Edit { edits })) = operation else {
        return Err(format!("no board.edit: {operation:?}").into());
    };
    let lines = edits
        .iter()
        .filter(|e| {
            matches!(
                e,
                BoardEdit::Sketch {
                    edit: SketchEdit::AddLine { .. }
                }
            )
        })
        .count();
    assert_eq!(lines, 4);
    // The card names the tool and shows the board parameters.
    let (output, _) = slot(&mut editor, &engine, &ctx, vec![]);
    let card = painted(&output);
    assert!(card.iter().any(|(t, _)| t == "Rectangle"));
    assert!(card.iter().any(|(t, _)| t.starts_with("BOARD PARAMETERS")));
    // Esc on the canvas steps back: the tool, then the mode.
    canvas(
        &mut editor,
        &engine,
        &ctx,
        vec![egui::Event::PointerMoved(hover)],
    );
    canvas(&mut editor, &engine, &ctx, key(egui::Key::Escape));
    assert_eq!(editor.shape.tool, ShapeTool::Select);
    canvas(&mut editor, &engine, &ctx, key(egui::Key::Escape));
    assert!(!editor.shape.active);
    Ok(())
}

#[test]
fn the_board_card_offers_edit_shape_and_start_sketch_changes_nothing_when_refused() -> Fallible {
    let mut engine = Engine::demo()?;
    let ctx = egui::Context::default();
    fonts::setup_bundled_fonts(&ctx);
    let mut editor = PcbEditor::default();
    let (output, _) = slot(&mut editor, &engine, &ctx, vec![]);
    assert!(painted(&output).iter().any(|(t, _)| t == "Rectangle"));
    let operation = click_text(&mut editor, &engine, &ctx, "Edit shape…")?;
    let Some(operation @ Operation::Board(BoardCommand::StartSketch)) = operation else {
        return Err(format!("no board.start_sketch: {operation:?}").into());
    };
    let before = engine.project().clone();
    let revision = engine.revision();
    // Without a board evaluator the engine refuses, and nothing changes.
    assert!(engine.apply(revision, operation).is_err());
    assert_eq!(engine.project(), &before);
    assert_eq!(engine.revision(), revision);
    editor.sync(&engine);
    assert!(!editor.shape.active);
    Ok(())
}

#[test]
fn the_board_card_edits_a_board_parameter() -> Fallible {
    let width = FootprintParam {
        name: ParamName("width".into()),
        expr: Expr("80 mm".into()),
        unit: ParamUnit::Mm,
        comment: String::new(),
    };
    let half = FootprintParam {
        name: ParamName("half".into()),
        expr: Expr("width / 2".into()),
        unit: ParamUnit::Mm,
        comment: String::new(),
    };
    let engine = sketch_engine(vec![width, half])?;
    let ctx = egui::Context::default();
    fonts::setup_bundled_fonts(&ctx);
    let mut editor = PcbEditor::default();
    let (output, _) = slot(&mut editor, &engine, &ctx, vec![]);
    let shown = painted(&output);
    // Each parameter is one list row: its name and its value, the number
    // and its dim unit in their own slots; an expression reads `fx expr =`
    // and its result inline.
    let has = |s: &str| shown.iter().any(|(t, _)| t == s);
    for text in ["width", "80", "mm", "half", "fx", "width / 2", " = ", "40"] {
        assert!(has(text), "{text}: {shown:?}");
    }
    // The two values line up on their decimal point: "80" and "40"
    // (the same width) sit at one x.
    let right = |s: &str| shown.iter().find(|(t, _)| t == s).map(|(_, at)| at.x);
    let (a, b) = (right("80").ok_or("80")?, right("40").ok_or("40")?);
    assert!((a - b).abs() < 0.5, "aligned: {a} {b}");
    let board =
        crate::pcb_editor::board_shape_ui::sketch_of(engine.project()).ok_or("no sketch")?;
    let edit = parameter_edit(board, &ParamName("width".into()), "90 mm", ParamUnit::Mm)?;
    assert!(matches!(edit, BoardEdit::SetParameter { param } if param.expr.0 == "90 mm"));
    // Demo run 3: a bare number keeps the unit every other field shows.
    let edit = parameter_edit(board, &ParamName("width".into()), " 52 ", ParamUnit::Mm)?;
    assert!(
        matches!(&edit, BoardEdit::SetParameter { param } if param.expr.0 == "52 mm"),
        "{edit:?}"
    );
    let edit = parameter_edit(board, &ParamName("width".into()), "40 + 12", ParamUnit::Mm)?;
    assert!(
        matches!(&edit, BoardEdit::SetParameter { param } if param.expr.0 == "40 + 12"),
        "an expression is kept as typed: {edit:?}"
    );
    assert!(
        parameter_edit(
            board,
            &ParamName("width".into()),
            "depth * 2",
            ParamUnit::Mm
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn the_placement_card_moves_and_resets_the_designator_in_one_step_each() -> Fallible {
    use sparksmith_core::InstanceCommand;
    let mut engine = Engine::demo()?;
    let ctx = egui::Context::default();
    fonts::setup_bundled_fonts(&ctx);
    let mut editor = PcbEditor::default();
    let id = engine
        .project()
        .placements
        .first()
        .map(|p| p.instance)
        .ok_or("the demo has no placement")?;
    editor.set_selection(Some(id), None, None);
    let (output, op) = slot(&mut editor, &engine, &ctx, vec![]);
    assert!(op.is_none());
    let shown = painted(&output);
    assert!(shown.iter().any(|(t, _)| t == "DESIGNATOR"), "{shown:?}");
    assert!(
        !shown.iter().any(|(t, _)| t == "Reset"),
        "nothing moved yet"
    );
    // A typed offset commits once, as one command.
    editor.text_offset = [1.5, -0.5];
    let (_, op) = slot(&mut editor, &engine, &ctx, vec![]);
    let Some(op @ Operation::Instance(InstanceCommand::SetTextPlacement { .. })) = op else {
        return Err(format!("no text placement: {op:?}").into());
    };
    let (_, again) = slot(&mut editor, &engine, &ctx, vec![]);
    assert!(again.is_none(), "sent twice: {again:?}");
    let revision = engine.revision();
    engine.apply(revision, op)?;
    assert_eq!(engine.revision(), revision + 1);
    let stored = |engine: &Engine| {
        engine
            .project()
            .instance(id)
            .map(|i| i.text_placements.values().next().map(|p| p.offset))
    };
    assert_eq!(stored(&engine), Some(Some([1.5, -0.5])));
    // The card shows the stored value and offers Reset.
    let op = click_text(&mut editor, &engine, &ctx, "Reset")?.ok_or("no reset")?;
    engine.apply(engine.revision(), op)?;
    assert_eq!(stored(&engine), Some(None));
    assert_eq!(engine.revision(), revision + 2);
    // Undo brings the moved text back, and the card follows the document.
    engine.undo()?;
    slot(&mut editor, &engine, &ctx, vec![]);
    assert_eq!(editor.text_offset, [1.5, -0.5]);
    Ok(())
}

/// The app's host evaluator is SolidCraft's board sketch engine: on a real
/// engine, Shape's start and a Cutout gesture each solve as one board step,
/// the cutout lands in the solved board and one undo removes it.
#[test]
fn the_host_evaluator_solves_a_cutout_gesture_on_the_engine() -> Fallible {
    let evaluator = host_evaluator().ok_or("no host board evaluator")?;
    let hosted = || sparksmith_core::Evaluators {
        footprint: None,
        board: Some(evaluator),
    };
    let mut engine = Engine::showcase()?;
    engine.apply_hosted(
        engine.revision(),
        Operation::Board(BoardCommand::StartSketch),
        hosted(),
    )?;
    let BoardShape::Sketch(board) = engine.project().board_shape.clone() else {
        return Err("the board did not become a sketch".into());
    };
    let view = scene_view(&board)?;
    let scene = Scene {
        board: &board,
        view: &view,
        evaluator,
        tolerance: 0.3,
    };
    let options = ShapeOptions::default();
    let mut gesture = Gesture::default();
    press(
        ShapeTool::Cutout,
        &options,
        &mut gesture,
        [10.0, 10.0],
        &scene,
    )?;
    let Step::Commit(commit) = press(
        ShapeTool::Cutout,
        &options,
        &mut gesture,
        [14.0, 13.0],
        &scene,
    )?
    else {
        return Err("the cutout did not commit".into());
    };
    let before = engine.revision();
    engine.apply_hosted(before, operation(commit.edits), hosted())?;
    assert_eq!(engine.revision(), before + 1, "one step");
    assert_eq!(engine.project().board().cutouts.len(), 1);
    engine.undo()?;
    assert!(engine.project().board().cutouts.is_empty());
    Ok(())
}

/// Finding 3 of the UI run: after a card edit in the board shape sketch,
/// with a field still focused, ⌘S passes the card and the canvas untouched,
/// so the app's dispatcher (which runs after them) sees it and saves.
#[test]
fn the_shape_mode_leaves_command_s_for_the_dispatcher() -> Fallible {
    let engine = sketch_engine(Vec::new())?;
    let ctx = egui::Context::default();
    fonts::setup_bundled_fonts(&ctx);
    let mut editor = PcbEditor::default();
    editor.shape.evaluator = Some(&HOST);
    let _ = editor.on_tool("board-shape", &engine);
    assert!(editor.shape.active);
    assert_eq!(
        editor.shortcut_context(),
        crate::commands::Context::BoardShape
    );
    let mut value = "inset = 4".to_owned();
    let field = egui::Id::unique("card-field");
    let save = egui::Event::Key {
        key: egui::Key::S,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::COMMAND,
    };
    let mut left = None;
    for (round, events) in [vec![], vec![save]].into_iter().enumerate() {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    Pos2::ZERO,
                    egui::vec2(1200.0, 900.0),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                let response = ui.add(egui::TextEdit::singleline(&mut value).id(field));
                if round == 0 {
                    response.request_focus();
                }
                let _ = editor.subject_slot(ui, &engine);
                let _ = editor.canvas(ui, &engine);
                let _ = refusal::take_mark(ui.ctx());
                let typing = ui.ctx().egui_wants_keyboard_input();
                let kept = ui.input(|i| {
                    i.events.iter().any(|e| {
                        matches!(
                            e,
                            egui::Event::Key {
                                key: egui::Key::S,
                                pressed: true,
                                modifiers,
                                ..
                            } if modifiers.command
                        )
                    })
                });
                left = Some((typing, kept));
            },
        );
        output.textures_delta.clear();
    }
    assert_eq!(left, Some((true, true)), "(typing, ⌘S still there)");
    Ok(())
}

/// Run 3, finding 18: the two inset dimensions to a corner hole put their
/// labels on one spot; the second slides along its own line, clear of the
/// first, and a clear label stays where its dimension put it.
#[test]
fn corner_dimension_labels_do_not_overlap() {
    use crate::pcb_editor::board_shape_ui::clear_of;
    use egui::{Rect, vec2};
    // The horizontal inset's label at the corner, then the vertical one's
    // nearly on top of it (as in the run's top-left corner).
    let horizontal = Rect::from_center_size(Pos2::new(330.0, 412.0), vec2(56.0, 16.0));
    let vertical = Rect::from_center_size(Pos2::new(352.0, 415.0), vec2(56.0, 16.0));
    let moved = clear_of(vertical, vec2(0.0, 1.0), &[horizontal]);
    assert!(!moved.intersects(horizontal), "{moved:?}");
    // It moved along its own (vertical) line only, by the least it needed.
    assert!((moved.center().x - vertical.center().x).abs() < 1e-3);
    assert!((moved.center().y - vertical.center().y).abs() <= 18.0 + 1e-3);
    let clear = Rect::from_center_size(Pos2::new(600.0, 600.0), vec2(56.0, 16.0));
    assert_eq!(clear_of(clear, vec2(1.0, 0.0), &[horizontal]), clear);
}

/// Edge names sit at the middle of their edge, outside the outline, apart
/// from each other; a name whose place is taken (a dimension label, a part)
/// slides clear of it.
#[test]
fn edge_names_centre_outside_their_edges_and_keep_clear() {
    use crate::pcb_editor::board_shape_ui::edge_name_rect;
    use egui::Rect;
    // A 300 × 200 point board on screen, y down.
    let [a, b, c, d] = [
        Pos2::new(100.0, 300.0),
        Pos2::new(400.0, 300.0),
        Pos2::new(400.0, 100.0),
        Pos2::new(100.0, 100.0),
    ];
    let centre = Pos2::new(250.0, 200.0);
    let size = egui::vec2(36.0, 12.0);
    let edges = [[a, b], [b, c], [c, d], [d, a]];
    let board = Rect::from_two_pos(a, c);
    let mut taken: Vec<Rect> = Vec::new();
    for (k, edge) in edges.iter().enumerate() {
        let r = edge_name_rect(edge, centre, size, 4.0, &taken);
        assert!(r.is_some(), "edge {k}");
        let Some(r) = r else { continue };
        let mid = edge[0] + (edge[1] - edge[0]) / 2.0;
        // Centred on the edge's middle along it, outside the board.
        let along = (edge[1] - edge[0]).normalized();
        assert!((r.center() - mid).dot(along).abs() < 0.5, "edge {k}: {r:?}");
        assert!(!board.shrink(0.5).intersects(r), "edge {k} outside: {r:?}");
        assert!(taken.iter().all(|o| !o.intersects(r)));
        taken.push(r);
    }
    // A dimension label over the right edge's middle pushes its name off.
    let label = Rect::from_center_size(Pos2::new(430.0, 200.0), egui::vec2(80.0, 16.0));
    let r = edge_name_rect(&[b, c], centre, size, 4.0, &[label]);
    assert!(r.is_some_and(|r| !r.intersects(label) && !board.shrink(0.5).intersects(r)));
}

// --- Free-form drawing tools on the engine ---------------------------------

/// The showcase board turned into its sketch on SolidCraft's board engine,
/// with the editor in the Board shape mode on it.
struct Drawn {
    engine: Engine,
    ctx: egui::Context,
    editor: PcbEditor,
    view: crate::pcb_editor::BoardView,
}

fn hosted() -> Fallible<sparksmith_core::Evaluators<'static>> {
    Ok(sparksmith_core::Evaluators {
        footprint: None,
        board: Some(host_evaluator().ok_or("no host board evaluator")?),
    })
}

impl Drawn {
    fn new() -> Fallible<Self> {
        let mut engine = Engine::showcase()?;
        engine.apply_hosted(
            engine.revision(),
            Operation::Board(BoardCommand::StartSketch),
            hosted()?,
        )?;
        let ctx = egui::Context::default();
        fonts::setup_bundled_fonts(&ctx);
        let mut editor = PcbEditor::default();
        assert!(editor.on_tool("board-shape", &engine).is_none());
        assert!(editor.shape.active);
        let (_, _, rect) = canvas(&mut editor, &engine, &ctx, vec![]);
        let view = board_view(
            rect,
            engine.project().board_size,
            1.0,
            egui::Vec2::ZERO,
            false,
        );
        Ok(Self {
            engine,
            ctx,
            editor,
            view,
        })
    }

    /// The point `fx`, `fy` of the way across the board, in mm.
    fn at(&self, fx: f64, fy: f64) -> Point {
        let [w, h] = self.engine.project().board_size;
        [w * fx, h * fy]
    }

    fn frame(&mut self, events: Vec<egui::Event>) -> Option<Operation> {
        canvas(&mut self.editor, &self.engine, &self.ctx, events).1
    }

    fn hover(&mut self, at: Point) -> Option<Operation> {
        let pos = self.view.screen(at);
        self.frame(vec![egui::Event::PointerMoved(pos)])
    }

    fn click(&mut self, at: Point) -> Option<Operation> {
        let pos = self.view.screen(at);
        let pressed = self.frame(press_at(pos));
        self.frame(release_at(pos)).or(pressed)
    }

    fn key(&mut self, key: egui::Key, shift: bool) -> Option<Operation> {
        let modifiers = if shift {
            egui::Modifiers::SHIFT
        } else {
            egui::Modifiers::NONE
        };
        self.frame(vec![egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }])
    }

    /// Apply a gesture's operation: one step on the engine.
    fn apply(&mut self, operation: Option<Operation>) -> Fallible<Vec<BoardEdit>> {
        let Some(op) = operation else {
            return Err("the gesture sent nothing".into());
        };
        let Operation::Board(BoardCommand::Edit { edits }) = op.clone() else {
            return Err(format!("not a board.edit: {op:?}").into());
        };
        let before = self.engine.revision();
        self.engine.apply_hosted(before, op, hosted()?)?;
        assert_eq!(self.engine.revision(), before + 1, "one step");
        Ok(edits)
    }

    fn board(&self) -> Fallible<BoardSketch> {
        match &self.engine.project().board_shape {
            BoardShape::Sketch(board) => Ok((**board).clone()),
            BoardShape::Rectangle => Err("the board is not a sketch".into()),
        }
    }
}

/// The same point, clicked: within the f32 screen position a pointer
/// carries (far under a pixel).
fn near(a: Point, b: Point) -> bool {
    (a[0] - b[0]).hypot(a[1] - b[1]) < 1e-4
}

/// The area inside a solved loop, mm².
fn ring_area(outline: &sparksmith_core::library::Outline) -> f64 {
    let ring = crate::pcb_editor::board_shape_ui::ring(outline);
    sparksmith_core::geometry::signed_area2(&ring).abs() / 2.0
}

fn distinct_curves(cutout: &sparksmith_core::model::board::SolvedCutout) -> usize {
    cutout
        .curves
        .iter()
        .collect::<std::collections::BTreeSet<_>>()
        .len()
}

/// The registry gives the free-form tools mnemonic keys of their own in
/// the Board shape mode, and Enter finishes a spline.
#[test]
fn the_free_form_tools_have_their_own_keys() {
    use crate::commands::{Chord, Context, Registry, active_contexts};
    use egui::Key as K;
    let registry = Registry::standard();
    let active = active_contexts(Context::BoardShape);
    let id = |chord| registry.resolve(chord, &active).map(|c| c.id.clone());
    assert_eq!(id(Chord::key(K::S)), Some("shape.bs-spline".into()));
    assert_eq!(
        id(Chord::key(K::C).with_shift()),
        Some("shape.bs-ellipse".into())
    );
    assert_eq!(
        id(Chord::key(K::A).with_shift()),
        Some("shape.bs-conic".into())
    );
    assert_eq!(id(Chord::key(K::Enter)), Some("shape.finish".into()));
    // The tools they generalise keep their keys.
    assert_eq!(id(Chord::key(K::C)), Some("shape.bs-circle".into()));
    assert_eq!(id(Chord::key(K::A)), Some("shape.bs-arc".into()));
}

/// Spline (S): fit points, then a click on the first point closes it
/// smoothly. The preview follows the pointer without a command; the
/// closing click is one `board.edit` (one undo step), and the closed
/// spline is a cutout of the solved board.
#[test]
fn a_spline_closed_on_its_first_point_is_one_step_and_a_cutout() -> Fallible {
    let mut d = Drawn::new()?;
    let start = d.at(0.3, 0.3);
    d.hover(start);
    assert!(d.key(egui::Key::S, false).is_none());
    assert_eq!(d.editor.shape.tool, ShapeTool::Spline);
    let points = [start, d.at(0.5, 0.25), d.at(0.6, 0.45), d.at(0.4, 0.55)];
    let revision = d.engine.revision();
    for p in points {
        assert!(d.click(p).is_none(), "a point commits nothing");
    }
    // The live preview: the spline through the points and the pointer,
    // from a scratch sketch; the document is untouched.
    let pointer = d.at(0.35, 0.5);
    assert!(d.hover(pointer).is_none());
    let curve = preview_curve(
        ShapeTool::Spline,
        &d.editor.shape.options,
        &d.editor.shape.gesture.pending,
        pointer,
        0.01,
    )
    .ok_or("no preview")?;
    let line = sparksmith_solidcraft::board::free_form_preview(&curve).ok_or("no line")?;
    for p in points.iter().chain([&pointer]) {
        let closest = line
            .iter()
            .map(|q| (q[0] - p[0]).hypot(q[1] - p[1]))
            .fold(f64::INFINITY, f64::min);
        assert!(closest < 0.05, "the preview passes {p:?}: {closest}");
    }
    assert_eq!(d.engine.revision(), revision);
    // Clicking the first point closes it.
    let operation = d.click(start);
    let edits = d.apply(operation)?;
    let (drawn, periodic, control) = edits
        .iter()
        .find_map(|e| match e {
            BoardEdit::AddSpline {
                points,
                periodic,
                control,
                ..
            } => Some((points.clone(), *periodic, *control)),
            _ => None,
        })
        .ok_or("no AddSpline")?;
    assert_eq!(drawn.len(), 4);
    assert!(drawn.iter().zip(points).all(|(a, b)| near(*a, b)));
    assert!(periodic && !control, "a closed fit-point spline");
    assert!(d.editor.shape.gesture.pending.is_empty());
    let board = d.board()?;
    assert!(board.fault.is_none(), "{:?}", board.fault);
    let [cutout] = board.solved.cutouts.as_slice() else {
        return Err(format!("{} cutouts", board.solved.cutouts.len()).into());
    };
    assert_eq!(distinct_curves(cutout), 1);
    assert!(ring_area(&cutout.outline) > 1.0);
    d.engine.undo()?;
    assert!(d.board()?.solved.cutouts.is_empty());
    Ok(())
}

/// An open spline ends on Enter or on a double-click on its last point;
/// its open ends are the board's fault, marked on the canvas, and the
/// board keeps its last shape. Backspace takes back a point; Esc drops
/// the spline.
#[test]
fn an_open_spline_ends_on_enter_or_a_double_click_and_shows_its_open_ends() -> Fallible {
    let mut d = Drawn::new()?;
    let (a, b, c) = (d.at(0.2, 0.3), d.at(0.4, 0.5), d.at(0.6, 0.35));
    d.hover(a);
    d.key(egui::Key::S, false);
    // Backspace takes back the last point; Esc drops them all.
    d.click(a);
    d.click(b);
    d.key(egui::Key::Backspace, false);
    assert_eq!(d.editor.shape.gesture.pending.len(), 1);
    d.key(egui::Key::Escape, false);
    assert!(d.editor.shape.gesture.pending.is_empty());
    assert_eq!(d.editor.shape.tool, ShapeTool::Spline);
    // Enter finishes the spline.
    for p in [a, b, c] {
        assert!(d.click(p).is_none());
    }
    assert_eq!(d.editor.shape.gesture.pending.len(), 3);
    let operation = d.key(egui::Key::Enter, false);
    assert!(
        d.editor.shape.gesture.pending.is_empty(),
        "Enter did not finish: {:?}",
        d.editor.shape.gesture.pending
    );
    let edits = d.apply(operation)?;
    assert!(edits.iter().any(|e| matches!(
        e,
        BoardEdit::AddSpline { points, periodic: false, .. } if points.len() == 3
    )));
    let board = d.board()?;
    let fault = board.fault.as_ref().ok_or("no fault for the open ends")?;
    assert_eq!(fault.kind, ShapeFaultKind::Open);
    for end in [a, c] {
        assert!(
            fault
                .at
                .iter()
                .any(|p| (p[0] - end[0]).hypot(p[1] - end[1]) < 0.01),
            "{end:?} in {:?}",
            fault.at
        );
    }
    assert!(board.solved.cutouts.is_empty(), "the last shape is kept");
    // The canvas marks both open ends.
    let (output, _, _) = canvas(&mut d.editor, &d.engine, &d.ctx, vec![]);
    fn circles(shape: &Shape, out: &mut Vec<(Pos2, f32)>) {
        match shape {
            Shape::Circle(c) => out.push((c.center, c.radius)),
            Shape::Vec(v) => v.iter().for_each(|s| circles(s, out)),
            _ => {}
        }
    }
    let mut found = Vec::new();
    for s in &output.shapes {
        circles(&s.shape, &mut found);
    }
    for end in [a, c] {
        let at = d.view.screen(end);
        assert!(
            found
                .iter()
                .any(|(p, r)| p.distance(at) < 0.5 && (*r - 6.0).abs() < 1e-3),
            "no open-end marker at {end:?}"
        );
    }
    d.engine.undo()?;
    // A double-click on the last point finishes it too.
    for p in [a, b] {
        assert!(d.click(p).is_none());
    }
    let operation = d.click(b);
    let edits = d.apply(operation)?;
    assert!(edits.iter().any(|e| matches!(
        e,
        BoardEdit::AddSpline { points, .. } if points.len() == 2
    )));
    Ok(())
}

/// Ellipse (Shift+C): the centre, the end of the major axis, then a point
/// on it. One `board.edit` draws the ellipse through that point, and the
/// closed ellipse is a cutout of its area.
#[test]
fn an_ellipse_gesture_is_one_step_and_a_cutout() -> Fallible {
    let mut d = Drawn::new()?;
    let [w, h] = d.engine.project().board_size;
    let r = (w.min(h) * 0.1).max(1.0);
    let centre = d.at(0.5, 0.5);
    let major = [centre[0] + 2.0 * r, centre[1]];
    let on = [centre[0], centre[1] + r];
    d.hover(centre);
    d.key(egui::Key::C, true);
    assert_eq!(d.editor.shape.tool, ShapeTool::Ellipse);
    assert!(d.click(centre).is_none());
    assert!(d.click(major).is_none());
    // The preview is the ellipse through the pointer.
    d.hover(on);
    let preview = preview_curve(
        ShapeTool::Ellipse,
        &d.editor.shape.options,
        &d.editor.shape.gesture.pending,
        on,
        0.01,
    );
    assert!(
        matches!(
            preview,
            Some(FreeFormCurve::Ellipse { minor_radius, .. }) if (minor_radius - r).abs() < 1e-4
        ),
        "{preview:?} for r = {r}, clicks {:?}",
        d.editor.shape.gesture.pending
    );
    let operation = d.click(on);
    let edits = d.apply(operation)?;
    let [
        BoardEdit::AddEllipse {
            center,
            major: end,
            minor_radius,
            construction: false,
        },
    ] = edits.as_slice()
    else {
        return Err(format!("not one ellipse: {edits:?}").into());
    };
    assert!(near(*center, centre) && near(*end, major));
    assert!((minor_radius - r).abs() < 1e-4);
    let board = d.board()?;
    assert!(board.fault.is_none(), "{:?}", board.fault);
    let [cutout] = board.solved.cutouts.as_slice() else {
        return Err("no ellipse cutout".into());
    };
    let expected = std::f64::consts::PI * 2.0 * r * r;
    let area = ring_area(&cutout.outline);
    assert!(
        (area - expected).abs() < expected * 0.01,
        "{area} vs {expected}"
    );
    Ok(())
}

/// Conic (Shift+A): from the start to the end, shaped by the apex. Its
/// ends snap to a line's ends and are tied to them in the same step, so
/// the line and the conic close a D-shaped cutout.
#[test]
fn a_conic_tied_to_a_line_closes_a_cutout_in_one_step() -> Fallible {
    let mut d = Drawn::new()?;
    let (a, b) = (d.at(0.4, 0.3), d.at(0.4, 0.6));
    let apex = d.at(0.6, 0.45);
    d.hover(a);
    d.key(egui::Key::L, false);
    d.click(a);
    let line = d.click(b);
    d.apply(line)?;
    d.key(egui::Key::Escape, false);
    d.key(egui::Key::A, true);
    assert_eq!(d.editor.shape.tool, ShapeTool::Conic);
    let revision = d.engine.revision();
    let coincident = |board: &BoardSketch| -> Result<usize> {
        Ok(scene_view(board)?
            .constraints
            .iter()
            .filter(|k| k.kind == "Coincident")
            .count())
    };
    let before = coincident(&d.board()?)?;
    assert!(d.click(b).is_none());
    assert!(d.click(a).is_none());
    d.hover(apex);
    assert_eq!(d.engine.revision(), revision, "the preview changes nothing");
    let operation = d.click(apex);
    let edits = d.apply(operation)?;
    assert!(edits.iter().any(|e| matches!(
        e,
        BoardEdit::AddConic { rho, .. } if (*rho - 0.5).abs() < 1e-12
    )));
    // The engine's AddConic ties the ends to the points they are drawn on;
    // the tool adds no second tie.
    assert!(!edits.iter().any(|e| matches!(
        e,
        BoardEdit::Sketch {
            edit: SketchEdit::AddConstraint {
                kind: GeomConstraint::Coincident,
                ..
            }
        }
    )));
    let board = d.board()?;
    assert_eq!(
        coincident(&board)?,
        before + 2,
        "both ends tied to the line"
    );
    assert!(board.fault.is_none(), "{:?}", board.fault);
    let [cutout] = board.solved.cutouts.as_slice() else {
        return Err("no D-shaped cutout".into());
    };
    assert_eq!(distinct_curves(cutout), 2);
    d.engine.undo()?;
    assert!(d.board()?.solved.cutouts.is_empty());
    Ok(())
}

/// Demo run item 8: the selected dimension's Value field marks an invalid
/// value; an undo, which reloads the sketch and its values, clears the mark.
#[test]
fn an_undo_clears_a_sketch_dimension_value_error() -> Fallible {
    let mut d = Drawn::new()?;
    let spare = BoardEdit::SetParameter {
        param: FootprintParam {
            name: ParamName("spare".into()),
            expr: Expr("1 mm".into()),
            unit: ParamUnit::Mm,
            comment: String::new(),
        },
    };
    d.apply(Some(operation(vec![spare])))?;
    let view = scene_view(&d.board()?)?;
    let width = view
        .constraints
        .iter()
        .find(|k| k.param.as_deref() == Some("width"))
        .map(|k| k.id.clone())
        .ok_or("no width dimension")?;
    d.editor.shape.dimension = Some(width);
    let red = |output: &egui::FullOutput| {
        fn walk(shape: &Shape, red: egui::Color32) -> bool {
            match shape {
                Shape::Vec(shapes) => shapes.iter().any(|s| walk(s, red)),
                Shape::Rect(r) => r.stroke.color == red && r.fill == egui::Color32::TRANSPARENT,
                _ => false,
            }
        }
        let red = sparksmith_ui::theme::Tokens::get().error;
        output.shapes.iter().any(|c| walk(&c.shape, red))
    };
    slot(&mut d.editor, &d.engine, &d.ctx, vec![]);
    let (output, _) = slot(&mut d.editor, &d.engine, &d.ctx, vec![]);
    // The Value row's field: the "width" text on the "Value" label's line.
    let shown = painted(&output);
    let row = shown
        .iter()
        .find(|(t, _)| t == "Value")
        .map(|(_, p)| p.y)
        .ok_or("no Value row")?;
    let field = shown
        .iter()
        .find(|(t, p)| t == "width" && (p.y - row).abs() < 4.0)
        .map(|(_, p)| *p)
        .ok_or("no Value field showing width")?;
    let mut events = press_at(field);
    events.extend(release_at(field));
    slot(&mut d.editor, &d.engine, &d.ctx, events);
    slot(
        &mut d.editor,
        &d.engine,
        &d.ctx,
        vec![
            egui::Event::Key {
                key: egui::Key::A,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::COMMAND,
            },
            egui::Event::Text("1 x = 2".into()),
        ],
    );
    let (_, sent) = slot(&mut d.editor, &d.engine, &d.ctx, key(egui::Key::Enter));
    assert!(sent.is_none(), "an invalid value was sent: {sent:?}");
    let (output, _) = slot(&mut d.editor, &d.engine, &d.ctx, vec![]);
    assert!(red(&output), "the invalid value is not marked");
    d.engine.undo()?;
    let (output, _) = slot(&mut d.editor, &d.engine, &d.ctx, vec![]);
    assert!(!red(&output), "the undone field stays marked");
    assert!(painted(&output).iter().any(|(t, _)| t == "width"));
    Ok(())
}

/// Demo run item 6: the shipped ESP32 example's board sketch is fully
/// defined; its mounting holes and its mounting slot sit `inset` from
/// their edges, so a width change carries the right-hand hole and the
/// slot along.
#[test]
fn the_esp32_examples_holes_follow_the_board_width() -> Fallible {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/esp32-s31-devboard/esp32-s31-devboard.sparksmith");
    let mut engine = Engine::from_file(sparksmith_io::read_project(&path)?)?;
    let BoardShape::Sketch(board) = engine.project().board_shape.clone() else {
        return Err("the example's board is not a sketch".into());
    };
    assert_eq!(board.solve.status, SolveStatus::Solved, "{:?}", board.solve);
    assert_eq!(board.solve.dof, 0);
    let width = FootprintParam {
        name: ParamName("width".into()),
        expr: Expr("50 mm".into()),
        unit: ParamUnit::Mm,
        comment: "Board width".into(),
    };
    engine.apply_hosted(
        engine.revision(),
        operation(vec![BoardEdit::SetParameter { param: width }]),
        hosted()?,
    )?;
    let mut xs: Vec<f64> = engine
        .project()
        .board()
        .holes
        .iter()
        .map(|h| h.at[0])
        .collect();
    xs.sort_by(f64::total_cmp);
    assert_eq!(xs.len(), 3);
    for (x, want) in xs.iter().zip([3.5, 3.5, 46.5]) {
        assert!((x - want).abs() < 1e-6, "{xs:?}");
    }
    // The slot's caps, 2.7 mm across and 1 mm apart, about 46.5 mm.
    let board = engine.project().board();
    let slot = board
        .cutouts
        .iter()
        .find(|c| {
            matches!(
                c.kind,
                sparksmith_core::model::board::CutoutKind::Slot { .. }
            )
        })
        .ok_or("the example's mounting slot")?;
    let xs: Vec<f64> = slot.outline.flatten(0.001).iter().map(|p| p[0]).collect();
    let (lo, hi) = (
        xs.iter().copied().fold(f64::INFINITY, f64::min),
        xs.iter().copied().fold(f64::NEG_INFINITY, f64::max),
    );
    assert!(
        (lo - 44.65).abs() < 1e-3 && (hi - 48.35).abs() < 1e-3,
        "the slot spans {lo} to {hi}"
    );
    Ok(())
}

/// Each of J1's Dimensions rows on the ESP32 example reads differently:
/// the horizontal and the vertical dimension between the same two
/// references name their axis ("Origin → U3 · x", "Origin → U3 · y").
#[test]
fn j1s_dimension_rows_name_their_axis() -> Fallible {
    use sparksmith_core::library::footprint_source::DimKind;
    use sparksmith_core::model::dimension::PartRelation;
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/esp32-s31-devboard/esp32-s31-devboard.sparksmith");
    let engine = Engine::from_file(sparksmith_io::read_project(&path)?)?;
    let project = engine.project();
    let j1 = project
        .instance_by_designator("J1")
        .map(|i| i.id)
        .ok_or("no J1")?;
    let rows: Vec<(PartRelation, String)> = edge_card::dimensions_of(project, j1)
        .into_iter()
        .map(|d| {
            (
                d.kind,
                edge_card::name_parts(project, None, j1, d, &|_| true).join(" → "),
            )
        })
        .collect();
    let axis = |kind: DimKind, end: &str| {
        rows.iter()
            .any(|(k, n)| *k == PartRelation::Dim(kind) && n.ends_with(end))
    };
    assert!(axis(DimKind::HorizontalDistance, " · x"), "{rows:?}");
    assert!(axis(DimKind::VerticalDistance, " · y"), "{rows:?}");
    let mut names: Vec<&str> = rows.iter().map(|(_, n)| n.as_str()).collect();
    names.sort_unstable();
    let count = names.len();
    names.dedup();
    assert_eq!(names.len(), count, "{rows:?}");
    Ok(())
}

/// The sketch with the named edge's curve deleted, as Delete sends it.
fn opened(d: &mut Drawn, edge: &str) -> Fallible {
    let board = d.board()?;
    let curve = board
        .edges
        .iter()
        .find(|e| e.name.0 == edge)
        .map(|e| SketchRef::Curve(e.curve.clone()))
        .ok_or("no such edge")?;
    d.apply(Some(operation(delete_edits(&board, &[curve]))))?;
    let fault = d.board()?.fault.ok_or("the outline is still closed")?;
    assert_eq!(fault.kind, ShapeFaultKind::Open);
    Ok(())
}

/// GUI check 1, K5: a line drawn back to the corner at the origin snaps to
/// it and closes the outline.
#[test]
fn a_line_to_the_origin_corner_closes_the_outline() -> Fallible {
    let mut d = Drawn::new()?;
    opened(&mut d, "left")?;
    let top = d.at(0.0, 1.0);
    d.hover(top);
    d.key(egui::Key::L, false);
    assert_eq!(d.editor.shape.tool, ShapeTool::Line);
    d.click([top[0] - 0.05, top[1] + 0.05]);
    // A hand's click, beside the corner.
    let line = d.click([-0.05, 0.04]);
    d.apply(line)?;
    let board = d.board()?;
    assert_eq!(board.fault, None);
    assert!(board.current);
    Ok(())
}

/// GUI check 1, K6: an arc whose ends are clicked on existing points ends
/// on both, wherever its centre was clicked, and closes the outline.
#[test]
fn an_arc_between_two_corners_closes_the_outline() -> Fallible {
    let mut d = Drawn::new()?;
    opened(&mut d, "right")?;
    let (low, high) = (d.at(1.0, 0.0), d.at(1.0, 1.0));
    d.hover(low);
    d.key(egui::Key::A, false);
    assert_eq!(d.editor.shape.tool, ShapeTool::Arc);
    // Not as far from the one corner as from the other.
    let mut centre = d.at(0.9, 0.5);
    centre[1] += 0.4;
    d.click(centre);
    d.click([low[0] + 0.05, low[1] - 0.05]);
    let arc = d.click([high[0] + 0.05, high[1] + 0.05]);
    d.apply(arc)?;
    let board = d.board()?;
    assert_eq!(board.fault, None);
    assert!(board.current);
    // The corners stayed where they were.
    let view = scene_view(&board)?;
    for corner in [low, high] {
        assert!(
            view.points.iter().any(|p| dist(p.at, corner) < 1e-6),
            "{corner:?}: {:?}",
            view.points
        );
    }
    Ok(())
}

/// With a free start, the end's point sets the radius and the start goes
/// onto that circle.
#[test]
fn an_arcs_free_start_gives_way_to_its_tied_end() -> Fallible {
    let at = |p: Point| Click {
        at: p,
        snapped: None,
    };
    let end = Click {
        at: [4.0, 5.0],
        snapped: Some(SketchPointId("p9".into())),
    };
    let (c, s, e) = arc_points(&at([0.0, 5.0]), &at([0.0, 0.0]), &end).ok_or("an arc")?;
    assert!(dist(c, [0.0, 5.0]) < 1e-12 && dist(e, end.at) < 1e-12);
    assert!(dist(s, [0.0, 1.0]) < 1e-12, "{s:?}");
    Ok(())
}

/// `d` on the same board with every fill unfilled, as a file opens.
fn unfilled() -> Fallible<Drawn> {
    let mut d = Drawn::new()?;
    let mut file = d.engine.to_file();
    file.project
        .fills
        .iter_mut()
        .for_each(|f| f.computed = None);
    d.engine = Engine::from_file(file)?;
    d.frame(vec![]);
    assert!(d.editor.shape.active);
    Ok(d)
}

/// The fills' copper is stored as the background worker stores it.
fn store(d: &mut Drawn) -> Fallible {
    let revision = d.engine.revision();
    assert!(d.engine.refill_stale()? > 0, "no fill was stale");
    assert_eq!(d.engine.revision(), revision + 1);
    Ok(())
}

/// Text typed over a sketch dimension's value outlives a fill store.
#[test]
fn a_typed_sketch_dimension_outlives_a_fill_store() -> Fallible {
    let mut d = unfilled()?;
    let view = scene_view(&d.board()?)?;
    let width = view
        .constraints
        .iter()
        .find(|k| k.param.as_deref() == Some("width"))
        .map(|k| k.id.clone())
        .ok_or("no width dimension")?;
    d.editor.shape.dimension = Some(width);
    slot(&mut d.editor, &d.engine, &d.ctx, vec![]);
    let (output, _) = slot(&mut d.editor, &d.engine, &d.ctx, vec![]);
    let shown = painted(&output);
    let row = shown
        .iter()
        .find(|(t, _)| t == "Value")
        .map(|(_, p)| p.y)
        .ok_or("no Value row")?;
    let field = shown
        .iter()
        .find(|(t, p)| t == "width" && (p.y - row).abs() < 4.0)
        .map(|(_, p)| *p)
        .ok_or("no Value field showing width")?;
    let mut events = press_at(field);
    events.extend(release_at(field));
    slot(&mut d.editor, &d.engine, &d.ctx, events);
    let mut typed = key(egui::Key::A);
    if let Some(egui::Event::Key { modifiers, .. }) = typed.first_mut() {
        *modifiers = egui::Modifiers::COMMAND;
    }
    typed.push(egui::Event::Text("width = 48 mm".into()));
    slot(&mut d.editor, &d.engine, &d.ctx, typed);
    store(&mut d)?;
    let (output, _) = slot(&mut d.editor, &d.engine, &d.ctx, vec![]);
    assert!(
        painted(&output).iter().any(|(t, _)| t == "width = 48 mm"),
        "{:?}",
        painted(&output)
    );
    let (_, sent) = slot(&mut d.editor, &d.engine, &d.ctx, key(egui::Key::Enter));
    let Some(Operation::Board(BoardCommand::Edit { edits })) = sent else {
        return Err(format!("nothing was sent: {sent:?}").into());
    };
    assert!(parameters_of(&edits).iter().any(|p| p.expr.0 == "48 mm"));
    Ok(())
}

/// A sketch point being dragged stays in hand through a fill store.
#[test]
fn a_sketch_point_drag_outlives_a_fill_store() -> Fallible {
    let mut d = unfilled()?;
    let at = d.at(0.3, 0.3);
    // No fill depends on a free point: it makes none fresh or stale.
    d.apply(Some(operation(vec![BoardEdit::Sketch {
        edit: SketchEdit::AddPoint { at },
    }])))?;
    d.hover(at);
    let screen = |d: &Drawn, p: Point| d.view.screen(p);
    let pos = screen(&d, at);
    d.frame(press_at(pos));
    for mm in [0.5, 1.0, 1.5] {
        d.hover([at[0] + mm, at[1]]);
    }
    store(&mut d)?;
    let to = [at[0] + 2.0, at[1]];
    d.hover(to);
    let pos = screen(&d, to);
    let Some(op) = d.frame(release_at(pos)) else {
        return Err("the drag was dropped".into());
    };
    d.apply(Some(op))?;
    let view = scene_view(&d.board()?)?;
    assert!(
        view.points.iter().any(|p| dist(p.at, to) < 0.2),
        "{:?}",
        view.points
    );
    Ok(())
}

/// Every constraint tool has its own key in the registry, as Constraints
/// has, and the card shows the registry's key.
#[test]
fn the_constraint_tools_keys_come_from_the_registry() -> Fallible {
    use crate::commands::{Context, Registry, active_contexts};
    let registry = Registry::standard();
    let active = active_contexts(Context::BoardShape);
    let constraints = ShapeTool::ALL
        .into_iter()
        .filter(|t| matches!(t, ShapeTool::Constrain(_)));
    let mut seen = std::collections::BTreeSet::new();
    for id in constraints.map(ShapeTool::id).chain([CONSTRAINTS_TOOL]) {
        let command = registry.get_parts("shape", id).ok_or(id)?;
        let chord = registry.chord(command).ok_or(id)?;
        let found = registry.resolve(chord, &active).map(|c| c.id.clone());
        assert_eq!(found, Some(format!("shape.{id}")));
        assert!(seen.insert(format!("{chord:?}")), "{id}");
    }
    assert_eq!(seen.len(), 13);
    let mut d = Drawn::new()?;
    d.editor
        .shape
        .arm(ShapeTool::Constrain(GeomConstraint::Vertical));
    let (output, _) = slot(&mut d.editor, &d.engine, &d.ctx, vec![]);
    let command = registry.get("shape.bs-vertical").ok_or("no command")?;
    let key = registry.shortcut(command).ok_or("no key")?;
    assert!(painted(&output).iter().any(|(t, _)| *t == key), "{key}");
    Ok(())
}

/// A construction line of the board sketch and its curve.
fn guide(d: &mut Drawn, a: Point, b: Point) -> Fallible<SketchCurveId> {
    d.apply(Some(operation(vec![BoardEdit::Sketch {
        edit: SketchEdit::AddLine {
            a,
            b,
            construction: true,
        },
    }])))?;
    let view = scene_view(&d.board()?)?;
    Ok(view.curves.last().ok_or("no curve")?.id.clone())
}

fn middle(d: &Drawn, curve: &SketchCurveId) -> Fallible<Point> {
    let view = scene_view(&d.board()?)?;
    match view.curve(curve).map(|c| c.geometry.clone()) {
        Some(ViewGeometry::Line { a, b }) => Ok(scale(add(a, b), 0.5)),
        other => Err(format!("{other:?}").into()),
    }
}

/// Nothing is drawn until something is pointed at; then the glyph of what it
/// is under can be clicked, and Delete removes the constraint in one step.
#[test]
fn a_constraints_glyph_shows_on_hover_and_deletes_it() -> Fallible {
    let mut d = Drawn::new()?;
    let (a, b) = (d.at(0.3, 0.3), d.at(0.6, 0.33));
    let line = guide(&mut d, a, b)?;
    let at = middle(&d, &line)?;
    d.hover(at);
    assert!(d.key(egui::Key::Z, false).is_none());
    let op = d.click(at);
    let edits = d.apply(op)?;
    assert!(
        matches!(
            edits.as_slice(),
            [BoardEdit::Sketch {
                edit: SketchEdit::AddConstraint {
                    kind: GeomConstraint::Horizontal,
                    ..
                }
            }]
        ),
        "{edits:?}"
    );
    let added = d.board()?;
    d.key(egui::Key::Escape, false);
    assert_eq!(d.editor.shape.tool, ShapeTool::Select);
    // Off the sketch, no glyph; G shows them all and back.
    let away = d.at(0.45, 0.6);
    for _ in 0..2 {
        d.hover(away);
    }
    assert!(d.editor.shape.glyphs.is_empty());
    d.key(egui::Key::G, false);
    d.hover(away);
    assert!(!d.editor.shape.glyphs.is_empty(), "all of them");
    d.key(egui::Key::G, false);
    for _ in 0..2 {
        d.hover(away);
    }
    assert!(d.editor.shape.glyphs.is_empty());
    let at = middle(&d, &line)?;
    for _ in 0..2 {
        d.hover(at);
    }
    let view = scene_view(&added)?;
    let id = view
        .constraints
        .iter()
        .find(|k| k.kind == "Horizontal" && k.curves.contains(&line))
        .map(|k| k.id.clone())
        .ok_or("no constraint")?;
    let glyph = |d: &Drawn| {
        let found = d.editor.shape.glyphs.iter().find(|(k, _)| *k == id);
        found.map(|(_, r)| r.center())
    };
    let pos = glyph(&d).ok_or("no glyph")?;
    d.frame(vec![egui::Event::PointerMoved(pos)]);
    let pos = glyph(&d).ok_or("gone before the pointer reached it")?;
    d.frame(press_at(pos));
    d.frame(release_at(pos));
    assert_eq!(
        d.editor.shape.selection,
        vec![SketchRef::Constraint(id.clone())]
    );
    assert!(d.editor.can_delete(d.engine.project()));
    let op = d.key(egui::Key::Delete, false);
    d.apply(op)?;
    let left = scene_view(&d.board()?)?;
    assert!(!left.constraints.iter().any(|k| k.id == id));
    assert!(left.curve(&line).is_some(), "the line stays");
    d.engine.undo()?;
    assert_eq!(d.board()?, added);
    Ok(())
}

/// A point dropped on another point joins it: one step, one place, and
/// moving one moves both.
#[test]
fn a_point_dropped_on_another_joins_it() -> Fallible {
    let mut d = Drawn::new()?;
    let (p, q) = (d.at(0.3, 0.3), d.at(0.5, 0.45));
    let (a, b) = (d.at(0.3, 0.6), d.at(0.6, 0.7));
    guide(&mut d, p, a)?;
    guide(&mut d, q, b)?;
    let near_q = [q[0] + 0.05, q[1]];
    let pos = d.view.screen(p);
    d.hover(p);
    d.frame(press_at(pos));
    for f in [0.3, 0.7, 1.0] {
        d.hover([p[0] + (near_q[0] - p[0]) * f, p[1] + (near_q[1] - p[1]) * f]);
    }
    let pos = d.view.screen(near_q);
    let op = d.frame(release_at(pos));
    let edits = d.apply(op)?;
    assert!(
        matches!(
            edits.as_slice(),
            [
                BoardEdit::Sketch {
                    edit: SketchEdit::Drag { .. }
                },
                BoardEdit::Sketch {
                    edit: SketchEdit::AddConstraint {
                        kind: GeomConstraint::Coincident,
                        ..
                    }
                },
            ]
        ),
        "{edits:?}"
    );
    let view = scene_view(&d.board()?)?;
    let there = view.points.iter().filter(|p| dist(p.at, q) < 1e-3);
    assert_eq!(there.count(), 2);
    Ok(())
}

/// Drags the point at `from` to `to` and releases it: where the last frame
/// showed every point is where the document has it, within a nanometre.
/// Returns the landed sketch.
fn dragged_as_shown(d: &mut Drawn, from: Point, to: Point) -> Fallible<SketchView> {
    d.hover(from);
    let pos = d.view.screen(from);
    d.frame(press_at(pos));
    for f in [0.2, 0.5, 0.8, 1.0] {
        d.hover([
            from[0] + (to[0] - from[0]) * f,
            from[1] + (to[1] - from[1]) * f,
        ]);
    }
    let shown = d.editor.shape.dragged().ok_or("nothing is dragged")?;
    let pos = d.view.screen(to);
    let op = d.frame(release_at(pos));
    d.apply(op)?;
    let landed = scene_view(&d.board()?)?;
    for p in &shown.points {
        let at = landed.point(&p.id).ok_or("a point is gone")?.at;
        assert!(dist(p.at, at) <= 1e-6, "{:?}: {:?} {at:?}", p.id, p.at);
    }
    Ok(landed)
}

/// `Drawn`, seen from the top or the bottom.
fn seen(from_bottom: bool) -> Fallible<Drawn> {
    let mut d = Drawn::new()?;
    d.editor.from_bottom = from_bottom;
    let (_, _, rect) = canvas(&mut d.editor, &d.engine, &d.ctx, vec![]);
    d.view = board_view(
        rect,
        d.engine.project().board_size,
        1.0,
        egui::Vec2::ZERO,
        from_bottom,
    );
    Ok(d)
}

/// A cutout's corner, square or rounded, and the outline's: each lands
/// where it was shown, from either side. (A corner of a rectangle went a
/// quarter of the way.)
#[test]
fn a_dragged_corner_lands_where_it_is_shown() -> Fallible {
    for from_bottom in [false, true] {
        for radius in [0.0, 1.0] {
            let mut d = seen(from_bottom)?;
            let (a, b) = (d.at(0.3, 0.3), d.at(0.5, 0.5));
            let board = d.board()?;
            let mut stage = Stage::new(host_evaluator().ok_or("no evaluator")?, &board.sketch);
            let curves = rectangle(&mut stage, a, b, radius, None, false)?;
            let edits = stage.edits.clone();
            d.apply(Some(operation(edits)))?;
            let view = scene_view(&d.board()?)?;
            let from = view
                .points
                .iter()
                .filter(|p| {
                    curves
                        .iter()
                        .filter_map(|c| view.curve(c))
                        .any(|c| c.points.contains(&p.id))
                })
                .map(|p| p.at)
                .max_by(|p, q| (p[0] + p[1]).total_cmp(&(q[0] + q[1])))
                .ok_or("no corner")?;
            let to = [from[0] + 2.3, from[1] + 1.7];
            let landed = dragged_as_shown(&mut d, from, to)?;
            if radius == 0.0 {
                let there = landed.points.iter().filter(|p| dist(p.at, to) < 0.02);
                assert_eq!(there.count(), 2, "both ends at the pointer");
            }
        }
        let mut d = seen(from_bottom)?;
        let set: Vec<SketchRef> = scene_view(&d.board()?)?
            .constraints
            .iter()
            .filter(|k| k.value.is_some())
            .map(|k| SketchRef::Constraint(k.id.clone()))
            .collect();
        d.apply(Some(operation(vec![BoardEdit::Sketch {
            edit: SketchEdit::Delete { ids: set },
        }])))?;
        let from = d.at(1.0, 1.0);
        let to = [from[0] + 1.5, from[1] + 2.0];
        let landed = dragged_as_shown(&mut d, from, to)?;
        assert!(landed.points.iter().any(|p| dist(p.at, to) < 0.02));
    }
    Ok(())
}

/// A point over another shows the join its release makes.
#[test]
fn a_point_dropped_on_another_is_shown_joined() -> Fallible {
    let mut d = Drawn::new()?;
    let (p, q) = (d.at(0.3, 0.3), d.at(0.5, 0.45));
    let (a, b) = (d.at(0.3, 0.6), d.at(0.6, 0.7));
    guide(&mut d, p, a)?;
    guide(&mut d, q, b)?;
    let landed = dragged_as_shown(&mut d, p, [q[0] + 0.05, q[1]])?;
    let there = landed.points.iter().filter(|p| dist(p.at, q) < 1e-3);
    assert_eq!(there.count(), 2);
    Ok(())
}

/// A line's own ends, and ends joined already, are no place to join.
#[test]
fn a_line_is_not_closed_up_by_dropping_its_ends_together() -> Fallible {
    let mut d = Drawn::new()?;
    let (a, b) = (d.at(0.3, 0.3), d.at(0.6, 0.33));
    let line = guide(&mut d, a, b)?;
    let view = scene_view(&d.board()?)?;
    let points = view.curve(&line).ok_or("no line")?.points.clone();
    assert!(join_target(&view, &view, &points[0], b, 0.5).is_none());
    assert_eq!(drop_edits(&view, &view, &points[0], b, b, 0.5).len(), 1);
    Ok(())
}

/// Deleting a constraint names the parts dimensioned to what it held.
#[test]
fn held_by_lists_the_constraints_points_and_curves() -> Fallible {
    let board = board_of([40.0, 20.0], Vec::new())?;
    let view = scene_view(&board)?;
    let k = view
        .constraints
        .iter()
        .find(|k| k.kind == "Horizontal")
        .ok_or("no Horizontal")?;
    let held = held_by(&view, &[SketchRef::Constraint(k.id.clone())]);
    let curve = k.curves.first().ok_or("no curve")?;
    assert!(held.contains(&SketchRef::Curve(curve.clone())));
    let joined = view
        .constraints
        .iter()
        .find(|k| k.kind == "Coincident")
        .ok_or("no Coincident")?;
    let held = held_by(&view, &[SketchRef::Constraint(joined.id.clone())]);
    let curves = held
        .iter()
        .filter(|r| matches!(r, SketchRef::Curve(_)))
        .count();
    assert_eq!(curves, 2, "both sides of the corner: {held:?}");
    Ok(())
}

/// Free-form curves are picked, and a constraint's picks take them where
/// the engine does.
#[test]
fn constraint_picks_take_free_form_curves() -> Fallible {
    use crate::part_editor::footprint_dims::{ConstrainPicks, constrain_picks, pick};
    use sparksmith_solidcraft::footprint::view::{ViewCurve, ViewPoint};
    let curve = |id: &str, geometry| ViewCurve {
        id: SketchCurveId(id.into()),
        geometry,
        construction: false,
        points: Vec::new(),
    };
    let points = vec![[0.0, 10.0], [5.0, 12.0], [10.0, 10.0]];
    let view = SketchView {
        points: vec![ViewPoint {
            id: SketchPointId("p".into()),
            at: [2.0, 2.0],
            fixed: false,
        }],
        curves: vec![
            curve("free", ViewGeometry::FreeForm { points }),
            curve(
                "line",
                ViewGeometry::Line {
                    a: [0.0, 0.0],
                    b: [10.0, 0.0],
                },
            ),
        ],
        ..Default::default()
    };
    let free = SketchRef::Curve(SketchCurveId("free".into()));
    let line = SketchRef::Curve(SketchCurveId("line".into()));
    let point = SketchRef::Point(SketchPointId("p".into()));
    assert_eq!(pick(&view, [5.0, 12.1], 0.5), Some(free.clone()));
    let picks = |kind, refs: &[&SketchRef]| {
        let refs: Vec<SketchRef> = refs.iter().map(|r| (*r).clone()).collect();
        constrain_picks(&view, kind, &refs)
    };
    use GeomConstraint as G;
    assert_eq!(picks(G::Tangent, &[&line, &free]), ConstrainPicks::Ready);
    assert_eq!(
        picks(G::PointOnCurve, &[&point, &free]),
        ConstrainPicks::Ready
    );
    assert!(matches!(
        picks(G::Equal, &[&free]),
        ConstrainPicks::Wrong(_)
    ));
    assert!(matches!(
        picks(G::Tangent, &[&line, &line]),
        ConstrainPicks::Wrong(_)
    ));
    Ok(())
}

/// GUI run 2, R2-A1: the very constraint that is there already is refused
/// by its name.
#[test]
fn a_constraint_there_already_is_refused_by_name() -> Fallible {
    let mut d = Drawn::new()?;
    let (a, b) = (d.at(0.3, 0.3), d.at(0.6, 0.33));
    let line = guide(&mut d, a, b)?;
    let at = middle(&d, &line)?;
    let tool = ShapeTool::Constrain(GeomConstraint::Horizontal);
    d.editor.shape.arm(tool);
    d.hover(at);
    let op = d.click(at);
    d.apply(op)?;
    let board = d.board()?;
    let view = scene_view(&board)?;
    let scene = Scene {
        board: &board,
        view: &view,
        evaluator: host_evaluator().ok_or("no evaluator")?,
        tolerance: 0.5,
    };
    let at = middle(&d, &line)?;
    let options = ShapeOptions::default();
    let Err(why) = press(tool, &options, &mut Gesture::default(), at, &scene) else {
        return Err("taken".into());
    };
    let why = refusal::shown_error(&why);
    assert!(why.starts_with("Horizontal already sets this"), "{why}");
    Ok(())
}

/// A new parameter's name and expression stay in the add row until the
/// board has it (a refusal then shows under them).
#[test]
fn a_new_parameter_keeps_its_text_until_the_board_has_it() -> Fallible {
    let mut d = Drawn::new()?;
    let params = &mut d.editor.shape.params;
    params.adding = true;
    params.new_name = "gap".into();
    params.new_expr = "2 mm".into();
    slot(&mut d.editor, &d.engine, &d.ctx, vec![]);
    let op = click_text(&mut d.editor, &d.engine, &d.ctx, "Add")?;
    let params = &d.editor.shape.params;
    assert!(params.adding, "sent, not answered");
    assert_eq!((&*params.new_name, &*params.new_expr), ("gap", "2 mm"));
    d.apply(op)?;
    slot(&mut d.editor, &d.engine, &d.ctx, vec![]);
    let params = &d.editor.shape.params;
    assert!(!params.adding && params.new_name.is_empty());
    Ok(())
}

/// GUI check 2, N20: a parameter stored as a bare number shows its unit.
#[test]
fn a_bare_numbers_parameter_shows_its_unit() -> Fallible {
    let mut d = Drawn::new()?;
    let units = |d: &mut Drawn| {
        let (output, _) = slot(&mut d.editor, &d.engine, &d.ctx, vec![]);
        let shown = painted(&output);
        shown.iter().filter(|(t, _)| t == "mm").count()
    };
    let before = units(&mut d);
    d.apply(Some(operation(vec![BoardEdit::SetParameter {
        param: FootprintParam {
            name: ParamName("hole_in".into()),
            expr: Expr("3.2".into()),
            unit: ParamUnit::Mm,
            comment: String::new(),
        },
    }])))?;
    assert_eq!(units(&mut d), before + 1);
    Ok(())
}
