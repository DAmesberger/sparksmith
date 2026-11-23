//! The no-lead generator: DFN/SON packages (terminals on the two long
//! sides) and QFN packages (on all four), with or without an exposed pad,
//! as a symbol, package, footprint and 3D model.
//!
//! Terminals are numbered counter-clockwise seen from above, from the −x
//! end of the −y row: the −y row in +x, then (four sides) the +x row in
//! +y, the +y row in −x and the −x row in −y. The exposed pad follows as
//! terminal `EP`.
//!
//! Lands are estimated (an original rule of thumb, not a land-pattern
//! standard), as the host's no-lead template did: each perimeter land is
//! 0.05 mm wider than its terminal, but never so wide that neighbours at
//! the pitch lose the least mask web (`mask_web` plus the inherited mask
//! opening on both sides), and never narrower than the terminal; it is the
//! terminal's length + 0.4 mm long, centred on the terminal. With an
//! exposed pad, every perimeter land's heel is pulled back so it keeps
//! `pad_clearance` from the pad (at most half the land). On four sides,
//! the end lands of two sides that meet at a corner give up the least heel
//! that leaves the same copper gap between them.
//!
//! The presets are the no-lead starter table, in nm: JEDEC MO-220 (QFN)
//! and MO-229 (DFN) body sizes, pitches and terminals, each with one
//! typical exposed pad size, as the table says.
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
    Scope, Side, Symbol, SymbolPin, Terminal,
};
use sparksmith_generator_sdk::{Generator, export_generator};

/// The starter rows, in nm.
const PRESETS: &str = include_str!("../presets.json");

/// The generator's version (`Description::version`): 6 takes terminals
/// and lands given one by one (`terminal_numbers`, `given_land_numbers`;
/// the `sawn-terminals` part file); 8 gives the TI and Sensirion presets
/// their datasheets' pin tables (`pin_table`) and the TI and ADI presets
/// their ordering codes (`part_numbers`).
const VERSION: u32 = 9;

/// Most terminals on one side: the part files' looks are assigned up to
/// this count.
const MAX_PER_SIDE: i64 = 64;

/// A land is this much wider than its terminal…
const LAND_WIDTH_EXTRA_NM: i64 = 50_000;
/// …and this much longer.
const LAND_LENGTH_EXTRA_NM: i64 = 400_000;

/// The pin-1 dot: a filled silkscreen circle of this radius beyond pad 1's
/// −x edge, clear of its mask opening by the silkscreen clearance (as the
/// host's template marks pin 1 of a package of more than two terminals).
const PIN_ONE_DOT_RADIUS_NM: i64 = 200_000;

/// Leadframe thickness (est.: 0.2 mm is the common QFN frame), at most a
/// third of the body height.
const FRAME_NM: i64 = 200_000;
/// An LGA's lands: thin gold over nickel on its substrate (est.).
const LGA_LAND_NM: i64 = 50_000;
/// Round of the sawn body's top edges (est.: a saw-singulated moulding
/// keeps nearly sharp edges; a small round only catches the light), at
/// most a tenth of the body height.
const TOP_EDGE_NM: i64 = 30_000;
/// The pin-1 mark: a laser-marked dot (sawn packages are moulded as one
/// block, so they carry no moulded dimple; TI's RGV and DML outlines show
/// a pin-1 index area and Sensirion's SHT4x a dot), this deep, its radius
/// 6 % of the shorter body side within 0.1–0.25 mm, centred two radii in
/// from the −x, −y corner (est.).
const DOT_DEPTH_NM: i64 = 5_000;
const DOT_RADIUS_NM: [i64; 2] = [100_000, 250_000];

pub struct NoLead;

export_generator!(NoLead);

fn length(key: &str, label: &str, range: [i64; 2], default: i64, group: &str) -> Param {
    Param {
        key: key.into(),
        label: label.into(),
        help: None,
        ty: ParamType::Length {
            min_nm: range[0],
            max_nm: range[1],
            step_nm: None,
        },
        default: ParamValue::Int(default),
        group: Some(group.into()),
        context: None,
    }
}

/// The largest given land size or offset, 10 mm.
const MAX_LAND_NM: i64 = 10_000_000;

/// A parameter of a given land pattern (0: estimated).
fn given(key: &str, label: &str, range: [i64; 2], help: &str) -> Param {
    let mut param = length(key, label, range, 0, "lands");
    param.help = Some(help.into());
    param
}

fn count(key: &str, label: &str, min: i64, default: i64) -> Param {
    Param {
        key: key.into(),
        label: label.into(),
        help: None,
        ty: ParamType::Count {
            min,
            max: MAX_PER_SIDE,
        },
        default: ParamValue::Int(default),
        group: Some("terminals".into()),
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
    let mut web = length("mask_web", "Least mask web", [0, MM], 100_000, "lands");
    web.help = Some(
        "The least solder-mask web between two lands; the project's mask rules fill it.".into(),
    );
    web.context = Some("project.mask.web_min".into());
    let mut clearance = length(
        "pad_clearance",
        "Exposed pad clearance",
        [0, MM],
        200_000,
        "lands",
    );
    clearance.help = Some(
        "The least gap between the exposed pad and a perimeter land; the project's land rules fill it."
            .into(),
    );
    clearance.context = Some("project.land.exposed_pad_clearance".into());
    let mut params = vec![
        land::mask_expansion_param(include!("../../standalone-mask-expansion-nm.json")),
        Param {
            key: "sides".into(),
            label: "Terminal sides".into(),
            help: None,
            ty: ParamType::Enum {
                choices: vec![
                    Choice {
                        key: "two".into(),
                        label: "Two (DFN, SON)".into(),
                    },
                    Choice {
                        key: "four".into(),
                        label: "Four (QFN)".into(),
                    },
                ],
            },
            default: ParamValue::Text("four".into()),
            group: Some("terminals".into()),
            context: None,
        },
        count("terminals", "Terminals per long side", 1, 4),
        count(
            "terminals_short",
            "Terminals per short side (0 on two sides)",
            0,
            4,
        ),
        length("pitch", "Pitch", [100 * UM, 3 * MM], 500_000, "terminals"),
        length(
            "terminal_length",
            "Terminal length",
            [50 * UM, 3 * MM],
            400_000,
            "terminals",
        ),
        length(
            "terminal_width",
            "Terminal width",
            [50 * UM, 2 * MM],
            250_000,
            "terminals",
        ),
        length(
            "body_length",
            "Body length (x)",
            [300 * UM, 30 * MM],
            3 * MM,
            "body",
        ),
        length(
            "body_width",
            "Body width (y)",
            [300 * UM, 30 * MM],
            3 * MM,
            "body",
        ),
        length(
            "body_height",
            "Body height",
            [100 * UM, 5 * MM],
            900_000,
            "body",
        ),
        Param {
            key: "exposed_pad".into(),
            label: "Exposed pad".into(),
            help: None,
            ty: ParamType::Bool,
            default: ParamValue::Bool(true),
            group: Some("exposed pad".into()),
            context: None,
        },
        length(
            "pad_length",
            "Exposed pad length (x)",
            [50 * UM, 30 * MM],
            1_700_000,
            "exposed pad",
        ),
        length(
            "pad_width",
            "Exposed pad width (y)",
            [50 * UM, 30 * MM],
            1_700_000,
            "exposed pad",
        ),
        web,
        clearance,
        given("land_width", "Land width", [0, MAX_LAND_NM], "The recommended land pattern's land width; 0 for the estimate from the terminal."),
        given("land_length", "Land length", [0, MAX_LAND_NM], "The recommended land pattern's land length; 0 for the estimate from the terminal."),
        given("land_offset", "Land offset outward", [-MAX_LAND_NM, MAX_LAND_NM], "How far each land's centre lies outward from its terminal's centre, every side alike; negative is inward. With a given width or length the lands are as given: no heel is trimmed for the exposed pad and no corner cleared."),
        Param {
            key: "construction".into(),
            label: "Construction".into(),
            help: Some("Sawn moulding with copper bared at the cut (DFN, QFN); a land grid array of gold lands under a laminate substrate (LGA); or a sawn DFN with an opening in its top over the sensor (open-cavity sensors such as Sensirion's SHT4x).".into()),
            ty: ParamType::Enum {
                choices: vec![
                    Choice {
                        key: "sawn".into(),
                        label: "Sawn moulding".into(),
                    },
                    Choice {
                        key: "lga".into(),
                        label: "Land grid array".into(),
                    },
                    Choice {
                        key: "sensor_window".into(),
                        label: "Sensor opening in the top".into(),
                    },
                ],
            },
            default: ParamValue::Text("sawn".into()),
            group: Some("body".into()),
            context: None,
        },
        Param {
            key: "wettable_flank".into(),
            label: "Wettable flanks".into(),
            help: Some("Sawn with an exposed pad: each terminal's outer end has a plated step solder wets, so the joint can be inspected from the side (automotive QFN and DFN).".into()),
            ty: ParamType::Bool,
            default: ParamValue::Bool(false),
            group: Some("terminals".into()),
            context: None,
        },
        length(
            "pullback",
            "Terminal pull-back",
            [0, MM],
            0,
            "terminals",
        ),
        length(
            "window_diameter",
            "Sensor opening diameter",
            [100 * UM, 5 * MM],
            600_000,
            "body",
        ),
    ];
    params.extend(explicit_params());
    params.push(sparksmith_generator_sdk::marking::param());
    params.push(sparksmith_generator_sdk::marking::height_param());
    params.push(sparksmith_generator_sdk::marking::codes_param());
    let rule = |text: &str, fields: &[&str]| Rule {
        text: text.into(),
        fields: fields.iter().map(|f| (*f).into()).collect(),
    };
    let rules = vec![
        rule(
            "Terminal width < pitch, so terminals do not touch",
            &["terminal_width", "pitch"],
        ),
        rule(
            "A side's terminals fit along its body edge",
            &[
                "terminals",
                "terminals_short",
                "pitch",
                "body_length",
                "body_width",
            ],
        ),
        rule(
            "2 × terminal length < body width (and length on four sides)",
            &["terminal_length", "body_width", "body_length"],
        ),
        rule(
            "The exposed pad lies inside the terminals",
            &["pad_length", "pad_width"],
        ),
    ];
    let offer = PartOffer {
        families: vec!["no_lead".into()],
        scopes: vec![Scope::Component, Scope::Package],
        grammar: vec![Grammar {
            scope: Scope::Component,
            patterns: vec![
                "qfn{count} {body_length:length}x{body_width:length}? {pitch:length}?".into(),
                "dfn{count} {body_length:length}x{body_width:length}? {pitch:length}?".into(),
            ],
            examples: vec!["QFN-16 3x3".into(), "DFN-8 2x2 0.5".into()],
        }],
        instance_fields: Vec::new(),
    };
    Description {
        abi: v1::ABI,
        id: "no-lead".into(),
        version: VERSION,
        title: "No-lead packages (DFN, QFN)".into(),
        licence: "MIT OR Apache-2.0".into(),
        deterministic: true,
        capabilities: vec![Capability {
            name: v1::GENERATE_PART.into(),
            effect: Effect::None,
            title: Some("Make a DFN or QFN part".into()),
            params,
            rules,
            presets,
            part: Some(offer),
        }],
        needs: Vec::new(),
    }
}

/// A list parameter in `group`.
fn list(key: &str, label: &str, help: &str, of: ParamType, max: usize, group: &str) -> Param {
    Param {
        key: key.into(),
        label: label.into(),
        help: Some(help.into()),
        ty: ParamType::List {
            of: alloc::boxed::Box::new(of),
            max_len: u32::try_from(max).unwrap_or(0),
        },
        default: ParamValue::List(Vec::new()),
        group: Some(group.into()),
        context: None,
    }
}

/// The parameters of terminals and lands given one by one.
fn explicit_params() -> Vec<Param> {
    let number = || ParamType::Text {
        max_chars: explicit::MAX_NUMBER_CHARS + 2,
    };
    let coord = || ParamType::Length {
        min_nm: -15 * MM,
        max_nm: 15 * MM,
        step_nm: None,
    };
    let size = || ParamType::Length {
        min_nm: 10 * UM,
        max_nm: 30 * MM,
        step_nm: None,
    };
    let slots = explicit::TERMINAL_SLOTS;
    let most = explicit::MAX_LANDS;
    let t = "terminals given one by one";
    let l = "lands given one by one";
    vec![
        list(
            "terminal_numbers",
            "Number of each terminal rectangle",
            "Terminals given one by one, for packages no row pattern makes (L-shaped corner terminals, pins joined by lugs, pins at their own pitch): each entry is one rectangle of bared leadframe, and rectangles of one number are one terminal. When given, sides, counts, pitch, terminal sizes and the exposed pad are not used; the exposed pad is a terminal like any other.",
            number(),
            slots,
            t,
        ),
        list(
            "terminal_x",
            "Centre x of each terminal rectangle",
            "From the body centre.",
            coord(),
            slots,
            t,
        ),
        list(
            "terminal_y",
            "Centre y of each terminal rectangle",
            "From the body centre.",
            coord(),
            slots,
            t,
        ),
        list(
            "terminal_size_x",
            "Size x of each terminal rectangle",
            "Along x.",
            size(),
            slots,
            t,
        ),
        list(
            "terminal_size_y",
            "Size y of each terminal rectangle",
            "Along y.",
            size(),
            slots,
            t,
        ),
        list(
            "terminal_lift",
            "Lift of each terminal rectangle",
            "Its underside above the seating plane: 0 bares it underneath; more makes a half-etched neck the moulding covers, bared only at the sawn edge. Empty: all 0.",
            ParamType::Length {
                min_nm: 0,
                max_nm: MM,
                step_nm: None,
            },
            slots,
            t,
        ),
        list(
            "given_land_numbers",
            "Number of each given land",
            "The recommended land pattern, land by land; several lands may share a number. \"(N)\" lands terminal N without showing its number (a lug, a via in the exposed pad). Empty: each bared terminal rectangle grown 0.025 mm on every side.",
            number(),
            most,
            l,
        ),
        list(
            "given_land_x",
            "Centre x of each given land",
            "From the body centre.",
            coord(),
            most,
            l,
        ),
        list(
            "given_land_y",
            "Centre y of each given land",
            "From the body centre.",
            coord(),
            most,
            l,
        ),
        list(
            "given_land_size_x",
            "Size x of each given land",
            "Along x; a round land's diameter.",
            size(),
            most,
            l,
        ),
        list(
            "given_land_size_y",
            "Size y of each given land",
            "Along y.",
            size(),
            most,
            l,
        ),
        list(
            "given_land_shapes",
            "Shape of each given land",
            "Empty: all rectangles.",
            ParamType::Enum {
                choices: vec![
                    Choice {
                        key: "rect".into(),
                        label: "Rectangle".into(),
                    },
                    Choice {
                        key: "round".into(),
                        label: "Round".into(),
                    },
                ],
            },
            most,
            l,
        ),
        list(
            "given_land_drill",
            "Plated hole of each given land",
            "0 for a surface land; a round land with a plated hole (a via) otherwise. Empty: all 0.",
            ParamType::Length {
                min_nm: 0,
                max_nm: 5 * MM,
                step_nm: None,
            },
            most,
            l,
        ),
    ]
}

type Point2 = [i64; 2];

/// A no-lead package's values, checked.
struct Dims {
    four: bool,
    /// Terminals per long (±y) side and per short (±x) side (0 on two).
    n: i64,
    m: i64,
    pitch: i64,
    body: [i64; 3],
    terminal_length: i64,
    terminal_width: i64,
    pad: Option<Point2>,
    mask_web: i64,
    mask_expansion: i64,
    pad_clearance: i64,
    /// How far the terminals' outer ends lie in from the body edge.
    pullback: i64,
    construction: Construction,
    wettable: bool,
    window: i64,
    /// The recommended land pattern's land size and outward offset (0:
    /// estimated).
    land_width: i64,
    land_length: i64,
    land_offset: i64,
}

/// How the package is built (the part file it uses).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Construction {
    Sawn,
    Lga,
    SensorWindow,
}

impl Dims {
    /// Perimeter terminals.
    fn leads(&self) -> i64 {
        2 * (self.n + self.m)
    }

    fn body(&self) -> Box2 {
        Box2::centred([0, 0], [self.body[0], self.body[1]])
    }

    /// Lead `k` (from 0) lies on a short (±x) side.
    fn across_x(&self, k: i64) -> bool {
        let (n, m) = (self.n, self.m);
        self.four && ((n..n + m).contains(&k) || (2 * n + m..2 * (n + m)).contains(&k))
    }

    /// Where each lead's terminal centre is, in lead order.
    fn lead_points(&self) -> Vec<Point2> {
        let tl = self.terminal_length;
        // Half the body less half the terminal, across y and across x.
        let reach = [
            div_round(self.body[1] - tl, 2).unwrap_or(0) - self.pullback,
            div_round(self.body[0] - tl, 2).unwrap_or(0) - self.pullback,
        ];
        let along = |i: i64, count: i64| centred(i, count, self.pitch);
        let (n, m) = (self.n, self.m);
        let mut out: Vec<Point2> = (0..n).map(|i| [along(i, n), -reach[0]]).collect();
        out.extend((0..m).map(|i| [reach[1], along(i, m)]));
        out.extend((0..n).rev().map(|i| [along(i, n), reach[0]]));
        out.extend((0..m).rev().map(|i| [-reach[1], along(i, m)]));
        out
    }

    /// The least copper gap between two lands: the mask web plus the
    /// captured mask expansion on both sides.
    fn copper_gap(&self) -> i64 {
        (self.mask_web + 2 * self.mask_expansion).max(0)
    }
}

/// `(i − (n − 1) / 2) × pitch`, to the nearest nanometre.
fn centred(i: i64, n: i64, pitch: i64) -> i64 {
    div_round((2 * i - (n - 1)) * pitch, 2).unwrap_or(0)
}

fn dims(args: &Args<'_>, estimate_lands: bool) -> Result<Dims, Vec<Problem>> {
    let one = |p: Problem| vec![p];
    let four = args.text("sides").map_err(one)? == "four";
    let pad = if args.flag("exposed_pad").map_err(one)? {
        Some([
            args.int("pad_length").map_err(one)?,
            args.int("pad_width").map_err(one)?,
        ])
    } else {
        None
    };
    let d = Dims {
        four,
        n: args.int("terminals").map_err(one)?,
        m: args.int("terminals_short").map_err(one)?,
        pitch: args.int("pitch").map_err(one)?,
        body: [
            args.int("body_length").map_err(one)?,
            args.int("body_width").map_err(one)?,
            args.int("body_height").map_err(one)?,
        ],
        terminal_length: args.int("terminal_length").map_err(one)?,
        terminal_width: args.int("terminal_width").map_err(one)?,
        pad,
        mask_web: args.int("mask_web").map_err(one)?,
        mask_expansion: args.int(v1::BOARD_MASK_EXPANSION).map_err(one)?,
        pad_clearance: args.int("pad_clearance").map_err(one)?,
        pullback: args.int("pullback").map_err(one)?,
        construction: match args.text("construction").map_err(one)? {
            "lga" => Construction::Lga,
            "sensor_window" => Construction::SensorWindow,
            _ => Construction::Sawn,
        },
        wettable: args.flag("wettable_flank").map_err(one)?,
        window: args.int("window_diameter").map_err(one)?,
        land_width: args.int("land_width").map_err(one)?,
        land_length: args.int("land_length").map_err(one)?,
        land_offset: args.int("land_offset").map_err(one)?,
    };
    let mut problems = Vec::new();
    let mut rule = |ok: bool, field: &str, text: String| {
        if !ok {
            problems.push(problem("param.inconsistent", field, text));
        }
    };
    let [bl, bw, _] = d.body;
    let (tl, tw) = (d.terminal_length, d.terminal_width);
    // A row of `count` terminals at the pitch must fit along `edge`.
    let fits = |count: i64, edge: i64| (count - 1) * d.pitch + tw <= edge;
    rule(
        tw < d.pitch,
        "terminal_width",
        format!(
            "Terminals {} mm wide touch at a pitch of {} mm.",
            mm(tw),
            mm(d.pitch)
        ),
    );
    rule(
        fits(d.n, bl),
        "terminals",
        format!(
            "{} terminals at {} mm do not fit along the {} mm body length.",
            d.n,
            mm(d.pitch),
            mm(bl)
        ),
    );
    rule(
        2 * tl < bw,
        "terminal_length",
        format!(
            "Opposite terminals {} mm long meet across the {} mm body width.",
            mm(tl),
            mm(bw)
        ),
    );
    let inner = if four {
        rule(
            d.m >= 1,
            "terminals_short",
            "A four-sided package needs a terminal on each short side.".into(),
        );
        rule(
            fits(d.m, bw),
            "terminals_short",
            format!(
                "{} terminals at {} mm do not fit along the {} mm body width.",
                d.m,
                mm(d.pitch),
                mm(bw)
            ),
        );
        rule(
            2 * tl < bl,
            "terminal_length",
            format!(
                "Opposite terminals {} mm long meet across the {} mm body length.",
                mm(tl),
                mm(bl)
            ),
        );
        [bl - 2 * tl, bw - 2 * tl]
    } else {
        rule(
            d.m == 0,
            "terminals_short",
            "A two-sided package has no short-side terminals (0).".into(),
        );
        [bl, bw - 2 * tl]
    };
    rule(
        2 * (tl + d.pullback) < bw.min(if four { bl } else { bw }),
        "pullback",
        format!(
            "Terminals {} mm long pulled back {} mm meet across the body.",
            mm(tl),
            mm(d.pullback)
        ),
    );
    match d.construction {
        Construction::Sawn => rule(
            !d.wettable || d.pad.is_some(),
            "wettable_flank",
            "Wettable flanks are made for packages with an exposed pad.".into(),
        ),
        Construction::Lga => rule(
            four && d.pad.is_none() && !d.wettable && 4 * LGA_LAND_NM < d.body[2],
            "construction",
            "An LGA has lands on four sides, no exposed pad or wettable flanks, and a body over 0.2 mm tall.".into(),
        ),
        Construction::SensorWindow => rule(
            !four && d.pad.is_none() && !d.wettable && 2 * d.window < bl.min(bw),
            "construction",
            "A sensor opening is made in a two-sided package without an exposed pad or wettable flanks, its diameter under half the body.".into(),
        ),
    }
    if let Some([pl, pw]) = d.pad {
        rule(
            pl < inner[0] && pw < inner[1],
            "pad_length",
            format!(
                "A {} × {} mm exposed pad does not fit inside the terminals ({} × {} mm).",
                mm(pl),
                mm(pw),
                mm(inner[0]),
                mm(inner[1])
            ),
        );
    }
    if estimate_lands && d.land_width == 0 && tw < d.pitch && (d.n > 1 || d.m > 1) {
        rule(
            tw + d.copper_gap() <= d.pitch,
            "mask_web",
            format!(
                "At {} mm pitch a {} mm terminal cannot keep a {} mm mask web with {} mm mask expansion per side. Give a verified land width or choose compatible mask rules.",
                mm(d.pitch),
                mm(tw),
                mm(d.mask_web),
                mm(d.mask_expansion)
            ),
        );
    }
    if problems.is_empty() {
        Ok(d)
    } else {
        Err(problems)
    }
}

/// A length in mm as people write it: at least one decimal, at most as
/// many as it needs (0.5, 3.0, 1.65).
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

/// A length in mm without a trailing `.0` (3, 0.5, 1.65), for names.
fn short_mm(nm: i64) -> String {
    let text = mm(nm);
    match text.strip_suffix(".0") {
        Some(whole) => whole.to_string(),
        None => text,
    }
}

/// The axis a perimeter land runs along: its long one.
fn long_axis(size: Point2) -> usize {
    usize::from(size[1] > size[0])
}

/// Shorten a rectangular land by `2 × half` nm at its heel (towards the
/// centre along `axis`), keeping its outer end in place: the half step is
/// rounded to whole nanometres once, so the trim is even.
fn shorten(pad: &mut Pad, axis: usize, half: i64) {
    let PadShape::Rect { size } = &mut pad.shape else {
        return;
    };
    size[axis] -= 2 * half;
    if pad.at[axis] < 0 {
        pad.at[axis] -= half;
    } else {
        pad.at[axis] += half;
    }
}

/// A rectangle in picometres: `[min x, min y, max x, max y]`.
type Pm = [i128; 4];

/// The copper box of a rectangular pad in picometres.
fn box_pm(pad: &Pad) -> Option<Pm> {
    let PadShape::Rect { size } = pad.shape else {
        return None;
    };
    // 2 × centre ± size in half-nanometres, then × 500 in picometres.
    let lo = |a: usize| i128::from(2 * pad.at[a] - size[a]) * 500;
    let hi = |a: usize| i128::from(2 * pad.at[a] + size[a]) * 500;
    Some([lo(0), lo(1), hi(0), hi(1)])
}

/// The squared distance between two rectangles (0 when they touch).
fn gap_squared(a: &Pm, b: &Pm) -> i128 {
    let dx = (a[0] - b[2]).max(b[0] - a[2]).max(0);
    let dy = (a[1] - b[3]).max(b[1] - a[3]).max(0);
    dx * dx + dy * dy
}

/// The estimated lands, in lead order, then the exposed pad.
fn pads(d: &Dims) -> Vec<Pad> {
    let gap = d.copper_gap();
    let given = d.land_width > 0 || d.land_length > 0;
    let width = if d.land_width > 0 {
        d.land_width
    } else {
        (d.terminal_width + LAND_WIDTH_EXTRA_NM)
            .min(d.pitch - gap)
            .max(d.terminal_width)
    };
    let land_length = if d.land_length > 0 {
        d.land_length
    } else {
        d.terminal_length + LAND_LENGTH_EXTRA_NM
    };
    let mut pads: Vec<Pad> = d
        .lead_points()
        .into_iter()
        .enumerate()
        .map(|(k, terminal)| {
            let k = i64::try_from(k).unwrap_or(0);
            // Outward from the body, across the land's side.
            let at = if d.across_x(k) {
                [
                    terminal[0] + terminal[0].signum() * d.land_offset,
                    terminal[1],
                ]
            } else {
                [
                    terminal[0],
                    terminal[1] + terminal[1].signum() * d.land_offset,
                ]
            };
            let size = if d.across_x(k) {
                [land_length, width]
            } else {
                [width, land_length]
            };
            land(&(k + 1).to_string(), at, size)
        })
        .collect();
    let leads = pads.len();
    if let Some(pad) = d.pad {
        pads.push(land("EP", [0, 0], pad));
        if !given {
            trim_heels(&mut pads[..leads], d, pad, land_length);
        }
    }
    if d.four && !given {
        clear_corners(&mut pads[..leads], gap);
    }
    pads
}

fn land(key: &str, at: Point2, size: Point2) -> Pad {
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

/// Pull back every perimeter land's heel so it keeps the exposed pad
/// clearance from `pad`, by at most half its estimated length.
fn trim_heels(lands: &mut [Pad], d: &Dims, pad: Point2, land_length: i64) {
    // Twice the gap between a land's heel and the pad, across y (and x).
    let gap = |body: i64, pad: i64| body - d.terminal_length - land_length - pad;
    let mut worst = gap(d.body[1], pad[1]);
    if d.four {
        worst = worst.min(gap(d.body[0], pad[0]));
    }
    let trim = (2 * d.pad_clearance - worst).min(land_length);
    if trim <= 0 {
        return;
    }
    for p in lands {
        let PadShape::Rect { size } = p.shape else {
            continue;
        };
        let axis = long_axis(size);
        if 2 * size[axis] <= trim {
            continue;
        }
        // `trim` is twice the trim; half of the trim, rounded.
        shorten(p, axis, div_round(trim, 4).unwrap_or(0));
    }
}

/// Pull back the heels of the lands that meet at a corner of a four-sided
/// package, so their copper keeps `gap` apart: both lands of a pair give
/// up the same length, the least that does it; a pair that cannot be
/// cleared within half of the shorter land is left as it is.
fn clear_corners(lands: &mut [Pad], gap: i64) {
    let g = i128::from(gap) * 1000;
    let g2 = g * g;
    let info: Vec<(usize, Pm, i128)> = lands
        .iter()
        .filter_map(|p| {
            let PadShape::Rect { size } = p.shape else {
                return None;
            };
            let axis = long_axis(size);
            let outward = if p.at[axis] < 0 { -1 } else { 1 };
            Some((axis, box_pm(p)?, outward))
        })
        .collect();
    if info.len() != lands.len() {
        return;
    }
    // The box with its heel pulled back by `by` pm.
    let pulled = |(axis, b, outward): &(usize, Pm, i128), by: i128| {
        let mut b = *b;
        if *outward > 0 {
            b[*axis] += by;
        } else {
            b[*axis + 2] -= by;
        }
        b
    };
    let mut pull = vec![0_i128; lands.len()];
    for (i, a) in info.iter().enumerate() {
        for (j, b) in info.iter().enumerate().skip(i + 1) {
            if a.0 == b.0 || gap_squared(&a.1, &b.1) >= g2 {
                continue;
            }
            let length = |l: &(usize, Pm, i128)| l.1[l.0 + 2] - l.1[l.0];
            let most = length(a).min(length(b)) / 2;
            let clear = |by: i128| gap_squared(&pulled(a, by), &pulled(b, by)) >= g2;
            if !clear(most) {
                continue;
            }
            // The least whole picometre that clears.
            let (mut lo, mut hi) = (0_i128, most);
            while hi - lo > 1 {
                let mid = lo + (hi - lo) / 2;
                if clear(mid) {
                    hi = mid;
                } else {
                    lo = mid;
                }
            }
            if let Some(p) = pull.get_mut(i) {
                *p = (*p).max(hi);
            }
            if let Some(p) = pull.get_mut(j) {
                *p = (*p).max(hi);
            }
        }
    }
    for ((p, (axis, _, _)), by) in lands.iter_mut().zip(&info).zip(pull) {
        if by > 0 {
            // Half the trim, rounded to whole nanometres.
            let half = div_round(i64::try_from(by).unwrap_or(0), 2000).unwrap_or(0);
            shorten(p, *axis, half);
        }
    }
}

/// A rectangle with the first half of the terminals down its left side and
/// the rest up its right side, on the symbol grid, the designator above
/// and the name below (the host's block symbol): every pin one module long.
fn symbol(keys: &[String]) -> Symbol {
    let step = draw::MODULE_NM;
    let count = i64::try_from(keys.len()).unwrap_or(0);
    let left = (count + 1) / 2;
    let rows = left.max(1);
    let top = (rows - 1) / 2 * step;
    let mut d = Drawing::default();
    d.shape(
        Primitive::Rect {
            min: [-2 * step, top - rows * step],
            max: [2 * step, top + step],
        },
        draw::STROKE_NM,
        Fill::Body,
    );
    d.labels(top + step, top - rows * step);
    for (k, key) in keys.iter().enumerate() {
        let k = i64::try_from(k).unwrap_or(0);
        let (at, rot) = if k < left {
            ([-3 * step, top - k * step], RIGHT)
        } else {
            ([3 * step, top - (count - 1 - k) * step], LEFT)
        };
        d.pins.push(SymbolPin {
            terminal: key.clone(),
            at,
            length_nm: step,
            rot_mdeg: rot,
            show_name: true,
            show_number: true,
        });
    }
    Symbol {
        units: vec![d.unit("A")],
    }
}

/// The pin-1 dot's radius on a body whose shorter side is `short`.
fn dot_radius(short: i64) -> i64 {
    let (lo, hi) = (DOT_RADIUS_NM[0], DOT_RADIUS_NM[1]);
    let radius = div_round(short * 6, 100).unwrap_or(lo).clamp(lo, hi);
    // On a body too small for it, the dot shrinks to fit.
    if 6 * radius >= short {
        div_round(short, 8).unwrap_or(0)
    } else {
        radius
    }
}

/// The leadframe (or an LGA's land) thickness under a body `height` tall.
fn frame_of(height: i64, lga: bool) -> i64 {
    let frame = if lga { LGA_LAND_NM } else { FRAME_NM };
    frame.min(div_round(height, 3).unwrap_or(0))
}

/// The top edges' round on a body `height` tall.
fn edge_round(height: i64) -> i64 {
    TOP_EDGE_NM.min(div_round(height, 10).unwrap_or(0))
}

/// The SHA-256 of each part file in the bundle (`parts/`), written by the
/// build script.
mod parts {
    include!(concat!(env!("OUT_DIR"), "/parts.rs"));
}

/// The package's 3D model: one of the bundle's four part files (two or
/// four sides, with or without an exposed pad), every parameter given. A
/// moulded body on the seating plane holds the leadframe's terminals flush
/// with its underside and side faces, one body pattern per side whose
/// count is a parameter, and the exposed pad; the body is cut round them,
/// so terminals and moulding stay disjoint. Its top edges are rounded, and
/// a dimple marks the pin-1 corner.
fn model(d: &Dims) -> v1::Model {
    let [bl, bw, bh] = d.body;
    let radius = dot_radius(bl.min(bw));
    let mut values = v1::Params::new();
    let mut set = |name: &str, v: i64| {
        values.insert(name.into(), ParamValue::Int(v));
    };
    set("terminals", d.n);
    if d.four {
        set("terminals_short", d.m);
    }
    set("pitch", d.pitch);
    set("body_length", bl);
    set("body_width", bw);
    set("body_height", bh);
    set("terminal_length", d.terminal_length);
    set("terminal_width", d.terminal_width);
    set("pullback", d.pullback);
    set("frame", frame_of(bh, d.construction == Construction::Lga));
    set("edge_round", edge_round(bh));
    set("mark_radius", radius);
    set("mark_depth", DOT_DEPTH_NM);
    if let Some([pl, pw]) = d.pad {
        set("pad_length", pl);
        set("pad_width", pw);
    }
    if d.construction == Construction::SensorWindow {
        set("window_diameter", d.window);
    }
    let asset = match (d.construction, d.four, d.pad.is_some(), d.wettable) {
        (Construction::Lga, ..) => parts::LGA,
        (Construction::SensorWindow, ..) => parts::DFN_WINDOW,
        (_, false, false, _) => parts::DFN,
        (_, false, true, false) => parts::DFN_EP,
        (_, false, true, true) => parts::DFN_EP_WETTABLE,
        (_, true, false, _) => parts::QFN,
        (_, true, true, false) => parts::QFN_EP,
        (_, true, true, true) => parts::QFN_EP_WETTABLE,
    };
    v1::Model::PartFile {
        asset: asset.into(),
        values,
    }
}

/// The top marking the parameters ask for, laser-etched, with the preset's
/// code table (`marking_codes`) for a line's `{code}`.
pub(crate) fn marking(args: &Args<'_>) -> Result<Option<v1::Marking>, Problem> {
    let codes = sparksmith_generator_sdk::marking::codes(args)?;
    Ok(sparksmith_generator_sdk::marking::of(args, false)?.map(|m| v1::Marking { codes, ..m }))
}
fn generate(request: &Request, estimate_lands: bool) -> Result<Generation, Vec<Problem>> {
    let description = description_with(Vec::new());
    let args = checked(&description, &request.params)?;
    if explicit::given(&args)? {
        return explicit::generate(request, &args);
    }
    let d = dims(&args, estimate_lands)?;
    let pads = pads(&d);
    let body = d.body();
    let courtyard = land::courtyard(&pads, Some(body)).unwrap_or_default();
    let top = courtyard.iter().map(|p| p[1]).max().unwrap_or(body.max[1]);
    let mut silk = land::silk_outline(&pads, body, d.mask_expansion);
    silk.push(land::silk_designator(body, top));
    if pads.len() > 2
        && let Some(first) = pads.first()
    {
        let width = land::pad_box(first).size()[0];
        silk.push(Graphic {
            key: "pin1".into(),
            shape: Primitive::Circle {
                center: [
                    first.at[0]
                        - div_round(width, 2).unwrap_or(0)
                        - d.mask_expansion
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
    let keys: Vec<String> = pads.iter().map(|p| p.key.clone()).collect();
    let package = Package {
        terminals: keys.clone(),
        geometry: PackageGeometry {
            envelope_nm: d.body,
            seating_nm: 0,
            // Each terminal's centre (its land's heel may be trimmed), and
            // the exposed pad's.
            leads: keys
                .iter()
                .zip(d.lead_points().into_iter().chain(d.pad.map(|_| [0, 0])))
                .map(|(key, at)| Lead {
                    key: key.clone(),
                    contact: [at[0], at[1], 0],
                })
                .collect(),
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
    let family = if d.four { "QFN" } else { "DFN" };
    let pad = d.pad.map_or(String::new(), |[pl, pw]| {
        if pl == pw {
            format!(" EP {}", short_mm(pl))
        } else {
            format!(" EP {}×{}", short_mm(pl), short_mm(pw))
        }
    });
    let name = format!(
        "{family}-{} {}×{} {}{pad}",
        d.leads(),
        short_mm(d.body[0]),
        short_mm(d.body[1]),
        short_mm(d.pitch)
    );
    Ok(Generation {
        orderable: Vec::new(),
        marking: marking(&args).map_err(|p| vec![p])?,
        component: component_part.then(|| Component {
            ties: Default::default(),
            assembly: Default::default(),
            terminals: keys
                .iter()
                .map(|key| Terminal {
                    key: key.clone(),
                    name: key.clone(),
                    direction: Direction::Passive,
                    alternates: Vec::new(),
                    drive: None,
                    unconnected: None,
                })
                .collect(),
            prefix: "U".into(),
            name,
            fields: Vec::new(),
        }),
        symbol: component_part.then(|| symbol(&keys)),
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
            if d.land_width > 0 || d.land_length > 0 || d.land_offset != 0 {
                "Lands are the given land size and offset; confirm them, and the exposed pad, \
             against the device's recommended land pattern."
                    .into()
            } else {
                "Lands are estimated from the terminals; confirm them, and the exposed pad, against \
             the device's recommended land pattern."
                .into()
            },
        ],
    })
}

impl Generator for NoLead {
    fn describe() -> Description {
        description()
    }

    fn generate(request: &Request) -> Result<Generation, Vec<Problem>> {
        generate(request, true)
    }

    fn generate_for_given_lands(request: &Request) -> Result<Generation, Vec<Problem>> {
        generate(request, false)
    }
}

mod explicit;

#[cfg(test)]
mod tests;
