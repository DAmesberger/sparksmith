//! Filled vias against the fab profile, and the two mistakes a fill invites.
//!
//! - [`NOT_MADE`]: the profile's drill rules exclude the fill, or the via's
//!   hole is outside the sizes it fills. Never waivable.
//! - [`NOT_STATED`]: the fabricator offers it without stating what it makes.
//!   A person may accept that, with a reason; there is no number to keep.
//! - [`IN_PAD_OPEN`]: an open hole under a solder paste opening.
//! - [`TOLD`]: the fabricator is told which vias to fill by their hole size,
//!   and open vias share it.
use super::grid::BoundsGrid;
use super::limits::{self, mm};
use super::{CURVE_SLACK_MM, EPS};
use crate::diagnostics::{Finding, FindingCode, ObjectRef, Requirement};
use crate::geometry::{Bounds, Shape};
use crate::library::{PadKind, PasteRule};
use crate::model::{Project, Via, ViaFill};
use crate::rules::RuleKey;
use crate::rules::drills::{Verdict, through_fill};
use std::collections::BTreeMap;

pub const NOT_MADE: &str = FindingCode::BoardDrillNotMade.as_str();
pub const NOT_STATED: &str = FindingCode::BoardDrillNotStated.as_str();
pub const IN_PAD_OPEN: &str = FindingCode::BoardViaInPadOpen.as_str();
pub const TOLD: &str = FindingCode::BoardViaFillTold.as_str();

/// Vias one finding names.
const NAMED: usize = 16;

/// A hole size in whole micrometres, the drill files' step.
fn um(drill: f64) -> i64 {
    // Bounded: a via's drill is at most a few millimetres.
    (drill * 1000.0).round() as i64
}

fn count(vias: &[&Via]) -> String {
    match vias.len() {
        1 => "1 via".into(),
        n => format!("{n} vias"),
    }
}

fn objects(vias: &[&Via]) -> Vec<ObjectRef> {
    vias.iter()
        .take(NAMED)
        .map(|v| ObjectRef::Via { id: v.id })
        .collect()
}

pub fn findings(project: &Project) -> Vec<Finding> {
    let mut out = Vec::new();
    profile(project, &mut out);
    in_pad(project, &mut out);
    told(project, &mut out);
    out
}

/// What the profile says of the fills the board uses. Without a profile
/// nothing is checked, as everywhere.
fn profile(project: &Project, out: &mut Vec<Finding>) {
    let Some(fab) = &project.rules.profile else {
        return;
    };
    for fill in ViaFill::ALL.into_iter().filter(|f| !f.is_none()) {
        let vias: Vec<&Via> = project.vias.iter().filter(|v| v.fill == fill).collect();
        if vias.is_empty() {
            continue;
        }
        let what = fill.label().to_lowercase();
        match through_fill(&fab.drill_rules, fill) {
            Verdict::NotMade(_) => out.push(
                Finding::new(
                    FindingCode::BoardDrillNotMade,
                    format!(
                        "{} does not make {what} vias; the board has {}",
                        fab.name,
                        count(&vias)
                    ),
                    objects(&vias),
                )
                .requiring(Some(Requirement::ViaFill {
                    fill: fill.id().into(),
                })),
            ),
            Verdict::NotStated(_) => out.push(Finding::new(
                FindingCode::BoardDrillNotStated,
                format!(
                    "{} does not state what it makes of {what} vias; the board has {}",
                    fab.name,
                    count(&vias)
                ),
                vec![ObjectRef::Board],
            )),
            Verdict::Made(_) => sizes(project, &vias, out),
        }
    }
}

/// The holes of filled vias against the sizes the fabricator fills.
fn sizes(project: &Project, vias: &[&Via], out: &mut Vec<Finding>) {
    let rules = &project.rules;
    let effective = rules.effective();
    for via in vias {
        let name = project.net_name(via.net);
        let me = vec![ObjectRef::Via { id: via.id }];
        if effective
            .holes
            .capped_drill_min
            .is_some_and(|min| via.drill < min - EPS)
        {
            let need = limits::need(rules, &effective, RuleKey::CappedDrillMin);
            out.push(
                Finding::new(
                    FindingCode::BoardDrillNotMade,
                    format!(
                        "A filled via on {name} has a {} mm hole; {}",
                        limits::mm_against(via.drill, need.mm),
                        need.text()
                    ),
                    me.clone(),
                )
                .requiring(need.at_most(via.drill)),
            );
        }
        if let Some(max) = effective
            .holes
            .capped_drill_max
            .filter(|max| via.drill > max + EPS)
        {
            out.push(
                Finding::new(
                    FindingCode::BoardDrillNotMade,
                    format!(
                        "A filled via on {name} has a {} mm hole; needs at most {} mm ({})",
                        limits::mm_against(via.drill, max),
                        mm(max),
                        limits::cite(rules, RuleKey::CappedDrillMax)
                    ),
                    me,
                )
                .requiring(limits::at_least(
                    rules,
                    RuleKey::CappedDrillMax,
                    via.drill,
                )),
            );
        }
    }
}

/// Open holes of board vias under a pad's paste, on either face: a through
/// via has both.
fn in_pad(project: &Project, out: &mut Vec<Finding>) {
    let open: Vec<&Via> = project.vias.iter().filter(|v| v.fill.is_none()).collect();
    if open.is_empty() {
        return;
    }
    let reduction = project.rules.effective().paste.reduction;
    let reach = |via: &Via| via.drill / 2.0;
    let grid = BoundsGrid::new(
        open.iter().map(|via| {
            let (r, [x, y]) = (reach(via), via.position);
            Some(Bounds {
                min: [x - r, y - r],
                max: [x + r, y + r],
            })
        }),
        0.0,
    );
    let mut labels = None;
    let mut near = Vec::new();
    for pad in super::placed_pads(project) {
        // Paste grows past the land only by a negative reduction or an
        // explicit window.
        let grown = match &pad.pad.paste {
            PasteRule::Inherit => -reduction,
            PasteRule::None => continue,
            PasteRule::Reduction(r) => -r.to_mm_f64(),
            PasteRule::Windows(_) => 0.0,
        };
        let mut extent = pad.outline().bounds(0.0);
        if let PasteRule::Windows(windows) = &pad.pad.paste {
            let t = pad.pad_transform();
            for w in windows {
                extent = extent.union(w.shape.to_mm().transformed(&t).bounds(0.0));
            }
        }
        grid.near(&extent, grown.max(0.0), &mut near);
        if near.is_empty()
            || !crate::instances::ties::prints_paste(project, pad.instance, pad.terminal)
        {
            continue;
        }
        let t = pad.pad_transform();
        let shrunk = |by: f64| -> Vec<Shape> {
            pad.pad
                .shape
                .offset_pieces(-by, CURVE_SLACK_MM)
                .into_iter()
                .map(|p| p.outer.transformed(&t))
                .collect()
        };
        let openings = match &pad.pad.paste {
            PasteRule::Inherit => match pad.pad.kind {
                PadKind::Smd | PadKind::Mechanical if pad.pad.hole.is_none() => shrunk(reduction),
                _ => Vec::new(),
            },
            PasteRule::None => Vec::new(),
            PasteRule::Reduction(r) => shrunk(r.to_mm_f64()),
            PasteRule::Windows(windows) => windows
                .iter()
                .map(|w| w.shape.to_mm().transformed(&t))
                .collect(),
        };
        for via in near.iter().filter_map(|k| open.get(*k)) {
            if !openings
                .iter()
                .any(|s| s.distance(via.position, true) < reach(via) - EPS)
            {
                continue;
            }
            let labels = labels.get_or_insert_with(|| super::pad_labels(project));
            let label = labels
                .get(&(pad.instance, pad.pad.id))
                .cloned()
                .unwrap_or_else(|| pad.pad.id.to_string());
            out.push(Finding::new(
                FindingCode::BoardViaInPadOpen,
                format!(
                    "Via on {} in {} {label} is open under solder paste",
                    project.net_name(via.net),
                    super::outline::part_name(project, pad.instance)
                ),
                vec![
                    ObjectRef::Via { id: via.id },
                    ObjectRef::Pad {
                        instance: pad.instance,
                        pad: pad.pad.id,
                    },
                ],
            ));
        }
    }
}

/// Filled and open vias of one hole size, where the fabricator is told by
/// it: one finding a size.
fn told(project: &Project, out: &mut Vec<Finding>) {
    let Some(fab) = &project.rules.profile else {
        return;
    };
    let by_size = |fill| match through_fill(&fab.drill_rules, fill) {
        Verdict::Made(Some(rule)) | Verdict::NotStated(rule) => {
            rule.order.as_ref().is_some_and(|o| o.told_by_size)
        }
        Verdict::Made(None) | Verdict::NotMade(_) => false,
    };
    let mut sizes: BTreeMap<i64, (Vec<&Via>, usize)> = BTreeMap::new();
    for via in &project.vias {
        let (filled, open) = sizes.entry(um(via.drill)).or_default();
        if via.fill.is_none() {
            *open += 1;
        } else {
            filled.push(via);
        }
    }
    for (filled, open) in sizes.into_values() {
        let Some(first) = filled.first() else {
            continue;
        };
        if open == 0 || !filled.iter().any(|v| by_size(v.fill)) {
            continue;
        }
        out.push(Finding::new(
            FindingCode::BoardViaFillTold,
            format!(
                "Filled and open vias share {} mm; {} is told by hole size",
                mm(first.drill),
                fab.name
            ),
            objects(&filled),
        ));
    }
}
