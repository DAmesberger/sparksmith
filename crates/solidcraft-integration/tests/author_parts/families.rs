//! The part files of the electromechanical generator families
//! (`generators/{tactile,reed,buzzer,electrolytic,power-inductor,standoff}`):
//! original parametric recreations drawn by Solidcraft commands over plain
//! parameters, each saved at the size of the manufacturer drawing it was
//! measured from (cited in the file's sidecar). Original work (MIT OR
//! Apache-2.0); no CAD or EDA library data was copied.
//!
//! Frame as every part file's: the origin at the footprint origin, z = 0
//! the seating plane, x along the package length. Bodies are disjoint: a
//! lead that enters a body cuts its root out of it (`keep_tools`), and
//! inlays (a polarity print, a label) are cut into the body they sit in.
use super::{MaterialId, assign_look, run};
use serde_json::{Value, json};
use solidcraft_engine::Session;

/// How a primitive combines.
enum Op<'a> {
    /// A new body of this name.
    New(&'a str),
    /// Joined to this body.
    Join(&'a str),
    /// Cut from these bodies.
    Cut(&'a [&'a str]),
}

fn with_op(mut p: Value, name: &str, op: &Op<'_>) -> Value {
    if let Some(map) = p.as_object_mut() {
        map.insert("name".into(), json!(name));
        match op {
            Op::New(body) => {
                map.insert("body_name".into(), json!(body));
            }
            Op::Join(target) => {
                map.insert("operation".into(), json!("join"));
                map.insert("targets".into(), json!([target]));
            }
            Op::Cut(targets) => {
                map.insert("operation".into(), json!("cut"));
                map.insert("targets".into(), json!(targets));
            }
        }
    }
    p
}

/// A box from `corner` of `size` (expressions).
fn bx(s: &mut Session, name: &str, op: Op<'_>, corner: [&str; 3], size: [&str; 3]) {
    let p = json!({"corner": corner, "length": size[0], "width": size[1], "height": size[2]});
    run(s, "solid.box", with_op(p, name, &op));
}

/// A box over x `x0`..`x1`, y `y0`..`y1`, z `z0`..`z1` (expressions).
fn span_box(s: &mut Session, name: &str, op: Op<'_>, [x0, x1, y0, y1, z0, z1]: [&str; 6]) {
    bx(
        s,
        name,
        op,
        [x0, y0, z0],
        [
            &format!("({x1}) - ({x0})"),
            &format!("({y1}) - ({y0})"),
            &format!("({z1}) - ({z0})"),
        ],
    );
}

/// A cylinder of `diameter` from `base` along `axis` (+z or +x), `height`
/// long: a circle on an XY or YZ sketch through `base`, extruded, as the
/// MELF part file draws its round bodies, so the body's bounds are its
/// exact outline (a cylinder primitive's bounds are its control hull,
/// about 1.39 radii out). With `taper` the walls draw in by that angle.
#[allow(clippy::too_many_arguments)]
fn disc(
    s: &mut Session,
    name: &str,
    op: Op<'_>,
    base: [&str; 3],
    axis: [i32; 3],
    diameter: &str,
    height: &str,
    taper: Option<&str>,
) {
    let (plane, offset, centre) = if axis[0] != 0 {
        ("YZ", base[0], [base[1], base[2]])
    } else {
        ("XY", base[2], [base[0], base[1]])
    };
    let reply = run(
        s,
        "sketch.create",
        json!({"plane": plane, "offset": offset, "name": format!("{name} sketch")}),
    );
    run(
        s,
        "sketch.draw.dimensioned",
        json!({"command": "sketch.circle.center",
            "params": {"center": centre, "radius": 0.5},
            "dimensions": {"diameter": diameter}}),
    );
    run(s, "sketch.finish", json!({}));
    let mut p = json!({"sketch": reply["sketch"], "distance": height});
    if let (Some(taper), Some(map)) = (taper, p.as_object_mut()) {
        map.insert("taper".into(), json!(taper));
    }
    run(s, "solid.extrude", with_op(p, name, &op));
}

/// A straight cylinder ([`disc`] without a taper).
fn cyl(
    s: &mut Session,
    name: &str,
    op: Op<'_>,
    base: [&str; 3],
    axis: [i32; 3],
    diameter: &str,
    height: &str,
) {
    disc(s, name, op, base, axis, diameter, height, None);
}

/// Cut `tools` from `target`, keeping the tools (a lead's root, an inlay).
fn cut_keep(s: &mut Session, name: &str, target: &str, tools: &[&str]) {
    run(
        s,
        "solid.combine",
        json!({"target": target, "tools": tools, "operation": "cut", "keep_tools": true,
            "name": name}),
    );
}

/// The user parameters `(name, expression, unit, comment)`.
fn params(s: &mut Session, rows: &[(&str, &str, &str, &str)]) {
    let rows: Vec<Value> = rows
        .iter()
        .map(|(name, expression, unit, comment)| {
            json!({"name": name, "expression": expression, "unit": unit, "comment": comment})
        })
        .collect();
    run(
        s,
        "parameters.import",
        json!({"text": serde_json::to_string(&rows).unwrap_or_default()}),
    );
}

fn new_design(title: &str) -> Session {
    let mut s = Session::default();
    run(&mut s, "file.new", json!({"name": title}));
    s
}

/// The `marking-<name>` plane at `origin`, u along +x, normal +z, sized by
/// `marking_<name>_u` × `_v`.
fn marking_up(s: &mut Session, name: &str, origin: [&str; 3], [u, v]: [&str; 2]) {
    let stem = name.replace('-', "_");
    params(
        s,
        &[
            (
                &format!("marking_{stem}_u"),
                u,
                "mm",
                "marking area along its u axis",
            ),
            (
                &format!("marking_{stem}_v"),
                v,
                "mm",
                "marking area across it",
            ),
        ],
    );
    run(
        s,
        "construct.plane.offset",
        json!({"base": {"origin": origin, "x_dir": [1, 0, 0], "y_dir": [0, 1, 0]},
            "offset": "0 mm", "name": format!("marking-{name}")}),
    );
}

/// Evaluate, refuse any failing feature, and return the design's bytes.
fn finish(s: &mut Session) -> Vec<u8> {
    s.refresh();
    let errors: Vec<String> = s
        .model
        .results
        .iter()
        .filter_map(|r| r.error.clone().map(|e| format!("{}: {e}", r.name)))
        .collect();
    assert!(errors.is_empty(), "{errors:?}");
    for b in &s.model.state().bodies {
        if let Ok(m) = b.measure() {
            eprintln!("  {}: {:.6} mm³", b.name, m.volume);
        }
    }
    solidcraft_engine::io::write_design(&s.doc).unwrap()
}

// ---------------------------------------------------------------------
// Tactile switches
// ---------------------------------------------------------------------

/// One J-bent terminal of the mini tactile switch at the `minus` (−x) or
/// +x end, centred at y `y`: a foot under the base from the tip inward,
/// the leg up the end, and the tab back into the base at `lead_rise`.
fn j_terminal(s: &mut Session, name: &str, minus: bool, y: &str) {
    let m = |e: &str| {
        if minus {
            format!("- ({e})")
        } else {
            e.to_owned()
        }
    };
    let (y0, y1) = (
        format!("({y}) - terminal_width / 2"),
        format!("({y}) + terminal_width / 2"),
    );
    let order = |a: String, b: String| if minus { (b, a) } else { (a, b) };
    let (fx0, fx1) = order(m("span / 2 - foot_length"), m("span / 2"));
    span_box(
        s,
        name,
        Op::New(name),
        [&fx0, &fx1, &y0, &y1, "0 mm", "lead_thickness"],
    );
    let (lx0, lx1) = order(m("span / 2 - lead_thickness"), m("span / 2"));
    span_box(
        s,
        &format!("{name} leg"),
        Op::Join(name),
        [&lx0, &lx1, &y0, &y1, "0 mm", "lead_rise"],
    );
    let (tx0, tx1) = order(m("body_length / 2 - lead_root"), m("span / 2"));
    span_box(
        s,
        &format!("{name} tab"),
        Op::Join(name),
        [
            &tx0,
            &tx1,
            &y0,
            &y1,
            "lead_rise - lead_thickness",
            "lead_rise",
        ],
    );
}

/// The `tactile` generator's mini top-push switch (Panasonic EVQP2 class):
/// a black LCP base on four J-bent tin terminals, a nickel-silver cover
/// whose side tabs fold down the long sides, and a white POM push plate
/// through the cover's opening, its upright edges rounded. With `ground`,
/// the cover's ground tab runs down the −y side to the board (the
/// EVQP2's unplated frame terminal). Terminals 1 and 2 at −x (y −, +),
/// 3 and 4 at +x. Saved at the EVQP2V02M of Panasonic's catalogue.
pub fn author_tactile_mini(ground: bool) -> Vec<u8> {
    let mut s = new_design(if ground {
        "Tactile switch, mini, top push, ground tab"
    } else {
        "Tactile switch, mini, top push"
    });
    params(
        &mut s,
        &[
            ("body_length", "4.7 mm", "mm", "mounting base along x"),
            ("body_width", "3.5 mm", "mm", "mounting base across (y)"),
            (
                "height",
                "2.1 mm",
                "mm",
                "push plate top above the seating plane",
            ),
            (
                "cover_height",
                "1.55 mm",
                "mm",
                "cover top above the seating plane",
            ),
            (
                "cover_thickness",
                "0.1 mm",
                "mm",
                "cover sheet thickness (est.)",
            ),
            ("span", "5.5 mm", "mm", "overall length over the terminals"),
            ("terminal_width", "0.6 mm", "mm", "terminal width"),
            (
                "terminal_pitch",
                "1.7 mm",
                "mm",
                "terminal centres across (y)",
            ),
            (
                "lead_thickness",
                "0.15 mm",
                "mm",
                "terminal sheet thickness (est.)",
            ),
            (
                "lead_rise",
                "0.5 mm",
                "mm",
                "height of the J's tab above the seating plane (est.)",
            ),
            (
                "foot_length",
                "0.7 mm",
                "mm",
                "foot length under the base from the tip (est.)",
            ),
            (
                "lead_root",
                "0.3 mm",
                "mm",
                "how far a tab reaches into the base (est.)",
            ),
            ("plate_length", "2.6 mm", "mm", "push plate along x"),
            ("plate_width", "1.6 mm", "mm", "push plate across (y)"),
            (
                "plate_round",
                "0.5 mm",
                "mm",
                "round of the push plate's upright edges",
            ),
            (
                "plate_clearance",
                "0.1 mm",
                "mm",
                "cover opening clearance round the plate (est.)",
            ),
            (
                "tab_length",
                "0.8 mm",
                "mm",
                "cover side tab along x (est.)",
            ),
            (
                "tab_drop",
                "0.7 mm",
                "mm",
                "how far the cover's side tabs reach down (est.)",
            ),
            ("ground_length", "1 mm", "mm", "ground tab along x"),
            (
                "ground_out",
                "0.3 mm",
                "mm",
                "ground tab reach beyond the −y side",
            ),
        ],
    );
    let (bl, bw, ch, ct) = (4.7, 3.5, 1.55, 0.1);
    // The base, narrowed by the cover sheet each side for the side tabs.
    bx(
        &mut s,
        "base",
        Op::New("base"),
        [
            "- body_length / 2",
            "- body_width / 2 + cover_thickness",
            "lead_thickness",
        ],
        [
            "body_length",
            "body_width - 2 * cover_thickness",
            "cover_height - cover_thickness - lead_thickness",
        ],
    );
    // The base's upright corners, softened (picked at the saved size).
    let corners: Vec<[f64; 3]> = [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]]
        .iter()
        .map(|[x, y]| [x * bl / 2.0, y * (bw / 2.0 - ct), ch / 2.0])
        .collect();
    run(
        &mut s,
        "solid.fillet",
        json!({"name": "base corners", "body": "base", "radius": "0.15 mm", "edges": corners}),
    );
    // The cover sheet with its opening, and the side tabs.
    bx(
        &mut s,
        "cover",
        Op::New("cover"),
        [
            "- body_length / 2",
            "- body_width / 2",
            "cover_height - cover_thickness",
        ],
        ["body_length", "body_width", "cover_thickness"],
    );
    for (k, x) in [(1, "-"), (2, "")] {
        for (side, y0, y1) in [
            (
                "-y",
                "- body_width / 2",
                "- body_width / 2 + cover_thickness",
            ),
            ("+y", "body_width / 2 - cover_thickness", "body_width / 2"),
        ] {
            let x0 = format!("{x} body_length / 4 - tab_length / 2");
            let x1 = format!("{x} body_length / 4 + tab_length / 2");
            span_box(
                &mut s,
                &format!("cover tab {k} {side}"),
                Op::Join("cover"),
                [
                    &x0,
                    &x1,
                    y0,
                    y1,
                    "cover_height - cover_thickness - tab_drop",
                    "cover_height - cover_thickness",
                ],
            );
        }
    }
    if ground {
        // The frame's ground tab: down the −y side at the centre, then
        // out along the board.
        span_box(
            &mut s,
            "ground tab",
            Op::Join("cover"),
            [
                "- ground_length / 2",
                "ground_length / 2",
                "- body_width / 2",
                "- body_width / 2 + cover_thickness",
                "0 mm",
                "cover_height - cover_thickness",
            ],
        );
        span_box(
            &mut s,
            "ground foot",
            Op::Join("cover"),
            [
                "- ground_length / 2",
                "ground_length / 2",
                "- body_width / 2 - ground_out",
                "- body_width / 2",
                "0 mm",
                "cover_thickness",
            ],
        );
    }
    bx(
        &mut s,
        "cover opening",
        Op::Cut(&["cover"]),
        [
            "- plate_length / 2 - plate_clearance",
            "- plate_width / 2 - plate_clearance",
            "cover_height - 2 * cover_thickness",
        ],
        [
            "plate_length + 2 * plate_clearance",
            "plate_width + 2 * plate_clearance",
            "3 * cover_thickness",
        ],
    );
    // The push plate, standing on the base through the opening.
    bx(
        &mut s,
        "push plate",
        Op::New("push plate"),
        [
            "- plate_length / 2",
            "- plate_width / 2",
            "cover_height - cover_thickness",
        ],
        [
            "plate_length",
            "plate_width",
            "height - cover_height + cover_thickness",
        ],
    );
    let (pl, pw, h) = (2.6, 1.6, 2.1);
    let plate_corners: Vec<[f64; 3]> = [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]]
        .iter()
        .map(|[x, y]| [x * pl / 2.0, y * pw / 2.0, (ch + h) / 2.0])
        .collect();
    run(
        &mut s,
        "solid.fillet",
        json!({"name": "plate corners", "body": "push plate", "radius": "plate_round",
            "edges": plate_corners}),
    );
    run(
        &mut s,
        "solid.fillet",
        json!({"name": "plate top", "body": "push plate", "radius": "0.08 mm",
            "edges": [[0.0, -pw / 2.0, h]]}),
    );
    for (k, minus, y) in [
        (1, true, "- terminal_pitch / 2"),
        (2, true, "terminal_pitch / 2"),
        (3, false, "- terminal_pitch / 2"),
        (4, false, "terminal_pitch / 2"),
    ] {
        j_terminal(&mut s, &format!("terminal {k}"), minus, y);
    }
    cut_keep(
        &mut s,
        "terminal roots",
        "base",
        &["terminal 1", "terminal 2", "terminal 3", "terminal 4"],
    );
    assign_look(&mut s, &["base"], MaterialId::Lcp);
    assign_look(&mut s, &["cover"], MaterialId::NickelSilver);
    assign_look(&mut s, &["push plate"], MaterialId::PomWhite);
    assign_look(
        &mut s,
        &["terminal 1", "terminal 2", "terminal 3", "terminal 4"],
        MaterialId::TinMatte,
    );
    marking_up(
        &mut s,
        "top",
        ["0 mm", "0 mm", "height"],
        ["0.6 * plate_length", "0.6 * plate_width"],
    );
    finish(&mut s)
}

/// One gull-wing terminal of the square tactile switch: out of the base's
/// side at `lead_exit`, down beside it and out along the board to the tip.
fn gull_terminal(s: &mut Session, name: &str, minus: bool, y: &str) {
    let m = |e: &str| {
        if minus {
            format!("- ({e})")
        } else {
            e.to_owned()
        }
    };
    let order = |a: String, b: String| if minus { (b, a) } else { (a, b) };
    let (y0, y1) = (
        format!("({y}) - terminal_width / 2"),
        format!("({y}) + terminal_width / 2"),
    );
    let (fx0, fx1) = order(m("body_length / 2 + lead_gap"), m("span / 2"));
    span_box(
        s,
        name,
        Op::New(name),
        [&fx0, &fx1, &y0, &y1, "0 mm", "lead_thickness"],
    );
    let (lx0, lx1) = order(
        m("body_length / 2 + lead_gap"),
        m("body_length / 2 + lead_gap + lead_thickness"),
    );
    span_box(
        s,
        &format!("{name} leg"),
        Op::Join(name),
        [&lx0, &lx1, &y0, &y1, "0 mm", "lead_exit"],
    );
    let (tx0, tx1) = order(
        m("body_length / 2 - lead_root"),
        m("body_length / 2 + lead_gap + lead_thickness"),
    );
    span_box(
        s,
        &format!("{name} tab"),
        Op::Join(name),
        [
            &tx0,
            &tx1,
            &y0,
            &y1,
            "lead_exit - lead_thickness",
            "lead_exit",
        ],
    );
}

/// The `tactile` generator's square top-push switch (6 × 6 and 4.5 × 4.5
/// mm classes): a black PA9T base, a nickel-silver cover whose side tabs
/// fold down the ±y sides and whose four corner studs stand up, a round
/// black plunger and four gull-wing tin terminals at ±x. Terminals 1 (−x)
/// and 2 (+x) at −y, 3 and 4 at +y. Saved at E-Switch's TL3305A (4.5 ×
/// 4.5 mm, 3.8 mm).
pub fn author_tactile_square() -> Vec<u8> {
    let mut s = new_design("Tactile switch, square, top push");
    params(
        &mut s,
        &[
            ("body_length", "4.5 mm", "mm", "base along x"),
            (
                "body_width",
                "4.5 mm",
                "mm",
                "base across (y), over the cover tabs",
            ),
            (
                "body_height",
                "1.75 mm",
                "mm",
                "cover top above the seating plane (est.)",
            ),
            (
                "height",
                "3.8 mm",
                "mm",
                "plunger top above the seating plane",
            ),
            ("plunger_diameter", "2.5 mm", "mm", "plunger diameter"),
            (
                "cover_thickness",
                "0.15 mm",
                "mm",
                "cover sheet thickness (est.)",
            ),
            (
                "stud_diameter",
                "0.7 mm",
                "mm",
                "cover corner stud diameter (est.)",
            ),
            (
                "stud_height",
                "0.25 mm",
                "mm",
                "cover corner stud height (est.)",
            ),
            ("span", "7.5 mm", "mm", "overall length over the terminals"),
            ("terminal_width", "0.7 mm", "mm", "terminal width"),
            (
                "terminal_pitch",
                "3 mm",
                "mm",
                "terminal centres across (y)",
            ),
            ("lead_thickness", "0.3 mm", "mm", "terminal sheet thickness"),
            (
                "lead_exit",
                "0.7 mm",
                "mm",
                "terminal top where it leaves the base (est.)",
            ),
            (
                "lead_gap",
                "0.3 mm",
                "mm",
                "gap between the base and a terminal's leg (est.)",
            ),
            (
                "lead_root",
                "0.3 mm",
                "mm",
                "how far a terminal reaches into the base (est.)",
            ),
            (
                "tab_length",
                "1.2 mm",
                "mm",
                "cover side tab along x (est.)",
            ),
        ],
    );
    let (bl, bw, bh, ct) = (4.5, 4.5, 1.75, 0.15);
    bx(
        &mut s,
        "base",
        Op::New("base"),
        [
            "- body_length / 2",
            "- body_width / 2 + cover_thickness",
            "0 mm",
        ],
        [
            "body_length",
            "body_width - 2 * cover_thickness",
            "body_height - cover_thickness",
        ],
    );
    let corners: Vec<[f64; 3]> = [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]]
        .iter()
        .map(|[x, y]| [x * bl / 2.0, y * (bw / 2.0 - ct), bh / 2.0])
        .collect();
    run(
        &mut s,
        "solid.fillet",
        json!({"name": "base corners", "body": "base", "radius": "0.2 mm", "edges": corners}),
    );
    bx(
        &mut s,
        "cover",
        Op::New("cover"),
        [
            "- body_length / 2",
            "- body_width / 2",
            "body_height - cover_thickness",
        ],
        ["body_length", "body_width", "cover_thickness"],
    );
    for (side, y0, y1) in [
        (
            "-y",
            "- body_width / 2",
            "- body_width / 2 + cover_thickness",
        ),
        ("+y", "body_width / 2 - cover_thickness", "body_width / 2"),
    ] {
        span_box(
            &mut s,
            &format!("cover tab {side}"),
            Op::Join("cover"),
            [
                "- tab_length / 2",
                "tab_length / 2",
                y0,
                y1,
                "lead_exit",
                "body_height - cover_thickness",
            ],
        );
    }
    for (k, x, y) in [(1, "-", "-"), (2, "", "-"), (3, "", ""), (4, "-", "")] {
        let at = format!("{x} (body_length / 2 - stud_diameter)");
        let at_y = format!("{y} (body_width / 2 - stud_diameter)");
        cyl(
            &mut s,
            &format!("cover stud {k}"),
            Op::Join("cover"),
            [&at, &at_y, "body_height"],
            [0, 0, 1],
            "stud_diameter",
            "stud_height",
        );
    }
    cyl(
        &mut s,
        "cover opening",
        Op::Cut(&["cover"]),
        ["0 mm", "0 mm", "body_height - 2 * cover_thickness"],
        [0, 0, 1],
        "plunger_diameter + 0.3 mm",
        "3 * cover_thickness",
    );
    cyl(
        &mut s,
        "plunger",
        Op::New("plunger"),
        ["0 mm", "0 mm", "body_height - cover_thickness"],
        [0, 0, 1],
        "plunger_diameter",
        "height - body_height + cover_thickness",
    );
    let h = 3.8;
    run(
        &mut s,
        "solid.fillet",
        json!({"name": "plunger top", "body": "plunger", "radius": "0.15 mm",
            "edges": [[1.25, 0.0, h]]}),
    );
    for (k, minus, y) in [
        (1, true, "- terminal_pitch / 2"),
        (2, false, "- terminal_pitch / 2"),
        (3, true, "terminal_pitch / 2"),
        (4, false, "terminal_pitch / 2"),
    ] {
        gull_terminal(&mut s, &format!("terminal {k}"), minus, y);
    }
    cut_keep(
        &mut s,
        "terminal roots",
        "base",
        &["terminal 1", "terminal 2", "terminal 3", "terminal 4"],
    );
    assign_look(&mut s, &["base", "plunger"], MaterialId::Pa9t);
    assign_look(&mut s, &["cover"], MaterialId::NickelSilver);
    assign_look(
        &mut s,
        &["terminal 1", "terminal 2", "terminal 3", "terminal 4"],
        MaterialId::TinMatte,
    );
    marking_up(
        &mut s,
        "top",
        ["0 mm", "0 mm", "height"],
        ["0.6 * plunger_diameter", "0.6 * plunger_diameter"],
    );
    finish(&mut s)
}

// ---------------------------------------------------------------------
// Reed switch
// ---------------------------------------------------------------------

/// The `reed` generator's surface-mount reed switch (Littelfuse MDSM-4
/// class): a borosilicate glass envelope (a tube whose ends taper to the
/// seals, hollow inside) on its axis `height − tube_diameter / 2` above the
/// board, and two tin-plated leads: each a round wire through its seal
/// into the envelope, flattened there into a reed blade (the two overlap
/// at the centre, one above the axis and one below, a contact gap apart),
/// bent down outside the glass to a flattened foot. Saved at the MDSM-4R.
pub fn author_reed() -> Vec<u8> {
    let mut s = new_design("Reed switch, surface mount");
    params(
        &mut s,
        &[
            ("tube_length", "15.24 mm", "mm", "glass envelope length"),
            ("tube_diameter", "2.28 mm", "mm", "glass envelope diameter"),
            (
                "height",
                "2.84 mm",
                "mm",
                "envelope top above the seating plane",
            ),
            (
                "bend_span",
                "17.52 mm",
                "mm",
                "over the leads' downward legs",
            ),
            ("span", "19.3 mm", "mm", "overall length over the feet"),
            ("lead_diameter", "0.5 mm", "mm", "lead wire diameter (est.)"),
            ("foot_width", "1 mm", "mm", "flattened foot width"),
            (
                "foot_thickness",
                "0.2 mm",
                "mm",
                "flattened foot thickness (est.)",
            ),
            (
                "seal_length",
                "1.6 mm",
                "mm",
                "tapered seal length at each end (est.)",
            ),
            (
                "seal_diameter",
                "1 mm",
                "mm",
                "diameter at the seal's tip (est.)",
            ),
            ("wall", "0.3 mm", "mm", "glass wall thickness (est.)"),
            ("blade_width", "0.45 mm", "mm", "reed blade width (est.)"),
            (
                "blade_thickness",
                "0.15 mm",
                "mm",
                "reed blade thickness (est.)",
            ),
            (
                "blade_overlap",
                "0.8 mm",
                "mm",
                "reed blades' overlap at the centre (est.)",
            ),
            (
                "contact_gap",
                "0.05 mm",
                "mm",
                "gap between the open blades (est.)",
            ),
            (
                "axis_z",
                "height - tube_diameter / 2",
                "mm",
                "envelope axis height",
            ),
            (
                "seal_taper",
                "atan2((tube_diameter - seal_diameter) / 2, seal_length)",
                "deg",
                "the seals' taper",
            ),
            (
                "leg_x",
                "bend_span / 2 - lead_diameter / 2",
                "mm",
                "leg centre x",
            ),
            (
                "blade_tip",
                "blade_overlap / 2",
                "mm",
                "how far a blade reaches past the centre",
            ),
        ],
    );
    // The envelope: a tube between two cones, hollowed.
    cyl(
        &mut s,
        "glass tube",
        Op::New("glass"),
        ["- tube_length / 2 + seal_length", "0 mm", "axis_z"],
        [1, 0, 0],
        "tube_diameter",
        "tube_length - 2 * seal_length",
    );
    // The seals: from the tip's diameter at one end out to the tube's, and
    // from the tube's in to the tip's at the other.
    for (k, base, from, taper) in [
        (1, "- tube_length / 2", "seal_diameter", "seal_taper"),
        (
            2,
            "tube_length / 2 - seal_length",
            "tube_diameter",
            "- seal_taper",
        ),
    ] {
        disc(
            &mut s,
            &format!("glass seal {k}"),
            Op::Join("glass"),
            [base, "0 mm", "axis_z"],
            [1, 0, 0],
            from,
            "seal_length",
            Some(taper),
        );
    }
    cyl(
        &mut s,
        "glass cavity",
        Op::Cut(&["glass"]),
        ["- tube_length / 2 + seal_length", "0 mm", "axis_z"],
        [1, 0, 0],
        "tube_diameter - 2 * wall",
        "tube_length - 2 * seal_length",
    );
    for (k, minus) in [(1, true), (2, false)] {
        let name = format!("terminal {k}");
        let sg = if minus { "-" } else { "" };
        let m = |e: &str| format!("{sg} ({e})");
        // The foot, the leg up to the axis, a ball at the bend, the wire
        // along the axis through the seal, and the blade.
        let (fx0, fx1) = if minus {
            (m("span / 2"), m("bend_span / 2 - lead_diameter"))
        } else {
            (m("bend_span / 2 - lead_diameter"), m("span / 2"))
        };
        span_box(
            &mut s,
            &name,
            Op::New(&name),
            [
                &fx0,
                &fx1,
                "- foot_width / 2",
                "foot_width / 2",
                "0 mm",
                "foot_thickness",
            ],
        );
        cyl(
            &mut s,
            &format!("{name} leg"),
            Op::Join(&name),
            [&m("leg_x"), "0 mm", "foot_thickness / 2"],
            [0, 0, 1],
            "lead_diameter",
            "axis_z - foot_thickness / 2",
        );
        run(
            &mut s,
            "solid.sphere",
            with_op(
                json!({"center": [m("leg_x"), "0 mm", "axis_z"], "diameter": "lead_diameter"}),
                &format!("{name} bend"),
                &Op::Join(&name),
            ),
        );
        let wire_to = "tube_length / 2 - seal_length + 0.2 mm";
        let (wx, wl) = if minus {
            (m("leg_x"), format!("leg_x - ({wire_to})"))
        } else {
            (wire_to.to_owned(), format!("leg_x - ({wire_to})"))
        };
        cyl(
            &mut s,
            &format!("{name} wire"),
            Op::Join(&name),
            [&wx, "0 mm", "axis_z"],
            [1, 0, 0],
            "lead_diameter",
            &wl,
        );
        // Blade 1 above the axis, blade 2 below, the gap between them.
        let z0 = if minus {
            "axis_z + contact_gap / 2"
        } else {
            "axis_z - contact_gap / 2 - blade_thickness"
        };
        let (bx0, bx1) = if minus {
            (m(wire_to), "blade_tip".to_owned())
        } else {
            ("- blade_tip".to_owned(), wire_to.to_owned())
        };
        span_box(
            &mut s,
            &format!("{name} blade"),
            Op::Join(&name),
            [
                &bx0,
                &bx1,
                "- blade_width / 2",
                "blade_width / 2",
                z0,
                &format!("({z0}) + blade_thickness"),
            ],
        );
    }
    cut_keep(&mut s, "seals", "glass", &["terminal 1", "terminal 2"]);
    assign_look(&mut s, &["glass"], MaterialId::BorosilicateGlass);
    assign_look(&mut s, &["terminal 1", "terminal 2"], MaterialId::TinMatte);
    finish(&mut s)
}

// ---------------------------------------------------------------------
// Magnetic buzzer
// ---------------------------------------------------------------------

/// The `buzzer` generator's surface-mount magnetic buzzer (CUI CMI-9653S
/// class): a square black LCP case with one upright corner bevelled (the
/// orientation corner, at +x +y), the sound port in the top centre, and
/// two gold terminals at ±x: each a plate up the case's end and a foot
/// under it. With `label`, the polyimide masking label over the top, cut
/// round the port. Terminal 1 (+) at −x. Saved at the CMI-9653S-SMT.
pub fn author_buzzer(label: bool) -> Vec<u8> {
    let mut s = new_design(if label {
        "Magnetic buzzer, surface mount, masking label"
    } else {
        "Magnetic buzzer, surface mount"
    });
    params(
        &mut s,
        &[
            ("body_length", "9.6 mm", "mm", "case along x"),
            ("body_width", "9.6 mm", "mm", "case across (y)"),
            ("height", "5 mm", "mm", "top above the seating plane"),
            (
                "corner_bevel",
                "1.2 mm",
                "mm",
                "the orientation corner's bevel (est.)",
            ),
            (
                "edge_round",
                "0.3 mm",
                "mm",
                "round of the case's top edges (est.)",
            ),
            (
                "port_diameter",
                "1.5 mm",
                "mm",
                "sound port diameter (est.)",
            ),
            ("port_depth", "1 mm", "mm", "sound port depth (est.)"),
            ("terminal_width", "1.5 mm", "mm", "terminal width (y)"),
            (
                "terminal_rise",
                "2.4 mm",
                "mm",
                "terminal plate height up the case end",
            ),
            (
                "terminal_out",
                "0.25 mm",
                "mm",
                "terminal plate thickness beyond the case end",
            ),
            (
                "terminal_depth",
                "1.2 mm",
                "mm",
                "foot length under the case (est.)",
            ),
            (
                "lead_thickness",
                "0.1 mm",
                "mm",
                "foot thickness under the case",
            ),
            (
                "label_diameter",
                "7.6 mm",
                "mm",
                "masking label diameter (est.)",
            ),
            (
                "label_thickness",
                "0.05 mm",
                "mm",
                "masking label thickness (est.)",
            ),
        ],
    );
    let (bl, bw, h) = (9.6, 9.6, 5.0);
    let top = if label {
        "height - label_thickness"
    } else {
        "height"
    };
    bx(
        &mut s,
        "case",
        Op::New("case"),
        ["- body_length / 2", "- body_width / 2", "lead_thickness"],
        [
            "body_length",
            "body_width",
            &format!("{top} - lead_thickness"),
        ],
    );
    let top_z = if label { h - 0.05 } else { h };
    run(
        &mut s,
        "solid.chamfer",
        json!({"name": "orientation corner", "body": "case", "distance": "corner_bevel",
            "edges": [[bl / 2.0, bw / 2.0, h / 2.0]]}),
    );
    run(
        &mut s,
        "solid.fillet",
        json!({"name": "top edges", "body": "case", "radius": "edge_round",
            "edges": [[0.0, -bw / 2.0, top_z], [bl / 2.0, 0.0, top_z], [0.0, bw / 2.0, top_z],
                [-bl / 2.0, 0.0, top_z]]}),
    );
    cyl(
        &mut s,
        "sound port",
        Op::Cut(&["case"]),
        ["0 mm", "0 mm", &format!("{top} - port_depth")],
        [0, 0, 1],
        "port_diameter",
        "port_depth + 1 mm",
    );
    for (k, minus) in [(1, true), (2, false)] {
        let name = format!("terminal {k}");
        let sg = if minus { "-" } else { "" };
        let m = |e: &str| format!("{sg} ({e})");
        let (px0, px1) = if minus {
            (m("body_length / 2 + terminal_out"), m("body_length / 2"))
        } else {
            (m("body_length / 2"), m("body_length / 2 + terminal_out"))
        };
        span_box(
            &mut s,
            &name,
            Op::New(&name),
            [
                &px0,
                &px1,
                "- terminal_width / 2",
                "terminal_width / 2",
                "0 mm",
                "terminal_rise",
            ],
        );
        let (fx0, fx1) = if minus {
            (
                m("body_length / 2 + terminal_out"),
                m("body_length / 2 - terminal_depth"),
            )
        } else {
            (
                m("body_length / 2 - terminal_depth"),
                m("body_length / 2 + terminal_out"),
            )
        };
        span_box(
            &mut s,
            &format!("{name} foot"),
            Op::Join(&name),
            [
                &fx0,
                &fx1,
                "- terminal_width / 2",
                "terminal_width / 2",
                "0 mm",
                "lead_thickness",
            ],
        );
    }
    assign_look(&mut s, &["case"], MaterialId::Lcp);
    assign_look(&mut s, &["terminal 1", "terminal 2"], MaterialId::Gold);
    if label {
        cyl(
            &mut s,
            "label",
            Op::New("label"),
            ["0 mm", "0 mm", "height - label_thickness"],
            [0, 0, 1],
            "label_diameter",
            "label_thickness",
        );
        cyl(
            &mut s,
            "label port",
            Op::Cut(&["label"]),
            ["0 mm", "0 mm", "height - 2 * label_thickness"],
            [0, 0, 1],
            "port_diameter",
            "3 * label_thickness",
        );
        assign_look(&mut s, &["label"], MaterialId::Polyimide);
    }
    marking_up(
        &mut s,
        "top",
        ["0 mm", "0 mm", "height"],
        ["0.6 * body_length", "0.6 * body_width"],
    );
    finish(&mut s)
}

// ---------------------------------------------------------------------
// Aluminium electrolytic capacitor
// ---------------------------------------------------------------------

/// The `electrolytic` generator's surface-mount aluminium electrolytic
/// capacitor (vertical chip, EIAJ sizes): a black PA9T seat plate whose
/// two corners at terminal 1 (+, −x) are bevelled, two flat tin terminals
/// in grooves under it, and the aluminium can on it: rolled seal bead near
/// the bottom, rounded rim, the negative print (black ink, inlaid) over the
/// +x part of the top, and with `vent` the scored cross of the pressure
/// relief vent. Saved at Panasonic's size code D (6.3 × 5.8 mm).
pub fn author_electrolytic(vent: bool) -> Vec<u8> {
    let mut s = new_design(if vent {
        "Aluminium electrolytic capacitor, SMD, vent"
    } else {
        "Aluminium electrolytic capacitor, SMD"
    });
    params(
        &mut s,
        &[
            ("can_diameter", "6.3 mm", "mm", "can diameter (D)"),
            (
                "height",
                "5.8 mm",
                "mm",
                "overall height above the seating plane (L)",
            ),
            ("base_length", "6.6 mm", "mm", "seat plate along x (A)"),
            ("base_width", "6.6 mm", "mm", "seat plate across (B)"),
            ("base_height", "1 mm", "mm", "seat plate height (est.)"),
            (
                "base_bevel",
                "1 mm",
                "mm",
                "bevel of the seat plate's + corners (est.)",
            ),
            (
                "span",
                "7.8 mm",
                "mm",
                "overall length over the terminals (H)",
            ),
            (
                "terminal_gap",
                "1.8 mm",
                "mm",
                "gap between the terminals (P)",
            ),
            ("terminal_width", "0.65 mm", "mm", "terminal width (W)"),
            (
                "lead_thickness",
                "0.3 mm",
                "mm",
                "terminal thickness (est.)",
            ),
            (
                "bead_height",
                "0.8 mm",
                "mm",
                "seal bead centre above the seat plate (est.)",
            ),
            ("bead_depth", "0.2 mm", "mm", "seal bead depth (est.)"),
            (
                "rim_round",
                "0.35 mm",
                "mm",
                "round of the can's rim (est.)",
            ),
            (
                "print_from",
                "0.15 * can_diameter",
                "mm",
                "x where the negative print begins (est.)",
            ),
            (
                "print_thickness",
                "0.02 mm",
                "mm",
                "negative print ink thickness (est.)",
            ),
            (
                "vent_length",
                "0.6 * can_diameter",
                "mm",
                "vent score length (est.)",
            ),
            (
                "vent_width",
                "0.12 mm",
                "mm",
                "vent score width at the top (est.)",
            ),
            (
                "vent_depth",
                "0.015 mm",
                "mm",
                "vent score depth, a V (est.)",
            ),
        ],
    );
    let (d, h, a, bh) = (6.3, 5.8, 6.6, 1.0);
    bx(
        &mut s,
        "base",
        Op::New("base"),
        ["- base_length / 2", "- base_width / 2", "0 mm"],
        ["base_length", "base_width", "base_height"],
    );
    run(
        &mut s,
        "solid.chamfer",
        json!({"name": "base bevels", "body": "base", "distance": "base_bevel",
            "edges": [[-a / 2.0, -a / 2.0, bh / 2.0], [-a / 2.0, a / 2.0, bh / 2.0]]}),
    );
    cyl(
        &mut s,
        "can",
        Op::New("can"),
        ["0 mm", "0 mm", "base_height"],
        [0, 0, 1],
        "can_diameter",
        "height - base_height",
    );
    run(
        &mut s,
        "solid.fillet",
        json!({"name": "rim", "body": "can", "radius": "rim_round",
            "edges": [[d / 2.0, 0.0, h]]}),
    );
    // The rolled seal bead: a groove `bead_depth` deep and twice as high
    // round the can (a ring cut from it).
    cyl(
        &mut s,
        "seal bead ring",
        Op::New("seal bead"),
        ["0 mm", "0 mm", "base_height + bead_height - bead_depth"],
        [0, 0, 1],
        "can_diameter + 1 mm",
        "2 * bead_depth",
    );
    cyl(
        &mut s,
        "seal bead inside",
        Op::Cut(&["seal bead"]),
        [
            "0 mm",
            "0 mm",
            "base_height + bead_height - bead_depth - 1 mm",
        ],
        [0, 0, 1],
        "can_diameter - 2 * bead_depth",
        "2 * bead_depth + 2 mm",
    );
    run(
        &mut s,
        "solid.combine",
        json!({"target": "can", "tools": ["seal bead"], "operation": "cut",
            "name": "seal bead groove"}),
    );
    // The negative print: the top's flat disc beyond print_from, inlaid.
    cyl(
        &mut s,
        "print",
        Op::New("print"),
        ["0 mm", "0 mm", "height - print_thickness"],
        [0, 0, 1],
        "can_diameter - 2 * rim_round",
        "print_thickness",
    );
    span_box(
        &mut s,
        "print edge",
        Op::Cut(&["print"]),
        [
            "- can_diameter",
            "print_from",
            "- can_diameter",
            "can_diameter",
            "height - 1 mm",
            "height + 1 mm",
        ],
    );
    cut_keep(&mut s, "print inlay", "can", &["print"]);
    if vent {
        // Each arm a V-groove, `vent_width` open at the top and
        // `vent_depth` deep, swept along its axis: its sloped walls shade
        // apart from the flat top seen from any side, as the stamped
        // score does. The YZ plane draws (y, z) as is, normal +x; the XZ
        // plane draws (x, −z), normal +y.
        for (k, plane, flip) in [(1, "YZ", ""), (2, "XZ", "-")] {
            let sketch = format!("vent score {k} sketch");
            run(
                &mut s,
                "sketch.create",
                json!({"plane": plane, "offset": "- vent_length / 2", "name": sketch}),
            );
            let z = |e: &str| format!("{flip}({e})");
            run(
                &mut s,
                "sketch.line",
                json!({"points": [
                    ["- vent_width / 2", z("height")],
                    ["0 mm", z("height - vent_depth")],
                    ["vent_width / 2", z("height")],
                    ["vent_width / 2", z("height + 1 mm")],
                    ["- vent_width / 2", z("height + 1 mm")],
                ], "closed": true}),
            );
            run(&mut s, "sketch.finish", json!({}));
            run(
                &mut s,
                "solid.extrude",
                with_op(
                    json!({"sketch": sketch, "distance": "vent_length"}),
                    &format!("vent score {k}"),
                    &Op::Cut(&["can", "print"]),
                ),
            );
        }
    }
    for (k, minus) in [(1, true), (2, false)] {
        let name = format!("terminal {k}");
        let (x0, x1) = if minus {
            ("- span / 2", "- terminal_gap / 2")
        } else {
            ("terminal_gap / 2", "span / 2")
        };
        span_box(
            &mut s,
            &name,
            Op::New(&name),
            [
                x0,
                x1,
                "- terminal_width / 2",
                "terminal_width / 2",
                "0 mm",
                "lead_thickness",
            ],
        );
    }
    cut_keep(
        &mut s,
        "terminal grooves",
        "base",
        &["terminal 1", "terminal 2"],
    );
    assign_look(&mut s, &["base"], MaterialId::Pa9t);
    assign_look(&mut s, &["can"], MaterialId::Aluminium);
    assign_look(&mut s, &["print"], MaterialId::MarkingInkBlack);
    assign_look(&mut s, &["terminal 1", "terminal 2"], MaterialId::TinMatte);
    // The value is printed on the bare part of the top.
    marking_up(
        &mut s,
        "top",
        [
            "(print_from - can_diameter / 2 + rim_round) / 2",
            "0 mm",
            "height",
        ],
        [
            "print_from + can_diameter / 2 - rim_round",
            "0.6 * can_diameter",
        ],
    );
    finish(&mut s)
}

// ---------------------------------------------------------------------
// Power inductors
// ---------------------------------------------------------------------

/// The `power-inductor` generator's moulded power inductor (Coilcraft
/// XEL4030 class): a block of moulded magnetic composite with rounded
/// upright and top edges, and two tin bottom terminals inlaid flush in its
/// underside at ±x. Saved at the XEL4030.
pub fn author_inductor_molded() -> Vec<u8> {
    let mut s = new_design("Power inductor, moulded");
    params(
        &mut s,
        &[
            ("body_length", "4 mm", "mm", "body along x"),
            ("body_width", "4 mm", "mm", "body across (y)"),
            ("height", "3.1 mm", "mm", "top above the seating plane"),
            (
                "terminal_length",
                "0.82 mm",
                "mm",
                "bottom terminal along x",
            ),
            (
                "terminal_width",
                "3.25 mm",
                "mm",
                "bottom terminal across (y)",
            ),
            ("terminal_gap", "1.57 mm", "mm", "gap between the terminals"),
            (
                "terminal_thickness",
                "0.05 mm",
                "mm",
                "terminal plating thickness (est.)",
            ),
            (
                "corner_round",
                "0.2 mm",
                "mm",
                "round of the upright edges (est.)",
            ),
            (
                "edge_round",
                "0.15 mm",
                "mm",
                "round of the top edges (est.)",
            ),
        ],
    );
    let (bl, bw, h) = (4.0, 4.0, 3.1);
    bx(
        &mut s,
        "core",
        Op::New("core"),
        ["- body_length / 2", "- body_width / 2", "0 mm"],
        ["body_length", "body_width", "height"],
    );
    let corners: Vec<[f64; 3]> = [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]]
        .iter()
        .map(|[x, y]| [x * bl / 2.0, y * bw / 2.0, h / 2.0])
        .collect();
    run(
        &mut s,
        "solid.fillet",
        json!({"name": "corner round", "body": "core", "radius": "corner_round",
            "edges": corners}),
    );
    run(
        &mut s,
        "solid.fillet",
        json!({"name": "edge round", "body": "core", "radius": "edge_round",
            "edges": [[0.0, -bw / 2.0, h]]}),
    );
    for (k, minus) in [(1, true), (2, false)] {
        let name = format!("terminal {k}");
        let (x0, x1) = if minus {
            ("- terminal_gap / 2 - terminal_length", "- terminal_gap / 2")
        } else {
            ("terminal_gap / 2", "terminal_gap / 2 + terminal_length")
        };
        span_box(
            &mut s,
            &name,
            Op::New(&name),
            [
                x0,
                x1,
                "- terminal_width / 2",
                "terminal_width / 2",
                "0 mm",
                "terminal_thickness",
            ],
        );
    }
    cut_keep(
        &mut s,
        "terminal recesses",
        "core",
        &["terminal 1", "terminal 2"],
    );
    assign_look(&mut s, &["core"], MaterialId::Ferrite);
    assign_look(&mut s, &["terminal 1", "terminal 2"], MaterialId::TinMatte);
    marking_up(
        &mut s,
        "top",
        ["0 mm", "0 mm", "height"],
        ["0.7 * body_length", "0.7 * body_width"],
    );
    finish(&mut s)
}

/// The `power-inductor` generator's shielded wound power inductor (Bourns
/// SRR class): a ferrite sleeve, square with bevelled upright corners and
/// a round bore, the drum core's top flange in the bore, an epoxy ring
/// between them, and two tin bottom terminals at ±x. With `window`, the
/// sleeve is open over the middle of its ±y sides and the enamelled
/// winding shows through (the WE-PD class). Saved at the SRR6028.
pub fn author_inductor_shielded(window: bool) -> Vec<u8> {
    let mut s = new_design(if window {
        "Power inductor, shielded, winding windows"
    } else {
        "Power inductor, shielded"
    });
    params(
        &mut s,
        &[
            ("body_length", "6.8 mm", "mm", "body along x"),
            ("body_width", "6.8 mm", "mm", "body across (y)"),
            ("height", "2.8 mm", "mm", "top above the seating plane"),
            (
                "corner_bevel",
                "1 mm",
                "mm",
                "bevel of the sleeve's upright corners (est.)",
            ),
            (
                "bore_diameter",
                "5.9 mm",
                "mm",
                "sleeve bore, the drum flange's seat (est.)",
            ),
            (
                "flange_diameter",
                "5.6 mm",
                "mm",
                "drum core top flange diameter (est.)",
            ),
            (
                "flange_thickness",
                "0.5 mm",
                "mm",
                "drum core flange thickness (est.)",
            ),
            (
                "winding_diameter",
                "5.2 mm",
                "mm",
                "winding outer diameter (est.)",
            ),
            ("terminal_length", "2.3 mm", "mm", "bottom terminal along x"),
            (
                "terminal_width",
                "4 mm",
                "mm",
                "bottom terminal across (y) (est.)",
            ),
            (
                "terminal_thickness",
                "0.08 mm",
                "mm",
                "terminal thickness (est.)",
            ),
            (
                "window_width",
                "3 mm",
                "mm",
                "winding window along x (est.)",
            ),
        ],
    );
    let (bl, bw, h) = (6.8, 6.8, 2.8);
    bx(
        &mut s,
        "sleeve",
        Op::New("sleeve"),
        [
            "- body_length / 2",
            "- body_width / 2",
            "terminal_thickness",
        ],
        ["body_length", "body_width", "height - terminal_thickness"],
    );
    let corners: Vec<[f64; 3]> = [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]]
        .iter()
        .map(|[x, y]| [x * bl / 2.0, y * bw / 2.0, h / 2.0])
        .collect();
    run(
        &mut s,
        "solid.chamfer",
        json!({"name": "corner bevels", "body": "sleeve", "distance": "corner_bevel",
            "edges": corners}),
    );
    // The bore through the top down to the base plate (the winding space).
    cyl(
        &mut s,
        "bore",
        Op::Cut(&["sleeve"]),
        ["0 mm", "0 mm", "height - flange_thickness"],
        [0, 0, 1],
        "bore_diameter",
        "flange_thickness + 1 mm",
    );
    cyl(
        &mut s,
        "winding space",
        Op::Cut(&["sleeve"]),
        ["0 mm", "0 mm", "terminal_thickness + flange_thickness"],
        [0, 0, 1],
        "winding_diameter",
        "height - 2 * flange_thickness - terminal_thickness",
    );
    cyl(
        &mut s,
        "flange",
        Op::New("flange"),
        ["0 mm", "0 mm", "height - flange_thickness"],
        [0, 0, 1],
        "flange_diameter",
        "flange_thickness",
    );
    // The epoxy ring between the flange and the sleeve.
    cyl(
        &mut s,
        "adhesive",
        Op::New("adhesive"),
        ["0 mm", "0 mm", "height - flange_thickness"],
        [0, 0, 1],
        "bore_diameter",
        "flange_thickness - 0.05 mm",
    );
    cut_keep(&mut s, "adhesive ring", "adhesive", &["flange"]);
    cyl(
        &mut s,
        "winding",
        Op::New("winding"),
        ["0 mm", "0 mm", "terminal_thickness + flange_thickness"],
        [0, 0, 1],
        "winding_diameter",
        "height - 2 * flange_thickness - terminal_thickness",
    );
    if window {
        for (side, y0, y1) in [("-y", "- body_width", "0 mm"), ("+y", "0 mm", "body_width")] {
            span_box(
                &mut s,
                &format!("window {side}"),
                Op::Cut(&["sleeve"]),
                [
                    "- window_width / 2",
                    "window_width / 2",
                    y0,
                    y1,
                    "terminal_thickness + flange_thickness",
                    "height - flange_thickness",
                ],
            );
        }
    }
    for (k, minus) in [(1, true), (2, false)] {
        let name = format!("terminal {k}");
        let (x0, x1) = if minus {
            ("- body_length / 2", "- body_length / 2 + terminal_length")
        } else {
            ("body_length / 2 - terminal_length", "body_length / 2")
        };
        span_box(
            &mut s,
            &name,
            Op::New(&name),
            [
                x0,
                x1,
                "- terminal_width / 2",
                "terminal_width / 2",
                "0 mm",
                "terminal_thickness",
            ],
        );
    }
    assign_look(&mut s, &["sleeve", "flange"], MaterialId::Ferrite);
    assign_look(&mut s, &["adhesive"], MaterialId::EpoxyMould);
    assign_look(&mut s, &["winding"], MaterialId::Polyimide);
    assign_look(&mut s, &["terminal 1", "terminal 2"], MaterialId::TinMatte);
    marking_up(
        &mut s,
        "top",
        ["0 mm", "0 mm", "height"],
        ["0.5 * flange_diameter", "0.5 * flange_diameter"],
    );
    finish(&mut s)
}

// ---------------------------------------------------------------------
// Standoffs
// ---------------------------------------------------------------------

/// The `standoff` generator's surface-mount spacer with an internal thread
/// (Würth WA-SMSI class): a tin-plated steel tube on the board, its top
/// outer edge and bore mouth chamfered; with `spigot`, the narrower spigot
/// below the seating plane that locates it in the board's hole. Saved at
/// the 9774100360R (M3, 10 mm).
pub fn author_standoff(spigot: bool) -> Vec<u8> {
    let mut s = new_design(if spigot {
        "SMD spacer, internal thread, spigot"
    } else {
        "SMD spacer, internal thread"
    });
    let mut rows = vec![
        (
            "length",
            "10 mm",
            "mm",
            "height above the seating plane (L)",
        ),
        ("outer_diameter", "6 mm", "mm", "outer diameter"),
        (
            "bore_diameter",
            "2.5 mm",
            "mm",
            "threaded bore, the thread's minor diameter",
        ),
        ("chamfer", "0.3 mm", "mm", "top outer edge chamfer (est.)"),
        ("bore_chamfer", "0.25 mm", "mm", "bore mouth chamfer (est.)"),
    ];
    if spigot {
        rows.push(("spigot_diameter", "4.2 mm", "mm", "spigot diameter"));
        rows.push((
            "spigot_length",
            "1.4 mm",
            "mm",
            "spigot length below the seating plane",
        ));
    }
    params(&mut s, &rows);
    let (l, od, bore) = (10.0, 6.0, 2.5);
    cyl(
        &mut s,
        "tube",
        Op::New("spacer"),
        ["0 mm", "0 mm", "0 mm"],
        [0, 0, 1],
        "outer_diameter",
        "length",
    );
    if spigot {
        cyl(
            &mut s,
            "spigot",
            Op::Join("spacer"),
            ["0 mm", "0 mm", "- spigot_length"],
            [0, 0, 1],
            "spigot_diameter",
            "spigot_length",
        );
    }
    cyl(
        &mut s,
        "bore",
        Op::Cut(&["spacer"]),
        ["0 mm", "0 mm", "- 3 mm"],
        [0, 0, 1],
        "bore_diameter",
        "length + 6 mm",
    );
    run(
        &mut s,
        "solid.chamfer",
        json!({"name": "top chamfer", "body": "spacer", "distance": "chamfer",
            "edges": [[od / 2.0, 0.0, l]]}),
    );
    run(
        &mut s,
        "solid.chamfer",
        json!({"name": "bore chamfer", "body": "spacer", "distance": "bore_chamfer",
            "edges": [[bore / 2.0, 0.0, l]]}),
    );
    assign_look(&mut s, &["spacer"], MaterialId::TinMatte);
    marking_up(
        &mut s,
        "top",
        ["0 mm", "0 mm", "length"],
        ["0.5 * outer_diameter", "0.5 * outer_diameter"],
    );
    finish(&mut s)
}
