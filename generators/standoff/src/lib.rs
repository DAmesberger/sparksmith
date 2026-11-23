//! The standoff generator: surface-mount spacers with an internal thread
//! (a tin-plated steel tube soldered to an annular land, with or without a
//! spigot that locates it in the board's hole) as a symbol, package,
//! footprint and 3D model.
//!
//! The parameters are the drawing's: the tube (`length` above the board,
//! `outer_diameter`, `bore_diameter`, the thread's minor diameter), the
//! spigot (`spigot_diameter`, `spigot_length`; 0 for none) and the
//! manufacturer's land (`land_diameter` round a non-plated hole of
//! `hole_diameter`), taken as given. The chamfers are estimated.
#![cfg_attr(target_arch = "wasm32", no_std)]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use sparksmith_generator_sdk::args::{Args, checked, problem};
use sparksmith_generator_sdk::draw::{Drawing, UP};
use sparksmith_generator_sdk::land::{self, Box2};
use sparksmith_generator_sdk::units::{MM, UM, div_round, mm_text};
use sparksmith_generator_sdk::v1::{
    self, BindingEntry, Capability, Component, Description, Direction, Effect, Fill, Footprint,
    Generation, Grammar, Hole, HoleShape, Lead, Opening, Package, PackageGeometry, Pad, PadShape,
    Param, ParamType, ParamValue, PartOffer, Preset, Primitive, Problem, Request, Rule, Scope,
    Side, Symbol, Terminal,
};
use sparksmith_generator_sdk::{Generator, export_generator};

/// The presets, in nm, with their citations.
const PRESETS: &str = include_str!("../presets.json");

/// The generator's version (`Description::version`).
/// 3: the SDK's pin table (`pin_table`) and part numbers
/// (`part_numbers`).
const VERSION: u32 = 3;

/// The top outer chamfer and the bore's mouth chamfer (est.), each at most
/// an eighth of the wall.
const CHAMFER_NM: i64 = 300_000;
const BORE_CHAMFER_NM: i64 = 250_000;

pub struct Standoff;

export_generator!(Standoff);

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

fn description() -> Description {
    description_with(sparksmith_generator_sdk::presets(PRESETS).unwrap_or_default())
}

fn description_with(presets: Vec<Preset>) -> Description {
    // The defaults are Würth Elektronik's 9774100360R (M3, 10 mm).
    let params = vec![
        length(
            "length",
            "Length above the board",
            [500 * UM, 50 * MM],
            10_000_000,
            "spacer",
        ),
        length(
            "outer_diameter",
            "Outer diameter",
            [MM, 20 * MM],
            6_000_000,
            "spacer",
        ),
        length(
            "bore_diameter",
            "Bore (thread minor diameter)",
            [300 * UM, 15 * MM],
            2_500_000,
            "spacer",
        ),
        length(
            "spigot_diameter",
            "Spigot diameter",
            [0, 20 * MM],
            4_200_000,
            "spigot",
        ),
        length(
            "spigot_length",
            "Spigot length below the board (0: none)",
            [0, 10 * MM],
            1_400_000,
            "spigot",
        ),
        length(
            "land_diameter",
            "Land diameter",
            [MM, 30 * MM],
            7_400_000,
            "land",
        ),
        length(
            "hole_diameter",
            "Board hole diameter (non-plated)",
            [0, 20 * MM],
            4_400_000,
            "land",
        ),
    ];
    let rule = |text: &str, fields: &[&str]| Rule {
        text: text.into(),
        fields: fields.iter().map(|f| (*f).into()).collect(),
    };
    let rules = vec![
        rule(
            "bore < outer diameter; with a spigot, bore < spigot < outer diameter and spigot < hole",
            &[
                "bore_diameter",
                "outer_diameter",
                "spigot_diameter",
                "hole_diameter",
            ],
        ),
        rule("hole < land diameter", &["hole_diameter", "land_diameter"]),
    ];
    let offer = PartOffer {
        families: vec!["standoff".into()],
        scopes: vec![Scope::Component, Scope::Package],
        grammar: vec![Grammar {
            scope: Scope::Component,
            patterns: vec!["smd spacer {size:code}".into()],
            examples: vec!["smd spacer M3 10 mm".into()],
        }],
        instance_fields: Vec::new(),
    };
    Description {
        abi: v1::ABI,
        id: "standoff".into(),
        version: VERSION,
        title: "SMD spacers with internal thread".into(),
        licence: "MIT OR Apache-2.0".into(),
        deterministic: true,
        capabilities: vec![Capability {
            name: v1::GENERATE_PART.into(),
            effect: Effect::None,
            title: Some("Make a surface-mount spacer with an internal thread".into()),
            params,
            rules,
            presets,
            part: Some(offer),
        }],
        needs: Vec::new(),
    }
}

struct Dims {
    length: i64,
    od: i64,
    bore: i64,
    spigot: Option<[i64; 2]>,
    land: i64,
    hole: i64,
}

fn part_of(n: i64, d: i64) -> i64 {
    div_round(n, d).unwrap_or(0)
}

fn dims(args: &Args<'_>) -> Result<Dims, Vec<Problem>> {
    let one = |p: Problem| vec![p];
    let int = |k: &str| args.int(k).map_err(one);
    let spigot_length = int("spigot_length")?;
    let d = Dims {
        length: int("length")?,
        od: int("outer_diameter")?,
        bore: int("bore_diameter")?,
        spigot: (spigot_length > 0)
            .then_some(int("spigot_diameter")?)
            .map(|dia| [dia, spigot_length]),
        land: int("land_diameter")?,
        hole: int("hole_diameter")?,
    };
    let mut problems = Vec::new();
    let mut rule = |ok: bool, field: &str, message: &str| {
        if !ok {
            problems.push(problem("param.inconsistent", field, message.into()));
        }
    };
    rule(
        d.bore + 2 * 200 * UM <= d.od,
        "bore_diameter",
        "The bore must leave a wall of at least 0.2 mm (bore + 0.4 mm ≤ outer diameter).",
    );
    if let Some([dia, _]) = d.spigot {
        rule(
            dia > d.bore + 2 * 100 * UM && dia < d.od,
            "spigot_diameter",
            "The spigot must lie between the bore (with a 0.1 mm wall) and the outer diameter.",
        );
        rule(
            d.hole > dia,
            "hole_diameter",
            "The board's hole must clear the spigot (hole > spigot).",
        );
    }
    rule(
        d.hole < d.land,
        "hole_diameter",
        "The hole must lie inside the land (hole < land diameter).",
    );
    if problems.is_empty() {
        Ok(d)
    } else {
        Err(problems)
    }
}

/// A mounting part (as a mounting hole is drawn): a circle with a cross,
/// its one terminal below.
fn symbol() -> Symbol {
    let mut d = Drawing::default();
    let m = |v: i64| v * MM / 1000;
    d.pin("1", [0, m(-5000)], UP, m(2500));
    d.open(Primitive::Circle {
        center: [0, 0],
        radius_nm: m(2500),
    });
    d.line([m(-1750), m(-1750)], [m(1750), m(1750)]);
    d.line([m(-1750), m(1750)], [m(1750), m(-1750)]);
    d.labels(m(2500), m(-5000));
    Symbol {
        units: vec![d.unit("A")],
    }
}

mod parts {
    include!(concat!(env!("OUT_DIR"), "/parts.rs"));
}

fn model(d: &Dims) -> v1::Model {
    let mut values = v1::Params::new();
    let mut set = |name: &str, nm: i64| {
        values.insert(name.into(), ParamValue::Int(nm));
    };
    let wall = part_of(d.od - d.bore, 2);
    set("length", d.length);
    set("outer_diameter", d.od);
    set("bore_diameter", d.bore);
    set(
        "chamfer",
        CHAMFER_NM.min(part_of(wall, 4)).min(part_of(d.length, 4)),
    );
    set(
        "bore_chamfer",
        BORE_CHAMFER_NM
            .min(part_of(wall, 4))
            .min(part_of(d.length, 4)),
    );
    let asset = match d.spigot {
        Some([dia, len]) => {
            set("spigot_diameter", dia);
            set("spigot_length", len);
            parts::STANDOFF_SPIGOT
        }
        None => parts::STANDOFF,
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
    // The land, and the board's non-plated hole through it (a mechanical
    // hole: it lands no terminal).
    let mut pads = vec![Pad {
        key: "1".into(),
        terminal: "1".into(),
        shape: PadShape::Circle {
            diameter_nm: d.land,
        },
        at: [0, 0],
        rot_mdeg: 0,
        side: Side::Top,
        hole: None,
        mask: Opening::Default,
        paste: Opening::Default,
        number: None,
        kind: None,
    }];
    if d.hole > 0 {
        pads.push(Pad {
            key: "hole".into(),
            terminal: "1".into(),
            shape: PadShape::Circle {
                diameter_nm: d.hole,
            },
            at: [0, 0],
            rot_mdeg: 0,
            side: Side::Top,
            hole: Some(Hole {
                shape: HoleShape::Round {
                    diameter_nm: d.hole,
                },
                plated: false,
                offset: None,
            }),
            mask: Opening::Default,
            paste: Opening::None,
            number: None,
            kind: None,
        });
    }
    let envelope = [d.od, d.od, d.length];
    let body = Box2::centred([0, 0], [d.od, d.od]);
    let courtyard = land::courtyard(&pads, Some(body)).unwrap_or_default();
    let top = courtyard.iter().map(|p| p[1]).max().unwrap_or(body.max[1]);
    let silk = vec![land::silk_designator(body, top)];
    // The contact on the tube's wall, clear of the bore.
    let x = part_of(part_of(d.od, 2) + part_of(d.bore, 2), 2);
    let package = Package {
        terminals: vec!["1".into()],
        geometry: PackageGeometry {
            envelope_nm: envelope,
            seating_nm: 0,
            leads: vec![Lead {
                key: "1".into(),
                contact: [x, 0, 0],
            }],
            pin1: "1".into(),
        },
    };
    let footprint = Footprint {
        bridges: Vec::new(),
        pads,
        courtyard,
        silk,
        assembly: vec![v1::Graphic {
            key: "assembly-outline".into(),
            shape: Primitive::Circle {
                center: [0, 0],
                radius_nm: part_of(d.od, 2),
            },
            stroke_nm: land::ASSEMBLY_STROKE_NM,
            fill: Fill::None,
        }],
        keepouts: Vec::new(),
    };
    let component_part = request.scope == Scope::Component;
    let name = format!(
        "SMD spacer Ø{} × {} mm",
        mm_text(d.od).trim_end_matches(" mm"),
        mm_text(d.length).trim_end_matches(" mm"),
    );
    Ok(Generation {
        orderable: Vec::new(),
        marking: None,
        component: component_part.then(|| Component {
            tie: None,
            assembly: Default::default(),
            terminals: vec![Terminal {
                key: "1".into(),
                name: "1".into(),
                direction: Direction::Passive,
                alternates: Vec::new(),
                drive: None,
            }],
            prefix: "H".into(),
            name,
            fields: vec![String::from("value")],
        }),
        symbol: component_part.then(symbol),
        package,
        footprint,
        binding: if component_part {
            vec![BindingEntry {
                package_terminal: "1".into(),
                terminal: "1".into(),
            }]
        } else {
            Vec::new()
        },
        model: Some(model(&d)),
        fields: Vec::new(),
        notes: vec![
            "The land and hole are the manufacturer's as given; the chamfers are estimated.".into(),
        ],
    })
}

impl Generator for Standoff {
    fn describe() -> Description {
        description()
    }

    fn generate(request: &Request) -> Result<Generation, Vec<Problem>> {
        generate(request)
    }
}

#[cfg(test)]
mod tests;
