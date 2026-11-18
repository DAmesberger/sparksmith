//! Lands as package matching sees them, and their fit to a preset's.
//!
//! The lands a designer draws for one package vary beyond sizes and spans:
//! an exposed pad split into numbered sub-pads (`41_1` … `41_9`) with
//! thermal vias between them, leading zeros (`01`), mounting tabs named `S1`
//! or `MP`, alignment holes left unnumbered, a two-terminal part's lands
//! named `P` and `N`, a standoff's land drawn as one plated pad. Matching
//! therefore compares *lands*: keyed by their number's base, grouped, with
//! pads that do not carry the body (thermal vias) left out, and lands whose
//! names do not agree paired by geometry. Nothing here changes a terminal,
//! a pad or an electrical mapping; it only decides which model fits.
use super::{BTreeMap, ModelPlacement, Pad};
use sparksmith_core::library::{Hole, HoleShape, PadKind, PadShape};

/// The lands' bounding box, along each axis, within this fraction of the
/// preset's: it tells a 0603 chip from an 0805 one whatever their spans.
const EXTENT_FRACTION: f64 = 0.25;
/// Each land dimension within this fraction of the preset's (rejects only
/// absurd matches; the land size is the designer's).
const LAND_FRACTION: f64 = 0.5;
/// A plated round pad stands for a land round a non-plated hole when its
/// outer and hole diameters are each within this fraction of the preset's.
pub(super) const RING_FRACTION: f64 = 0.1;
/// Pad orientation within 0.1° of the preset's (or a quarter turn from it).
const ANGLE_TOLERANCE_MDEG: i64 = 100;
/// A hole is concentric with a land within 1 µm.
const CONCENTRIC_MM: f64 = 0.001;
/// Lands paired by position, at most this many in one set (a pose search
/// over more would cost more than the match is worth).
const MAX_PAIRED_BY_POSITION: usize = 64;

/// What a land's label says, after its sub-pad suffix: a number (leading
/// zeros dropped), a name, or nothing.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Key {
    Number(String),
    Name(String),
    None,
}
impl Key {
    /// `41_3` is a sub-pad of land `41`, `EP_1` of `EP`; `01` is `1`.
    pub(super) fn of(label: &str) -> Self {
        let label = label.trim();
        let base = match label.rsplit_once('_') {
            Some((base, suffix))
                if !base.is_empty()
                    && !suffix.is_empty()
                    && suffix.bytes().all(|b| b.is_ascii_digit()) =>
            {
                base
            }
            _ => label,
        };
        if base.is_empty() {
            Self::None
        } else if base.bytes().all(|b| b.is_ascii_digit()) {
            let trimmed = base.trim_start_matches('0');
            Self::Number(if trimmed.is_empty() { "0" } else { trimmed }.to_owned())
        } else {
            Self::Name(base.to_uppercase())
        }
    }
}

/// A two-terminal part's polarity: positive (`+`, anode) or negative.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Polarity {
    Positive,
    Negative,
}
impl Polarity {
    /// The general conventions for a polar terminal's name: `+`/`-`,
    /// `P`/`N` (positive, negative), `A`/`K` (anode, cathode) and the words.
    pub(super) fn of(name: &str) -> Option<Self> {
        match name.trim().to_uppercase().as_str() {
            "+" | "P" | "A" | "POS" | "PLUS" | "ANODE" => Some(Self::Positive),
            "-" | "\u{2212}" | "N" | "K" | "NEG" | "MINUS" | "CATHODE" => Some(Self::Negative),
            _ => None,
        }
    }
    fn sign(self) -> &'static str {
        match self {
            Self::Positive => "+",
            Self::Negative => "\u{2212}",
        }
    }
}

/// How a land relates to a hole through it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ring {
    /// Not a round land round a round hole.
    No,
    /// A plated round pad with a round hole.
    Plated,
    /// A surface round land with a concentric non-plated hole (a standoff's
    /// or press-fit part's land).
    Unplated,
}

/// One land: a pad, or a ring merged from a land and its hole.
#[derive(Clone, Debug)]
pub(super) struct Land {
    key: Key,
    label: String,
    pad: Pad,
    ring: Ring,
    polarity: Option<Polarity>,
}

/// The lands of a footprint (imported or a preset's), ready to fit.
#[derive(Clone, Debug)]
pub(super) struct Lands {
    lands: Vec<Land>,
    extent: [f64; 2],
}
impl Lands {
    /// `polarity` names a pad's polarity by its label, when its part
    /// declares one.
    pub(super) fn new(pads: &[Pad], polarity: impl Fn(&str) -> Option<Polarity>) -> Self {
        let mut surface: BTreeMap<Key, Vec<&Pad>> = BTreeMap::new();
        for pad in pads.iter().filter(|p| p.kind == PadKind::Smd) {
            surface.entry(Key::of(&pad.number)).or_default().push(pad);
        }
        let kept: Vec<&Pad> = pads.iter().filter(|p| !thermal_via(p, &surface)).collect();
        let holes: Vec<usize> = (0..kept.len())
            .filter(|&j| kept[j].kind == PadKind::NonPlatedHole)
            .collect();
        let mut used = vec![false; kept.len()];
        let mut lands = Vec::with_capacity(kept.len());
        for (i, pad) in kept.iter().enumerate() {
            if used[i] {
                continue;
            }
            let mut pad = (*pad).clone();
            let mut ring = Ring::No;
            if is_round(&pad) && pad.hole.is_none() && pad.kind != PadKind::NonPlatedHole {
                // A surface land and the non-plated hole through its centre.
                if let Some(&j) = holes
                    .iter()
                    .find(|&&j| !used[j] && concentric_hole(kept[j], &pad).is_some())
                {
                    pad.hole = concentric_hole(kept[j], &pad);
                    used[j] = true;
                    ring = Ring::Unplated;
                }
            } else if pad.kind == PadKind::PlatedThrough
                && is_round(&pad)
                && matches!(
                    pad.hole,
                    Some(Hole {
                        shape: HoleShape::Round { .. },
                        plated: true,
                        ..
                    })
                )
            {
                ring = Ring::Plated;
            }
            used[i] = true;
            lands.push(Land {
                key: Key::of(&pad.number),
                label: pad.number.clone(),
                polarity: polarity(&pad.number),
                pad,
                ring,
            });
        }
        let extent = extent(lands.iter().map(|l| &l.pad));
        Self { lands, extent }
    }
    pub(super) fn is_empty(&self) -> bool {
        self.lands.is_empty()
    }
    /// The bound on centre error for this preset: the larger of `floor` and
    /// `fraction` of its smallest centre-to-centre distance.
    pub(super) fn tolerance_nm(&self, floor: f64, fraction: f64) -> f64 {
        let mut least = f64::INFINITY;
        for (k, a) in self.lands.iter().enumerate() {
            for b in &self.lands[k + 1..] {
                let [ax, ay] = a.pad.position.to_mm_f64();
                let [bx, by] = b.pad.position.to_mm_f64();
                let d = (ax - bx).hypot(ay - by) * 1e6;
                if d > 0.0 {
                    least = least.min(d);
                }
            }
        }
        if least.is_finite() {
            floor.max(fraction * least)
        } else {
            floor
        }
    }
    /// The lands' count, for the two-pad rule.
    pub(super) fn len(&self) -> usize {
        self.lands.len()
    }
}

fn is_round(pad: &Pad) -> bool {
    matches!(pad.shape, PadShape::Circle { .. })
}
fn round_hole(pad: &Pad) -> Option<f64> {
    match pad.hole.as_ref()?.shape {
        HoleShape::Round { diameter_nm } => Some(diameter_nm.to_mm_f64()),
        HoleShape::Slot { .. } => None,
    }
}
/// `hole`'s non-plated round hole, when it lies at `land`'s centre and
/// inside it.
fn concentric_hole(hole: &Pad, land: &Pad) -> Option<Hole> {
    if hole.kind != PadKind::NonPlatedHole {
        return None;
    }
    let found = hole.hole?;
    let diameter = round_hole(hole)?;
    let [hx, hy] = hole.position.to_mm_f64();
    let [lx, ly] = land.position.to_mm_f64();
    let offset = found.offset.to_mm_f64();
    let concentric = (hx + offset[0] - lx).hypot(hy + offset[1] - ly) <= CONCENTRIC_MM;
    (concentric && diameter < land.shape.extent()[0] && !found.plated).then_some(Hole {
        offset: Default::default(),
        ..found
    })
}

/// A plated hole that only stitches a surface land of the same base number
/// to other layers (a thermal via): smaller than every such land and within
/// their joint bounds. It carries no part of the body.
fn thermal_via(pad: &Pad, surface: &BTreeMap<Key, Vec<&Pad>>) -> bool {
    if pad.kind != PadKind::PlatedThrough {
        return false;
    }
    let key = Key::of(&pad.number);
    let Some(lands) = surface.get(&key).filter(|_| key != Key::None) else {
        return false;
    };
    let size = pad.shape.extent();
    let largest = size[0].max(size[1]);
    if lands.iter().any(|l| {
        let e = l.shape.extent();
        e[0].min(e[1]) <= largest
    }) {
        return false;
    }
    let [lo, hi] = bounds(lands.iter().copied());
    let [x, y] = pad.position.to_mm_f64();
    let r = 0.5 * largest;
    x >= lo[0] - r && x <= hi[0] + r && y >= lo[1] - r && y <= hi[1] + r
}

fn bounds<'a>(pads: impl Iterator<Item = &'a Pad>) -> [[f64; 2]; 2] {
    let mut lo = [f64::INFINITY; 2];
    let mut hi = [f64::NEG_INFINITY; 2];
    for pad in pads {
        let b = pad.bounds();
        for axis in 0..2 {
            lo[axis] = lo[axis].min(b.min[axis]);
            hi[axis] = hi[axis].max(b.max[axis]);
        }
    }
    [lo, hi]
}
fn extent<'a>(pads: impl Iterator<Item = &'a Pad>) -> [f64; 2] {
    let [lo, hi] = bounds(pads);
    [hi[0] - lo[0], hi[1] - lo[1]]
}
pub(super) fn loose(imported: f64, preset: f64) -> bool {
    (imported - preset).abs() <= LAND_FRACTION * preset.abs()
}
fn within(imported: f64, preset: f64, fraction: f64) -> bool {
    (imported - preset).abs() <= fraction * preset.abs()
}

fn shape_class(shape: &PadShape) -> u8 {
    match shape {
        // A custom land is compared by its extent, as a rectangle.
        PadShape::Rect { .. } | PadShape::RoundRect { .. } | PadShape::Custom { .. } => 0,
        PadShape::Circle { .. } => 1,
        PadShape::Oblong { .. } => 2,
    }
}
fn same_hole(a: &Option<Hole>, b: &Option<Hole>, plating: bool) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => {
            (!plating || a.plated == b.plated)
                && match (&a.shape, &b.shape) {
                    (HoleShape::Round { diameter_nm: x }, HoleShape::Round { diameter_nm: y }) => {
                        loose(x.to_mm_f64(), y.to_mm_f64())
                    }
                    (
                        HoleShape::Slot {
                            width_nm: w,
                            length_nm: l,
                            ..
                        },
                        HoleShape::Slot {
                            width_nm: v,
                            length_nm: m,
                            ..
                        },
                    ) => loose(w.to_mm_f64(), v.to_mm_f64()) && loose(l.to_mm_f64(), m.to_mm_f64()),
                    _ => false,
                }
        }
        _ => false,
    }
}

/// How two lands may pair, apart from their position and orientation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pairing {
    /// The same kind of land; orientation and sizes still to check.
    Same,
    /// Round lands round a hole of compatible diameters: done.
    Ring { cross: bool },
    /// Mechanical holes of compatible diameters: done.
    Hole,
}

/// Whether `a` (imported) may pair with `b` (preset). `named` lands paired
/// by their labels; others (mechanical, unnamed) pair as a class, where a
/// tab may be surface copper or a mechanical land and an alignment hole
/// plated or not.
fn pairs(a: &Land, b: &Land, named: bool) -> Option<Pairing> {
    if let (Some(x), Some(y)) = (a.polarity, b.polarity)
        && x != y
    {
        return None;
    }
    if a.pad.side != b.pad.side {
        return None;
    }
    if a.ring != Ring::No
        && b.ring != Ring::No
        && (a.ring == Ring::Unplated || b.ring == Ring::Unplated)
    {
        let (Some(ah), Some(bh)) = (round_hole(&a.pad), round_hole(&b.pad)) else {
            return None;
        };
        let fits = within(ah, bh, RING_FRACTION)
            && within(
                a.pad.shape.extent()[0],
                b.pad.shape.extent()[0],
                RING_FRACTION,
            );
        return fits.then_some(Pairing::Ring {
            cross: a.ring != b.ring,
        });
    }
    if !named && a.pad.hole.is_some() && b.pad.hole.is_some() {
        return same_hole(&a.pad.hole, &b.pad.hole, false).then_some(Pairing::Hole);
    }
    let kind = a.pad.kind == b.pad.kind
        || (!named
            && a.pad.kind.copper() == b.pad.kind.copper()
            && a.pad.hole.is_none()
            && b.pad.hole.is_none());
    // A through-hole land is a ring round its hole, shaped by convention
    // (a square pin 1); surface land shapes must agree.
    let shape = a.pad.hole.is_some() || shape_class(&a.pad.shape) == shape_class(&b.pad.shape);
    (kind && shape && same_hole(&a.pad.hole, &b.pad.hole, true)).then_some(Pairing::Same)
}

/// Whether `a`'s land, turned by `quarter_turn`, has `b`'s orientation and
/// (loosely) its size.
fn oriented(a: &Pad, b: &Pad, quarter_turn: i64) -> bool {
    oriented_sizes(
        a.shape.extent(),
        i64::from(a.rotation_mdeg.millidegrees()),
        b.shape.extent(),
        i64::from(b.rotation_mdeg.millidegrees()),
        quarter_turn,
    )
}
fn oriented_sizes(ae: [f64; 2], ar: i64, be: [f64; 2], br: i64, quarter_turn: i64) -> bool {
    let turn = (ar - br - quarter_turn * 90_000).rem_euclid(180_000);
    let same = turn.min(180_000 - turn) <= ANGLE_TOLERANCE_MDEG
        && loose(ae[0], be[0])
        && loose(ae[1], be[1]);
    let quarter =
        (turn - 90_000).abs() <= ANGLE_TOLERANCE_MDEG && loose(ae[0], be[1]) && loose(ae[1], be[0]);
    same || quarter
}

/// How well imported lands fit a preset's at its best pose.
#[derive(Clone, Debug)]
pub(super) struct Fit {
    pub placement: ModelPlacement,
    /// The largest land-centre distance from the preset's, nanometres.
    pub error_nm: f64,
    /// The bound on it for this preset, nanometres.
    pub tolerance_nm: f64,
    /// A plated pad stands for a land round a non-plated hole: the lands
    /// alone do not tell the part, so its name must.
    pub cross_kind: bool,
    /// Lands paired other than by equal labels, in words ("P→1 (+)").
    pub pairing: Vec<String>,
}
impl Fit {
    pub(super) fn excess(&self) -> f64 {
        self.error_nm - self.tolerance_nm
    }
}

/// Groups of land indices by key; unkeyed lands each alone, under `None`.
fn groups(lands: &Lands) -> (BTreeMap<&Key, Vec<usize>>, Vec<usize>) {
    let mut keyed: BTreeMap<&Key, Vec<usize>> = BTreeMap::new();
    let mut loose = Vec::new();
    for (i, land) in lands.lands.iter().enumerate() {
        if land.key == Key::None {
            loose.push(i);
        } else {
            keyed.entry(&land.key).or_default().push(i);
        }
    }
    (keyed, loose)
}

/// The labels of a group, short: `41_1…41_9` or `S1`.
fn labels(lands: &Lands, group: &[usize]) -> String {
    let mut names: Vec<&str> = group
        .iter()
        .map(|&i| lands.lands[i].label.as_str())
        .collect();
    names.sort_unstable();
    names.dedup();
    match names.as_slice() {
        [] => String::new(),
        [""] => "(unnumbered)".into(),
        [one] => (*one).to_owned(),
        [first, .., last] => format!("{first}…{last}"),
    }
}

/// One pose's pairs: (imported centre, preset centre turned) per pair.
struct Posed {
    deltas: Vec<[f64; 2]>,
    cross: bool,
    pairing: Vec<String>,
}

/// Fit imported lands to a preset's at quarter turns and the best common
/// translation. Lands pair by key (number or name); keyed groups of
/// different sizes compare as one land (their bounds); lands whose keys the
/// other side lacks pair by position, as a class, when no number is left
/// unpaired on both sides. `None` when the lands are not the same set;
/// otherwise the nearest pose, however far.
pub(super) fn fit(imported: &Lands, preset: &Lands, tolerance_nm: f64) -> Option<Fit> {
    if imported.is_empty() || preset.is_empty() {
        return None;
    }
    let (ig, il) = groups(imported);
    let (pg, pl) = groups(preset);
    let mut paired = Vec::new();
    let mut ipool = il;
    let mut ppool = pl;
    let mut numbers_left = [false; 2];
    for (key, group) in &ig {
        match pg.get(key) {
            Some(other) => paired.push((group.as_slice(), other.as_slice())),
            None => {
                numbers_left[0] |= matches!(key, Key::Number(_));
                ipool.extend(group);
            }
        }
    }
    for (key, group) in &pg {
        if !ig.contains_key(key) {
            numbers_left[1] |= matches!(key, Key::Number(_));
            ppool.extend(group);
        }
    }
    // Numbering constrains orientation: numbers both sides use differently
    // are another package.
    if (numbers_left[0] && numbers_left[1])
        || ipool.len() != ppool.len()
        || ipool.len() > MAX_PAIRED_BY_POSITION
    {
        return None;
    }
    let near = |a: f64, b: f64| (a - b).abs() <= EXTENT_FRACTION * b.abs();
    let [ix, iy] = imported.extent;
    let [cx, cy] = preset.extent;
    let mut best: Option<Fit> = None;
    for quarter_turn in 0..4_i64 {
        let spread = if quarter_turn % 2 == 0 {
            near(ix, cx) && near(iy, cy)
        } else {
            near(ix, cy) && near(iy, cx)
        };
        if !spread {
            continue;
        }
        let angle = (quarter_turn * 90) as f64;
        let (c, s) = sparksmith_core::geometry::cos_sin(angle);
        let rotate = |p: [f64; 2]| [c * p[0] - s * p[1], s * p[0] + c * p[1]];
        let Some(posed) = pose(
            imported,
            preset,
            &paired,
            &ipool,
            &ppool,
            quarter_turn,
            &rotate,
        ) else {
            continue;
        };
        let count = posed.deltas.len() as f64;
        let offset = [
            posed.deltas.iter().map(|d| d[0]).sum::<f64>() / count,
            posed.deltas.iter().map(|d| d[1]).sum::<f64>() / count,
        ];
        let error_nm = posed
            .deltas
            .iter()
            .map(|d| (d[0] - offset[0]).hypot(d[1] - offset[1]) * 1e6)
            .fold(0.0, f64::max);
        if best.as_ref().is_none_or(|b| error_nm < b.error_nm) {
            // Whole nanometres, so a pose round-trips exactly.
            let offset = offset.map(|v| (v * 1e6).round() / 1e6);
            let placement =
                ModelPlacement::from_mm([offset[0], offset[1], 0.0], [0.0, 0.0, angle]).ok()?;
            best = Some(Fit {
                placement,
                error_nm,
                tolerance_nm,
                cross_kind: posed.cross,
                pairing: posed.pairing,
            });
        }
    }
    best
}

fn centroid(lands: &Lands, group: &[usize], map: &dyn Fn([f64; 2]) -> [f64; 2]) -> [f64; 2] {
    let n = group.len().max(1) as f64;
    let mut sum = [0.0; 2];
    for &i in group {
        let p = map(lands.lands[i].pad.position.to_mm_f64());
        sum[0] += p[0];
        sum[1] += p[1];
    }
    [sum[0] / n, sum[1] / n]
}

/// Pairs at one quarter turn, or `None` when a land finds no partner.
fn pose(
    imported: &Lands,
    preset: &Lands,
    paired: &[(&[usize], &[usize])],
    ipool: &[usize],
    ppool: &[usize],
    quarter_turn: i64,
    rotate: &dyn Fn([f64; 2]) -> [f64; 2],
) -> Option<Posed> {
    let identity = |p: [f64; 2]| p;
    // A first translation: paired groups' centroids, else all lands'.
    let anchor = if paired.is_empty() {
        let all_i: Vec<usize> = (0..imported.lands.len()).collect();
        let all_p: Vec<usize> = (0..preset.lands.len()).collect();
        let a = centroid(imported, &all_i, &identity);
        let b = centroid(preset, &all_p, rotate);
        [a[0] - b[0], a[1] - b[1]]
    } else {
        let mut sum = [0.0; 2];
        for (gi, gp) in paired {
            let a = centroid(imported, gi, &identity);
            let b = centroid(preset, gp, rotate);
            sum[0] += a[0] - b[0];
            sum[1] += a[1] - b[1];
        }
        let n = paired.len() as f64;
        [sum[0] / n, sum[1] / n]
    };
    let mut posed = Posed {
        deltas: Vec::new(),
        cross: false,
        pairing: Vec::new(),
    };
    for (gi, gp) in paired {
        if gi.len() == gp.len() && gi.len() <= MAX_PAIRED_BY_POSITION {
            assign(
                imported,
                preset,
                gi,
                gp,
                true,
                quarter_turn,
                rotate,
                anchor,
                &mut posed,
            )?;
            if gi.len() > 1 {
                posed.pairing.push(format!(
                    "{} to {} by position",
                    labels(imported, gi),
                    labels(preset, gp)
                ));
            }
        } else {
            // One land, or an array, of the same number: compared as one.
            let b = &preset.lands[gp[0]];
            let alike = |lands: &Lands, group: &[usize]| {
                group
                    .iter()
                    .all(|&i| pairs(&lands.lands[i], b, true) == Some(Pairing::Same))
            };
            if !alike(imported, gi) || !alike(preset, gp) {
                return None;
            }
            let [alo, ahi] = bounds(gi.iter().map(|&i| &imported.lands[i].pad));
            let [blo, bhi] = bounds(gp.iter().map(|&i| &preset.lands[i].pad));
            let ae = [ahi[0] - alo[0], ahi[1] - alo[1]];
            let be = [bhi[0] - blo[0], bhi[1] - blo[1]];
            if !oriented_sizes(ae, 0, be, 0, quarter_turn) {
                return None;
            }
            let ac = [0.5 * (alo[0] + ahi[0]), 0.5 * (alo[1] + ahi[1])];
            let bc = rotate([0.5 * (blo[0] + bhi[0]), 0.5 * (blo[1] + bhi[1])]);
            posed.deltas.push([ac[0] - bc[0], ac[1] - bc[1]]);
            posed.pairing.push(format!(
                "{} as one land {} ({} pads for {})",
                labels(imported, gi),
                labels(preset, gp),
                gi.len(),
                gp.len()
            ));
        }
    }
    if !ipool.is_empty() {
        let before = posed.pairing.len();
        assign(
            imported,
            preset,
            ipool,
            ppool,
            false,
            quarter_turn,
            rotate,
            anchor,
            &mut posed,
        )?;
        let polar = ipool.iter().any(|&i| imported.lands[i].polarity.is_some())
            && ppool.iter().any(|&j| preset.lands[j].polarity.is_some());
        let how = if polar {
            "by polarity and position"
        } else {
            "by position"
        };
        for note in &mut posed.pairing[before..] {
            note.push_str(&format!(" ({how})"));
        }
    }
    (!posed.deltas.is_empty()).then_some(posed)
}

/// Pairs equal-sized sets of lands nearest first at the pose.
#[allow(clippy::too_many_arguments)]
fn assign(
    imported: &Lands,
    preset: &Lands,
    from: &[usize],
    to: &[usize],
    named: bool,
    quarter_turn: i64,
    rotate: &dyn Fn([f64; 2]) -> [f64; 2],
    anchor: [f64; 2],
    posed: &mut Posed,
) -> Option<()> {
    let mut options = Vec::with_capacity(from.len() * to.len());
    for &i in from {
        let a = &imported.lands[i];
        let ap = a.pad.position.to_mm_f64();
        for &j in to {
            let b = &preset.lands[j];
            let Some(how) = pairs(a, b, named).filter(|how| match how {
                Pairing::Same => oriented(&a.pad, &b.pad, quarter_turn),
                Pairing::Ring { .. } | Pairing::Hole => true,
            }) else {
                continue;
            };
            let bp = rotate(b.pad.position.to_mm_f64());
            let delta = [ap[0] - bp[0], ap[1] - bp[1]];
            let d = (delta[0] - anchor[0]).hypot(delta[1] - anchor[1]);
            options.push((d, i, j, delta, how));
        }
    }
    options.sort_by(|x, y| x.0.total_cmp(&y.0).then(x.1.cmp(&y.1)).then(x.2.cmp(&y.2)));
    let mut taken_i = std::collections::BTreeSet::new();
    let mut taken_j = std::collections::BTreeSet::new();
    for (_, i, j, delta, how) in options {
        if taken_i.contains(&i) || taken_j.contains(&j) {
            continue;
        }
        taken_i.insert(i);
        taken_j.insert(j);
        posed.deltas.push(delta);
        if let Pairing::Ring { cross: true } = how {
            posed.cross = true;
        }
        let a = &imported.lands[i];
        let b = &preset.lands[j];
        if !named {
            let sign = b
                .polarity
                .map(|p| format!(" ({})", p.sign()))
                .unwrap_or_default();
            let shown = |l: &Land| {
                if l.label.is_empty() {
                    "(unnumbered)".to_owned()
                } else {
                    l.label.clone()
                }
            };
            posed
                .pairing
                .push(format!("{}→{}{sign}", shown(a), shown(b)));
        }
    }
    (taken_i.len() == from.len()).then_some(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sparksmith_core::units::{Angle, PointNm};
    use sparksmith_core::{PackageTerminalId, PadId};
    type Fallible = Result<(), Box<dyn std::error::Error>>;
    type Made = Result<Pad, sparksmith_core::Error>;

    fn smd(number: &str, at: [f64; 2], size: [f64; 2]) -> Made {
        Pad::smd_rect_mm(PadId(1), PackageTerminalId(1), number, at, size)
    }
    fn round(number: &str, at: [f64; 2], diameter: f64) -> Made {
        let mut pad = smd(number, at, [diameter; 2])?;
        pad.shape = PadShape::circle_mm(diameter)?;
        Ok(pad)
    }
    fn hole(diameter: f64, plated: bool) -> Result<Option<Hole>, sparksmith_core::Error> {
        Ok(Some(Hole {
            shape: HoleShape::round_mm(diameter)?,
            plated,
            offset: PointNm::from_mm_f64_rounded([0.0, 0.0])?,
        }))
    }
    fn tht(number: &str, at: [f64; 2], d: f64, drill: f64) -> Made {
        let mut pad = round(number, at, d)?;
        pad.kind = PadKind::PlatedThrough;
        pad.hole = hole(drill, true)?;
        Ok(pad)
    }
    fn npth(number: &str, at: [f64; 2], drill: f64) -> Made {
        let mut pad = round(number, at, drill)?;
        pad.kind = PadKind::NonPlatedHole;
        pad.terminal = None;
        pad.hole = hole(drill, false)?;
        Ok(pad)
    }
    /// The fit of `imported` to `preset` (whose pin 1 is `+` when `polar`),
    /// when within the preset's bound.
    fn fits(imported: &[Pad], preset: &[Pad], polar: bool) -> Option<Fit> {
        let preset = Lands::new(preset, |label| match (polar, label) {
            (true, "1") => Some(Polarity::Positive),
            (true, "2") => Some(Polarity::Negative),
            _ => None,
        });
        let tolerance = preset.tolerance_nm(150_000.0, 0.15);
        fit(&Lands::new(imported, Polarity::of), &preset, tolerance)
            .filter(|f| f.error_nm <= f.tolerance_nm)
    }
    fn turned(pads: &[Pad], quarter_turns: i32) -> Result<Vec<Pad>, sparksmith_core::Error> {
        pads.iter()
            .map(|pad| {
                let mut pad = pad.clone();
                let mut p = pad.position.to_mm_f64();
                for _ in 0..quarter_turns {
                    p = [-p[1], p[0]];
                }
                pad.position = PointNm::from_mm_f64_rounded(p)?;
                pad.rotation_mdeg = Angle::from_degrees_f64_rounded(
                    pad.rotation_mdeg.to_degrees_f64() + 90.0 * f64::from(quarter_turns),
                )?;
                Ok(pad)
            })
            .collect()
    }
    fn renumbered(pads: &[Pad], from: &str, to: &str) -> Vec<Pad> {
        pads.iter()
            .map(|p| {
                let mut p = p.clone();
                p.number = p.number.replace(from, to);
                p
            })
            .collect()
    }

    #[test]
    fn keys_drop_sub_pad_suffixes_and_leading_zeros_only() {
        assert_eq!(Key::of("41_7"), Key::Number("41".into()));
        assert_eq!(Key::of("01"), Key::Number("1".into()));
        assert_eq!(Key::of("000"), Key::Number("0".into()));
        assert_eq!(Key::of("EP_1"), Key::Name("EP".into()));
        assert_eq!(Key::of("sw-a"), Key::Name("SW-A".into()));
        assert_eq!(Key::of("P_MOUNT"), Key::Name("P_MOUNT".into()));
        assert_eq!(Key::of("_3"), Key::Name("_3".into()));
        assert_eq!(Key::of("  "), Key::None);
    }

    /// Four signal lands round an exposed pad, an original synthetic
    /// package.
    fn quad(ep: &[Pad]) -> Result<Vec<Pad>, sparksmith_core::Error> {
        let mut pads = vec![
            smd("1", [-2.0, 0.0], [0.6, 1.2])?,
            smd("2", [0.0, -2.0], [1.2, 0.6])?,
            smd("3", [2.0, 0.0], [0.6, 1.2])?,
            smd("4", [0.0, 2.0], [1.2, 0.6])?,
        ];
        pads.extend_from_slice(ep);
        Ok(pads)
    }

    #[test]
    fn split_exposed_pads_match_one_land_or_an_array_and_thermal_vias_are_left_out() -> Fallible {
        let preset = quad(&[smd("5", [0.0, 0.0], [2.0, 2.0])?])?;
        let mut split = Vec::new();
        for (n, at) in [[-0.5, -0.5], [0.5, -0.5], [-0.5, 0.5], [0.5, 0.5]]
            .into_iter()
            .enumerate()
        {
            split.push(smd(&format!("5_{}", n + 1), at, [0.9, 0.9])?);
        }
        let sub_pads = split.clone();
        // Thermal vias between the sub-pads, numbered like one of them.
        for at in [[0.0, 0.0], [0.0, -0.5], [-0.5, 0.0]] {
            split.push(tht("5_2", at, 0.4, 0.2)?);
        }
        let imported = quad(&split)?;
        let fit = fits(&imported, &preset, false).ok_or("split exposed pad")?;
        assert!(fit.error_nm < 1.0, "{}", fit.error_nm);
        assert!(
            fit.pairing
                .iter()
                .any(|p| p.contains("5_1…5_4 as one land 5")),
            "{:?}",
            fit.pairing
        );
        // A preset array of four lands numbered 5 pairs land by land.
        let array = quad(
            &renumbered(&sub_pads, "5_1", "5")
                .into_iter()
                .map(|mut p| {
                    p.number = "5".into();
                    p
                })
                .collect::<Vec<_>>(),
        )?;
        let fit = fits(&imported, &array, false).ok_or("array")?;
        assert!(fit.error_nm < 1.0);
        // Near misses: sub-pads spread to twice the preset's land; another
        // base number; a plated hole as large as the sub-pads is a pin.
        let mut wide = imported.clone();
        for pad in wide
            .iter_mut()
            .filter(|p| p.kind == PadKind::Smd && p.number.starts_with("5_"))
        {
            let [x, y] = pad.position.to_mm_f64();
            pad.position = PointNm::from_mm_f64_rounded([2.6 * x, 2.6 * y])?;
        }
        assert!(fits(&wide, &preset, false).is_none(), "sub-pads too wide");
        assert!(
            fits(&renumbered(&imported, "5_", "6_"), &preset, false).is_none(),
            "another number"
        );
        let mut pin = imported.clone();
        pin.push(tht("5_2", [0.0, 0.0], 0.9, 0.5)?);
        assert!(fits(&pin, &preset, false).is_none(), "a pin, not a via");
        Ok(())
    }

    /// Two lands 8 mm apart.
    fn pair(left: &str, right: &str) -> Result<Vec<Pad>, sparksmith_core::Error> {
        Ok(vec![
            smd(left, [-4.0, 0.0], [2.5, 3.0])?,
            smd(right, [4.0, 0.0], [2.5, 3.0])?,
        ])
    }

    #[test]
    fn named_two_pad_lands_pair_by_polarity_where_the_preset_declares_it() -> Fallible {
        let preset = pair("1", "2")?;
        // Positive at the right: the model turns half a turn so that its
        // pin 1 lands on P.
        let fit = fits(&pair("N", "P")?, &preset, true).ok_or("P/N")?;
        assert_eq!(fit.placement.rotation_deg()[2], 180.0);
        let by_polarity = |s: &str| format!("{s} (by polarity and position)");
        assert!(
            fit.pairing.contains(&by_polarity("P→1 (+)")),
            "{:?}",
            fit.pairing
        );
        assert!(fit.pairing.contains(&by_polarity("N→2 (\u{2212})")));
        let fit = fits(&pair("A", "K")?, &preset, true).ok_or("A/K")?;
        assert_eq!(fit.placement.rotation_deg()[2], 0.0);
        let fit = fits(&turned(&pair("-", "+")?, 1)?, &preset, true).ok_or("turned")?;
        assert_eq!(fit.placement.rotation_deg()[2], 270.0);
        // No declared polarity: geometry only, the first nearest pose.
        let fit = fits(&pair("N", "P")?, &preset, false).ok_or("geometry only")?;
        assert_eq!(fit.placement.rotation_deg()[2], 0.0);
        assert!(
            fit.pairing.iter().all(|p| p.ends_with("(by position)")),
            "{:?}",
            fit.pairing
        );
        // Near misses: a span far from the preset's; a third land; both
        // lands positive.
        let mut far = pair("N", "P")?;
        far[1].position = PointNm::from_mm_f64_rounded([7.5, 0.0])?;
        assert!(fits(&far, &preset, true).is_none());
        let mut three = pair("N", "P")?;
        three.push(smd("X", [0.0, 3.0], [1.0, 1.0])?);
        assert!(fits(&three, &preset, true).is_none());
        assert!(fits(&pair("P", "+")?, &preset, true).is_none());
        Ok(())
    }

    #[test]
    fn a_plated_round_pad_stands_for_a_land_round_a_non_plated_hole() -> Fallible {
        let preset = vec![round("1", [0.0, 0.0], 7.4)?, npth("hole", [0.0, 0.0], 4.4)?];
        let fit = fits(&[tht("1", [0.0, 0.1], 7.4, 4.4)?], &preset, false).ok_or("ring")?;
        assert!(fit.cross_kind, "the lands alone do not tell the part");
        // Within 10%: 7.0 round 4.1.
        assert!(fits(&[tht("1", [0.0, 0.0], 7.0, 4.1)?], &preset, false).is_some());
        // Near misses: an M3 clearance hole (3.2 mm), a 6 mm land, a hole
        // off the land's centre (no ring).
        assert!(fits(&[tht("1", [0.0, 0.0], 7.4, 3.2)?], &preset, false).is_none());
        assert!(fits(&[tht("1", [0.0, 0.0], 6.0, 4.4)?], &preset, false).is_none());
        let off = vec![round("1", [0.0, 0.0], 7.4)?, npth("hole", [1.0, 0.0], 4.4)?];
        assert!(fits(&[tht("1", [0.0, 0.0], 7.4, 4.4)?], &off, false).is_none());
        // Plated pads on both sides compare as before, not as rings.
        let header = [tht("1", [0.0, 0.0], 1.7, 1.0)?];
        let fit = fits(&[tht("1", [0.0, 0.0], 2.0, 1.0)?], &header, false).ok_or("pin")?;
        assert!(!fit.cross_kind);
        Ok(())
    }

    /// An original connector: four contacts at 1 mm, two tabs behind.
    fn connector(tabs: [&str; 2], numbers: [&str; 4]) -> Result<Vec<Pad>, sparksmith_core::Error> {
        let mut pads = Vec::new();
        for (n, number) in numbers.iter().enumerate() {
            pads.push(smd(number, [n as f64 - 1.5, 0.0], [0.6, 1.55])?);
        }
        pads.push(smd(tabs[0], [-2.8, 3.9], [1.2, 1.8])?);
        pads.push(smd(tabs[1], [2.8, 3.9], [1.2, 1.8])?);
        Ok(pads)
    }

    #[test]
    fn mechanical_tabs_and_holes_pair_as_a_class_whatever_their_names() -> Fallible {
        let mut preset = connector(["MP", "MP"], ["1", "2", "3", "4"])?;
        for pad in preset.iter_mut().filter(|p| p.number == "MP") {
            pad.kind = PadKind::Mechanical;
        }
        let imported = turned(&connector(["S1", "S2"], ["01", "02", "03", "04"])?, 3)?;
        let fit = fits(&imported, &preset, false).ok_or("tabs as a class")?;
        assert_eq!(fit.placement.rotation_deg()[2], 270.0);
        assert!(
            fit.pairing.iter().any(|p| p.starts_with("S1→MP")),
            "{:?}",
            fit.pairing
        );
        // Tabs numbered on after the contacts pair as a class too.
        assert!(
            fits(
                &connector(["5", "6"], ["1", "2", "3", "4"])?,
                &preset,
                false
            )
            .is_some()
        );
        // Near misses: a tab a millimetre off; a third tab; contacts numbered
        // the other way round (pin 1 turns the model, the tabs then miss).
        let mut off = connector(["S1", "S2"], ["1", "2", "3", "4"])?;
        off[5].position = PointNm::from_mm_f64_rounded([3.8, 3.9])?;
        assert!(fits(&off, &preset, false).is_none());
        let mut extra = connector(["S1", "S2"], ["1", "2", "3", "4"])?;
        extra.push(smd("S3", [0.0, 3.9], [1.2, 1.8])?);
        assert!(fits(&extra, &preset, false).is_none());
        let reversed = connector(["S1", "S2"], ["4", "3", "2", "1"])?;
        assert!(fits(&reversed, &preset, false).is_none());
        // Numbers both sides use differently are another package.
        let other = connector(["S1", "S2"], ["1", "2", "3", "7"])?;
        assert!(fits(&other, &preset, false).is_none());
        // An unnumbered non-plated alignment hole pairs with a plated one.
        let mut pegged = preset.clone();
        let mut peg = tht("MP", [0.0, 2.0], 1.2, 1.0)?;
        peg.kind = PadKind::Mechanical;
        pegged.push(peg);
        let mut with_hole = connector(["S1", "S2"], ["1", "2", "3", "4"])?;
        with_hole.push(npth("", [0.0, 2.0], 1.0)?);
        let fit = fits(&with_hole, &pegged, false).ok_or("peg hole")?;
        assert!(
            fit.pairing.iter().any(|p| p.starts_with("(unnumbered)→MP")),
            "{:?}",
            fit.pairing
        );
        let mut wide_hole = connector(["S1", "S2"], ["1", "2", "3", "4"])?;
        wide_hole.push(npth("", [0.0, 2.0], 1.6)?);
        assert!(
            fits(&wide_hole, &pegged, false).is_none(),
            "hole 60% larger"
        );
        Ok(())
    }
}
