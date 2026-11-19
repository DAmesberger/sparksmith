//! The tabbed generator: small-outline transistor packages and power
//! packages with unequal sides (SOT-23, SC-70, SOT-223, SOT-89, DPAK,
//! D²PAK), as a symbol, package, footprint and 3D model.
//!
//! Gull-wing leads leave a moulded body on its −y and +y sides, each row
//! with its own count and spacing (`step` grid positions of `pitch` between
//! neighbouring leads, all on one grid centred on the body), and the +y
//! side may instead hold a tab: a wide gull-wing tab lead (SOT-223, SOT-89)
//! or a flat flange on the seating plane under the body (DPAK, D²PAK).
//! Leads are numbered counter-clockwise from above: the −y row from −x to
//! +x, the +y row from +x to −x, then the tab.
//!
//! Lands follow the host's estimates for these packages (an original rule
//! of thumb, not a land-pattern standard): a lead's land is 0.2 mm wider
//! than the lead and 0.6 mm longer than its foot, its centre 0.1 mm outside
//! the foot's; a tab lead's land likewise; a flange's land is 0.2 mm wider
//! and 0.4 mm longer than its solderable area and centred on it. With
//! `lands` set to `explicit`, the datasheet's recommended land pattern
//! replaces the estimates: every lead's land is `land_width` across the
//! lead by `land_length`, the two rows `land_span` apart centre to centre,
//! and the tab's land `tab_land_width` by `tab_land_length` where the
//! estimate puts it.
//!
//! The presets are the tabbed starter table, in nm, with its citations
//! (JEDEC TO-236, MO-178, TO-261, MO-203) and notes.
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
    Footprint, Generation, Grammar, Graphic, Lead, Opening, Package, PackageGeometry, Pad,
    PadShape, Param, ParamType, ParamValue, PartOffer, Preset, Primitive, Problem, Request, Rule,
    Scope, Side, Symbol, Terminal,
};
use sparksmith_generator_sdk::{Generator, export_generator};

/// The starter rows, in nm.
const PRESETS: &str = include_str!("../presets.json");

/// The generator's version (`Description::version`): 5 sets the marking
/// text at the height its preset gives (`marking_height`).
const VERSION: u32 = 5;

/// Most leads on one side, and most grid positions between neighbours.
pub const MAX_LEADS_PER_SIDE: i64 = 16;
const MAX_STEP: i64 = 16;

/// Land estimates: a lead's land is this much wider than the lead…
const LAND_WIDTH_EXTRA_NM: i64 = 200_000;
/// …and this much longer than its foot (toe 0.4 mm, heel 0.2 mm), its
/// centre moved out by half the difference…
const LAND_LENGTH_EXTRA_NM: i64 = 600_000;
const TOE_HEEL_SHIFT_NM: i64 = 100_000;
/// …and a flange's land this much longer than its solderable area,
/// centred on it.
const FLANGE_LAND_LENGTH_EXTRA_NM: i64 = 400_000;

/// The pin-1 dot: a filled silkscreen circle of this radius beyond pad 1's
/// −x edge, clear of its mask opening by the silkscreen clearance (as the
/// host's template marks pin 1 of a package of more than two terminals).
const PIN_ONE_DOT_RADIUS_NM: i64 = 200_000;

/// Rounds of the moulding's upright corners and top edges (est.: transfer
/// moulded bodies leave the mould with 0.1–0.3 mm radii), capped by 6 % of
/// the body's shorter side and a quarter of the room the leads leave at
/// its ends, and (the top edges) by 10 % of the body's height and half the
/// room above the leads.
const CORNER_ROUND_NM: i64 = 200_000;
const TOP_EDGE_ROUND_NM: i64 = 120_000;
/// tan 8°, the part files' mould draft (Microchip's SOT-23 and SOIC
/// drawings give 5–15°), as a fraction.
const DRAFT_TAN: [i64; 2] = [14_054, 100_000];

/// How far a lead root reaches into the moulding (at most a third of the
/// half body), and the shoulder's share of the room for the bend: the lead
/// profile the part file draws, used here only to refuse bends that do not
/// fit.
const ROOT_NM: i64 = 250_000;

pub struct Tabbed;

export_generator!(Tabbed);

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

fn count(key: &str, label: &str, min: i64, max: i64, default: i64) -> Param {
    Param {
        key: key.into(),
        label: label.into(),
        help: None,
        ty: ParamType::Count { min, max },
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
    // The lands: estimated from the leads, or the datasheet's land pattern
    // as given (the land defaults are the default package's estimates).
    let lands = Param {
        key: "lands".into(),
        label: "Lands".into(),
        help: Some(
            "Given: every lead's land is land width × land length, the rows land span apart \
             (centre to centre), the tab's land tab land width × tab land length."
                .into(),
        ),
        ty: ParamType::Enum {
            choices: vec![
                choice("estimated", "Estimated from the leads"),
                choice("explicit", "Given"),
            ],
        },
        default: ParamValue::Text("estimated".into()),
        group: Some("lands".into()),
        context: None,
    };
    let construction = Param {
        key: "construction".into(),
        label: "Construction".into(),
        help: Some(
            "A moulded body with gull-wing leads, or a PLCC LED's white body with a clear-filled cavity over its chips, its leads folded down the sides and under it (5050 addressable LEDs)."
                .into(),
        ),
        ty: ParamType::Enum {
            choices: vec![
                choice("moulded", "Moulded, gull-wing leads"),
                choice("led_plcc", "PLCC LED, leads folded under"),
            ],
        },
        default: ParamValue::Text("moulded".into()),
        group: Some("body".into()),
        context: None,
    };
    // The defaults: an original SOT-23-3 in round numbers, from no drawing
    // (the host's typical tabbed form).
    let mut params = vec![
        count("leads_s", "Leads on the −y side", 1, MAX_LEADS_PER_SIDE, 2),
        count("leads_n", "Leads on the +y side", 0, MAX_LEADS_PER_SIDE, 1),
        count("step_s", "Grid positions between −y leads", 1, MAX_STEP, 2),
        count("step_n", "Grid positions between +y leads", 1, MAX_STEP, 1),
        length(
            "pitch",
            "Lead grid pitch",
            100 * UM,
            20 * MM,
            950_000,
            "leads",
        ),
        length(
            "body_length",
            "Body length",
            200 * UM,
            50 * MM,
            2_900_000,
            "body",
        ),
        length(
            "body_width",
            "Body width",
            200 * UM,
            50 * MM,
            1_300_000,
            "body",
        ),
        length("body_height", "Body height", 100 * UM, 20 * MM, MM, "body"),
        length("standoff", "Standoff", 0, 2 * MM, 50 * UM, "body"),
        length(
            "span",
            "Lead span, tip to tip",
            300 * UM,
            60 * MM,
            2_400_000,
            "leads",
        ),
        length(
            "lead_width",
            "Lead width",
            50 * UM,
            5 * MM,
            400 * UM,
            "leads",
        ),
        length(
            "lead_thickness",
            "Lead thickness",
            20 * UM,
            2 * MM,
            120 * UM,
            "leads",
        ),
        length(
            "foot_length",
            "Foot length",
            50 * UM,
            10 * MM,
            400 * UM,
            "leads",
        ),
        Param {
            key: "tab".into(),
            label: "Tab on the +y side".into(),
            help: None,
            ty: ParamType::Enum {
                choices: vec![
                    choice("none", "None"),
                    choice("lead", "Tab lead (gull-wing)"),
                    choice("flange", "Flange under the body"),
                ],
            },
            default: ParamValue::Text("none".into()),
            group: Some("tab".into()),
            context: None,
        },
        length("tab_width", "Tab width", 100 * UM, 50 * MM, 3 * MM, "tab"),
        length(
            "tab_length",
            "Tab length on the seating plane",
            100 * UM,
            50 * MM,
            900 * UM,
            "tab",
        ),
        length(
            "flange",
            "Flange beyond the body's +y edge",
            0,
            20 * MM,
            MM,
            "tab",
        ),
        lands,
        length(
            "land_width",
            "Land width across the lead",
            50 * UM,
            10 * MM,
            600 * UM,
            "lands",
        ),
        length("land_length", "Land length", 50 * UM, 10 * MM, MM, "lands"),
        length(
            "land_span",
            "Rows' land span, centre to centre",
            100 * UM,
            60 * MM,
            2_200_000,
            "lands",
        ),
        length(
            "tab_land_width",
            "Tab land width",
            100 * UM,
            50 * MM,
            3_200_000,
            "lands",
        ),
        length(
            "tab_land_length",
            "Tab land length",
            100 * UM,
            50 * MM,
            1_500_000,
            "lands",
        ),
    ];
    params.push(construction);
    params.push(sparksmith_generator_sdk::marking::param());
    params.push(sparksmith_generator_sdk::marking::height_param());
    params.push(sparksmith_generator_sdk::marking::dot_param());
    let rules = vec![
        Rule {
            text: "Neighbouring leads must not touch: step × pitch > lead width".into(),
            fields: vec![
                "step_s".into(),
                "step_n".into(),
                "pitch".into(),
                "lead_width".into(),
            ],
        },
        Rule {
            text: "Each row, and the tab, fits along the body length with room at its corners"
                .into(),
            fields: vec![
                "leads_s".into(),
                "leads_n".into(),
                "tab_width".into(),
                "body_length".into(),
            ],
        },
        Rule {
            text: "The span leaves more than a lead thickness outside the body for the bend: \
                   (span − body width) / 2 − foot length > lead thickness"
                .into(),
            fields: vec![
                "span".into(),
                "body_width".into(),
                "foot_length".into(),
                "lead_thickness".into(),
            ],
        },
        Rule {
            text: "A tab takes the whole +y side: no +y leads with a tab".into(),
            fields: vec!["tab".into(), "leads_n".into()],
        },
        Rule {
            text: "Given lands must not touch: land span > land length, step × pitch > land width"
                .into(),
            fields: vec![
                "land_span".into(),
                "land_length".into(),
                "land_width".into(),
            ],
        },
    ];
    let offer = PartOffer {
        families: vec!["tabbed".into()],
        scopes: vec![Scope::Component, Scope::Package],
        grammar: vec![Grammar {
            scope: Scope::Component,
            patterns: vec![
                "sot-23-{leads_s}".into(),
                "sot {leads_s} leads".into(),
                "dpak".into(),
            ],
            examples: vec!["SOT-23-5".into(), "SOT-223".into(), "DPAK".into()],
        }],
        instance_fields: Vec::new(),
    };
    Description {
        abi: v1::ABI,
        id: "tabbed".into(),
        version: VERSION,
        title: "Tabbed small-outline and power packages".into(),
        licence: "MIT OR Apache-2.0".into(),
        deterministic: true,
        capabilities: vec![Capability {
            name: v1::GENERATE_PART.into(),
            effect: Effect::None,
            title: Some("Make a SOT, SOT-223 or DPAK package".into()),
            params,
            rules,
            presets,
            part: Some(offer),
        }],
        needs: Vec::new(),
    }
}

fn choice(key: &str, label: &str) -> Choice {
    Choice {
        key: key.into(),
        label: label.into(),
    }
}

/// The +y side's tab.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    None,
    /// A wide gull-wing tab lead reaching the span like the leads.
    Lead,
    /// A flat flange on the seating plane under the body, its outer edge
    /// `flange` beyond the body's +y edge.
    Flange,
}

/// A package's values, checked.
struct Dims {
    leads: [i64; 2],
    step: [i64; 2],
    pitch: i64,
    body_length: i64,
    body_width: i64,
    body_height: i64,
    standoff: i64,
    span: i64,
    lead_width: i64,
    lead_thickness: i64,
    foot_length: i64,
    tab: Tab,
    tab_width: i64,
    tab_length: i64,
    flange: i64,
    /// The datasheet's land pattern, when given.
    lands: Option<Lands>,
    /// A PLCC LED's body, its leads folded under it.
    plcc: bool,
}

/// A given land pattern: the lead land `[across the lead, along it]`, the
/// rows' land span (centre to centre) and the tab land.
#[derive(Clone, Copy)]
struct Lands {
    lead: Point2,
    span: i64,
    tab: Point2,
}

type Point2 = [i64; 2];

/// `(i − (n − 1) / 2) × pitch × step`, to the nearest nanometre.
fn centred(i: i64, n: i64, pitch: i64) -> i64 {
    div_round((2 * i - (n - 1)) * pitch, 2).unwrap_or(0)
}

/// `n / d` to the nearest nanometre (0 for a zero divisor, which the
/// callers never pass).
fn half(n: i64) -> i64 {
    div_round(n, 2).unwrap_or(0)
}

impl Dims {
    fn lead_count(&self) -> i64 {
        self.leads[0] + self.leads[1] + i64::from(self.tab != Tab::None)
    }

    /// The x of each lead of `row` (0: −y, 1: +y), from −x to +x.
    fn row_x(&self, row: usize) -> Vec<i64> {
        let (n, step) = (self.leads[row], self.step[row]);
        (0..n).map(|i| centred(i, n, self.pitch * step)).collect()
    }

    /// Length a row takes along x (0 for an empty row).
    fn row_length(&self, row: usize) -> i64 {
        let n = self.leads[row];
        if n == 0 {
            return 0;
        }
        (n - 1) * self.step[row] * self.pitch + self.lead_width
    }

    /// Where a lead's foot touches the seating plane, across y: the centre
    /// of its foot.
    fn reach(&self) -> i64 {
        half(self.span - self.foot_length)
    }

    /// The +y distance of the tab's outer edge from the centre (doubled, so
    /// it stays whole).
    fn tab_edge_twice(&self) -> i64 {
        match self.tab {
            Tab::Flange => self.body_width + 2 * self.flange,
            _ => self.span,
        }
    }

    /// Where the tab touches the seating plane: the centre of its contact.
    fn tab_point(&self) -> Option<Point2> {
        (self.tab != Tab::None).then(|| [0, half(self.tab_edge_twice() - self.tab_length)])
    }

    /// Lead contacts in lead order: the −y row in +x, the +y row in −x,
    /// then the tab.
    fn lead_points(&self) -> Vec<Point2> {
        let reach = self.reach();
        let mut out: Vec<Point2> = self.row_x(0).into_iter().map(|x| [x, -reach]).collect();
        out.extend(self.row_x(1).into_iter().rev().map(|x| [x, reach]));
        out.extend(self.tab_point());
        out
    }

    /// The envelope: the body's length, the widest of the span, the body
    /// and the tab (centred), and the body's top.
    fn envelope(&self) -> [i64; 3] {
        let width = self
            .span
            .max(self.body_width)
            .max(if self.tab == Tab::None {
                0
            } else {
                self.tab_edge_twice()
            });
        [self.body_length, width, self.standoff + self.body_height]
    }

    /// The room the leads leave at the body's ends for its corner rounds:
    /// a quarter of what the widest row (or the tab) leaves of the length.
    fn corner_room(&self) -> i64 {
        let widest = self
            .row_length(0)
            .max(self.row_length(1))
            .max(if self.tab == Tab::None {
                0
            } else {
                self.tab_width
            });
        div_round(self.body_length - widest, 4).unwrap_or(0)
    }

    /// The moulding's rounds: (upright corners, top edges).
    fn rounds(&self) -> (i64, i64) {
        let short = self.body_length.min(self.body_width);
        // The mould draft draws the top face's outline in by this much from
        // the parting line's (half the body height × tan 8°, the part
        // file's `draft`); the corners keep a round of at least 0.04 mm
        // there.
        let inset = div_round(self.body_height * DRAFT_TAN[0], 2 * DRAFT_TAN[1]).unwrap_or(0);
        let corner = CORNER_ROUND_NM
            .min(div_round(6 * short, 100).unwrap_or(0))
            .min(self.corner_room())
            .max(inset + 40 * UM);
        // Room above the leads: (top − lead exit − t / 2) / 2, the lead
        // leaving at mid-height of the moulding.
        let above = div_round(self.body_height - self.lead_thickness, 4).unwrap_or(0);
        let edge = TOP_EDGE_ROUND_NM
            .min(div_round(self.body_height, 10).unwrap_or(0))
            .min(above)
            .min(div_round(6 * (corner - inset), 10).unwrap_or(0));
        (corner, edge)
    }
}

fn dims(args: &Args<'_>) -> Result<Dims, Vec<Problem>> {
    let one = |p: Problem| vec![p];
    let tab = match args.text("tab").map_err(one)? {
        "lead" => Tab::Lead,
        "flange" => Tab::Flange,
        _ => Tab::None,
    };
    let d = Dims {
        leads: [
            args.int("leads_s").map_err(one)?,
            args.int("leads_n").map_err(one)?,
        ],
        step: [
            args.int("step_s").map_err(one)?,
            args.int("step_n").map_err(one)?,
        ],
        pitch: args.int("pitch").map_err(one)?,
        body_length: args.int("body_length").map_err(one)?,
        body_width: args.int("body_width").map_err(one)?,
        body_height: args.int("body_height").map_err(one)?,
        standoff: args.int("standoff").map_err(one)?,
        span: args.int("span").map_err(one)?,
        lead_width: args.int("lead_width").map_err(one)?,
        lead_thickness: args.int("lead_thickness").map_err(one)?,
        foot_length: args.int("foot_length").map_err(one)?,
        tab,
        tab_width: args.int("tab_width").map_err(one)?,
        tab_length: args.int("tab_length").map_err(one)?,
        flange: args.int("flange").map_err(one)?,
        lands: match args.text("lands").map_err(one)? {
            "explicit" => Some(Lands {
                lead: [
                    args.int("land_width").map_err(one)?,
                    args.int("land_length").map_err(one)?,
                ],
                span: args.int("land_span").map_err(one)?,
                tab: [
                    args.int("tab_land_width").map_err(one)?,
                    args.int("tab_land_length").map_err(one)?,
                ],
            }),
            _ => None,
        },
        plcc: args.text("construction").map_err(one)? == "led_plcc",
    };
    let problems = problems(&d);
    if problems.is_empty() {
        Ok(d)
    } else {
        Err(problems)
    }
}

/// The host's form rules for a tabbed package (the same sentences), the
/// room the moulding's corner rounds need, and the lead bends' fit.
fn problems(d: &Dims) -> Vec<Problem> {
    let mut out = Vec::new();
    let mut rule = |ok: bool, field: &str, message: &str| {
        if !ok {
            out.push(problem("param.inconsistent", field, message.to_string()));
        }
    };
    rule(
        d.leads[1] >= 1 || d.tab != Tab::None,
        "leads_n",
        "The +y side needs a lead or a tab.",
    );
    for (row, side) in [(0usize, "−y"), (1, "+y")] {
        let field = if row == 0 { "step_s" } else { "step_n" };
        if d.leads[row] >= 2 {
            rule(
                d.step[row] * d.pitch > d.lead_width,
                field,
                &format!("The {side} leads must not touch (step × pitch > lead width)."),
            );
        }
        if d.leads[row] >= 1 {
            rule(
                d.row_length(row) <= d.body_length,
                if row == 0 { "leads_s" } else { "leads_n" },
                &format!("The {side} leads must fit along the body length."),
            );
        }
    }
    rule(
        d.span > d.body_width,
        "span",
        "The lead span must reach past the body width.",
    );
    let t = d.lead_thickness;
    if d.plcc {
        rule(
            d.tab == Tab::None,
            "tab",
            "A PLCC LED has leads on both sides and no tab.",
        );
        rule(
            d.standoff >= t && d.span - d.body_width >= 2 * t && 2 * d.foot_length <= d.span,
            "foot_length",
            "A PLCC LED's leads fold under its body: the standoff holds their feet (standoff ≥ lead thickness), the span leaves a lead thickness beside the body, and each foot ends before the middle (foot length ≤ span / 2).",
        );
    } else {
        rule(
            2 * d.foot_length <= d.span - d.body_width,
            "foot_length",
            "The foot must lie outside the body (foot length ≤ (span − body width) / 2).",
        );
        rule(
            // Lead exit (standoff + height / 2) − t / 2 > t, doubled.
            2 * d.standoff + d.body_height - t > 2 * t && t < d.body_height,
            "lead_thickness",
            "Leads must fit the body height: thinner than the body, and leaving it (at mid-height) \
             more than 1.5 × lead thickness above the seating plane.",
        );
        rule(
            d.span - 2 * d.foot_length - d.body_width > 2 * t,
            "span",
            "The span must leave more than a lead thickness outside the body for the lead to bend \
             down to its foot ((span − body width) / 2 − foot length > lead thickness).",
        );
    }
    match d.tab {
        Tab::None => {}
        Tab::Lead | Tab::Flange => {
            rule(
                d.leads[1] == 0,
                "leads_n",
                "A tab takes the whole +y side: the +y side has no other leads (0).",
            );
            rule(
                d.tab_width <= d.body_length,
                "tab_width",
                "The tab must fit along the body length.",
            );
        }
    }
    match d.tab {
        Tab::Lead => rule(
            d.span - 2 * d.tab_length - d.body_width > 2 * t,
            "tab_length",
            "The tab lead's foot must leave more than a lead thickness outside the body to bend \
             ((span − body width) / 2 − tab length > lead thickness).",
        ),
        Tab::Flange => rule(
            d.tab_length <= d.body_width + d.flange,
            "tab_length",
            "The flange must end under the body (tab length ≤ body width + protrusion).",
        ),
        Tab::None => {}
    }
    if let Some(given) = d.lands {
        rule(
            given.span > given.lead[1],
            "land_span",
            "The two rows' lands must not touch (land span > land length).",
        );
        for row in 0..2 {
            if d.leads[row] >= 2 {
                rule(
                    d.step[row] * d.pitch > given.lead[0],
                    "land_width",
                    "Neighbouring lands must not touch (step × pitch > land width).",
                );
            }
        }
    }
    if out.is_empty() {
        if d.corner_room() <= 0 {
            out.push(problem(
                "param.inconsistent",
                "body_length",
                "The leads and the tab must leave room at the body's ends for its rounded \
                 corners: the widest row (or the tab) must be shorter than the body length."
                    .to_string(),
            ));
        }
        let mut feet = vec![d.foot_length];
        if d.tab == Tab::Lead {
            feet.push(d.tab_length);
        }
        if !d.plcc && feet.into_iter().any(|foot| !bends_fit(d, foot)) {
            out.push(problem(
                "param.inconsistent",
                "lead_thickness",
                "The leads are too thick for their bends: the lead's run between the body, the \
                 slope and the foot is too short to bend round (make the leads thinner, the span \
                 wider or the foot longer)."
                    .to_string(),
            ));
        }
    }
    out
}

/// Whether both bends of a lead whose foot is `foot` long fit their
/// segments, as the part file draws them: the centreline from the root
/// inside the body out at the exit height to the shoulder, down to the
/// foot and out to the tip, each corner bent at the lead thickness (less
/// where 45 % of a segment cannot hold it, never below half the
/// thickness), a bend never taking more than half a segment.
fn bends_fit(d: &Dims, foot: i64) -> bool {
    let t = i128::from(d.lead_thickness);
    let hb = i128::from(d.body_width) / 2;
    let root = i128::from(ROOT_NM).min(hb / 3);
    let exit = i128::from(d.standoff) + i128::from(d.body_height) / 2;
    let foot_in = i128::from(d.span) / 2 - i128::from(foot);
    let shoulder = hb + 3 * (foot_in - hb) / 10;
    let dx = foot_in - shoulder;
    let dy = exit - t / 2;
    if dx <= 0 || dy <= 0 {
        return false;
    }
    let slope = (dx * dx + dy * dy).isqrt();
    // tan(θ / 2) = dy / (slope + dx).
    let k_den = slope + dx;
    let first = shoulder - (hb - root);
    [(first, slope), (slope, i128::from(foot))]
        .into_iter()
        .all(|(lin, lout)| {
            let shorter = lin.min(lout);
            let fit = 45 * shorter * k_den / (100 * dy);
            let r = t.min(fit).max(t / 2 + t / 20_000);
            // The bend's tangent length r × tan(θ / 2) at most half the
            // shorter segment.
            shorter > 0 && 2 * r * dy <= shorter * k_den
        })
}

/// A length in mm as people write it: at least one decimal, at most as
/// many as it needs (2.9, 0.95, 2.285).
fn mm(nm: i64) -> String {
    let whole = nm / MM;
    let mut frac = format!("{:06}", (nm % MM).abs());
    while frac.len() > 1 && frac.ends_with('0') {
        frac.pop();
    }
    format!("{whole}.{frac}")
}

fn pads(d: &Dims) -> Vec<Pad> {
    let lead = match d.lands {
        Some(given) => given.lead,
        None => [
            d.lead_width + LAND_WIDTH_EXTRA_NM,
            d.foot_length + LAND_LENGTH_EXTRA_NM,
        ],
    };
    let given_tab = d.lands.map(|l| l.tab);
    let leads = d.leads[0] + d.leads[1];
    d.lead_points()
        .into_iter()
        .enumerate()
        .map(|(k, [x, y])| {
            let k = i64::try_from(k).unwrap_or(i64::MAX);
            let key = (k + 1).to_string();
            let tab = k >= leads;
            let (size, at) = match (tab, d.tab) {
                (true, Tab::Flange) => (
                    given_tab.unwrap_or([
                        d.tab_width + LAND_WIDTH_EXTRA_NM,
                        d.tab_length + FLANGE_LAND_LENGTH_EXTRA_NM,
                    ]),
                    [x, y],
                ),
                (true, _) => (
                    given_tab.unwrap_or([
                        d.tab_width + LAND_WIDTH_EXTRA_NM,
                        d.tab_length + LAND_LENGTH_EXTRA_NM,
                    ]),
                    [x, y + TOE_HEEL_SHIFT_NM],
                ),
                (false, _) => match d.lands {
                    // The datasheet's rows, centre to centre.
                    Some(given) => (lead, [x, y.signum() * half(given.span)]),
                    // A toe-heavy land just outside the foot.
                    None => (lead, [x, y + y.signum() * TOE_HEEL_SHIFT_NM]),
                },
            };
            Pad {
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
            }
        })
        .collect()
}

/// A rectangle with the first half of the pins down its left side and the
/// rest up its right side, on the symbol grid, the designator above and
/// the name below: an IC's starting point, as the host's templates draw
/// it.
fn symbol(pins: i64) -> Symbol {
    let s = draw::MODULE_NM;
    let left = (pins + 1) / 2;
    let rows = left.max(1);
    let top = (rows - 1) / 2 * s;
    let mut d = Drawing::default();
    d.shape(
        Primitive::Rect {
            min: [-2 * s, top - rows * s],
            max: [2 * s, top + s],
        },
        draw::STROKE_NM,
        Fill::Body,
    );
    d.labels(top + s, top - rows * s);
    for k in 0..pins {
        let terminal = (k + 1).to_string();
        let (at, rot) = if k < left {
            ([-3 * s, top - k * s], RIGHT)
        } else {
            ([3 * s, top - (pins - 1 - k) * s], LEFT)
        };
        d.pins.push(v1::SymbolPin {
            terminal,
            at,
            length_nm: s,
            rot_mdeg: rot,
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

/// The 3D model: one of the bundle's three part files, by the +y side's
/// shape, every parameter given. Each draws a moulded body on its standoff
/// with bent gull-wing leads in rows (a body pattern per row whose count
/// and spacing are parameters), the body cut round them.
/// - `tabbed-rows`: leads on both sides (SOT-23, SC-70);
/// - `tabbed-tab-lead`: −y leads and a wide gull-wing tab lead (SOT-223,
///   SOT-89);
/// - `tabbed-flange`: −y leads and a flat flange under the body (DPAK).
fn model(d: &Dims) -> v1::Model {
    let (corner, edge) = d.rounds();
    let mut values = v1::Params::new();
    let mut set = |name: &str, v: i64| {
        values.insert(name.into(), ParamValue::Int(v));
    };
    set("pitch", d.pitch);
    set("body_length", d.body_length);
    set("body_width", d.body_width);
    set("body_height", d.body_height);
    set("standoff", d.standoff);
    set("span", d.span);
    set("lead_width", d.lead_width);
    set("lead_thickness", d.lead_thickness);
    set("foot_length", d.foot_length);
    set("row_s_leads", d.leads[0]);
    set("row_s_step", d.step[0]);
    set("corner_round", corner);
    set("edge_round", edge);
    let asset = match d.tab {
        Tab::None => {
            set("row_n_leads", d.leads[1]);
            set("row_n_step", d.step[1]);
            if d.plcc {
                parts::TABBED_PLCC_LED
            } else {
                parts::TABBED_ROWS
            }
        }
        Tab::Lead => {
            set("tab_width", d.tab_width);
            set("tab_length", d.tab_length);
            parts::TABBED_TAB_LEAD
        }
        Tab::Flange => {
            set("tab_width", d.tab_width);
            set("tab_length", d.tab_length);
            set("flange_protrusion", d.flange);
            parts::TABBED_FLANGE
        }
    };
    v1::Model::PartFile {
        asset: asset.into(),
        values,
    }
}

/// A default name from the values: the lead counts and the body.
fn name(d: &Dims) -> String {
    let body = format!("{} × {} mm", mm(d.body_length), mm(d.body_width));
    let pitch = mm(d.pitch);
    match d.tab {
        Tab::None => format!(
            "SOT {} leads ({} + {}), {pitch} mm grid, {body}",
            d.leads[0] + d.leads[1],
            d.leads[0],
            d.leads[1]
        ),
        Tab::Lead => format!("SOT {} leads and tab, {pitch} mm grid, {body}", d.leads[0]),
        Tab::Flange => format!(
            "DPAK {} leads and flange, {pitch} mm grid, {body}",
            d.leads[0]
        ),
    }
}

fn generate(request: &Request) -> Result<Generation, Vec<Problem>> {
    let description = description_with(Vec::new());
    let args = checked(&description, &request.params)?;
    let d = dims(&args)?;
    let pads = pads(&d);
    let [length, width, height] = d.envelope();
    let body = Box2::centred([0, 0], [length, width]);
    let courtyard = land::courtyard(&pads, Some(body)).unwrap_or_default();
    let top = courtyard.iter().map(|p| p[1]).max().unwrap_or(body.max[1]);
    let mut silk = land::silk_outline(&pads, body, land::DEFAULT_MASK_NM);
    silk.push(land::silk_designator(body, top));
    // More than two terminals always: the pin-1 dot.
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
    let keys: Vec<String> = (1..=d.lead_count()).map(|k| k.to_string()).collect();
    let package = Package {
        terminals: keys.clone(),
        geometry: PackageGeometry {
            envelope_nm: [length, width, height],
            seating_nm: 0,
            leads: keys
                .iter()
                .zip(d.lead_points())
                .map(|(key, [x, y])| Lead {
                    key: key.clone(),
                    contact: [x, y, 0],
                })
                .collect(),
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
    Ok(Generation {
        marking: sparksmith_generator_sdk::marking::of(&args, false).map_err(|p| vec![p])?,
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
        symbol: component_part.then(|| symbol(d.lead_count())),
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
        notes: vec![if d.lands.is_some() {
            "Lands are the given land pattern; confirm them against the device's recommended \
             land pattern."
                .into()
        } else {
            "Lands are estimated from the leads (0.2 mm wider, 0.6 mm longer than the foot); \
             confirm them against the device's recommended land pattern."
                .into()
        }],
    })
}

impl Generator for Tabbed {
    fn describe() -> Description {
        description()
    }

    fn generate(request: &Request) -> Result<Generation, Vec<Problem>> {
        generate(request)
    }
}

#[cfg(test)]
mod tests;
