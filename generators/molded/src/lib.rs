//! The molded generator: molded two-terminal packages (SOD-123/323/523,
//! SMA/SMB/SMC, moulded tantalum A–E, MELF and MiniMELF) as a symbol,
//! package, footprint and 3D model, for a diode or a polarised capacitor.
//!
//! Terminal 1 is at −x: a diode's cathode (the band end), a capacitor's
//! positive terminal. Lands follow the host's estimate for molded
//! terminals (an original rule of thumb, not a land-pattern standard): a
//! land of terminal length + 0.6 mm by terminal width + 0.2 mm, its corners
//! rounded by a quarter of its short side, centred 0.1 mm outward of the
//! terminal's contact (toe 0.4 mm, heel 0.2 mm). Pin 1 carries a silkscreen
//! dot beyond its land.
//!
//! The presets are the molded starter table, in nm, with its citations.
#![cfg_attr(target_arch = "wasm32", no_std)]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use sparksmith_generator_sdk::args::{Args, checked, problem};
use sparksmith_generator_sdk::draw::{self, Drawing, LEFT, RIGHT};
use sparksmith_generator_sdk::land::{self, Box2};
use sparksmith_generator_sdk::units::{MM, UM, div_round, mm_text};
use sparksmith_generator_sdk::v1::{
    self, BindingEntry, Capability, Choice, Component, Description, Direction, Effect, Fill,
    Footprint, Generation, Grammar, Graphic, InstanceField, Lead as LeadContact, Opening, Package,
    PackageGeometry, Pad, PadShape, Param, ParamType, ParamValue, PartOffer, Preset, Primitive,
    Problem, Request, Rule, Scope, Side, Symbol, Terminal,
};
use sparksmith_generator_sdk::{Generator, export_generator};

/// The starter rows, in nm, with their citations.
const PRESETS: &str = include_str!("../presets.json");

/// The generator's version (`Description::version`): 4 sets the marking
/// text at the height its preset gives (`marking_height`); 5 marks a
/// polarised capacitor's value by its value code ([`value_code_line`]).
/// 6: the SDK's pin table (`pin_table`) and part numbers
/// (`part_numbers`).
const VERSION: u32 = 6;

/// A moulded polarised (tantalum) capacitor's value code as marking data:
/// the capacitance in picofarads as the three-digit code and the rated
/// voltage's letter, one line (`106 A`, 10 µF 10 V), as Kyocera AVX's TAJ
/// series marks every case from A to E (TAJ data sheet TDS-PTNO-0024 Rev 3,
/// Marking: polarity band at the anode, the pF code and the voltage code;
/// the maker's logo and its ID code line are the maker's own and are left
/// out). The letters are the named `tantalum` table of the host's
/// standards. The host fills both from each placed part's Capacitance and
/// Rated voltage; a part with a value no code holds, or a voltage with no
/// letter, draws no line (a wrong code is never drawn).
fn value_code_line(code: &str) -> Option<String> {
    match code {
        "pf_letter" => Some(String::from(
            "{field:Capacitance|cap3} {field:Rated voltage|letter:tantalum}",
        )),
        _ => None,
    }
}

/// Estimated land: terminal length plus this along x…
const LAND_EXTRA_LENGTH_NM: i64 = 600_000;
/// …terminal width plus this across…
const LAND_EXTRA_WIDTH_NM: i64 = 200_000;
/// …centred this far outward of the terminal's contact (toe 0.4 mm, heel
/// 0.2 mm).
const LAND_SHIFT_NM: i64 = 100_000;

/// The pin-1 dot: a filled silkscreen circle of this radius beyond pad 1's
/// −x edge, clear of its mask opening by the silkscreen clearance (as the
/// host's template marks a polarised two-terminal part).
const PIN_ONE_DOT_RADIUS_NM: i64 = 200_000;

/// Leadframe thickness of flat and bent leads, and of J-leads (est.:
/// small-signal SOD leadframes are 0.1–0.15 mm, power J-lead and tantalum
/// frames 0.15–0.3 mm), each at most a quarter of the body height.
const FLAT_LEAD_NM: i64 = 120_000;
const J_LEAD_NM: i64 = 150_000;
/// The gap under a bent-lead body (est.), at most a tenth of its height.
const BENT_STANDOFF_NM: i64 = 50_000;
/// How far a lead root reaches into the moulding, at most a sixth of the
/// body length.
const ROOT_NM: i64 = 250_000;
/// A bent lead's shoulder lies this share (thousandths) of the way from
/// the body end to the foot.
const SHOULDER_PERMILLE: i64 = 300;
/// Rounds of the moulding's upright corners and top edges (est.: moulded
/// diode and tantalum bodies show 0.1–0.3 mm radii), capped by the body.
const CORNER_NM: i64 = 200_000;
const TOP_EDGE_NM: i64 = 150_000;
/// The least corner round: where the terminals are as wide as the body,
/// the corners keep a hairline round so the one part file serves (the
/// builder left them sharp).
const MIN_CORNER_NM: i64 = UM;
/// Round of a MELF cap's outer edge (est.).
const CAP_EDGE_NM: i64 = 80_000;

pub struct Molded;

export_generator!(Molded);

/// How the two terminals are formed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Form {
    /// Flat leads under the body ends (SOD-123F, SOD-523).
    Flat,
    /// Leads leaving the body ends at mid-height and bending down to a
    /// flat foot (SOD-123, SOD-323).
    Bent,
    /// J-leads folded down the end faces and under the body (SMA/SMB/SMC,
    /// tantalum).
    JLead,
    /// Cylindrical end caps (MELF, MiniMELF).
    Melf,
}

impl Form {
    const ALL: [(Self, &'static str, &'static str); 4] = [
        (Self::Flat, "flat", "Flat leads"),
        (Self::Bent, "bent", "Bent leads"),
        (Self::JLead, "j_lead", "J-leads"),
        (Self::Melf, "melf", "MELF end caps"),
    ];
}

/// What the part is: the symbol, terminal names and prefix.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Part {
    Diode,
    PolarisedCapacitor,
}

impl Part {
    const ALL: [(Self, &'static str, &'static str); 2] = [
        (Self::Diode, "diode", "Diode"),
        (
            Self::PolarisedCapacitor,
            "polarised_capacitor",
            "Polarised capacitor",
        ),
    ];

    fn noun(self) -> &'static str {
        match self {
            Self::Diode => "diode",
            Self::PolarisedCapacitor => "polarised capacitor",
        }
    }

    fn prefix(self) -> &'static str {
        match self {
            Self::Diode => "D",
            Self::PolarisedCapacitor => "C",
        }
    }
}

fn find<T: Copy>(all: &[(T, &'static str, &'static str)], key: &str) -> Option<T> {
    all.iter().find(|(_, k, _)| *k == key).map(|(v, _, _)| *v)
}

fn length(key: &str, label: &str, min_nm: i64, max_nm: i64, default: i64) -> Param {
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
        group: Some("body".into()),
        context: None,
    }
}

fn enumeration(
    key: &str,
    label: &str,
    options: &[(&str, &str)],
    default: &str,
    group: &str,
) -> Param {
    Param {
        key: key.into(),
        label: label.into(),
        help: None,
        ty: ParamType::Enum {
            choices: options
                .iter()
                .map(|(key, label)| Choice {
                    key: (*key).into(),
                    label: (*label).into(),
                })
                .collect(),
        },
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

/// The description, with `presets` (generate checks parameters against it
/// without reading the preset table).
fn description_with(presets: Vec<Preset>) -> Description {
    let parts: Vec<(&str, &str)> = Part::ALL.iter().map(|(_, k, l)| (*k, *l)).collect();
    let leads: Vec<(&str, &str)> = Form::ALL.iter().map(|(_, k, l)| (*k, *l)).collect();
    // The defaults are the SOD-123 starter row.
    let mut params = vec![
        enumeration("part", "Part", &parts, "diode", "part"),
        enumeration("lead", "Terminals", &leads, "flat", "body"),
        length("body_length", "Body length", 200 * UM, 20 * MM, 2_700_000),
        length(
            "body_width",
            "Body width (MELF: diameter)",
            100 * UM,
            15 * MM,
            1_600_000,
        ),
        length(
            "body_height",
            "Body height (MELF: diameter)",
            100 * UM,
            10 * MM,
            1_100_000,
        ),
        length("span", "Overall length", 200 * UM, 25 * MM, 3_700_000),
        length(
            "terminal_length",
            "Terminal length (MELF: cap length)",
            50 * UM,
            5 * MM,
            500_000,
        ),
        length(
            "terminal_width",
            "Terminal width (MELF: cap diameter)",
            50 * UM,
            15 * MM,
            600_000,
        ),
        Param {
            key: "polarised".into(),
            label: "Polarity band".into(),
            help: Some("A band on the body at terminal 1.".into()),
            ty: ParamType::Bool,
            default: ParamValue::Bool(true),
            group: Some("body".into()),
            context: None,
        },
    ];
    params.push(sparksmith_generator_sdk::marking::param());
    params.push(sparksmith_generator_sdk::marking::height_param());
    let mut code = enumeration(
        "value_code",
        "Capacitor value code",
        &[
            ("pf_letter", "pF code and voltage letter"),
            ("none", "None"),
        ],
        "pf_letter",
        "marking",
    );
    code.help = Some(
        "The value code a polarised capacitor carries: the capacitance in pF as three digits and the rated voltage's letter (106 A: 10 µF, 10 V), as moulded tantalum capacitors are marked; none leaves it unmarked. Diodes carry none. Lines in Top marking replace it."
            .into(),
    );
    params.push(code);
    let rule = |text: &str, fields: &[&str]| Rule {
        text: text.into(),
        fields: fields.iter().map(|f| (*f).into()).collect(),
    };
    let rules = vec![
        rule(
            "2 × terminal length < overall length",
            &["terminal_length", "span"],
        ),
        rule(
            "Leads: terminal width ≤ body width; overall length > body length",
            &[
                "lead",
                "terminal_width",
                "body_width",
                "span",
                "body_length",
            ],
        ),
        rule(
            "MELF: body height = body width ≤ cap diameter; overall − 2 × cap length ≤ body length ≤ overall",
            &[
                "lead",
                "body_height",
                "body_width",
                "terminal_width",
                "body_length",
            ],
        ),
    ];
    let offer = PartOffer {
        families: vec!["molded".into()],
        scopes: vec![Scope::Component, Scope::Package],
        grammar: vec![Grammar {
            scope: Scope::Component,
            patterns: vec!["{part} {size:code}".into(), "{value} {size:code}".into()],
            examples: vec![
                "diode SOD-123".into(),
                "diode SMA".into(),
                "tantalum case B".into(),
            ],
        }],
        instance_fields: vec![InstanceField {
            standard_field: "value".into(),
            when: v1::Params::new(),
        }],
    };
    Description {
        abi: v1::ABI,
        id: "molded".into(),
        version: VERSION,
        title: "Molded two-terminal packages".into(),
        licence: "MIT OR Apache-2.0".into(),
        deterministic: true,
        capabilities: vec![Capability {
            name: v1::GENERATE_PART.into(),
            effect: Effect::None,
            title: Some("Make a molded diode or polarised capacitor".into()),
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
    part: Part,
    lead: Form,
    body_length: i64,
    body_width: i64,
    body_height: i64,
    span: i64,
    terminal_length: i64,
    terminal_width: i64,
    polarised: bool,
}

/// `n / d` to the nearest nanometre (the divisors here are small and
/// positive and the values bounded, so it never fails).
fn part_of(n: i64, d: i64) -> i64 {
    div_round(n, d).unwrap_or(0)
}

impl Dims {
    /// The leadframe thickness; 0 for a MELF.
    fn lead_thickness(&self) -> i64 {
        let quarter = part_of(self.body_height, 4);
        match self.lead {
            Form::Flat | Form::Bent => FLAT_LEAD_NM.min(quarter),
            Form::JLead => J_LEAD_NM.min(quarter),
            Form::Melf => 0,
        }
    }

    /// Height of the moulded body's underside (bent leads stand it off).
    fn body_bottom(&self) -> i64 {
        match self.lead {
            Form::Bent => BENT_STANDOFF_NM.min(part_of(self.body_height, 10)),
            _ => 0,
        }
    }

    /// Height of a bent or J-lead's centre line where it leaves the body.
    fn lead_exit(&self) -> i64 {
        let bottom = self.body_bottom();
        bottom + part_of(self.body_height - bottom, 2)
    }

    /// The package's envelope (a MELF is as wide and tall as its widest
    /// round).
    fn envelope(&self) -> [i64; 3] {
        let width = self.body_width.max(self.terminal_width);
        let height = if self.lead == Form::Melf {
            width
        } else {
            self.body_height
        };
        [self.span.max(self.body_length), width, height]
    }

    /// The x of terminal 2's contact (terminal 1's mirrored).
    fn contact_x(&self) -> i64 {
        part_of(self.span - self.terminal_length, 2)
    }
}

fn inconsistent(field: &str, message: &str) -> Problem {
    problem("param.inconsistent", field, message.into())
}

/// The core's form rules for molded packages, in integers.
fn dims(args: &Args<'_>) -> Result<Dims, Vec<Problem>> {
    let one = |p: Problem| vec![p];
    let key = args.text("part").map_err(one)?;
    let part = find(&Part::ALL, key).ok_or_else(|| {
        vec![problem(
            "param.out_of_range",
            "part",
            format!("Part \"{key}\" is not one this generator makes."),
        )]
    })?;
    let key = args.text("lead").map_err(one)?;
    let lead = find(&Form::ALL, key).ok_or_else(|| {
        vec![problem(
            "param.out_of_range",
            "lead",
            format!("Terminals \"{key}\" are not a form this generator makes."),
        )]
    })?;
    let d = Dims {
        part,
        lead,
        body_length: args.int("body_length").map_err(one)?,
        body_width: args.int("body_width").map_err(one)?,
        body_height: args.int("body_height").map_err(one)?,
        span: args.int("span").map_err(one)?,
        terminal_length: args.int("terminal_length").map_err(one)?,
        terminal_width: args.int("terminal_width").map_err(one)?,
        polarised: args.flag("polarised").map_err(one)?,
    };
    let (bl, span, tl) = (d.body_length, d.span, d.terminal_length);
    let t = d.lead_thickness();
    let mut problems = Vec::new();
    let mut rule = |ok: bool, field: &str, message: &str| {
        if !ok {
            problems.push(inconsistent(field, message));
        }
    };
    rule(
        2 * tl < span,
        "terminal_length",
        "The two terminals must not meet (2 × terminal length < overall length).",
    );
    match lead {
        Form::Melf => {
            rule(
                (d.body_height - d.body_width).abs() <= 1,
                "body_height",
                "A MELF body is round: its height must equal its width.",
            );
            rule(
                d.terminal_width >= d.body_width,
                "terminal_width",
                "A MELF cap must be at least as wide as the body (terminal width ≥ body width).",
            );
            rule(
                bl <= span && bl >= span - 2 * tl,
                "body_length",
                "A MELF body must reach both caps and end within the overall length \
                 (overall − 2 × terminal length ≤ body length ≤ overall).",
            );
        }
        Form::Flat | Form::Bent | Form::JLead => {
            rule(
                d.terminal_width <= d.body_width,
                "terminal_width",
                "The terminals must not be wider than the body.",
            );
            rule(
                span > bl,
                "span",
                "The terminals must reach past the body ends (overall length > body length).",
            );
        }
    }
    // Twice the exit height less the half thickness, so halves stay whole.
    let exit2 = 2 * d.lead_exit() - t;
    match lead {
        Form::Melf | Form::Flat => {}
        Form::Bent => {
            rule(
                2 * tl <= span - bl,
                "terminal_length",
                "A bent lead's foot must lie outside the body \
                 (terminal length ≤ (overall − body length) / 2).",
            );
            rule(
                span - bl - 2 * tl > 2 * t,
                "span",
                "A bent lead needs more than its thickness outside the body to bend down \
                 ((overall − body length) / 2 − terminal length > lead thickness).",
            );
            rule(
                tl > t,
                "terminal_length",
                "The foot must be longer than the lead thickness.",
            );
            rule(
                exit2 > 2 * t,
                "body_height",
                "The body must be tall enough for the lead to leave it above the seating plane.",
            );
        }
        Form::JLead => {
            rule(
                span - bl > 3 * t,
                "span",
                "A J-lead needs room beside the body end \
                 ((overall − body length) / 2 > 1.5 × lead thickness).",
            );
            rule(
                tl > 2 * t,
                "terminal_length",
                "A J-lead's foot must be longer than twice the lead thickness.",
            );
            rule(
                exit2 > 3 * t,
                "body_height",
                "The body must be tall enough for the J-lead to fold down its end.",
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
    let x = d.contact_x() + LAND_SHIFT_NM;
    let size = [
        d.terminal_length + LAND_EXTRA_LENGTH_NM,
        d.terminal_width + LAND_EXTRA_WIDTH_NM,
    ];
    let radius = part_of(size[0].min(size[1]), 4);
    [("1", -x), ("2", x)]
        .iter()
        .map(|(key, x)| Pad {
            key: (*key).into(),
            terminal: (*key).into(),
            shape: PadShape::RoundRect {
                size,
                radius_nm: radius,
            },
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

/// The symbols of the host's standard drawings (IEC 60617 and IEEE 315 as
/// published), terminal 1 the cathode or the positive plate.
fn symbol(part: Part) -> Symbol {
    let mut d = Drawing::default();
    let m = |v: i64| v * MM / 1000; // thousandths of a mm
    match part {
        Part::Diode => {
            // Anode (terminal 2) left, cathode (terminal 1, the bar) right.
            d.pin("2", [-draw::TWO_TERMINAL_PIN_X_NM, 0], RIGHT, m(3750));
            d.pin("1", [draw::TWO_TERMINAL_PIN_X_NM, 0], LEFT, m(3750));
            d.shape(
                Primitive::Polygon {
                    points: vec![[m(-1250), m(1250)], [m(-1250), m(-1250)], [m(1250), 0]],
                },
                draw::STROKE_NM,
                Fill::Body,
            );
            d.line([m(1250), m(-1250)], [m(1250), m(1250)]);
            d.labels(m(1600), m(-1600));
        }
        Part::PolarisedCapacitor => {
            // A straight + plate (terminal 1, left) and a curved − plate:
            // an arc about (3, 0) of radius 2.5 mm through (0.5, 0), its
            // ends 1.75 mm either side; a + beside the + plate.
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
        }
    }
    Symbol {
        units: vec![d.unit("A")],
    }
}

/// `√v` to the nearest integer.
fn root_round(v: i64) -> i64 {
    let root = v.max(0).isqrt();
    // (root + ½)² = root² + root + ¼: above it the next integer is nearer.
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

/// A bend's centre-line radius: the lead thickness, less where a segment
/// beside it is too short (each gives at most 0.45 of its length to one
/// bend: `0.45 × shorter / tan(turn / 2)`, with `tan(turn / 2)` as
/// `tan_num / tan_den`), but always just over half the thickness so the
/// inside keeps a radius.
fn bend_radius(t: i64, shorter: i64, tan_num: i64, tan_den: i64) -> i64 {
    let fit = div_round(45 * shorter * tan_den, 100 * tan_num).unwrap_or(t);
    t.min(fit).max(part_of(t * 10_001, 20_000))
}

/// The molded package's 3D model: the bundle's part file for its terminal
/// form, with the band when polarised, at every value the file takes but
/// the band's and the marking area's, which follow the body.
///
/// Moulded bodies: the leadframe thickness, how far a lead root reaches
/// into the body, the rounds (corners clear of the terminals, top edges
/// clear of the leads), and per form the flat lead's length, or the bent
/// strip's exit height, shoulder and bend radii. MELF: the cap edge round.
fn model(d: &Dims) -> v1::Model {
    let mut values = v1::Params::new();
    let mut set = |name: &str, nm: i64| {
        values.insert(name.into(), ParamValue::Int(nm));
    };
    let (bl, bw, bh) = (d.body_length, d.body_width, d.body_height);
    let (span, tl, tw) = (d.span, d.terminal_length, d.terminal_width);
    set("body_length", bl);
    set("span", span);
    set("terminal_length", tl);
    let asset = if d.lead == Form::Melf {
        set("body_diameter", bw);
        set("cap_diameter", tw);
        let round = CAP_EDGE_NM
            .min(part_of(15 * tl, 100))
            .min(part_of(tw, 20))
            .min(part_of((tw - bw).max(0), 2) + part_of(tl, 2));
        set("cap_round", round);
        if d.polarised {
            parts::MELF_BAND
        } else {
            parts::MELF
        }
    } else {
        set("body_width", bw);
        set("body_height", bh);
        set("terminal_width", tw);
        let t = d.lead_thickness();
        let h = part_of(t, 2);
        set("lead_thickness", t);
        let root = ROOT_NM.min(part_of(bl, 6));
        set("lead_root", root);
        let (hb, hs) = (part_of(bl, 2), part_of(span, 2));
        let exit = d.lead_exit();
        match d.lead {
            Form::Flat => {
                // From the tip to the contact's inner end, or further in
                // to reach under the body by the root.
                let under = part_of(span - bl, 2) + root;
                set("lead_length", if tl >= under { tl } else { under });
            }
            Form::Bent => {
                set("body_bottom", d.body_bottom());
                set("lead_exit", exit);
                let foot = hs - tl;
                let shoulder = hb + part_of(SHOULDER_PERMILLE * (foot - hb), 1000);
                set("lead_shoulder", shoulder);
                // The slope's run and drop; tan(turn / 2) = drop / (slope + run).
                let (run, drop) = (foot - shoulder, exit - h);
                let slope = root_round(run * run + drop * drop);
                let run_out = shoulder - (hb - root);
                set(
                    "bend_1",
                    bend_radius(t, run_out.min(slope), drop, slope + run),
                );
                set("bend_2", bend_radius(t, slope.min(tl), drop, slope + run));
            }
            Form::JLead => {
                set("lead_exit", exit);
                let outer = hs - h;
                let run_out = outer - (hb - root);
                let down = exit - h;
                let under = tl - h;
                set("bend_1", bend_radius(t, run_out.min(down), 1, 1));
                set("bend_2", bend_radius(t, down.min(under), 1, 1));
            }
            Form::Melf => {}
        }
        // Upright corners clear of the terminals, the top edges clear of
        // the leads, each capped by the body and the band's inset.
        let cap = part_of(8 * bl.min(bw), 100).min(part_of(64 * bl, 1000));
        let corner = if bw > tw {
            CORNER_NM.min(cap).min(part_of(bw - tw, 4))
        } else {
            0
        };
        let lead_top = if d.lead == Form::Flat { t } else { exit + h };
        let mut edge = TOP_EDGE_NM
            .min(part_of(12 * bh, 100))
            .min(cap)
            .min(part_of(bh - lead_top, 2));
        let corner = if corner > 0 {
            edge = edge.min(corner);
            corner
        } else {
            edge = edge.min(MIN_CORNER_NM);
            MIN_CORNER_NM
        };
        set("corner_round", corner);
        set("edge_round", edge);
        match (d.lead, d.polarised) {
            (Form::Flat, false) => parts::MOLDED_FLAT,
            (Form::Flat, true) => parts::MOLDED_FLAT_BAND,
            (Form::Bent, false) => parts::MOLDED_BENT,
            (Form::Bent, true) => parts::MOLDED_BENT_BAND,
            (_, false) => parts::MOLDED_J_LEAD,
            (_, true) => parts::MOLDED_J_LEAD_BAND,
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
    let envelope = d.envelope();
    let body = Box2::centred([0, 0], [envelope[0], envelope[1]]);
    let courtyard = land::courtyard(&pads, Some(body)).unwrap_or_default();
    let top = courtyard.iter().map(|p| p[1]).max().unwrap_or(body.max[1]);
    let mut silk = land::silk_outline(&pads, body, land::DEFAULT_MASK_NM);
    silk.push(land::silk_designator(body, top));
    if let Some(first) = pads.first() {
        // Pin 1 (the cathode or the + terminal) is always marked.
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
                LeadContact {
                    key: "1".into(),
                    contact: [-x, 0, 0],
                },
                LeadContact {
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
    let names = match d.part {
        Part::Diode => ["K", "A"],
        Part::PolarisedCapacitor => ["1", "2"],
    };
    let class = if d.lead == Form::Melf {
        "MELF"
    } else {
        "Molded"
    };
    let name = format!(
        "{class} {} {} × {} mm",
        d.part.noun(),
        mm_text(d.body_length).trim_end_matches(" mm"),
        mm_text(d.body_width).trim_end_matches(" mm"),
    );
    let mut marking = sparksmith_generator_sdk::marking::of(&args, false).map_err(|p| vec![p])?;
    if marking.as_ref().is_none_or(|m| m.lines.is_empty()) && d.part == Part::PolarisedCapacitor {
        let code = args.text("value_code").map_err(|p| vec![p])?;
        if let Some(line) = value_code_line(code) {
            let text_height =
                sparksmith_generator_sdk::marking::height(&args).map_err(|p| vec![p])?;
            let m = marking.get_or_insert_with(v1::Marking::default);
            m.lines = vec![line];
            m.text_height = text_height;
        }
    }
    Ok(Generation {
        orderable: Vec::new(),
        marking,
        component: component_part.then(|| Component {
            terminals: ["1", "2"]
                .iter()
                .zip(names)
                .map(|(key, name)| Terminal {
                    key: (*key).into(),
                    name: name.into(),
                    direction: Direction::Passive,
                    alternates: Vec::new(),
                })
                .collect(),
            prefix: d.part.prefix().into(),
            name,
            fields: vec![String::from("value")],
        }),
        symbol: component_part.then(|| symbol(d.part)),
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
            "Lands are estimated from the terminals (terminal length + 0.6 mm by width + 0.2 mm); \
             confirm them against the part's recommended land pattern."
                .into(),
        ],
    })
}

impl Generator for Molded {
    fn describe() -> Description {
        description()
    }

    fn generate(request: &Request) -> Result<Generation, Vec<Problem>> {
        generate(request)
    }
}

#[cfg(test)]
mod tests;
