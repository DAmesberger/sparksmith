//! The ESP32-S31 dev board is a parametric design. Its outline is a board
//! sketch that solves with no freedom left, every dimension of it bound to
//! a named board parameter: a tab under the USB-C receptacle, a notch under
//! the module's antenna between two ears, three mounting holes and a
//! mounting slot, and a pass-through window. Every placed part is held by
//! part dimensions to a board edge or to the part it serves.
//!
//! Changing the driving parameters moves all of it together: the outline,
//! the holes, the slot and the window; the dimensioned parts and the copper
//! on their pads; the pours once they are refilled; the exact 3D model; and
//! the published interface a linked design references, with a cutout the
//! linked design projects from it. Changing them back restores the board. A
//! change that would break the design is refused with the reason and
//! changes nothing, or it is reported by the production check.
#[path = "../../io/tests/support/esp32_preflight.rs"]
mod esp32_preflight;

use esp32_preflight::{USB_UNCOUPLED_MM, assert_usb_preflight};
use serde_json::{Value, json};
use sparksmith_automation::Session;
use sparksmith_core::geometry::Point;
use sparksmith_core::issues::IssueScope;
use sparksmith_core::layout::outline::BoardEdit;
use sparksmith_core::layout::{self, CopperGraph};
use sparksmith_core::library::footprint_source::{Expr, SketchEdit, SketchRef, SolveStatus};
use sparksmith_core::library::{Outline, Segment};
use sparksmith_core::model::board::{
    BoardShape, BoardSketch, ConstraintTarget, CutoutKind, ShapeFault, ShapeFaultKind,
};
use sparksmith_core::model::dimension::BoardRef;
use sparksmith_core::model::fill::FillRegion;
use sparksmith_core::{Engine, Project, Query, QueryResult};
use sparksmith_solidcraft::linked_design::{Face, LinkedDesign, Operation as Extrude};
use std::collections::BTreeMap;
use std::path::Path;

type Fallible<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

const EXAMPLE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../examples/esp32-s31-devboard/esp32-s31-devboard.sparksmith"
);

/// The parts held at their offset from the receptacle, and from the module.
const BESIDE_J1: [&str; 8] = ["U3", "R4", "R5", "U2", "C1", "C2", "R2", "D1"];
const BESIDE_U1: [&str; 8] = ["C3", "R1", "C4", "R3", "D2", "SW1", "SW2", "J4"];

fn ok(session: &mut Session, request: Value) -> Fallible<Value> {
    let reply = session.dispatch(request);
    if reply["ok"] != true {
        return Err(format!("refused: {reply}").into());
    }
    Ok(reply["result"].clone())
}

fn apply(session: &mut Session, editor: &str, command: &str, params: Value) -> Fallible<Value> {
    let revision = session.engine().revision();
    ok(
        session,
        json!({"command": "project.apply", "params": {"expected_revision": revision,
            "operation": {"editor": editor, "operation": {"command": command, "params": params}}}}),
    )
}

/// The example, copied into `dir` and opened, its pours filled as opening
/// in the desktop fills them.
fn open(dir: &Path) -> Fallible<Session> {
    let path = dir.join("board.sparksmith");
    std::fs::copy(EXAMPLE, &path)?;
    let mut session = Session::new(Engine::default());
    ok(
        &mut session,
        json!({"command": "project.open", "params": {"expected_revision": 0,
            "path": path.display().to_string()}}),
    )?;
    refill(&mut session)?;
    Ok(session)
}

fn refill(session: &mut Session) -> Fallible {
    apply(
        session,
        "Fill",
        "fill.refill",
        json!({"target": {"fills": "all"}}),
    )?;
    Ok(())
}

/// Set board parameters in one `board.edit`: one undo step.
fn set(session: &mut Session, values: &[(&str, &str)]) -> Fallible<Value> {
    let edits: Vec<Value> = values
        .iter()
        .map(|(name, expr)| {
            json!({"op": "set_parameter", "param": {"name": name, "expr": expr, "unit": "mm",
                "comment": ""}})
        })
        .collect();
    apply(session, "Board", "board.edit", json!({"edits": edits}))
}

fn sketch(project: &Project) -> Fallible<&BoardSketch> {
    match &project.board_shape {
        BoardShape::Sketch(sketch) => Ok(sketch),
        BoardShape::Rectangle => Err("the board is a plain rectangle".into()),
    }
}

/// Every placed part's position, by designator.
fn positions(project: &Project) -> BTreeMap<String, Point> {
    project
        .placements
        .iter()
        .filter_map(|p| Some((project.instance(p.instance)?.designator.clone(), p.position)))
        .collect()
}

fn near(a: Point, b: Point) -> bool {
    (a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6
}

/// Whether the outline has a vertex at `at`.
fn has_vertex(outline: &Outline, at: Point) -> bool {
    std::iter::once(outline.start)
        .chain(outline.segments.iter().map(Segment::to))
        .any(|p| near(p, at))
}

/// The box of an outline's vertices and arcs.
fn extent(outline: &Outline) -> [Point; 2] {
    let ring = outline.flatten(0.001);
    let fold = |pick: fn(f64, f64) -> f64, start: f64, k: usize| {
        ring.iter().map(|p| p[k]).fold(start, pick)
    };
    [
        [
            fold(f64::min, f64::INFINITY, 0),
            fold(f64::min, f64::INFINITY, 1),
        ],
        [
            fold(f64::max, f64::NEG_INFINITY, 0),
            fold(f64::max, f64::NEG_INFINITY, 1),
        ],
    ]
}

fn within(found: [Point; 2], lo: Point, hi: Point, slack: f64) -> bool {
    (found[0][0] - lo[0]).abs() <= slack
        && (found[0][1] - lo[1]).abs() <= slack
        && (found[1][0] - hi[0]).abs() <= slack
        && (found[1][1] - hi[1]).abs() <= slack
}

/// The production check finds the one USB coverage warning and nothing
/// else, and no net is left unconnected.
fn assert_valid(session: &Session, what: &str) -> Fallible {
    let engine = session.engine();
    let QueryResult::Preflight(report) = engine.query(Query::Preflight { profile: None })? else {
        return Err("not a preflight report".into());
    };
    assert_usb_preflight(&report, USB_UNCOUPLED_MM);
    let lost: Vec<_> = layout::checks(engine.project(), &Default::default())
        .into_iter()
        .filter(|f| {
            matches!(
                f.code.as_str(),
                "board.unrouted" | "board.track_dangling" | "board.via_dangling"
            )
        })
        .collect();
    assert!(lost.is_empty(), "{what}: {lost:#?}");
    assert!(
        CopperGraph::of(engine.project()).airwires().is_empty(),
        "{what}: airwires"
    );
    Ok(())
}

/// One datum of `interface.list`, by its display name.
fn datum(session: &mut Session, name: &str) -> Fallible<Value> {
    let list = ok(
        session,
        json!({"command": "project.query", "params": {"query": "interface.list", "params": {}}}),
    )?;
    list["result"]["datums"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|d| d["name"] == name)
        .cloned()
        .ok_or_else(|| format!("no datum {name}").into())
}

/// The outline a profile datum publishes.
fn published_loop(datum: &Value) -> Fallible<Outline> {
    Ok(serde_json::from_value(
        datum["geometry"]["loops"][0].clone(),
    )?)
}

fn publish(session: &mut Session, design: &Path) -> Fallible<Value> {
    let revision = session.engine().revision();
    ok(
        session,
        json!({"command": "interface.publish", "params": {"path": design.display().to_string(),
            "tier": "interface", "expected_revision": revision}}),
    )
}

/// A linked design on the published interface: a wall
/// in front of the USB tab with a cutout projected from the receptacle's
/// opening. Returns it with the wall's own box faces.
fn linked_wall(design: &Path, opening: &str) -> Fallible<(LinkedDesign, Vec<String>)> {
    let mut linked = LinkedDesign::new("Assembly")?;
    let bodies = linked.link(&design.display().to_string(), "Board")?;
    assert!(bodies.iter().any(|b| b == opening), "{bodies:?}");
    let front = linked
        .faces(opening)?
        .into_iter()
        .find(|f| f.normal.is_some_and(|n| n[1] < -0.99))
        .ok_or("the opening's outer face")?;
    linked.solid_box("Wall", [-5.0, -3.0, -2.0], [60.0, 2.0, 12.0])?;
    let box_faces = linked.faces("Wall")?.into_iter().map(|f| f.name).collect();
    linked.projected(
        "USB opening",
        [[0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
        opening,
        front.point,
        2.0,
        Extrude::Cut,
    )?;
    let errors = linked.feature_errors();
    assert!(errors.is_empty(), "{errors:?}");
    Ok((linked, box_faces))
}

/// A boss `name` under the mounting hole at `at`, 2.7 mm across, in a
/// linked design: the hole's rim projected onto a plane 6 mm under the
/// board and extruded 4 mm. The rim is the reference that follows a moved
/// hole; SolidCraft keeps the first outline of the hole's wall projected
/// as a face (`LinkedDesign::projected`).
fn boss(linked: &mut LinkedDesign, name: &str, at: Point) -> Fallible<Vec<Face>> {
    linked.projected_edge(
        name,
        [[0.0, 0.0, -6.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        "board",
        [at[0] + 1.35, at[1], 0.0],
        4.0,
        Extrude::New,
    )?;
    let made = faces(linked, name)?;
    assert!(
        made.iter()
            .any(|f| f.normal.is_none() && near([f.centroid[0], f.centroid[1]], at)),
        "{name} stands under the hole at {at:?}: {made:?}"
    );
    Ok(made)
}

/// The name of the linked board's curved face that stands round `at`: a
/// mounting hole's wall, as the linked design names it.
fn hole_wall(linked: &mut LinkedDesign, at: Point) -> Fallible<String> {
    Ok(linked
        .faces("board")?
        .into_iter()
        .find(|f| f.normal.is_none() && near([f.centroid[0], f.centroid[1]], at))
        .ok_or_else(|| format!("no hole wall round {at:?}"))?
        .name)
}

fn faces(linked: &mut LinkedDesign, body: &str) -> Fallible<Vec<Face>> {
    let mut faces = linked.faces(body)?;
    faces.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(faces)
}

/// Each face of `after` moved by `by` from the face of that name in
/// `before`, within 1 µm.
fn assert_moved(before: &[Face], after: &[Face], by: [f64; 3], what: &str) {
    assert_eq!(before.len(), after.len(), "{what}: face count");
    for (a, b) in before.iter().zip(after) {
        assert_eq!(a.name, b.name, "{what}");
        let axes = a.centroid.iter().zip(&b.centroid).zip(&by);
        for (axis, ((from, to), want)) in axes.enumerate() {
            let moved = to - from;
            assert!(
                (moved - want).abs() <= 1e-3,
                "{what}: {} moved {moved} along axis {axis}, not {want}",
                a.name
            );
        }
    }
}

/// The sketch solves with no freedom left, no fault and no idle parameter;
/// every driving dimension of it is bound to a board parameter, so no point
/// is where it is by a number. The shape is the one described: arcs,
/// fillets, the tab and the notch, a slot, a cutout and three holes. Every
/// placed part is held by two part dimensions.
#[test]
fn the_outline_is_fully_defined_by_its_parameters_and_every_part_is_held() -> Fallible {
    let dir = tempfile::tempdir()?;
    let session = open(dir.path())?;
    let project = session.engine().project();
    let sketch = sketch(project)?;
    assert_eq!(sketch.solve.status, SolveStatus::Solved);
    assert_eq!(sketch.solve.dof, 0);
    assert!(sketch.fault.is_none(), "{:?}", sketch.fault);
    assert!(sketch.idle.is_empty(), "{:?}", sketch.idle);
    let stored: Value = serde_json::from_str(&sketch.sketch.0)?;
    let constraints = stored["constraints"].as_array().ok_or("constraints")?;
    let unbound: Vec<&Value> = constraints
        .iter()
        .filter(|k| k.get("value").is_some() && k.get("param").is_none())
        .collect();
    assert!(
        unbound.is_empty(),
        "dimensions held by a number: {unbound:?}"
    );
    let bound = constraints
        .iter()
        .filter(|k| k.get("param").is_some())
        .count();
    assert!(bound >= 30, "{bound} dimensions bound to parameters");
    // Only the origin is fixed in place.
    let fixed: Vec<&str> = stored["points"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|p| p["fixed"] == true)
        .filter_map(|p| p["id"].as_str())
        .collect();
    assert_eq!(fixed, ["origin"]);

    let board = project.board();
    let arcs = board
        .outer
        .segments
        .iter()
        .filter(|s| matches!(s, Segment::Arc { .. }))
        .count();
    // Six outside corners and four inside corners are rounded.
    assert_eq!((board.outer.segments.len(), arcs), (22, 10));
    assert!(within(extent(&board.outer), [0.0, 0.0], [40.0, 64.0], 1e-6));
    // The tab's square corners, 12 mm apart about the centre line, 1 mm
    // out of the shoulders; the antenna notch's floor, 5.5 mm down.
    for at in [[14.0, 0.0], [26.0, 0.0], [1.5, 1.0], [38.5, 1.0]] {
        assert!(has_vertex(&board.outer, at), "{at:?}");
    }
    for at in [[8.9, 58.5], [31.1, 58.5], [8.4, 59.0], [31.6, 59.0]] {
        assert!(has_vertex(&board.outer, at), "{at:?}");
    }
    let kinds: Vec<CutoutKind> = board.cutouts.iter().map(|c| c.kind).collect();
    assert_eq!(
        kinds,
        [CutoutKind::Slot { plated: false }, CutoutKind::Cutout]
    );
    let holes: Vec<(Point, f64)> = board.holes.iter().map(|h| (h.at, h.drill)).collect();
    assert_eq!(
        holes,
        [([3.5, 3.5], 2.7), ([36.5, 3.5], 2.7), ([3.5, 60.5], 2.7)]
    );

    // Every placed part is named by at least two part dimensions (one
    // along each axis), as their target or by a feature of theirs.
    for placement in project.placements.iter() {
        let target = ConstraintTarget::Placement(placement.instance);
        let held = project
            .part_dimensions
            .iter()
            .filter(|d| {
                d.refs
                    .iter()
                    .any(|r| matches!(r, BoardRef::Part(f) if f.target() == target))
            })
            .count();
        let name = &project
            .instance(placement.instance)
            .ok_or("an instance")?
            .designator;
        assert!(held >= 2, "{name} is held by {held} dimensions");
    }
    assert_valid(&session, "the shipped board")
}

/// Wider, taller, a wider tab, a shallower notch, larger corners, a longer
/// slot and a smaller window, in one step: the outline, the dimensioned
/// parts, the pours, the 3D model, the interface and a linked design on it
/// all follow, and the board is still a valid design. Back at the shipped
/// values, in one step, the board is the shipped board again.
#[test]
fn changing_the_driving_parameters_moves_everything_that_follows_them() -> Fallible {
    let dir = tempfile::tempdir()?;
    let mut session = open(dir.path())?;
    let start = session.engine().to_file();
    let was = positions(session.engine().project());
    let j1 = session
        .engine()
        .project()
        .instance_by_designator("J1")
        .ok_or("J1")?
        .id;
    let opening = format!("i{j1}.opening");

    // A linked design on the interface as published now.
    let design = dir.path().join("board.solidcraft");
    publish(&mut session, &design)?;
    let (mut linked, box_faces) = linked_wall(&design, &opening)?;
    let wall = faces(&mut linked, "Wall")?;
    let walls = [
        hole_wall(&mut linked, [3.5, 3.5])?,
        hole_wall(&mut linked, [36.5, 3.5])?,
        hole_wall(&mut linked, [3.5, 60.5])?,
    ];
    let board_faces = faces(&mut linked, "board")?.len();
    let bosses = [
        boss(&mut linked, "Boss 2", [36.5, 3.5])?,
        boss(&mut linked, "Boss 3", [3.5, 60.5])?,
    ];
    let warnings = linked.feature_warnings();
    assert!(warnings.is_empty(), "{warnings:?}");

    let (w, h) = (45.0, 70.0);
    set(
        &mut session,
        &[
            ("width", "45 mm"),
            ("height", "70 mm"),
            ("usb_tab_w", "14 mm"),
            ("ant_d", "5 mm"),
            ("corner_r", "2 mm"),
            ("slot_play", "1.5 mm"),
            ("win_h", "8 mm"),
        ],
    )?;

    // The outline: the same shape, on the new values.
    let project = session.engine().project();
    let sketch = sketch(project)?;
    assert_eq!(
        (sketch.solve.status, sketch.solve.dof),
        (SolveStatus::Solved, 0)
    );
    assert!(sketch.fault.is_none() && sketch.idle.is_empty());
    assert_eq!(project.board_size, [w, h]);
    let board = project.board().into_owned();
    assert_eq!(board.outer.segments.len(), 22);
    assert!(within(extent(&board.outer), [0.0, 0.0], [w, h], 1e-6));
    // The tab is 14 mm wide about the new centre line; the shoulders' outer
    // corners take the 2 mm radius.
    for at in [[15.5, 0.0], [29.5, 0.0], [2.0, 1.0], [43.0, 1.0]] {
        assert!(has_vertex(&board.outer, at), "{at:?}");
    }
    // The notch keeps its 23.2 mm between ears now 10.9 mm wide, its floor
    // 5 mm under the top edge.
    for at in [[11.4, 65.0], [33.6, 65.0], [10.9, 65.5], [34.1, 65.5]] {
        assert!(has_vertex(&board.outer, at), "{at:?}");
    }
    // The holes keep their inset from the edges that moved.
    let holes: Vec<Point> = board.holes.iter().map(|h| h.at).collect();
    assert!(
        near(holes[0], [3.5, 3.5])
            && near(holes[1], [w - 3.5, 3.5])
            && near(holes[2], [3.5, h - 3.5]),
        "{holes:?}"
    );
    // The slot: centred 3.5 mm from the right and top edges, its cap
    // centres 1.5 mm apart, 2.7 mm wide.
    let slot = extent(&board.cutouts[0].outline);
    assert!(
        within(
            slot,
            [w - 3.5 - 0.75 - 1.35, h - 3.5 - 1.35],
            [w - 3.5 + 0.75 + 1.35, h - 3.5 + 1.35],
            1e-3
        ),
        "{slot:?}"
    );
    // The window: 8 × 8 mm, its centre still 7 mm left of the centre line
    // and its lower side 19 mm above the tab's edge.
    let window = extent(&board.cutouts[1].outline);
    assert!(
        within(window, [w / 2.0 - 11.0, 19.0], [w / 2.0 - 3.0, 27.0], 1e-6),
        "{window:?}"
    );

    // The parts: the receptacle and the module on the centre line, the
    // receptacle's face on the tab's edge and the module's antenna end on
    // the top edge; the headers a header span apart about the centre line;
    // the fiducials 14 mm either side of it; every other part where it
    // was beside the receptacle or the module.
    let now = positions(project);
    let (dx, dy) = ((w - 40.0) / 2.0, h - 64.0);
    let expect = |part: &str, by: Point| -> Fallible {
        let (a, b) = (was[part], now[part]);
        assert!(
            near(b, [a[0] + by[0], a[1] + by[1]]),
            "{part} went from {a:?} to {b:?}"
        );
        Ok(())
    };
    assert!(near(now["J1"], [w / 2.0, 3.675]), "{:?}", now["J1"]);
    assert!(near(now["U1"], [w / 2.0, h - 15.0]), "{:?}", now["U1"]);
    assert!(
        near(now["J2"], [(w - 34.92) / 2.0, 21.0]),
        "{:?}",
        now["J2"]
    );
    assert!(
        near(now["J3"], [w - (w - 34.92) / 2.0, 21.0]),
        "{:?}",
        now["J3"]
    );
    assert!(near(now["FID1"], [w / 2.0 + 14.0, 6.0]));
    assert!(near(now["FID2"], [w / 2.0 - 14.0, 32.0]));
    assert!(near(now["FID3"], [w / 2.0 + 14.0, h - 19.0]));
    for part in BESIDE_J1 {
        expect(part, [dx, 0.0])?;
    }
    for part in BESIDE_U1 {
        expect(part, [dx, dy])?;
    }
    assert_eq!(now.len(), 23, "every placed part was checked");

    // The pours: each follows the new outline at once, and its copper is
    // stale until it is refilled; refilled, it reaches into the ground the
    // board gained and the board is a valid design.
    for fill in project.fills.iter() {
        assert_eq!(fill.region, FillRegion::Board);
        assert_eq!(fill.outline, board.outer, "{}", fill.name);
        assert!(layout::fill::is_stale(project, fill), "{}", fill.name);
    }
    refill(&mut session)?;
    let project = session.engine().project();
    for fill in project.fills.iter() {
        assert!(!layout::fill::is_stale(project, fill), "{}", fill.name);
        let copper = fill.computed.as_ref().ok_or("filled")?;
        let reach = copper
            .regions
            .iter()
            .flat_map(|r| r.outer.iter())
            .fold([0.0_f64, 0.0], |m, p| [m[0].max(p[0]), m[1].max(p[1])]);
        assert!(
            reach[0] > w - 0.5 && reach[0] < w && reach[1] > h - 0.5 && reach[1] < h,
            "{}: copper reaches {reach:?}",
            fill.name
        );
    }
    assert_valid(&session, "the changed board")?;

    // The 3D model: the plan's laminate spans the new outline, and the
    // exact export builds every planned body as planned.
    {
        let _context = sparksmith_core::library::standards::Context::fixture().map(|c| c.enter());
        let plan = sparksmith_solidcraft::project::plan::of(project)?;
        let slab = plan.laminate.first().ok_or("a laminate slab")?;
        let (lo, hi) = slab.bounds()?;
        assert!(
            lo[0].abs() < 1e-3
                && lo[1].abs() < 1e-3
                && (hi[0] - w).abs() < 1e-3
                && (hi[1] - h).abs() < 1e-3,
            "the laminate spans {lo:?} to {hi:?}"
        );
    }
    let model = dir.path().join("model.solidcraft");
    let exported = ok(
        &mut session,
        json!({"command": "project.export_native", "params": {"path": model.display().to_string()}}),
    )?;
    assert_eq!(exported["check"]["differs"], false, "{exported}");
    assert_eq!(exported["check"]["planned"], exported["check"]["built"]);
    assert!(model.exists());

    // The interface a linked design references: the outline, the cutouts,
    // the holes and the receptacle's opening, where the board has them now.
    let outline = published_loop(&datum(&mut session, "Board outline")?)?;
    assert_eq!(outline, board.outer);
    let slot_datum = published_loop(&datum(&mut session, "Slot 1")?)?;
    assert_eq!(slot_datum, board.cutouts[0].outline);
    for (name, at) in [
        ("Mounting hole 1", [3.5, 3.5]),
        ("Mounting hole 2", [w - 3.5, 3.5]),
        ("Mounting hole 3", [3.5, h - 3.5]),
    ] {
        let hole = datum(&mut session, name)?;
        let point: Vec<f64> = serde_json::from_value(hole["geometry"]["point"].clone())?;
        assert!(near([point[0], point[1]], at), "{name}: {point:?}");
        assert_eq!(hole["geometry"]["diameter"], 2.7);
    }
    let j1_opening = datum(&mut session, "J1 opening")?;
    let origin: Vec<f64> =
        serde_json::from_value(j1_opening["geometry"]["frame"]["origin"].clone())?;
    assert!(
        (origin[0] - w / 2.0).abs() < 1e-9 && origin[1].abs() < 1e-9,
        "{origin:?}"
    );

    // Published again and its links updated, the linked design's copy of the
    // board has every face under the name it had, the mounting holes'
    // walls where the holes are now, the bosses drawn from two holes' rims
    // stand under their holes, and the cutout projected from the
    // receptacle's opening follows the receptacle; the wall's own faces
    // stay.
    publish(&mut session, &design)?;
    let update = linked.update_links()?;
    assert!(
        update.failed.is_empty() && !update.updated.is_empty(),
        "{update:?}"
    );
    assert!(
        linked.feature_errors().is_empty(),
        "{:?}",
        linked.feature_errors()
    );
    assert_eq!(faces(&mut linked, "board")?.len(), board_faces);
    for (name, at) in walls
        .iter()
        .zip([[3.5, 3.5], [w - 3.5, 3.5], [3.5, h - 3.5]])
    {
        assert_eq!(
            &hole_wall(&mut linked, at)?,
            name,
            "the hole wall round {at:?}"
        );
    }
    let wall_now = faces(&mut linked, "Wall")?;
    assert_eq!(wall_now.len(), wall.len());
    let cut = |all: &[Face]| -> Vec<Face> {
        all.iter()
            .filter(|f| !box_faces.contains(&f.name))
            .cloned()
            .collect()
    };
    assert!(cut(&wall).len() >= 4, "the cutout's faces");
    assert_moved(&cut(&wall), &cut(&wall_now), [dx, 0.0, 0.0], "the cutout");
    for ((name, before), by) in ["Boss 2", "Boss 3"]
        .into_iter()
        .zip(&bosses)
        .zip([[w - 40.0, 0.0, 0.0], [0.0, h - 64.0, 0.0]])
    {
        assert_moved(before, &faces(&mut linked, name)?, by, name);
    }
    let warnings = linked.feature_warnings();
    assert!(warnings.is_empty(), "{warnings:?}");

    // Back at the shipped values, in one step: narrower and shorter than
    // the copper stood, which the parts carry back with them. The board
    // is the shipped board again.
    set(
        &mut session,
        &[
            ("width", "40 mm"),
            ("height", "64 mm"),
            ("usb_tab_w", "12 mm"),
            ("ant_d", "5.5 mm"),
            ("corner_r", "1.5 mm"),
            ("slot_play", "1 mm"),
            ("win_h", "10 mm"),
        ],
    )?;
    refill(&mut session)?;
    let project = session.engine().project();
    assert_eq!(project.board(), start.project.board());
    let back = positions(project);
    for (part, at) in &was {
        assert!(near(back[part], *at), "{part}: {:?}", back[part]);
    }
    assert_eq!(project.tracks.len(), start.project.tracks.len());
    assert_valid(&session, "the board back at its shipped values")
}

/// A change the design cannot take is refused with the reason, in words
/// that name what is in the way, and changes nothing; a change the shape
/// takes but the layout does not is reported by the production check.
#[test]
fn a_parameter_change_that_breaks_the_design_is_reported() -> Fallible {
    let dir = tempfile::tempdir()?;
    let mut session = open(dir.path())?;
    let before = session.engine().to_file();
    let revision = session.engine().revision();
    let refused = |session: &mut Session, name: &str, expr: &str, words: &[&str]| -> Fallible {
        let error = set(session, &[(name, expr)])
            .err()
            .ok_or_else(|| format!("{name} = {expr} was accepted"))?
            .to_string();
        for word in words {
            assert!(error.contains(word), "{name} = {expr}: {error}");
        }
        assert_eq!(session.engine().revision(), revision, "{name} = {expr}");
        assert!(session.engine().to_file() == before, "{name} = {expr}");
        Ok(())
    };
    // The header rows closer together than their routed buses allow: the
    // copper on their pads cannot follow.
    refused(
        &mut session,
        "header_span",
        "32.92 mm",
        &["Cannot keep connected copper on", "IO5", "board.carry"],
    )?;
    // An inside corner radius larger than the tab stands out.
    refused(
        &mut session,
        "inner_r",
        "2 mm",
        &["turn the shape inside out", "board.shape_unsolved"],
    )?;
    // A notch wider than the board, and a board narrower than its tab.
    refused(
        &mut session,
        "ant_w",
        "60 mm",
        &["cannot be satisfied", "board.shape_unsolved"],
    )?;
    refused(
        &mut session,
        "width",
        "20 mm",
        &["cannot be satisfied", "board.shape_unsolved"],
    )?;
    // A board narrower than the header rows are apart.
    refused(&mut session, "width", "30 mm", &["out of range"])?;

    // A notch 0.8 mm wider still solves, and reaches the BOOT switch: the
    // production check blocks on the pad it leaves too close to the edge.
    set(&mut session, &[("ant_w", "24 mm")])?;
    refill(&mut session)?;
    let QueryResult::Preflight(report) =
        session.engine().query(Query::Preflight { profile: None })?
    else {
        return Err("not a preflight report".into());
    };
    assert_eq!((report.errors, report.blocking), (1, 1), "{report:?}");
    let item = report
        .items
        .iter()
        .find(|i| i.code == "board.edge_clearance")
        .ok_or("an edge clearance finding")?;
    assert!(
        item.title
            .contains("SW2 pad 2 is 0.15 mm from the board edge"),
        "{}",
        item.title
    );
    Ok(())
}

/// The production check's report.
fn production(session: &mut Session) -> Fallible<Value> {
    let reply = ok(
        session,
        json!({"command": "project.query", "params": {
            "query": "fabrication.preflight", "params": {}}}),
    )?;
    Ok(reply["result"].clone())
}

/// The report's items of one code.
fn items(report: &Value, code: &str) -> Vec<Value> {
    report["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|i| i["code"] == code)
        .cloned()
        .collect()
}

/// An export is refused, as a folder and as a ZIP file, and writes nothing.
fn assert_not_exported(session: &mut Session, output: &Path) -> Fallible {
    for zip in [false, true] {
        let output = output.with_extension(if zip { "zip" } else { "files" });
        let reply = session.dispatch(json!({"command": "project.export_fabrication",
            "params": {"output": output.display().to_string(), "zip": zip}}));
        assert_eq!(reply["ok"], false, "{reply}");
        assert!(
            reply["error"].to_string().contains("fabrication.blocked"),
            "{reply}"
        );
        assert!(!output.exists(), "{} was written", output.display());
    }
    Ok(())
}

fn open_at(path: &Path) -> Fallible<Session> {
    let mut session = Session::new(Engine::default());
    ok(
        &mut session,
        json!({"command": "project.open", "params": {"expected_revision": 0,
            "path": path.display().to_string()}}),
    )?;
    Ok(session)
}

fn save_at(engine: Engine, path: &Path) -> Fallible {
    ok(
        &mut Session::new(engine),
        json!({"command": "project.save", "params": {"path": path.display().to_string()}}),
    )?;
    Ok(())
}

/// Apply one edit of the board sketch; the curves it created.
fn draw(session: &mut Session, edit: SketchEdit) -> Fallible<Vec<SketchRef>> {
    let edit = BoardEdit::Sketch { edit };
    let outcome = apply(session, "Board", "board.edit", json!({"edits": [edit]}))?;
    let created: Option<Vec<SketchRef>> =
        serde_json::from_value(outcome["outcome"]["created"].clone())?;
    Ok(created
        .unwrap_or_default()
        .into_iter()
        .filter(|r| matches!(r, SketchRef::Curve(_)))
        .collect())
}

/// A circle off the board: a second region, so the drawn curves are no board
/// and the last valid one stays.
fn second_region() -> SketchEdit {
    SketchEdit::AddCircle {
        center: [100.0, 20.0],
        radius: 2.0,
        construction: false,
    }
}

/// From the board of `session`, whose outline `code` reports, no way leads to
/// files: the waive command refuses, an export writes nothing, and a waiver a
/// saved project holds counts for nothing, then and when the outline is
/// `repair`ed, which exports, and drawn wrong `again`.
fn assert_never_waived(
    mut session: Session,
    dir: &Path,
    code: &str,
    repair: impl Fn(&mut Session) -> Fallible,
    again: impl Fn(&mut Session) -> Fallible,
) -> Fallible {
    let found = production(&mut session)?;
    let [outline] = &items(&found, code)[..] else {
        return Err(format!("{code}: {found}").into());
    };
    assert!(outline.get("waiver").is_none() && outline["severity"] == "error");
    assert!(outline["fix"].as_str().is_some_and(|f| !f.is_empty()));
    assert_not_exported(&mut session, &dir.join("fresh"))?;

    // The command refuses, by itself and among others, and changes nothing.
    let keys: Vec<String> = found["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|i| i["key"].as_str().map(str::to_owned))
        .collect();
    let key = outline["key"].as_str().ok_or("a key")?.to_owned();
    let revision = session.engine().revision();
    for asked in [vec![key.clone()], keys.clone()] {
        let reply = session.dispatch(json!({"command": "project.apply", "params": {
            "expected_revision": revision,
            "operation": {"editor": "Fabrication", "operation": {
                "command": "fabrication.waive",
                "params": {"keys": asked, "reason": "known"}}}}}));
        assert!(
            reply["error"]
                .to_string()
                .contains("fabrication.not_waivable"),
            "{reply}"
        );
    }
    assert_eq!(session.engine().revision(), revision);
    assert!(session.engine().project().production.waivers.is_empty());

    // A project saved with every finding waived, as one saved before the
    // rule may be.
    let mut file = session.engine().to_file();
    let mut all = keys;
    all.sort();
    all.dedup();
    file.project.production.waivers = all
        .into_iter()
        .map(|key| sparksmith_core::model::production::Waiver {
            key,
            reason: "known".into(),
        })
        .collect();
    let saved = dir.join("waived.sparksmith");
    save_at(Engine::from_file(file)?, &saved)?;
    let mut reopened = open_at(&saved)?;
    let held = |session: &mut Session, what: &str| -> Fallible {
        let project = session.engine().project();
        assert!(
            project.production.waivers.iter().any(|w| w.key == key),
            "{what}: the waiver is gone"
        );
        let found = production(session)?;
        let [outline] = &items(&found, code)[..] else {
            return Err(format!("{what}: {found}").into());
        };
        assert!(outline.get("waiver").is_none(), "{what}: {outline}");
        assert_eq!(outline["severity"], "error", "{what}");
        // Every other finding is waived: the outline alone blocks.
        assert_eq!(found["blocking"], 1, "{what}: {found}");
        let QueryResult::Issues(issues) = session.engine().query(Query::Issues {
            scope: IssueScope::All,
        })?
        else {
            return Err("not an issue report".into());
        };
        assert!(
            issues.rows.iter().any(|r| r.group.code == code),
            "{what}: not in Issues"
        );
        Ok(())
    };
    refill(&mut reopened)?;
    held(&mut reopened, "reopened")?;
    assert_not_exported(&mut reopened, &dir.join("reopened"))?;

    // Made valid and refilled, the same project writes its files.
    repair(&mut reopened)?;
    refill(&mut reopened)?;
    assert!(items(&production(&mut reopened)?, code).is_empty());
    let output = dir.join("valid");
    ok(
        &mut reopened,
        json!({"command": "project.export_fabrication",
            "params": {"output": output.display().to_string(), "zip": false}}),
    )?;
    assert!(output.exists());

    again(&mut reopened)?;
    refill(&mut reopened)?;
    held(&mut reopened, "wrong again")?;
    assert_not_exported(&mut reopened, &dir.join("again"))
}

/// An outline drawn wrong is never waived into files: a second region, which
/// leaves the board its last valid outline, and a line that closes nothing.
/// The held parts still solve, so nothing refuses the edit itself.
#[test]
fn an_invalid_outline_is_never_waived_into_files_of_the_last_one() -> Fallible {
    let stray = || SketchEdit::AddLine {
        a: [100.0, 20.0],
        b: [110.0, 20.0],
        construction: false,
    };
    type Draw = fn() -> SketchEdit;
    let cases: [(&str, bool, Draw); 2] = [
        ("board.shape_invalid", false, second_region),
        ("board.shape_open", true, stray),
    ];
    for (code, current, wrong) in cases {
        let dir = tempfile::tempdir()?;
        let mut session = open(dir.path())?;
        assert!(items(&production(&mut session)?, code).is_empty());
        let last = sketch(session.engine().project())?.solved.clone();
        let curves = draw(&mut session, wrong())?;
        assert!(!curves.is_empty(), "{code}");
        refill(&mut session)?;
        let drawn = sketch(session.engine().project())?;
        assert_eq!(drawn.current, current, "{code}");
        assert!(drawn.fault.is_some(), "{code}");
        if !current {
            assert_eq!(drawn.solved, last, "{code}: the last valid outline");
        }
        assert_never_waived(
            session,
            dir.path(),
            code,
            |s| {
                draw(
                    s,
                    SketchEdit::Delete {
                        ids: curves.clone(),
                    },
                )?;
                Ok(())
            },
            |s| draw(s, wrong()).map(|_| ()),
        )?;
    }
    Ok(())
}

/// GUI check K1: a project a build saved with `width` 30 mm, which its
/// outline cannot be, beside its last valid outline of 40 by 64 mm (the
/// command now refuses that width). It opens as it was saved, and nothing
/// but a valid outline makes files of it.
#[test]
fn a_project_saved_with_an_invalid_outline_opens_and_is_never_exported() -> Fallible {
    let dir = tempfile::tempdir()?;
    let mut file = open(dir.path())?.engine().to_file();
    let BoardShape::Sketch(stored) = &mut file.project.board_shape else {
        return Err("the board is a plain rectangle".into());
    };
    let width = stored
        .parameters
        .iter_mut()
        .find(|p| p.name.0 == "width")
        .ok_or("width")?;
    width.expr = Expr("30 mm".into());
    stored.current = false;
    stored.fault = Some(ShapeFault {
        kind: ShapeFaultKind::Invalid,
        message: "The board's curves cross or touch at (26.6, 61.85).".into(),
        at: vec![[26.6, 61.85]],
    });
    let saved = dir.path().join("saved.sparksmith");
    save_at(Engine::from_file(file)?, &saved)?;

    let mut session = open_at(&saved)?;
    refill(&mut session)?;
    let project = session.engine().project();
    assert_eq!(project.board_size, [40.0, 64.0]);
    assert!(project.board_shape.invalid().is_some());
    let width = sketch(project)?
        .parameters
        .iter()
        .find(|p| p.name.0 == "width")
        .ok_or("width")?;
    assert_eq!(width.expr.0, "30 mm");
    assert_never_waived(
        session,
        dir.path(),
        "board.shape_invalid",
        |s| set(s, &[("width", "40 mm")]).map(|_| ()),
        |s| draw(s, second_region()).map(|_| ()),
    )
}
