//! Authors Sparksmith's built-in part files (docs/part-templates.md §11)
//! through Solidcraft's own commands and public API, and writes them under
//! `library/parts/`. Original work (MIT OR Apache-2.0).
//!
//! The ESP32-S31-WROOM-3 module body starts from the module builder's
//! construction of the example project's form (so its geometry is the
//! generated model's, measured equal by `part_file` parity tests), moved
//! into the root component of a new design with plain parameter names
//! (`can_height`, `pcb_thickness`, …) and plain body names. Appearances are
//! named after Sparksmith materials; the marking surfaces are construction
//! planes `marking-<name>` placed by expressions over those parameters,
//! sized by `marking_<name>_u` / `_v`; `pin1` is a construction point.
//!
//! The mechanical starter set (docs/mechanical-parts.md §M7: SMD nut,
//! round spacer, finned heatsink) is drawn directly from primitives and a
//! parametric pattern over plain parameters, with generic illustrative
//! dimensions.
//!
//! `cargo test -p sparksmith-solidcraft --test author_parts -- --ignored`
//! (`AUTHOR_ONLY=<name>` writes one part file only)
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::json;
use solidcraft_engine::Session;
use solidcraft_engine::doc::clone_expression;
use sparksmith_core::ids::PackageTerminalId;
use sparksmith_core::library::{
    GeneratedMaterials, PackageDefinition, PackageForm, PackageModel, PackageTerminal,
};
use sparksmith_core::materials::MaterialId;
use sparksmith_solidcraft::models::{self, look, part_file};
use sparksmith_solidcraft::native::Native;
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

const STEM: &str = "pkgT";

fn wroom3() -> PackageDefinition {
    // The fixture is the module datasheet's millimetres: the wire form.
    let form: PackageForm = serde_json::from_str::<
        sparksmith_core::library::package_form::PackageFormParams,
    >(include_str!("template_spike_wroom3.json"))
    .unwrap()
    .into();
    PackageDefinition {
        next_local: 100,
        terminals: (1..=99)
            .map(|i| PackageTerminal {
                id: PackageTerminalId(i),
                designation: i.to_string(),
            })
            .collect(),
        model: PackageModel::Generated {
            family_rev: 1,
            materials: GeneratedMaterials::default(),
        },
        form,
        ..PackageDefinition::default()
    }
}

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

/// The WROOM-3 module body as a part file's bytes.
pub fn author_wroom3() -> Vec<u8> {
    let package = wroom3();
    let PackageForm::Module(form) = &package.form else {
        panic!("a module form");
    };
    // 1. The module builder's construction in a scratch design.
    let mut native = Native::new("scratch").unwrap();
    let def = models::build_definition(&mut native, STEM, &package).unwrap();
    let src = native.session();
    let from = src
        .doc
        .components
        .iter()
        .find(|c| c.name == STEM)
        .map(|c| c.id)
        .unwrap();
    let plain = |n: &str| n.strip_prefix(&format!("{STEM} ")).unwrap_or(n).to_owned();
    let parameters: BTreeMap<String, String> = src
        .doc
        .param_defs()
        .into_iter()
        .map(|d| {
            let p = d
                .name
                .strip_prefix(&format!("{STEM}_"))
                .unwrap_or(&d.name)
                .to_owned();
            (d.name, p)
        })
        .collect();
    let mut bodies: BTreeMap<String, String> = src
        .world_state()
        .unwrap()
        .bodies
        .iter()
        .map(|b| (b.name.clone(), plain(&b.name)))
        .collect();
    let features: Vec<_> = src
        .doc
        .features
        .iter()
        .filter(|f| f.component == from)
        .cloned()
        .collect();
    for f in &features {
        for n in f.body_names.iter().chain(&f.participants) {
            bodies.entry(n.clone()).or_insert_with(|| plain(n));
        }
    }
    // 2. A new design: the parameters by their plain names, the features
    // in its root component.
    let mut part = Session::default();
    run(
        &mut part,
        "file.new",
        json!({"name": "ESP32-S31-WROOM-3 module"}),
    );
    let rows: Vec<serde_json::Value> = src
        .doc
        .params
        .iter()
        .filter(|p| !p.model && p.name.starts_with(&format!("{STEM}_")))
        .map(|p| {
            json!({"name": parameters[&p.name], "expression": clone_expression(&p.expr, &parameters),
                "unit": p.unit, "comment": p.comment})
        })
        .collect();
    run(
        &mut part,
        "parameters.import",
        json!({"text": serde_json::to_string(&rows).unwrap()}),
    );
    {
        let doc = part.doc_mut();
        let mut next = doc.next_id.max(1);
        let mut ids = BTreeMap::new();
        let mut names = BTreeMap::new();
        for f in &features {
            ids.insert(f.id, next);
            next += 1;
            names.insert(f.name.clone(), plain(&f.name));
        }
        doc.next_id = next;
        // Feature inputs and dimensions the builder named (not user
        // parameters) travel with their features.
        for p in src.doc.params.iter().filter(|p| p.model) {
            let mut q = p.clone();
            q.name = parameters[&p.name].clone();
            q.expr = clone_expression(&p.expr, &parameters);
            doc.params.push(q);
        }
        for f in &features {
            let mut g = f.clone();
            g.remap_clone(&ids, &names, &bodies, &parameters).unwrap();
            g.id = ids[&f.id];
            g.name = names[&f.name].clone();
            g.component = 0;
            doc.features.push(g);
        }
    }
    part.refresh();
    let errors: Vec<String> = part
        .model
        .results
        .iter()
        .filter_map(|r| r.error.clone().map(|e| format!("{}: {e}", r.name)))
        .collect();
    assert!(errors.is_empty(), "{errors:?}");
    // 3. Appearances named after Sparksmith materials.
    let mut by_material: BTreeMap<_, Vec<String>> = BTreeMap::new();
    for b in &def.bodies {
        by_material
            .entry(b.material)
            .or_default()
            .push(plain(&b.name));
    }
    for (material, names) in by_material {
        let d = look::definition(material).unwrap();
        let l = look::saved(&d);
        let [r, g, b] = l.srgb8;
        for chunk in names.chunks(200) {
            run(
                &mut part,
                "appearance.assign",
                json!({"bodies": chunk, "name": part_file::appearance_name(material),
                    "color": format!("#{r:02x}{g:02x}{b:02x}"), "opacity": 1.0,
                    "pbr": {"metallic": l.metallic, "roughness": l.roughness, "dielectric_ior": l.ior}}),
            );
        }
    }
    // 4. Marking planes over the part's own parameters, matching the
    // module form's anchors (`surface::anchors`) in place and size. The
    // builder draws the can's centre and the antenna from numbers, so those
    // stay numbers here too.
    use sparksmith_core::library::marking::surface::{BoxFace, MODULE_TRACE_MM};
    let mm = |v: f64| format!("({v} mm)");
    let [cx, cy] = form.can_centre();
    let board_top = "(pad_thickness + board_thickness)".to_owned();
    let top = format!("({board_top} + can_height)");
    let side_z = format!("({board_top} + {top}) / 2");
    let can_end = format!("({} + can_length / 2)", mm(cy));
    let silk = format!("{board_top} + {}", mm(MODULE_TRACE_MM));
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
    let antenna_from = form.length() / 2.0 - form.antenna_length().min(form.length());
    let marks = vec![
        Mark {
            name: "top",
            origin: [mm(cx), mm(cy), top.clone()],
            u: x,
            normal: up,
            size: ["can_width".into(), "can_length".into()],
        },
        Mark {
            name: "can-top",
            origin: [mm(cx), mm(cy), top.clone()],
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
            [format!("{} + can_width / 2", mm(cx)), mm(cy)],
            "can_length",
        ),
        side(
            "can-neg-x",
            BoxFace::NegX,
            [format!("{} - can_width / 2", mm(cx)), mm(cy)],
            "can_length",
        ),
        side(
            "can-pos-y",
            BoxFace::PosY,
            [mm(cx), format!("{} + can_length / 2", mm(cy))],
            "can_width",
        ),
        side(
            "can-neg-y",
            BoxFace::NegY,
            [mm(cx), format!("{} - can_length / 2", mm(cy))],
            "can_width",
        ),
        Mark {
            name: "pcb-top",
            origin: [
                "0 mm".into(),
                format!("({can_end} + length / 2) / 2"),
                silk.clone(),
            ],
            u: x,
            normal: up,
            size: ["width".into(), format!("length / 2 - {can_end}")],
        },
        Mark {
            name: "antenna-area",
            origin: [
                "0 mm".into(),
                format!("({} + length / 2) / 2", mm(antenna_from)),
                silk.clone(),
            ],
            u: x,
            normal: up,
            size: ["width".into(), format!("length / 2 - {}", mm(antenna_from))],
        },
    ];
    let mut extra = Vec::new();
    for m in &marks {
        let stem = m.name.replace('-', "_");
        extra.push(
            json!({"name": format!("marking_{stem}_u"), "expression": m.size[0], "unit": "mm",
            "comment": format!("marking-{} size along its u axis", m.name)}),
        );
        extra.push(
            json!({"name": format!("marking_{stem}_v"), "expression": m.size[1], "unit": "mm",
            "comment": format!("marking-{} size across it", m.name)}),
        );
    }
    run(
        &mut part,
        "parameters.import",
        json!({"text": serde_json::to_string(&extra).unwrap()}),
    );
    for m in &marks {
        let y = cross(m.normal, m.u);
        run(
            &mut part,
            "construct.plane.offset",
            json!({"base": {"origin": m.origin, "x_dir": m.u, "y_dir": y}, "offset": "0 mm",
                "name": format!("marking-{}", m.name)}),
        );
    }
    // 5. Pin 1: the first lead point, on the seating plane.
    let first = package.form.lead_points().first().copied().unwrap();
    run(
        &mut part,
        "construct.point.vertex",
        json!({"point": {"point": [first[0], first[1], 0.0]}, "name": "pin1"}),
    );
    solidcraft_engine::io::write_design(&part.doc)
}

/// A chip part file of the `chip` generator's bundle: the in-code chip
/// builder's construction of a 0603 chip of `kind` (its parameters and
/// body names are already plain in a standalone build), with each body's
/// appearance named after its Sparksmith material. The generator gives
/// every parameter a value, so the 0603 numbers are only the file's
/// defaults.
pub fn author_chip(kind: sparksmith_core::library::ChipKind, title: &str) -> Vec<u8> {
    use sparksmith_core::library::ChipForm;
    let form = PackageForm::Chip(Box::new(ChipForm {
        kind,
        length_nm: nm(1.6),
        width_nm: nm(0.8),
        height_nm: nm(if kind == sparksmith_core::library::ChipKind::Led {
            0.75
        } else {
            0.45
        }),
        termination_nm: nm(0.3),
    }));
    let package = PackageDefinition {
        next_local: 3,
        terminals: (1..=2)
            .map(|i| PackageTerminal {
                id: PackageTerminalId(i),
                designation: i.to_string(),
            })
            .collect(),
        model: PackageModel::Generated {
            family_rev: models::family_rev(&form).unwrap(),
            materials: GeneratedMaterials::default(),
        },
        form,
        ..PackageDefinition::default()
    };
    let mut native = Native::new(title).unwrap();
    let bodies = models::build_standalone(&mut native, &package).unwrap();
    let mut s = Session::default();
    *s.doc_mut() = (*native.session().doc).clone();
    s.refresh();
    let mut by_material: BTreeMap<MaterialId, Vec<&str>> = BTreeMap::new();
    for b in &bodies {
        by_material.entry(b.material).or_default().push(&b.name);
    }
    for (material, names) in by_material {
        assign_look(&mut s, &names, material);
    }
    s.refresh();
    solidcraft_engine::io::write_design(&s.doc)
}

fn author_chip_resistor() -> Vec<u8> {
    author_chip(
        sparksmith_core::library::ChipKind::Resistor,
        "Chip resistor",
    )
}

fn author_chip_capacitor() -> Vec<u8> {
    author_chip(
        sparksmith_core::library::ChipKind::Capacitor,
        "Chip capacitor",
    )
}

fn author_chip_ferrite() -> Vec<u8> {
    author_chip(
        sparksmith_core::library::ChipKind::Inductor,
        "Chip inductor or ferrite bead",
    )
}

fn author_chip_led() -> Vec<u8> {
    author_chip(sparksmith_core::library::ChipKind::Led, "Chip LED")
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
        format!("{from}")
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
/// along +x, normal +z; then the design's bytes.
fn finish_part(mut s: Session, top: &str) -> Vec<u8> {
    run(
        &mut s,
        "construct.plane.offset",
        json!({"base": {"origin": ["0 mm", "0 mm", top], "x_dir": [1, 0, 0], "y_dir": [0, 1, 0]},
            "offset": "0 mm", "name": "marking-top"}),
    );
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

#[test]
#[ignore = "writes library/parts/*.solidcraft, the chip, header and molded generators' part files and the examples' part files"]
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
    let parts: [(&str, &str, Author); 19] = [
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
        ("library/parts", "esp32-s31-wroom-3", author_wroom3),
        ("library/parts", "smd-nut", author_smd_nut),
        ("library/parts", "round-spacer", author_round_spacer),
        ("library/parts", "finned-heatsink", author_finned_heatsink),
        // The ESP32 dev board example's switches: the example's own part
        // file, not a built-in.
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
    // `AUTHOR_ONLY=pin-header` writes that one part file only;
    // `AUTHOR_ONLY=generators/molded/parts` the files of one folder.
    let only = std::env::var("AUTHOR_ONLY").ok();
    for (folder, name, author) in parts {
        if only.as_deref().is_some_and(|o| o != name && o != folder) {
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
