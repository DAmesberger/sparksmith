//! The solder mask web between the openings of the assembled board: what
//! the mask film (`fabrication::artwork`) opens, measured between pads of
//! different placed parts, and within one part where the board's mask
//! expansion rule (not the footprint's own defaults) narrows a web. The
//! footprint lint (`footprint.mask_web`) measures one definition under its
//! own defaults; it cannot see a neighbour or the board's rule.
//!
//! [`MASK_WEB`]: two openings on one side whose web, the copper gap less
//! both openings' expansions (the footprint lint's measure), is under the
//! effective `min mask web`. Openings of copper that touches are one
//! opening (no web lies between them). A tented pad opens nothing.
use super::limits::{BoardRules, mm_against};
use super::{CopperRing, PlacedPad, placed_pads};
use crate::diagnostics::{Finding, Measured, ObjectRef, Unit};
use crate::ids::Id;
use crate::instances;
use crate::library::{PadCopper, PadDefaults};
use crate::model::{Layer, Project};
use crate::rules::RuleKey;
use std::collections::BTreeMap;

/// Two pads' solder mask openings on the assembled board leave a web
/// narrower than the fab profile's `min mask web`.
pub const MASK_WEB: &str = crate::diagnostics::FindingCode::BoardMaskWeb.as_str();

/// A web of exactly the minimum passes: sums of decimal sizes and
/// positions land a few ulps either side of it.
const WEB_TOLERANCE_MM: f64 = 1e-6;

/// Whether a pad opens the mask on `side`: through-hole pads on both, a
/// surface pad on its own.
fn opens(pad: &PlacedPad, side: Layer) -> bool {
    match pad.pad.kind.copper() {
        PadCopper::Through => true,
        PadCopper::OneSide => pad.layer == side,
        PadCopper::None => pad.pad.kind.through_hole(),
    }
}

/// Every [`MASK_WEB`] finding of `project`.
pub fn findings(project: &Project) -> Vec<Finding> {
    let limits = BoardRules::of(project);
    let need = limits.need(RuleKey::MaskWebMin);
    let min = need.mm;
    if min <= 0.0 {
        return Vec::new();
    }
    let board = project.rules.effective().mask.expansion;
    let pads = placed_pads(project);
    // Each placed part's footprint defaults: the footprint lint's view of
    // its own webs.
    let defaults: BTreeMap<Id, PadDefaults> = project
        .instances
        .iter()
        .filter_map(|i| {
            let physical = instances::resolve(&project.library, i).ok()?.physical?;
            Some((i.id, physical.footprint.definition.defaults))
        })
        .collect();
    let openings: Vec<(usize, f64, CopperRing)> = pads
        .iter()
        .enumerate()
        .filter_map(|(k, p)| {
            let grow = p.mask_expansion(board)?;
            let ring = CopperRing::of_placed_pad(p);
            ring.bounds?;
            Some((k, grow, ring))
        })
        .collect();
    let reach = openings.iter().map(|o| o.1).fold(0.0, f64::max) * 2.0 + min;
    let grid = super::grid::BoundsGrid::new(openings.iter().map(|o| o.2.bounds), reach);
    let labels = super::pad_labels(project);
    let name = |p: &PlacedPad| {
        let part = super::outline::part_name(project, p.instance);
        let number = labels
            .get(&(p.instance, p.pad.id))
            .cloned()
            .unwrap_or_else(|| p.pad.id.to_string());
        format!("{part} pad {number}")
    };
    let mut out = Vec::new();
    let mut near = Vec::new();
    for (i, (ka, ga, ra)) in openings.iter().enumerate() {
        let (Some(a), Some(bounds)) = (pads.get(*ka), ra.bounds) else {
            continue;
        };
        grid.near(&bounds, reach, &mut near);
        for (kb, gb, rb) in near
            .iter()
            .filter(|j| **j > i)
            .filter_map(|j| openings.get(*j))
        {
            let Some(b) = pads.get(*kb) else {
                continue;
            };
            let Some(side) = [Layer::Top, Layer::Bottom]
                .into_iter()
                .find(|s| opens(a, *s) && opens(b, *s))
            else {
                continue;
            };
            let gap = ra.gap(rb, ga + gb + min + 1.0);
            // Copper that touches is one land: its openings merge.
            if gap <= super::EPS {
                continue;
            }
            let web = gap - ga - gb;
            if web >= min - WEB_TOLERANCE_MM {
                continue;
            }
            // Within one part, the footprint lint reports a web its own
            // defaults narrow; the board reports only what its rule did.
            if a.instance == b.instance
                && let Some(d) = defaults.get(&a.instance)
            {
                let own = |p: &PlacedPad| p.pad.mask_expansion(d);
                if let (Some(fa), Some(fb)) = (own(a), own(b))
                    && gap - fa - fb < min - WEB_TOLERANCE_MM
                {
                    continue;
                }
            }
            out.push(
                Finding::new(
                    crate::diagnostics::FindingCode::BoardMaskWeb,
                    format!(
                        "The solder-mask web between {} and {} on the {} side is {} mm; {}",
                        name(a),
                        name(b),
                        if side == Layer::Top { "top" } else { "bottom" },
                        mm_against(web.max(0.0), min),
                        need.text()
                    ),
                    vec![
                        ObjectRef::Pad {
                            instance: a.instance,
                            pad: a.pad.id,
                        },
                        ObjectRef::Pad {
                            instance: b.instance,
                            pad: b.pad.id,
                        },
                    ],
                )
                .requiring(need.at_most(web))
                .measuring([Measured::at_least(
                    web,
                    min,
                    Unit::Mm,
                    WEB_TOLERANCE_MM,
                )]),
            );
        }
    }
    out
}

#[cfg(test)]
mod tests;
