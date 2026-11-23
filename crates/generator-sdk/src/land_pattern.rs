//! An explicit land pattern: every generator's `land_pattern` parameter.
//!
//! A generator computes its lands from its package parameters. A
//! manufacturer's recommended land pattern, or the lands a board already
//! has (a KiCad import), may differ: other sizes per row, mixed pitches,
//! split exposed pads, paste arrays, custom outlines. The `land_pattern`
//! parameter gives those lands as data, one pad per item, each the ABI's
//! [`Pad`] in canonical JSON in the package frame (the body centred on the
//! origin, as the generator draws it). When it is not empty the
//! generation's pads are exactly these; the body, symbol, fields and
//! marking stay the generator's. The courtyard and silkscreen outline are
//! drawn again round the given lands.
//!
//! The SDK declares the parameter for every generator ([`declare`]) and
//! applies it around the generator's own `generate` ([`generate`]), so a
//! generator neither declares nor reads it.
use crate::Generator;
use crate::land::{self, Box2};
use alloc::boxed::Box;
use alloc::collections::BTreeSet;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use sparksmith_generator_abi::codec;
use sparksmith_generator_abi::v1::{
    BOARD_MASK_EXPANSION, Description, Fill, GENERATE_PART, Generation, Graphic, Opening, Pad,
    PadKind, Param, ParamType, ParamValue, Primitive, Problem, Request, limits,
};

/// The parameter's key.
pub const KEY: &str = "land_pattern";

/// The `land_pattern` parameter: at most [`limits::PADS`] pads, none by
/// default (the generator's own lands).
pub fn param() -> Param {
    Param {
        key: KEY.into(),
        label: "Land pattern".into(),
        help: Some(
            "The lands as given (a manufacturer's land pattern, or a board's own lands), one pad per item, each the generator ABI's pad in JSON in the package frame: key, terminal (a package terminal, or empty for a non-plated hole or a mechanical or fiducial pad), shape, at, rot_mdeg, side, hole, mask, paste, number, kind. Empty: the generator's own lands."
                .into(),
        ),
        ty: ParamType::List {
            of: Box::new(ParamType::Text {
                max_chars: limits::TEXT_CHARS as u32,
            }),
            max_len: limits::PADS as u32,
        },
        default: ParamValue::List(Vec::new()),
        group: Some("lands".into()),
        context: None,
    }
}

/// `description` with the `land_pattern` parameter added to its part
/// capability (unless it declares one itself).
pub fn declare(mut description: Description) -> Description {
    if let Some(part) = description
        .capabilities
        .iter_mut()
        .find(|c| c.name == GENERATE_PART)
        && !part.params.iter().any(|p| p.key == KEY)
    {
        part.params.push(param());
    }
    description
}

fn problem(message: String) -> Problem {
    Problem {
        code: "param.land_pattern".into(),
        field: Some(KEY.into()),
        message,
    }
}

/// Run `G` for `request`: the `land_pattern` parameter is taken out of the
/// request (the generator's own description does not declare it), the
/// generator makes the part, and the given lands, if any, replace its
/// pads.
pub fn generate<G: Generator>(request: &Request) -> Result<Generation, Vec<Problem>> {
    let declared_by_generator = G::describe().params().iter().any(|p| p.key == KEY);
    if declared_by_generator {
        return G::generate(request);
    }
    let mut own = request.clone();
    let lands = match own.params.remove(KEY) {
        None => Vec::new(),
        Some(ParamValue::List(items)) => items,
        Some(_) => {
            return Err(alloc::vec![problem(
                "The land pattern is not a list.".into()
            )]);
        }
    };
    if lands.is_empty() {
        return G::generate(&own);
    }
    let Some(ParamValue::Int(mask_expansion)) = request.params.get(BOARD_MASK_EXPANSION) else {
        return Err(alloc::vec![problem(
            "Supply the board mask expansion in nanometres for the given lands.".into()
        )]);
    };
    let mut generation = G::generate_for_given_lands(&own)?;
    apply(&lands, *mask_expansion, &mut generation).map_err(|p| alloc::vec![p])?;
    Ok(generation)
}

/// Read one item: a pad in canonical JSON.
fn read(item: &ParamValue, n: usize) -> Result<Pad, Problem> {
    let ParamValue::Text(text) = item else {
        return Err(problem(format!("Land {} is not text.", n + 1)));
    };
    codec::read(text.as_bytes(), limits::TEXT_CHARS * 4)
        .map_err(|e| problem(format!("Land {} is not a pad: {e}.", n + 1)))
}

/// Whether `pad` may land no terminal: a non-plated hole, or copper that
/// is declared mechanical or a fiducial.
fn free(pad: &Pad) -> bool {
    pad.kind.is_some() || pad.hole.as_ref().is_some_and(|h| !h.plated)
}

/// Replace `generation`'s pads with `items` and draw the courtyard and the
/// silkscreen outline again round them.
pub fn apply(
    items: &[ParamValue],
    mask_expansion: i64,
    generation: &mut Generation,
) -> Result<(), Problem> {
    let pads = items
        .iter()
        .enumerate()
        .map(|(n, item)| read(item, n))
        .collect::<Result<Vec<_>, _>>()?;
    let terminals: BTreeSet<&str> = generation
        .package
        .terminals
        .iter()
        .map(String::as_str)
        .collect();
    let mut keys = BTreeSet::new();
    let mut landed = BTreeSet::new();
    for pad in &pads {
        if !keys.insert(pad.key.as_str()) {
            return Err(problem(format!(
                "The land pattern has two pads keyed \"{}\".",
                pad.key
            )));
        }
        if pad.terminal.is_empty() {
            if !free(pad) {
                return Err(problem(format!(
                    "Land \"{}\" lands no terminal; only a non-plated hole or a mechanical or fiducial pad may.",
                    pad.key
                )));
            }
        } else if !terminals.contains(pad.terminal.as_str()) {
            return Err(problem(format!(
                "Land \"{}\" lands terminal \"{}\", which this package does not have.",
                pad.key, pad.terminal
            )));
        } else if pad.kind == Some(PadKind::Fiducial) {
            return Err(problem(format!(
                "Land \"{}\" is a fiducial, which lands no terminal.",
                pad.key
            )));
        } else {
            landed.insert(pad.terminal.as_str());
        }
        if matches!(pad.mask, Opening::Windows(_)) {
            return Err(problem(format!(
                "Land \"{}\" gives its solder mask windows; only paste has windows.",
                pad.key
            )));
        }
    }
    if let Some(missing) = terminals.iter().find(|t| !landed.contains(**t)) {
        return Err(problem(format!(
            "The land pattern has no land for package terminal \"{missing}\"."
        )));
    }
    let [length, width, _] = generation.package.geometry.envelope_nm;
    let body = Box2::centred([0, 0], [length, width]);
    let footprint = &mut generation.footprint;
    if let Some(courtyard) = land::courtyard(&pads, Some(body)) {
        footprint.courtyard = courtyard;
    }
    // The outline broken round the new lands; the generator's other
    // silkscreen kept where all of its ink (stroke and solid fill) clears
    // every land's effective mask opening by the silkscreen clearance.
    let keep_outs: Vec<Box2> = pads
        .iter()
        .filter_map(|p| land::opening_box(p, mask_expansion))
        .map(|opening| opening.inflated(land::SILK_PAD_CLEARANCE_NM))
        .collect();
    let mut silk: Vec<_> = footprint
        .silk
        .iter()
        .filter(|g| !g.key.starts_with("outline-"))
        .filter(|g| ink_clear(g, &keep_outs))
        .cloned()
        .collect();
    silk.extend(land::silk_outline(&pads, body, mask_expansion));
    footprint.silk = silk;
    footprint.pads = pads;
    generation.notes.push(format!(
        "The land pattern is given: {} lands.",
        footprint.pads.len()
    ));
    Ok(())
}

/// Whether all of `g`'s ink keeps out of every box: its stroke (half its
/// width round its path) and, for a solid closed shape, its inside. A
/// point or end test misses ink that reaches across a box. Integer
/// arithmetic only (a module whose results are hashed may not use floating
/// point): coordinates are doubled so half the stroke is whole, and
/// distances are compared squared.
fn ink_clear(g: &Graphic, boxes: &[Box2]) -> bool {
    let half = i128::from(g.stroke_nm.max(0));
    let solid = g.fill != Fill::None;
    let p = |q: &[i64; 2]| [2 * i128::from(q[0]), 2 * i128::from(q[1])];
    let boxes: Vec<[I2; 2]> = boxes.iter().map(|b| [p(&b.min), p(&b.max)]).collect();
    let path_clear = |points: &[I2], closed: bool| {
        let n = points.len();
        let segments = if closed { n } else { n.saturating_sub(1) };
        boxes.iter().all(|b| {
            let edges_clear =
                (0..segments).all(|i| segment_box_clear(points[i], points[(i + 1) % n], b, half));
            let inside = closed
                && solid
                && (points.first().is_some_and(|q| box_contains(b, *q))
                    || contains(points, [(b[0][0] + b[1][0]) / 2, (b[0][1] + b[1][1]) / 2]));
            edges_clear && !inside
        })
    };
    // A circle's ink (a disc, or a ring of the stroke) against each box:
    // clear outside it, or for a ring wholly inside its hole.
    let circle_clear = |c: I2, r: i128, solid: bool| {
        boxes.iter().all(|b| {
            let near = point_box_distance2(c, b);
            let far = corners(b)
                .iter()
                .map(|q| distance2(c, *q))
                .max()
                .unwrap_or(0);
            let (outer, inner) = (r + half, r - half);
            near > outer * outer || (!solid && inner > 0 && far < inner * inner)
        })
    };
    match &g.shape {
        Primitive::Line { a, b } => path_clear(&[p(a), p(b)], false),
        Primitive::Rect { min, max } => path_clear(
            &[p(min), p(&[max[0], min[1]]), p(max), p(&[min[0], max[1]])],
            true,
        ),
        Primitive::Polyline { points } => {
            path_clear(&points.iter().map(p).collect::<Vec<_>>(), false)
        }
        Primitive::Polygon { points } => {
            path_clear(&points.iter().map(p).collect::<Vec<_>>(), true)
        }
        Primitive::Circle { center, radius_nm } => {
            circle_clear(p(center), 2 * i128::from(*radius_nm), solid)
        }
        // Judged as its whole circle's ring: never kept over a land, at
        // worst dropped where only the rest of its circle would reach one.
        Primitive::Arc { center, start, .. } => {
            let c = p(center);
            let r2 = u128::try_from(distance2(c, p(start))).unwrap_or(0);
            // The radius rounded up, so the ring is never judged narrower.
            let mut r = r2.isqrt();
            if r * r < r2 {
                r += 1;
            }
            circle_clear(c, i128::try_from(r).unwrap_or(i128::MAX / 4), false)
        }
        Primitive::Text { .. } => true,
    }
}

/// A point in doubled nanometres.
type I2 = [i128; 2];

fn distance2(a: I2, b: I2) -> i128 {
    let (dx, dy) = (a[0] - b[0], a[1] - b[1]);
    dx * dx + dy * dy
}

fn corners(b: &[I2; 2]) -> [I2; 4] {
    [b[0], [b[1][0], b[0][1]], b[1], [b[0][0], b[1][1]]]
}

fn box_contains(b: &[I2; 2], q: I2) -> bool {
    (b[0][0]..=b[1][0]).contains(&q[0]) && (b[0][1]..=b[1][1]).contains(&q[1])
}

fn point_box_distance2(q: I2, b: &[I2; 2]) -> i128 {
    let dx = (b[0][0] - q[0]).max(q[0] - b[1][0]).max(0);
    let dy = (b[0][1] - q[1]).max(q[1] - b[1][1]).max(0);
    dx * dx + dy * dy
}

/// Whether `q` lies farther than `h` from segment `a`–`b`.
fn point_segment_clear(q: I2, a: I2, b: I2, h: i128) -> bool {
    let d = [b[0] - a[0], b[1] - a[1]];
    let len2 = d[0] * d[0] + d[1] * d[1];
    let t = (q[0] - a[0]) * d[0] + (q[1] - a[1]) * d[1];
    if t <= 0 || len2 == 0 {
        return distance2(q, a) > h * h;
    }
    if t >= len2 {
        return distance2(q, b) > h * h;
    }
    let cross = d[0] * (q[1] - a[1]) - d[1] * (q[0] - a[0]);
    // |cross| / |d| > h, squared; board coordinates keep these within i128.
    cross.saturating_mul(cross) > (h * h).saturating_mul(len2)
}

/// Whether segments `a`–`b` and `c`–`d` cross or touch.
fn segments_cross(a: I2, b: I2, c: I2, d: I2) -> bool {
    let side = |p: I2, q: I2, r: I2| {
        ((q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0])).signum()
    };
    side(c, d, a) * side(c, d, b) <= 0 && side(a, b, c) * side(a, b, d) <= 0
}

/// Whether segment `a`–`c` keeps farther than `h` from box `b`.
fn segment_box_clear(a: I2, c: I2, b: &[I2; 2], h: i128) -> bool {
    if box_contains(b, a) || box_contains(b, c) {
        return false;
    }
    let k = corners(b);
    if (0..4).any(|i| segments_cross(a, c, k[i], k[(i + 1) % 4])) {
        return false;
    }
    point_box_distance2(a, b) > h * h
        && point_box_distance2(c, b) > h * h
        && k.iter().all(|q| point_segment_clear(*q, a, c, h))
}

/// Whether `q` lies inside the closed polygon `points` (even-odd).
fn contains(points: &[I2], q: I2) -> bool {
    let n = points.len();
    let mut inside = false;
    for i in 0..n {
        let (a, b) = (points[i], points[(i + 1) % n]);
        if (a[1] > q[1]) != (b[1] > q[1]) {
            // q.x < a.x + (q.y − a.y)(b.x − a.x)/(b.y − a.y), multiplied out.
            let dy = b[1] - a[1];
            let lhs = (q[0] - a[0]) * dy;
            let rhs = (q[1] - a[1]) * (b[0] - a[0]);
            if (dy > 0 && lhs < rhs) || (dy < 0 && lhs > rhs) {
                inside = !inside;
            }
        }
    }
    inside
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use sparksmith_generator_abi::v1::{
        self, Capability, Effect, Footprint, Hole, HoleShape, Lead, Model, OutlineSegment, Package,
        PackageGeometry, PadShape, PartOffer, Scope, Side, Window,
    };

    /// A two-terminal fixture generator with its own two lands.
    struct Two;
    impl Generator for Two {
        fn describe() -> Description {
            Description {
                abi: v1::ABI,
                id: "two".into(),
                version: 1,
                title: "Two lands".into(),
                licence: "MIT OR Apache-2.0".into(),
                deterministic: true,
                capabilities: vec![Capability {
                    name: GENERATE_PART.into(),
                    effect: Effect::None,
                    title: None,
                    params: vec![land::mask_expansion_param(50_000)],
                    rules: Vec::new(),
                    presets: Vec::new(),
                    part: Some(PartOffer {
                        families: Vec::new(),
                        scopes: vec![Scope::Package],
                        grammar: Vec::new(),
                        instance_fields: Vec::new(),
                    }),
                }],
                needs: Vec::new(),
            }
        }
        fn generate(request: &Request) -> Result<Generation, Vec<Problem>> {
            crate::args::checked(&Self::describe(), &request.params)?;
            let land = |key: &str, x: i64| Pad {
                key: key.into(),
                terminal: key.into(),
                shape: PadShape::Rect {
                    size: [600_000, 900_000],
                },
                at: [x, 0],
                rot_mdeg: 0,
                side: Side::Top,
                hole: None,
                mask: Opening::Default,
                paste: Opening::Default,
                number: None,
                kind: None,
            };
            Ok(Generation {
                orderable: Vec::new(),
                component: None,
                symbol: None,
                package: Package {
                    terminals: vec!["1".into(), "2".into()],
                    geometry: PackageGeometry {
                        envelope_nm: [1_600_000, 800_000, 500_000],
                        seating_nm: 0,
                        leads: vec![
                            Lead {
                                key: "1".into(),
                                contact: [-700_000, 0, 0],
                            },
                            Lead {
                                key: "2".into(),
                                contact: [700_000, 0, 0],
                            },
                        ],
                        pin1: "1".into(),
                    },
                },
                footprint: Footprint {
                    bridges: Vec::new(),
                    pads: vec![land("1", -750_000), land("2", 750_000)],
                    courtyard: vec![[-1, -1], [1, -1], [1, 1], [-1, 1]],
                    silk: Vec::new(),
                    assembly: Vec::new(),
                    keepouts: Vec::new(),
                },
                binding: Vec::new(),
                model: Some(Model::PartFile {
                    asset: "ab".repeat(32),
                    values: Default::default(),
                }),
                fields: Vec::new(),
                notes: Vec::new(),
                marking: None,
            })
        }
    }

    fn json(pad: &Pad) -> ParamValue {
        ParamValue::Text(
            String::from_utf8(codec::encode(pad).unwrap_or_default()).unwrap_or_default(),
        )
    }

    /// A manufacturer's land: a custom outline with an arc, a paste
    /// array, an offset slot.
    fn given(key: &str, terminal: &str, x: i64) -> Pad {
        Pad {
            key: key.into(),
            terminal: terminal.into(),
            shape: PadShape::Outline {
                start: [-400_000, -500_000],
                segments: vec![
                    OutlineSegment {
                        to: [400_000, -500_000],
                        bulge_ppb: None,
                    },
                    OutlineSegment {
                        to: [400_000, 500_000],
                        bulge_ppb: Some(414_213_562),
                    },
                    OutlineSegment {
                        to: [-400_000, -500_000],
                        bulge_ppb: None,
                    },
                ],
            },
            at: [x, 120_000],
            rot_mdeg: 30_000,
            side: Side::Top,
            hole: Some(Hole {
                shape: HoleShape::Slot {
                    size: [600_000, 300_000],
                    rot_mdeg: Some(45_000),
                },
                plated: true,
                offset: Some([50_000, 0]),
            }),
            mask: Opening::Expansion(30_000),
            paste: Opening::Windows(vec![Window::Rect {
                min: [-100_000, -100_000],
                max: [100_000, 100_000],
                radius_nm: 20_000,
            }]),
            number: Some(String::from(terminal)),
            kind: None,
        }
    }

    fn request(lands: Vec<ParamValue>) -> Request {
        let mut params = v1::Params::new();
        params.insert(KEY.into(), ParamValue::List(lands));
        params.insert(BOARD_MASK_EXPANSION.into(), ParamValue::Int(50_000));
        Request {
            abi: v1::ABI,
            scope: Scope::Package,
            params,
        }
    }

    #[test]
    fn every_generator_declares_the_land_pattern_once() {
        let described = declare(Two::describe());
        assert_eq!(
            described.params().iter().filter(|p| p.key == KEY).count(),
            1
        );
        assert_eq!(declare(described.clone()), described);
    }

    #[test]
    fn given_lands_replace_the_generators_and_keep_every_detail() -> Result<(), Vec<Problem>> {
        // Without lands the generator's own pads stand.
        let own = generate::<Two>(&request(Vec::new()))?;
        assert_eq!(own.footprint.pads.len(), 2);
        let lands = vec![
            given("a", "1", -900_000),
            given("b", "2", 900_000),
            Pad {
                key: "hole".into(),
                terminal: String::new(),
                shape: PadShape::Circle {
                    diameter_nm: 500_000,
                },
                at: [0, 1_000_000],
                rot_mdeg: 0,
                side: Side::Top,
                hole: Some(Hole {
                    shape: HoleShape::Round {
                        diameter_nm: 500_000,
                    },
                    plated: false,
                    offset: None,
                }),
                mask: Opening::Default,
                paste: Opening::None,
                number: Some(String::new()),
                kind: None,
            },
        ];
        let made = generate::<Two>(&request(lands.iter().map(json).collect()))?;
        assert_eq!(made.footprint.pads, lands);
        // The courtyard encloses the given lands, not the generator's.
        let reach = made.footprint.courtyard.iter().map(|p| p[0].abs()).max();
        assert!(reach.is_some_and(|r| r >= 900_000 + 650_000), "{reach:?}");
        // Body, leads and model are the generator's.
        assert_eq!(made.package, own.package);
        assert_eq!(made.model, own.model);
        Ok(())
    }

    /// The generator's silkscreen is kept only where all of its ink clears
    /// the given lands' effective openings: a polarity dot whose centre is
    /// clear but whose disc reaches the opening, and a line whose ends are
    /// clear but which crosses a land, are left out. Land 1 (0.6 × 0.9 mm
    /// at x = −0.75 mm) opens 0.05 mm wider (the default mask rule) and
    /// keeps 0.1 mm of silkscreen clearance: no ink above y = 0.6 mm or
    /// right of x = −0.3 mm may reach it.
    #[test]
    fn silkscreen_is_kept_only_where_all_its_ink_clears_the_lands() -> Result<(), Problem> {
        let mut made = generate::<Two>(&request(Vec::new())).map_err(|mut p| p.remove(0))?;
        let graphic = |key: &str, shape: Primitive, fill: Fill| Graphic {
            key: key.into(),
            shape,
            stroke_nm: 150_000,
            fill,
        };
        let dot = |key: &str, y: i64| {
            graphic(
                key,
                Primitive::Circle {
                    center: [-750_000, y],
                    radius_nm: 200_000,
                },
                Fill::Solid,
            )
        };
        made.footprint.silk = vec![
            // Centre 0.15 mm above the keep-out; the disc and its stroke
            // reach 0.125 mm into it.
            dot("dot-over", 750_000),
            // 1 mm up: its ink reaches down to 0.725 mm, clear.
            dot("dot-clear", 1_000_000),
            // Ends far left and right of every land, crossing both.
            graphic(
                "line-across",
                Primitive::Line {
                    a: [-1_500_000, 0],
                    b: [1_500_000, 0],
                },
                Fill::None,
            ),
            graphic(
                "line-clear",
                Primitive::Line {
                    a: [-1_500_000, 800_000],
                    b: [1_500_000, 800_000],
                },
                Fill::None,
            ),
        ];
        let lands: Vec<ParamValue> = made.footprint.pads.iter().map(json).collect();
        apply(&lands, 50_000, &mut made)?;
        let kept: Vec<&str> = made
            .footprint
            .silk
            .iter()
            .map(|g| g.key.as_str())
            .filter(|k| !k.starts_with("outline-"))
            .collect();
        assert_eq!(kept, vec!["dot-clear", "line-clear"]);
        Ok(())
    }

    #[test]
    fn lands_that_do_not_fit_the_package_are_refused_in_plain_words() {
        let refusal = |lands: Vec<Pad>| {
            generate::<Two>(&request(lands.iter().map(json).collect()))
                .err()
                .and_then(|p| p.first().map(|p| p.message.clone()))
                .unwrap_or_default()
        };
        assert!(refusal(vec![given("a", "1", 0)]).contains("no land for package terminal \"2\""));
        assert!(
            refusal(vec![given("a", "1", 0), given("b", "3", 0)])
                .contains("terminal \"3\", which this package does not have")
        );
        assert!(
            refusal(vec![given("a", "1", 0), given("a", "2", 0)]).contains("two pads keyed \"a\"")
        );
        let mut copper = given("c", "", 0);
        copper.hole = None;
        assert!(
            refusal(vec![given("a", "1", 0), given("b", "2", 0), copper])
                .contains("only a non-plated hole or a mechanical or fiducial pad")
        );
        let mut masked = given("b", "2", 0);
        masked.mask = Opening::Windows(Vec::new());
        assert!(refusal(vec![given("a", "1", 0), masked]).contains("only paste has windows"));
        let broken = generate::<Two>(&request(vec![ParamValue::Text("{".into())]));
        assert!(broken.is_err());
    }
}
