//! The wire-to-board generator: surface-mount shrouded headers of one row
//! of contacts at a pitch with a metal mounting tab at each end, side entry
//! (the mating face square to the board) or top entry (the mating face up),
//! as a symbol, package, footprint and 3D model.
//!
//! Frame: the origin is the centre of the signal land row; pins run along
//! x, pin 1 at −x; the contacts' tails leave the shroud towards +y and the
//! mounting tabs and the mating face (side entry) lie towards −y. z = 0 is
//! the seating plane.
//!
//! Lands are the connector maker's recommended PCB layout, given as
//! parameters (the land size, the tab lands' size, their gap beyond the
//! outer pin and their offset from the signal row), never estimated. The
//! body is placed on the lands by the drawing's offsets: `front` puts the
//! shroud face away from the tails, and `lead_reach` the tails' tips past
//! the shroud's rear, so the 3D model's lead tips stand on the lands. The
//! presets cite the makers' public catalogue drawings; what a drawing does
//! not give (wall, cavity, contact section, tab thickness) is estimated,
//! as each preset says.
#![cfg_attr(target_arch = "wasm32", no_std)]

extern crate alloc;

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use sparksmith_generator_sdk::args::{Args, checked, problem};
use sparksmith_generator_sdk::draw::{self, Drawing, RIGHT, UP};
use sparksmith_generator_sdk::land::{self, Box2};
use sparksmith_generator_sdk::units::{MM, UM, div_round};
use sparksmith_generator_sdk::v1::{
    self, BindingEntry, Capability, Choice, Component, Description, Direction, Effect, Fill,
    Footprint, Generation, Grammar, Graphic, Lead, Opening, Package, PackageGeometry, Pad,
    PadShape, Param, ParamType, ParamValue, PartOffer, Preset, Primitive, Problem, Request, Rule,
    Scope, Side, Symbol, SymbolPin, Terminal,
};
use sparksmith_generator_sdk::{Generator, export_generator};

/// The starter rows, in nm.
const PRESETS: &str = include_str!("../presets.json");

/// The generator's version (`Description::version`).
/// 4: the SDK's pin table (`pin_table`) and part numbers
/// (`part_numbers`).
const VERSION: u32 = 4;

/// Most contacts in the row.
const MAX_PINS: i64 = 20;

/// The polarising grooves in the cavity's floor (side entry) or rear wall
/// (top entry): this wide, at most 0.4 of the pitch, and half the wall
/// deep (est.).
const KEY_WIDTH_NM: i64 = 400_000;

/// Rounds of the shroud's upright corners (est.), at most a fifth of the
/// wall beside the cavity.
const CORNER_ROUND_NM: i64 = 200_000;

/// A tab's foot stops this far inside its land's outer edge (est.).
const FOOT_INSET_NM: i64 = 100_000;

/// The pin-1 dot on the silkscreen.
const PIN_ONE_DOT_RADIUS_NM: i64 = 150_000;

/// Pin numbers on the symbol (the header generator's rule).
const PIN_NUMBER_ADVANCE_NM: i64 = 900_000;
const PIN_NUMBER_CLEARANCE_NM: i64 = 400_000;

pub struct WireToBoard;

export_generator!(WireToBoard);

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

fn choice(key: &str, label: &str) -> Choice {
    Choice {
        key: key.into(),
        label: label.into(),
    }
}

fn pick(key: &str, label: &str, choices: Vec<Choice>, default: &str, group: &str) -> Param {
    Param {
        key: key.into(),
        label: label.into(),
        help: None,
        ty: ParamType::Enum { choices },
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

/// The description, with `presets`. The defaults are a 4-way 1.0 mm side
/// entry header (the SH-style starter row).
fn description_with(presets: Vec<Preset>) -> Description {
    let params = vec![
        Param {
            key: "pins".into(),
            label: "Contacts".into(),
            help: None,
            ty: ParamType::Count {
                min: 2,
                max: MAX_PINS,
            },
            default: ParamValue::Int(4),
            group: Some("pins".into()),
            context: None,
        },
        length("pitch", "Pitch", 500 * UM, 5 * MM, MM, "pins"),
        pick(
            "entry",
            "Entry",
            vec![
                choice("side", "Side entry (mating face square to the board)"),
                choice("top", "Top entry (mating face up)"),
            ],
            "side",
            "pins",
        ),
        pick(
            "colour",
            "Shroud colour",
            vec![
                choice("natural", "Natural (ivory)"),
                choice("white", "White"),
                choice("black", "Black"),
            ],
            "natural",
            "shroud",
        ),
        length(
            "body_length",
            "Shroud length (along the pins, tabs included)",
            MM,
            100 * MM,
            6 * MM,
            "shroud",
        ),
        length(
            "body_depth",
            "Shroud depth (across the pins)",
            MM,
            30 * MM,
            4_250_000,
            "shroud",
        ),
        length(
            "body_height",
            "Height above the seating plane",
            MM,
            30 * MM,
            2_900_000,
            "shroud",
        ),
        length("standoff", "Standoff", 0, MM, 50_000, "shroud"),
        length(
            "front",
            "Signal lands' centre to the shroud face away from the tails",
            0,
            30 * MM,
            4_575_000,
            "shroud",
        ),
        length(
            "cavity_width",
            "Mating cavity width",
            500 * UM,
            100 * MM,
            5_100_000,
            "shroud",
        ),
        length(
            "cavity_height",
            "Mating cavity height (side entry) or depth across the pins (top entry)",
            200 * UM,
            30 * MM,
            2_350_000,
            "shroud",
        ),
        length(
            "cavity_depth",
            "Mating cavity depth from the mating face",
            200 * UM,
            30 * MM,
            3_500_000,
            "shroud",
        ),
        length(
            "floor",
            "Wall under the cavity",
            50 * UM,
            10 * MM,
            300_000,
            "shroud",
        ),
        length(
            "latch_width",
            "Latch window width",
            100 * UM,
            100 * MM,
            3 * MM,
            "shroud",
        ),
        length(
            "latch_length",
            "Latch window length from the mating face",
            100 * UM,
            30 * MM,
            MM,
            "shroud",
        ),
        length(
            "lead_width",
            "Contact width",
            50 * UM,
            3 * MM,
            300_000,
            "contacts",
        ),
        length(
            "lead_thickness",
            "Contact thickness",
            50 * UM,
            2 * MM,
            150_000,
            "contacts",
        ),
        length(
            "lead_reach",
            "Tail tips past the shroud's rear",
            0,
            10 * MM,
            700_000,
            "contacts",
        ),
        length(
            "post_length",
            "Contact length in the cavity",
            200 * UM,
            30 * MM,
            2 * MM,
            "contacts",
        ),
        length(
            "land_width",
            "Signal land width",
            100 * UM,
            5 * MM,
            600_000,
            "lands",
        ),
        length(
            "land_length",
            "Signal land length",
            100 * UM,
            10 * MM,
            1_550_000,
            "lands",
        ),
        length(
            "tab_land_width",
            "Tab land width (along the pins)",
            100 * UM,
            10 * MM,
            1_200_000,
            "lands",
        ),
        length(
            "tab_land_length",
            "Tab land length (across the pins)",
            100 * UM,
            10 * MM,
            1_800_000,
            "lands",
        ),
        length(
            "tab_land_gap",
            "Outer pin's centre to the tab land's inner edge",
            0,
            10 * MM,
            700_000,
            "lands",
        ),
        length(
            "tab_land_offset",
            "Signal lands' centre to the tab lands' centre (towards the front)",
            0,
            30 * MM,
            3_875_000,
            "lands",
        ),
        length(
            "tab_thickness",
            "Tab sheet thickness",
            50 * UM,
            MM,
            150_000,
            "tabs",
        ),
        length(
            "tab_height",
            "Tab height",
            200 * UM,
            30 * MM,
            2 * MM,
            "tabs",
        ),
    ];
    let rules = vec![
        Rule {
            text: "Contact and land width < pitch".into(),
            fields: vec!["lead_width".into(), "land_width".into(), "pitch".into()],
        },
        Rule {
            text: "The contact row fits the cavity, the cavity the shroud".into(),
            fields: vec![
                "pins".into(),
                "pitch".into(),
                "cavity_width".into(),
                "body_length".into(),
            ],
        },
        Rule {
            text: "The tails' tips stand on their lands".into(),
            fields: vec![
                "front".into(),
                "body_depth".into(),
                "lead_reach".into(),
                "land_length".into(),
            ],
        },
        Rule {
            text: "Each tab stands in the shroud's end wall, beside the cavity".into(),
            fields: vec![
                "tab_land_gap".into(),
                "tab_land_width".into(),
                "tab_thickness".into(),
                "cavity_width".into(),
            ],
        },
    ];
    let offer = PartOffer {
        families: vec!["wire_to_board".into()],
        scopes: vec![Scope::Component, Scope::Package],
        grammar: vec![Grammar {
            scope: Scope::Component,
            patterns: vec![
                "wire to board {pins} {pitch:length}?".into(),
                "shrouded header {pins} {pitch:length}? smd".into(),
            ],
            examples: vec![
                "wire to board 4 1mm".into(),
                "shrouded header 2 2mm smd".into(),
            ],
        }],
        instance_fields: Vec::new(),
    };
    Description {
        abi: v1::ABI,
        id: "wire-to-board".into(),
        version: VERSION,
        title: "Wire-to-board SMD headers".into(),
        licence: "MIT OR Apache-2.0".into(),
        deterministic: true,
        capabilities: vec![Capability {
            name: v1::GENERATE_PART.into(),
            effect: Effect::None,
            title: Some("Make a wire-to-board header".into()),
            params,
            rules,
            presets,
            part: Some(offer),
        }],
        needs: Vec::new(),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Entry {
    Side,
    Top,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Colour {
    Natural,
    White,
    Black,
}

/// A header's values, checked.
struct Dims {
    pins: i64,
    pitch: i64,
    entry: Entry,
    colour: Colour,
    body_length: i64,
    body_depth: i64,
    body_height: i64,
    standoff: i64,
    front: i64,
    cavity_width: i64,
    cavity_height: i64,
    cavity_depth: i64,
    floor: i64,
    latch_width: i64,
    latch_length: i64,
    lead_width: i64,
    lead_thickness: i64,
    lead_reach: i64,
    post_length: i64,
    land: [i64; 2],
    tab_land: [i64; 2],
    tab_land_gap: i64,
    tab_land_offset: i64,
    tab_thickness: i64,
    tab_height: i64,
}

/// `(i − (n − 1) / 2) × pitch`, to the nearest nanometre.
fn centred(i: i64, n: i64, pitch: i64) -> i64 {
    div_round((2 * i - (n - 1)) * pitch, 2).unwrap_or(0)
}

fn half(v: i64) -> i64 {
    div_round(v, 2).unwrap_or(0)
}

impl Dims {
    /// The shroud face away from the tails (the mating face, side entry).
    fn y_front(&self) -> i64 {
        -self.front
    }
    /// The shroud face the tails leave by.
    fn y_rear(&self) -> i64 {
        self.body_depth - self.front
    }
    /// The tails' tips.
    fn tip_y(&self) -> i64 {
        self.y_rear() + self.lead_reach
    }
    /// The outer pins' centre, from the axis.
    fn outer_pin(&self) -> i64 {
        half((self.pins - 1) * self.pitch)
    }
    /// The tab lands' centre, from the axis.
    fn tab_x(&self) -> i64 {
        self.outer_pin() + self.tab_land_gap + half(self.tab_land[0])
    }
    /// A tab's foot reaches this far out along x.
    fn foot_x(&self) -> i64 {
        self.tab_x() + half(self.tab_land[0]) - FOOT_INSET_NM
    }
    /// A tab's foot and plate span these y: inside its land and under the
    /// shroud, clear of its rounded corners.
    fn tab_y(&self) -> [i64; 2] {
        let inset = FOOT_INSET_NM.min(self.tab_land[1] / 4);
        let clear = 2 * self.corner_round();
        [
            (-self.tab_land_offset - half(self.tab_land[1]) + inset).max(self.y_front() + clear),
            (-self.tab_land_offset + half(self.tab_land[1]) - inset).min(self.y_rear() - clear),
        ]
    }
    /// The shroud outline, footprint frame.
    fn body(&self) -> Box2 {
        Box2 {
            min: [-half(self.body_length), self.y_front()],
            max: [half(self.body_length), self.y_rear()],
        }
    }
    /// The cavity's floor: over the bottom wall (side entry), or
    /// `cavity_depth` under the top face (top entry).
    fn cavity_bottom(&self) -> i64 {
        match self.entry {
            Entry::Side => self.standoff + self.floor,
            Entry::Top => self.body_height - self.cavity_depth,
        }
    }
    /// Side entry: the cavity's top, under the roof.
    fn cavity_top(&self) -> i64 {
        self.cavity_bottom() + self.cavity_height
    }
    /// Where a tail turns down (side entry) or along the board (top entry),
    /// and its bend's centreline radius.
    fn bend_y(&self) -> i64 {
        self.y_rear() + self.lead_thickness
    }
    fn post_z(&self) -> i64 {
        // Side entry: the contact runs at the cavity's mid-height.
        self.cavity_bottom() + half(self.cavity_height)
    }
    /// Top entry: the posts' line across the pins, the middle of the
    /// shroud (and of its cavity), and their top.
    fn post_y(&self) -> i64 {
        self.y_front() + half(self.body_depth)
    }
    fn post_top(&self) -> i64 {
        self.cavity_bottom() + self.post_length
    }
    /// The bends' centreline radius: the contact's thickness, less where a
    /// segment cannot hold it.
    fn bend_r(&self) -> i64 {
        let t = self.lead_thickness;
        let h = half(t);
        let segments = match self.entry {
            Entry::Side => [self.post_z() - h, self.tip_y() - self.bend_y()],
            Entry::Top => [self.post_top() - h, self.tip_y() - self.post_y()],
        };
        let room = segments.iter().map(|s| s * 2 / 5).min().unwrap_or(0);
        t.min(room)
    }
    /// The key grooves: width and depth.
    fn key(&self) -> [i64; 2] {
        [KEY_WIDTH_NM.min(self.pitch * 2 / 5), half(self.floor)]
    }
    fn corner_round(&self) -> i64 {
        CORNER_ROUND_NM.min((self.body_length - self.cavity_width) / 10)
    }
    /// Where a tail touches the board: the middle of its foot on the land.
    fn contact_y(&self) -> i64 {
        let from = match self.entry {
            Entry::Side => self.bend_y(),
            Entry::Top => self.post_y(),
        };
        let lo = from.max(-half(self.land[1]));
        let hi = self.tip_y().min(half(self.land[1]));
        half(lo + hi)
    }
    /// The highest point of the part.
    fn top(&self) -> i64 {
        self.body_height
    }
}

fn mm(nm: i64) -> String {
    let whole = nm / MM;
    let mut frac = format!("{:06}", (nm % MM).abs());
    while frac.len() > 1 && frac.ends_with('0') {
        frac.pop();
    }
    format!("{whole}.{frac}")
}

fn dims(args: &Args<'_>) -> Result<Dims, Vec<Problem>> {
    let one = |p: Problem| vec![p];
    let int = |k: &str| args.int(k).map_err(one);
    let d = Dims {
        pins: int("pins")?,
        pitch: int("pitch")?,
        entry: match args.text("entry").map_err(one)? {
            "top" => Entry::Top,
            _ => Entry::Side,
        },
        colour: match args.text("colour").map_err(one)? {
            "white" => Colour::White,
            "black" => Colour::Black,
            _ => Colour::Natural,
        },
        body_length: int("body_length")?,
        body_depth: int("body_depth")?,
        body_height: int("body_height")?,
        standoff: int("standoff")?,
        front: int("front")?,
        cavity_width: int("cavity_width")?,
        cavity_height: int("cavity_height")?,
        cavity_depth: int("cavity_depth")?,
        floor: int("floor")?,
        latch_width: int("latch_width")?,
        latch_length: int("latch_length")?,
        lead_width: int("lead_width")?,
        lead_thickness: int("lead_thickness")?,
        lead_reach: int("lead_reach")?,
        post_length: int("post_length")?,
        land: [int("land_width")?, int("land_length")?],
        tab_land: [int("tab_land_width")?, int("tab_land_length")?],
        tab_land_gap: int("tab_land_gap")?,
        tab_land_offset: int("tab_land_offset")?,
        tab_thickness: int("tab_thickness")?,
        tab_height: int("tab_height")?,
    };
    let mut problems = Vec::new();
    let mut refuse = |field: &str, message: String| {
        problems.push(problem("param.inconsistent", field, message));
    };
    if d.lead_width >= d.pitch || d.land[0] >= d.pitch {
        refuse(
            "pitch",
            format!(
                "Contacts of {} mm on lands of {} mm do not fit a pitch of {} mm.",
                mm(d.lead_width),
                mm(d.land[0]),
                mm(d.pitch)
            ),
        );
    }
    if (d.pins - 1) * d.pitch + d.lead_width >= d.cavity_width {
        refuse(
            "cavity_width",
            format!(
                "{} contacts at {} mm do not fit a cavity {} mm wide.",
                d.pins,
                mm(d.pitch),
                mm(d.cavity_width)
            ),
        );
    }
    if d.cavity_width >= d.body_length {
        refuse(
            "body_length",
            format!(
                "A cavity {} mm wide does not fit a shroud {} mm long.",
                mm(d.cavity_width),
                mm(d.body_length)
            ),
        );
    }
    // Each tab stands in its end wall, clear of the cavity and inside the
    // shroud.
    let tab_in = d.tab_x() - half(d.tab_thickness);
    let tab_out = d.tab_x() + half(d.tab_thickness);
    if tab_in <= half(d.cavity_width) || tab_out >= half(d.body_length) {
        refuse(
            "tab_land_gap",
            format!(
                "A tab at {} mm from the axis does not stand in the shroud's end wall ({} to {} mm).",
                mm(d.tab_x()),
                mm(half(d.cavity_width)),
                mm(half(d.body_length))
            ),
        );
    }
    let [tab_y0, tab_y1] = d.tab_y();
    if 2 * (tab_y1 - tab_y0) < d.tab_land[1] {
        refuse(
            "tab_land_offset",
            format!(
                "The tab lands ({} mm from the signal lands) do not lie under the shroud's ends.",
                mm(d.tab_land_offset)
            ),
        );
    }
    // The tails' tips stand on their lands, past the shroud.
    let tip = d.tip_y();
    if tip > half(d.land[1]) || tip < -half(d.land[1]) {
        refuse(
            "lead_reach",
            format!(
                "The tails end {} mm from the signal lands' centre, off lands {} mm long.",
                mm(tip),
                mm(d.land[1])
            ),
        );
    }
    let t = d.lead_thickness;
    let foot = match d.entry {
        Entry::Side => tip - d.bend_y(),
        Entry::Top => tip - d.post_y(),
    };
    if foot < 2 * t {
        refuse(
            "lead_reach",
            format!(
                "The tails' feet ({} mm) are shorter than two contact thicknesses.",
                mm(foot)
            ),
        );
    }
    if d.bend_r() * 5 < t * 3 {
        refuse(
            "lead_thickness",
            format!(
                "Contacts {} mm thick cannot bend within the shroud's height.",
                mm(t)
            ),
        );
    }
    match d.entry {
        Entry::Side => {
            if d.standoff + d.floor + d.cavity_height >= d.body_height {
                refuse(
                    "cavity_height",
                    format!(
                        "A cavity {} mm high over a {} mm floor leaves no roof under a {} mm shroud.",
                        mm(d.cavity_height),
                        mm(d.floor),
                        mm(d.body_height)
                    ),
                );
            }
            if d.cavity_depth + 2 * t >= d.body_depth || d.post_length >= d.cavity_depth {
                refuse(
                    "cavity_depth",
                    format!(
                        "A cavity {} mm deep (contacts {} mm into it) leaves no rear wall in a {} mm shroud.",
                        mm(d.cavity_depth),
                        mm(d.post_length),
                        mm(d.body_depth)
                    ),
                );
            }
            if d.latch_length >= d.cavity_depth || d.latch_width >= d.cavity_width {
                refuse(
                    "latch_width",
                    format!(
                        "A latch window {} × {} mm does not fit the cavity's roof.",
                        mm(d.latch_width),
                        mm(d.latch_length)
                    ),
                );
            }
            if d.post_z() + half(t) >= d.cavity_top() {
                refuse(
                    "cavity_height",
                    "The contacts do not fit the cavity's height.".into(),
                );
            }
        }
        Entry::Top => {
            if d.cavity_height + 2 * t >= d.body_depth {
                refuse(
                    "cavity_height",
                    format!(
                        "A cavity {} mm across the pins leaves no walls in a {} mm shroud.",
                        mm(d.cavity_height),
                        mm(d.body_depth)
                    ),
                );
            }
            if d.cavity_depth + d.floor + d.standoff >= d.body_height {
                refuse(
                    "cavity_depth",
                    format!(
                        "A cavity {} mm deep over a {} mm floor does not fit a {} mm shroud.",
                        mm(d.cavity_depth),
                        mm(d.floor),
                        mm(d.body_height)
                    ),
                );
            }
            if d.post_top() >= d.body_height {
                refuse(
                    "post_length",
                    format!(
                        "Contacts {} mm above the cavity's floor stand over the {} mm shroud.",
                        mm(d.post_length),
                        mm(d.body_height)
                    ),
                );
            }
            if d.latch_length >= d.body_height - d.cavity_bottom()
                || d.latch_width >= d.cavity_width
            {
                refuse(
                    "latch_width",
                    format!(
                        "A latch window {} × {} mm does not fit the front wall.",
                        mm(d.latch_width),
                        mm(d.latch_length)
                    ),
                );
            }
        }
    }
    if d.tab_height >= d.body_height {
        refuse(
            "tab_height",
            format!(
                "Tabs {} mm high stand over the {} mm shroud.",
                mm(d.tab_height),
                mm(d.body_height)
            ),
        );
    }
    if problems.is_empty() {
        Ok(d)
    } else {
        Err(problems)
    }
}

fn pads(d: &Dims) -> Vec<Pad> {
    let mut pads: Vec<Pad> = (0..d.pins)
        .map(|k| {
            let key = (k + 1).to_string();
            Pad {
                terminal: key.clone(),
                key,
                shape: PadShape::Rect { size: d.land },
                at: [centred(k, d.pins, d.pitch), 0],
                rot_mdeg: 0,
                side: Side::Top,
                hole: None,
                mask: Opening::Default,
                paste: Opening::Default,
                number: None,
                kind: None,
            }
        })
        .collect();
    for (key, sign) in [("MP1", -1), ("MP2", 1)] {
        pads.push(Pad {
            key: key.into(),
            terminal: key.into(),
            shape: PadShape::Rect { size: d.tab_land },
            at: [sign * d.tab_x(), -d.tab_land_offset],
            rot_mdeg: 0,
            side: Side::Top,
            hole: None,
            mask: Opening::Default,
            paste: Opening::Default,
            number: None,
            kind: None,
        });
    }
    pads
}

/// A narrow body with the contacts down its left side in pin order and the
/// mounting tabs' pin under it, on the symbol grid (the header generator's
/// drawing).
fn symbol(pins: i64) -> Symbol {
    let module = draw::MODULE_NM;
    let mut d = Drawing::default();
    let bottom = -(pins + 1) * module;
    d.shape(
        Primitive::Rect {
            min: [-module, bottom],
            max: [module, module],
        },
        draw::STROKE_NM,
        Fill::Body,
    );
    d.labels(module, bottom);
    for k in 0..pins {
        let number = (k + 1).to_string();
        let digits = i64::try_from(number.len()).unwrap_or(i64::MAX);
        let need = digits * PIN_NUMBER_ADVANCE_NM + 2 * PIN_NUMBER_CLEARANCE_NM;
        let length = if need > module {
            (need + module - 1) / module * module
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
    d.pins.push(SymbolPin {
        terminal: "MP".into(),
        at: [0, bottom - module],
        length_nm: module,
        rot_mdeg: UP,
        show_name: true,
        show_number: false,
    });
    Symbol {
        units: vec![d.unit("A")],
    }
}

/// The SHA-256 of each part file in the bundle (`parts/`), written by the
/// build script.
mod parts {
    include!(concat!(env!("OUT_DIR"), "/parts.rs"));
}

/// The 3D model: the bundle's part file for the entry and shroud colour,
/// every value given (the part file builds; this module only measures).
fn model(d: &Dims) -> v1::Model {
    let mut values = v1::Params::new();
    let mut set = |name: &str, v: i64| {
        values.insert(name.into(), ParamValue::Int(v));
    };
    let [tab_y0, tab_y1] = d.tab_y();
    let [key_width, key_depth] = d.key();
    set("pins", d.pins);
    set("pitch", d.pitch);
    set("body_length", d.body_length);
    set("y_front", d.y_front());
    set("y_rear", d.y_rear());
    set("standoff", d.standoff);
    set("top", d.body_height);
    set("cavity_width", d.cavity_width);
    set("cavity_bottom", d.cavity_bottom());
    set("latch_width", d.latch_width);
    set("key_width", key_width);
    set("key_depth", key_depth);
    set("lead_width", d.lead_width);
    set("lead_thickness", d.lead_thickness);
    set("tip_y", d.tip_y());
    set("bend_r", d.bend_r());
    set("tab_x", d.tab_x());
    set("tab_thickness", d.tab_thickness);
    set("tab_y0", tab_y0);
    set("tab_y1", tab_y1);
    set("tab_height", d.tab_height);
    set("foot_x", d.foot_x());
    set("corner_round", d.corner_round());
    match d.entry {
        Entry::Side => {
            set("cavity_top", d.cavity_top());
            set("cavity_back", d.y_front() + d.cavity_depth);
            set("latch_back", d.y_front() + d.latch_length);
            set("post_tip", d.y_front() + d.cavity_depth - d.post_length);
            set("post_z", d.post_z());
            set("bend_y", d.bend_y());
        }
        Entry::Top => {
            let y0 = d.post_y() - half(d.cavity_height);
            set("cavity_y0", y0);
            set("cavity_y1", y0 + d.cavity_height);
            set("latch_bottom", d.body_height - d.latch_length);
            set("post_y", d.post_y());
            set("post_top", d.post_top());
        }
    }
    let asset = match (d.entry, d.colour) {
        (Entry::Side, Colour::Natural) => parts::WTB_SIDE_NATURAL,
        (Entry::Side, Colour::White) => parts::WTB_SIDE_WHITE,
        (Entry::Side, Colour::Black) => parts::WTB_SIDE_BLACK,
        (Entry::Top, Colour::Natural) => parts::WTB_TOP_NATURAL,
        (Entry::Top, Colour::White) => parts::WTB_TOP_WHITE,
        (Entry::Top, Colour::Black) => parts::WTB_TOP_BLACK,
    };
    v1::Model::PartFile {
        asset: asset.into(),
        values,
    }
}

/// The smallest box centred on the origin holding the part (the host's
/// envelope check), x, y.
fn envelope_xy(d: &Dims) -> [i64; 2] {
    let x = half(d.body_length).max(d.foot_x());
    let y = d.front.max(d.tip_y().abs()).max(d.y_rear().abs());
    [2 * x, 2 * y]
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
    // Pin 1: a dot past pad 1's tail end, clear of its mask opening.
    silk.push(Graphic {
        key: "pin1".into(),
        shape: Primitive::Circle {
            center: [
                centred(0, d.pins, d.pitch),
                half(d.land[1])
                    + land::DEFAULT_MASK_NM
                    + land::SILK_PAD_CLEARANCE_NM
                    + 2 * PIN_ONE_DOT_RADIUS_NM,
            ],
            radius_nm: PIN_ONE_DOT_RADIUS_NM,
        },
        stroke_nm: 0,
        fill: Fill::Solid,
    });
    let mut keys: Vec<String> = (1..=d.pins).map(|k| k.to_string()).collect();
    keys.push("MP1".into());
    keys.push("MP2".into());
    let [ex, ey] = envelope_xy(&d);
    let contact_y = d.contact_y();
    let [tab_y0, tab_y1] = d.tab_y();
    let foot_mid = half(d.tab_x() - half(d.tab_thickness) + d.foot_x());
    let leads: Vec<Lead> = pads
        .iter()
        .map(|p| {
            let contact = if p.key.starts_with("MP") {
                let sign = if p.at[0] < 0 { -1 } else { 1 };
                [sign * foot_mid, half(tab_y0 + tab_y1), 0]
            } else {
                [p.at[0], contact_y, 0]
            };
            Lead {
                key: p.key.clone(),
                contact,
            }
        })
        .collect();
    let package = Package {
        terminals: keys.clone(),
        geometry: PackageGeometry {
            envelope_nm: [ex, ey, d.top()],
            seating_nm: 0,
            leads,
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
    let entry = match d.entry {
        Entry::Side => "side entry",
        Entry::Top => "top entry",
    };
    let name = format!(
        "Wire-to-board header {} way {} mm {entry}",
        d.pins,
        mm(d.pitch)
    );
    let mut terminals: Vec<Terminal> = (1..=d.pins)
        .map(|k| Terminal {
            key: k.to_string(),
            name: k.to_string(),
            direction: Direction::Passive,
            alternates: Vec::new(),
            drive: None,
        })
        .collect();
    terminals.push(Terminal {
        key: "MP".into(),
        name: "MP".into(),
        direction: Direction::Passive,
        alternates: Vec::new(),
        drive: None,
    });
    Ok(Generation {
        orderable: Vec::new(),
        marking: None,
        component: component_part.then(|| Component {
            terminals,
            prefix: "J".into(),
            name,
            fields: Vec::new(),
        }),
        symbol: component_part.then(|| symbol(d.pins)),
        package,
        footprint,
        binding: if component_part {
            keys.iter()
                .map(|k| BindingEntry {
                    package_terminal: k.clone(),
                    terminal: if k.starts_with("MP") {
                        "MP".into()
                    } else {
                        k.clone()
                    },
                })
                .collect()
        } else {
            Vec::new()
        },
        model: Some(model(&d)),
        fields: Vec::new(),
        notes: vec![
            "Lands are the land parameters as given: the maker's recommended layout only where they were taken from it. The cavity, walls, contact section and tab sheet are estimated where the drawing gives none."
                .into(),
        ],
    })
}

impl Generator for WireToBoard {
    fn describe() -> Description {
        description()
    }

    fn generate(request: &Request) -> Result<Generation, Vec<Problem>> {
        generate(request)
    }
}

#[cfg(test)]
mod tests;
