//! The microSD generator: surface-mount microSD card sockets with a metal
//! shell, eight contacts in a row at the card's entry and the shell's
//! solder feet (ground and the card-detect switch), as a symbol, package,
//! footprint and 3D model.
//!
//! Frame: x = 0 is the card's centre line, y = 0 the contact lands'
//! centre; the card enters from −y, contact 1 (DAT2) lies towards +x and
//! contact 8 (DAT1) towards −x, as the cards' pads are numbered. z = 0 is
//! the seating plane.
//!
//! Every land is the maker's recommended mounting pattern as parameters
//! (the contact row's pitch and its −x contact, and each mechanical land's
//! centre and size), never estimated. The shell and the contacts' tails
//! are placed by the drawing's outline, so the tails' tips and the solder
//! feet stand on their lands. The preset cites the maker's public
//! catalogue; what it does not give (sheet, insulator, contact section,
//! the shell's front relative to the lands) is estimated, as it says.
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
    self, BindingEntry, Capability, Component, Description, Direction, Effect, Fill, Footprint,
    Generation, Grammar, Lead, Opening, Package, PackageGeometry, Pad, PadShape, Param, ParamType,
    ParamValue, PartOffer, Preset, Primitive, Problem, Request, Rule, Scope, Side, Symbol,
    SymbolPin, Terminal,
};
use sparksmith_generator_sdk::{Generator, export_generator};

/// The starter rows, in nm.
const PRESETS: &str = include_str!("../presets.json");

/// The generator's version (`Description::version`).
/// 3: the SDK's pin table (`pin_table`) and part numbers
/// (`part_numbers`).
const VERSION: u32 = 4;

/// A microSD card's eight contacts, by number: their signal names.
const SIGNALS: [&str; 8] = [
    "DAT2", "CD/DAT3", "CMD", "VDD", "CLK", "VSS", "DAT0", "DAT1",
];

/// The mechanical lands: key, the package terminal's component terminal,
/// and the label of its parameters.
const FEET: [(&str, &str, &str); 6] = [
    ("G1", "SHIELD", "g1"),
    ("G2", "SHIELD", "g2"),
    ("G3", "SHIELD", "g3"),
    ("G4", "SHIELD", "g4"),
    ("CD_A", "CD_A", "cd_a"),
    ("CD_B", "CD_B", "cd_b"),
];

/// A solder foot stops this far inside its land on every side (est.).
const FOOT_INSET_NM: i64 = 100_000;

/// The pin-1 dot on the silkscreen.
const PIN_ONE_DOT_RADIUS_NM: i64 = 150_000;

pub struct MicroSd;

export_generator!(MicroSd);

fn length(key: &str, label: String, min_nm: i64, max_nm: i64, default: i64, group: &str) -> Param {
    Param {
        key: key.into(),
        label,
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

fn len(key: &str, label: &str, min_nm: i64, max_nm: i64, default: i64, group: &str) -> Param {
    length(key, label.into(), min_nm, max_nm, default, group)
}

/// A signed coordinate parameter.
fn coord(key: &str, label: String, default: i64, group: &str) -> Param {
    length(key, label, -30 * MM, 30 * MM, default, group)
}

/// The defaults of the mechanical lands (DM3D-style): centre x, centre y,
/// width, length.
const FEET_DEFAULT: [[i64; 4]; 6] = [
    [-5_700_000, 125_000, 1_300_000, 1_500_000],
    [5_650_000, 125_000, 1_500_000, 1_500_000],
    [-5_950_000, 7_725_000, 800_000, 1_500_000],
    [6_000_000, 8_375_000, 800_000, 1_400_000],
    [5_600_000, 10_800_000, 1_000_000, 1_550_000],
    [-5_625_000, 9_225_000, 1_450_000, 1_000_000],
];

/// The full description. Unreadable presets are a build bug that the
/// golden and preset tests catch; describe still answers, without them.
fn description() -> Description {
    description_with(sparksmith_generator_sdk::presets(PRESETS).unwrap_or_default())
}

fn description_with(presets: Vec<Preset>) -> Description {
    let mut params = vec![
        land::mask_expansion_param(include!("../../standalone-mask-expansion-nm.json")),
        len(
            "pitch",
            "Contact pitch",
            500 * UM,
            3 * MM,
            1_100_000,
            "contacts",
        ),
        coord(
            "first_x",
            "Contact 8's centre from the card's centre line".into(),
            -4_500_000,
            "contacts",
        ),
        len(
            "land_width",
            "Contact land width",
            100 * UM,
            3 * MM,
            700_000,
            "lands",
        ),
        len(
            "land_length",
            "Contact land length",
            100 * UM,
            5 * MM,
            1_750_000,
            "lands",
        ),
        coord(
            "shell_x0",
            "Shell's −x side from the card's centre line".into(),
            -5_950_000,
            "shell",
        ),
        coord(
            "shell_x1",
            "Shell's +x side from the card's centre line".into(),
            6_000_000,
            "shell",
        ),
        coord(
            "front",
            "Shell's front (card entry) from the contact lands' centre".into(),
            -550_000,
            "shell",
        ),
        len("depth", "Shell depth", 2 * MM, 40 * MM, 11_450_000, "shell"),
        len(
            "height",
            "Height above the seating plane",
            500 * UM,
            10 * MM,
            1_550_000,
            "shell",
        ),
        len("sheet", "Shell sheet", 50 * UM, MM, 150_000, "shell"),
        len(
            "foot",
            "Solder feet's thickness",
            50 * UM,
            MM,
            150_000,
            "shell",
        ),
        len(
            "lead_width",
            "Contact width",
            100 * UM,
            2 * MM,
            400_000,
            "contacts",
        ),
        len(
            "lead_thickness",
            "Contact thickness",
            50 * UM,
            MM,
            100_000,
            "contacts",
        ),
        coord(
            "tip_y",
            "Contact tails' tips from the contact lands' centre".into(),
            -650_000,
            "contacts",
        ),
        len(
            "foot_length",
            "Contact tails' length on the board",
            200 * UM,
            5 * MM,
            MM,
            "contacts",
        ),
        len(
            "arm_length",
            "Contact springs' length",
            500 * UM,
            30 * MM,
            4_150_000,
            "contacts",
        ),
    ];
    for (i, (key, _, stem)) in FEET.iter().enumerate() {
        let d = FEET_DEFAULT.get(i).copied().unwrap_or_default();
        params.push(coord(
            &format!("{stem}_x"),
            format!("{key} land's centre x"),
            d[0],
            "lands",
        ));
        params.push(coord(
            &format!("{stem}_y"),
            format!("{key} land's centre y"),
            d[1],
            "lands",
        ));
        params.push(length(
            &format!("{stem}_w"),
            format!("{key} land's width"),
            300 * UM,
            5 * MM,
            d[2],
            "lands",
        ));
        params.push(length(
            &format!("{stem}_l"),
            format!("{key} land's length"),
            300 * UM,
            5 * MM,
            d[3],
            "lands",
        ));
    }
    let rules = vec![
        Rule {
            text: "Contact land width < pitch; the contacts fit inside the shell".into(),
            fields: vec![
                "land_width".into(),
                "pitch".into(),
                "first_x".into(),
                "shell_x0".into(),
                "shell_x1".into(),
            ],
        },
        Rule {
            text: "The tails' tips stand on their lands".into(),
            fields: vec!["tip_y".into(), "land_length".into()],
        },
    ];
    let offer = PartOffer {
        families: vec!["memory_card".into()],
        scopes: vec![Scope::Component, Scope::Package],
        grammar: vec![Grammar {
            scope: Scope::Component,
            patterns: vec!["microsd socket".into(), "micro sd card connector".into()],
            examples: vec!["microsd socket".into()],
        }],
        instance_fields: Vec::new(),
    };
    Description {
        abi: v1::ABI,
        id: "microsd".into(),
        version: VERSION,
        title: "MicroSD card sockets".into(),
        licence: "MIT OR Apache-2.0".into(),
        deterministic: true,
        capabilities: vec![Capability {
            name: v1::GENERATE_PART.into(),
            effect: Effect::None,
            title: Some("Make a microSD socket".into()),
            params,
            rules,
            presets,
            part: Some(offer),
        }],
        needs: Vec::new(),
    }
}

/// A socket's values, checked.
struct Dims {
    pitch: i64,
    first_x: i64,
    land: [i64; 2],
    shell_x: [i64; 2],
    front: i64,
    depth: i64,
    height: i64,
    sheet: i64,
    foot: i64,
    lead_width: i64,
    lead_thickness: i64,
    tip_y: i64,
    foot_length: i64,
    arm_length: i64,
    /// Each mechanical land: centre x, centre y, width, length.
    feet: [[i64; 4]; 6],
}

fn half(v: i64) -> i64 {
    div_round(v, 2).unwrap_or(0)
}

impl Dims {
    fn rear(&self) -> i64 {
        self.front + self.depth
    }
    /// Contact `k` (from 1)'s x: contact 8 at `first_x`, contact 1 seven
    /// pitches towards +x.
    fn contact_x(&self, k: i64) -> i64 {
        self.first_x + (8 - k) * self.pitch
    }
    fn rise_y(&self) -> i64 {
        self.tip_y + self.foot_length
    }
    /// The insulator's floor: from under the shell's front to its rear,
    /// its top this far up.
    fn floor_top(&self) -> i64 {
        self.foot + (self.height - self.foot) / 5
    }
    fn floor_front(&self) -> i64 {
        self.rise_y() - self.lead_thickness * 3
    }
    fn arm_z(&self) -> i64 {
        self.floor_top() + self.lead_thickness
    }
    fn arm_end(&self) -> i64 {
        self.rise_y() + self.arm_length
    }
    /// The insulator's rear block: the rear sixth of the shell.
    fn block_front(&self) -> i64 {
        self.rear() - self.depth / 7
    }
    fn bend_r(&self) -> i64 {
        let t = self.lead_thickness;
        let rise = self.arm_z() - half(t);
        t.min(rise * 2 / 5).min(self.foot_length * 2 / 5)
    }
    /// A foot's box: its land inset on every side.
    fn foot_box(&self, i: usize) -> Box2 {
        let [x, y, w, l] = self.feet.get(i).copied().unwrap_or_default();
        Box2 {
            min: [x - half(w) + FOOT_INSET_NM, y - half(l) + FOOT_INSET_NM],
            max: [x + half(w) - FOOT_INSET_NM, y + half(l) - FOOT_INSET_NM],
        }
    }
    fn contact_y(&self) -> i64 {
        let lo = self.tip_y.max(-half(self.land[1]));
        let hi = self.rise_y().min(half(self.land[1]));
        half(lo + hi)
    }
}

fn mm(nm: i64) -> String {
    let sign = if nm < 0 { "-" } else { "" };
    let nm = nm.abs();
    let whole = nm / MM;
    let mut frac = format!("{:06}", nm % MM);
    while frac.len() > 1 && frac.ends_with('0') {
        frac.pop();
    }
    format!("{sign}{whole}.{frac}")
}

fn dims(args: &Args<'_>) -> Result<Dims, Vec<Problem>> {
    let one = |p: Problem| vec![p];
    let int = |k: &str| args.int(k).map_err(one);
    let mut feet = [[0; 4]; 6];
    for (i, (_, _, stem)) in FEET.iter().enumerate() {
        if let Some(f) = feet.get_mut(i) {
            *f = [
                int(&format!("{stem}_x"))?,
                int(&format!("{stem}_y"))?,
                int(&format!("{stem}_w"))?,
                int(&format!("{stem}_l"))?,
            ];
        }
    }
    let d = Dims {
        pitch: int("pitch")?,
        first_x: int("first_x")?,
        land: [int("land_width")?, int("land_length")?],
        shell_x: [int("shell_x0")?, int("shell_x1")?],
        front: int("front")?,
        depth: int("depth")?,
        height: int("height")?,
        sheet: int("sheet")?,
        foot: int("foot")?,
        lead_width: int("lead_width")?,
        lead_thickness: int("lead_thickness")?,
        tip_y: int("tip_y")?,
        foot_length: int("foot_length")?,
        arm_length: int("arm_length")?,
        feet,
    };
    let mut problems = Vec::new();
    let mut refuse = |field: &str, message: String| {
        problems.push(problem("param.inconsistent", field, message));
    };
    let inner = [d.shell_x[0] + d.sheet, d.shell_x[1] - d.sheet];
    let row = [
        d.contact_x(8) - half(d.lead_width.max(d.land[0])),
        d.contact_x(1) + half(d.lead_width.max(d.land[0])),
    ];
    if d.land[0] >= d.pitch || d.lead_width >= d.pitch {
        refuse(
            "land_width",
            format!(
                "Lands of {} mm do not fit a pitch of {} mm.",
                mm(d.land[0]),
                mm(d.pitch)
            ),
        );
    }
    if row[0] <= inner[0] || row[1] >= inner[1] || inner[0] >= 0 || inner[1] <= 0 {
        refuse(
            "first_x",
            format!(
                "The contacts ({} to {} mm) do not fit inside the shell ({} to {} mm).",
                mm(row[0]),
                mm(row[1]),
                mm(inner[0]),
                mm(inner[1])
            ),
        );
    }
    if d.tip_y < -half(d.land[1]) || d.tip_y > half(d.land[1]) || d.tip_y >= d.front {
        refuse(
            "tip_y",
            format!(
                "Tails ending {} mm from the lands' centre are off lands {} mm long or under the shell.",
                mm(d.tip_y),
                mm(d.land[1])
            ),
        );
    }
    if d.arm_end() >= d.block_front() || d.floor_front() <= d.front {
        refuse(
            "arm_length",
            format!(
                "Springs {} mm long do not fit the shell {} mm deep.",
                mm(d.arm_length),
                mm(d.depth)
            ),
        );
    }
    if d.arm_z() + d.lead_thickness >= d.height - d.sheet || d.bend_r() * 5 < d.lead_thickness * 3 {
        refuse(
            "height",
            format!(
                "Contacts {} mm thick do not fit a shell {} mm high.",
                mm(d.lead_thickness),
                mm(d.height)
            ),
        );
    }
    for (i, (key, _, stem)) in FEET.iter().enumerate() {
        let b = d.foot_box(i);
        let clear_of_row = b.max[0] < row[0] || b.min[0] > row[1] || b.min[1] > d.arm_end();
        if b.min[0] >= b.max[0] || b.min[1] >= b.max[1] || !clear_of_row {
            refuse(
                &format!("{stem}_x"),
                format!("The {key} land does not hold a foot clear of the contacts."),
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
    let pad = |key: String, at: [i64; 2], size: [i64; 2]| Pad {
        terminal: key.clone(),
        key,
        shape: PadShape::Rect { size },
        at,
        rot_mdeg: 0,
        side: Side::Top,
        hole: None,
        mask: Opening::Default,
        paste: Opening::Default,
        number: None,
        kind: None,
    };
    let mut pads: Vec<Pad> = (1..=8)
        .map(|k| pad(k.to_string(), [d.contact_x(k), 0], d.land))
        .collect();
    for (i, (key, _, _)) in FEET.iter().enumerate() {
        let [x, y, w, l] = d.feet.get(i).copied().unwrap_or_default();
        pads.push(pad((*key).into(), [x, y], [w, l]));
    }
    pads
}

/// The socket's symbol: the eight contacts by signal down the left, the
/// card-detect switch's two terminals and the shield on the right.
fn symbol() -> Symbol {
    let module = draw::MODULE_NM;
    let mut sym = Drawing::default();
    let bottom = -9 * module;
    sym.shape(
        Primitive::Rect {
            min: [-3 * module, bottom],
            max: [3 * module, module],
        },
        draw::STROKE_NM,
        Fill::Body,
    );
    sym.labels(module, bottom);
    for k in 0..8 {
        sym.pins.push(SymbolPin {
            terminal: (k + 1).to_string(),
            at: [-5 * module, -k * module],
            length_nm: 2 * module,
            rot_mdeg: RIGHT,
            show_name: true,
            show_number: true,
        });
    }
    for (i, terminal) in ["CD_A", "CD_B"].iter().enumerate() {
        let i = i64::try_from(i).unwrap_or(0);
        sym.pins.push(SymbolPin {
            terminal: (*terminal).into(),
            at: [5 * module, -i * module],
            length_nm: 2 * module,
            rot_mdeg: draw::LEFT,
            show_name: true,
            show_number: false,
        });
    }
    sym.pins.push(SymbolPin {
        terminal: "SHIELD".into(),
        at: [0, bottom - module],
        length_nm: module,
        rot_mdeg: UP,
        show_name: true,
        show_number: false,
    });
    Symbol {
        units: vec![sym.unit("A")],
    }
}

/// The SHA-256 of each part file in the bundle (`parts/`), written by the
/// build script.
mod parts {
    include!(concat!(env!("OUT_DIR"), "/parts.rs"));
}

fn model(d: &Dims) -> v1::Model {
    let mut values = v1::Params::new();
    let mut set = |name: &str, v: i64| {
        values.insert(name.into(), ParamValue::Int(v));
    };
    let width = d.shell_x[1] - d.shell_x[0];
    set("contacts", 8);
    set("pitch", d.pitch);
    set("first_x", d.first_x);
    set("shell_x0", d.shell_x[0]);
    set("shell_x1", d.shell_x[1]);
    set("front", d.front);
    set("rear", d.rear());
    set("height", d.height);
    set("sheet", d.sheet);
    set("foot", d.foot);
    set("floor_top", d.floor_top());
    set("floor_front", d.floor_front());
    set("block_front", d.block_front());
    set("lead_width", d.lead_width);
    set("lead_thickness", d.lead_thickness);
    set("tip_y", d.tip_y);
    set("rise_y", d.rise_y());
    set("arm_z", d.arm_z());
    set("arm_end", d.arm_end());
    set("bend_r", d.bend_r());
    set("window_x", width / 6);
    set("window_w", width / 10);
    let y0 = d.front + d.depth / 2;
    set("window_y0", y0);
    set("window_y1", y0 + d.depth / 6);
    for (i, (_, _, stem)) in FEET.iter().enumerate() {
        let b = d.foot_box(i);
        set(&format!("{stem}_x0"), b.min[0]);
        set(&format!("{stem}_x1"), b.max[0]);
        set(&format!("{stem}_y0"), b.min[1]);
        set(&format!("{stem}_y1"), b.max[1]);
    }
    v1::Model::PartFile {
        asset: parts::MICROSD_PUSH_PULL.into(),
        values,
    }
}

fn generate(request: &Request) -> Result<Generation, Vec<Problem>> {
    let description = description_with(Vec::new());
    let args = checked(&description, &request.params)?;
    let mask_expansion = args.int(v1::BOARD_MASK_EXPANSION).map_err(|p| vec![p])?;
    let d = dims(&args)?;
    let pads = pads(&d);
    let body = Box2 {
        min: [d.shell_x[0], d.front],
        max: [d.shell_x[1], d.rear()],
    };
    let courtyard = land::courtyard(&pads, Some(body)).unwrap_or_default();
    let top = courtyard.iter().map(|p| p[1]).max().unwrap_or(body.max[1]);
    let mut silk = land::silk_outline(&pads, body, mask_expansion);
    silk.push(land::silk_designator(body, top));
    silk.push(v1::Graphic {
        key: "pin1".into(),
        shape: Primitive::Circle {
            center: [
                d.contact_x(1),
                -half(d.land[1])
                    - mask_expansion
                    - land::SILK_PAD_CLEARANCE_NM
                    - 2 * PIN_ONE_DOT_RADIUS_NM,
            ],
            radius_nm: PIN_ONE_DOT_RADIUS_NM,
        },
        stroke_nm: 0,
        fill: Fill::Solid,
    });
    let keys: Vec<String> = pads.iter().map(|p| p.key.clone()).collect();
    let contact_y = d.contact_y();
    let mut leads: Vec<Lead> = (1..=8)
        .map(|k| Lead {
            key: k.to_string(),
            contact: [d.contact_x(k), contact_y, 0],
        })
        .collect();
    let mut reach = [
        d.shell_x[0].abs().max(d.shell_x[1]),
        d.front.abs().max(d.rear()).max(d.tip_y.abs()),
    ];
    for (i, (key, _, _)) in FEET.iter().enumerate() {
        let b = d.foot_box(i);
        let c = b.center();
        leads.push(Lead {
            key: (*key).into(),
            contact: [c[0], c[1], 0],
        });
        reach[0] = reach[0].max(b.min[0].abs()).max(b.max[0].abs());
        reach[1] = reach[1].max(b.min[1].abs()).max(b.max[1].abs());
    }
    let package = Package {
        terminals: keys.clone(),
        geometry: PackageGeometry {
            envelope_nm: [2 * reach[0], 2 * reach[1], d.height],
            seating_nm: 0,
            leads,
            pin1: "1".into(),
        },
    };
    let footprint = Footprint {
        bridges: Vec::new(),
        pads,
        courtyard,
        silk,
        assembly: land::assembly(body),
        keepouts: Vec::new(),
    };
    let component_part = request.scope == Scope::Component;
    let mut terminals: Vec<Terminal> = SIGNALS
        .iter()
        .enumerate()
        .map(|(i, name)| Terminal {
            key: (i + 1).to_string(),
            name: (*name).into(),
            direction: Direction::Passive,
            alternates: Vec::new(),
            drive: None,
            unconnected: None,
        })
        .collect();
    for key in ["CD_A", "CD_B", "SHIELD"] {
        terminals.push(Terminal {
            key: key.into(),
            name: key.into(),
            direction: Direction::Passive,
            alternates: Vec::new(),
            drive: None,
            unconnected: None,
        });
    }
    Ok(Generation {
        orderable: Vec::new(),
        marking: None,
        component: component_part.then(|| Component {
            ties: Default::default(),
            assembly: Default::default(),
            terminals,
            prefix: "J".into(),
            name: "MicroSD socket, push-pull".into(),
            fields: Vec::new(),
        }),
        symbol: component_part.then(symbol),
        package,
        footprint,
        binding: if component_part {
            keys.iter()
                .map(|k| BindingEntry {
                    package_terminal: k.clone(),
                    terminal: FEET
                        .iter()
                        .find(|(key, _, _)| key == k)
                        .map(|(_, terminal, _)| (*terminal).to_string())
                        .unwrap_or_else(|| k.clone()),
                })
                .collect()
        } else {
            Vec::new()
        },
        model: Some(model(&d)),
        fields: Vec::new(),
        notes: vec![
            "Lands are the maker's recommended mounting pattern as given; keep conductive traces out of the area the maker marks under the card. The insulator, sheet and contact section are estimated."
                .into(),
        ],
    })
}

impl Generator for MicroSd {
    fn describe() -> Description {
        description()
    }

    fn generate(request: &Request) -> Result<Generation, Vec<Problem>> {
        generate(request)
    }
}

#[cfg(test)]
mod tests;
