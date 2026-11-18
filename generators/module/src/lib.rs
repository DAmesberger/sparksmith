//! The module generator: radio and processor modules (a small board with a
//! shield can, pad rows along its edges as castellated half-holes or LGA
//! lands, an optional ground-pad array under it, single corner pads and an
//! optional printed antenna at one end), as a symbol, package, footprint
//! and 3D model.
//!
//! Frame (the host's module form's): seen from above, x across the module
//! (`width`), y along it (`length`), the antenna end at +y, the origin at
//! the outline centre. Pads are numbered from 1 through the rows in the
//! order listed; each row runs counter-clockwise seen from above (a left
//! row downwards, a bottom row towards +x, a right row upwards, a top row
//! towards −x) from its first pad, `pitch` apart. Each row pad is
//! `pad_length` across its edge and `pad_width` along it. After the rows
//! comes the ground array as one terminal (`ground_number`, landed by
//! every pad of the array, turned `ground_rotation` about its centre),
//! then each single pad as its own terminal.
//!
//! Lands are the module datasheet's recommended land pattern as the values
//! give it: a row pad's land is `land_width` wide along its edge and the
//! pad grown by `land_outer` and `land_inner` at its outer and inner ends;
//! ground and single pads land at their own size. The antenna keep-out
//! (the antenna region grown by `keepout_left`, `keepout_right` and
//! `keepout_top` beyond the −x, +x and +y edges) is a copper keep-out of
//! the host board on every layer.
//!
//! The 3D model is one of four part files (LGA pads or castellated, the
//! ground array along the axes or turned 45°), every parameter given:
//! eight row slots (two per side),
//! sixteen single-pad slots, a ground array, a shield can and the antenna
//! meander; what a module does not have is placed beyond the −x edge and
//! cut away there, so the bodies left are the module's.
//!
//! The presets are the module form's typical values (an original synthetic
//! module, estimated) and the ESP32-S31-WROOM-3 from Espressif's public
//! datasheet.
#![cfg_attr(target_arch = "wasm32", no_std)]

extern crate alloc;

use alloc::collections::BTreeSet;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use sparksmith_generator_sdk::args::{Args, checked, problem};
use sparksmith_generator_sdk::draw::{self, Drawing, LEFT, RIGHT};
use sparksmith_generator_sdk::land::{self, Box2};
use sparksmith_generator_sdk::units::{DEG, MM, UM, div_round};
use sparksmith_generator_sdk::v1::{
    self, BindingEntry, Capability, Choice, Component, Description, Direction, Effect, Fill,
    Footprint, Generation, Grammar, Graphic, Keepout, Lead, Opening, Package, PackageGeometry, Pad,
    PadShape, Param, ParamType, ParamValue, PartOffer, Preset, Primitive, Problem, Request, Rule,
    Scope, Side, Symbol, SymbolPin, Terminal,
};
use sparksmith_generator_sdk::{Generator, export_generator};

/// The presets, in nm.
const PRESETS: &str = include_str!("../presets.json");

/// The generator's version (`Description::version`).
const VERSION: u32 = 2;

/// Row slots of the part files: two per side.
pub const ROWS_PER_SIDE: usize = 2;
pub const ROW_SLOTS: usize = 4 * ROWS_PER_SIDE;
/// Most pads of one row (the part files' looks are assigned up to it).
pub const MAX_ROW_PADS: i64 = 64;
/// Single-pad slots of the part files.
pub const SINGLE_SLOTS: usize = 16;
/// Most ground pads along either direction of the array.
pub const MAX_GROUND_SIDE: i64 = 16;
/// Most module width or length: what a module lacks is parked 100 mm
/// beyond its −x edge and cut away within 60 mm of there.
const MAX_SIZE_NM: i64 = 100 * MM;
/// Where the parts a module lacks are placed: this far beyond the −x edge.
const PARK_BEYOND_NM: i64 = 100 * MM;
/// Longest pad number.
const MAX_NUMBER_CHARS: u32 = 16;

/// Pad thickness under the board (est.: ENIG over 35 µm copper; the
/// host's `MODULE_PAD_MM`).
const PAD_NM: i64 = 35_000;
/// Castellation plating in the notch (est.: plated copper and finish).
const PLATING_NM: i64 = 30_000;
/// Round of the can's top edges (est.: a drawn can's bend radius), at most
/// a quarter of the can's height and a tenth of its shorter side.
const CAN_EDGE_NM: i64 = 300_000;
/// The antenna meander's defaults (est.): trace width, inset in the
/// antenna region; turns per 10 mm of module width when the turns are not
/// given, at most 64.
const TRACE_WIDTH_NM: i64 = 450_000;
const ANTENNA_MARGIN_NM: i64 = 800_000;
const TURNS_PER_10_MM: i64 = 4;
const MAX_TURNS: i64 = 64;
/// The shield can's stamped emboss (est.): a groove round the label area,
/// this wide and deep, inset by the `can_emboss_inset` parameter. Without
/// the emboss the groove shrinks to a micrometre, too small to see, so one
/// part file serves.
const EMBOSS_WIDTH_NM: i64 = 300_000;
const EMBOSS_DEPTH_NM: i64 = 50_000;

/// The pin-1 dot: a filled silkscreen circle of this radius beyond pad 1's
/// −x edge, clear of its mask opening by the silkscreen clearance (as the
/// host's template marks pin 1 of a package of more than two terminals).
const PIN_ONE_DOT_RADIUS_NM: i64 = 200_000;

/// The fixed point of [`draw::cos_sin`].
const ONE: i128 = 1 << 60;

pub struct Module;

export_generator!(Module);

/// The side a row runs along.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SideOf {
    Left,
    Bottom,
    Right,
    Top,
}

impl SideOf {
    const KEYS: [(&'static str, &'static str, SideOf); 4] = [
        ("left", "Left (−x), numbered downwards", SideOf::Left),
        ("bottom", "Bottom (−y), numbered towards +x", SideOf::Bottom),
        ("right", "Right (+x), numbered upwards", SideOf::Right),
        ("top", "Top (+y), numbered towards −x", SideOf::Top),
    ];

    fn of(key: &str) -> Option<Self> {
        Self::KEYS.iter().find(|k| k.0 == key).map(|k| k.2)
    }

    /// The slot index of the side's first row (left, bottom, right, top).
    fn index(self) -> usize {
        match self {
            Self::Left => 0,
            Self::Bottom => 1,
            Self::Right => 2,
            Self::Top => 3,
        }
    }

    /// The unit step from one pad to the next.
    fn step(self) -> Point2 {
        match self {
            Self::Left => [0, -1],
            Self::Bottom => [1, 0],
            Self::Right => [0, 1],
            Self::Top => [-1, 0],
        }
    }

    /// The outward normal.
    fn outward(self) -> Point2 {
        match self {
            Self::Left => [-1, 0],
            Self::Bottom => [0, -1],
            Self::Right => [1, 0],
            Self::Top => [0, 1],
        }
    }

    fn across_x(self) -> bool {
        matches!(self, Self::Left | Self::Right)
    }
}

type Point2 = [i64; 2];

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

fn count(key: &str, label: &str, range: [i64; 2], default: i64, group: &str) -> Param {
    Param {
        key: key.into(),
        label: label.into(),
        help: None,
        ty: ParamType::Count {
            min: range[0],
            max: range[1],
        },
        default: ParamValue::Int(default),
        group: Some(group.into()),
        context: None,
    }
}

fn flag(key: &str, label: &str, default: bool, group: &str) -> Param {
    Param {
        key: key.into(),
        label: label.into(),
        help: None,
        ty: ParamType::Bool,
        default: ParamValue::Bool(default),
        group: Some(group.into()),
        context: None,
    }
}

fn text(key: &str, label: &str, default: &str, group: &str) -> Param {
    Param {
        key: key.into(),
        label: label.into(),
        help: None,
        ty: ParamType::Text {
            max_chars: MAX_NUMBER_CHARS,
        },
        default: ParamValue::Text(default.into()),
        group: Some(group.into()),
        context: None,
    }
}

fn list(key: &str, label: &str, of: ParamType, max: usize, default: Vec<ParamValue>) -> Param {
    Param {
        key: key.into(),
        label: label.into(),
        help: None,
        ty: ParamType::List {
            of: alloc::boxed::Box::new(of),
            max_len: u32::try_from(max).unwrap_or(0),
        },
        default: ParamValue::List(default),
        group: Some(
            if key.starts_with("row") {
                "rows"
            } else {
                "single pads"
            }
            .into(),
        ),
        context: None,
    }
}

fn ints(v: &[i64]) -> Vec<ParamValue> {
    v.iter().map(|x| ParamValue::Int(*x)).collect()
}

fn texts(v: &[&str]) -> Vec<ParamValue> {
    v.iter().map(|x| ParamValue::Text((*x).into())).collect()
}

/// A length within a module, either sign.
const COORD: [i64; 2] = [-MAX_SIZE_NM / 2, MAX_SIZE_NM / 2];

/// The full description. Unreadable presets are a build bug that the
/// golden and preset tests catch; describe still answers, without them.
fn description() -> Description {
    description_with(sparksmith_generator_sdk::presets(PRESETS).unwrap_or_default())
}

/// The description, with `presets`. The defaults are the host module
/// form's typical values: an original synthetic 18 × 25 mm castellated
/// module, 14 + 10 + 14 pads at 1.27 mm, a 3 × 3 ground array, a 6 mm
/// antenna end.
fn description_with(presets: Vec<Preset>) -> Description {
    let coord = ParamType::Length {
        min_nm: COORD[0],
        max_nm: COORD[1],
        step_nm: None,
    };
    let size = ParamType::Length {
        min_nm: 10 * UM,
        max_nm: 20 * MM,
        step_nm: None,
    };
    let sides = ParamType::Enum {
        choices: SideOf::KEYS
            .iter()
            .map(|(key, label, _)| Choice {
                key: (*key).into(),
                label: (*label).into(),
            })
            .collect(),
    };
    let mut row_sides = list(
        "row_sides",
        "Side of each pad row",
        sides,
        ROW_SLOTS,
        texts(&["left", "bottom", "right"]),
    );
    row_sides.help = Some(
        "One entry per row, in numbering order; at most two rows on a side. Row counts and first pad centres are listed in the same order."
            .into(),
    );
    let mut ground_rotation = Param {
        key: "ground_rotation".into(),
        label: "Ground array turn".into(),
        help: Some("Counter-clockwise, a multiple of 45°.".into()),
        ty: ParamType::Angle {
            min_mdeg: -360 * DEG,
            max_mdeg: 360 * DEG,
        },
        default: ParamValue::Int(0),
        group: Some("ground".into()),
        context: None,
    };
    ground_rotation.help =
        Some("Counter-clockwise about the array's centre, a multiple of 45°.".into());
    let params = vec![
        length(
            "width",
            "Module width (x)",
            [2 * MM, MAX_SIZE_NM],
            18 * MM,
            "outline",
        ),
        length(
            "length",
            "Module length (y, antenna end at +y)",
            [2 * MM, MAX_SIZE_NM],
            25 * MM,
            "outline",
        ),
        length(
            "height",
            "Seated height (board and can)",
            [200 * UM, 20 * MM],
            3_200_000,
            "outline",
        ),
        length(
            "board_thickness",
            "Board thickness",
            [100 * UM, 5 * MM],
            800_000,
            "outline",
        ),
        length(
            "antenna_length",
            "Antenna region depth from the +y edge (0: none)",
            [0, MAX_SIZE_NM],
            6 * MM,
            "outline",
        ),
        length(
            "antenna_trace",
            "Antenna trace width",
            [100 * UM, 2 * MM],
            TRACE_WIDTH_NM,
            "antenna",
        ),
        length(
            "antenna_margin",
            "Antenna meander inset in its region",
            [0, 10 * MM],
            ANTENNA_MARGIN_NM,
            "antenna",
        ),
        count(
            "antenna_turns",
            "Antenna meander turns (0: four per 10 mm of width)",
            [0, MAX_TURNS],
            0,
            "antenna",
        ),
        flag("can_emboss", "Emboss groove round the can's label area", true, "can"),
        length(
            "can_emboss_inset",
            "Emboss groove inset from the can's sides",
            [100 * UM, 20 * MM],
            MM,
            "can",
        ),
        length(
            "can_width",
            "Shield can width (x)",
            [100 * UM, MAX_SIZE_NM],
            16_600_000,
            "can",
        ),
        length(
            "can_length",
            "Shield can length (y)",
            [100 * UM, MAX_SIZE_NM],
            17_400_000,
            "can",
        ),
        length("can_x", "Shield can centre x", COORD, 0, "can"),
        length("can_y", "Shield can centre y", COORD, -2_500_000, "can"),
        length(
            "pitch",
            "Pad pitch within a row",
            [100 * UM, 10 * MM],
            1_270_000,
            "rows",
        ),
        length(
            "pad_length",
            "Row pad across its edge",
            [50 * UM, 10 * MM],
            1_200_000,
            "rows",
        ),
        length(
            "pad_width",
            "Row pad along its edge",
            [50 * UM, 10 * MM],
            900_000,
            "rows",
        ),
        length(
            "castellation",
            "Castellation diameter (0: LGA pads)",
            [0, 5 * MM],
            600_000,
            "rows",
        ),
        length(
            "land_width",
            "Row land width along its edge",
            [50 * UM, 10 * MM],
            900_000,
            "rows",
        ),
        length(
            "land_outer",
            "Row land growth beyond the pad's outer end",
            [-5 * MM, 5 * MM],
            300_000,
            "rows",
        ),
        length(
            "land_inner",
            "Row land growth beyond the pad's inner end",
            [-5 * MM, 5 * MM],
            0,
            "rows",
        ),
        row_sides,
        list(
            "row_counts",
            "Pads of each row",
            ParamType::Count {
                min: 1,
                max: MAX_ROW_PADS,
            },
            ROW_SLOTS,
            ints(&[14, 10, 14]),
        ),
        list(
            "row_x",
            "First pad centre x of each row",
            coord.clone(),
            ROW_SLOTS,
            ints(&[-8_400_000, -5_715_000, 8_400_000]),
        ),
        list(
            "row_y",
            "First pad centre y of each row",
            coord.clone(),
            ROW_SLOTS,
            ints(&[5_500_000, -11_900_000, -11_010_000]),
        ),
        flag("ground", "Ground-pad array", true, "ground"),
        count(
            "ground_rows",
            "Ground array rows",
            [1, MAX_GROUND_SIDE],
            3,
            "ground",
        ),
        count(
            "ground_columns",
            "Ground array columns",
            [1, MAX_GROUND_SIDE],
            3,
            "ground",
        ),
        length(
            "ground_pitch_x",
            "Ground pitch along the array's x",
            [50 * UM, 20 * MM],
            1_600_000,
            "ground",
        ),
        length(
            "ground_pitch_y",
            "Ground pitch along the array's y",
            [50 * UM, 20 * MM],
            1_600_000,
            "ground",
        ),
        length(
            "ground_size_x",
            "Ground pad size along the array's x",
            [50 * UM, 20 * MM],
            1_100_000,
            "ground",
        ),
        length(
            "ground_size_y",
            "Ground pad size along the array's y",
            [50 * UM, 20 * MM],
            1_100_000,
            "ground",
        ),
        length("ground_x", "Ground array centre x", COORD, 0, "ground"),
        length(
            "ground_y",
            "Ground array centre y",
            COORD,
            -2 * MM,
            "ground",
        ),
        ground_rotation,
        text("ground_number", "Ground terminal number", "39", "ground"),
        list(
            "single_numbers",
            "Number of each single pad",
            ParamType::Text {
                max_chars: MAX_NUMBER_CHARS,
            },
            SINGLE_SLOTS,
            Vec::new(),
        ),
        list(
            "single_x",
            "Centre x of each single pad",
            coord.clone(),
            SINGLE_SLOTS,
            Vec::new(),
        ),
        list(
            "single_y",
            "Centre y of each single pad",
            coord,
            SINGLE_SLOTS,
            Vec::new(),
        ),
        list(
            "single_length",
            "Size x of each single pad",
            size.clone(),
            SINGLE_SLOTS,
            Vec::new(),
        ),
        list(
            "single_width",
            "Size y of each single pad",
            size,
            SINGLE_SLOTS,
            Vec::new(),
        ),
        flag(
            "keepout",
            "Antenna keep-out on the host board",
            true,
            "keep-out",
        ),
        length(
            "keepout_left",
            "Keep-out beyond the −x edge",
            [0, 50 * MM],
            0,
            "keep-out",
        ),
        length(
            "keepout_right",
            "Keep-out beyond the +x edge",
            [0, 50 * MM],
            0,
            "keep-out",
        ),
        length(
            "keepout_top",
            "Keep-out beyond the +y edge",
            [0, 50 * MM],
            0,
            "keep-out",
        ),
    ];
    let rule = |text: &str, fields: &[&str]| Rule {
        text: text.into(),
        fields: fields.iter().map(|f| (*f).into()).collect(),
    };
    let rules = vec![
        rule(
            "Every row pad lies within the outline; at most two rows on a side",
            &["row_sides", "row_counts", "row_x", "row_y"],
        ),
        rule(
            "Row pads and lands are narrower than the pitch; a castellation narrower than its pad",
            &["pad_width", "land_width", "pitch", "castellation"],
        ),
        rule(
            "The shield can lies within the module, clear of the antenna region, and the height leaves room for it",
            &[
                "can_width",
                "can_length",
                "can_x",
                "can_y",
                "height",
                "board_thickness",
            ],
        ),
        rule(
            "Ground pads do not touch and lie within the module; the turn is a multiple of 45°",
            &["ground_pitch_x", "ground_size_x", "ground_rotation"],
        ),
        rule(
            "Terminal numbers are unique (rows count from 1)",
            &["ground_number", "single_numbers"],
        ),
    ];
    let offer = PartOffer {
        families: vec!["module".into()],
        scopes: vec![Scope::Component, Scope::Package],
        grammar: vec![Grammar {
            scope: Scope::Component,
            patterns: vec!["module {width:length}x{length:length}?".into()],
            examples: vec!["module 18x25".into()],
        }],
        instance_fields: Vec::new(),
    };
    Description {
        abi: v1::ABI,
        id: "module".into(),
        version: VERSION,
        title: "Radio and processor modules (castellated or LGA)".into(),
        licence: "MIT OR Apache-2.0".into(),
        deterministic: true,
        capabilities: vec![Capability {
            name: v1::GENERATE_PART.into(),
            effect: Effect::None,
            title: Some("Make a radio or processor module part".into()),
            params,
            rules,
            presets,
            part: Some(offer),
        }],
        needs: Vec::new(),
    }
}

/// One pad row, checked.
#[derive(Clone, Copy, Debug)]
struct Row {
    side: SideOf,
    count: i64,
    first: Point2,
}

/// One single pad, checked.
#[derive(Clone, Debug)]
struct Single {
    number: String,
    at: Point2,
    size: Point2,
}

/// The ground array, checked.
#[derive(Clone, Debug)]
struct Ground {
    rows: i64,
    cols: i64,
    pitch: Point2,
    size: Point2,
    centre: Point2,
    rotation: i64,
    number: String,
}

/// A module's values, checked.
struct Dims {
    width: i64,
    length: i64,
    height: i64,
    board: i64,
    antenna: i64,
    /// The meander's trace width and inset, and its turns (0: from the
    /// width).
    trace: i64,
    margin: i64,
    turns: i64,
    /// The emboss groove's inset from the can's sides, when it has one.
    emboss: Option<i64>,
    can: [i64; 2],
    can_at: Point2,
    pitch: i64,
    pad: Point2,
    castellation: i64,
    land_width: i64,
    land_extend: Point2,
    rows: Vec<Row>,
    ground: Option<Ground>,
    singles: Vec<Single>,
    /// Growth beyond −x, +x, +y; `None` without a keep-out.
    keepout: Option<[i64; 3]>,
}

impl Dims {
    fn half(&self) -> Point2 {
        [half(self.width), half(self.length)]
    }

    /// Each row pad's row and centre, in numbering order.
    fn row_centres(&self) -> Vec<(Row, Point2)> {
        let mut out = Vec::new();
        for row in &self.rows {
            let d = row.side.step();
            for k in 0..row.count {
                let t = k * self.pitch;
                out.push((*row, [row.first[0] + d[0] * t, row.first[1] + d[1] * t]));
            }
        }
        out
    }

    /// A row pad's size in the module frame (x, y).
    fn row_pad_size(&self, side: SideOf) -> Point2 {
        if side.across_x() {
            self.pad
        } else {
            [self.pad[1], self.pad[0]]
        }
    }

    /// Whether a row pad at `centre` on `side` reaches the outline within
    /// 0.05 mm (the host's rule for a castellation).
    fn at_edge(&self, side: SideOf, centre: Point2) -> bool {
        // Twice the reach, in nm: 2 × (centre outward + half the pad −
        // half the outline) ≥ −0.1 mm.
        let out = side.outward();
        let along = centre[0] * out[0] + centre[1] * out[1];
        let half_outline = if side.across_x() {
            self.width
        } else {
            self.length
        };
        2 * along + self.pad[0] - half_outline >= -100_000
    }

    /// Terminal numbers in lead order.
    fn designations(&self) -> Vec<String> {
        let rows: i64 = self.rows.iter().map(|r| r.count).sum();
        let mut out: Vec<String> = (1..=rows).map(|k| k.to_string()).collect();
        if let Some(g) = &self.ground {
            out.push(g.number.clone());
        }
        out.extend(self.singles.iter().map(|s| s.number.clone()));
        out
    }

    /// The ground pad centres, row by row from the array's +y, columns from
    /// its −x, turned about the centre; rounded to the nm.
    fn ground_centres(&self) -> Vec<Point2> {
        let Some(g) = &self.ground else {
            return Vec::new();
        };
        let (c, s) = draw::cos_sin(g.rotation);
        let mut out = Vec::new();
        for r in 0..g.rows {
            for col in 0..g.cols {
                // Twice the offsets from the centre, exact in nm.
                let u2 = i128::from((2 * col - (g.cols - 1)) * g.pitch[0]);
                let v2 = -i128::from((2 * r - (g.rows - 1)) * g.pitch[1]);
                let x = scaled(c * u2 - s * v2);
                let y = scaled(s * u2 + c * v2);
                out.push([g.centre[0] + x, g.centre[1] + y]);
            }
        }
        out
    }
}

/// `v / (2 × ONE)` rounded to the nearest nm, halves away from zero.
fn scaled(v: i128) -> i64 {
    let d = 2 * ONE;
    let q = if v >= 0 { (v + ONE) / d } else { (v - ONE) / d };
    i64::try_from(q).unwrap_or(0)
}

fn half(v: i64) -> i64 {
    div_round(v, 2).unwrap_or(0)
}

/// The integers of a list parameter (empty when it is not one).
fn int_list(request: &Request, key: &str) -> Vec<i64> {
    match request.params.get(key) {
        Some(ParamValue::List(items)) => items
            .iter()
            .filter_map(|v| match v {
                ParamValue::Int(i) => Some(*i),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// The texts of a list parameter (empty when it is not one).
fn text_list(request: &Request, key: &str) -> Vec<String> {
    match request.params.get(key) {
        Some(ParamValue::List(items)) => items
            .iter()
            .filter_map(|v| match v {
                ParamValue::Text(t) => Some(t.trim().to_string()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// A length in mm as people write it.
fn mm(nm: i64) -> String {
    sparksmith_generator_sdk::units::mm_text(nm)
}

/// A pad number: 1–16 printable characters, no space.
fn valid_number(n: &str) -> bool {
    !n.is_empty()
        && n.chars().count() <= MAX_NUMBER_CHARS as usize
        && n.chars().all(|c| c.is_ascii_graphic())
}

fn dims(args: &Args<'_>, request: &Request) -> Result<Dims, Vec<Problem>> {
    let one = |p: Problem| vec![p];
    let int = |key: &str| args.int(key).map_err(one);
    let mut problems = Vec::new();
    let mut rule = |ok: bool, field: &str, text: String| {
        if !ok {
            problems.push(problem("param.inconsistent", field, text));
        }
    };
    // Rows: parallel lists.
    let sides = text_list(request, "row_sides");
    let counts = int_list(request, "row_counts");
    let xs = int_list(request, "row_x");
    let ys = int_list(request, "row_y");
    let n = sides.len();
    rule(
        n >= 1 && counts.len() == n && xs.len() == n && ys.len() == n,
        "row_sides",
        format!(
            "A module has 1–{ROW_SLOTS} pad rows, each with a side, a count and its first pad's x and y ({n} sides, {} counts, {} x, {} y).",
            counts.len(),
            xs.len(),
            ys.len()
        ),
    );
    let mut rows = Vec::new();
    for (k, side) in sides.iter().enumerate() {
        let (Some(side), Some(count), Some(x), Some(y)) =
            (SideOf::of(side), counts.get(k), xs.get(k), ys.get(k))
        else {
            continue;
        };
        rows.push(Row {
            side,
            count: *count,
            first: [*x, *y],
        });
    }
    for side in SideOf::KEYS.iter().map(|k| k.2) {
        let on = rows.iter().filter(|r| r.side == side).count();
        rule(
            on <= ROWS_PER_SIDE,
            "row_sides",
            format!(
                "{on} rows run along one side; the module's part files hold {ROWS_PER_SIDE} a side."
            ),
        );
    }
    let ground = if args.flag("ground").map_err(one)? {
        Some(Ground {
            rows: int("ground_rows")?,
            cols: int("ground_columns")?,
            pitch: [int("ground_pitch_x")?, int("ground_pitch_y")?],
            size: [int("ground_size_x")?, int("ground_size_y")?],
            centre: [int("ground_x")?, int("ground_y")?],
            rotation: int("ground_rotation")?,
            number: args.text("ground_number").map_err(one)?.trim().to_string(),
        })
    } else {
        None
    };
    let numbers = text_list(request, "single_numbers");
    let sx = int_list(request, "single_x");
    let sy = int_list(request, "single_y");
    let sl = int_list(request, "single_length");
    let sw = int_list(request, "single_width");
    let m = numbers.len();
    rule(
        sx.len() == m && sy.len() == m && sl.len() == m && sw.len() == m,
        "single_numbers",
        format!(
            "Each single pad needs a number, a centre x and y and a size x and y ({m} numbers, {} x, {} y, {} sizes x, {} sizes y).",
            sx.len(),
            sy.len(),
            sl.len(),
            sw.len()
        ),
    );
    let singles: Vec<Single> = numbers
        .iter()
        .enumerate()
        .filter_map(|(k, number)| {
            Some(Single {
                number: number.clone(),
                at: [*sx.get(k)?, *sy.get(k)?],
                size: [*sl.get(k)?, *sw.get(k)?],
            })
        })
        .collect();
    let keepout = if args.flag("keepout").map_err(one)? {
        Some([
            int("keepout_left")?,
            int("keepout_right")?,
            int("keepout_top")?,
        ])
    } else {
        None
    };
    let d = Dims {
        width: int("width")?,
        length: int("length")?,
        height: int("height")?,
        board: int("board_thickness")?,
        antenna: int("antenna_length")?,
        trace: int("antenna_trace")?,
        margin: int("antenna_margin")?,
        turns: int("antenna_turns")?,
        emboss: if args.flag("can_emboss").map_err(one)? {
            Some(int("can_emboss_inset")?)
        } else {
            None
        },
        can: [int("can_width")?, int("can_length")?],
        can_at: [int("can_x")?, int("can_y")?],
        pitch: int("pitch")?,
        pad: [int("pad_length")?, int("pad_width")?],
        castellation: int("castellation")?,
        land_width: int("land_width")?,
        land_extend: [int("land_outer")?, int("land_inner")?],
        rows,
        ground,
        singles,
        keepout,
    };
    let [hw, hl] = [d.width, d.length];
    // Inside the outline, in doubled nm: 2·(c − s/2) ≥ −W, 2·(c + s/2) ≤ W.
    let inside = |c: Point2, s: Point2| {
        2 * c[0] - s[0] >= -hw
            && 2 * c[0] + s[0] <= hw
            && 2 * c[1] - s[1] >= -hl
            && 2 * c[1] + s[1] <= hl
    };
    rule(
        d.antenna < d.length,
        "antenna_length",
        format!(
            "The antenna region ({}) must be shorter than the module ({}).",
            mm(d.antenna),
            mm(d.length)
        ),
    );
    rule(
        d.castellation < d.pad[1],
        "castellation",
        format!(
            "The castellation ({}) is 0 (LGA pads) or narrower than its pad ({}).",
            mm(d.castellation),
            mm(d.pad[1])
        ),
    );
    rule(
        d.height > d.board + PAD_NM,
        "height",
        format!(
            "The height ({}) must leave room for the can above the board ({}) and its pads.",
            mm(d.height),
            mm(d.board)
        ),
    );
    rule(
        d.pad[1] < d.pitch && d.land_width < d.pitch,
        "pitch",
        format!(
            "Row pads ({}) and lands ({}) must be narrower than the pitch ({}).",
            mm(d.pad[1]),
            mm(d.land_width),
            mm(d.pitch)
        ),
    );
    rule(
        d.land_extend.iter().all(|v| 2 * v > -d.pad[0]),
        "land_outer",
        "A land may shrink by less than half the pad length at each end.".into(),
    );
    let total: i64 = d.rows.iter().map(|r| r.count).sum();
    rule(
        total <= 512,
        "row_counts",
        format!("The module has {total} row pads; a package has at most 512."),
    );
    rule(
        d.row_centres()
            .iter()
            .all(|(row, c)| inside(*c, d.row_pad_size(row.side))),
        "row_x",
        "Every row pad must lie within the module outline.".into(),
    );
    let [cw, cl] = d.can;
    let cy = d.can_at[1];
    rule(
        inside(d.can_at, d.can) && 2 * cy + cl <= hl - 2 * d.antenna && cw <= d.width,
        "can_y",
        "The shield can must lie within the module, clear of the antenna region.".into(),
    );
    if let Some(inset) = d.emboss {
        rule(
            2 * (inset + EMBOSS_WIDTH_NM) + MM <= cw.min(cl),
            "can_emboss_inset",
            "The emboss groove must leave at least 1 mm of label area inside the can \
             (2 × (inset + 0.3 mm) + 1 mm ≤ the can's shorter side)."
                .into(),
        );
    }
    if let Some(g) = &d.ground {
        rule(
            g.rows * g.cols <= 256,
            "ground_rows",
            "A ground array has at most 256 pads.".into(),
        );
        rule(
            (g.cols < 2 || g.size[0] < g.pitch[0]) && (g.rows < 2 || g.size[1] < g.pitch[1]),
            "ground_size_x",
            "Ground pads must not touch (size < pitch).".into(),
        );
        rule(
            g.rotation % (45 * DEG) == 0,
            "ground_rotation",
            format!(
                "The ground array is turned {}°; the module's part files turn it by a multiple of 45°.",
                g.rotation / DEG
            ),
        );
        // Within the module as the host checks it: each pad's
        // circumscribed square (its diagonal, rounded up).
        let diagonal = isqrt(g.size[0] * g.size[0] + g.size[1] * g.size[1]);
        rule(
            d.ground_centres()
                .iter()
                .all(|c| inside(*c, [diagonal, diagonal])),
            "ground_x",
            "The ground array must lie within the module.".into(),
        );
        rule(
            valid_number(&g.number),
            "ground_number",
            format!(
                "The ground terminal needs a number of 1–{MAX_NUMBER_CHARS} printable characters."
            ),
        );
    }
    rule(
        d.singles.len() <= SINGLE_SLOTS
            && d.singles
                .iter()
                .all(|p| valid_number(&p.number) && inside(p.at, p.size)),
        "single_numbers",
        format!(
            "Single pads (at most {SINGLE_SLOTS}) need a number and a place within the module."
        ),
    );
    let numbers = d.designations();
    let unique: BTreeSet<&String> = numbers.iter().collect();
    rule(
        unique.len() == numbers.len(),
        "ground_number",
        "Terminal numbers must be unique (rows count from 1).".into(),
    );
    if problems.is_empty() {
        Ok(d)
    } else {
        Err(problems)
    }
}

/// ⌈√v⌉ for v ≥ 0.
fn isqrt(v: i64) -> i64 {
    if v <= 0 {
        return 0;
    }
    let mut x = v;
    let mut y = (x + 1) / 2;
    while y < x {
        x = y;
        y = (x + v / x) / 2;
    }
    if x * x < v { x + 1 } else { x }
}

fn rect(
    key: String,
    terminal: &str,
    at: Point2,
    size: Point2,
    rot: i64,
    number: Option<String>,
) -> Pad {
    Pad {
        key,
        terminal: terminal.into(),
        shape: PadShape::Rect { size },
        at,
        rot_mdeg: rot,
        side: Side::Top,
        hole: None,
        mask: Opening::Default,
        paste: Opening::Default,
        number,
    }
}

/// The lands, in lead order: each row pad's land (`land_width` wide, the
/// pad grown at its ends), every ground pad (turned with the array)
/// landing the ground terminal, then the single pads at their size.
fn pads(d: &Dims) -> Vec<Pad> {
    let [outer, inner] = d.land_extend;
    let across = d.pad[0] + outer + inner;
    let mut out = Vec::new();
    for (k, (row, c)) in d.row_centres().into_iter().enumerate() {
        let n = row.side.outward();
        // The land's centre moves out by (outer − inner) / 2.
        let shift = |c: i64, n: i64| c + div_round(n * (outer - inner), 2).unwrap_or(0);
        let at = [shift(c[0], n[0]), shift(c[1], n[1])];
        let size = if row.side.across_x() {
            [across, d.land_width]
        } else {
            [d.land_width, across]
        };
        let number = (k + 1).to_string();
        out.push(rect(number.clone(), &number, at, size, 0, None));
    }
    if let Some(g) = &d.ground {
        let centres = d.ground_centres();
        let several = centres.len() > 1;
        for (k, c) in centres.into_iter().enumerate() {
            let key = if several {
                format!("{}#{}", g.number, k + 1)
            } else {
                g.number.clone()
            };
            let number = several.then(|| g.number.clone());
            out.push(rect(key, &g.number, c, g.size, g.rotation, number));
        }
    }
    for p in &d.singles {
        out.push(rect(p.number.clone(), &p.number, p.at, p.size, 0, None));
    }
    out
}

/// The lead contacts, one per terminal: a row pad's centre, or on a
/// castellated pad half a castellation radius in from the edge (under the
/// pad and its plating); the first ground pad's centre; each single pad's.
fn leads(d: &Dims) -> Vec<Lead> {
    let r = d.castellation / 2;
    let mut out = Vec::new();
    for (k, (row, c)) in d.row_centres().into_iter().enumerate() {
        let at = if r > 0 && d.at_edge(row.side, c) {
            let n = row.side.outward();
            let [hw, hl] = d.half();
            let edge = if row.side.across_x() { hw } else { hl };
            let inward = edge - half(r);
            if row.side.across_x() {
                [n[0] * inward, c[1]]
            } else {
                [c[0], n[1] * inward]
            }
        } else {
            c
        };
        out.push(Lead {
            key: (k + 1).to_string(),
            contact: [at[0], at[1], 0],
        });
    }
    if let (Some(g), Some(first)) = (&d.ground, d.ground_centres().first()) {
        out.push(Lead {
            key: g.number.clone(),
            contact: [first[0], first[1], 0],
        });
    }
    for p in &d.singles {
        out.push(Lead {
            key: p.number.clone(),
            contact: [p.at[0], p.at[1], 0],
        });
    }
    out
}

/// The host-board keep-out over the antenna, when the module has both.
fn keepouts(d: &Dims) -> Vec<Keepout> {
    let Some([left, right, top]) = d.keepout else {
        return Vec::new();
    };
    if d.antenna <= 0 {
        return Vec::new();
    }
    let [hw, hl] = d.half();
    let min = [-hw - left, hl - d.antenna];
    let max = [hw + right, hl + top];
    vec![Keepout {
        name: "antenna".into(),
        outline: vec![min, [max[0], min[1]], max, [min[0], max[1]]],
    }]
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

/// The SHA-256 of each part file in the bundle (`parts/`), written by the
/// build script.
mod parts {
    include!(concat!(env!("OUT_DIR"), "/parts.rs"));
}

/// The antenna meander as the host's module builder drew it: `None`
/// without one; else (turns, step).
fn meander(d: &Dims) -> Option<(i64, i64)> {
    if d.antenna <= 2 * d.margin + d.trace {
        return None;
    }
    let span = d.width - 2 * d.margin;
    let turns = if d.turns > 0 {
        d.turns
    } else {
        (span * TURNS_PER_10_MM / (10 * MM)).clamp(1, MAX_TURNS)
    };
    // The step is span / (2 × turns), at least two trace widths.
    if span < 2 * turns * 2 * d.trace {
        return None;
    }
    // Every bend needs two bend radii of straight run on each side: the
    // runs are the step and the legs, 0.6 of the region less its margins.
    // The centreline's bend radius is 1.0001 half trace widths.
    let two_radii = div_round(d.trace * 10_001, 10_000)?;
    let rise = d.antenna - 2 * d.margin;
    if two_radii * 2 * turns > span || 5 * two_radii > 3 * rise {
        return None;
    }
    Some((turns, div_round(span, 2 * turns)?))
}

/// The package's 3D model: the bundle's part file with the ground array
/// along the axes or turned 45°, every parameter given (see the crate
/// documentation).
fn model(d: &Dims) -> v1::Model {
    let [hw, hl] = d.half();
    let park = -hw - PARK_BEYOND_NM;
    let mut values = v1::Params::new();
    let mut set = |name: &str, v: i64| {
        values.insert(name.into(), ParamValue::Int(v));
    };
    set("width", d.width);
    set("length", d.length);
    set("height", d.height);
    set("board_thickness", d.board);
    set("can_width", d.can[0]);
    set("can_length", d.can[1]);
    set("can_x", d.can_at[0]);
    set("can_y", d.can_at[1]);
    let z_top = PAD_NM + d.board;
    set(
        "can_round",
        CAN_EDGE_NM
            .min(div_round(d.height - z_top, 4).unwrap_or(0))
            .min(div_round(d.can[0].min(d.can[1]), 10).unwrap_or(0)),
    );
    set("pitch", d.pitch);
    set("pad_length", d.pad[0]);
    set("pad_width", d.pad[1]);
    let r = half(d.castellation);
    // Castellated (the castellated files): a castellation diameter and a
    // row pad meeting the outline, as the builder notched the board.
    let castellated = r > 0
        && d.row_centres()
            .iter()
            .any(|(row, c)| d.at_edge(row.side, *c));
    if castellated {
        set("notch_radius", r);
        set("plating", PLATING_NM);
    }
    set("park_x", park);
    set("antenna_length", d.antenna);
    set("antenna_depth", d.antenna);
    set("antenna_trace", d.trace);
    set("antenna_margin", d.margin);
    match d.emboss {
        Some(inset) => {
            set("can_emboss_inset", inset);
            set("can_emboss_width", EMBOSS_WIDTH_NM);
            set("can_emboss_depth", EMBOSS_DEPTH_NM);
        }
        None => {
            set("can_emboss_inset", CAN_EDGE_NM + MM);
            set("can_emboss_width", UM);
            set("can_emboss_depth", UM);
        }
    }
    // Rows into their slots: row k keeps slot k when that slot is on its
    // side (so its bodies keep the builder's names), else the first free
    // slot of its side.
    let mut slots: [Option<Row>; ROW_SLOTS] = [None; ROW_SLOTS];
    let mut later = Vec::new();
    for (k, row) in d.rows.iter().enumerate() {
        if k < ROW_SLOTS && k % 4 == row.side.index() && slots[k].is_none() {
            slots[k] = Some(*row);
        } else {
            later.push(*row);
        }
    }
    for row in later {
        let slot = (0..ROWS_PER_SIDE)
            .map(|n| row.side.index() + 4 * n)
            .find(|&i| slots.get(i).is_some_and(Option::is_none));
        if let Some(i) = slot
            && let Some(s) = slots.get_mut(i)
        {
            *s = Some(row);
        }
    }
    for (i, slot) in slots.iter().enumerate() {
        let k = i + 1;
        let (count, first, notches, notch) = match slot {
            Some(row) => {
                let edge = castellated && d.at_edge(row.side, row.first);
                let n = row.side.outward();
                let at = if row.side.across_x() {
                    [n[0] * hw, row.first[1]]
                } else {
                    [row.first[0], n[1] * hl]
                };
                if edge {
                    (row.count, row.first, row.count, at)
                } else {
                    (row.count, row.first, 1, [park, 0])
                }
            }
            None => (1, [park, 0], 1, [park, 0]),
        };
        set(&format!("row_{k}_count"), count);
        set(&format!("row_{k}_x"), first[0]);
        set(&format!("row_{k}_y"), first[1]);
        if castellated {
            set(&format!("notch_{k}_count"), notches);
            set(&format!("notch_{k}_x"), notch[0]);
            set(&format!("notch_{k}_y"), notch[1]);
        }
    }
    // The ground array along the file's directions: +x and −y, or turned
    // 45°. The form's columns run at the array's turn, its rows a quarter
    // turn clockwise of them; m quarter turns past the file's directions
    // swap the counts (odd m) and move the seed to the corner the file's
    // pattern starts from.
    let mut diagonal = false;
    match &d.ground {
        Some(g) => {
            let q = (g.rotation / (45 * DEG)).rem_euclid(8);
            diagonal = q % 2 == 1;
            let m = (q / 2) % 4;
            let swap = m % 2 == 1;
            let (cols, rows, px, py, sx, sy) = if swap {
                (g.rows, g.cols, g.pitch[1], g.pitch[0], g.size[1], g.size[0])
            } else {
                (g.cols, g.rows, g.pitch[0], g.pitch[1], g.size[0], g.size[1])
            };
            let (r0, c0) = match m {
                0 => (0, 0),
                1 => (0, g.cols - 1),
                2 => (g.rows - 1, g.cols - 1),
                _ => (g.rows - 1, 0),
            };
            let centres = d.ground_centres();
            let seed = usize::try_from(r0 * g.cols + c0)
                .ok()
                .and_then(|i| centres.get(i).copied())
                .unwrap_or(g.centre);
            set("ground_rows", rows);
            set("ground_columns", cols);
            set("ground_pitch_x", px);
            set("ground_pitch_y", py);
            set("ground_size_x", sx);
            set("ground_size_y", sy);
            set("ground_x", seed[0]);
            set("ground_y", seed[1]);
        }
        None => {
            set("ground_rows", 1);
            set("ground_columns", 1);
            set("ground_pitch_x", 1_600_000);
            set("ground_pitch_y", 1_600_000);
            set("ground_size_x", 1_100_000);
            set("ground_size_y", 1_100_000);
            set("ground_x", park);
            set("ground_y", 0);
        }
    }
    for k in 0..SINGLE_SLOTS {
        let (at, size) = d
            .singles
            .get(k)
            .map_or(([park, 0], [800_000, 800_000]), |p| (p.at, p.size));
        let k = k + 1;
        set(&format!("single_{k}_x"), at[0]);
        set(&format!("single_{k}_y"), at[1]);
        set(&format!("single_{k}_length"), size[0]);
        set(&format!("single_{k}_width"), size[1]);
    }
    let span = d.width - 2 * d.margin;
    let x0 = -hw + d.margin;
    // Parked: the meander centred on park_x.
    let parked = park - half(span) - x0;
    match meander(d) {
        Some((turns, step)) => {
            set("antenna_turns", turns);
            set("antenna_units", (turns - 1).max(1));
            set("antenna_step", step);
            set("antenna_shift", 0);
            set("antenna_unit_shift", if turns > 1 { 0 } else { parked });
        }
        None => {
            // Something drawable, all of it parked: one turn over the
            // span, in a region of at least 2.5 mm.
            set("antenna_depth", d.antenna.max(2_500_000));
            set("antenna_turns", 1);
            set("antenna_units", 1);
            set("antenna_step", half(span).max(1_000_000));
            set("antenna_shift", parked);
            set("antenna_unit_shift", 0);
        }
    }
    // The board beyond the can and the antenna region, for the marking
    // planes: a plane needs a positive size, so 1 µm where there is none.
    set(
        "mark_pcb_depth",
        (hl - d.can_at[1] - half(d.can[1])).max(UM),
    );
    set("mark_antenna_depth", d.antenna.max(UM));
    let pin1 = leads_first(d);
    set("pin1_x", pin1[0]);
    set("pin1_y", pin1[1]);
    v1::Model::PartFile {
        asset: match (castellated, diagonal) {
            (false, false) => parts::MODULE,
            (false, true) => parts::MODULE_DIAGONAL,
            (true, false) => parts::MODULE_CASTELLATED,
            (true, true) => parts::MODULE_CASTELLATED_DIAGONAL,
        }
        .into(),
        values,
    }
}

/// Pin 1's pad centre.
fn leads_first(d: &Dims) -> Point2 {
    d.row_centres().first().map_or([0, 0], |(_, c)| *c)
}

fn generate(request: &Request) -> Result<Generation, Vec<Problem>> {
    let description = description_with(Vec::new());
    let args = checked(&description, &request.params)?;
    let d = dims(&args, request)?;
    let pads = pads(&d);
    let body = Box2::centred([0, 0], [d.width, d.length]);
    let courtyard = land::courtyard(&pads, Some(body)).unwrap_or_default();
    let top = courtyard.iter().map(|p| p[1]).max().unwrap_or(body.max[1]);
    let mut silk = land::silk_outline(&pads, body, land::DEFAULT_MASK_NM);
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
    let keys = d.designations();
    let package = Package {
        terminals: keys.clone(),
        geometry: PackageGeometry {
            envelope_nm: [d.width, d.length, d.height],
            seating_nm: 0,
            leads: leads(&d),
            pin1: keys.first().cloned().unwrap_or_default(),
        },
    };
    let footprint = Footprint {
        pads,
        courtyard,
        silk,
        assembly: land::assembly(body),
        keepouts: keepouts(&d),
    };
    let component_part = request.scope == Scope::Component;
    let name = format!(
        "Module {}×{} mm, {} pads",
        mm_short(d.width),
        mm_short(d.length),
        keys.len()
    );
    let notes = vec![
        "The lands are the values' recommended land pattern; confirm them against the module datasheet."
            .to_string(),
    ];
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
        notes,
    })
}

/// A length in mm without trailing zeros (18, 25.5), for names.
fn mm_short(nm: i64) -> String {
    let text = mm(nm);
    let number = text.strip_suffix(" mm").unwrap_or(&text);
    let trimmed = number.trim_end_matches('0').trim_end_matches('.');
    trimmed.to_string()
}

impl Generator for Module {
    fn describe() -> Description {
        description()
    }

    fn generate(request: &Request) -> Result<Generation, Vec<Problem>> {
        generate(request)
    }
}

#[cfg(test)]
mod tests;
