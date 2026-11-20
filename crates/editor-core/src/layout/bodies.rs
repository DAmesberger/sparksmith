//! Placed 3D bodies against their pads, in board coordinates
//! (`board.body_off_pads`, `project.check_bodies`). Only the bodies the
//! active assembly variant fits are checked.
//!
//! Each lead contact a package stores ([`PackageLead`], model frame) goes
//! to the board the way the 3D view places the body
//! ([`model_to_board`]: model placement, bottom-side half turn, rotation,
//! seating height); each pad goes to the board the way the copper is drawn
//! ([`placement_transform`]). The contact is then read in the footprint
//! frame against the lands of its terminal by the one lead-on-land rule
//! ([`package_fit::land_miss`]), so the 3D path and the 2D path are checked
//! against each other, not against themselves. A package without stored
//! lead contacts is listed, never guessed.
use super::{model_to_board, placed_courtyards, placement_transform, seat_height};
use crate::diagnostics::{Finding, ObjectRef};
use crate::geometry::{Bounds, Point, Point3};
use crate::ids::Id;
use crate::instances;
use crate::library::package_fit::{LEAD_OFF_LAND_FRACTION, land_miss};
use crate::library::{FootprintDefinition, PackageDefinition, PackageModel, PadKind};
use crate::model::{Layer, Placement, Project};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The finding a placed body gives when a lead contact misses its land.
pub const BODY_OFF_PADS: &str = "board.body_off_pads";

/// One lead contact of a placed body.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct PlacedLead {
    /// The package terminal's designation (the pad number).
    pub terminal: String,
    /// The contact on the board, mm (z: height in the board's frame).
    pub at: Point3,
    /// How far it misses the nearest land of its terminal past the rule
    /// (on the land, within the middle half across it), mm; ≤ 0 lands.
    /// `None` when no copper pad lands the terminal.
    pub miss_mm: Option<f64>,
    /// Its distance from the centre of that land, mm.
    pub off_centre_mm: Option<f64>,
}

/// Whether pin 1's lead lands pin 1's pad.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum PinOne {
    /// The copper pad nearest pin 1's lead lands pin 1.
    Agrees,
    /// The copper pad nearest pin 1's lead is another terminal's.
    Disagrees { nearest: String },
    /// The package has no terminal 1 with a lead contact and a pad.
    Unknown,
}

/// One placed part's body against its pads.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct PlacedBody {
    pub instance: Id,
    pub designator: String,
    /// What the 3D model is (`PackageModel::describe`).
    pub model: String,
    pub layer: Layer,
    pub rotation_deg: f64,
    /// Each lead contact, worst first.
    pub leads: Vec<PlacedLead>,
    /// Leads off their lands (`miss_mm` > 0).
    pub off_pads: usize,
    pub worst_miss_mm: Option<f64>,
    pub worst_off_centre_mm: Option<f64>,
    pub pin_one: PinOne,
    /// The body's box seen from above, on the board, mm.
    pub body: Bounds,
    /// The box of its copper pads on the board, mm.
    pub pads: Option<Bounds>,
    /// The box of its courtyard on the board, mm.
    pub courtyard: Option<Bounds>,
    /// The body's height over its side's board surface, mm.
    pub height_mm: f64,
    /// The lowest lead contact's height over that surface, mm (0 seats).
    pub seat_mm: Option<f64>,
}

/// Every placed body with lead contacts, and the placed parts whose body
/// has none to check.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct BodiesReport {
    /// Bodies with lead contacts, in placement order.
    pub parts: Vec<PlacedBody>,
    /// Placed parts with a model but without lead contacts, by designator.
    pub without_leads: Vec<String>,
    /// Placed parts without a 3D model of their own (an envelope box).
    pub envelopes: usize,
}

impl BodiesReport {
    /// Parts with at least one lead off its land.
    pub fn off_pads(&self) -> usize {
        self.parts.iter().filter(|p| p.off_pads > 0).count()
    }
}

fn copper(kind: PadKind) -> bool {
    matches!(kind, PadKind::Smd | PadKind::PlatedThrough)
}

/// The body's corners in the model frame: the part file's evaluated
/// bounds, else the envelope box.
fn model_corners(package: &PackageDefinition) -> [Point3; 8] {
    match &package.model {
        PackageModel::PartFile(part) => {
            let [lo, hi] = part.derived.bounds();
            std::array::from_fn(|i| {
                [
                    if i & 1 == 0 { lo[0] } else { hi[0] },
                    if i & 2 == 0 { lo[1] } else { hi[1] },
                    if i & 4 == 0 { lo[2] } else { hi[2] },
                ]
            })
        }
        _ => package.envelope_corners(),
    }
}

/// One placed body against its footprint's pads, `None` without lead
/// contacts (or a stackup without copper on its side).
pub fn placed_body(
    project: &Project,
    placement: &Placement,
    designator: &str,
    package: &PackageDefinition,
    footprint: &FootprintDefinition,
    courtyard: Option<Bounds>,
) -> Option<PlacedBody> {
    if package.leads.is_empty() {
        return None;
    }
    let to_board = model_to_board(placement, package, &project.stackup).ok()?;
    let pad_frame = placement_transform(placement, &footprint.datum);
    let back = pad_frame.inverse();
    let surface = seat_height(placement, &project.stackup).ok()?;
    let height = |z: f64| (z - surface) * up(placement);
    let designation = |t| {
        package
            .terminal(t)
            .map_or_else(|| t.to_string(), |t| t.designation.clone())
    };
    let pads: Vec<_> = footprint.pads.iter().filter(|p| copper(p.kind)).collect();
    let mut leads = Vec::with_capacity(package.leads.len());
    let mut first = None;
    for lead in &package.leads {
        let [x, y] = lead.contact.to_mm_f64();
        let at = to_board.apply([x, y, 0.0]);
        let local: Point = back.apply([at[0], at[1]]);
        let o = to_board.apply_vector([x, y, 0.0]);
        let outward = back.apply_vector([o[0], o[1]]);
        let own: Vec<_> = pads
            .iter()
            .filter(|p| p.terminal == Some(lead.terminal))
            .collect();
        let best = own
            .iter()
            .map(|p| (land_miss(p, local, outward), p))
            .min_by(|a, b| a.0.total_cmp(&b.0));
        let terminal = designation(lead.terminal);
        if terminal == "1" && first.is_none() {
            let nearest = pads.iter().min_by(|a, b| {
                distance(a.position(), local).total_cmp(&distance(b.position(), local))
            });
            first = Some(match (best, nearest) {
                (Some(_), Some(n)) if n.terminal == Some(lead.terminal) => PinOne::Agrees,
                (Some(_), Some(n)) => PinOne::Disagrees {
                    nearest: n.number.clone(),
                },
                _ => PinOne::Unknown,
            });
        }
        leads.push(PlacedLead {
            terminal,
            at,
            miss_mm: best.map(|(m, _)| m),
            off_centre_mm: best.map(|(_, p)| distance(p.position(), local)),
        });
    }
    leads.sort_by(|a, b| {
        b.miss_mm
            .unwrap_or(f64::NEG_INFINITY)
            .total_cmp(&a.miss_mm.unwrap_or(f64::NEG_INFINITY))
    });
    let worst = |f: fn(&PlacedLead) -> Option<f64>| leads.iter().filter_map(f).reduce(f64::max);
    let corners = model_corners(package).map(|c| to_board.apply(c));
    let body = Bounds::of(corners.iter().map(|c| [c[0], c[1]]))?;
    let height_mm = corners.iter().map(|c| height(c[2])).fold(0.0, f64::max);
    let pad_box = pads
        .iter()
        .filter_map(|p| {
            let b = p.bounds();
            Bounds::of(
                [
                    [b.min[0], b.min[1]],
                    [b.max[0], b.min[1]],
                    [b.max[0], b.max[1]],
                    [b.min[0], b.max[1]],
                ]
                .map(|q| pad_frame.apply(q)),
            )
        })
        .reduce(Bounds::union);
    Some(PlacedBody {
        instance: placement.instance,
        designator: designator.to_owned(),
        model: package.model.describe(),
        layer: placement.layer,
        rotation_deg: placement.rotation_deg,
        off_pads: leads
            .iter()
            .filter(|l| l.miss_mm.is_some_and(|m| m > MISS_EPSILON_MM))
            .count(),
        worst_miss_mm: worst(|l| l.miss_mm),
        worst_off_centre_mm: worst(|l| l.off_centre_mm),
        seat_mm: leads.iter().map(|l| height(l.at[2])).reduce(f64::min),
        leads,
        pin_one: first.unwrap_or(PinOne::Unknown),
        body,
        pads: pad_box,
        courtyard,
        height_mm,
    })
}

/// A miss below a nanometre is on the land (rounding).
const MISS_EPSILON_MM: f64 = 1e-6;

/// +1 on the top, −1 on the bottom: the direction away from the board on
/// the placement's side.
fn up(placement: &Placement) -> f64 {
    match placement.layer {
        Layer::Top => 1.0,
        Layer::Bottom => -1.0,
    }
}

fn distance(a: Point, b: Point) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

/// Every placed body of `project` against its pads.
pub fn bodies(project: &Project) -> BodiesReport {
    let mut courtyards: BTreeMap<Id, Bounds> = BTreeMap::new();
    for c in placed_courtyards(project) {
        let b = c.shape.bounds(0.0);
        courtyards
            .entry(c.instance)
            .and_modify(|a| *a = a.union(b))
            .or_insert(b);
    }
    let mut report = BodiesReport::default();
    for placement in &project.placements {
        let Some(instance) = project.instance(placement.instance) else {
            continue;
        };
        // The part the active variant fits: none, or another Part's body
        // on the same lands (docs/assembly-variants.md).
        let fitted = instances::variants::fitted(project, instance);
        if !fitted.populated() {
            continue;
        }
        let Some(physical) = fitted.physical else {
            continue;
        };
        let package = &physical.package.definition;
        // A part of no body has nothing to land.
        if matches!(package.model, PackageModel::NoBody) {
            continue;
        }
        if package.leads.is_empty() && matches!(package.model, PackageModel::Envelope { .. }) {
            report.envelopes += 1;
            continue;
        }
        match placed_body(
            project,
            placement,
            &instance.designator,
            package,
            &physical.footprint.definition,
            courtyards.get(&instance.id).copied(),
        ) {
            Some(body) => report.parts.push(body),
            None => report.without_leads.push(instance.designator.clone()),
        }
    }
    report
}

/// `board.body_off_pads`: one finding per placed part whose body has a
/// lead contact off its land.
pub fn checks(project: &Project) -> Vec<Finding> {
    bodies(project)
        .parts
        .iter()
        .filter(|b| b.off_pads > 0)
        .map(|b| {
            Finding::new(
                BODY_OFF_PADS,
                format!(
                    "{}: {} body lead(s) off their pads, by up to {:.3} mm past {} % of the land width (estimate).",
                    b.designator,
                    b.off_pads,
                    b.worst_miss_mm.unwrap_or(0.0),
                    LEAD_OFF_LAND_FRACTION * 100.0
                ),
                vec![ObjectRef::Instance { id: b.instance }],
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Result;
    use crate::diagnostics::Error;
    use crate::geometry::Transform3;
    use crate::layout::tests::{fixture, ids};
    use crate::library::{ModelPlacement, PackageLead};
    use crate::units::PointNm;

    fn missing(what: &str) -> Error {
        Error::Invalid(format!("fixture: {what}"))
    }

    /// A rigid transform's inverse applied to `p`: `Rᵀ (p − t)`.
    fn back(t: &Transform3, p: Point3) -> Point3 {
        let d: Point3 = std::array::from_fn(|i| p[i] - t.translation[i]);
        std::array::from_fn(|i| (0..3).map(|k| t.rotation[k][i] * d[k]).sum())
    }

    /// The demo board with R1's footprint made asymmetric (pad 2 moved off
    /// the axis, so a mirror or a wrong turn shows) and its package's lead
    /// contacts on the pad centres, in the model frame of `model`.
    fn asymmetric(model: &ModelPlacement) -> Result<(Project, Id)> {
        let mut project = fixture()?;
        let (r1, _) = ids()?;
        let instance = project.instance(r1).ok_or_else(|| missing("R1"))?;
        let physical = instances::resolve(&project.library, instance)?
            .physical
            .ok_or_else(|| missing("R1 footprint"))?;
        let (footprint, package) = (physical.footprint.id, physical.package.id);
        for f in project.library.footprints.iter_mut() {
            if f.id == footprint
                && let Some(pad) = f.definition.pads.get_mut(1)
            {
                let [x, _] = pad.position.to_mm_f64();
                pad.position = PointNm::from_mm_f64_rounded([x, 0.7])?;
            }
        }
        let fp = project
            .library
            .footprints
            .iter()
            .find(|f| f.id == footprint)
            .map(|f| f.definition.clone())
            .ok_or_else(|| missing("footprint"))?;
        let frame = model.transform();
        for p in project.library.packages.iter_mut() {
            if p.id != package {
                continue;
            }
            p.definition.placement = *model;
            p.definition.leads = fp
                .pads
                .iter()
                .filter_map(|pad| {
                    let c = crate::library::package_fit::to_datum(&fp.datum, pad.position());
                    Some((pad.terminal?, back(&frame, [c[0], c[1], 0.0])))
                })
                .map(|(terminal, q)| {
                    Ok(PackageLead {
                        terminal,
                        contact: PointNm::from_mm_f64_rounded([q[0], q[1]])?,
                    })
                })
                .collect::<Result<_>>()?;
        }
        Ok((project, r1))
    }

    fn place(project: &mut Project, r1: Id, rotation_deg: f64, layer: Layer) -> Result<()> {
        let placement = project
            .placements
            .iter_mut()
            .find(|p| p.instance == r1)
            .ok_or_else(|| missing("R1 placement"))?;
        placement.rotation_deg = rotation_deg;
        placement.layer = layer;
        Ok(())
    }

    fn r1_body(project: &Project) -> Result<PlacedBody> {
        let (r1, _) = ids()?;
        bodies(project)
            .parts
            .into_iter()
            .find(|b| b.instance == r1)
            .ok_or_else(|| missing("R1 body"))
    }

    /// Top and bottom, every quarter turn and an odd angle, with an
    /// identity and a turned model placement: every lead on its pad's
    /// centre, pin 1 on pad 1, seated on its side's surface.
    #[test]
    fn leads_land_on_their_pads_on_both_sides_at_every_rotation() -> Result<()> {
        for model in [
            ModelPlacement::default(),
            ModelPlacement::from_mm([0.4, -0.3, 0.0], [0.0, 0.0, 90.0])?,
            ModelPlacement::from_mm([0.0, 0.2, 0.0], [0.0, 0.0, 180.0])?,
        ] {
            let (mut project, r1) = asymmetric(&model)?;
            for layer in [Layer::Top, Layer::Bottom] {
                for rotation in [0.0, 90.0, 180.0, 270.0, 33.0] {
                    place(&mut project, r1, rotation, layer)?;
                    let body = r1_body(&project)?;
                    let case = format!("{layer:?} at {rotation}° with {model:?}");
                    assert_eq!(body.off_pads, 0, "{case}: {:?}", body.leads);
                    assert!(body.worst_off_centre_mm.is_some_and(|d| d < 1e-5), "{case}");
                    assert_eq!(body.pin_one, PinOne::Agrees, "{case}");
                    assert!(body.seat_mm.is_some_and(|z| z.abs() < 1e-9), "{case}");
                    assert!(body.height_mm > 0.0, "{case}");
                    assert!(checks(&project).is_empty(), "{case}");
                }
            }
        }
        Ok(())
    }

    /// A body turned half a turn against its pads, or mirrored (a bottom
    /// view read as a top view), is found on both sides, and the board
    /// check names the part.
    #[test]
    fn a_turned_or_mirrored_body_is_found_on_both_sides() -> Result<()> {
        let (mut project, r1) = asymmetric(&ModelPlacement::default())?;
        let turn = |project: &mut Project, t: Transform3| -> Result<()> {
            for p in project.library.packages.iter_mut() {
                for lead in &mut p.definition.leads {
                    let [x, y] = lead.contact.to_mm_f64();
                    let q = t.apply([x, y, 0.0]);
                    lead.contact = PointNm::from_mm_f64_rounded([q[0], q[1]])?;
                }
            }
            Ok(())
        };
        turn(&mut project, Transform3::rot_z(180.0))?;
        for layer in [Layer::Top, Layer::Bottom] {
            for rotation in [0.0, 90.0] {
                place(&mut project, r1, rotation, layer)?;
                let body = r1_body(&project)?;
                assert!(body.off_pads > 0, "{layer:?} {rotation}°");
                assert!(matches!(body.pin_one, PinOne::Disagrees { .. }));
                let findings = checks(&project);
                assert!(
                    findings.iter().any(|f| f.code == BODY_OFF_PADS
                        && f.objects.contains(&ObjectRef::Instance { id: r1 })),
                    "{findings:?}"
                );
            }
        }
        // Undo the turn, then mirror across x instead (y → −y).
        turn(&mut project, Transform3::rot_z(180.0))?;
        turn(&mut project, Transform3::rot_x(180.0))?;
        for layer in [Layer::Top, Layer::Bottom] {
            place(&mut project, r1, 0.0, layer)?;
            assert!(r1_body(&project)?.off_pads > 0, "{layer:?}");
        }
        Ok(())
    }
}
