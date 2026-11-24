//! The ESP32-S31 dev board keeps its sole USB coverage warning through edits that
//! move J1, the USB-C receptacle: its 3D dimension from the tab's edge
//! 0 → 1 mm and back, the board's width 40 → 50 mm and back, and its height
//! 64 → 70 mm. The parts beside J1 are held at fixed offsets from it, so
//! the cluster moves as one, and the routes leaving it keep their
//! directions (`layout::dimensions::carry`): the USB pair stays coupled and
//! its skew stays within its class limit. The module and the parts beside
//! it are held too, to the centre line and the top edge: a wider board
//! moves every part with its centre line, so its routes move as they are,
//! and a taller one stretches the buses and the pair between the two
//! groups.
//!
//! After each edit the stale fills are refilled as the desktop's fill
//! worker does (`compute_stale`, stored without an undo step); then the
//! production check (`fabrication.preflight`) names the measured uncoupled
//! USB tracks and finds no other issue. One undo restores the board exactly.
#[path = "../../io/tests/support/esp32_preflight.rs"]
mod esp32_preflight;

use esp32_preflight::{USB_UNCOUPLED_MM, assert_usb_preflight};
use sparksmith_automation::session::hosted;
use sparksmith_core::format::EditorFile;
use sparksmith_core::layout::dimensions::DimensionCommand;
use sparksmith_core::layout::fill::{FillCache, compute_stale};
use sparksmith_core::layout::length;
use sparksmith_core::layout::outline::{BoardCommand, BoardEdit};
use sparksmith_core::library::footprint_source::{
    DimValue, Expr, FootprintParam, ParamName, ParamUnit,
};
use sparksmith_core::model::dimension::{BoardRef, FeatureRef, PartDimension};
use sparksmith_core::{Engine, Operation, Project, Query, QueryResult};
use std::path::Path;

type Fallible<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn example() -> Fallible<Engine> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/esp32-s31-devboard/esp32-s31-devboard.sparksmith");
    let mut engine = Engine::from_file(sparksmith_io::read_project(&path)?)?;
    // A file holds no fill copper; fill it as opening does.
    engine.refill_stale()?;
    Ok(engine)
}

fn check_preflight(engine: &Engine, uncoupled_mm: f64) -> Fallible {
    let QueryResult::Preflight(report) = engine.query(Query::Preflight { profile: None })? else {
        return Err("not a preflight report".into());
    };
    assert_usb_preflight(&report, uncoupled_mm);
    Ok(())
}

/// The USB pair's skew and its class limit.
fn skew(project: &Project) -> Fallible<(f64, f64)> {
    let report = length::report(project);
    let usb = report.pairs.first().ok_or("no pair row")?;
    Ok((usb.skew, usb.max_skew.ok_or("no skew limit")?))
}

/// Apply `operation`, refill the stale fills without an undo step, and
/// check the remaining USB coverage. Returns the file before the edit.
fn clean_edit(
    engine: &mut Engine,
    what: &str,
    operation: Operation,
    uncoupled_mm: f64,
) -> Fallible<EditorFile> {
    let before = engine.to_file();
    engine.apply_hosted(engine.revision(), operation, hosted())?;
    let fresh = compute_stale(engine.project(), &mut FillCache::default())?;
    engine.store_fills(engine.revision(), fresh)?;
    check_preflight(engine, uncoupled_mm)?;
    let (skew, limit) = skew(engine.project())?;
    assert!(skew <= limit, "{what}: skew {skew} > {limit}");
    Ok(before)
}

/// Undo one step: the board is exactly as it was.
fn undo_to(engine: &mut Engine, what: &str, before: &EditorFile) -> Fallible {
    engine.undo()?;
    assert!(&engine.to_file() == before, "{what}: one undo restores it");
    Ok(())
}

fn set_board(name: &str, mm: f64) -> Operation {
    Operation::Board(BoardCommand::Edit {
        edits: vec![BoardEdit::SetParameter {
            param: FootprintParam {
                name: ParamName(name.into()),
                expr: Expr(format!("{mm} mm")),
                unit: ParamUnit::Mm,
                comment: String::new(),
            },
        }],
    })
}

/// The dimension holding J1's shell face to the bottom edge.
fn j1_face(engine: &Engine) -> Fallible<PartDimension> {
    let j1 = engine
        .project()
        .instance_by_designator("J1")
        .ok_or("J1")?
        .id;
    Ok(engine
        .project()
        .part_dimensions
        .iter()
        .find(|d| {
            d.refs.iter().any(|r| {
                matches!(r, BoardRef::Part(FeatureRef::Face { placement, .. }) if *placement == j1)
            })
        })
        .cloned()
        .ok_or("no dimension on J1's face")?)
}

fn set_j1(held: &PartDimension, mm: f64) -> Operation {
    let mut dimension = held.clone();
    dimension.value = Some(DimValue::Fixed(mm));
    Operation::Dimension(DimensionCommand::Edit { dimension })
}

#[test]
fn the_board_starts_with_the_usb_coverage_warning() -> Fallible {
    let engine = example()?;
    check_preflight(&engine, USB_UNCOUPLED_MM)
}

#[test]
fn j1_moved_up_by_its_dimension_and_back_stays_clean() -> Fallible {
    let mut engine = example()?;
    let held = j1_face(&engine)?;
    assert_eq!(held.value, Some(DimValue::Fixed(0.0)));
    let start = clean_edit(
        &mut engine,
        "J1 1 mm from the edge",
        set_j1(&held, 1.0),
        USB_UNCOUPLED_MM,
    )?;
    let moved = clean_edit(
        &mut engine,
        "J1 back on the edge",
        set_j1(&held, 0.0),
        USB_UNCOUPLED_MM,
    )?;
    undo_to(&mut engine, "J1 back on the edge", &moved)?;
    undo_to(&mut engine, "J1 1 mm from the edge", &start)?;
    Ok(())
}

#[test]
fn the_board_widened_and_back_stays_clean() -> Fallible {
    let mut engine = example()?;
    assert_eq!(engine.project().board_size, [40.0, 64.0]);
    let start = clean_edit(
        &mut engine,
        "the board 50 mm wide",
        set_board("width", 50.0),
        // The module follows the centre line as J1 does, so the pair's
        // paths move as they are.
        USB_UNCOUPLED_MM,
    )?;
    let moved = |designator: &str| -> Fallible<[f64; 2]> {
        let id = |p: &Project| p.instance_by_designator(designator).map(|i| i.id);
        let at = |p: &Project| id(p).and_then(|i| p.placement_of(i)).map(|p| p.position);
        let (was, now) = (
            at(&start.project).ok_or("placed")?,
            at(engine.project()).ok_or("placed")?,
        );
        Ok([now[0] - was[0], now[1] - was[1]])
    };
    for part in ["J1", "U3", "U1", "J2", "J3", "SW2"] {
        let by = moved(part)?;
        assert!(
            (by[0] - 5.0).abs() < 1e-6 && by[1].abs() < 1e-6,
            "{part} moved {by:?}"
        );
    }
    let wide = clean_edit(
        &mut engine,
        "the board 40 mm wide again",
        set_board("width", 40.0),
        USB_UNCOUPLED_MM,
    )?;
    undo_to(&mut engine, "the board 40 mm wide again", &wide)?;
    undo_to(&mut engine, "the board 50 mm wide", &start)?;
    Ok(())
}

#[test]
fn the_board_made_taller_stays_clean() -> Fallible {
    let mut engine = example()?;
    let start = clean_edit(
        &mut engine,
        "the board 70 mm tall",
        set_board("height", 70.0),
        USB_UNCOUPLED_MM,
    )?;
    undo_to(&mut engine, "the board 70 mm tall", &start)?;
    Ok(())
}
