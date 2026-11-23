//! The header generator: straight pin headers of rows × columns square
//! posts through a moulded base, and the female sockets they mate with
//! (`style`): a through-hole socket strip, or a surface-mount socket
//! entered from below through holes in the board. Each as a symbol,
//! package, footprint and 3D model.
//!
//! Through-hole lands follow the host's estimate for through-hole posts (an
//! original rule of thumb, not a land-pattern standard): a round drill of
//! the post's diagonal + 0.1 mm, plated, and a land of the diagonal +
//! 0.8 mm, kept 0.25 mm of copper apart at the tighter pitch but never
//! below a 0.2 mm ring; pin 1 square, the others round, or oval across the
//! rows when a land length is given. A bottom-entry socket's lands are
//! given (its recommended land pattern): a rectangular land per tail across
//! the rows and a non-plated hole per position for the mating pin. Pins
//! are numbered across the rows first: pin 1 at −x, −y, pin 2 beside it in
//! the next row.
//!
//! The presets are the header starter table, in nm: typical proportions,
//! estimated, as the table says, and sockets from Würth Elektronik's
//! WR-PHD drawings.
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
    self, BindingEntry, Capability, Choice, Component, Description, Direction, Effect, Fill,
    Footprint, Generation, Grammar, Graphic, Hole, HoleShape, Lead, Opening, Package,
    PackageGeometry, Pad, PadShape, Param, ParamType, ParamValue, PartOffer, Preset, Primitive,
    Problem, Request, Rule, Scope, Side, Symbol, SymbolPin, Terminal,
};
use sparksmith_generator_sdk::{Generator, export_generator};

/// The starter rows, in nm.
const PRESETS: &str = include_str!("../presets.json");

/// The generator's version (`Description::version`).
/// 5: the SDK's pin table (`pin_table`) and part numbers
/// (`part_numbers`).
const VERSION: u32 = 5;

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

/// A socket's openings, as fractions of its tighter pitch in thousandths
/// (est., read off Würth Elektronik's WR-PHD drawings at 2.54 mm: a
/// square entry of about 1 mm under a lead-in about 1.7 mm across at the
/// face, a cavity wider than the entry holding the contact's two leaves).
const LEAD_IN_PERMILLE: i64 = 680;
const ENTRY_PERMILLE: i64 = 400;
const CAVITY_PERMILLE: i64 = 560;
/// The contact's leaves: sheet thickness, the gap between them (they grip
/// a post narrower than the entry) and their width, the last two as
/// fractions of the entry in thousandths (est.).
const LEAF_NM: i64 = 150_000;
const LEAF_GAP_PERMILLE: i64 = 450;
const LEAF_WIDTH_PERMILLE: i64 = 800;
/// A socket's floor under the cavity and roof over it, at most (est.).
const SOCKET_FLOOR_NM: i64 = 1_000_000;
const SOCKET_ROOF_NM: i64 = 300_000;
/// A surface-mount tail's sheet thickness and bend radius (est.).
const TAIL_SHEET_NM: i64 = 200_000;
const TAIL_BEND_NM: i64 = 200_000;
/// A socket's upright corners' round (est.).
const SOCKET_CORNER_NM: i64 = 150_000;

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

/// What the generator makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Style {
    /// Square posts through a moulded base.
    Pin,
    /// A female socket strip with through-hole tails.
    Socket,
    /// A female socket on surface-mount tails, entered from below through
    /// a hole in the board per position.
    SocketSmdBottom,
}

impl Style {
    const ALL: [(Self, &'static str, &'static str); 3] = [
        (Self::Pin, "pin", "Pin header"),
        (Self::Socket, "socket", "Socket, through-hole"),
        (
            Self::SocketSmdBottom,
            "socket_smd_bottom",
            "Socket, SMD, bottom entry",
        ),
    ];

    fn of(key: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .find(|(_, k, _)| *k == key)
            .map(|(s, _, _)| *s)
    }
}

fn helped(mut param: Param, help: &str) -> Param {
    param.help = Some(help.into());
    param
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
        Param {
            key: "style".into(),
            label: "Style".into(),
            help: None,
            ty: ParamType::Enum {
                choices: Style::ALL
                    .iter()
                    .map(|(_, key, label)| Choice {
                        key: (*key).into(),
                        label: (*label).into(),
                    })
                    .collect(),
            },
            default: ParamValue::Text("pin".into()),
            group: Some("body".into()),
            context: None,
        },
        count("rows", "Rows", MAX_ROWS, 2),
        count("columns", "Pins per row", MAX_COLUMNS, 5),
        length(
            "pitch",
            "Pitch along a row",
            500 * UM,
            10 * MM,
            2_540_000,
            "pins",
        ),
        length(
            "row_pitch",
            "Row pitch",
            500 * UM,
            10 * MM,
            2_540_000,
            "pins",
        ),
        helped(
            length("pin_side", "Pin side", 100 * UM, 2 * MM, 640_000, "pins"),
            "A socket's tail width.",
        ),
        length(
            "pin_above",
            "Pin length above the body (pin header)",
            100 * UM,
            30 * MM,
            6 * MM,
            "pins",
        ),
        length(
            "pin_below",
            "Pin length below the seating plane (through-hole)",
            100 * UM,
            20 * MM,
            3 * MM,
            "pins",
        ),
        length(
            "body_height",
            "Body height",
            200 * UM,
            20 * MM,
            2_540_000,
            "body",
        ),
        helped(
            length(
                "land_length",
                "Land length across the rows",
                0,
                10 * MM,
                0,
                "lands",
            ),
            "Through-hole: 0 is a round land, a longer one an oval across the rows. \
             SMD socket: the tail's land, required.",
        ),
        length(
            "land_width",
            "Land width along the row (SMD socket)",
            100 * UM,
            5 * MM,
            900_000,
            "lands",
        ),
        length(
            "land_span",
            "Lands' centres across the rows (SMD socket)",
            500 * UM,
            30 * MM,
            6_400_000,
            "lands",
        ),
        length(
            "mating_hole",
            "Mating pin's hole in the board (SMD socket)",
            100 * UM,
            5 * MM,
            1_100_000,
            "lands",
        ),
        length(
            "tail_span",
            "Tails' tips across the rows (SMD socket)",
            500 * UM,
            30 * MM,
            7_200_000,
            "body",
        ),
        length(
            "standoff",
            "Body underside above the board (SMD socket)",
            0,
            2 * MM,
            200_000,
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
        Rule {
            text: "SMD socket: two rows, a land length given, the lands and tails outside the body"
                .into(),
            fields: vec![
                "rows".into(),
                "land_length".into(),
                "land_span".into(),
                "tail_span".into(),
            ],
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
        title: "Pin headers and sockets".into(),
        licence: "MIT OR Apache-2.0".into(),
        deterministic: true,
        capabilities: vec![Capability {
            name: v1::GENERATE_PART.into(),
            effect: Effect::None,
            title: Some("Make a pin header or socket".into()),
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
    style: Style,
    rows: i64,
    columns: i64,
    pitch: i64,
    row_pitch: i64,
    body_height: i64,
    pin_side: i64,
    pin_above: i64,
    pin_below: i64,
    land_length: i64,
    land_width: i64,
    land_span: i64,
    mating_hole: i64,
    tail_span: i64,
    standoff: i64,
}

impl Dims {
    fn pins(&self) -> i64 {
        self.rows * self.columns
    }

    /// The moulded base: one pitch per pin long, one row pitch per row
    /// wide, centred.
    fn body(&self) -> Box2 {
        Box2::centred(
            [0, 0],
            [self.columns * self.pitch, self.rows * self.row_pitch],
        )
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
    let style = args.text("style").map_err(one)?;
    let d = Dims {
        style: Style::of(style).ok_or_else(|| {
            one(problem(
                "param.out_of_range",
                "style",
                format!("No header style \"{style}\"."),
            ))
        })?,
        rows: args.int("rows").map_err(one)?,
        columns: args.int("columns").map_err(one)?,
        pitch: args.int("pitch").map_err(one)?,
        row_pitch: args.int("row_pitch").map_err(one)?,
        body_height: args.int("body_height").map_err(one)?,
        pin_side: args.int("pin_side").map_err(one)?,
        pin_above: args.int("pin_above").map_err(one)?,
        pin_below: args.int("pin_below").map_err(one)?,
        land_length: args.int("land_length").map_err(one)?,
        land_width: args.int("land_width").map_err(one)?,
        land_span: args.int("land_span").map_err(one)?,
        mating_hole: args.int("mating_hole").map_err(one)?,
        tail_span: args.int("tail_span").map_err(one)?,
        standoff: args.int("standoff").map_err(one)?,
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
    if problems.is_empty()
        && let Some((field, message)) = style_problem(&d)
    {
        problems.push(problem("param.inconsistent", field, message));
    }
    if problems.is_empty() {
        Ok(d)
    } else {
        Err(problems)
    }
}

/// What a socket's style needs of its values, the first unmet.
fn style_problem(d: &Dims) -> Option<(&'static str, String)> {
    let no_cavity = || {
        Some((
            "body_height",
            format!(
                "A body {} mm high leaves no cavity for the contacts.",
                mm(d.body_height)
            ),
        ))
    };
    match d.style {
        Style::Pin | Style::Socket if d.land_length > 0 => {
            let land = drill_and_land(d).1;
            if d.land_length < land {
                return Some((
                    "land_length",
                    format!(
                        "An oval land {} mm long is shorter than its {} mm width.",
                        mm(d.land_length),
                        mm(land)
                    ),
                ));
            }
        }
        Style::Pin | Style::Socket => {}
        Style::SocketSmdBottom => {
            let width = d.rows * d.row_pitch;
            let pitch = d.pitch.min(d.row_pitch);
            if d.rows != 2 {
                return Some((
                    "rows",
                    format!("A bottom-entry SMD socket has two rows, not {}.", d.rows),
                ));
            }
            if d.land_length == 0 {
                return Some((
                    "land_length",
                    "A bottom-entry SMD socket needs its land length.".into(),
                ));
            }
            if d.land_span - d.land_length <= d.row_pitch + d.mating_hole {
                return Some((
                    "land_span",
                    format!(
                        "Lands {} mm long with centres {} mm apart reach the mating pins' holes.",
                        mm(d.land_length),
                        mm(d.land_span)
                    ),
                ));
            }
            if d.tail_span <= width {
                return Some((
                    "tail_span",
                    format!(
                        "Tails {} mm tip to tip do not leave a body {} mm wide.",
                        mm(d.tail_span),
                        mm(width)
                    ),
                ));
            }
            if d.land_width >= d.pitch {
                return Some((
                    "land_width",
                    format!(
                        "Lands {} mm wide do not fit a pitch of {} mm.",
                        mm(d.land_width),
                        mm(d.pitch)
                    ),
                ));
            }
            if d.mating_hole >= pitch {
                return Some((
                    "mating_hole",
                    format!(
                        "Holes of {} mm do not fit a pitch of {} mm.",
                        mm(d.mating_hole),
                        mm(pitch)
                    ),
                ));
            }
            if d.standoff + 2 * (entry_depth(d) + SOCKET_ROOF_NM) + LEAF_NM >= d.body_height {
                return no_cavity();
            }
        }
    }
    if d.style == Style::Socket
        && floor(d) + entry_depth(d) + SOCKET_ROOF_NM + LEAF_NM >= d.body_height
    {
        return no_cavity();
    }
    None
}

/// `nm × permille / 1000`, to the nearest nanometre.
fn permille(nm: i64, permille: i64) -> i64 {
    div_round(nm * permille, 1000).unwrap_or(0)
}

/// A socket's openings: the lead-in's side at the face, the entry's side
/// under it and the cavity's side, from the tighter pitch.
fn openings(d: &Dims) -> (i64, i64, i64) {
    let pitch = if d.rows > 1 {
        d.pitch.min(d.row_pitch)
    } else {
        d.pitch
    };
    (
        permille(pitch, LEAD_IN_PERMILLE),
        permille(pitch, ENTRY_PERMILLE),
        permille(pitch, CAVITY_PERMILLE),
    )
}

/// The lead-in's depth: a 45° chamfer from the lead-in to the entry.
fn entry_depth(d: &Dims) -> i64 {
    let (lead_in, entry, _) = openings(d);
    div_round(lead_in - entry, 2).unwrap_or(0)
}

/// A through-hole socket's floor under its cavities.
fn floor(d: &Dims) -> i64 {
    SOCKET_FLOOR_NM.min(div_round(d.body_height, 4).unwrap_or(0))
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
    if d.style == Style::SocketSmdBottom {
        return smd_socket_pads(d);
    }
    let (drill, land) = drill_and_land(d);
    (0..d.pins())
        .map(|k| {
            let key = (k + 1).to_string();
            Pad {
                terminal: key.clone(),
                key,
                // Pin 1 square, the others round; or oval across the rows.
                shape: match (k == 0, d.land_length > land) {
                    (true, false) => PadShape::Rect { size: [land, land] },
                    (false, false) => PadShape::Circle { diameter_nm: land },
                    (true, true) => PadShape::Rect {
                        size: [land, d.land_length],
                    },
                    (false, true) => PadShape::Oval {
                        size: [land, d.land_length],
                    },
                },
                at: d.at(k),
                rot_mdeg: 0,
                side: Side::Top,
                hole: Some(Hole {
                    shape: HoleShape::Round { diameter_nm: drill },
                    plated: true,
                    offset: None,
                }),
                mask: Opening::Default,
                paste: Opening::Default,
                number: None,
                kind: None,
            }
        })
        .collect()
}

/// A bottom-entry socket's lands: per pin a rectangular land across the
/// rows, outside its row at half the land span; per position the mating
/// pin's non-plated hole, unnumbered, landing no terminal.
fn smd_socket_pads(d: &Dims) -> Vec<Pad> {
    let half = div_round(d.land_span, 2).unwrap_or(0);
    let lands = (0..d.pins()).map(|k| {
        let key = (k + 1).to_string();
        let [x, y] = d.at(k);
        Pad {
            terminal: key.clone(),
            key,
            shape: PadShape::Rect {
                size: [d.land_width, d.land_length],
            },
            at: [x, if y < 0 { -half } else { half }],
            rot_mdeg: 0,
            side: Side::Top,
            hole: None,
            mask: Opening::Default,
            paste: Opening::Default,
            number: None,
            kind: None,
        }
    });
    let holes = (0..d.pins()).map(|k| Pad {
        key: format!("H{}", k + 1),
        terminal: String::new(),
        shape: PadShape::Circle {
            diameter_nm: d.mating_hole,
        },
        at: d.at(k),
        rot_mdeg: 0,
        side: Side::Top,
        hole: Some(Hole {
            shape: HoleShape::Round {
                diameter_nm: d.mating_hole,
            },
            plated: false,
            offset: None,
        }),
        mask: Opening::Default,
        paste: Opening::None,
        number: Some(String::new()),
        kind: None,
    });
    lands.chain(holes).collect()
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
    if d.style != Style::Pin {
        return socket_model(d);
    }
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

/// A socket's 3D model: the bundle's part file for its style, every
/// parameter given. The black housing stands on the seating plane, or
/// `standoff` above it; per position a square entry under a 45° lead-in
/// opens into a cavity holding the gold contact's two leaves.
/// Through-hole: the entry opens at the top, the contact's tail of
/// `pin_side` reaches `pin_below` under the seating plane. Bottom entry:
/// the entry opens at the bottom, over the board's hole, and a window with
/// the same lead-in at the top; each contact's tail leaves the housing's
/// long side low and runs out on the board to `tail_span` / 2 from the
/// centre line.
fn socket_model(d: &Dims) -> v1::Model {
    let (lead_in, entry, cavity) = openings(d);
    let depth = entry_depth(d);
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
    set("lead_in", lead_in);
    set("entry", entry);
    set("entry_depth", depth);
    set("cavity", cavity);
    set("leaf", LEAF_NM);
    set("leaf_gap", permille(entry, LEAF_GAP_PERMILLE));
    set("leaf_width", permille(entry, LEAF_WIDTH_PERMILLE));
    set("corner_round", SOCKET_CORNER_NM);
    set("cavity_top", d.body_height - depth - SOCKET_ROOF_NM);
    let asset = if d.style == Style::Socket {
        set("pin_below", d.pin_below);
        set("floor_z", floor(d));
        parts::PIN_SOCKET
    } else {
        set("standoff", d.standoff);
        set("cavity_bottom", d.standoff + depth + SOCKET_ROOF_NM);
        set("tail_span", d.tail_span);
        set("tail_sheet", TAIL_SHEET_NM);
        set("tail_bend", TAIL_BEND_NM);
        parts::SOCKET_SMD_BOTTOM
    };
    v1::Model::PartFile {
        asset: asset.into(),
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
            // Posts below the seating plane are not part of the envelope;
            // a surface-mount socket's tails on the board are.
            envelope_nm: [
                d.columns * d.pitch,
                if d.style == Style::SocketSmdBottom {
                    d.tail_span.max(d.rows * d.row_pitch)
                } else {
                    d.rows * d.row_pitch
                },
                d.body_height
                    + if d.style == Style::Pin {
                        d.pin_above
                    } else {
                        0
                    },
            ],
            seating_nm: 0,
            leads: pads
                .iter()
                .filter(|p| !p.terminal.is_empty())
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
        keepouts: Vec::new(),
    };
    let component_part = request.scope == Scope::Component;
    let kind = match d.style {
        Style::Pin => "Pin header",
        Style::Socket => "Pin socket",
        Style::SocketSmdBottom => "SMD socket, bottom entry,",
    };
    let name = format!("{kind} {}×{} {} mm", d.rows, d.columns, mm(d.pitch));
    Ok(Generation {
        orderable: Vec::new(),
        marking: None,
        component: component_part.then(|| Component {
            terminals: keys
                .iter()
                .map(|key| Terminal {
                    key: key.clone(),
                    name: key.clone(),
                    direction: Direction::Passive,
                    alternates: Vec::new(),
                    drive: None,
                    unconnected: None,
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
        notes: vec![if d.style == Style::SocketSmdBottom {
            "Lands, land span and the mating pins' holes are as given; confirm them \
             against the socket's recommended land pattern."
                .into()
        } else {
            "Lands and drills are estimated from the post (drill: its diagonal + 0.1 mm); \
             confirm them against the header's recommended drilling."
                .into()
        }],
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
