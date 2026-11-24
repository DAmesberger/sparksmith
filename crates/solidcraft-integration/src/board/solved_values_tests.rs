//! The solved board is one unit: its shape and its parameter values belong
//! to the same solve, and are replaced together or not at all.
#![allow(clippy::unwrap_used)]

use super::tests::{Fallible, apply, close, created_curves_of, edit, set, sketch, sketched};
use sparksmith_core::interface::{DatumGeometry, ExportTarget, InterfaceCommand};
use sparksmith_core::layout::outline::BoardEdit;
use sparksmith_core::library::footprint_source::{ParamName, ParamValue, SketchEdit, SketchRef};
use sparksmith_core::{Engine, Operation, Query, QueryResult, format};

fn export(engine: &mut Engine, name: &str) -> Fallible<()> {
    apply(
        engine,
        Operation::Interface(InterfaceCommand::Export {
            target: ExportTarget::BoardParameter {
                name: ParamName(name.into()),
            },
            name: None,
        }),
    )?;
    Ok(())
}

/// What `interface.list` gives for an exported board parameter, mm.
fn listed(engine: &Engine, name: &str) -> Fallible<Option<f64>> {
    let QueryResult::InterfaceList(list) = engine.query(Query::InterfaceList {})? else {
        return Err("the interface".into());
    };
    let token = format!("board.parameter.{name}");
    let row = list.datums.iter().find(|d| d.token == token);
    Ok(match &row.ok_or(token)?.geometry {
        Some(DatumGeometry::Parameter { value, .. }) => Some(*value),
        _ => None,
    })
}

/// Take the bottom line away: the outline is open.
fn open_outline(engine: &mut Engine) -> Fallible<()> {
    let bottom = sketch(engine)?
        .edges
        .iter()
        .find(|e| e.name.0 == "bottom")
        .map(|e| e.curve.clone())
        .ok_or("bottom")?;
    edit(
        engine,
        vec![BoardEdit::Sketch {
            edit: SketchEdit::Delete {
                ids: vec![SketchRef::Curve(bottom)],
            },
        }],
    )?;
    Ok(())
}

/// Reproduction 1: an unused parameter the host cannot give a value is
/// refused naming it; it does not vanish from the stored values.
#[test]
fn a_parameter_without_a_value_refuses_the_edit_naming_it() -> Fallible<()> {
    let mut engine = sketched()?;
    let (revision, before) = (engine.revision(), engine.to_file());
    for expr in ["1e12 mm", "30 deg", "width / 0", "nowhere + 1 mm"] {
        let error = edit(&mut engine, vec![set("spare", expr)])
            .err()
            .ok_or_else(|| format!("`{expr}` accepted"))?
            .to_string();
        assert!(error.contains("spare"), "{expr}: {error}");
        assert_eq!(engine.revision(), revision, "{expr}");
        assert_eq!(engine.to_file(), before, "{expr}");
    }
    // The person types another expression.
    edit(&mut engine, vec![set("spare", "width / 2")])?;
    Ok(())
}

/// Reproduction 2: with an invalid outline the shape stays the last valid
/// one, and so does what the interface says of the parameters.
#[test]
fn values_stay_with_the_last_valid_shape() -> Fallible<()> {
    let mut engine = sketched()?;
    export(&mut engine, "width")?;
    assert_eq!(listed(&engine, "width")?, Some(60.0));
    open_outline(&mut engine)?;
    edit(&mut engine, vec![set("width", "75 mm")])?;
    assert_eq!(engine.project().board_size, [60.0, 40.0]);
    assert_eq!(listed(&engine, "width")?, Some(60.0));
    Ok(())
}

/// What the solved board stores: each parameter's name and value, mm.
fn stored(engine: &Engine) -> Fallible<Vec<(String, f64)>> {
    sketch(engine)?
        .solved
        .values
        .iter()
        .map(|v| match v.value {
            ParamValue::Length(mm) => Ok((v.name.0.clone(), mm)),
            other => Err(format!("{other:?}").into()),
        })
        .collect()
}

fn pairs(values: &[(&str, f64)]) -> Vec<(String, f64)> {
    values.iter().map(|(n, v)| ((*n).to_owned(), *v)).collect()
}

fn close_outline(engine: &mut Engine, width: f64) -> Fallible<()> {
    edit(
        engine,
        vec![BoardEdit::Sketch {
            edit: SketchEdit::AddLine {
                a: [0.0, 0.0],
                b: [width, 0.0],
                construction: false,
            },
        }],
    )?;
    Ok(())
}

fn remove(name: &str) -> BoardEdit {
    BoardEdit::RemoveParameter {
        name: ParamName(name.into()),
    }
}

/// A board just started has its two parameters' values; a valid edit
/// replaces shape and values together, and undo and redo take both.
#[test]
fn shape_and_values_are_replaced_together() -> Fallible<()> {
    let mut engine = sketched()?;
    assert_eq!(
        stored(&engine)?,
        pairs(&[("width", 60.0), ("height", 40.0)])
    );
    edit(
        &mut engine,
        vec![set("width", "75 mm"), set("half", "width / 2")],
    )?;
    let wide = pairs(&[("width", 75.0), ("height", 40.0), ("half", 37.5)]);
    assert_eq!(engine.project().board_size, [75.0, 40.0]);
    assert_eq!(stored(&engine)?, wide);
    engine.undo()?;
    assert_eq!(engine.project().board_size, [60.0, 40.0]);
    assert_eq!(
        stored(&engine)?,
        pairs(&[("width", 60.0), ("height", 40.0)])
    );
    engine.redo()?;
    assert_eq!(engine.project().board_size, [75.0, 40.0]);
    assert_eq!(stored(&engine)?, wide);
    // Renamed (the old one removed, the new one added), then removed.
    edit(
        &mut engine,
        vec![remove("half"), set("middle", "width / 2")],
    )?;
    assert_eq!(
        stored(&engine)?,
        pairs(&[("width", 75.0), ("height", 40.0), ("middle", 37.5)])
    );
    edit(&mut engine, vec![remove("middle")])?;
    assert_eq!(
        stored(&engine)?,
        pairs(&[("width", 75.0), ("height", 40.0)])
    );
    Ok(())
}

/// While the outline is open, parameters are kept as typed, renamed and
/// removed ones too, and the solved board is kept whole, through undo, redo
/// and a file; closing it again replaces the whole of it.
#[test]
fn an_invalid_outline_keeps_the_solved_board_whole() -> Fallible<()> {
    let mut engine = sketched()?;
    edit(&mut engine, vec![set("spare", "3 mm")])?;
    let valid = engine.project().board().into_owned();
    open_outline(&mut engine)?;
    edit(
        &mut engine,
        vec![
            set("width", "75 mm"),
            remove("spare"),
            set("other", "width / 3"),
        ],
    )?;
    let typed = |engine: &Engine| -> Fallible<Vec<(String, String)>> {
        let sketch = sketch(engine)?;
        let each = sketch.parameters.iter();
        Ok(each.map(|p| (p.name.0.clone(), p.expr.0.clone())).collect())
    };
    let now = typed(&engine)?;
    assert!(now.contains(&("width".into(), "75 mm".into())), "{now:?}");
    assert!(
        now.contains(&("other".into(), "width / 3".into())),
        "{now:?}"
    );
    assert!(!now.iter().any(|(n, _)| n == "spare"), "{now:?}");
    assert_eq!(engine.project().board().into_owned(), valid);
    assert!(sketch(&engine)?.invalid().is_some());

    engine.undo()?;
    assert_eq!(engine.project().board().into_owned(), valid);
    engine.redo()?;
    assert_eq!(engine.project().board().into_owned(), valid);
    assert_eq!(typed(&engine)?, now);

    let mut reopened = Engine::from_file(format::decode(&format::encode(&engine.to_file())?)?)?;
    assert_eq!(reopened.project().board().into_owned(), valid);
    assert_eq!(typed(&reopened)?, now);
    close_outline(&mut reopened, 75.0)?;
    assert!(sketch(&reopened)?.invalid().is_none());
    assert!(close(reopened.project().board_size, [75.0, 40.0]));
    assert_eq!(
        stored(&reopened)?,
        pairs(&[("width", 75.0), ("height", 40.0), ("other", 25.0)])
    );
    Ok(())
}

/// A stray open line makes the outline invalid although the rest still
/// closes; the board keeps its last valid solve until it is gone.
#[test]
fn a_stray_open_line_keeps_the_solved_board_too() -> Fallible<()> {
    let mut engine = sketched()?;
    let valid = engine.project().board().into_owned();
    let stray = created_curves_of(edit(
        &mut engine,
        vec![BoardEdit::Sketch {
            edit: SketchEdit::AddLine {
                a: [20.0, 20.0],
                b: [30.0, 20.0],
                construction: false,
            },
        }],
    )?);
    edit(&mut engine, vec![set("width", "75 mm")])?;
    assert!(sketch(&engine)?.invalid().is_some());
    assert_eq!(engine.project().board().into_owned(), valid);
    edit(
        &mut engine,
        vec![BoardEdit::Sketch {
            edit: SketchEdit::Delete {
                ids: stray.into_iter().map(SketchRef::Curve).collect(),
            },
        }],
    )?;
    assert_eq!(engine.project().board_size, [75.0, 40.0]);
    assert_eq!(
        stored(&engine)?,
        pairs(&[("width", 75.0), ("height", 40.0)])
    );
    Ok(())
}

/// A cutout can be deleted while the outline is open; the kept shape still
/// has it.
#[test]
fn a_cutout_is_deleted_while_the_outline_is_open() -> Fallible<()> {
    let mut engine = sketched()?;
    let circle = created_curves_of(edit(
        &mut engine,
        vec![BoardEdit::Sketch {
            edit: SketchEdit::AddCircle {
                center: [30.0, 20.0],
                radius: 2.0,
                construction: false,
            },
        }],
    )?);
    assert_eq!(engine.project().board().cutouts.len(), 1);
    open_outline(&mut engine)?;
    edit(
        &mut engine,
        vec![BoardEdit::Sketch {
            edit: SketchEdit::Delete {
                ids: circle.into_iter().map(SketchRef::Curve).collect(),
            },
        }],
    )?;
    assert_eq!(engine.project().board().cutouts.len(), 1);
    close_outline(&mut engine, 60.0)?;
    assert!(engine.project().board().cutouts.is_empty());
    Ok(())
}

/// Fabrication output of a board whose outline is invalid is refused, and
/// no waiver lifts it; the native export is refused, writing nothing.
#[test]
fn an_invalid_outline_is_not_exported() -> Fallible<()> {
    use crate::export::{Unfit, Watch, board};
    use sparksmith_core::fabrication::preflight::{FabricationCommand, can_waive};
    use sparksmith_core::layout::outline::{SHAPE_INVALID, SHAPE_OPEN, SHAPE_UNSOLVED};
    use sparksmith_core::model::interface::Tier;
    let mut engine = sketched()?;
    open_outline(&mut engine)?;
    for code in [SHAPE_OPEN, SHAPE_INVALID, SHAPE_UNSOLVED] {
        assert!(!can_waive(code), "{code}");
    }
    let QueryResult::Preflight(report) = engine.query(Query::Preflight { profile: None })? else {
        return Err("preflight".into());
    };
    let item = report.items.iter().find(|i| i.code == SHAPE_OPEN);
    let item = item.ok_or("open")?;
    assert!(item.blocks() && !item.offers_waiver());
    let waived = apply(
        &mut engine,
        Operation::Fabrication(FabricationCommand::Waive {
            keys: vec![item.key.clone()],
            reason: String::new(),
        }),
    );
    let refused = waived.err().ok_or("waived")?.to_string();
    assert!(
        refused.contains("make the board outline valid"),
        "{refused}"
    );

    let dir = tempfile::tempdir()?;
    let path = dir.path().join("board.solidcraft");
    let path = path.to_string_lossy().into_owned();
    for unfit in [Unfit::Refuse, Unfit::Write] {
        let refused = board(
            engine.project(),
            Tier::Mechanical,
            crate::native::files::test_file(&path),
            &Watch::new(),
            unfit,
        )
        .err()
        .ok_or("exported")?
        .to_string();
        assert!(refused.contains("export.outline_invalid"), "{refused}");
        assert_eq!(std::fs::read_dir(dir.path())?.count(), 0);
    }
    Ok(())
}
