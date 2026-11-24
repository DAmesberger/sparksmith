//! Fill copper computed in the background and stored while the person is
//! typing or dragging: it is no change of theirs, so nothing they hold is
//! lost. The copper is handed over exactly between two of their inputs
//! (`FillWorker::held`), never waited for.
use super::ax_harness::{Ax, Fallible};
use super::*;
use crate::fill_worker::{FillWorker, Held};
use egui::{Key, Modifiers, Role};
use sparksmith_core::BoardShape;
use sparksmith_core::library::footprint_source::DimValue;

/// The app on `engine`'s PCB in a tall window, with a fill's copper out (as
/// an edit of it leaves it) and the worker's results in the test's hand.
fn held(engine: Engine) -> Fallible<(Ax, Held)> {
    use sparksmith_core::layout::fill::FillCommand;
    let mut ax = Ax::with_engine(engine, Workspace::Pcb)?;
    ax.window = egui::vec2(1380.0, 1400.0);
    let (worker, held) = FillWorker::held();
    ax.app.fills = worker;
    let mut fill = ax
        .app
        .engine
        .project()
        .fills
        .iter()
        .find(|f| !f.locked)
        .cloned()
        .ok_or("no fill")?;
    fill.name = "Held back".into();
    ax.app.engine.apply_hosted(
        ax.app.engine.revision(),
        Operation::Fill(FillCommand::Edit {
            fill: Box::new(fill),
        }),
        sparksmith_core::Evaluators {
            footprint: None,
            board: crate::pcb_editor::board_shape::host_evaluator(),
        },
    )?;
    ax.settle();
    Ok((ax, held))
}

/// The copper is ready and stored now: one revision, no step.
fn store(ax: &mut Ax, held: &Held) -> Fallible {
    let before = ax.app.engine.revision();
    let content = ax.app.engine.content_revision();
    let step = ax.app.engine.undo_step();
    assert!(held.release(&ax.app.engine)? > 0, "no fill was stale");
    ax.frame(vec![]);
    ax.frame(vec![]);
    assert_eq!(
        ax.app.engine.revision(),
        before + 1,
        "the copper was stored"
    );
    assert_eq!(ax.app.engine.content_revision(), content);
    assert_eq!(
        ax.app.engine.undo_step().map(|s| s.line()),
        step.map(|s| s.line())
    );
    Ok(())
}

fn sketched() -> Fallible<Engine> {
    let mut engine = Engine::showcase()?;
    engine.apply_hosted(
        engine.revision(),
        Operation::Board(sparksmith_core::layout::outline::BoardCommand::StartSketch),
        sparksmith_core::Evaluators {
            footprint: None,
            board: crate::pcb_editor::board_shape::host_evaluator(),
        },
    )?;
    Ok(engine)
}

/// A value typed in a board parameter's row outlives the store and commits.
#[test]
fn a_typed_board_parameter_outlives_a_fill_store() -> Fallible {
    let (mut ax, held) = held(sketched()?)?;
    let row = ax.find_prefix(Role::ListBoxOption, "width")?;
    ax.click_at(row.center());
    if !ax.ctx.text_edit_focused() {
        ax.key(Key::Enter);
    }
    ax.key_with(Key::A, Modifiers::COMMAND);
    let wider = format!("{} mm", ax.app.engine.project().board_size[0] + 2.0);
    ax.type_text(&wider);
    store(&mut ax, &held)?;
    assert!(ax.ctx.text_edit_focused(), "the field kept the keyboard");
    assert!(ax.texts.contains(&wider), "{:?}", ax.texts);
    ax.key(Key::Enter);
    ax.settle();
    let BoardShape::Sketch(board) = &ax.app.engine.project().board_shape else {
        return Err("not a sketch".into());
    };
    let stored = board.parameters.iter().find(|p| p.name.0 == "width");
    assert_eq!(stored.map(|p| p.expr.0.as_str()), Some(wider.as_str()));
    Ok(())
}

/// A value typed in a part dimension's row outlives the store and commits.
#[test]
fn a_typed_part_dimension_outlives_a_fill_store() -> Fallible {
    let _context = sparksmith_core::library::standards::Context::fixture().map(|c| c.enter());
    const FACE: &str = "Front face \u{2192} bottom edge";
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/esp32-s31-devboard/esp32-s31-devboard.sparksmith");
    let engine = Engine::from_file(sparksmith_io::read_project(&path)?)?;
    let j1 = engine
        .project()
        .instance_by_designator("J1")
        .map(|i| i.id)
        .ok_or("no J1")?;
    let (mut ax, held) = held(engine)?;
    ax.app.pcb.set_selection(Some(j1), None, None);
    ax.settle();
    let row = ax.find(Role::ListBoxOption, FACE)?;
    ax.click_at(egui::pos2(row.rect.left() + 30.0, row.rect.center().y));
    ax.key(Key::Enter);
    ax.settle();
    ax.find(Role::TextInput, FACE)?;
    ax.type_text("1 mm");
    store(&mut ax, &held)?;
    ax.find(Role::TextInput, FACE)?;
    ax.key(Key::Enter);
    ax.settle();
    let committed = ax.app.engine.project().part_dimensions.iter().any(|d| {
        d.value == Some(DimValue::Fixed(1.0))
            && d.refs.iter().any(|r| {
                matches!(
                    r,
                    sparksmith_core::model::dimension::BoardRef::Part(f)
                        if f.target()
                            == sparksmith_core::model::board::ConstraintTarget::Placement(j1)
                )
            })
    });
    assert!(committed, "{}", ax.dump());
    Ok(())
}

/// A part being dragged stays in hand through the store and lands where it
/// is let go.
#[test]
fn a_part_drag_outlives_a_fill_store() -> Fallible {
    let (mut ax, held) = held(Engine::showcase()?)?;
    let r1 = ax
        .app
        .engine
        .project()
        .instance_by_designator("R1")
        .map(|i| i.id)
        .ok_or("no R1")?;
    let place = |ax: &Ax| {
        let p = ax.app.engine.project().placement_of(r1);
        p.map(|p| p.position).ok_or("R1 is not placed")
    };
    let start = place(&ax)?;
    let screen = |ax: &Ax, at: [f64; 2]| ax.app.pcb.layout_to_screen(at).ok_or("no canvas");
    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    let from = screen(&ax, start)?;
    ax.hover(from);
    ax.frame(vec![button(from, true)]);
    for mm in [0.5, 1.0, 1.5] {
        let at = screen(&ax, [start[0], start[1] + mm])?;
        ax.hover(at);
    }
    store(&mut ax, &held)?;
    let to = screen(&ax, [start[0], start[1] + 2.0])?;
    ax.hover(to);
    ax.hover(to);
    ax.frame(vec![button(to, false)]);
    ax.settle();
    let end = place(&ax)?;
    assert!(end != start, "the drag was dropped: {:?}", ax.texts);
    assert!(
        (end[1] - start[1] - 2.0).abs() < 0.6,
        "{start:?} -> {end:?}"
    );
    Ok(())
}
