//! Every drilled and routed hole of the board, and the tool table: through
//! vias, pad holes and slots, pad-owned vias, board mounting holes and
//! plated board slots. Non-plated board cutouts and slots are milled along
//! the profile layer instead (`artwork`).
use crate::ids::Id;
use crate::layout;
pub use crate::layout::physical::{HoleFeature, HoleFunction, stadium};
use crate::model::Project;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Diameters within this many mm share one tool.
pub const TOOL_RESOLUTION_MM: f64 = 0.001;

/// One hole and what it belongs to.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct DrillHole {
    pub feature: HoleFeature,
    pub plated: bool,
    pub function: HoleFunction,
    /// The net of a plated hole, when it has one.
    #[serde(default)]
    pub net: Option<Id>,
    /// The placed instance and pad label of a pad hole.
    #[serde(default)]
    pub component: Option<Id>,
    #[serde(default)]
    pub pad: Option<String>,
}

/// One tool: holes of one diameter, plating, kind (hit or slot) and
/// function. `number` counts from 1 across the board, plated tools first,
/// so one number names one tool in the drill table, the drill map and both
/// Excellon files.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct DrillTool {
    pub number: u32,
    pub diameter: f64,
    pub plated: bool,
    pub slot: bool,
    pub function: HoleFunction,
    pub count: usize,
}

/// The tools of a board, plated first, each group by diameter.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct DrillTable {
    pub tools: Vec<DrillTool>,
}

impl DrillTable {
    /// The tool a hole is drilled with.
    pub fn tool_of(&self, hole: &DrillHole) -> Option<&DrillTool> {
        let key = tool_key(hole);
        self.tools
            .iter()
            .find(|t| (t.plated, nm(t.diameter), t.slot, t.function) == key)
    }
    pub fn plated(&self) -> impl Iterator<Item = &DrillTool> {
        self.tools.iter().filter(|t| t.plated)
    }
    pub fn non_plated(&self) -> impl Iterator<Item = &DrillTool> {
        self.tools.iter().filter(|t| !t.plated)
    }
}

/// A diameter in whole tool-resolution steps.
fn nm(diameter: f64) -> i64 {
    // Bounded: diameters are validated to well under 1 m.
    (diameter / TOOL_RESOLUTION_MM).round() as i64
}

fn tool_key(hole: &DrillHole) -> (bool, i64, bool, HoleFunction) {
    (
        hole.plated,
        nm(hole.feature.diameter()),
        hole.feature.is_slot(),
        hole.function,
    )
}

/// Every hole of `project` in a fixed order: vias, then pad holes in
/// placement and pad order, then board mounting holes and plated slots:
/// the board's physical holes ([`layout::physical::holes`]), which the
/// checks measure.
pub fn holes(project: &Project) -> Vec<DrillHole> {
    let labels = layout::pad_labels(project);
    layout::physical::holes(project)
        .into_iter()
        .map(|hole| {
            let (component, pad) = match hole.owner {
                layout::physical::HoleOwner::Pad { instance, pad, .. } => {
                    (Some(instance), labels.get(&(instance, pad)).cloned())
                }
                _ => (None, None),
            };
            DrillHole {
                feature: hole.feature,
                plated: hole.plated,
                function: hole.function,
                net: hole.net,
                component,
                pad,
            }
        })
        .collect()
}

/// The tool table of `holes`: plated tools first, then non-plated, each by
/// diameter, then hits before slots, then function; numbered from 1
/// across both, so the non-plated file continues where the plated one
/// stops (T1–T3 plated, T4 non-plated).
pub fn table(holes: &[DrillHole]) -> DrillTable {
    let mut counts: std::collections::BTreeMap<(bool, i64, bool, HoleFunction), (f64, usize)> =
        Default::default();
    for hole in holes {
        let entry = counts
            .entry(tool_key(hole))
            .or_insert((hole.feature.diameter(), 0));
        entry.1 += 1;
    }
    let mut keys: Vec<_> = counts.into_iter().collect();
    // Plated (true) first.
    keys.sort_by_key(|((plated, d, slot, f), _)| (!*plated, *d, *slot, *f));
    let mut tools = Vec::with_capacity(keys.len());
    let mut n = 0u32;
    for ((plated, d, slot, function), (_, count)) in keys {
        n = n.saturating_add(1);
        tools.push(DrillTool {
            number: n,
            diameter: d as f64 * TOOL_RESOLUTION_MM,
            plated,
            slot,
            function,
            count,
        });
    }
    DrillTable { tools }
}
