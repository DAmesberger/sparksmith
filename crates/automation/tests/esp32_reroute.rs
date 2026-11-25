//! Parts moved by a dimension or a turn re-route the copper they drag
//! (`layout::router::reroute`): on the ESP32-S31 dev board, J1 moved along
//! the tab's edge by its dimension takes U3 with it, which dimensions hold
//! beside it, while the module stays, and their USB pair is re-routed as a
//! pair instead of being carried into clearance errors or out of coupling;
//! a part turned 37° re-routes its tracks. (The board's width once moved
//! J1 alone; it now moves the module with it, so the same move is made by
//! J1's own dimension from the left edge.) After each edit
//! the stale fills are refilled as the desktop's fill worker does, the
//! board has no rule error, the pair keeps within its class limits, the
//! report names what was re-routed, and one undo restores the board.
use sparksmith_automation::session::{ApplyParams, Request, Session, hosted};
use sparksmith_core::format::EditorFile;
use sparksmith_core::layout::dimensions::DimensionCommand;
use sparksmith_core::layout::fill::{FillCache, compute_stale};
use sparksmith_core::layout::pair::{PairLeg, PairRouteRequest};
use sparksmith_core::layout::{self, LayoutCommand, RouteEditSet, length};
use sparksmith_core::library::footprint_source::{DimValue, Expr};
use sparksmith_core::model::CopperLayer;
use sparksmith_core::model::board::ConstraintTarget;
use sparksmith_core::model::dimension::{BoardRef, FeatureRef};
use sparksmith_core::{Engine, Operation};
use std::path::Path;

type Fallible<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn example() -> Fallible<Engine> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/esp32-s31-devboard/esp32-s31-devboard.sparksmith");
    Ok(Engine::from_file(sparksmith_io::read_project(&path)?)?)
}

/// J1 `mm` right of the board's centre line: its dimension from the left
/// edge, `width / 2`, edited. The parts held beside J1 go with it; the
/// module and the headers stay.
fn j1_right(engine: &Engine, mm: f64) -> Fallible<Operation> {
    let j1 = engine
        .project()
        .instance_by_designator("J1")
        .ok_or("J1")?
        .id;
    let mut dimension = engine
        .project()
        .part_dimensions
        .iter()
        .find(|d| {
            matches!(d.refs.first(), Some(BoardRef::Part(FeatureRef::Origin {
                target: ConstraintTarget::Placement(p),
            })) if *p == j1)
                && matches!(d.refs.get(1), Some(BoardRef::Board(_)))
        })
        .cloned()
        .ok_or("J1's dimension from the left edge")?;
    dimension.value = Some(DimValue::Expr(Expr(format!("width / 2 + {mm} mm"))));
    Ok(Operation::Dimension(DimensionCommand::Edit { dimension }))
}

fn turn(engine: &Engine, designator: &str, by: f64) -> Fallible<Operation> {
    let instance = engine
        .project()
        .instance_by_designator(designator)
        .ok_or(designator.to_owned())?
        .id;
    let placement = engine
        .project()
        .placement_of(instance)
        .ok_or("not placed")?;
    Ok(Operation::Layout(LayoutCommand::Move {
        instance,
        position: placement.position,
        rotation_deg: placement.rotation_deg + by,
        layer: placement.layer,
    }))
}

/// Rule errors on the board (clearance, hole and keepout findings, and any
/// blocking one) and the USB pair's findings, once the stale fills are
/// refilled without an undo step; but those the board accepts at their value
/// (J1's two locating holes, which JLCPCB does not state).
fn issues(engine: &mut Engine) -> Fallible<Vec<String>> {
    use sparksmith_core::fabrication::preflight;
    let fresh = compute_stale(engine.project(), &mut FillCache::default())?;
    engine.store_fills(engine.revision(), fresh)?;
    let accepted: std::collections::BTreeSet<String> = preflight::report(
        engine.project(),
        engine.drafts(),
        None,
        engine.check_settings(),
    )?
    .items
    .into_iter()
    .filter(|i| i.waiver.is_some())
    .map(|i| i.key)
    .collect();
    Ok(layout::checks(engine.project(), engine.check_settings())
        .into_iter()
        .filter(|f| !accepted.contains(&preflight::key(&f.code, &f.objects)))
        .filter(|f| {
            f.blocks.is_some()
                || f.code.starts_with("pair.")
                || f.code.starts_with("keepout.")
                || f.code.contains("clearance")
        })
        .map(|f| format!("{}: {}", f.code, f.message))
        .collect())
}

/// The USB pair within its class's uncoupled-length and skew limits.
fn coupled(engine: &Engine) -> Fallible {
    let report = length::report(engine.project());
    let usb = report.pairs.first().ok_or("no pair row")?;
    let (uncoupled, skew) = (
        usb.max_uncoupled.ok_or("no uncoupled limit")?,
        usb.max_skew.ok_or("no skew limit")?,
    );
    assert!(usb.uncoupled <= uncoupled, "{} mm uncoupled", usb.uncoupled);
    assert!(usb.skew <= skew, "{} mm skew", usb.skew);
    Ok(())
}

/// Apply `operation` and check the board is clean; returns the file before.
fn clean_edit(engine: &mut Engine, operation: Operation) -> Fallible<EditorFile> {
    let before = engine.to_file();
    engine.apply_hosted(engine.revision(), operation, hosted())?;
    assert_eq!(issues(engine)?, Vec::<String>::new());
    coupled(engine)?;
    Ok(before)
}

fn undo_to(engine: &mut Engine, before: &EditorFile) -> Fallible {
    engine.undo()?;
    assert!(&engine.to_file() == before, "one undo restores the board");
    Ok(())
}

/// The USB pair re-routed from U3 to U1 by hand along another centre
/// line, as a designer might: down from U3, a 45° jog right, down past the
/// module and in to U1's pins. Clean and coupled as it stands.
fn rerouted_by_hand() -> Fallible<Engine> {
    let mut engine = example()?;
    let pair = engine
        .project()
        .rules
        .diff_pairs
        .first()
        .ok_or("no pair")?
        .clone();
    // The pair's tracks below U3's pads (y 13.55 mm): its run to U1.
    let remove = engine
        .project()
        .tracks
        .iter()
        .filter(|t| pair.has(t.net) && t.start[1].min(t.end[1]) >= 13.55 - 1e-6)
        .map(|t| t.id)
        .collect();
    engine.apply(
        engine.revision(),
        Operation::Layout(LayoutCommand::ApplyRoute {
            edits: RouteEditSet {
                remove,
                ..Default::default()
            },
        }),
    )?;
    engine.apply(
        engine.revision(),
        Operation::Layout(LayoutCommand::RoutePair {
            request: PairRouteRequest {
                pair: pair.id,
                start: [[20.95, 13.55], [19.05, 13.55]],
                legs: vec![PairLeg {
                    layer: CopperLayer::Top,
                    points: vec![
                        [20.0, 13.55],
                        [20.0, 16.0],
                        [21.0, 17.0],
                        [21.0, 29.0],
                        [25.1, 33.1],
                        [25.1, 33.6],
                    ],
                }],
                end: Some([[25.525, 34.7], [24.675, 34.7]]),
                width: None,
                gap: None,
            },
        }),
    )?;
    assert_eq!(issues(&mut engine)?, Vec::<String>::new());
    coupled(&engine)?;
    Ok(engine)
}

/// The re-route report names both halves of the pair, re-routed.
fn pair_rerouted(engine: &Engine) {
    let report = engine.rerouted();
    assert!(report.unrouted.is_empty(), "{report:?}");
    for net in ["USB_DP", "USB_DN"] {
        assert!(report.rerouted.iter().any(|n| n == net), "{report:?}");
    }
}

/// Night finding R3 "pair coupling on resize": J1 moved 0.5 mm along the
/// edge (then by a board 41 mm wide) carried the two halves differently,
/// leaving 21.03 mm uncoupled.
#[test]
fn j1_moved_half_a_millimetre_keeps_the_usb_pair_coupled() -> Fallible {
    let mut engine = example()?;
    let operation = j1_right(&engine, 0.5)?;
    let before = clean_edit(&mut engine, operation)?;
    pair_rerouted(&engine);
    undo_to(&mut engine, &before)
}

/// J1 moved 2 mm takes R2's via to the +3V3 plane onto IO42's track. The
/// via is shoved clear and stays, R2 stays on +3V3, and no via is lost
/// (the edit was once accepted with that via gone, R2 off its plane).
#[test]
fn j1_moved_two_millimetres_keeps_r2_on_its_plane() -> Fallible {
    let mut engine = example()?;
    let vias = |e: &Engine| -> Vec<u64> { e.project().vias.iter().map(|v| v.id).collect() };
    let had = vias(&engine);
    for mm in [2.0, -2.0] {
        let operation = j1_right(&engine, mm)?;
        let before = engine.to_file();
        engine.apply_hosted(engine.revision(), operation, hosted())?;
        assert_eq!(vias(&engine), had, "{mm} mm");
        println!("{mm} mm: {:?}", engine.rerouted());
        let found = issues(&mut engine)?;
        println!("{mm} mm: {found:?}");
        assert!(
            !found.iter().any(|f| f.contains("clearance")),
            "{mm} mm: {found:?}"
        );
        let open: Vec<_> = layout::airwires(engine.project());
        assert!(open.is_empty(), "{mm} mm: {open:?}");
        undo_to(&mut engine, &before)?;
    }
    Ok(())
}

/// Night finding R7-1: with the pair re-routed by hand, J1 moved 1 mm
/// along the edge (then by a board 42 mm wide) carried it across U3's pad
/// and into itself (clearance errors).
#[test]
fn j1_moved_a_millimetre_reroutes_a_hand_routed_usb_pair_clear() -> Fallible {
    let mut engine = rerouted_by_hand()?;
    let operation = j1_right(&engine, 1.0)?;
    let before = clean_edit(&mut engine, operation)?;
    pair_rerouted(&engine);
    undo_to(&mut engine, &before)
}

/// Night finding R6-6: a routed part turned 37° re-routes its tracks
/// instead of leaving them across its pads.
#[test]
fn a_part_turned_37_degrees_reroutes_its_tracks() -> Fallible {
    let mut engine = example()?;
    let operation = turn(&engine, "U2", 37.0)?;
    let before = clean_edit(&mut engine, operation)?;
    let report = engine.rerouted();
    assert!(!report.rerouted.is_empty(), "{report:?}");
    assert!(report.unrouted.is_empty(), "{report:?}");
    undo_to(&mut engine, &before)
}

/// The same edit on the same board gives the same board, re-routed
/// copper included.
#[test]
fn rerouting_is_deterministic() -> Fallible {
    let mut boards = Vec::new();
    for _ in 0..2 {
        let mut engine = rerouted_by_hand()?;
        let mut reports = Vec::new();
        let operation = j1_right(&engine, 1.0)?;
        engine.apply_hosted(engine.revision(), operation, hosted())?;
        reports.push(engine.rerouted().clone());
        let operation = turn(&engine, "U2", 37.0)?;
        engine.apply_hosted(engine.revision(), operation, hosted())?;
        reports.push(engine.rerouted().clone());
        assert!(
            reports.iter().all(|r| !r.rerouted.is_empty()),
            "{reports:?}"
        );
        boards.push((engine.to_file(), reports));
    }
    assert!(boards.first() == boards.get(1));
    Ok(())
}

/// CLI and MCP replies carry the re-route report and its summary line.
#[test]
fn the_reply_names_what_was_rerouted() -> Fallible {
    let mut session = Session::new(example()?);
    let vias =
        |s: &Session| -> Vec<u64> { s.engine().project().vias.iter().map(|v| v.id).collect() };
    let before = vias(&session);
    let reply = session.execute(Request::Apply(ApplyParams {
        expected_revision: session.engine().revision(),
        operation: j1_right(session.engine(), 0.5)?,
    }))?;
    assert_eq!(vias(&session), before, "no via is lost");
    // The pair beside J1. The regulator's +3V3 via (470), carried with J1's
    // cluster, ends 0.128 mm from IO42's bottom track: clear of JLCPCB
    // 4-layer's 0.09 mm track to via, so IO42 stays (under PCBWay 4-layer's
    // 0.18 mm minimum it was re-routed too). The via is clear of the pair
    // once that is re-routed, so it stays.
    let rerouted = &reply["rerouted"];
    assert_eq!(
        rerouted["rerouted"],
        serde_json::json!(["USB_DP", "USB_DN"])
    );
    assert_eq!(rerouted["unrouted"], serde_json::json!([]));
    assert_eq!(rerouted["summary"], "Re-routed 2 connections");
    // An edit that moves nothing has no report.
    let reply = session.execute(Request::Apply(ApplyParams {
        expected_revision: session.engine().revision(),
        operation: j1_right(session.engine(), 0.5)?,
    }))?;
    assert!(reply.get("rerouted").is_none());
    Ok(())
}

/// Times the edits above, apply included (`--ignored --nocapture`).
#[test]
#[ignore = "timing report"]
fn reroute_timings() -> Fallible {
    let median = |mut v: Vec<f64>| {
        v.sort_by(f64::total_cmp);
        v.get(v.len() / 2).copied().unwrap_or(0.0)
    };
    type Start = fn() -> Fallible<Engine>;
    let cases: Vec<(&str, Start, &str)> = vec![
        ("J1 0.5 mm along the edge, example", example, "0.5"),
        ("J1 1 mm along the edge, example", example, "1"),
        (
            "J1 1 mm along the edge, pair re-routed by hand",
            rerouted_by_hand,
            "1",
        ),
        ("U2 turned 37°", example, "U2"),
    ];
    for (name, start, edit) in cases {
        let mut times = Vec::new();
        for _ in 0..15 {
            let mut engine = start()?;
            let operation = match edit {
                "U2" => turn(&engine, "U2", 37.0)?,
                mm => j1_right(&engine, mm.parse()?)?,
            };
            let t = std::time::Instant::now();
            engine.apply_hosted(engine.revision(), operation, hosted())?;
            times.push(t.elapsed().as_secs_f64() * 1e3);
        }
        let max = times.iter().copied().fold(0.0, f64::max);
        println!("{name}: median {:.1} ms, max {max:.1} ms", median(times));
    }
    Ok(())
}
