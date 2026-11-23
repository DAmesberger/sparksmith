//! The schematic view of deliberate joins. Electrical checks can follow a
//! closed tie while the source netlist retains every distinct net and pin.
use crate::diagnostics::{Finding, ObjectRef};
use crate::ids::{Endpoint, Id, TerminalId};
use crate::library::tie::{JumperState, TieForm, TiePurpose};
use crate::model::Project;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TieRow {
    pub instance: Id,
    pub designator: String,
    pub form: TieForm,
    pub purpose: TiePurpose,
    pub ends: BTreeMap<TerminalId, Option<Id>>,
    pub state: JumperState,
}

pub fn rows(project: &Project) -> Vec<TieRow> {
    if !project.instances.iter().any(|i| i.tie.is_some()) {
        return vec![];
    }
    let nets: BTreeMap<_, _> = project
        .nets
        .iter()
        .flat_map(|n| n.terminals.iter().map(move |ep| (*ep, n.id)))
        .collect();
    project
        .instances
        .iter()
        .filter_map(|i| {
            let usage = i.tie.as_ref()?;
            let spec = project
                .library
                .component(i.component)?
                .definition
                .tie
                .as_ref()?;
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
            Some(TieRow {
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
            })
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Node {
    Net(Id),
    Tie(Id),
}

/// Connected groups for ERC and ties on a cycle. The bipartite graph
/// represents a multi-end star as a single node, so it cannot make a loop by itself.
#[derive(Default)]
pub struct Network {
    groups: BTreeMap<Id, Id>,
    pub loops: BTreeSet<Id>,
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
                    .entry(Node::Tie(t.instance))
                    .or_default()
                    .push(Node::Net(net));
                adjacency
                    .entry(Node::Net(net))
                    .or_default()
                    .push(Node::Tie(t.instance));
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
        let owner = ObjectRef::Instance { id: t.instance };
        let mut distinct = BTreeSet::new();
        for net in t.ends.values().flatten() {
            if !distinct.insert(*net) {
                out.push(Finding::new(
                    "tie.same_net",
                    format!(
                        "{} has two ends on {}.",
                        t.designator,
                        project.net_name(*net)
                    ),
                    vec![owner, ObjectRef::Net { id: *net }],
                ));
                break;
            }
        }
        if network.loops.contains(&t.instance) {
            out.push(Finding::new(
                "tie.loop",
                format!("{} is on a loop of closed ties.", t.designator),
                vec![owner],
            ));
            if t.purpose == TiePurpose::StarGround {
                out.push(Finding::new(
                    "tie.star_points",
                    format!(
                        "{} has another path between its star-ground nets.",
                        t.designator
                    ),
                    vec![owner],
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
                if let Some(i) = project.instance(id).filter(|i| i.tie.is_none()) {
                    out.push(Finding::new(
                        "tie.other_bridge",
                        format!(
                            "{} also spans the star-ground nets of {}.",
                            i.designator, t.designator
                        ),
                        vec![owner, ObjectRef::Instance { id }],
                    ));
                }
            }
        }
    }
    out
}
