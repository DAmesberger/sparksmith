//! Human-readable row titles for concrete objects. Classification lives on FindingCode.
use super::names::Names;
use super::{MAX_TITLE, Severity};
use crate::diagnostics::{Finding, FindingCode, ObjectRef};
use std::collections::BTreeSet;

pub(super) const NO_PART: &str = FindingCode::PurchasingNoPart.as_str();
#[cfg(test)]
pub(super) const UNMAPPED_PACKAGE_TERMINAL: &str = FindingCode::BindingMappingMissing.as_str();

pub(super) fn severity(project: &crate::model::Project, finding: &Finding) -> Severity {
    if finding.code == FindingCode::BindingMappingMissing {
        return Severity::ToDo;
    }
    if finding.blocks.is_some() {
        return Severity::Error;
    }
    // A loop through a star-ground tie defeats the star: an error, where
    // another loop through ties is a warning.
    if finding.code == FindingCode::TieLoop
        && finding
            .objects
            .iter()
            .any(|object| star_terminal(project, *object))
    {
        return Severity::Error;
    }
    finding.code.severity()
}

fn star_terminal(project: &crate::model::Project, object: ObjectRef) -> bool {
    let ObjectRef::Terminal { endpoint } = object else {
        return false;
    };
    let Some(instance) = project.instance(endpoint.instance) else {
        return false;
    };
    let Some(component) = project.library.component(instance.component) else {
        return false;
    };
    component.definition.ties.iter().any(|spec| {
        spec.ends.contains(&endpoint.terminal)
            && instance
                .ties
                .get(&spec.id)
                .is_some_and(|usage| usage.purpose == crate::library::tie::TiePurpose::StarGround)
    })
}

/// Includes production-only group names which are not document findings.
pub fn group_title(name: &str) -> String {
    if let Ok(code) = name.parse::<FindingCode>() {
        return code.group_title().into();
    }
    let rule = name.rsplit('.').next().unwrap_or(name).replace('_', " ");
    crate::library::names::capitalised(rule.trim())
}

/// `text` as one line of at most [`MAX_TITLE`] characters with no final
/// full stop: the first clause, clipped at a word with "…".
pub(super) fn line(text: &str) -> String {
    let first = text.lines().next().unwrap_or_default().trim();
    let clause = first.split("; ").next().unwrap_or(first).trim();
    let clause = clause.trim_end_matches('.').trim_end();
    clip(clause)
}

/// The first line of `text` without a final full stop, unclipped: the
/// cause shown under a row.
pub(super) fn line_full(text: &str) -> String {
    let first = text.lines().next().unwrap_or_default().trim();
    first.trim_end_matches('.').trim_end().to_owned()
}

/// At most [`MAX_TITLE`] characters, cut at a word boundary with "…".
pub(super) fn clip(text: &str) -> String {
    if text.chars().count() <= MAX_TITLE {
        return text.to_owned();
    }
    let keep: String = text.chars().take(MAX_TITLE - 1).collect();
    let cut = keep
        .rfind(' ')
        .filter(|at| *at > MAX_TITLE / 2)
        .unwrap_or(keep.len());
    let head = keep[..cut].trim_end_matches([' ', ',', ';', ':', '.']);
    format!("{head}…")
}

/// Distinct names of the referenced objects of `kind`, in first-seen order.
fn names_of(names: &Names<'_>, findings: &[&Finding], pick: fn(&ObjectRef) -> bool) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for object in findings.iter().flat_map(|f| f.objects.iter()) {
        if pick(object) {
            let name = names.object(object);
            if seen.insert(name.clone()) {
                out.push(name);
            }
        }
    }
    out
}

/// The parties of a clearance or courtyard finding: the instances and
/// tracks it names, by name.
fn parties(names: &Names<'_>, findings: &[&Finding]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for object in findings.iter().flat_map(|f| f.objects.iter()) {
        let name = match *object {
            ObjectRef::Pad { instance, .. } | ObjectRef::Instance { id: instance } => {
                names.designator(instance)
            }
            ObjectRef::Track { .. } => names.object(object),
            _ => continue,
        };
        if seen.insert(name.clone()) {
            out.push(name);
        }
    }
    out
}

fn is_pad(object: &ObjectRef) -> bool {
    matches!(object, ObjectRef::Pad { .. })
}

/// "R1, R2 and R3", shortened to "R1, R2 and 3 more" past four names.
fn list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [init @ .., last] if items.len() <= 4 => format!("{} and {last}", init.join(", ")),
        _ => format!(
            "{} and {} more",
            items.get(..2).unwrap_or_default().join(", "),
            items.len() - 2
        ),
    }
}

/// The one-line title of a row: its primary finding and every finding
/// merged into it. Titles name objects by their user names.
pub(super) fn title(names: &Names<'_>, findings: &[&Finding]) -> String {
    let Some(first) = findings.first() else {
        return String::new();
    };
    let message = first.message.as_str();
    let title = match first.code.as_str() {
        // A mounting hole's washer is named by the board object, which says
        // nothing: the message names the hole ("the plated mounting hole
        // at (40, 20) mm / a track on GND").
        "board.clearance" if findings.len() == 1 && first.objects.contains(&ObjectRef::Board) => {
            let pair = message
                .split_once(": ")
                .map_or(message, |(_, rest)| rest)
                .rsplit_once(" (")
                .map_or(message, |(pair, _)| pair);
            format!("{} too close", pair.replace(" / ", " ↔ "))
        }
        "board.overlap" => {
            let objects = names_of(names, findings, |_| true);
            format!("{} touch", objects.join(" ↔ "))
        }
        "board.clearance" => {
            let pads = names_of(names, findings, is_pad);
            match parties(names, findings).as_slice() {
                [a, b] if findings.len() > 1 => {
                    format!("{a} ↔ {b}: {} pads too close", pads.len())
                }
                _ => {
                    let objects = names_of(names, findings, |_| true);
                    format!("{} too close", objects.join(" ↔ "))
                }
            }
        }
        "board.courtyard_overlap" => {
            // The top side is the usual case; only the bottom is named.
            let side = if message.contains("bottom") {
                " (bottom)"
            } else {
                ""
            };
            format!(
                "{} courtyards overlap{side}",
                parties(names, findings).join(" ↔ ")
            )
        }
        "board.outside_outline" => {
            let what = if message.contains("extends into a slot") {
                "extends into a slot"
            } else if message.contains("extends into a cutout") {
                "extends into a cutout"
            } else {
                "extends beyond the board outline"
            };
            format!("{} {what}", list(&names_of(names, findings, |_| true)))
        }
        "board.unrouted" => {
            let nets = names_of(names, findings, |o| matches!(o, ObjectRef::Net { .. }));
            let pads = names_of(names, findings, is_pad);
            match pads.as_slice() {
                [pad] => format!("{}: {pad} is unrouted", list(&nets)),
                _ => format!("{}: {} pads unrouted", list(&nets), pads.len()),
            }
        }
        "board.unplaced" => format!("{} not placed", list(&names_of(names, findings, |_| true))),
        "board.body_off_pads" => format!(
            "{}: body leads off their pads",
            list(&names_of(names, findings, |_| true))
        ),
        "variant.substitute_unfit" => format!(
            "{}: variant part no longer fits its footprint",
            list(&names_of(names, findings, |o| {
                matches!(o, ObjectRef::Instance { .. })
            }))
        ),
        // Short enough for the Issues row: the hole and what it is near.
        "board.hole_clearance" => {
            let near = names_of(names, findings, |o| {
                !matches!(o, ObjectRef::Board | ObjectRef::BoardFeature { .. })
            });
            let what = if message.contains(" overlaps ") {
                "overlaps"
            } else {
                "too close to"
            };
            match near.as_slice() {
                [] => line(message),
                _ => format!("Hole {what} {}", list(&near)),
            }
        }
        "instance.unit_unplaced" => format!(
            "{} is neither placed nor marked unused",
            names_of(names, findings, |_| true).join(", ")
        ),
        "import.board_joins_nets" => format!(
            "{} joined on the imported board",
            list(&names_of(names, findings, |o| {
                matches!(o, ObjectRef::Net { .. } | ObjectRef::Terminal { .. })
            }))
        ),
        "import.board_splits_net" => format!(
            "{} split on the imported board",
            list(&names_of(names, findings, |o| matches!(
                o,
                ObjectRef::Net { .. }
            )))
        ),
        "instance.unmapped_pad" => format!(
            "{}: pin added by import",
            list(&names_of(names, findings, |_| true))
        ),
        "instance.terminal_unaccounted" => format!(
            "{} unconnected",
            names_of(names, findings, |_| true).join(", ")
        ),
        "config.function_unresolved" => format!(
            "{}: no pin function",
            names_of(names, findings, |_| true).join(", ")
        ),
        "instance.no_physical_choice" => format!(
            "{}: no footprint",
            names_of(names, findings, |_| true).join(", ")
        ),
        NO_PART => format!(
            "{}: no Part",
            names_of(names, findings, |_| true).join(", ")
        ),
        "component.mapping_missing" if findings.len() > 1 => {
            let package = names_of(names, findings, |o| {
                matches!(o, ObjectRef::LibraryObject { .. })
            })
            .into_iter()
            .find(|n| n.starts_with("package terminal"));
            match package {
                Some(p) => format!(
                    "{} has no electrical terminal",
                    crate::library::names::capitalised(&p)
                ),
                None => line(message),
            }
        }
        code if code.starts_with("part.") && findings.len() > 1 => {
            let instances = names_of(names, findings, |o| matches!(o, ObjectRef::Instance { .. }));
            let rest = message.split_once(": ").map_or(message, |(_, rest)| rest);
            format!("{}: {}", list(&instances), line(rest))
        }
        _ => line(message),
    };
    clip(title.trim())
}
