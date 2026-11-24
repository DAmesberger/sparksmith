//! What a random board is: a list of atoms drawn from a seed, as plain data.
//! An atom names its parts, pads and nets by number and is nothing when
//! what it names is not there, so every subset of a case's atoms is a case:
//! that is what shrinking takes away, and what a reproduction line names.
use crate::geometry::Point;
use crate::model::Layer;
use crate::rules::CopperObject;

/// The board, mm. The parts stand on a grid of cells; what is cut and
/// drilled of the board itself lies between them.
pub(super) const BOARD: Point = [38.0, 26.0];
/// Where parts stand.
pub(super) const CELLS: [Point; 6] = [
    [7.5, 7.5],
    [19.0, 7.5],
    [30.5, 7.5],
    [7.5, 18.5],
    [19.0, 18.5],
    [30.5, 18.5],
];
/// Between two pads of a part, mm: a footprint is three by three spots.
pub(super) const PITCH: f64 = 2.4;

/// A small deterministic generator (xorshift64*, seeded by splitmix64):
/// no clock and no state but its own.
pub(super) struct Rng(u64);

impl Rng {
    pub(super) fn new(seed: u64) -> Self {
        let mut z = seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        Self((z ^ (z >> 31)).max(1))
    }

    pub(super) fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    pub(super) fn below(&mut self, n: usize) -> usize {
        (self.next() >> 16) as usize % n.max(1)
    }

    pub(super) fn chance(&mut self, of_ten: usize) -> bool {
        self.below(10) < of_ten
    }

    pub(super) fn pick<T: Copy>(&mut self, of: &[T]) -> Option<T> {
        of.get(self.below(of.len())).copied()
    }

    /// A value in `[lo, hi)`, a whole number of `step`s from `lo`.
    pub(super) fn steps(&mut self, lo: f64, hi: f64, step: f64) -> f64 {
        let n = ((hi - lo) / step).floor().max(1.0) as usize;
        let mm = lo + self.below(n) as f64 * step;
        // Coordinates are whole nanometres, as the files hold them.
        (mm * 1e6).round() / 1e6
    }

    /// A place on the board: mostly on a grid, else on no grid at all.
    fn place(&mut self) -> Point {
        let step = if self.chance(7) { 0.25 } else { 0.001_003 };
        [
            self.steps(1.5, BOARD[0] - 1.5, step),
            self.steps(1.5, BOARD[1] - 1.5, step),
        ]
    }

    fn turn(&mut self) -> f64 {
        match self.below(4) {
            0 => 0.0,
            1 => 90.0 * self.below(4) as f64,
            2 => 15.0 * self.below(24) as f64,
            _ => self.steps(0.0, 360.0, 0.731),
        }
    }

    fn side(&mut self) -> Layer {
        if self.chance(6) {
            Layer::Top
        } else {
            Layer::Bottom
        }
    }
}

/// The kinds of pad a part has.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Form {
    Rect,
    Round,
    RoundRect,
    /// Two of four corners rounded.
    OneSided,
    Oblong,
    /// Lines and an arc.
    Dee,
    /// A corner that goes in.
    Ell,
    /// On the side away from the part.
    FarSide,
    Through,
    ThroughSquare,
    /// Through, its hole off the pad's middle.
    ThroughOff,
    ThroughSlot,
    /// A hole without copper.
    Bare,
    Fiducial,
    /// Copper of no net, its paste in windows.
    Windows,
    /// Copper of no net with a plated hole of its own.
    Drilled,
}

pub(super) const FORMS: [Form; 16] = [
    Form::Rect,
    Form::Round,
    Form::RoundRect,
    Form::OneSided,
    Form::Oblong,
    Form::Dee,
    Form::Ell,
    Form::FarSide,
    Form::Through,
    Form::ThroughSquare,
    Form::ThroughOff,
    Form::ThroughSlot,
    Form::Bare,
    Form::Fiducial,
    Form::Windows,
    Form::Drilled,
];

impl Form {
    /// Whether a terminal lands on it.
    pub(super) fn lands(self) -> bool {
        !matches!(
            self,
            Self::Bare | Self::Fiducial | Self::Windows | Self::Drilled
        )
    }
}

/// How a pad is masked or pasted where it does not follow the board.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Opening {
    Board,
    Own,
    Closed,
}

/// What a part prints.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Print {
    Line,
    Arc,
    Circle,
    Solid,
    Text,
    Designator,
}

pub(super) const PRINTS: [Print; 6] = [
    Print::Line,
    Print::Arc,
    Print::Circle,
    Print::Solid,
    Print::Text,
    Print::Designator,
];

/// What is cut or drilled of the board itself, each at a place of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Cut {
    /// The four corners rounded.
    Corners,
    /// The right side an arc outwards.
    Bow,
    /// An arc into the board at its top.
    Bay,
    Window,
    Slot,
    PlatedSlot,
    PlatedHole,
    BareHole,
}

pub(super) const CUTS: [Cut; 8] = [
    Cut::Corners,
    Cut::Bow,
    Cut::Bay,
    Cut::Window,
    Cut::Slot,
    Cut::PlatedSlot,
    Cut::PlatedHole,
    Cut::BareHole,
];

/// A pour, as it is added.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Pour {
    pub net: usize,
    /// No net at all.
    pub free: bool,
    pub layer: usize,
    /// The whole board, else drawn: a box with one round corner.
    pub plane: bool,
    pub from: Point,
    pub to: Point,
    pub priority: u32,
    pub clearance: Option<f64>,
    pub islands: bool,
    pub solid: bool,
    pub hatched: bool,
}

/// How a route goes past other copper.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Past {
    Shove,
    Round,
    Straight,
}

/// One edit. A `pick` counts through what is there when it is made.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Edit {
    Place {
        pick: usize,
        to: Point,
        turn: f64,
        side: Layer,
    },
    Move {
        pick: usize,
        by: Point,
        turn: f64,
    },
    Flip {
        pick: usize,
    },
    MoveTwo {
        pick: usize,
        by: Point,
    },
    /// Between two pads of a net, in steps as the Route tool makes them.
    Link {
        net: usize,
        a: usize,
        b: usize,
        layer: usize,
        past: Past,
    },
    /// From a via of a net to a pad of it.
    FromVia {
        net: usize,
        via: usize,
        pad: usize,
        layer: usize,
        past: Past,
    },
    Straight {
        net: usize,
        layer: usize,
        from: Point,
        to: Point,
        width: f64,
    },
    /// An arc: tracks are straight, so it is a run of chords, as an import
    /// makes it.
    Arc {
        net: usize,
        layer: usize,
        centre: Point,
        radius: f64,
        from: f64,
        sweep: f64,
    },
    /// To a place, through a via there, and on.
    Change {
        net: usize,
        pad: usize,
        at: Point,
        to: Point,
        layer: usize,
    },
    Via {
        net: usize,
        at: Point,
        drill: f64,
        tented: bool,
        capped: bool,
    },
    Drag {
        pick: usize,
        by: f64,
        past: Past,
    },
    MoveVia {
        pick: usize,
        by: Point,
    },
    DeleteVia {
        pick: usize,
    },
    DeleteTrack {
        pick: usize,
    },
    DeleteSeveral {
        pick: usize,
        count: usize,
    },
    Width {
        pick: usize,
        width: f64,
    },
    Cap {
        pick: usize,
        capped: bool,
    },
    Tent {
        pick: usize,
        tented: bool,
    },
    LockVia {
        pick: usize,
        locked: bool,
    },
    LockTrack {
        pick: usize,
        locked: bool,
    },
    Pour(Pour),
    Repour {
        pick: usize,
        priority: u32,
        clearance: Option<f64>,
    },
    DeletePour {
        pick: usize,
    },
    Refill,
    Keepout {
        from: Point,
        to: Point,
        layer: Option<usize>,
    },
    DeleteKeepout {
        pick: usize,
    },
    Rule {
        object: CopperObject,
        mm: f64,
    },
    Undo {
        steps: usize,
    },
    Redo {
        steps: usize,
    },
    Reopen,
    /// A symbol copied and pasted: a new part, not on the board yet.
    Paste {
        pick: usize,
    },
    /// What must be refused, whatever the board.
    Wrong(usize),
}

/// One thing a board is made of, or one edit of it.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Atom {
    Cut(Cut),
    Rule {
        object: CopperObject,
        mm: f64,
    },
    /// A class of one net's own, with its clearance and that to fills.
    Class {
        net: usize,
        clearance: f64,
        fill: Option<f64>,
    },
    Part {
        part: usize,
        turn: f64,
        side: Layer,
        datum: bool,
    },
    Pad {
        part: usize,
        spot: usize,
        form: Form,
        turn: f64,
        mask: Opening,
        paste: Opening,
    },
    Print {
        part: usize,
        spot: usize,
        print: Print,
        far: bool,
    },
    /// A pad's terminal on a net.
    Join {
        part: usize,
        spot: usize,
        net: usize,
    },
    /// Copper of the board at its start.
    Copper(Edit),
    /// An edit of the board that is made.
    Edit(Edit),
}

/// A board and its edits.
#[derive(Clone, Debug)]
pub(super) struct Case {
    pub seed: u64,
    pub layers: usize,
    /// Each atom with its number in the seed's whole list.
    pub atoms: Vec<(usize, Atom)>,
}

/// The nets a board may have.
pub(super) const NETS: usize = 4;
/// Edits of a case.
const EDITS: usize = 10;

impl Case {
    /// The whole case of `seed`: two, four or six layers, by the seed.
    pub(super) fn of(seed: u64) -> Self {
        let layers = [2, 4, 6][(seed % 3) as usize];
        let rng = &mut Rng::new(seed);
        let mut atoms = Vec::new();
        for cut in CUTS {
            if rng.chance(5) {
                atoms.push(Atom::Cut(cut));
            }
        }
        for object in [
            CopperObject::Track,
            CopperObject::Pad,
            CopperObject::Via,
            CopperObject::Fill,
            CopperObject::BoardEdge,
            CopperObject::ViaHole,
            CopperObject::PthHole,
            CopperObject::NpthHole,
        ] {
            if rng.chance(4) {
                atoms.push(Atom::Rule {
                    object,
                    mm: rng.steps(0.2, 0.45, 0.01),
                });
            }
        }
        if rng.chance(6) {
            atoms.push(Atom::Class {
                net: rng.below(NETS),
                clearance: rng.steps(0.2, 0.3, 0.01),
                fill: rng.chance(5).then(|| rng.steps(0.2, 0.5, 0.01)),
            });
        }
        for part in 0..2 + rng.below(3) {
            atoms.push(Atom::Part {
                part,
                turn: rng.turn(),
                side: rng.side(),
                datum: rng.chance(3),
            });
            for spot in 0..9 {
                if !rng.chance(6) {
                    continue;
                }
                let Some(form) = rng.pick(&FORMS) else {
                    continue;
                };
                let opening = |rng: &mut Rng| match rng.below(6) {
                    0 => Opening::Own,
                    1 => Opening::Closed,
                    _ => Opening::Board,
                };
                atoms.push(Atom::Pad {
                    part,
                    spot,
                    form,
                    turn: rng.turn(),
                    mask: opening(rng),
                    paste: opening(rng),
                });
                if form.lands() && rng.chance(8) {
                    atoms.push(Atom::Join {
                        part,
                        spot,
                        net: rng.below(NETS),
                    });
                }
            }
            for print in PRINTS {
                if rng.chance(4) {
                    atoms.push(Atom::Print {
                        part,
                        spot: rng.below(9),
                        print,
                        far: rng.chance(3),
                    });
                }
            }
        }
        if rng.chance(5) {
            atoms.push(Atom::Copper(keepout(rng, layers)));
        }
        for _ in 0..8 + rng.below(6) {
            atoms.push(Atom::Copper(copper(rng, layers)));
        }
        for _ in 0..2 + rng.below(3) {
            atoms.push(Atom::Copper(Edit::Pour(pour(rng, layers))));
        }
        for k in 0..EDITS {
            // The last kind goes by the seed, so every kind is tried.
            let kind = match k + 1 == EDITS {
                true => (seed % KINDS as u64) as usize,
                false => rng.below(KINDS),
            };
            let edit = edit(rng, layers, kind);
            atoms.push(Atom::Edit(edit));
        }
        Self {
            seed,
            layers,
            atoms: atoms.into_iter().enumerate().collect(),
        }
    }

    /// The case with only the atoms of these numbers.
    pub(super) fn only(&self, numbers: &[usize]) -> Self {
        Self {
            atoms: self
                .atoms
                .iter()
                .filter(|(k, _)| numbers.contains(k))
                .cloned()
                .collect(),
            ..*self
        }
    }

    pub(super) fn numbers(&self) -> Vec<usize> {
        self.atoms.iter().map(|(k, _)| *k).collect()
    }

    /// What the reproduction variable is to say: `17` or `17:0-3,9`.
    pub(super) fn name(&self) -> String {
        let mut runs: Vec<(usize, usize)> = Vec::new();
        for k in self.numbers() {
            match runs.last_mut() {
                Some((_, last)) if *last + 1 == k => *last = k,
                _ => runs.push((k, k)),
            }
        }
        let runs: Vec<String> = runs
            .iter()
            .map(|(a, b)| match a == b {
                true => a.to_string(),
                false => format!("{a}-{b}"),
            })
            .collect();
        format!("{}:{}", self.seed, runs.join(","))
    }

    /// The case [`Self::name`] names; `seed` alone is the whole case.
    pub(super) fn named(name: &str) -> super::Fallible<Self> {
        let (seed, numbers) = match name.split_once(':') {
            Some((seed, numbers)) => (seed, Some(numbers)),
            None => (name, None),
        };
        let case = Self::of(seed.trim().parse()?);
        let Some(numbers) = numbers else {
            return Ok(case);
        };
        let mut kept = Vec::new();
        for run in numbers.split(',').filter(|r| !r.trim().is_empty()) {
            let (a, b) = run.split_once('-').unwrap_or((run, run));
            kept.extend(a.trim().parse::<usize>()?..=b.trim().parse::<usize>()?);
        }
        Ok(case.only(&kept))
    }
}

fn past(rng: &mut Rng) -> Past {
    match rng.below(4) {
        0 => Past::Straight,
        1 => Past::Round,
        _ => Past::Shove,
    }
}

fn pour(rng: &mut Rng, layers: usize) -> Pour {
    let (a, b) = (rng.place(), rng.place());
    Pour {
        net: rng.below(NETS),
        free: rng.below(12) == 0,
        layer: rng.below(layers),
        plane: rng.chance(4),
        from: [a[0].min(b[0]) - 2.0, a[1].min(b[1]) - 2.0],
        to: [a[0].max(b[0]) + 2.0, a[1].max(b[1]) + 2.0],
        priority: rng.below(3) as u32,
        clearance: rng.chance(3).then(|| rng.steps(0.2, 0.6, 0.01)),
        islands: rng.chance(5),
        solid: rng.chance(4),
        hatched: rng.below(8) == 0,
    }
}

fn keepout(rng: &mut Rng, layers: usize) -> Edit {
    let at = rng.place();
    Edit::Keepout {
        from: at,
        to: [
            at[0] + rng.steps(1.0, 4.0, 0.1),
            at[1] + rng.steps(1.0, 4.0, 0.1),
        ],
        layer: rng.chance(5).then(|| rng.below(layers)),
    }
}

/// Copper a board starts with.
fn copper(rng: &mut Rng, layers: usize) -> Edit {
    let net = rng.below(NETS);
    let layer = rng.below(layers);
    match rng.below(10) {
        0..=3 => Edit::Link {
            net,
            a: rng.below(8),
            b: rng.below(8),
            layer,
            past: past(rng),
        },
        4 | 5 => Edit::Via {
            net,
            at: rng.place(),
            drill: rng.pick(&[0.2, 0.3, 0.35]).unwrap_or(0.3),
            tented: rng.chance(6),
            capped: rng.chance(3),
        },
        6 => Edit::FromVia {
            net,
            via: rng.below(4),
            pad: rng.below(8),
            layer,
            past: past(rng),
        },
        7 => Edit::Straight {
            net,
            layer,
            from: rng.place(),
            to: rng.place(),
            width: rng.steps(0.15, 0.6, 0.05),
        },
        8 => Edit::Arc {
            net,
            layer,
            centre: rng.place(),
            radius: rng.steps(1.0, 3.0, 0.1),
            from: rng.turn(),
            sweep: rng.steps(40.0, 280.0, 10.0),
        },
        _ => Edit::Change {
            net,
            pad: rng.below(8),
            at: rng.place(),
            to: rng.place(),
            layer,
        },
    }
}

/// The kinds [`edit`] counts, the commoner ones several times.
const KINDS: usize = 42;

fn edit(rng: &mut Rng, layers: usize, kind: usize) -> Edit {
    let pick = rng.below(64);
    let by = |rng: &mut Rng| [rng.steps(-4.0, 4.0, 0.25), rng.steps(-4.0, 4.0, 0.25)];
    match kind {
        0..=3 => Edit::Move {
            pick,
            by: by(rng),
            turn: if rng.chance(5) { 0.0 } else { rng.turn() },
        },
        4 | 5 => Edit::Flip { pick },
        6 => Edit::MoveTwo { pick, by: by(rng) },
        7..=14 => copper(rng, layers),
        15..=17 => Edit::Drag {
            pick,
            by: rng.steps(-2.0, 2.0, 0.05),
            past: past(rng),
        },
        18 | 19 => Edit::MoveVia { pick, by: by(rng) },
        20 => Edit::DeleteVia { pick },
        21 => Edit::DeleteTrack { pick },
        22 => Edit::DeleteSeveral {
            pick,
            count: 1 + rng.below(4),
        },
        23 => Edit::Width {
            pick,
            width: rng.steps(0.1, 1.0, 0.05),
        },
        24 => Edit::Cap {
            pick,
            capped: rng.chance(7),
        },
        25 => Edit::Tent {
            pick,
            tented: rng.chance(5),
        },
        26 => Edit::LockVia {
            pick,
            locked: rng.chance(7),
        },
        27 => Edit::LockTrack {
            pick,
            locked: rng.chance(7),
        },
        28 => Edit::Pour(pour(rng, layers)),
        29 => Edit::Repour {
            pick,
            priority: rng.below(3) as u32,
            clearance: rng.chance(5).then(|| rng.steps(0.2, 0.6, 0.01)),
        },
        30 => Edit::DeletePour { pick },
        31 | 32 => Edit::Refill,
        33 => keepout(rng, layers),
        34 => Edit::Rule {
            object: CopperObject::Track,
            mm: rng.steps(0.2, 0.5, 0.01),
        },
        35 => Edit::Undo {
            steps: 1 + rng.below(3),
        },
        36 => Edit::Redo {
            steps: 1 + rng.below(3),
        },
        37 => Edit::Reopen,
        38 => Edit::Paste { pick },
        39 => Edit::Place {
            pick,
            to: rng.place(),
            turn: rng.turn(),
            side: rng.side(),
        },
        40 => Edit::DeleteKeepout { pick },
        _ => Edit::Wrong(rng.below(6)),
    }
}
