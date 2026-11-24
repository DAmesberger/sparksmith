//! The vias and copper layers a net class allows: routing refuses what it
//! does not, and [`findings`] tells what got in by import or a class edit.
use crate::diagnostics::{Error, Finding, FindingCode, Measured, ObjectRef, Result, Unit};
use crate::ids::Id;
use crate::model::{CopperLayer, Project};
use crate::rules::{ClassLayer, ClassSpan, NetClass};
use std::collections::BTreeMap;

pub const CLASS: &str = FindingCode::BoardViaClass.as_str();

/// Tracks one finding names.
const NAMED: usize = 16;

fn class(project: &Project, net: Id) -> Option<&NetClass> {
    super::net_class(project, net).and_then(|c| project.rules.class(c))
}

fn vias(n: u32) -> String {
    match n {
        0 => "no vias".into(),
        1 => "1 via".into(),
        n => format!("{n} vias"),
    }
}

/// 1st, 2nd, 3rd, 4th, 11th, 21st.
fn ordinal(n: usize) -> String {
    let end = match (n % 10, n % 100) {
        (1, t) if t != 11 => "st",
        (2, t) if t != 12 => "nd",
        (3, t) if t != 13 => "rd",
        _ => "th",
    };
    format!("{n}{end}")
}

/// Whether `net`'s class allows through vias at all, and how many.
fn allowed(class: &NetClass) -> Option<u32> {
    let rule = class.vias.as_ref()?;
    let through = rule
        .spans
        .as_ref()
        .is_none_or(|s| s.contains(&ClassSpan::Through));
    if through { rule.max } else { Some(0) }
}

/// Refuses vias on `net` past what its class allows, where the board had
/// `before` of them before the edit: an edit that leaves no more than there
/// were is never refused for them.
pub(super) fn check_vias(project: &Project, net: Id, before: usize) -> Result<()> {
    let now = project.vias.iter().filter(|v| v.net == net).count();
    if now <= before {
        return Ok(());
    }
    let Some((class, max)) = class(project, net).and_then(|c| Some((c, allowed(c)?))) else {
        return Ok(());
    };
    if now <= max as usize {
        return Ok(());
    }
    Err(Error::Invalid(format!(
        "{}: net class {} allows {}",
        project.net_name(net),
        class.name,
        vias(max)
    )))
}

/// Refuses copper of `net` on a layer its class does not allow.
pub(super) fn check_layer(project: &Project, net: Id, layer: CopperLayer) -> Result<()> {
    let Some(class) = class(project, net) else {
        return Ok(());
    };
    match &class.layers {
        Some(layers) if !layers.contains(&ClassLayer::from(layer)) => Err(Error::Invalid(format!(
            "{}: net class {} does not allow {}",
            project.net_name(net),
            class.name,
            layer.name()
        ))),
        _ => Ok(()),
    }
}

/// Whether routing `net` may change to `layer`, through a new via when
/// `via`: what a Route tool asks before it offers the change.
pub fn check_change(project: &Project, net: Id, layer: CopperLayer, via: bool) -> Result<()> {
    check_layer(project, net, layer)?;
    if !via {
        return Ok(());
    }
    let Some((class, max)) = class(project, net).and_then(|c| Some((c, allowed(c)?))) else {
        return Ok(());
    };
    let now = project.vias.iter().filter(|v| v.net == net).count();
    if now < max as usize {
        return Ok(());
    }
    Err(Error::Invalid(format!(
        "{}: net class {} allows {}",
        project.net_name(net),
        class.name,
        vias(max)
    )))
}

pub fn findings(project: &Project) -> Vec<Finding> {
    let rules = &project.rules;
    if rules
        .net_classes
        .iter()
        .all(|c| c.vias.is_none() && c.layers.is_none())
    {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut seen: BTreeMap<Id, usize> = BTreeMap::new();
    let mut classes: BTreeMap<Id, Option<&NetClass>> = BTreeMap::new();
    for via in &project.vias {
        let found = *classes
            .entry(via.net)
            .or_insert_with(|| class(project, via.net));
        let Some((class, max)) = found.and_then(|c| Some((c, allowed(c)?))) else {
            continue;
        };
        let n = seen.entry(via.net).or_default();
        *n += 1;
        if *n <= max as usize {
            continue;
        }
        let which = if max == 0 {
            String::new()
        } else {
            format!(", this is the {}", ordinal(*n))
        };
        out.push(
            Finding::new(
                FindingCode::BoardViaClass,
                format!(
                    "Via on {}: net class {} allows {}{which}",
                    project.net_name(via.net),
                    class.name,
                    vias(max)
                ),
                vec![ObjectRef::Via { id: via.id }],
            )
            .measuring([Measured::at_most(
                *n as f64,
                f64::from(max),
                Unit::Count,
                1e-9,
            )]),
        );
    }
    // Tracks on a layer, by net.
    let mut tracks: BTreeMap<(Id, CopperLayer), Vec<Id>> = BTreeMap::new();
    for track in &project.tracks {
        let found = *classes
            .entry(track.net)
            .or_insert_with(|| class(project, track.net));
        let layer = ClassLayer::from(track.layer);
        if found.is_some_and(|c| c.layers.as_ref().is_some_and(|l| !l.contains(&layer))) {
            tracks
                .entry((track.net, track.layer))
                .or_default()
                .push(track.id);
        }
    }
    for ((net, layer), ids) in tracks {
        let Some(Some(class)) = classes.get(&net) else {
            continue;
        };
        out.push(
            Finding::new(
                FindingCode::BoardViaClass,
                format!(
                    "Track on {}: net class {} does not allow {}",
                    project.net_name(net),
                    class.name,
                    layer.name()
                ),
                ids.iter()
                    .take(NAMED)
                    .map(|id| ObjectRef::Track { id: *id })
                    .collect(),
            )
            // Accepted for as many tracks as there are.
            .measuring([Measured::at_most(ids.len() as f64, 0.0, Unit::Count, 1e-9)]),
        );
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn an_ordinal_reads_as_spoken() {
        for (n, words) in [
            (1, "1st"),
            (2, "2nd"),
            (3, "3rd"),
            (4, "4th"),
            (11, "11th"),
            (22, "22nd"),
        ] {
            assert_eq!(super::ordinal(n), words);
        }
    }
}
