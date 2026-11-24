//! Cross-section of a fabricated neck between the lands named by its bridge.
use super::CopperGraph;
use crate::diagnostics::{Finding, FindingCode, Measured, Unit};
use crate::ids::Id;
use crate::instances::{self, ties::TieRow};
use crate::layout::{PlacedBridge, PlacedPad, fill::geom, physical, placed_pads};
use crate::library::tie::{JumperBridge, JumperState, TieForm, TiePurpose};
use crate::model::Project;
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Clone, Default, Debug, PartialEq, Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct TieCurrent {
    pub required_a: Option<f64>,
    pub capacity_a: Option<f64>,
    pub margin_a: Option<f64>,
    pub area_mm2: Option<f64>,
    pub source: Option<String>,
}

// A strip of width w can pass the neck when, after taking w/2 from
// each edge, one remaining piece still reaches all of the end lands.
// Include the actual lands as end caps so the length of a short neck
// does not limit the result. Remove drilled material before measuring.
fn section(
    project: &Project,
    bridge: &PlacedBridge,
    pads: &BTreeMap<(Id, crate::ids::PadId), PlacedPad>,
) -> Option<f64> {
    let tol = crate::layout::copper::CURVE_SLACK_MM;
    let outline = geom::ccw(bridge.outline.flatten(tol))?;
    let mut rings = vec![outline];
    let mut holes = Vec::new();
    for id in &bridge.pads {
        let pad = pads.get(&(bridge.instance, *id))?;
        rings.push(geom::ccw(pad.outline().flatten(tol))?);
        holes.extend(
            physical::pad_holes(pad)
                .into_iter()
                .filter_map(|(h, _)| geom::ccw(h.ring().ring)),
        );
    }
    let bounds = geom::bounds(&rings)?;
    let lands: Vec<_> = rings.iter().skip(1).map(|r| vec![r.clone()]).collect();
    let source = geom::difference_rings(&geom::union(&rings), &holes);
    let mut high = (bounds[1][0] - bounds[0][0]).min(bounds[1][1] - bounds[0][1]);
    let mut low = 0.0;
    for _ in 0..42 {
        let width = (high + low) * 0.5;
        let shrunk = geom::offset(&source, -width * 0.5, tol);
        if shrunk.iter().any(|r| {
            lands.iter().all(|land| {
                !geom::intersect(std::slice::from_ref(r), std::slice::from_ref(land)).is_empty()
            })
        }) {
            low = width;
        } else {
            high = width;
        }
    }
    let foil = project
        .stackup
        .copper
        .get(project.stackup.index(bridge.layer)?)?;
    Some(low * foil.thickness_um / 1000.0)
}

impl CopperGraph {
    pub fn tie_currents(
        &self,
        project: &Project,
        rows: &[TieRow],
    ) -> BTreeMap<(Id, crate::ids::TieId), TieCurrent> {
        let mut result = BTreeMap::new();
        if rows.is_empty() {
            return result;
        }
        let field = crate::library::standards::get()
            .ok()
            .and_then(|s| s.fields.local(&project.library, "Rated current"));
        let pads = std::cell::OnceCell::new();
        for row in rows {
            let Some(instance) = project.instance(row.instance) else {
                continue;
            };
            let (required_a, rated) = instances::ties::currents(project, instance, field);
            let mut current = TieCurrent {
                required_a,
                ..Default::default()
            };
            match row.form {
                TieForm::ZeroOhm if row.state == JumperState::Closed => {
                    if let Some((source, capacity)) = rated {
                        current.capacity_a = Some(capacity);
                        current.source = Some(source);
                    }
                }
                TieForm::Copper
                | TieForm::SolderJumper {
                    bridge: JumperBridge::Trace,
                    ..
                } if row.purpose != TiePurpose::KelvinSense => {
                    if let Some(rule) = &project.rules.current
                        && let Some(table) = rule
                            .table
                            .as_ref()
                            .or_else(|| project.rules.profile.as_ref()?.current_table.as_ref())
                        && let Some(rise) = rule.rise_c
                    {
                        let pads = pads.get_or_init(|| {
                            placed_pads(project)
                                .into_iter()
                                .map(|p| ((p.instance, p.pad.id), p))
                                .collect()
                        });
                        let areas: Option<Vec<_>> = self
                            .ties()
                            .iter()
                            .filter(|t| {
                                t.bridge.instance == row.instance && t.bridge.group == row.group
                            })
                            .map(|t| section(project, &t.bridge, pads))
                            .collect();
                        current.area_mm2 = areas.and_then(|v| v.into_iter().reduce(f64::min));
                        current.capacity_a = current.area_mm2.and_then(|a| table.capacity(rise, a));
                        current.source = Some(format!(
                            "{}, edition {}, figure {}",
                            table.source, table.edition, table.figure
                        ));
                    }
                }
                _ => {}
            }
            current.margin_a = current.capacity_a.zip(required_a).map(|(c, r)| c - r);
            result.insert((row.instance, row.group), current);
        }
        result
    }

    pub(super) fn current_findings(&self, project: &Project) -> Vec<Finding> {
        let rows = instances::ties::rows(project);
        let currents = self.tie_currents(project, &rows);
        let mut out = Vec::new();
        for row in &rows {
            if !row.form.needs_bridge()
                || row.purpose == TiePurpose::KelvinSense
                || row.state != JumperState::Closed
            {
                continue;
            }
            let Some(current) = currents.get(&(row.instance, row.group)) else {
                continue;
            };
            let Some(need) = current.required_a else {
                continue;
            };
            let name = row.label(project);
            let finding = match current.capacity_a {
                None => Finding::new(
                    FindingCode::TieCurrentUnchecked,
                    format!(
                        "{name} requires {need} A. Set the allowed temperature rise and supply an IPC-2152 table citing its edition and figure, with a point covering the placed bridge. Sparksmith ships no curve data."
                    ),
                    row.objects([]),
                ),
                Some(cap) if cap < need => Finding::new(
                    FindingCode::TieCurrentExceeded,
                    format!(
                        "{name} requires {need} A. Its bridge is rated for {cap} A by {}. The long-conductor rating is conservative for a short neck.",
                        current.source.as_deref().unwrap_or("the supplied table")
                    ),
                    row.objects([]),
                )
                .measuring([Measured::at_least(cap, need, Unit::Amperes, 0.0)]),
                _ => continue,
            };
            out.push(finding);
        }
        out
    }
}
