//! The power-inductor generator: surface-mount power inductors with two
//! bottom terminals, moulded (a block of magnetic composite) or shielded
//! wound (a ferrite sleeve round a drum core, closed or with winding
//! windows), as a symbol, package, footprint and 3D model.
//!
//! The parameters are the drawing's: the body (`body_length`,
//! `body_width`, `height`), the bottom terminals (`terminal_length` along
//! x, `terminal_width` across, `terminal_gap` between them) and the
//! manufacturer's land pattern (each land `land_length` × `land_width`,
//! centred `land_x` from the middle), taken as given. A shielded part's
//! terminals reach the body's ends (gap + 2 × terminal length = body
//! length). The rounds, bevels and the drum core inside are estimated.
#![cfg_attr(target_arch = "wasm32", no_std)]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use sparksmith_generator_sdk::args::{Args, checked, problem};
use sparksmith_generator_sdk::draw::{self, Drawing};
use sparksmith_generator_sdk::land::{self, Box2};
use sparksmith_generator_sdk::units::{DEG, MM, UM, div_round, mm_text};
use sparksmith_generator_sdk::v1::{
    self, BindingEntry, Capability, Choice, Component, Description, Direction, Effect, Footprint,
    Generation, Grammar, InstanceField, Lead, Opening, Package, PackageGeometry, Pad, PadShape,
    Param, ParamType, ParamValue, PartOffer, Preset, Problem, Request, Rule, Scope, Side, Symbol,
    Terminal,
};
use sparksmith_generator_sdk::{Generator, export_generator};

/// The presets, in nm, with their citations.
const PRESETS: &str = include_str!("../presets.json");

/// The generator's version (`Description::version`).
/// 3: the SDK's pin table (`pin_table`) and part numbers
/// (`part_numbers`).
const VERSION: u32 = 3;

/// Moulded: the terminals' plating, the upright and top rounds (est.).
const PLATING_NM: i64 = 50_000;
const CORNER_ROUND_NM: i64 = 200_000;
const EDGE_ROUND_NM: i64 = 150_000;
/// Shielded (est.): the terminals' thickness, the sleeve's corner bevel
/// (at most 15 % of the body), the bore's wall to the body's side, the
/// flange's clearance in the bore, the winding's clearance to the bore,
/// the flange's thickness and the window (at most 44 % of the body).
const TERMINAL_NM: i64 = 80_000;
const BEVEL_NM: i64 = 1_000_000;
const SLEEVE_WALL_NM: i64 = 450_000;
const FLANGE_CLEARANCE_NM: i64 = 150_000;
const WINDING_CLEARANCE_NM: i64 = 350_000;
const FLANGE_NM: i64 = 500_000;
const WINDOW_NM: i64 = 3_000_000;

pub struct PowerInductor;

export_generator!(PowerInductor);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Style {
    Molded,
    Shielded,
}

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
    // The defaults are Coilcraft's XEL4030.
    let params = vec![
        Param {
            key: "style".into(),
            label: "Construction".into(),
            help: None,
            ty: ParamType::Enum {
                choices: vec![
                    Choice {
                        key: "molded".into(),
                        label: "Moulded composite".into(),
                    },
                    Choice {
                        key: "shielded".into(),
                        label: "Shielded wound (ferrite sleeve)".into(),
                    },
                ],
            },
            default: ParamValue::Text("molded".into()),
            group: Some("body".into()),
            context: None,
        },
        length(
            "body_length",
            "Body length (x)",
            [MM, 30 * MM],
            4_000_000,
            "body",
        ),
        length(
            "body_width",
            "Body width (y)",
            [MM, 30 * MM],
            4_000_000,
            "body",
        ),
        length("height", "Height", [500 * UM, 20 * MM], 3_100_000, "body"),
        length(
            "terminal_length",
            "Terminal length (x)",
            [100 * UM, 15 * MM],
            820_000,
            "terminals",
        ),
        length(
            "terminal_width",
            "Terminal width (y)",
            [100 * UM, 30 * MM],
            3_250_000,
            "terminals",
        ),
        length(
            "terminal_gap",
            "Gap between the terminals",
            [100 * UM, 25 * MM],
            1_570_000,
            "terminals",
        ),
        Param {
            key: "window".into(),
            label: "Winding windows".into(),
            help: Some("Shielded: the sleeve open over its ±y sides, the winding showing.".into()),
            ty: ParamType::Bool,
            default: ParamValue::Bool(false),
            group: Some("body".into()),
            context: None,
        },
        length(
            "land_x",
            "Land centre from the middle (x)",
            [100 * UM, 20 * MM],
            1_185_000,
            "lands",
        ),
        length(
            "land_length",
            "Land length (x)",
            [200 * UM, 15 * MM],
            980_000,
            "lands",
        ),
        length(
            "land_width",
            "Land width (y)",
            [200 * UM, 30 * MM],
            3_400_000,
            "lands",
        ),
    ];
    let rule = |text: &str, fields: &[&str]| Rule {
        text: text.into(),
        fields: fields.iter().map(|f| (*f).into()).collect(),
    };
    let rules = vec![
        rule(
            "gap + 2 × terminal length ≤ body length (shielded: =); terminal width ≤ body width",
            &[
                "terminal_gap",
                "terminal_length",
                "body_length",
                "terminal_width",
                "body_width",
            ],
        ),
        rule(
            "The lands are apart: land x > land length / 2",
            &["land_x", "land_length"],
        ),
    ];
    let offer = PartOffer {
        families: vec!["power-inductor".into()],
        scopes: vec![Scope::Component, Scope::Package],
        grammar: vec![Grammar {
            scope: Scope::Component,
            patterns: vec!["power inductor {size:code}".into()],
            examples: vec!["power inductor 4x4".into(), "power inductor 6.8x6.8".into()],
        }],
        instance_fields: vec![InstanceField {
            standard_field: "value".into(),
            when: v1::Params::new(),
        }],
    };
    Description {
        abi: v1::ABI,
        id: "power-inductor".into(),
        version: VERSION,
        title: "Power inductors (SMD)".into(),
        licence: "MIT OR Apache-2.0".into(),
        deterministic: true,
        capabilities: vec![Capability {
            name: v1::GENERATE_PART.into(),
            effect: Effect::None,
            title: Some("Make a surface-mount power inductor".into()),
            params,
            rules,
            presets,
            part: Some(offer),
        }],
        needs: Vec::new(),
    }
}

struct Dims {
    style: Style,
    bl: i64,
    bw: i64,
    h: i64,
    tl: i64,
    tw: i64,
    gap: i64,
    window: bool,
    land_x: i64,
    land: [i64; 2],
}

fn part_of(n: i64, d: i64) -> i64 {
    div_round(n, d).unwrap_or(0)
}

impl Dims {
    fn contact_x(&self) -> i64 {
        part_of(self.gap + self.tl, 2)
    }

    /// The shielded sleeve's bore, flange and winding diameters.
    fn drum(&self) -> [i64; 3] {
        let bore = self.bl.min(self.bw) - 2 * SLEEVE_WALL_NM;
        [
            bore,
            bore - 2 * FLANGE_CLEARANCE_NM,
            bore - 2 * WINDING_CLEARANCE_NM,
        ]
    }

    fn flange(&self) -> i64 {
        FLANGE_NM.min(part_of(self.h, 6))
    }
}

fn dims(args: &Args<'_>) -> Result<Dims, Vec<Problem>> {
    let one = |p: Problem| vec![p];
    let int = |k: &str| args.int(k).map_err(one);
    let key = args.text("style").map_err(one)?;
    let style = match key {
        "molded" => Style::Molded,
        "shielded" => Style::Shielded,
        _ => {
            return Err(vec![problem(
                "param.out_of_range",
                "style",
                format!("Construction \"{key}\" is not one this generator makes."),
            )]);
        }
    };
    let d = Dims {
        style,
        bl: int("body_length")?,
        bw: int("body_width")?,
        h: int("height")?,
        tl: int("terminal_length")?,
        tw: int("terminal_width")?,
        gap: int("terminal_gap")?,
        window: args.flag("window").map_err(one)?,
        land_x: int("land_x")?,
        land: [int("land_length")?, int("land_width")?],
    };
    let mut problems = Vec::new();
    let mut rule = |ok: bool, field: &str, message: &str| {
        if !ok {
            problems.push(problem("param.inconsistent", field, message.into()));
        }
    };
    let reach = d.gap + 2 * d.tl;
    match style {
        Style::Molded => rule(
            reach <= d.bl,
            "terminal_gap",
            "The terminals must lie under the body (gap + 2 × terminal length ≤ body length).",
        ),
        Style::Shielded => {
            rule(
                reach == d.bl,
                "terminal_gap",
                "A shielded part's terminals reach the body's ends (gap + 2 × terminal length = body length).",
            );
            rule(
                d.drum()[2] > MM && d.h > 3 * d.flange(),
                "height",
                "The body is too small for a sleeve round a drum core (at least 1.9 mm across, \
                 three flanges high).",
            );
        }
    }
    rule(
        d.tw <= d.bw,
        "terminal_width",
        "The terminals must not be wider than the body.",
    );
    rule(
        2 * d.land_x > d.land[0],
        "land_x",
        "The two lands must not meet (land x > land length / 2).",
    );
    if problems.is_empty() {
        Ok(d)
    } else {
        Err(problems)
    }
}

fn pads(d: &Dims) -> Vec<Pad> {
    [("1", -d.land_x), ("2", d.land_x)]
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

/// The inductor of the host's standard drawings: four half-turns.
fn symbol() -> Symbol {
    let mut d = Drawing::default();
    let m = |v: i64| v * MM / 1000;
    d.two_pins("1", "2", m(2500), m(2500));
    for k in 0..4i64 {
        if let Some(arc) = draw::arc([m(-1875) + k * m(1250), 0], m(625), 180 * DEG, -180 * DEG) {
            d.open(arc);
        }
    }
    d.labels(m(625), m(-500));
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
    set("body_length", d.bl);
    set("body_width", d.bw);
    set("height", d.h);
    set("terminal_length", d.tl);
    set("terminal_width", d.tw);
    let small = d.bl.min(d.bw);
    let asset = match d.style {
        Style::Molded => {
            set("terminal_gap", d.gap);
            set("terminal_thickness", PLATING_NM);
            set("corner_round", CORNER_ROUND_NM.min(part_of(small, 20)));
            set("edge_round", EDGE_ROUND_NM.min(part_of(d.h, 20)));
            parts::INDUCTOR_MOLDED
        }
        Style::Shielded => {
            let [bore, flange, winding] = d.drum();
            set("corner_bevel", BEVEL_NM.min(part_of(15 * small, 100)));
            set("bore_diameter", bore);
            set("flange_diameter", flange);
            set("flange_thickness", d.flange());
            set("winding_diameter", winding);
            set("terminal_thickness", TERMINAL_NM);
            set("window_width", WINDOW_NM.min(part_of(44 * d.bl, 100)));
            if d.window {
                parts::INDUCTOR_SHIELDED_WINDOW
            } else {
                parts::INDUCTOR_SHIELDED
            }
        }
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
    let envelope = [d.bl, d.bw, d.h];
    let body = Box2::centred([0, 0], [d.bl, d.bw]);
    let courtyard = land::courtyard(&pads, Some(body)).unwrap_or_default();
    let top = courtyard.iter().map(|p| p[1]).max().unwrap_or(body.max[1]);
    let mut silk = land::silk_outline(&pads, body, land::DEFAULT_MASK_NM);
    silk.push(land::silk_designator(body, top));
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
        "Power inductor {} × {} × {} mm",
        mm_text(d.bl).trim_end_matches(" mm"),
        mm_text(d.bw).trim_end_matches(" mm"),
        mm_text(d.h).trim_end_matches(" mm"),
    );
    Ok(Generation {
        orderable: Vec::new(),
        marking: None,
        component: component_part.then(|| Component {
            tie: None,
            assembly: Default::default(),
            terminals: ["1", "2"]
                .iter()
                .map(|key| Terminal {
                    key: (*key).into(),
                    name: (*key).into(),
                    direction: Direction::Passive,
                    alternates: Vec::new(),
                    drive: None,
                })
                .collect(),
            prefix: "L".into(),
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
            "Lands are the manufacturer's as given; rounds, bevels and the drum core are \
             estimated."
                .into(),
        ],
    })
}

impl Generator for PowerInductor {
    fn describe() -> Description {
        description()
    }

    fn generate(request: &Request) -> Result<Generation, Vec<Problem>> {
        generate(request)
    }
}

#[cfg(test)]
mod tests;
