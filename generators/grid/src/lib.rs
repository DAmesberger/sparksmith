//! The grid generator: grid arrays of contacts under a body, as a symbol,
//! package, footprint and 3D model: BGA and WLCSP on solder balls, LGA on
//! flat lands, fully populated or with positions left empty.
//!
//! Frame: x along the body length (columns, numbered 1, 2, … from −x), y
//! along the body width (rows, lettered from +y), seen from above. Ball A1
//! is at the −x, +y corner. Rows take their names from the row-letter
//! alphabet (JEDEC JESD95-1 leaves out I, O, Q, S, X and Z); beyond its
//! last letter they run AA, AB, … Terminals are in row-major order from A1
//! (row A from −x to +x, then row B, …), leaving out the depopulated
//! positions, which keep their names, so every other ball keeps its own.
//!
//! Lands are estimated, as the host's grid template estimates them: round,
//! non-solder-mask defined, 0.8 of the ball's diameter (a land grid's land
//! is its own diameter), centred on each contact, with the project's mask
//! and paste rules. Not a land-pattern standard; a device's recommended
//! lands replace them.
//!
//! The 3D model is one of four part files: balls or lands, each full or
//! with cut-outs. A full grid is one seed contact and a two-way body
//! pattern of `rows` × `columns`. A depopulated grid is the same full
//! pattern with up to [`MAX_VOIDS`] rectangular cut-outs that remove the
//! empty positions whole; the module covers the empty positions with as
//! few rectangles as its greedy cover finds and refuses a map that needs
//! more (a part file has a fixed number of features, and no pattern of
//! this Solidcraft skips single instances).
//!
//! The presets are the grid starter table, in nm: matrices and pitches of
//! the JEDEC fine-pitch BGA conventions, body and ball sizes typical,
//! estimated, as the table says.
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

/// The generator's version (`Description::version`): 4 sets the marking
/// text at the height its preset gives (`marking_height`).
const VERSION: u32 = 4;

/// Most rows or columns of one grid (the host's grid form allows as many).
const MAX_SIDE: i64 = 64;
/// Most positions, empty ones included: the part file's contacts are one
/// rectangular pattern of every position, and Solidcraft patterns at most
/// 1000 instances. The looks are assigned up to this count.
pub const MAX_POSITIONS: i64 = 1000;
/// Most contacts: the host's lead bound for a typed package (512) and the
/// full part files' looks.
pub const MAX_CONTACTS: i64 = 512;
/// Rectangular cut-outs of the depopulated part files.
pub const MAX_VOIDS: usize = 16;
/// Longest row-letter alphabet.
const MAX_ROW_LETTERS: u32 = 64;
/// The JEDEC row-letter alphabet: A–Z without I, O, Q, S, X and Z.
const JEDEC_ROW_LETTERS: &str = "ABCDEFGHJKLMNPRTUVWY";

/// A ball grid's land: this share of the ball's diameter, in fifths (0.8,
/// the host's estimated NSMD land).
const LAND_SHARE_FIFTHS: i64 = 4;

/// The pin-1 dot: a filled silkscreen circle of this radius beyond pad A1's
/// −x edge, clear of its mask opening by the silkscreen clearance (as the
/// host's template marks pin 1 of a package of more than two terminals).
const PIN_ONE_DOT_RADIUS_NM: i64 = 200_000;

/// LGA land thickness (est.: plated lands stand 20–40 µm), at most a
/// quarter of the body height.
const LAND_THICKNESS_NM: i64 = 30_000;
/// Round of the body's top edges (est.: sawn substrates stay sharp), at
/// most a tenth of the body height.
const TOP_EDGE_NM: i64 = 50_000;
/// Pin-1 dimple depth (est.), and its radius: 6 % of the shorter body side
/// within 0.1–0.4 mm, centred two radii in from the −x, +y corner.
const DIMPLE_DEPTH_NM: i64 = 30_000;
const DIMPLE_RADIUS_NM: [i64; 2] = [100_000, 400_000];

pub struct Grid;

export_generator!(Grid);

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

fn count(key: &str, label: &str, default: i64) -> Param {
    Param {
        key: key.into(),
        label: label.into(),
        help: None,
        ty: ParamType::Count {
            min: 1,
            max: MAX_SIDE,
        },
        default: ParamValue::Int(default),
        group: Some("matrix".into()),
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
    let mut depopulated = Param {
        key: "depopulated".into(),
        label: "Empty positions".into(),
        help: None,
        ty: ParamType::List {
            of: alloc::boxed::Box::new(ParamType::Text { max_chars: 8 }),
            max_len: u32::try_from(MAX_POSITIONS - 1).unwrap_or(0),
        },
        default: ParamValue::List(Vec::new()),
        group: Some("matrix".into()),
        context: None,
    };
    depopulated.help = Some(
        "Positions without a contact, by name (E5, AA12); every other contact keeps its name."
            .into(),
    );
    let mut letters = Param {
        key: "row_letters".into(),
        label: "Row letters".into(),
        help: None,
        ty: ParamType::Text {
            max_chars: MAX_ROW_LETTERS,
        },
        default: ParamValue::Text(JEDEC_ROW_LETTERS.into()),
        group: Some("matrix".into()),
        context: None,
    };
    letters.help = Some(
        "The letters rows are named with, in order; after the last, rows run AA, AB, … (JEDEC leaves out I, O, Q, S, X and Z)."
            .into(),
    );
    let mut params = vec![
        Param {
            key: "contact".into(),
            label: "Contacts".into(),
            help: None,
            ty: ParamType::Enum {
                choices: vec![
                    Choice {
                        key: "ball".into(),
                        label: "Solder balls (BGA, WLCSP)".into(),
                    },
                    Choice {
                        key: "land".into(),
                        label: "Lands (LGA)".into(),
                    },
                ],
            },
            default: ParamValue::Text("ball".into()),
            group: Some("matrix".into()),
            context: None,
        },
        count("rows", "Rows (y)", 4),
        count("columns", "Columns (x)", 5),
        length(
            "pitch",
            "Pitch along a row (x)",
            [100 * UM, 5 * MM],
            800_000,
            "matrix",
        ),
        length(
            "row_pitch",
            "Pitch between rows (y)",
            [100 * UM, 5 * MM],
            800_000,
            "matrix",
        ),
        depopulated,
        letters,
        length(
            "body_length",
            "Body length (x)",
            [300 * UM, 100 * MM],
            5 * MM,
            "body",
        ),
        length(
            "body_width",
            "Body width (y)",
            [300 * UM, 100 * MM],
            4 * MM,
            "body",
        ),
        length(
            "body_height",
            "Body height above the contacts",
            [50 * UM, 10 * MM],
            800_000,
            "body",
        ),
        length(
            "contact_diameter",
            "Ball or land diameter",
            [50 * UM, 3 * MM],
            400_000,
            "contacts",
        ),
        length(
            "contact_height",
            "Ball height (0 for lands)",
            [0, 3 * MM],
            300_000,
            "contacts",
        ),
    ];
    params.push(sparksmith_generator_sdk::marking::param());
    params.push(sparksmith_generator_sdk::marking::height_param());
    let rule = |text: &str, fields: &[&str]| Rule {
        text: text.into(),
        fields: fields.iter().map(|f| (*f).into()).collect(),
    };
    let rules = vec![
        rule(
            "Contact diameter < pitch, so contacts do not touch",
            &["contact_diameter", "pitch", "row_pitch"],
        ),
        rule(
            "The columns fit along the body length and the rows across its width",
            &[
                "rows",
                "columns",
                "pitch",
                "row_pitch",
                "body_length",
                "body_width",
            ],
        ),
        rule(
            "A ball is lower than it is wide; a land has no ball height",
            &["contact", "contact_height", "contact_diameter"],
        ),
        rule(
            "At most 1000 positions and 512 contacts",
            &["rows", "columns", "depopulated"],
        ),
    ];
    let offer = PartOffer {
        families: vec!["grid".into()],
        scopes: vec![Scope::Component, Scope::Package],
        grammar: vec![Grammar {
            scope: Scope::Component,
            patterns: vec![
                "bga{count} {body_length:length}x{body_width:length}? {pitch:length}?".into(),
                "lga{count} {body_length:length}x{body_width:length}? {pitch:length}?".into(),
                "wlcsp{count} {body_length:length}x{body_width:length}? {pitch:length}?".into(),
            ],
            examples: vec!["BGA-64 7x7 0.8".into(), "WLCSP-9 1.4x1.4 0.4".into()],
        }],
        instance_fields: Vec::new(),
    };
    Description {
        abi: v1::ABI,
        id: "grid".into(),
        version: VERSION,
        title: "Grid arrays (BGA, LGA, WLCSP)".into(),
        licence: "MIT OR Apache-2.0".into(),
        deterministic: true,
        capabilities: vec![Capability {
            name: v1::GENERATE_PART.into(),
            effect: Effect::None,
            title: Some("Make a BGA, LGA or WLCSP part".into()),
            params,
            rules,
            presets,
            part: Some(offer),
        }],
        needs: Vec::new(),
    }
}

type Point2 = [i64; 2];

/// A grid's values, checked.
struct Dims {
    balls: bool,
    rows: i64,
    cols: i64,
    pitch: Point2,
    body: [i64; 3],
    diameter: i64,
    /// Ball height; 0 for lands.
    height: i64,
    letters: Vec<char>,
    /// Empty (row, column) positions, 0-based.
    empty: BTreeSet<(i64, i64)>,
}

impl Dims {
    /// The populated positions in lead order (row-major from A1).
    fn populated(&self) -> Vec<(i64, i64)> {
        (0..self.rows)
            .flat_map(|r| (0..self.cols).map(move |c| (r, c)))
            .filter(|p| !self.empty.contains(p))
            .collect()
    }

    /// The contact centre of position (row, column).
    fn centre(&self, row: i64, col: i64) -> Point2 {
        [
            centred(col, self.cols, self.pitch[0]),
            -centred(row, self.rows, self.pitch[1]),
        ]
    }

    /// The name of row `row` (0-based): its letter, or after the last
    /// letter two letters (AA, AB, …), as the host's grid form names rows.
    fn row_name(&self, row: i64) -> String {
        let n = i64::try_from(self.letters.len()).unwrap_or(1).max(1);
        let mut i = row;
        let mut out = Vec::new();
        loop {
            let at = usize::try_from(i % n).unwrap_or(0);
            out.push(self.letters.get(at).copied().unwrap_or('A'));
            if i < n {
                break;
            }
            i = i / n - 1;
        }
        out.iter().rev().collect()
    }

    fn ball_name(&self, row: i64, col: i64) -> String {
        format!("{}{}", self.row_name(row), col + 1)
    }

    /// The position a name like "C7" (any case) names inside the matrix.
    fn position_of(&self, name: &str) -> Option<(i64, i64)> {
        let name = name.trim().to_ascii_uppercase();
        let split = name.find(|c: char| c.is_ascii_digit())?;
        let (letters, digits) = name.split_at(split);
        if letters.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let col = digits.parse::<i64>().ok()?.checked_sub(1)?;
        if !(0..self.cols).contains(&col) {
            return None;
        }
        (0..self.rows)
            .find(|&r| self.row_name(r) == letters)
            .map(|r| (r, col))
    }

    fn body(&self) -> Box2 {
        Box2::centred([0, 0], [self.body[0], self.body[1]])
    }

    /// The copper land's diameter.
    fn land(&self) -> i64 {
        if self.balls {
            div_round(self.diameter * LAND_SHARE_FIFTHS, 5).unwrap_or(self.diameter)
        } else {
            self.diameter
        }
    }
}

/// `(i − (n − 1) / 2) × pitch`, to the nearest nanometre.
fn centred(i: i64, n: i64, pitch: i64) -> i64 {
    div_round((2 * i - (n - 1)) * pitch, 2).unwrap_or(0)
}

/// The row-letter alphabet: 1–64 distinct capitals A–Z in order.
fn alphabet(text: &str) -> Option<Vec<char>> {
    let letters: Vec<char> = text.chars().collect();
    let valid = !letters.is_empty()
        && letters.len() <= MAX_ROW_LETTERS as usize
        && letters.iter().all(char::is_ascii_uppercase)
        && letters.windows(2).all(|w| w[0] < w[1]);
    valid.then_some(letters)
}

fn dims(args: &Args<'_>, request: &Request) -> Result<Dims, Vec<Problem>> {
    let one = |p: Problem| vec![p];
    let balls = args.text("contact").map_err(one)? == "ball";
    let row_letters = args.text("row_letters").map_err(one)?;
    let letters = alphabet(row_letters).ok_or_else(|| {
        one(problem(
            "param.inconsistent",
            "row_letters",
            format!(
                "Row letters are 1–{MAX_ROW_LETTERS} distinct capitals A–Z in alphabetical order (JEDEC: {JEDEC_ROW_LETTERS}); \"{row_letters}\" is not."
            ),
        ))
    })?;
    let mut d = Dims {
        balls,
        rows: args.int("rows").map_err(one)?,
        cols: args.int("columns").map_err(one)?,
        pitch: [
            args.int("pitch").map_err(one)?,
            args.int("row_pitch").map_err(one)?,
        ],
        body: [
            args.int("body_length").map_err(one)?,
            args.int("body_width").map_err(one)?,
            args.int("body_height").map_err(one)?,
        ],
        diameter: args.int("contact_diameter").map_err(one)?,
        height: args.int("contact_height").map_err(one)?,
        letters,
        empty: BTreeSet::new(),
    };
    let mut problems = Vec::new();
    let mut rule = |ok: bool, field: &str, text: String| {
        if !ok {
            problems.push(problem("param.inconsistent", field, text));
        }
    };
    let positions = d.rows * d.cols;
    rule(
        positions <= MAX_POSITIONS,
        "rows",
        format!(
            "A {} × {} grid has {positions} positions; the grid's part files hold at most {MAX_POSITIONS}.",
            d.rows, d.cols
        ),
    );
    // The empty positions, by name. Read from the request: the SDK's
    // typed access has no list, and the check above has already made it
    // a list of short texts.
    if let Some(ParamValue::List(items)) = request.params.get("depopulated") {
        for item in items {
            let ParamValue::Text(name) = item else {
                continue;
            };
            match d.position_of(name) {
                Some(p) if d.empty.insert(p) => {}
                Some(_) => rule(
                    false,
                    "depopulated",
                    format!("The empty position {name} is listed twice."),
                ),
                None => rule(
                    false,
                    "depopulated",
                    format!(
                        "{name} names no position of the {} × {} grid.",
                        d.rows, d.cols
                    ),
                ),
            }
        }
    }
    let empty = i64::try_from(d.empty.len()).unwrap_or(i64::MAX);
    let contacts = positions - empty;
    rule(
        contacts >= 1,
        "depopulated",
        "A grid keeps at least one contact.".into(),
    );
    rule(
        contacts <= MAX_CONTACTS,
        "depopulated",
        format!("The grid has {contacts} contacts; a package has at most {MAX_CONTACTS}."),
    );
    let [bl, bw, _] = d.body;
    let dia = d.diameter;
    rule(
        (d.cols < 2 || dia < d.pitch[0]) && (d.rows < 2 || dia < d.pitch[1]),
        "contact_diameter",
        format!(
            "Contacts {} mm across touch at a pitch of {} × {} mm.",
            mm(dia),
            mm(d.pitch[0]),
            mm(d.pitch[1])
        ),
    );
    rule(
        (d.cols - 1) * d.pitch[0] + dia <= bl,
        "columns",
        format!(
            "{} columns at {} mm do not fit along the {} mm body length.",
            d.cols,
            mm(d.pitch[0]),
            mm(bl)
        ),
    );
    rule(
        (d.rows - 1) * d.pitch[1] + dia <= bw,
        "rows",
        format!(
            "{} rows at {} mm do not fit across the {} mm body width.",
            d.rows,
            mm(d.pitch[1]),
            mm(bw)
        ),
    );
    if balls {
        rule(
            d.height > 0 && d.height < dia,
            "contact_height",
            format!(
                "A ball {} mm high is not lower than it is wide ({} mm), or has no height.",
                mm(d.height),
                mm(dia)
            ),
        );
    } else {
        rule(
            d.height == 0,
            "contact_height",
            format!(
                "A land grid has no ball height (0, is {} mm).",
                mm(d.height)
            ),
        );
    }
    if problems.is_empty() {
        let voids = voids(&d).len();
        if voids > MAX_VOIDS {
            problems.push(problem(
                "param.inconsistent",
                "depopulated",
                format!(
                    "The empty positions need {voids} rectangular cut-outs in the 3D model; the grid's part file has {MAX_VOIDS}. Empty positions that form fewer rectangles fit."
                ),
            ));
        }
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

/// A rectangle of positions: first row, first column, last row, last
/// column (inclusive).
type Block = [i64; 4];

/// The empty positions as rectangles of empty positions, which may
/// overlap: from each empty position not yet covered (in lead order), as
/// far right as the positions are empty, then as far down as every
/// position of that run is.
fn voids(d: &Dims) -> Vec<Block> {
    let mut covered: BTreeSet<(i64, i64)> = BTreeSet::new();
    let mut out = Vec::new();
    for &(r, c) in &d.empty {
        if covered.contains(&(r, c)) {
            continue;
        }
        let mut c1 = c;
        while d.empty.contains(&(r, c1 + 1)) {
            c1 += 1;
        }
        let mut r1 = r;
        while (c..=c1).all(|k| d.empty.contains(&(r1 + 1, k))) {
            r1 += 1;
        }
        for rr in r..=r1 {
            for cc in c..=c1 {
                covered.insert((rr, cc));
            }
        }
        out.push([r, c, r1, c1]);
    }
    out
}

/// A rectangle of positions as a box on the body, `[x, y, length, width]`
/// (corner at −x, −y): from half a pitch outside its outer contacts,
/// within the body outline.
fn void_box(d: &Dims, [r0, c0, r1, c1]: Block) -> [i64; 4] {
    let half = |p: i64| div_round(p, 2).unwrap_or(0);
    let (hl, hw) = (half(d.body[0]), half(d.body[1]));
    let x0 = (d.centre(r0, c0)[0] - half(d.pitch[0])).max(-hl);
    let x1 = (d.centre(r0, c1)[0] + half(d.pitch[0])).min(hl);
    let y1 = (d.centre(r0, c0)[1] + half(d.pitch[1])).min(hw);
    let y0 = (d.centre(r1, c0)[1] - half(d.pitch[1])).max(-hw);
    [x0, y0, x1 - x0, y1 - y0]
}

/// The estimated lands, in lead order.
fn pads(d: &Dims) -> Vec<Pad> {
    let land = d.land();
    d.populated()
        .into_iter()
        .map(|(r, c)| {
            let key = d.ball_name(r, c);
            Pad {
                key: key.clone(),
                terminal: key,
                shape: PadShape::Circle { diameter_nm: land },
                at: d.centre(r, c),
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

/// The package's 3D model: one of the bundle's four part files (balls or
/// lands, full or with cut-outs), every parameter given. A body (substrate
/// and moulding as one block) of `body_length` × `body_width` ×
/// `body_height` stands on the balls (`contact_height` high) or on the
/// seating plane over the lands, which are set into its underside
/// (`contact_height` is then the land's thickness); its top edges are
/// rounded and a dimple marks the A1 corner.
fn model(d: &Dims) -> v1::Model {
    let [bl, bw, bh] = d.body;
    let short = bl.min(bw);
    let (lo, hi) = (DIMPLE_RADIUS_NM[0], DIMPLE_RADIUS_NM[1]);
    let mut radius = div_round(short * 6, 100).unwrap_or(lo).clamp(lo, hi);
    // On a body too small for it, the dimple shrinks to fit.
    if 6 * radius >= short {
        radius = div_round(short, 8).unwrap_or(0);
    }
    let depth = if 4 * DIMPLE_DEPTH_NM < bh {
        DIMPLE_DEPTH_NM
    } else {
        div_round(bh, 8).unwrap_or(0)
    };
    let contact_height = if d.balls {
        d.height
    } else {
        LAND_THICKNESS_NM.min(div_round(bh, 4).unwrap_or(0))
    };
    let mut values = v1::Params::new();
    let mut set = |name: &str, v: i64| {
        values.insert(name.into(), ParamValue::Int(v));
    };
    set("rows", d.rows);
    set("columns", d.cols);
    set("pitch", d.pitch[0]);
    set("row_pitch", d.pitch[1]);
    set("body_length", bl);
    set("body_width", bw);
    set("body_height", bh);
    set("contact_diameter", d.diameter);
    set("contact_height", contact_height);
    set(
        "edge_round",
        TOP_EDGE_NM.min(div_round(bh, 10).unwrap_or(0)),
    );
    set("mark_radius", radius);
    set("mark_depth", depth);
    let blocks = voids(d);
    let full = blocks.is_empty();
    if let Some(&first) = blocks.first() {
        // Every cut-out of the file is used: the ones the map does not
        // need repeat the first, which then removes nothing more.
        for k in 0..MAX_VOIDS {
            let [x, y, l, w] = void_box(d, blocks.get(k).copied().unwrap_or(first));
            set(&format!("void_{}_x", k + 1), x);
            set(&format!("void_{}_y", k + 1), y);
            set(&format!("void_{}_length", k + 1), l);
            set(&format!("void_{}_width", k + 1), w);
        }
    }
    let asset = match (d.balls, full) {
        (true, true) => parts::GRID_BALL,
        (true, false) => parts::GRID_BALL_VOIDS,
        (false, true) => parts::GRID_LAND,
        (false, false) => parts::GRID_LAND_VOIDS,
    };
    v1::Model::PartFile {
        asset: asset.into(),
        values,
    }
}

fn generate(request: &Request) -> Result<Generation, Vec<Problem>> {
    let description = description_with(Vec::new());
    let args = checked(&description, &request.params)?;
    let d = dims(&args, request)?;
    let pads = pads(&d);
    let body = d.body();
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
    let keys: Vec<String> = pads.iter().map(|p| p.key.clone()).collect();
    let package = Package {
        terminals: keys.clone(),
        geometry: PackageGeometry {
            envelope_nm: [d.body[0], d.body[1], d.height + d.body[2]],
            seating_nm: 0,
            leads: pads
                .iter()
                .map(|p| Lead {
                    key: p.key.clone(),
                    contact: [p.at[0], p.at[1], 0],
                })
                .collect(),
            pin1: keys.first().cloned().unwrap_or_default(),
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
    let family = if d.balls { "BGA" } else { "LGA" };
    let pitch = if d.pitch[0] == d.pitch[1] {
        short_mm(d.pitch[0])
    } else {
        format!("{}×{}", short_mm(d.pitch[0]), short_mm(d.pitch[1]))
    };
    let name = format!(
        "{family}-{} {pitch} {}×{} {}×{}",
        keys.len(),
        d.rows,
        d.cols,
        short_mm(d.body[0]),
        short_mm(d.body[1])
    );
    let mut notes = vec![
        "Lands are estimated from the contacts; confirm them against the device's recommended \
         land pattern."
            .to_string(),
    ];
    let blocks = voids(&d).len();
    if blocks > 0 {
        notes.push(format!(
            "{} empty positions, removed from the 3D model by {blocks} rectangular cut-outs.",
            d.empty.len()
        ));
    }
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

impl Generator for Grid {
    fn describe() -> Description {
        description()
    }

    fn generate(request: &Request) -> Result<Generation, Vec<Problem>> {
        generate(request)
    }
}

#[cfg(test)]
mod tests;
