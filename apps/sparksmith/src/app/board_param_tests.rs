//! Board parameter and dimension fields through the whole app's frame and
//! its accessibility tree: what Enter, Esc, ⌘Z and a refusal do there.
use super::ax_harness::{Ax, Fallible};
use super::*;
use egui::{Key, Modifiers, Role};
use sparksmith_core::BoardShape;

/// The showcase board as a sketch, on Layout's Board card. Its fills are
/// poured for this shape already: the app's fill worker would store them
/// later, as a revision of its own at no set frame (`Engine::store_fills`).
/// It does after each later sketch edit, so these tests compare the sketch,
/// never the revision.
fn sketched() -> Fallible<Ax> {
    let mut engine = Engine::showcase()?;
    engine.apply_hosted(
        engine.revision(),
        Operation::Board(sparksmith_core::layout::outline::BoardCommand::StartSketch),
        sparksmith_core::Evaluators {
            footprint: None,
            board: crate::pcb_editor::board_shape::host_evaluator(),
        },
    )?;
    engine.refill_stale()?;
    Ax::with_engine(engine, Workspace::Pcb)
}

fn width(ax: &Ax) -> Fallible<String> {
    let BoardShape::Sketch(board) = &ax.app.engine.project().board_shape else {
        return Err("the board is not a sketch".into());
    };
    let p = board.parameters.iter().find(|p| p.name.0 == "width");
    Ok(p.ok_or("no width")?.expr.0.clone())
}

/// Open the `name` row's value and type `text` over it.
fn retype(ax: &mut Ax, name: &str, text: &str) -> Fallible {
    let row = ax.find_prefix(Role::ListBoxOption, name)?;
    ax.click_at(row.center());
    if !ax.ctx.text_edit_focused() {
        ax.key(Key::Enter);
    }
    assert!(
        ax.ctx.text_edit_focused(),
        "the value takes the keyboard: {row:?} {:?}",
        ax.seen
    );
    ax.key_with(Key::A, Modifiers::COMMAND);
    ax.type_text(text);
    Ok(())
}

/// GUI check 1, K4: ⌘Z while a parameter's value is typed stays the
/// field's; after Enter committed it (and after Esc), the list row that has
/// the keyboard leaves it to the project.
#[test]
fn command_z_undoes_the_project_once_a_parameter_row_committed() -> Fallible {
    let mut ax = sketched()?;
    let stored = width(&ax)?;
    let wider = format!("{} mm", ax.app.engine.project().board_size[0] + 2.0);
    retype(&mut ax, "width", &wider)?;
    ax.command_key(Key::Z);
    assert_eq!(width(&ax)?, stored, "the field's own undo");
    ax.key_with(Key::A, Modifiers::COMMAND);
    ax.type_text(&wider);
    ax.key(Key::Enter);
    ax.settle();
    assert_eq!(width(&ax)?, wider);
    assert!(!ax.ctx.text_edit_focused());
    ax.command_key(Key::Z);
    ax.settle();
    assert_eq!(width(&ax)?, stored, "one ⌘Z undoes the value");
    ax.key_with(Key::Z, Modifiers::COMMAND | Modifiers::SHIFT);
    ax.settle();
    assert_eq!(width(&ax)?, wider, "⇧⌘Z redoes it");
    // Esc leaves the field without a commit; ⌘Z is the project's again.
    retype(&mut ax, "width", "1 mm")?;
    ax.key(Key::Escape);
    ax.settle();
    ax.command_key(Key::Z);
    ax.settle();
    assert_eq!(width(&ax)?, stored, "⌘Z after Esc");
    Ok(())
}

fn board(ax: &Ax) -> Fallible<sparksmith_core::model::board::BoardSketch> {
    match &ax.app.engine.project().board_shape {
        BoardShape::Sketch(board) => Ok((**board).clone()),
        BoardShape::Rectangle => Err("the board is not a sketch".into()),
    }
}

fn dimensions(ax: &Ax) -> Fallible<usize> {
    let source = crate::pcb_editor::board_shape::source_of(&board(ax)?);
    let view = sparksmith_solidcraft::footprint::view::sketch_view(&source)?;
    Ok(view
        .constraints
        .iter()
        .filter(|k| k.value.is_some())
        .count())
}

/// Apply a `board.edit`, with the fills poured for it (see [`sketched`]).
fn edit(ax: &mut Ax, edits: Vec<sparksmith_core::layout::outline::BoardEdit>) -> Fallible {
    ax.app.engine.apply_hosted(
        ax.app.engine.revision(),
        crate::pcb_editor::board_shape::operation(edits),
        sparksmith_core::Evaluators {
            footprint: None,
            board: crate::pcb_editor::board_shape::host_evaluator(),
        },
    )?;
    ax.app.engine.refill_stale()?;
    ax.settle();
    Ok(())
}

/// The Board shape mode with a free sketch point at `at`, and the
/// Dimension tool's two clicks on it and the left edge.
fn placed(ax: &mut Ax, at: [f64; 2]) -> Fallible {
    use sparksmith_core::library::footprint_source::SketchEdit;
    edit(
        ax,
        vec![sparksmith_core::layout::outline::BoardEdit::Sketch {
            edit: SketchEdit::AddPoint { at },
        }],
    )?;
    let _ = ax.app.pcb.on_tool("board-shape", &ax.app.engine);
    ax.settle();
    let _ = ax.app.pcb.on_tool("bs-dimension", &ax.app.engine);
    ax.settle();
    let [_, h] = ax.app.engine.project().board_size;
    for p in [at, [0.0, h * 0.5]] {
        let p = ax.app.pcb.layout_to_screen(p).ok_or("no canvas")?;
        ax.click_at(p);
    }
    Ok(())
}

/// GUI check 1, F11 and item 5: a new dimension whose value is refused
/// leaves nothing behind, Enter keeps the text and the reason, and the next
/// accepted value makes the dimension and its parameter in one undo step.
#[test]
fn a_refused_value_leaves_no_dimension() -> Fallible {
    let mut ax = sketched()?;
    placed(&mut ax, [7.0, 9.0])?;
    let (before, count) = (board(&ax)?, dimensions(&ax)?);
    let typed = |ax: &mut Ax, text: &str| {
        assert!(ax.ctx.text_edit_focused(), "the value takes the keyboard");
        ax.key_with(Key::A, Modifiers::COMMAND);
        ax.type_text(text);
        ax.key(Key::Enter);
        ax.settle();
    };
    // Not an expression, then one only the engine can refuse.
    for refused in ["1 x = 2", "gap = nowhere + 1", "gap = -5 mm"] {
        typed(&mut ax, refused);
        assert_eq!(board(&ax)?, before, "{refused}");
        assert_eq!(dimensions(&ax)?, count, "{refused}");
        assert!(ax.texts.iter().any(|t| t == refused), "the text is kept");
    }
    // Esc drops it: nothing was added, and Esc went no further.
    ax.key(Key::Escape);
    ax.settle();
    assert_eq!(board(&ax)?, before);
    let refs = [
        [7.0, 9.0],
        [0.0, ax.app.engine.project().board_size[1] * 0.5],
    ];
    for p in refs {
        let p = ax.app.pcb.layout_to_screen(p).ok_or("no canvas")?;
        ax.click_at(p);
    }
    assert_eq!(board(&ax)?, before, "placed, not yet added");
    typed(&mut ax, "gap = 6 mm");
    assert_eq!(dimensions(&ax)?, count + 1);
    let named = |ax: &Ax| {
        Ok::<_, Box<dyn std::error::Error>>(board(ax)?.parameters.iter().any(|p| p.name.0 == "gap"))
    };
    assert!(named(&ax)?);
    ax.command_key(Key::Z);
    ax.settle();
    assert_eq!(board(&ax)?, before, "one ⌘Z undoes both");
    Ok(())
}

/// Leaving the field takes the measured value, as a card field commits.
#[test]
fn leaving_a_new_dimensions_field_takes_its_value() -> Fallible {
    let mut ax = sketched()?;
    placed(&mut ax, [7.0, 9.0])?;
    let count = dimensions(&ax)?;
    assert!(ax.ctx.text_edit_focused());
    // Empty canvas, clear of the sketch.
    let p = ax
        .app
        .pcb
        .layout_to_screen([20.0, -4.0])
        .ok_or("no canvas")?;
    ax.click_at(p);
    ax.settle();
    assert_eq!(dimensions(&ax)?, count + 1);
    Ok(())
}

/// GUI check 1, K12: a dimension on an edge the sketch already sets is
/// refused, naming what sets it.
#[test]
fn a_redundant_dimension_is_refused_where_it_is_placed() -> Fallible {
    let mut ax = sketched()?;
    let _ = ax.app.pcb.on_tool("board-shape", &ax.app.engine);
    ax.settle();
    let _ = ax.app.pcb.on_tool("bs-dimension", &ax.app.engine);
    ax.settle();
    let (before, count) = (board(&ax)?, dimensions(&ax)?);
    let [w, h] = ax.app.engine.project().board_size;
    for p in [[w, h * 0.5], [w + 3.0, h * 0.5]] {
        let p = ax.app.pcb.layout_to_screen(p).ok_or("no canvas")?;
        ax.click_at(p);
    }
    assert_eq!(dimensions(&ax)?, count);
    assert_eq!(board(&ax)?, before);
    assert!(!ax.ctx.text_edit_focused(), "no value is asked for");
    let says = "‘height’ already sets this";
    assert!(ax.texts.iter().any(|t| t.contains(says)), "{:?}", ax.texts);
    Ok(())
}

/// GUI check 1, F7: with Issues open and a long list, the reason a
/// parameter's value was refused is in sight, right under its row.
#[test]
fn a_refused_parameters_reason_is_in_sight_with_issues_open() -> Fallible {
    use sparksmith_core::layout::outline::BoardEdit;
    use sparksmith_core::library::footprint_source::{Expr, FootprintParam, ParamName, ParamUnit};
    let mut ax = sketched()?;
    ax.window = egui::vec2(1380.0, 720.0);
    let spare = |i: usize| BoardEdit::SetParameter {
        param: FootprintParam {
            name: ParamName(format!("spare{i}")),
            expr: Expr(format!("{i} mm")),
            unit: ParamUnit::Mm,
            comment: String::new(),
        },
    };
    edit(&mut ax, (1..8).map(spare).collect())?;
    let _ = ax.app.pcb.on_tool("board-shape", &ax.app.engine);
    ax.app.issues.open();
    ax.settle();
    assert!(ax.app.issues.expanded());
    retype(&mut ax, "width", "2000 mm")?;
    ax.key(Key::Enter);
    ax.settle();
    let why = "is out of range";
    let (text, rect) = ax
        .seen
        .iter()
        .find(|(t, _)| t.contains(why))
        .cloned()
        .ok_or_else(|| format!("out of sight: {:?}", ax.texts))?;
    assert!(rect.height() > 8.0, "{text}: {rect:?}");
    let row = ax.find_prefix(Role::ListBoxOption, "width")?.rect;
    assert!((rect.top() - row.bottom()).abs() < 12.0, "{row:?} {rect:?}");
    Ok(())
}

/// The constraints of `kind` (Solidcraft's name) the board sketch holds.
fn constraints(ax: &Ax, kind: &str) -> Fallible<usize> {
    let source = crate::pcb_editor::board_shape::source_of(&board(ax)?);
    let view = sparksmith_solidcraft::footprint::view::sketch_view(&source)?;
    Ok(view.constraints.iter().filter(|k| k.kind == kind).count())
}

/// The Board shape mode with one construction line for each pair of ends.
fn lines(ax: &mut Ax, ends: &[[[f64; 2]; 2]]) -> Fallible {
    use sparksmith_core::layout::outline::BoardEdit;
    use sparksmith_core::library::footprint_source::SketchEdit;
    let add = |[a, b]: &[[f64; 2]; 2]| BoardEdit::Sketch {
        edit: SketchEdit::AddLine {
            a: *a,
            b: *b,
            construction: true,
        },
    };
    edit(ax, ends.iter().map(add).collect())?;
    let _ = ax.app.pcb.on_tool("board-shape", &ax.app.engine);
    ax.settle();
    Ok(())
}

fn click_mm(ax: &mut Ax, at: [f64; 2]) -> Fallible {
    let p = ax.app.pcb.layout_to_screen(at).ok_or("no canvas")?;
    ax.click_at(p);
    ax.settle();
    Ok(())
}

/// A constraint tool armed by its key takes its pick, commits one step that
/// one ⌘Z undoes, and one the sketch sets already is refused naming it.
#[test]
fn a_constraint_tool_takes_its_picks_in_one_undo_step() -> Fallible {
    let mut ax = sketched()?;
    lines(
        &mut ax,
        &[[[9.0, 7.0], [17.0, 7.5]], [[9.0, 10.0], [17.0, 10.0]]],
    )?;
    let (before, count) = (board(&ax)?, constraints(&ax, "Horizontal")?);
    let over = ax
        .app
        .pcb
        .layout_to_screen([13.0, 8.5])
        .ok_or("no canvas")?;
    ax.hover(over);
    ax.key(Key::Z);
    ax.settle();
    let armed = |ax: &Ax, name: &str| {
        let hint = format!("Board shape · {name}");
        ax.texts.iter().any(|t| t.starts_with(&hint))
    };
    assert!(armed(&ax, "Horizontal"), "{:?}", ax.texts);
    click_mm(&mut ax, [13.0, 7.25])?;
    assert_eq!(constraints(&ax, "Horizontal")?, count + 1);
    ax.command_key(Key::Z);
    ax.settle();
    assert_eq!(board(&ax)?, before, "one ⌘Z undoes it");
    // Both lines horizontal: Parallel between them says nothing new.
    for at in [[13.0, 7.25], [13.0, 10.0]] {
        click_mm(&mut ax, at)?;
    }
    assert_eq!(constraints(&ax, "Horizontal")?, count + 2);
    let held = board(&ax)?;
    ax.key(Key::P);
    ax.settle();
    assert!(armed(&ax, "Parallel"), "{:?}", ax.texts);
    let source = crate::pcb_editor::board_shape::source_of(&held);
    let view = sparksmith_solidcraft::footprint::view::sketch_view(&source)?;
    for line in view.curves.iter().rev().take(2) {
        let sparksmith_solidcraft::footprint::view::ViewGeometry::Line { a, b } = line.geometry
        else {
            return Err("not a line".into());
        };
        click_mm(&mut ax, [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0])?;
    }
    assert_eq!(board(&ax)?, held, "refused");
    let says = "Horizontal already set";
    assert!(ax.texts.iter().any(|t| t.contains(says)), "{:?}", ax.texts);
    Ok(())
}

/// In the smallest window the Constrain group is in the ribbon's More tools
/// menu, every constraint with it, and one picked there is armed.
#[test]
fn a_small_window_offers_every_constraint() -> Fallible {
    let mut ax = sketched()?;
    ax.window = egui::vec2(760.0, 560.0);
    let _ = ax.app.pcb.on_tool("board-shape", &ax.app.engine);
    ax.settle();
    ax.button("More tools")?;
    ax.settle();
    for name in ["Coincident", "Perpendicular", "Fix"] {
        assert!(ax.has(Role::Button, name), "{name}: {}", ax.dump());
    }
    ax.click(Role::Button, "Horizontal")?;
    ax.settle();
    let armed = |t: &String| t.starts_with("Board shape · Horizontal");
    assert!(ax.texts.iter().any(armed), "{:?}", ax.texts);
    Ok(())
}

/// At an ordinary width the Constrain group is on the ribbon itself.
#[test]
fn the_constrain_group_fits_an_ordinary_window() -> Fallible {
    let mut ax = sketched()?;
    ax.window = egui::vec2(1000.0, 700.0);
    let _ = ax.app.pcb.on_tool("board-shape", &ax.app.engine);
    ax.settle();
    let shown = |t: &str| ax.seen.iter().any(|(s, r)| s == t && r.width() > 8.0);
    assert!(shown("Constrain") && shown("Constraints"), "{:?}", ax.seen);
    Ok(())
}
