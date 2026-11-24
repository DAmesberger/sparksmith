//! The report of every issue made from the last state's ([`Checked`]):
//! a state whose change touched only tracks and vias keeps the findings of
//! every check that reads neither (the Component and Schematic areas, and
//! the board families [`BoardCheck::reads_copper`] leaves out), and every
//! state keeps the copper outlines of the tracks and vias it did not
//! change ([`BoardCopper::next`]). Anything else is checked again in full.
//!
//! The rows are always made by the one assembly of the full report
//! (waivers, merging, wording, order), so a report made this way equals
//! [`super::report`] of the same state: the differential tests apply many
//! edits and compare the two after each.
use super::{
    IssueArea, IssueReport, IssueScope, assemble, board_area, component_findings,
    schematic_findings,
};
use crate::diagnostics::{Finding, Result};
use crate::ids::{Id, PadId};
use crate::layout::{
    self, BoardCheck, BoardCopper, CheckSettings, ConnectivityOrder, CopperChanges, CopperGraph,
    GraphSource, Items, NetCounts,
};
use crate::library::Drafts;
use crate::library::standards::Context;
use crate::model::{CopperLayer, Project};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

/// What changed between two states of a project.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Change {
    /// Nothing the checks read.
    None,
    /// Tracks, vias and the identity allocator; nothing else.
    Copper,
    /// Anything else.
    Other,
}

/// What changed from `old` to `new`. Every field but tracks, vias and the
/// next identity is compared; a section both still share compares by its
/// handle alone.
fn change(old: &Project, new: &Project) -> Change {
    if std::ptr::eq(old, new) {
        return Change::None;
    }
    let mut probe = old.clone();
    probe.tracks = new.tracks.clone();
    probe.vias = new.vias.clone();
    probe.next_id = new.next_id;
    if probe != *new {
        return Change::Other;
    }
    if old.tracks == new.tracks && old.vias == new.vias {
        Change::None
    } else {
        Change::Copper
    }
}

/// One state's report of every area ([`IssueScope::All`]), with the raw
/// findings and copper outlines it was made from, so the next state's
/// report is made from it ([`Checked::next`]).
pub struct Checked {
    project: Arc<Project>,
    drafts: Arc<Drafts>,
    settings: CheckSettings,
    /// The context it was made in: its standards are read by findings
    /// (what a tie's part is rated for), its help by rows (how to fix).
    context: Option<Arc<Context>>,
    copper: Arc<BoardCopper>,
    component: Vec<Finding>,
    schematic: Vec<Finding>,
    board: Vec<(BoardCheck, Vec<Finding>)>,
    /// The findings of the families that check item by item, per item.
    items: Vec<(BoardCheck, Items)>,
    /// Where the pad holes are, for whether a change reaches them.
    holes: Option<layout::PadHoleIndex>,
    /// Each net's copper graph counts, when the state's graph is complete.
    counts: Option<Arc<BTreeMap<Id, NetCounts>>>,
    /// The index of each connected pad, for the connectivity order.
    pads: Arc<HashMap<(Id, PadId), usize>>,
    /// Each fill's net, layer and key reach ([`layout::fill::key_reaches`]).
    fills: Arc<Vec<layout::fill::KeyReach>>,
    /// The board interface's and the assembly link's findings.
    links: Vec<Finding>,
    /// Whether the clearance check stopped at its pair limit.
    stopped: bool,
    report: Arc<IssueReport>,
    /// Which checks this state made from the last one's (tests).
    #[cfg(test)]
    paths: Paths,
}

/// Which checks a state made from the last state's rather than in full.
#[cfg(test)]
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Paths {
    pub kept: bool,
    pub clearance: bool,
    pub vias: bool,
    pub pad_holes: bool,
    pub connectivity: bool,
}

impl Checked {
    /// Whether the context current on this thread is the one it was made
    /// in.
    fn in_context(&self) -> bool {
        match (&self.context, Context::current().ok()) {
            (Some(made), Some(now)) => Arc::ptr_eq(made, &now),
            (None, None) => true,
            _ => false,
        }
    }

    /// Every check of `project` and `drafts` under `settings`, in the
    /// context current on this thread. `graph` gives the state's copper
    /// graph, asked for only when a check needs it.
    pub fn full(
        project: &Arc<Project>,
        drafts: &Arc<Drafts>,
        settings: &CheckSettings,
        graph: &dyn Fn() -> Arc<CopperGraph>,
    ) -> Result<Self> {
        let copper = Arc::new(BoardCopper::of(project));
        Self::make(project, drafts, settings, copper, graph, None)
    }

    /// The report of `project` and `drafts` under `settings`, made from
    /// `self`, the report of an earlier (or any other) state: equal to
    /// [`Checked::full`] of the same state.
    pub fn next(
        &self,
        project: &Arc<Project>,
        drafts: &Arc<Drafts>,
        settings: &CheckSettings,
        graph: &dyn Fn() -> Arc<CopperGraph>,
    ) -> Result<Self> {
        let change = change(&self.project, project);
        let same_inputs =
            Arc::ptr_eq(&self.drafts, drafts) && self.settings == *settings && self.in_context();
        if same_inputs && change == Change::None {
            return Ok(Self {
                project: project.clone(),
                drafts: drafts.clone(),
                settings: *settings,
                context: self.context.clone(),
                copper: self.copper.clone(),
                component: self.component.clone(),
                schematic: self.schematic.clone(),
                board: self.board.clone(),
                items: self.items.clone(),
                holes: self.holes.clone(),
                counts: self.counts.clone(),
                pads: self.pads.clone(),
                fills: self.fills.clone(),
                links: self.links.clone(),
                stopped: self.stopped,
                report: self.report.clone(),
                #[cfg(test)]
                paths: Paths::default(),
            });
        }
        let same_lands = change != Change::Other;
        let copper = Arc::new(self.copper.next(project, same_lands));
        let kept = (same_inputs && change == Change::Copper && !self.stopped).then_some(self);
        Self::make(project, drafts, settings, copper, graph, kept)
    }

    fn make(
        project: &Arc<Project>,
        drafts: &Arc<Drafts>,
        settings: &CheckSettings,
        copper: Arc<BoardCopper>,
        graph: &dyn Fn() -> Arc<CopperGraph>,
        kept: Option<&Self>,
    ) -> Result<Self> {
        let source = GraphSource::lazy(graph);
        let (pads, fills) = match kept {
            Some(last) => (last.pads.clone(), last.fills.clone()),
            None => (
                Arc::new(ConnectivityOrder::pad_index(project)),
                Arc::new(layout::fill::key_reaches(project)),
            ),
        };
        // A copper change checks again only the items it reaches, and the
        // connectivity of the nets it reaches.
        let changes = kept.map(|last| copper.changes_from(&last.copper));
        let touched = changes
            .as_ref()
            .map(|c| reached_nets(c, &fills))
            .unwrap_or_default();
        let sub = Sub::default();
        let graph_of = |extra: &BTreeSet<Id>| match kept {
            Some(last) => sub.graph(project, &touched, extra, last.counts.as_deref()),
            None => {
                source.get();
                source.made()
            }
        };
        let last_items = |check: BoardCheck| {
            kept.and_then(|l| l.items.iter().find(|(c, _)| *c == check))
                .map(|(_, items)| items)
        };
        let mut items: Vec<(BoardCheck, Items)> = Vec::new();
        for check in [
            BoardCheck::Clearance,
            BoardCheck::Vias,
            BoardCheck::Physical,
        ] {
            if let Some(made) =
                check.items(project, &copper, last_items(check).zip(changes.as_ref()))
            {
                items.push((check, made));
            }
        }
        let last_holes = last_items(BoardCheck::PadHoles)
            .zip(kept.and_then(|l| l.holes.as_ref()))
            .zip(changes.as_ref())
            .map(|((items, index), changes)| (items, index, changes, &touched));
        let mut holes = None;
        if let Some((made, index)) = layout::pad_hole_items(&copper, project, &graph_of, last_holes)
        {
            items.push((BoardCheck::PadHoles, made));
            holes = Some(index);
        }
        // The connectivity of the nets reached, from a graph of those nets;
        // every other net's findings are the last state's.
        let mut connectivity = kept
            .filter(|_| !project.instances.iter().any(|i| !i.ties.is_empty()))
            .and_then(|last| {
                let old = last
                    .board
                    .iter()
                    .find(|(c, _)| *c == BoardCheck::Connectivity)?;
                // Nothing the copper graph reads changed.
                if touched.is_empty() {
                    return Some(old.1.clone());
                }
                let graph = graph_of(&BTreeSet::new())?;
                let nets = sub.nets();
                let order = ConnectivityOrder::new(project, &pads);
                let kept: Vec<&Finding> = old
                    .1
                    .iter()
                    .filter(|f| order.net(f).is_none_or(|n| !nets.contains(&n)))
                    .collect();
                // Both lists are in the graph's order: merge them.
                let mut findings = Vec::with_capacity(kept.len());
                let mut from = 0usize;
                for f in graph.findings(project) {
                    let key = order.key(&f);
                    let to = from
                        + kept
                            .get(from..)
                            .map_or(0, |rest| rest.partition_point(|o| order.key(o) < key));
                    findings.extend(
                        kept.get(from..to)
                            .into_iter()
                            .flatten()
                            .map(|o| (*o).clone()),
                    );
                    findings.push(f);
                    from = to;
                }
                findings.extend(kept.get(from..).into_iter().flatten().map(|o| (*o).clone()));
                Some(findings)
            });
        let made_connectivity = connectivity.is_some();
        let (board, stopped) =
            layout::check_families(project, &copper, &source, settings, &mut |check| {
                if let Some((_, items)) = items.iter().find(|(c, _)| *c == check) {
                    return Some(items.findings().cloned().collect());
                }
                if check == BoardCheck::Connectivity && connectivity.is_some() {
                    return connectivity.take();
                }
                let kept = kept.filter(|_| !check.reads_copper())?;
                kept.board
                    .iter()
                    .find(|(c, _)| *c == check)
                    .map(|(_, f)| f.clone())
            });
        let counts = match (kept, made_connectivity) {
            (Some(last), true) => sub.counts().or_else(|| last.counts.clone()),
            _ => source.made().and_then(|g| g.net_counts()).map(Arc::new),
        };
        let (component, schematic, links) = match kept {
            Some(last) => (
                last.component.clone(),
                last.schematic.clone(),
                last.links.clone(),
            ),
            None => (
                component_findings(project, drafts)?,
                schematic_findings(project),
                board_area([], project),
            ),
        };
        let areas = vec![
            (IssueArea::Component, component.clone()),
            (IssueArea::Schematic, schematic.clone()),
            (
                IssueArea::Board,
                board
                    .iter()
                    .flat_map(|(_, f)| f.iter().cloned())
                    .chain(links.iter().cloned())
                    .collect(),
            ),
        ];
        // The Component and Schematic rows of a copper change are the last
        // report's: their findings are, and no name they show is a track's
        // or a via's.
        let rows = |area: IssueArea| {
            let last = kept?;
            if area == IssueArea::Board {
                return None;
            }
            let rows: Vec<&super::IssueRow> =
                last.report.rows.iter().filter(|r| r.area == area).collect();
            let named = |o: &crate::diagnostics::ObjectRef| {
                matches!(
                    o,
                    crate::diagnostics::ObjectRef::Track { .. }
                        | crate::diagnostics::ObjectRef::Via { .. }
                )
            };
            if rows.iter().any(|r| r.objects().any(named)) {
                return None;
            }
            Some(rows.into_iter().cloned().collect())
        };
        let report = Arc::new(assemble(project, drafts, areas, &IssueScope::All, &rows));
        #[cfg(test)]
        let paths = Paths {
            kept: kept.is_some(),
            clearance: kept.is_some() && items.iter().any(|(c, _)| *c == BoardCheck::Clearance),
            vias: kept.is_some() && items.iter().any(|(c, _)| *c == BoardCheck::Vias),
            pad_holes: kept.is_some() && items.iter().any(|(c, _)| *c == BoardCheck::PadHoles),
            connectivity: made_connectivity,
        };
        Ok(Self {
            project: project.clone(),
            drafts: drafts.clone(),
            settings: *settings,
            context: Context::current().ok(),
            copper,
            component,
            schematic,
            board,
            items,
            holes,
            counts,
            pads,
            fills,
            links,
            stopped,
            report,
            #[cfg(test)]
            paths,
        })
    }

    /// Whether this is the report of exactly this state: the same
    /// project and drafts allocations, the same settings and the context
    /// current on this thread.
    pub fn describes(
        &self,
        project: &Arc<Project>,
        drafts: &Arc<Drafts>,
        settings: &CheckSettings,
    ) -> bool {
        Arc::ptr_eq(&self.project, project)
            && Arc::ptr_eq(&self.drafts, drafts)
            && self.settings == *settings
            && self.in_context()
    }

    /// The report of every area.
    pub fn report(&self) -> &Arc<IssueReport> {
        &self.report
    }

    /// Which checks this state made from the last one's.
    #[cfg(test)]
    pub(super) fn paths(&self) -> Paths {
        self.paths
    }

    /// The state's board copper outlines.
    pub fn copper(&self) -> &Arc<BoardCopper> {
        &self.copper
    }
}

/// The nets whose connectivity a copper change can change: the nets of the
/// tracks and vias it changed, and the nets of every fill on a layer where
/// a changed track or via comes within some fill's key reach (whether a
/// fill's copper is fresh, and so joins its net, follows its key, which
/// also hashes the keys of the fills it clears on its layer).
fn reached_nets(changes: &CopperChanges, fills: &[layout::fill::KeyReach]) -> BTreeSet<Id> {
    let mut nets = changes.nets.clone();
    let reached: BTreeSet<CopperLayer> = fills
        .iter()
        .filter(|(_, _, reach)| {
            reach.is_some_and(|r| changes.bounds.iter().any(|b| r.intersects(b)))
        })
        .map(|(_, layer, _)| *layer)
        .collect();
    nets.extend(
        fills
            .iter()
            .filter(|(_, layer, _)| reached.contains(layer))
            .filter_map(|(net, _, _)| *net),
    );
    nets
}

/// The copper graph of the nets a copper change reached (and of any nets
/// more that a check asked for), made once, with each net's counts. It
/// stands in for the whole board's graph on those nets only while the
/// whole board's graph would be complete: the totals of the last state's
/// counts with these nets' counts replaced are within the limits.
/// The nets a [`Sub`] graph holds, the graph and every net's counts.
type Made = (BTreeSet<Id>, Arc<CopperGraph>, Arc<BTreeMap<Id, NetCounts>>);

#[derive(Default)]
struct Sub {
    made: std::cell::RefCell<Option<Made>>,
}

impl Sub {
    fn graph(
        &self,
        project: &Project,
        touched: &BTreeSet<Id>,
        extra: &BTreeSet<Id>,
        last: Option<&BTreeMap<Id, NetCounts>>,
    ) -> Option<Arc<CopperGraph>> {
        let nets: BTreeSet<Id> = touched.union(extra).copied().collect();
        if let Some((have, graph, _)) = self.made.borrow().as_ref()
            && nets.is_subset(have)
        {
            return Some(graph.clone());
        }
        let last = last?;
        let list: Vec<Id> = nets.iter().copied().collect();
        let graph = Arc::new(CopperGraph::of_nets(project, &list));
        let mine = graph.net_counts()?;
        let mut counts: BTreeMap<Id, NetCounts> = last
            .iter()
            .filter(|(net, _)| !nets.contains(net))
            .map(|(net, c)| (*net, *c))
            .collect();
        counts.extend(mine);
        if !NetCounts::total(counts.values()).within_limits() {
            return None;
        }
        *self.made.borrow_mut() = Some((nets, graph.clone(), Arc::new(counts)));
        Some(graph)
    }

    /// The nets of the graph made.
    fn nets(&self) -> BTreeSet<Id> {
        self.made
            .borrow()
            .as_ref()
            .map(|(nets, _, _)| nets.clone())
            .unwrap_or_default()
    }

    /// Every net's counts with the graph made.
    fn counts(&self) -> Option<Arc<BTreeMap<Id, NetCounts>>> {
        self.made.borrow().as_ref().map(|(_, _, c)| c.clone())
    }
}

#[cfg(test)]
#[path = "tests/incremental.rs"]
mod tests;
