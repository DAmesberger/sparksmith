//! The fiducial generator: the optical alignment marks a pick-and-place
//! machine and a stencil printer find the board by, as a symbol, package
//! and footprint, complete without a body.
//!
//! A fiducial is a round land of bare copper (a pad of kind fiducial, which
//! lands no terminal and takes no paste) in a solder mask opening, inside a
//! clear area that the board keeps free of copper, silkscreen and other
//! parts. The clear area is the footprint's courtyard (other parts' courtyards
//! may not enter it) and a copper keep-out on the fiducial's own outer layer
//! (no fill, track or via in it). The parameters are the assembler's
//! guideline values, taken as given: `copper_diameter`, `mask_diameter` (the
//! opening) and `clear_diameter`; `sides` puts a second dot on the far side
//! of the board. The part has no body (`Model::NoBody`): the board plan
//! draws the land, so the 3D view and the exports add nothing for it.
#![cfg_attr(target_arch = "wasm32", no_std)]

extern crate alloc;

use alloc::format;
use alloc::vec;
use alloc::vec::Vec;
use sparksmith_generator_sdk::args::{Args, checked, problem};
use sparksmith_generator_sdk::draw::{self, Drawing};
use sparksmith_generator_sdk::land;
use sparksmith_generator_sdk::units::{MM, UM, div_round, mm_text};
use sparksmith_generator_sdk::v1::{
    self, Capability, Choice, Component, Description, Effect, Fill, Footprint, Generation, Grammar,
    Keepout, Opening, Package, PackageGeometry, Pad, PadKind, PadShape, Param, ParamType,
    ParamValue, PartOffer, Point, Preset, Primitive, Problem, Request, Rule, Scope, Side, Symbol,
};
use sparksmith_generator_sdk::{Generator, export_generator};

/// The presets, in nm, with their citations.
const PRESETS: &str = include_str!("../presets.json");

/// The generator's version (`Description::version`).
/// 2: the SDK's pin table (`pin_table`) and part numbers
/// (`part_numbers`).
const VERSION: u32 = 3;

/// The package envelope's height: 1 oz outer copper, 35 µm (IPC-4562's
/// nominal 34.8 µm, rounded as fabricators quote it; the fab profiles'
/// copper weight), the land's own thickness. The part has no body: the
/// board draws the land.
const DOT_THICKNESS_NM: i64 = 35_000;

/// Corners of the polygon that stands for the round clear area (courtyard
/// and keep-out, which the ABI gives as polygons). The polygon is drawn
/// round the circle, so it covers all of it.
const CLEAR_CORNERS: i64 = 32;

pub struct Fiducial;

export_generator!(Fiducial);

fn length(key: &str, label: &str, [min_nm, max_nm]: [i64; 2], default: i64) -> Param {
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
        group: Some("fiducial".into()),
        context: None,
    }
}

fn description() -> Description {
    description_with(sparksmith_generator_sdk::presets(PRESETS).unwrap_or_default())
}

fn description_with(presets: Vec<Preset>) -> Description {
    // The defaults are JLCPCB's: a 1 mm dot in a 2 mm opening, the clear
    // area the opening (presets.json cites them).
    let params = vec![
        length(
            "copper_diameter",
            "Copper diameter",
            [100 * UM, 10 * MM],
            MM,
        ),
        length(
            "mask_diameter",
            "Solder mask opening diameter",
            [100 * UM, 20 * MM],
            2 * MM,
        ),
        length(
            "clear_diameter",
            "Clear area diameter (no copper, silkscreen or parts)",
            [100 * UM, 20 * MM],
            2 * MM,
        ),
        Param {
            key: "sides".into(),
            label: "Sides".into(),
            help: Some(
                "One: the dot on the side the part is placed on (place it on the bottom for a bottom fiducial). Both: a second dot opposite it on the far side.".into(),
            ),
            ty: ParamType::Enum {
                choices: vec![
                    Choice {
                        key: "one".into(),
                        label: "The side it is placed on".into(),
                    },
                    Choice {
                        key: "both".into(),
                        label: "Both sides".into(),
                    },
                ],
            },
            default: ParamValue::Text("one".into()),
            group: Some("fiducial".into()),
            context: None,
        },
    ];
    let rule = |text: &str, fields: &[&str]| Rule {
        text: text.into(),
        fields: fields.iter().map(|f| (*f).into()).collect(),
    };
    let rules = vec![rule(
        "copper < mask opening ≤ clear area",
        &["copper_diameter", "mask_diameter", "clear_diameter"],
    )];
    let offer = PartOffer {
        families: vec!["fiducial".into()],
        scopes: vec![Scope::Component, Scope::Package],
        grammar: vec![Grammar {
            scope: Scope::Component,
            patterns: vec!["fiducial {size:code}".into()],
            examples: vec!["fiducial 1 mm".into()],
        }],
        instance_fields: Vec::new(),
    };
    Description {
        abi: v1::ABI,
        id: "fiducial".into(),
        version: VERSION,
        title: "Fiducials".into(),
        licence: "MIT OR Apache-2.0".into(),
        deterministic: true,
        capabilities: vec![Capability {
            name: v1::GENERATE_PART.into(),
            effect: Effect::None,
            title: Some("Make a fiducial: a bare copper dot in a clear area".into()),
            params,
            rules,
            presets,
            part: Some(offer),
        }],
        needs: Vec::new(),
    }
}

struct Dims {
    copper: i64,
    mask: i64,
    clear: i64,
    both: bool,
}

fn half(v: i64) -> i64 {
    div_round(v, 2).unwrap_or(0)
}

fn dims(args: &Args<'_>) -> Result<Dims, Vec<Problem>> {
    let one = |p: Problem| vec![p];
    let int = |k: &str| args.int(k).map_err(one);
    let d = Dims {
        copper: int("copper_diameter")?,
        mask: int("mask_diameter")?,
        clear: int("clear_diameter")?,
        both: args.text("sides").map_err(one)? == "both",
    };
    let mut problems = Vec::new();
    if d.mask <= d.copper {
        problems.push(problem(
            "param.inconsistent",
            "mask_diameter",
            "The solder mask opening must be wider than the copper dot (mask opening > copper diameter).".into(),
        ));
    }
    if d.clear < d.mask {
        problems.push(problem(
            "param.inconsistent",
            "clear_diameter",
            "The clear area must hold the whole mask opening, so no other copper shows in it (clear area ≥ mask opening).".into(),
        ));
    }
    if problems.is_empty() {
        Ok(d)
    } else {
        Err(problems)
    }
}

/// A polygon of [`CLEAR_CORNERS`] corners whose edges touch the circle of
/// `diameter` about the origin from outside, so it covers the circle: each
/// corner at the radius over cos(180° / corners), rounded up to the nm.
fn round_area(diameter: i64) -> Vec<Point> {
    let turn = 360_000;
    let step = turn / CLEAR_CORNERS;
    let (cos, _) = draw::cos_sin(step / 2);
    let one: i128 = 1 << 60;
    let r = i128::from(half(diameter));
    let corner = if cos > 0 {
        i64::try_from((r * one + cos - 1) / cos).unwrap_or(0) + 1
    } else {
        0
    };
    (0..CLEAR_CORNERS)
        .filter_map(|k| draw::on_circle([0, 0], corner, k * step))
        .collect()
}

/// A board feature (as a fiducial is drawn): a filled dot in a ring, with
/// no pins, since it lands no terminal.
fn symbol() -> Symbol {
    let mut d = Drawing::default();
    let m = |v: i64| v * MM / 1000;
    d.open(Primitive::Circle {
        center: [0, 0],
        radius_nm: m(1250),
    });
    d.shape(
        Primitive::Circle {
            center: [0, 0],
            radius_nm: m(500),
        },
        0,
        Fill::Solid,
    );
    d.labels(m(1250), m(-1250));
    Symbol {
        units: vec![d.unit("A")],
    }
}

fn dot(key: &str, side: Side, d: &Dims) -> Pad {
    Pad {
        key: key.into(),
        terminal: "".into(),
        shape: PadShape::Circle {
            diameter_nm: d.copper,
        },
        at: [0, 0],
        rot_mdeg: 0,
        side,
        hole: None,
        mask: Opening::Expansion(half(d.mask - d.copper)),
        paste: Opening::None,
        number: None,
        kind: Some(PadKind::Fiducial),
    }
}

fn generate(request: &Request) -> Result<Generation, Vec<Problem>> {
    let description = description_with(Vec::new());
    let args = checked(&description, &request.params)?;
    let d = dims(&args)?;
    let area = round_area(d.clear);
    let mut pads = vec![dot("1", Side::Top, &d)];
    let mut keepouts = vec![Keepout {
        name: "clear area".into(),
        outline: area.clone(),
        side: Some(Side::Top),
    }];
    if d.both {
        pads.push(dot("2", Side::Bottom, &d));
        keepouts.push(Keepout {
            name: "clear area (far side)".into(),
            outline: area.clone(),
            side: Some(Side::Bottom),
        });
    }
    let package = Package {
        terminals: Vec::new(),
        geometry: PackageGeometry {
            envelope_nm: [d.copper, d.copper, DOT_THICKNESS_NM],
            seating_nm: 0,
            leads: Vec::new(),
            pin1: "".into(),
        },
    };
    let footprint = Footprint {
        pads,
        courtyard: area,
        // No silkscreen: the clear area is kept free of it.
        silk: Vec::new(),
        assembly: vec![v1::Graphic {
            key: "assembly-outline".into(),
            shape: Primitive::Circle {
                center: [0, 0],
                radius_nm: half(d.copper),
            },
            stroke_nm: land::ASSEMBLY_STROKE_NM,
            fill: Fill::None,
        }],
        keepouts,
    };
    let component_part = request.scope == Scope::Component;
    let name = format!(
        "Fiducial Ø{} mm{}",
        mm_text(d.copper).trim_end_matches(" mm"),
        if d.both { ", both sides" } else { "" }
    );
    Ok(Generation {
        orderable: Vec::new(),
        marking: None,
        component: component_part.then(|| Component {
            assembly: v1::Assembly::CopperOnly,
            terminals: Vec::new(),
            prefix: "FID".into(),
            name,
            fields: Vec::new(),
        }),
        symbol: component_part.then(symbol),
        package,
        footprint,
        binding: Vec::new(),
        // Complete without a body: the land is the whole part, and the
        // board plan draws it.
        model: Some(v1::Model::NoBody),
        fields: Vec::new(),
        notes: vec![
            "The copper, mask opening and clear area are the assembler's guideline values as given; the part has no body of its own: the board draws the land.".into(),
        ],
    })
}

impl Generator for Fiducial {
    fn describe() -> Description {
        description()
    }

    fn generate(request: &Request) -> Result<Generation, Vec<Problem>> {
        generate(request)
    }
}

#[cfg(test)]
mod tests;
