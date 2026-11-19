//! "Fit leads to pads": a pure rigid fit of tagged lead-contact points onto
//! pad centres, the one-click placement actions, and the advisory placement
//! findings. The core never reads geometry files: lead contacts come from
//! the package's stored lead contacts ([`PackageDefinition::leads`], each
//! naming its terminal) or from the host (the integration crate, for
//! imported models) as [`LeadPair`]s.
use super::footprint::{FootprintDefinition, PAD_ARC_TOLERANCE_MM, PadKind};
use super::package_model::{
    self, Axis, ModelPlacement, PackageEdit, PackageModel, QuickAction, model_frame,
};
use super::publish::object;
use super::{Datum, PackageDefinition, Pad};
use crate::diagnostics::{Error, Finding, Result};
use crate::geometry::{Bounds, Point, Point3, Transform3, cos_sin};
use crate::ids::{DefinitionId, PackageTerminalId, PadId};
use crate::units::{self, Nm, PointNm};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Lead contact off its land by more than this fraction of the land width
/// (estimate, advisory).
pub const LEAD_OFF_LAND_FRACTION: f64 = 0.25;

/// Most lead/pad pairs one fit takes.
pub const MAX_LEAD_PAIRS: usize = 4096;

/// A lead contact further than this from the seating plane, or a body below
/// it by more, gives a finding (mm, estimate).
pub const SEAT_TOLERANCE_MM: f64 = 0.01;

/// A mirrored assignment is reported when its error is below this share of
/// the fitted error (and the fit itself is off by more than
/// [`MIRROR_MIN_RMS_MM`]).
const MIRROR_SHARE: f64 = 0.5;
const MIRROR_MIN_RMS_MM: f64 = 0.05;

/// Which rotations the fit may use.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FitRotation {
    Free,
    Quarter,
}

/// One terminal's lead-contact centroid in the model frame and the centre
/// of the pad it lands on, in the datum frame: a `library.fit_leads`
/// parameter, held in nanometres and carried in millimetres (`lead`
/// `[x, y, z]`, `pad_centre` `[x, y]`). [`Self::lead`] and
/// [`Self::pad_centre`] give millimetres for the fit.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LeadPair {
    pub terminal: PackageTerminalId,
    pub pad: PadId,
    #[serde(rename = "lead", with = "triple_mm")]
    #[schemars(with = "[units::Length; 3]")]
    pub lead_nm: [Nm; 3],
    #[serde(rename = "pad_centre", with = "units::point_mm")]
    #[schemars(with = "units::PointMm")]
    pub pad_centre_nm: PointNm,
}

impl LeadPair {
    /// A pair from computed millimetres, through the one crossing.
    pub fn from_mm(
        terminal: PackageTerminalId,
        pad: PadId,
        lead: Point3,
        pad_centre: Point,
    ) -> Result<Self> {
        Ok(Self {
            terminal,
            pad,
            lead_nm: super::model3d::triple_nm(lead)?,
            pad_centre_nm: PointNm::from_mm_f64_rounded(pad_centre)?,
        })
    }

    /// The lead-contact centroid in the model frame, mm.
    pub fn lead(&self) -> Point3 {
        self.lead_nm.map(Nm::to_mm_f64)
    }

    /// The pad's centre in the datum frame, mm.
    pub fn pad_centre(&self) -> Point {
        self.pad_centre_nm.to_mm_f64()
    }
}

/// Serde adapter: an `[x, y, z]` of [`Nm`] carried as three
/// [`units::Length`]s.
mod triple_mm {
    use crate::units::{Length, Nm};
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(p: &[Nm; 3], s: S) -> Result<S::Ok, S::Error> {
        p.map(Length).serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[Nm; 3], D::Error> {
        <[Length; 3]>::deserialize(d).map(|p| p.map(|l| l.0))
    }
}

/// Residual of one terminal after the fit, in mm.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LeadResidual {
    pub terminal: PackageTerminalId,
    pub error_mm: f64,
}

/// The fit result: the placement that seats the leads, and its errors. A
/// reply: the placement is carried in millimetres and degrees.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LeadFit {
    #[serde(with = "package_model::model_placement_mm")]
    #[schemars(with = "package_model::ModelPlacementParams")]
    pub placement: ModelPlacement,
    pub rms_mm: f64,
    pub residuals: Vec<LeadResidual>,
    /// A mirrored assignment would fit better (pin order reversed).
    pub mirrored_better: bool,
}

/// 2D rigid least-squares fit of `pairs`, then seat on z = 0, starting from
/// `current` (its x/y rotations are kept).
///
/// The leads are first turned by `current`'s x and y rotations; the fit then finds
/// the turn about the datum z axis and the x/y shift that best map their
/// x/y onto the pad centres (2D Kabsch), and lifts the model so its lowest
/// lead contact lies on the seating plane. With [`FitRotation::Quarter`]
/// only the four quarter turns are tried. A single pair only translates.
pub fn fit_leads(
    current: &ModelPlacement,
    pairs: &[LeadPair],
    rotation: FitRotation,
) -> Result<LeadFit> {
    validate_pairs(pairs)?;
    let leads: Vec<Point3> = pairs.iter().map(LeadPair::lead).collect();
    let targets: Vec<Point> = pairs.iter().map(LeadPair::pad_centre).collect();
    let (placement, errors, mirrored_better) = fit_points(current, &leads, &targets, rotation)?;
    let residuals: Vec<LeadResidual> = pairs
        .iter()
        .zip(errors)
        .map(|(p, error_mm)| LeadResidual {
            terminal: p.terminal,
            error_mm,
        })
        .collect();
    Ok(LeadFit {
        placement,
        rms_mm: rms(residuals.iter().map(|r| r.error_mm)),
        residuals,
        mirrored_better,
    })
}

/// [`fit_leads`] over bare points: lead contacts (model frame) and the
/// centres they land (datum frame), paired by index, such as a locating
/// peg and its unnumbered hole, which land no terminal. Returns the
/// placement, each point's error (mm) and whether a mirrored assignment
/// fits better.
pub fn fit_points(
    current: &ModelPlacement,
    leads: &[Point3],
    targets: &[Point],
    rotation: FitRotation,
) -> Result<(ModelPlacement, Vec<f64>, bool)> {
    if leads.is_empty() || leads.len() > MAX_LEAD_PAIRS || leads.len() != targets.len() {
        return Err(Error::Invalid(format!(
            "package.fit_pairs: fitting takes 1–{MAX_LEAD_PAIRS} lead/pad pairs"
        )));
    }
    // The z turn is solved from scratch; only the tilt is kept, so a
    // quarter-turn fit of an untilted model is exact.
    let [rx, ry, _] = current.rotation_deg();
    let base = Transform3::intrinsic_xyz([rx, ry, 0.0]);
    let turned: Vec<Point3> = leads.iter().map(|p| base.apply(*p)).collect();
    let flat: Vec<Point> = turned.iter().map(|q| [q[0], q[1]]).collect();
    let (angle, shift) = solve_2d(&flat, targets, rotation);
    let lowest = turned.iter().map(|q| q[2]).fold(f64::INFINITY, f64::min);
    let outer = Transform3::translation([shift[0], shift[1], -lowest])
        .then(&Transform3::rot_z(angle))
        .then(&base);
    let placement = placement_of(&outer)?;
    let frame = placement.transform();
    let errors: Vec<f64> = leads
        .iter()
        .zip(targets)
        .map(|(lead, c)| {
            let q = frame.apply(*lead);
            (q[0] - c[0]).hypot(q[1] - c[1])
        })
        .collect();
    let rms_mm = rms(errors.iter().copied());
    // The same leads mirrored across the x axis: what a reversed (clockwise)
    // pin order would land.
    let mirrored: Vec<Point> = flat.iter().map(|q| [q[0], -q[1]]).collect();
    let (m_angle, m_shift) = solve_2d(&mirrored, targets, rotation);
    let m_rms = rms(mirrored.iter().zip(targets).map(|(q, p)| {
        let r = rotate(*q, m_angle);
        (r[0] + m_shift[0] - p[0]).hypot(r[1] + m_shift[1] - p[1])
    }));
    let mirrored_better = rms_mm > MIRROR_MIN_RMS_MM && m_rms < MIRROR_SHARE * rms_mm;
    Ok((placement, errors, mirrored_better))
}

/// Pairs and the placement hold whole nanometres within ±2 m: finite and in
/// range by type.
fn validate_pairs(pairs: &[LeadPair]) -> Result<()> {
    if pairs.is_empty() || pairs.len() > MAX_LEAD_PAIRS {
        return Err(Error::Invalid(format!(
            "package.fit_pairs: fitting takes 1–{MAX_LEAD_PAIRS} lead/pad pairs"
        )));
    }
    let mut seen = BTreeSet::new();
    for pair in pairs {
        if !seen.insert(pair.terminal) {
            return Err(Error::Invalid(format!(
                "package.fit_pairs: terminal {} is paired twice",
                pair.terminal
            )));
        }
    }
    Ok(())
}

fn rms(errors: impl Iterator<Item = f64>) -> f64 {
    let (sum, n) = errors.fold((0.0, 0usize), |(s, n), e| (s + e * e, n + 1));
    if n == 0 { 0.0 } else { (sum / n as f64).sqrt() }
}

fn rotate(p: Point, deg: f64) -> Point {
    let (c, s) = cos_sin(deg);
    [c * p[0] - s * p[1], s * p[0] + c * p[1]]
}

fn centroid(points: &[Point]) -> Point {
    let n = points.len().max(1) as f64;
    let sum = points
        .iter()
        .fold([0.0, 0.0], |a, p| [a[0] + p[0], a[1] + p[1]]);
    [sum[0] / n, sum[1] / n]
}

/// The turn (degrees, about the origin) and shift mapping `from` onto `to`
/// by least squares: `to ≈ R(angle)·from + shift`.
fn solve_2d(from: &[Point], to: &[Point], rotation: FitRotation) -> (f64, Point) {
    let (cf, ct) = (centroid(from), centroid(to));
    let shift_for = |angle: f64| {
        let r = rotate(cf, angle);
        [ct[0] - r[0], ct[1] - r[1]]
    };
    let angle = match rotation {
        FitRotation::Free => {
            let (mut dot, mut cross) = (0.0, 0.0);
            for (a, b) in from.iter().zip(to) {
                let (a, b) = ([a[0] - cf[0], a[1] - cf[1]], [b[0] - ct[0], b[1] - ct[1]]);
                dot += a[0] * b[0] + a[1] * b[1];
                cross += a[0] * b[1] - a[1] * b[0];
            }
            if dot.hypot(cross) < 1e-12 {
                0.0
            } else {
                snap_degrees(cross.atan2(dot).to_degrees())
            }
        }
        FitRotation::Quarter => {
            let error = |angle: f64| {
                let shift = shift_for(angle);
                from.iter()
                    .zip(to)
                    .map(|(a, b)| {
                        let r = rotate(*a, angle);
                        (r[0] + shift[0] - b[0]).powi(2) + (r[1] + shift[1] - b[1]).powi(2)
                    })
                    .sum::<f64>()
            };
            // Ties keep the smallest turn, so a symmetric part stays put.
            [0.0, 90.0, -90.0, 180.0]
                .into_iter()
                .map(|a| (a, error(a)))
                .fold((0.0, f64::INFINITY), |best, (a, e)| {
                    if e < best.1 - 1e-12 { (a, e) } else { best }
                })
                .0
        }
    };
    (angle, shift_for(angle))
}

/// An angle within 1e-9° of a quarter turn becomes exact, in (−180, 180].
fn snap_degrees(deg: f64) -> f64 {
    let quarter = (deg / 90.0).round() * 90.0;
    let deg = if (deg - quarter).abs() < 1e-9 {
        quarter
    } else {
        deg
    };
    let wrapped = deg.rem_euclid(360.0);
    let wrapped = if wrapped > 180.0 {
        wrapped - 360.0
    } else {
        wrapped
    };
    if wrapped == 0.0 { 0.0 } else { wrapped }
}

/// The intrinsic X → Y → Z angles (degrees) of a rotation matrix:
/// `R = Rx(a)·Ry(b)·Rz(c)`. Quarter turns come out exact.
pub fn intrinsic_xyz_of(r: &[[f64; 3]; 3]) -> Point3 {
    let sb = r[0][2].clamp(-1.0, 1.0);
    let (a, b, c) = if sb.abs() > 1.0 - 1e-12 {
        // Gimbal lock: only a ± c is defined; put it all in a.
        (
            r[2][1].atan2(r[1][1]),
            sb.signum() * 90.0_f64.to_radians(),
            0.0,
        )
    } else {
        (
            (-r[1][2]).atan2(r[2][2]),
            sb.asin(),
            (-r[0][1]).atan2(r[0][0]),
        )
    };
    [a, b, c].map(|v| snap_degrees(v.to_degrees()))
}

/// The placement of a rigid model → datum transform (no mirror), stored
/// through the one crossing.
fn placement_of(t: &Transform3) -> Result<ModelPlacement> {
    ModelPlacement::from_mm(t.translation, intrinsic_xyz_of(&t.rotation)).map_err(|_| {
        Error::Invalid("package.placement: the result is not finite or exceeds ±2 m".into())
    })
}

/// `outer ∘ placement`: the placement moved by a datum-frame transform.
pub fn moved(placement: &ModelPlacement, outer: &Transform3) -> Result<ModelPlacement> {
    placement_of(&outer.then(&placement.transform()))
}

/// A turn of `deg` about datum `axis` through `pivot`.
pub fn turn_about(axis: Axis, deg: f64, pivot: Point3) -> Transform3 {
    let turn = match axis {
        Axis::X => Transform3::rot_x(deg),
        Axis::Y => Transform3::rot_y(deg),
        Axis::Z => Transform3::rot_z(deg),
    };
    Transform3::translation(pivot)
        .then(&turn)
        .then(&Transform3::translation(pivot.map(|v| -v)))
}

/// The body's box in the model frame (min, max): the envelope.
fn body_box(package: &PackageDefinition) -> (Point3, Point3) {
    let [l, w, h] = package.envelope();
    ([-l / 2.0, -w / 2.0, 0.0], [l / 2.0, w / 2.0, h])
}

/// Each stored lead contact of `package` (model frame, on the seating
/// plane) with its terminal.
fn contacts(package: &PackageDefinition) -> Vec<(PackageTerminalId, Point3)> {
    package
        .leads
        .iter()
        .map(|l| {
            let [x, y] = l.contact.to_mm_f64();
            (l.terminal, [x, y, 0.0])
        })
        .collect()
}

fn box_corners((min, max): (Point3, Point3)) -> [Point3; 8] {
    std::array::from_fn(|k| {
        [
            if k & 1 == 0 { min[0] } else { max[0] },
            if k & 2 == 0 { min[1] } else { max[1] },
            if k & 4 == 0 { min[2] } else { max[2] },
        ]
    })
}

/// The body's corners in the datum frame under `placement`.
pub fn body_corners(package: &PackageDefinition, placement: &ModelPlacement) -> [Point3; 8] {
    let t = placement.transform();
    box_corners(body_box(package)).map(|p| t.apply(p))
}

/// The centre of the placed body in the datum frame: the pivot quick turns
/// and the triad use.
pub fn body_centre(package: &PackageDefinition, placement: &ModelPlacement) -> Point3 {
    let (min, max) = body_box(package);
    placement
        .transform()
        .apply(std::array::from_fn(|i| (min[i] + max[i]) / 2.0))
}

/// Footprint frame → datum frame.
pub fn to_datum(datum: &Datum, p: Point) -> Point {
    rotate(
        [p[0] - datum.offset()[0], p[1] - datum.offset()[1]],
        -datum.rotation_deg(),
    )
}

/// Datum frame → footprint frame: [`to_datum`] undone.
pub fn from_datum(datum: &Datum, p: Point) -> Point {
    let q = rotate(p, datum.rotation_deg());
    [q[0] + datum.offset()[0], q[1] + datum.offset()[1]]
}

fn copper(kind: PadKind) -> bool {
    matches!(kind, PadKind::Smd | PadKind::PlatedThrough)
}

/// The lead/pad pairs of `package`'s stored lead contacts on `footprint`:
/// each contact with the centroid of the copper pads landing its terminal,
/// in the datum frame. Terminals without a pad are left out; never inferred
/// from numbers or names.
pub fn form_pairs(
    package: &PackageDefinition,
    footprint: &FootprintDefinition,
) -> Result<Vec<LeadPair>> {
    let contacts = contacts(package);
    if contacts.is_empty() {
        return Err(Error::Invalid(
            "package.fit_no_leads: the package has no lead contacts; pass lead pairs".into(),
        ));
    }
    let pairs: Vec<LeadPair> = contacts
        .into_iter()
        .filter_map(|(terminal, lead)| {
            let pads: Vec<_> = footprint
                .pads
                .iter()
                .filter(|p| p.terminal == Some(terminal) && copper(p.kind))
                .collect();
            let first = pads.first()?;
            let centres: Vec<Point> = pads
                .iter()
                .map(|p| to_datum(&footprint.datum, p.position()))
                .collect();
            Some(LeadPair::from_mm(
                terminal,
                first.id,
                lead,
                centroid(&centres),
            ))
        })
        .collect::<Result<_>>()?;
    if pairs.is_empty() {
        return Err(Error::Invalid(
            "package.fit_no_pads: no footprint pad lands a terminal of the package".into(),
        ));
    }
    Ok(pairs)
}

/// Host-supplied pairs name terminals of `package` and, when the landing
/// footprint is known, its pads landing that terminal; never accepted
/// unchecked.
pub fn check_pair_ids(
    package: &PackageDefinition,
    footprint: Option<&FootprintDefinition>,
    pairs: &[LeadPair],
) -> Result<()> {
    for pair in pairs {
        if package.terminal(pair.terminal).is_none() {
            return Err(Error::NotFound(format!(
                "package.fit_pairs: the package has no terminal {}",
                pair.terminal
            )));
        }
        if let Some(footprint) = footprint {
            let pad = footprint.pad(pair.pad).ok_or_else(|| {
                Error::NotFound(format!(
                    "package.fit_pairs: the footprint has no pad {}",
                    pair.pad
                ))
            })?;
            if pad.terminal != Some(pair.terminal) {
                return Err(Error::Invalid(format!(
                    "package.fit_pairs: pad {} does not land terminal {}",
                    pair.pad, pair.terminal
                )));
            }
        }
    }
    Ok(())
}

/// A fit residual that placement cannot remove: the leads' spread and the
/// pads' differ, so the body or the footprint must change. Returns `(lead
/// spacing, pad spacing)` of two pairs far apart (the pad furthest from the
/// pads' centroid and the pad furthest from it), when they differ by more
/// than `tolerance_mm`. Linear in the pairs.
pub fn spacing_mismatch(pairs: &[LeadPair], tolerance_mm: f64) -> Option<(f64, f64)> {
    let distance = |p: Point, q: Point| (p[0] - q[0]).hypot(p[1] - q[1]);
    let furthest = |from: Point| {
        pairs.iter().max_by(|a, b| {
            distance(a.pad_centre(), from).total_cmp(&distance(b.pad_centre(), from))
        })
    };
    let centre = centroid(&pairs.iter().map(LeadPair::pad_centre).collect::<Vec<_>>());
    let a = furthest(centre)?;
    let b = furthest(a.pad_centre())?;
    let (la, lb) = (a.lead(), b.lead());
    let leads = distance([la[0], la[1]], [lb[0], lb[1]]);
    let pads = distance(a.pad_centre(), b.pad_centre());
    ((leads - pads).abs() > tolerance_mm).then_some((leads, pads))
}

fn needs_footprint(action: QuickAction) -> Error {
    Error::Invalid(format!(
        "package.quick_needs_footprint: {} needs the footprint the package lands on",
        action.label()
    ))
}

/// Move the lowest lead contact (or, without leads, the body's underside)
/// onto the seating plane.
fn seat(package: &PackageDefinition, placement: &ModelPlacement) -> Result<ModelPlacement> {
    let t = placement.transform();
    let contacts: Vec<Point3> = contacts(package).into_iter().map(|(_, p)| p).collect();
    let lowest = if contacts.is_empty() {
        body_corners(package, placement)
            .iter()
            .map(|p| p[2])
            .fold(f64::INFINITY, f64::min)
    } else {
        contacts
            .iter()
            .map(|p| t.apply(*p)[2])
            .fold(f64::INFINITY, f64::min)
    };
    moved(placement, &Transform3::translation([0.0, 0.0, -lowest]))
}

/// Centre the lead contacts on their pads in x/y; without typed leads, the
/// body's centre on the centroid of all copper pads.
fn centre_on_pads(
    package: &PackageDefinition,
    footprint: &FootprintDefinition,
    placement: &ModelPlacement,
) -> Result<ModelPlacement> {
    let t = placement.transform();
    let (from, to) = match form_pairs(package, footprint) {
        Ok(pairs) => (
            centroid(
                &pairs
                    .iter()
                    .map(|p| {
                        let q = t.apply(p.lead());
                        [q[0], q[1]]
                    })
                    .collect::<Vec<_>>(),
            ),
            centroid(&pairs.iter().map(LeadPair::pad_centre).collect::<Vec<_>>()),
        ),
        Err(_) => {
            let pads: Vec<Point> = footprint
                .pads
                .iter()
                .filter(|p| copper(p.kind))
                .map(|p| to_datum(&footprint.datum, p.position()))
                .collect();
            if pads.is_empty() {
                return Err(Error::Invalid(
                    "package.quick_no_pads: the footprint has no copper pads".into(),
                ));
            }
            let c = body_centre(package, placement);
            ([c[0], c[1]], centroid(&pads))
        }
    };
    moved(
        placement,
        &Transform3::translation([to[0] - from[0], to[1] - from[1], 0.0]),
    )
}

/// The quarter turn about z that brings terminal 1's lead nearest pad 1,
/// then centred on the pads.
fn pin1_to_pad1(
    package: &PackageDefinition,
    footprint: &FootprintDefinition,
    placement: &ModelPlacement,
) -> Result<ModelPlacement> {
    let pairs = form_pairs(package, footprint)?;
    let first = package.terminals.first().map(|t| t.id);
    let pin1 = pairs
        .iter()
        .find(|p| Some(p.terminal) == first)
        .ok_or_else(|| {
            Error::Invalid("package.quick_no_pad1: terminal 1 has no footprint pad".into())
        })?;
    let pivot = body_centre(package, placement);
    let mut best: Option<(f64, ModelPlacement)> = None;
    for k in 0..4 {
        let turned = moved(placement, &turn_about(Axis::Z, 90.0 * f64::from(k), pivot))?;
        let centred = centre_on_pads(package, footprint, &turned)?;
        let q = centred.transform().apply(pin1.lead());
        let c = pin1.pad_centre();
        let error = (q[0] - c[0]).hypot(q[1] - c[1]);
        if best.as_ref().is_none_or(|(e, _)| error < *e - 1e-9) {
            best = Some((error, centred));
        }
    }
    best.map(|(_, p)| p)
        .ok_or_else(|| Error::Invalid("package.quick_no_pad1: no candidate turn".into()))
}

/// The placement a quick action gives. `footprint` is the footprint the
/// package lands on; the pad-relative actions refuse without it.
pub fn apply_quick(
    package: &PackageDefinition,
    footprint: Option<&FootprintDefinition>,
    action: QuickAction,
) -> Result<ModelPlacement> {
    let current = &package.placement;
    let pivot = body_centre(package, current);
    match action {
        QuickAction::Rotate {
            axis,
            quarter_turns,
        } => {
            if !(-3..=3).contains(&quarter_turns) || quarter_turns == 0 {
                return Err(Error::Invalid(
                    "package.quick_turns: rotate by 1–3 quarter turns either way".into(),
                ));
            }
            moved(
                current,
                &turn_about(axis, 90.0 * f64::from(quarter_turns), pivot),
            )
        }
        QuickAction::Flip => moved(current, &turn_about(Axis::X, 180.0, pivot)),
        QuickAction::StandUp => {
            // The source's +y becomes the datum's +z: a quarter turn about
            // the model's own x axis, then seated.
            let stood = placement_of(&current.transform().then(&Transform3::rot_x(90.0)))?;
            seat(package, &stood)
        }
        QuickAction::Seat => seat(package, current),
        QuickAction::CentreOnPads => {
            centre_on_pads(package, footprint.ok_or(needs_footprint(action))?, current)
        }
        QuickAction::Pin1ToPad1 => {
            pin1_to_pad1(package, footprint.ok_or(needs_footprint(action))?, current)
        }
        QuickAction::CentreOnCourtyard => {
            let footprint = footprint.ok_or(needs_footprint(action))?;
            let to = courtyard_centre(footprint).ok_or_else(|| {
                Error::Invalid(
                    "package.quick_no_pads: the footprint has no courtyard and no pads".into(),
                )
            })?;
            let to = to_datum(&footprint.datum, to);
            moved(
                current,
                &Transform3::translation([to[0] - pivot[0], to[1] - pivot[1], 0.0]),
            )
        }
    }
}

/// The centre a body sits over on `footprint`, in the footprint frame: the
/// box of its courtyard graphics, or without any the box of every pad and
/// hole. `None` for a footprint with neither.
pub fn courtyard_centre(footprint: &FootprintDefinition) -> Option<Point> {
    use super::graphics::{DrawingLayer, Item};
    footprint
        .graphics
        .iter()
        .filter(|g| g.layer == DrawingLayer::Courtyard)
        .filter_map(|g| match &g.item {
            Item::Shape { shape, .. } => Some(shape.to_mm().bounds(0.0)),
            Item::Text(_) => None,
        })
        .reduce(Bounds::union)
        .or_else(|| footprint.pads.iter().map(Pad::bounds).reduce(Bounds::union))
        .map(|b| b.center())
}

/// Where an envelope model sits by default on `footprint`: over the
/// courtyard, so a footprint whose origin is not its middle (a connector
/// drawn from the board edge) does not leave the box hanging off it. Only an
/// envelope without lead contacts at the default placement has a default:
/// a body with lead contacts is located by its leads, and a placement
/// someone set stays. `None` when nothing would move.
pub fn envelope_default(
    package: &PackageDefinition,
    footprint: &FootprintDefinition,
) -> Result<Option<ModelPlacement>> {
    if !matches!(package.model, PackageModel::Envelope { .. })
        || !package.leads.is_empty()
        || package.placement != ModelPlacement::default()
    {
        return Ok(None);
    }
    if courtyard_centre(footprint).is_none() {
        return Ok(None);
    }
    let placement = apply_quick(package, Some(footprint), QuickAction::CentreOnCourtyard)?;
    let [x, y, _] = placement.translation_nm;
    let moved = x != Nm::ZERO || y != Nm::ZERO;
    Ok(moved.then_some(placement))
}

/// [`package_model::edit_package`] with the footprint the package lands
/// on, so the pad-relative quick actions can run. Atomic: any failure
/// returns an error and nothing else.
pub fn edit_package_in(
    package: &PackageDefinition,
    footprint: Option<&FootprintDefinition>,
    edits: &[PackageEdit],
) -> Result<PackageDefinition> {
    if edits.is_empty() || edits.len() > 256 {
        return Err(Error::Invalid(
            "library.edit_package takes 1–256 edits".into(),
        ));
    }
    let mut out = package.clone();
    for edit in edits {
        match edit {
            PackageEdit::ApplyQuick(action) => {
                out.placement = apply_quick(&out, footprint, *action)?;
            }
            other => out = package_model::edit_package(&out, std::slice::from_ref(other))?,
        }
    }
    Ok(out)
}

/// Where one typed lead contact lands under a placement.
#[derive(Clone, Debug, PartialEq)]
pub struct LeadState {
    pub terminal: PackageTerminalId,
    /// The contact in the datum frame (z: height above the seating plane).
    pub at: Point3,
    /// How far it misses its best land: past [`LEAD_OFF_LAND_FRACTION`] of
    /// the land width across the land, or outside it (≤ 0 is on the land);
    /// `None` when no copper pad lands the terminal.
    pub miss_mm: Option<f64>,
    pub pads: Vec<PadId>,
    /// Where the contact sits on that best land.
    pub spot: Option<LandSpot>,
}

/// Where a lead contact sits on a land, in the land's own axes: along the
/// lead (its direction from the package's centre) and across it, the axes
/// [`land_miss`] reads. A lead at the package's centre (an exposed pad) has
/// no direction: along is then the land's longer side, positive along the
/// pad's own axis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LandSpot {
    pub pad: PadId,
    /// The contact's offset from the land's centre along the lead, mm:
    /// positive outward, away from the package's centre.
    pub along_mm: f64,
    /// Its offset across the lead, mm: positive to the left of outward
    /// (counter-clockwise, seen from the top).
    pub across_mm: f64,
    /// The land's size along the lead and across it, mm.
    pub length_mm: f64,
    pub width_mm: f64,
}

/// The index of the land axis across the lead (0: the pad's x, 1: its y)
/// and the sign of outward along the other axis, in the pad's frame.
fn land_axes(pad: &Pad, outward: Point) -> (usize, f64) {
    let extent = pad.shape.extent();
    let along = rotate(outward, -pad.rotation_deg());
    if along[0].hypot(along[1]) < OUTWARD_MIN_MM {
        (if extent[0] <= extent[1] { 0 } else { 1 }, 1.0)
    } else if along[0].abs() >= along[1].abs() {
        (1, if along[0] < 0.0 { -1.0 } else { 1.0 })
    } else {
        (0, if along[1] < 0.0 { -1.0 } else { 1.0 })
    }
}

/// `q` (footprint frame, mm) in `pad`'s frame, from its centre.
fn pad_local(pad: &Pad, q: Point) -> Point {
    rotate(
        [q[0] - pad.position()[0], q[1] - pad.position()[1]],
        -pad.rotation_deg(),
    )
}

/// Where a lead contact at `q` (footprint frame, mm) running along
/// `outward` sits on `pad`'s land.
pub fn land_spot(pad: &Pad, q: Point, outward: Point) -> LandSpot {
    let local = pad_local(pad, q);
    let extent = pad.shape.extent();
    let (across, sign) = land_axes(pad, outward);
    let along = 1 - across;
    // Left of outward: a quarter turn counter-clockwise of the along axis.
    let left = if along == 0 { local[1] } else { -local[0] };
    LandSpot {
        pad: pad.id,
        along_mm: sign * local[along],
        across_mm: sign * left,
        length_mm: extent[along],
        width_mm: extent[across],
    }
}

/// How far a lead contact at `q` (footprint frame, mm) misses `pad`'s
/// land: past [`LEAD_OFF_LAND_FRACTION`] of the land width across the
/// lead, or outside the land; ≤ 0 is on the land. The one lead-on-land
/// rule. `outward` is the lead's direction from the package's centre in
/// the footprint frame (its contact in the model frame, turned): a lead
/// runs along it, so "across" is the land axis nearer perpendicular to it
/// (a gull-wing foot's land width, a chip termination's land width, which
/// is often the land's longer side). A lead at the centre (an exposed pad)
/// has no direction: across is then the land's narrower side.
pub fn land_miss(pad: &Pad, q: Point, outward: Point) -> f64 {
    let local = pad_local(pad, q);
    let extent = pad.shape.extent();
    let (across, _) = land_axes(pad, outward);
    let lateral = local[across].abs() - LEAD_OFF_LAND_FRACTION * extent[across];
    let outside = pad.land(PAD_ARC_TOLERANCE_MM).distance(q, true);
    lateral.max(outside)
}

/// A lead contact this near the package's centre has no outward direction.
const OUTWARD_MIN_MM: f64 = 1e-6;

/// The stored lead contacts of `package` under `placement` against the
/// footprint's lands; empty when the package has none.
pub fn lead_states(
    package: &PackageDefinition,
    footprint: &FootprintDefinition,
    placement: &ModelPlacement,
) -> Vec<LeadState> {
    let datum = placement.transform();
    let frame = model_frame(&footprint.datum, placement);
    contacts(package)
        .into_iter()
        .map(|(terminal, lead)| {
            let q = frame.apply(lead);
            let o = frame.apply_vector(lead);
            let (q, o) = ([q[0], q[1]], [o[0], o[1]]);
            let pads: Vec<_> = footprint
                .pads
                .iter()
                .filter(|p| p.terminal == Some(terminal) && copper(p.kind))
                .collect();
            let best = pads
                .iter()
                .map(|pad| (land_miss(pad, q, o), *pad))
                .reduce(|a, b| if b.0 < a.0 { b } else { a });
            LeadState {
                terminal,
                at: datum.apply(lead),
                miss_mm: best.map(|(miss, _)| miss),
                pads: pads.iter().map(|p| p.id).collect(),
                spot: best.map(|(_, pad)| land_spot(pad, q, o)),
            }
        })
        .collect()
}

/// Advisory placement findings of `package` on `footprint` (whose lineage
/// is `footprint_id`): lead contacts off their lands (more than
/// [`LEAD_OFF_LAND_FRACTION`] of the land width across it, or outside it),
/// lead contacts off the seating plane, and a body below the board.
pub fn placement_findings(
    package: &PackageDefinition,
    footprint: &FootprintDefinition,
    footprint_id: DefinitionId,
) -> Vec<Finding> {
    let mut out = Vec::new();
    let package_id = footprint.package.id;
    let lowest = body_corners(package, &package.placement)
        .iter()
        .map(|p| p[2])
        .fold(f64::INFINITY, f64::min);
    if lowest < -SEAT_TOLERANCE_MM {
        out.push(Finding::new(
            "package.body_below_board",
            format!(
                "The package body reaches {:.3} mm below the seating plane, into the board.",
                -lowest
            ),
            vec![crate::diagnostics::ObjectRef::Definition { id: package_id }],
        ));
    }
    let (mut off, mut off_count, mut off_worst) = (Vec::new(), 0usize, 0.0_f64);
    let (mut unseated, mut unseated_worst) = (Vec::new(), 0.0_f64);
    for lead in lead_states(package, footprint, &package.placement) {
        if lead.at[2].abs() > SEAT_TOLERANCE_MM {
            unseated.push(object(package_id, lead.terminal.0));
            unseated_worst = unseated_worst.max(lead.at[2].abs());
        }
        if let Some(miss) = lead.miss_mm.filter(|m| *m > 1e-9) {
            off.extend(lead.pads.iter().map(|p| object(footprint_id, p.0)));
            off.push(object(package_id, lead.terminal.0));
            off_count += 1;
            off_worst = off_worst.max(miss);
        }
    }
    if !off.is_empty() {
        out.push(Finding::new(
            "package.lead_off_land",
            format!(
                "{} lead contact(s) lie off their lands, by up to {:.3} mm past {} % of the land width (estimate).",
                off_count,
                off_worst,
                LEAD_OFF_LAND_FRACTION * 100.0
            ),
            off,
        ));
    }
    if !unseated.is_empty() {
        out.push(Finding::new(
            "package.lead_unseated",
            format!(
                "{} lead contact(s) are off the seating plane, by up to {unseated_worst:.3} mm; use Seat.",
                unseated.len()
            ),
            unseated,
        ));
    }
    out
}

#[cfg(test)]
mod tests;
