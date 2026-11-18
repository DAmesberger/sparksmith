//! The through-hole generator: axial and radial two-lead parts, dual
//! in-line packages and single-row TO packages (TO-92, TO-220 style), as a
//! symbol, package, footprint and 3D model.
//!
//! The seating plane z = 0 is the board's top face; leads reach
//! `lead_below` under it into their drills. Lead order: an axial or radial
//! part's lead 1 at −x (the cathode or negative end of a polarised part);
//! a DIP counter-clockwise from the −x end of the −y row (pin 1 bottom
//! left, seen from above); a TO row from −x.
//!
//! Lands follow the host's estimate for through-hole leads (an original
//! rule of thumb, not a land-pattern standard): a plated round drill
//! clearing the lead's largest cross-section by 0.3 mm, rounded up to
//! 0.05 mm, and a land of the drill + 0.6 mm kept 0.25 mm of copper inside
//! the lead pitch but never below a 0.2 mm ring, rounded down to 0.05 mm.
//! A DIP's or TO's pin 1 is a rounded square, the others round.
//!
//! The presets are the through-hole starter table, in nm, with its
//! citations: JEDEC MS-001 and MS-011 (DIP), TO-220, TO-226 (TO-92) and
//! DO-204 (DO-35, DO-41) as published; the axial resistor and radial can
//! rows are typical series values, estimated, as the table says.
#![cfg_attr(target_arch = "wasm32", no_std)]

extern crate alloc;

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use sparksmith_generator_sdk::args::{Args, checked, problem};
use sparksmith_generator_sdk::draw::{self, Drawing, LEFT, RIGHT};
use sparksmith_generator_sdk::land::{self, Box2};
use sparksmith_generator_sdk::units::{MM, UM, div_round, mm_text, round_to};
use sparksmith_generator_sdk::v1::{
    self, BindingEntry, Capability, Choice, Component, Description, Direction, Effect, Fill,
    Footprint, Generation, Grammar, Graphic, HAlign, Hole, HoleShape, Lead, Opening, Package,
    PackageGeometry, Pad, PadShape, Param, ParamType, ParamValue, PartOffer, Preset, Primitive,
    Problem, Request, Rule, Scope, Side, Symbol, SymbolPin, Terminal, TextContent, VAlign,
};
use sparksmith_generator_sdk::{Generator, export_generator};

/// The starter rows, in nm, with their citations.
const PRESETS: &str = include_str!("../presets.json");

/// The generator's version (`Description::version`).
const VERSION: u32 = 1;

/// Most pins of a DIP (both rows) and of a TO row.
const MAX_DIP_PINS: i64 = 128;
const MAX_TO_PINS: i64 = 64;

/// Estimated standoffs (body underside above the board) when none is
/// given: an axial or radial body rests on the board; a DIP stands on its
/// lead shoulders (JEDEC MS-001 gives at least 0.38 mm); a TO body sits on
/// its lead stops (est.).
const DIP_STANDOFF_NM: i64 = 500_000;
const TO_STANDOFF_NM: i64 = 1_000_000;
/// A DIP lead's strip thickness (est.: 0.2–0.36 mm); `lead_diameter` is
/// its width.
const DIP_LEAD_THICKNESS_NM: i64 = 250_000;
/// Share of a DIP body's height, in thousandths, at which the leads leave
/// it (est.).
const DIP_LEAD_EXIT_MILLI: i64 = 300;

/// The drill clears the lead's largest cross-section by this, rounded up
/// to [`LAND_STEP_NM`]…
const DRILL_CLEARANCE_NM: i64 = 300_000;
/// …the land adds this ring (both sides together)…
const RING_NM: i64 = 600_000;
/// …but stays this much copper inside the lead pitch…
const LAND_GAP_NM: i64 = 250_000;
/// …keeping at least this ring, rounded down to [`LAND_STEP_NM`].
const MIN_RING_NM: i64 = 200_000;
const LAND_STEP_NM: i64 = 50_000;
/// Pin 1's rounded-square land: corners of a quarter of its side.
const PIN_ONE_CORNER_SHARE: i64 = 4;

/// The pin-1 dot: a filled silkscreen circle of this radius beyond pad 1's
/// −x edge, clear of its mask opening by the silkscreen clearance (as the
/// host's template marks pin 1 of a package of more than two terminals or
/// of a polarised two-terminal part).
const PIN_ONE_DOT_RADIUS_NM: i64 = 200_000;

/// How far a lead root reaches into the body (est.), at most a quarter of
/// the body along the lead (a third of a DIP's half width).
const ROOT_NM: i64 = 500_000;
/// Round of an axial body's ends and of a radial body's top (est.).
const END_ROUND_NM: i64 = 300_000;
/// Rounds of a moulded block's upright corners and top edges (est.).
const CORNER_ROUND_NM: i64 = 300_000;
const EDGE_ROUND_NM: i64 = 200_000;

pub struct Tht;

export_generator!(Tht);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Construction {
    Axial,
    Radial,
    Dip,
    To,
}

impl Construction {
    const ALL: [(Self, &'static str, &'static str); 4] = [
        (Self::Axial, "axial", "Axial (body lying, leads bent down)"),
        (Self::Radial, "radial", "Radial (body standing up)"),
        (Self::Dip, "dip", "Dual in-line (DIP)"),
        (Self::To, "to", "Single-row TO (TO-92, TO-220)"),
    ];

    fn from_key(key: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .find(|(_, k, _)| *k == key)
            .map(|(c, _, _)| *c)
    }
}

/// The schematic symbol.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Sym {
    /// A box with the pins down its left side, then up its right.
    Box,
    Resistor,
    Diode,
    PolarisedCapacitor,
}

impl Sym {
    const ALL: [(Self, &'static str, &'static str); 4] = [
        (Self::Box, "box", "Box with numbered pins"),
        (Self::Resistor, "resistor", "Resistor"),
        (Self::Diode, "diode", "Diode"),
        (
            Self::PolarisedCapacitor,
            "polarised_capacitor",
            "Polarised capacitor",
        ),
    ];

    fn from_key(key: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .find(|(_, k, _)| *k == key)
            .map(|(s, _, _)| *s)
    }

    /// The designator prefix the symbol customarily takes.
    fn prefix(self) -> &'static str {
        match self {
            Self::Box => "U",
            Self::Resistor => "R",
            Self::Diode => "D",
            Self::PolarisedCapacitor => "C",
        }
    }

    /// Whether pin 1 is marked on the footprint of a two-lead part.
    fn polarised(self) -> bool {
        matches!(self, Self::Diode | Self::PolarisedCapacitor)
    }
}

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

fn helped(mut param: Param, help: &str) -> Param {
    param.help = Some(help.into());
    param
}

fn choices(pairs: &[(&str, &str)]) -> Vec<Choice> {
    pairs
        .iter()
        .map(|(key, label)| Choice {
            key: (*key).into(),
            label: (*label).into(),
        })
        .collect()
}

fn enumeration(
    key: &str,
    label: &str,
    options: &[(&str, &str)],
    default: &str,
    group: &str,
) -> Param {
    Param {
        key: key.into(),
        label: label.into(),
        help: None,
        ty: ParamType::Enum {
            choices: choices(options),
        },
        default: ParamValue::Text(default.into()),
        group: Some(group.into()),
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
    let constructions: Vec<(&str, &str)> =
        Construction::ALL.iter().map(|(_, k, l)| (*k, *l)).collect();
    let symbols: Vec<(&str, &str)> = Sym::ALL.iter().map(|(_, k, l)| (*k, *l)).collect();
    let mut style = enumeration(
        "resistor_style",
        "Resistor symbol",
        &[("iec", "IEC (rectangle)"), ("us", "US (zigzag)")],
        "iec",
        "symbol",
    );
    style.context = Some("project.symbols.resistor_style".into());
    let params = vec![
        enumeration(
            "construction",
            "Construction",
            &constructions,
            "dip",
            "package",
        ),
        helped(
            Param {
                key: "pins".into(),
                label: "Pins".into(),
                help: None,
                ty: ParamType::Count {
                    min: 1,
                    max: MAX_DIP_PINS,
                },
                default: ParamValue::Int(8),
                group: Some("leads".into()),
                context: None,
            },
            "A DIP's pins in both rows, a TO row's leads; an axial or radial part has two.",
        ),
        length(
            "lead_pitch",
            "Lead pitch",
            100 * UM,
            100 * MM,
            2_540_000,
            "leads",
        ),
        helped(
            length(
                "lead_diameter",
                "Lead diameter",
                50 * UM,
                5 * MM,
                460_000,
                "leads",
            ),
            "A DIP lead's width; a TO lead's square side.",
        ),
        length(
            "lead_below",
            "Lead length below the board",
            50 * UM,
            30 * MM,
            3_300_000,
            "leads",
        ),
        helped(
            length(
                "body_pitch",
                "Lead pitch at the body",
                0,
                100 * MM,
                0,
                "leads",
            ),
            "Formed leads (radial, TO): the pitch where the leads leave the body; 0 is straight leads.",
        ),
        helped(
            length("bend_radius", "Bend radius", 0, 5 * MM, 0, "leads"),
            "The centreline radius of the lead bends; 0 is one lead diameter (est.).",
        ),
        helped(
            enumeration(
                "standoff_basis",
                "Standoff",
                &[
                    ("estimated", "Estimated for the construction"),
                    ("given", "Given"),
                ],
                "estimated",
                "body",
            ),
            "Estimated: an axial or radial body on the board, a DIP 0.5 mm and a TO body 1 mm above it.",
        ),
        length(
            "standoff",
            "Body underside above the board",
            0,
            20 * MM,
            0,
            "body",
        ),
        length(
            "row_pitch",
            "Row pitch (DIP)",
            500 * UM,
            100 * MM,
            7_620_000,
            "body",
        ),
        length(
            "body_length",
            "Body length",
            200 * UM,
            100 * MM,
            9_270_000,
            "body",
        ),
        length(
            "body_width",
            "Body width (DIP, TO)",
            200 * UM,
            50 * MM,
            6_350_000,
            "body",
        ),
        length(
            "body_height",
            "Body height (radial, DIP, TO)",
            200 * UM,
            100 * MM,
            3_300_000,
            "body",
        ),
        length(
            "body_diameter",
            "Body diameter (axial, radial)",
            200 * UM,
            50 * MM,
            2_500_000,
            "body",
        ),
        enumeration("symbol", "Symbol", &symbols, "box", "symbol"),
        style,
    ];
    let rules = vec![
        Rule {
            text: "Axial: (lead pitch − body length) / 2 > lead diameter, so the leads bend down outside the body".into(),
            fields: vec!["lead_pitch".into(), "body_length".into(), "lead_diameter".into()],
        },
        Rule {
            text: "DIP: an even number of pins, the leads of a row along the body and the rows outside it".into(),
            fields: vec!["pins".into(), "row_pitch".into(), "body_width".into()],
        },
        Rule {
            text: "Formed leads need a standoff of at least two lead diameters".into(),
            fields: vec!["body_pitch".into(), "standoff".into(), "lead_diameter".into()],
        },
    ];
    let offer = PartOffer {
        families: vec!["tht".into()],
        scopes: vec![Scope::Component, Scope::Package],
        grammar: vec![Grammar {
            scope: Scope::Component,
            patterns: vec![
                "dip {pins} {row_pitch:length}?".into(),
                "to-92".into(),
                "to-220".into(),
                "axial {lead_pitch:length}?".into(),
                "radial {body_diameter:length}?".into(),
            ],
            examples: vec![
                "DIP-8".into(),
                "dip 28 15.24mm".into(),
                "TO-92".into(),
                "axial resistor".into(),
            ],
        }],
        instance_fields: Vec::new(),
    };
    Description {
        abi: v1::ABI,
        id: "tht".into(),
        version: VERSION,
        title: "Through-hole packages".into(),
        licence: "MIT OR Apache-2.0".into(),
        deterministic: true,
        capabilities: vec![Capability {
            name: v1::GENERATE_PART.into(),
            effect: Effect::None,
            title: Some("Make a through-hole part".into()),
            params,
            rules,
            presets,
            part: Some(offer),
        }],
        needs: Vec::new(),
    }
}

/// A part's values, checked, with the estimates filled in.
struct Dims {
    construction: Construction,
    /// Leads in all (axial and radial: 2).
    pins: i64,
    pitch: i64,
    d: i64,
    below: i64,
    /// The pitch where the leads leave the body (the board's when straight).
    body_pitch: i64,
    /// The centreline bend radius.
    bend: i64,
    standoff: i64,
    row_pitch: i64,
    body_length: i64,
    body_width: i64,
    body_height: i64,
    body_diameter: i64,
    symbol: Sym,
}

impl Dims {
    /// Whether the leads are formed (their pitch changes below the body).
    /// A single TO lead is straight whatever its pitches.
    fn formed(&self) -> bool {
        self.body_pitch != self.pitch && self.pins > 1
    }

    /// Leads in a row: a DIP's per side, otherwise all.
    fn per_row(&self) -> i64 {
        match self.construction {
            Construction::Dip => self.pins / 2,
            _ => self.pins,
        }
    }

    /// The lead contacts on the seating plane, in lead order.
    fn lead_points(&self) -> Vec<Point2> {
        let p = self.pitch;
        match self.construction {
            Construction::Axial | Construction::Radial => {
                let half = div_round(p, 2).unwrap_or(0);
                vec![[-half, 0], [half, 0]]
            }
            Construction::Dip => {
                let n = self.pins / 2;
                let y = div_round(self.row_pitch, 2).unwrap_or(0);
                let mut out: Vec<Point2> = (0..n).map(|i| [along(i, n, p), -y]).collect();
                out.extend((0..n).rev().map(|i| [along(i, n, p), y]));
                out
            }
            Construction::To => (0..self.pins)
                .map(|i| [along(i, self.pins, p), 0])
                .collect(),
        }
    }

    /// Body and leads above the seating plane: x, y and z.
    fn envelope(&self) -> [i64; 3] {
        let (s, d) = (self.standoff, self.d);
        match self.construction {
            Construction::Axial => [
                self.body_length.max(self.pitch + d),
                self.body_diameter,
                s + self.body_diameter,
            ],
            Construction::Radial => [
                self.body_diameter.max(self.pitch + d),
                self.body_diameter,
                s + self.body_height,
            ],
            Construction::Dip => [
                self.body_length,
                self.body_width.max(self.row_pitch + DIP_LEAD_THICKNESS_NM),
                s + self.body_height,
            ],
            Construction::To => {
                let row = (self.pins - 1) * self.pitch + d;
                [
                    self.body_length.max(row),
                    self.body_width.max(d),
                    s + self.body_height,
                ]
            }
        }
    }

    /// The lead's largest cross-section: a round lead's diameter, a DIP
    /// strip's diagonal, a square TO lead's diagonal.
    fn section(&self) -> i64 {
        match self.construction {
            Construction::Axial | Construction::Radial => self.d,
            Construction::Dip => hypot(self.d, DIP_LEAD_THICKNESS_NM),
            Construction::To => hypot(self.d, self.d),
        }
    }

    /// The drill and land diameters.
    fn drill_and_land(&self) -> (i64, i64) {
        let drill = round_to(self.section() + DRILL_CLEARANCE_NM, LAND_STEP_NM, true).unwrap_or(0);
        let land = (drill + RING_NM)
            .min(self.pitch - LAND_GAP_NM)
            .max(drill + MIN_RING_NM);
        (drill, round_to(land, LAND_STEP_NM, false).unwrap_or(0))
    }

    /// Whether pin 1's land is a rounded square: DIP and TO parts.
    fn square_pin_one(&self) -> bool {
        matches!(self.construction, Construction::Dip | Construction::To)
    }
}

type Point2 = [i64; 2];

/// `(i − (n − 1) / 2) × pitch`, to the nearest nanometre.
fn along(i: i64, n: i64, pitch: i64) -> i64 {
    div_round((2 * i - (n - 1)) * pitch, 2).unwrap_or(0)
}

/// `√(a² + b²)` to the nearest nanometre.
fn hypot(a: i64, b: i64) -> i64 {
    let square = a * a + b * b;
    let root = square.isqrt();
    // (root + ½)² = root² + root + ¼: above it the next integer is nearer.
    if square - root * root > root {
        root + 1
    } else {
        root
    }
}

fn inconsistent(field: &str, message: String) -> Problem {
    problem("param.inconsistent", field, message)
}

fn dims(args: &Args<'_>) -> Result<Dims, Vec<Problem>> {
    let one = |p: Problem| vec![p];
    let key = args.text("construction").map_err(one)?;
    let construction = Construction::from_key(key).ok_or_else(|| {
        vec![problem(
            "param.out_of_range",
            "construction",
            format!("Construction \"{key}\" is not one this generator makes."),
        )]
    })?;
    let key = args.text("symbol").map_err(one)?;
    let symbol = Sym::from_key(key).ok_or_else(|| {
        vec![problem(
            "param.out_of_range",
            "symbol",
            format!("Symbol \"{key}\" is not one this generator draws."),
        )]
    })?;
    let d = args.int("lead_diameter").map_err(one)?;
    let pitch = args.int("lead_pitch").map_err(one)?;
    let body_pitch = match args.int("body_pitch").map_err(one)? {
        0 => pitch,
        given => given,
    };
    let bend = match args.int("bend_radius").map_err(one)? {
        0 => d,
        given => given,
    };
    let standoff = if args.text("standoff_basis").map_err(one)? == "given" {
        args.int("standoff").map_err(one)?
    } else {
        match construction {
            Construction::Axial | Construction::Radial => 0,
            Construction::Dip => DIP_STANDOFF_NM,
            Construction::To => TO_STANDOFF_NM,
        }
    };
    let pins = match construction {
        Construction::Axial | Construction::Radial => 2,
        _ => args.int("pins").map_err(one)?,
    };
    let dims = Dims {
        construction,
        pins,
        pitch,
        d,
        below: args.int("lead_below").map_err(one)?,
        body_pitch,
        bend,
        standoff,
        row_pitch: args.int("row_pitch").map_err(one)?,
        body_length: args.int("body_length").map_err(one)?,
        body_width: args.int("body_width").map_err(one)?,
        body_height: args.int("body_height").map_err(one)?,
        body_diameter: args.int("body_diameter").map_err(one)?,
        symbol,
    };
    let problems = consistency(&dims, args.int("body_pitch").map_err(one)? != 0);
    if problems.is_empty() {
        Ok(dims)
    } else {
        Err(problems)
    }
}

/// The construction's rules (the host's through-hole form rules), each a
/// problem in plain words naming its field.
fn consistency(f: &Dims, body_pitch_given: bool) -> Vec<Problem> {
    let mut out = Vec::new();
    let mut rule = |ok: bool, field: &str, message: String| {
        if !ok {
            out.push(inconsistent(field, message));
        }
    };
    let (d, p, s) = (f.d, f.pitch, f.standoff);
    let formed = f.formed();
    match f.construction {
        Construction::Axial => {
            rule(
                p - f.body_length > 2 * d,
                "lead_pitch",
                format!(
                    "Leads at {} cannot bend down outside a body {} long: (lead pitch − body length) / 2 must exceed the lead diameter ({}).",
                    mm_text(p),
                    mm_text(f.body_length),
                    mm_text(d)
                ),
            );
            rule(
                d < f.body_diameter,
                "lead_diameter",
                format!(
                    "Leads of {} are not thinner than the body ({}).",
                    mm_text(d),
                    mm_text(f.body_diameter)
                ),
            );
            rule(
                !body_pitch_given,
                "body_pitch",
                "Axial leads have no pitch at the body; leave it 0.".into(),
            );
        }
        Construction::Radial => {
            rule(
                f.body_pitch + d <= f.body_diameter,
                "body_pitch",
                format!(
                    "Leads {} apart at the body and {} thick do not leave the underside of a body {} across.",
                    mm_text(f.body_pitch),
                    mm_text(d),
                    mm_text(f.body_diameter)
                ),
            );
            rule(
                p > d,
                "lead_pitch",
                format!(
                    "Leads of {} at a pitch of {} would touch.",
                    mm_text(d),
                    mm_text(p)
                ),
            );
        }
        Construction::Dip => {
            rule(
                f.pins >= 2 && f.pins % 2 == 0,
                "pins",
                format!(
                    "A DIP has an even number of pins, 2 or more; {} is not.",
                    f.pins
                ),
            );
            rule(
                p > d,
                "lead_pitch",
                format!(
                    "Leads of {} at a pitch of {} would touch.",
                    mm_text(d),
                    mm_text(p)
                ),
            );
            let n = f.pins / 2;
            rule(
                (n - 1).max(0) * p + d <= f.body_length,
                "body_length",
                format!(
                    "{n} leads at {} do not fit along a body {} long.",
                    mm_text(p),
                    mm_text(f.body_length)
                ),
            );
            rule(
                f.row_pitch - f.body_width > 2 * DIP_LEAD_THICKNESS_NM,
                "row_pitch",
                format!(
                    "Rows {} apart leave no room for the lead bend outside a body {} wide.",
                    mm_text(f.row_pitch),
                    mm_text(f.body_width)
                ),
            );
            rule(
                !body_pitch_given,
                "body_pitch",
                "DIP leads have no separate pitch at the body; leave it 0.".into(),
            );
        }
        Construction::To => {
            rule(
                f.pins < 2 || p > d,
                "lead_pitch",
                format!(
                    "Leads of {} at a pitch of {} would touch.",
                    mm_text(d),
                    mm_text(p)
                ),
            );
            rule(
                (f.pins - 1) * f.body_pitch + d <= f.body_length,
                "body_length",
                format!(
                    "{} leads at {} do not leave the underside of a body {} long.",
                    f.pins,
                    mm_text(f.body_pitch),
                    mm_text(f.body_length)
                ),
            );
            rule(
                f.pins < 2 || f.body_pitch > d,
                "body_pitch",
                format!(
                    "Leads of {} touch where they leave the body at {}.",
                    mm_text(d),
                    mm_text(f.body_pitch)
                ),
            );
            rule(
                d <= f.body_width,
                "lead_diameter",
                format!(
                    "Leads of {} do not fit a body {} wide.",
                    mm_text(d),
                    mm_text(f.body_width)
                ),
            );
            rule(
                f.pins <= MAX_TO_PINS,
                "pins",
                format!("A TO row has at most {MAX_TO_PINS} leads."),
            );
            rule(
                !formed || f.pins == 3,
                "body_pitch",
                "Formed TO leads are made for three-lead packages (TO-92) only; give 0 for straight leads.".into(),
            );
        }
    }
    if formed {
        rule(
            s >= 2 * d,
            "standoff",
            format!(
                "Formed leads need a standoff of at least two lead diameters ({}) to bend in.",
                mm_text(2 * d)
            ),
        );
        rule(
            f.body_pitch < p,
            "body_pitch",
            "Leads formed inward (a pitch at the body above the board's) are not modelled.".into(),
        );
    }
    out
}

fn pads(d: &Dims) -> Vec<Pad> {
    let (drill, land) = d.drill_and_land();
    d.lead_points()
        .into_iter()
        .enumerate()
        .map(|(k, at)| {
            let key = (k + 1).to_string();
            Pad {
                terminal: key.clone(),
                key,
                shape: if k == 0 && d.square_pin_one() {
                    PadShape::RoundRect {
                        size: [land, land],
                        radius_nm: land / PIN_ONE_CORNER_SHARE,
                    }
                } else {
                    PadShape::Circle { diameter_nm: land }
                },
                at,
                rot_mdeg: 0,
                side: Side::Top,
                hole: Some(Hole {
                    shape: HoleShape::Round { diameter_nm: drill },
                    plated: true,
                }),
                mask: Opening::Default,
                paste: Opening::Default,
                number: None,
            }
        })
        .collect()
}

/// `v` thousandths of a millimetre.
fn um(v: i64) -> i64 {
    v * MM / 1000
}

/// A box with the first half of the pins down its left side and the rest
/// up its right, numbered and named by their numbers, on the symbol grid
/// (the host's box symbol).
fn box_symbol(pins: i64) -> Symbol {
    let step = draw::MODULE_NM;
    let left = (pins + 1) / 2;
    let rows = left.max(1);
    let top = ((rows - 1) / 2) * step;
    let min = [-2 * step, top - rows * step];
    let max = [2 * step, top + step];
    let mut d = Drawing::default();
    d.shape(Primitive::Rect { min, max }, draw::STROKE_NM, Fill::Body);
    text(
        &mut d,
        TextContent::Designator,
        [0, max[1] + draw::LABEL_GAP_NM],
        VAlign::Bottom,
    );
    text(
        &mut d,
        TextContent::ComponentName,
        [0, min[1] - draw::LABEL_GAP_NM],
        VAlign::Top,
    );
    for k in 0..pins {
        let (at, rot) = if k < left {
            ([-3 * step, top - k * step], RIGHT)
        } else {
            ([3 * step, top - (pins - 1 - k) * step], LEFT)
        };
        d.pins.push(SymbolPin {
            terminal: (k + 1).to_string(),
            at,
            length_nm: step,
            rot_mdeg: rot,
            show_name: true,
            show_number: true,
        });
    }
    Symbol {
        units: vec![d.unit("A")],
    }
}

/// A centred label of the symbol text height.
fn text(d: &mut Drawing, content: TextContent, at: Point2, v_align: VAlign) {
    d.shape(
        Primitive::Text {
            at,
            height_nm: draw::TEXT_HEIGHT_NM,
            content,
            h_align: HAlign::Center,
            v_align,
        },
        0,
        Fill::None,
    );
}

/// A two-lead part's drawing (the host's standard drawings): a resistor's
/// IEC rectangle or US zigzag; a diode's triangle and bar, terminal 1 the
/// cathode at the right; a polarised capacitor's straight + plate
/// (terminal 1, left) and curved − plate.
fn two_lead_symbol(symbol: Sym, resistor_style: &str) -> Symbol {
    let mut d = Drawing::default();
    match symbol {
        Sym::Resistor => {
            d.two_pins("1", "2", um(2500), um(2500));
            if resistor_style == "us" {
                let mut points = vec![[um(-2500), 0]];
                for k in 0..6i64 {
                    // x = −2.5 + 5 (2k + 1) / 12 mm
                    let x = um(-2500) + div_round(um(5000) * (2 * k + 1), 12).unwrap_or(0);
                    points.push([x, if k % 2 == 0 { um(1000) } else { um(-1000) }]);
                }
                points.push([um(2500), 0]);
                d.open(Primitive::Polyline { points });
            } else {
                d.shape(
                    Primitive::Rect {
                        min: [um(-2500), um(-1000)],
                        max: [um(2500), um(1000)],
                    },
                    draw::STROKE_NM,
                    Fill::Body,
                );
            }
            d.labels(um(1000), um(-1000));
        }
        Sym::Diode => {
            // Anode (terminal 2) left, cathode (terminal 1) right.
            d.pin("2", [-draw::TWO_TERMINAL_PIN_X_NM, 0], RIGHT, um(3750));
            d.pin("1", [draw::TWO_TERMINAL_PIN_X_NM, 0], LEFT, um(3750));
            d.shape(
                Primitive::Polygon {
                    points: vec![[um(-1250), um(1250)], [um(-1250), um(-1250)], [um(1250), 0]],
                },
                draw::STROKE_NM,
                Fill::Body,
            );
            d.line([um(1250), um(-1250)], [um(1250), um(1250)]);
            d.labels(um(1600), um(-1600));
        }
        Sym::Box => return box_symbol(2),
        Sym::PolarisedCapacitor => {
            d.two_pins("1", "2", um(500), um(500));
            d.plate([um(-500), um(-1750)], [um(-500), um(1750)]);
            // The − plate: an arc about (3, 0) mm through (0.5, 0) mm whose
            // ends reach 1.75 mm either side.
            let (centre, radius, reach) = (um(3000), um(2500), um(1750));
            let x = centre - hypot_leg(radius, reach);
            d.shape(
                Primitive::Arc {
                    center: [centre, 0],
                    start: [x, reach],
                    end: [x, -reach],
                    clockwise: false,
                },
                draw::PLATE_STROKE_NM,
                Fill::None,
            );
            // The + mark beside the + plate.
            d.line([um(-2250), um(1500)], [um(-1250), um(1500)]);
            d.line([um(-1750), um(1000)], [um(-1750), um(2000)]);
            d.labels(um(2000), um(-1750));
        }
    }
    Symbol {
        units: vec![d.unit("A")],
    }
}

/// `√(c² − a²)` to the nearest nanometre (a right triangle's other leg).
fn hypot_leg(c: i64, a: i64) -> i64 {
    let square = c * c - a * a;
    let root = square.max(0).isqrt();
    if square - root * root > root {
        root + 1
    } else {
        root
    }
}

/// The SHA-256 of each part file in the bundle (`parts/`), written by the
/// build script.
mod parts {
    include!(concat!(env!("OUT_DIR"), "/parts.rs"));
}

/// `n / d` to the nearest nanometre (the divisors here are small and
/// positive, so it never fails).
fn part_of(n: i64, d: i64) -> i64 {
    div_round(n, d).unwrap_or(0)
}

/// The rounds of a moulded block: its upright corners and top edges, kept
/// within half of `clear` (the room between the outermost leads and the
/// block's edge) and by the block's size.
fn block_rounds(d: &Dims, clear: i64) -> (i64, i64) {
    let short = d.body_length.min(d.body_width);
    let corner = CORNER_ROUND_NM
        .min(part_of(short * 8, 100))
        .min(part_of(clear, 2));
    let edge = EDGE_ROUND_NM.min(part_of(d.body_height, 10)).min(corner);
    (corner, edge)
}

/// The part's 3D model: one of the bundle's part files, by outline class,
/// with every parameter it takes. Every file shares the leads' values
/// (`lead_pitch`, `lead_diameter`, `lead_below`, `standoff`); each lead
/// root reaches `root` into its body, which is cut around it, and bent
/// leads take the centreline `bend_radius` (each file narrows it where a
/// segment is too short, as formed leads are).
///
/// - `tht-axial`: a round body lying on its standoff along x, each lead
///   leaving an end on the axis and bending down at the lead pitch.
/// - `tht-radial`: a round body standing up on straight round leads.
/// - `tht-radial-formed`: the same on square leads formed out from the
///   pitch at the body to the board's.
/// - `tht-dip`: a moulded block, strip leads leaving its long sides at
///   0.3 of its height and bending down at the rows, a row a body pattern.
/// - `tht-to`: a moulded block on square leads straight down, a pattern.
/// - `tht-to-formed`: the same with three leads, the outer two formed.
fn model(d: &Dims) -> v1::Model {
    let mut values = v1::Params::new();
    let mut set = |name: &str, v: i64| {
        values.insert(name.into(), ParamValue::Int(v));
    };
    set("lead_pitch", d.pitch);
    set("lead_diameter", d.d);
    set("lead_below", d.below);
    set("standoff", d.standoff);
    let asset = match d.construction {
        Construction::Axial => {
            set("body_diameter", d.body_diameter);
            set("body_length", d.body_length);
            set("bend_radius", d.bend);
            set("root", ROOT_NM.min(part_of(d.body_length, 4)));
            let round = END_ROUND_NM
                .min(part_of(d.body_diameter * 2, 10))
                .min(part_of(d.body_length, 6));
            set("end_round", round);
            parts::THT_AXIAL
        }
        Construction::Radial => {
            set("body_diameter", d.body_diameter);
            set("body_height", d.body_height);
            set("root", ROOT_NM.min(part_of(d.body_height, 4)));
            let round = END_ROUND_NM
                .min(part_of(d.body_diameter, 10))
                .min(part_of(d.body_height, 6));
            set("top_round", round);
            if d.formed() {
                set("body_pitch", d.body_pitch);
                set("bend_radius", d.bend);
                parts::THT_RADIAL_FORMED
            } else {
                parts::THT_RADIAL
            }
        }
        Construction::Dip => {
            let n = d.per_row();
            set("per_row", n);
            set("row_pitch", d.row_pitch);
            set("body_length", d.body_length);
            set("body_width", d.body_width);
            set("body_height", d.body_height);
            set("bend_radius", d.bend);
            set("lead_thickness", DIP_LEAD_THICKNESS_NM);
            set(
                "lead_exit",
                d.standoff + part_of(DIP_LEAD_EXIT_MILLI * d.body_height, 1000),
            );
            set("root", ROOT_NM.min(part_of(d.body_width, 6)));
            let row = (n - 1) * d.pitch + d.d;
            let (corner, edge) = block_rounds(d, part_of(d.body_length - row, 2));
            set("corner_round", corner);
            set("edge_round", edge);
            parts::THT_DIP
        }
        Construction::To => {
            set("body_length", d.body_length);
            set("body_width", d.body_width);
            set("body_height", d.body_height);
            set("root", ROOT_NM.min(part_of(d.body_height, 4)));
            let row = (d.pins - 1) * d.body_pitch + d.d;
            let clear = part_of(d.body_length - row, 2)
                .min(part_of(d.body_width - d.d, 2))
                .max(0);
            let (corner, edge) = block_rounds(d, clear);
            set("corner_round", corner);
            set("edge_round", edge);
            if d.formed() {
                set("body_pitch", d.body_pitch);
                set("bend_radius", d.bend);
                parts::THT_TO_FORMED
            } else {
                set("pins", d.pins);
                parts::THT_TO
            }
        }
    };
    v1::Model::PartFile {
        asset: asset.into(),
        values,
    }
}

/// The component's default name.
fn name(d: &Dims) -> String {
    let mm = |v: i64| String::from(mm_text(v).trim_end_matches(" mm"));
    match d.construction {
        Construction::Axial => format!(
            "Axial Ø{} × {} mm, leads at {} mm",
            mm(d.body_diameter),
            mm(d.body_length),
            mm(d.pitch)
        ),
        Construction::Radial => format!(
            "Radial Ø{} × {} mm, leads at {} mm",
            mm(d.body_diameter),
            mm(d.body_height),
            mm(d.pitch)
        ),
        Construction::Dip => format!("DIP-{}, rows {} mm apart", d.pins, mm(d.row_pitch)),
        Construction::To => format!("TO, {} leads at {} mm", d.pins, mm(d.pitch)),
    }
}

/// The component's terminals: a diode's cathode `K` (terminal 1) and anode
/// `A`; otherwise named by their numbers.
fn terminals(d: &Dims) -> Vec<Terminal> {
    (1..=d.pins)
        .map(|k| Terminal {
            key: k.to_string(),
            name: match (d.symbol, k) {
                (Sym::Diode, 1) if d.pins == 2 => "K".into(),
                (Sym::Diode, _) if d.pins == 2 => "A".into(),
                _ => k.to_string(),
            },
            direction: Direction::Passive,
        })
        .collect()
}

fn generate(request: &Request) -> Result<Generation, Vec<Problem>> {
    let description = description_with(Vec::new());
    let args = checked(&description, &request.params)?;
    let d = dims(&args)?;
    let component_part = request.scope == Scope::Component;
    if component_part && d.symbol != Sym::Box && d.pins != 2 {
        return Err(vec![inconsistent(
            "symbol",
            format!(
                "A {} symbol needs two leads; this package has {}.",
                Sym::ALL
                    .iter()
                    .find(|(s, _, _)| *s == d.symbol)
                    .map_or("", |(_, _, l)| l)
                    .to_lowercase(),
                d.pins
            ),
        )]);
    }
    let pads = pads(&d);
    let envelope = d.envelope();
    let body = Box2::centred([0, 0], [envelope[0], envelope[1]]);
    let courtyard = land::courtyard(&pads, Some(body)).unwrap_or_default();
    let top = courtyard.iter().map(|p| p[1]).max().unwrap_or(body.max[1]);
    let mut silk = land::silk_outline(&pads, body, land::DEFAULT_MASK_NM);
    silk.push(land::silk_designator(body, top));
    if (d.pins > 2 || d.symbol.polarised())
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
    let keys: Vec<String> = (1..=d.pins).map(|k| k.to_string()).collect();
    let package = Package {
        terminals: keys.clone(),
        geometry: PackageGeometry {
            // Leads below the seating plane are not part of the envelope.
            envelope_nm: envelope,
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
        keepouts: Vec::new(),
    };
    let symbol = if d.symbol == Sym::Box {
        box_symbol(d.pins)
    } else {
        two_lead_symbol(d.symbol, args.text("resistor_style").map_err(|p| vec![p])?)
    };
    Ok(Generation {
        marking: None,
        component: component_part.then(|| Component {
            terminals: terminals(&d),
            prefix: d.symbol.prefix().into(),
            name: name(&d),
            fields: Vec::new(),
        }),
        symbol: component_part.then_some(symbol),
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
            "Drills and lands are estimated from the lead (drill: its largest cross-section + 0.3 mm); \
             confirm them against the part's recommended hole and land sizes."
                .into(),
        ],
    })
}

impl Generator for Tht {
    fn describe() -> Description {
        description()
    }

    fn generate(request: &Request) -> Result<Generation, Vec<Problem>> {
        generate(request)
    }
}

#[cfg(test)]
mod tests;
