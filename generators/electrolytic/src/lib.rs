//! The electrolytic generator: surface-mount aluminium electrolytic
//! capacitors (vertical chip, the EIAJ size codes) as a symbol, package,
//! footprint and 3D model.
//!
//! The parameters are the drawing's letters: the can (`can_diameter` D,
//! `height` L), the seat plate (`base_length` A, `base_width` B), the
//! terminals (`span` H over both, `terminal_gap` P between them,
//! `terminal_width` W) and the manufacturer's land pattern (`land_gap`
//! between the lands, each `land_length` × `land_width`), taken as given.
//! Terminal 1 is + at −x, where the seat plate's corners are bevelled; the
//! can's top carries the negative print over its +x side.
//!
//! The presets are the size codes of Panasonic's FK series drawing and
//! land table, and KEMET's EDK 6.3 × 7.7 mm, with their citations.
#![cfg_attr(target_arch = "wasm32", no_std)]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use sparksmith_generator_sdk::args::{Args, checked, problem};
use sparksmith_generator_sdk::draw::{self, Drawing};
use sparksmith_generator_sdk::land::{self, Box2};
use sparksmith_generator_sdk::units::{MM, UM, div_round, mm_text};
use sparksmith_generator_sdk::v1::{
    self, BindingEntry, Capability, Component, Description, Direction, Effect, Fill, Footprint,
    Generation, Grammar, Graphic, InstanceField, Lead, Opening, Package, PackageGeometry, Pad,
    PadShape, Param, ParamType, ParamValue, PartOffer, Preset, Primitive, Problem, Request, Rule,
    Scope, Side, Symbol, Terminal,
};
use sparksmith_generator_sdk::{Generator, export_generator};

/// The size codes, in nm, with their citations.
const PRESETS: &str = include_str!("../presets.json");

/// The generator's version (`Description::version`): 2 prints the values
/// on the can top in black ([`marking`]); 4 at the height its preset gives
/// (`marking_height`) and the vent scored as a V; 5 scores it finer and
/// shallower (the `-vent` part file), a subtle cross under the print.
/// 6: the SDK's pin table (`pin_table`) and part numbers
/// (`part_numbers`).
const VERSION: u32 = 6;

/// The seat plate's height (est.: 0.9–1.2 mm on 4–10 mm parts), at most a
/// quarter of the overall height.
const BASE_HEIGHT_NM: i64 = 1_000_000;
/// The terminal thickness (est.), at most a third of the seat plate.
const LEAD_NM: i64 = 300_000;
/// The rolled seal bead's centre above the seat plate and its depth (est.).
const BEAD_HEIGHT_NM: i64 = 800_000;
const BEAD_DEPTH_NM: i64 = 200_000;
/// The can rim's round (est.), at most an eighteenth of the diameter.
const RIM_NM: i64 = 350_000;
/// The pin-1 (+) dot beyond pad 1, as the molded generator marks it.
const PIN_ONE_DOT_RADIUS_NM: i64 = 200_000;

pub struct Electrolytic;

export_generator!(Electrolytic);

fn length(key: &str, label: &str, [min_nm, max_nm]: [i64; 2], default: i64, group: &str) -> Param {
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

/// The full description. Unreadable presets are a build bug that the
/// golden and preset tests catch; describe still answers, without them.
fn description() -> Description {
    description_with(sparksmith_generator_sdk::presets(PRESETS).unwrap_or_default())
}

/// The description, with `presets`.
fn description_with(presets: Vec<Preset>) -> Description {
    // The defaults are Panasonic's size code D (6.3 × 5.8 mm).
    let params = vec![
        length(
            "can_diameter",
            "Can diameter (D)",
            [2 * MM, 25 * MM],
            6_300_000,
            "can",
        ),
        length("height", "Height (L)", [2 * MM, 30 * MM], 5_800_000, "can"),
        length(
            "base_length",
            "Seat plate length (A)",
            [2 * MM, 30 * MM],
            6_600_000,
            "body",
        ),
        length(
            "base_width",
            "Seat plate width (B)",
            [2 * MM, 30 * MM],
            6_600_000,
            "body",
        ),
        length(
            "span",
            "Overall length over the terminals (H)",
            [2 * MM, 35 * MM],
            7_800_000,
            "terminals",
        ),
        length(
            "terminal_gap",
            "Gap between the terminals (P)",
            [200 * UM, 20 * MM],
            1_800_000,
            "terminals",
        ),
        length(
            "terminal_width",
            "Terminal width (W)",
            [200 * UM, 5 * MM],
            650_000,
            "terminals",
        ),
        length(
            "land_gap",
            "Gap between the lands (a)",
            [200 * UM, 20 * MM],
            1_800_000,
            "lands",
        ),
        length(
            "land_length",
            "Land length (b)",
            [300 * UM, 15 * MM],
            3_200_000,
            "lands",
        ),
        length(
            "land_width",
            "Land width (c)",
            [300 * UM, 10 * MM],
            1_600_000,
            "lands",
        ),
        Param {
            key: "vent".into(),
            label: "Vent cross".into(),
            help: Some("The scored cross of the pressure relief vent on the can's top.".into()),
            ty: ParamType::Bool,
            default: ParamValue::Bool(true),
            group: Some("can".into()),
            context: None,
        },
        sparksmith_generator_sdk::marking::height_param(),
    ];
    let rule = |text: &str, fields: &[&str]| Rule {
        text: text.into(),
        fields: fields.iter().map(|f| (*f).into()).collect(),
    };
    let rules = vec![
        rule(
            "D ≤ A and D ≤ B: the can stands on the seat plate",
            &["can_diameter", "base_length", "base_width"],
        ),
        rule(
            "P < H ≤ 2 × A; W < B",
            &[
                "terminal_gap",
                "span",
                "base_length",
                "terminal_width",
                "base_width",
            ],
        ),
        rule("L > 2 mm over the seat plate", &["height"]),
    ];
    let offer = PartOffer {
        families: vec!["electrolytic".into()],
        scopes: vec![Scope::Component, Scope::Package],
        grammar: vec![Grammar {
            scope: Scope::Component,
            patterns: vec!["electrolytic {size:code}".into()],
            examples: vec![
                "electrolytic 6.3x5.8".into(),
                "aluminium capacitor size D".into(),
            ],
        }],
        instance_fields: vec![InstanceField {
            standard_field: "value".into(),
            when: v1::Params::new(),
        }],
    };
    Description {
        abi: v1::ABI,
        id: "electrolytic".into(),
        version: VERSION,
        title: "Aluminium electrolytic capacitors (SMD)".into(),
        licence: "MIT OR Apache-2.0".into(),
        deterministic: true,
        capabilities: vec![Capability {
            name: v1::GENERATE_PART.into(),
            effect: Effect::None,
            title: Some("Make a surface-mount aluminium electrolytic capacitor".into()),
            params,
            rules,
            presets,
            part: Some(offer),
        }],
        needs: Vec::new(),
    }
}

/// A part's values, checked.
struct Dims {
    d: i64,
    l: i64,
    a: i64,
    b: i64,
    h: i64,
    p: i64,
    w: i64,
    land_gap: i64,
    land: [i64; 2],
    vent: bool,
}

fn part_of(n: i64, d: i64) -> i64 {
    div_round(n, d).unwrap_or(0)
}

impl Dims {
    fn base_height(&self) -> i64 {
        BASE_HEIGHT_NM.min(part_of(self.l, 4))
    }

    fn envelope(&self) -> [i64; 3] {
        [self.h.max(self.a), self.b.max(self.d), self.l]
    }

    /// The x of terminal 2's contact: the middle of its exposed strip.
    fn contact_x(&self) -> i64 {
        part_of(self.p + part_of(self.h - self.p, 2), 2)
    }

    /// The x of land 2's centre.
    fn land_x(&self) -> i64 {
        part_of(self.land_gap + self.land[0], 2)
    }
}

fn dims(args: &Args<'_>) -> Result<Dims, Vec<Problem>> {
    let one = |p: Problem| vec![p];
    let int = |k: &str| args.int(k).map_err(one);
    let d = Dims {
        d: int("can_diameter")?,
        l: int("height")?,
        a: int("base_length")?,
        b: int("base_width")?,
        h: int("span")?,
        p: int("terminal_gap")?,
        w: int("terminal_width")?,
        land_gap: int("land_gap")?,
        land: [int("land_length")?, int("land_width")?],
        vent: args.flag("vent").map_err(one)?,
    };
    let mut problems = Vec::new();
    let mut rule = |ok: bool, field: &str, message: &str| {
        if !ok {
            problems.push(problem("param.inconsistent", field, message.into()));
        }
    };
    rule(
        d.d <= d.a && d.d <= d.b,
        "can_diameter",
        "The can must stand on the seat plate (D ≤ A and D ≤ B).",
    );
    rule(
        d.p < d.h,
        "terminal_gap",
        "The terminals must not overlap (P < H).",
    );
    rule(
        d.h <= 2 * d.a,
        "span",
        "The terminals reach at most a seat plate's length beyond it (H ≤ 2 × A).",
    );
    rule(
        d.w < d.b,
        "terminal_width",
        "A terminal must be narrower than the seat plate (W < B).",
    );
    rule(
        d.l > d.base_height() + 2 * MM,
        "height",
        "The can must stand at least 2 mm above the seat plate.",
    );
    if problems.is_empty() {
        Ok(d)
    } else {
        Err(problems)
    }
}

fn pads(d: &Dims) -> Vec<Pad> {
    let x = d.land_x();
    [("1", -x), ("2", x)]
        .iter()
        .map(|(key, x)| Pad {
            key: (*key).into(),
            terminal: (*key).into(),
            shape: PadShape::Rect { size: d.land },
            at: [*x, 0],
            rot_mdeg: 0,
            side: Side::Top,
            hole: None,
            mask: Opening::Default,
            paste: Opening::Default,
            number: None,
            kind: None,
        })
        .collect()
}

/// The polarised capacitor of the host's standard drawings (IEC 60617 and
/// IEEE 315 as published): a straight + plate (terminal 1) and a curved −
/// plate, a + beside the + plate.
fn symbol() -> Symbol {
    let mut d = Drawing::default();
    let m = |v: i64| v * MM / 1000;
    d.two_pins("1", "2", m(500), m(500));
    d.plate([m(-500), m(-1750)], [m(-500), m(1750)]);
    let (r, half) = (m(2500), m(1750));
    let x = m(3000) - root_round(r * r - half * half);
    d.shape(
        Primitive::Arc {
            center: [m(3000), 0],
            start: [x, half],
            end: [x, -half],
            clockwise: false,
        },
        draw::PLATE_STROKE_NM,
        Fill::None,
    );
    d.line([m(-2250), m(1500)], [m(-1250), m(1500)]);
    d.line([m(-1750), m(1000)], [m(-1750), m(2000)]);
    d.labels(m(2000), m(-1750));
    Symbol {
        units: vec![d.unit("A")],
    }
}

/// `√v` to the nearest integer.
fn root_round(v: i64) -> i64 {
    let root = v.max(0).isqrt();
    if v - root * root > root {
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

/// The 3D model: the bundle's part file, with the vent or without, at the
/// drawing's values and these estimates: the seat plate's height and its
/// bevels, the terminal thickness, the seal bead and the rim round. The
/// negative print, the vent and the marking area follow the can.
fn model(d: &Dims) -> v1::Model {
    let mut values = v1::Params::new();
    let mut set = |name: &str, nm: i64| {
        values.insert(name.into(), ParamValue::Int(nm));
    };
    let base = d.base_height();
    set("can_diameter", d.d);
    set("height", d.l);
    set("base_length", d.a);
    set("base_width", d.b);
    set("base_height", base);
    // The + corners' bevels: 15 % of the plate, within the gap beside the
    // can's footprint.
    set(
        "base_bevel",
        part_of(15 * d.a.min(d.b), 100).min(part_of(d.a.min(d.b) - part_of(d.d, 2), 2)),
    );
    set("span", d.h);
    set("terminal_gap", d.p);
    set("terminal_width", d.w);
    set("lead_thickness", LEAD_NM.min(part_of(base, 3)));
    let can = d.l - base;
    set("bead_height", BEAD_HEIGHT_NM.min(part_of(can, 4)));
    set("bead_depth", BEAD_DEPTH_NM.min(part_of(d.d, 30)));
    set("rim_round", RIM_NM.min(part_of(d.d, 18)));
    v1::Model::PartFile {
        asset: if d.vent {
            parts::ELECTROLYTIC_SMD_VENT
        } else {
            parts::ELECTROLYTIC_SMD
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
    let envelope = d.envelope();
    let body = Box2::centred([0, 0], [d.a, d.b]);
    let courtyard = land::courtyard(
        &pads,
        Some(Box2::centred([0, 0], [envelope[0], envelope[1]])),
    )
    .unwrap_or_default();
    let top = courtyard.iter().map(|p| p[1]).max().unwrap_or(body.max[1]);
    let mut silk = land::silk_outline(&pads, body, land::DEFAULT_MASK_NM);
    silk.push(land::silk_designator(body, top));
    if let Some(first) = pads.first() {
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
    let x = d.contact_x();
    let package = Package {
        terminals: vec!["1".into(), "2".into()],
        geometry: PackageGeometry {
            envelope_nm: envelope,
            seating_nm: 0,
            leads: vec![
                Lead {
                    key: "1".into(),
                    contact: [-x, 0, 0],
                },
                Lead {
                    key: "2".into(),
                    contact: [x, 0, 0],
                },
            ],
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
    let name = format!(
        "Aluminium electrolytic {} × {} mm",
        mm_text(d.d).trim_end_matches(" mm"),
        mm_text(d.l).trim_end_matches(" mm"),
    );
    Ok(Generation {
        orderable: Vec::new(),
        marking: Some(marking(
            sparksmith_generator_sdk::marking::height(&args).map_err(|p| vec![p])?,
        )),
        component: component_part.then(|| Component {
            tie: None,
            assembly: Default::default(),
            terminals: [("1", "+"), ("2", "−")]
                .iter()
                .map(|(key, name)| Terminal {
                    key: (*key).into(),
                    name: (*name).into(),
                    direction: Direction::Passive,
                    alternates: Vec::new(),
                    drive: None,
                })
                .collect(),
            prefix: "C".into(),
            name,
            fields: vec![String::from("value")],
        }),
        symbol: component_part.then(symbol),
        package,
        footprint,
        binding: if component_part {
            ["1", "2"]
                .iter()
                .map(|k| BindingEntry {
                    package_terminal: (*k).into(),
                    terminal: (*k).into(),
                })
                .collect()
        } else {
            Vec::new()
        },
        model: Some(model(&d)),
        fields: Vec::new(),
        notes: vec![
            "Lands are the manufacturer's (a, b, c) as given; the seat plate's height, the \
             terminal thickness, the seal bead and the rim are estimated."
                .into(),
        ],
    })
}

impl Generator for Electrolytic {
    fn describe() -> Description {
        description()
    }

    fn generate(request: &Request) -> Result<Generation, Vec<Problem>> {
        generate(request)
    }
}

#[cfg(test)]
mod tests;

/// The can top's print: the capacitance in µF and the rated voltage in
/// black on the bare part of the top (the part file's `marking-top`,
/// beside the inlaid negative sector), at `text_height`.
fn marking(text_height: Option<i64>) -> v1::Marking {
    v1::Marking {
        lines: sparksmith_generator_sdk::marking::electrolytic_lines(),
        text_height,
        colour: Some(sparksmith_generator_sdk::marking::CAN_TOP_INK),
        ..v1::Marking::default()
    }
}
