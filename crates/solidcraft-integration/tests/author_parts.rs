//! Authors Sparksmith's shipped part files (docs/part-templates.md §11)
//! through Solidcraft's own commands and public API, and writes them under
//! `library/parts/`. Original work (MIT OR Apache-2.0).
//!
//! The ESP32-S31-WROOM-3 module body (`library/parts/esp32-s31-wroom-3`)
//! is no longer written here: it was saved from the retired in-code module
//! builder's construction of the example project's form (commit
//! 95f065dd and before), and stays as it was, library data the example
//! projects pack byte for byte. The `module` generator's part files below
//! build the same bodies at its values (`part_file` parity test).
//! Appearances are named after Sparksmith materials; the marking surfaces
//! are construction planes `marking-<name>` placed by expressions over a
//! part's parameters, sized by `marking_<name>_u` / `_v`; `pin1` is a
//! construction point.
//!
//! The mechanical starter set (docs/mechanical-parts.md §M7: SMD nut,
//! round spacer, finned heatsink) is drawn directly from primitives and a
//! parametric pattern over plain parameters, with generic illustrative
//! dimensions.
//!
//! `cargo test -p sparksmith-solidcraft --test author_parts -- --ignored`
//! (`AUTHOR_ONLY=<name>[,<name>…]` writes those part files only)
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::json;
use solidcraft_engine::Session;
use sparksmith_core::materials::MaterialId;
use sparksmith_solidcraft::models::{look, part_file};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// A test length in mm, as stored.
#[allow(dead_code)]
fn nm(mm: f64) -> sparksmith_core::units::Nm {
    sparksmith_core::units::Nm::from_mm_f64_rounded(mm).unwrap_or_default()
}

/// A test point or size in mm, as stored.
#[allow(dead_code)]
fn pt(p: [f64; 2]) -> sparksmith_core::units::PointNm {
    sparksmith_core::units::PointNm::from_mm_f64_rounded(p).unwrap_or_default()
}

#[path = "author_parts/tht.rs"]
mod tht;

#[path = "author_parts/connectors.rs"]
mod connectors;
#[path = "author_parts/families.rs"]
mod families;

fn run(s: &mut Session, command: &str, params: serde_json::Value) -> serde_json::Value {
    s.execute(command, &params)
        .unwrap_or_else(|e| panic!("{command} {params}: {e}"))
}

/// A `marking-<name>` plane: origin expressions, u axis and normal.
struct Mark {
    name: &'static str,
    origin: [String; 3],
    u: [f64; 3],
    normal: [f64; 3],
    size: [String; 2],
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// The `header` generator's part file: a straight pin header of `rows` ×
/// `pins` square posts through a moulded base, the in-code header
/// builder's construction made parametric in its counts. The base is one
/// pitch per pin long and one row pitch per row wide on the seating plane;
/// a post of `pin_side` stands `pin_above` over the base and reaches
/// `pin_below` under the seating plane at pin 1 (−x, −y), and a
/// rectangular pattern of `pins` × `rows` replays it. The base's hole for
/// each post is a cut of the post's own section, patterned the same way,
/// so posts and base stay disjoint at any count and nothing names the
/// pattern's bodies. Rounds on the base's upright corners and top edges.
/// Saved at a 2 × 5 header of 2.54 mm; the generator gives every value.
pub fn author_pin_header() -> Vec<u8> {
    let mut s = Session::default();
    run(&mut s, "file.new", json!({"name": "Pin header"}));
    let params = [
        ("rows", "2", "", "rows of pins (y)"),
        ("pins", "5", "", "pins per row (x)"),
        ("pitch", "2.54 mm", "mm", "pin pitch along a row"),
        ("row_pitch", "2.54 mm", "mm", "pitch between rows"),
        (
            "body_height",
            "2.54 mm",
            "mm",
            "base height above the seating plane",
        ),
        ("pin_side", "0.64 mm", "mm", "square post side"),
        ("pin_above", "6 mm", "mm", "post length above the base"),
        (
            "pin_below",
            "3 mm",
            "mm",
            "post length below the seating plane",
        ),
        (
            "corner_round",
            "0.3 mm",
            "mm",
            "round of the base's upright corners",
        ),
        (
            "edge_round",
            "0.2 mm",
            "mm",
            "round of the base's top edges",
        ),
    ];
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
    run(
        &mut s,
        "solid.box",
        json!({"corner": ["- pins * pitch / 2", "- rows * row_pitch / 2", "0 mm"],
            "length": "pins * pitch", "width": "rows * row_pitch", "height": "body_height",
            "name": "body", "body_name": "body"}),
    );
    let post = json!([
        "- (pins - 1) / 2 * pitch - pin_side / 2",
        "- (rows - 1) / 2 * row_pitch - pin_side / 2",
        "- pin_below"
    ]);
    run(
        &mut s,
        "solid.box",
        json!({"corner": post, "length": "pin_side", "width": "pin_side",
            "height": "pin_below + body_height + pin_above", "name": "pin", "body_name": "pin"}),
    );
    run(
        &mut s,
        "solid.box",
        json!({"corner": post, "length": "pin_side", "width": "pin_side",
            "height": "pin_below + body_height + pin_above", "operation": "cut",
            "targets": ["body"], "name": "pin hole"}),
    );
    let pattern = |what: serde_json::Value, name: &str| {
        let mut p = json!({"dir1": [1, 0, 0], "count1": "pins", "spacing1": "pitch",
            "dir2": [0, 1, 0], "count2": "rows", "spacing2": "row_pitch", "name": name});
        if let (Some(map), Some(extra)) = (p.as_object_mut(), what.as_object()) {
            map.extend(extra.clone());
        }
        p
    };
    run(
        &mut s,
        "solid.pattern.rectangular",
        pattern(json!({"features": ["pin hole"]}), "pin holes"),
    );
    run(
        &mut s,
        "solid.pattern.rectangular",
        pattern(json!({"bodies": ["pin"]}), "pin pattern"),
    );
    // The rounds, picked at the saved size: 12.7 × 5.08 × 2.54 mm.
    let (x, y, z) = (6.35, 2.54, 2.54);
    run(
        &mut s,
        "solid.fillet",
        json!({"name": "corner round", "body": "body", "radius": "corner_round",
            "edges": [[-x, -y, z / 2.0], [x, -y, z / 2.0], [x, y, z / 2.0], [-x, y, z / 2.0]]}),
    );
    run(
        &mut s,
        "solid.fillet",
        json!({"name": "edge round", "body": "body", "radius": "edge_round",
            "edges": [[0.0, -y, z], [x, 0.0, z], [0.0, y, z], [-x, 0.0, z]]}),
    );
    // Looks are kept by body name, and a pattern names its copies
    // `pin (2)`, `pin (3)`, …: every post the generator can ask for (its
    // 4 × 64 most) gets the gold look at that count, then the file goes
    // back to 2 × 5.
    let counts = |s: &mut Session, rows: u32, pins: u32| {
        run(
            s,
            "parameters.change",
            json!({"changes": {"rows": rows.to_string(), "pins": pins.to_string()}}),
        );
        s.refresh();
    };
    counts(&mut s, 4, 64);
    let pins: Vec<String> = s
        .model
        .state()
        .bodies
        .iter()
        .map(|b| b.name.clone())
        .filter(|n| n != "body")
        .collect();
    assert_eq!(pins.len(), 256);
    let pins: Vec<&str> = pins.iter().map(String::as_str).collect();
    assign_look(&mut s, &["body"], MaterialId::Pbt);
    assign_look(&mut s, &pins, MaterialId::Gold);
    counts(&mut s, 2, 5);
    let errors: Vec<String> = s
        .model
        .results
        .iter()
        .filter_map(|r| r.error.clone().map(|e| format!("{}: {e}", r.name)))
        .collect();
    assert!(errors.is_empty(), "{errors:?}");
    solidcraft_engine::io::write_design(&s.doc)
}

/// How a molded two-terminal package's terminals are formed: one part
/// file each (the lead bodies differ in their construction), with and
/// without the polarity band (a band is a body of its own).
#[derive(Clone, Copy, PartialEq, Eq)]
enum MoldedLead {
    /// Flat strips on the seating plane under the body ends (SOD-123F,
    /// SOD-323F, SOD-523).
    Flat,
    /// Strips leaving the body ends at mid-height and bending down to a
    /// flat foot (SOD-123, SOD-323).
    Bent,
    /// J-leads folded down the end faces and under the body (SMA/SMB/SMC,
    /// moulded tantalum).
    JLead,
}

/// A user parameter row for `parameters.import`.
fn param(name: &str, expression: &str, unit: &str, comment: &str) -> serde_json::Value {
    json!({"name": name, "expression": expression, "unit": unit, "comment": comment})
}

fn import(s: &mut Session, rows: &[serde_json::Value]) {
    run(
        s,
        "parameters.import",
        json!({"text": serde_json::to_string(rows).unwrap()}),
    );
}

/// A box over x `from`..`to` (expressions on the +x side; `minus` mirrors
/// it to −x), y ± `width` / 2 and z `z0`..`z0 + height`.
#[allow(clippy::too_many_arguments)]
fn x_box(
    s: &mut Session,
    name: &str,
    body: Option<&str>,
    [from, to]: [&str; 2],
    minus: bool,
    width: &str,
    [z0, height]: [&str; 2],
    operation: Option<(&str, &str)>,
) {
    let x = if minus {
        format!("- ({to})")
    } else {
        from.to_owned()
    };
    let mut p = json!({"corner": [x, format!("- ({width}) / 2"), z0],
        "length": format!("({to}) - ({from})"), "width": width, "height": height, "name": name});
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

fn fillet(s: &mut Session, name: &str, body: &str, at: [f64; 3], radius: &str) {
    run(
        s,
        "solid.fillet",
        json!({"name": name, "body": body, "radius": radius, "edges": [at]}),
    );
}

fn assert_evaluates(s: &mut Session) {
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
}

/// A transfer-moulded body `body` as the two mould halves leave it: the
/// outline `length` × `width` (expressions) with its upright corners
/// rounded by `corner` lies at the parting line `split`, where the leads
/// leave; the upper half rises to `top` and the lower half falls to
/// `bottom`, each wall drawing in by the `draft` angle away from the line
/// (Microchip's SOIC and SOT drawings give a mould draft of 5–15° top and
/// bottom), so the line reads as a crease round the body. The top edge
/// loop is then rounded by `edge`, picked at `pick` (a point on the top
/// edge at the saved size).
#[allow(clippy::too_many_arguments)]
fn moulded_body(
    s: &mut Session,
    body: &str,
    [length, width]: [&str; 2],
    [bottom, split, top]: [&str; 3],
    corner: &str,
    draft: &str,
    edge: &str,
    pick: [f64; 3],
) {
    let sketch = format!("{body} parting line");
    run(
        s,
        "sketch.create",
        json!({"plane": "XY", "offset": split, "name": sketch}),
    );
    run(
        s,
        "sketch.rectangle.rounded",
        json!({"p0": [format!("-({length}) / 2"), format!("-({width}) / 2")],
            "p1": [format!("({length}) / 2"), format!("({width}) / 2")], "radius": corner}),
    );
    run(s, "sketch.finish", json!({}));
    run(
        s,
        "solid.extrude",
        json!({"sketch": sketch, "distance": format!("({top}) - ({split})"),
            "taper": format!("-({draft})"), "operation": "new",
            "name": format!("{body} upper mould"), "body_name": body}),
    );
    run(
        s,
        "solid.extrude",
        json!({"sketch": sketch, "distance": format!("({split}) - ({bottom})"),
            "direction": "negative", "taper": format!("-({draft})"), "operation": "join",
            "targets": [body], "name": format!("{body} lower mould")}),
    );
    run(
        s,
        "solid.fillet",
        json!({"name": format!("{body} top round"), "body": body, "radius": edge,
            "edges": [pick], "fit": "reduce"}),
    );
}

/// `marking-top`, the area of the top left for text: its centre `x` and
/// height `z`, u along +x, normal +z; sized by `marking_top_u` and `_v`.
fn marking_top(s: &mut Session, [x, z]: [&str; 2], [u, v]: [&str; 2]) {
    import(
        s,
        &[
            param(
                "marking_top_u",
                u,
                "mm",
                "marking-top size along its u axis",
            ),
            param("marking_top_v", v, "mm", "marking-top size across it"),
        ],
    );
    run(
        s,
        "construct.plane.offset",
        json!({"base": {"origin": [x, "0 mm", z], "x_dir": [1, 0, 0], "y_dir": [0, 1, 0]},
            "offset": "0 mm", "name": "marking-top"}),
    );
}

/// The polarity band's parameters: a fifth to a sixth of the body (0.15 of
/// its length), inset 0.08 of the length from the end of the visible top
/// at terminal 1 (`top_from`), across 0.8 of the width, `depth` deep (the
/// core's band area, `MoldedForm::band_area`, est.).
fn band_params(s: &mut Session, top_from: &str, width: &str, depth: &str) {
    import(
        s,
        &[
            param(
                "band_width",
                "0.15 * body_length",
                "mm",
                "polarity band length along x",
            ),
            param(
                "band_start",
                &format!("{top_from} + 0.08 * body_length"),
                "mm",
                "polarity band's end towards terminal 1",
            ),
            param(
                "band_across",
                &format!("0.8 * {width}"),
                "mm",
                "polarity band across the body",
            ),
            param("band_depth", depth, "mm", "how deep the band is inlaid"),
        ],
    );
}

/// The `molded` generator's part file of `lead` terminals, with the
/// polarity band at terminal 1 when `band`: a moulded epoxy body with two
/// tin terminals, terminal 1 at −x, terminal 2 at +x. Rebuilt from the
/// in-code molded builder's construction by Solidcraft commands:
///
/// - the body a box from `body_bottom` (0 but under bent leads) to
///   `body_height`, cut round the leads it holds (kept disjoint), its
///   upright corners and top edges rounded (`corner_round`, `edge_round`);
/// - flat leads: boxes of `lead_length` × `terminal_width` ×
///   `lead_thickness` from the tips inward;
/// - bent leads: a strip of `lead_thickness` from `lead_root` inside the
///   body end out at `lead_exit` to `lead_shoulder`, down to the foot
///   (`terminal_length` long) at the tip, as the union of three straight
///   boxes mitred at the bends (the slope a box turned about y by the
///   strip's angle) whose outer and inner bend edges are rounded about
///   the bend centre (centreline radii `bend_1`, `bend_2`);
/// - J-leads: the same strip out at `lead_exit`, down the end face and
///   under the body, made as a box with the inside of the J cut out;
/// - the band a thin box inlaid in the top.
///
/// Saved at a typical size of the class (flat: SOD-123F, bent: SOD-123,
/// J-lead: SMA); the generator gives every value but the band's and the
/// marking area's, which follow the body.
fn author_molded(lead: MoldedLead, band: bool) -> Vec<u8> {
    // The saved size, and the values the generator computes for it (the
    // core's estimates: leadframe 0.12 mm (flat, bent) or 0.15 mm (J),
    // at most a quarter of the height; a root of at most a sixth of the
    // body length; bends of the lead thickness, the bent lead's shoulder
    // 0.3 of the way out; rounds capped by the body).
    let (bl, bw, bh, span, tl, tw) = match lead {
        MoldedLead::Flat => (2.7, 1.6, 1.1, 3.7, 0.5, 0.6),
        MoldedLead::Bent => (2.7, 1.6, 1.1, 3.7, 0.3, 0.6),
        MoldedLead::JLead => (4.3, 2.6, 2.2, 5.2, 1.1, 1.4),
    };
    let t: f64 = if lead == MoldedLead::JLead {
        0.15
    } else {
        0.12
    };
    let t = t.min(bh / 4.0);
    let h = t / 2.0;
    let bottom: f64 = if lead == MoldedLead::Bent {
        0.05f64.min(bh / 10.0)
    } else {
        0.0
    };
    let exit = bottom + 0.5 * (bh - bottom);
    let root = 0.25f64.min(bl / 6.0);
    let (hb, hs) = (bl / 2.0, span / 2.0);
    let foot_in = hs - tl;
    let shoulder = hb + 0.3 * (foot_in - hb);
    let lead_top = if lead == MoldedLead::Flat {
        t
    } else {
        exit + h
    };
    let cap = (0.08 * bl.min(bw)).min(0.064 * bl);
    let corner = 0.2f64.min(cap).min((bw - tw) / 4.0);
    let edge = 0.15f64
        .min(0.12 * bh)
        .min(cap)
        .min((bh - lead_top) / 2.0)
        .min(corner);
    let title = match (lead, band) {
        (MoldedLead::Flat, false) => "Molded two-terminal, flat leads",
        (MoldedLead::Flat, true) => "Molded two-terminal, flat leads, band",
        (MoldedLead::Bent, false) => "Molded two-terminal, bent leads",
        (MoldedLead::Bent, true) => "Molded two-terminal, bent leads, band",
        (MoldedLead::JLead, false) => "Molded two-terminal, J-leads",
        (MoldedLead::JLead, true) => "Molded two-terminal, J-leads, band",
    };
    let mut s = Session::default();
    run(&mut s, "file.new", json!({"name": title}));
    let mm = |v: f64| format!("{} mm", (v * 1e6).round() / 1e6);
    let mut rows = vec![
        param("body_length", &mm(bl), "mm", "moulded body length (x)"),
        param("body_width", &mm(bw), "mm", "moulded body width (y)"),
        param(
            "body_height",
            &mm(bh),
            "mm",
            "body top above the seating plane",
        ),
        param("span", &mm(span), "mm", "overall length, tip to tip"),
        param(
            "terminal_length",
            &mm(tl),
            "mm",
            "terminal contact length on the seating plane, from the tip",
        ),
        param("terminal_width", &mm(tw), "mm", "terminal width (y)"),
        param("lead_thickness", &mm(t), "mm", "leadframe thickness"),
        param(
            "lead_root",
            &mm(root),
            "mm",
            "how far a lead reaches into the body",
        ),
        param(
            "corner_round",
            &mm(corner),
            "mm",
            "round of the body's upright corners",
        ),
        param(
            "edge_round",
            &mm(edge),
            "mm",
            "round of the body's top edges",
        ),
    ];
    match lead {
        MoldedLead::Flat => {
            let length = if tl >= hs - hb + root {
                tl
            } else {
                hs - hb + root
            };
            rows.push(param(
                "lead_length",
                &mm(length),
                "mm",
                "flat lead length from the tip inward",
            ));
        }
        MoldedLead::Bent | MoldedLead::JLead => {
            if lead == MoldedLead::Bent {
                rows.push(param(
                    "body_bottom",
                    &mm(bottom),
                    "mm",
                    "the body's underside above the seating plane",
                ));
                rows.push(param(
                    "lead_shoulder",
                    &mm(shoulder),
                    "mm",
                    "x of the bend where the lead turns down",
                ));
            }
            rows.push(param(
                "lead_exit",
                &mm(exit),
                "mm",
                "height of the lead's centre line where it leaves the body",
            ));
            rows.push(param(
                "bend_1",
                &mm(t),
                "mm",
                "centre-line radius of the bend at the body",
            ));
            rows.push(param(
                "bend_2",
                &mm(t),
                "mm",
                "centre-line radius of the bend at the foot",
            ));
        }
    }
    import(&mut s, &rows);
    if lead == MoldedLead::Bent {
        // The slope: its run and drop, its angle below +x and how far the
        // mitred ends reach past each bend's corner.
        import(
            &mut s,
            &[
                param(
                    "lead_run",
                    "span / 2 - terminal_length - lead_shoulder",
                    "mm",
                    "the slope's run along x",
                ),
                param(
                    "lead_drop",
                    "lead_exit - lead_thickness / 2",
                    "mm",
                    "the slope's drop",
                ),
                param(
                    "lead_angle",
                    "atan2(lead_drop, lead_run)",
                    "deg",
                    "the slope below the horizontal",
                ),
                param(
                    "lead_miter",
                    "lead_thickness / 2 * tan(lead_angle / 2)",
                    "mm",
                    "how far a mitred end reaches past its bend's corner",
                ),
            ],
        );
    }
    let bottom_expr = if lead == MoldedLead::Bent {
        "body_bottom"
    } else {
        "0 mm"
    };
    run(
        &mut s,
        "solid.box",
        json!({"corner": ["- body_length / 2", "- body_width / 2", bottom_expr],
            "length": "body_length", "width": "body_width",
            "height": format!("body_height - {bottom_expr}"), "name": "body", "body_name": "body"}),
    );
    for (k, minus) in [(1, true), (2, false)] {
        let name = format!("terminal {k}");
        let sign = if minus { -1.0 } else { 1.0 };
        match lead {
            MoldedLead::Flat => x_box(
                &mut s,
                &name,
                Some(&name),
                ["span / 2 - lead_length", "span / 2"],
                minus,
                "terminal_width",
                ["0 mm", "lead_thickness"],
                None,
            ),
            MoldedLead::JLead => {
                // The J's bounding box from whichever reaches further in,
                // the run's root or the foot's inner end, less the inside
                // of the J, what lies inward of the foot below the run and
                // what lies inward of the run's root above the foot.
                let from = "min(body_length / 2 - lead_root, span / 2 - terminal_length)";
                x_box(
                    &mut s,
                    &name,
                    Some(&name),
                    [from, "span / 2"],
                    minus,
                    "terminal_width",
                    ["0 mm", "lead_exit + lead_thickness / 2"],
                    None,
                );
                let cut = |s: &mut Session, what: &str, to: &str, z: [&str; 2]| {
                    x_box(
                        s,
                        &format!("{name} {what}"),
                        None,
                        [&format!("{from} - 1 mm"), to],
                        minus,
                        "terminal_width + 2 mm",
                        z,
                        Some(("cut", &name)),
                    );
                };
                cut(
                    &mut s,
                    "inside",
                    "span / 2 - lead_thickness",
                    ["lead_thickness", "lead_exit - 3 * lead_thickness / 2"],
                );
                cut(
                    &mut s,
                    "inward of the foot",
                    "span / 2 - terminal_length",
                    ["- 1 mm", "lead_exit - lead_thickness / 2 + 1 mm"],
                );
                cut(
                    &mut s,
                    "inward of the root",
                    "body_length / 2 - lead_root",
                    ["lead_thickness", "lead_exit + lead_thickness / 2"],
                );
                let (outer, inner) = (hs, hs - t);
                for (bend, outer_z, inner_z) in [(1, exit + h, exit - h), (2, 0.0, t)] {
                    fillet(
                        &mut s,
                        &format!("{name} bend {bend} outside"),
                        &name,
                        [sign * outer, 0.0, outer_z],
                        &format!("bend_{bend} + lead_thickness / 2"),
                    );
                    fillet(
                        &mut s,
                        &format!("{name} bend {bend} inside"),
                        &name,
                        [sign * inner, 0.0, inner_z],
                        &format!("bend_{bend} - lead_thickness / 2"),
                    );
                }
            }
            MoldedLead::Bent => {
                // The run out of the body to past the shoulder, the slope,
                // and the foot from before its bend to the tip, each
                // reaching its mitre.
                x_box(
                    &mut s,
                    &name,
                    Some(&name),
                    ["body_length / 2 - lead_root", "lead_shoulder + lead_miter"],
                    minus,
                    "terminal_width",
                    ["lead_exit - lead_thickness / 2", "lead_thickness"],
                    None,
                );
                let slope = format!("{name} slope");
                run(
                    &mut s,
                    "solid.box",
                    json!({"corner": ["- hypot(lead_run, lead_drop) / 2 - lead_miter",
                            "- terminal_width / 2", "- lead_thickness / 2"],
                        "length": "hypot(lead_run, lead_drop) + 2 * lead_miter",
                        "width": "terminal_width", "height": "lead_thickness",
                        "name": slope, "body_name": slope}),
                );
                let x = "(lead_shoulder + span / 2 - terminal_length) / 2";
                run(
                    &mut s,
                    "solid.move",
                    json!({"bodies": [slope], "axis": [0, 1, 0],
                        "angle": if minus { "- lead_angle" } else { "lead_angle" },
                        "translate": [if minus { format!("- {x}") } else { x.to_owned() }, "0 mm",
                            "(lead_exit + lead_thickness / 2) / 2"],
                        "name": format!("{slope} placed")}),
                );
                run(
                    &mut s,
                    "solid.combine",
                    json!({"target": name, "tools": [slope], "operation": "join",
                        "name": format!("{name} joined")}),
                );
                x_box(
                    &mut s,
                    &format!("{name} foot"),
                    None,
                    ["span / 2 - terminal_length - lead_miter", "span / 2"],
                    minus,
                    "terminal_width",
                    ["0 mm", "lead_thickness"],
                    Some(("join", &name)),
                );
                let (drop, run_x) = (exit - h, foot_in - shoulder);
                let miter = h * (drop.atan2(run_x) / 2.0).tan();
                for (bend, outer, inner) in [
                    (
                        1,
                        [shoulder + miter, exit + h],
                        [shoulder - miter, exit - h],
                    ),
                    (2, [foot_in - miter, 0.0], [foot_in + miter, t]),
                ] {
                    fillet(
                        &mut s,
                        &format!("{name} bend {bend} outside"),
                        &name,
                        [sign * outer[0], 0.0, outer[1]],
                        &format!("bend_{bend} + lead_thickness / 2"),
                    );
                    fillet(
                        &mut s,
                        &format!("{name} bend {bend} inside"),
                        &name,
                        [sign * inner[0], 0.0, inner[1]],
                        &format!("bend_{bend} - lead_thickness / 2"),
                    );
                }
            }
        }
    }
    run(
        &mut s,
        "solid.combine",
        json!({"target": "body", "tools": ["terminal 1", "terminal 2"], "operation": "cut",
            "keep_tools": true, "name": "lead roots"}),
    );
    // The rounds, picked at the saved size: upright corners above the
    // leads, then the top edges.
    let z = (lead_top + bh) / 2.0;
    let corners: Vec<[f64; 3]> = [
        [-hb, -bw / 2.0],
        [hb, -bw / 2.0],
        [hb, bw / 2.0],
        [-hb, bw / 2.0],
    ]
    .iter()
    .map(|[x, y]| [*x, *y, z])
    .collect();
    run(
        &mut s,
        "solid.fillet",
        json!({"name": "corner round", "body": "body", "radius": "corner_round", "edges": corners}),
    );
    run(
        &mut s,
        "solid.fillet",
        json!({"name": "edge round", "body": "body", "radius": "edge_round",
            "edges": [[0.0, -bw / 2.0, bh], [hb, 0.0, bh], [0.0, bw / 2.0, bh], [-hb, 0.0, bh]]}),
    );
    if band {
        band_params(&mut s, "- body_length / 2", "body_width", "0.03 mm");
        run(
            &mut s,
            "solid.box",
            json!({"corner": ["band_start", "- band_across / 2", "body_height - band_depth"],
                "length": "band_width", "width": "band_across", "height": "band_depth",
                "name": "band", "body_name": "band"}),
        );
        run(
            &mut s,
            "solid.combine",
            json!({"target": "body", "tools": ["band"], "operation": "cut", "keep_tools": true,
                "name": "band inlay"}),
        );
        assign_look(&mut s, &["band"], MaterialId::EpoxyLaserMarked);
    }
    assign_look(&mut s, &["body"], MaterialId::EpoxyMould);
    assign_look(&mut s, &["terminal 1", "terminal 2"], MaterialId::TinMatte);
    // The top left for text: beside the band (or the inset), to the inset
    // before terminal 2 (`MoldedForm::marking_area`).
    let from = if band {
        "band_start + band_width"
    } else {
        "- body_length / 2 + 0.08 * body_length"
    };
    let to = "body_length / 2 - 0.08 * body_length";
    marking_top(
        &mut s,
        [&format!("(({from}) + ({to})) / 2"), "body_height"],
        [&format!("({to}) - ({from})"), "0.8 * body_width"],
    );
    assert_evaluates(&mut s);
    solidcraft_engine::io::write_design(&s.doc)
}

/// The `molded` generator's MELF part file, with the polarity band at
/// terminal 1 when `band`: a round borosilicate glass body between two
/// tin end caps on one axis along x, `cap_diameter` / 2 above the seating
/// plane; the body is cut where the caps seat on it, each cap's outer
/// edge is rounded (`cap_round`), and the band is a thin ring of dark
/// epoxy inlaid in the glass. Rebuilt from the in-code molded builder's
/// construction by Solidcraft commands; saved at a MiniMELF (DO-213AA).
fn author_melf(band: bool) -> Vec<u8> {
    let (bl, bd, span, tl, cd) = (3.5, 1.5, 3.5, 0.4, 1.5);
    let rc = cd / 2.0;
    let cap_round = 0.08f64
        .min(0.15 * tl)
        .min(0.1 * rc)
        .min((rc - bd / 2.0).max(0.0) + 0.5 * tl);
    let mut s = Session::default();
    run(
        &mut s,
        "file.new",
        json!({"name": if band { "MELF, band" } else { "MELF" }}),
    );
    let mm = |v: f64| format!("{} mm", (v * 1e6).round() / 1e6);
    import(
        &mut s,
        &[
            param("body_length", &mm(bl), "mm", "glass body length (x)"),
            param("body_diameter", &mm(bd), "mm", "glass body diameter"),
            param("span", &mm(span), "mm", "overall length, cap to cap"),
            param("terminal_length", &mm(tl), "mm", "end cap length"),
            param("cap_diameter", &mm(cd), "mm", "end cap diameter"),
            param(
                "cap_round",
                &mm(cap_round),
                "mm",
                "round of each cap's outer edge",
            ),
        ],
    );
    // Each round body is circles drawn on a YZ sketch at `from` (x),
    // centred on the axis, extruded `length` along +x.
    let disc =
        |s: &mut Session, name: &str, from: &str, diameters: &[(&str, f64)], length: &str| {
            let reply = run(
                s,
                "sketch.create",
                json!({"plane": "YZ", "offset": from, "name": format!("{name} sketch")}),
            );
            for (diameter, at) in diameters {
                run(
                    s,
                    "sketch.draw.dimensioned",
                    json!({"command": "sketch.circle.center",
                    "params": {"center": ["0 mm", "cap_diameter / 2"], "radius": at / 2.0},
                    "dimensions": {"diameter": diameter}}),
                );
            }
            run(s, "sketch.finish", json!({}));
            run(
                s,
                "solid.extrude",
                json!({"sketch": reply["sketch"], "distance": length, "operation": "new",
                "name": name, "body_name": name}),
            );
        };
    disc(
        &mut s,
        "body",
        "- body_length / 2",
        &[("body_diameter", bd)],
        "body_length",
    );
    for (k, x) in [(1, "- span / 2"), (2, "span / 2 - terminal_length")] {
        disc(
            &mut s,
            &format!("cap {k}"),
            x,
            &[("cap_diameter", cd)],
            "terminal_length",
        );
    }
    run(
        &mut s,
        "solid.combine",
        json!({"target": "body", "tools": ["cap 1", "cap 2"], "operation": "cut",
            "keep_tools": true, "name": "cap seats"}),
    );
    for (k, x) in [(1, -span / 2.0), (2, span / 2.0)] {
        let name = format!("cap {k}");
        fillet(
            &mut s,
            &format!("{name} round"),
            &name,
            [x, 0.0, cd],
            "cap_round",
        );
    }
    if band {
        // The visible top runs between the caps (or the body's ends).
        band_params(
            &mut s,
            "- min(span / 2 - terminal_length, body_length / 2)",
            "body_diameter",
            "min(0.03 mm, body_diameter / 8)",
        );
        disc(
            &mut s,
            "band",
            "band_start",
            &[
                ("body_diameter", bd),
                ("body_diameter - 2 * band_depth", bd - 0.06),
            ],
            "band_width",
        );
        run(
            &mut s,
            "solid.combine",
            json!({"target": "body", "tools": ["band"], "operation": "cut", "keep_tools": true,
                "name": "band inlay"}),
        );
        assign_look(&mut s, &["band"], MaterialId::EpoxyMould);
    }
    assign_look(&mut s, &["body"], MaterialId::BorosilicateGlass);
    assign_look(&mut s, &["cap 1", "cap 2"], MaterialId::TinMatte);
    let half = "min(span / 2 - terminal_length, body_length / 2)";
    let from = if band {
        "band_start + band_width".to_owned()
    } else {
        format!("- {half} + 0.08 * body_length")
    };
    let to = format!("{half} - 0.08 * body_length");
    marking_top(
        &mut s,
        [
            &format!("(({from}) + ({to})) / 2"),
            "max((cap_diameter - body_diameter) / 2, 0 mm) + body_diameter",
        ],
        [&format!("({to}) - ({from})"), "0.8 * body_diameter"],
    );
    assert_evaluates(&mut s);
    solidcraft_engine::io::write_design(&s.doc)
}

fn author_molded_flat() -> Vec<u8> {
    author_molded(MoldedLead::Flat, false)
}
fn author_molded_flat_band() -> Vec<u8> {
    author_molded(MoldedLead::Flat, true)
}
fn author_molded_bent() -> Vec<u8> {
    author_molded(MoldedLead::Bent, false)
}
fn author_molded_bent_band() -> Vec<u8> {
    author_molded(MoldedLead::Bent, true)
}
fn author_molded_j_lead() -> Vec<u8> {
    author_molded(MoldedLead::JLead, false)
}
fn author_molded_j_lead_band() -> Vec<u8> {
    author_molded(MoldedLead::JLead, true)
}
fn author_melf_plain() -> Vec<u8> {
    author_melf(false)
}
fn author_melf_band() -> Vec<u8> {
    author_melf(true)
}

/// The kind of a `chip` generator part file.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Chip {
    Resistor,
    Capacitor,
    Ferrite,
    Led,
}

/// A part file of the `chip` generator's bundle: a two-terminal chip of
/// `length` × `width` × `height` with `termination` long end caps, as the
/// manufacturers' photographs and drawings show them (Yageo RC, Murata
/// GRM, Würth WL-SMCW general specifications: the caps are the part's
/// outline and stand proud of the body by their plating). Each cap is matte
/// tin over the end and a band of `termination` on the four faces, its
/// outer edges rounded by `end_round` (dipped and barrel-plated); the body
/// between them is `plating` smaller each way so the caps stand proud, its
/// long edges rounded by `body_round`. A resistor's alumina substrate
/// carries its dark overcoat (`overcoat` thick, inset from the long edges
/// and short of the caps, so a white rim shows round it), where its value
/// code is printed; an MLCC's body is bare barium titanate ceramic; a
/// ferrite bead's dark ferrite. An LED's white substrate is `substrate`
/// thick with the caps wrapping it, under a clear lens rounded by
/// `lens_round` with the die and its bond pad inside. Saved at an 0603
/// (1608 metric); the generator gives every value.
fn author_chip(kind: Chip) -> Vec<u8> {
    let mut s = Session::default();
    let title = match kind {
        Chip::Resistor => "Chip resistor body",
        Chip::Capacitor => "Chip capacitor (MLCC) body",
        Chip::Ferrite => "Chip ferrite bead body",
        Chip::Led => "Chip LED body",
    };
    run(&mut s, "file.new", json!({"name": title}));
    let led = kind == Chip::Led;
    let resistor = kind == Chip::Resistor;
    let (l, w, h, t) = (1.6, 0.8, if led { 0.75 } else { 0.45 }, 0.3);
    let mut rows = vec![
        param("length", "1.6 mm", "mm", "length over the terminations (x)"),
        param("width", "0.8 mm", "mm", "width (y)"),
        param(
            "height",
            if led { "0.75 mm" } else { "0.45 mm" },
            "mm",
            "height above the seating plane",
        ),
        param(
            "termination",
            "0.3 mm",
            "mm",
            "each termination's length, end cap and band",
        ),
        param(
            "body_round",
            if resistor { "0.03 mm" } else { "0.045 mm" },
            "mm",
            "round of the body's long edges",
        ),
        param(
            "end_round",
            "0.0675 mm",
            "mm",
            "round of the terminations' outer edges",
        ),
        param(
            "plating",
            "min(0.015 mm, 0.04 * height, termination / 10)",
            "mm",
            "how far the terminations stand proud of the body (est.: tin over nickel over the silver base)",
        ),
        param(
            "marking_top_u",
            "length - 2 * termination - 0.1 mm",
            "mm",
            "marking-top size along its u axis",
        ),
        param(
            "marking_top_v",
            "0.7 * width",
            "mm",
            "marking-top size across it",
        ),
    ];
    if resistor {
        rows.push(param(
            "overcoat",
            "0.03 mm",
            "mm",
            "the protective overcoat's thickness",
        ));
        rows.push(param(
            "overcoat_inset",
            "min(0.04 mm, width / 8)",
            "mm",
            "the white rim left round the overcoat",
        ));
    }
    if led {
        rows.push(param(
            "substrate",
            "0.25 mm",
            "mm",
            "the LED's substrate thickness",
        ));
        rows.push(param(
            "lens_round",
            "0.1 mm",
            "mm",
            "round of the lens's top edges",
        ));
        rows.push(param(
            "die",
            "min(0.25 mm, width / 3, length - 2 * termination - 0.1 mm)",
            "mm",
            "the LED die's side (est.)",
        ));
    }
    import(&mut s, &rows);
    // The body between the caps, `plating` smaller each way (an LED's is
    // its substrate).
    let body_top = match kind {
        Chip::Resistor => "height - plating - overcoat",
        Chip::Led => "substrate",
        _ => "height - plating",
    };
    run(
        &mut s,
        "solid.box",
        json!({"corner": ["- length / 2 + termination", "- width / 2 + plating", "plating"],
            "length": "length - 2 * termination", "width": "width - 2 * plating",
            "height": format!("{body_top} - plating"), "name": "body", "body_name": "body"}),
    );
    let cap_top = if led { "substrate" } else { "height" };
    for (k, x) in [(1, "- length / 2"), (2, "length / 2 - termination")] {
        let name = format!("termination {k}");
        run(
            &mut s,
            "solid.box",
            json!({"corner": [x, "- width / 2", "0 mm"], "length": "termination",
                "width": "width", "height": cap_top, "name": name, "body_name": name}),
        );
    }
    // The rounds, picked at the saved size.
    let p = (0.015f64).min(0.04 * h).min(t / 10.0);
    let top = match kind {
        Chip::Resistor => h - p - 0.03,
        Chip::Led => 0.25,
        _ => h - p,
    };
    let (yb, xm) = (w / 2.0 - p, 0.0);
    run(
        &mut s,
        "solid.fillet",
        json!({"name": "body round", "body": "body", "radius": "body_round",
            "tangent_chain": false,
            "edges": [[xm, -yb, p], [xm, yb, p], [xm, -yb, top], [xm, yb, top]]}),
    );
    let ct = if led { 0.25 } else { h };
    for (k, sign) in [(1, -1.0), (2, 1.0)] {
        let xe = sign * l / 2.0;
        let xb = sign * (l / 2.0 - t / 2.0);
        run(
            &mut s,
            "solid.fillet",
            json!({"name": format!("termination {k} round"), "body": format!("termination {k}"),
                "radius": "end_round", "tangent_chain": false,
                "edges": [[xe, 0.0, 0.0], [xe, 0.0, ct], [xe, -w / 2.0, ct / 2.0],
                    [xe, w / 2.0, ct / 2.0], [xb, -w / 2.0, 0.0], [xb, w / 2.0, 0.0],
                    [xb, -w / 2.0, ct], [xb, w / 2.0, ct]]}),
        );
    }
    let mut overcoat = None;
    if resistor {
        // The overcoat, a little short of the caps.
        run(
            &mut s,
            "solid.box",
            json!({"corner": ["- length / 2 + termination + overcoat_inset",
                    "- width / 2 + overcoat_inset", "height - plating - overcoat"],
                "length": "length - 2 * termination - 2 * overcoat_inset",
                "width": "width - 2 * overcoat_inset", "height": "overcoat",
                "name": "overcoat", "body_name": "overcoat"}),
        );
        overcoat = Some("overcoat");
    }
    if led {
        // The lens over the substrate and the caps, with the die's pocket.
        run(
            &mut s,
            "solid.box",
            json!({"corner": ["- length / 2", "- width / 2", "substrate"], "length": "length",
                "width": "width", "height": "height - substrate", "name": "lens",
                "body_name": "lens"}),
        );
        run(
            &mut s,
            "solid.fillet",
            json!({"name": "lens round", "body": "lens", "radius": "lens_round",
                "edges": [[0.0, -w / 2.0, h], [l / 2.0, 0.0, h], [0.0, w / 2.0, h],
                    [-l / 2.0, 0.0, h]]}),
        );
        let die = json!({"corner": ["- die / 2", "- die / 2", "substrate"], "length": "die",
            "width": "die", "height": "die / 2"});
        let mut pocket = die.clone();
        if let Some(map) = pocket.as_object_mut() {
            map.insert("operation".into(), json!("cut"));
            map.insert("targets".into(), json!(["lens"]));
            map.insert("name".into(), json!("die pocket"));
        }
        run(&mut s, "solid.box", pocket);
        let mut body = die;
        if let Some(map) = body.as_object_mut() {
            map.insert("name".into(), json!("die"));
            map.insert("body_name".into(), json!("die"));
        }
        run(&mut s, "solid.box", body);
    }
    let (body_look, top) = match kind {
        Chip::Resistor => (MaterialId::Alumina96, "height - plating"),
        Chip::Capacitor => (MaterialId::BariumTitanate, "height - plating"),
        Chip::Ferrite => (MaterialId::Ferrite, "height - plating"),
        Chip::Led => (MaterialId::Alumina96, "height"),
    };
    assign_look(&mut s, &["body"], body_look);
    assign_look(&mut s, &["termination 1", "termination 2"], MaterialId::TinMatte);
    if let Some(o) = overcoat {
        assign_look(&mut s, &[o], MaterialId::ResistorOvercoat);
    }
    if led {
        assign_look(&mut s, &["lens"], MaterialId::SiliconeClear);
        assign_look(&mut s, &["die"], MaterialId::Lcp);
    }
    finish_part(s, top)
}

fn author_chip_resistor() -> Vec<u8> {
    author_chip(Chip::Resistor)
}

fn author_chip_capacitor() -> Vec<u8> {
    author_chip(Chip::Capacitor)
}

fn author_chip_ferrite() -> Vec<u8> {
    author_chip(Chip::Ferrite)
}

fn author_chip_led() -> Vec<u8> {
    author_chip(Chip::Led)
}

/// How a `no-lead` part file's package is built.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum NoLead {
    /// A sawn moulded package (MAP-moulded, singulated by sawing): vertical
    /// sides with the copper leadframe bare at the cut, the terminals'
    /// undersides plated with matte tin; `wettable` adds the plated step
    /// of a wettable flank at each terminal's outer end.
    Sawn { wettable: bool },
    /// A land grid array: gold lands under a laminate substrate with a
    /// moulded cap (ADI's ADXL343, CC-14-1).
    Lga,
    /// An open-cavity sensor package: a sawn DFN with gold-plated terminals
    /// and a round opening centred in the top, the sensing film at its
    /// bottom (Sensirion's SHT4x).
    SensorWindow,
}

/// A part file of the `no-lead` generator's bundle: a DFN (terminals on
/// the two long sides, `four` false) or QFN package, with or without an
/// exposed pad (`pad`), built as `kind` says. A moulded body of
/// `body_length` × `body_width` × `body_height` stands on the seating
/// plane; each side's first terminal (`terminal_width` along the side,
/// `terminal_length` in from `pullback` inside its edge, `frame` thick)
/// lies flush with the underside, and a body pattern of `terminals` (long
/// sides) or `terminals_short` (short sides) at `pitch` replays it. A
/// terminal is the leadframe's copper (bare where the saw cut it) on a
/// `plating` thick finish (tin, or gold on an LGA or sensor package); an
/// LGA's lands are gold through. The body's pocket for each terminal is a
/// cut of the terminal's own box, patterned the same way, so terminals and
/// moulding stay disjoint at any count and nothing names the pattern's
/// bodies; the exposed pad (`pad_length` × `pad_width`, centred) is cut out
/// the same way. Sawn sides stay square; the top edges are rounded by
/// `edge_round`. Pin 1 is marked by a laser-marked dot of `mark_radius`,
/// `mark_depth` deep, two radii in from the −x, −y corner (sawn packages
/// have no moulded dimple: TI's RGV and DML outlines show a pin-1 index
/// area, Sensirion's SHT4x a laser dot). Saved at a QFN-16 3 × 3 mm,
/// 0.5 mm (four sides) or a DFN-8 3 × 3 mm, 0.65 mm; the generator gives
/// every value.
pub fn author_no_lead(four: bool, pad: bool, kind: NoLead) -> Vec<u8> {
    let mut s = Session::default();
    let title = match (four, pad, kind) {
        (_, _, NoLead::Lga) => "LGA package".to_owned(),
        (_, _, NoLead::SensorWindow) => "DFN sensor package with top opening".to_owned(),
        (four, pad, NoLead::Sawn { wettable }) => format!(
            "{}{}{}",
            if four { "QFN package" } else { "DFN package" },
            if pad { " with exposed pad" } else { "" },
            if wettable { ", wettable flanks" } else { "" }
        ),
    };
    run(&mut s, "file.new", json!({"name": title}));
    let (pitch, width, pad_size) = if four {
        ("0.5 mm", "0.25 mm", ["1.7 mm", "1.7 mm"])
    } else {
        ("0.65 mm", "0.3 mm", ["2.4 mm", "1.6 mm"])
    };
    let lga = kind == NoLead::Lga;
    let window = kind == NoLead::SensorWindow;
    let wettable = kind == NoLead::Sawn { wettable: true };
    let mut params = vec![
        ("terminals", "4", "", "terminals per long (±y) side"),
        ("pitch", pitch, "mm", "terminal pitch"),
        ("body_length", "3 mm", "mm", "body length (x)"),
        ("body_width", "3 mm", "mm", "body width (y)"),
        (
            "body_height",
            "0.9 mm",
            "mm",
            "body height above the seating plane",
        ),
        (
            "terminal_length",
            "0.4 mm",
            "mm",
            "terminal length in from its outer end",
        ),
        (
            "terminal_width",
            width,
            "mm",
            "terminal width along the edge",
        ),
        (
            "pullback",
            "0 mm",
            "mm",
            "how far the terminals' outer ends lie in from the body edge",
        ),
        ("frame", "0.2 mm", "mm", "leadframe (terminal) thickness"),
        (
            "plating",
            "0.01 mm",
            "mm",
            "finish under the terminals (matte tin 7–20 µm, est.)",
        ),
        (
            "edge_round",
            "0.03 mm",
            "mm",
            "round of the body's top edges",
        ),
        ("mark_radius", "0.2 mm", "mm", "pin-1 laser dot radius"),
        ("mark_depth", "0.005 mm", "mm", "pin-1 laser dot depth"),
        (
            "marking_top_u",
            "0.7 * body_length",
            "mm",
            "marking-top size along its u axis",
        ),
        (
            "marking_top_v",
            "0.5 * body_width",
            "mm",
            "marking-top size across it",
        ),
    ];
    if four {
        params.insert(
            1,
            ("terminals_short", "4", "", "terminals per short (±x) side"),
        );
    }
    if pad {
        params.push(("pad_length", pad_size[0], "mm", "exposed pad length (x)"));
        params.push(("pad_width", pad_size[1], "mm", "exposed pad width (y)"));
    }
    if wettable {
        params.push((
            "flank_height",
            "0.1 mm",
            "mm",
            "the wettable flank's plated step: its height up the terminal's end",
        ));
        params.push((
            "flank_depth",
            "0.05 mm",
            "mm",
            "the plated step's depth in from the terminal's end",
        ));
    }
    if lga {
        params.push((
            "substrate",
            "0.21 mm",
            "mm",
            "laminate substrate under the moulded cap (ADXL343 CC-14-1: 0.95 − 0.74)",
        ));
    }
    if window {
        params.push((
            "window_diameter",
            "0.6 mm",
            "mm",
            "the sensor opening's diameter (SHT4x: 0.6 mm)",
        ));
        params.push((
            "window_depth",
            "0.25 mm",
            "mm",
            "the sensor opening's depth to the sensing film (est.)",
        ));
    }
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
    if lga {
        run(
            &mut s,
            "solid.box",
            json!({"corner": ["- body_length / 2", "- body_width / 2", "0 mm"],
                "length": "body_length", "width": "body_width", "height": "substrate",
                "name": "substrate", "body_name": "substrate"}),
        );
        run(
            &mut s,
            "solid.box",
            json!({"corner": ["- body_length / 2", "- body_width / 2", "substrate"],
                "length": "body_length", "width": "body_width", "height": "body_height - substrate",
                "name": "body", "body_name": "body"}),
        );
    } else {
        run(
            &mut s,
            "solid.box",
            json!({"corner": ["- body_length / 2", "- body_width / 2", "0 mm"],
                "length": "body_length", "width": "body_width", "height": "body_height",
                "name": "body", "body_name": "body"}),
        );
    }
    // The body the terminals' pockets cut: the substrate under an LGA's
    // cap, else the moulding.
    let carrier = if lga { "substrate" } else { "body" };
    // Per row: its name, the corner of its first terminal's box, the box's
    // size, the pattern's direction and count, and the outer end's face
    // as (axis index, which end of the box).
    let first = |count: &str| format!("- ({count} - 1) / 2 * pitch - terminal_width / 2");
    let along_x = ["terminal_width", "terminal_length"];
    let along_y = ["terminal_length", "terminal_width"];
    struct Row {
        name: &'static str,
        corner: [String; 2],
        size: [&'static str; 2],
        dir: [i32; 3],
        count: &'static str,
        /// The axis the outer end faces along (0 x, 1 y) and whether it is
        /// the box's far (+) end.
        outer: (usize, bool),
    }
    let south = Row {
        name: "row_s",
        corner: [first("terminals"), "- body_width / 2 + pullback".into()],
        size: along_x,
        dir: [1, 0, 0],
        count: "terminals",
        outer: (1, false),
    };
    let north = Row {
        name: "row_n",
        corner: [
            first("terminals"),
            "body_width / 2 - pullback - terminal_length".into(),
        ],
        size: along_x,
        dir: [1, 0, 0],
        count: "terminals",
        outer: (1, true),
    };
    let rows_of = if four {
        vec![
            south,
            Row {
                name: "row_e",
                corner: [
                    "body_length / 2 - pullback - terminal_length".into(),
                    first("terminals_short"),
                ],
                size: along_y,
                dir: [0, 1, 0],
                count: "terminals_short",
                outer: (0, true),
            },
            north,
            Row {
                name: "row_w",
                corner: ["- body_length / 2 + pullback".into(), first("terminals_short")],
                size: along_y,
                dir: [0, 1, 0],
                count: "terminals_short",
                outer: (0, false),
            },
        ]
    } else {
        vec![south, north]
    };
    let mut seeds = Vec::new();
    for row in &rows_of {
        let [cx, cy] = &row.corner;
        let [l, w] = row.size;
        let terminal = format!("{} terminal", row.name);
        let plating = format!("{} plating", row.name);
        // The pocket: the terminal's whole box.
        run(
            &mut s,
            "solid.box",
            json!({"corner": [cx, cy, "0 mm"], "length": l, "width": w, "height": "frame",
                "operation": "cut", "targets": [carrier], "name": format!("{} pocket", row.name)}),
        );
        let mut bodies = vec![terminal.clone()];
        if lga {
            run(
                &mut s,
                "solid.box",
                json!({"corner": [cx, cy, "0 mm"], "length": l, "width": w, "height": "frame",
                    "name": terminal, "body_name": terminal}),
            );
        } else {
            // Copper over its finish.
            run(
                &mut s,
                "solid.box",
                json!({"corner": [cx, cy, "plating"], "length": l, "width": w,
                    "height": "frame - plating", "name": terminal, "body_name": terminal}),
            );
            run(
                &mut s,
                "solid.box",
                json!({"corner": [cx, cy, "0 mm"], "length": l, "width": w, "height": "plating",
                    "name": plating, "body_name": plating}),
            );
            bodies.push(plating);
        }
        if wettable {
            // The step at the outer end: cut from the copper and filled
            // with plating, its face a hair inside the cut.
            let (axis, far) = row.outer;
            let step = |inset: &str| -> ([String; 2], [String; 2]) {
                let mut corner = [cx.clone(), cy.clone()];
                let mut size = [l.to_owned(), w.to_owned()];
                let len = if axis == 0 { l } else { w };
                corner[axis] = if far {
                    format!("{} + {len} - flank_depth", row.corner[axis])
                } else {
                    format!("{} + {inset}", row.corner[axis])
                };
                size[axis] = format!("flank_depth - {inset}");
                (corner, size)
            };
            let (corner, size) = step("0 mm");
            run(
                &mut s,
                "solid.box",
                json!({"corner": [corner[0], corner[1], "plating"], "length": size[0],
                    "width": size[1], "height": "flank_height - plating", "operation": "cut",
                    "targets": [terminal], "name": format!("{} flank step", row.name)}),
            );
            let (corner, size) = step("0.005 mm");
            let flank = format!("{} flank", row.name);
            run(
                &mut s,
                "solid.box",
                json!({"corner": [corner[0], corner[1], "plating"], "length": size[0],
                    "width": size[1], "height": "flank_height - plating",
                    "name": flank, "body_name": flank}),
            );
            bodies.push(flank);
        }
        run(
            &mut s,
            "solid.pattern.rectangular",
            json!({"features": [format!("{} pocket", row.name)], "dir1": row.dir,
                "count1": row.count, "spacing1": "pitch", "name": format!("{} pockets", row.name)}),
        );
        run(
            &mut s,
            "solid.pattern.rectangular",
            json!({"bodies": bodies, "dir1": row.dir, "count1": row.count, "spacing1": "pitch",
                "name": format!("{} pattern", row.name)}),
        );
        seeds.push(terminal);
    }
    if pad {
        let corner = json!(["- pad_length / 2", "- pad_width / 2", "0 mm"]);
        run(
            &mut s,
            "solid.box",
            json!({"corner": corner, "length": "pad_length", "width": "pad_width",
                "height": "frame", "name": "exposed pad", "body_name": "exposed pad"}),
        );
        run(
            &mut s,
            "solid.box",
            json!({"corner": corner, "length": "pad_length", "width": "pad_width",
                "height": "frame", "operation": "cut", "targets": [carrier],
                "name": "exposed pad pocket"}),
        );
    }
    // The top edges, picked at the saved size: 3 × 3 × 0.9 mm.
    let (x, y, z) = (1.5, 1.5, 0.9);
    run(
        &mut s,
        "solid.fillet",
        json!({"name": "edge round", "body": "body", "radius": "edge_round",
            "edges": [[0.0, -y, z], [x, 0.0, z], [0.0, y, z], [-x, 0.0, z]]}),
    );
    if window {
        // The opening, and the sensing film at its bottom.
        run(
            &mut s,
            "solid.cylinder",
            json!({"name": "sensor opening", "radius": "window_diameter / 2",
                "height": "window_depth + 0.1 mm",
                "base": ["0 mm", "0 mm", "body_height - window_depth"],
                "axis": "Z", "operation": "cut", "targets": ["body"]}),
        );
        run(
            &mut s,
            "solid.cylinder",
            json!({"name": "sensing film", "radius": "window_diameter / 2",
                "height": "0.01 mm",
                "base": ["0 mm", "0 mm", "body_height - window_depth"],
                "axis": "Z", "body_name": "sensing film"}),
        );
    }
    // The pin-1 laser dot: a shallow disc of laser-marked moulding.
    let dot = ["- body_length / 2 + 2 * mark_radius", "- body_width / 2 + 2 * mark_radius"];
    run(
        &mut s,
        "solid.cylinder",
        json!({"name": "pin 1 mark", "base": [dot[0], dot[1], "body_height - mark_depth"],
            "radius": "mark_radius", "height": "2 * mark_depth", "axis": "Z",
            "operation": "cut", "targets": ["body"]}),
    );
    run(
        &mut s,
        "solid.cylinder",
        json!({"name": "pin 1 dot", "base": [dot[0], dot[1], "body_height - mark_depth"],
            "radius": "mark_radius", "height": "mark_depth", "axis": "Z",
            "body_name": "pin 1 dot"}),
    );
    marking_plane(&mut s, "body_height");
    // Looks are kept by body name, and a pattern names its copies
    // `row_s terminal (2)`, `(3)`, …: every terminal the generator can ask
    // for (64 a side) gets its look at that count, on a body long enough
    // to hold them, then the file goes back to its saved values.
    let counts = |s: &mut Session, n: u32, body: &str| {
        let mut changes = serde_json::Map::new();
        changes.insert("terminals".into(), json!(n.to_string()));
        if four {
            changes.insert("terminals_short".into(), json!(n.to_string()));
        }
        changes.insert("body_length".into(), json!(body));
        changes.insert("body_width".into(), json!(body));
        run(s, "parameters.change", json!({ "changes": changes }));
        s.refresh();
    };
    counts(&mut s, 64, "40 mm");
    let names: Vec<String> = s
        .model
        .state()
        .bodies
        .iter()
        .map(|b| b.name.clone())
        .collect();
    let sides = if four { 4 } else { 2 };
    let of = |what: &str| -> Vec<&str> {
        names
            .iter()
            .map(String::as_str)
            .filter(|n| n.contains(what))
            .collect()
    };
    let copper = of("terminal");
    assert_eq!(copper.len(), sides * 64, "{names:?}");
    assert!(seeds.iter().all(|seed| copper.contains(&seed.as_str())));
    let finish = if lga || window {
        MaterialId::Gold
    } else {
        MaterialId::TinMatte
    };
    assign_look(&mut s, &["body"], MaterialId::EpoxyMould);
    if lga {
        assign_look(&mut s, &["substrate"], MaterialId::SolderMaskGreen);
        assign_look(&mut s, &copper, MaterialId::Gold);
    } else {
        assign_look(&mut s, &copper, MaterialId::CopperAlloy);
        let plating = of("plating");
        assert_eq!(plating.len(), sides * 64, "{names:?}");
        assign_look(&mut s, &plating, finish);
    }
    if wettable {
        let flanks = of("flank");
        assert_eq!(flanks.len(), sides * 64, "{names:?}");
        assign_look(&mut s, &flanks, MaterialId::TinMatte);
    }
    if pad {
        assign_look(&mut s, &["exposed pad"], finish);
    }
    if window {
        assign_look(&mut s, &["sensing film"], MaterialId::Polyimide);
    }
    assign_look(&mut s, &["pin 1 dot"], MaterialId::EpoxyLaserMarked);
    counts(&mut s, 4, "3 mm");
    let errors: Vec<String> = s
        .model
        .results
        .iter()
        .filter_map(|r| r.error.clone().map(|e| format!("{}: {e}", r.name)))
        .collect();
    assert!(errors.is_empty(), "{errors:?}");
    solidcraft_engine::io::write_design(&s.doc)
}

fn author_dfn() -> Vec<u8> {
    author_no_lead(false, false, NoLead::Sawn { wettable: false })
}

fn author_dfn_ep() -> Vec<u8> {
    author_no_lead(false, true, NoLead::Sawn { wettable: false })
}

fn author_dfn_ep_wettable() -> Vec<u8> {
    author_no_lead(false, true, NoLead::Sawn { wettable: true })
}

fn author_dfn_window() -> Vec<u8> {
    author_no_lead(false, false, NoLead::SensorWindow)
}

fn author_qfn() -> Vec<u8> {
    author_no_lead(true, false, NoLead::Sawn { wettable: false })
}

fn author_qfn_ep() -> Vec<u8> {
    author_no_lead(true, true, NoLead::Sawn { wettable: false })
}

fn author_qfn_ep_wettable() -> Vec<u8> {
    author_no_lead(true, true, NoLead::Sawn { wettable: true })
}

fn author_lga() -> Vec<u8> {
    author_no_lead(true, false, NoLead::Lga)
}

/// Rectangular cut-outs of a depopulated grid's part file (the grid
/// generator's `MAX_VOIDS`).
const GRID_VOIDS: usize = 16;

/// A part file of the `grid` generator's bundle: a grid array on solder
/// balls (`balls`, BGA and WLCSP) or on lands (LGA), full or with
/// rectangular cut-outs (`voids`), the in-code grid builder's construction
/// made parametric in its counts. The contacts are one seed at A1 (−x, +y)
/// of `contact_diameter` and `contact_height` (a land's thickness) and a
/// body pattern of `columns` along +x at `pitch` and `rows` along −y at
/// `row_pitch`, so the copies are `contact 1 (2)`, `(3)`, … at any count.
/// Each cut-out `void_<k>` (`_x`, `_y` its −x, −y corner, `_length`,
/// `_width`) removes the contacts inside it whole, before the body exists;
/// cut-outs a grid does not need repeat the first. The body (substrate and
/// moulding as one block) of `body_length` × `body_width` × `body_height`
/// stands on the balls, or on the seating plane over the lands, cut round
/// each land by a pattern of one cut with the contacts' counts (nothing
/// names the pattern's bodies) and filled again over every cut-out. Its
/// top edges are rounded by `edge_round`, and a dimple of `mark_radius`,
/// `mark_depth` deep, two radii in from the −x, +y corner marks A1. Saved
/// at a 4 × 5 grid of 0.8 mm on a 5 × 4 mm body (with cut-outs: rows B–C,
/// columns 2–4 empty); the generator gives every value.
pub fn author_grid(balls: bool, voids: bool) -> Vec<u8> {
    let mut s = Session::default();
    let title = match (balls, voids) {
        (true, false) => "Ball grid array",
        (true, true) => "Ball grid array with empty positions",
        (false, false) => "Land grid array",
        (false, true) => "Land grid array with empty positions",
    };
    run(&mut s, "file.new", json!({"name": title}));
    let height = if balls { "0.3 mm" } else { "0.03 mm" };
    let mut rows = vec![
        param("rows", "4", "", "rows of contacts (y), A from +y"),
        param("columns", "5", "", "columns of contacts (x), 1 from −x"),
        param("pitch", "0.8 mm", "mm", "pitch along a row (x)"),
        param("row_pitch", "0.8 mm", "mm", "pitch between rows (y)"),
        param("body_length", "5 mm", "mm", "body length (x)"),
        param("body_width", "4 mm", "mm", "body width (y)"),
        param(
            "body_height",
            "0.8 mm",
            "mm",
            "body height above the contacts",
        ),
        param("contact_diameter", "0.4 mm", "mm", "ball or land diameter"),
        param(
            "contact_height",
            height,
            "mm",
            if balls {
                "ball height (standoff)"
            } else {
                "land thickness, set into the body"
            },
        ),
        param(
            "edge_round",
            "0.05 mm",
            "mm",
            "round of the body's top edges",
        ),
        param("mark_radius", "0.24 mm", "mm", "A1 dimple radius"),
        param("mark_depth", "0.03 mm", "mm", "A1 dimple depth"),
    ];
    // The saved cut-out: rows B–C, columns 2–4 (half a pitch outside the
    // contacts there).
    let saved_void = ["-1.2 mm", "-0.8 mm", "2.4 mm", "1.6 mm"];
    if voids {
        for k in 1..=GRID_VOIDS {
            for (axis, value, what) in [
                ("x", saved_void[0], "−x edge"),
                ("y", saved_void[1], "−y edge"),
                ("length", saved_void[2], "length (x)"),
                ("width", saved_void[3], "width (y)"),
            ] {
                rows.push(param(
                    &format!("void_{k}_{axis}"),
                    value,
                    "mm",
                    &format!("cut-out {k}: {what}"),
                ));
            }
        }
    }
    import(&mut s, &rows);
    let seed = json!([
        "- (columns - 1) / 2 * pitch",
        "(rows - 1) / 2 * row_pitch",
        "0 mm"
    ]);
    run(
        &mut s,
        "solid.cylinder",
        json!({"base": seed, "diameter": "contact_diameter", "height": "contact_height",
            "axis": "Z", "name": "contact 1", "body_name": "contact 1"}),
    );
    let pattern = |what: serde_json::Value, name: &str| {
        let mut p = json!({"dir1": [1, 0, 0], "count1": "columns", "spacing1": "pitch",
            "dir2": [0, -1, 0], "count2": "rows", "spacing2": "row_pitch", "name": name});
        if let (Some(map), Some(extra)) = (p.as_object_mut(), what.as_object()) {
            map.extend(extra.clone());
        }
        p
    };
    run(
        &mut s,
        "solid.pattern.rectangular",
        pattern(json!({"bodies": ["contact 1"]}), "contact 1 pattern"),
    );
    let void = |k: usize, axis: &str| format!("void_{k}_{axis}");
    if voids {
        // Only contacts exist yet, so each cut-out (no targets) removes
        // the contacts inside it and nothing else.
        for k in 1..=GRID_VOIDS {
            run(
                &mut s,
                "solid.box",
                json!({"corner": [void(k, "x"), void(k, "y"), "- contact_height"],
                    "length": void(k, "length"), "width": void(k, "width"),
                    "height": "3 * contact_height", "operation": "cut",
                    "name": format!("void {k}")}),
            );
        }
    }
    let base = if balls { "contact_height" } else { "0 mm" };
    run(
        &mut s,
        "solid.box",
        json!({"corner": ["- body_length / 2", "- body_width / 2", base],
            "length": "body_length", "width": "body_width", "height": "body_height",
            "name": "body", "body_name": "body"}),
    );
    if !balls {
        run(
            &mut s,
            "solid.cylinder",
            json!({"base": seed, "diameter": "contact_diameter", "height": "contact_height",
                "axis": "Z", "operation": "cut", "targets": ["body"],
                "name": "contact 1 pocket"}),
        );
        run(
            &mut s,
            "solid.pattern.rectangular",
            pattern(json!({"features": ["contact 1 pocket"]}), "pockets"),
        );
        if voids {
            for k in 1..=GRID_VOIDS {
                run(
                    &mut s,
                    "solid.box",
                    json!({"corner": [void(k, "x"), void(k, "y"), "0 mm"],
                        "length": void(k, "length"), "width": void(k, "width"),
                        "height": "contact_height", "operation": "join", "targets": ["body"],
                        "name": format!("void {k} fill")}),
                );
            }
        }
    }
    // The top edges, picked at the saved size: 5 × 4 mm, its top at
    // 1.1 mm on balls, 0.8 mm over lands.
    let (x, y, z) = (2.5, 2.0, if balls { 1.1 } else { 0.8 });
    run(
        &mut s,
        "solid.fillet",
        json!({"name": "edge round", "body": "body", "radius": "edge_round",
            "edges": [[0.0, -y, z], [x, 0.0, z], [0.0, y, z], [-x, 0.0, z]]}),
    );
    run(
        &mut s,
        "solid.cylinder",
        json!({"name": "pin 1 mark",
            "base": ["- body_length / 2 + 2 * mark_radius", "body_width / 2 - 2 * mark_radius",
                format!("{base} + body_height - mark_depth")],
            "radius": "mark_radius", "height": "2 * mark_depth", "axis": "Z",
            "operation": "cut", "targets": ["body"]}),
    );
    // Looks are kept by body name, and the pattern names its copies
    // `contact 1 (2)`, `(3)`, …: every contact the generator can ask for
    // (512 positions full, 1000 with cut-outs) gets its look at that
    // count, on a body large enough and with the cut-outs moved off the
    // grid, then the file goes back to its saved values.
    let (most_rows, most_columns, most) = if voids { (20, 50, 1000) } else { (8, 64, 512) };
    let mut changes = serde_json::Map::new();
    changes.insert("rows".into(), json!(most_rows.to_string()));
    changes.insert("columns".into(), json!(most_columns.to_string()));
    changes.insert("body_length".into(), json!("60 mm"));
    changes.insert("body_width".into(), json!("20 mm"));
    if voids {
        for k in 1..=GRID_VOIDS {
            changes.insert(void(k, "x"), json!("100 mm"));
            changes.insert(void(k, "y"), json!("100 mm"));
        }
    }
    run(&mut s, "parameters.change", json!({ "changes": changes }));
    s.refresh();
    let contacts: Vec<String> = s
        .model
        .state()
        .bodies
        .iter()
        .map(|b| b.name.clone())
        .filter(|n| n != "body")
        .collect();
    assert_eq!(contacts.len(), most, "{contacts:?}");
    let contacts: Vec<&str> = contacts.iter().map(String::as_str).collect();
    assign_look(&mut s, &["body"], MaterialId::EpoxyMould);
    assign_look(
        &mut s,
        &contacts,
        if balls {
            MaterialId::SolderSac
        } else {
            MaterialId::Gold
        },
    );
    let mut changes = serde_json::Map::new();
    changes.insert("rows".into(), json!("4"));
    changes.insert("columns".into(), json!("5"));
    changes.insert("body_length".into(), json!("5 mm"));
    changes.insert("body_width".into(), json!("4 mm"));
    if voids {
        for k in 1..=GRID_VOIDS {
            changes.insert(void(k, "x"), json!(saved_void[0]));
            changes.insert(void(k, "y"), json!(saved_void[1]));
        }
    }
    run(&mut s, "parameters.change", json!({ "changes": changes }));
    s.refresh();
    let errors: Vec<String> = s
        .model
        .results
        .iter()
        .filter_map(|r| r.error.clone().map(|e| format!("{}: {e}", r.name)))
        .collect();
    assert!(errors.is_empty(), "{errors:?}");
    let left = s.model.state().bodies.len();
    assert_eq!(
        left,
        if voids { 1 + 20 - 6 } else { 1 + 20 },
        "bodies saved"
    );
    solidcraft_engine::io::write_design(&s.doc)
}

fn author_grid_ball() -> Vec<u8> {
    author_grid(true, false)
}

fn author_grid_ball_voids() -> Vec<u8> {
    author_grid(true, true)
}

fn author_grid_land() -> Vec<u8> {
    author_grid(false, false)
}

fn author_grid_land_voids() -> Vec<u8> {
    author_grid(false, true)
}

/// The +y side of a `tabbed` generator part file.
#[derive(Clone, Copy, PartialEq, Eq)]
enum TabbedSide {
    /// A second row of leads (SOT-23, SC-70).
    Rows,
    /// A wide gull-wing tab lead (SOT-223, SOT-89).
    TabLead,
    /// A flat flange on the seating plane under the body (DPAK, D²PAK).
    Flange,
}

/// The parameters of a bent gull-wing lead profile whose foot is `foot`
/// long, prefixed `p`: where the foot starts (`p_foot_in`), the shoulder,
/// the slope's run and length, tan of half the bend angle (`p_k`), the two
/// bends' centreline radii (`p_r1` at the shoulder, `p_r2` at the foot:
/// the lead thickness, less where 45 % of a segment cannot hold it, never
/// below half the thickness) and the miter of the sharp outline's corners.
fn lead_profile_params(p: &str, foot: &str) -> Vec<(String, String, &'static str, String)> {
    let at_least = "lead_thickness / 2 * 1.0001";
    vec![
        (
            format!("{p}_foot_in"),
            format!("span / 2 - {foot}"),
            "mm",
            "where the foot starts, across y".into(),
        ),
        (
            format!("{p}_shoulder"),
            format!("half_body + 0.3 * ({p}_foot_in - half_body)"),
            "mm",
            "where the lead bends down, across y".into(),
        ),
        (
            format!("{p}_dx"),
            format!("{p}_foot_in - {p}_shoulder"),
            "mm",
            "the slope's run across y".into(),
        ),
        (
            format!("{p}_slope"),
            format!("hypot({p}_dx, lead_drop)"),
            "mm",
            "the slope's length".into(),
        ),
        (
            format!("{p}_k"),
            format!("lead_drop / ({p}_slope + {p}_dx)"),
            "",
            "tan of half the bend angle".into(),
        ),
        (
            format!("{p}_r1"),
            format!(
                "max(min(lead_thickness, 0.45 * min({p}_shoulder - (half_body - lead_root), {p}_slope) / {p}_k), {at_least})"
            ),
            "mm",
            "the shoulder bend's centreline radius".into(),
        ),
        (
            format!("{p}_r2"),
            format!("max(min(lead_thickness, 0.45 * min({p}_slope, {foot}) / {p}_k), {at_least})"),
            "mm",
            "the foot bend's centreline radius".into(),
        ),
        (
            format!("{p}_miter"),
            format!("lead_thickness / 2 * {p}_k"),
            "mm",
            "how far the outline's sharp corners lie past the centreline's".into(),
        ),
    ]
}

/// A bent lead `name` in the YZ plane at x = `offset`, `width` wide along
/// +x, on the +y side (`plus`) or mirrored onto −y, from the profile
/// parameters `p`: the outline of the centreline (root inside the body at
/// the exit height, out to the shoulder, down to the foot at half the
/// thickness, out to the tip at half the span) thickened to the lead
/// thickness, its four corners rounded concentric with the bends. The
/// lead's root is cut out of the body by the same profile (`<cut>`), so
/// lead and moulding stay disjoint.
fn bent_lead(
    s: &mut Session,
    name: &str,
    cut: &str,
    p: &str,
    plus: bool,
    offset: &str,
    width: &str,
) {
    let sketch = format!("{name} sketch");
    run(
        s,
        "sketch.create",
        json!({"plane": "YZ", "offset": offset, "name": sketch}),
    );
    let u = |e: String| {
        if plus { e } else { format!("-({e})") }
    };
    let top = "lead_exit + lead_thickness / 2";
    let bottom = "lead_exit - lead_thickness / 2";
    let points = vec![
        [u("half_body - lead_root".into()), top.into()],
        [u(format!("{p}_shoulder + {p}_miter")), top.into()],
        [
            u(format!("{p}_foot_in + {p}_miter")),
            "lead_thickness".into(),
        ],
        [u("span / 2".into()), "lead_thickness".into()],
        [u("span / 2".into()), "0 mm".into()],
        [u(format!("{p}_foot_in - {p}_miter")), "0 mm".into()],
        [u(format!("{p}_shoulder - {p}_miter")), bottom.into()],
        [u("half_body - lead_root".into()), bottom.into()],
    ];
    let reply = run(s, "sketch.line", json!({"points": points, "closed": true}));
    let lines: Vec<String> = reply["curves"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c.as_str().unwrap().to_owned())
        .collect();
    assert_eq!(lines.len(), 8, "{reply}");
    // The shoulder bend: outer on the top face, inner below; the foot bend
    // the other way round.
    let h = "lead_thickness / 2";
    for (a, b, radius) in [
        (0, 1, format!("{p}_r1 + {h}")),
        (1, 2, format!("{p}_r2 - {h}")),
        (4, 5, format!("{p}_r2 + {h}")),
        (5, 6, format!("{p}_r1 - {h}")),
    ] {
        run(
            s,
            "sketch.fillet",
            json!({"a": lines[a], "b": lines[b], "radius": radius}),
        );
    }
    run(s, "sketch.finish", json!({}));
    run(
        s,
        "solid.extrude",
        json!({"sketch": sketch, "distance": width, "operation": "new",
            "name": name, "body_name": name}),
    );
    run(
        s,
        "solid.extrude",
        json!({"sketch": sketch, "distance": width, "operation": "cut",
            "targets": ["body"], "name": cut}),
    );
}

/// A part file of the `tabbed` generator's bundle: a moulded body on its
/// standoff with gull-wing leads on −y (`row_s`) and, by `side`, a second
/// row on +y (`row_n`), a gull-wing tab lead or a flat flange under the
/// body (`tab`), as the in-code tabbed builder made them. Each row is one
/// bent lead at the row's −x end and a body pattern of `row_<s|n>_leads`
/// at `row_<s|n>_step` × `pitch`; the body is cut round each lead by a cut
/// of the lead's own profile, patterned the same way, so leads and
/// moulding stay disjoint at any count and nothing names the pattern's
/// bodies. Rounds on the moulding's upright corners (above the leads) and
/// top edges. The lead profile follows its parameters through the derived
/// parameters of `lead_profile_params`. Saved at the SOT-23-5, SOT-223 and
/// DPAK starter values; the generator gives every input value.
fn author_tabbed(side: TabbedSide) -> Vec<u8> {
    let (title, values): (&str, [&str; 9]) = match side {
        // pitch, length, width, height, standoff, span, lead width,
        // thickness, foot.
        TabbedSide::Rows => (
            "SOT-23 and SC-70 package",
            [
                "0.95 mm", "2.9 mm", "1.6 mm", "1.1 mm", "0.075 mm", "2.8 mm", "0.4 mm", "0.15 mm",
                "0.4 mm",
            ],
        ),
        TabbedSide::TabLead => (
            "SOT-223 and SOT-89 package",
            [
                "2.3 mm", "6.5 mm", "3.5 mm", "1.6 mm", "0.06 mm", "7 mm", "0.7 mm", "0.3 mm",
                "0.9 mm",
            ],
        ),
        TabbedSide::Flange => (
            "DPAK and D2PAK package",
            [
                "2.285 mm", "6.6 mm", "6.1 mm", "2.3 mm", "0 mm", "11.7 mm", "0.76 mm", "0.5 mm",
                "1.5 mm",
            ],
        ),
    };
    let [pitch, bl, bw, bh, standoff, span, lw, lt, foot] = values;
    let mut params: Vec<(String, String, &'static str, String)> = [
        ("pitch", pitch, "mm", "the lead grid's pitch along x"),
        ("body_length", bl, "mm", "moulded body length (x)"),
        ("body_width", bw, "mm", "moulded body width (y)"),
        (
            "body_height",
            bh,
            "mm",
            "moulded body height above the standoff",
        ),
        (
            "standoff",
            standoff,
            "mm",
            "body bottom above the seating plane",
        ),
        ("span", span, "mm", "lead tip to lead tip across y"),
        ("lead_width", lw, "mm", "lead width along x"),
        ("lead_thickness", lt, "mm", "lead thickness"),
        (
            "foot_length",
            foot,
            "mm",
            "a lead's foot on the seating plane",
        ),
    ]
    .into_iter()
    .map(|(n, e, u, c)| (n.into(), e.into(), u, c.into()))
    .collect();
    let mut count = |name: &str, v: &str, comment: &str| {
        params.push((name.into(), v.into(), "", comment.into()));
    };
    let (s_leads, s_step) = match side {
        TabbedSide::Rows => ("3", "1"),
        TabbedSide::TabLead => ("3", "1"),
        TabbedSide::Flange => ("2", "2"),
    };
    count("row_s_leads", s_leads, "leads on the −y side");
    count("row_s_step", s_step, "grid positions between −y leads");
    if side == TabbedSide::Rows {
        count("row_n_leads", "2", "leads on the +y side");
        count("row_n_step", "2", "grid positions between +y leads");
    }
    let mut length = |name: &str, v: &str, comment: &str| {
        params.push((name.into(), v.into(), "mm", comment.into()));
    };
    match side {
        TabbedSide::Rows => {}
        TabbedSide::TabLead => {
            length("tab_width", "3 mm", "tab lead width along x");
            length("tab_length", "0.9 mm", "tab lead foot on the seating plane");
        }
        TabbedSide::Flange => {
            length("tab_width", "5.3 mm", "flange width along x");
            length("tab_length", "5.3 mm", "flange solderable length along y");
            length(
                "flange_protrusion",
                "1 mm",
                "flange outer edge beyond the body's +y edge",
            );
        }
    }
    // Derived: the lead's exit and profile.
    length(
        "lead_exit",
        "standoff + body_height / 2",
        "height of the lead centreline where it leaves the body",
    );
    params.push((
        "draft".into(),
        "8 deg".into(),
        "deg",
        "mould draft of the walls above and below the parting line (Microchip SOT-23 and SOIC drawings: 5–15°)".into(),
    ));
    let mut length = |name: &str, v: &str, comment: &str| {
        params.push((name.into(), v.into(), "mm", comment.into()));
    };
    length(
        "draft_inset",
        "(standoff + body_height - lead_exit) * tan(draft)",
        "how far the top face's edge lies in from the parting line's",
    );
    length(
        "corner_round",
        "max(min(0.2 mm, 0.1 * body_width), draft_inset + 0.04 mm)",
        "round of the body's upright corners at the parting line",
    );
    length(
        "edge_round",
        "min(0.1 mm, 0.08 * body_height, (standoff + body_height - lead_exit - lead_thickness / 2) / 2, 0.6 * (corner_round - draft_inset))",
        "round of the body's top edges, clear of the lead exit and the top's corner arcs",
    );
    length("half_body", "body_width / 2", "half the body width");
    length(
        "lead_root",
        "min(0.25 mm, half_body / 3)",
        "how far a lead reaches into the body",
    );
    length(
        "lead_drop",
        "lead_exit - lead_thickness / 2",
        "the slope's drop from the exit to the foot",
    );
    // The marking area: 80 % of the top face each way, clear of its
    // rounded edges (the tabbed form's marking area).
    length(
        "marking_top_u",
        "0.8 * (body_length - 2 * draft_inset)",
        "marking-top size along its u axis",
    );
    length(
        "marking_top_v",
        "0.7 * (body_width - 2 * draft_inset)",
        "marking-top size across it",
    );
    params.extend(lead_profile_params("lead", "foot_length"));
    if side == TabbedSide::TabLead {
        params.extend(lead_profile_params("tab_lead", "tab_length"));
    }
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
    // The moulding, its top round picked at the saved size.
    let mm = |e: &str| -> f64 { e.trim_end_matches(" mm").parse().unwrap() };
    let inset = mm(bh) / 2.0 * 8f64.to_radians().tan();
    moulded_body(
        &mut s,
        "body",
        ["body_length", "body_width"],
        ["standoff", "lead_exit", "standoff + body_height"],
        "corner_round",
        "draft",
        "edge_round",
        [0.0, -(mm(bw) / 2.0 - inset), mm(standoff) + mm(bh)],
    );
    let rows: &[(&str, bool)] = match side {
        TabbedSide::Rows => &[("row_s", false), ("row_n", true)],
        _ => &[("row_s", false)],
    };
    for (row, plus) in rows {
        bent_lead(
            &mut s,
            &format!("{row} lead"),
            &format!("{row} root"),
            "lead",
            *plus,
            &format!("- ({row}_leads - 1) / 2 * {row}_step * pitch - lead_width / 2"),
            "lead_width",
        );
    }
    match side {
        TabbedSide::Rows => {}
        TabbedSide::TabLead => bent_lead(
            &mut s,
            "tab",
            "tab root",
            "tab_lead",
            true,
            "- tab_width / 2",
            "tab_width",
        ),
        TabbedSide::Flange => {
            let corner = json!([
                "- tab_width / 2",
                "body_width / 2 + flange_protrusion - tab_length",
                "0 mm"
            ]);
            run(
                &mut s,
                "solid.box",
                json!({"corner": corner, "length": "tab_width", "width": "tab_length",
                    "height": "lead_thickness", "name": "tab", "body_name": "tab"}),
            );
            run(
                &mut s,
                "solid.box",
                json!({"corner": corner, "length": "tab_width", "width": "tab_length",
                    "height": "lead_thickness", "operation": "cut", "targets": ["body"],
                    "name": "tab root"}),
            );
        }
    }
    for (row, _) in rows {
        let pattern = |what: serde_json::Value, name: String| {
            let mut p = json!({"dir1": [1, 0, 0], "count1": format!("{row}_leads"),
                "spacing1": format!("{row}_step * pitch"), "name": name});
            if let (Some(map), Some(extra)) = (p.as_object_mut(), what.as_object()) {
                map.extend(extra.clone());
            }
            p
        };
        run(
            &mut s,
            "solid.pattern.rectangular",
            pattern(
                json!({"features": [format!("{row} root")]}),
                format!("{row} roots"),
            ),
        );
        run(
            &mut s,
            "solid.pattern.rectangular",
            pattern(
                json!({"bodies": [format!("{row} lead")]}),
                format!("{row} pattern"),
            ),
        );
    }
    // Looks are kept by body name, and a pattern names its copies
    // `row_s lead (2)`, …: every lead the generator can ask for (16 a side)
    // gets the tin look at that count, then the file goes back to its own.
    // The body is made long enough for 16 leads at one grid step.
    let mut changes = serde_json::Map::new();
    for (row, _) in rows {
        changes.insert(format!("{row}_leads"), json!("16"));
        changes.insert(format!("{row}_step"), json!("1"));
    }
    changes.insert("body_length".into(), json!("60 mm"));
    run(&mut s, "parameters.change", json!({"changes": changes}));
    s.refresh();
    let leads: Vec<String> = s
        .model
        .state()
        .bodies
        .iter()
        .map(|b| b.name.clone())
        .filter(|n| n != "body")
        .collect();
    assert_eq!(
        leads.len(),
        16 * rows.len() + usize::from(side != TabbedSide::Rows)
    );
    let leads: Vec<&str> = leads.iter().map(String::as_str).collect();
    assign_look(&mut s, &["body"], MaterialId::EpoxyMould);
    assign_look(&mut s, &leads, MaterialId::TinMatte);
    // Back to the saved counts and steps.
    let mut changes = serde_json::Map::new();
    changes.insert("row_s_leads".into(), json!(s_leads));
    changes.insert("row_s_step".into(), json!(s_step));
    if side == TabbedSide::Rows {
        changes.insert("row_n_leads".into(), json!("2"));
        changes.insert("row_n_step".into(), json!("2"));
    }
    changes.insert("body_length".into(), json!(bl));
    run(&mut s, "parameters.change", json!({"changes": changes}));
    s.refresh();
    finish_part(s, "standoff + body_height")
}

/// A part file of the `tabbed` generator's bundle: a PLCC LED with its
/// driver (Worldsemi's WS2812B, 5050), as the datasheet's drawing shows it:
/// a white housing on the leads' feet, a round cavity in its top filled
/// with clear encapsulant over the leadframe, the driver die and three LED
/// chips, the pin-3 corner cut; each lead leaves the housing's side,
/// runs down it and folds under the housing to its foot. The −y and +y
/// rows are patterns with their own counts and grid steps, as the other
/// tabbed files. Saved at the WS2812B's values; the generator gives every
/// input value.
fn author_plcc_led() -> Vec<u8> {
    let mut s = Session::default();
    run(&mut s, "file.new", json!({"name": "PLCC LED package"}));
    let rows = vec![
        param("pitch", "3.3 mm", "mm", "the lead grid's pitch along x"),
        param("body_length", "5 mm", "mm", "housing length (x)"),
        param("body_width", "5 mm", "mm", "housing width (y)"),
        param(
            "body_height",
            "1.42 mm",
            "mm",
            "housing height above the standoff",
        ),
        param(
            "standoff",
            "0.15 mm",
            "mm",
            "housing bottom above the seating plane: the feet under it",
        ),
        param("span", "5.4 mm", "mm", "lead tip to lead tip across y"),
        param("lead_width", "0.9 mm", "mm", "lead width along x"),
        param("lead_thickness", "0.15 mm", "mm", "lead thickness"),
        param(
            "foot_length",
            "1 mm",
            "mm",
            "a lead's foot under the housing, from its outer face",
        ),
        param("row_s_leads", "2", "", "leads on the −y side"),
        param("row_s_step", "1", "", "grid positions between −y leads"),
        param("row_n_leads", "2", "", "leads on the +y side"),
        param("row_n_step", "1", "", "grid positions between +y leads"),
        param(
            "corner_round",
            "0.1 mm",
            "mm",
            "round of the housing's upright corners",
        ),
        param(
            "edge_round",
            "0.05 mm",
            "mm",
            "round of the housing's top edges",
        ),
        param(
            "lead_exit",
            "standoff + min(0.5 mm, body_height / 3)",
            "mm",
            "height of the lead's centreline where it leaves the housing (est.)",
        ),
        param(
            "lens_diameter",
            "0.78 * min(body_length, body_width)",
            "mm",
            "the cavity's diameter at the top (WS2812B drawing: about 3.9 of 5 mm)",
        ),
        param(
            "lens_depth",
            "0.45 * body_height",
            "mm",
            "the cavity's depth to the leadframe (est.)",
        ),
        param(
            "notch",
            "0.4 mm",
            "mm",
            "the cut corner over pin 3 (WS2812B drawing)",
        ),
        param(
            "marking_top_u",
            "0.5 * body_length",
            "mm",
            "marking-top size along its u axis",
        ),
        param(
            "marking_top_v",
            "0.1 * body_width",
            "mm",
            "marking-top size across it",
        ),
    ];
    import(&mut s, &rows);
    run(
        &mut s,
        "solid.box",
        json!({"corner": ["- body_length / 2", "- body_width / 2", "standoff"],
            "length": "body_length", "width": "body_width", "height": "body_height",
            "name": "housing", "body_name": "body"}),
    );
    let (l, w, h0, h) = (5.0, 5.0, 0.15, 1.42);
    let top = h0 + h;
    run(
        &mut s,
        "solid.chamfer",
        json!({"name": "pin 3 corner", "body": "body", "distance": "notch",
            "edges": [[l / 2.0, w / 2.0, h0 + h / 2.0]]}),
    );
    run(
        &mut s,
        "solid.fillet",
        json!({"name": "corner round", "body": "body", "radius": "corner_round",
            "tangent_chain": false,
            "edges": [[-l / 2.0, -w / 2.0, h0 + h / 2.0], [l / 2.0, -w / 2.0, h0 + h / 2.0],
                [-l / 2.0, w / 2.0, h0 + h / 2.0]]}),
    );
    // The cavity and what lies in it: the leadframe, the driver die and
    // three LED chips, under the encapsulant.
    let floor = "standoff + body_height - lens_depth";
    // Round outlines are sketched circles, extruded, so the bodies' bounds
    // are their outlines.
    let circle = |s: &mut Session, name: &str| {
        run(
            s,
            "sketch.create",
            json!({"plane": "XY", "offset": floor, "name": name}),
        );
        run(
            s,
            "sketch.draw.dimensioned",
            json!({"command": "sketch.circle.center",
                "params": {"center": ["0 mm", "0 mm"], "radius": 0.5},
                "dimensions": {"diameter": "lens_diameter"}}),
        );
        run(s, "sketch.finish", json!({}));
    };
    circle(&mut s, "cavity outline");
    run(
        &mut s,
        "solid.extrude",
        json!({"sketch": "cavity outline", "distance": "lens_depth + 0.1 mm",
            "operation": "cut", "targets": ["body"], "name": "cavity"}),
    );
    run(
        &mut s,
        "solid.box",
        json!({"corner": ["- 0.3 * lens_diameter", "- 0.2 * lens_diameter", floor],
            "length": "0.6 * lens_diameter", "width": "0.4 * lens_diameter",
            "height": "0.02 mm", "name": "leadframe", "body_name": "leadframe"}),
    );
    let chips = [
        ("driver die", "-0.12 * lens_diameter", "0 mm", "0.18 * lens_diameter", "0.15 mm"),
        ("red chip", "0.1 * lens_diameter", "-0.1 * lens_diameter", "0.07 * lens_diameter", "0.12 mm"),
        ("green chip", "0.1 * lens_diameter", "0 mm", "0.07 * lens_diameter", "0.12 mm"),
        ("blue chip", "0.1 * lens_diameter", "0.1 * lens_diameter", "0.07 * lens_diameter", "0.12 mm"),
    ];
    for (name, x, y, side, height) in chips {
        run(
            &mut s,
            "solid.box",
            json!({"corner": [format!("{x} - {side} / 2"), format!("{y} - {side} / 2"),
                    format!("{floor} + 0.02 mm")],
                "length": side, "width": side, "height": height, "name": name,
                "body_name": name}),
        );
    }
    circle(&mut s, "encapsulant outline");
    run(
        &mut s,
        "solid.extrude",
        json!({"sketch": "encapsulant outline", "distance": "lens_depth",
            "operation": "new", "name": "encapsulant", "body_name": "lens"}),
    );
    run(
        &mut s,
        "solid.combine",
        json!({"target": "lens", "tools": ["leadframe", "driver die", "red chip", "green chip",
                "blue chip"], "operation": "cut", "keep_tools": true, "name": "lens around the dies"}),
    );
    // Each row's first lead: out of the housing's side, down it and under
    // it, as three joined boxes; then the row's pattern.
    for (row, plus) in [("row_s", false), ("row_n", true)] {
        let lead = format!("{row} lead");
        let x = format!("- ({row}_leads - 1) / 2 * {row}_step * pitch - lead_width / 2");
        let y = |from: &str, to: &str| -> (String, String) {
            if plus {
                (from.to_owned(), format!("({to}) - ({from})"))
            } else {
                (format!("- ({to})"), format!("({to}) - ({from})"))
            }
        };
        let (y0, dy) = y("body_width / 2", "span / 2 - lead_thickness");
        run(
            &mut s,
            "solid.box",
            json!({"corner": [x, y0, "lead_exit - lead_thickness / 2"], "length": "lead_width",
                "width": dy, "height": "lead_thickness", "name": format!("{row} lead stub"),
                "body_name": lead}),
        );
        let (y0, dy) = y("span / 2 - lead_thickness", "span / 2");
        run(
            &mut s,
            "solid.box",
            json!({"corner": [x, y0, "0 mm"], "length": "lead_width", "width": dy,
                "height": "lead_exit + lead_thickness / 2", "operation": "join",
                "targets": [lead], "name": format!("{row} lead side")}),
        );
        let (y0, dy) = y("span / 2 - foot_length", "span / 2 - lead_thickness");
        run(
            &mut s,
            "solid.box",
            json!({"corner": [x, y0, "0 mm"], "length": "lead_width", "width": dy,
                "height": "lead_thickness", "operation": "join", "targets": [lead],
                "name": format!("{row} lead foot")}),
        );
        run(
            &mut s,
            "solid.pattern.rectangular",
            json!({"bodies": [lead], "dir1": [1, 0, 0], "count1": format!("{row}_leads"),
                "spacing1": format!("{row}_step * pitch"), "name": format!("{row} pattern")}),
        );
    }
    run(
        &mut s,
        "solid.fillet",
        json!({"name": "edge round", "body": "body", "radius": "edge_round", "fit": "reduce",
            "edges": [[0.0, -w / 2.0, top]]}),
    );
    // Looks: every lead at the most leads a side (16), then back.
    let mut changes = serde_json::Map::new();
    for row in ["row_s", "row_n"] {
        changes.insert(format!("{row}_leads"), json!("16"));
        changes.insert(format!("{row}_step"), json!("1"));
    }
    changes.insert("pitch".into(), json!("1 mm"));
    changes.insert("lead_width".into(), json!("0.5 mm"));
    changes.insert("body_length".into(), json!("20 mm"));
    run(&mut s, "parameters.change", json!({ "changes": changes }));
    s.refresh();
    let leads: Vec<String> = s
        .model
        .state()
        .bodies
        .iter()
        .map(|b| b.name.clone())
        .filter(|n| n.contains(" lead"))
        .collect();
    assert_eq!(leads.len(), 32, "{leads:?}");
    let leads: Vec<&str> = leads.iter().map(String::as_str).collect();
    assign_look(&mut s, &["body"], MaterialId::PaWhite);
    assign_look(&mut s, &["lens"], MaterialId::SiliconeClear);
    assign_look(
        &mut s,
        &["leadframe", "driver die"],
        MaterialId::Aluminium,
    );
    assign_look(
        &mut s,
        &["red chip", "green chip", "blue chip"],
        MaterialId::Lcp,
    );
    assign_look(&mut s, &leads, MaterialId::TinMatte);
    let mut changes = serde_json::Map::new();
    for row in ["row_s", "row_n"] {
        changes.insert(format!("{row}_leads"), json!("2"));
        changes.insert(format!("{row}_step"), json!("1"));
    }
    changes.insert("pitch".into(), json!("3.3 mm"));
    changes.insert("lead_width".into(), json!("0.9 mm"));
    changes.insert("body_length".into(), json!("5 mm"));
    run(&mut s, "parameters.change", json!({ "changes": changes }));
    s.refresh();
    finish_part(s, "standoff + body_height")
}

fn author_tabbed_rows() -> Vec<u8> {
    author_tabbed(TabbedSide::Rows)
}

fn author_tabbed_tab_lead() -> Vec<u8> {
    author_tabbed(TabbedSide::TabLead)
}

fn author_tabbed_flange() -> Vec<u8> {
    author_tabbed(TabbedSide::Flange)
}

/// One row of gull-wing leads in a part file: its body name stem, the
/// origin plane its side profile is drawn on, how profile (s, z) maps to
/// sketch (u, v) (signs; a mirroring map turns arcs round), the axis it
/// runs along and its count, and the derived-parameter prefix of its
/// rows' profile (`y` across the long sides, `x` across the short).
struct WingRow {
    part: &'static str,
    plane: &'static str,
    signs: [i8; 2],
    axis: [i8; 3],
    count: &'static str,
    across: &'static str,
}

/// The derived parameters of one gull-wing profile class `a` (`y`: rows
/// on the ±y sides, half body `half_body`, span `span`): the bent lead's
/// side profile as the in-code gull-wing builder drew it (a strip of the
/// lead's thickness along a centreline from 0.25 mm (at most a third of
/// the half body) inside the body at the lead exit, out 30 % of the room
/// to the foot, down to the foot's centreline and out to the tip, each
/// corner bent at a centreline radius of the thickness, reduced so a bend
/// takes at most 45 % of a segment). `half` is tan(θ / 2) of the sloped
/// segment, `ux`, `uz` its direction.
fn wing_profile_params(
    a: &str,
    half_body: &str,
    span: &str,
) -> Vec<(String, String, &'static str, String)> {
    let p = |name: &str| format!("{a}_{name}");
    let row = |name: &str, expr: String, unit: &'static str, comment: &str| {
        (p(name), expr, unit, format!("{comment} ({a} rows)"))
    };
    vec![
        row(
            "root",
            format!("min(0.25 mm, {half_body} / 3)"),
            "mm",
            "how far a lead root reaches into the body",
        ),
        row(
            "foot",
            format!("{span} / 2 - foot_length"),
            "mm",
            "where the foot begins",
        ),
        row(
            "shoulder",
            format!("{half_body} + 0.3 * ({} - {half_body})", p("foot")),
            "mm",
            "where the lead turns down",
        ),
        row(
            "dx",
            format!("{} - {}", p("foot"), p("shoulder")),
            "mm",
            "the sloped segment's run",
        ),
        row(
            "dz",
            "lead_exit - lead_thickness / 2".into(),
            "mm",
            "the sloped segment's drop",
        ),
        row(
            "len",
            format!("hypot({}, {})", p("dx"), p("dz")),
            "mm",
            "the sloped segment's length",
        ),
        row(
            "half",
            format!("{} / ({} + {})", p("dz"), p("len"), p("dx")),
            "",
            "tan of half the bend angle",
        ),
        row(
            "ux",
            format!("{} / {}", p("dx"), p("len")),
            "",
            "the sloped segment's direction along s",
        ),
        row(
            "uz",
            format!("{} / {}", p("dz"), p("len")),
            "",
            "the sloped segment's drop per length",
        ),
        row(
            "bend_b",
            format!(
                "max(min(lead_thickness, 0.45 * min({} + 0.3 * ({} - {half_body}), {}) / {}), lead_thickness / 2 * 1.0001)",
                p("root"),
                p("foot"),
                p("len"),
                p("half")
            ),
            "mm",
            "centreline radius of the upper bend",
        ),
        row(
            "bend_c",
            format!(
                "max(min(lead_thickness, 0.45 * min({}, foot_length) / {}), lead_thickness / 2 * 1.0001)",
                p("len"),
                p("half")
            ),
            "mm",
            "centreline radius of the lower bend",
        ),
        row(
            "tb",
            format!("{} * {}", p("bend_b"), p("half")),
            "mm",
            "the upper bend's tangent length",
        ),
        row(
            "tc",
            format!("{} * {}", p("bend_c"), p("half")),
            "mm",
            "the lower bend's tangent length",
        ),
    ]
}

/// Draw one gull-wing lead's side profile (the closed outline of the bent
/// strip: lines and four bend arcs) in a new sketch on `row.plane`,
/// offset to the row's first lead, every point an expression over the
/// part's parameters; returns the sketch's name.
fn wing_sketch(s: &mut Session, row: &WingRow, half_body: &str, span: &str) -> String {
    let a = row.across;
    let p = |name: &str| format!("{a}_{name}");
    let h = "lead_thickness / 2";
    let (hb, root, foot, shoulder) = (half_body, p("root"), p("foot"), p("shoulder"));
    let (ux, uz, tb, tc) = (p("ux"), p("uz"), p("tb"), p("tc"));
    let hs = format!("{span} / 2");
    // (s, z) of each point.
    let points: Vec<(&str, String, String)> = vec![
        ("1", format!("{hb} - {root}"), format!("lead_exit + {h}")),
        (
            "2",
            format!("{shoulder} - {tb}"),
            format!("lead_exit + {h}"),
        ),
        (
            "3",
            format!("{shoulder} + {tb} * {ux} + {h} * {uz}"),
            format!("lead_exit - {tb} * {uz} + {h} * {ux}"),
        ),
        (
            "4",
            format!("{foot} - {tc} * {ux} + {h} * {uz}"),
            format!("{h} + {tc} * {uz} + {h} * {ux}"),
        ),
        ("5", format!("{foot} + {tc}"), "lead_thickness".into()),
        ("6", hs.clone(), "lead_thickness".into()),
        ("7", hs, "0 mm".into()),
        ("8", format!("{foot} + {tc}"), "0 mm".into()),
        (
            "9",
            format!("{foot} - {tc} * {ux} - {h} * {uz}"),
            format!("{h} + {tc} * {uz} - {h} * {ux}"),
        ),
        (
            "10",
            format!("{shoulder} + {tb} * {ux} - {h} * {uz}"),
            format!("lead_exit - {tb} * {uz} - {h} * {ux}"),
        ),
        (
            "11",
            format!("{shoulder} - {tb}"),
            format!("lead_exit - {h}"),
        ),
        ("12", format!("{hb} - {root}"), format!("lead_exit - {h}")),
        (
            "cb",
            format!("{shoulder} - {tb}"),
            format!("lead_exit - {}", p("bend_b")),
        ),
        (
            "cc",
            format!("{foot} + {tc}"),
            format!("{h} + {}", p("bend_c")),
        ),
    ];
    let sign = |k: i8, e: &str| {
        if k < 0 {
            format!("-({e})")
        } else {
            format!("({e})")
        }
    };
    let sketch = format!("{} profile", row.part);
    run(
        s,
        "sketch.create",
        json!({"plane": row.plane, "offset": format!("-({} - 1) / 2 * pitch - lead_width / 2", row.count),
            "name": sketch}),
    );
    for (id, ps, pz) in &points {
        run(
            s,
            "sketch.point",
            json!({"point": [sign(row.signs[0], ps), sign(row.signs[1], pz)], "id": format!("w{id}")}),
        );
    }
    let w = |id: &str| format!("w{id}");
    let line = |s: &mut Session, a: &str, b: &str| {
        run(
            s,
            "sketch.line",
            json!({"points": [w(a), w(b)], "infer": false}),
        );
    };
    // Arcs run counter-clockwise from start to end in the profile's own
    // (s, z) frame; a mirroring map turns them round.
    let mirrored = row.signs[0] * row.signs[1] < 0;
    let arc = |s: &mut Session, centre: &str, start: &str, end: &str| {
        let (start, end) = if mirrored { (end, start) } else { (start, end) };
        run(
            s,
            "sketch.arc.center_point",
            json!({"center": w(centre), "start": w(start), "end": w(end)}),
        );
    };
    line(s, "1", "2");
    arc(s, "cb", "3", "2");
    line(s, "3", "4");
    arc(s, "cc", "4", "5");
    line(s, "5", "6");
    line(s, "6", "7");
    line(s, "7", "8");
    arc(s, "cc", "9", "8");
    line(s, "9", "10");
    arc(s, "cb", "10", "11");
    line(s, "11", "12");
    line(s, "12", "1");
    run(s, "sketch.finish", json!({}));
    sketch
}

/// A gull-wing part file of the `gull-wing` generator's bundle: the
/// in-code gull-wing builder's construction made parametric in its counts.
/// A moulded `body` on its standoff; per row of leads (`row_s`, `row_n`,
/// and on four sides `row_e`, `row_w`, in the builder's order) one formed
/// lead extruded `lead_width` from its side profile and a body pattern of
/// `leads` (`leads_short`) at `pitch` replaying it, and the same profile
/// cut out of the body as the lead's root, patterned the same way, so the
/// leads and the moulding stay disjoint at any count and nothing names
/// the pattern's bodies. Then the body's rounded upright corners (clear of
/// the end leads) and top edges (clear of the lead exit) and the pin-1
/// dimple, each derived from the dimensions as the builder derived them.
/// Saved at SOIC-8 (two sides) or LQFP-48 7 × 7 mm (four); the generator
/// gives every dimension.
fn author_gull_wing(four: bool, bevel: bool) -> Vec<u8> {
    let mut s = Session::default();
    let title = if four {
        "Gull-wing package, four sides (QFP)"
    } else {
        "Gull-wing package, two sides (SOIC, SSOP, TSSOP)"
    };
    run(&mut s, "file.new", json!({"name": title}));
    // The saved size: SOIC-8 3.9 mm or LQFP-48 7 × 7 mm 0.5 mm.
    let (n, m, pitch, bl, bw, bh, standoff, span, span_x, lw, t, foot) = if four {
        (12, 12, 0.5, 7.0, 7.0, 1.4, 0.1, 9.0, 9.0, 0.22, 0.145, 0.6)
    } else {
        (
            4, 0, 1.27, 4.9, 3.9, 1.45, 0.175, 6.0, 0.0, 0.41, 0.2, 0.835,
        )
    };
    let mm = |v: f64| format!("{v} mm");
    let mut params: Vec<(String, String, &'static str, String)> = vec![
        (
            "leads".into(),
            n.to_string(),
            "",
            "leads on each long side (±y)".into(),
        ),
        ("pitch".into(), mm(pitch), "mm", "lead pitch".into()),
        (
            "body_length".into(),
            mm(bl),
            "mm",
            "moulded body length (x)".into(),
        ),
        (
            "body_width".into(),
            mm(bw),
            "mm",
            "moulded body width (y)".into(),
        ),
        (
            "body_height".into(),
            mm(bh),
            "mm",
            "moulded body height above its underside".into(),
        ),
        (
            "standoff".into(),
            mm(standoff),
            "mm",
            "seating plane to the body's underside".into(),
        ),
        (
            "span".into(),
            mm(span),
            "mm",
            "lead tip to tip across y".into(),
        ),
        ("lead_width".into(), mm(lw), "mm", "lead width".into()),
        (
            "lead_thickness".into(),
            mm(t),
            "mm",
            "lead thickness".into(),
        ),
        (
            "foot_length".into(),
            mm(foot),
            "mm",
            "flat foot length on the seating plane".into(),
        ),
    ];
    if four {
        params.insert(
            1,
            (
                "leads_short".into(),
                m.to_string(),
                "",
                "leads on each short side (±x)".into(),
            ),
        );
        params.insert(
            8,
            (
                "span_x".into(),
                mm(span_x),
                "mm",
                "lead tip to tip across x".into(),
            ),
        );
    }
    params.push((
        "lead_exit".into(),
        "standoff + body_height / 2".into(),
        "mm",
        "where the leads leave the body: mid-height".into(),
    ));
    params.extend(wing_profile_params("y", "body_width / 2", "span"));
    if four {
        params.extend(wing_profile_params("x", "body_length / 2", "span_x"));
    }
    let row_room =
        |count: &str, edge: &str| format!("({edge} - (({count} - 1) * pitch + lead_width)) / 4");
    let corner = if four {
        format!(
            "min(0.2 mm, 0.06 * min(body_length, body_width), {}, {})",
            row_room("leads", "body_length"),
            row_room("leads_short", "body_width")
        )
    } else {
        format!(
            "min(0.2 mm, 0.06 * min(body_length, body_width), {})",
            row_room("leads", "body_length")
        )
    };
    params.push((
        "draft".into(),
        "8 deg".into(),
        "deg",
        "mould draft of the walls above and below the parting line (Microchip SOIC drawing C04-057: 5–15°)".into(),
    ));
    params.push((
        "draft_inset".into(),
        "(standoff + body_height - lead_exit) * tan(draft)".into(),
        "mm",
        "how far the top face's edge lies in from the parting line's".into(),
    ));
    params.push((
        "corner_round".into(),
        format!("max({corner}, draft_inset + 0.04 mm)"),
        "mm",
        "round of the body's upright corners at the parting line, clear of the end leads".into(),
    ));
    params.push((
        "edge_round".into(),
        "min(0.1 mm, 0.08 * body_height, (standoff + body_height - lead_exit - lead_thickness / 2) / 2, 0.6 * (corner_round - draft_inset))".into(),
        "mm",
        "round of the body's top edges, clear of the lead exit and the top's corner arcs".into(),
    ));
    params.push((
        "dimple_radius".into(),
        "min(max(0.08 * min(body_length, body_width), 0.15 mm), 0.5 mm)".into(),
        "mm",
        "pin-1 dimple radius".into(),
    ));
    params.push((
        "dimple_depth".into(),
        "0.04 mm".into(),
        "mm",
        "pin-1 dimple depth".into(),
    ));
    if bevel {
        params.push((
            "bevel".into(),
            "0.35 mm".into(),
            "mm",
            "the pin-1 side's bevelled top edge, 45° (Microchip SOIC drawing C04-057: chamfer h 0.25–0.50)".into(),
        ));
    }
    params.push((
        "marking_top_u".into(),
        "0.75 * (body_length - 2 * draft_inset)".into(),
        "mm",
        "marking-top size along its u axis".into(),
    ));
    params.push((
        "marking_top_v".into(),
        "0.6 * (body_width - 2 * draft_inset)".into(),
        "mm",
        "marking-top size across it".into(),
    ));
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
    // The moulding, picked at the saved size.
    let inset = bh / 2.0 * 8f64.to_radians().tan();
    moulded_body(
        &mut s,
        "body",
        ["body_length", "body_width"],
        ["standoff", "lead_exit", "standoff + body_height"],
        "corner_round",
        "draft",
        "edge_round",
        [0.0, -(bw / 2.0 - inset), standoff + bh],
    );
    // The builder's rows: −y, (+x), +y, (−x). YZ maps sketch (u, v) to
    // (offset, u, v), XZ to (u, offset, −v).
    let y_row = |part: &'static str, plus: bool| WingRow {
        part,
        plane: "YZ",
        signs: [if plus { 1 } else { -1 }, 1],
        axis: [1, 0, 0],
        count: "leads",
        across: "y",
    };
    let x_row = |part: &'static str, plus: bool| WingRow {
        part,
        plane: "XZ",
        signs: [if plus { 1 } else { -1 }, -1],
        axis: [0, 1, 0],
        count: "leads_short",
        across: "x",
    };
    let mut wing_rows = vec![y_row("row_s", false)];
    if four {
        wing_rows.push(x_row("row_e", true));
        wing_rows.push(y_row("row_n", true));
        wing_rows.push(x_row("row_w", false));
    } else {
        wing_rows.push(y_row("row_n", true));
    }
    for row in &wing_rows {
        let (half_body, span) = if row.across == "y" {
            ("body_width / 2", "span")
        } else {
            ("body_length / 2", "span_x")
        };
        let sketch = wing_sketch(&mut s, row, half_body, span);
        let lead = format!("{} lead", row.part);
        run(
            &mut s,
            "solid.extrude",
            json!({"sketch": sketch, "distance": "lead_width", "operation": "new",
                "name": lead, "body_name": lead}),
        );
        let root = format!("{} root", row.part);
        run(
            &mut s,
            "solid.extrude",
            json!({"sketch": sketch, "distance": "lead_width", "operation": "cut",
                "targets": ["body"], "name": root}),
        );
        run(
            &mut s,
            "solid.pattern.rectangular",
            json!({"bodies": [lead], "dir1": row.axis, "count1": row.count, "spacing1": "pitch",
                "name": format!("{} pattern", row.part)}),
        );
        run(
            &mut s,
            "solid.pattern.rectangular",
            json!({"features": [root], "dir1": row.axis, "count1": row.count, "spacing1": "pitch",
                "name": format!("{} roots", row.part)}),
        );
    }
    // The pin-1 dimple, two radii in from the top face's corner.
    run(
        &mut s,
        "solid.cylinder",
        json!({"name": "pin 1 mark", "radius": "dimple_radius", "height": "2 * dimple_depth",
            "base": ["- body_length / 2 + draft_inset + 2 * dimple_radius",
                "- body_width / 2 + draft_inset + 2 * dimple_radius",
                "standoff + body_height - dimple_depth"],
            "axis": "Z", "operation": "cut", "targets": ["body"]}),
    );
    if bevel {
        // The pin-1 side's top edge bevelled at 45° along the whole body:
        // a prism through the body's length, cut away.
        let y0 = "- (body_width / 2 - draft_inset)";
        let top = "standoff + body_height";
        run(
            &mut s,
            "sketch.create",
            json!({"plane": "YZ", "offset": "- body_length", "name": "pin 1 bevel profile"}),
        );
        run(
            &mut s,
            "sketch.line",
            json!({"points": [
                [format!("{y0} - 0.5 mm"), format!("{top} - bevel - 0.5 mm")],
                [format!("{y0} - 0.5 mm"), format!("{top} + 0.5 mm")],
                [format!("{y0} + bevel + 0.5 mm"), format!("{top} + 0.5 mm")]],
                "closed": true}),
        );
        run(&mut s, "sketch.finish", json!({}));
        run(
            &mut s,
            "solid.extrude",
            json!({"sketch": "pin 1 bevel profile", "distance": "2 * body_length",
                "operation": "cut", "targets": ["body"], "name": "pin 1 bevel"}),
        );
    }
    marking_plane(&mut s, "standoff + body_height");
    // Looks are kept by body name, and a pattern names its copies
    // `row_s lead (2)`, …: every lead the generator can ask for (128 a
    // side) gets the lead look at that count, then the file goes back to
    // its saved counts.
    let counts = |s: &mut Session, leads: u32, short: u32, pitch: &str| {
        let mut changes = serde_json::Map::new();
        changes.insert("leads".into(), json!(leads.to_string()));
        changes.insert("pitch".into(), json!(pitch));
        // Room for the most leads at the smallest pitch.
        changes.insert(
            "body_length".into(),
            json!(if leads > n {
                "70 mm".to_owned()
            } else {
                mm(bl)
            }),
        );
        if four {
            changes.insert("leads_short".into(), json!(short.to_string()));
            changes.insert(
                "body_width".into(),
                json!(if short > m {
                    "70 mm".to_owned()
                } else {
                    mm(bw)
                }),
            );
            changes.insert(
                "span_x".into(),
                json!(if leads > n {
                    "72 mm".to_owned()
                } else {
                    mm(span_x)
                }),
            );
            changes.insert(
                "span".into(),
                json!(if short > m {
                    "72 mm".to_owned()
                } else {
                    mm(span)
                }),
            );
        }
        run(s, "parameters.change", json!({"changes": changes}));
        s.refresh();
    };
    let most = 128;
    counts(&mut s, most, if four { most } else { 0 }, "0.5 mm");
    let leads: Vec<String> = s
        .model
        .state()
        .bodies
        .iter()
        .map(|b| b.name.clone())
        .filter(|n| n != "body")
        .collect();
    assert_eq!(leads.len(), if four { 4 * 128 } else { 2 * 128 });
    let leads: Vec<&str> = leads.iter().map(String::as_str).collect();
    // The gull-wing family's materials: a moulded epoxy body, satin
    // bright tin leads.
    assign_look(&mut s, &["body"], MaterialId::EpoxyMould);
    for chunk in leads.chunks(200) {
        assign_look(&mut s, chunk, MaterialId::TinBright);
    }
    counts(&mut s, n, m, &mm(pitch));
    let errors: Vec<String> = s
        .model
        .results
        .iter()
        .filter_map(|r| r.error.clone().map(|e| format!("{}: {e}", r.name)))
        .collect();
    assert!(errors.is_empty(), "{errors:?}");
    solidcraft_engine::io::write_design(&s.doc)
}

fn author_gull_wing_two_sided() -> Vec<u8> {
    author_gull_wing(false, false)
}

fn author_gull_wing_two_sided_bevel() -> Vec<u8> {
    author_gull_wing(false, true)
}

fn author_gull_wing_four_sided() -> Vec<u8> {
    author_gull_wing(true, false)
}

/// Pad-row slots of a module part file: two rows on each side, in the
/// order left, bottom, right, top, then the second row of each side (the
/// `module` generator's `ROW_SLOTS`).
const MODULE_SIDES: [&str; 8] = [
    "left", "bottom", "right", "top", "left", "bottom", "right", "top",
];
/// Single-pad slots (the generator's `SINGLE_SLOTS`).
const MODULE_SINGLES: usize = 16;
/// Most pads of one row and most ground pads along one grid direction,
/// whose looks the file carries (the generator's `MAX_ROW_PADS`,
/// `MAX_GROUND_SIDE`).
const MODULE_MOST_ROW: usize = 64;
const MODULE_MOST_GROUND: usize = 16;
/// √½, the turn of a diagonal ground array, as `f64` writes cos 45°.
const HALF_SQRT2: f64 = std::f64::consts::FRAC_1_SQRT_2;

/// The step from one pad of a row on `side` to the next (counter-clockwise
/// round the module, seen from above), and the inward normal of the side.
fn module_side(side: &str) -> ([f64; 3], [f64; 2]) {
    match side {
        "left" => ([0.0, -1.0, 0.0], [1.0, 0.0]),
        "bottom" => ([1.0, 0.0, 0.0], [0.0, 1.0]),
        "right" => ([0.0, 1.0, 0.0], [-1.0, 0.0]),
        _ => ([-1.0, 0.0, 0.0], [0.0, -1.0]),
    }
}

/// One step of a closed sketch outline: a line to a point, or a clockwise
/// arc about a centre to a point (expressions in mm).
enum Step<'a> {
    Line([&'a str; 2]),
    ClockwiseArc {
        centre: [&'a str; 2],
        to: [&'a str; 2],
    },
}

/// A closed outline in the open sketch from `start`, its points tagged
/// `tag`: sketch points fixed by expressions, lines and centre-point arcs
/// (a clockwise arc from p to q is drawn counter-clockwise from q to p).
fn sketch_outline(s: &mut Session, tag: &str, start: [&str; 2], steps: &[Step<'_>]) {
    let mut n = 0usize;
    let mut point = |s: &mut Session, at: [&str; 2]| -> String {
        n += 1;
        let id = format!("{tag}{n}");
        run(
            s,
            "sketch.point",
            json!({"point": [at[0], at[1]], "id": id}),
        );
        id
    };
    let first = point(s, start);
    let mut last = first.clone();
    for (k, step) in steps.iter().enumerate() {
        let closing = k + 1 == steps.len();
        match step {
            Step::Line(to) => {
                let next = if closing {
                    first.clone()
                } else {
                    point(s, *to)
                };
                run(
                    s,
                    "sketch.line",
                    json!({"points": [last, next], "infer": false}),
                );
                last = next;
            }
            Step::ClockwiseArc { centre, to } => {
                let c = point(s, *centre);
                let next = if closing {
                    first.clone()
                } else {
                    point(s, *to)
                };
                run(
                    s,
                    "sketch.arc.center_point",
                    json!({"center": c, "start": next, "end": last}),
                );
                last = next;
            }
        }
    }
}

/// A part file of the `module` generator's bundle: a radio or processor
/// module (a soldermasked board on its gold pads, a nickel-silver shield
/// can, a printed antenna meander at the +y end), the retired in-code
/// module builder's construction made parametric in its rows, ground
/// array, single pads, castellations and antenna. Frame as the module
/// form's: x across (`width`), y along (`length`), the antenna end at +y,
/// the origin at the outline centre.
///
/// - **Board**: a box of `width` × `length` × `board_thickness` standing on
///   the pads (`pad_thickness`); in the `castellated` files the outline
///   with its notches drawn in (see below), extruded once.
/// - **Rows**: eight slots, two on each side (left, bottom, right, top,
///   then each side's second). Slot `i` is a seed pad `row i pad` at its
///   first pad's centre (`row_i_x`, `row_i_y`) and a body pattern of
///   `row_i_count` at `pitch` along the side (counter-clockwise), so the
///   copies are `row i pad (2)`, `(3)`, … as the builder named them. Each
///   pad is `pad_length` across its edge and `pad_width` along it.
/// - **Castellations** (`castellated` files only; LGA modules use files
///   without them): per slot, a half ring of plating `row i castellation`
///   (from `notch_radius − plating` to `notch_radius` about `notch_i_x`,
///   `notch_i_y` on the edge, through the board) and a body pattern of
///   `notch_i_count`. The notches are circles of `notch_radius` in the
///   board's sketch, each slot's a sketch pattern of `notch_i_count` at
///   `pitch`, and the board is the profile round the outline's centre: no
///   boolean, where a cut of the board per notch (or per slot's pattern of
///   cuts) cost about 1 s a module.
/// - **Ground array**: a seed `ground pad` of `ground_size_x` ×
///   `ground_size_y` at (`ground_x`, `ground_y`) and a two-way body
///   pattern of `ground_columns` × `ground_rows` at `ground_pitch_x`,
///   `ground_pitch_y`: along +x and −y in this file, or turned 45°
///   (`diagonal`, the seed a turned square).
/// - **Single pads** `single pad 1` … `16` of `single_k_length` ×
///   `single_k_width` at `single_k_x`, `single_k_y`.
/// - **Shield can** `can_width` × `can_length` at (`can_x`, `can_y`) on the
///   board up to `height`, its top edges rounded by `can_round`.
/// - **Antenna**: the builder's meander (`antenna_turns` up-and-down turns
///   of `antenna_step` across the antenna region inset by `antenna_margin`,
///   a feed leg down to the region's lower edge, `antenna_trace` wide,
///   bent with the builder's centreline radius of 1.0001 half widths) as a
///   head, `antenna_units` joined copies of one turn and a tail.
/// - **Parking**: what a module does not have (a slot without a row, a
///   row without castellations, no ground array, unused single pads, no
///   antenna, the turn copies of a one-turn meander) is placed at
///   `park_x`, beyond the −x edge, and one cut removes everything there;
///   so one file serves every module, and the bodies left are exactly the
///   builder's.
///
/// Looks are kept by body name: every name the patterns can give (64 pads
/// a row, 16 × 16 ground pads) carries its look. Marking planes as the
/// WROOM-3 part file's (`surface::anchors` of the module form); `pin1` at
/// (`pin1_x`, `pin1_y`). Saved at the module form's typical values (an
/// original synthetic 18 × 25 mm castellated module: 14 + 10 + 14 pads at
/// 1.27 mm, a 3 × 3 ground array, turned 45° in the diagonal file, a 6 mm
/// antenna end); the generator gives every value.
pub fn author_module(diagonal: bool, castellated: bool) -> Vec<u8> {
    let mut s = Session::default();
    let title = match (castellated, diagonal) {
        (false, false) => "Radio module",
        (false, true) => "Radio module, ground array turned 45°",
        (true, false) => "Castellated radio module",
        (true, true) => "Castellated radio module, ground array turned 45°",
    };
    run(&mut s, "file.new", json!({"name": title}));
    let mm = |v: f64| format!("{v} mm");
    // The typical module: rows left (14), bottom (10), right (14).
    let (hw, hl) = (9.0, 12.5);
    let saved_rows: [(usize, [f64; 2], Option<[f64; 2]>); 3] = [
        (14, [-hw + 0.6, hl - 7.0], Some([-hw, hl - 7.0])),
        (10, [-5.715, -hl + 0.6], Some([-5.715, -hl])),
        (
            14,
            [hw - 0.6, hl - 7.0 - 13.0 * 1.27],
            Some([hw, hl - 7.0 - 13.0 * 1.27]),
        ),
    ];
    let park = -hw - 100.0;
    let mut rows = vec![
        param("width", "18 mm", "mm", "module outline across (x)"),
        param(
            "length",
            "25 mm",
            "mm",
            "module outline along (y); the antenna end is +y",
        ),
        param(
            "height",
            "3.2 mm",
            "mm",
            "seated height: pads, board and can",
        ),
        param("board_thickness", "0.8 mm", "mm", "module board thickness"),
        param(
            "pad_thickness",
            "0.035 mm",
            "mm",
            "pad thickness under the board (est.: ENIG over 35 µm copper)",
        ),
        param("can_width", "16.6 mm", "mm", "shield can across (x)"),
        param("can_length", "17.4 mm", "mm", "shield can along (y)"),
        param("can_x", "0 mm", "mm", "shield can centre x"),
        param("can_y", "-2.5 mm", "mm", "shield can centre y"),
        param(
            "can_height",
            "height - board_thickness - pad_thickness",
            "mm",
            "shield can height above the board",
        ),
        param("can_round", "0.3 mm", "mm", "round of the can's top edges"),
        param(
            "can_emboss_inset",
            "1 mm",
            "mm",
            "the stamped emboss groove's inset from the can's sides",
        ),
        param(
            "can_emboss_width",
            "0.3 mm",
            "mm",
            "the emboss groove's width (est.)",
        ),
        param(
            "can_emboss_depth",
            "0.05 mm",
            "mm",
            "the emboss groove's depth (est.)",
        ),
        param("pitch", "1.27 mm", "mm", "pad pitch within every row"),
        param("pad_length", "1.2 mm", "mm", "row pad across its edge"),
        param("pad_width", "0.9 mm", "mm", "row pad along its edge"),
        param(
            "park_x",
            &mm(park),
            "mm",
            "x where what the module does not have is placed and cut away",
        ),
        param(
            "antenna_length",
            "6 mm",
            "mm",
            "antenna region depth from the +y edge",
        ),
        param(
            "antenna_margin",
            "0.8 mm",
            "mm",
            "meander inset in the antenna region (est.)",
        ),
        param(
            "antenna_trace",
            "0.45 mm",
            "mm",
            "antenna trace width (est.)",
        ),
        param(
            "antenna_thickness",
            "0.035 mm",
            "mm",
            "antenna trace thickness (est.: 35 µm copper)",
        ),
        param("antenna_turns", "6", "", "meander turns"),
        param(
            "antenna_units",
            "5",
            "",
            "copies of one turn between the head and the tail (turns − 1, at least 1)",
        ),
        param(
            "antenna_step",
            &mm(16.4 / 12.0),
            "mm",
            "meander step along x",
        ),
        param(
            "antenna_shift",
            "0 mm",
            "mm",
            "antenna x shift (to park_x when there is none)",
        ),
        param(
            "antenna_unit_shift",
            "0 mm",
            "mm",
            "x shift of the turn copies (to park_x for a one-turn meander)",
        ),
        param("ant_s", "antenna_step", "mm", "meander step along x"),
        param("ant_h", "antenna_trace / 2", "mm", "half the trace width"),
        param(
            "ant_r",
            "ant_h * 1.0001",
            "mm",
            "bend radius of the trace's centreline",
        ),
        param("ant_ro", "ant_r + ant_h", "mm", "outer bend radius"),
        param(
            "ant_x0",
            "- width / 2 + antenna_margin + antenna_shift",
            "mm",
            "meander start x",
        ),
        param(
            "antenna_depth",
            "6 mm",
            "mm",
            "depth of the region the meander is drawn in: the antenna region's, or room for a parked one",
        ),
        param(
            "ant_y0",
            "length / 2 - antenna_depth + antenna_margin",
            "mm",
            "feed leg foot y",
        ),
        param(
            "ant_y1",
            "length / 2 - antenna_margin",
            "mm",
            "meander top y",
        ),
        param(
            "ant_yl",
            "ant_y0 + 0.4 * (ant_y1 - ant_y0)",
            "mm",
            "meander bottom y",
        ),
        param(
            "ant_ux",
            "ant_x0 + antenna_unit_shift",
            "mm",
            "first turn copy x",
        ),
        param(
            "ant_tx",
            "ant_x0 + 2 * (antenna_turns - 1) * antenna_step",
            "mm",
            "tail x",
        ),
        param("ground_rows", "3", "", "ground array rows"),
        param("ground_columns", "3", "", "ground array columns"),
        param(
            "ground_pitch_x",
            "1.6 mm",
            "mm",
            "ground pitch along the array's x",
        ),
        param(
            "ground_pitch_y",
            "1.6 mm",
            "mm",
            "ground pitch along the array's y",
        ),
        param(
            "ground_size_x",
            "1.1 mm",
            "mm",
            "ground pad size along the array's x",
        ),
        param(
            "ground_size_y",
            "1.1 mm",
            "mm",
            "ground pad size along the array's y",
        ),
        param(
            "mark_pcb_depth",
            "6.3 mm",
            "mm",
            "board beyond the can's +y end (the pcb-top marking plane; 1 µm where the can reaches the edge)",
        ),
        param(
            "mark_antenna_depth",
            "6 mm",
            "mm",
            "the antenna-area marking plane's depth (1 µm without an antenna)",
        ),
        param("pin1_x", &mm(saved_rows[0].1[0]), "mm", "pin 1 x"),
        param("pin1_y", &mm(saved_rows[0].1[1]), "mm", "pin 1 y"),
    ];
    // The seed ground pad: row 0, column 0 of the array about (0, −2).
    let seed = if diagonal {
        [-2.0 * 1.6 * HALF_SQRT2, -2.0]
    } else {
        [-1.6, -2.0 + 1.6]
    };
    rows.push(param(
        "ground_x",
        &mm(seed[0]),
        "mm",
        "seed ground pad centre x",
    ));
    rows.push(param(
        "ground_y",
        &mm(seed[1]),
        "mm",
        "seed ground pad centre y",
    ));
    for (i, side) in MODULE_SIDES.iter().enumerate() {
        let k = i + 1;
        let (count, first, notch) = saved_rows
            .get(i)
            .map_or((1, [park, 0.0], [park, 0.0]), |(n, f, c)| {
                (*n, *f, c.unwrap_or([park, 0.0]))
            });
        rows.push(param(
            &format!("row_{k}_count"),
            &count.to_string(),
            "",
            &format!("pads of slot {k} ({side} side)"),
        ));
        rows.push(param(
            &format!("row_{k}_x"),
            &mm(first[0]),
            "mm",
            &format!("slot {k}: first pad centre x"),
        ));
        rows.push(param(
            &format!("row_{k}_y"),
            &mm(first[1]),
            "mm",
            &format!("slot {k}: first pad centre y"),
        ));
        let notches = if saved_rows.get(i).is_some() {
            count
        } else {
            1
        };
        if !castellated {
            continue;
        }
        rows.push(param(
            &format!("notch_{k}_count"),
            &notches.to_string(),
            "",
            &format!("castellations of slot {k}"),
        ));
        rows.push(param(
            &format!("notch_{k}_x"),
            &mm(notch[0]),
            "mm",
            &format!("slot {k}: first castellation centre x (on the edge)"),
        ));
        rows.push(param(
            &format!("notch_{k}_y"),
            &mm(notch[1]),
            "mm",
            &format!("slot {k}: first castellation centre y"),
        ));
    }
    if castellated {
        rows.push(param(
            "notch_radius",
            "0.3 mm",
            "mm",
            "castellation half-hole radius",
        ));
        rows.push(param(
            "plating",
            "0.03 mm",
            "mm",
            "castellation plating thickness in the notch (est.)",
        ));
    }
    for k in 1..=MODULE_SINGLES {
        rows.push(param(
            &format!("single_{k}_x"),
            &mm(park),
            "mm",
            &format!("single pad {k} centre x"),
        ));
        rows.push(param(
            &format!("single_{k}_y"),
            "0 mm",
            "mm",
            &format!("single pad {k} centre y"),
        ));
        rows.push(param(
            &format!("single_{k}_length"),
            "0.8 mm",
            "mm",
            &format!("single pad {k} size x"),
        ));
        rows.push(param(
            &format!("single_{k}_width"),
            "0.8 mm",
            "mm",
            &format!("single pad {k} size y"),
        ));
    }
    import(&mut s, &rows);
    // The board on its pads: a box, or (castellated) the outline with
    // every notch drawn in, extruded once, as the builder drew it.
    if castellated {
        run(
            &mut s,
            "sketch.create",
            json!({"plane": "XY", "offset": "pad_thickness", "name": "board sketch"}),
        );
        let (l, r, b, t) = ("- width / 2", "width / 2", "- length / 2", "length / 2");
        sketch_outline(
            &mut s,
            "b",
            [l, b],
            &[
                Step::Line([r, b]),
                Step::Line([r, t]),
                Step::Line([l, t]),
                Step::Line([l, b]),
            ],
        );
        for (i, side) in MODULE_SIDES.iter().enumerate() {
            let k = i + 1;
            let (step, _) = module_side(side);
            let id = format!("notch {k}");
            run(
                &mut s,
                "sketch.circle.center",
                json!({"center": [format!("notch_{k}_x"), format!("notch_{k}_y")],
                    "radius": "notch_radius", "id": id}),
            );
            run(
                &mut s,
                "sketch.pattern.rectangular",
                json!({"entities": [id], "dir": [step[0], step[1]],
                    "count": format!("notch_{k}_count"), "spacing": "pitch"}),
            );
        }
        run(&mut s, "sketch.finish", json!({}));
        run(
            &mut s,
            "solid.extrude",
            json!({"sketch": "board sketch", "profiles": [{"point": [0.0, 0.0]}],
                "distance": "board_thickness", "operation": "new",
                "name": "board", "body_name": "board"}),
        );
        // The board is the profile round the outline's centre, which no
        // notch reaches. The command keeps a picked point as the curves
        // round it, which name every notch copy and so fail at another
        // count; the stored point pick is found again at every count.
        let points: solidcraft_engine::doc::ProfileSel =
            serde_json::from_value(json!({"type": "points", "points": [[0.0, 0.0]]})).unwrap();
        let board = s
            .doc_mut()
            .features
            .iter_mut()
            .find(|f| f.name == "board")
            .unwrap();
        match &mut board.kind {
            solidcraft_engine::doc::FeatureKind::Extrude { profiles, .. } => *profiles = points,
            other => panic!("board: {other:?}"),
        }
        s.refresh();
    } else {
        run(
            &mut s,
            "solid.box",
            json!({"corner": ["- width / 2", "- length / 2", "pad_thickness"],
                "length": "width", "width": "length", "height": "board_thickness",
                "name": "board", "body_name": "board"}),
        );
    }
    // Rows: seed pad and pattern; castellation plating (a half ring and
    // its pattern).
    for (i, side) in MODULE_SIDES.iter().enumerate() {
        let k = i + 1;
        let (step, inward) = module_side(side);
        let across_x = matches!(*side, "left" | "right");
        let (sx, sy) = if across_x {
            ("pad_length", "pad_width")
        } else {
            ("pad_width", "pad_length")
        };
        let pad = format!("row {k} pad");
        run(
            &mut s,
            "solid.box",
            json!({"corner": [format!("row_{k}_x - {sx} / 2"), format!("row_{k}_y - {sy} / 2"), "0 mm"],
                "length": sx, "width": sy, "height": "pad_thickness",
                "name": pad, "body_name": pad}),
        );
        run(
            &mut s,
            "solid.pattern.rectangular",
            json!({"bodies": [pad], "dir1": step, "count1": format!("row_{k}_count"),
                "spacing1": "pitch", "name": format!("row {k} pattern")}),
        );
        if !castellated {
            continue;
        }
        // The half ring inward of the edge point: counter-clockwise from
        // −t to +t, t the inward normal turned a quarter left.
        let t = [-inward[1], inward[0]];
        let along = |sign: f64, radius: &str| -> [String; 2] {
            let term = |c: f64| {
                if c * sign > 0.0 {
                    format!(" + {radius}")
                } else if c * sign < 0.0 {
                    format!(" - {radius}")
                } else {
                    String::new()
                }
            };
            [
                format!("notch_{k}_x{}", term(t[0])),
                format!("notch_{k}_y{}", term(t[1])),
            ]
        };
        let sketch = format!("row {k} castellation sketch");
        run(
            &mut s,
            "sketch.create",
            json!({"plane": "XY", "offset": "pad_thickness", "name": sketch}),
        );
        let [o1, o2, i2, i1] = [
            along(-1.0, "notch_radius"),
            along(1.0, "notch_radius"),
            along(1.0, "(notch_radius - plating)"),
            along(-1.0, "(notch_radius - plating)"),
        ];
        let c = [format!("notch_{k}_x"), format!("notch_{k}_y")];
        let tag = format!("c{k}_");
        let id = |n: &str| format!("{tag}{n}");
        for (n, p) in [
            ("c", &c),
            ("o1", &o1),
            ("o2", &o2),
            ("i1", &i1),
            ("i2", &i2),
        ] {
            run(
                &mut s,
                "sketch.point",
                json!({"point": [p[0], p[1]], "id": id(n)}),
            );
        }
        run(
            &mut s,
            "sketch.arc.center_point",
            json!({"center": id("c"), "start": id("o1"), "end": id("o2")}),
        );
        run(
            &mut s,
            "sketch.line",
            json!({"points": [id("o2"), id("i2")], "infer": false}),
        );
        run(
            &mut s,
            "sketch.arc.center_point",
            json!({"center": id("c"), "start": id("i1"), "end": id("i2")}),
        );
        run(
            &mut s,
            "sketch.line",
            json!({"points": [id("i1"), id("o1")], "infer": false}),
        );
        run(&mut s, "sketch.finish", json!({}));
        let plate = format!("row {k} castellation");
        run(
            &mut s,
            "solid.extrude",
            json!({"sketch": sketch, "distance": "board_thickness", "operation": "new",
                "name": plate, "body_name": plate}),
        );
        run(
            &mut s,
            "solid.pattern.rectangular",
            json!({"bodies": [plate], "dir1": step, "count1": format!("notch_{k}_count"),
                "spacing1": "pitch", "name": format!("row {k} castellation pattern")}),
        );
    }
    // The ground array.
    let ground = "ground pad";
    let (dir1, dir2) = if diagonal {
        run(
            &mut s,
            "sketch.create",
            json!({"plane": "XY", "offset": "0 mm", "name": "ground pad sketch"}),
        );
        let k = HALF_SQRT2;
        let corner = |u: &str, v: &str| -> [String; 2] {
            [
                format!("ground_x + {k} * (({u}) - ({v}))"),
                format!("ground_y + {k} * (({u}) + ({v}))"),
            ]
        };
        let (hx, hy) = ("ground_size_x / 2", "ground_size_y / 2");
        let (mx, my) = ("- ground_size_x / 2", "- ground_size_y / 2");
        let p = [
            corner(mx, my),
            corner(hx, my),
            corner(hx, hy),
            corner(mx, hy),
        ];
        let at = |q: &[String; 2]| [q[0].clone(), q[1].clone()];
        let [a, b, c, d] = [at(&p[0]), at(&p[1]), at(&p[2]), at(&p[3])];
        sketch_outline(
            &mut s,
            "g",
            [&a[0], &a[1]],
            &[
                Step::Line([&b[0], &b[1]]),
                Step::Line([&c[0], &c[1]]),
                Step::Line([&d[0], &d[1]]),
                Step::Line([&a[0], &a[1]]),
            ],
        );
        run(&mut s, "sketch.finish", json!({}));
        run(
            &mut s,
            "solid.extrude",
            json!({"sketch": "ground pad sketch", "distance": "pad_thickness", "operation": "new",
                "name": ground, "body_name": ground}),
        );
        ([k, k, 0.0], [k, -k, 0.0])
    } else {
        run(
            &mut s,
            "solid.box",
            json!({"corner": ["ground_x - ground_size_x / 2", "ground_y - ground_size_y / 2", "0 mm"],
                "length": "ground_size_x", "width": "ground_size_y", "height": "pad_thickness",
                "name": ground, "body_name": ground}),
        );
        ([1.0, 0.0, 0.0], [0.0, -1.0, 0.0])
    };
    run(
        &mut s,
        "solid.pattern.rectangular",
        json!({"bodies": [ground], "dir1": dir1, "count1": "ground_columns", "spacing1": "ground_pitch_x",
            "dir2": dir2, "count2": "ground_rows", "spacing2": "ground_pitch_y", "name": "ground pattern"}),
    );
    // Single pads.
    for k in 1..=MODULE_SINGLES {
        let name = format!("single pad {k}");
        run(
            &mut s,
            "solid.box",
            json!({"corner": [format!("single_{k}_x - single_{k}_length / 2"),
                    format!("single_{k}_y - single_{k}_width / 2"), "0 mm"],
                "length": format!("single_{k}_length"), "width": format!("single_{k}_width"),
                "height": "pad_thickness", "name": name, "body_name": name}),
        );
    }
    // The shield can, its top edges rounded (picked at the saved size:
    // x ±8.3, y −11.2 … 6.2, top at 3.2 mm).
    run(
        &mut s,
        "solid.box",
        json!({"corner": ["can_x - can_width / 2", "can_y - can_length / 2", "pad_thickness + board_thickness"],
            "length": "can_width", "width": "can_length", "height": "can_height",
            "name": "shield can", "body_name": "shield can"}),
    );
    run(
        &mut s,
        "solid.fillet",
        json!({"name": "can round", "body": "shield can", "radius": "can_round",
            "edges": [[0.0, -11.2, 3.2], [8.3, -2.5, 3.2], [0.0, 6.2, 3.2], [-8.3, -2.5, 3.2]]}),
    );
    // The stamped emboss round the can's label area: a rectangular groove
    // `can_emboss_width` wide and `can_emboss_depth` deep, inset
    // `can_emboss_inset` from the can's sides (a ring of two boxes cut
    // from the can). The label area inside stays at the can's top.
    run(
        &mut s,
        "solid.box",
        json!({"corner": ["can_x - can_width / 2 + can_emboss_inset",
                "can_y - can_length / 2 + can_emboss_inset", "height - can_emboss_depth"],
            "length": "can_width - 2 * can_emboss_inset",
            "width": "can_length - 2 * can_emboss_inset", "height": "can_emboss_depth + 1 mm",
            "name": "can emboss", "body_name": "can emboss"}),
    );
    run(
        &mut s,
        "solid.box",
        json!({"corner": ["can_x - can_width / 2 + can_emboss_inset + can_emboss_width",
                "can_y - can_length / 2 + can_emboss_inset + can_emboss_width",
                "height - 2 * can_emboss_depth"],
            "length": "can_width - 2 * (can_emboss_inset + can_emboss_width)",
            "width": "can_length - 2 * (can_emboss_inset + can_emboss_width)",
            "height": "2 * can_emboss_depth + 2 mm", "operation": "cut", "targets": ["can emboss"],
            "name": "can emboss inside"}),
    );
    run(
        &mut s,
        "solid.combine",
        json!({"target": "shield can", "tools": ["can emboss"], "operation": "cut",
            "name": "can emboss groove"}),
    );
    // The antenna meander: head, turn copies, tail, joined.
    let antenna_sketch = |s: &mut Session, name: &str| {
        run(
            s,
            "sketch.create",
            json!({"plane": "XY", "offset": "pad_thickness + board_thickness", "name": name}),
        );
    };
    antenna_sketch(&mut s, "antenna head sketch");
    {
        let x = |e: &str| format!("ant_x0 {e}");
        let (x_mh, x_ph, x_r, x_hs) =
            (x("- ant_h"), x("+ ant_h"), x("+ ant_r"), x("+ 0.5 * ant_s"));
        let (y1r, y1ph, y1mh) = ("ant_y1 - ant_r", "ant_y1 + ant_h", "ant_y1 - ant_h");
        sketch_outline(
            &mut s,
            "h",
            [&x_mh, "ant_y0"],
            &[
                Step::Line([&x_mh, y1r]),
                Step::ClockwiseArc {
                    centre: [&x_r, y1r],
                    to: [&x_r, y1ph],
                },
                Step::Line([&x_hs, y1ph]),
                Step::Line([&x_hs, y1mh]),
                Step::Line([&x_ph, y1mh]),
                Step::Line([&x_ph, "ant_y0"]),
                Step::Line([&x_mh, "ant_y0"]),
            ],
        );
    }
    run(&mut s, "sketch.finish", json!({}));
    run(
        &mut s,
        "solid.extrude",
        json!({"sketch": "antenna head sketch", "distance": "antenna_thickness", "operation": "new",
            "name": "antenna head", "body_name": "antenna"}),
    );
    // One turn from the middle of a top run to the middle of the next
    // (`unit`), or from the middle of the last top run to the end of the
    // last up leg (tail).
    let turn = |s: &mut Session, tag: &str, base: &str, tail: bool| {
        let x = |e: &str| format!("{base} {e}");
        let xs = |a: &str| x(a);
        let p = [
            xs("+ 0.5 * ant_s"),       // 0
            xs("+ ant_s - ant_r"),     // 1
            xs("+ ant_s + ant_h"),     // 2
            xs("+ 2 * ant_s - ant_h"), // 3
            xs("+ 2 * ant_s + ant_r"), // 4
            xs("+ 2.5 * ant_s"),       // 5
            xs("+ 2 * ant_s + ant_h"), // 6
            xs("+ 2 * ant_s - ant_r"), // 7
            xs("+ ant_s + ant_r"),     // 8
            xs("+ ant_s - ant_h"),     // 9
        ];
        let (y1ph, y1mh, y1r) = ("ant_y1 + ant_h", "ant_y1 - ant_h", "ant_y1 - ant_r");
        let (ylph, ylmh, ylr) = ("ant_yl + ant_h", "ant_yl - ant_h", "ant_yl + ant_r");
        let mut steps = vec![
            Step::Line([&p[1], y1ph]),
            Step::ClockwiseArc {
                centre: [&p[1], y1r],
                to: [&p[2], y1r],
            },
            Step::Line([&p[2], ylph]),
            Step::Line([&p[3], ylph]),
        ];
        if tail {
            steps.push(Step::Line([&p[3], "ant_y1"]));
            steps.push(Step::Line([&p[6], "ant_y1"]));
        } else {
            steps.push(Step::Line([&p[3], y1r]));
            steps.push(Step::ClockwiseArc {
                centre: [&p[4], y1r],
                to: [&p[4], y1ph],
            });
            steps.push(Step::Line([&p[5], y1ph]));
            steps.push(Step::Line([&p[5], y1mh]));
            steps.push(Step::Line([&p[6], y1mh]));
        }
        steps.push(Step::Line([&p[6], ylr]));
        steps.push(Step::ClockwiseArc {
            centre: [&p[7], ylr],
            to: [&p[7], ylmh],
        });
        steps.push(Step::Line([&p[8], ylmh]));
        steps.push(Step::ClockwiseArc {
            centre: [&p[8], ylr],
            to: [&p[9], ylr],
        });
        steps.push(Step::Line([&p[9], y1mh]));
        steps.push(Step::Line([&p[0], y1mh]));
        steps.push(Step::Line([&p[0], y1ph]));
        sketch_outline(s, tag, [&p[0], y1ph], &steps);
    };
    antenna_sketch(&mut s, "antenna turn sketch");
    turn(&mut s, "u", "ant_ux", false);
    run(&mut s, "sketch.finish", json!({}));
    run(
        &mut s,
        "solid.extrude",
        json!({"sketch": "antenna turn sketch", "distance": "antenna_thickness", "operation": "join",
            "targets": ["antenna"], "name": "antenna turn"}),
    );
    run(
        &mut s,
        "solid.pattern.rectangular",
        json!({"features": ["antenna turn"], "dir1": [1, 0, 0], "count1": "antenna_units",
            "spacing1": "2 * antenna_step", "name": "antenna turns"}),
    );
    antenna_sketch(&mut s, "antenna tail sketch");
    turn(&mut s, "t", "ant_tx", true);
    run(&mut s, "sketch.finish", json!({}));
    run(
        &mut s,
        "solid.extrude",
        json!({"sketch": "antenna tail sketch", "distance": "antenna_thickness", "operation": "join",
            "targets": ["antenna"], "name": "antenna tail"}),
    );
    // Everything parked beyond the −x edge goes.
    run(
        &mut s,
        "solid.box",
        json!({"corner": ["park_x - 60 mm", "- length / 2 - 10 mm", "-1 mm"],
            "length": "120 mm", "width": "length + 20 mm", "height": "height + 2 mm",
            "operation": "cut", "name": "parked"}),
    );
    // Marking planes: the module form's anchors.
    use sparksmith_core::library::marking::surface::BoxFace;
    let board_top = "(pad_thickness + board_thickness)".to_owned();
    let top = format!("({board_top} + can_height)");
    let side_z = format!("({board_top} + {top}) / 2");
    let silk = format!("{board_top} + antenna_thickness");
    let side = |name: &'static str, face: BoxFace, origin: [String; 2], along: &str| {
        let (normal, u) = face.frame();
        Mark {
            name,
            origin: [origin[0].clone(), origin[1].clone(), side_z.clone()],
            u,
            normal,
            size: [along.into(), "can_height".into()],
        }
    };
    let (up, x) = BoxFace::Top.frame();
    let marks = vec![
        Mark {
            name: "top",
            origin: ["can_x".into(), "can_y".into(), top.clone()],
            u: x,
            normal: up,
            size: ["can_width".into(), "can_length".into()],
        },
        Mark {
            name: "can-top",
            origin: ["can_x".into(), "can_y".into(), top.clone()],
            u: x,
            normal: up,
            size: [
                "can_width - 2 * can_round".into(),
                "can_length - 2 * can_round".into(),
            ],
        },
        side(
            "can-pos-x",
            BoxFace::PosX,
            ["can_x + can_width / 2".into(), "can_y".into()],
            "can_length",
        ),
        side(
            "can-neg-x",
            BoxFace::NegX,
            ["can_x - can_width / 2".into(), "can_y".into()],
            "can_length",
        ),
        side(
            "can-pos-y",
            BoxFace::PosY,
            ["can_x".into(), "can_y + can_length / 2".into()],
            "can_width",
        ),
        side(
            "can-neg-y",
            BoxFace::NegY,
            ["can_x".into(), "can_y - can_length / 2".into()],
            "can_width",
        ),
        Mark {
            name: "pcb-top",
            origin: [
                "0 mm".into(),
                "length / 2 - mark_pcb_depth / 2".into(),
                silk.clone(),
            ],
            u: x,
            normal: up,
            size: ["width".into(), "mark_pcb_depth".into()],
        },
        Mark {
            name: "antenna-area",
            origin: [
                "0 mm".into(),
                "length / 2 - mark_antenna_depth / 2".into(),
                silk.clone(),
            ],
            u: x,
            normal: up,
            size: ["width".into(), "mark_antenna_depth".into()],
        },
    ];
    let mut extra = Vec::new();
    for m in &marks {
        let stem = m.name.replace('-', "_");
        extra.push(param(
            &format!("marking_{stem}_u"),
            &m.size[0],
            "mm",
            &format!("marking-{} size along its u axis", m.name),
        ));
        extra.push(param(
            &format!("marking_{stem}_v"),
            &m.size[1],
            "mm",
            &format!("marking-{} size across it", m.name),
        ));
    }
    import(&mut s, &extra);
    for m in &marks {
        let y = cross(m.normal, m.u);
        run(
            &mut s,
            "construct.plane.offset",
            json!({"base": {"origin": m.origin, "x_dir": m.u, "y_dir": y}, "offset": "0 mm",
                "name": format!("marking-{}", m.name)}),
        );
    }
    // Pin 1 follows its parameters through a sketch point on the seating
    // plane.
    run(
        &mut s,
        "sketch.create",
        json!({"plane": "XY", "offset": "0 mm", "name": "pin 1 sketch"}),
    );
    run(
        &mut s,
        "sketch.point",
        json!({"point": ["pin1_x", "pin1_y"], "id": "p1"}),
    );
    run(&mut s, "sketch.finish", json!({}));
    run(
        &mut s,
        "construct.point.vertex",
        json!({"point": {"sketch": "pin 1 sketch", "point": "p1"}, "name": "pin1"}),
    );
    // Looks, kept by body name: every name the patterns can give, at the
    // largest counts with nothing parked (the park moved off), then the
    // saved values again.
    let mut most = serde_json::Map::new();
    most.insert("park_x".into(), json!("1000 mm"));
    for k in 1..=MODULE_SIDES.len() {
        most.insert(format!("row_{k}_count"), json!(MODULE_MOST_ROW.to_string()));
        if castellated {
            most.insert(
                format!("notch_{k}_count"),
                json!(MODULE_MOST_ROW.to_string()),
            );
        }
    }
    most.insert("ground_rows".into(), json!(MODULE_MOST_GROUND.to_string()));
    most.insert(
        "ground_columns".into(),
        json!(MODULE_MOST_GROUND.to_string()),
    );
    run(&mut s, "parameters.change", json!({ "changes": most }));
    s.refresh();
    let names: Vec<String> = s
        .model
        .state()
        .bodies
        .iter()
        .map(|b| b.name.clone())
        .collect();
    let mut gold: Vec<&str> = Vec::new();
    for n in &names {
        match n.as_str() {
            "board" | "shield can" | "antenna" => {}
            other => gold.push(other),
        }
    }
    assert_eq!(
        gold.len(),
        (1 + usize::from(castellated)) * MODULE_SIDES.len() * MODULE_MOST_ROW
            + MODULE_MOST_GROUND * MODULE_MOST_GROUND
            + MODULE_SINGLES,
        "{names:?}"
    );
    assign_look(&mut s, &["board"], MaterialId::SolderMaskGreen);
    assign_look(&mut s, &["shield can"], MaterialId::NickelSilver);
    assign_look(&mut s, &["antenna"], MaterialId::SolderMaskOverCopper);
    for chunk in gold.chunks(200) {
        assign_look(&mut s, chunk, MaterialId::Gold);
    }
    let mut saved = serde_json::Map::new();
    saved.insert("park_x".into(), json!(mm(park)));
    for (i, _) in MODULE_SIDES.iter().enumerate() {
        let k = i + 1;
        let n = saved_rows.get(i).map_or(1, |r| r.0);
        saved.insert(format!("row_{k}_count"), json!(n.to_string()));
        if castellated {
            saved.insert(format!("notch_{k}_count"), json!(n.to_string()));
        }
    }
    saved.insert("ground_rows".into(), json!("3"));
    saved.insert("ground_columns".into(), json!("3"));
    run(&mut s, "parameters.change", json!({ "changes": saved }));
    s.refresh();
    let errors: Vec<String> = s
        .model
        .results
        .iter()
        .filter_map(|r| r.error.clone().map(|e| format!("{}: {e}", r.name)))
        .collect();
    assert!(errors.is_empty(), "{errors:?}");
    let left: Vec<String> = s
        .model
        .state()
        .bodies
        .iter()
        .map(|b| b.name.clone())
        .collect();
    // Board, 38 pads (and their plating), 9 ground pads, can, antenna.
    let plated = if castellated { 38 } else { 0 };
    assert_eq!(
        left.len(),
        1 + 38 + plated + 9 + 2,
        "bodies saved: {left:?}"
    );
    solidcraft_engine::io::write_design(&s.doc)
}

fn author_module_straight() -> Vec<u8> {
    author_module(false, false)
}

fn author_module_diagonal() -> Vec<u8> {
    author_module(true, false)
}

fn author_module_castellated() -> Vec<u8> {
    author_module(false, true)
}

fn author_module_castellated_diagonal() -> Vec<u8> {
    author_module(true, true)
}

/// Give `bodies` the Solidcraft appearance named after `material`, with
/// Sparksmith's opaque PBR projection of its look.
fn assign_look(s: &mut Session, bodies: &[&str], material: MaterialId) {
    let d = look::definition(material).unwrap();
    let l = look::saved(&d);
    let [r, g, b] = l.srgb8;
    run(
        s,
        "appearance.assign",
        json!({"bodies": bodies, "name": part_file::appearance_name(material),
            "color": format!("#{r:02x}{g:02x}{b:02x}"), "opacity": 1.0,
            "pbr": {"metallic": l.metallic, "roughness": l.roughness, "dielectric_ior": l.ior}}),
    );
}

/// A new design `title` with the user parameters `(name, expression,
/// unit, comment)` and the `marking-top` size parameters.
fn new_part(title: &str, params: &[(&str, &str, &str, &str)], mark: [&str; 2]) -> Session {
    let mut s = Session::default();
    run(&mut s, "file.new", json!({"name": title}));
    let mut rows: Vec<serde_json::Value> = params
        .iter()
        .map(|(name, expression, unit, comment)| {
            json!({"name": name, "expression": expression, "unit": unit, "comment": comment})
        })
        .collect();
    rows.push(
        json!({"name": "marking_top_u", "expression": mark[0], "unit": "mm",
        "comment": "marking-top size along its u axis"}),
    );
    rows.push(
        json!({"name": "marking_top_v", "expression": mark[1], "unit": "mm",
        "comment": "marking-top size across it"}),
    );
    run(
        &mut s,
        "parameters.import",
        json!({"text": serde_json::to_string(&rows).unwrap()}),
    );
    s
}

/// The `marking-top` plane at the top centre (`top` an expression), u
/// along +x, normal +z (its size parameters already given).
fn marking_plane(s: &mut Session, top: &str) {
    run(
        s,
        "construct.plane.offset",
        json!({"base": {"origin": ["0 mm", "0 mm", top], "x_dir": [1, 0, 0], "y_dir": [0, 1, 0]},
            "offset": "0 mm", "name": "marking-top"}),
    );
}

/// The `marking-top` plane at the top centre (`top` an expression), u
/// along +x, normal +z; then the design's bytes.
fn finish_part(mut s: Session, top: &str) -> Vec<u8> {
    marking_plane(&mut s, top);
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

/// A tube standing on z = 0: a cylinder of diameter `outer` with a
/// through bore of diameter `inner`, `height` tall.
fn tube(s: &mut Session, body: &str, outer: &str, inner: &str) {
    run(
        s,
        "solid.cylinder",
        json!({"diameter": outer, "height": "height", "name": "Body", "body_name": body}),
    );
    run(
        s,
        "solid.cylinder",
        json!({"diameter": inner, "height": "height + 2 mm", "base": ["0 mm", "0 mm", "-1 mm"],
            "operation": "cut", "targets": [body], "name": "Bore"}),
    );
}

/// An SMD nut: a tin-plated tube soldered to the board by its base.
/// Generic illustrative dimensions.
pub fn author_smd_nut() -> Vec<u8> {
    let mut s = new_part(
        "SMD nut",
        &[
            ("thread_d", "3 mm", "mm", "thread (bore) diameter"),
            ("outer_d", "5 mm", "mm", "outer diameter"),
            ("height", "3 mm", "mm", "height above the seating plane"),
        ],
        ["outer_d * 0.6", "outer_d * 0.6"],
    );
    tube(&mut s, "Nut", "outer_d", "thread_d");
    assign_look(&mut s, &["Nut"], MaterialId::TinMatte);
    finish_part(s, "height")
}

/// A round spacer: a plastic tube. Generic illustrative dimensions.
pub fn author_round_spacer() -> Vec<u8> {
    let mut s = new_part(
        "Round spacer",
        &[
            ("inner_d", "3.2 mm", "mm", "bore diameter"),
            ("outer_d", "6 mm", "mm", "outer diameter"),
            ("height", "5 mm", "mm", "height above the seating plane"),
        ],
        ["outer_d * 0.6", "outer_d * 0.6"],
    );
    tube(&mut s, "Spacer", "outer_d", "inner_d");
    assign_look(&mut s, &["Spacer"], MaterialId::Pa9t);
    finish_part(s, "height")
}

/// A finned heatsink: a base plate and `fin_count` fins along x, evenly
/// spaced across the width with the outer fins flush with the sides. The
/// first fin joins the base, and a rectangular pattern whose count is the
/// `fin_count` parameter replays it, so the part stays one body at any
/// count. Generic illustrative dimensions.
pub fn author_finned_heatsink() -> Vec<u8> {
    let mut s = new_part(
        "Finned heatsink",
        &[
            (
                "length",
                "20 mm",
                "mm",
                "size along x (the fins' direction)",
            ),
            ("width", "20 mm", "mm", "size along y"),
            (
                "height",
                "10 mm",
                "mm",
                "overall height above the seating plane",
            ),
            ("base_height", "2 mm", "mm", "base plate thickness"),
            ("fin_count", "6", "", "number of fins (at least 2)"),
            ("fin_thickness", "1.2 mm", "mm", "thickness of one fin"),
        ],
        ["length", "width"],
    );
    run(
        &mut s,
        "solid.box",
        json!({"corner": ["-length / 2", "-width / 2", "0 mm"], "length": "length",
            "width": "width", "height": "base_height", "name": "Base", "body_name": "Heatsink"}),
    );
    run(
        &mut s,
        "solid.box",
        json!({"corner": ["-length / 2", "-width / 2", "base_height"], "length": "length",
            "width": "fin_thickness", "height": "height - base_height", "operation": "join",
            "targets": ["Heatsink"], "name": "Fin"}),
    );
    run(
        &mut s,
        "solid.pattern.rectangular",
        json!({"features": ["Fin"], "dir1": [0, 1, 0], "count1": "fin_count",
            "spacing1": "(width - fin_thickness) / (fin_count - 1)", "name": "Fins"}),
    );
    assign_look(&mut s, &["Heatsink"], MaterialId::Aluminium);
    finish_part(s, "height")
}

/// The Omron B3U-1000P top-actuated tactile switch of the ESP32-S31 dev
/// board example: a black case, a metal cover, the round black plunger
/// and two tin-plated terminals, one each side along x. Dimensions from
/// Omron's public B3U catalog (en-b3u.pdf, p.2, Dimensions, B3U-1000P(M)):
/// case 3 x 2.5 mm, 1.2 mm to the cover, 1.6 mm over the 1.5 mm
/// plunger, 4 mm over the terminals, terminals 1.4 mm wide. The drawing
/// gives no cover or terminal thickness; those two are illustrative. No
/// CAD or EDA library data was copied.
pub fn author_b3u_1000p() -> Vec<u8> {
    let mut s = new_part(
        "Omron B3U-1000P tactile switch",
        &[
            ("length", "3 mm", "mm", "case size along x"),
            ("width", "2.5 mm", "mm", "case size along y"),
            (
                "body_height",
                "1.2 mm",
                "mm",
                "seating plane to the cover top",
            ),
            ("height", "1.6 mm", "mm", "seating plane to the plunger top"),
            ("plunger_d", "1.5 mm", "mm", "plunger diameter"),
            (
                "terminal_span",
                "4 mm",
                "mm",
                "size along x over the terminals",
            ),
            ("terminal_width", "1.4 mm", "mm", "terminal size along y"),
            (
                "cover_thickness",
                "0.1 mm",
                "mm",
                "metal cover thickness (illustrative)",
            ),
            (
                "terminal_thickness",
                "0.15 mm",
                "mm",
                "terminal thickness (illustrative)",
            ),
        ],
        ["length * 0.6", "width * 0.6"],
    );
    run(
        &mut s,
        "solid.box",
        json!({"corner": ["-length / 2", "-width / 2", "0 mm"], "length": "length",
            "width": "width", "height": "body_height - cover_thickness", "name": "Case",
            "body_name": "Case"}),
    );
    run(
        &mut s,
        "solid.box",
        json!({"corner": ["-length / 2", "-width / 2", "body_height - cover_thickness"],
            "length": "length", "width": "width", "height": "cover_thickness", "name": "Cover",
            "body_name": "Cover"}),
    );
    run(
        &mut s,
        "solid.cylinder",
        json!({"diameter": "plunger_d", "height": "height - body_height",
            "base": ["0 mm", "0 mm", "body_height"], "name": "Plunger", "body_name": "Plunger"}),
    );
    for (name, x) in [
        ("Terminal 1", "length / 2"),
        ("Terminal 2", "-terminal_span / 2"),
    ] {
        run(
            &mut s,
            "solid.box",
            json!({"corner": [x, "-terminal_width / 2", "0 mm"],
                "length": "(terminal_span - length) / 2", "width": "terminal_width",
                "height": "terminal_thickness", "name": name, "body_name": name}),
        );
    }
    assign_look(&mut s, &["Case"], MaterialId::Lcp);
    assign_look(&mut s, &["Cover"], MaterialId::NickelSilver);
    assign_look(&mut s, &["Plunger"], MaterialId::Pa9t);
    assign_look(&mut s, &["Terminal 1", "Terminal 2"], MaterialId::TinMatte);
    finish_part(s, "body_height")
}

/// In the active sketch: a stadium (a slot with fully rounded ends), its
/// straight sides along the sketch's u axis, centred on its v axis at `v`,
/// `length` overall and `high` across (expressions over the design's
/// parameters; `at` their values now). Every point is placed by
/// expressions, so the profile follows the parameters. `tag` keeps the
/// point ids of two stadiums in one sketch apart.
fn stadium(s: &mut Session, tag: &str, [length, high, v]: [&str; 3], at: [f64; 3]) {
    let [l, h, v0] = at;
    let r = format!("(({high}) / 2)");
    let flat = format!("(({length}) / 2 - {r})");
    let (f, rr) = (l / 2.0 - h / 2.0, h / 2.0);
    let points = [
        (
            "bl",
            format!("-{flat}"),
            format!("({v}) - {r}"),
            [-f, v0 - rr],
        ),
        ("br", flat.clone(), format!("({v}) - {r}"), [f, v0 - rr]),
        ("tr", flat.clone(), format!("({v}) + {r}"), [f, v0 + rr]),
        (
            "tl",
            format!("-{flat}"),
            format!("({v}) + {r}"),
            [-f, v0 + rr],
        ),
        ("cr", flat.clone(), format!("({v})"), [f, v0]),
        ("cl", format!("-{flat}"), format!("({v})"), [-f, v0]),
    ];
    for (id, x, y, _) in &points {
        run(
            s,
            "sketch.point",
            json!({"point": [x, y], "id": format!("{tag}{id}")}),
        );
    }
    let p = |id: &str| format!("{tag}{id}");
    run(
        s,
        "sketch.line",
        json!({"points": [p("bl"), p("br")], "infer": false}),
    );
    run(
        s,
        "sketch.arc.center_point",
        json!({"center": p("cr"), "start": p("br"), "end": p("tr")}),
    );
    run(
        s,
        "sketch.line",
        json!({"points": [p("tr"), p("tl")], "infer": false}),
    );
    run(
        s,
        "sketch.arc.center_point",
        json!({"center": p("cl"), "start": p("tl"), "end": p("bl")}),
    );
}

/// The GCT USB4105-GF-A USB-C receptacle of the ESP32-S31 dev board
/// example, from GCT's public USB4105 drawing (rev B4, sheet 1):
///
/// - **Shell**: a nickel stadium tube, open at the mating face (−y), from a
///   sketch of its outer and inner outline extruded over the depth. The
///   outline is 8.94 × 7.35 × 3.31 mm and the 8.34 × 2.56 mm opening is
///   centred 1.73 mm up; the 0.30 mm sheet and the outer stadium (3.16 mm
///   high, from 0.15 mm up) follow from those. Its four stakes stand in the
///   plane of the side walls, joined to it: the front pair centred 2.60 mm
///   from the mating face and 1.10 mm long, the rear pair 4.18 mm behind.
///   Only the part above the seating plane is drawn (a part file's lowest
///   geometry is the seating plane), from z = 0 up to the opening's centre,
///   where they meet the rounded walls.
/// - **Insulator** (LCP) closing the shell's rear behind the cavity, and the
///   **Tongue** (LCP), a plate with rounded corners centred in the opening.
/// - Twelve gold solder **tails** in the rear row, at the drawing's x
///   positions and widths (0.20 mm signal, 0.45 mm shared VBUS and GND).
/// - The **`opening` datum** (slice B2): a construction plane on the
///   mating face at the opening's centre, its normal out along the plug's
///   way in, sized by `opening_u` × `opening_v` with round ends
///   (`opening_r`) and `opening_depth` deep: the plug's shell, which fills
///   the 8.34 × 2.56 mm opening, out to the drawing's mating view's
///   "1.85 Min" where the plug's overmold begins.
///
/// Origin at the outline centre, as the footprint's. The shell's front
/// face (the extrusion's start face, `F2:start` of the Shell body) is what
/// the board holds flush to its edge. Illustrative, as their parameters
/// say: the cavity depth, the tongue, the rear stakes' length and the
/// tails' thickness, reach and run under the shell. No CAD or EDA library
/// data was copied.
pub fn author_usb4105() -> Vec<u8> {
    let mut s = new_part(
        "GCT USB4105-GF-A USB-C receptacle",
        &[
            ("width", "8.94 mm", "mm", "shell across (x)"),
            (
                "depth",
                "7.35 mm",
                "mm",
                "shell deep (y), mating face at -y",
            ),
            ("height", "3.31 mm", "mm", "seated height"),
            ("opening_w", "8.34 mm", "mm", "mating opening across"),
            ("opening_h", "2.56 mm", "mm", "mating opening high"),
            ("opening_z", "1.73 mm", "mm", "mating opening centre height"),
            (
                "shell_t",
                "(width - opening_w) / 2",
                "mm",
                "shell sheet thickness",
            ),
            (
                "cavity_depth",
                "6.2 mm",
                "mm",
                "how deep the opening reaches (illustrative)",
            ),
            ("tongue_w", "6.7 mm", "mm", "tongue across (illustrative)"),
            (
                "tongue_t",
                "0.7 mm",
                "mm",
                "tongue thickness (illustrative)",
            ),
            (
                "tongue_recess",
                "0.6 mm",
                "mm",
                "tongue tip behind the mating face (illustrative)",
            ),
            (
                "tongue_r",
                "0.3 mm",
                "mm",
                "tongue corner radius (illustrative)",
            ),
            (
                "stake_front_y",
                "2.6 mm",
                "mm",
                "front stake centre from the mating face",
            ),
            (
                "stake_pitch",
                "4.18 mm",
                "mm",
                "front to rear stake centres",
            ),
            ("stake_len", "1.1 mm", "mm", "front stake length along y"),
            (
                "rear_stake_len",
                "stake_len",
                "mm",
                "rear stake length along y (illustrative)",
            ),
            ("pitch", "0.5 mm", "mm", "signal tail pitch"),
            ("signal_tail_w", "0.2 mm", "mm", "signal tail width"),
            (
                "power_tail_w",
                "0.45 mm",
                "mm",
                "shared VBUS and GND tail width",
            ),
            ("vbus_x", "2.4 mm", "mm", "VBUS tail centres from x = 0"),
            ("gnd_x", "3.2 mm", "mm", "GND tail centres from x = 0"),
            ("tail_t", "0.15 mm", "mm", "tail thickness (illustrative)"),
            (
                "tail_reach",
                "0.38 mm",
                "mm",
                "how far the tails reach past the rear (illustrative)",
            ),
            (
                "tail_under",
                "0.6 mm",
                "mm",
                "how far the tails run under the shell (illustrative)",
            ),
            // The `opening` datum (slice B2): the aperture the mating
            // plug needs is the plug's shell, which fills the receptacle's
            // opening, out to where the plug's overmold begins.
            (
                "opening_u",
                "opening_w",
                "mm",
                "opening datum across: the plug's shell",
            ),
            (
                "opening_v",
                "opening_h",
                "mm",
                "opening datum high: the plug's shell",
            ),
            (
                "opening_r",
                "opening_h / 2",
                "mm",
                "opening datum corner radius: round ends",
            ),
            (
                "opening_depth",
                "1.85 mm",
                "mm",
                "opening datum depth: the plug's shell before its overmold (mating view, 1.85 min)",
            ),
        ],
        ["width * 0.6", "depth * 0.6"],
    );
    let (w, h, ow, oh, oz) = (8.94, 3.31, 8.34, 2.56, 1.73);
    let opening = ["opening_w", "opening_h", "-opening_z"];
    // The XZ plane maps sketch (u, v) to (u, offset, −v), normal +y: the
    // shell's sketch lies in the mating face and is extruded back.
    run(
        &mut s,
        "sketch.create",
        json!({"plane": "XZ", "offset": "-depth / 2", "name": "Shell profile"}),
    );
    stadium(
        &mut s,
        "o",
        ["width", "2 * (height - opening_z)", "-opening_z"],
        [w, 2.0 * (h - oz), -oz],
    );
    stadium(&mut s, "i", opening, [ow, oh, -oz]);
    run(&mut s, "sketch.finish", json!({}));
    run(
        &mut s,
        "solid.extrude",
        json!({"sketch": "Shell profile", "distance": "depth", "operation": "new",
            "name": "Shell", "body_name": "Shell"}),
    );
    // The stakes, in the side walls' plane, joined to the shell.
    let rear = "stake_front_y + stake_pitch";
    for (name, x, y, len) in [
        ("Stake 1", "-width / 2", "stake_front_y", "stake_len"),
        (
            "Stake 2",
            "width / 2 - shell_t",
            "stake_front_y",
            "stake_len",
        ),
        ("Stake 3", "-width / 2", rear, "rear_stake_len"),
        ("Stake 4", "width / 2 - shell_t", rear, "rear_stake_len"),
    ] {
        run(
            &mut s,
            "solid.box",
            json!({"corner": [x, format!("-depth / 2 + {y} - {len} / 2"), "0 mm"],
                "length": "shell_t", "width": len, "height": "opening_z",
                "operation": "join", "targets": ["Shell"], "name": name}),
        );
    }
    // The insulator closing the rear: the opening's stadium, behind the
    // cavity.
    run(
        &mut s,
        "sketch.create",
        json!({"plane": "XZ", "offset": "-depth / 2 + cavity_depth", "name": "Insulator profile"}),
    );
    stadium(&mut s, "i", opening, [ow, oh, -oz]);
    run(&mut s, "sketch.finish", json!({}));
    run(
        &mut s,
        "solid.extrude",
        json!({"sketch": "Insulator profile", "distance": "depth - cavity_depth",
            "operation": "new", "name": "Insulator", "body_name": "Insulator"}),
    );
    // The tongue: a rounded rectangle at its lower face, from its tip to
    // the insulator, extruded up its thickness.
    run(
        &mut s,
        "sketch.create",
        json!({"plane": "XY", "offset": "opening_z - tongue_t / 2", "name": "Tongue profile"}),
    );
    run(
        &mut s,
        "sketch.rectangle.rounded",
        json!({"p0": ["-tongue_w / 2", "-depth / 2 + tongue_recess"],
            "p1": ["tongue_w / 2", "-depth / 2 + cavity_depth"], "radius": "tongue_r"}),
    );
    run(&mut s, "sketch.finish", json!({}));
    run(
        &mut s,
        "solid.extrude",
        json!({"sketch": "Tongue profile", "distance": "tongue_t", "operation": "new",
            "name": "Tongue", "body_name": "Tongue"}),
    );
    // The solder tails in the drawing's x order (the footprint's lands).
    let mut centres: Vec<(String, &str)> = vec![
        ("-gnd_x".into(), "power_tail_w"),
        ("-vbus_x".into(), "power_tail_w"),
    ];
    for k in 0..8 {
        centres.push((format!("({k} - 3.5) * pitch"), "signal_tail_w"));
    }
    centres.push(("vbus_x".into(), "power_tail_w"));
    centres.push(("gnd_x".into(), "power_tail_w"));
    let mut tails = Vec::new();
    for ((x, tw), designation) in centres.iter().zip([
        "A1B12", "A4B9", "B8", "A5", "B7", "A6", "A7", "B6", "A8", "B5", "B4A9", "B1A12",
    ]) {
        let name = format!("Tail {designation}");
        run(
            &mut s,
            "solid.box",
            json!({"corner": [format!("{x} - {tw} / 2"), "depth / 2 - tail_under", "0 mm"],
                "length": tw, "width": "tail_under + tail_reach", "height": "tail_t",
                "name": name, "body_name": name}),
        );
        tails.push(name);
    }
    let tails: Vec<&str> = tails.iter().map(String::as_str).collect();
    assign_look(&mut s, &["Shell"], MaterialId::Nickel);
    assign_look(&mut s, &["Insulator", "Tongue"], MaterialId::Lcp);
    // The opening plane on the mating face at the opening's centre: x
    // across, y up, so its normal (x × y) is −y, out along the plug's way
    // in. Made last, so the bodies keep their feature ids: the example
    // board holds J1 by the Shell's front face, `F2:start`.
    run(
        &mut s,
        "construct.plane.offset",
        json!({"base": {"origin": ["0 mm", "-depth / 2", "opening_z"], "x_dir": [1, 0, 0],
            "y_dir": [0, 0, 1]}, "offset": "0 mm", "name": "opening"}),
    );
    assign_look(&mut s, &tails, MaterialId::Gold);
    finish_part(s, "height")
}

/// The shipped USB-C receptacle's body (`library/parts/gct-usb4105`, the
/// GCT USB4105 of its drawing, rev B4 sheet 1): the bodies, names and
/// looks the retired in-code USB-C builder made, by the same construction,
/// so a package that stored that builder's model keeps it. Origin at the
/// outline centre, mating face at −y, z = 0 the seating plane.
///
/// - **shell**: a nickel stadium tube over the depth, open at the mating
///   face, its inner outline the opening, its outer one the opening grown
///   by the sheet.
/// - **insulator** (LCP) closing the shell's rear, and the **tongue** (LCP),
///   a plate centred in the opening.
/// - **tail 1**–**tail 12** (gold), in the lands' x order, from under the
///   shell's rear onto their lands.
/// - **stake 1**–**stake 4** (nickel), in the side walls' plane at ±x, the
///   front and the rear pair, from below the board up to where the shell's
///   rounded underside meets their inner face.
/// - **peg 1**, **peg 2** (LCP): the locating pegs, up to the shell's
///   underside from the seating plane (a part file's lowest non-lead body
///   is its seating plane).
/// - The **`opening` datum**: a construction plane on the mating face at
///   the opening's centre, its normal out along the plug's way in, the
///   opening's size with round ends, `opening_depth` deep (the drawing's
///   mating view, "1.85 Min", where the plug's overmold begins).
///
/// The parameters are the drawing's values under the names a stored USB-C
/// form maps to; the tongue, insulator, tails' thickness and run under the
/// shell and the stakes' clearance in their slots are estimates, as their
/// comments say. No CAD or EDA library data was copied.
pub fn author_usb_c_receptacle() -> Vec<u8> {
    let mut s = Session::default();
    run(
        &mut s,
        "file.new",
        json!({"name": "USB-C receptacle, USB 2.0 (GCT USB4105)"}),
    );
    let rows: Vec<serde_json::Value> = [
        ("width", "8.94 mm", "shell across (x), over the stakes"),
        ("depth", "7.35 mm", "shell deep (y), mating face at -y"),
        ("height", "3.31 mm", "seated height"),
        ("opening_w", "8.34 mm", "mating opening across"),
        ("opening_h", "2.56 mm", "mating opening high"),
        ("opening_z", "1.73 mm", "mating opening centre height"),
        ("shell_t", "0.3 mm", "shell sheet thickness"),
        ("pitch", "0.5 mm", "signal land pitch"),
        ("land_y", "3.68 mm", "contact lands' centre line (y)"),
        ("land_length", "1.15 mm", "contact land length (y)"),
        ("signal_land_w", "0.3 mm", "signal land width"),
        ("power_land_w", "0.6 mm", "shared VBUS and GND land width"),
        ("vbus_x", "2.4 mm", "VBUS land centres from x = 0"),
        ("gnd_x", "3.2 mm", "GND land centres from x = 0"),
        ("front_y", "-1.075 mm", "front stake slot centre (y)"),
        ("front_slot", "1.4 mm", "front stake slot length (y)"),
        ("rear_y", "3.105 mm", "rear stake slot centre (y)"),
        ("rear_slot", "1.7 mm", "rear stake slot length (y)"),
        (
            "stake_length",
            "0.95 mm",
            "how far the stakes reach below the seating plane",
        ),
        ("peg_x", "2.89 mm", "locating peg centres from x = 0"),
        ("peg_y", "2.605 mm", "locating peg centre (y)"),
        ("peg_d", "0.5 mm", "locating peg diameter"),
        (
            "tongue_w",
            "opening_w * 0.8",
            "tongue across (estimate: about 6.7 mm in the 8.34 mm opening)",
        ),
        (
            "tongue_t",
            "opening_h * 0.27",
            "tongue thickness (estimate: about 0.7 mm)",
        ),
        (
            "tongue_recess",
            "0.6 mm",
            "tongue tip behind the mating face (estimate)",
        ),
        (
            "insulator_depth",
            "2.5 mm",
            "insulator closing the shell's rear (estimate)",
        ),
        ("tail_t", "0.15 mm", "solder tail thickness (estimate)"),
        (
            "tail_under",
            "0.6 mm",
            "how far the tails run under the shell's rear (estimate)",
        ),
        (
            "signal_tail_w",
            "signal_land_w * 2 / 3",
            "signal tail width: the drawing's 0.20 mm on its 0.30 mm land",
        ),
        (
            "power_tail_w",
            "power_land_w * 0.75",
            "shared tail width: the drawing's 0.45 mm on its 0.60 mm land",
        ),
        (
            "tail_end",
            "land_y + land_length * 0.4",
            "where the tails end, short of their lands' outer end",
        ),
        (
            "stake_clearance",
            "0.3 mm",
            "how much shorter a stake is than its slot (estimate: 1.10 mm in the 1.40 mm slot)",
        ),
        (
            "shell_r",
            "opening_h / 2 + shell_t",
            "outer radius of the shell's round ends",
        ),
        (
            "stake_beyond",
            "min(max(width / 2 - shell_t - (opening_w / 2 + shell_t - shell_r), 0 mm), shell_r)",
            "how far a stake's inner face lies beyond the start of the shell's round end",
        ),
        (
            "stake_top",
            "opening_z - sqrt(shell_r * shell_r - stake_beyond * stake_beyond)",
            "where the shell's rounded underside meets the stakes",
        ),
        (
            "opening_u",
            "opening_w",
            "opening datum across: the plug's shell",
        ),
        (
            "opening_v",
            "opening_h",
            "opening datum high: the plug's shell",
        ),
        (
            "opening_r",
            "opening_h / 2",
            "opening datum corner radius: round ends",
        ),
        (
            "opening_depth",
            "1.85 mm",
            "opening datum depth: the plug's shell before its overmold (mating view, 1.85 min)",
        ),
    ]
    .iter()
    .map(|(name, expression, comment)| param(name, expression, "mm", comment))
    .collect();
    import(&mut s, &rows);
    let (ow, oh, oz, t) = (8.34, 2.56, 1.73, 0.3);
    let inner = ["opening_w", "opening_h", "-opening_z"];
    // The XZ plane maps sketch (u, v) to (u, offset, −v), normal +y.
    run(
        &mut s,
        "sketch.create",
        json!({"plane": "XZ", "offset": "-depth / 2", "name": "Shell profile"}),
    );
    stadium(
        &mut s,
        "o",
        [
            "opening_w + 2 * shell_t",
            "opening_h + 2 * shell_t",
            "-opening_z",
        ],
        [ow + 2.0 * t, oh + 2.0 * t, -oz],
    );
    stadium(&mut s, "i", inner, [ow, oh, -oz]);
    run(&mut s, "sketch.finish", json!({}));
    run(
        &mut s,
        "solid.extrude",
        json!({"sketch": "Shell profile", "distance": "depth", "operation": "new",
            "name": "Shell", "body_name": "shell"}),
    );
    run(
        &mut s,
        "sketch.create",
        json!({"plane": "XZ", "offset": "depth / 2 - insulator_depth", "name": "Insulator profile"}),
    );
    stadium(&mut s, "i", inner, [ow, oh, -oz]);
    run(&mut s, "sketch.finish", json!({}));
    run(
        &mut s,
        "solid.extrude",
        json!({"sketch": "Insulator profile", "distance": "insulator_depth",
            "operation": "new", "name": "Insulator", "body_name": "insulator"}),
    );
    run(
        &mut s,
        "solid.box",
        json!({"corner": ["-tongue_w / 2", "-depth / 2 + tongue_recess", "opening_z - tongue_t / 2"],
            "length": "tongue_w", "width": "depth - insulator_depth - tongue_recess",
            "height": "tongue_t", "name": "Tongue", "body_name": "tongue"}),
    );
    // The tails in the lands' x order.
    let mut centres: Vec<(String, &str)> = vec![
        ("-gnd_x".into(), "power_tail_w"),
        ("-vbus_x".into(), "power_tail_w"),
    ];
    for k in 0..8 {
        centres.push((format!("({k} - 3.5) * pitch"), "signal_tail_w"));
    }
    centres.push(("vbus_x".into(), "power_tail_w"));
    centres.push(("gnd_x".into(), "power_tail_w"));
    let mut tails = Vec::new();
    for (k, (x, w)) in centres.iter().enumerate() {
        let name = format!("tail {}", k + 1);
        run(
            &mut s,
            "solid.box",
            json!({"corner": [format!("{x} - {w} / 2"), "depth / 2 - tail_under", "0 mm"],
                "length": w, "width": "tail_end - (depth / 2 - tail_under)", "height": "tail_t",
                "name": format!("Tail {}", k + 1), "body_name": name}),
        );
        tails.push(name);
    }
    // The stakes: front −x, front +x, rear −x, rear +x.
    let mut stakes = Vec::new();
    for (k, (x, y, slot)) in [
        ("-width / 2", "front_y", "front_slot"),
        ("width / 2 - shell_t", "front_y", "front_slot"),
        ("-width / 2", "rear_y", "rear_slot"),
        ("width / 2 - shell_t", "rear_y", "rear_slot"),
    ]
    .into_iter()
    .enumerate()
    {
        let name = format!("stake {}", k + 1);
        run(
            &mut s,
            "solid.box",
            json!({"corner": [x, format!("{y} - ({slot} - stake_clearance) / 2"), "-stake_length"],
                "length": "shell_t", "width": format!("{slot} - stake_clearance"),
                "height": "stake_length + stake_top",
                "name": format!("Stake {}", k + 1), "body_name": name}),
        );
        stakes.push(name);
    }
    for (k, x) in ["-peg_x", "peg_x"].into_iter().enumerate() {
        run(
            &mut s,
            "solid.cylinder",
            json!({"diameter": "peg_d", "height": "opening_z - shell_r",
                "base": [x, "peg_y", "0 mm"],
                "name": format!("Peg {}", k + 1), "body_name": format!("peg {}", k + 1)}),
        );
    }
    let mut nickel: Vec<&str> = vec!["shell"];
    nickel.extend(stakes.iter().map(String::as_str));
    assign_look(&mut s, &nickel, MaterialId::Nickel);
    assign_look(
        &mut s,
        &["insulator", "tongue", "peg 1", "peg 2"],
        MaterialId::Lcp,
    );
    let tails: Vec<&str> = tails.iter().map(String::as_str).collect();
    assign_look(&mut s, &tails, MaterialId::Gold);
    // The opening plane on the mating face at the opening's centre: x
    // across, y up, so its normal (x × y) is −y, out along the plug's way in.
    run(
        &mut s,
        "construct.plane.offset",
        json!({"base": {"origin": ["0 mm", "-depth / 2", "opening_z"], "x_dir": [1, 0, 0],
            "y_dir": [0, 0, 1]}, "offset": "0 mm", "name": "opening"}),
    );
    assert_evaluates(&mut s);
    solidcraft_engine::io::write_design(&s.doc)
}

#[test]
#[ignore = "writes library/parts/*.solidcraft, the chip, header, gull-wing, no-lead, molded and tabbed generators' part files and the examples' part files"]
fn author_parts() {
    // The repository's shipped material library (the program loads it
    // through sparksmith-io at start-up).
    let materials =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library/materials");
    let library = sparksmith_core::materials::MaterialLibrary::load(|name| {
        std::fs::read_to_string(materials.join(name))
            .map_err(|e| sparksmith_core::Error::Invalid(e.to_string()))
    })
    .map_err(|e| e.to_string());
    let standards = sparksmith_core::library::standards::Context::fixture().unwrap();
    let context = std::sync::Arc::new(
        sparksmith_core::library::standards::Context::new((**standards.standards()).clone())
            .with_materials(library),
    );
    let _context = context.enter();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    type Author = fn() -> Vec<u8>;
    let parts: [(&str, &str, Author); 70] = [
        // The chip generator's bundle (`generators/chip/parts/`).
        (
            "generators/chip/parts",
            "chip-resistor",
            author_chip_resistor,
        ),
        (
            "generators/chip/parts",
            "chip-capacitor",
            author_chip_capacitor,
        ),
        ("generators/chip/parts", "chip-ferrite", author_chip_ferrite),
        ("generators/chip/parts", "chip-led", author_chip_led),
        // The header generator's bundle (`generators/header/parts/`).
        ("generators/header/parts", "pin-header", author_pin_header),
        // The gull-wing generator's bundle (`generators/gull-wing/parts/`).
        (
            "generators/gull-wing/parts",
            "gull-wing-two-sided",
            author_gull_wing_two_sided,
        ),
        (
            "generators/gull-wing/parts",
            "gull-wing-two-sided-bevel",
            author_gull_wing_two_sided_bevel,
        ),
        (
            "generators/gull-wing/parts",
            "gull-wing-four-sided",
            author_gull_wing_four_sided,
        ),
        // The no-lead generator's bundle (`generators/no-lead/parts/`).
        ("generators/no-lead/parts", "dfn", author_dfn),
        ("generators/no-lead/parts", "dfn-ep", author_dfn_ep),
        ("generators/no-lead/parts", "qfn", author_qfn),
        ("generators/no-lead/parts", "qfn-ep", author_qfn_ep),
        (
            "generators/no-lead/parts",
            "qfn-ep-wettable",
            author_qfn_ep_wettable,
        ),
        (
            "generators/no-lead/parts",
            "dfn-ep-wettable",
            author_dfn_ep_wettable,
        ),
        ("generators/no-lead/parts", "lga", author_lga),
        ("generators/no-lead/parts", "dfn-window", author_dfn_window),
        // The molded generator's bundle (`generators/molded/parts/`): one
        // file per terminal form, each with and without the band.
        ("generators/molded/parts", "molded-flat", author_molded_flat),
        (
            "generators/molded/parts",
            "molded-flat-band",
            author_molded_flat_band,
        ),
        ("generators/molded/parts", "molded-bent", author_molded_bent),
        (
            "generators/molded/parts",
            "molded-bent-band",
            author_molded_bent_band,
        ),
        (
            "generators/molded/parts",
            "molded-j-lead",
            author_molded_j_lead,
        ),
        (
            "generators/molded/parts",
            "molded-j-lead-band",
            author_molded_j_lead_band,
        ),
        ("generators/molded/parts", "melf", author_melf_plain),
        ("generators/molded/parts", "melf-band", author_melf_band),
        // The through-hole generator's bundle (`generators/tht/parts/`).
        ("generators/tht/parts", "tht-axial", tht::author_tht_axial),
        ("generators/tht/parts", "tht-radial", tht::author_tht_radial),
        (
            "generators/tht/parts",
            "tht-radial-formed",
            tht::author_tht_radial_formed,
        ),
        ("generators/tht/parts", "tht-dip", tht::author_tht_dip),
        ("generators/tht/parts", "tht-to", tht::author_tht_to),
        (
            "generators/tht/parts",
            "tht-to-formed",
            tht::author_tht_to_formed,
        ),
        // The grid generator's bundle (`generators/grid/parts/`): balls or
        // lands, each full or with cut-outs.
        ("generators/grid/parts", "grid-ball", author_grid_ball),
        (
            "generators/grid/parts",
            "grid-ball-voids",
            author_grid_ball_voids,
        ),
        ("generators/grid/parts", "grid-land", author_grid_land),
        (
            "generators/grid/parts",
            "grid-land-voids",
            author_grid_land_voids,
        ),
        // The tabbed generator's bundle (`generators/tabbed/parts/`).
        ("generators/tabbed/parts", "tabbed-rows", author_tabbed_rows),
        (
            "generators/tabbed/parts",
            "tabbed-tab-lead",
            author_tabbed_tab_lead,
        ),
        (
            "generators/tabbed/parts",
            "tabbed-flange",
            author_tabbed_flange,
        ),
        (
            "generators/tabbed/parts",
            "tabbed-plcc-led",
            author_plcc_led,
        ),
        // The module generator's bundle (`generators/module/parts/`): LGA
        // pads or castellated, the ground array along the axes or turned
        // 45°.
        ("generators/module/parts", "module", author_module_straight),
        (
            "generators/module/parts",
            "module-diagonal",
            author_module_diagonal,
        ),
        (
            "generators/module/parts",
            "module-castellated",
            author_module_castellated,
        ),
        (
            "generators/module/parts",
            "module-castellated-diagonal",
            author_module_castellated_diagonal,
        ),
        // The electromechanical families' bundles (`author_parts/families.rs`).
        ("generators/tactile/parts", "tactile-mini", || {
            families::author_tactile_mini(false)
        }),
        ("generators/tactile/parts", "tactile-mini-ground", || {
            families::author_tactile_mini(true)
        }),
        (
            "generators/tactile/parts",
            "tactile-square",
            families::author_tactile_square,
        ),
        ("generators/reed/parts", "reed-smd", families::author_reed),
        ("generators/buzzer/parts", "buzzer-smd", || {
            families::author_buzzer(false)
        }),
        ("generators/buzzer/parts", "buzzer-smd-label", || {
            families::author_buzzer(true)
        }),
        ("generators/electrolytic/parts", "electrolytic-smd", || {
            families::author_electrolytic(false)
        }),
        (
            "generators/electrolytic/parts",
            "electrolytic-smd-vent",
            || families::author_electrolytic(true),
        ),
        (
            "generators/power-inductor/parts",
            "inductor-molded",
            families::author_inductor_molded,
        ),
        (
            "generators/power-inductor/parts",
            "inductor-shielded",
            || families::author_inductor_shielded(false),
        ),
        (
            "generators/power-inductor/parts",
            "inductor-shielded-window",
            || families::author_inductor_shielded(true),
        ),
        ("generators/standoff/parts", "standoff", || {
            families::author_standoff(false)
        }),
        ("generators/standoff/parts", "standoff-spigot", || {
            families::author_standoff(true)
        }),
        // The wire-to-board generator's bundle
        // (`generators/wire-to-board/parts/`): side and top entry, each in
        // the three shroud colours.
        (
            "generators/wire-to-board/parts",
            "wtb-side-natural",
            connectors::author_wtb_side_natural,
        ),
        (
            "generators/wire-to-board/parts",
            "wtb-side-white",
            connectors::author_wtb_side_white,
        ),
        (
            "generators/wire-to-board/parts",
            "wtb-side-black",
            connectors::author_wtb_side_black,
        ),
        (
            "generators/wire-to-board/parts",
            "wtb-top-natural",
            connectors::author_wtb_top_natural,
        ),
        (
            "generators/wire-to-board/parts",
            "wtb-top-white",
            connectors::author_wtb_top_white,
        ),
        (
            "generators/wire-to-board/parts",
            "wtb-top-black",
            connectors::author_wtb_top_black,
        ),
        // The board-to-board generator's bundle
        // (`generators/board-to-board/parts/`): with and without alignment
        // pins.
        (
            "generators/board-to-board/parts",
            "b2b-dual-row",
            connectors::author_b2b_dual_row,
        ),
        (
            "generators/board-to-board/parts",
            "b2b-dual-row-pegs",
            connectors::author_b2b_dual_row_pegs,
        ),
        // The microSD generator's bundle (`generators/microsd/parts/`).
        (
            "generators/microsd/parts",
            "microsd-push-pull",
            connectors::author_microsd_push_pull,
        ),
        ("library/parts", "smd-nut", author_smd_nut),
        ("library/parts", "round-spacer", author_round_spacer),
        ("library/parts", "finned-heatsink", author_finned_heatsink),
        // The shipped USB-C receptacle component's body.
        ("library/parts", "gct-usb4105", author_usb_c_receptacle),
        // The ESP32 dev board example's switches: the example's own part
        // file, not a shipped one.
        (
            "examples/esp32-s31-devboard/parts",
            "b3u-1000p",
            author_b3u_1000p,
        ),
        // Its USB-C receptacle J1, held flush to the board edge by the
        // shell's front face.
        (
            "examples/esp32-s31-devboard/parts",
            "usb4105",
            author_usb4105,
        ),
    ];
    // `AUTHOR_ONLY=pin-header` (or `dfn,qfn`) writes those part files
    // only; a folder (`generators/molded/parts`) writes its files.
    let only = std::env::var("AUTHOR_ONLY").ok();
    for (folder, name, author) in parts {
        if only
            .as_deref()
            .is_some_and(|o| !o.split(',').any(|o| o == name || o == folder))
        {
            continue;
        }
        let dir = root.join(folder);
        std::fs::create_dir_all(&dir).unwrap();
        let bytes = author();
        let path = dir.join(format!("{name}.solidcraft"));
        std::fs::write(&path, &bytes).unwrap();
        let report = part_file::evaluate(&bytes, &BTreeMap::new())
            .unwrap()
            .report();
        eprintln!(
            "wrote {} ({} KiB): {} parameters, {} bodies, {} surfaces, {:.1} ms, findings {:?}",
            path.display(),
            bytes.len() / 1024,
            report.parameters.len(),
            report.bodies.len(),
            report.surfaces.len(),
            report.evaluate_ms,
            report.findings
        );
        for p in &report.parameters {
            eprintln!(
                "  {} = {} [{}] -> {:?}",
                p.name, p.expression, p.unit, p.value
            );
        }
        for b in &report.bodies {
            eprintln!("  body {}: {:?}", b.name, b.bounds_mm);
        }
        assert!(report.ok, "{name}: {:?}", report.findings);
    }
}
