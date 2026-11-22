//! The board-to-board generator: surface-mount dual-row terminal strips
//! (square posts standing in a moulded insulator, their tails leaving its
//! bottom and turning out along the board, one row each side), optionally
//! with two alignment pins under the insulator, as a symbol, package,
//! footprint and 3D model.
//!
//! Frame: the origin is the strip's centre; positions run along x, pin 1 at
//! −x in the −y row, pin 2 beside it in the +y row (odd pins −y, even +y).
//! z = 0 is the seating plane; the insulator stands on its standoff over
//! the tails, the alignment pins reach under the seating plane into their
//! holes.
//!
//! Lands are the maker's recommended layout as parameters (width, length
//! and the outer edges' span), never estimated by the module; the tails'
//! span puts their tips on them. Posts are gold-plated, tails and
//! alignment pins tin-plated (the usual selective plating). The presets cite
//! the makers' public drawings.
#![cfg_attr(target_arch = "wasm32", no_std)]

extern crate alloc;

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use sparksmith_generator_sdk::args::{Args, checked, problem};
use sparksmith_generator_sdk::draw::{self, Drawing, LEFT, RIGHT};
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

/// Most positions per row.
const MAX_POSITIONS: i64 = 50;

/// The insulator's upright corners' round (est.).
const CORNER_ROUND_NM: i64 = 100_000;

/// An alignment pin's hole: the pin's diameter plus this, unplated and
/// landing no terminal (Samtec's TSM -A layout: a 0.062" pin in a 0.068"
/// non-plated hole).
const PEG_HOLE_EXTRA_NM: i64 = 130_000;

/// The pin-1 dot on the silkscreen.
const PIN_ONE_DOT_RADIUS_NM: i64 = 150_000;

pub struct BoardToBoard;

export_generator!(BoardToBoard);

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

/// The full description. Unreadable presets are a build bug that the
/// golden and preset tests catch; describe still answers, without them.
fn description() -> Description {
    description_with(sparksmith_generator_sdk::presets(PRESETS).unwrap_or_default())
}

/// The description, with `presets`. The defaults are a 2 × 5 strip of
/// 1.27 mm (the FTSH-style starter row).
fn description_with(presets: Vec<Preset>) -> Description {
    let params = vec![
        Param {
            key: "positions".into(),
            label: "Positions per row".into(),
            help: None,
            ty: ParamType::Count {
                min: 2,
                max: MAX_POSITIONS,
            },
            default: ParamValue::Int(5),
            group: Some("pins".into()),
            context: None,
        },
        length(
            "pitch",
            "Pitch along a row",
            500 * UM,
            5 * MM,
            1_270_000,
            "pins",
        ),
        length(
            "row_pitch",
            "Row pitch",
            500 * UM,
            10 * MM,
            1_270_000,
            "pins",
        ),
        Param {
            key: "alignment".into(),
            label: "Alignment pins".into(),
            help: None,
            ty: ParamType::Enum {
                choices: vec![choice("none", "None"), choice("pins", "Two alignment pins")],
            },
            default: ParamValue::Text("none".into()),
            group: Some("pins".into()),
            context: None,
        },
        length(
            "body_trim",
            "Insulator shorter than positions × pitch by",
            0,
            2 * MM,
            0,
            "insulator",
        ),
        length(
            "body_width",
            "Insulator width",
            500 * UM,
            20 * MM,
            3_430_000,
            "insulator",
        ),
        length(
            "body_height",
            "Insulator height",
            300 * UM,
            20 * MM,
            2_510_000,
            "insulator",
        ),
        length(
            "standoff",
            "Insulator's bottom over the seating plane",
            0,
            10 * MM,
            410_000,
            "insulator",
        ),
        length(
            "post_side",
            "Square post side",
            100 * UM,
            2 * MM,
            410_000,
            "pins",
        ),
        length(
            "post_height",
            "Post above the insulator",
            200 * UM,
            30 * MM,
            3_050_000,
            "pins",
        ),
        length(
            "tail_span",
            "Tails' span, tip to tip",
            MM,
            30 * MM,
            5_800_000,
            "pins",
        ),
        length(
            "land_width",
            "Land width",
            100 * UM,
            5 * MM,
            740_000,
            "lands",
        ),
        length(
            "land_length",
            "Land length",
            100 * UM,
            10 * MM,
            2_790_000,
            "lands",
        ),
        length(
            "land_span",
            "Lands' outer edges, row to row",
            MM,
            30 * MM,
            6_860_000,
            "lands",
        ),
        length(
            "peg_diameter",
            "Alignment pin diameter at the insulator",
            200 * UM,
            5 * MM,
            1_600_000,
            "alignment",
        ),
        length(
            "peg_tip",
            "Alignment pin diameter at its tip",
            100 * UM,
            5 * MM,
            1_210_000,
            "alignment",
        ),
        length(
            "peg_length",
            "Alignment pin length under the insulator",
            200 * UM,
            10 * MM,
            2_540_000,
            "alignment",
        ),
        length(
            "peg_inset",
            "Alignment pins inside the outer positions by",
            0,
            10 * MM,
            635_000,
            "alignment",
        ),
    ];
    let rules = vec![
        Rule {
            text: "Post side < pitch and row pitch; land width < pitch".into(),
            fields: vec![
                "post_side".into(),
                "pitch".into(),
                "row_pitch".into(),
                "land_width".into(),
            ],
        },
        Rule {
            text: "The tails' tips stand on their lands".into(),
            fields: vec!["tail_span".into(), "land_span".into(), "land_length".into()],
        },
        Rule {
            text: "Alignment pins clear the tails".into(),
            fields: vec!["peg_diameter".into(), "peg_inset".into()],
        },
    ];
    let offer = PartOffer {
        families: vec!["board_to_board".into()],
        scopes: vec![Scope::Component, Scope::Package],
        grammar: vec![Grammar {
            scope: Scope::Component,
            patterns: vec![
                "board to board 2x{positions} {pitch:length}?".into(),
                "terminal strip 2x{positions} {pitch:length}? smd".into(),
            ],
            examples: vec![
                "board to board 2x5 1.27mm".into(),
                "terminal strip 2x8 2.54mm smd".into(),
            ],
        }],
        instance_fields: Vec::new(),
    };
    Description {
        abi: v1::ABI,
        id: "board-to-board".into(),
        version: VERSION,
        title: "Board-to-board SMD terminal strips".into(),
        licence: "MIT OR Apache-2.0".into(),
        deterministic: true,
        capabilities: vec![Capability {
            name: v1::GENERATE_PART.into(),
            effect: Effect::None,
            title: Some("Make a dual-row SMD terminal strip".into()),
            params,
            rules,
            presets,
            part: Some(offer),
        }],
        needs: Vec::new(),
    }
}

/// A strip's values, checked.
struct Dims {
    positions: i64,
    pitch: i64,
    row_pitch: i64,
    pegs: bool,
    body_trim: i64,
    body_width: i64,
    body_height: i64,
    standoff: i64,
    post_side: i64,
    post_height: i64,
    tail_span: i64,
    land: [i64; 2],
    land_span: i64,
    peg_diameter: i64,
    peg_tip: i64,
    peg_length: i64,
    peg_inset: i64,
}

fn half(v: i64) -> i64 {
    div_round(v, 2).unwrap_or(0)
}

/// `(i − (n − 1) / 2) × pitch`, to the nearest nanometre.
fn centred(i: i64, n: i64, pitch: i64) -> i64 {
    div_round((2 * i - (n - 1)) * pitch, 2).unwrap_or(0)
}

impl Dims {
    fn pins(&self) -> i64 {
        2 * self.positions
    }
    fn body_length(&self) -> i64 {
        self.positions * self.pitch - self.body_trim
    }
    fn body_top(&self) -> i64 {
        self.standoff + self.body_height
    }
    fn post_top(&self) -> i64 {
        self.body_top() + self.post_height
    }
    /// Where a post (gold) meets its tail (tin), inside the insulator.
    fn z_split(&self) -> i64 {
        self.standoff + half(self.body_height)
    }
    fn tail_tip(&self) -> i64 {
        half(self.tail_span)
    }
    /// The lands' centre from the axis.
    fn land_y(&self) -> i64 {
        half(self.land_span - self.land[1])
    }
    /// The tails' bend radius (centreline).
    fn bend_r(&self) -> i64 {
        let t = self.post_side;
        let foot = self.tail_tip() - half(self.row_pitch);
        let drop = self.z_split() - half(t);
        t.min(foot * 2 / 5).min(drop * 2 / 5)
    }
    /// The pocket round each post on the insulator's top: side and depth.
    fn pocket(&self) -> [i64; 2] {
        let side = (self.post_side * 9 / 5)
            .min(self.pitch * 7 / 10)
            .min(self.row_pitch * 7 / 10);
        [side, self.body_height / 8]
    }
    /// The grooves between positions on the insulator's long faces: width
    /// and depth.
    fn groove(&self) -> [i64; 2] {
        [
            self.pitch * 3 / 20,
            ((self.body_width - self.row_pitch - self.post_side) / 4).max(UM),
        ]
    }
    fn corner_round(&self) -> i64 {
        CORNER_ROUND_NM.min(self.groove()[1])
    }
    /// The alignment pins' x, from the axis.
    fn peg_x(&self) -> i64 {
        half((self.positions - 1) * self.pitch) - self.peg_inset
    }
    /// Where a tail touches the board: its foot's middle on the land.
    fn contact_y(&self) -> i64 {
        let lo =
            (half(self.row_pitch) + half(self.post_side)).max(self.land_y() - half(self.land[1]));
        let hi = self.tail_tip().min(self.land_y() + half(self.land[1]));
        half(lo + hi)
    }
    fn at(&self, k: i64) -> [i64; 2] {
        let (column, row) = (k / 2, k % 2);
        [
            centred(column, self.positions, self.pitch),
            if row == 0 {
                -self.land_y()
            } else {
                self.land_y()
            },
        ]
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
        positions: int("positions")?,
        pitch: int("pitch")?,
        row_pitch: int("row_pitch")?,
        pegs: args.text("alignment").map_err(one)? == "pins",
        body_trim: int("body_trim")?,
        body_width: int("body_width")?,
        body_height: int("body_height")?,
        standoff: int("standoff")?,
        post_side: int("post_side")?,
        post_height: int("post_height")?,
        tail_span: int("tail_span")?,
        land: [int("land_width")?, int("land_length")?],
        land_span: int("land_span")?,
        peg_diameter: int("peg_diameter")?,
        peg_tip: int("peg_tip")?,
        peg_length: int("peg_length")?,
        peg_inset: int("peg_inset")?,
    };
    let mut problems = Vec::new();
    let mut refuse = |field: &str, message: String| {
        problems.push(problem("param.inconsistent", field, message));
    };
    if d.post_side >= d.pitch || d.post_side >= d.row_pitch || d.land[0] >= d.pitch {
        refuse(
            "post_side",
            format!(
                "Posts of {} mm on lands of {} mm do not fit a {} × {} mm grid.",
                mm(d.post_side),
                mm(d.land[0]),
                mm(d.pitch),
                mm(d.row_pitch)
            ),
        );
    }
    if d.body_trim >= d.pitch || d.row_pitch + d.post_side >= d.body_width {
        refuse(
            "body_width",
            format!(
                "Two rows {} mm apart do not fit an insulator {} mm wide.",
                mm(d.row_pitch),
                mm(d.body_width)
            ),
        );
    }
    // The tails' tips on their lands, past the insulator's sides' posts.
    let tip = d.tail_tip();
    let land = [d.land_y() - half(d.land[1]), d.land_y() + half(d.land[1])];
    if d.land[1] >= d.land_span || tip < land[0] || tip > land[1] {
        refuse(
            "tail_span",
            format!(
                "Tails spanning {} mm end off lands spanning {} mm.",
                mm(d.tail_span),
                mm(d.land_span)
            ),
        );
    }
    if tip - half(d.row_pitch) < 2 * d.post_side || d.bend_r() * 5 < d.post_side * 3 {
        refuse(
            "tail_span",
            format!(
                "Tails spanning {} mm leave no foot past rows {} mm apart.",
                mm(d.tail_span),
                mm(d.row_pitch)
            ),
        );
    }
    if d.pegs {
        let r = half(d.peg_diameter);
        // Clear of every tail's upright run (x within a post's half side of
        // a position, y from the row inwards to its half side).
        let column = |x: i64| {
            let n = d.positions;
            (0..n)
                .map(|k| (centred(k, n, d.pitch) - x).abs())
                .min()
                .unwrap_or(0)
        };
        let dx = (column(d.peg_x()) - half(d.post_side)).max(0);
        let dy = (half(d.row_pitch) - half(d.post_side)).max(0);
        let clear = dx * dx + dy * dy > r * r || dy > r;
        if !clear || d.peg_x() <= 0 || d.peg_tip > d.peg_diameter || d.peg_diameter >= d.body_width
        {
            refuse(
                "peg_diameter",
                format!(
                    "Alignment pins of {} mm at {} mm from the centre do not clear the tails.",
                    mm(d.peg_diameter),
                    mm(d.peg_x())
                ),
            );
        }
    }
    if problems.is_empty() {
        Ok(d)
    } else {
        Err(problems)
    }
}

fn pads(d: &Dims) -> Vec<Pad> {
    let mut pads: Vec<Pad> = (0..d.pins())
        .map(|k| {
            let key = (k + 1).to_string();
            Pad {
                terminal: key.clone(),
                key,
                shape: PadShape::Rect {
                    size: [d.land[0], d.land[1]],
                },
                at: d.at(k),
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
    if d.pegs {
        let hole = d.peg_diameter + PEG_HOLE_EXTRA_NM;
        for (key, sign) in [("A1", -1), ("A2", 1)] {
            pads.push(Pad {
                key: key.into(),
                terminal: String::new(),
                shape: PadShape::Circle { diameter_nm: hole },
                at: [sign * d.peg_x(), 0],
                rot_mdeg: 0,
                side: Side::Top,
                hole: Some(Hole {
                    shape: HoleShape::Round { diameter_nm: hole },
                    plated: false,
                    offset: None,
                }),
                mask: Opening::Default,
                paste: Opening::None,
                number: Some(String::new()),
                kind: None,
            });
        }
    }
    pads
}

/// Two columns of pins, odd on the left and even on the right, as a
/// dual-row connector is drawn; the alignment pins (mechanical) under it.
fn symbol(d: &Dims) -> Symbol {
    let module = draw::MODULE_NM;
    let mut sym = Drawing::default();
    let rows = d.positions;
    let bottom = -rows * module;
    sym.shape(
        Primitive::Rect {
            min: [-2 * module, bottom],
            max: [2 * module, module],
        },
        draw::STROKE_NM,
        Fill::Body,
    );
    sym.labels(module, bottom);
    for k in 0..d.pins() {
        let (column, row) = (k / 2, k % 2);
        let y = -column * module;
        sym.pins.push(SymbolPin {
            terminal: (k + 1).to_string(),
            at: if row == 0 {
                [-4 * module, y]
            } else {
                [4 * module, y]
            },
            length_nm: 2 * module,
            rot_mdeg: if row == 0 { RIGHT } else { LEFT },
            show_name: false,
            show_number: true,
        });
    }
    Symbol {
        units: vec![sym.unit("A")],
    }
}

/// The SHA-256 of each part file in the bundle (`parts/`), written by the
/// build script.
mod parts {
    include!(concat!(env!("OUT_DIR"), "/parts.rs"));
}

/// The 3D model: the bundle's part file with or without alignment pins,
/// every value given.
fn model(d: &Dims) -> v1::Model {
    let mut values = v1::Params::new();
    let mut set = |name: &str, v: i64| {
        values.insert(name.into(), ParamValue::Int(v));
    };
    let [pocket, pocket_depth] = d.pocket();
    let [groove_width, groove_depth] = d.groove();
    set("positions", d.positions);
    set("pitch", d.pitch);
    set("row_pitch", d.row_pitch);
    set("body_length", d.body_length());
    set("body_width", d.body_width);
    set("standoff", d.standoff);
    set("body_top", d.body_top());
    set("post_side", d.post_side);
    set("post_top", d.post_top());
    set("z_split", d.z_split());
    set("tail_tip", d.tail_tip());
    set("bend_r", d.bend_r());
    set("pocket", pocket);
    set("pocket_depth", pocket_depth);
    set("groove_width", groove_width);
    set("groove_depth", groove_depth);
    set("corner_round", d.corner_round());
    if d.pegs {
        set("peg_x", d.peg_x());
        set("peg_diameter", d.peg_diameter);
        set("peg_tip", d.peg_tip);
        set("peg_bottom", d.standoff - d.peg_length);
    }
    v1::Model::PartFile {
        asset: if d.pegs {
            parts::B2B_DUAL_ROW_PEGS
        } else {
            parts::B2B_DUAL_ROW
        }
        .into(),
        values,
    }
}

fn generate(request: &Request) -> Result<Generation, Vec<Problem>> {
    let description = description_with(Vec::new());
    let args = checked(&description, &request.params)?;
    let d = dims(&args)?;
    let pads = pads(&d);
    let body = Box2::centred([0, 0], [d.body_length(), d.body_width]);
    let courtyard = land::courtyard(&pads, Some(body)).unwrap_or_default();
    let top = courtyard.iter().map(|p| p[1]).max().unwrap_or(body.max[1]);
    let mut silk = land::silk_outline(&pads, body, land::DEFAULT_MASK_NM);
    silk.push(land::silk_designator(body, top));
    // Pin 1: a dot past pad 1's outer end.
    let first = d.at(0);
    silk.push(Graphic {
        key: "pin1".into(),
        shape: Primitive::Circle {
            center: [
                first[0],
                first[1]
                    - half(d.land[1])
                    - land::DEFAULT_MASK_NM
                    - land::SILK_PAD_CLEARANCE_NM
                    - 2 * PIN_ONE_DOT_RADIUS_NM,
            ],
            radius_nm: PIN_ONE_DOT_RADIUS_NM,
        },
        stroke_nm: 0,
        fill: Fill::Solid,
    });
    // The tails' pads; the alignment pins' holes land no terminal.
    let keys: Vec<String> = pads
        .iter()
        .filter(|p| !p.terminal.is_empty())
        .map(|p| p.key.clone())
        .collect();
    let contact_y = d.contact_y();
    // A tail's contact on its land.
    let leads: Vec<Lead> = pads
        .iter()
        .filter(|p| !p.terminal.is_empty())
        .map(|p| Lead {
            key: p.key.clone(),
            contact: [p.at[0], p.at[1].signum() * contact_y, 0],
        })
        .collect();
    let package = Package {
        terminals: keys.clone(),
        geometry: PackageGeometry {
            envelope_nm: [d.body_length(), d.body_width.max(d.tail_span), d.post_top()],
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
    let name = format!(
        "Board-to-board strip 2×{} {} mm{}",
        d.positions,
        mm(d.pitch),
        if d.pegs { " with alignment pins" } else { "" }
    );
    let terminals: Vec<Terminal> = (1..=d.pins())
        .map(|k| Terminal {
            key: k.to_string(),
            name: k.to_string(),
            direction: Direction::Passive,
            alternates: Vec::new(),
            drive: None,
            unconnected: None,
        })
        .collect();
    let mut notes = vec![
        "Lands are the land parameters as given: the maker's recommended layout only where they were taken from it. The insulator's grooves and pockets are drawn to typical proportions."
            .into(),
    ];
    if d.pegs {
        notes.push(
            "The alignment pins' holes are unplated, the pin's diameter + 0.13 mm (Samtec's TSM -A layout); they land no terminal."
                .into(),
        );
    }
    Ok(Generation {
        orderable: Vec::new(),
        marking: None,
        component: component_part.then(|| Component {
            terminals,
            prefix: "J".into(),
            name,
            fields: Vec::new(),
        }),
        symbol: component_part.then(|| symbol(&d)),
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
        notes,
    })
}

impl Generator for BoardToBoard {
    fn describe() -> Description {
        description()
    }

    fn generate(request: &Request) -> Result<Generation, Vec<Problem>> {
        generate(request)
    }
}

#[cfg(test)]
mod tests;
