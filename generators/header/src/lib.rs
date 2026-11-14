//! The header generator: straight pin headers of rows × columns square
//! posts through a moulded base, as a symbol, package, footprint and 3D
//! model.
//!
//! Lands follow the host's estimate for through-hole posts (an original
//! rule of thumb, not a land-pattern standard): a round drill of the post's
//! diagonal + 0.1 mm, plated, and a land of the diagonal + 0.8 mm, kept
//! 0.25 mm of copper apart at the tighter pitch but never below a 0.2 mm
//! ring; pin 1 square, the others round. Pins are numbered across the rows
//! first: pin 1 at −x, −y, pin 2 beside it in the next row.
//!
//! The presets are the header starter table, in nm: typical proportions,
//! estimated, as the table says.
#![cfg_attr(target_arch = "wasm32", no_std)]

extern crate alloc;

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use sparksmith_generator_sdk::args::{Args, checked, problem};
use sparksmith_generator_sdk::draw::{self, Drawing, RIGHT};
use sparksmith_generator_sdk::land::{self, Box2};
use sparksmith_generator_sdk::units::{MM, UM, div_round};
use sparksmith_generator_sdk::v1::{
    self, BindingEntry, Capability, Component, Description, Direction, Effect, Fill, Footprint,
    Generation, Grammar, Graphic, Hole, HoleShape, Lead, Opening, Package, PackageGeometry, Pad,
    PadShape, Param, ParamType, ParamValue, PartOffer, Preset, Primitive, Problem, Request, Rule,
    Scope, Side, Symbol, SymbolPin, Terminal,
};
use sparksmith_generator_sdk::{Generator, export_generator};

/// The starter rows, in nm.
const PRESETS: &str = include_str!("../presets.json");

/// The generator's version (`Description::version`).
const VERSION: u32 = 1;

/// Most rows and pins per row.
const MAX_ROWS: i64 = 4;
const MAX_COLUMNS: i64 = 64;

/// The drill: the post's diagonal plus this.
const DRILL_EXTRA_NM: i64 = 100_000;
/// The land: the post's diagonal plus this…
const LAND_EXTRA_NM: i64 = 800_000;
/// …leaving this much copper between neighbouring lands…
const LAND_GAP_NM: i64 = 250_000;
/// …but never less ring than this on the drill (both sides together).
const MIN_RING_NM: i64 = 200_000;

/// The pin-1 dot: a filled silkscreen circle of this radius beyond pad 1's
/// −x edge, clear of its mask opening by the silkscreen clearance (as the
/// host's template marks pin 1 of a package of more than two terminals).
const PIN_ONE_DOT_RADIUS_NM: i64 = 200_000;

/// Rounds of the moulded base's upright corners and top edges (est.),
/// capped by the room between the outermost posts and the base's edge and
/// by the base's height.
const CORNER_ROUND_NM: i64 = 300_000;
const EDGE_ROUND_NM: i64 = 200_000;

/// Pin numbers on the symbol: a character advances 0.9 of the pin text
/// height (1 mm) and the stub keeps 0.4 mm clear at each end; a stub too
/// short for its number grows by whole modules (the host's symbol rule).
const PIN_NUMBER_ADVANCE_NM: i64 = 900_000;
const PIN_NUMBER_CLEARANCE_NM: i64 = 400_000;
const MAX_PIN_LENGTH_NM: i64 = 50 * MM;

pub struct Header;

export_generator!(Header);

fn length(key: &str, label: &str, min_nm: i64, max_nm: i64, default: i64, group: &str) -> Param {
    Param {
        key: key.into(),
        label: label.into(),
        help: None,
        ty: ParamType::Length {
            min_nm,
            max_nm,
            step_nm: None,
        },
        default: ParamValue::Int(default),
        group: Some(group.into()),
        context: None,
    }
}

fn count(key: &str, label: &str, max: i64, default: i64) -> Param {
    Param {
        key: key.into(),
        label: label.into(),
        help: None,
        ty: ParamType::Count { min: 1, max },
        default: ParamValue::Int(default),
        group: Some("pins".into()),
        context: None,
    }
}

/// The full description. Unreadable presets are a build bug that the
/// golden and preset tests catch; describe still answers, without them.
fn description() -> Description {
    description_with(sparksmith_generator_sdk::presets(PRESETS).unwrap_or_default())
}

/// The description, with `presets` (generate checks parameters against it
/// without reading the preset table).
fn description_with(presets: Vec<Preset>) -> Description {
    let params = vec![
        count("rows", "Rows", MAX_ROWS, 2),
        count("columns", "Pins per row", MAX_COLUMNS, 5),
        length("pitch", "Pitch along a row", 500 * UM, 10 * MM, 2_540_000, "pins"),
        length("row_pitch", "Row pitch", 500 * UM, 10 * MM, 2_540_000, "pins"),
        length("pin_side", "Pin side", 100 * UM, 2 * MM, 640_000, "pins"),
        length(
            "pin_above",
            "Pin length above the body",
            100 * UM,
            30 * MM,
            6 * MM,
            "pins",
        ),
        length(
            "pin_below",
            "Pin length below the seating plane",
            100 * UM,
            20 * MM,
            3 * MM,
            "pins",
        ),
        length(
            "body_height",
            "Body height",
            200 * UM,
            10 * MM,
            2_540_000,
            "body",
        ),
    ];
    let rules = vec![
        Rule {
            text: "Pin side < pitch, so pins along a row do not touch".into(),
            fields: vec!["pin_side".into(), "pitch".into()],
        },
        Rule {
            text: "Pin side < row pitch, so the pins fit the body width".into(),
            fields: vec!["pin_side".into(), "row_pitch".into()],
        },
    ];
    let offer = PartOffer {
        families: vec!["header".into()],
        scopes: vec![Scope::Component, Scope::Package],
        grammar: vec![Grammar {
            scope: Scope::Component,
            patterns: vec![
                "header {rows}x{columns} {pitch:length}?".into(),
                "pin header {rows}x{columns} {pitch:length}?".into(),
            ],
            examples: vec![
                "header 1x4".into(),
                "header 2x5 2mm".into(),
                "pin header 1×3".into(),
            ],
        }],
        instance_fields: Vec::new(),
    };
    Description {
        abi: v1::ABI,
        id: "header".into(),
        version: VERSION,
        title: "Pin headers".into(),
        licence: "MIT OR Apache-2.0".into(),
        deterministic: true,
        capabilities: vec![Capability {
            name: v1::GENERATE_PART.into(),
            effect: Effect::None,
            title: Some("Make a pin header".into()),
            params,
            rules,
            presets,
            part: Some(offer),
        }],
        needs: Vec::new(),
    }
}

/// A header's values, checked.
struct Dims {
    rows: i64,
    columns: i64,
    pitch: i64,
    row_pitch: i64,
    body_height: i64,
    pin_side: i64,
    pin_above: i64,
    pin_below: i64,
}

impl Dims {
    fn pins(&self) -> i64 {
        self.rows * self.columns
    }

    /// The moulded base: one pitch per pin long, one row pitch per row
    /// wide, centred.
    fn body(&self) -> Box2 {
        Box2::centred([0, 0], [self.columns * self.pitch, self.rows * self.row_pitch])
    }

    /// The centre of pin `k` (from 0, across the rows first).
    fn at(&self, k: i64) -> Point2 {
        let (column, row) = (k / self.rows, k % self.rows);
        [
            centred(column, self.columns, self.pitch),
            centred(row, self.rows, self.row_pitch),
        ]
    }
}

type Point2 = [i64; 2];

/// `(i − (n − 1) / 2) × pitch`, to the nearest nanometre.
fn centred(i: i64, n: i64, pitch: i64) -> i64 {
    div_round((2 * i - (n - 1)) * pitch, 2).unwrap_or(0)
}

fn dims(args: &Args<'_>) -> Result<Dims, Vec<Problem>> {
    let one = |p: Problem| vec![p];
    let d = Dims {
        rows: args.int("rows").map_err(one)?,
        columns: args.int("columns").map_err(one)?,
        pitch: args.int("pitch").map_err(one)?,
        row_pitch: args.int("row_pitch").map_err(one)?,
        body_height: args.int("body_height").map_err(one)?,
        pin_side: args.int("pin_side").map_err(one)?,
        pin_above: args.int("pin_above").map_err(one)?,
        pin_below: args.int("pin_below").map_err(one)?,
    };
    let mut problems = Vec::new();
    for (field, pitch, what) in [
        ("pitch", d.pitch, "pitch"),
        ("row_pitch", d.row_pitch, "row pitch"),
    ] {
        if d.pin_side >= pitch {
            problems.push(problem(
                "param.inconsistent",
                field,
                format!(
                    "Pins of {} mm side do not fit a {what} of {} mm.",
                    mm(d.pin_side),
                    mm(pitch)
                ),
            ));
        }
    }
    if problems.is_empty() {
        Ok(d)
    } else {
        Err(problems)
    }
}

/// A length in mm as people write it: at least one decimal, at most as
/// many as it needs (2.54, 2.0, 1.27, 0.635).
fn mm(nm: i64) -> String {
    let whole = nm / MM;
    let mut frac = format!("{:06}", (nm % MM).abs());
    while frac.len() > 1 && frac.ends_with('0') {
        frac.pop();
    }
    format!("{whole}.{frac}")
}

/// `side × √2` (a square post's diagonal) to the nearest nanometre.
fn diagonal(side: i64) -> i64 {
    let square = 2 * side * side;
    let root = square.isqrt();
    // (root + ½)² = root² + root + ¼: above it the next integer is nearer.
    if square - root * root > root {
        root + 1
    } else {
        root
    }
}

/// The drill and land diameters.
fn drill_and_land(d: &Dims) -> (i64, i64) {
    let diagonal = diagonal(d.pin_side);
    let drill = diagonal + DRILL_EXTRA_NM;
    let pitch = if d.rows > 1 {
        d.pitch.min(d.row_pitch)
    } else {
        d.pitch
    };
    let land = (diagonal + LAND_EXTRA_NM)
        .min(pitch - LAND_GAP_NM)
        .max(drill + MIN_RING_NM);
    (drill, land)
}

fn pads(d: &Dims) -> Vec<Pad> {
    let (drill, land) = drill_and_land(d);
    (0..d.pins())
        .map(|k| {
            let key = (k + 1).to_string();
            Pad {
                terminal: key.clone(),
                key,
                // Pin 1 square, the others round.
                shape: if k == 0 {
                    PadShape::Rect { size: [land, land] }
                } else {
                    PadShape::Circle { diameter_nm: land }
                },
                at: d.at(k),
                rot_mdeg: 0,
                side: Side::Top,
                hole: Some(Hole {
                    shape: HoleShape::Round { diameter_nm: drill },
                    plated: true,
                }),
                mask: Opening::Default,
                paste: Opening::Default,
            }
        })
        .collect()
}

/// A narrow body with every pin down its left side in pin order, on the
/// symbol grid: a connector's starting point, as the host's templates draw
/// it. A stub too short for its pin number grows by whole modules, its
/// connection end moving out.
fn symbol(pins: i64) -> Symbol {
    let module = draw::MODULE_NM;
    let mut d = Drawing::default();
    d.shape(
        Primitive::Rect {
            min: [-module, -pins * module],
            max: [module, module],
        },
        draw::STROKE_NM,
        Fill::Body,
    );
    d.labels(module, -pins * module);
    for k in 0..pins {
        let number = (k + 1).to_string();
        let digits = i64::try_from(number.len()).unwrap_or(i64::MAX);
        let need = digits * PIN_NUMBER_ADVANCE_NM + 2 * PIN_NUMBER_CLEARANCE_NM;
        let length = if need > module {
            let modules = (need + module - 1) / module;
            (modules * module).min(MAX_PIN_LENGTH_NM)
        } else {
            module
        };
        d.pins.push(SymbolPin {
            terminal: number,
            at: [-module - length, -k * module],
            length_nm: length,
            rot_mdeg: RIGHT,
            show_name: true,
            show_number: true,
        });
    }
    Symbol {
        units: vec![d.unit("A")],
    }
}

/// The SHA-256 of each part file in the bundle (`parts/`), written by the
/// build script.
mod parts {
    include!(concat!(env!("OUT_DIR"), "/parts.rs"));
}

/// The header's 3D model: the bundle's part file (a moulded base on the
/// seating plane with square gold posts through it, standing `pin_above`
/// over the base and reaching `pin_below` under the seating plane, the
/// posts a rows × pins pattern), every parameter given. The base's corner
/// and top-edge rounds stay clear of the outermost posts.
fn model(d: &Dims) -> v1::Model {
    let clear = div_round(d.pitch - d.pin_side, 2)
        .unwrap_or(0)
        .min(div_round(d.row_pitch - d.pin_side, 2).unwrap_or(0));
    let corner = CORNER_ROUND_NM.min(div_round(clear, 2).unwrap_or(0));
    let edge = EDGE_ROUND_NM
        .min(div_round(d.body_height, 10).unwrap_or(0))
        .min(corner);
    let mut values = v1::Params::new();
    let mut set = |name: &str, v: i64| {
        values.insert(name.into(), ParamValue::Int(v));
    };
    set("rows", d.rows);
    set("pins", d.columns);
    set("pitch", d.pitch);
    set("row_pitch", d.row_pitch);
    set("body_height", d.body_height);
    set("pin_side", d.pin_side);
    set("pin_above", d.pin_above);
    set("pin_below", d.pin_below);
    set("corner_round", corner);
    set("edge_round", edge);
    v1::Model::PartFile {
        asset: parts::PIN_HEADER.into(),
        values,
    }
}

fn generate(request: &Request) -> Result<Generation, Vec<Problem>> {
    let description = description_with(Vec::new());
    let args = checked(&description, &request.params)?;
    let d = dims(&args)?;
    let pads = pads(&d);
    let body = d.body();
    let courtyard = land::courtyard(&pads, Some(body)).unwrap_or_default();
    let top = courtyard.iter().map(|p| p[1]).max().unwrap_or(body.max[1]);
    let mut silk = land::silk_outline(&pads, body, land::DEFAULT_MASK_NM);
    silk.push(land::silk_designator(body, top));
    if d.pins() > 2
        && let Some(first) = pads.first()
    {
        let half = land::pad_box(first).size()[0] / 2;
        silk.push(Graphic {
            key: "pin1".into(),
            shape: Primitive::Circle {
                center: [
                    first.at[0]
                        - half
                        - land::DEFAULT_MASK_NM
                        - land::SILK_PAD_CLEARANCE_NM
                        - 2 * PIN_ONE_DOT_RADIUS_NM,
                    first.at[1],
                ],
                radius_nm: PIN_ONE_DOT_RADIUS_NM,
            },
            stroke_nm: 0,
            fill: Fill::Solid,
        });
    }
    let keys: Vec<String> = (1..=d.pins()).map(|k| k.to_string()).collect();
    let package = Package {
        terminals: keys.clone(),
        geometry: PackageGeometry {
            // Posts below the seating plane are not part of the envelope.
            envelope_nm: [
                d.columns * d.pitch,
                d.rows * d.row_pitch,
                d.body_height + d.pin_above,
            ],
            seating_nm: 0,
            leads: pads
                .iter()
                .map(|p| Lead {
                    key: p.key.clone(),
                    contact: [p.at[0], p.at[1], 0],
                })
                .collect(),
            pin1: "1".into(),
        },
    };
    let footprint = Footprint {
        pads,
        courtyard,
        silk,
        assembly: land::assembly(body),
    };
    let component_part = request.scope == Scope::Component;
    let name = format!(
        "Pin header {}×{} {} mm",
        d.rows,
        d.columns,
        mm(d.pitch)
    );
    Ok(Generation {
        component: component_part.then(|| Component {
            terminals: keys
                .iter()
                .map(|key| Terminal {
                    key: key.clone(),
                    name: key.clone(),
                    direction: Direction::Passive,
                })
                .collect(),
            prefix: "J".into(),
            name,
            fields: Vec::new(),
        }),
        symbol: component_part.then(|| symbol(d.pins())),
        package,
        footprint,
        binding: if component_part {
            keys.iter()
                .map(|k| BindingEntry {
                    package_terminal: k.clone(),
                    terminal: k.clone(),
                })
                .collect()
        } else {
            Vec::new()
        },
        model: Some(model(&d)),
        fields: Vec::new(),
        notes: vec![
            "Lands and drills are estimated from the post (drill: its diagonal + 0.1 mm); \
             confirm them against the header's recommended drilling."
                .into(),
        ],
    })
}

impl Generator for Header {
    fn describe() -> Description {
        description()
    }

    fn generate(request: &Request) -> Result<Generation, Vec<Problem>> {
        generate(request)
    }
}

#[cfg(test)]
mod tests;
