//! The gull-wing generator: moulded packages with formed leads on two
//! sides (SOIC, SOP, SSOP, TSSOP, MSOP, QSOP) or four (QFP), as a symbol,
//! package, footprint and 3D model.
//!
//! Leads are numbered counter-clockwise seen from above, from the −x end
//! of the −y row: the −y row in +x, then (four sides) the +x row in +y, the
//! +y row in −x and the −x row in −y. Each land is centred on its lead's
//! foot: as wide as the lead plus the project's gull-wing land extra (a
//! context parameter, `project.land.gull_wing_width_extra`) and 0.6 mm
//! longer than the foot (the host's estimate, an original rule of thumb,
//! not a land-pattern standard).
//!
//! The presets are the gull-wing starter table in nm, each citing the
//! JEDEC outline it follows (MS-012, MS-013, MO-150, MO-137, MO-153,
//! MO-187, MS-026), by document only.
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
    Footprint, Generation, Graphic, Lead, Opening, Package, PackageGeometry, Pad, PadShape, Param,
    ParamType, ParamValue, PartOffer, Preset, Primitive, Problem, Request, Rule, Scope, Side,
    Symbol, SymbolPin, Terminal,
};
use sparksmith_generator_sdk::{Generator, export_generator};

/// The starter rows, in nm.
const PRESETS: &str = include_str!("../presets.json");

/// The generator's version (`Description::version`).
const VERSION: u32 = 1;

/// Most leads on one side: the part files' lead looks are named up to
/// this count, and four sides of it are the core's 512 leads of a form.
const MAX_LEADS_PER_SIDE: i64 = 128;

/// Longest length a form stores: 1000 mm.
const MAX_LENGTH_NM: i64 = 1_000 * MM;
/// Smallest positive length a form stores: 0.01 mm.
const MIN_LENGTH_NM: i64 = 10 * UM;

/// A land is the foot plus this long (the host's estimate).
const LAND_LENGTH_EXTRA_NM: i64 = 600_000;

/// The pin-1 dot: a filled silkscreen circle of this radius beyond pad 1's
/// −x edge, clear of its mask opening by the silkscreen clearance (as the
/// host's template marks pin 1 of a package of more than two terminals).
const PIN_ONE_DOT_RADIUS_NM: i64 = 200_000;

pub struct GullWing;

export_generator!(GullWing);

fn length(key: &str, label: &str, min_nm: i64, default: i64, group: &str) -> Param {
    Param {
        key: key.into(),
        label: label.into(),
        help: None,
        ty: ParamType::Length {
            min_nm,
            max_nm: MAX_LENGTH_NM,
            step_nm: None,
        },
        default: ParamValue::Int(default),
        group: Some(group.into()),
        context: None,
    }
}

fn count(key: &str, label: &str, min: i64, default: i64) -> Param {
    Param {
        key: key.into(),
        label: label.into(),
        help: None,
        ty: ParamType::Count {
            min,
            max: MAX_LEADS_PER_SIDE,
        },
        default: ParamValue::Int(default),
        group: Some("leads".into()),
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
    let sides = Param {
        key: "sides".into(),
        label: "Lead rows".into(),
        help: None,
        ty: ParamType::Enum {
            choices: vec![
                Choice {
                    key: "two".into(),
                    label: "Two sides (SOIC, SSOP, TSSOP)".into(),
                },
                Choice {
                    key: "four".into(),
                    label: "Four sides (QFP)".into(),
                },
            ],
        },
        default: ParamValue::Text("two".into()),
        group: Some("leads".into()),
        context: None,
    };
    let mut extra = length(
        "land_width_extra",
        "Land width over the lead",
        0,
        200_000,
        "lands",
    );
    extra.help =
        Some("How much wider than its lead a land is; the project's land rules fill it.".into());
    extra.context = Some("project.land.gull_wing_width_extra".into());
    let mut short = count("leads_short", "Leads per short side", 0, 0);
    short.help = Some("Four sides only; 0 for two.".into());
    let mut span_x = length("span_x", "Lead span across x", 0, 0, "leads");
    span_x.help = Some("Four sides only: tip to tip across the short sides; 0 for two.".into());
    let params = vec![
        sides,
        count("leads", "Leads per long side", 1, 4),
        short,
        length("pitch", "Pitch", MIN_LENGTH_NM, 1_270_000, "leads"),
        length(
            "body_length",
            "Body length (x)",
            MIN_LENGTH_NM,
            4_900_000,
            "body",
        ),
        length(
            "body_width",
            "Body width (y)",
            MIN_LENGTH_NM,
            3_900_000,
            "body",
        ),
        length(
            "body_height",
            "Body height",
            MIN_LENGTH_NM,
            1_450_000,
            "body",
        ),
        length("standoff", "Standoff", 0, 175_000, "body"),
        length(
            "span",
            "Lead span across y",
            MIN_LENGTH_NM,
            6_000_000,
            "leads",
        ),
        span_x,
        length("lead_width", "Lead width", MIN_LENGTH_NM, 410_000, "leads"),
        length(
            "lead_thickness",
            "Lead thickness",
            MIN_LENGTH_NM,
            200_000,
            "leads",
        ),
        length(
            "foot_length",
            "Foot length",
            MIN_LENGTH_NM,
            835_000,
            "leads",
        ),
        extra,
    ];
    let rule = |text: &str, fields: &[&str]| Rule {
        text: text.into(),
        fields: fields.iter().map(|f| (*f).into()).collect(),
    };
    let rules = vec![
        rule(
            "Lead width < pitch, so the leads do not touch",
            &["lead_width", "pitch"],
        ),
        rule(
            "(leads − 1) × pitch + lead width ≤ the body edge they run along",
            &["leads", "leads_short", "pitch", "lead_width"],
        ),
        rule(
            "Foot length ≤ (span − body) / 2, and more than a lead thickness of room to bend",
            &["foot_length", "span", "span_x", "lead_thickness"],
        ),
        rule(
            "Lead thickness < body height, leaving the body more than 1.5 × its thickness above the seating plane",
            &["lead_thickness", "body_height", "standoff"],
        ),
    ];
    let offer = PartOffer {
        families: vec!["gull_wing".into()],
        scopes: vec![Scope::Component, Scope::Package],
        grammar: Vec::new(),
        instance_fields: Vec::new(),
    };
    Description {
        abi: v1::ABI,
        id: "gull-wing".into(),
        version: VERSION,
        title: "Gull-wing packages".into(),
        licence: "MIT OR Apache-2.0".into(),
        deterministic: true,
        capabilities: vec![Capability {
            name: v1::GENERATE_PART.into(),
            effect: Effect::None,
            title: Some("Make a gull-wing package".into()),
            params,
            rules,
            presets,
            part: Some(offer),
        }],
        needs: Vec::new(),
    }
}

/// A package's values, checked.
struct Dims {
    four: bool,
    /// Leads per long (±y) side and per short (±x) side.
    n: i64,
    m: i64,
    pitch: i64,
    body_length: i64,
    body_width: i64,
    body_height: i64,
    standoff: i64,
    span: i64,
    span_x: i64,
    lead_width: i64,
    lead_thickness: i64,
    foot_length: i64,
    land_extra: i64,
}

type Point2 = [i64; 2];

/// `(i − (n − 1) / 2) × pitch`, to the nearest nanometre.
fn centred(i: i64, n: i64, pitch: i64) -> i64 {
    div_round((2 * i - (n - 1)) * pitch, 2).unwrap_or(0)
}

impl Dims {
    fn leads(&self) -> i64 {
        2 * (self.n + if self.four { self.m } else { 0 })
    }

    /// The envelope's outline: the body, and across the lead rows the
    /// span.
    fn outline(&self) -> Box2 {
        let length = if self.four {
            self.span_x.max(self.body_length)
        } else {
            self.body_length
        };
        Box2::centred([0, 0], [length, self.span.max(self.body_width)])
    }

    /// Where each lead's foot touches the seating plane, in lead order,
    /// with whether it lies on a short (±x) side.
    fn feet(&self) -> Vec<(Point2, bool)> {
        let (n, m, p) = (self.n, self.m, self.pitch);
        let reach_y = div_round(self.span - self.foot_length, 2).unwrap_or(0);
        let reach_x = div_round(self.span_x - self.foot_length, 2).unwrap_or(0);
        let mut out: Vec<(Point2, bool)> = (0..n)
            .map(|i| ([centred(i, n, p), -reach_y], false))
            .collect();
        if self.four {
            out.extend((0..m).map(|i| ([reach_x, centred(i, m, p)], true)));
        }
        out.extend((0..n).rev().map(|i| ([centred(i, n, p), reach_y], false)));
        if self.four {
            out.extend((0..m).rev().map(|i| ([-reach_x, centred(i, m, p)], true)));
        }
        out
    }
}

fn dims(args: &Args<'_>) -> Result<Dims, Vec<Problem>> {
    let one = |p: Problem| vec![p];
    let d = Dims {
        four: args.text("sides").map_err(one)? == "four",
        n: args.int("leads").map_err(one)?,
        m: args.int("leads_short").map_err(one)?,
        pitch: args.int("pitch").map_err(one)?,
        body_length: args.int("body_length").map_err(one)?,
        body_width: args.int("body_width").map_err(one)?,
        body_height: args.int("body_height").map_err(one)?,
        standoff: args.int("standoff").map_err(one)?,
        span: args.int("span").map_err(one)?,
        span_x: args.int("span_x").map_err(one)?,
        lead_width: args.int("lead_width").map_err(one)?,
        lead_thickness: args.int("lead_thickness").map_err(one)?,
        foot_length: args.int("foot_length").map_err(one)?,
        land_extra: args.int("land_width_extra").map_err(one)?,
    };
    let mut problems = Vec::new();
    let mut rule = |ok: bool, field: &str, message: String| {
        if !ok {
            problems.push(problem("param.inconsistent", field, message));
        }
    };
    let t = d.lead_thickness;
    // A row of `k` leads at the pitch must fit along `edge`.
    let fits = |k: i64, edge: i64| (k - 1) * d.pitch + d.lead_width <= edge;
    rule(
        d.pitch > d.lead_width,
        "lead_width",
        format!(
            "Leads {} mm wide touch at a pitch of {} mm.",
            mm(d.lead_width),
            mm(d.pitch)
        ),
    );
    rule(
        fits(d.n, d.body_length),
        "leads",
        format!(
            "{} leads at {} mm do not fit along the {} mm body length.",
            d.n,
            mm(d.pitch),
            mm(d.body_length)
        ),
    );
    // Leads leave the body at mid-height, more than a thickness above the
    // seating plane: standoff + height / 2 − t / 2 > t.
    rule(
        t < d.body_height && 2 * d.standoff + d.body_height > 3 * t,
        "lead_thickness",
        format!(
            "Leads {} mm thick do not fit a body {} mm high on a {} mm standoff.",
            mm(t),
            mm(d.body_height),
            mm(d.standoff)
        ),
    );
    // Across each pair of rows: the span reaches past the body, the foot
    // lies outside it and the lead has more than a thickness to bend.
    let mut across = vec![("span", d.span, d.body_width, "width")];
    if d.four {
        across.push(("span_x", d.span_x, d.body_length, "length"));
    }
    for (field, span, body, what) in across {
        rule(
            span > body
                && 2 * d.foot_length <= span - body
                && span - body - 2 * d.foot_length > 2 * t,
            field,
            format!(
                "A span of {} mm leaves no room outside the body {what} of {} mm for {} mm feet and the bend of {} mm leads.",
                mm(span),
                mm(body),
                mm(d.foot_length),
                mm(t)
            ),
        );
    }
    if d.four {
        rule(
            d.m >= 1,
            "leads_short",
            "Each short side of a four-sided package needs a lead.".into(),
        );
        rule(
            fits(d.m, d.body_width),
            "leads_short",
            format!(
                "{} leads at {} mm do not fit along the {} mm body width.",
                d.m,
                mm(d.pitch),
                mm(d.body_width)
            ),
        );
    } else {
        rule(
            d.m == 0 && d.span_x == 0,
            "leads_short",
            "A two-sided package has no short-side leads or span (0).".into(),
        );
    }
    if problems.is_empty() {
        Ok(d)
    } else {
        Err(problems)
    }
}

/// A length in mm as people write it: at least one decimal, at most as
/// many as it needs (1.27, 0.5, 0.635).
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

fn pads(d: &Dims) -> Vec<Pad> {
    let land = [
        d.lead_width + d.land_extra,
        d.foot_length + LAND_LENGTH_EXTRA_NM,
    ];
    d.feet()
        .into_iter()
        .enumerate()
        .map(|(k, (at, short_side))| {
            let key = (k + 1).to_string();
            Pad {
                terminal: key.clone(),
                key,
                shape: PadShape::Rect {
                    size: if short_side { [land[1], land[0]] } else { land },
                },
                at,
                rot_mdeg: 0,
                side: Side::Top,
                hole: None,
                mask: Opening::Default,
                paste: Opening::Default,
                number: None,
            }
        })
        .collect()
}

/// A rectangle with the first half of the pins down its left side and the
/// rest up its right side, on the symbol grid, the designator above and
/// the name below (the host's IC box). Pin names of at most three
/// characters (512 leads) always fit its four-module width.
fn symbol(pins: i64) -> Symbol {
    let module = draw::MODULE_NM;
    let left = (pins + 1) / 2;
    let rows = left.max(1);
    let top = (rows - 1) / 2 * module;
    let mut d = Drawing::default();
    d.shape(
        Primitive::Rect {
            min: [-2 * module, top - rows * module],
            max: [2 * module, top + module],
        },
        draw::STROKE_NM,
        Fill::Body,
    );
    d.labels(top + module, top - rows * module);
    for k in 0..pins {
        let (at, rot_mdeg) = if k < left {
            ([-3 * module, top - k * module], RIGHT)
        } else {
            ([3 * module, top - (pins - 1 - k) * module], LEFT)
        };
        d.pins.push(SymbolPin {
            terminal: (k + 1).to_string(),
            at,
            length_nm: module,
            rot_mdeg,
            show_name: true,
            show_number: true,
        });
    }
    Symbol {
        units: vec![d.unit("A")],
    }
}

/// The SHA-256 of each part file in the bundle (`parts/`), written by the
/// build script.
mod parts {
    include!(concat!(env!("OUT_DIR"), "/parts.rs"));
}

/// The package's 3D model: the bundle's two-sided or four-sided part file
/// (a moulded body on its standoff, cut round the roots of formed leads
/// that leave it at mid-height, bend down and end in a flat foot on the
/// seating plane; rounded corners and top edges and a pin-1 dimple, each
/// derived in the file from these values), every dimension given.
fn model(d: &Dims) -> v1::Model {
    let mut values = v1::Params::new();
    let mut set = |name: &str, v: i64| {
        values.insert(name.into(), ParamValue::Int(v));
    };
    set("leads", d.n);
    set("pitch", d.pitch);
    set("body_length", d.body_length);
    set("body_width", d.body_width);
    set("body_height", d.body_height);
    set("standoff", d.standoff);
    set("span", d.span);
    set("lead_width", d.lead_width);
    set("lead_thickness", d.lead_thickness);
    set("foot_length", d.foot_length);
    if d.four {
        set("leads_short", d.m);
        set("span_x", d.span_x);
    }
    v1::Model::PartFile {
        asset: if d.four {
            parts::GULL_WING_FOUR_SIDED
        } else {
            parts::GULL_WING_TWO_SIDED
        }
        .into(),
        values,
    }
}

/// The part's name, e.g. "SO-8 1.27 mm, 4.9 × 3.9 mm body" or "QFP-64
/// 0.5 mm, 10.0 × 10.0 mm body".
fn name(d: &Dims) -> String {
    format!(
        "{}-{} {} mm, {} × {} mm body",
        if d.four { "QFP" } else { "SO" },
        d.leads(),
        mm(d.pitch),
        mm(d.body_length),
        mm(d.body_width)
    )
}

fn generate(request: &Request) -> Result<Generation, Vec<Problem>> {
    let description = description_with(Vec::new());
    let args = checked(&description, &request.params)?;
    let d = dims(&args)?;
    let pads = pads(&d);
    let outline = d.outline();
    let courtyard = land::courtyard(&pads, Some(outline)).unwrap_or_default();
    let top = courtyard
        .iter()
        .map(|p| p[1])
        .max()
        .unwrap_or(outline.max[1]);
    let mut silk = land::silk_outline(&pads, outline, land::DEFAULT_MASK_NM);
    silk.push(land::silk_designator(outline, top));
    if d.leads() > 2
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
    let keys: Vec<String> = (1..=d.leads()).map(|k| k.to_string()).collect();
    let size = outline.size();
    let package = Package {
        terminals: keys.clone(),
        geometry: PackageGeometry {
            envelope_nm: [size[0], size[1], d.standoff + d.body_height],
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
        assembly: land::assembly(outline),
        keepouts: Vec::new(),
    };
    let component_part = request.scope == Scope::Component;
    Ok(Generation {
        component: component_part.then(|| Component {
            terminals: keys
                .iter()
                .map(|key| Terminal {
                    key: key.clone(),
                    name: key.clone(),
                    direction: Direction::Passive,
                })
                .collect(),
            prefix: "U".into(),
            name: name(&d),
            fields: Vec::new(),
        }),
        symbol: component_part.then(|| symbol(d.leads())),
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
            "Lands are estimated from the leads (the foot + 0.6 mm long, the lead plus the land \
             rule's extra wide); confirm them against the package's recommended land pattern."
                .into(),
        ],
    })
}

impl Generator for GullWing {
    fn describe() -> Description {
        description()
    }

    fn generate(request: &Request) -> Result<Generation, Vec<Problem>> {
        generate(request)
    }
}

#[cfg(test)]
mod tests;
