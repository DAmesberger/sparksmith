//! What must hold of a board after every edit, each on what was seen of it
//! ([`Seen`]): a mutant changes what was seen, never the product.
use super::Fallible;
use super::truth::{self, Copper, Shape};
use crate::diagnostics::Severity;
use crate::diagnostics::{Finding, FindingCode};
use crate::fabrication::preflight;
use crate::fabrication::readback;
use crate::fabrication::{self, FabricationOptions, Identity, ProductionSnapshot};
use crate::geometry::Point;
use crate::ids::{Id, PadId};
use crate::issues::{self, IssueReport, IssueScope, can_waive};
use crate::layout::CopperGraph;
use crate::layout::fill::{self, RefillTarget};
use crate::layout::router::reroute::Rerouted;
use crate::library::PadKind;
use crate::model::board::CutoutKind;
use crate::model::fill::{FilledRegion, IslandRemoval};
use crate::model::{CopperLayer, Document, Fill, Layer, Project};
use crate::rules::{CopperObject, NetClass, RuleKey, RuleValue};
use crate::{Engine, ObjectRef, format};
use sparksmith_fabcheck::raster::{self, Bitmap};
use sparksmith_fabcheck::{drills, excellon, gerber};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// The rules of this layer, as its messages and mutants name them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Rule {
    /// A saved and reopened board is the board.
    Reopen,
    /// Undo and redo go back and forth exactly.
    History,
    /// A refused edit changes nothing.
    Refusal,
    /// Nothing is made from a report or a graph of an earlier state.
    Cache,
    /// The copper graph joins what is joined, and says what is not.
    Connectivity,
    /// No edit loses copper without saying so.
    Silent,
    /// Copper of two nets that meets is a finding no waiver accepts.
    Short,
    /// Copper of two nets nearer than the rules allow is told.
    Close,
    /// What is told as an error blocks the production check.
    Blocks,
    /// A board without a valid outline gives no files.
    Outline,
    /// A fill is the board's now, or told as stale.
    Stale,
    /// A fill has the islands that join its net, and no others.
    Island,
    /// A fill keeps every clearance.
    Clearance,
    /// The check before every export passes.
    ReadBack,
    /// The files draw what the design has.
    Image,
    /// Read back, the files list every hole, via, track and pad once.
    Inventory,
    /// The board could not be made, edited or written at all.
    Error,
}

#[derive(Clone, Debug)]
pub(super) struct Broken {
    pub rule: Rule,
    pub text: String,
}

pub(super) fn broken(rule: Rule, text: impl Into<String>) -> Broken {
    Broken {
        rule,
        text: text.into(),
    }
}

/// Each connected pad, by net, with the number of the island it is on.
pub(super) type Islands = BTreeMap<Id, BTreeMap<(Id, PadId), usize>>;

pub(super) fn islands(graph: &CopperGraph) -> Islands {
    let mut out = Islands::new();
    for (k, pad) in graph.pads().iter().enumerate() {
        // A pad of no island is one of its own.
        let island = graph.pad_island(k).unwrap_or(usize::MAX - k);
        out.entry(pad.net)
            .or_default()
            .insert((pad.instance, pad.pad), island);
    }
    out
}

/// What was seen of a board.
#[derive(Clone)]
pub(super) struct Seen {
    pub project: Project,
    /// What the copper graph joins, when it found every joint.
    pub islands: Option<Islands>,
    /// The graph's own airwires, as the nets they are of.
    pub airwires: BTreeSet<Id>,
    pub findings: Vec<Finding>,
    /// Its fills filled again, as a host does before it makes files.
    pub made: Project,
    /// The files of [`Self::made`]; none when only the board was looked at.
    pub files: Vec<(String, Vec<u8>)>,
    /// The release the files state.
    pub identity: Identity,
    /// The files were refused as the outline is not valid.
    pub refused: bool,
}

/// The board with every fill computed again.
pub(super) fn refilled(project: &Project) -> Fallible<Project> {
    let mut made = project.clone();
    for (id, copper) in fill::refill(project, RefillTarget::All)? {
        if let Some(f) = made.fills.iter_mut().find(|f| f.id == id) {
            f.computed = Some(copper);
        }
    }
    Ok(made)
}

pub(super) fn options() -> FabricationOptions {
    let mut options = FabricationOptions {
        base_name: Some("random".into()),
        ..FabricationOptions::default()
    };
    // The PDFs are the io crate's, and tested there.
    options.outputs.drawings = false;
    options
}

impl Seen {
    /// The board alone, without its files.
    pub(super) fn board(project: &Project) -> Fallible<Self> {
        let graph = CopperGraph::of(project);
        Ok(Self {
            project: project.clone(),
            islands: graph.complete().then(|| islands(&graph)),
            airwires: graph.airwires().iter().map(|(a, _)| a.net).collect(),
            findings: crate::layout::checks(project, &Default::default()),
            made: refilled(project)?,
            files: Vec::new(),
            identity: Identity::default(),
            refused: true,
        })
    }

    /// The board and its files.
    pub(super) fn files(project: &Project) -> Fallible<Self> {
        let mut seen = Self::board(project)?;
        let set = fabrication::generate(
            &ProductionSnapshot::resolve(&seen.made, None)?,
            &options(),
            &Default::default(),
        );
        if project.board_shape.invalid().is_some() {
            // Refused, whatever else it is: `outline`.
            seen.refused = set.is_err();
            return Ok(seen);
        }
        let set = set?;
        seen.identity = set.identity;
        seen.files = set.files.into_iter().map(|f| (f.name, f.bytes)).collect();
        Ok(seen)
    }

    fn told(&self, code: FindingCode, object: ObjectRef) -> bool {
        self.findings
            .iter()
            .any(|f| f.code == code && f.objects.contains(&object))
    }

    /// The fills no finding says are stale.
    fn current(&self) -> impl Iterator<Item = &Fill> {
        self.project
            .fills
            .iter()
            .filter(|f| !self.told(FindingCode::FillStale, ObjectRef::Fill { id: f.id }))
    }
}

/// Every rule of the board that was seen; those of its files when there are
/// any, drawn at `pixel` mm.
pub(super) fn all(seen: &Seen, pixel: f64) -> Fallible<Vec<Broken>> {
    let mut out = Vec::new();
    connectivity(seen, &mut out);
    shorts(seen, &mut out);
    close(seen, &mut out);
    blocks(seen, &mut out)?;
    stale(seen, &mut out);
    clearance(&seen.made, None, &mut out)?;
    let fresh: BTreeSet<Id> = seen.current().map(|f| f.id).collect();
    clearance(&seen.project, Some(&fresh), &mut out)?;
    if !seen.files.is_empty() {
        islands_stay(seen, &mut out)?;
        read_back(seen, &mut out)?;
        image(seen, pixel, &mut out)?;
        inventory(seen, &mut out)?;
    }
    Ok(out)
}

/// The graph may join what is no further apart than this: its own micron
/// and the chords of both sides. What overlaps at all it must join.
const REACH: f64 = 0.004;

/// The islands of `a` are unions of the islands of `b`: whatever `b` joins,
/// `a` does. Returns two pads where not.
fn coarser(
    a: &BTreeMap<(Id, PadId), usize>,
    b: &BTreeMap<(Id, PadId), usize>,
) -> Option<[(Id, PadId); 2]> {
    let mut of: BTreeMap<usize, ((Id, PadId), Option<usize>)> = BTreeMap::new();
    for (pad, island) in b {
        let mine = a.get(pad).copied();
        match of.get(island) {
            None => {
                of.insert(*island, (*pad, mine));
            }
            Some((first, other)) if *other != mine => return Some([*first, *pad]),
            Some(_) => {}
        }
    }
    None
}

fn connectivity(seen: &Seen, out: &mut Vec<Broken>) {
    // Stale copper joins nothing, in the graph and here.
    let fresh: BTreeSet<Id> = seen.current().map(|f| f.id).collect();
    let mut pieces = truth::copper(&seen.project);
    pieces.extend(
        truth::fills(&seen.project)
            .into_iter()
            .filter(|c| c.fill.is_some_and(|id| fresh.contains(&id))),
    );
    let Some(graph) = &seen.islands else {
        if !seen
            .findings
            .iter()
            .any(|f| f.code == FindingCode::BoardCheckLimit)
        {
            out.push(broken(
                Rule::Connectivity,
                "a graph that stopped and no finding says so",
            ));
        }
        return;
    };
    let (wide, narrow) = (
        truth::joined(&pieces, Some(REACH)),
        truth::joined(&pieces, None),
    );
    for net in seen.project.nets.iter() {
        let name = &net.name;
        let of = |islands: &Islands| islands.get(&net.id).cloned().unwrap_or_default();
        let (graph, wide, narrow) = (of(graph), of(&wide), of(&narrow));
        // Every pad of the netlist is one the graph knows.
        let pads: BTreeSet<_> = narrow.keys().collect();
        if graph.keys().collect::<BTreeSet<_>>() != pads {
            out.push(broken(
                Rule::Connectivity,
                format!("{name}: the graph has other pads than the netlist"),
            ));
            continue;
        }
        if let Some([a, b]) = coarser(&wide, &graph) {
            out.push(broken(
                Rule::Connectivity,
                format!("{name}: the graph joins {a:?} and {b:?}, the copper does not"),
            ));
        }
        if let Some([a, b]) = coarser(&graph, &narrow) {
            out.push(broken(
                Rule::Connectivity,
                format!("{name}: copper joins {a:?} and {b:?}, the graph does not"),
            ));
        }
        // What the copper leaves open is said, by a finding and an airwire.
        let open = wide.values().collect::<BTreeSet<_>>().len() > 1;
        let object = ObjectRef::Net { id: net.id };
        if open && !seen.told(FindingCode::BoardUnrouted, object) {
            out.push(broken(
                Rule::Connectivity,
                format!("{name} is open and no finding says so"),
            ));
        }
        if open && !seen.airwires.contains(&net.id) {
            out.push(broken(
                Rule::Connectivity,
                format!("{name} is open and has no airwire"),
            ));
        }
    }
    // A part that is not on the board is said to be missing.
    for instance in seen.project.instances.iter() {
        let placed = seen
            .project
            .placements
            .iter()
            .any(|p| p.instance == instance.id);
        let object = ObjectRef::Instance { id: instance.id };
        if !placed && !seen.told(FindingCode::BoardUnplaced, object) {
            out.push(broken(
                Rule::Connectivity,
                format!(
                    "{} is not on the board and no finding says so",
                    instance.designator
                ),
            ));
        }
    }
}

fn shorts(seen: &Seen, out: &mut Vec<Broken>) {
    let mut pieces = truth::copper(&seen.project);
    pieces.extend(truth::fills(&seen.project));
    let tells = |f: &Finding, a: ObjectRef, b: ObjectRef| {
        if can_waive(f.code.as_str()) {
            return false;
        }
        let (a, b) = (f.objects.contains(&a), f.objects.contains(&b));
        match f.code {
            FindingCode::BoardCheckLimit => true,
            // Copper that is not the board's: nothing is made of it.
            FindingCode::FillStale => a || b,
            // More than a fill lists.
            code if code.as_str() == fill::CLEARANCE_UNLISTED => a || b,
            _ => a && b,
        }
    };
    let mut seen_pairs = BTreeSet::new();
    for (k, a) in pieces.iter().enumerate() {
        for b in &pieces[..k] {
            let (Some(m), Some(n)) = (a.net, b.net) else {
                continue;
            };
            let (Some(x), Some(y)) = (a.object, b.object) else {
                continue;
            };
            if m == n || !a.layers.iter().any(|l| b.on(*l)) || !a.shape.overlaps(&b.shape) {
                continue;
            }
            let pair = format!("{x:?} and {y:?}");
            if !seen.findings.iter().any(|f| tells(f, x, y)) && seen_pairs.insert(pair.clone()) {
                out.push(broken(
                    Rule::Short,
                    format!(
                        "{} and {} meet ({pair}) and no finding that cannot be waived says so",
                        seen.project.net_name(m),
                        seen.project.net_name(n)
                    ),
                ));
            }
        }
    }
}

/// Pads, tracks and vias of different nets, each pair measured again: nearer
/// than the rules allow is a finding that names both. The rules are the test
/// profile's number and the classes' own clearances, which two pads of one
/// part and the two halves of a differential pair are free of.
fn close(seen: &Seen, out: &mut Vec<Broken>) {
    let project = &seen.project;
    let pieces = truth::copper(project);
    let class = |net: Option<Id>| {
        let assigned = project
            .rules
            .assignments
            .iter()
            .find(|a| Some(a.net) == net)?;
        let class = assigned.class;
        let found = project.rules.net_classes.iter().find(|c| c.id == class)?;
        found.clearance
    };
    let part = |o: ObjectRef| match o {
        ObjectRef::Pad { instance, .. } => Some(instance),
        _ => None,
    };
    for (k, a) in pieces.iter().enumerate() {
        for b in &pieces[..k] {
            let (Some(x), Some(y)) = (a.object, b.object) else {
                continue;
            };
            if a.net.is_none() || b.net.is_none() || a.net == b.net {
                continue;
            }
            if !a.layers.iter().any(|l| b.on(*l)) {
                continue;
            }
            let mut need = COPPER;
            let halves = project.rules.diff_pairs.iter().any(|p| {
                let own = [Some(p.positive), Some(p.negative)];
                own.contains(&a.net) && own.contains(&b.net)
            });
            if !halves && (part(x).is_none() || part(x) != part(y)) {
                for net in [a.net, b.net] {
                    need = need.max(class(net).unwrap_or(0.0));
                }
            }
            let gap = a.shape.gap(&b.shape, need);
            // Copper of two tied nets that touches elsewhere is told by the
            // tie, in place of the short.
            let touching = gap <= REACH;
            let told = |f: &Finding| {
                f.code == FindingCode::BoardCheckLimit
                    || (matches!(
                        f.code,
                        FindingCode::BoardClearance | FindingCode::BoardOverlap
                    ) || touching && f.code == FindingCode::TieSecondJoin)
                        && f.objects.contains(&x)
                        && f.objects.contains(&y)
            };
            if gap < need - SLACK && !seen.findings.iter().any(told) {
                out.push(broken(
                    Rule::Close,
                    format!("{x:?} and {y:?} are {gap:.4} mm apart, {need:.4} needed"),
                ));
            }
        }
    }
}

/// Every error of the board's checks, and what is left to do on it, is an
/// item of the production check that blocks; and a board without a valid
/// outline gives no files.
fn blocks(seen: &Seen, out: &mut Vec<Broken>) -> Fallible {
    let report = preflight::report(
        &seen.project,
        &Default::default(),
        None,
        &Default::default(),
    )?;
    let blocking: BTreeSet<&str> = report
        .items
        .iter()
        .filter(|i| i.blocks())
        .map(|i| i.key.as_str())
        .collect();
    for f in &seen.findings {
        let must = f.code.severity() == Severity::Error
            || matches!(
                f.code,
                FindingCode::BoardUnrouted | FindingCode::BoardUnplaced | FindingCode::FillStale
            );
        let key = preflight::key(&f.code, &f.objects);
        if must && !blocking.contains(key.as_str()) {
            out.push(broken(
                Rule::Blocks,
                format!("{key} ({}) does not block", f.message),
            ));
        }
    }
    let invalid = seen.project.board_shape.invalid().is_some();
    if invalid && (report.ready() || !seen.refused) {
        out.push(broken(Rule::Outline, "files of a board that is none"));
    }
    Ok(())
}

fn stale(seen: &Seen, out: &mut Vec<Broken>) {
    for fill in seen.current() {
        let again = seen.made.fills.iter().find(|f| f.id == fill.id);
        let regions = |f: Option<&Fill>| {
            f.and_then(|f| f.computed.as_ref())
                .map(|c| c.regions.clone())
        };
        if regions(again) != regions(Some(fill)) {
            out.push(broken(
                Rule::Stale,
                format!(
                    "fill {} is not what the board gives and no finding says it is stale",
                    fill.id
                ),
            ));
        }
    }
}

fn removes(fill: &Fill) -> bool {
    fill.net.is_some() && fill.settings.islands != IslandRemoval::Never
}

fn regions(project: &Project, id: Id) -> Vec<FilledRegion> {
    project
        .fill(id)
        .and_then(|f| f.computed.as_ref())
        .map_or_else(Vec::new, |c| c.regions.clone())
}

/// Every island of a fill that removes what joins nothing reaches a pad, via
/// or track of its net, through its net's other fills if need be; and one
/// the fill has when it keeps every island, but not else, does not.
fn islands_stay(seen: &Seen, out: &mut Vec<Broken>) -> Fallible {
    let project = &seen.made;
    let mut pieces = truth::copper(project);
    pieces.extend(truth::fills(project));
    let groups = truth::grouped(&pieces, Some(REACH));
    let reached: BTreeSet<usize> = pieces
        .iter()
        .zip(&groups)
        .filter(|(p, _)| p.fill.is_none())
        .map(|(_, g)| *g)
        .collect();
    for fill in project.fills.iter().filter(|f| removes(f)) {
        let floats = pieces
            .iter()
            .zip(&groups)
            .any(|(p, g)| p.fill == Some(fill.id) && !reached.contains(g));
        if floats {
            out.push(broken(
                Rule::Island,
                format!("fill {} keeps an island that joins nothing", fill.id),
            ));
        }
        let mut keeps = project.clone();
        if let Some(f) = keeps.fills.iter_mut().find(|f| f.id == fill.id) {
            f.settings.islands = IslandRemoval::Never;
        }
        let keeps = refilled(&keeps)?;
        let others = |p: &Project| -> Vec<Vec<FilledRegion>> {
            p.fills
                .iter()
                .filter(|f| f.layer == fill.layer && f.net != fill.net && above(f, fill))
                .map(|f| regions(p, f.id))
                .collect()
        };
        // What it clears is other copper then: its islands say nothing.
        if others(project) != others(&keeps) {
            continue;
        }
        let kept = regions(project, fill.id);
        for region in regions(&keeps, fill.id) {
            if kept.contains(&region) {
                continue;
            }
            let shape = Shape::of_region(&region);
            let joins = pieces.iter().zip(&groups).find(|(p, g)| {
                p.net == fill.net
                    && p.on(fill.layer)
                    && reached.contains(g)
                    && p.fill != Some(fill.id)
                    && p.shape.overlaps(&shape)
                    // Not through a fill that another net's kept from it
                    // while it was judged (`layout::fill::groups`).
                    && !gave_up(project, fill, p.fill)
            });
            if let Some((p, _)) = joins {
                out.push(broken(
                    Rule::Island,
                    format!("fill {} lost an island that joins {:?}", fill.id, p.object),
                ));
            }
        }
    }
    Ok(())
}

/// Whether `a` is poured before `b`.
fn above(a: &Fill, b: &Fill) -> bool {
    a.priority > b.priority || a.priority == b.priority && a.id < b.id
}

/// Whether a fill of another net between `fill` and the lower `other` gave
/// up islands.
fn gave_up(project: &Project, fill: &Fill, other: Option<Id>) -> bool {
    let Some(other) = other.and_then(|id| project.fill(id)) else {
        return false;
    };
    project.fills.iter().any(|f| {
        f.layer == fill.layer
            && f.net != fill.net
            && above(fill, f)
            && above(f, other)
            && f.computed.as_ref().is_some_and(|c| c.removed_islands > 0)
    })
}

/// What the copper checks allow for chords (`proof`).
const SLACK: f64 = super::super::proof::SPACING_SLACK_MM;
/// The least clearances of the boards' profile (`rules/fixtures`), mm.
const COPPER: f64 = 0.2;
const HOLE: f64 = 0.25;
const EDGE: f64 = 0.3;

/// The clearance a fill keeps to `object` of `net`, read from the document's
/// own records: the overrides, the classes and the fill, and else the
/// profile's numbers above. The largest of what they ask is kept.
fn required(
    project: &Project,
    fill: &Fill,
    object: CopperObject,
    net: Option<Id>,
    own_gap: Option<f64>,
) -> f64 {
    let key = RuleKey::Clearance {
        a: CopperObject::Fill,
        b: object,
    }
    .normalized();
    let rule = project
        .rules
        .overrides
        .iter()
        .find(|o| o.key == key)
        .and_then(|o| match o.value {
            RuleValue::Length(mm) => Some(mm),
            _ => None,
        });
    let (least, copper) = match object {
        CopperObject::Track | CopperObject::Pad | CopperObject::Via | CopperObject::Fill => {
            (COPPER, true)
        }
        CopperObject::BoardEdge => (EDGE, false),
        _ => (HOLE, false),
    };
    let mut mm = rule.unwrap_or(least);
    let class = |net: Option<Id>| -> Option<&NetClass> {
        let id = project
            .rules
            .assignments
            .iter()
            .find(|a| Some(a.net) == net)?
            .class;
        project.rules.net_classes.iter().find(|c| c.id == id)
    };
    let own = |c: Option<&NetClass>| c.and_then(|c| c.fill_clearance.or(c.clearance));
    mm = mm.max(own(class(fill.net)).unwrap_or(0.0));
    if copper {
        mm = mm.max(own(class(net)).unwrap_or(0.0));
        mm = mm.max(own_gap.or(fill.settings.clearance).unwrap_or(COPPER));
    }
    mm
}

/// What a fill's copper is measured against.
struct Against {
    what: String,
    kind: CopperObject,
    net: Option<Id>,
    /// Copper of the fill's own net is its own; a bare hole is nobody's.
    own: bool,
    /// Further nets it is of: a tie's bridge, which no fill may meet.
    also: Vec<Option<Id>>,
    /// The gap fills keep to this pad in place of their own.
    gap: Option<f64>,
    shape: Shape,
    layers: Vec<CopperLayer>,
}

/// The clearance a pad's own override asks of fills, or its part's.
fn pad_gap(project: &Project, instance: Id, pad: PadId) -> Option<f64> {
    let placement = project.placements.iter().find(|p| p.instance == instance)?;
    let own = placement
        .pad_fill_overrides
        .iter()
        .find(|e| e.pad == pad)
        .and_then(|e| e.settings.clearance);
    own.or(placement.fill_override.clearance)
        .map(|c| c.to_mm_f64())
}

fn against(project: &Project) -> Fallible<Vec<Against>> {
    let stack = project.stackup.layers();
    let mut out: Vec<Against> = truth::copper(project)
        .into_iter()
        .map(|c| Against {
            what: format!("{:?}", c.object),
            kind: c.kind,
            net: c.net,
            own: c.net.is_some(),
            also: Vec::new(),
            gap: match c.object {
                Some(ObjectRef::Pad { instance, pad }) => pad_gap(project, instance, pad),
                _ => None,
            },
            shape: c.shape,
            layers: c.layers,
        })
        .collect();
    for bridge in truth::bridges(project) {
        out.push(Against {
            what: format!("the bridge of {}", bridge.instance),
            kind: CopperObject::Track,
            net: None,
            own: false,
            also: bridge.nets.into_iter().map(Some).collect(),
            gap: None,
            shape: bridge.shape,
            layers: vec![bridge.layer],
        });
    }
    for hole in truth::holes(project)? {
        out.push(Against {
            what: format!("the hole at {:?}", hole.from),
            kind: hole.kind,
            net: hole.net,
            own: hole.plated && hole.net.is_some(),
            also: Vec::new(),
            gap: None,
            shape: hole.shape(),
            layers: stack.clone(),
        });
    }
    Ok(out)
}

/// Every fill's copper (or that of `only` these) against everything it must
/// keep clear of, by distances between segments.
fn clearance(project: &Project, only: Option<&BTreeSet<Id>>, out: &mut Vec<Broken>) -> Fallible {
    let things = against(project)?;
    let board = project.board();
    let outer = Shape::of_outline(&board.outer);
    let cutouts: Vec<(Shape, bool)> = board
        .cutouts
        .iter()
        .map(|c| {
            let plated = c.kind == (CutoutKind::Slot { plated: true });
            (Shape::of_outline(&c.outline), plated)
        })
        .collect();
    let regions: Vec<Copper> = truth::fills(project);
    let measured = |f: &Fill| only.is_none_or(|ids| ids.contains(&f.id));
    for fill in project.fills.iter().filter(|f| measured(f)) {
        let mine: Vec<&Copper> = regions.iter().filter(|r| r.fill == Some(fill.id)).collect();
        let mut short = |what: &str, gap: f64, need: f64| {
            out.push(broken(
                Rule::Clearance,
                format!(
                    "fill {} of {:?} on {:?} is {gap:.4} mm from {what}, {need:.4} needed",
                    fill.id, fill.net, fill.layer
                ),
            ));
        };
        let outline = Shape::of_outline(&fill.outline);
        for region in &mine {
            for t in &things {
                let same = t.own && fill.net.is_some() && t.net == fill.net;
                if same || !t.layers.contains(&fill.layer) {
                    continue;
                }
                let need = std::iter::once(&t.net)
                    .chain(&t.also)
                    .map(|net| required(project, fill, t.kind, *net, t.gap))
                    .fold(0.0, f64::max);
                let gap = region.shape.gap(&t.shape, need);
                if gap < need - SLACK {
                    short(&t.what, gap, need);
                }
            }
            let edge = required(project, fill, CopperObject::BoardEdge, None, None);
            let first = region.shape.rings.first().and_then(|r| r.first());
            if first.is_some_and(|p| !outer.inside(*p)) {
                short("the board, which it is outside of", 0.0, edge);
            }
            let gap = region.shape.edges_apart(&outer, edge);
            if gap < edge - SLACK {
                short("the board's edge", gap, edge);
            }
            for (cut, plated) in &cutouts {
                let need = match plated {
                    true => edge.max(required(project, fill, CopperObject::PthHole, None, None)),
                    false => edge,
                };
                let gap = region.shape.gap(cut, need);
                if gap < need - SLACK {
                    short("a cutout", gap, need);
                }
            }
            for keepout in project.keepouts.iter() {
                if !keepout.forbid.fills || !keepout.on(fill.layer) {
                    continue;
                }
                let sunk = region.shape.sunk(&Shape::of_outline(&keepout.outline));
                if sunk > SLACK {
                    short("the inside of a keepout", -sunk, 0.0);
                }
            }
            // Nowhere past what was drawn.
            let past = region
                .shape
                .rings
                .iter()
                .flatten()
                .filter(|p| !outline.inside(**p))
                .map(|p| outline.edge(*p))
                .fold(0.0, f64::max);
            if past > SLACK {
                short("its own outline, which it is outside of,", -past, 0.0);
            }
            // Fills of other nets on the layer, each pair once.
            for other in &regions {
                let later = other.fill.is_some_and(|id| id > fill.id);
                let foreign = fill.net.is_none() || other.net != fill.net;
                let Some(them) = project.fills.iter().find(|f| other.fill == Some(f.id)) else {
                    continue;
                };
                if !later || !foreign || them.layer != fill.layer || !measured(them) {
                    continue;
                }
                // The one that gives way keeps its clearance: the lower
                // priority, and of two alike the later.
                let need = if them.priority <= fill.priority {
                    required(project, them, CopperObject::Fill, fill.net, None)
                } else {
                    required(project, fill, CopperObject::Fill, them.net, None)
                };
                let gap = region.shape.gap(&other.shape, need);
                if gap < need - SLACK {
                    short(&format!("fill {}", them.id), gap, need);
                }
            }
        }
    }
    Ok(())
}

fn read_back(seen: &Seen, out: &mut Vec<Broken>) -> Fallible {
    let snapshot = ProductionSnapshot::resolve(&seen.made, None)?;
    let options = options();

    let found = readback::check(
        &readback::Set {
            snapshot: &snapshot,
            options: &options,
            identity: &seen.identity,
            drawing: None,
        },
        &seen.files,
        &mut |_| true,
    )?;
    out.extend(found.iter().map(|m| broken(Rule::ReadBack, m.to_string())));
    Ok(())
}

pub(super) fn text<'a>(files: &'a [(String, Vec<u8>)], name: &str) -> Fallible<Option<&'a str>> {
    let name = format!("random-{name}");
    Ok(match files.iter().find(|f| f.0 == name) {
        Some((_, bytes)) => Some(std::str::from_utf8(bytes)?),
        None => None,
    })
}

const SIDES: [(Layer, &str); 2] = [(Layer::Top, "top"), (Layer::Bottom, "bottom")];
const DRILLS: [(bool, &str); 2] = [(true, "plated.drl"), (false, "non-plated.drl")];

/// The files drawn by `fabcheck`'s rasteriser against the design painted
/// pixel by pixel from [`truth`].
fn image(seen: &Seen, pixel: f64, out: &mut Vec<Broken>) -> Fallible {
    let project = &seen.made;
    // Off every grid the design is drawn on: no pixel's middle is on an
    // edge.
    let grid = super::super::expected::board_bitmap(project, 1.003_17, pixel)?;
    let mut compare = |name: &str, got: Bitmap, want: Bitmap| -> Fallible {
        let c = raster::compare(&got, &want)?;
        let edge = super::super::compare::perimeter(&want);
        if let Some(at) = super::super::compare::thick(&got, &want) {
            out.push(broken(Rule::Image, format!("{name} differs round {at:?}")));
        } else if c.xor_area > super::super::compare::allowed(&c, edge) {
            out.push(broken(
                Rule::Image,
                format!("{name} differs by {:.5} mm²", c.xor_area),
            ));
        }
        Ok(())
    };
    let drawn = |name: &str| -> Fallible<Bitmap> {
        let mut b = grid.blank();
        if let Some(text) = text(&seen.files, name)? {
            raster::render_gerber(&gerber::parse(text)?, &mut b)?;
        }
        Ok(b)
    };
    let mut pieces = truth::copper(project);
    pieces.extend(truth::fills(project));
    let bridges = truth::bridges(project);
    for layer in project.stackup.layers() {
        let mut want = grid.blank();
        for bridge in bridges.iter().filter(|b| b.layer == layer) {
            bridge.shape.paint(0.0, &mut want);
        }
        for piece in pieces.iter().filter(|c| c.on(layer)) {
            piece.shape.paint(0.0, &mut want);
        }
        let name = copper_name(project, layer)?;
        compare(&name, drawn(&name)?, want)?;
    }
    for (side, word) in SIDES {
        for (name, shapes) in [
            (format!("mask-{word}.gbr"), truth::mask(project, side)),
            (format!("paste-{word}.gbr"), truth::paste(project, side)?),
        ] {
            let mut want = grid.blank();
            for (shape, by) in &shapes {
                shape.paint(*by, &mut want);
            }
            compare(&name, drawn(&name)?, want)?;
        }
    }
    let holes = truth::holes(project)?;
    for (plated, name) in DRILLS {
        let mut got = grid.blank();
        if let Some(text) = text(&seen.files, name)? {
            raster::render_drill(&excellon::parse(text)?, &mut got)?;
        }
        let mut want = grid.blank();
        for hole in holes.iter().filter(|h| h.plated == plated) {
            hole.shape().paint(0.0, &mut want);
        }
        compare(name, got, want)?;
    }
    Ok(())
}

fn copper_name(project: &Project, layer: CopperLayer) -> Fallible<String> {
    Ok(super::super::proof::layer_name(project, layer)?.0)
}

/// How far a place may be rounded on its way through a file, mm.
const ROUNDED: f64 = 0.0008;

fn at(a: Point, b: Point) -> bool {
    (a[0] - b[0]).hypot(a[1] - b[1]) <= ROUNDED
}

/// From the design through the files and `fabcheck`'s readers, back to a
/// list of what the design has: every hole, via and track once, at its
/// place, of its size and net, every pad by its part and number on every
/// layer it is on, and nothing besides.
fn inventory(seen: &Seen, out: &mut Vec<Broken>) -> Fallible {
    let project = &seen.made;
    let mut say = |text: String| out.push(broken(Rule::Inventory, text));
    let holes = truth::holes(project)?;
    for (plated, name) in DRILLS {
        let written = match text(&seen.files, name)? {
            Some(text) => drills::hits(&excellon::parse(text)?, |_| true)?,
            None => Vec::new(),
        };
        let design: Vec<drills::DrillHit> = holes
            .iter()
            .filter(|h| h.plated == plated)
            .map(|h| drills::DrillHit {
                feature: match at(h.from, h.to) {
                    true => drills::Feature::Round { at: h.from },
                    false => drills::Feature::Slot {
                        from: h.from,
                        to: h.to,
                    },
                },
                diameter: h.width,
            })
            .collect();
        let diff = drills::pair(&written, &design, ROUNDED);
        if !diff.passes() {
            say(format!(
                "{name}: not in the design {:?}, not drilled {:?}",
                diff.unmatched_a, diff.unmatched_b
            ));
        }
    }
    let pads = truth::pads(project);
    for layer in project.stackup.layers() {
        let name = copper_name(project, layer)?;
        let Some(text) = text(&seen.files, &name)? else {
            say(format!("no {name}"));
            continue;
        };
        let file = gerber::parse(text)?;
        let net = |o: &gerber::Object| o.object_attributes.get(".N").map(|v| v.join(","));
        let width = |o: &gerber::Object| match o.aperture.and_then(|a| file.apertures.get(&a)) {
            Some(gerber::Aperture {
                template: gerber::Template::Circle { diameter, .. },
                ..
            }) => Some(*diameter),
            _ => None,
        };
        let function = |o: &gerber::Object| {
            o.aperture_attributes
                .get(".AperFunction")
                .and_then(|v| v.first())
                .cloned()
                .unwrap_or_default()
        };
        // Vias and tracks: what is written, each crossed off the design's.
        let mut vias: Vec<_> = project.vias.iter().collect();
        let mut tracks: Vec<_> = project.tracks.iter().filter(|t| t.layer == layer).collect();
        for o in &file.objects {
            let named = |id: Id| net(o).as_deref() == Some(project.net_name(id));
            let near = |w: f64| width(o).is_some_and(|x| (x - w).abs() <= ROUNDED);
            match o.kind {
                gerber::Kind::Flash { at: p } if function(o) == "ViaPad" => {
                    match vias
                        .iter()
                        .position(|v| at(v.position, p) && near(v.diameter) && named(v.net))
                    {
                        Some(k) => {
                            vias.remove(k);
                        }
                        None => say(format!("{name}: a via at {p:?} the design has not")),
                    }
                }
                gerber::Kind::Draw { from, to } if function(o) == "Conductor" => {
                    match tracks.iter().position(|t| {
                        (at(t.start, from) && at(t.end, to) || at(t.start, to) && at(t.end, from))
                            && near(t.width)
                            && named(t.net)
                    }) {
                        Some(k) => {
                            tracks.remove(k);
                        }
                        None => say(format!(
                            "{name}: a track {from:?} to {to:?} the design has not"
                        )),
                    }
                }
                _ => {}
            }
        }
        for v in vias {
            say(format!("{name}: no via at {:?}", v.position));
        }
        for t in tracks {
            say(format!("{name}: no track {:?} to {:?}", t.start, t.end));
        }
        // Pads by part and number, each with its net.
        let written: BTreeSet<(String, String)> = file
            .objects
            .iter()
            .filter(|o| o.object_attributes.contains_key(".C"))
            .filter_map(|o| {
                let p = o.object_attributes.get(".P")?;
                Some((p.join(","), net(o).unwrap_or_default()))
            })
            .collect();
        let design: BTreeSet<(String, String)> = pads
            .iter()
            .filter(|p| p.on(layer) && matches!(p.pad.kind, PadKind::Smd | PadKind::PlatedThrough))
            .filter_map(|p| {
                Some((
                    format!("{},{}", p.part, p.pad.number),
                    project.net_name(p.net?).to_owned(),
                ))
            })
            .collect();
        let missing: Vec<_> = design.difference(&written).collect();
        if !missing.is_empty() {
            say(format!("{name}: pads that are not there: {missing:?}"));
        }
        let named: Vec<_> = written
            .difference(&design)
            .filter(|(_, net)| project.nets.iter().any(|n| n.name == *net))
            .collect();
        if !named.is_empty() {
            say(format!("{name}: pads on nets they are not on: {named:?}"));
        }
        let bridged = file
            .objects
            .iter()
            .filter(|o| function(o) == "EtchedComponent")
            .count();
        let bridges = truth::bridges(project)
            .iter()
            .filter(|b| b.layer == layer)
            .count();
        if bridged != bridges {
            say(format!(
                "{name}: {bridged} bridges, {bridges} in the design"
            ));
        }
        let drawn = file
            .objects
            .iter()
            .filter(|o| o.object_attributes.contains_key(".C"))
            .filter(|o| function(o) != "EtchedComponent")
            .count();
        let wanted = pads.iter().filter(|p| p.on(layer)).count();
        if drawn != wanted {
            say(format!("{name}: {drawn} pads, {wanted} in the design"));
        }
    }
    Ok(())
}

/// Two states of the document that must be one, but for the counters of the
/// board's interface, which never go back (a slot handed out once is never
/// handed out again).
pub(super) fn same(a: &Document, b: &Document) -> bool {
    let mut project = (*a.project).clone();
    project.interface = b.project.interface.clone();
    project == *b.project && a.drafts == b.drafts
}

/// What a person sees of an engine's state: what a refused edit keeps.
#[derive(Clone, PartialEq)]
pub(super) struct State {
    document: Document,
    revision: u64,
    undo: bool,
    redo: bool,
}

impl State {
    pub(super) fn of(engine: &Engine) -> Self {
        Self {
            document: engine.document().clone(),
            revision: engine.revision(),
            undo: engine.can_undo(),
            redo: engine.can_redo(),
        }
    }

    pub(super) fn document(&self) -> &Document {
        &self.document
    }

    pub(super) fn revision(&self) -> u64 {
        self.revision
    }
}

/// [`Rule::Refusal`]: the state after a refused edit is the one before.
pub(super) fn refused(before: &State, after: &State) -> Option<Broken> {
    (before != after).then(|| broken(Rule::Refusal, "a refused edit changed something"))
}

/// [`Rule::History`]: undo gave `back` from the state after `before`, and
/// redo gave `again`.
pub(super) fn history(
    before: &Document,
    after: &Document,
    back: &Document,
    again: &Document,
) -> Option<Broken> {
    if !same(back, before) {
        return Some(broken(Rule::History, "undo is not the state before"));
    }
    (!same(again, after)).then(|| broken(Rule::History, "redo is not the state after"))
}

/// [`Rule::Reopen`]: `bytes`, the saved board, opened and filled again.
pub(super) fn reopened(project: &Seen, engine: &Engine, bytes: &[u8]) -> Fallible<Vec<Broken>> {
    let mut out = Vec::new();
    let file = format::decode(bytes)?;
    let saved = format::as_saved(engine.to_file());
    if file != saved {
        out.push(broken(Rule::Reopen, "the opened board is another"));
    }
    if format::encode(&file)? != bytes {
        out.push(broken(Rule::Reopen, "saved again, the file is another"));
    }
    let mut opened = Engine::from_file(file)?;
    opened.refill_stale()?;
    for fill in project.made.fills.iter() {
        let again = opened.project().fill(fill.id);
        let regions = |f: &Fill| f.computed.as_ref().map(|c| c.regions.clone());
        if again.and_then(regions) != regions(fill) {
            out.push(broken(
                Rule::Reopen,
                format!("fill {} fills otherwise once it is opened", fill.id),
            ));
        }
    }
    Ok(out)
}

/// What an engine keeps between revisions.
pub(super) struct Kept {
    pub islands: Option<Islands>,
    pub issues: Arc<IssueReport>,
}

impl Kept {
    pub(super) fn of(engine: &Engine) -> Fallible<Self> {
        let graph = engine.copper_graph();
        Ok(Self {
            islands: graph.complete().then(|| islands(&graph)),
            issues: engine.issue_report()?,
        })
    }
}

/// [`Rule::Cache`]: what the engine keeps between revisions against the same
/// made afresh, in the engine's own context, which a report reads.
pub(super) fn cached(kept: &Kept, engine: &Engine, seen: &Seen) -> Fallible<Vec<Broken>> {
    let mut out = Vec::new();
    if kept.islands != seen.islands {
        out.push(broken(Rule::Cache, "the engine's copper graph is old"));
    }
    let _context = engine.enter_context();
    let fresh = issues::report(
        engine.project(),
        engine.drafts(),
        &IssueScope::All,
        engine.check_settings(),
    )?;
    if *kept.issues != fresh {
        let (old, new) = (&kept.issues.rows, &fresh.rows);
        let at = old.iter().zip(new).position(|(a, b)| a != b);
        let at = at.unwrap_or(old.len().min(new.len()));
        out.push(broken(
            Rule::Cache,
            format!(
                "the engine's issues are old: row {at} of {} is {:?}, made afresh (of {}) {:?}",
                old.len(),
                old.get(at),
                new.len(),
                new.get(at)
            ),
        ));
    }
    Ok(out)
}

/// [`Rule::Silent`]: an edit that is no deletion left a net fewer vias than
/// it had, and neither its report of what it re-routed nor a finding of an
/// open net names it; or it changed the netlist.
pub(super) fn lost(
    before: &Project,
    after: &Project,
    rerouted: &Rerouted,
    findings: &[Finding],
) -> Vec<Broken> {
    let mut out = Vec::new();
    for net in before.nets.iter() {
        match after.net(net.id) {
            Some(n) if n == net => {}
            _ => out.push(broken(
                Rule::Silent,
                format!("{}: the netlist changed", net.name),
            )),
        }
        let vias = |p: &Project| p.vias.iter().filter(|v| v.net == net.id).count();
        let said = rerouted
            .rerouted
            .iter()
            .chain(&rerouted.unrouted)
            .any(|n| *n == net.name)
            || findings
                .iter()
                .filter(|f| f.code == FindingCode::BoardUnrouted)
                .any(|f| f.objects.contains(&ObjectRef::Net { id: net.id }));
        if vias(after) < vias(before) && !said {
            out.push(broken(
                Rule::Silent,
                format!(
                    "{}: {} of {} vias are left and nothing says so",
                    net.name,
                    vias(after),
                    vias(before)
                ),
            ));
        }
    }
    out
}
