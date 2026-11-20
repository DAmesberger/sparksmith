//! The shipped generators' presets, made once per import: which package a
//! source footprint's lands are ([`Presets::identify`]). The part itself is
//! made by the preset's generator with the source's lands as its land
//! pattern ([`super::parts`]), so identifying a package never changes a pad.
use super::lands::{self, Fit, Lands, Polarity};
use super::{BTreeMap, Engine, ModelPlacement, Pad};
use sparksmith_core::ids::ContentHash;
use sparksmith_core::library::PadKind;
use sparksmith_core::library::generated::{self, GeneratorRuntime, expand};
use sparksmith_core::library::package_fit::land_miss;
use sparksmith_core::units::Nm;
use sparksmith_generators::abi::v1;
use sparksmith_io::generator_bundle::GeneratorBundle;
use sparksmith_io::shipped::ShippedLibrary;
use sparksmith_kicad::models::Facts;
use std::sync::Arc;

// A preset identifies the *package*; its lands are one design choice among
// many (designers size lands and pad spans for their process). Matching
// therefore holds land centres to a package-relative bound, the land
// pattern's overall extent (which carries the body size) moderately, and
// single land sizes only loosely ([`lands`] says how lands pair).

/// Least bound on each land centre's distance from the preset's, after the
/// best common translation and quarter turn: 0.15 mm.
const CENTRE_FLOOR_NM: f64 = 150_000.0;
/// The bound grows to this fraction of the preset's smallest centre-to-centre
/// distance (its pitch, or a two-pad part's span) when that is larger. 15%
/// admits common chip land spans (a 0603 chip's pads 1.3 to 1.65 mm apart).
const CENTRE_FRACTION: f64 = 0.15;
/// Lands of at most this many pads need a preset of the designator's prefix
/// or the footprint's name.
const PREFIX_REQUIRED_MAX_PADS: usize = 2;
/// Presets whose centre errors differ by at most 1 µm are equally near.
const TIE_NM: f64 = 1_000.0;
/// A footprint's body outline tells a preset when its extent is within this
/// fraction of the preset's body along each axis ...
const BODY_FRACTION: f64 = 0.15;
/// ... and the next preset's is at least this fraction further off.
const BODY_MARGIN: f64 = 0.02;
/// A name or alias names a preset in a footprint's name only when it has
/// at least this many letters and digits, one of them a digit (so "module"
/// or "size B" never do).
const MIN_NAME_CHARS: usize = 4;

const MAX_PRESETS: usize = 512;
pub(super) struct Candidate {
    pub(super) bundle: Arc<GeneratorBundle>,
    pub(super) generation: v1::Generation,
    /// The generator's name in the shipped library and the preset's key.
    pub(super) generator: String,
    pub(super) preset: String,
    pub(super) scope: v1::Scope,
    /// The preset's own parameters.
    pub(super) params: v1::Params,
    lands: Lands,
    tolerance_nm: f64,
    pub(super) name: String,
    /// The preset's key and aliases, as letters and digits in upper case.
    names: Vec<String>,
    /// The choices the preset makes away from its generator's defaults
    /// (`socket` of a header's style, `black` of a shroud's colour), in
    /// lower case: a variant whose lands are another preset's too.
    departures: Vec<String>,
    /// The body's length and width (the package envelope), millimetres.
    body_mm: [f64; 2],
    /// The courtyard's extent, millimetres.
    courtyard_mm: Option<[f64; 2]>,
}
#[derive(Default)]
pub(super) struct Presets {
    /// The runtime the importer chose (batch admission); else the
    /// engine's, else the process's.
    runtime: Option<Arc<dyn GeneratorRuntime + Send + Sync>>,
    candidates: Option<Vec<Candidate>>,
    failure: Option<String>,
    matches: BTreeMap<ContentHash, Result<Found, Miss>>,
}
/// The package a footprint's lands are: a preset, and how they fit it.
#[derive(Clone, Debug)]
pub(super) struct Found {
    pub(super) index: usize,
    /// The preset's frame in the footprint's: a quarter turn, then an
    /// offset in whole nanometres.
    pub(super) placement: ModelPlacement,
    /// How the lands paired and the preset was told apart, in words.
    pub(super) notes: Vec<String>,
    /// Each footprint pad (by index) with the preset pad its land paired.
    pub(super) pads: Vec<(usize, usize)>,
    /// The preset (`generator/key`).
    pub(super) name: String,
}

/// Letters and digits only, upper case: how names compare.
fn plain(text: &str) -> String {
    text.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

/// The polarity of each of a generation's pads, by label, when the part
/// names its terminals `+` and `−` (or anode and cathode).
fn polarities(generation: &v1::Generation) -> BTreeMap<String, Polarity> {
    let mut out = BTreeMap::new();
    let Some(component) = &generation.component else {
        return out;
    };
    for pad in &generation.footprint.pads {
        let polarity = generation
            .binding
            .iter()
            .find(|b| b.package_terminal == pad.terminal)
            .and_then(|b| component.terminals.iter().find(|t| t.key == b.terminal))
            .and_then(|t| Polarity::of(&t.name));
        if let Some(polarity) = polarity {
            out.insert(
                pad.number.clone().unwrap_or_else(|| pad.key.clone()),
                polarity,
            );
        }
    }
    out
}

fn mm2(v: [i64; 2]) -> [f64; 2] {
    v.map(|n| n as f64 / 1e6)
}

impl Presets {
    /// Presets made with `runtime`, whatever runtime the engine has.
    pub(super) fn with_runtime(runtime: Arc<dyn GeneratorRuntime + Send + Sync>) -> Self {
        Self {
            runtime: Some(runtime),
            ..Self::default()
        }
    }
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
                let polarity = polarities(&generation);
                let lands = Lands::new(&pads, |label| polarity.get(label).copied());
                let tolerance_nm = lands.tolerance_nm(CENTRE_FLOOR_NM, CENTRE_FRACTION);
                let envelope = generation.package.geometry.envelope_nm;
                let courtyard = &generation.footprint.courtyard;
                let courtyard_mm = (!courtyard.is_empty()).then(|| {
                    let lo = [0, 1].map(|a| courtyard.iter().map(|p| p[a]).min().unwrap_or(0));
                    let hi = [0, 1].map(|a| courtyard.iter().map(|p| p[a]).max().unwrap_or(0));
                    mm2([hi[0] - lo[0], hi[1] - lo[1]])
                });
                let departures = description
                    .params()
                    .iter()
                    .filter(|p| matches!(p.ty, v1::ParamType::Enum { .. }))
                    .filter_map(|p| match (preset.params.get(&p.key), &p.default) {
                        (Some(v1::ParamValue::Text(chosen)), v1::ParamValue::Text(default))
                            if chosen != default =>
                        {
                            Some(chosen.to_lowercase())
                        }
                        _ => None,
                    })
                    .collect();
                let names = std::iter::once(&preset.key)
                    .chain(&preset.aliases)
                    .map(|n| plain(n))
                    .filter(|n| n.len() >= MIN_NAME_CHARS && n.bytes().any(|b| b.is_ascii_digit()))
                    .collect();
                candidates.push(Candidate {
                    generator: name.clone(),
                    preset: preset.key.clone(),
                    scope,
                    params: preset.params.clone(),
                    bundle: bundle.clone(),
                    lands,
                    tolerance_nm,
                    name: format!("{name}/{}", preset.key),
                    names,
                    departures,
                    body_mm: mm2([envelope[0], envelope[1]]),
                    courtyard_mm,
                    generation,
                });
            }
        }
        super::check_time(deadline)?;
        self.candidates = Some(candidates);
        Ok(())
    }
    /// The preset `pads` (a footprint's lands) are, or why none is; the
    /// placed part's manufacturer part number `mpn`, when it names one of
    /// the fitting presets, chooses among them.
    pub(super) fn identify(
        &mut self,
        engine: &Engine,
        pads: &[Pad],
        prefix: &str,
        facts: &Facts,
        mpn: Option<&str>,
        deadline: std::time::Instant,
    ) -> Result<Result<Found, Miss>, String> {
        super::check_time(deadline)?;
        if pads.iter().all(|p| p.number.trim().is_empty()) {
            return Ok(Err(Miss::NoLands));
        }
        let runtime = self
            .runtime
            .clone()
            .or_else(|| engine.generator_runtime().cloned())
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
            &serde_json::to_vec(&(
                geometry,
                prefix,
                &facts.name,
                &facts.value,
                &facts.description,
                facts.fab_extent_nm,
                facts.courtyard_extent_nm,
                mpn,
            ))
            .map_err(|e| e.to_string())?,
        );
        let found = if let Some(found) = self.matches.get(&key) {
            found.clone()
        } else {
            let imported = Lands::new(pads, Polarity::of);
            let mut fitting = Vec::new();
            let mut nearest: Option<(usize, Fit)> = None;
            let mut off_lands: Option<(usize, String, f64)> = None;
            for (index, candidate) in candidates.iter().enumerate() {
                super::check_time(deadline)?;
                let Some(fit) = lands::fit(&imported, &candidate.lands, candidate.tolerance_nm)
                else {
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
                choose(candidates, fitting, prefix, facts, mpn)
            };
            self.matches.insert(key, found.clone());
            found
        };
        Ok(found)
    }

    /// The preset `found` names.
    pub(super) fn candidate(&self, index: usize) -> Option<&Candidate> {
        self.candidates.as_ref()?.get(index)
    }

    /// The runtime presets are made with.
    pub(super) fn runtime(&self, engine: &Engine) -> Arc<dyn GeneratorRuntime + Send + Sync> {
        self.runtime
            .clone()
            .or_else(|| engine.generator_runtime().cloned())
            .unwrap_or_else(crate::session::generators::runtime)
    }

    /// When the part number `mpn` names a package other than `found`'s
    /// (another preset's name or alias, standing apart in it), or carries
    /// the matched preset's name run on into a variant of it (`…-1U` for a
    /// `…-1` preset), the finding naming both; the lands are kept as they
    /// are. `None` when the part number agrees or names no shipped package.
    pub(super) fn contradiction(&self, mpn: &str, found: Option<&Found>) -> Option<String> {
        let candidates = self.candidates.as_ref()?;
        let matched = found.and_then(|f| candidates.get(f.index));
        let named_apart: Vec<&Candidate> = candidates
            .iter()
            .filter(|c| c.names.iter().any(|n| names_apart(mpn, n)))
            .collect();
        let lands = |c: &Candidate| c.name.clone();
        if let Some(matched) = matched {
            if named_apart.iter().any(|c| std::ptr::eq(*c, matched)) {
                return None;
            }
            // The longest name of another package the part number carries
            // standing apart (a name the matched package also carries, an
            // alias several packages share like `QFP-64`, names no other),
            // against the longest of the matched package's names it runs
            // on past: the longer tells what the part number names
            // (`SOT-23-6X` runs on past `SOT-23-6`, though `SOT-23` stands
            // apart in it).
            let other = named_apart
                .iter()
                .filter_map(|c| {
                    let n = c
                        .names
                        .iter()
                        .filter(|n| names_apart(mpn, n) && !matched.names.contains(n))
                        .map(String::len)
                        .max()?;
                    Some((*c, n))
                })
                .max_by_key(|(_, n)| *n);
            let variant = matched
                .names
                .iter()
                .filter(|n| runs_on(mpn, n))
                .max_by_key(|n| n.len());
            match (other, variant) {
                (Some((_, n)), Some(name)) if name.len() > n => {
                    return Some(variant_finding(mpn, name, &lands(matched)));
                }
                (Some((other, _)), _) => {
                    return Some(format!(
                        "the part number {mpn} names the package {}, but the lands are the package {}; the lands are kept as the source has them",
                        lands(other),
                        lands(matched)
                    ));
                }
                (None, Some(name)) => return Some(variant_finding(mpn, name, &lands(matched))),
                (None, None) => {}
            }
            return None;
        }
        named_apart.first().map(|c| {
            format!(
                "the part number {mpn} names the package {}, but the lands match no shipped package; the lands are kept as the source has them",
                lands(c)
            )
        })
    }
}

/// The finding for a part number that runs a matched package's name on.
fn variant_finding(mpn: &str, name: &str, package: &str) -> String {
    format!(
        "the part number {mpn} is a variant of {name}, not the package {package} the lands are (its name runs on past {name}); the lands are kept as the source has them"
    )
}

/// Where `name` (a preset name, plain) occurs in `mpn` (plain), each
/// occurrence's end.
fn occurrences(mpn: &str, name: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(at) = mpn.get(from..).and_then(|rest| rest.find(name)) {
        let start = from + at;
        out.push((start, start + name.len()));
        from = start + 1;
    }
    out
}

/// Whether `mpn` names `name` standing apart: at its start, ending where a
/// separator or the end follows in the written part number.
fn names_apart(mpn: &str, name: &str) -> bool {
    let written = written_plain(mpn);
    occurrences(&written.plain, name)
        .iter()
        .any(|&(start, end)| start == 0 && written.boundary_after(end))
}

/// Whether `mpn` begins with `name` run on into more letters or digits
/// with no separator between (`ESP32-S3-WROOM-1U` past `ESP32-S3-WROOM-1`).
fn runs_on(mpn: &str, name: &str) -> bool {
    let written = written_plain(mpn);
    occurrences(&written.plain, name)
        .iter()
        .any(|&(start, end)| {
            start == 0 && end < written.plain.len() && !written.boundary_after(end)
        })
}

/// A part number as letters and digits, with where its separators were.
struct Written {
    plain: String,
    /// For each plain character, whether a separator follows it in the
    /// written text.
    separated: Vec<bool>,
}
impl Written {
    fn boundary_after(&self, end: usize) -> bool {
        end >= self.plain.len() || end == 0 || self.separated.get(end - 1).copied().unwrap_or(true)
    }
}
fn written_plain(text: &str) -> Written {
    let mut plain = String::new();
    let mut separated = Vec::new();
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            plain.push(c.to_ascii_uppercase());
            separated.push(false);
        } else if let Some(last) = separated.last_mut() {
            *last = true;
        }
    }
    Written { plain, separated }
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
    /// Equally near presets that neither the name, the reference prefix
    /// nor the body outline tells apart.
    Tie { presets: Vec<String> },
    /// Presets fit, but none declares a prefix that begins the designator's.
    PrefixMismatch { nearest: String },
    /// A plated pad fits a preset's land round a non-plated hole, but the
    /// footprint's name does not name the preset.
    Unnamed { nearest: String },
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
                "no shipped preset has these lands (count, numbers, kind, side, hole, shape, sizes and extent)"
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
            Self::Tie { presets } => write!(
                f,
                "tie: shipped presets {} fit equally and neither the footprint's name, the reference prefix nor a body outline chooses one",
                presets.join(", ")
            ),
            Self::Unnamed { nearest } => write!(
                f,
                "shipped preset {nearest} has a land round a non-plated hole where the footprint has a plated pad; such lands do not tell the part, and the footprint's name does not name it"
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

/// How long a preset name or alias of `candidate` the footprint's name
/// carries (0 for none). A name ending (or starting) in a digit must not run
/// on into another digit: "1X1" is not in "1X10".
fn named(candidate: &Candidate, footprint: &str) -> usize {
    let footprint = plain(footprint);
    let bytes = footprint.as_bytes();
    let mut best = 0;
    for name in &candidate.names {
        let mut from = 0;
        while let Some(at) = footprint
            .get(from..)
            .and_then(|rest| rest.find(name.as_str()))
        {
            let start = from + at;
            let end = start + name.len();
            let digit =
                |i: Option<usize>| i.and_then(|i| bytes.get(i)).is_some_and(u8::is_ascii_digit);
            let runs_on = (name.as_bytes().last().is_some_and(u8::is_ascii_digit)
                && digit(Some(end)))
                || (name.as_bytes().first().is_some_and(u8::is_ascii_digit)
                    && digit(start.checked_sub(1)));
            if !runs_on {
                best = best.max(name.len());
                break;
            }
            from = start + 1;
        }
    }
    best
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
        // The lands the lead's number names, read as matching reads them
        // (`01` is `1`, `41_3` a sub-pad of `41`).
        let key = lands::Key::of(&lead.key);
        let miss = pads
            .iter()
            .filter(|p| {
                matches!(p.kind, PadKind::Smd | PadKind::PlatedThrough)
                    && lands::Key::of(&p.number) == key
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

/// How far a body outline of `extent` mm is from the preset's at the fit's
/// quarter turn, as the larger relative difference of the two axes.
fn body_distance(extent: [f64; 2], preset: [f64; 2], fit: &Fit) -> f64 {
    let turn = fit.placement.rotation_deg()[2].round() as i64;
    let preset = if turn.rem_euclid(180) == 90 {
        [preset[1], preset[0]]
    } else {
        preset
    };
    (0..2)
        .map(|a| {
            if preset[a] > 0.0 {
                (extent[a] - preset[a]).abs() / preset[a]
            } else {
                f64::INFINITY
            }
        })
        .fold(0.0, f64::max)
}

/// Variants of a package that share its lands (a female socket and a male
/// pin header, a shroud's colours) told apart by evidence: a preset whose
/// choices depart from its generator's defaults ([`Candidate::departures`])
/// is shown when the part number or the footprint's name names it, or
/// every word of each such choice (`socket`, `smd` and `bottom` of
/// `socket_smd_bottom`) appears in the part number, the footprint's name,
/// the part's value or the footprint's description;
/// then the generator's presets of the same kind (component prefix)
/// without that evidence give way to it. Without evidence a departing
/// preset gives way to a fitting preset of the same generator and kind
/// that keeps the defaults (the male header); where none fits (the lands
/// are the variant's own), to its plain twin with the same leads and lands
/// ([`twin`]), at the variant's fit; else it stays.
fn by_evidence(
    candidates: &[Candidate],
    pool: Vec<(usize, Fit)>,
    facts: &Facts,
    mpn: Option<&str>,
    notes: &mut Vec<String>,
) -> Vec<(usize, Fit)> {
    let text = [
        facts.name.as_str(),
        &facts.value,
        &facts.description,
        mpn.unwrap_or_default(),
    ]
    .join(" ")
    .to_lowercase();
    let shown = |index: usize| {
        let c = &candidates[index];
        // Named by the part number or the footprint's name, or every word
        // of its choices written somewhere.
        let named_so = mpn.is_some_and(|mpn| c.names.iter().any(|n| names_apart(mpn, n)))
            || named(c, &facts.name) > 0;
        !c.departures.is_empty()
            && (named_so
                || c.departures.iter().all(|choice| {
                    choice
                        .split(['_', '-', ' '])
                        .filter(|w| !w.is_empty())
                        .all(|w| text.contains(w))
                }))
    };
    // Variants of one another: one generator's presets of one kind of
    // part (the component's prefix: a chip resistor is no variant of a
    // chip capacitor).
    let kind = |index: usize| {
        let c = &candidates[index];
        let prefix = c.generation.component.as_ref().map(|c| c.prefix.as_str());
        (c.generator.as_str(), prefix.unwrap_or_default())
    };
    let kinds: std::collections::BTreeSet<(&str, &str)> =
        pool.iter().map(|(i, _)| kind(*i)).collect();
    let mut kept = Vec::with_capacity(pool.len());
    let mut dropped = Vec::new();
    for of_kind in kinds {
        let of: Vec<&(usize, Fit)> = pool.iter().filter(|(i, _)| kind(*i) == of_kind).collect();
        let evidenced = of.iter().any(|(i, _)| shown(*i));
        let plain = of.iter().any(|(i, _)| candidates[*i].departures.is_empty());
        for entry in of {
            let c = &candidates[entry.0];
            if evidenced {
                if shown(entry.0) {
                    kept.push(entry.clone());
                } else {
                    dropped.push(c.name.clone());
                }
            } else if c.departures.is_empty() || plain {
                if c.departures.is_empty() {
                    kept.push(entry.clone());
                } else {
                    dropped.push(c.name.clone());
                }
            } else if let Some(twin) = twin(candidates, entry.0) {
                // Its lands are the variant's own (a socket's oval lands
                // a male header's footprint may have too), so no plain
                // preset fits them; the plain twin with the same leads at
                // the same places takes its fit.
                dropped.push(c.name.clone());
                kept.push((twin, entry.1.clone()));
            } else {
                kept.push(entry.clone());
            }
        }
    }
    if !dropped.is_empty() {
        dropped.sort();
        notes.push(format!(
            "Variants without evidence in the part number, name, value or description set aside: {}.",
            dropped.join(", ")
        ));
    }
    // The pool's order (preset order) decides nothing; keep it anyway.
    kept.sort_by_key(|(i, _)| *i);
    kept
}

/// The preset of `index`'s generator and kind that keeps its generator's
/// defaults and has the same leads at the same places and the same lands
/// in the same order (a male pin header of a socket's pins and pitch).
fn twin(candidates: &[Candidate], index: usize) -> Option<usize> {
    let of = &candidates[index];
    let prefix = |c: &Candidate| c.generation.component.as_ref().map(|c| c.prefix.clone());
    let leads = |c: &Candidate| {
        let mut leads: Vec<(String, [i64; 2])> = c
            .generation
            .package
            .geometry
            .leads
            .iter()
            .map(|l| (l.key.clone(), [l.contact[0], l.contact[1]]))
            .collect();
        leads.sort();
        leads
    };
    let pads = |c: &Candidate| -> Vec<String> {
        c.generation
            .footprint
            .pads
            .iter()
            .map(|p| p.terminal.clone())
            .collect()
    };
    candidates.iter().position(|c| {
        c.departures.is_empty()
            && c.generator == of.generator
            && prefix(c) == prefix(of)
            && leads(c) == leads(of)
            && pads(c) == pads(of)
    })
}

/// The fitting presets narrowed, in order, by a preset the part number
/// `mpn` names standing apart (the longest such name), else by a preset the
/// footprint's name names (the longest such name), else by the longest
/// declared prefix that begins the designator's ("RFB" is a resistor's "R", "FB" a ferrite
/// bead's); then the nearest centres decide, and among equally near ones
/// the footprint's body outline (fabrication layer, else courtyard). A tie
/// is no choice. Two-pad lands (chip resistors, capacitors, inductors,
/// LEDs, diodes) do not tell the body type, so there a preset neither named
/// nor of the designator's prefix keeps the placeholder: a wrong-looking
/// body is worse. Multi-pad packages (SOT, SO, QFN) keep their body
/// whatever the designator. A plated pad standing for a land round a
/// non-plated hole needs the name: a plated mounting hole has those lands.
fn choose(
    candidates: &[Candidate],
    fitting: Vec<(usize, Fit)>,
    prefix: &str,
    facts: &Facts,
    mpn: Option<&str>,
) -> Result<Found, Miss> {
    let nearest_name = |pool: &[(usize, Fit)]| {
        let best = pool
            .iter()
            .map(|(_, f)| f.error_nm)
            .fold(f64::INFINITY, f64::min);
        pool.iter()
            .find(|(_, f)| f.error_nm <= best)
            .map(|(i, _)| candidates[*i].name.clone())
            .unwrap_or_default()
    };
    let mut notes = Vec::new();
    let in_mpn = |index: &usize| {
        mpn.and_then(|mpn| {
            candidates[*index]
                .names
                .iter()
                .filter(|n| names_apart(mpn, n))
                .map(String::len)
                .max()
        })
        .unwrap_or(0)
    };
    let by_mpn = fitting.iter().map(|(i, _)| in_mpn(i)).max().unwrap_or(0);
    let by_name = fitting
        .iter()
        .map(|(i, _)| named(&candidates[*i], &facts.name))
        .max()
        .unwrap_or(0);
    let pool: Vec<(usize, Fit)> = if by_mpn > 0 {
        notes.push(format!(
            "Named by the part number {:?}.",
            mpn.unwrap_or_default()
        ));
        fitting
            .into_iter()
            .filter(|(i, _)| in_mpn(i) == by_mpn)
            .collect()
    } else if by_name > 0 {
        notes.push(format!("Named by the footprint's name {:?}.", facts.name));
        fitting
            .into_iter()
            .filter(|(i, _)| named(&candidates[*i], &facts.name) == by_name)
            .collect()
    } else {
        let (cross, fitting): (Vec<_>, Vec<_>) =
            fitting.into_iter().partition(|(_, f)| f.cross_kind);
        if fitting.is_empty() {
            return Err(Miss::Unnamed {
                nearest: nearest_name(&cross),
            });
        }
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
        let longest = fitting.iter().map(|(i, _)| declared(i)).max().unwrap_or(0);
        let two_pad = fitting
            .first()
            .is_some_and(|(i, _)| candidates[*i].lands.len() <= PREFIX_REQUIRED_MAX_PADS);
        if longest == 0 && two_pad {
            return Err(Miss::PrefixMismatch {
                nearest: nearest_name(&fitting),
            });
        }
        // (With `longest == 0`, every fitting multi-pad preset stays.)
        fitting
            .into_iter()
            .filter(|(i, _)| declared(i) == longest)
            .collect()
    };
    let pool = by_evidence(candidates, pool, facts, mpn, &mut notes);
    let best = pool
        .iter()
        .map(|(_, f)| f.error_nm)
        .fold(f64::INFINITY, f64::min);
    let mut nearest: Vec<_> = pool
        .into_iter()
        .filter(|(_, f)| f.error_nm <= best + TIE_NM)
        .collect();
    if nearest.len() > 1 {
        let outline = match (facts.fab_extent_nm, facts.courtyard_extent_nm) {
            (Some(fab), _) => Some(("fabrication outline", fab, None)),
            (None, Some(courtyard)) => Some(("courtyard", courtyard, Some(()))),
            (None, None) => None,
        };
        if let Some((what, extent, courtyard)) = outline {
            let extent = extent.map(|n| n.to_mm_f64());
            let distance = |(i, fit): &(usize, Fit)| {
                let c = &candidates[*i];
                match courtyard {
                    None => body_distance(extent, c.body_mm, fit),
                    Some(()) => c
                        .courtyard_mm
                        .map_or(f64::INFINITY, |p| body_distance(extent, p, fit)),
                }
            };
            let mut ranked: Vec<(f64, usize)> = nearest
                .iter()
                .enumerate()
                .map(|(k, entry)| (distance(entry), k))
                .collect();
            ranked.sort_by(|a, b| a.0.total_cmp(&b.0));
            if let [(first, k), (second, _), ..] = ranked.as_slice()
                && *first <= BODY_FRACTION
                && *second - *first >= BODY_MARGIN
            {
                let chosen = nearest.swap_remove(*k);
                notes.push(format!(
                    "Told apart by the footprint's {what} ({:.2} × {:.2} mm).",
                    extent[0], extent[1]
                ));
                nearest = vec![chosen];
            }
        }
    }
    match nearest.as_slice() {
        [(index, fit)] => {
            if !fit.pairing.is_empty() {
                notes.insert(0, format!("Lands paired: {}.", fit.pairing.join(", ")));
            }
            Ok(Found {
                index: *index,
                placement: fit.placement,
                notes,
                pads: fit.pads.clone(),
                name: candidates[*index].name.clone(),
            })
        }
        _ => {
            let mut presets: Vec<String> = nearest
                .iter()
                .map(|(i, _)| candidates[*i].name.clone())
                .collect();
            presets.sort();
            Err(Miss::Tie { presets })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sparksmith_core::library::{CornerMask, PadShape};
    use sparksmith_core::units::{Angle, Nm, PointNm};
    use sparksmith_core::{PackageTerminalId, PadId};
    type Fallible = Result<(), Box<dyn std::error::Error>>;

    fn fit(imported: &[Pad], preset: &[Pad]) -> Option<Fit> {
        let preset = Lands::new(preset, |_| None);
        let tolerance = preset.tolerance_nm(CENTRE_FLOOR_NM, CENTRE_FRACTION);
        lands::fit(&Lands::new(imported, Polarity::of), &preset, tolerance)
    }

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
        candidates(presets)?
            .iter()
            .find(|c| c.name == name)
            .ok_or_else(|| format!("No {name} preset"))
            .and_then(|c| expand::land_pattern(&c.generation).map_err(|e| e.to_string()))
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
        // Own variants pick their package by the reference prefix.
        for (pads, prefix, preset) in [
            (chip(1.5, [0.85, 0.95])?, "R", "chip/resistor-0603"),
            (chip(1.6, [0.8, 0.9])?, "C", "chip/capacitor-0603"),
            (chip(1.9, [1.05, 1.5])?, "R", "chip/resistor-0805"),
            (chip(1.65, [0.8, 0.95])?, "RFB", "chip/resistor-0603"),
            (chip(1.65, [0.8, 0.95])?, "FB", "chip/ferrite-0603"),
        ] {
            let choice = presets
                .identify(&engine, &pads, prefix, &Facts::default(), None, deadline)?
                .map_err(|miss| format!("{preset}: {miss}"))?;
            assert!(choice.name.contains(preset), "{}", choice.name);
        }
        // A prefix no fitting preset declares keeps the placeholder: a
        // crystal's (Y) lands of a 1210 chip's size get no resistor body.
        let r0603 = lands_of(&presets, "chip/resistor-0603")?;
        assert!(matches!(
            presets.identify(&engine, &r0603, "?", &Facts::default(), None, deadline)?,
            Err(Miss::PrefixMismatch { .. })
        ));
        let mut crystal = chip(2.4, [1.0, 3.0])?;
        for pad in &mut crystal {
            let [x, y] = pad.position.to_mm_f64();
            pad.position = PointNm::from_mm_f64_rounded([y, x])?;
            pad.shape = PadShape::round_rect_mm([3.0, 1.0], 0.25, None, CornerMask::ALL)?;
        }
        assert!(
            accepted(&crystal, &lands_of(&presets, "chip/resistor-1210")?),
            "the lands fit a 1210 chip"
        );
        match presets.identify(&engine, &crystal, "Y", &Facts::default(), None, deadline)? {
            // Nearest by centres is the XEL4030 (2.39 mm span), which declares L.
            Err(Miss::PrefixMismatch { nearest }) => {
                assert_eq!(nearest, "power-inductor/power-inductor-xel4030");
            }
            other => return Err(format!("a Y part got {:?}", other.map(|c| c.name)).into()),
        }
        // The power-inductor family declares L: an inductor on its lands
        // gets its body, not the placeholder.
        let xel4030 = lands_of(&presets, "power-inductor/power-inductor-xel4030")?;
        let choice = presets
            .identify(&engine, &xel4030, "L", &Facts::default(), None, deadline)?
            .map_err(|miss| format!("XEL4030 under L: {miss}"))?;
        assert!(
            choice
                .name
                .contains("power-inductor/power-inductor-xel4030"),
            "{}",
            choice.name
        );
        // A multi-pad package keeps its body under an unusual designator.
        let sot = lands_of(&presets, "tabbed/tabbed-sot-23-6")?;
        let choice = presets
            .identify(&engine, &sot, "XQ", &Facts::default(), None, deadline)?
            .map_err(|miss| format!("SOT-23-6 under XQ: {miss}"))?;
        assert!(choice.name.contains("tabbed/tabbed-sot-23-6"));
        for name in [
            "tabbed/tabbed-sot-23-3",
            "gull-wing/gull-wing-soic-8-3.9",
            "no-lead/no-lead-qfn-12-3x3-0.5",
        ] {
            let pads = lands_of(&presets, name)?;
            let choice = presets
                .identify(&engine, &pads, "U", &Facts::default(), None, deadline)?
                .map_err(|miss| format!("{name}: {miss}"))?;
            assert!(choice.name.contains(name));
            let repeated = presets
                .identify(&engine, &pads, "U", &Facts::default(), None, deadline)?
                .map_err(|miss| miss.to_string())?;
            assert_eq!(choice.index, repeated.index);
            // Identifying makes nothing.
            assert!(engine.project().library.models.is_empty());
            // One pad moved a millimetre: another package, not this one.
            let mut far = pads.clone();
            let [x, y] = far[0].position.to_mm_f64();
            far[0].position = PointNm::from_mm_f64_rounded([x - 1.0, y])?;
            assert!(
                presets
                    .identify(&engine, &far, "U", &Facts::default(), None, deadline)?
                    .is_err(),
                "far lands accepted for {name}"
            );
        }
        Ok(())
    }

    fn named(name: &str) -> Facts {
        Facts {
            name: name.into(),
            ..Facts::default()
        }
    }
    fn renamed(pads: &[Pad], names: &[(&str, &str)]) -> Vec<Pad> {
        pads.iter()
            .map(|p| {
                let mut p = p.clone();
                if let Some((_, to)) = names.iter().find(|(from, _)| *from == p.number) {
                    p.number = (*to).into();
                }
                p
            })
            .collect()
    }
    fn quarter_turned(pads: &[Pad]) -> Result<Vec<Pad>, sparksmith_core::Error> {
        pads.iter()
            .map(|p| {
                let mut p = p.clone();
                let [x, y] = p.position.to_mm_f64();
                p.position = PointNm::from_mm_f64_rounded([-y, x])?;
                p.rotation_mdeg =
                    Angle::from_degrees_f64_rounded(p.rotation_mdeg.to_degrees_f64() + 90.0)?;
                Ok(p)
            })
            .collect()
    }
    fn setup() -> Result<(Presets, Engine), Box<dyn std::error::Error>> {
        let presets = loaded()?;
        let mut engine = Engine::demo()?;
        engine.set_generators(crate::session::generators::fixture_runtime());
        Ok((presets, engine))
    }

    /// Electrolytic size codes share their lands: the footprint's name
    /// chooses, a name that runs on into another digit does not, and no
    /// name is a tie that chooses none.
    #[test]
    fn footprint_names_choose_among_identical_lands_and_otherwise_a_tie_chooses_none() -> Fallible {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
        let (mut presets, engine) = setup()?;
        let lands = lands_of(&presets, "electrolytic/electrolytic-panasonic-d")?;
        for (name, preset) in [
            ("CAPAE660X550N", "electrolytic/electrolytic-6.3x5.4"),
            ("CP_Elec_6.3x7.7", "electrolytic/electrolytic-panasonic-d8"),
        ] {
            let choice = presets
                .identify(&engine, &lands, "C", &named(name), None, deadline)?
                .map_err(|miss| format!("{name}: {miss}"))?;
            assert!(choice.name.contains(preset), "{}", choice.name);
            assert!(
                choice
                    .notes
                    .join(" ")
                    .contains("Named by the footprint's name")
            );
        }
        for name in ["CP_Elec_16.3x7.7", "CP_Own", ""] {
            match presets.identify(&engine, &lands, "C", &named(name), None, deadline)? {
                Err(Miss::Tie { presets: tied }) => {
                    for preset in [
                        "electrolytic/electrolytic-6.3x5.4",
                        "electrolytic/electrolytic-panasonic-d",
                        "electrolytic/electrolytic-panasonic-d8",
                    ] {
                        assert!(tied.iter().any(|t| t == preset), "{name}: {tied:?}");
                    }
                    let miss = Miss::Tie { presets: tied }.to_string();
                    assert!(miss.starts_with("tie: "), "{miss}");
                }
                other => {
                    let got = other.map(|c| c.name);
                    return Err(format!("{name}: {got:?}").into());
                }
            }
        }
        Ok(())
    }

    /// Among equally near presets the footprint's body outline chooses one
    /// whose body it is near, when no other is about as near.
    #[test]
    fn a_body_outline_tells_equally_near_presets_apart_or_leaves_a_tie() -> Fallible {
        let mut presets = loaded()?;
        let lands = lands_of(&presets, "electrolytic/electrolytic-panasonic-d")?;
        let mut all = presets.candidates.take().ok_or("No presets")?;
        let mut take = |name: &str| {
            let at = all.iter().position(|c| c.name == name);
            at.map(|at| all.swap_remove(at))
                .ok_or_else(|| format!("No {name}"))
        };
        let mut two = vec![
            take("electrolytic/electrolytic-panasonic-d")?,
            take("electrolytic/electrolytic-panasonic-d8")?,
        ];
        let body = two[0].body_mm;
        let courtyard = two[0].courtyard_mm.ok_or("No courtyard")?;
        two[1].body_mm = body.map(|v| 1.25 * v);
        two[1].courtyard_mm = Some(courtyard.map(|v| 1.25 * v));
        let imported = Lands::new(&lands, Polarity::of);
        let fitting = || {
            two.iter()
                .enumerate()
                .filter_map(|(i, c)| Some((i, lands::fit(&imported, &c.lands, c.tolerance_nm)?)))
                .collect::<Vec<_>>()
        };
        assert_eq!(fitting().len(), 2);
        let nm = |v: [f64; 2]| -> Result<[Nm; 2], sparksmith_core::Error> {
            Ok([
                Nm::from_mm_f64_rounded(v[0])?,
                Nm::from_mm_f64_rounded(v[1])?,
            ])
        };
        let fab = |scale: f64| -> Result<Facts, sparksmith_core::Error> {
            Ok(Facts {
                fab_extent_nm: Some(nm(body.map(|v| scale * v))?),
                ..Facts::default()
            })
        };
        let found = choose(&two, fitting(), "C", &fab(1.0)?, None).map_err(|m| m.to_string())?;
        assert_eq!(found.index, 0);
        assert!(
            found
                .notes
                .iter()
                .any(|n| n.contains("fabrication outline"))
        );
        let found = choose(&two, fitting(), "C", &fab(1.24)?, None).map_err(|m| m.to_string())?;
        assert_eq!(found.index, 1);
        // Near misses: an outline between the two (12% and 10.4% off) and
        // one far from both are ties.
        for scale in [1.12, 1.6] {
            assert!(
                matches!(
                    choose(&two, fitting(), "C", &fab(scale)?, None),
                    Err(Miss::Tie { .. })
                ),
                "{scale}"
            );
        }
        // Without a fabrication outline, the courtyard against the preset's.
        let by_courtyard = Facts {
            courtyard_extent_nm: Some(nm(courtyard)?),
            ..Facts::default()
        };
        let found = choose(&two, fitting(), "C", &by_courtyard, None).map_err(|m| m.to_string())?;
        assert_eq!(found.index, 0);
        assert!(found.notes.iter().any(|n| n.contains("courtyard")));
        Ok(())
    }

    /// A standoff's land drawn as one plated pad: the preset named in the
    /// footprint's name fits; unnamed, a plated mounting hole has the same
    /// lands, so it keeps the placeholder.
    #[test]
    fn a_plated_standoff_land_needs_the_footprint_name() -> Fallible {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
        let (mut presets, engine) = setup()?;
        let preset = lands_of(&presets, "standoff/standoff-wurth-9774100360r")?;
        let land = preset.iter().find(|p| p.number == "1").ok_or("No land")?;
        let plated = |drill: f64| -> Result<Vec<Pad>, sparksmith_core::Error> {
            let mut pad = land.clone();
            pad.kind = sparksmith_core::library::PadKind::PlatedThrough;
            pad.hole = Some(sparksmith_core::library::Hole {
                shape: sparksmith_core::library::HoleShape::round_mm(drill)?,
                plated: true,
                offset: PointNm::from_mm_f64_rounded([0.0, 0.0])?,
            });
            Ok(vec![pad])
        };
        let choice = presets
            .identify(
                &engine,
                &plated(4.4)?,
                "U",
                &named("Own9774100360R"),
                None,
                deadline,
            )?
            .map_err(|miss| miss.to_string())?;
        assert!(choice.name.contains("standoff/standoff-wurth-9774100360r"));
        for (drill, name, prefix) in [
            (4.4, "", "H"),
            (4.4, "Own_M3_Spacer", "U"),
            (3.2, "Own9774100360R", "U"),
        ] {
            let got = presets.identify(
                &engine,
                &plated(drill)?,
                prefix,
                &named(name),
                None,
                deadline,
            )?;
            assert!(got.is_err(), "{drill} {name:?}: {:?}", got.map(|c| c.name));
        }
        assert!(matches!(
            presets.identify(
                &engine,
                &plated(4.4)?,
                "H",
                &Facts::default(),
                None,
                deadline
            )?,
            Err(Miss::Unnamed { .. })
        ));
        Ok(())
    }

    /// A buzzer's lands named P and N, positive at the right, and a side
    /// entry header turned a quarter with its tabs named S1 and S2.
    #[test]
    fn polar_names_and_mechanical_tabs_identify_shipped_presets() -> Fallible {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
        let (mut presets, engine) = setup()?;
        let buzzer = lands_of(&presets, "buzzer/buzzer-cmi-9653s-smt")?;
        let polar = renamed(&buzzer, &[("1", "N"), ("2", "P")]);
        let choice = presets
            .identify(
                &engine,
                &polar,
                "LS",
                &named("Own_CMI-9653S-SMT"),
                None,
                deadline,
            )?
            .map_err(|miss| miss.to_string())?;
        assert!(
            choice.notes.join(" ").contains("P→1 (+)"),
            "{:?}",
            choice.notes
        );
        assert_eq!(choice.placement.rotation_deg()[2], 180.0);
        assert!(matches!(
            presets.identify(&engine, &polar, "LS", &Facts::default(), None, deadline)?,
            Err(Miss::PrefixMismatch { .. })
        ));
        let header = lands_of(&presets, "wire-to-board/wtb-sh-side-4")?;
        let tabs = quarter_turned(&renamed(
            &header,
            &[("MP1", "S1"), ("MP2", "S2"), ("1", "01")],
        ))?;
        let choice = presets
            .identify(&engine, &tabs, "J", &Facts::default(), None, deadline)?
            .map_err(|miss| miss.to_string())?;
        assert!(
            choice.name.contains("wire-to-board/wtb-sh-side-4"),
            "{}",
            choice.name
        );
        assert_eq!(choice.placement.rotation_deg()[2], 90.0);
        let one_tab: Vec<Pad> = tabs.iter().filter(|p| p.number != "S2").cloned().collect();
        assert!(
            presets
                .identify(&engine, &one_tab, "J", &Facts::default(), None, deadline)?
                .is_err()
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
            let own = expand::land_pattern(&candidate.generation)?;
            if candidate.generation.package.geometry.leads.is_empty() || fit(&own, &own).is_none() {
                continue;
            }
            for quarter in 0..4 {
                let angle = f64::from(quarter * 90);
                let (c, s) = sparksmith_core::geometry::cos_sin(angle);
                let mut turned = own.clone();
                for pad in &mut turned {
                    let [x, y] = pad.position.to_mm_f64();
                    pad.position = PointNm::from_mm_f64_rounded([c * x - s * y, s * x + c * y])?;
                    let r = pad.rotation_mdeg.millidegrees() + quarter * 90_000;
                    pad.rotation_mdeg = Angle::normalized(i64::from(r));
                }
                let pose = fit(&turned, &own).ok_or("own lands fit")?;
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
        let mut moved = expand::land_pattern(&sot.generation)?;
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
        let error = presets
            .identify(
                &engine,
                &chip(1.6, [0.7, 0.8])?,
                "R",
                &Facts::default(),
                None,
                std::time::Instant::now() + std::time::Duration::from_secs(120),
            )
            .err()
            .ok_or("busy runtime was converted into a placeholder")?;
        assert!(error.contains("busy"), "{error}");
        assert_eq!(runtime.generations.load(Ordering::Relaxed), 2);
        assert!(presets.candidates.is_none());
        assert!(presets.matches.is_empty());
        assert_eq!(engine.revision(), revision);
        assert!(engine.project().library.models.is_empty());
        Ok(())
    }
    /// A single-row male header footprint with long oval lands is a female
    /// socket's lands too: without evidence the male header is chosen; the
    /// socket only when the part number, the footprint's name, the value or
    /// the description says so. Original synthetic footprints.
    #[test]
    fn a_socket_needs_evidence_and_otherwise_the_male_header_is_chosen() -> Fallible {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
        let (mut presets, engine) = setup()?;
        let oval = lands_of(&presets, "header/socket-1x4-2.54")?;
        let round = lands_of(&presets, "header/header-1x4-2.54")?;
        let facts = |name: &str, value: &str, description: &str| Facts {
            name: name.into(),
            value: value.into(),
            description: description.into(),
            ..Facts::default()
        };
        let none = facts(
            "Own_Conn_4_Oval",
            "Conn_4",
            "four-way connector, oval lands",
        );
        for (lands, what) in [(&oval, "oval"), (&round, "round")] {
            let choice = presets
                .identify(&engine, lands, "J", &none, None, deadline)?
                .map_err(|miss| format!("{what}: {miss}"))?;
            assert_eq!(
                choice.name, "header/header-1x4-2.54",
                "{what}: {:?}",
                choice.notes
            );
        }
        for (evidence, mpn) in [
            (facts("Own_Socket_4_Oval", "", ""), None),
            (facts("Own_4", "Conn_01x04_Socket", ""), None),
            (
                facts("Own_4", "Conn_4", "female socket strip, 4 ways"),
                None,
            ),
            (facts("Own_4", "", ""), Some("61300411821")),
        ] {
            let choice = presets
                .identify(&engine, &oval, "J", &evidence, mpn, deadline)?
                .map_err(|miss| format!("{evidence:?}: {miss}"))?;
            assert_eq!(
                choice.name, "header/socket-1x4-2.54",
                "{evidence:?} {mpn:?}: {:?}",
                choice.notes
            );
        }
        Ok(())
    }
}
