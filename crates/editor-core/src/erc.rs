//! Net-level electrical rules over active terminal behavior. Each terminal's
//! behavior is its active function from `instances::function_resolution`;
//! an unresolved terminal is reported and excluded from the drive rules, so
//! ERC never assumes all alternatives at once and never picks one.
//!
//! Findings describe circuit state and never block a component stage. A
//! terminal alone on its net is not an ERC condition: completeness reports it
//! once as `instance.terminal_unaccounted`. Pin rule `warn` verdicts are ERC
//! notes (`config.rule_note`); `require` verdicts and rule failures block
//! electrical resolution and are completeness findings.
use crate::diagnostics::{Finding, ObjectRef};
use crate::functions::{ActiveFunction, Direction, Drive, Resolution};
use crate::ids::{Endpoint, Id};
use crate::instances;
use crate::model::{ComponentInstance, Net, Project};
use std::collections::{BTreeMap, BTreeSet};

mod labels;

/// What one net terminal contributes to the net rules.
enum Behavior<'a> {
    Active(&'a ActiveFunction),
    Unresolved,
}

/// A terminal for a message: "R1 terminal ‘VCC’", by designator and the
/// terminal's name in the instance's component revision.
fn terminal(project: &Project, endpoint: Endpoint) -> String {
    let Some(instance) = project.instance(endpoint.instance) else {
        return format!("terminal {}", endpoint.terminal);
    };
    let name = project
        .library
        .component(instance.component)
        .and_then(|c| c.definition.terminal(endpoint.terminal))
        .map_or_else(
            || endpoint.terminal.to_string(),
            |t| format!("‘{}’", t.name),
        );
    format!("{} terminal {name}", instance.designator)
}

fn behavior<'a>(
    resolutions: &'a BTreeMap<Id, Resolution>,
    endpoint: Endpoint,
) -> Option<Behavior<'a>> {
    let resolution = resolutions.get(&endpoint.instance)?;
    if let Some(active) = resolution.active(endpoint.terminal) {
        return Some(Behavior::Active(active));
    }
    resolution
        .unresolved
        .contains(&endpoint.terminal)
        .then_some(Behavior::Unresolved)
}

fn net_checks(
    project: &Project,
    net: &Net,
    resolutions: &BTreeMap<Id, Resolution>,
    power_nets: &BTreeSet<Id>,
    out: &mut Vec<Finding>,
) {
    let this = ObjectRef::Net { id: net.id };
    let mut active = Vec::new();
    let mut unresolved = Vec::new();
    for endpoint in &net.terminals {
        match behavior(resolutions, *endpoint) {
            Some(Behavior::Active(function)) => active.push((*endpoint, function)),
            Some(Behavior::Unresolved) => unresolved.push(*endpoint),
            None => {}
        }
    }
    let connected = net.terminals.len() >= 2;
    if connected {
        for endpoint in &unresolved {
            out.push(Finding::new(
                "erc.unchecked_terminal",
                format!(
                    "{}: {} has no active function and is not checked.",
                    net.name,
                    terminal(project, *endpoint)
                ),
                vec![
                    this,
                    ObjectRef::Terminal {
                        endpoint: *endpoint,
                    },
                ],
            ));
        }
        // Every pair of drivers, judged by the one matrix; each kind of
        // fight is reported once per net.
        let drives: Vec<Drive> = active
            .iter()
            .filter(|(_, f)| matches!(f.direction, Direction::Output | Direction::Bidirectional))
            .map(|(_, f)| f.drive)
            .collect();
        let mut fights = BTreeSet::new();
        for (i, a) in drives.iter().enumerate() {
            for b in drives.get(i + 1..).unwrap_or_default() {
                if let Some(fight) = contention(*a, *b) {
                    fights.insert(fight);
                }
            }
        }
        for fight in fights {
            out.push(Finding::new(
                fight.code(),
                fight.message(&net.name),
                vec![this],
            ));
        }
        let has = |direction: Direction| active.iter().any(|(_, f)| f.direction == direction);
        if has(Direction::Input)
            && !has(Direction::Output)
            && !has(Direction::Bidirectional)
            && !has(Direction::Passive)
        {
            out.push(Finding::new(
                "erc.input_only_net",
                format!("{}: inputs are connected with no driver.", net.name),
                vec![this],
            ));
        }
    }
    // A net with a power port reports a missing source once, as
    // `erc.power_port_undriven`.
    if !power_nets.contains(&net.id) && !active.iter().any(|(_, f)| supplies(f.direction)) {
        for (endpoint, _) in active
            .iter()
            .filter(|(_, f)| f.direction == Direction::PowerInput)
        {
            out.push(Finding::new(
                "erc.undriven_power_input",
                format!(
                    "{} has no power-output driver on its net.",
                    terminal(project, *endpoint)
                ),
                vec![ObjectRef::Terminal {
                    endpoint: *endpoint,
                }],
            ));
        }
    }
}

/// How two drivers on one net fight, if they do.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Contention {
    /// Two push-pull drivers: each drives high and low against the other.
    PushPull,
    /// A push-pull driver against an open-drain sink: it drives high while
    /// the sink pulls low.
    OpenDrain,
    /// An open-source driver against a push-pull driver or an open-drain
    /// sink: it sources high while the other pulls low.
    OpenSource,
}

impl Contention {
    pub fn code(self) -> &'static str {
        match self {
            Self::PushPull => "erc.driver_conflict",
            Self::OpenDrain => "erc.open_drain_contention",
            Self::OpenSource => "erc.open_source_contention",
        }
    }

    fn message(self, net: &str) -> String {
        match self {
            Self::PushPull => format!("{net}: push-pull drivers are connected to each other."),
            Self::OpenDrain => {
                format!("{net}: a push-pull driver shares the net with an open-drain driver.")
            }
            Self::OpenSource => {
                format!("{net}: an open-source driver shares the net with a driver that pulls low.")
            }
        }
    }
}

/// The drive compatibility matrix, the one place that says which drivers
/// may share a net: open drains wire-AND, open sources wire-OR, and every
/// pair that can drive the net high and low at once fights. A drive of
/// `None` drives nothing. Symmetric.
pub fn contention(a: Drive, b: Drive) -> Option<Contention> {
    use Drive::{None, OpenDrain, OpenSource, PushPull};
    match (a, b) {
        (None, _) | (_, None) | (OpenDrain, OpenDrain) | (OpenSource, OpenSource) => Option::None,
        (PushPull, PushPull) => Some(Contention::PushPull),
        (PushPull, OpenDrain) | (OpenDrain, PushPull) => Some(Contention::OpenDrain),
        (OpenSource, PushPull | OpenDrain) | (PushPull | OpenDrain, OpenSource) => {
            Some(Contention::OpenSource)
        }
    }
}

/// Whether a terminal of `direction` supplies a power net: only a power
/// output does. A passive pin (a capacitor, a connector contact) supplies
/// nothing; a connector that brings power in declares that contact a
/// power output. The one answer for `erc.undriven_power_input` and
/// `erc.power_port_undriven`.
pub fn supplies(direction: Direction) -> bool {
    direction == Direction::PowerOutput
}

/// Every ERC finding of the project, net by net.
pub fn checks(project: &Project) -> Vec<Finding> {
    let resolutions: BTreeMap<Id, Resolution> = project
        .instances
        .iter()
        .filter_map(|i| {
            instances::function_resolution(&project.library, i)
                .ok()
                .map(|r| (i.id, r))
        })
        .collect();
    let ties = instances::ties::rows(project);
    let network = instances::ties::Network::of(&ties);
    let mut out = instances::ties::findings(project, &ties, &network);
    let named_power = labels::power_nets(project);
    let mut power_nets = named_power.clone();
    let mut driven = std::collections::BTreeSet::new();
    if !ties.is_empty() {
        driven.extend(labels::grounded(project).iter().map(|n| network.group(*n)));
        for net in &project.nets {
            if net.terminals.iter().any(|e| matches!(behavior(&resolutions,*e),Some(Behavior::Active(f)) if supplies(f.direction))) {
                driven.insert(network.group(net.id));
            }
        }
        power_nets.extend(
            project
                .nets
                .iter()
                .filter(|n| driven.contains(&network.group(n.id)))
                .map(|n| n.id),
        );
    }
    if ties.is_empty() {
        for net in &project.nets {
            net_checks(project, net, &resolutions, &power_nets, &mut out);
        }
    } else {
        // ERC sees the combined terminals, while the stored netlist, class
        // membership, board copper and the physical lengths stay independent.
        let mut groups = BTreeMap::<Id, Vec<&Net>>::new();
        for net in &project.nets {
            groups.entry(network.group(net.id)).or_default().push(net);
        }
        let tie_pins: BTreeSet<_> = ties
            .iter()
            .flat_map(|t| {
                t.ends.keys().map(|&terminal| Endpoint {
                    instance: t.instance,
                    terminal,
                })
            })
            .collect();
        for (id, members) in groups {
            let group = Net {
                id,
                name: members
                    .iter()
                    .map(|n| n.name.as_str())
                    .collect::<Vec<_>>()
                    .join(" / "),
                terminals: members
                    .iter()
                    .flat_map(|n| &n.terminals)
                    .filter(|e| !tie_pins.contains(e))
                    .copied()
                    .collect(),
            };
            if members.iter().any(|n| power_nets.contains(&n.id)) {
                power_nets.insert(id);
            }
            let start = out.len();
            net_checks(project, &group, &resolutions, &power_nets, &mut out);
            for finding in &mut out[start..] {
                if finding.objects.contains(&ObjectRef::Net { id }) {
                    finding.objects.extend(
                        members
                            .iter()
                            .filter(|n| n.id != id)
                            .map(|n| ObjectRef::Net { id: n.id }),
                    );
                }
            }
        }
    }
    labels::checks(
        project,
        &resolutions,
        &|net| driven.contains(&network.group(net)),
        &mut out,
    );
    for instance in &project.instances {
        if let Some(resolution) = resolutions.get(&instance.id) {
            let rules = project
                .library
                .component(instance.component)
                .map(|c| c.definition.pin_rules.as_slice())
                .unwrap_or_default();
            rule_notes(instance, resolution, rules, &mut out);
        }
    }
    out
}

/// Where a rule came from in its reference document, as a message suffix
/// (" (source: page 16, Table 5)"); empty when the rule names none.
pub(crate) fn rule_source(rules: &[crate::functions::PinRule], rule: crate::ids::RuleId) -> String {
    let Some(source) = rules
        .iter()
        .find(|r| r.id == rule)
        .and_then(|r| r.source.as_ref())
    else {
        return String::new();
    };
    let mut parts = Vec::new();
    if let Some(page) = source.page {
        parts.push(format!("page {page}"));
    }
    if let Some(table) = source
        .table
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
    {
        parts.push(table.to_owned());
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!(" (source: {})", parts.join(", "))
    }
}

/// Pin rule `warn` verdicts (C1): advisory, never blocking. The message
/// keeps the rule's datasheet source.
fn rule_notes(
    instance: &ComponentInstance,
    resolution: &Resolution,
    rules: &[crate::functions::PinRule],
    out: &mut Vec<Finding>,
) {
    for note in &resolution.notes {
        let mut objects = vec![ObjectRef::Instance { id: instance.id }];
        objects.extend(note.terminals.iter().map(|terminal| ObjectRef::Terminal {
            endpoint: Endpoint {
                instance: instance.id,
                terminal: *terminal,
            },
        }));
        out.push(Finding::new(
            "config.rule_note",
            format!(
                "{}: {}{}",
                instance.designator,
                note.message,
                rule_source(rules, note.rule)
            ),
            objects,
        ));
    }
}
