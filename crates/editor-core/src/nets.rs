//! Persistent net identity. Wires and label names define connectivity; nets
//! record which connected terminal group carries which stable ID and name.
//! Labels name their nets ([`names`]).
//!
//! After any schematic change, [`reconcile`] keeps every existing net ID with
//! the connected group holding most of its former terminals. Merged groups keep
//! the routed (or else oldest) ID; split-off groups and new terminals receive new
//! IDs. Tracks are never retagged here: callers decide whether a change to
//! routed nets is acceptable.
//!
//! A change that joins two nets that each carry a name of their own (VBUS
//! and GND) merges them only when its caller asks for that merge after a
//! review ([`named_merges`], [`refuse_named_merges`]).
use crate::diagnostics::{Error, ObjectRef, Result};
use crate::ids::{Endpoint, Id, SheetId};
use crate::model::sheet::SheetPath;
use crate::model::{LabelKind, Net, Project, WireEnd};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// What a reconciliation did to net identities.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NetChanges {
    /// (merged-away ID, surviving ID)
    pub merged: Vec<(Id, Id)>,
    /// (original ID, new ID of a split-off group)
    pub split: Vec<(Id, Id)>,
    pub created: Vec<Id>,
    /// IDs whose terminals all disappeared.
    pub removed: Vec<Id>,
}

/// Every terminal of every instance, whatever its unit's state, in sorted
/// order.
pub fn terminals(project: &Project) -> BTreeSet<Endpoint> {
    let mut result = BTreeSet::new();
    for instance in &project.instances {
        if let Some(component) = project.library.component(instance.component) {
            for terminal in &component.definition.terminals {
                result.insert(Endpoint {
                    instance: instance.id,
                    terminal: terminal.id,
                });
            }
        }
    }
    result
}

/// A terminal is accounted for when it is connected (its net has at least two
/// terminals), marked project no-connect, or not electrical (device NC or
/// reserved), which needs no decision.
pub fn accounted_for(project: &Project, terminal: Endpoint) -> bool {
    let Some(instance) = project.instance(terminal.instance) else {
        return false;
    };
    let no_decision = project
        .library
        .component(instance.component)
        .and_then(|c| c.definition.terminal(terminal.terminal))
        .is_some_and(|t| !t.is_electrical());
    no_decision
        || instance.no_connect.contains(&terminal.terminal)
        || project
            .net_of(terminal)
            .is_some_and(|net| net.terminals.len() >= 2)
}

/// Union-find over terminals and the other connection points (junctions,
/// labels and sheet pins): terminals take indices `0..terminals.len()`, the
/// others the indices after them, so a group's root is its smallest
/// terminal whenever it has one.
///
/// Wires join their two ends. Names join labels by scope, never by drawn
/// position: local and hierarchical labels of one name on one sheet, global
/// labels and power ports of one name everywhere, and a sheet pin with the
/// hierarchical labels of its name on the child sheet. Names compare
/// ASCII-case-insensitively, as net names do.
struct Nodes {
    index: BTreeMap<(usize, WireEnd), usize>,
    parent: Vec<usize>,
}

/// Scope zero holds physical terminals and the drawing objects of top-level pages.
/// Other indices identify full paths in the validated occurrence table.
fn scopes(project: &Project) -> BTreeMap<SheetId, Vec<usize>> {
    let mut scopes = BTreeMap::<_, Vec<_>>::new();
    for sheet in &project.sheets.list {
        if project.is_root_sheet(sheet.id) {
            scopes.entry(sheet.id).or_default().push(0);
        }
    }
    for (i, u) in project.sheets.uses.iter().enumerate() {
        scopes.entry(u.definition).or_default().push(i + 1);
    }
    scopes
}

fn scoped_end(project: &Project, scope: usize, end: WireEnd) -> (usize, WireEnd) {
    match end {
        WireEnd::Terminal(mut pin) => {
            if let Some(u) = scope
                .checked_sub(1)
                .and_then(|i| project.sheets.uses.get(i))
                && let Some(id) = u.parts.get(&pin.instance)
            {
                pin.instance = *id;
            }
            (0, WireEnd::Terminal(pin))
        }
        _ => (scope, end),
    }
}

impl Nodes {
    fn insert(&mut self, key: (usize, WireEnd)) -> usize {
        *self.index.entry(key).or_insert_with(|| {
            let at = self.parent.len();
            self.parent.push(at);
            at
        })
    }

    fn new(project: &Project, terminals: &BTreeSet<Endpoint>) -> Self {
        let mut nodes = Self {
            index: BTreeMap::new(),
            parent: Vec::new(),
        };
        for pin in terminals {
            nodes.insert((0, WireEnd::Terminal(*pin)));
        }
        let scopes = scopes(project);
        let on = |sheet: SheetId| scopes.get(&sheet).into_iter().flatten().copied();
        for j in &project.junctions {
            for scope in on(j.sheet) {
                nodes.insert((scope, WireEnd::junction(j.id)));
            }
        }
        let mut names = BTreeMap::new();
        let mut ports = BTreeMap::new();
        for label in &project.labels {
            for scope in on(label.sheet) {
                let name = label.name.to_ascii_lowercase();
                let at = nodes.insert((scope, WireEnd::label(label.id)));
                let key = (
                    (!label.kind.is_global()).then_some((scope, label.sheet)),
                    name.clone(),
                );
                nodes.union(at, *names.entry(key).or_insert(at));
                if matches!(label.kind, LabelKind::Hierarchical { .. }) {
                    ports.insert((scope, name), at);
                }
            }
        }
        let child_scopes: BTreeMap<_, _> = project
            .sheets
            .uses
            .iter()
            .enumerate()
            .map(|(i, u)| (&u.path, i + 1))
            .collect();
        for symbol in &project.sheet_symbols {
            for scope in on(symbol.sheet) {
                let parent = scope
                    .checked_sub(1)
                    .and_then(|i| project.sheets.uses.get(i))
                    .map_or_else(|| SheetPath::root(symbol.sheet), |u| u.path.clone());
                let child = child_scopes.get(&parent.child(symbol.id));
                for pin in &symbol.pins {
                    let at = nodes.insert((scope, WireEnd::sheet_pin(pin.id)));
                    if let Some(port) =
                        child.and_then(|c| ports.get(&(*c, pin.name.to_ascii_lowercase())))
                    {
                        nodes.union(at, *port);
                    }
                }
            }
        }
        for wire in &project.wires {
            for scope in on(wire.sheet) {
                if let (Some(&a), Some(&b)) = (
                    nodes.index.get(&scoped_end(project, scope, wire.from)),
                    nodes.index.get(&scoped_end(project, scope, wire.to)),
                ) {
                    nodes.union(a, b);
                }
            }
        }
        nodes
    }
    fn union(&mut self, a: usize, b: usize) {
        let (a, b) = (self.find(a), self.find(b));
        if a != b {
            self.parent[a.max(b)] = a.min(b);
        }
    }
    fn find(&mut self, mut at: usize) -> usize {
        while self.parent[at] != at {
            self.parent[at] = self.parent[self.parent[at]];
            at = self.parent[at];
        }
        at
    }
    fn root(&mut self, end: (usize, WireEnd)) -> Option<usize> {
        let at = *self.index.get(&end)?;
        Some(self.find(at))
    }
    /// The net each root carries, from the nets' terminals.
    fn root_nets(&mut self, project: &Project) -> BTreeMap<usize, Id> {
        let mut root_net = BTreeMap::new();
        for net in &project.nets {
            for terminal in &net.terminals {
                if let Some(root) = self.root((0, WireEnd::Terminal(*terminal))) {
                    root_net.insert(root, net.id);
                }
            }
        }
        root_net
    }
}

/// Wire-connected terminal groups, ordered by their smallest terminal.
/// Wires joined through junctions connect their terminals.
pub fn components(project: &Project, terminals: &BTreeSet<Endpoint>) -> Vec<Vec<Endpoint>> {
    let mut nodes = Nodes::new(project, terminals);
    let mut groups: BTreeMap<usize, Vec<Endpoint>> = BTreeMap::new();
    for (i, terminal) in terminals.iter().enumerate() {
        let root = nodes.find(i);
        groups.entry(root).or_default().push(*terminal);
    }
    // Roots are the smallest index of each group, so this is min-terminal order.
    groups.into_values().collect()
}

/// The occurrence shown for each definition, selected explicitly or the first use.
fn visible_scopes(project: &Project, path: Option<&SheetPath>) -> BTreeMap<SheetId, usize> {
    let mut visible: BTreeMap<_, _> = scopes(project)
        .into_iter()
        .filter_map(|(sheet, uses)| Some((sheet, *uses.first()?)))
        .collect();
    if let Some((i, u)) = path.and_then(|p| {
        project
            .sheets
            .uses
            .iter()
            .enumerate()
            .find(|(_, u)| &u.path == p)
    }) {
        visible.insert(u.definition, i + 1);
    }
    visible
}

/// The net each wire belongs to, by wire ID. A wire whose connected group
/// reaches no terminal has none.
pub fn wire_nets(project: &Project) -> BTreeMap<Id, Id> {
    wire_nets_in(project, None)
}

pub fn wire_nets_in(project: &Project, path: Option<&SheetPath>) -> BTreeMap<Id, Id> {
    let terminals = terminals(project);
    let mut nodes = Nodes::new(project, &terminals);
    let root_net = nodes.root_nets(project);
    let visible = visible_scopes(project, path);
    project
        .wires
        .iter()
        .filter_map(|w| {
            let scope = *visible.get(&w.sheet)?;
            let root = nodes
                .root(scoped_end(project, scope, w.from))
                .or_else(|| nodes.root(scoped_end(project, scope, w.to)))?;
            Some((w.id, *root_net.get(&root)?))
        })
        .collect()
}

/// The drawing and channel of a wire carrying a physical net.
pub fn wire_of_net(project: &Project, net: Id) -> Option<(Id, SheetId, Option<&SheetPath>)> {
    let scopes = scopes(project);
    let mut nodes = Nodes::new(project, &terminals(project));
    let nets = nodes.root_nets(project);
    for wire in &project.wires {
        for &scope in scopes.get(&wire.sheet).into_iter().flatten() {
            let root = nodes
                .root(scoped_end(project, scope, wire.from))
                .or_else(|| nodes.root(scoped_end(project, scope, wire.to)));
            if root.and_then(|r| nets.get(&r)) == Some(&net) {
                let path = scope
                    .checked_sub(1)
                    .and_then(|i| project.sheets.uses.get(i))
                    .map(|u| &u.path);
                return Some((wire.id, wire.sheet, path));
            }
        }
    }
    None
}

/// The visible definition's first occurrence, for callers editing its drawing.
pub fn point_nets(project: &Project) -> BTreeMap<WireEnd, Id> {
    point_nets_in(project, None)
}

pub fn point_nets_in(project: &Project, path: Option<&SheetPath>) -> BTreeMap<WireEnd, Id> {
    let scopes: BTreeSet<_> = visible_scopes(project, path).into_values().collect();
    let mut nodes = Nodes::new(project, &terminals(project));
    let root_net = nodes.root_nets(project);
    let points: Vec<_> = nodes
        .index
        .keys()
        .filter(|(_, end)| end.terminal().is_none())
        .copied()
        .collect();
    let mut result = BTreeMap::new();
    for key in points {
        if scopes.contains(&key.0)
            && let Some(net) = nodes.root(key).and_then(|r| root_net.get(&r))
        {
            result.insert(key.1, *net);
        }
    }
    result
}

/// What kind of object supplies a name to a net, in priority order: the
/// first kind present names the net.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum NameKind {
    Power,
    Global,
    Local,
    /// A hierarchical label or a sheet pin.
    Hierarchical,
}

/// One label or sheet pin on a net.
#[derive(Clone, Debug, PartialEq)]
pub struct NameSource {
    /// `WireEnd::Label` or `WireEnd::SheetPin`.
    pub end: WireEnd,
    pub name: String,
    pub kind: NameKind,
    pub path: SheetPath,
    /// The sheet it is drawn on.
    pub sheet: SheetId,
}

/// Each sheet's depth below its top-level ancestor (0 for a top-level
/// sheet), bounded by the sheet count so a malformed cycle terminates.
pub fn sheet_depths(project: &Project) -> BTreeMap<SheetId, usize> {
    let mut depths = BTreeMap::new();
    for u in &project.sheets.uses {
        depths.entry(u.definition).or_insert(u.path.symbols.len());
    }
    for s in &project.sheets.list {
        depths.entry(s.id).or_insert(0);
    }
    depths
}

/// The labels and sheet pins on each net, by net ID, in priority order
/// (kind, sheet depth, page order, name, then ID).
pub fn name_sources(project: &Project) -> BTreeMap<Id, Vec<NameSource>> {
    let terminals = terminals(project);
    let mut nodes = Nodes::new(project, &terminals);
    let root_net = nodes.root_nets(project);
    let mut out: BTreeMap<Id, Vec<NameSource>> = BTreeMap::new();
    let scopes = scopes(project);
    let mut push = |sheet: SheetId, end, name: &str, kind| {
        for &scope in scopes.get(&sheet).into_iter().flatten() {
            if let Some(net) = nodes.root((scope, end)).and_then(|r| root_net.get(&r)) {
                let path = scope
                    .checked_sub(1)
                    .and_then(|i| project.sheets.uses.get(i))
                    .map_or_else(|| SheetPath::root(sheet), |u| u.path.clone());
                out.entry(*net).or_default().push(NameSource {
                    end,
                    name: name.to_owned(),
                    kind,
                    sheet,
                    path,
                });
            }
        }
    };
    for label in &project.labels {
        let kind = match label.kind {
            LabelKind::Power { .. } => NameKind::Power,
            LabelKind::Global { .. } => NameKind::Global,
            LabelKind::Local => NameKind::Local,
            LabelKind::Hierarchical { .. } => NameKind::Hierarchical,
        };
        push(label.sheet, WireEnd::label(label.id), &label.name, kind);
    }
    for symbol in &project.sheet_symbols {
        for pin in &symbol.pins {
            push(
                symbol.sheet,
                WireEnd::sheet_pin(pin.id),
                &pin.name,
                NameKind::Hierarchical,
            );
        }
    }
    let order = |s: SheetId| project.sheets.number(s).unwrap_or(usize::MAX);
    for sources in out.values_mut() {
        sources.sort_by(|a, b| {
            (a.kind, a.path.symbols.len(), order(a.sheet), &a.name, a.end).cmp(&(
                b.kind,
                b.path.symbols.len(),
                order(b.sheet),
                &b.name,
                b.end,
            ))
        });
    }
    out
}

/// A sheet name as a net-name qualifier: whitespace and control characters
/// become `_`.
fn qualifier(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_whitespace() || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect()
}

/// `text` cut to at most `max` bytes on a character boundary.
fn truncated(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// Longest net name, in bytes ([`name_valid`]).
const MAX_NAME: usize = 64;

/// `wanted` if no net took it yet (case-insensitively), else made unique
/// with the net's ID.
fn claim(wanted: &str, id: Id, taken: &mut BTreeSet<String>) -> String {
    let mut name = truncated(wanted, MAX_NAME).to_owned();
    if taken.contains(&name.to_ascii_lowercase()) {
        let suffix = format!("~{id}");
        name = format!(
            "{}{suffix}",
            truncated(wanted, MAX_NAME.saturating_sub(suffix.len()))
        );
    }
    taken.insert(name.to_ascii_lowercase());
    name
}

/// Every net's name (docs/schematic-capture.md, "Which name wins"). A net
/// with labels or sheet pins takes the name of the first by
/// [`name_sources`] order: power port, global label, local label, then
/// hierarchical label or sheet pin; among those, the one nearest the top of
/// the hierarchy, on the earliest page, alphabetically first. A local or
/// hierarchical name another net also carries is qualified with its sheet's
/// name (`Power/EN`). A net without labels keeps its stored name unless a
/// labelled net now carries it, when it falls back to `N<id>`. A global
/// name is unique by construction: the same global name is the same net.
pub fn names(project: &Project) -> BTreeMap<Id, String> {
    let sources = name_sources(project);
    let chosen: BTreeMap<Id, &NameSource> = sources
        .iter()
        .filter_map(|(net, s)| Some((*net, s.first()?)))
        .collect();
    let mut taken = BTreeSet::new();
    let mut out = BTreeMap::new();
    for (net, source) in &chosen {
        if matches!(source.kind, NameKind::Power | NameKind::Global) {
            out.insert(*net, claim(&source.name, *net, &mut taken));
        }
    }
    let mut local_count: BTreeMap<String, usize> = BTreeMap::new();
    for source in chosen.values() {
        if matches!(source.kind, NameKind::Local | NameKind::Hierarchical) {
            *local_count
                .entry(source.name.to_ascii_lowercase())
                .or_default() += 1;
        }
    }
    for (net, source) in &chosen {
        if !matches!(source.kind, NameKind::Local | NameKind::Hierarchical) {
            continue;
        }
        let lower = source.name.to_ascii_lowercase();
        let shared = local_count.get(&lower).copied().unwrap_or(0) > 1 || taken.contains(&lower);
        let wanted = if shared {
            let sheet = qualifier(&source.path.display(project));
            format!("{sheet}/{}", source.name)
        } else {
            source.name.clone()
        };
        out.insert(*net, claim(&wanted, *net, &mut taken));
    }
    for net in &project.nets {
        if out.contains_key(&net.id) {
            continue;
        }
        let stored = if taken.contains(&net.name.to_ascii_lowercase()) {
            default_name(net.id)
        } else {
            net.name.clone()
        };
        out.insert(net.id, claim(&stored, net.id, &mut taken));
    }
    out
}

/// Rebuild `project.nets` from wires while preserving existing identities.
pub fn reconcile(project: &mut Project) -> Result<NetChanges> {
    let terminals = terminals(project);
    let groups = components(project, &terminals);
    let old_net_of: BTreeMap<Endpoint, Id> = project
        .nets
        .iter()
        .flat_map(|net| net.terminals.iter().map(move |t| (*t, net.id)))
        .collect();
    let routed = project.routed_nets();
    // Overlap of each old net with each group; the largest overlap is its home.
    let mut overlap: BTreeMap<Id, BTreeMap<usize, usize>> = BTreeMap::new();
    for (group, members) in groups.iter().enumerate() {
        for terminal in members {
            if let Some(id) = old_net_of.get(terminal) {
                *overlap.entry(*id).or_default().entry(group).or_default() += 1;
            }
        }
    }
    let mut homed: BTreeMap<usize, Vec<Id>> = BTreeMap::new();
    for (id, counts) in &overlap {
        // max_by_key keeps the last maximum; iterate reversed to prefer the
        // earliest group on ties.
        if let Some((group, _)) = counts.iter().rev().max_by_key(|(_, count)| **count) {
            homed.entry(*group).or_default().push(*id);
        }
    }
    let names: BTreeMap<Id, String> = project
        .nets
        .iter()
        .map(|n| (n.id, n.name.clone()))
        .collect();
    let mut changes = NetChanges::default();
    let mut nets = Vec::with_capacity(groups.len());
    let mut group_net = Vec::with_capacity(groups.len());
    for (group, members) in groups.into_iter().enumerate() {
        let candidates = homed.remove(&group).unwrap_or_default();
        let survivor = match candidates
            .iter()
            .find(|id| routed.contains(id))
            .or(candidates.first())
        {
            Some(id) => *id,
            None => {
                let id = project.allocate_net()?;
                changes.created.push(id);
                id
            }
        };
        for id in &candidates {
            if *id != survivor {
                changes.merged.push((*id, survivor));
            }
        }
        // A name a person gave a net outlives a merge with a net nobody
        // named, whichever ID survives.
        let given = |id: &Id| names.get(id).filter(|n| **n != default_name(*id));
        let name = given(&survivor)
            .or_else(|| candidates.iter().find_map(given))
            .cloned()
            .unwrap_or_else(|| default_name(survivor));
        group_net.push(survivor);
        nets.push(Net {
            id: survivor,
            name,
            terminals: members,
        });
    }
    for (id, counts) in &overlap {
        for group in counts.keys() {
            let assigned = group_net.get(*group).copied();
            if let Some(assigned) = assigned
                && assigned != *id
                && !changes.merged.contains(&(*id, assigned))
            {
                changes.split.push((*id, assigned));
            }
        }
    }
    let kept: BTreeSet<Id> = nets.iter().map(|n| n.id).collect();
    let merged: BTreeSet<Id> = changes.merged.iter().map(|(id, _)| *id).collect();
    changes.removed = project
        .nets
        .iter()
        .map(|n| n.id)
        .filter(|id| !kept.contains(id) && !merged.contains(id))
        .collect();
    nets.sort_by_key(|n| n.id);
    let old = std::mem::replace(&mut project.nets, nets.into());
    let names = self::names(project);
    for net in &mut project.nets {
        if let Some(name) = names.get(&net.id) {
            net.name.clone_from(name);
        }
    }
    // Unchanged nets stay the section the undo history shares.
    if *old == *project.nets {
        project.nets = old;
    }
    Ok(changes)
}

/// One net a [`NamedMerge`] joins, as it was before the change.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MergedNet {
    pub net: Id,
    pub name: String,
}

/// Nets that each carried a name a person gave them before a change (a
/// label, power port, sheet pin or a stored name, [`given_names`]) and are
/// one net after it,
/// with no name in common: VBUS and GND. Such a merge applies only when the
/// caller asks for it after a review (`ApplyUpdate::merge_named_nets`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NamedMerge {
    /// The nets that become one, oldest first.
    pub nets: Vec<MergedNet>,
    /// The net they become: the routed one survives, otherwise the oldest
    /// ([`reconcile`]).
    pub net: Id,
    /// The name it carries after the change.
    pub name: String,
    /// What joins them: the labels, sheet pins and wires on that net the
    /// change adds or alters, at most [`MAX_JOINED_BY`].
    pub joined_by: Vec<ObjectRef>,
}

/// The most objects a [`NamedMerge`] lists as joining its nets.
pub const MAX_JOINED_BY: usize = 32;

impl NamedMerge {
    /// "VBUS and GND", "VBUS, 3V3 and GND".
    pub fn names(&self) -> String {
        let names: Vec<&str> = self.nets.iter().map(|n| n.name.as_str()).collect();
        match names.split_last() {
            Some((last, rest)) if !rest.is_empty() => format!("{} and {last}", rest.join(", ")),
            Some((last, _)) => (*last).to_owned(),
            None => String::new(),
        }
    }

    /// One line: "Joins VBUS and GND into one net, GND".
    pub fn line(&self) -> String {
        format!("Joins {} into one net, {}", self.names(), self.name)
    }

    /// The full wording, naming what joins the nets in `after`.
    pub fn detail(&self, after: &Project) -> String {
        let mut joiners: Vec<String> = Vec::new();
        let mut wires = 0usize;
        for object in &self.joined_by {
            match object {
                ObjectRef::Label { id } => {
                    if let Some(label) = after.label(*id) {
                        joiners.push(format!("the {} {}", label.kind.noun(), label.name));
                    }
                }
                ObjectRef::SheetPin { id } => {
                    if let Some(pin) = after
                        .sheet_symbols
                        .iter()
                        .flat_map(|s| &s.pins)
                        .find(|p| p.id == *id)
                    {
                        joiners.push(format!("the sheet pin {}", pin.name));
                    }
                }
                ObjectRef::Wire { .. } => wires += 1,
                _ => {}
            }
        }
        match wires {
            0 => {}
            1 => joiners.push("a wire".into()),
            n => joiners.push(format!("{n} wires")),
        }
        let by = match joiners.split_last() {
            Some((last, rest)) if !rest.is_empty() => {
                format!("{} and {last} join them", rest.join(", "))
            }
            Some((last, _)) => format!("{last} joins them"),
            None => "this change joins them".into(),
        };
        format!(
            "Nets {} become one net, {}: {by}. Apply only if they are meant to be one net.",
            self.names(),
            self.name
        )
    }
}

/// The names people gave each net, lowercased, by net ID: the names of its
/// labels, power ports and sheet pins ([`name_sources`]), or, on a net with
/// none, the name stored on the net itself when it is not the generated
/// `N<id>` (a `schematic.net_rename`, or the name a removed label left).
/// A net nobody named is not listed. However a person named a net, this is
/// the one answer to whether its identity is protected.
pub fn given_names(project: &Project) -> BTreeMap<Id, BTreeSet<String>> {
    let mut out: BTreeMap<Id, BTreeSet<String>> = name_sources(project)
        .into_iter()
        .map(|(id, sources)| {
            let names = sources
                .iter()
                .map(|s| s.name.to_ascii_lowercase())
                .collect();
            (id, names)
        })
        .collect();
    for net in project.nets.iter().filter(|n| !auto_named(n)) {
        out.entry(net.id)
            .or_insert_with(|| BTreeSet::from([net.name.to_ascii_lowercase()]));
    }
    out
}

/// Every [`NamedMerge`] from `before` to `after`. Only nets that join
/// terminals of two named nets are looked at, so a change that merges
/// nothing costs one pass over the terminals.
pub fn named_merges(before: &Project, after: &Project) -> Vec<NamedMerge> {
    let before_net: BTreeMap<Endpoint, Id> = before
        .nets
        .iter()
        .flat_map(|net| net.terminals.iter().map(move |t| (*t, net.id)))
        .collect();
    let joined: Vec<(&Net, BTreeSet<Id>)> = after
        .nets
        .iter()
        .filter_map(|net| {
            let ids: BTreeSet<Id> = net
                .terminals
                .iter()
                .filter_map(|t| before_net.get(t).copied())
                .collect();
            (ids.len() >= 2).then_some((net, ids))
        })
        .collect();
    if joined.is_empty() {
        return Vec::new();
    }
    let names = given_names(before);
    let mut out = Vec::new();
    for (net, ids) in joined {
        let named: Vec<(Id, &BTreeSet<String>)> = ids
            .iter()
            .filter_map(|id| Some((*id, names.get(id)?)))
            .collect();
        let apart = named.iter().enumerate().any(|(i, (_, a))| {
            named
                .get(i + 1..)
                .unwrap_or_default()
                .iter()
                .any(|(_, b)| a.is_disjoint(b))
        });
        if !apart {
            continue;
        }
        out.push(NamedMerge {
            nets: named
                .iter()
                .map(|(id, _)| MergedNet {
                    net: *id,
                    name: before.net_name(*id).to_owned(),
                })
                .collect(),
            net: net.id,
            name: net.name.clone(),
            joined_by: joined_by(before, after, net.id),
        });
    }
    out
}

/// The labels, sheet pins and wires on `net` in `after` that `before` does
/// not have as they are.
fn joined_by(before: &Project, after: &Project, net: Id) -> Vec<ObjectRef> {
    let mut out = Vec::new();
    let before_pin = |id: Id| {
        before
            .sheet_symbols
            .iter()
            .flat_map(|s| &s.pins)
            .find(|p| p.id == id)
    };
    for (end, at) in point_nets(after) {
        if at != net {
            continue;
        }
        match end {
            WireEnd::Label(l) if before.label(l.label) != after.label(l.label) => {
                out.push(ObjectRef::Label { id: l.label });
            }
            WireEnd::SheetPin(p) => {
                let now = after
                    .sheet_symbols
                    .iter()
                    .flat_map(|s| &s.pins)
                    .find(|x| x.id == p.sheet_pin);
                if before_pin(p.sheet_pin) != now {
                    out.push(ObjectRef::SheetPin { id: p.sheet_pin });
                }
            }
            _ => {}
        }
    }
    for (wire, at) in wire_nets(after) {
        if at == net
            && before.wires.iter().find(|w| w.id == wire)
                != after.wires.iter().find(|w| w.id == wire)
        {
            out.push(ObjectRef::Wire { id: wire });
        }
    }
    out.truncate(MAX_JOINED_BY);
    out
}

/// Refuse a move (a symbol, label, sheet symbol or wire segment dragged
/// on the sheet) that joins nets which each already connected two or more
/// pins: dropping a pin on a wire connects a pin that was free, but a move
/// never shorts two circuits by where it lands (E2E run 13: C3 moved by its
/// Position fields merged EN into GND). The person connects them on
/// purpose with a wire, or, for named nets, through a reviewed update.
pub fn refuse_moved_merges(before: &Project, after: &Project) -> Result<()> {
    let before_net: BTreeMap<Endpoint, Id> = before
        .nets
        .iter()
        .filter(|net| net.terminals.len() >= 2)
        .flat_map(|net| net.terminals.iter().map(move |t| (*t, net.id)))
        .collect();
    for net in &after.nets {
        let joined: BTreeSet<Id> = net
            .terminals
            .iter()
            .filter_map(|t| before_net.get(t).copied())
            .collect();
        if joined.len() < 2 {
            continue;
        }
        let names: Vec<String> = joined
            .iter()
            .map(|id| {
                before
                    .nets
                    .iter()
                    .find(|n| n.id == *id)
                    .map_or_else(|| format!("N{id}"), |n| described(before, n))
            })
            .collect();
        return Err(Error::Conflict(format!(
            "net.moved_merge: this move would join {} into one net by where it lands. Move it elsewhere, or connect them on purpose with a wire.",
            names.join(" and ")
        )));
    }
    Ok(())
}

/// Refuse a change from `before` to `after` that makes a [`NamedMerge`]:
/// the caller previews it and applies it as a reviewed update that asks
/// for the merge.
pub fn refuse_named_merges(before: &Project, after: &Project) -> Result<()> {
    let merges = named_merges(before, after);
    let Some(first) = merges.first() else {
        return Ok(());
    };
    Err(Error::Conflict(format!(
        "net.named_merge: {} Review it first: named nets join only through a reviewed update that asks for the merge (merge_named_nets: true).",
        first.detail(after)
    )))
}

pub fn default_name(id: Id) -> String {
    format!("N{id}")
}

/// Whether `net` still carries its automatic name (`N<id>`): nobody named it.
pub fn auto_named(net: &Net) -> bool {
    net.name == default_name(net.id)
}

/// Pins a [`described`] net lists before it counts the rest.
const DESCRIBED_PINS: usize = 6;

/// A net as a picker lists it: a named net by its name ("VBUS"), an
/// automatically named one with the pins it joins, as designator and pin
/// name ("N2 · J1 CC1, R3 2"), sorted, the first six and "+n more".
pub fn described(project: &Project, net: &Net) -> String {
    if !auto_named(net) {
        return net.name.clone();
    }
    let mut pins: Vec<String> = net
        .terminals
        .iter()
        .map(|e| {
            let Some(instance) = project.instance(e.instance) else {
                return format!("pin {}", e.terminal);
            };
            let name = project
                .library
                .component(instance.component)
                .and_then(|c| c.definition.terminal(e.terminal))
                .map(|t| t.name.trim())
                .filter(|n| !n.is_empty())
                .map_or_else(|| e.terminal.to_string(), str::to_owned);
            format!("{} {name}", instance.designator)
        })
        .collect();
    if pins.is_empty() {
        return net.name.clone();
    }
    pins.sort();
    let more = pins.len().saturating_sub(DESCRIBED_PINS);
    pins.truncate(DESCRIBED_PINS);
    let mut out = format!("{} · {}", net.name, pins.join(", "));
    if more > 0 {
        out.push_str(&format!(", +{more} more"));
    }
    out
}

pub fn name_valid(name: &str) -> bool {
    !name.trim().is_empty()
        && name.len() <= 64
        && !name.chars().any(|c| c.is_control() || c.is_whitespace())
}

/// Net records must match wire connectivity exactly.
pub fn validate(project: &Project) -> Result<()> {
    let invalid = |message: &str| Err(Error::Invalid(message.into()));
    let terminals = terminals(project);
    let mut seen_ids = BTreeSet::new();
    let mut names = BTreeSet::new();
    let mut members: BTreeMap<Endpoint, Id> = BTreeMap::new();
    for net in &project.nets {
        if net.id == 0 || net.id >= project.next_net || !seen_ids.insert(net.id) {
            return invalid("net identities must be nonzero, allocated and unique");
        }
        if !name_valid(&net.name) || !names.insert(net.name.to_lowercase()) {
            return invalid("net names must be unique, 1–64 bytes, without spaces");
        }
        if net.terminals.is_empty() || !net.terminals.windows(2).all(|w| w[0] < w[1]) {
            return invalid("net terminals must be nonempty and strictly sorted");
        }
        for terminal in &net.terminals {
            if !terminals.contains(terminal) {
                return invalid("a net references a missing terminal");
            }
            if members.insert(*terminal, net.id).is_some() {
                return invalid("a terminal belongs to more than one net");
            }
        }
    }
    if members.len() != terminals.len() {
        return invalid("every terminal must belong to a net");
    }
    let by_id: BTreeMap<Id, &Net> = project.nets.iter().map(|n| (n.id, n)).collect();
    for group in components(project, &terminals) {
        let id = group.first().and_then(|t| members.get(t));
        let Some(net) = id.and_then(|id| by_id.get(id)) else {
            return invalid("a connected terminal group has no net");
        };
        if net.terminals != group {
            return invalid("net membership does not match wire connectivity");
        }
    }
    let names = self::names(project);
    if project
        .nets
        .iter()
        .any(|n| names.get(&n.id) != Some(&n.name))
    {
        return invalid("net names do not match their labels");
    }
    Ok(())
}
