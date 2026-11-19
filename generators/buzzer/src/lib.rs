//! The buzzer generator: surface-mount magnetic buzzers (a square case on
//! two end terminals) as a symbol, package, footprint and 3D model.
//!
//! The parameters are the drawing's: the case (`body_length`,
//! `body_width`, `height`), the terminals (`terminal_width`, how far each
//! rises up the case end, `terminal_rise`, and stands proud of it,
//! `terminal_out`) and the manufacturer's land pattern (each land
//! `land_length` × `land_width`, centred `land_x` from the middle), taken
//! as given; `label` adds the masking label over the top. Terminal 1 is +
//! at −x; the case's bevelled corner is at +x +y. The bevel, the sound
//! port, the top round and the feet are estimated.
#![cfg_attr(target_arch = "wasm32", no_std)]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use sparksmith_generator_sdk::args::{Args, checked, problem};
use sparksmith_generator_sdk::draw::{self, Drawing, UP};
use sparksmith_generator_sdk::land::{self, Box2};
use sparksmith_generator_sdk::units::{DEG, MM, UM, div_round, mm_text};
use sparksmith_generator_sdk::v1::{
    self, BindingEntry, Capability, Component, Description, Direction, Effect, Fill, Footprint,
    Generation, Grammar, Graphic, InstanceField, Lead, Opening, Package, PackageGeometry, Pad,
    PadShape, Param, ParamType, ParamValue, PartOffer, Preset, Primitive, Problem, Request, Rule,
    Scope, Side, Symbol, Terminal,
};
use sparksmith_generator_sdk::{Generator, export_generator};

/// The presets, in nm, with their citations.
const PRESETS: &str = include_str!("../presets.json");

/// The generator's version (`Description::version`).
const VERSION: u32 = 2;

/// Estimates: the orientation bevel (at most an eighth of the case), the
/// top round (a tenth of the height), the sound port (a sixth of the case,
/// a quarter of the height deep), the foot under the case (an eighth of
/// it) and its thickness, and the masking label.
const BEVEL_NM: i64 = 1_200_000;
const TOP_ROUND_NM: i64 = 300_000;
const PORT_NM: i64 = 1_500_000;
const PORT_DEPTH_NM: i64 = 1_000_000;
const FOOT_NM: i64 = 1_200_000;
const FOOT_THICKNESS_NM: i64 = 100_000;
const LABEL_THICKNESS_NM: i64 = 50_000;
/// The pin-1 (+) dot beyond pad 1.
const PIN_ONE_DOT_RADIUS_NM: i64 = 200_000;

pub struct Buzzer;

export_generator!(Buzzer);

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
    // The defaults are CUI Devices' CMI-9653S-SMT.
    let params = vec![
        length(
            "body_length",
            "Case length (x)",
            [3 * MM, 40 * MM],
            9_600_000,
            "case",
        ),
        length(
            "body_width",
            "Case width (y)",
            [3 * MM, 40 * MM],
            9_600_000,
            "case",
        ),
        length("height", "Height", [MM, 20 * MM], 5_000_000, "case"),
        length(
            "terminal_width",
            "Terminal width",
            [200 * UM, 10 * MM],
            1_500_000,
            "terminals",
        ),
        length(
            "terminal_rise",
            "Terminal rise up the case end",
            [200 * UM, 20 * MM],
            2_400_000,
            "terminals",
        ),
        length(
            "terminal_out",
            "Terminal beyond the case end",
            [50 * UM, 2 * MM],
            250_000,
            "terminals",
        ),
        length(
            "land_x",
            "Land centre from the middle (x)",
            [MM, 30 * MM],
            3_800_000,
            "lands",
        ),
        length(
            "land_length",
            "Land length (x)",
            [300 * UM, 10 * MM],
            3_000_000,
            "lands",
        ),
        length(
            "land_width",
            "Land width (y)",
            [300 * UM, 10 * MM],
            2_500_000,
            "lands",
        ),
        Param {
            key: "label".into(),
            label: "Masking label".into(),
            help: Some("The polyimide label over the top, as delivered for washing.".into()),
            ty: ParamType::Bool,
            default: ParamValue::Bool(false),
            group: Some("case".into()),
            context: None,
        },
    ];
    let rule = |text: &str, fields: &[&str]| Rule {
        text: text.into(),
        fields: fields.iter().map(|f| (*f).into()).collect(),
    };
    let rules = vec![
        rule(
            "Terminal width < case width; terminal rise < height",
            &["terminal_width", "body_width", "terminal_rise", "height"],
        ),
        rule(
            "The lands are apart: land x > land length / 2",
            &["land_x", "land_length"],
        ),
    ];
    let offer = PartOffer {
        families: vec!["buzzer".into()],
        scopes: vec![Scope::Component, Scope::Package],
        grammar: vec![Grammar {
            scope: Scope::Component,
            patterns: vec!["buzzer {size:code}".into()],
            examples: vec!["buzzer 9.6x9.6".into(), "magnetic buzzer CMI-9653S".into()],
        }],
        instance_fields: vec![InstanceField {
            standard_field: "value".into(),
            when: v1::Params::new(),
        }],
    };
    Description {
        abi: v1::ABI,
        id: "buzzer".into(),
        version: VERSION,
        title: "Magnetic buzzers (SMD)".into(),
        licence: "MIT OR Apache-2.0".into(),
        deterministic: true,
        capabilities: vec![Capability {
            name: v1::GENERATE_PART.into(),
            effect: Effect::None,
            title: Some("Make a surface-mount magnetic buzzer".into()),
            params,
            rules,
            presets,
            part: Some(offer),
        }],
        needs: Vec::new(),
    }
}

struct Dims {
    bl: i64,
    bw: i64,
    h: i64,
    tw: i64,
    rise: i64,
    out: i64,
    land_x: i64,
    land: [i64; 2],
    label: bool,
}

fn part_of(n: i64, d: i64) -> i64 {
    div_round(n, d).unwrap_or(0)
}

impl Dims {
    fn foot(&self) -> i64 {
        FOOT_NM.min(part_of(self.bl, 8))
    }

    fn envelope(&self) -> [i64; 3] {
        [self.bl + 2 * self.out, self.bw, self.h]
    }

    /// The x of terminal 2's contact: the middle of its foot.
    fn contact_x(&self) -> i64 {
        part_of(self.bl + self.out - self.foot(), 2)
    }
}

fn dims(args: &Args<'_>) -> Result<Dims, Vec<Problem>> {
    let one = |p: Problem| vec![p];
    let int = |k: &str| args.int(k).map_err(one);
    let d = Dims {
        bl: int("body_length")?,
        bw: int("body_width")?,
        h: int("height")?,
        tw: int("terminal_width")?,
        rise: int("terminal_rise")?,
        out: int("terminal_out")?,
        land_x: int("land_x")?,
        land: [int("land_length")?, int("land_width")?],
        label: args.flag("label").map_err(one)?,
    };
    let mut problems = Vec::new();
    let mut rule = |ok: bool, field: &str, message: &str| {
        if !ok {
            problems.push(problem("param.inconsistent", field, message.into()));
        }
    };
    rule(
        d.tw < d.bw,
        "terminal_width",
        "A terminal must be narrower than the case.",
    );
    rule(
        d.rise < d.h,
        "terminal_rise",
        "A terminal must end below the case's top.",
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
        })
        .collect()
}

/// A buzzer (IEC 60617 as published): a dome on its base line, the two
/// terminals below it at the base line's ends, on the 2.5 mm symbol grid,
/// a + beside terminal 1.
fn symbol() -> Symbol {
    let mut d = Drawing::default();
    let m = |v: i64| v * MM / 1000;
    d.pin("1", [m(-2500), m(-5000)], UP, m(2500));
    d.pin("2", [m(2500), m(-5000)], UP, m(2500));
    d.line([m(-2500), m(-2500)], [m(2500), m(-2500)]);
    if let Some(dome) = draw::arc([0, m(-2500)], m(2500), 0, 180 * DEG) {
        d.open(dome);
    }
    d.line([m(-1875), m(-3750)], [m(-1125), m(-3750)]);
    d.line([m(-1500), m(-4125)], [m(-1500), m(-3375)]);
    d.labels(0, m(-5000));
    Symbol {
        units: vec![d.unit("A")],
    }
}

mod parts {
    include!(concat!(env!("OUT_DIR"), "/parts.rs"));
}

/// The 3D model: the bundle's part file, with the label or without, at
/// the drawing's values and the estimates.
fn model(d: &Dims) -> v1::Model {
    let mut values = v1::Params::new();
    let mut set = |name: &str, nm: i64| {
        values.insert(name.into(), ParamValue::Int(nm));
    };
    let small = d.bl.min(d.bw);
    set("body_length", d.bl);
    set("body_width", d.bw);
    set("height", d.h);
    set("corner_bevel", BEVEL_NM.min(part_of(small, 8)));
    set("edge_round", TOP_ROUND_NM.min(part_of(d.h, 10)));
    set("port_diameter", PORT_NM.min(part_of(small, 6)));
    set("port_depth", PORT_DEPTH_NM.min(part_of(d.h, 4)));
    set("terminal_width", d.tw);
    set("terminal_rise", d.rise);
    set("terminal_out", d.out);
    set("terminal_depth", d.foot());
    set("lead_thickness", FOOT_THICKNESS_NM.min(part_of(d.h, 20)));
    set("label_diameter", part_of(4 * small, 5));
    set("label_thickness", LABEL_THICKNESS_NM);
    v1::Model::PartFile {
        asset: if d.label {
            parts::BUZZER_SMD_LABEL
        } else {
            parts::BUZZER_SMD
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
    let body = Box2::centred([0, 0], [d.bl, d.bw]);
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
        pads,
        courtyard,
        silk,
        assembly: land::assembly(body),
        keepouts: Vec::new(),
    };
    let component_part = request.scope == Scope::Component;
    let name = format!(
        "Magnetic buzzer {} × {} mm",
        mm_text(d.bl).trim_end_matches(" mm"),
        mm_text(d.bw).trim_end_matches(" mm"),
    );
    Ok(Generation {
        marking: None,
        component: component_part.then(|| Component {
            terminals: [("1", "+"), ("2", "−")]
                .iter()
                .map(|(key, name)| Terminal {
                    key: (*key).into(),
                    name: (*name).into(),
                    direction: Direction::Passive,
                })
                .collect(),
            prefix: "BZ".into(),
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
            "Lands are the manufacturer's as given; the bevel, port, top round and feet are \
             estimated."
                .into(),
        ],
    })
}

impl Generator for Buzzer {
    fn describe() -> Description {
        description()
    }

    fn generate(request: &Request) -> Result<Generation, Vec<Problem>> {
        generate(request)
    }
}

#[cfg(test)]
mod tests;
