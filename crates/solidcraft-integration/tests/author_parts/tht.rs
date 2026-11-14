//! The `tht` generator's part files (`generators/tht/parts/`): through-hole
//! axial, radial, DIP and TO bodies with their leads, the in-code
//! through-hole builder's construction (`models/tht.rs`, retired with these
//! files) authored by Solidcraft commands over plain parameters, so each
//! file follows its values. Original work (MIT OR Apache-2.0).
//!
//! Every file has the leads' parameters (`lead_pitch`, `lead_diameter`,
//! `lead_below`, `standoff`, `root`) and the body's; bent leads also
//! `bend_radius`. The seating plane z = 0 is the board's top face; leads
//! reach `lead_below` under it. A lead's root reaches `root` into its body,
//! which is cut round it, so leads and body stay disjoint. Bent and formed
//! leads are strips of uniform section whose side profile is the centreline
//! thickened and bent round each corner with the centreline radius
//! `bend_radius`, narrowed where a segment cannot hold its bend (at most
//! 45 % of the shorter segment, never below half the section), as the
//! builder's `bent_strip` does: every sketch point is an expression, and a
//! bend's radius and tangent length are parameters of the file
//! (`<lead>_r<k>`, `<lead>_t<k>`).
use super::{MaterialId, assign_look, run};
use serde_json::json;
use solidcraft_engine::Session;

/// An expression and its value now (mm, or a plain number), so the author
/// knows which way a bend turns and which way to draw an arc.
#[derive(Clone)]
struct E {
    s: String,
    v: f64,
}

impl E {
    fn new(s: &str, v: f64) -> Self {
        Self { s: s.into(), v }
    }
    fn num(v: f64) -> Self {
        Self {
            s: format!("{v}"),
            v,
        }
    }
    fn mm(v: f64) -> Self {
        Self {
            s: format!("{v} mm"),
            v,
        }
    }
    fn add(&self, o: &Self) -> Self {
        Self {
            s: format!("({} + {})", self.s, o.s),
            v: self.v + o.v,
        }
    }
    fn sub(&self, o: &Self) -> Self {
        Self {
            s: format!("({} - {})", self.s, o.s),
            v: self.v - o.v,
        }
    }
    fn mul(&self, o: &Self) -> Self {
        // A unit vector's exact components shorten the product; a zero
        // keeps the other factor's unit (a length stays a length).
        match (self.s.as_str(), o.s.as_str()) {
            (_, "1") => return self.clone(),
            (_, "-1") => return self.neg(),
            ("1", _) => return o.clone(),
            ("-1", _) => return o.neg(),
            ("0", _) | (_, "0") => {
                return Self {
                    s: format!("(0 * {} * {})", self.s, o.s),
                    v: 0.0,
                };
            }
            _ => {}
        }
        Self {
            s: format!("({} * {})", self.s, o.s),
            v: self.v * o.v,
        }
    }
    fn neg(&self) -> Self {
        Self {
            s: format!("(-{})", self.s),
            v: -self.v,
        }
    }
}

/// A point or direction of a lead's side profile, in the strip's own
/// plane.
type V = [E; 2];

fn vadd(a: &V, b: &V) -> V {
    [a[0].add(&b[0]), a[1].add(&b[1])]
}
fn vsub(a: &V, b: &V) -> V {
    [a[0].sub(&b[0]), a[1].sub(&b[1])]
}
fn vmul(a: &V, k: &E) -> V {
    [a[0].mul(k), a[1].mul(k)]
}
/// The left normal of a unit direction.
fn left(d: &V) -> V {
    [d[1].neg(), d[0].clone()]
}

/// A centreline segment's unit direction: exact when it runs along an
/// axis (its ends share an expression), else over its length (`len`).
fn unit(a: &V, b: &V, len: &E) -> V {
    let axis = |i: usize| {
        let d = b[i].v - a[i].v;
        E::num(if d > 0.0 { 1.0 } else { -1.0 })
    };
    if a[0].s == b[0].s {
        [E::num(0.0), axis(1)]
    } else if a[1].s == b[1].s {
        [axis(0), E::num(0.0)]
    } else {
        let d = vsub(b, a);
        [
            E {
                s: format!("({} / {})", d[0].s, len.s),
                v: d[0].v / len.v,
            },
            E {
                s: format!("({} / {})", d[1].s, len.s),
                v: d[1].v / len.v,
            },
        ]
    }
}

/// How a strip's plane maps onto a sketch plane of the design.
#[derive(Clone, Copy)]
enum On {
    /// (x, z) on the XZ plane, drawn at sketch (x, −z); normal +y.
    Xz,
    /// (y, z) on the YZ plane, drawn as is; normal +x.
    Yz,
}

impl On {
    fn plane(self) -> &'static str {
        match self {
            Self::Xz => "XZ",
            Self::Yz => "YZ",
        }
    }
    fn sketch(self, p: &V) -> V {
        match self {
            Self::Xz => [p[0].clone(), p[1].neg()],
            Self::Yz => p.clone(),
        }
    }
}

/// One lead's side profile: its centreline points, its thickness in the
/// plane (`h2` the expression of half of it) and its bend radius.
struct Strip<'a> {
    /// Names the sketch (`<tag> profile`) and the bend parameters.
    tag: &'a str,
    points: Vec<V>,
    half: E,
    radius: E,
    on: On,
}

/// The bend parameters of `strip` (added to the design as user
/// parameters, mm) and the closed outline's corner points and arcs, in
/// sketch coordinates: `(points, arcs)` where each arc is (start, end,
/// centre) indices into the points, the outline running through the points
/// in order.
fn outline(s: &mut Session, strip: &Strip<'_>) -> (Vec<V>, Vec<(usize, usize, V)>) {
    let pts = &strip.points;
    let n = pts.len();
    let lens: Vec<E> = (0..n - 1)
        .map(|i| {
            let (a, b) = (&pts[i], &pts[i + 1]);
            if a[0].s == b[0].s {
                let d = b[1].sub(&a[1]);
                E {
                    s: format!("abs({})", d.s),
                    v: d.v.abs(),
                }
            } else if a[1].s == b[1].s {
                let d = b[0].sub(&a[0]);
                E {
                    s: format!("abs({})", d.s),
                    v: d.v.abs(),
                }
            } else {
                let d = vsub(b, a);
                E {
                    s: format!("hypot({}, {})", d[0].s, d[1].s),
                    v: d[0].v.hypot(d[1].v),
                }
            }
        })
        .collect();
    let dirs: Vec<V> = (0..n - 1)
        .map(|i| unit(&pts[i], &pts[i + 1], &lens[i]))
        .collect();
    // Each bend: (from, to, centre) on the centreline.
    let mut bends: Vec<(V, V, V)> = Vec::new();
    let mut params = Vec::new();
    for k in 0..n - 2 {
        let (din, dout) = (&dirs[k], &dirs[k + 1]);
        let cross = din[0].v * dout[1].v - din[1].v * dout[0].v;
        let sign = if cross > 0.0 { 1.0 } else { -1.0 };
        // tan(|turn| / 2) = |cross| / (1 + dot); exactly 1 at a right
        // angle between two axis-aligned segments.
        let right_angle = din
            .iter()
            .chain(dout)
            .all(|c| c.s == "0" || c.s == "1" || c.s == "-1");
        let tan_half = if right_angle {
            E::num(1.0)
        } else {
            let c = din[0].mul(&dout[1]).sub(&din[1].mul(&dout[0]));
            let d = din[0].mul(&dout[0]).add(&din[1].mul(&dout[1]));
            E {
                s: format!("(abs({}) / (1 + {}))", c.s, d.s),
                v: c.v.abs() / (1.0 + d.v),
            }
        };
        let shorter = E {
            s: format!("min({}, {})", lens[k].s, lens[k + 1].s),
            v: lens[k].v.min(lens[k + 1].v),
        };
        let fit = E {
            s: format!("0.45 * {} / {}", shorter.s, tan_half.s),
            v: 0.45 * shorter.v / tan_half.v,
        };
        let floor = strip.half.v * 1.0001;
        let r_value = strip.radius.v.min(fit.v).max(floor);
        let r_name = format!("{}_r{}", strip.tag, k + 1);
        let t_name = format!("{}_t{}", strip.tag, k + 1);
        params.push(json!({"name": r_name, "unit": "mm",
            "expression": format!("max(min({}, {}), {} * 1.0001)", strip.radius.s, fit.s, strip.half.s),
            "comment": format!("{} bend {}: centreline radius", strip.tag, k + 1)}));
        let t = E::new(&t_name, r_value * tan_half.v);
        params.push(json!({"name": t_name, "unit": "mm",
            "expression": if right_angle { r_name.clone() } else { format!("{r_name} * {}", tan_half.s) },
            "comment": format!("{} bend {}: tangent length", strip.tag, k + 1)}));
        let r = E::new(&r_name, r_value);
        let corner = &pts[k + 1];
        let from = vsub(corner, &vmul(din, &t));
        let to = vadd(corner, &vmul(dout, &t));
        let centre = vadd(&from, &vmul(&left(din), &r.mul(&E::num(sign))));
        bends.push((from, to, centre));
    }
    run(
        s,
        "parameters.import",
        json!({"text": serde_json::to_string(&params).unwrap()}),
    );
    let h = &strip.half;
    let side = |p: &V, d: &V, sign: f64| vadd(p, &vmul(&left(d), &h.mul(&E::num(sign))));
    let mut points = vec![side(&pts[0], &dirs[0], 1.0)];
    let mut arcs = Vec::new();
    for (k, (from, to, centre)) in bends.iter().enumerate() {
        points.push(side(from, &dirs[k], 1.0));
        points.push(side(to, &dirs[k + 1], 1.0));
        arcs.push((points.len() - 2, points.len() - 1, centre.clone()));
    }
    points.push(side(&pts[n - 1], &dirs[n - 2], 1.0));
    points.push(side(&pts[n - 1], &dirs[n - 2], -1.0));
    for (k, (from, to, centre)) in bends.iter().enumerate().rev() {
        points.push(side(to, &dirs[k + 1], -1.0));
        points.push(side(from, &dirs[k], -1.0));
        arcs.push((points.len() - 2, points.len() - 1, centre.clone()));
    }
    points.push(side(&pts[0], &dirs[0], -1.0));
    let on = strip.on;
    let points: Vec<V> = points.iter().map(|p| on.sketch(p)).collect();
    let arcs = arcs
        .into_iter()
        .map(|(a, b, c)| (a, b, on.sketch(&c)))
        .collect();
    (points, arcs)
}

/// Draw `strip`'s outline in a new sketch `<tag> <what>` on its plane,
/// `offset` along the normal; returns the sketch's name.
fn strip_sketch(s: &mut Session, strip: &Strip<'_>, what: &str, offset: &str) -> String {
    let (points, arcs) = outline(s, strip);
    let name = format!("{} {what}", strip.tag);
    run(
        s,
        "sketch.create",
        json!({"plane": strip.on.plane(), "offset": offset, "name": name}),
    );
    let id = |i: usize| format!("p{i}");
    for (i, p) in points.iter().enumerate() {
        run(
            s,
            "sketch.point",
            json!({"point": [p[0].s, p[1].s], "id": id(i)}),
        );
    }
    let count = points.len();
    let mut i = 0;
    while i < count {
        let j = (i + 1) % count;
        if let Some((_, _, c)) = arcs.iter().find(|(a, _, _)| *a == i) {
            let cid = format!("c{i}");
            run(
                s,
                "sketch.point",
                json!({"point": [c[0].s, c[1].s], "id": cid}),
            );
            // Counter-clockwise from start to end in the sketch.
            let (a, b) = (&points[i], &points[j]);
            let cross =
                (a[0].v - c[0].v) * (b[1].v - c[1].v) - (a[1].v - c[1].v) * (b[0].v - c[0].v);
            let (start, end) = if cross > 0.0 { (i, j) } else { (j, i) };
            run(
                s,
                "sketch.arc.center_point",
                json!({"center": cid, "start": id(start), "end": id(end)}),
            );
        } else {
            run(
                s,
                "sketch.line",
                json!({"points": [id(i), id(j)], "infer": false}),
            );
        }
        i += 1;
    }
    run(s, "sketch.finish", json!({}));
    name
}

/// A lead strip `body`, `width` across its plane (centred on `offset`'s
/// side: the sketch at `offset`, extruded `width` along the normal).
fn strip_body(s: &mut Session, strip: &Strip<'_>, body: &str, offset: &str, width: &str) {
    let sketch = strip_sketch(s, strip, "profile", offset);
    run(
        s,
        "solid.extrude",
        json!({"sketch": sketch, "distance": width, "operation": "new",
            "name": body, "body_name": body}),
    );
}

/// The same strip cut out of `target` (the body round its root).
fn strip_cut(
    s: &mut Session,
    strip: &Strip<'_>,
    cut: &str,
    target: &str,
    offset: &str,
    width: &str,
) {
    let sketch = strip_sketch(s, strip, "root", offset);
    run(
        s,
        "solid.extrude",
        json!({"sketch": sketch, "distance": width, "operation": "cut",
            "targets": [target], "name": cut}),
    );
}

/// A new design `title` with the user parameters `(name, expression,
/// unit, comment)`.
fn design(title: &str, params: &[(&str, &str, &str, &str)]) -> Session {
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

/// The shared lead parameters, at `[pitch, diameter, below, standoff]`.
fn lead_params(
    values: [&'static str; 5],
) -> Vec<(&'static str, &'static str, &'static str, &'static str)> {
    let [pitch, d, below, standoff, root] = values;
    vec![
        ("lead_pitch", pitch, "mm", "lead pitch on the board"),
        (
            "lead_diameter",
            d,
            "mm",
            "lead diameter (a strip's width, a square lead's side)",
        ),
        (
            "lead_below",
            below,
            "mm",
            "lead length below the seating plane",
        ),
        (
            "standoff",
            standoff,
            "mm",
            "body underside above the seating plane",
        ),
        (
            "root",
            root,
            "mm",
            "how far each lead reaches into the body",
        ),
    ]
}

/// Cut `target` round `tools`, keeping them.
fn cut_round(s: &mut Session, target: &str, tools: &[&str]) {
    run(
        s,
        "solid.combine",
        json!({"target": target, "tools": tools, "operation": "cut", "keep_tools": true,
            "name": "lead roots"}),
    );
}

fn fillet(s: &mut Session, name: &str, body: &str, radius: &str, edges: &[[f64; 3]]) {
    run(
        s,
        "solid.fillet",
        json!({"name": name, "body": body, "radius": radius, "edges": edges}),
    );
}

/// The design's bytes, every feature evaluated without error.
fn finish(s: Session) -> Vec<u8> {
    let errors: Vec<String> = s
        .model
        .results
        .iter()
        .filter_map(|r| r.error.clone().map(|e| format!("{}: {e}", r.name)))
        .collect();
    assert!(errors.is_empty(), "{errors:?}");
    solidcraft_engine::io::write_design(&s.doc)
}

/// The rounds of a moulded block `[lo, hi]`: its four upright corner edges
/// and four top edges, picked at the saved size.
fn block_rounds(s: &mut Session, lo: [f64; 3], hi: [f64; 3]) {
    let z = (lo[2] + hi[2]) / 2.0;
    let mid = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0];
    fillet(
        s,
        "corner round",
        "body",
        "corner_round",
        &[
            [lo[0], lo[1], z],
            [hi[0], lo[1], z],
            [hi[0], hi[1], z],
            [lo[0], hi[1], z],
        ],
    );
    fillet(
        s,
        "edge round",
        "body",
        "edge_round",
        &[
            [mid[0], lo[1], hi[2]],
            [hi[0], mid[1], hi[2]],
            [mid[0], hi[1], hi[2]],
            [lo[0], mid[1], hi[2]],
        ],
    );
}

/// The body names of `s` other than `body`.
fn leads(s: &Session) -> Vec<String> {
    s.model
        .state()
        .bodies
        .iter()
        .map(|b| b.name.clone())
        .filter(|n| n != "body")
        .collect()
}

fn looks(s: &mut Session) {
    let leads = leads(s);
    let leads: Vec<&str> = leads.iter().map(String::as_str).collect();
    assign_look(s, &["body"], MaterialId::EpoxyMould);
    assign_look(s, &leads, MaterialId::TinMatte);
}

fn counts(s: &mut Session, name: &str, n: u32) {
    run(
        s,
        "parameters.change",
        json!({"changes": {name: n.to_string()}}),
    );
    s.refresh();
}

/// An axial part: a round body lying on its standoff along x, each lead
/// leaving an end on the body's axis, running out and bending down at the
/// lead pitch, a lead's section square across its diameter. Saved at a
/// 0.25 W resistor (Ø2.5 × 6.5 mm, leads 0.6 mm at 10.16 mm).
pub fn author_tht_axial() -> Vec<u8> {
    let (dia, length, pitch, d, below, root) = (2.5, 6.5, 10.16, 0.6, 3.0, 0.5);
    let mut params = lead_params(["10.16 mm", "0.6 mm", "3 mm", "0 mm", "0.5 mm"]);
    params.extend([
        ("body_diameter", "2.5 mm", "mm", "body diameter"),
        ("body_length", "6.5 mm", "mm", "body length along x"),
        (
            "bend_radius",
            "0.6 mm",
            "mm",
            "centreline radius of the lead bends",
        ),
        ("end_round", "0.3 mm", "mm", "round of the body's end edges"),
    ]);
    let mut s = design("Axial through-hole part", &params);
    // The body as the builder drew it: a circle on the YZ plane, extruded
    // along x.
    run(
        &mut s,
        "sketch.create",
        json!({"plane": "YZ", "offset": "- body_length / 2", "name": "body profile"}),
    );
    run(
        &mut s,
        "sketch.circle.center",
        json!({"center": ["0 mm", "standoff + body_diameter / 2"], "radius": "body_diameter / 2"}),
    );
    run(&mut s, "sketch.finish", json!({}));
    run(
        &mut s,
        "solid.extrude",
        json!({"sketch": "body profile", "distance": "body_length", "operation": "new",
            "name": "body", "body_name": "body"}),
    );
    let zc = E::new("(standoff + body_diameter / 2)", dia / 2.0);
    let inner = E::new("(body_length / 2 - root)", length / 2.0 - root);
    let half = E::new("(lead_pitch / 2)", pitch / 2.0);
    let low = E::new("(- lead_below)", -below);
    let mut names = Vec::new();
    for (k, sign) in [(1, -1.0), (2, 1.0)] {
        let side = |e: &E| if sign < 0.0 { e.neg() } else { e.clone() };
        let tag = format!("lead{k}");
        let strip = Strip {
            tag: &tag,
            points: vec![
                [side(&inner), zc.clone()],
                [side(&half), zc.clone()],
                [side(&half), low.clone()],
            ],
            half: E::new("(lead_diameter / 2)", d / 2.0),
            radius: E::new("bend_radius", d),
            on: On::Xz,
        };
        let name = format!("lead {k}");
        strip_body(
            &mut s,
            &strip,
            &name,
            "- lead_diameter / 2",
            "lead_diameter",
        );
        names.push(name);
    }
    let tools: Vec<&str> = names.iter().map(String::as_str).collect();
    cut_round(&mut s, "body", &tools);
    let top = dia;
    fillet(
        &mut s,
        "end round",
        "body",
        "end_round",
        &[[-length / 2.0, 0.0, top], [length / 2.0, 0.0, top]],
    );
    looks(&mut s);
    finish(s)
}

/// A round body `name` standing on the XY plane `offset` up: a circle at
/// `centre` of `radius`, extruded `height` (as the builder drew them, so
/// the body's bounds are its own).
fn disc(s: &mut Session, name: &str, offset: &str, centre: [&str; 2], radius: &str, height: &str) {
    let sketch = format!("{name} profile");
    run(
        s,
        "sketch.create",
        json!({"plane": "XY", "offset": offset, "name": sketch}),
    );
    run(
        s,
        "sketch.circle.center",
        json!({"center": centre, "radius": radius}),
    );
    run(s, "sketch.finish", json!({}));
    run(
        s,
        "solid.extrude",
        json!({"sketch": sketch, "distance": height, "operation": "new",
            "name": name, "body_name": name}),
    );
}

/// The can top's marking area, as the host's radial form offers it: a
/// rectangle of 0.8 × 0.6 of the top inside its round, centred.
const CAN_TOP: [(&str, &str, &str, &str); 2] = [
    (
        "marking_top_u",
        "0.8 * (body_diameter - 2 * top_round)",
        "mm",
        "marking-top size along its u axis",
    ),
    (
        "marking_top_v",
        "0.6 * (body_diameter - 2 * top_round)",
        "mm",
        "marking-top size across it",
    ),
];

/// The `marking-top` plane on the can top, u along +x, normal +z.
fn can_top(s: &mut Session) {
    run(
        s,
        "construct.plane.offset",
        json!({"base": {"origin": ["0 mm", "0 mm", "standoff + body_height"], "x_dir": [1, 0, 0],
            "y_dir": [0, 1, 0]}, "offset": "0 mm", "name": "marking-top"}),
    );
}

/// A radial part on straight round leads: a round body standing on its
/// standoff, each lead from `lead_below` under the board up `root` into
/// the body. Saved at Ø5 × 11 mm, leads 0.5 mm at 2 mm.
pub fn author_tht_radial() -> Vec<u8> {
    let (dia, height) = (5.0, 11.0);
    let mut params = lead_params(["2 mm", "0.5 mm", "3 mm", "0 mm", "0.5 mm"]);
    params.extend([
        ("body_diameter", "5 mm", "mm", "body diameter"),
        (
            "body_height",
            "11 mm",
            "mm",
            "body height above its standoff",
        ),
        ("top_round", "0.3 mm", "mm", "round of the body's top edge"),
    ]);
    params.extend(CAN_TOP);
    let mut s = design("Radial through-hole part", &params);
    disc(
        &mut s,
        "body",
        "standoff",
        ["0 mm", "0 mm"],
        "body_diameter / 2",
        "body_height",
    );
    for (k, x) in [(1, "- lead_pitch / 2"), (2, "lead_pitch / 2")] {
        let name = format!("lead {k}");
        disc(
            &mut s,
            &name,
            "- lead_below",
            [x, "0 mm"],
            "lead_diameter / 2",
            "lead_below + standoff + root",
        );
    }
    cut_round(&mut s, "body", &["lead 1", "lead 2"]);
    fillet(
        &mut s,
        "top round",
        "body",
        "top_round",
        &[[dia / 2.0, 0.0, height]],
    );
    can_top(&mut s);
    looks(&mut s);
    finish(s)
}

/// Formed square leads of a row of `n` (2 or 3): lead `i` leaves the body
/// at its pitch-at-the-body position, `root` inside it, runs down to three
/// quarters of the standoff, jogs out to its board position at a quarter
/// of it and enters the board; a middle lead runs straight. Returns the
/// lead names.
fn formed_row(s: &mut Session, n: usize, values: [f64; 5]) -> Vec<String> {
    let [pitch, body_pitch, d, below, standoff] = values;
    let root = 0.5;
    let top = E::new("(standoff + root)", standoff + root);
    let upper = E::new("(0.75 * standoff)", 0.75 * standoff);
    let lower = E::new("(0.25 * standoff)", 0.25 * standoff);
    let low = E::new("(- lead_below)", -below);
    let mut names = Vec::new();
    for i in 0..n {
        let k = i as f64 - (n as f64 - 1.0) / 2.0;
        let name = format!("lead {}", i + 1);
        if k == 0.0 {
            // Straight down the middle: a box of the lead's square section.
            run(
                s,
                "solid.box",
                json!({"corner": ["- lead_diameter / 2", "- lead_diameter / 2", "- lead_below"],
                    "length": "lead_diameter", "width": "lead_diameter",
                    "height": "lead_below + standoff + root", "name": name, "body_name": name}),
            );
        } else {
            let at = |p: &str, v: f64| E::new(&format!("({k} * {p})"), k * v);
            let (xb, xp) = (at("body_pitch", body_pitch), at("lead_pitch", pitch));
            let tag = format!("lead{}", i + 1);
            let strip = Strip {
                tag: &tag,
                points: vec![
                    [xb.clone(), top.clone()],
                    [xb, upper.clone()],
                    [xp.clone(), lower.clone()],
                    [xp, low.clone()],
                ],
                half: E::new("(lead_diameter / 2)", d / 2.0),
                radius: E::new("bend_radius", d),
                on: On::Xz,
            };
            strip_body(s, &strip, &name, "- lead_diameter / 2", "lead_diameter");
        }
        names.push(name);
    }
    names
}

/// A radial part on formed square leads: as [`author_tht_radial`], each
/// lead leaving the body at the pitch at the body and formed out to the
/// lead pitch within the standoff. Saved at Ø5 × 11 mm, leads 0.5 mm from
/// 2 mm at the body to 5 mm on the board, 1.5 mm standoff.
pub fn author_tht_radial_formed() -> Vec<u8> {
    let (dia, height) = (5.0, 11.0);
    let mut params = lead_params(["5 mm", "0.5 mm", "3 mm", "1.5 mm", "0.5 mm"]);
    params.extend([
        (
            "body_pitch",
            "2 mm",
            "mm",
            "lead pitch where the leads leave the body",
        ),
        (
            "bend_radius",
            "0.5 mm",
            "mm",
            "centreline radius of the lead bends",
        ),
        ("body_diameter", "5 mm", "mm", "body diameter"),
        (
            "body_height",
            "11 mm",
            "mm",
            "body height above its standoff",
        ),
        ("top_round", "0.3 mm", "mm", "round of the body's top edge"),
    ]);
    params.extend(CAN_TOP);
    let mut s = design("Radial through-hole part, formed leads", &params);
    disc(
        &mut s,
        "body",
        "standoff",
        ["0 mm", "0 mm"],
        "body_diameter / 2",
        "body_height",
    );
    let names = formed_row(&mut s, 2, [5.0, 2.0, 0.5, 3.0, 1.5]);
    let tools: Vec<&str> = names.iter().map(String::as_str).collect();
    cut_round(&mut s, "body", &tools);
    fillet(
        &mut s,
        "top round",
        "body",
        "top_round",
        &[[dia / 2.0, 0.0, 1.5 + height]],
    );
    can_top(&mut s);
    looks(&mut s);
    finish(s)
}

/// A dual in-line package: a moulded block on its standoff, strip leads
/// (`lead_thickness` thick, `lead_diameter` wide) leaving its long sides
/// at `lead_exit` and bending down at the rows `row_pitch` apart. Each row
/// is a body pattern of `per_row` strips at the lead pitch from the −x
/// end, and the block's cut round each row's roots a feature pattern of
/// one cut, so nothing names the pattern's bodies. Saved at a DIP-8 of
/// 300 mil (9.27 × 6.35 × 3.3 mm); its looks at 64 per row.
pub fn author_tht_dip() -> Vec<u8> {
    let (length, width, height, standoff) = (9.27, 6.35, 3.3, 0.5);
    let (row, d, t, below, exit, root) =
        (7.62, 0.46, 0.25, 3.3, 0.5 + 0.99, 0.5f64.min(width / 6.0));
    let mut params = vec![("per_row", "4", "", "leads in each row (x)")];
    params.extend(lead_params([
        "2.54 mm", "0.46 mm", "3.3 mm", "0.5 mm", "0.5 mm",
    ]));
    params.extend([
        ("row_pitch", "7.62 mm", "mm", "pitch between the rows (y)"),
        ("lead_thickness", "0.25 mm", "mm", "lead strip thickness"),
        (
            "lead_exit",
            "1.49 mm",
            "mm",
            "height at which the leads leave the body",
        ),
        (
            "bend_radius",
            "0.46 mm",
            "mm",
            "centreline radius of the lead bends",
        ),
        ("body_length", "9.27 mm", "mm", "body length (x)"),
        ("body_width", "6.35 mm", "mm", "body width (y)"),
        (
            "body_height",
            "3.3 mm",
            "mm",
            "body height above its standoff",
        ),
        (
            "corner_round",
            "0.3 mm",
            "mm",
            "round of the body's upright corners",
        ),
        (
            "edge_round",
            "0.2 mm",
            "mm",
            "round of the body's top edges",
        ),
    ]);
    let mut s = design("Dual in-line package", &params);
    run(
        &mut s,
        "solid.box",
        json!({"corner": ["- body_length / 2", "- body_width / 2", "standoff"],
            "length": "body_length", "width": "body_width", "height": "body_height",
            "name": "body", "body_name": "body"}),
    );
    let exit_z = E::new("lead_exit", exit);
    let inner = E::new("(body_width / 2 - root)", width / 2.0 - root);
    let half_row = E::new("(row_pitch / 2)", row / 2.0);
    let low = E::new("(- lead_below)", -below);
    let offset = "- (per_row - 1) / 2 * lead_pitch - lead_diameter / 2";
    for (part, sign) in [("row_s", -1.0), ("row_n", 1.0)] {
        let side = |e: &E| if sign < 0.0 { e.neg() } else { e.clone() };
        let strip = Strip {
            tag: part,
            points: vec![
                [side(&inner), exit_z.clone()],
                [side(&half_row), exit_z.clone()],
                [side(&half_row), low.clone()],
            ],
            half: E::new("(lead_thickness / 2)", t / 2.0),
            radius: E::new("bend_radius", d),
            on: On::Yz,
        };
        let lead = format!("{part} lead");
        strip_body(&mut s, &strip, &lead, offset, "lead_diameter");
        let cut = format!("{part} root");
        strip_cut(&mut s, &strip, &cut, "body", offset, "lead_diameter");
        let pattern = |what: serde_json::Value, name: String| {
            let mut p = json!({"dir1": [1, 0, 0], "count1": "per_row", "spacing1": "lead_pitch",
                "name": name});
            if let (Some(map), Some(extra)) = (p.as_object_mut(), what.as_object()) {
                map.extend(extra.clone());
            }
            p
        };
        run(
            &mut s,
            "solid.pattern.rectangular",
            pattern(json!({"features": [cut]}), format!("{part} roots")),
        );
        run(
            &mut s,
            "solid.pattern.rectangular",
            pattern(json!({"bodies": [lead]}), format!("{part} pattern")),
        );
    }
    block_rounds(
        &mut s,
        [-length / 2.0, -width / 2.0, standoff],
        [length / 2.0, width / 2.0, standoff + height],
    );
    // Looks are kept by body name: every lead the generator can ask for
    // (64 per row) gets the tin look at that count, then the file goes
    // back to four per row.
    counts(&mut s, "per_row", 64);
    assert_eq!(leads(&s).len(), 128);
    looks(&mut s);
    counts(&mut s, "per_row", 4);
    finish(s)
}

/// The TO block's parameters after the leads'.
fn to_block_params() -> [(&'static str, &'static str, &'static str, &'static str); 5] {
    [
        (
            "body_length",
            "10 mm",
            "mm",
            "body length along the row (x)",
        ),
        ("body_width", "4.5 mm", "mm", "body width (y)"),
        (
            "body_height",
            "15.5 mm",
            "mm",
            "body height above its standoff",
        ),
        (
            "corner_round",
            "0.3 mm",
            "mm",
            "round of the body's upright corners",
        ),
        (
            "edge_round",
            "0.2 mm",
            "mm",
            "round of the body's top edges",
        ),
    ]
}

fn to_block(s: &mut Session) {
    run(
        s,
        "solid.box",
        json!({"corner": ["- body_length / 2", "- body_width / 2", "standoff"],
            "length": "body_length", "width": "body_width", "height": "body_height",
            "name": "body", "body_name": "body"}),
    );
}

/// A single-row TO package on straight leads: a moulded block on its
/// standoff, square leads of `lead_diameter` straight down at the lead
/// pitch, `pins` of them from −x as a body pattern, the block's cut round
/// their roots a feature pattern of one cut. Saved at a TO-220-3 (10 ×
/// 4.5 × 15.5 mm, 0.8 mm leads at 2.54 mm, 1 mm standoff); its looks at 64
/// leads.
pub fn author_tht_to() -> Vec<u8> {
    let (length, width, height, standoff) = (10.0, 4.5, 15.5, 1.0);
    let mut params = vec![("pins", "3", "", "leads in the row (x)")];
    params.extend(lead_params([
        "2.54 mm", "0.8 mm", "3.5 mm", "1 mm", "0.5 mm",
    ]));
    params.extend(to_block_params());
    let mut s = design("TO package, straight leads", &params);
    to_block(&mut s);
    let corner = json!([
        "- (pins - 1) / 2 * lead_pitch - lead_diameter / 2",
        "- lead_diameter / 2",
        "- lead_below"
    ]);
    run(
        &mut s,
        "solid.box",
        json!({"corner": corner, "length": "lead_diameter", "width": "lead_diameter",
            "height": "lead_below + standoff + root", "name": "lead 1", "body_name": "lead 1"}),
    );
    run(
        &mut s,
        "solid.box",
        json!({"corner": corner, "length": "lead_diameter", "width": "lead_diameter",
            "height": "lead_below + standoff + root", "operation": "cut", "targets": ["body"],
            "name": "lead root"}),
    );
    for (what, name) in [
        (json!({"features": ["lead root"]}), "lead roots"),
        (json!({"bodies": ["lead 1"]}), "lead pattern"),
    ] {
        let mut p = json!({"dir1": [1, 0, 0], "count1": "pins", "spacing1": "lead_pitch",
            "name": name});
        if let (Some(map), Some(extra)) = (p.as_object_mut(), what.as_object()) {
            map.extend(extra.clone());
        }
        run(&mut s, "solid.pattern.rectangular", p);
    }
    block_rounds(
        &mut s,
        [-length / 2.0, -width / 2.0, standoff],
        [length / 2.0, width / 2.0, standoff + height],
    );
    counts(&mut s, "pins", 64);
    assert_eq!(leads(&s).len(), 64);
    looks(&mut s);
    counts(&mut s, "pins", 3);
    finish(s)
}

/// A three-lead TO package (TO-92) on formed leads: the block of
/// [`author_tht_to`], the outer leads formed out from the pitch at the
/// body to the lead pitch within the standoff and the middle one straight.
/// Saved at a formed TO-92 (4.8 × 3.8 × 4.8 mm, 0.45 mm leads from 1.27 mm
/// at the body to 2.54 mm, 1 mm standoff).
pub fn author_tht_to_formed() -> Vec<u8> {
    let (length, width, height, standoff) = (4.8, 3.8, 4.8, 1.0);
    let mut params = lead_params(["2.54 mm", "0.45 mm", "3 mm", "1 mm", "0.5 mm"]);
    params.extend([
        (
            "body_pitch",
            "1.27 mm",
            "mm",
            "lead pitch where the leads leave the body",
        ),
        (
            "bend_radius",
            "0.45 mm",
            "mm",
            "centreline radius of the lead bends",
        ),
        (
            "body_length",
            "4.8 mm",
            "mm",
            "body length along the row (x)",
        ),
        ("body_width", "3.8 mm", "mm", "body width (y)"),
        (
            "body_height",
            "4.8 mm",
            "mm",
            "body height above its standoff",
        ),
        (
            "corner_round",
            "0.3 mm",
            "mm",
            "round of the body's upright corners",
        ),
        (
            "edge_round",
            "0.2 mm",
            "mm",
            "round of the body's top edges",
        ),
    ]);
    let mut s = design("TO package, formed leads", &params);
    to_block(&mut s);
    let names = formed_row(&mut s, 3, [2.54, 1.27, 0.45, 3.0, standoff]);
    let tools: Vec<&str> = names.iter().map(String::as_str).collect();
    cut_round(&mut s, "body", &tools);
    block_rounds(
        &mut s,
        [-length / 2.0, -width / 2.0, standoff],
        [length / 2.0, width / 2.0, standoff + height],
    );
    looks(&mut s);
    finish(s)
}
