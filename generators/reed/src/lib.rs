//! The reed generator: surface-mount reed switches (a glass envelope on
//! two formed leads) as a symbol, package, footprint and 3D model.
//!
//! The parameters are the drawing's: the envelope (`tube_length`,
//! `tube_diameter`), the seated `height`, the leads (`bend_span` over the
//! downward legs, `span` overall, `foot_width`) and the manufacturer's land
//! pattern (`land_gap` between the lands, each `land_length` ×
//! `land_width`), taken as given. The wire, seals, glass wall and reed
//! blades inside are estimated from the envelope.
//!
//! The presets are the manufacturer drawings they cite.
#![cfg_attr(target_arch = "wasm32", no_std)]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use sparksmith_generator_sdk::args::{Args, checked, problem};
use sparksmith_generator_sdk::draw::Drawing;
use sparksmith_generator_sdk::land::{self, Box2};
use sparksmith_generator_sdk::units::{MM, UM, div_round, mm_text};
use sparksmith_generator_sdk::v1::{
    self, BindingEntry, Capability, Component, Description, Direction, Effect, Fill, Footprint,
    Generation, Grammar, InstanceField, Lead, Opening, Package, PackageGeometry, Pad, PadShape,
    Param, ParamType, ParamValue, PartOffer, Preset, Primitive, Problem, Request, Rule, Scope,
    Side, Symbol, Terminal,
};
use sparksmith_generator_sdk::{Generator, export_generator};

/// The presets, in nm, with their citations.
const PRESETS: &str = include_str!("../presets.json");

/// The generator's version (`Description::version`).
const VERSION: u32 = 1;

/// Lead wire diameter (est.), at most a quarter of the envelope.
const WIRE_NM: i64 = 500_000;
/// Flattened foot thickness (est.).
const FOOT_NM: i64 = 200_000;
/// Tapered seal length (est.), at most an eighth of the envelope.
const SEAL_NM: i64 = 1_600_000;
/// Glass wall (est.), at most a seventh of the diameter.
const WALL_NM: i64 = 300_000;
/// Reed blades (est.): width, thickness, overlap and the open gap.
const BLADE_WIDTH_NM: i64 = 450_000;
const BLADE_NM: i64 = 150_000;
const OVERLAP_NM: i64 = 800_000;
const GAP_NM: i64 = 50_000;

pub struct Reed;

export_generator!(Reed);

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
    // The defaults are Littelfuse's MDSM-4.
    let params = vec![
        length(
            "tube_length",
            "Envelope length",
            [3 * MM, 60 * MM],
            15_240_000,
            "envelope",
        ),
        length(
            "tube_diameter",
            "Envelope diameter",
            [MM, 6 * MM],
            2_280_000,
            "envelope",
        ),
        length(
            "height",
            "Seated height",
            [MM, 10 * MM],
            2_840_000,
            "envelope",
        ),
        length(
            "bend_span",
            "Over the leads' legs",
            [4 * MM, 70 * MM],
            17_520_000,
            "leads",
        ),
        length(
            "span",
            "Overall length",
            [4 * MM, 75 * MM],
            19_300_000,
            "leads",
        ),
        length(
            "foot_width",
            "Foot width",
            [200 * UM, 3 * MM],
            1_000_000,
            "leads",
        ),
        length(
            "land_gap",
            "Gap between the lands",
            [2 * MM, 70 * MM],
            16_000_000,
            "lands",
        ),
        length(
            "land_length",
            "Land length",
            [300 * UM, 10 * MM],
            2_340_000,
            "lands",
        ),
        length(
            "land_width",
            "Land width",
            [300 * UM, 5 * MM],
            1_300_000,
            "lands",
        ),
    ];
    let rule = |text: &str, fields: &[&str]| Rule {
        text: text.into(),
        fields: fields.iter().map(|f| (*f).into()).collect(),
    };
    let rules = vec![
        rule(
            "The legs stand clear of the envelope: (bend span − envelope length) / 2 > wire",
            &["bend_span", "tube_length"],
        ),
        rule("Bend span < overall length", &["bend_span", "span"]),
        rule(
            "The envelope stands clear of the feet: height − diameter > foot",
            &["height", "tube_diameter"],
        ),
    ];
    let offer = PartOffer {
        families: vec!["reed".into()],
        scopes: vec![Scope::Component, Scope::Package],
        grammar: vec![Grammar {
            scope: Scope::Component,
            patterns: vec!["reed switch {size:code}".into()],
            examples: vec!["reed switch MDSM-4".into()],
        }],
        instance_fields: vec![InstanceField {
            standard_field: "value".into(),
            when: v1::Params::new(),
        }],
    };
    Description {
        abi: v1::ABI,
        id: "reed".into(),
        version: VERSION,
        title: "Reed switches (SMD)".into(),
        licence: "MIT OR Apache-2.0".into(),
        deterministic: true,
        capabilities: vec![Capability {
            name: v1::GENERATE_PART.into(),
            effect: Effect::None,
            title: Some("Make a surface-mount reed switch".into()),
            params,
            rules,
            presets,
            part: Some(offer),
        }],
        needs: Vec::new(),
    }
}

struct Dims {
    tube_length: i64,
    tube_diameter: i64,
    height: i64,
    bend_span: i64,
    span: i64,
    foot_width: i64,
    land_gap: i64,
    land: [i64; 2],
}

fn part_of(n: i64, d: i64) -> i64 {
    div_round(n, d).unwrap_or(0)
}

impl Dims {
    fn wire(&self) -> i64 {
        WIRE_NM.min(part_of(self.tube_diameter, 4))
    }

    fn foot(&self) -> i64 {
        FOOT_NM.min(part_of(self.height, 10))
    }

    fn envelope(&self) -> [i64; 3] {
        [
            self.span,
            self.tube_diameter.max(self.foot_width),
            self.height,
        ]
    }

    /// The x of terminal 2's contact: the middle of its foot.
    fn contact_x(&self) -> i64 {
        part_of(
            part_of(self.span, 2) + part_of(self.bend_span, 2) - self.wire(),
            2,
        )
    }
}

fn dims(args: &Args<'_>) -> Result<Dims, Vec<Problem>> {
    let one = |p: Problem| vec![p];
    let int = |k: &str| args.int(k).map_err(one);
    let d = Dims {
        tube_length: int("tube_length")?,
        tube_diameter: int("tube_diameter")?,
        height: int("height")?,
        bend_span: int("bend_span")?,
        span: int("span")?,
        foot_width: int("foot_width")?,
        land_gap: int("land_gap")?,
        land: [int("land_length")?, int("land_width")?],
    };
    let mut problems = Vec::new();
    let mut rule = |ok: bool, field: &str, message: &str| {
        if !ok {
            problems.push(problem("param.inconsistent", field, message.into()));
        }
    };
    rule(
        d.bend_span - d.tube_length > 2 * d.wire(),
        "bend_span",
        "The leads' legs must stand clear of the envelope's ends.",
    );
    rule(
        d.bend_span < d.span,
        "span",
        "The feet must reach past the legs (bend span < overall length).",
    );
    rule(
        d.height - d.tube_diameter > d.foot(),
        "height",
        "The envelope must stand clear of the feet (height − diameter > the foot).",
    );
    if problems.is_empty() {
        Ok(d)
    } else {
        Err(problems)
    }
}

fn pads(d: &Dims) -> Vec<Pad> {
    let x = part_of(d.land_gap + d.land[0], 2);
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
        })
        .collect()
}

/// A normally open switch (IEC 60617 as published) in its envelope: the
/// blade from terminal 1 lifted off terminal 2's contact.
fn symbol() -> Symbol {
    let mut d = Drawing::default();
    let m = |v: i64| v * MM / 1000;
    d.two_pins("1", "2", m(2500), m(2500));
    d.line([m(-2500), 0], [m(1500), m(1000)]);
    d.line([m(1250), 0], [m(2500), 0]);
    d.shape(
        Primitive::Rect {
            min: [m(-2000), m(-1000)],
            max: [m(2000), m(1750)],
        },
        sparksmith_generator_sdk::draw::STROKE_NM / 2,
        Fill::None,
    );
    d.labels(m(1750), m(-1000));
    Symbol {
        units: vec![d.unit("A")],
    }
}

mod parts {
    include!(concat!(env!("OUT_DIR"), "/parts.rs"));
}

/// The 3D model: the bundle's part file at the drawing's values and the
/// estimates for the wire, foot, seals, wall and blades.
fn model(d: &Dims) -> v1::Model {
    let mut values = v1::Params::new();
    let mut set = |name: &str, nm: i64| {
        values.insert(name.into(), ParamValue::Int(nm));
    };
    let wire = d.wire();
    let bore = d.tube_diameter - 2 * WALL_NM.min(part_of(d.tube_diameter, 7));
    set("tube_length", d.tube_length);
    set("tube_diameter", d.tube_diameter);
    set("height", d.height);
    set("bend_span", d.bend_span);
    set("span", d.span);
    set("lead_diameter", wire);
    set("foot_width", d.foot_width);
    set("foot_thickness", d.foot());
    set("seal_length", SEAL_NM.min(part_of(d.tube_length, 8)));
    set("seal_diameter", (2 * wire).min(d.tube_diameter - 200 * UM));
    set("wall", part_of(d.tube_diameter - bore, 2));
    set("blade_width", BLADE_WIDTH_NM.min(part_of(2 * bore, 5)));
    set("blade_thickness", BLADE_NM.min(part_of(bore, 8)));
    set("blade_overlap", OVERLAP_NM.min(part_of(d.tube_length, 10)));
    set("contact_gap", GAP_NM);
    v1::Model::PartFile {
        asset: parts::REED_SMD.into(),
        values,
    }
}

fn generate(request: &Request) -> Result<Generation, Vec<Problem>> {
    let description = description_with(Vec::new());
    let args = checked(&description, &request.params)?;
    let d = dims(&args)?;
    let pads = pads(&d);
    let envelope = d.envelope();
    let body = Box2::centred([0, 0], [d.tube_length, d.tube_diameter]);
    let courtyard = land::courtyard(
        &pads,
        Some(Box2::centred([0, 0], [envelope[0], envelope[1]])),
    )
    .unwrap_or_default();
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
        pads,
        courtyard,
        silk,
        assembly: land::assembly(body),
        keepouts: Vec::new(),
    };
    let component_part = request.scope == Scope::Component;
    let name = format!(
        "Reed switch {} × Ø{} mm",
        mm_text(d.tube_length).trim_end_matches(" mm"),
        mm_text(d.tube_diameter).trim_end_matches(" mm"),
    );
    Ok(Generation {
        marking: None,
        component: component_part.then(|| Component {
            terminals: ["1", "2"]
                .iter()
                .map(|key| Terminal {
                    key: (*key).into(),
                    name: (*key).into(),
                    direction: Direction::Passive,
                })
                .collect(),
            prefix: "SW".into(),
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
            "Lands are the manufacturer's as given; keep magnets and steel clear of the \
             envelope's axis as its datasheet says."
                .into(),
        ],
    })
}

impl Generator for Reed {
    fn describe() -> Description {
        description()
    }

    fn generate(request: &Request) -> Result<Generation, Vec<Problem>> {
        generate(request)
    }
}

#[cfg(test)]
mod tests;
