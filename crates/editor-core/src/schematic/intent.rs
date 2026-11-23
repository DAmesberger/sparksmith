//! Compare exact terminal memberships against reconciled schematic nets.
//! Candidate corrections use the regular command engine on a private fork.
use super::SchematicCommand;
use crate::diagnostics::{Error, Result};
use crate::ids::{Endpoint, Id};
use crate::model::{Net, Project, WireEnd};
use crate::nets;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Names are compared case-insensitively. Each list is the full intended
/// membership of that net. Terminals use the identity returned by schematic.pins.
pub type IntentNets = BTreeMap<String, Vec<Endpoint>>;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Difference {
    pub net: String,
    pub terminal: Endpoint,
    pub pin: String,
    pub actual_net: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub missing: Vec<Difference>,
    pub extra: Vec<Difference>,
    pub wrong: Vec<Difference>,
    /// Directly usable as apply.operations at this query's revision.
    pub operations: Vec<Correction>,
    /// A conflict that prevents a complete, verified correction.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked: Option<String>,
}

/// Only the typed operations which an intent check can propose.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "op", content = "params", deny_unknown_fields)]
pub enum Correction {
    #[serde(rename = "schematic.connect")]
    Connect { from: Endpoint, to: Endpoint },
    #[serde(rename = "schematic.disconnect")]
    Disconnect { wire: Id },
    #[serde(rename = "schematic.net_rename")]
    Rename { net: Id, name: String },
}

impl Correction {
    pub(crate) fn command(&self) -> SchematicCommand {
        match self {
            Self::Connect { from, to } => SchematicCommand::Connect {
                from: (*from).into(),
                to: (*to).into(),
            },
            Self::Disconnect { wire } => SchematicCommand::Disconnect { wire: *wire },
            Self::Rename { net, name } => SchematicCommand::RenameNet {
                net: *net,
                name: name.clone(),
            },
        }
    }
}

/// The query owns a disposable engine fork. Every proposed step runs there
/// through the same typed edit as a later apply.
pub(crate) trait Trial {
    fn snapshot(&self) -> &Project;
    fn try_edit(&mut self, command: SchematicCommand) -> Result<()>;
}

struct Intent<'a> {
    nets: &'a IntentNets,
    names: BTreeMap<String, &'a str>,
    owners: BTreeMap<Endpoint, &'a str>,
}

fn pin_name(project: &Project, ep: Endpoint) -> String {
    let Some(instance) = project.instance(ep.instance) else {
        return format!("instance {} terminal {}", ep.instance, ep.terminal);
    };
    let pin = project
        .library
        .component(instance.component)
        .and_then(|c| c.definition.terminal(ep.terminal));
    match pin {
        Some(pin) => format!(
            "{} pin {} (terminal {})",
            instance.designator, pin.name, ep.terminal
        ),
        None => format!("{} terminal {}", instance.designator, ep.terminal),
    }
}

fn current(project: &Project, ep: Endpoint) -> Result<&Net> {
    project.net_of(ep).ok_or_else(|| {
        Error::NotFound(format!(
            "No schematic net contains {}.",
            pin_name(project, ep)
        ))
    })
}

impl<'a> Intent<'a> {
    fn new(project: &Project, nets: &'a IntentNets, max_terminals: usize) -> Result<Self> {
        if nets.is_empty() || nets.len() > max_terminals {
            return Err(Error::Limit(format!("Name 1 to {max_terminals} nets.")));
        }
        let mut names = BTreeMap::new();
        let mut owners = BTreeMap::new();
        for (name, terminals) in nets {
            if !nets::name_valid(name)
                || names
                    .insert(name.to_ascii_lowercase(), name.as_str())
                    .is_some()
            {
                return Err(Error::Invalid(format!(
                    "Invalid or repeated net name {name:?}."
                )));
            }
            if terminals.is_empty() || terminals.len() > max_terminals.saturating_sub(owners.len())
            {
                return Err(Error::Limit(format!(
                    "Net {name} needs at least one terminal and an intent can name at most {max_terminals} terminals."
                )));
            }
            for &ep in terminals {
                let instance = project
                    .instance(ep.instance)
                    .ok_or_else(|| Error::NotFound(pin_name(project, ep)))?;
                let pin = project
                    .library
                    .component(instance.component)
                    .and_then(|c| c.definition.terminal(ep.terminal))
                    .ok_or_else(|| {
                        Error::NotFound(format!("Unknown {}.", pin_name(project, ep)))
                    })?;
                if !pin.is_electrical() || instance.no_connect.contains(&ep.terminal) {
                    return Err(Error::Invalid(format!(
                        "{} is marked no-connect or reserved.",
                        pin_name(project, ep)
                    )));
                }
                current(project, ep)?;
                if let Some(previous) = owners.insert(ep, name.as_str()) {
                    return Err(Error::Invalid(format!(
                        "{} appears twice in the net intent ({previous}, {name}).",
                        pin_name(project, ep)
                    )));
                }
            }
        }
        Ok(Self {
            nets,
            names,
            owners,
        })
    }

    fn declared(&self, net: &Net) -> Option<&str> {
        self.names.get(&net.name.to_ascii_lowercase()).copied()
    }

    fn differences(
        &self,
        project: &Project,
        max_differences: usize,
    ) -> Result<(Report, BTreeSet<Endpoint>)> {
        let mut report = Report::default();
        let mut detach = BTreeSet::new();
        for net in project.nets.iter() {
            let named = self.declared(net);
            let first = net
                .terminals
                .iter()
                .find_map(|ep| self.owners.get(ep).copied());
            if named.is_none() && first.is_none() {
                continue;
            }
            let auto = nets::auto_named(net);
            // A named net, or an existing group of unmentioned terminals,
            // retains its own identity. For a mixed anonymous group keep
            // its first intended group, in stable terminal order.
            let keep = if auto
                && named.is_none()
                && net.terminals.iter().all(|ep| self.owners.contains_key(ep))
            {
                first
            } else {
                named
            };
            for &ep in &net.terminals {
                let owner = self.owners.get(&ep).copied();
                let difference = |target: &str| Difference {
                    net: target.to_owned(),
                    terminal: ep,
                    pin: pin_name(project, ep),
                    actual_net: net.name.clone(),
                };
                if let Some(target) = named
                    && owner != named
                {
                    report.extra.push(difference(target));
                }
                if let Some(target) = owner
                    && named != Some(target)
                {
                    if auto && (net.terminals.len() == 1 || keep == Some(target)) {
                        report.missing.push(difference(target));
                    } else {
                        report.wrong.push(difference(target));
                    }
                }
                if owner != keep && (net.terminals.len() > 1 || !auto) {
                    detach.insert(ep);
                }
                if report.missing.len() + report.wrong.len() + report.extra.len() > max_differences
                {
                    return Err(Error::Limit(format!(
                        "The intent has more than {max_differences} differences at {} on {}.",
                        pin_name(project, ep),
                        net.name
                    )));
                }
            }
        }
        Ok((report, detach))
    }

    /// Memberships among terminals outside the intent, plus every other
    /// deliberately assigned name. IDs may change on normal reconciliation.
    fn outside(&self, project: &Project) -> BTreeMap<Vec<Endpoint>, Option<String>> {
        let mut groups = BTreeMap::new();
        for net in project.nets.iter() {
            let others: Vec<_> = net
                .terminals
                .iter()
                .filter(|ep| !self.owners.contains_key(ep))
                .copied()
                .collect();
            if !others.is_empty() {
                let name = (!nets::auto_named(net) && self.declared(net).is_none())
                    .then(|| net.name.clone());
                groups.insert(others, name);
            }
        }
        groups
    }
}

fn rehearse(
    intent: &Intent<'_>,
    before: &Project,
    trial: &mut impl Trial,
    max_steps: usize,
    detach: BTreeSet<Endpoint>,
) -> Result<Vec<Correction>> {
    let mut steps = Vec::new();
    let mut edit = |trial: &mut _, correction: Correction, ep| -> Result<()> {
        if steps.len() >= max_steps {
            return Err(Error::Limit(format!(
                "{} needs more than {max_steps} operations.",
                pin_name(before, ep)
            )));
        }
        Trial::try_edit(trial, correction.command())
            .map_err(|why| Error::Conflict(format!("{}: {why}", pin_name(before, ep))))?;
        steps.push(correction);
        Ok(())
    };
    // Remove only a unique terminal branch. A junction, label or named
    // merge whose intent cannot be established is never guessed.
    let mut removed = BTreeSet::new();
    for ep in detach {
        let mut wires = before.wires_at(ep);
        let Some(wire) = wires.next() else {
            return Err(Error::Conflict(format!(
                "{} has no wire branch that can separate it from {}.",
                pin_name(before, ep),
                current(before, ep)?.name
            )));
        };
        let other = if wire.from == WireEnd::Terminal(ep) {
            wire.to
        } else {
            wire.from
        };
        if !matches!(other, WireEnd::Terminal(_) | WireEnd::Junction(_)) {
            return Err(Error::Conflict(format!(
                "{} is attached to a label, sheet pin or free wire. Review the drawing before disconnecting it.",
                pin_name(before, ep)
            )));
        }
        if wires.next().is_some() {
            return Err(Error::Conflict(format!(
                "{} has several wires. Choose which branch to disconnect.",
                pin_name(before, ep)
            )));
        }
        if removed.insert(wire.id) {
            edit(trial, Correction::Disconnect { wire: wire.id }, ep)?;
        }
    }
    for (name, members) in intent.nets {
        let anchor = members
            .iter()
            .copied()
            .find(|&ep| {
                trial
                    .snapshot()
                    .net_of(ep)
                    .is_some_and(|n| n.name.eq_ignore_ascii_case(name))
            })
            .or_else(|| members.first().copied())
            .ok_or_else(|| Error::Invalid(format!("Net {name} is empty.")))?;
        let initial = current(trial.snapshot(), anchor)?;
        let net_id = initial.id;
        if !initial.name.eq_ignore_ascii_case(name) {
            if !nets::auto_named(initial) {
                return Err(Error::Conflict(format!(
                    "{} already belongs to {}. Renaming that net needs an explicit edit.",
                    pin_name(before, anchor),
                    initial.name
                )));
            }
            edit(
                trial,
                Correction::Rename {
                    net: net_id,
                    name: name.clone(),
                },
                anchor,
            )?;
        }
        for &ep in members {
            let project = trial.snapshot();
            if current(project, ep)?.id != current(project, anchor)?.id {
                edit(
                    trial,
                    Correction::Connect {
                        from: anchor,
                        to: ep,
                    },
                    ep,
                )?;
            }
        }
    }
    for (name, members) in intent.nets {
        let first = members
            .first()
            .copied()
            .ok_or_else(|| Error::Invalid(format!("Net {name} is empty.")))?;
        let wanted: BTreeSet<_> = members.iter().copied().collect();
        let net = trial
            .snapshot()
            .nets
            .iter()
            .find(|net| net.name.eq_ignore_ascii_case(name));
        if !net.is_some_and(|net| net.terminals.iter().copied().eq(wanted.iter().copied())) {
            return Err(Error::Conflict(format!(
                "The proposed edits do not give {name} its exact terminal membership at {}.",
                pin_name(before, first)
            )));
        }
    }
    let after = trial.snapshot();
    // Keep the identity of every name the intent did not ask to change.
    // A separate named net made entirely of requested terminals still
    // requires an explicit decision before its name can disappear.
    let names_after: BTreeSet<_> = after.nets.iter().map(|n| n.name.as_str()).collect();
    for net in before
        .nets
        .iter()
        .filter(|n| !nets::auto_named(n) && intent.declared(n).is_none())
    {
        if !names_after.contains(net.name.as_str())
            && let Some(&ep) = net.terminals.first()
        {
            return Err(Error::Conflict(format!(
                "{} belongs to {}. Removing that named net needs an explicit edit.",
                pin_name(before, ep),
                net.name
            )));
        }
    }
    let outside_after = intent.outside(after);
    for (group, name) in intent.outside(before) {
        if outside_after.get(&group) != Some(&name)
            && let Some(&ep) = group.first()
        {
            return Err(Error::Conflict(format!(
                "The proposed edits would change {} outside the stated net intent.",
                pin_name(before, ep)
            )));
        }
    }
    Ok(steps)
}

pub(crate) fn check(
    project: &Project,
    nets: &IntentNets,
    max_steps: usize,
    trial: &mut impl Trial,
) -> Result<Report> {
    let intent = Intent::new(project, nets, max_steps)?;
    // A misplaced terminal can appear as both extra and wrong.
    let (mut report, detach) = intent.differences(project, max_steps.saturating_mul(2))?;
    if !(report.missing.is_empty() && report.wrong.is_empty() && report.extra.is_empty()) {
        match rehearse(&intent, project, trial, max_steps, detach) {
            Ok(steps) => report.operations = steps,
            Err(why) => report.blocked = Some(why.to_string()),
        }
    }
    Ok(report)
}
