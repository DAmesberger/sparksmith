//! The tactile generator: surface-mount top-push tactile switches with
//! four terminals, mini (J-bent terminals under the base ends, a push
//! plate; the Panasonic EVQP2 class, optionally with the cover's ground
//! tab) or square (gull-wing terminals, a round plunger; the 6 × 6 and
//! 4.5 × 4.5 mm classes), as a symbol, package, footprint and 3D model.
//!
//! The parameters are the drawing's: the base (`body_length` along the
//! terminals' axis x, `body_width`), the cover's top (`body_height`), the
//! actuator's top (`height`) and size (`plunger_length` × `plunger_width`;
//! a square switch's plunger is round, `plunger_length` across), the
//! terminals (`span` over their tips, `terminal_width`, `terminal_pitch`
//! across), and the manufacturer's land pattern (lands `land_length` ×
//! `land_width` centred at ±`land_x`, ±`land_y`; the ground tab's land
//! `ground_land_length` × `ground_land_width` at −`ground_land_y`).
//!
//! Terminals: mini 1 (−x −y), 2 (−x +y), 3 (+x −y), 4 (+x +y), as the
//! EVQP2 drawing numbers them; square 1 (−x −y), 2 (+x −y), 3 (−x +y),
//! 4 (+x +y). `common` says which terminals are joined inside: `across`
//! (the pair at one y) or `along` (the pair at one x). The switch's
//! terminal 1 is the pair at −y (across) or −x (along); with the ground
//! tab, package terminal 5 is the switch's terminal 3 (`SH`). With
//! `separate`, each package terminal is its own component terminal of the
//! same key (the ground tab's 5, `SH`), the joints drawn in the symbol, as
//! the datasheets' circuit diagrams do. Without `ground_land` the ground
//! tab is drawn but lands nothing: no pad, no package terminal.
#![cfg_attr(target_arch = "wasm32", no_std)]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use sparksmith_generator_sdk::args::{Args, checked, problem};
use sparksmith_generator_sdk::draw::{Drawing, LEFT, RIGHT, TWO_TERMINAL_PIN_X_NM, UP};
use sparksmith_generator_sdk::land::{self, Box2};
use sparksmith_generator_sdk::units::{MM, UM, div_round, mm_text};
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
/// 4: the ground terminal's lead contact; 5: the SDK's pin table
/// (`pin_table`) and part numbers (`part_numbers`).
const VERSION: u32 = 5;

/// Estimates. Mini: the cover sheet, the J's tab height
/// (a third of the cover's), foot length, root in the base, the opening's
/// clearance, the side tabs and the ground tab's reach and length.
const MINI_COVER_NM: i64 = 100_000;
const MINI_RISE_NM: i64 = 500_000;
const MINI_FOOT_NM: i64 = 700_000;
const ROOT_NM: i64 = 300_000;
const CLEARANCE_NM: i64 = 100_000;
const MINI_TAB_NM: i64 = 800_000;
const MINI_TAB_DROP_NM: i64 = 700_000;
const GROUND_OUT_NM: i64 = 300_000;
const GROUND_LENGTH_NM: i64 = 1_000_000;
/// Square: the cover sheet, the corner studs, the terminal's exit height
/// (at most half the cover's), the gap to its leg and the side tabs.
const SQUARE_COVER_NM: i64 = 150_000;
const STUD_NM: i64 = 700_000;
const STUD_HEIGHT_NM: i64 = 250_000;
const EXIT_NM: i64 = 700_000;
const LEAD_GAP_NM: i64 = 300_000;
const SQUARE_TAB_NM: i64 = 1_200_000;

pub struct Tactile;

export_generator!(Tactile);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Style {
    Mini,
    Square,
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

fn choices(key: &str, label: &str, options: &[(&str, &str)], default: &str, group: &str) -> Param {
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

fn flag(key: &str, label: &str, help: &str, default: bool) -> Param {
    Param {
        key: key.into(),
        label: label.into(),
        help: Some(help.into()),
        ty: ParamType::Bool,
        default: ParamValue::Bool(default),
        group: Some("terminals".into()),
        context: None,
    }
}

fn description() -> Description {
    description_with(sparksmith_generator_sdk::presets(PRESETS).unwrap_or_default())
}

fn description_with(presets: Vec<Preset>) -> Description {
    // The defaults are Panasonic's EVQP2V02M.
    let params = vec![
        choices(
            "style",
            "Construction",
            &[
                ("mini", "Mini, J-bent terminals, push plate"),
                ("square", "Square, gull-wing terminals, round plunger"),
            ],
            "mini",
            "body",
        ),
        length(
            "body_length",
            "Base length (x, along the terminals)",
            [MM, 20 * MM],
            4_700_000,
            "body",
        ),
        length(
            "body_width",
            "Base width (y)",
            [MM, 20 * MM],
            3_500_000,
            "body",
        ),
        length(
            "body_height",
            "Cover top height",
            [300 * UM, 15 * MM],
            1_550_000,
            "body",
        ),
        length(
            "height",
            "Actuator top height",
            [500 * UM, 25 * MM],
            2_100_000,
            "body",
        ),
        length(
            "plunger_length",
            "Actuator length (square: diameter)",
            [300 * UM, 15 * MM],
            2_600_000,
            "body",
        ),
        length(
            "plunger_width",
            "Actuator width (mini)",
            [300 * UM, 15 * MM],
            1_600_000,
            "body",
        ),
        length(
            "span",
            "Overall length over the terminals",
            [MM, 30 * MM],
            5_500_000,
            "terminals",
        ),
        length(
            "terminal_width",
            "Terminal width",
            [100 * UM, 3 * MM],
            600_000,
            "terminals",
        ),
        length(
            "terminal_pitch",
            "Terminal centres across (y)",
            [300 * UM, 20 * MM],
            1_700_000,
            "terminals",
        ),
        length(
            "lead_thickness",
            "Terminal thickness",
            [50 * UM, MM],
            150_000,
            "terminals",
        ),
        choices(
            "common",
            "Terminals joined inside",
            &[
                ("across", "The pair at one y"),
                ("along", "The pair at one x"),
            ],
            "across",
            "terminals",
        ),
        flag(
            "separate",
            "Each terminal its own pin",
            "Every package terminal is its own component terminal; the joints inside are drawn in the symbol.",
            false,
        ),
        Param {
            key: "ground".into(),
            label: "Ground tab (mini)".into(),
            help: Some("The cover's unplated ground tab down the −y side.".into()),
            ty: ParamType::Bool,
            default: ParamValue::Bool(false),
            group: Some("terminals".into()),
            context: None,
        },
        flag(
            "ground_land",
            "Ground tab soldered (mini)",
            "The ground tab has its land and package terminal; off, it is drawn but lands nothing.",
            true,
        ),
        length(
            "land_x",
            "Land centre x",
            [200 * UM, 20 * MM],
            2_575_000,
            "lands",
        ),
        length(
            "land_y",
            "Land centre y",
            [100 * UM, 20 * MM],
            1_000_000,
            "lands",
        ),
        length(
            "land_length",
            "Land length (x)",
            [200 * UM, 10 * MM],
            1_450_000,
            "lands",
        ),
        length(
            "land_width",
            "Land width (y)",
            [200 * UM, 10 * MM],
            700_000,
            "lands",
        ),
        length(
            "ground_land_y",
            "Ground land centre −y",
            [200 * UM, 20 * MM],
            2_025_000,
            "lands",
        ),
        length(
            "ground_land_length",
            "Ground land length (x)",
            [200 * UM, 10 * MM],
            1_400_000,
            "lands",
        ),
        length(
            "ground_land_width",
            "Ground land width (y)",
            [200 * UM, 10 * MM],
            650_000,
            "lands",
        ),
    ];
    let rule = |text: &str, fields: &[&str]| Rule {
        text: text.into(),
        fields: fields.iter().map(|f| (*f).into()).collect(),
    };
    let rules = vec![
        rule(
            "body height < height; actuator inside the base",
            &["body_height", "height", "plunger_length", "plunger_width"],
        ),
        rule(
            "Terminals outside the base and apart: span > base length, pitch > terminal width",
            &["span", "body_length", "terminal_pitch", "terminal_width"],
        ),
        rule(
            "Lands apart: 2 × land x > land length, 2 × land y > land width",
            &["land_x", "land_length", "land_y", "land_width"],
        ),
    ];
    let offer = PartOffer {
        families: vec!["tactile".into()],
        scopes: vec![Scope::Component, Scope::Package],
        grammar: vec![Grammar {
            scope: Scope::Component,
            patterns: vec!["tactile switch {size:code}".into()],
            examples: vec![
                "tactile switch EVQP2".into(),
                "tactile switch 6x6".into(),
                "tactile switch 4.5x4.5".into(),
            ],
        }],
        instance_fields: vec![InstanceField {
            standard_field: "value".into(),
            when: v1::Params::new(),
        }],
    };
    Description {
        abi: v1::ABI,
        id: "tactile".into(),
        version: VERSION,
        title: "Tactile switches (SMD)".into(),
        licence: "MIT OR Apache-2.0".into(),
        deterministic: true,
        capabilities: vec![Capability {
            name: v1::GENERATE_PART.into(),
            effect: Effect::None,
            title: Some("Make a surface-mount tactile switch".into()),
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
    bh: i64,
    h: i64,
    plunger: [i64; 2],
    span: i64,
    tw: i64,
    pitch: i64,
    t: i64,
    along: bool,
    separate: bool,
    ground: bool,
    ground_soldered: bool,
    land_at: [i64; 2],
    land: [i64; 2],
    ground_land_y: i64,
    ground_land: [i64; 2],
}

fn part_of(n: i64, d: i64) -> i64 {
    div_round(n, d).unwrap_or(0)
}

impl Dims {
    /// The four terminals' keys and their contact points.
    fn terminals(&self) -> [(&'static str, [i64; 2]); 4] {
        let y = part_of(self.pitch, 2);
        let x = match self.style {
            // The middle of the foot under the base's end.
            Style::Mini => part_of(self.span, 2) - part_of(self.mini_foot(), 2),
            // The middle of the gull-wing's foot.
            Style::Square => part_of(
                part_of(self.span, 2) + part_of(self.bl, 2) + self.lead_gap(),
                2,
            ),
        };
        match self.style {
            Style::Mini => [
                ("1", [-x, -y]),
                ("2", [-x, y]),
                ("3", [x, -y]),
                ("4", [x, y]),
            ],
            Style::Square => [
                ("1", [-x, -y]),
                ("2", [x, -y]),
                ("3", [-x, y]),
                ("4", [x, y]),
            ],
        }
    }

    /// The ground tab's contact from the centre, along −y: the middle of
    /// its foot, which reaches `GROUND_OUT_NM` beyond the base's side.
    fn ground_contact_y(&self) -> i64 {
        part_of(self.bw, 2) + part_of(GROUND_OUT_NM, 2)
    }

    /// The ground tab has a land (and a package terminal).
    fn ground_landed(&self) -> bool {
        self.ground && self.ground_soldered
    }

    /// The package terminals joined inside as the switch's terminal 1 (the
    /// pair at −y across, −x along) and 2.
    fn pairs(&self) -> [[&'static str; 2]; 2] {
        let terminals = self.terminals();
        let mut pairs = [[""; 2]; 2];
        let mut filled = [0usize; 2];
        for (key, [x, y]) in terminals {
            let side = usize::from(if self.along { x >= 0 } else { y >= 0 });
            if let (Some(pair), Some(n)) = (pairs.get_mut(side), filled.get_mut(side))
                && let Some(slot) = pair.get_mut(*n)
            {
                *slot = key;
                *n += 1;
            }
        }
        pairs
    }

    fn mini_foot(&self) -> i64 {
        MINI_FOOT_NM.min(part_of(self.span, 2) - part_of(self.bl, 4))
    }

    fn lead_gap(&self) -> i64 {
        LEAD_GAP_NM.min(part_of(part_of(self.span - self.bl, 2) - self.t, 3))
    }

    fn envelope(&self) -> [i64; 3] {
        let width = if self.ground {
            self.bw + 2 * GROUND_OUT_NM
        } else {
            self.bw
        };
        [self.span.max(self.bl), width, self.h]
    }
}

fn dims(args: &Args<'_>) -> Result<Dims, Vec<Problem>> {
    let one = |p: Problem| vec![p];
    let int = |k: &str| args.int(k).map_err(one);
    let style = match args.text("style").map_err(one)? {
        "square" => Style::Square,
        _ => Style::Mini,
    };
    let d = Dims {
        style,
        bl: int("body_length")?,
        bw: int("body_width")?,
        bh: int("body_height")?,
        h: int("height")?,
        plunger: [int("plunger_length")?, int("plunger_width")?],
        span: int("span")?,
        tw: int("terminal_width")?,
        pitch: int("terminal_pitch")?,
        t: int("lead_thickness")?,
        along: args.text("common").map_err(one)? == "along",
        separate: args.flag("separate").map_err(one)?,
        ground: args.flag("ground").map_err(one)?,
        ground_soldered: args.flag("ground_land").map_err(one)?,
        land_at: [int("land_x")?, int("land_y")?],
        land: [int("land_length")?, int("land_width")?],
        ground_land_y: int("ground_land_y")?,
        ground_land: [int("ground_land_length")?, int("ground_land_width")?],
    };
    let mut problems = Vec::new();
    let mut rule = |ok: bool, field: &str, message: &str| {
        if !ok {
            problems.push(problem("param.inconsistent", field, message.into()));
        }
    };
    rule(
        d.bh + 100 * UM < d.h,
        "height",
        "The actuator must stand above the cover (height > cover top + 0.1 mm).",
    );
    let [pl, pw] = d.plunger;
    let fits = match style {
        Style::Mini => pl + 2 * CLEARANCE_NM < d.bl && pw + 2 * CLEARANCE_NM < d.bw,
        Style::Square => pl + 2 * STUD_NM < d.bl.min(d.bw),
    };
    rule(
        fits,
        "plunger_length",
        "The actuator must fit inside the cover (square: beside its corner studs).",
    );
    rule(
        d.span > d.bl + 2 * d.t + 200 * UM,
        "span",
        "The terminals must stand outside the base (span > base length + 2 × thickness + 0.2 mm).",
    );
    rule(
        d.pitch > d.tw && d.pitch + d.tw < d.bw,
        "terminal_pitch",
        "The terminals must lie apart and within the base's width.",
    );
    rule(
        d.t < part_of(d.bh, 3),
        "lead_thickness",
        "The terminals must be thinner than a third of the cover's height.",
    );
    rule(
        !(d.ground && style == Style::Square),
        "ground",
        "Only the mini switch has the ground tab.",
    );
    rule(
        2 * d.land_at[0] > d.land[0] && 2 * d.land_at[1] > d.land[1],
        "land_x",
        "The lands must not meet (2 × land x > land length, 2 × land y > land width).",
    );
    if problems.is_empty() {
        Ok(d)
    } else {
        Err(problems)
    }
}

fn pad(key: &str, at: [i64; 2], size: [i64; 2]) -> Pad {
    Pad {
        key: key.into(),
        terminal: key.into(),
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
}

fn pads(d: &Dims) -> Vec<Pad> {
    let [x, y] = d.land_at;
    let mut pads: Vec<Pad> = d
        .terminals()
        .iter()
        .map(|(key, at)| pad(key, [at[0].signum() * x, at[1].signum() * y], d.land))
        .collect();
    if d.ground_landed() {
        pads.push(pad("5", [0, -d.ground_land_y], d.ground_land));
    }
    pads
}

/// A normally open push-button (IEC 60617 as published): two contacts,
/// the bridging bar above them and its stem; the ground tab's terminal
/// (`ground`) below. With `separate`, each contact has the pins of its
/// joined pair, one below the other, the joint drawn between them.
fn symbol(ground: Option<&str>, separate: Option<[[&str; 2]; 2]>) -> Symbol {
    let mut d = Drawing::default();
    let m = |v: i64| v * MM / 1000;
    let x = TWO_TERMINAL_PIN_X_NM;
    match separate {
        None => d.two_pins("1", "2", m(2500), m(2500)),
        Some([[a, b], [c, e]]) => {
            d.pin(a, [-x, 0], RIGHT, x - m(2500));
            d.pin(b, [-x, m(-2500)], RIGHT, x - m(2500));
            d.pin(c, [x, 0], LEFT, x - m(2500));
            d.pin(e, [x, m(-2500)], LEFT, x - m(2500));
            d.line([m(-2500), m(-2500)], [m(-2500), 0]);
            d.line([m(2500), m(-2500)], [m(2500), 0]);
        }
    }
    d.line([m(-2500), 0], [m(-2500), m(500)]);
    d.line([m(2500), 0], [m(2500), m(500)]);
    d.line([m(-2500), m(1250)], [m(2500), m(1250)]);
    d.line([0, m(1250)], [0, m(2500)]);
    d.line([m(-750), m(2500)], [m(750), m(2500)]);
    if let Some(key) = ground {
        d.pin(key, [0, m(-5000)], UP, m(2500));
    }
    let bottom = match (ground, separate) {
        (Some(_), _) => m(-5000),
        (None, Some(_)) => m(-2500),
        (None, None) => m(-500),
    };
    d.labels(m(2500), bottom);
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
    set("span", d.span);
    set("terminal_width", d.tw);
    set("terminal_pitch", d.pitch);
    set("lead_thickness", d.t);
    set("lead_root", ROOT_NM.min(part_of(d.bl, 6)));
    let asset = match d.style {
        Style::Mini => {
            let cover = MINI_COVER_NM.min(part_of(d.bh, 10));
            set("cover_height", d.bh);
            set("cover_thickness", cover);
            set("lead_rise", MINI_RISE_NM.min(part_of(d.bh, 3)).max(2 * d.t));
            set("foot_length", d.mini_foot());
            set("plate_length", d.plunger[0]);
            set("plate_width", d.plunger[1]);
            set(
                "plate_round",
                part_of(d.plunger[0].min(d.plunger[1]), 2) - 100 * UM,
            );
            set("plate_clearance", CLEARANCE_NM);
            set("tab_length", MINI_TAB_NM.min(part_of(d.bl, 6)));
            set("tab_drop", MINI_TAB_DROP_NM.min(part_of(d.bh, 2)));
            set("ground_length", GROUND_LENGTH_NM.min(part_of(d.bl, 4)));
            set("ground_out", GROUND_OUT_NM);
            if d.ground {
                parts::TACTILE_MINI_GROUND
            } else {
                parts::TACTILE_MINI
            }
        }
        Style::Square => {
            set("body_height", d.bh);
            set("plunger_diameter", d.plunger[0]);
            set("cover_thickness", SQUARE_COVER_NM.min(part_of(d.bh, 8)));
            set("stud_diameter", STUD_NM.min(part_of(d.bl.min(d.bw), 6)));
            set("stud_height", STUD_HEIGHT_NM.min(part_of(d.h - d.bh, 4)));
            set("lead_exit", EXIT_NM.min(part_of(d.bh, 2)).max(2 * d.t));
            set("lead_gap", d.lead_gap());
            set("tab_length", SQUARE_TAB_NM.min(part_of(d.bl, 4)));
            parts::TACTILE_SQUARE
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
    let body = Box2::centred([0, 0], [d.bl, d.bw]);
    let courtyard = land::courtyard(
        &pads,
        Some(Box2::centred([0, 0], [envelope[0], envelope[1]])),
    )
    .unwrap_or_default();
    let top = courtyard.iter().map(|p| p[1]).max().unwrap_or(body.max[1]);
    let mut silk = land::silk_outline(&pads, body, land::DEFAULT_MASK_NM);
    silk.push(land::silk_designator(body, top));
    let terminals = d.terminals();
    let mut package_terminals: Vec<String> = terminals.iter().map(|(k, _)| (*k).into()).collect();
    if d.ground_landed() {
        package_terminals.push("5".into());
    }
    let package = Package {
        terminals: package_terminals,
        geometry: PackageGeometry {
            envelope_nm: envelope,
            seating_nm: 0,
            leads: terminals
                .iter()
                .map(|(key, [x, y])| Lead {
                    key: (*key).into(),
                    contact: [*x, *y, 0],
                })
                .chain(d.ground_landed().then(|| Lead {
                    key: "5".into(),
                    contact: [0, -d.ground_contact_y(), 0],
                }))
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
    let class = match d.style {
        Style::Mini => "mini",
        Style::Square => "square",
    };
    let name = format!(
        "Tactile switch {class} {} × {} × {} mm",
        mm_text(d.bl).trim_end_matches(" mm"),
        mm_text(d.bw).trim_end_matches(" mm"),
        mm_text(d.h).trim_end_matches(" mm"),
    );
    // The switch's terminal of each package terminal: 1 for the pair at −y
    // (across) or −x (along); separate, its own key.
    let mut binding: Vec<BindingEntry> = terminals
        .iter()
        .map(|(key, [x, y])| {
            let low = if d.along { *x < 0 } else { *y < 0 };
            let terminal = if d.separate {
                key
            } else if low {
                "1"
            } else {
                "2"
            };
            BindingEntry {
                package_terminal: (*key).into(),
                terminal: terminal.into(),
            }
        })
        .collect();
    let mut names: Vec<(&str, &str)> = if d.separate {
        terminals.iter().map(|(key, _)| (*key, *key)).collect()
    } else {
        vec![("1", "1"), ("2", "2")]
    };
    let ground_terminal = d
        .ground_landed()
        .then_some(if d.separate { "5" } else { "3" });
    if let Some(terminal) = ground_terminal {
        binding.push(BindingEntry {
            package_terminal: "5".into(),
            terminal: terminal.into(),
        });
        names.push((terminal, "SH"));
    }
    Ok(Generation {
        orderable: Vec::new(),
        marking: None,
        component: component_part.then(|| Component {
            terminals: names
                .iter()
                .map(|(key, name)| Terminal {
                    key: (*key).into(),
                    name: (*name).into(),
                    direction: Direction::Passive,
                    alternates: Vec::new(),
                    drive: None,
                    unconnected: None,
                })
                .collect(),
            prefix: "SW".into(),
            name,
            fields: vec![String::from("value")],
        }),
        symbol: component_part.then(|| symbol(ground_terminal, d.separate.then(|| d.pairs()))),
        package,
        footprint,
        binding: if component_part { binding } else { Vec::new() },
        model: Some(model(&d)),
        fields: Vec::new(),
        notes: vec![
            "Lands are the manufacturer's as given; sheet thicknesses, bends, studs and rounds \
             are estimated."
                .into(),
        ],
    })
}

impl Generator for Tactile {
    fn describe() -> Description {
        description()
    }

    fn generate(request: &Request) -> Result<Generation, Vec<Problem>> {
        generate(request)
    }
}

#[cfg(test)]
mod tests;
