//! The connector generators' part files: wire-to-board shrouded headers
//! (`generators/wire-to-board/parts/`), board-to-board dual-row headers
//! (`generators/board-to-board/parts/`) and the microSD socket
//! (`generators/microsd/parts/`), authored by Solidcraft commands over
//! plain parameters. Original work (MIT OR Apache-2.0).
//!
//! Every value a file's geometry follows is a parameter its generator
//! gives; the files compute nothing a generator must know (the envelope,
//! the lead contacts), so a module measures and a part file builds. The
//! seating plane z = 0 is the board's top face; the origin is the
//! footprint's. Contacts are strips of uniform section whose side profile
//! is an axis-aligned centreline thickened to the contact's thickness and
//! bent round each corner at a centreline radius (`ortho_strip`); a body a
//! contact passes through is cut by the same profile, so contacts and
//! mouldings stay disjoint at any count.
use super::{MaterialId, assign_look, run};
use serde_json::json;
use solidcraft_engine::Session;

/// An expression and its value at the saved size (mm), so the author knows
/// which way a strip turns.
#[derive(Clone)]
pub struct V {
    s: String,
    v: f64,
}

pub fn v(s: &str, v: f64) -> V {
    V { s: s.into(), v }
}

/// `p + k·h` for a unit step `k` of −1, 0 or 1.
fn step(p: &V, k: i32, h: &str) -> String {
    match k {
        0 => p.s.clone(),
        1 => format!("({}) + ({h})", p.s),
        _ => format!("({}) - ({h})", p.s),
    }
}

/// The unit direction of an axis-aligned segment.
fn dir(a: &(V, V), b: &(V, V)) -> (i32, i32) {
    let du = b.0.v - a.0.v;
    let dv = b.1.v - a.1.v;
    assert!(
        du.abs() < 1e-9 || dv.abs() < 1e-9,
        "segment not axis-aligned"
    );
    (
        if du.abs() < 1e-9 {
            0
        } else {
            du.signum() as i32
        },
        if dv.abs() < 1e-9 {
            0
        } else {
            dv.signum() as i32
        },
    )
}

/// A strip `name` of thickness `2·h` along the axis-aligned centreline
/// `points` (sketch u, v) in a sketch on `plane` at `offset`, extruded
/// `width` along the plane's normal, each corner rounded about a centreline
/// radius `r` (outer side `r + h`, inner `r − h`). With `cut`, the same
/// profile also cuts that body (feature `<name> cut`). Returns the sketch's
/// name.
#[allow(clippy::too_many_arguments)]
pub fn ortho_strip(
    s: &mut Session,
    name: &str,
    plane: &str,
    offset: &str,
    width: &str,
    points: &[(V, V)],
    h: &str,
    r: &str,
    cut: Option<&str>,
) -> String {
    let n = points.len() - 1;
    let dirs: Vec<(i32, i32)> = (0..n).map(|i| dir(&points[i], &points[i + 1])).collect();
    // Left normal of a direction (du, dv): (−dv, du).
    let normal = |d: (i32, i32)| (-d.1, d.0);
    let mut left = Vec::new();
    let mut right = Vec::new();
    for k in 0..=n {
        let (nu, nv) = if k == 0 {
            normal(dirs[0])
        } else if k == n {
            normal(dirs[n - 1])
        } else {
            let (a, b) = (normal(dirs[k - 1]), normal(dirs[k]));
            (a.0 + b.0, a.1 + b.1)
        };
        let p = &points[k];
        left.push(json!([step(&p.0, nu, h), step(&p.1, nv, h)]));
        right.push(json!([step(&p.0, -nu, h), step(&p.1, -nv, h)]));
    }
    let mut outline = left.clone();
    outline.extend(right.into_iter().rev());
    let sketch = format!("{name} sketch");
    run(
        s,
        "sketch.create",
        json!({"plane": plane, "offset": offset, "name": sketch}),
    );
    let reply = run(s, "sketch.line", json!({"points": outline, "closed": true}));
    let lines: Vec<String> = reply["curves"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c.as_str().unwrap().to_owned())
        .collect();
    assert_eq!(lines.len(), 2 * n + 2, "{reply}");
    for k in 1..n {
        let (a, b) = (dirs[k - 1], dirs[k]);
        let turn = a.0 * b.1 - a.1 * b.0;
        let (inner, outer) = (format!("({r}) - ({h})"), format!("({r}) + ({h})"));
        let (left_r, right_r) = if turn > 0 {
            (inner, outer)
        } else {
            (outer, inner)
        };
        run(
            s,
            "sketch.fillet",
            json!({"a": lines[k - 1], "b": lines[k], "radius": left_r}),
        );
        run(
            s,
            "sketch.fillet",
            json!({"a": lines[2 * n - k], "b": lines[2 * n - k + 1], "radius": right_r}),
        );
    }
    run(s, "sketch.finish", json!({}));
    run(
        s,
        "solid.extrude",
        json!({"sketch": sketch, "distance": width, "operation": "new",
            "name": name, "body_name": name}),
    );
    if let Some(target) = cut {
        run(
            s,
            "solid.extrude",
            json!({"sketch": sketch, "distance": width, "operation": "cut",
                "targets": [target], "name": format!("{name} cut")}),
        );
    }
    sketch
}

/// A box over x `x0..x1`, y `y0..y1`, z `z0..z1` (expressions).
#[allow(clippy::too_many_arguments)]
pub fn span_box(
    s: &mut Session,
    name: &str,
    [x0, x1]: [&str; 2],
    [y0, y1]: [&str; 2],
    [z0, z1]: [&str; 2],
    body: Option<&str>,
    operation: Option<(&str, &str)>,
) {
    let mut p = json!({"corner": [x0, y0, z0],
        "length": format!("({x1}) - ({x0})"), "width": format!("({y1}) - ({y0})"),
        "height": format!("({z1}) - ({z0})"), "name": name});
    if let Some(map) = p.as_object_mut() {
        if let Some(body) = body {
            map.insert("body_name".into(), json!(body));
        }
        if let Some((operation, target)) = operation {
            map.insert("operation".into(), json!(operation));
            map.insert("targets".into(), json!([target]));
        }
    }
    run(s, "solid.box", p);
}

/// Cut `target` round `tools`, keeping them.
pub fn cut_round(s: &mut Session, name: &str, target: &str, tools: &[&str]) {
    run(
        s,
        "solid.combine",
        json!({"target": target, "tools": tools, "operation": "cut", "keep_tools": true,
            "name": name}),
    );
}

/// A new design `title` with the user parameters `(name, expression,
/// unit, comment)`.
pub fn new_design(title: &str, params: &[(&str, String, &str, &str)]) -> Session {
    let mut s = Session::default();
    run(&mut s, "file.new", json!({"name": title}));
    let rows: Vec<serde_json::Value> = params
        .iter()
        .map(|(name, expression, unit, comment)| {
            json!({"name": name, "expression": expression, "unit": unit, "comment": comment})
        })
        .collect();
    run(
        &mut s,
        "parameters.import",
        json!({"text": serde_json::to_string(&rows).unwrap()}),
    );
    s
}

/// Set `changes` (parameter → expression) and evaluate.
pub fn set(s: &mut Session, changes: serde_json::Value) {
    run(s, "parameters.change", json!({"changes": changes}));
    s.refresh();
}

/// Every body's name but those in `except`.
pub fn bodies_but(s: &Session, except: &[&str]) -> Vec<String> {
    s.model
        .state()
        .bodies
        .iter()
        .map(|b| b.name.clone())
        .filter(|n| !except.contains(&n.as_str()))
        .collect()
}

/// A marking plane `marking-<name>` at `origin`, u along `x_dir`, its
/// normal `x_dir × y_dir` out of the part.
pub fn marking(s: &mut Session, name: &str, origin: [&str; 3], x_dir: [i32; 3], y_dir: [i32; 3]) {
    run(
        s,
        "construct.plane.offset",
        json!({"base": {"origin": origin, "x_dir": x_dir, "y_dir": y_dir},
            "offset": "0 mm", "name": format!("marking-{name}")}),
    );
}

/// The design's bytes, after checking every feature evaluates.
pub fn finish(mut s: Session) -> Vec<u8> {
    s.refresh();
    let errors: Vec<String> = s
        .model
        .results
        .iter()
        .filter_map(|r| r.error.clone().map(|e| format!("{}: {e}", r.name)))
        .collect();
    assert!(errors.is_empty(), "{errors:?}");
    solidcraft_engine::io::write_design(&s.doc)
}

// ---------------------------------------------------------------------
// Wire-to-board headers
// ---------------------------------------------------------------------

/// Where a wire-to-board header's mating face looks.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Entry {
    Side,
    Top,
}

/// The shroud material of a colour.
fn shroud_material(colour: &str) -> MaterialId {
    match colour {
        "white" => MaterialId::PaWhite,
        "black" => MaterialId::Pa9t,
        _ => MaterialId::PaNatural,
    }
}

/// A `wire-to-board` part file: a moulded shroud with a mating cavity,
/// a latch window and two polarising grooves, a row of tin-plated contacts
/// whose tails leave the shroud's rear (+y) and bend onto the board, and a
/// tin-plated mounting tab standing in each end wall with its foot on the
/// board. Side entry: the cavity opens at the front (−y) face, the
/// contacts run along y in it at mid-height, bend down outside the rear
/// face and out along +y; the latch window opens the roof at the front and
/// the grooves run along the cavity's floor by its side walls. Top entry:
/// the cavity opens upwards, the contacts stand up in it, run down through
/// the floor and out along +y under the shroud; the latch window opens the
/// front wall at the top and the grooves run down the rear wall. The
/// shroud's upright corners are rounded. Saved at the generator's 4-way
/// 1.0 mm starter rows (SH-style); the generator gives every value.
pub fn author_wtb(entry: Entry, colour: &str) -> Vec<u8> {
    let side = entry == Entry::Side;
    let title = format!(
        "Wire-to-board header, {} entry, {colour}",
        if side { "side" } else { "top" }
    );
    let mut p: Vec<(&str, String, &str, &str)> = vec![
        ("pins", "4".into(), "", "contacts in the row (x)"),
        ("pitch", "1 mm".into(), "mm", "contact pitch"),
        (
            "body_length",
            "6 mm".into(),
            "mm",
            "shroud length along x, centred",
        ),
        (
            "y_front",
            if side { "-4.575 mm" } else { "-3.225 mm" }.into(),
            "mm",
            "shroud face away from the tails (side entry: the mating face)",
        ),
        (
            "y_rear",
            "-0.325 mm".into(),
            "mm",
            "shroud face the tails leave by",
        ),
        ("standoff", "0.05 mm".into(), "mm", "shroud's bottom"),
        (
            "top",
            if side { "2.9 mm" } else { "4.25 mm" }.into(),
            "mm",
            "shroud's top",
        ),
        (
            "cavity_width",
            "5.1 mm".into(),
            "mm",
            "mating cavity along x",
        ),
        (
            "cavity_bottom",
            if side { "0.35 mm" } else { "1.25 mm" }.into(),
            "mm",
            "cavity's floor",
        ),
        ("latch_width", "3 mm".into(), "mm", "latch window along x"),
        (
            "key_width",
            "0.4 mm".into(),
            "mm",
            "polarising groove width",
        ),
        (
            "key_depth",
            "0.15 mm".into(),
            "mm",
            "polarising groove depth",
        ),
        ("lead_width", "0.3 mm".into(), "mm", "contact width along x"),
        (
            "lead_thickness",
            "0.15 mm".into(),
            "mm",
            "contact thickness",
        ),
        ("tip_y", "0.375 mm".into(), "mm", "tails' tips"),
        (
            "bend_r",
            "0.15 mm".into(),
            "mm",
            "contact bends' centreline radius",
        ),
        ("tab_x", "2.8 mm".into(), "mm", "tabs' plates' centre on x"),
        ("tab_thickness", "0.15 mm".into(), "mm", "tab sheet"),
        (
            "tab_y0",
            if side { "-4.395 mm" } else { "-3.045 mm" }.into(),
            "mm",
            "tabs' front edge",
        ),
        (
            "tab_y1",
            if side { "-3.075 mm" } else { "-1.725 mm" }.into(),
            "mm",
            "tabs' rear edge",
        ),
        (
            "tab_height",
            if side { "2 mm" } else { "2.5 mm" }.into(),
            "mm",
            "tabs' plates' top",
        ),
        ("foot_x", "3.3 mm".into(), "mm", "tabs' feet's outer end"),
        (
            "corner_round",
            "0.09 mm".into(),
            "mm",
            "round of the shroud's upright corners",
        ),
    ];
    if side {
        p.extend([
            ("cavity_top", "2.7 mm".into(), "mm", "cavity's roof"),
            (
                "cavity_back",
                "-1.075 mm".into(),
                "mm",
                "cavity's back wall",
            ),
            (
                "latch_back",
                "-3.575 mm".into(),
                "mm",
                "latch window's back edge",
            ),
            (
                "post_tip",
                "-3.075 mm".into(),
                "mm",
                "contacts' tips in the cavity",
            ),
            (
                "post_z",
                "1.525 mm".into(),
                "mm",
                "contacts' centreline height in the cavity",
            ),
            (
                "bend_y",
                "-0.175 mm".into(),
                "mm",
                "where the tails turn down (centreline)",
            ),
        ]);
    } else {
        p.extend([
            ("cavity_y0", "-2.725 mm".into(), "mm", "cavity's front wall"),
            ("cavity_y1", "-0.825 mm".into(), "mm", "cavity's rear wall"),
            (
                "latch_bottom",
                "3.05 mm".into(),
                "mm",
                "latch window's lower edge",
            ),
            (
                "post_y",
                "-1.775 mm".into(),
                "mm",
                "contacts' line across y (centreline)",
            ),
            ("post_top", "3.75 mm".into(), "mm", "contacts' tips"),
        ]);
    }
    p.extend([
        (
            "marking_top_u",
            "body_length * 0.5".into(),
            "mm",
            "marking area along its u axis",
        ),
        (
            "marking_top_v",
            if side {
                "(y_rear - latch_back) * 0.6"
            } else {
                "(top - cavity_bottom) * 0.3"
            }
            .into(),
            "mm",
            "marking area across it",
        ),
    ]);
    let mut s = new_design(&title, &p);
    span_box(
        &mut s,
        "shroud",
        ["- body_length / 2", "body_length / 2"],
        ["y_front", "y_rear"],
        ["standoff", "top"],
        Some("shroud"),
        None,
    );
    // The upright corners, picked at the saved size.
    let (x, y0, y1, zm) = (3.0, if side { -4.575 } else { -3.225 }, -0.325, 1.5);
    run(
        &mut s,
        "solid.fillet",
        json!({"name": "corner round", "body": "shroud", "radius": "corner_round",
            "edges": [[-x, y0, zm], [x, y0, zm], [x, y1, zm], [-x, y1, zm]]}),
    );
    let cut = Some(("cut", "shroud"));
    let (kx0, kx1) = (
        "cavity_width / 2 - 2 * key_width",
        "cavity_width / 2 - key_width",
    );
    if side {
        span_box(
            &mut s,
            "cavity",
            ["- cavity_width / 2", "cavity_width / 2"],
            ["y_front - 1 mm", "cavity_back"],
            ["cavity_bottom", "cavity_top"],
            None,
            cut,
        );
        span_box(
            &mut s,
            "latch window",
            ["- latch_width / 2", "latch_width / 2"],
            ["y_front - 1 mm", "latch_back"],
            ["cavity_top - 0.01 mm", "top + 1 mm"],
            None,
            cut,
        );
        for (name, xs) in [
            ("key groove 1", [format!("- ({kx1})"), format!("- ({kx0})")]),
            ("key groove 2", [kx0.to_owned(), kx1.to_owned()]),
        ] {
            span_box(
                &mut s,
                name,
                [&xs[0], &xs[1]],
                ["y_front - 1 mm", "cavity_back"],
                ["cavity_bottom - key_depth", "cavity_bottom + 0.01 mm"],
                None,
                cut,
            );
        }
        ortho_strip(
            &mut s,
            "contact",
            "YZ",
            "- (pins - 1) / 2 * pitch - lead_width / 2",
            "lead_width",
            &[
                (v("post_tip", -3.075), v("post_z", 1.525)),
                (v("bend_y", -0.175), v("post_z", 1.525)),
                (v("bend_y", -0.175), v("lead_thickness / 2", 0.075)),
                (v("tip_y", 0.375), v("lead_thickness / 2", 0.075)),
            ],
            "lead_thickness / 2",
            "bend_r",
            Some("shroud"),
        );
    } else {
        span_box(
            &mut s,
            "cavity",
            ["- cavity_width / 2", "cavity_width / 2"],
            ["cavity_y0", "cavity_y1"],
            ["cavity_bottom", "top + 1 mm"],
            None,
            cut,
        );
        span_box(
            &mut s,
            "latch window",
            ["- latch_width / 2", "latch_width / 2"],
            ["y_front - 1 mm", "cavity_y0 + 0.01 mm"],
            ["latch_bottom", "top + 1 mm"],
            None,
            cut,
        );
        for (name, xs) in [
            ("key groove 1", [format!("- ({kx1})"), format!("- ({kx0})")]),
            ("key groove 2", [kx0.to_owned(), kx1.to_owned()]),
        ] {
            span_box(
                &mut s,
                name,
                [&xs[0], &xs[1]],
                ["cavity_y1 - 0.01 mm", "cavity_y1 + key_depth"],
                ["cavity_bottom", "top + 1 mm"],
                None,
                cut,
            );
        }
        ortho_strip(
            &mut s,
            "contact",
            "YZ",
            "- (pins - 1) / 2 * pitch - lead_width / 2",
            "lead_width",
            &[
                (v("post_y", -1.775), v("post_top", 3.75)),
                (v("post_y", -1.775), v("lead_thickness / 2", 0.075)),
                (v("tip_y", 0.375), v("lead_thickness / 2", 0.075)),
            ],
            "lead_thickness / 2",
            "bend_r",
            Some("shroud"),
        );
    }
    run(
        &mut s,
        "solid.pattern.rectangular",
        json!({"features": ["contact cut"], "dir1": [1, 0, 0], "count1": "pins",
            "spacing1": "pitch", "name": "contact cuts"}),
    );
    run(
        &mut s,
        "solid.pattern.rectangular",
        json!({"bodies": ["contact"], "dir1": [1, 0, 0], "count1": "pins",
            "spacing1": "pitch", "name": "contact pattern"}),
    );
    // The tabs: a plate in each end wall and a foot out along x on the
    // board, one body each, the shroud cut round them.
    for (name, sign) in [("tab 1", "-"), ("tab 2", "")] {
        let plate = [
            format!("{sign}(tab_x - tab_thickness / 2)"),
            format!("{sign}(tab_x + tab_thickness / 2)"),
        ];
        let foot = [
            format!("{sign}(tab_x - tab_thickness / 2)"),
            format!("{sign}foot_x"),
        ];
        let order = |a: &[String; 2]| {
            if sign.is_empty() {
                [a[0].clone(), a[1].clone()]
            } else {
                [a[1].clone(), a[0].clone()]
            }
        };
        let plate = order(&plate);
        let foot = order(&foot);
        span_box(
            &mut s,
            &format!("{name} plate"),
            [&plate[0], &plate[1]],
            ["tab_y0", "tab_y1"],
            ["0 mm", "tab_height"],
            Some(name),
            None,
        );
        span_box(
            &mut s,
            &format!("{name} foot"),
            [&foot[0], &foot[1]],
            ["tab_y0", "tab_y1"],
            ["0 mm", "tab_thickness"],
            None,
            Some(("join", name)),
        );
        cut_round(&mut s, &format!("{name} slot"), "shroud", &[name]);
    }
    if side {
        marking(
            &mut s,
            "top",
            ["0 mm", "(latch_back + y_rear) / 2", "top"],
            [1, 0, 0],
            [0, 1, 0],
        );
    } else {
        marking(
            &mut s,
            "top",
            ["0 mm", "y_front", "(top + cavity_bottom) / 2"],
            [1, 0, 0],
            [0, 0, 1],
        );
    }
    // Looks are kept by body name, and a pattern names its copies
    // `contact (2)`, …: every contact the generator can ask for (20) gets
    // the tin look at that count, then the file goes back to 4.
    set(
        &mut s,
        json!({"pins": "20", "body_length": "22 mm", "cavity_width": "21.1 mm",
        "tab_x": "10.8 mm", "foot_x": "11.3 mm", "latch_width": "19 mm"}),
    );
    let contacts = bodies_but(&s, &["shroud", "tab 1", "tab 2"]);
    assert_eq!(contacts.len(), 20, "{contacts:?}");
    let contacts: Vec<&str> = contacts.iter().map(String::as_str).collect();
    assign_look(&mut s, &["shroud"], shroud_material(colour));
    assign_look(&mut s, &contacts, MaterialId::TinMatte);
    assign_look(&mut s, &["tab 1", "tab 2"], MaterialId::TinMatte);
    set(
        &mut s,
        json!({"pins": "4", "body_length": "6 mm", "cavity_width": "5.1 mm",
        "tab_x": "2.8 mm", "foot_x": "3.3 mm", "latch_width": "3 mm"}),
    );
    finish(s)
}

pub fn author_wtb_side_natural() -> Vec<u8> {
    author_wtb(Entry::Side, "natural")
}
pub fn author_wtb_side_white() -> Vec<u8> {
    author_wtb(Entry::Side, "white")
}
pub fn author_wtb_side_black() -> Vec<u8> {
    author_wtb(Entry::Side, "black")
}
pub fn author_wtb_top_natural() -> Vec<u8> {
    author_wtb(Entry::Top, "natural")
}
pub fn author_wtb_top_white() -> Vec<u8> {
    author_wtb(Entry::Top, "white")
}
pub fn author_wtb_top_black() -> Vec<u8> {
    author_wtb(Entry::Top, "black")
}

// ---------------------------------------------------------------------
// Board-to-board terminal strips
// ---------------------------------------------------------------------

/// A `board-to-board` part file: a black LCP insulator on its standoff,
/// grooved between positions along both long faces with a square pocket
/// round each post on its top, a gold square post per position in two
/// rows standing up through it, and per post a tin tail from inside the
/// insulator down through its bottom and out along ±y onto the board (a
/// bent strip, the insulator cut round it). With `pegs`, two tin alignment
/// pins taper from the insulator's bottom into the board on the centre
/// line. Saved at a 2 × 5 strip of 1.27 mm (without pins) or a 2 × 4 strip
/// of 2.54 mm (with pins), the generator's starter rows; the generator
/// gives every value.
pub fn author_b2b(pegs: bool) -> Vec<u8> {
    // positions, pitch, row, length, width, standoff, top, post, post top,
    // split, tip, bend, pocket, pocket depth, groove width, groove depth.
    let saved: [f64; 16] = if pegs {
        [
            4.0, 2.54, 2.54, 10.16, 5.08, 1.27, 3.81, 0.64, 14.48, 2.54, 3.72, 0.64, 1.152, 0.3175,
            0.381, 0.475,
        ]
    } else {
        [
            5.0, 1.27, 1.27, 6.35, 3.43, 0.41, 2.92, 0.41, 5.97, 1.665, 2.9, 0.41, 0.738, 0.31375,
            0.1905, 0.4375,
        ]
    };
    let mm = |v: f64| format!("{v} mm");
    let [
        n,
        pitch,
        row,
        length,
        width,
        standoff,
        top,
        post,
        post_top,
        split,
        tip,
        bend,
    ] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11].map(|i| saved[i]);
    let mut p: Vec<(&str, String, &str, &str)> = vec![
        ("positions", format!("{n}"), "", "positions per row (x)"),
        ("pitch", mm(pitch), "mm", "pitch along a row"),
        ("row_pitch", mm(row), "mm", "pitch between the rows"),
        (
            "body_length",
            mm(length),
            "mm",
            "insulator along x, centred",
        ),
        ("body_width", mm(width), "mm", "insulator along y, centred"),
        ("standoff", mm(standoff), "mm", "insulator's bottom"),
        ("body_top", mm(top), "mm", "insulator's top"),
        (
            "post_side",
            mm(post),
            "mm",
            "square post side, also the tails' section",
        ),
        ("post_top", mm(post_top), "mm", "posts' tips"),
        (
            "z_split",
            mm(split),
            "mm",
            "where posts meet their tails, inside the insulator",
        ),
        (
            "tail_tip",
            mm(tip),
            "mm",
            "tails' tips from the centre line",
        ),
        ("bend_r", mm(bend), "mm", "tails' bends' centreline radius"),
        (
            "pocket",
            mm(saved[12]),
            "mm",
            "square pocket round each post",
        ),
        ("pocket_depth", mm(saved[13]), "mm", "pocket depth"),
        (
            "groove_width",
            mm(saved[14]),
            "mm",
            "grooves between positions, along x",
        ),
        (
            "groove_depth",
            mm(saved[15]),
            "mm",
            "grooves' depth into the long faces",
        ),
        (
            "corner_round",
            "0.1 mm".into(),
            "mm",
            "round of the insulator's upright corners",
        ),
        (
            "marking_top_u",
            "body_length * 0.6".into(),
            "mm",
            "marking area along its u axis",
        ),
        (
            "marking_top_v",
            "groove_depth".into(),
            "mm",
            "marking area across it",
        ),
    ];
    if pegs {
        p.extend([
            (
                "peg_x",
                "2.54 mm".into(),
                "mm",
                "alignment pins' x from the centre",
            ),
            (
                "peg_diameter",
                "1.6 mm".into(),
                "mm",
                "alignment pin at the insulator",
            ),
            (
                "peg_tip",
                "1.21 mm".into(),
                "mm",
                "alignment pin at its tip",
            ),
            (
                "peg_bottom",
                "-1.27 mm".into(),
                "mm",
                "alignment pins' tips",
            ),
        ]);
    }
    let title = if pegs {
        "Dual-row SMD terminal strip with alignment pins"
    } else {
        "Dual-row SMD terminal strip"
    };
    let mut s = new_design(title, &p);
    span_box(
        &mut s,
        "insulator",
        ["- body_length / 2", "body_length / 2"],
        ["- body_width / 2", "body_width / 2"],
        ["standoff", "body_top"],
        Some("insulator"),
        None,
    );
    let (x, y, zm) = (length / 2.0, width / 2.0, (standoff + top) / 2.0);
    run(
        &mut s,
        "solid.fillet",
        json!({"name": "corner round", "body": "insulator", "radius": "corner_round",
            "edges": [[-x, -y, zm], [x, -y, zm], [x, y, zm], [-x, y, zm]]}),
    );
    let cut = Some(("cut", "insulator"));
    span_box(
        &mut s,
        "groove",
        [
            "- (positions - 2) / 2 * pitch - groove_width / 2",
            "- (positions - 2) / 2 * pitch + groove_width / 2",
        ],
        ["- body_width / 2 - 1 mm", "- body_width / 2 + groove_depth"],
        ["standoff - 1 mm", "body_top + 1 mm"],
        None,
        cut,
    );
    run(
        &mut s,
        "solid.pattern.rectangular",
        json!({"features": ["groove"], "dir1": [1, 0, 0], "count1": "positions - 1",
            "spacing1": "pitch", "dir2": [0, 1, 0], "count2": 2,
            "spacing2": "body_width - groove_depth + 1 mm", "name": "grooves"}),
    );
    let x0 = "- (positions - 1) / 2 * pitch";
    let xs = |w: &str| [format!("{x0} - {w} / 2"), format!("{x0} + {w} / 2")];
    let pocket_x = xs("pocket");
    span_box(
        &mut s,
        "pocket",
        [&pocket_x[0], &pocket_x[1]],
        [
            "- row_pitch / 2 - pocket / 2",
            "- row_pitch / 2 + pocket / 2",
        ],
        ["body_top - pocket_depth", "body_top + 1 mm"],
        None,
        cut,
    );
    let post_x = xs("post_side");
    span_box(
        &mut s,
        "post hole",
        [&post_x[0], &post_x[1]],
        [
            "- row_pitch / 2 - post_side / 2",
            "- row_pitch / 2 + post_side / 2",
        ],
        ["z_split", "body_top + 1 mm"],
        None,
        cut,
    );
    span_box(
        &mut s,
        "post",
        [&post_x[0], &post_x[1]],
        [
            "- row_pitch / 2 - post_side / 2",
            "- row_pitch / 2 + post_side / 2",
        ],
        ["z_split", "post_top"],
        Some("post"),
        None,
    );
    let grid = |what: serde_json::Value, name: &str| {
        let mut g = json!({"dir1": [1, 0, 0], "count1": "positions", "spacing1": "pitch",
            "dir2": [0, 1, 0], "count2": 2, "spacing2": "row_pitch", "name": name});
        if let (Some(map), Some(extra)) = (g.as_object_mut(), what.as_object()) {
            map.extend(extra.clone());
        }
        g
    };
    run(
        &mut s,
        "solid.pattern.rectangular",
        grid(
            json!({"features": ["pocket", "post hole"]}),
            "pockets and holes",
        ),
    );
    run(
        &mut s,
        "solid.pattern.rectangular",
        grid(json!({"bodies": ["post"]}), "posts"),
    );
    let h = post / 2.0;
    for (name, minus) in [("tail 1", true), ("tail 2", false)] {
        let sy = |e: &str, value: f64| {
            if minus {
                v(&format!("-({e})"), -value)
            } else {
                v(e, value)
            }
        };
        ortho_strip(
            &mut s,
            name,
            "YZ",
            &format!("{x0} - post_side / 2"),
            "post_side",
            &[
                (sy("row_pitch / 2", row / 2.0), v("z_split", split)),
                (sy("row_pitch / 2", row / 2.0), v("post_side / 2", h)),
                (sy("tail_tip", tip), v("post_side / 2", h)),
            ],
            "post_side / 2",
            "bend_r",
            Some("insulator"),
        );
        run(
            &mut s,
            "solid.pattern.rectangular",
            json!({"features": [format!("{name} cut")], "dir1": [1, 0, 0],
                "count1": "positions", "spacing1": "pitch", "name": format!("{name} cuts")}),
        );
        run(
            &mut s,
            "solid.pattern.rectangular",
            json!({"bodies": [name], "dir1": [1, 0, 0], "count1": "positions",
                "spacing1": "pitch", "name": format!("{name} pattern")}),
        );
    }
    if pegs {
        for (name, sign) in [("peg 1", "-"), ("peg 2", "")] {
            run(
                &mut s,
                "solid.cone",
                json!({"diameter": "peg_tip", "top_diameter": "peg_diameter",
                    "height": "standoff - peg_bottom",
                    "base": [format!("{sign}peg_x"), "0 mm", "peg_bottom"],
                    "name": name, "body_name": name}),
            );
        }
    }
    marking(
        &mut s,
        "top",
        ["0 mm", "0 mm", "body_top"],
        [1, 0, 0],
        [0, 1, 0],
    );
    // Every post and tail the generator can ask for (50 positions) gets
    // its look at that count, then the file goes back.
    let mut most = json!({"positions": "50", "body_length": "50 * pitch"});
    let mut back = json!({"positions": format!("{n}"), "body_length": mm(length)});
    if pegs {
        most["peg_x"] = json!("24 * pitch");
        back["peg_x"] = json!("2.54 mm");
    }
    set(&mut s, most);
    let named = |s: &Session, prefix: &str| -> Vec<String> {
        bodies_but(s, &[])
            .into_iter()
            .filter(|n| n.starts_with(prefix))
            .collect()
    };
    let posts = named(&s, "post");
    let tails = named(&s, "tail");
    assert_eq!((posts.len(), tails.len()), (100, 100));
    let posts: Vec<&str> = posts.iter().map(String::as_str).collect();
    let tails: Vec<&str> = tails.iter().map(String::as_str).collect();
    assign_look(&mut s, &["insulator"], MaterialId::Lcp);
    assign_look(&mut s, &posts, MaterialId::Gold);
    assign_look(&mut s, &tails, MaterialId::TinMatte);
    if pegs {
        assign_look(&mut s, &["peg 1", "peg 2"], MaterialId::TinMatte);
    }
    set(&mut s, back);
    finish(s)
}

// ---------------------------------------------------------------------
// MicroSD socket
// ---------------------------------------------------------------------

/// The microSD socket's solder feet: name and saved x0, x1, y0, y1 (mm).
pub const MICROSD_FEET: [(&str, [f64; 4]); 6] = [
    ("g1", [-6.25, -5.15, -0.525, 0.775]),
    ("g2", [5.0, 6.3, -0.525, 0.775]),
    ("g3", [-6.25, -5.65, 7.075, 8.375]),
    ("g4", [5.7, 6.3, 7.775, 8.975]),
    ("cd_a", [5.2, 6.0, 10.125, 11.475]),
    ("cd_b", [-6.25, -5.0, 8.825, 9.625]),
];

/// The `microsd` generator's part file, a push-pull microSD socket: a
/// nickel-silver shell (top plate and side and rear walls, open at the
/// front and below, with two windows in its top) standing on tin-plated
/// solder feet (four ground feet and the card-detect switch's two), a
/// black LCP insulator (a floor plate and a rear block) inside it, and a
/// row of gold contacts, each a tail on the board at the front that rises
/// through the floor into a spring arm reaching back under the card. The
/// frame's x origin is the card's centre line; y = 0 the contact lands'
/// centre, the card entering from −y. Saved at the DM3D-style starter row;
/// the generator gives every value.
pub fn author_microsd() -> Vec<u8> {
    let mut p: Vec<(&str, String, &str, &str)> = vec![
        ("contacts", "8".into(), "", "contacts in the row (x)"),
        ("pitch", "1.1 mm".into(), "mm", "contact pitch"),
        ("first_x", "-4.5 mm".into(), "mm", "the −x contact's centre"),
        ("shell_x0", "-5.95 mm".into(), "mm", "shell's −x side"),
        ("shell_x1", "6 mm".into(), "mm", "shell's +x side"),
        (
            "front",
            "-0.55 mm".into(),
            "mm",
            "shell's front (card entry)",
        ),
        ("rear", "10.9 mm".into(), "mm", "shell's rear"),
        ("height", "1.55 mm".into(), "mm", "shell's top"),
        ("sheet", "0.15 mm".into(), "mm", "shell sheet thickness"),
        (
            "foot",
            "0.15 mm".into(),
            "mm",
            "solder feet's thickness; the shell stands on them",
        ),
        ("floor_top", "0.35 mm".into(), "mm", "insulator floor's top"),
        (
            "floor_front",
            "0.05 mm".into(),
            "mm",
            "insulator floor's front edge",
        ),
        (
            "block_front",
            "9.3 mm".into(),
            "mm",
            "insulator's rear block's front face",
        ),
        ("lead_width", "0.4 mm".into(), "mm", "contact width"),
        ("lead_thickness", "0.1 mm".into(), "mm", "contact thickness"),
        ("tip_y", "-0.65 mm".into(), "mm", "contact tails' tips"),
        (
            "rise_y",
            "0.35 mm".into(),
            "mm",
            "where a contact rises through the floor",
        ),
        (
            "arm_z",
            "0.5 mm".into(),
            "mm",
            "spring arms' centreline height",
        ),
        ("arm_end", "4.5 mm".into(), "mm", "spring arms' ends"),
        (
            "bend_r",
            "0.1 mm".into(),
            "mm",
            "contact bends' centreline radius",
        ),
        (
            "window_x",
            "2 mm".into(),
            "mm",
            "top windows' half spacing along x",
        ),
        (
            "window_y0",
            "5.5 mm".into(),
            "mm",
            "top windows' front edge",
        ),
        ("window_y1", "7.5 mm".into(), "mm", "top windows' rear edge"),
        (
            "window_w",
            "1.2 mm".into(),
            "mm",
            "top windows' width along x",
        ),
        (
            "marking_top_u",
            "(shell_x1 - shell_x0) * 0.5".into(),
            "mm",
            "marking area along its u axis",
        ),
        (
            "marking_top_v",
            "(rear - front) * 0.25".into(),
            "mm",
            "marking area across it",
        ),
    ];
    let names: Vec<[String; 4]> = MICROSD_FEET
        .iter()
        .map(|(n, _)| ["x0", "x1", "y0", "y1"].map(|k| format!("{n}_{k}")))
        .collect();
    let comments = [
        "foot's −x edge",
        "foot's +x edge",
        "foot's front edge",
        "foot's rear edge",
    ];
    for (i, (_, values)) in MICROSD_FEET.iter().enumerate() {
        for k in 0..4 {
            p.push((
                names[i][k].as_str(),
                format!("{} mm", values[k]),
                "mm",
                comments[k],
            ));
        }
    }
    let mut s = new_design("MicroSD socket, push-pull", &p);
    // The shell: a block hollowed from below and the front.
    span_box(
        &mut s,
        "shell",
        ["shell_x0", "shell_x1"],
        ["front", "rear"],
        ["foot", "height"],
        Some("shell"),
        None,
    );
    let cut = Some(("cut", "shell"));
    span_box(
        &mut s,
        "shell hollow",
        ["shell_x0 + sheet", "shell_x1 - sheet"],
        ["front - 1 mm", "rear - sheet"],
        ["foot - 1 mm", "height - sheet"],
        None,
        cut,
    );
    for (name, xs) in [
        (
            "window 1",
            ["- window_x - window_w / 2", "- window_x + window_w / 2"],
        ),
        (
            "window 2",
            ["window_x - window_w / 2", "window_x + window_w / 2"],
        ),
    ] {
        span_box(
            &mut s,
            name,
            xs,
            ["window_y0", "window_y1"],
            ["height - sheet - 0.1 mm", "height + 1 mm"],
            None,
            cut,
        );
    }
    // The insulator: a floor plate and a rear block, one body.
    span_box(
        &mut s,
        "insulator",
        ["shell_x0 + sheet", "shell_x1 - sheet"],
        ["floor_front", "rear - sheet"],
        ["foot", "floor_top"],
        Some("insulator"),
        None,
    );
    span_box(
        &mut s,
        "rear block",
        ["shell_x0 + sheet", "shell_x1 - sheet"],
        ["block_front", "rear - sheet"],
        ["floor_top - 0.01 mm", "height - sheet"],
        None,
        Some(("join", "insulator")),
    );
    ortho_strip(
        &mut s,
        "contact",
        "YZ",
        "first_x - lead_width / 2",
        "lead_width",
        &[
            (v("tip_y", -0.65), v("lead_thickness / 2", 0.05)),
            (v("rise_y", 0.35), v("lead_thickness / 2", 0.05)),
            (v("rise_y", 0.35), v("arm_z", 0.5)),
            (v("arm_end", 4.5), v("arm_z", 0.5)),
        ],
        "lead_thickness / 2",
        "bend_r",
        Some("insulator"),
    );
    run(
        &mut s,
        "solid.pattern.rectangular",
        json!({"features": ["contact cut"], "dir1": [1, 0, 0], "count1": "contacts",
            "spacing1": "pitch", "name": "contact cuts"}),
    );
    run(
        &mut s,
        "solid.pattern.rectangular",
        json!({"bodies": ["contact"], "dir1": [1, 0, 0], "count1": "contacts",
            "spacing1": "pitch", "name": "contact pattern"}),
    );
    for (i, (name, _)) in MICROSD_FEET.iter().enumerate() {
        let n = &names[i];
        span_box(
            &mut s,
            &format!("foot {name}"),
            [&n[0], &n[1]],
            [&n[2], &n[3]],
            ["0 mm", "foot"],
            Some(&format!("foot {name}")),
            None,
        );
    }
    marking(
        &mut s,
        "top",
        ["(shell_x0 + shell_x1) / 2", "(front + rear) / 2", "height"],
        [1, 0, 0],
        [0, 1, 0],
    );
    let contacts: Vec<String> = bodies_but(&s, &[])
        .into_iter()
        .filter(|n| n.starts_with("contact"))
        .collect();
    assert_eq!(contacts.len(), 8, "{contacts:?}");
    let contacts: Vec<&str> = contacts.iter().map(String::as_str).collect();
    let feet: Vec<String> = MICROSD_FEET
        .iter()
        .map(|(n, _)| format!("foot {n}"))
        .collect();
    let feet: Vec<&str> = feet.iter().map(String::as_str).collect();
    assign_look(&mut s, &["shell"], MaterialId::NickelSilver);
    assign_look(&mut s, &["insulator"], MaterialId::Lcp);
    assign_look(&mut s, &contacts, MaterialId::Gold);
    assign_look(&mut s, &feet, MaterialId::TinBright);
    finish(s)
}

pub fn author_microsd_push_pull() -> Vec<u8> {
    author_microsd()
}

pub fn author_b2b_dual_row() -> Vec<u8> {
    author_b2b(false)
}
pub fn author_b2b_dual_row_pegs() -> Vec<u8> {
    author_b2b(true)
}

// ---------------------------------------------------------------------
// Female sockets (the header generator's sockets)
// ---------------------------------------------------------------------

/// One square of `side` round every position (pin 1 at `x0`, `y0`, a sketch
/// pattern of `pins` × `rows`) on an XY sketch `name sketch` at `z`, cut
/// into `target` by one extrude of `distance` (`taper`, `down` as given):
/// one Boolean for every position, where a feature pattern of the cut made
/// one a position (the socket files' evaluation time).
#[allow(clippy::too_many_arguments)]
fn every_opening(
    s: &mut Session,
    name: &str,
    target: &str,
    side: &str,
    z: &str,
    distance: &str,
    taper: Option<&str>,
    down: bool,
) {
    let sketch = format!("{name} sketch");
    run(
        s,
        "sketch.create",
        json!({"plane": "XY", "offset": z, "name": sketch}),
    );
    let corner = |sx: &str, sy: &str| {
        json!([
            format!("x0 {sx} ({side}) / 2"),
            format!("y0 {sy} ({side}) / 2")
        ])
    };
    let ids: Vec<String> = (1..=4).map(|k| format!("{name} {k}")).collect();
    run(
        s,
        "sketch.line",
        json!({"points": [corner("-", "-"), corner("+", "-"), corner("+", "+"), corner("-", "+")],
            "closed": true, "ids": ids}),
    );
    run(
        s,
        "sketch.pattern.rectangular",
        json!({"entities": ids, "dir": [1, 0], "count": "pins", "spacing": "pitch",
            "dir2": [0, 1], "count2": "rows", "spacing2": "row_pitch"}),
    );
    run(s, "sketch.finish", json!({}));
    let mut p = json!({"sketch": sketch, "distance": distance, "operation": "cut",
        "targets": [target], "name": name});
    if let Some(map) = p.as_object_mut() {
        if let Some(taper) = taper {
            map.insert("taper".into(), json!(taper));
        }
        if down {
            map.insert("direction".into(), json!("negative"));
        }
    }
    run(s, "solid.extrude", p);
}

/// A square lead-in at every position, cut into `target` from the face at
/// `z`: `lead_in` across at the face, drawing in at 45° to the entry at
/// `entry_depth`, into the part (down from a top face with `down`, else up
/// from a bottom face). The cut runs on half as deep again inside the
/// entry, so its bottom edges do not lie on the entry's walls.
fn lead_ins(s: &mut Session, name: &str, target: &str, z: &str, down: bool) {
    every_opening(
        s,
        name,
        target,
        "lead_in",
        z,
        "entry_depth * 1.5",
        Some("-45 deg"),
        down,
    );
}

/// A box `name` round pin 1's position: `side` square over z `z0..z1`.
fn pin_box(
    s: &mut Session,
    name: &str,
    side: &str,
    z: [&str; 2],
    body: Option<&str>,
    operation: Option<(&str, &str)>,
) {
    let x = [format!("x0 - ({side}) / 2"), format!("x0 + ({side}) / 2")];
    let y = [format!("y0 - ({side}) / 2"), format!("y0 + ({side}) / 2")];
    span_box(s, name, [&x[0], &x[1]], [&y[0], &y[1]], z, body, operation);
}

/// The contact's two leaves at pin 1's position, facing each other across
/// y over z `z`, as bodies `leaf a` (−y) and `leaf b` (+y).
fn leaves(s: &mut Session, z: [&str; 2]) {
    let x = ["x0 - leaf_width / 2", "x0 + leaf_width / 2"];
    span_box(
        s,
        "leaf a",
        x,
        ["y0 - leaf_gap / 2 - leaf", "y0 - leaf_gap / 2"],
        z,
        Some("leaf a"),
        None,
    );
    span_box(
        s,
        "leaf b",
        x,
        ["y0 + leaf_gap / 2", "y0 + leaf_gap / 2 + leaf"],
        z,
        Some("leaf b"),
        None,
    );
}

/// The parameters both socket files share, at a saved size: rows,
/// positions per row, pitch, body height and tail width (mm).
fn socket_params(
    rows: u32,
    pins: u32,
    pitch: f64,
    height: f64,
    tail: f64,
) -> Vec<(&'static str, String, &'static str, &'static str)> {
    let mm = |v: f64| format!("{} mm", (v * 1e4).round() / 1e4);
    vec![
        ("rows", format!("{rows}"), "", "rows of positions (y)"),
        ("pins", format!("{pins}"), "", "positions per row (x)"),
        ("pitch", mm(pitch), "mm", "pitch along a row"),
        ("row_pitch", mm(pitch), "mm", "pitch between rows"),
        (
            "body_height",
            mm(height),
            "mm",
            "insulator top above the seating plane",
        ),
        ("pin_side", mm(tail), "mm", "contact tail width"),
        (
            "lead_in",
            mm(pitch * 0.68),
            "mm",
            "entry lead-in's side at the face",
        ),
        ("entry", mm(pitch * 0.4), "mm", "square entry's side"),
        (
            "entry_depth",
            mm(pitch * 0.14),
            "mm",
            "lead-in's depth (45°)",
        ),
        ("cavity", mm(pitch * 0.56), "mm", "contact cavity's side"),
        ("leaf", "0.15 mm".into(), "mm", "contact leaf sheet"),
        (
            "leaf_gap",
            mm(pitch * 0.18),
            "mm",
            "gap between the two leaves",
        ),
        ("leaf_width", mm(pitch * 0.32), "mm", "leaf width along x"),
        (
            "corner_round",
            "0.15 mm".into(),
            "mm",
            "round of the insulator's upright corners",
        ),
        (
            "x0",
            "- (pins - 1) / 2 * pitch".into(),
            "mm",
            "pin 1's x (derived)",
        ),
        (
            "y0",
            "- (rows - 1) / 2 * row_pitch".into(),
            "mm",
            "pin 1's y (derived)",
        ),
    ]
}

/// A rectangular pattern of `what` (features or bodies) over every
/// position, `pins` × `rows`.
fn every_position(s: &mut Session, what: serde_json::Value, name: &str) {
    let mut p = json!({"dir1": [1, 0, 0], "count1": "pins", "spacing1": "pitch",
        "dir2": [0, 1, 0], "count2": "rows", "spacing2": "row_pitch", "name": name});
    if let (Some(map), Some(extra)) = (p.as_object_mut(), what.as_object()) {
        map.extend(extra.clone());
    }
    run(s, "solid.pattern.rectangular", p);
}

/// The header generator's through-hole socket (`pin-socket`): a black
/// PBT insulator on the seating plane, per position a square entry under a
/// 45° lead-in at the top opening into a wider cavity down to the floor,
/// and a gold contact: two leaves facing each other across the cavity
/// (they grip the mating post) on a tail of `pin_side` square that passes
/// the floor and reaches `pin_below` under the seating plane. The
/// insulator is three flush slabs (floor, cavity walls, entry walls), each
/// one extrude of its outline with its openings drawn in by a sketch
/// pattern over every position, and the lead-ins one tapered cut: no
/// Boolean per position, so the file evaluates in about a third of the time
/// a cut per opening and position took, and contacts and insulator stay
/// disjoint at any count. Saved at a 1 × 10
/// socket of 2.54 mm (Würth Elektronik WR-PHD 61301011821: insulator 8.5 mm
/// high, tails 3.1 mm); the generator gives every value.
pub fn author_pin_socket() -> Vec<u8> {
    let mut p = socket_params(1, 10, 2.54, 8.5, 0.64);
    p.extend([
        (
            "pin_below",
            "3.1 mm".to_owned(),
            "mm",
            "tail length below the seating plane",
        ),
        (
            "floor_z",
            "1 mm".into(),
            "mm",
            "insulator floor under the cavity",
        ),
        (
            "cavity_top",
            "body_height - entry_depth - 0.3 mm".into(),
            "mm",
            "cavity's roof under the entry",
        ),
    ]);
    let mut s = new_design("Pin socket", &p);
    slab(
        &mut s,
        "floor",
        "insulator",
        ["0 mm", "floor_z"],
        &[Hole::Square("pin_side")],
    );
    slab(
        &mut s,
        "cavity walls",
        "insulator walls",
        ["floor_z", "cavity_top"],
        &[Hole::Square("cavity")],
    );
    slab(
        &mut s,
        "entry walls",
        "insulator top",
        ["cavity_top", "body_height"],
        &[Hole::Square("entry")],
    );
    lead_ins(&mut s, "lead-ins", "insulator top", "body_height", true);
    pin_box(
        &mut s,
        "contact",
        "pin_side",
        ["- pin_below", "floor_z + 0.3 mm"],
        Some("contact"),
        None,
    );
    leaves(&mut s, ["floor_z", "cavity_top - leaf"]);
    run(
        &mut s,
        "solid.combine",
        json!({"target": "contact", "tools": ["leaf a", "leaf b"], "operation": "join",
            "name": "contact leaves"}),
    );
    every_position(&mut s, json!({"bodies": ["contact"]}), "contacts");
    // Every contact the generator can ask for (4 × 64) gets its look at
    // that count, then the file goes back.
    set(&mut s, json!({"rows": "4", "pins": "64"}));
    let insulator = ["insulator", "insulator walls", "insulator top"];
    let contacts = bodies_but(&s, &insulator);
    assert_eq!(contacts.len(), 256);
    let contacts: Vec<&str> = contacts.iter().map(String::as_str).collect();
    assign_look(&mut s, &insulator, MaterialId::Pbt);
    assign_look(&mut s, &contacts, MaterialId::Gold);
    set(&mut s, json!({"rows": "1", "pins": "10"}));
    finish(s)
}

/// The header generator's bottom-entry SMD socket (`socket-smd-bottom`):
/// a black PA9T insulator `standoff` above the board, per position a square
/// entry through it with a 45° lead-in at the bottom (over the board's
/// hole, where the mating post comes up) and the same at the top, a wider
/// cavity between holding the gold contact's two leaves, and per pin a
/// gold tail: a strip of `tail_sheet` and `pin_side` wide from inside the
/// insulator's long wall down through its bottom and out along ±y on the
/// board to `tail_span` / 2. The insulator is three flush slabs (the foot
/// up to the cavity, the cavity walls, the entry walls) drawn with their
/// openings and the tails' passages by sketch patterns, the lead-ins two
/// tapered cuts; the foot's passage reaches as far out as the tail's bend.
/// Saved at a 2 × 3
/// socket of 2.54 mm (Würth Elektronik WR-PHD 610006243021: insulator 5 mm
/// wide, top 3.7 mm and underside 0.2 mm over the board, tails 7.2 mm tip
/// to tip); the generator gives every value.
pub fn author_socket_smd_bottom() -> Vec<u8> {
    let mut p = socket_params(2, 3, 2.54, 3.7, 0.5);
    p.extend([
        (
            "standoff",
            "0.2 mm".to_owned(),
            "mm",
            "insulator underside above the board",
        ),
        (
            "cavity_bottom",
            "standoff + entry_depth + 0.3 mm".into(),
            "mm",
            "cavity's floor over the entry",
        ),
        (
            "cavity_top",
            "body_height - entry_depth - 0.3 mm".into(),
            "mm",
            "cavity's roof under the top window",
        ),
        ("tail_span", "7.2 mm".into(), "mm", "tails' tips across"),
        ("tail_sheet", "0.2 mm".into(), "mm", "tail sheet thickness"),
        (
            "tail_bend",
            "0.2 mm".into(),
            "mm",
            "tail bends' centreline radius",
        ),
        (
            "tail_y",
            "(row_pitch / 2 + cavity / 2 + rows * row_pitch / 2) / 2".into(),
            "mm",
            "tails' upright in the long walls (derived)",
        ),
    ]);
    let mut s = new_design("Bottom-entry SMD socket", &p);
    // The tails' passages up the long walls (x across pin 1, y each side
    // out from the wall's inside): the tail's section in the walls round
    // the cavity, and in the insulator's foot as far out as the bend's
    // inner fibre reaches.
    let x = ["x0 - pin_side / 2", "x0 + pin_side / 2"];
    let passage = |inner: &str, outer: &str| {
        let [l, r] = x.map(str::to_owned);
        [
            Hole::Rect([l.clone(), r.clone(), inner.to_owned(), outer.to_owned()]),
            Hole::Rect([l, r, neg(outer), neg(inner)]),
        ]
    };
    let [foot_a, foot_b] = passage("tail_y - tail_sheet / 2", "tail_y + tail_bend");
    let [wall_a, wall_b] = passage("tail_y - tail_sheet / 2", "tail_y + tail_sheet / 2");
    slab(
        &mut s,
        "foot",
        "insulator",
        ["standoff", "cavity_bottom"],
        &[Hole::Square("entry"), foot_a, foot_b],
    );
    slab(
        &mut s,
        "cavity walls",
        "insulator walls",
        ["cavity_bottom", "cavity_top"],
        &[Hole::Square("cavity"), wall_a, wall_b],
    );
    slab(
        &mut s,
        "entry walls",
        "insulator top",
        ["cavity_top", "body_height"],
        &[Hole::Square("entry")],
    );
    lead_ins(&mut s, "lead-ins below", "insulator", "standoff", false);
    lead_ins(
        &mut s,
        "lead-ins above",
        "insulator top",
        "body_height",
        true,
    );
    leaves(&mut s, ["cavity_bottom", "cavity_top"]);
    every_position(&mut s, json!({"bodies": ["leaf a", "leaf b"]}), "leaves");
    let (ty, tip, h) = ((1.27 + 0.71 + 2.54) / 2.0, 3.6, 0.1);
    for (name, minus) in [("tail 1", true), ("tail 2", false)] {
        let sy = |e: &str, value: f64| {
            if minus {
                v(&format!("-({e})"), -value)
            } else {
                v(e, value)
            }
        };
        ortho_strip(
            &mut s,
            name,
            "YZ",
            "x0 - pin_side / 2",
            "pin_side",
            &[
                (sy("tail_y", ty), v("cavity_top", 3.04)),
                (sy("tail_y", ty), v("tail_sheet / 2", h)),
                (sy("tail_span / 2", tip), v("tail_sheet / 2", h)),
            ],
            "tail_sheet / 2",
            "tail_bend",
            None,
        );
        run(
            &mut s,
            "solid.pattern.rectangular",
            json!({"bodies": [name], "dir1": [1, 0, 0], "count1": "pins",
                "spacing1": "pitch", "name": format!("{name} pattern")}),
        );
    }
    // Every leaf and tail the generator can ask for (2 × 64) gets its look
    // at that count, then the file goes back.
    set(&mut s, json!({"pins": "64"}));
    let insulator = ["insulator", "insulator walls", "insulator top"];
    let metal = bodies_but(&s, &insulator);
    assert_eq!(metal.len(), 6 * 64);
    let metal: Vec<&str> = metal.iter().map(String::as_str).collect();
    assign_look(&mut s, &insulator, MaterialId::Pa9t);
    assign_look(&mut s, &metal, MaterialId::Gold);
    set(&mut s, json!({"pins": "3"}));
    finish(s)
}

/// A slab of the socket insulator over z `z0..z1`: the insulator's outline
/// (its upright corners rounded by `corner_round`) with `holes` (a side
/// expression, or a rectangle's corners) round every position, a sketch
/// pattern, extruded once as body `body`, or joined to it when `join`: no
/// Boolean per position. Every slab is the same outline, so the joined
/// insulator's walls are flush.
fn slab(s: &mut Session, name: &str, body: &str, [z0, z1]: [&str; 2], holes: &[Hole<'_>]) {
    let sketch = format!("{name} sketch");
    run(
        s,
        "sketch.create",
        json!({"plane": "XY", "offset": z0, "name": sketch}),
    );
    let (l, r, b, t) = (
        "- pins * pitch / 2",
        "pins * pitch / 2",
        "- rows * row_pitch / 2",
        "rows * row_pitch / 2",
    );
    let outline: Vec<String> = (1..=4).map(|k| format!("{name} edge {k}")).collect();
    run(
        s,
        "sketch.line",
        json!({"points": [[l, b], [r, b], [r, t], [l, t]], "closed": true, "ids": outline}),
    );
    let mut boundary = outline.clone();
    for k in 0..4 {
        let reply = run(
            s,
            "sketch.fillet",
            json!({"a": outline[k], "b": outline[(k + 1) % 4], "radius": "corner_round"}),
        );
        if let Some(arc) = reply["arc"].as_str().or_else(|| reply["id"].as_str()) {
            boundary.push(arc.to_owned());
        } else if let Some(c) = reply["curves"].as_array() {
            boundary.extend(c.iter().filter_map(|c| c.as_str().map(str::to_owned)));
        } else {
            panic!("fillet reply {reply}");
        }
    }
    // Squares at every position; rectangles at every position along x.
    let (mut squares, mut along) = (Vec::new(), Vec::new());
    for (k, hole) in holes.iter().enumerate() {
        let corners = match hole {
            Hole::Square(side) => {
                let c = |sx: &str, sy: &str| {
                    json!([
                        format!("x0 {sx} ({side}) / 2"),
                        format!("y0 {sy} ({side}) / 2")
                    ])
                };
                [c("-", "-"), c("+", "-"), c("+", "+"), c("-", "+")]
            }
            Hole::Rect([x0, x1, y0, y1]) => [
                json!([x0, y0]),
                json!([x1, y0]),
                json!([x1, y1]),
                json!([x0, y1]),
            ],
        };
        let hole_ids: Vec<String> = (1..=4).map(|i| format!("{name} hole {k} {i}")).collect();
        run(
            s,
            "sketch.line",
            json!({"points": corners, "closed": true, "ids": hole_ids}),
        );
        match hole {
            Hole::Square(_) => squares.extend(hole_ids),
            Hole::Rect(_) => along.extend(hole_ids),
        }
    }
    if !squares.is_empty() {
        run(
            s,
            "sketch.pattern.rectangular",
            json!({"entities": squares, "dir": [1, 0], "count": "pins", "spacing": "pitch",
                "dir2": [0, 1], "count2": "rows", "spacing2": "row_pitch"}),
        );
    }
    if !along.is_empty() {
        run(
            s,
            "sketch.pattern.rectangular",
            json!({"entities": along, "dir": [1, 0], "count": "pins", "spacing": "pitch"}),
        );
    }
    run(s, "sketch.finish", json!({}));
    run(
        s,
        "solid.extrude",
        json!({"sketch": sketch, "distance": format!("({z1}) - ({z0})"), "name": name,
            "operation": "new", "body_name": body}),
    );
    // The region inside the outline, found again at every count.
    let curves: solidcraft_engine::doc::ProfileSel =
        serde_json::from_value(json!({"type": "curves", "loops": [boundary]})).unwrap();
    let feature = s
        .doc_mut()
        .features
        .iter_mut()
        .find(|f| f.name == name)
        .unwrap();
    match &mut feature.kind {
        solidcraft_engine::doc::FeatureKind::Extrude { profiles, .. } => *profiles = curves,
        other => panic!("{name}: {other:?}"),
    }
    s.refresh();
}

/// `-(e)`.
fn neg(e: &str) -> String {
    format!("-({e})")
}

/// A hole of a [`slab`] at pin 1: a square of a side (at every position),
/// or a rectangle `[x0, x1, y0, y1]` (at every position along x).
enum Hole<'a> {
    Square(&'a str),
    Rect([String; 4]),
}
