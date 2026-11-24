//! The solved board is one unit: its shape and its parameter values belong
//! to the same solve, and are replaced together or not at all.
#![allow(clippy::unwrap_used)]

use super::tests::{Fallible, apply, edit, set, sketch, sketched};
use sparksmith_core::interface::{DatumGeometry, ExportTarget, InterfaceCommand};
use sparksmith_core::layout::outline::BoardEdit;
use sparksmith_core::library::footprint_source::{ParamName, SketchEdit, SketchRef};
use sparksmith_core::{Engine, Operation, Query, QueryResult};

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

/// Take the bottom line away (the outline is open) and put it back, its
/// ends on the board's lower corners.
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
