//! The schematic view of deliberate joins. Electrical checks can follow a
//! closed tie while the source netlist retains every distinct net and pin.
use crate::diagnostics::{Finding, ObjectRef};
use crate::ids::{Endpoint, Id, TerminalId, TieId};
use crate::library::tie::{JumperState, TieForm, TiePurpose};
use crate::model::Project;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// A schematic netlist preserves each routed net and names the electrical
/// joins in the active assembly variant separately.
#[derive(Serialize, JsonSchema)]
pub struct Netlist<'a> {
    pub nets: &'a [crate::model::Net],
    pub ties: Vec<TieRow>,
}

impl<'a> Netlist<'a> {
    pub fn of(project: &'a Project) -> Self {
        Self {
            nets: &project.nets,
            ties: rows(project),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TieRow {
    pub group: TieId,
    pub instance: Id,
    pub designator: String,
    pub form: TieForm,
    pub purpose: TiePurpose,
    pub ends: BTreeMap<TerminalId, Option<Id>>,
    pub state: JumperState,
}

impl TieRow {
    /// The part designator, qualified by its group only when the part has several ties.
    pub fn label<'a>(&'a self, project: &Project) -> std::borrow::Cow<'a, str> {
        if project
            .instance(self.instance)
            .is_some_and(|i| i.ties.len() > 1)
        {
            format!("{} group {}", self.designator, self.group.0).into()
        } else {
            self.designator.as_str().into()
        }
    }

    /// Include one real terminal from the group so issue selection and waivers
    /// distinguish independent joins on a single physical component.
    pub(crate) fn objects(&self, related: impl IntoIterator<Item = ObjectRef>) -> Vec<ObjectRef> {
        std::iter::once(ObjectRef::Instance { id: self.instance })
            .chain(
                self.ends
                    .keys()
                    .take(1)
                    .map(|&terminal| ObjectRef::Terminal {
                        endpoint: Endpoint {
                            instance: self.instance,
                            terminal,
                        },
                    }),
            )
            .chain(related)
            .collect()
    }
}

pub fn form(project: &Project, instance: &crate::model::ComponentInstance) -> Option<TieForm> {
    project
        .library
        .component(instance.component)?
        .definition
        .ties
        .first()
        .map(|s| s.form)
}

/// The stencil for a three-pad jumper opens only the two lands of the chosen pair.
pub fn prints_paste(project: &Project, owner: Id, terminal: Option<Endpoint>) -> bool {
    let Some(instance) = project.instance(owner) else {
        return true;
    };
    let Some(component) = project.library.component(instance.component) else {
        return true;
    };
    let groups = &component.definition.ties;
    if groups.is_empty() {
        return true;
    }
    match component.definition.assembly {
        crate::library::tie::Assembly::CopperOnly => false,
        crate::library::tie::Assembly::Bought => true,
        crate::library::tie::Assembly::SolderOnly => {
            super::variants::population(instance, project.variants.active()).is_populated()
                && terminal.is_some_and(|ep| groups.iter().any(|s| s.ends.contains(&ep.terminal)))
        }
    }
}

pub fn rows(project: &Project) -> Vec<TieRow> {
    if !project.instances.iter().any(|i| !i.ties.is_empty()) {
        return vec![];
    }
    let nets: BTreeMap<_, _> = project
        .nets
        .iter()
        .flat_map(|n| n.terminals.iter().map(move |ep| (*ep, n.id)))
        .collect();
    let mut result = Vec::new();
    for i in project.instances.iter().filter(|i| !i.ties.is_empty()) {
        let Some(c) = project.library.component(i.component) else {
            continue;
        };
        for spec in &c.definition.ties {
            let Some(usage) = i.ties.get(&spec.id) else {
                continue;
            };
            let ends = spec
                .ends
                .iter()
                .map(|&terminal| {
                    (
                        terminal,
                        nets.get(&Endpoint {
                            instance: i.id,
                            terminal,
                        })
                        .copied(),
                    )
                })
                .collect();
            let closed = spec.form == TieForm::Copper
                || super::variants::population(i, project.variants.active()).is_populated();
            result.push(TieRow {
                group: spec.id,
                instance: i.id,
                designator: i.designator.clone(),
                form: spec.form,
                purpose: usage.purpose,
                ends,
                state: if closed {
                    JumperState::Closed
                } else {
                    JumperState::Open
                },
            });
        }
    }
    result
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Node {
    Net(Id),
    Tie((Id, TieId)),
}

/// Connected groups for ERC and ties on a cycle. The bipartite graph
/// represents a multi-end star as a single node, so it cannot make a loop by itself.
#[derive(Default)]
pub struct Network {
    groups: BTreeMap<Id, Id>,
    pub loops: BTreeSet<(Id, TieId)>,
}

impl Network {
    pub fn group(&self, net: Id) -> Id {
        self.groups.get(&net).copied().unwrap_or(net)
    }

    pub fn of(ties: &[TieRow]) -> Self {
        let mut adjacency: BTreeMap<Node, Vec<Node>> = BTreeMap::new();
        for t in ties.iter().filter(|t| t.state == JumperState::Closed) {
            let distinct: BTreeSet<_> = t.ends.values().flatten().copied().collect();
            for net in distinct {
                adjacency
                    .entry(Node::Tie((t.instance, t.group)))
                    .or_default()
                    .push(Node::Net(net));
                adjacency
                    .entry(Node::Net(net))
                    .or_default()
                    .push(Node::Tie((t.instance, t.group)));
            }
        }
        let mut network = Self::default();
        let mut discovered = BTreeMap::new();
        let mut low = BTreeMap::new();
        // Iterative low-link traversal. An edge on a cycle is either a
        // back edge or a tree edge whose child can reach an ancestor.
        for &root in adjacency.keys() {
            let Node::Net(group) = root else {
                continue;
            };
            if discovered.contains_key(&root) {
                continue;
            }
            let mut stack = vec![(root, None, 0usize)];
            let index = discovered.len();
            discovered.insert(root, index);
            low.insert(root, index);
            while let Some((node, parent, next)) = stack.pop() {
                if let Node::Net(n) = node {
                    network.groups.insert(n, group);
                }
                let start = discovered[&node];
                if let Some(&child) = adjacency.get(&node).and_then(|a| a.get(next)) {
                    stack.push((node, parent, next + 1));
                    if Some(child) == parent {
                        continue;
                    }
                    if let Some(&visited) = discovered.get(&child) {
                        if visited < start {
                            low.entry(node).and_modify(|n| *n = (*n).min(visited));
                            network.mark(node, child);
                        }
                    } else {
                        let index = discovered.len();
                        discovered.insert(child, index);
                        low.insert(child, index);
                        stack.push((child, Some(node), 0));
                    }
                } else if let Some(parent) = parent {
                    let child_low = low[&node];
                    if child_low <= discovered[&parent] {
                        network.mark(node, parent);
                    }
                    low.entry(parent).and_modify(|n| *n = (*n).min(child_low));
                }
            }
        }
        network
    }

    fn mark(&mut self, a: Node, b: Node) {
        if let Node::Tie(t) = a {
            self.loops.insert(t);
        }
        if let Node::Tie(t) = b {
            self.loops.insert(t);
        }
    }
}

pub fn findings(project: &Project, rows: &[TieRow], network: &Network) -> Vec<Finding> {
    let mut out = Vec::new();
    for t in rows {
        let mut distinct = BTreeSet::new();
        for net in t.ends.values().flatten() {
            if !distinct.insert(*net) {
                out.push(Finding::new(
                    FindingCode::TieSameNet,
                    format!(
                        "{} has two ends on {}.",
                        t.label(project),
                        project.net_name(*net)
                    ),
                    t.objects([ObjectRef::Net { id: *net }]),
                ));
                break;
            }
        }
        if network.loops.contains(&(t.instance, t.group)) {
            out.push(Finding::new(
                FindingCode::TieLoop,
                format!("{} is on a loop of closed ties.", t.label(project)),
                t.objects([]),
            ));
            if t.purpose == TiePurpose::StarGround {
                out.push(Finding::new(
                    FindingCode::TieStarPoints,
                    format!(
                        "{} has another path between its star-ground nets.",
                        t.label(project)
                    ),
                    t.objects([]),
                ));
            }
        }
        if t.purpose == TiePurpose::StarGround {
            let mut parts = BTreeMap::<Id, BTreeSet<Id>>::new();
            for net in distinct.iter().filter_map(|id| project.net(*id)) {
                for end in &net.terminals {
                    parts.entry(end.instance).or_default().insert(net.id);
                }
            }
            for (id, nets) in parts {
                if nets.len() < 2 {
                    continue;
                }
                if let Some(i) = project.instance(id).filter(|i| i.ties.is_empty()) {
                    out.push(Finding::new(
                        FindingCode::TieOtherBridge,
                        format!(
                            "{} also spans the star-ground nets of {}.",
                            i.designator,
                            t.label(project)
                        ),
                        t.objects([ObjectRef::Instance { id }]),
                    ));
                }
            }
        }
    }
    out
}

/// A landless copper package is the Kelvin implementation. It defaults to
/// that purpose as soon as the placement chooses its physical binding.
pub(super) fn initial_usage(
    library: &crate::library::Library,
    component: &crate::library::ComponentDefinition,
    choice: Option<crate::model::PhysicalChoice>,
) -> BTreeMap<TieId, crate::library::tie::TieUse> {
    if component.ties.is_empty() {
        return BTreeMap::new();
    }
    let landless = choice
        .and_then(|c| super::resolve_physical(library, component, c).ok())
        .is_some_and(|p| p.footprint.definition.pads.is_empty());
    component
        .ties
        .iter()
        .map(|s| {
            (
                s.id,
                crate::library::tie::TieUse {
                    purpose: if landless {
                        TiePurpose::KelvinSense
                    } else {
                        TiePurpose::None
                    },
                    ..Default::default()
                },
            )
        })
        .collect()
}

fn selected_group(
    i: &crate::model::ComponentInstance,
    group: Option<TieId>,
) -> crate::diagnostics::Result<TieId> {
    group
        .or_else(|| {
            (i.ties.len() == 1)
                .then(|| i.ties.keys().next().copied())
                .flatten()
        })
        .filter(|id| i.ties.contains_key(id))
        .ok_or_else(|| super::invalid(format!("Choose a tie group on {}.", i.designator)))
}

pub fn set_purpose(
    project: &mut Project,
    instance: Id,
    group: Option<TieId>,
    purpose: TiePurpose,
) -> crate::diagnostics::Result<()> {
    let i = super::instance_mut(project, instance)?;
    let group = selected_group(i, group)?;
    let name = &i.designator;
    let usage = i
        .ties
        .get_mut(&group)
        .ok_or_else(|| super::invalid(format!("{name} is an ordinary component.")))?;
    if !usage.anchors.is_empty() && purpose != TiePurpose::KelvinSense {
        return Err(super::invalid(format!(
            "Clear the anchors of {name} before changing its purpose."
        )));
    }
    usage.purpose = purpose;
    super::check_instance(project, instance)
}

pub fn set_anchor(
    project: &mut Project,
    instance: Id,
    group: Option<TieId>,
    end: TerminalId,
    anchor: Option<crate::library::tie::Anchor>,
) -> crate::diagnostics::Result<()> {
    let i = project
        .instance(instance)
        .ok_or_else(|| super::invalid("Choose a tie first.".into()))?;
    let group = selected_group(i, group)?;
    let spec = project
        .library
        .component(i.component)
        .and_then(|c| c.definition.ties.iter().find(|s| s.id == group));
    if !spec.is_some_and(|s| s.ends.contains(&end))
        || !i
            .ties
            .get(&group)
            .is_some_and(|u| u.purpose == TiePurpose::KelvinSense)
    {
        return Err(super::invalid(format!(
            "Choose a sense end of a Kelvin tie for {}.",
            i.designator
        )));
    }
    if let Some(a) = anchor {
        let valid = project
            .instance(a.instance)
            .filter(|other| other.id != instance && project.placement_of(other.id).is_some())
            .and_then(|other| super::resolve(&project.library, other).ok()?.physical)
            .and_then(|p| {
                p.binding
                    .definition
                    .terminal_of(p.footprint.definition.pad(a.pad)?.terminal?)
            })
            .is_some();
        if !valid {
            return Err(super::invalid(format!(
                "Anchor {} on an electrical pad of another placed part.",
                i.designator
            )));
        }
    }
    let usage = super::instance_mut(project, instance)?
        .ties
        .get_mut(&group)
        .ok_or_else(|| super::invalid("Choose a tie first.".into()))?;
    match anchor {
        Some(a) => {
            usage.anchors.insert(end, a);
        }
        None => {
            usage.anchors.remove(&end);
        }
    }
    Ok(())
}

/// The designer's requirement and the selected bought part's rated current.
/// Both values use the project's own copy of the standard field lineage.
pub(crate) fn currents(
    project: &Project,
    instance: &crate::model::ComponentInstance,
    field: Option<crate::ids::FieldRef>,
) -> (Option<f64>, Option<(String, f64)>) {
    let Some(field) = field else {
        return (None, None);
    };
    let amps = |v: &crate::fields::FieldValue| match v {
        crate::fields::FieldValue::Quantity { si, .. } => Some(si.to_f64()),
        _ => None,
    };
    let required = super::effective_fields(&project.library, instance)
        .ok()
        .and_then(|fields| {
            fields
                .into_iter()
                .find(|f| f.field.id == field.id)?
                .value
                .and_then(|v| amps(&v))
        });
    let rated = super::variants::fitted(project, instance)
        .part
        .and_then(|r| project.library.part(r))
        .and_then(|p| {
            let value = p.definition.specs.iter().find(|s| {
                s.field.id == field.id && s.meaning == crate::fields::SpecMeaning::MinimumRating
            })?;
            Some((p.definition.mpn.clone(), amps(&value.value)?))
        });
    (required, rated)
}
