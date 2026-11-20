//! Packages whose terminals no row pattern lands: each terminal is one or
//! more rectangles of leadframe given on their own (an L-shaped corner
//! terminal, pins joined by lugs, pins at their own pitch, the exposed
//! pad), and the land pattern is given land by land, as a manufacturer's
//! drawing gives it. The part file `sawn-terminals` places each rectangle
//! in one of its `TERMINAL_SLOTS` slots.
//!
//! A land numbered `(N)` lands terminal `N` without showing its number:
//! copper a land pattern draws beside a terminal's numbered land (a lug
//! joining two pins, a via in the exposed pad).

use super::{Point2, land, mm, parts, symbol};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use sparksmith_generator_sdk::args::{Args, problem};
use sparksmith_generator_sdk::land::{self as lands, Box2};
use sparksmith_generator_sdk::units::div_round;
use sparksmith_generator_sdk::v1::{
    self, BindingEntry, Component, Direction, Fill, Footprint, Generation, Graphic, Hole,
    HoleShape, Lead, Package, PackageGeometry, Pad, PadShape, ParamValue, Primitive, Problem,
    Request, Scope, Terminal,
};

/// Rectangles of leadframe the part file holds.
pub const TERMINAL_SLOTS: usize = 40;
/// Lands a given land pattern may have.
pub const MAX_LANDS: usize = 64;
/// The longest terminal number.
pub const MAX_NUMBER_CHARS: u32 = 16;
/// An estimated land is its rectangle grown by this on every side.
const LAND_GROWTH_NM: i64 = 25_000;
/// Where unused slots wait, beyond the −x edge (the part file's `park_x`).
const PARK_NM: i64 = 100_000_000;

/// One rectangle of a terminal's leadframe.
struct Rect {
    terminal: String,
    at: Point2,
    size: Point2,
    /// Its underside above the seating plane: 0 bares it underneath; more
    /// is a half-etched neck the moulding covers.
    lift: i64,
}

/// One given land.
struct GivenLand {
    terminal: String,
    /// It shows its terminal's number (`N`, not `(N)`).
    numbered: bool,
    at: Point2,
    size: Point2,
    round: bool,
    /// A plated hole's diameter; 0 for a surface land.
    drill: i64,
}

/// An explicit package, checked.
struct Explicit {
    body: [i64; 3],
    rects: Vec<Rect>,
    lands: Vec<GivenLand>,
    /// Terminal numbers in the order the rectangles first name them.
    terminals: Vec<String>,
}

fn ints(args: &Args<'_>, key: &str) -> Result<Vec<i64>, Problem> {
    Ok(args
        .list(key)?
        .iter()
        .filter_map(|v| match v {
            ParamValue::Int(i) => Some(*i),
            _ => None,
        })
        .collect())
}

fn texts(args: &Args<'_>, key: &str) -> Result<Vec<String>, Problem> {
    Ok(args
        .list(key)?
        .iter()
        .filter_map(|v| match v {
            ParamValue::Text(t) => Some(t.trim().to_string()),
            _ => None,
        })
        .collect())
}

/// A terminal number: 1–16 printable characters, no space or parenthesis.
fn valid_number(n: &str) -> bool {
    !n.is_empty()
        && n.chars().count() <= MAX_NUMBER_CHARS as usize
        && n.chars()
            .all(|c| c.is_ascii_graphic() && c != '(' && c != ')' && c != ':')
}

/// `(at, size)` lies within `[lo, hi]` of the centred body, in doubled nm.
fn inside(at: Point2, size: Point2, body: [i64; 3]) -> bool {
    (0..2).all(|a| 2 * at[a] - size[a] >= -body[a] && 2 * at[a] + size[a] <= body[a])
}

/// `q` lies on the rectangle (edges included).
fn on(q: Point2, at: Point2, size: Point2) -> bool {
    (0..2).all(|a| 2 * (q[a] - at[a]).abs() <= size[a])
}

/// Whether the request gives its terminals one by one.
pub fn given(args: &Args<'_>) -> Result<bool, Vec<Problem>> {
    Ok(!args
        .list("terminal_numbers")
        .map_err(|p| vec![p])?
        .is_empty())
}

fn read(args: &Args<'_>) -> Result<Explicit, Vec<Problem>> {
    let one = |p: Problem| vec![p];
    let body = [
        args.int("body_length").map_err(one)?,
        args.int("body_width").map_err(one)?,
        args.int("body_height").map_err(one)?,
    ];
    let frame = super::frame_of(body[2], false);
    let mut problems = Vec::new();
    let mut rule = |ok: bool, field: &str, text: String| {
        if !ok {
            problems.push(problem("param.inconsistent", field, text));
        }
    };
    rule(
        args.text("construction").map_err(one)? == "sawn"
            && !args.flag("wettable_flank").map_err(one)?,
        "construction",
        "Terminals given one by one make a sawn moulding without wettable flanks.".into(),
    );
    let numbers = texts(args, "terminal_numbers").map_err(one)?;
    let xs = ints(args, "terminal_x").map_err(one)?;
    let ys = ints(args, "terminal_y").map_err(one)?;
    let ls = ints(args, "terminal_size_x").map_err(one)?;
    let ws = ints(args, "terminal_size_y").map_err(one)?;
    let lifts = ints(args, "terminal_lift").map_err(one)?;
    let n = numbers.len();
    rule(
        xs.len() == n
            && ys.len() == n
            && ls.len() == n
            && ws.len() == n
            && (lifts.is_empty() || lifts.len() == n),
        "terminal_numbers",
        format!(
            "Each terminal rectangle needs a number, a centre x and y and a size x and y, and a lift when any has one ({n} numbers, {} x, {} y, {} sizes x, {} sizes y, {} lifts).",
            xs.len(),
            ys.len(),
            ls.len(),
            ws.len(),
            lifts.len()
        ),
    );
    rule(
        n <= TERMINAL_SLOTS,
        "terminal_numbers",
        format!("The part file holds {TERMINAL_SLOTS} terminal rectangles; {n} are given."),
    );
    let mut rects = Vec::new();
    let mut terminals: Vec<String> = Vec::new();
    for (k, number) in numbers.iter().enumerate() {
        let (Some(&x), Some(&y), Some(&l), Some(&w)) = (xs.get(k), ys.get(k), ls.get(k), ws.get(k))
        else {
            break;
        };
        let lift = lifts.get(k).copied().unwrap_or(0);
        rule(
            valid_number(number),
            "terminal_numbers",
            format!(
                "Terminal number \"{number}\" is not 1–{MAX_NUMBER_CHARS} printable characters without spaces, parentheses or colons."
            ),
        );
        rule(
            inside([x, y], [l, w], body),
            "terminal_x",
            format!(
                "Terminal {number}'s rectangle at ({}, {}) mm, {} × {} mm, reaches beyond the body.",
                mm(x),
                mm(y),
                mm(l),
                mm(w)
            ),
        );
        rule(
            (0..frame).contains(&lift),
            "terminal_lift",
            format!(
                "Terminal {number}'s lift {} mm is not under the leadframe's {} mm.",
                mm(lift),
                mm(frame)
            ),
        );
        if !terminals.contains(number) {
            terminals.push(number.clone());
        }
        rects.push(Rect {
            terminal: number.clone(),
            at: [x, y],
            size: [l, w],
            lift,
        });
    }
    for t in &terminals {
        rule(
            rects.iter().any(|r| &r.terminal == t && r.lift == 0),
            "terminal_lift",
            format!("Terminal {t} has no rectangle bared on the underside (lift 0)."),
        );
    }
    let numbers = texts(args, "given_land_numbers").map_err(one)?;
    let xs = ints(args, "given_land_x").map_err(one)?;
    let ys = ints(args, "given_land_y").map_err(one)?;
    let ls = ints(args, "given_land_size_x").map_err(one)?;
    let ws = ints(args, "given_land_size_y").map_err(one)?;
    let shapes = texts(args, "given_land_shapes").map_err(one)?;
    let drills = ints(args, "given_land_drill").map_err(one)?;
    let m = numbers.len();
    rule(
        xs.len() == m
            && ys.len() == m
            && ls.len() == m
            && ws.len() == m
            && (shapes.is_empty() || shapes.len() == m)
            && (drills.is_empty() || drills.len() == m),
        "given_land_numbers",
        format!(
            "Each given land needs a number, a centre x and y and a size x and y, and a shape and drill when any has one ({m} numbers, {} x, {} y, {} sizes x, {} sizes y, {} shapes, {} drills).",
            xs.len(),
            ys.len(),
            ls.len(),
            ws.len(),
            shapes.len(),
            drills.len()
        ),
    );
    let mut given = Vec::new();
    for (k, label) in numbers.iter().enumerate() {
        let (Some(&x), Some(&y), Some(&l), Some(&w)) = (xs.get(k), ys.get(k), ls.get(k), ws.get(k))
        else {
            break;
        };
        let (terminal, numbered) = match label.strip_prefix('(').and_then(|t| t.strip_suffix(')')) {
            Some(t) => (t.trim().to_string(), false),
            None => (label.clone(), true),
        };
        let round = shapes.get(k).is_some_and(|s| s == "round");
        let drill = drills.get(k).copied().unwrap_or(0);
        rule(
            terminals.contains(&terminal),
            "given_land_numbers",
            format!("Land \"{label}\" names no terminal the rectangles give."),
        );
        rule(
            !round || l == w,
            "given_land_size_x",
            format!("Round land \"{label}\" needs equal sizes x and y."),
        );
        rule(
            drill == 0 || (round && drill < l),
            "given_land_drill",
            format!("Land \"{label}\" with a hole must be round and wider than its hole."),
        );
        given.push(GivenLand {
            terminal,
            numbered,
            at: [x, y],
            size: [l, w],
            round,
            drill,
        });
    }
    rule(
        given.len() <= MAX_LANDS,
        "given_land_numbers",
        format!(
            "At most {MAX_LANDS} lands may be given; {} are.",
            given.len()
        ),
    );
    let e = Explicit {
        body,
        rects,
        lands: given,
        terminals,
    };
    if !e.lands.is_empty() {
        for t in &e.terminals {
            match e.lands.iter().find(|l| &l.terminal == t && l.numbered) {
                None => rule(
                    false,
                    "given_land_numbers",
                    format!("Terminal {t} has no land numbered {t}."),
                ),
                Some(l) => rule(
                    e.rects
                        .iter()
                        .any(|r| &r.terminal == t && r.lift == 0 && on(l.at, r.at, r.size)),
                    "given_land_x",
                    format!(
                        "Land {t}'s centre ({}, {}) mm is not on terminal {t}'s bared leadframe, so the joint has no lead.",
                        mm(l.at[0]),
                        mm(l.at[1])
                    ),
                ),
            }
        }
    }
    if problems.is_empty() {
        Ok(e)
    } else {
        Err(problems)
    }
}

impl Explicit {
    /// Each terminal's contact: its first numbered land's centre, else its
    /// first bared rectangle's.
    fn contact(&self, terminal: &str) -> Point2 {
        self.lands
            .iter()
            .find(|l| l.terminal == terminal && l.numbered)
            .map(|l| l.at)
            .or_else(|| {
                self.rects
                    .iter()
                    .find(|r| r.terminal == terminal && r.lift == 0)
                    .map(|r| r.at)
            })
            .unwrap_or([0, 0])
    }

    /// The lands: as given, else each bared rectangle grown a little.
    fn pads(&self) -> Vec<Pad> {
        let mut out: Vec<Pad> = Vec::new();
        let key = |terminal: &str, numbered: bool, out: &[Pad]| -> (String, Option<String>) {
            let first = numbered && !out.iter().any(|p| p.key == terminal);
            if first {
                (terminal.to_string(), None)
            } else {
                let k = out.iter().filter(|p| p.terminal == terminal).count() + 1;
                (
                    format!("{terminal}:{k}"),
                    Some(if numbered {
                        terminal.to_string()
                    } else {
                        String::new()
                    }),
                )
            }
        };
        if self.lands.is_empty() {
            for r in self.rects.iter().filter(|r| r.lift == 0) {
                let (k, number) = key(&r.terminal, true, &out);
                let mut pad = land(&k, r.at, r.size.map(|s| s + 2 * LAND_GROWTH_NM));
                pad.terminal = r.terminal.clone();
                pad.number = number;
                out.push(pad);
            }
        } else {
            for l in &self.lands {
                let (k, number) = key(&l.terminal, l.numbered, &out);
                let mut pad = land(&k, l.at, l.size);
                pad.terminal = l.terminal.clone();
                pad.number = number;
                if l.round {
                    pad.shape = PadShape::Circle {
                        diameter_nm: l.size[0],
                    };
                }
                if l.drill > 0 {
                    pad.hole = Some(Hole {
                        shape: HoleShape::Round {
                            diameter_nm: l.drill,
                        },
                        plated: true,
                        offset: None,
                    });
                }
                out.push(pad);
            }
        }
        out
    }

    /// The `sawn-terminals` part file at this package's values.
    fn model(&self) -> v1::Model {
        let [bl, bw, bh] = self.body;
        let radius = super::dot_radius(bl.min(bw));
        let mut values = v1::Params::new();
        let mut set = |name: &str, v: i64| {
            values.insert(name.into(), ParamValue::Int(v));
        };
        set("body_length", bl);
        set("body_width", bw);
        set("body_height", bh);
        set("frame", super::frame_of(bh, false));
        set("edge_round", super::edge_round(bh));
        set("mark_radius", radius);
        set("mark_depth", super::DOT_DEPTH_NM);
        // The dot two radii in from the corner nearest pin 1.
        let pin1 = self.terminals.first().map_or([0, 0], |t| self.contact(t));
        let toward = |c: i64, half: i64| {
            let sign = if c > 0 { 1 } else { -1 };
            sign * (half - 2 * radius)
        };
        set("mark_x", toward(pin1[0], div_round(bl, 2).unwrap_or(0)));
        set("mark_y", toward(pin1[1], div_round(bw, 2).unwrap_or(0)));
        let park = -div_round(bl, 2).unwrap_or(0) - PARK_NM;
        set("park_x", park);
        for k in 0..TERMINAL_SLOTS {
            let (at, size, lift) = self
                .rects
                .get(k)
                .map_or(([park, 0], [200_000, 200_000], 0), |r| {
                    (r.at, r.size, r.lift)
                });
            let k = k + 1;
            set(&format!("terminal_{k}_x"), at[0]);
            set(&format!("terminal_{k}_y"), at[1]);
            set(&format!("terminal_{k}_length"), size[0]);
            set(&format!("terminal_{k}_width"), size[1]);
            set(&format!("terminal_{k}_lift"), lift);
        }
        v1::Model::PartFile {
            asset: parts::SAWN_TERMINALS.into(),
            values,
        }
    }
}

/// The silkscreen pin-1 dot beyond pin 1's land, on the side of the body
/// that land is nearest.
fn pin_one_dot(first: &Pad, body: [i64; 3]) -> Graphic {
    let b = lands::pad_box(first);
    let r = super::PIN_ONE_DOT_RADIUS_NM;
    let gap = lands::DEFAULT_MASK_NM + lands::SILK_PAD_CLEARANCE_NM + 2 * r;
    // Across x when the land lies relatively nearer the ±x edge.
    let across_x = first.at[0].abs() * body[1] >= first.at[1].abs() * body[0];
    let center = if across_x {
        if first.at[0] > 0 {
            [b.max[0] + gap, first.at[1]]
        } else {
            [b.min[0] - gap, first.at[1]]
        }
    } else if first.at[1] > 0 {
        [first.at[0], b.max[1] + gap]
    } else {
        [first.at[0], b.min[1] - gap]
    };
    Graphic {
        key: "pin1".into(),
        shape: Primitive::Circle {
            center,
            radius_nm: r,
        },
        stroke_nm: 0,
        fill: Fill::Solid,
    }
}

pub fn generate(request: &Request, args: &Args<'_>) -> Result<Generation, Vec<Problem>> {
    let e = read(args)?;
    let pads = e.pads();
    let body = Box2::centred([0, 0], [e.body[0], e.body[1]]);
    let courtyard = lands::courtyard(&pads, Some(body)).unwrap_or_default();
    let top = courtyard.iter().map(|p| p[1]).max().unwrap_or(body.max[1]);
    let mut silk = lands::silk_outline(&pads, body, lands::DEFAULT_MASK_NM);
    silk.push(lands::silk_designator(body, top));
    if e.terminals.len() > 2
        && let Some(first) = e
            .terminals
            .first()
            .and_then(|t| pads.iter().find(|p| &p.key == t))
    {
        silk.push(pin_one_dot(first, e.body));
    }
    let keys = e.terminals.clone();
    let package = Package {
        terminals: keys.clone(),
        geometry: PackageGeometry {
            envelope_nm: e.body,
            seating_nm: 0,
            leads: keys
                .iter()
                .map(|key| {
                    let at = e.contact(key);
                    Lead {
                        key: key.clone(),
                        contact: [at[0], at[1], 0],
                    }
                })
                .collect(),
            pin1: keys.first().cloned().unwrap_or_default(),
        },
    };
    let footprint = Footprint {
        pads,
        courtyard,
        silk,
        assembly: lands::assembly(body),
        keepouts: Vec::new(),
    };
    let component_part = request.scope == Scope::Component;
    let name = format!(
        "No-lead-{} {}×{}",
        keys.len(),
        super::short_mm(e.body[0]),
        super::short_mm(e.body[1])
    );
    Ok(Generation {
        marking: crate::marking(args).map_err(|p| vec![p])?,
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
        model: Some(e.model()),
        fields: Vec::new(),
        notes: vec![if e.lands.is_empty() {
            "Lands are the terminals' bared leadframe grown 0.025 mm on every side; confirm them \
             against the device's recommended land pattern."
                .into()
        } else {
            "Lands are the given land pattern; confirm it against the device's recommended land \
             pattern."
                .into()
        }],
    })
}
