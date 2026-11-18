//! A per-import cache of shipped generator outputs. Pad matching selects a
//! mechanical approximation only; it never changes terminals, pads or wiring.
use super::{BTreeMap, Choice, Engine, ModelPlacement, ModelRef, PackageModel, Pad, publish_model};
use sparksmith_core::ids::ContentHash;
use sparksmith_core::library::generated::{self, GeneratorRuntime, expand};
use sparksmith_core::library::model3d::Redistribution;
use sparksmith_core::library::package_fit::land_miss;
use sparksmith_core::library::package_model::PartFileModel;
use sparksmith_core::library::{Hole, HoleShape, PadKind, PadShape};
use sparksmith_core::units::{Nm, PointNm};
use sparksmith_generators::abi::v1;
use sparksmith_io::generator_bundle::GeneratorBundle;
use sparksmith_io::shipped::ShippedLibrary;
use std::sync::Arc;

// A preset identifies the *package*; its lands are one design choice among
// many (designers size lands and pad spans for their process). Matching
// therefore holds pad centres to a package-relative bound, the land
// pattern's overall extent (which carries the body size) moderately, and
// single land sizes only loosely.

/// Least bound on each pad centre's distance from the preset's, after the
/// best common translation and quarter turn: 0.15 mm.
const CENTRE_FLOOR_NM: f64 = 150_000.0;
/// The bound grows to this fraction of the preset's smallest centre-to-centre
/// distance (its pitch, or a two-pad part's span) when that is larger. 15%
/// admits common chip land spans (a 0603 chip's pads 1.3 to 1.65 mm apart).
const CENTRE_FRACTION: f64 = 0.15;
/// The lands' bounding box, along each axis, within this fraction of the
/// preset's: it tells a 0603 chip from an 0805 one whatever their spans.
const EXTENT_FRACTION: f64 = 0.25;
/// Each land dimension within this fraction of the preset's (rejects only
/// absurd matches; the land size is the designer's).
const LAND_FRACTION: f64 = 0.5;
/// Pad orientation within 0.1° of the preset's (or a quarter turn from it).
const ANGLE_TOLERANCE_MDEG: i64 = 100;
/// Lands of at most this many pads need a preset of the designator's prefix.
const PREFIX_REQUIRED_MAX_PADS: usize = 2;
/// Presets whose centre errors differ by at most 1 µm are equally near.
const TIE_NM: f64 = 1_000.0;

fn extent(pads: &[Pad]) -> [f64; 2] {
    let mut lo = [f64::INFINITY; 2];
    let mut hi = [f64::NEG_INFINITY; 2];
    for pad in pads {
        let b = pad.bounds();
        for axis in 0..2 {
            lo[axis] = lo[axis].min(b.min[axis]);
            hi[axis] = hi[axis].max(b.max[axis]);
        }
    }
    [hi[0] - lo[0], hi[1] - lo[1]]
}
fn loose(imported: f64, preset: f64) -> bool {
    (imported - preset).abs() <= LAND_FRACTION * preset.abs()
}
const MAX_PRESETS: usize = 512;
struct Candidate {
    bundle: Arc<GeneratorBundle>,
    generation: v1::Generation,
    pads: Vec<Pad>,
    name: String,
}
#[derive(Default)]
pub(super) struct Presets {
    candidates: Option<Vec<Candidate>>,
    failure: Option<String>,
    chosen: BTreeMap<usize, Choice>,
    matches: BTreeMap<ContentHash, Result<(usize, ModelPlacement), Miss>>,
}
impl Presets {
    fn load(
        &mut self,
        runtime: &dyn GeneratorRuntime,
        deadline: std::time::Instant,
    ) -> Result<(), String> {
        if let Some(failure) = &self.failure {
            return Err(failure.clone());
        }
        let result = self.build(runtime, deadline);
        if let Err(failure) = &result {
            self.failure = Some(failure.clone());
        }
        result
    }
    fn build(
        &mut self,
        runtime: &dyn GeneratorRuntime,
        deadline: std::time::Instant,
    ) -> Result<(), String> {
        if self.candidates.is_some() {
            return Ok(());
        }
        super::check_time(deadline)?;
        let library = ShippedLibrary::locate().map_err(|e| e.to_string())?;
        let names = library.generators().map_err(|e| e.to_string())?;
        if names.len() > 64 {
            return Err("3D preset search exceeds 64 generators".into());
        }
        let mut candidates = Vec::new();
        let mut count = 0;
        for name in names {
            super::check_time(deadline)?;
            let bundle = Arc::new(library.generator(&name).map_err(|e| e.to_string())?);
            super::check_time(deadline)?;
            let description = runtime
                .describe(&bundle.module)
                .map_err(|e| e.to_string())?;
            super::check_time(deadline)?;
            if !description.scopes().contains(&v1::Scope::Package) {
                continue;
            }
            for preset in description.presets() {
                super::check_time(deadline)?;
                count += 1;
                if count > MAX_PRESETS {
                    return Err("3D preset search exceeds 512 presets".into());
                }
                let scope = if description.scopes().contains(&v1::Scope::Component) {
                    v1::Scope::Component
                } else {
                    v1::Scope::Package
                };
                let request = generated::complete(&description, scope, &preset.params)
                    .map_err(|e| e.to_string())?;
                super::check_time(deadline)?;
                let generation = runtime
                    .generate(&bundle.module, &request)
                    .map_err(|e| e.to_string())?
                    .generation;
                super::check_time(deadline)?;
                let pads = expand::land_pattern(&generation).map_err(|e| e.to_string())?;
                candidates.push(Candidate {
                    bundle: bundle.clone(),
                    generation,
                    pads,
                    name: format!("{name}/{}", preset.key),
                });
            }
        }
        super::check_time(deadline)?;
        self.candidates = Some(candidates);
        Ok(())
    }
    pub(super) fn match_model(
        &mut self,
        engine: &mut Engine,
        pads: &[Pad],
        prefix: &str,
        models: &mut BTreeMap<ContentHash, ModelRef>,
        deadline: std::time::Instant,
    ) -> Result<Result<Choice, Miss>, String> {
        super::check_time(deadline)?;
        if pads.iter().all(|p| p.number.is_empty()) {
            return Ok(Err(Miss::NoLands));
        }
        let runtime = engine
            .generator_runtime()
            .cloned()
            .unwrap_or_else(crate::session::generators::runtime);
        self.load(runtime.as_ref(), deadline)?;
        let Some(candidates) = &self.candidates else {
            return Ok(Err(Miss::NoPreset));
        };
        let mut geometry: Vec<_> = pads
            .iter()
            .map(|p| {
                (
                    &p.number,
                    p.kind,
                    &p.hole,
                    p.side,
                    p.position,
                    p.rotation_mdeg,
                    &p.shape,
                )
            })
            .collect();
        geometry.sort_by(|a, b| a.0.cmp(b.0));
        let key = sparksmith_core::library::bundle::asset_hash(
            &serde_json::to_vec(&(geometry, prefix)).map_err(|e| e.to_string())?,
        );
        let found = if let Some(found) = self.matches.get(&key) {
            found.clone()
        } else {
            let mut fitting = Vec::new();
            let mut nearest: Option<(usize, Fit)> = None;
            let mut off_lands: Option<(usize, String, f64)> = None;
            for (index, candidate) in candidates.iter().enumerate() {
                super::check_time(deadline)?;
                let Some(fit) = fit(pads, &candidate.pads) else {
                    continue;
                };
                if fit.error_nm <= fit.tolerance_nm {
                    // The body's leads, not only its lands, must land: a
                    // preset whose lead contacts miss these lands by the
                    // board check's rule is the wrong body for them.
                    match lead_off_land(pads, candidate, &fit.placement) {
                        None => fitting.push((index, fit)),
                        Some((lead, miss)) => {
                            if off_lands.as_ref().is_none_or(|(_, _, m)| miss < *m) {
                                off_lands = Some((index, lead, miss));
                            }
                        }
                    }
                } else if nearest
                    .as_ref()
                    .is_none_or(|(_, n)| fit.excess() < n.excess())
                {
                    nearest = Some((index, fit));
                }
            }
            let found = if fitting.is_empty() {
                Err(match (off_lands, nearest) {
                    (Some((index, lead, miss_mm)), _) => Miss::LeadOffLand {
                        nearest: candidates[index].name.clone(),
                        lead,
                        miss_mm,
                    },
                    (None, Some((index, fit))) => Miss::NoFit {
                        nearest: candidates[index].name.clone(),
                        error_nm: fit.error_nm,
                        tolerance_nm: fit.tolerance_nm,
                    },
                    (None, None) => Miss::NoPreset,
                })
            } else {
                choose(candidates, fitting, prefix)
            };
            self.matches.insert(key, found.clone());
            found
        };
        let (index, placement) = match found {
            Ok(found) => found,
            Err(miss) => return Ok(Err(miss)),
        };
        if let Some(choice) = self.chosen.get(&index) {
            let mut choice = choice.clone();
            choice.placement = placement;
            return Ok(Ok(choice));
        }
        let candidate = &candidates[index];
        super::check_time(deadline)?;
        let mut made = crate::session::generated_models::generated_model(
            engine,
            &candidate.generation,
            Some(&candidate.bundle),
        )
        .map_err(|e| e.to_string())?;
        super::check_time(deadline)?;
        made.definition.provenance.redistribution = Redistribution::Forbidden;
        let key = made.definition.asset.hash;
        let provenance = format!(
            "Generated from preset {} (bundle sha256 {}). Mechanical approximation of the package: the same numbered lands, each centre within the larger of 0.15 mm and 15% of the preset's pitch, the lands' extent within 25%, each land size within 50%, orientation within 0.1°, every lead contact on its land (the board check's rule); the longest declared reference prefix that begins the designator's narrows the candidates (for two-pad lands one is required), the nearest centres decide. No electrical mapping inferred. Project-only.",
            candidate.name, candidate.bundle.hash
        );
        let reference = if let Some(reference) = models.get(&key) {
            *reference
        } else {
            let bytes = made.data.take().ok_or("Preset model bytes unavailable")?;
            let reference = publish_model(engine, &made.name, made.definition, bytes, &provenance)?;
            models.insert(key, reference);
            reference
        };
        // The model is mechanical only; the import keeps its own terminal IDs.
        // Do not retain the generator's lead-to-terminal bindings.
        made.derived.leads.clear();
        let envelope_nm = super::envelope(made.derived.bounds())?;
        // Each lead contact by the pad number the lands were matched on:
        // mechanical contacts for the board check, never a net or mapping.
        let leads = candidate
            .generation
            .package
            .geometry
            .leads
            .iter()
            .map(|lead| {
                let [x, y, _] = lead.contact;
                Ok((lead.key.clone(), PointNm::new(Nm::new(x)?, Nm::new(y)?)))
            })
            .collect::<Result<Vec<_>, sparksmith_core::Error>>()
            .map_err(|e| e.to_string())?;
        let choice = Choice {
            leads,
            model: PackageModel::PartFile(Box::new(PartFileModel {
                model: reference,
                values: made.values,
                derived: made.derived,
            })),
            placement,
            envelope_nm,
            provenance,
        };
        super::check_time(deadline)?;
        self.chosen.insert(index, choice.clone());
        Ok(Ok(choice))
    }
}

/// Why no preset model was chosen.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Miss {
    /// No numbered lands to compare.
    NoLands,
    /// No preset has these numbered lands of these kinds.
    NoPreset,
    /// Presets with these lands, none within the centre bound.
    NoFit {
        nearest: String,
        error_nm: f64,
        tolerance_nm: f64,
    },
    /// Equally near presets the reference prefix does not tell apart.
    Ambiguous,
    /// Presets fit, but none declares a prefix that begins the designator's.
    PrefixMismatch { nearest: String },
    /// Presets' land centres fit, but a lead of each misses its land by
    /// the board check's rule: the body is wrong for these lands.
    LeadOffLand {
        nearest: String,
        lead: String,
        miss_mm: f64,
    },
}
impl std::fmt::Display for Miss {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoLands => write!(f, "no numbered lands to match a shipped preset"),
            Self::NoPreset => write!(
                f,
                "no shipped preset has these numbered lands (count, numbers, kind, side, hole, shape, sizes and extent)"
            ),
            Self::NoFit {
                nearest,
                error_nm,
                tolerance_nm,
            } => write!(
                f,
                "nearest shipped preset {nearest} has a pad centre {:.3} mm off (limit {:.3} mm)",
                error_nm / 1e6,
                tolerance_nm / 1e6
            ),
            Self::PrefixMismatch { nearest } => write!(
                f,
                "nearest shipped preset {nearest} fits the lands, but its reference prefix does not begin the designator's"
            ),
            Self::Ambiguous => write!(
                f,
                "several shipped presets fit equally and the reference prefix does not choose one"
            ),
            Self::LeadOffLand {
                nearest,
                lead,
                miss_mm,
            } => write!(
                f,
                "nearest shipped preset {nearest} fits the land centres, but its lead {lead} would miss its land by {miss_mm:.3} mm"
            ),
        }
    }
}

/// The longest declared prefix that begins the designator's narrows the
/// fitting presets ("RFB" is a resistor's "R", "FB" a ferrite bead's); the
/// nearest centres decide; a tie is no choice. Two-pad lands (chip
/// resistors, capacitors, inductors, LEDs, diodes) do not tell the body
/// type, so there a designator no fitting preset's prefix begins keeps the
/// placeholder: a wrong-looking body is worse. Multi-pad packages (SOT, SO,
/// QFN) keep their body whatever the designator.
fn choose(
    candidates: &[Candidate],
    fitting: Vec<(usize, Fit)>,
    prefix: &str,
) -> Result<(usize, ModelPlacement), Miss> {
    // The generator supplies this optional appearance hint. It never
    // replaces geometry matching or changes electrical definitions.
    let declared = |index: &usize| {
        candidates[*index]
            .generation
            .component
            .as_ref()
            .map(|c| c.prefix.to_ascii_uppercase())
            .filter(|p| !p.is_empty() && prefix.to_ascii_uppercase().starts_with(p.as_str()))
            .map_or(0, |p| p.len())
    };
    let nearest_of = |pool: &[(usize, Fit)]| {
        pool.iter()
            .map(|(_, f)| f.error_nm)
            .fold(f64::INFINITY, f64::min)
    };
    let longest = fitting.iter().map(|(i, _)| declared(i)).max().unwrap_or(0);
    let two_pad = fitting
        .first()
        .is_some_and(|(i, _)| candidates[*i].pads.len() <= PREFIX_REQUIRED_MAX_PADS);
    if longest == 0 && two_pad {
        let best = nearest_of(&fitting);
        let nearest = fitting
            .iter()
            .find(|(_, f)| f.error_nm <= best)
            .map(|(i, _)| candidates[*i].name.clone())
            .unwrap_or_default();
        return Err(Miss::PrefixMismatch { nearest });
    }
    let pool: Vec<_> = fitting
        .into_iter()
        .filter(|(i, _)| declared(i) == longest)
        .collect();
    // (With `longest == 0`, every fitting multi-pad preset stays.)
    let best = nearest_of(&pool);
    let nearest: Vec<_> = pool
        .into_iter()
        .filter(|(_, f)| f.error_nm <= best + TIE_NM)
        .collect();
    match nearest.as_slice() {
        [(index, fit)] => Ok((*index, fit.placement)),
        _ => Err(Miss::Ambiguous),
    }
}

/// The worst lead of `candidate` placed at `placement` (model frame →
/// these lands' frame) that misses the copper lands of its number by the
/// board check's lead-on-land rule ([`land_miss`]), and by how much (mm);
/// `None` when every lead lands.
fn lead_off_land(
    pads: &[Pad],
    candidate: &Candidate,
    placement: &ModelPlacement,
) -> Option<(String, f64)> {
    let frame = placement.transform();
    let mut worst: Option<(String, f64)> = None;
    for lead in &candidate.generation.package.geometry.leads {
        let [x, y, _] = lead
            .contact
            .map(|v| Nm::new(v).map_or(f64::NAN, Nm::to_mm_f64));
        let q = frame.apply([x, y, 0.0]);
        let o = frame.apply_vector([x, y, 0.0]);
        let miss = pads
            .iter()
            .filter(|p| {
                p.number == lead.key && matches!(p.kind, PadKind::Smd | PadKind::PlatedThrough)
            })
            .map(|p| land_miss(p, [q[0], q[1]], [o[0], o[1]]))
            .reduce(f64::min);
        if let Some(miss) = miss.filter(|m| *m > LEAD_MISS_EPSILON_MM)
            && worst.as_ref().is_none_or(|(_, w)| miss > *w)
        {
            worst = Some((lead.key.clone(), miss));
        }
    }
    worst
}

/// A miss below a nanometre is on the land (rounding).
const LEAD_MISS_EPSILON_MM: f64 = 1e-6;

fn shape_class(shape: &PadShape) -> u8 {
    match shape {
        PadShape::Rect { .. } | PadShape::RoundRect { .. } => 0,
        PadShape::Circle { .. } => 1,
        PadShape::Oblong { .. } => 2,
        PadShape::Custom { .. } => 3,
    }
}
fn same_hole(a: &Option<Hole>, b: &Option<Hole>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => {
            a.plated == b.plated
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

/// How well imported lands fit a preset's at its best pose.
#[derive(Clone, Copy, Debug)]
struct Fit {
    placement: ModelPlacement,
    /// The largest pad-centre distance from the preset's, nanometres.
    error_nm: f64,
    /// The bound on it for this preset, nanometres.
    tolerance_nm: f64,
}
impl Fit {
    fn excess(&self) -> f64 {
        self.error_nm - self.tolerance_nm
    }
}

/// The preset's bound on centre error: the larger of the floor and a
/// fraction of its smallest centre-to-centre distance.
fn tolerance_nm(candidate: &[Pad]) -> f64 {
    let mut least = f64::INFINITY;
    for (k, a) in candidate.iter().enumerate() {
        for b in &candidate[k + 1..] {
            let [ax, ay] = a.position.to_mm_f64();
            let [bx, by] = b.position.to_mm_f64();
            let d = (ax - bx).hypot(ay - by) * 1e6;
            if d > 0.0 {
                least = least.min(d);
            }
        }
    }
    if least.is_finite() {
        CENTRE_FLOOR_NM.max(CENTRE_FRACTION * least)
    } else {
        CENTRE_FLOOR_NM
    }
}

/// Fit numbered physical lands at quarter turns and the best common
/// translation (the centroid's). `None` when the lands are not the same set
/// (count, numbers, kind, side, hole, shape class, orientation, sizes
/// within [`LAND_FRACTION`]); otherwise the nearest pose, however far.
/// Numbering only constrains model orientation; electrical meaning is untouched.
fn fit(imported: &[Pad], candidate: &[Pad]) -> Option<Fit> {
    if imported.len() != candidate.len() || imported.iter().any(|p| p.number.is_empty()) {
        return None;
    }
    let mut by_number = BTreeMap::new();
    for pad in candidate {
        if by_number.insert(pad.number.as_str(), pad).is_some() {
            return None;
        }
    }
    let mut imported_numbers = std::collections::BTreeSet::new();
    let mut pairs = Vec::new();
    for pad in imported {
        if !imported_numbers.insert(pad.number.as_str()) {
            return None;
        }
        let other = by_number.get(pad.number.as_str())?;
        if pad.kind != other.kind
            || !same_hole(&pad.hole, &other.hole)
            || pad.side != other.side
            || shape_class(&pad.shape) == 3
            || shape_class(&other.shape) == 3
            // A through-hole land is a ring round its hole, shaped by
            // convention (a square pin 1); surface land shapes must agree.
            || (pad.hole.is_none() && shape_class(&pad.shape) != shape_class(&other.shape))
        {
            return None;
        }
        pairs.push((pad, *other));
    }
    let count = pairs.len() as f64;
    let tolerance_nm = tolerance_nm(candidate);
    let [ix, iy] = extent(imported);
    let [cx, cy] = extent(candidate);
    let near = |a: f64, b: f64| (a - b).abs() <= EXTENT_FRACTION * b.abs();
    let mut best: Option<Fit> = None;
    for quarter_turn in 0..4 {
        let angle = f64::from(quarter_turn * 90);
        let (c, s) = sparksmith_core::geometry::cos_sin(angle);
        let rotate = |p: [f64; 2]| [c * p[0] - s * p[1], s * p[0] + c * p[1]];
        let spread = if quarter_turn % 2 == 0 {
            near(ix, cx) && near(iy, cy)
        } else {
            near(ix, cy) && near(iy, cx)
        };
        let oriented = spread
            && pairs.iter().all(|(a, b)| {
                let ae = a.shape.extent();
                let be = b.shape.extent();
                let turn = (i64::from(a.rotation_mdeg.millidegrees())
                    - i64::from(b.rotation_mdeg.millidegrees())
                    - i64::from(quarter_turn) * 90_000)
                    .rem_euclid(180_000);
                let same = turn.min(180_000 - turn) <= ANGLE_TOLERANCE_MDEG
                    && loose(ae[0], be[0])
                    && loose(ae[1], be[1]);
                let quarter = (turn - 90_000).abs() <= ANGLE_TOLERANCE_MDEG
                    && loose(ae[0], be[1])
                    && loose(ae[1], be[0]);
                same || quarter
            });
        if !oriented {
            continue;
        }
        let deltas: Vec<[f64; 2]> = pairs
            .iter()
            .map(|(a, b)| {
                let aa = a.position.to_mm_f64();
                let bb = rotate(b.position.to_mm_f64());
                [aa[0] - bb[0], aa[1] - bb[1]]
            })
            .collect();
        let offset = [
            deltas.iter().map(|d| d[0]).sum::<f64>() / count,
            deltas.iter().map(|d| d[1]).sum::<f64>() / count,
        ];
        let error_nm = deltas
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
            });
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use sparksmith_core::library::CornerMask;
    use sparksmith_core::units::{Angle, PointNm};
    use sparksmith_core::{PackageTerminalId, PadId};
    type Fallible = Result<(), Box<dyn std::error::Error>>;

    /// Two numbered lands of `size` mm, `span` mm apart, as round rects.
    fn chip(span: f64, size: [f64; 2]) -> Result<Vec<Pad>, sparksmith_core::Error> {
        [(-0.5 * span, "1"), (0.5 * span, "2")]
            .into_iter()
            .enumerate()
            .map(|(n, (x, number))| {
                let mut pad = Pad::smd_rect(
                    PadId(n as u32 + 1),
                    PackageTerminalId(n as u32 + 1),
                    number,
                    PointNm::from_mm_f64_rounded([x, 0.0])?,
                    PointNm::from_mm_f64_rounded(size)?,
                );
                pad.shape = PadShape::round_rect_mm(size, 0.25, None, CornerMask::ALL)?;
                Ok(pad)
            })
            .collect()
    }
    fn accepted(imported: &[Pad], preset: &[Pad]) -> bool {
        fit(imported, preset).is_some_and(|f| f.error_nm <= f.tolerance_nm)
    }
    fn candidates(presets: &Presets) -> Result<&Vec<Candidate>, String> {
        presets
            .candidates
            .as_ref()
            .ok_or_else(|| "No presets".to_owned())
    }
    fn lands_of(presets: &Presets, name: &str) -> Result<Vec<Pad>, String> {
        Ok(candidates(presets)?
            .iter()
            .find(|c| c.name == name)
            .ok_or_else(|| format!("No {name} preset"))?
            .pads
            .clone())
    }
    fn loaded() -> Result<Presets, String> {
        let mut presets = Presets::default();
        let runtime = crate::session::generators::fixture_runtime();
        presets.load(
            runtime.as_ref(),
            std::time::Instant::now() + std::time::Duration::from_secs(120),
        )?;
        Ok(presets)
    }

    #[test]
    fn a_fit_finds_the_pose_and_bounds_centres_relative_to_the_package() -> Fallible {
        let source = chip(1.6, [0.7, 0.8])?;
        let mut moved = source.clone();
        for pad in &mut moved {
            let p = pad.position.to_mm_f64();
            pad.position = PointNm::from_mm_f64_rounded([-p[1] + 3.0, p[0] + 4.0])?;
            pad.rotation_mdeg = Angle::from_degrees_f64_rounded(90.0)?;
        }
        let pose = fit(&moved, &source).ok_or("rotated fit missing")?;
        assert_eq!(pose.placement.translation(), [3.0, 4.0, 0.0]);
        assert_eq!(pose.placement.rotation_deg(), [0.0, 0.0, 90.0]);
        assert!(pose.error_nm < 1.0);
        // The bound is the larger of 0.15 mm and 15% of the 1.6 mm span:
        // 0.24 mm per pad, so the span may differ by up to 0.48 mm.
        assert!((pose.tolerance_nm - 240_000.0).abs() < 1.0);
        let [x, y] = moved[1].position.to_mm_f64();
        moved[1].position = PointNm::from_mm_f64_rounded([x, y + 0.47])?;
        assert!(
            accepted(&moved, &source),
            "a wider span of the same package"
        );
        moved[1].position = PointNm::from_mm_f64_rounded([x, y + 0.49])?;
        assert!(!accepted(&moved, &source), "beyond the centre bound");
        moved[1].position = PointNm::from_mm_f64_rounded([x, y])?;
        moved[1].rotation_mdeg = Angle::from_degrees_f64_rounded(90.1)?;
        assert!(accepted(&moved, &source), "inclusive 0.1 degree limit");
        moved[1].rotation_mdeg = Angle::from_degrees_f64_rounded(90.101)?;
        assert!(
            fit(&moved, &source).is_none(),
            "one millidegree beyond limit"
        );
        let mut duplicate = source.clone();
        duplicate[1].number = "1".into();
        assert!(fit(&duplicate, &source).is_none());
        // Land sizes only loosely: a third larger still fits, twice not.
        assert!(accepted(&chip(1.6, [0.95, 0.95])?, &source));
        assert!(fit(&chip(1.6, [1.4, 1.6])?, &source).is_none());
        Ok(())
    }

    /// Designers' own lands for one package: different spans and land sizes
    /// identify the same chip; an 0603 and an 0805 never cross.
    #[test]
    fn own_land_variants_identify_the_chip_package_and_never_the_next_size() -> Fallible {
        let presets = loaded()?;
        let r0603 = lands_of(&presets, "chip/resistor-0603")?;
        let r0805 = lands_of(&presets, "chip/resistor-0805")?;
        let c0603 = lands_of(&presets, "chip/capacitor-0603")?;
        let c0805 = lands_of(&presets, "chip/capacitor-0805")?;
        for variant in [chip(1.5, [0.85, 0.95])?, chip(1.6, [0.8, 0.9])?] {
            assert!(accepted(&variant, &r0603) && accepted(&variant, &c0603));
            assert!(!accepted(&variant, &r0805) && !accepted(&variant, &c0805));
        }
        for variant in [chip(1.8, [1.0, 1.4])?, chip(1.9, [1.05, 1.5])?] {
            assert!(accepted(&variant, &r0805) && accepted(&variant, &c0805));
            assert!(!accepted(&variant, &r0603) && !accepted(&variant, &c0603));
        }
        assert!(!accepted(&r0603, &r0805) && !accepted(&r0805, &r0603));
        Ok(())
    }

    #[test]
    fn shipped_packages_create_real_models_by_prefix_and_refuse_ties_and_far_lands() -> Fallible {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
        let mut presets = loaded()?;
        let runtime = crate::session::generators::fixture_runtime();
        let mut engine = Engine::demo()?;
        engine.set_generators(runtime);
        let mut models = BTreeMap::new();
        // Own variants pick their package by the reference prefix.
        for (pads, prefix, preset) in [
            (chip(1.5, [0.85, 0.95])?, "R", "chip/resistor-0603"),
            (chip(1.6, [0.8, 0.9])?, "C", "chip/capacitor-0603"),
            (chip(1.9, [1.05, 1.5])?, "R", "chip/resistor-0805"),
            (chip(1.65, [0.8, 0.95])?, "RFB", "chip/resistor-0603"),
            (chip(1.65, [0.8, 0.95])?, "FB", "chip/ferrite-0603"),
        ] {
            let choice = presets
                .match_model(&mut engine, &pads, prefix, &mut models, deadline)?
                .map_err(|miss| format!("{preset}: {miss}"))?;
            assert!(choice.provenance.contains(preset), "{}", choice.provenance);
        }
        // A prefix no fitting preset declares keeps the placeholder: a test
        // point's lands of a 1210 chip's size get no resistor body; an
        // inductor's get the power inductor that declares "L".
        let r0603 = lands_of(&presets, "chip/resistor-0603")?;
        assert!(matches!(
            presets.match_model(&mut engine, &r0603, "?", &mut models, deadline)?,
            Err(Miss::PrefixMismatch { .. })
        ));
        let mut inductor = chip(2.4, [1.0, 3.0])?;
        for pad in &mut inductor {
            let [x, y] = pad.position.to_mm_f64();
            pad.position = PointNm::from_mm_f64_rounded([y, x])?;
            pad.shape = PadShape::round_rect_mm([3.0, 1.0], 0.25, None, CornerMask::ALL)?;
        }
        assert!(
            accepted(&inductor, &lands_of(&presets, "chip/resistor-1210")?),
            "the lands fit a 1210 chip"
        );
        match presets.match_model(&mut engine, &inductor, "TP", &mut models, deadline)? {
            Err(Miss::PrefixMismatch { .. }) => {}
            other => return Err(format!("a TP part got {:?}", other.map(|c| c.provenance)).into()),
        }
        let choice = presets
            .match_model(&mut engine, &inductor, "L", &mut models, deadline)?
            .map_err(|miss| format!("an L part: {miss}"))?;
        assert!(
            choice.provenance.contains("power-inductor/"),
            "{}",
            choice.provenance
        );
        // A multi-pad package keeps its body under an unusual designator.
        let sot = lands_of(&presets, "tabbed/tabbed-sot-23-6")?;
        let choice = presets
            .match_model(&mut engine, &sot, "XQ", &mut models, deadline)?
            .map_err(|miss| format!("SOT-23-6 under XQ: {miss}"))?;
        assert!(choice.provenance.contains("tabbed/tabbed-sot-23-6"));
        for name in [
            "tabbed/tabbed-sot-23-3",
            "gull-wing/gull-wing-soic-8-3.9",
            "no-lead/no-lead-qfn-12-3x3-0.5",
        ] {
            let pads = lands_of(&presets, name)?;
            let choice = presets
                .match_model(&mut engine, &pads, "U", &mut models, deadline)?
                .map_err(|miss| format!("{name}: {miss}"))?;
            assert!(choice.provenance.contains(name));
            let PackageModel::PartFile(part) = &choice.model else {
                return Err("not a part file".into());
            };
            assert!(
                part.derived.bodies >= 3,
                "body plus distinct terminals: {name}"
            );
            assert!(part.derived.bounds()[1][2] > 0.0);
            assert!(
                part.derived.leads.is_empty(),
                "importer must not infer electrical bindings"
            );
            let repeated = presets
                .match_model(&mut engine, &pads, "U", &mut models, deadline)?
                .map_err(|miss| miss.to_string())?;
            assert_eq!(choice.model, repeated.model);
            // One pad moved a millimetre: another package, not this one.
            let mut far = pads.clone();
            let [x, y] = far[0].position.to_mm_f64();
            far[0].position = PointNm::from_mm_f64_rounded([x - 1.0, y])?;
            assert!(
                presets
                    .match_model(&mut engine, &far, "U", &mut models, deadline)?
                    .is_err(),
                "far lands accepted for {name}"
            );
        }
        assert!(
            engine
                .project()
                .library
                .models
                .iter()
                .all(|m| m.definition.provenance.redistribution == Redistribution::Forbidden)
        );
        Ok(())
    }

    /// The body's leads must land, not only its land centres fit: every
    /// matchable shipped preset's leads land on its own lands, at every
    /// quarter turn,
    /// and a land moved off its lead is refused by the board check's rule.
    #[test]
    fn a_preset_whose_leads_miss_the_lands_is_refused() -> Fallible {
        let presets = loaded()?;
        for candidate in candidates(&presets)? {
            // Lands the matcher never takes (custom shapes, repeated
            // numbers) choose no body.
            if candidate.generation.package.geometry.leads.is_empty()
                || fit(&candidate.pads, &candidate.pads).is_none()
            {
                continue;
            }
            for quarter in 0..4 {
                let angle = f64::from(quarter * 90);
                let (c, s) = sparksmith_core::geometry::cos_sin(angle);
                let mut turned = candidate.pads.clone();
                for pad in &mut turned {
                    let [x, y] = pad.position.to_mm_f64();
                    pad.position = PointNm::from_mm_f64_rounded([c * x - s * y, s * x + c * y])?;
                    let r = pad.rotation_mdeg.millidegrees() + quarter * 90_000;
                    pad.rotation_mdeg = Angle::normalized(i64::from(r));
                }
                let pose = fit(&turned, &candidate.pads).ok_or("own lands fit")?;
                assert_eq!(
                    lead_off_land(&turned, candidate, &pose.placement),
                    None,
                    "{} at {angle}°",
                    candidate.name
                );
            }
        }
        let sot = candidates(&presets)?
            .iter()
            .find(|c| c.name == "tabbed/tabbed-sot-23-6")
            .ok_or("SOT-23-6 preset")?;
        let mut moved = sot.pads.clone();
        let [x, y] = moved[0].position.to_mm_f64();
        moved[0].position = PointNm::from_mm_f64_rounded([x + 0.8, y])?;
        let (lead, miss) = lead_off_land(&moved, sot, &ModelPlacement::default())
            .ok_or("a land moved off its lead")?;
        assert_eq!(lead, "1");
        assert!(miss > 0.0);
        Ok(())
    }

    #[test]
    fn injected_busy_runtime_refuses_without_caching_partial_candidates_or_publishing() -> Fallible
    {
        use std::sync::atomic::{AtomicUsize, Ordering};
        struct Busy {
            runtime: Arc<dyn GeneratorRuntime + Send + Sync>,
            generations: AtomicUsize,
        }
        impl GeneratorRuntime for Busy {
            fn describe(
                &self,
                module: &[u8],
            ) -> Result<v1::Description, sparksmith_generators::abi::Failure> {
                self.runtime.describe(module)
            }
            fn generate(
                &self,
                module: &[u8],
                request: &v1::Request,
            ) -> Result<sparksmith_generators::abi::Generated, sparksmith_generators::abi::Failure>
            {
                if self.generations.fetch_add(1, Ordering::Relaxed) == 0 {
                    self.runtime.generate(module, request)
                } else {
                    Err(sparksmith_generators::abi::Failure::one(
                        "generator.busy",
                        "Synthetic generator busy after one successful preset; retry later".into(),
                    ))
                }
            }
        }
        let mut presets = Presets::default();
        let mut engine = Engine::demo()?;
        let runtime = Arc::new(Busy {
            runtime: crate::session::generators::fixture_runtime(),
            generations: AtomicUsize::new(0),
        });
        engine.set_generators(runtime.clone());
        let revision = engine.revision();
        let mut models = BTreeMap::new();
        let error = presets
            .match_model(
                &mut engine,
                &chip(1.6, [0.7, 0.8])?,
                "R",
                &mut models,
                std::time::Instant::now() + std::time::Duration::from_secs(120),
            )
            .err()
            .ok_or("busy runtime was converted into a placeholder")?;
        assert!(error.contains("busy"), "{error}");
        assert_eq!(runtime.generations.load(Ordering::Relaxed), 2);
        assert!(presets.candidates.is_none());
        assert!(presets.matches.is_empty() && presets.chosen.is_empty() && models.is_empty());
        assert_eq!(engine.revision(), revision);
        assert!(engine.project().library.models.is_empty());
        Ok(())
    }
}
