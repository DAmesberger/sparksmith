//! A case's edits as commands of the engine, made of what the board holds
//! when they are made: the choices come from the document's own order.
use super::Fallible;
use super::build::{self, net};
use super::outline;
use super::plan::{BOARD, Edit, Past, Pour};
use super::truth::{self, PadOn};
use crate::geometry::Point;
use crate::ids::Id;
use crate::layout::arrange::PartMove;
use crate::layout::fill::{FillCommand, RefillTarget};
use crate::layout::pair::{PairLeg, PairRouteRequest};
use crate::layout::posture::Posture;
use crate::layout::router::drag::TrackDragRequest;
use crate::layout::router::{RouteMode, RouteStepRequest};
use crate::layout::several::Objects;
use crate::layout::{LayoutCommand, RouteEditSet, RouteRequest, ViaRequest};
use crate::library::{Outline, Segment};
use crate::model::fill::{
    FillRegion, FillSettings, FillSpec, Hatch, IslandRemoval, KeepoutLayers, KeepoutRules,
    KeepoutSpec,
};
use crate::model::{CopperLayer, Layer, Placement, Project, Track, Via, ViaFill, ViaSpan};
use crate::rules::PadConnection;
use crate::schematic::{ClipObject, SchematicCommand};
use crate::{Engine, Operation, Query, QueryResult};

/// The width a route is drawn at.
const WIDTH: f64 = 0.25;
/// How far a paste is from what was copied, mm: past every other symbol.
const APART: Point = [0.0, -200.0];

fn pick<T>(of: &[T], k: usize) -> Option<&T> {
    of.get(k.checked_rem(of.len())?)
}

fn layer(project: &Project, k: usize) -> CopperLayer {
    pick(&project.stackup.layers(), k)
        .copied()
        .unwrap_or(CopperLayer::Top)
}

fn pads_of(project: &Project, net: Id) -> Vec<PadOn> {
    truth::pads(project)
        .into_iter()
        .filter(|p| p.net == Some(net))
        .collect()
}

/// A layer a route from `pad` can start on: the wanted one, unless the pad
/// is on one side only.
fn from_pad(pad: &PadOn, wanted: CopperLayer) -> CopperLayer {
    match (pad.on(wanted), pad.side) {
        (true, _) => wanted,
        (false, Layer::Top) => CopperLayer::Top,
        (false, Layer::Bottom) => CopperLayer::Bottom,
    }
}

fn mode(past: Past) -> RouteMode {
    match past {
        Past::Shove => RouteMode::Shove,
        Past::Round => RouteMode::Walkaround,
        Past::Straight => RouteMode::Ignore,
    }
}

fn step(net: Id, layer: CopperLayer, head: Point, to: Point, past: Past) -> RouteStepRequest {
    RouteStepRequest {
        net,
        width: WIDTH,
        layer,
        head,
        to,
        posture: Posture::Diagonal,
        flipped: false,
        via: None,
        via_layer: None,
        mode: mode(past),
        move_vias: true,
        max_detour: None,
    }
}

fn via(net: Id, at: Point, drill: f64, tented: bool, capped: bool) -> ViaRequest {
    ViaRequest {
        net,
        position: at,
        drill,
        diameter: drill + 0.3,
        span: ViaSpan::Through,
        tented,
        fill: match capped {
            true => ViaFill::Capped,
            false => ViaFill::None,
        },
    }
}

fn boxed(from: Point, to: Point) -> Outline {
    Outline {
        start: from,
        segments: [[to[0], from[1]], to, [from[0], to[1]], from]
            .into_iter()
            .map(|to| Segment::Line { to })
            .collect(),
    }
}

/// A box whose upper right corner is a round of 1 mm.
fn drawn(from: Point, to: Point) -> Outline {
    Outline {
        start: from,
        segments: vec![
            Segment::Line {
                to: [to[0], from[1]],
            },
            Segment::Line {
                to: [to[0], to[1] - 1.0],
            },
            Segment::Arc {
                to: [to[0] - 1.0, to[1]],
                bulge: (std::f64::consts::PI / 8.0).tan(),
            },
            Segment::Line {
                to: [from[0], to[1]],
            },
            Segment::Line { to: from },
        ],
    }
}

fn pour(project: &Project, p: &Pour) -> Option<FillSpec> {
    let net = match p.free {
        true => None,
        false => Some(net(project, p.net)?),
    };
    let mut settings = FillSettings {
        clearance: p.clearance,
        hatch: p.hatched.then_some(Hatch {
            width: 0.4,
            gap: 0.6,
            angle_deg: 30.0,
        }),
        islands: match p.islands {
            true => IslandRemoval::Never,
            false => IslandRemoval::Unconnected,
        },
        ..FillSettings::default()
    };
    if p.solid {
        settings.connections.smd = Some(PadConnection::Solid);
        settings.connections.via = Some(PadConnection::Solid);
    }
    Some(FillSpec {
        name: String::new(),
        net,
        layer: layer(project, p.layer),
        region: match p.plane {
            true => FillRegion::Board,
            false => FillRegion::Drawn,
        },
        outline: match p.plane {
            // Derived: the engine draws it.
            true => Outline {
                start: [0.0, 0.0],
                segments: Vec::new(),
            },
            false => drawn(p.from, p.to),
        },
        priority: p.priority,
        settings,
    })
}

/// What an edit is now; `None` when the board has nothing it could be made
/// of. Undo, redo and reopening are the runner's.
pub(super) fn operation(engine: &Engine, edit: &Edit) -> Fallible<Option<Operation>> {
    let project = engine.project();
    if let Edit::Paste { pick: k } = edit {
        return pasted(engine, *k);
    }
    Ok(match edit {
        Edit::Rule { object, mm } => Some(build::rule(*object, *mm)),
        Edit::Pour(p) => pour(project, p).map(|fill| {
            Operation::Fill(FillCommand::Add {
                fill: Box::new(fill),
            })
        }),
        Edit::Refill
        | Edit::Repour { .. }
        | Edit::DeletePour { .. }
        | Edit::Keepout { .. }
        | Edit::DeleteKeepout { .. }
        | Edit::PadFill { .. } => fills(project, edit).map(Operation::Fill),
        Edit::Stretch { .. } | Edit::Window { .. } | Edit::Open => {
            outline::operation(project, edit)
        }
        _ => parts(project, edit)
            .or_else(|| routes(project, edit))
            .or_else(|| copper(project, edit))
            .map(Operation::Layout),
    })
}

fn pasted(engine: &Engine, k: usize) -> Fallible<Option<Operation>> {
    let project = engine.project();
    let Some(id) = pick(&project.instances, k).map(|i| i.id) else {
        return Ok(None);
    };
    match engine.query(Query::SheetCopy {
        sheet: None,
        objects: vec![ClipObject::Instance { id }],
        portable: false,
    })? {
        QueryResult::SheetClip(clip) => Ok(Some(Operation::Schematic(SchematicCommand::Paste {
            sheet: None,
            clip,
            offset: crate::tests::support::pt(APART),
        }))),
        other => Err(format!("unexpected {other:?}").into()),
    }
}

fn fills(project: &Project, edit: &Edit) -> Option<FillCommand> {
    Some(match edit {
        Edit::Refill => FillCommand::Refill {
            target: RefillTarget::All,
        },
        Edit::Repour {
            pick: k,
            priority,
            clearance,
        } => {
            let mut fill = pick(&project.fills, *k)?.clone();
            fill.priority = *priority;
            fill.settings.clearance = *clearance;
            FillCommand::Edit {
                fill: Box::new(fill),
            }
        }
        Edit::DeletePour { pick: k } => FillCommand::Delete {
            id: pick(&project.fills, *k)?.id,
        },
        Edit::Keepout { from, to, layer: k } => FillCommand::AddKeepout {
            keepout: Box::new(KeepoutSpec {
                name: String::new(),
                outline: boxed(*from, *to),
                layers: match k {
                    Some(k) => KeepoutLayers::Only(vec![layer(project, *k)]),
                    None => KeepoutLayers::All,
                },
                forbid: KeepoutRules::default(),
            }),
        },
        Edit::PadFill {
            pick: k,
            whole,
            settings,
        } => {
            let pads = truth::pads(project);
            let pad = pick(&pads, *k)?;
            FillCommand::SetPadOverride {
                instance: pad.instance,
                pad: (!whole).then_some(pad.pad.id),
                settings: *settings,
            }
        }
        Edit::DeleteKeepout { pick: k } => FillCommand::DeleteKeepout {
            id: pick(&project.keepouts, *k)?.id,
        },
        _ => return None,
    })
}

fn parts(project: &Project, edit: &Edit) -> Option<LayoutCommand> {
    let placed = |k: usize| -> Option<&Placement> { pick(&project.placements, k) };
    let moved = |p: &Placement, by: Point, turn: f64| PartMove {
        instance: p.instance,
        position: [p.position[0] + by[0], p.position[1] + by[1]],
        rotation_deg: p.rotation_deg + turn,
        layer: p.layer,
    };
    Some(match edit {
        Edit::Move { pick: k, by, turn } => {
            let m = moved(placed(*k)?, *by, *turn);
            LayoutCommand::Move {
                instance: m.instance,
                position: m.position,
                rotation_deg: m.rotation_deg,
                layer: m.layer,
            }
        }
        Edit::Flip { pick: k } => {
            let p = placed(*k)?;
            LayoutCommand::Move {
                instance: p.instance,
                position: p.position,
                rotation_deg: p.rotation_deg,
                layer: match p.layer {
                    Layer::Top => Layer::Bottom,
                    Layer::Bottom => Layer::Top,
                },
            }
        }
        Edit::MoveTwo { pick: k, by } => {
            let (a, b) = (placed(*k)?, placed(*k + 1)?);
            if a.instance == b.instance {
                return None;
            }
            LayoutCommand::MoveParts {
                moves: vec![moved(a, *by, 0.0), moved(b, *by, 0.0)],
                copper: None,
            }
        }
        Edit::Place {
            pick: k,
            to,
            turn,
            side,
        } => {
            let waiting: Vec<Id> = project
                .instances
                .iter()
                .map(|i| i.id)
                .filter(|id| !project.placements.iter().any(|p| p.instance == *id))
                .collect();
            LayoutCommand::Place {
                instance: *pick(&waiting, *k)?,
                position: *to,
                rotation_deg: *turn,
                layer: *side,
            }
        }
        _ => return None,
    })
}

fn routes(project: &Project, edit: &Edit) -> Option<LayoutCommand> {
    Some(match edit {
        Edit::Link {
            net: n,
            a,
            b,
            layer: wanted,
            past,
        } => {
            let net = net(project, *n)?;
            let pads = pads_of(project, net);
            let (from, to) = (pick(&pads, *a)?, pick(&pads, *b)?);
            if from.object() == to.object() {
                return None;
            }
            let on = from_pad(from, layer(project, *wanted));
            LayoutCommand::RouteStep {
                request: step(net, on, from.at, to.at, *past),
            }
        }
        Edit::FromVia {
            net: n,
            via: v,
            pad,
            layer: wanted,
            past,
        } => {
            let net = net(project, *n)?;
            let vias: Vec<&Via> = project.vias.iter().filter(|v| v.net == net).collect();
            let pads = pads_of(project, net);
            let (from, to) = (pick(&vias, *v)?, pick(&pads, *pad)?);
            let on = from_pad(to, layer(project, *wanted));
            LayoutCommand::RouteStep {
                request: step(net, on, from.position, to.at, *past),
            }
        }
        Edit::Change {
            net: n,
            pad,
            at,
            to,
            layer: wanted,
        } => {
            let net = net(project, *n)?;
            let pads = pads_of(project, net);
            let from = pick(&pads, *pad)?;
            let wanted = layer(project, *wanted);
            let on = from_pad(from, CopperLayer::Top);
            if on == wanted {
                return None;
            }
            LayoutCommand::RouteLayerChange {
                request: crate::layout::LayerChangeRequest {
                    request: RouteRequest {
                        net,
                        start: from.at,
                        end: *at,
                        width: WIDTH,
                        layer: on,
                    },
                    via: via(net, *at, 0.3, true, false),
                    to_layer: wanted,
                    end: *to,
                },
            }
        }
        Edit::Coupled {
            pads: [a, b],
            at,
            to,
            layer: k,
        } => {
            let pair = project.rules.diff_pairs.first()?;
            let (p, n) = (
                pads_of(project, pair.positive),
                pads_of(project, pair.negative),
            );
            LayoutCommand::RoutePair {
                request: PairRouteRequest {
                    pair: pair.id,
                    start: [pick(&p, *a)?.at, pick(&n, *b)?.at],
                    legs: vec![PairLeg {
                        layer: layer(project, *k),
                        points: vec![*at, *to],
                    }],
                    end: None,
                    width: Some(0.2),
                    gap: Some(0.2),
                },
            }
        }
        Edit::Straight {
            net: n,
            layer: k,
            from,
            to,
            width,
        } => LayoutCommand::Route {
            request: RouteRequest {
                net: net(project, *n)?,
                start: *from,
                end: *to,
                width: *width,
                layer: layer(project, *k),
            },
        },
        Edit::Arc {
            net: n,
            layer: k,
            centre,
            radius,
            from,
            sweep,
        } => {
            let (net, layer) = (net(project, *n)?, layer(project, *k));
            let chords = 8;
            let at = |i: usize| {
                let t = (from + sweep * i as f64 / chords as f64).to_radians();
                // Whole nanometres, as a file holds them.
                [centre[0] + radius * t.cos(), centre[1] + radius * t.sin()]
                    .map(|v| (v * 1e6).round() / 1e6)
            };
            LayoutCommand::ApplyRoute {
                edits: RouteEditSet {
                    add: (0..chords)
                        .map(|i| RouteRequest {
                            net,
                            start: at(i),
                            end: at(i + 1),
                            width: WIDTH,
                            layer,
                        })
                        .collect(),
                    ..RouteEditSet::default()
                },
            }
        }
        Edit::Via {
            net: n,
            at,
            drill,
            tented,
            capped,
        } => LayoutCommand::AddVia {
            via: via(net(project, *n)?, *at, *drill, *tented, *capped),
        },
        Edit::Drag { pick: k, by, past } => LayoutCommand::TrackDrag {
            request: TrackDragRequest {
                track: pick(&project.tracks, *k)?.id,
                offset: *by,
                posture: Posture::Diagonal,
                mode: mode(*past),
                move_vias: true,
                pair: Default::default(),
            },
        },
        _ => return None,
    })
}

fn copper(project: &Project, edit: &Edit) -> Option<LayoutCommand> {
    let via = |k: usize| -> Option<&Via> { pick(&project.vias, k) };
    let track = |k: usize| -> Option<&Track> { pick(&project.tracks, k) };
    Some(match edit {
        Edit::MoveVia { pick: k, by } => {
            let v = via(*k)?;
            LayoutCommand::MoveVia {
                id: v.id,
                position: [v.position[0] + by[0], v.position[1] + by[1]],
            }
        }
        Edit::DeleteVia { pick: k } => LayoutCommand::DeleteVia { id: via(*k)?.id },
        Edit::DeleteTrack { pick: k } => LayoutCommand::DeleteTrack { id: track(*k)?.id },
        Edit::DeleteSeveral { pick: k, count } => {
            let mut tracks: Vec<Id> = (0..*count)
                .filter_map(|i| track(*k + i).map(|t| t.id))
                .collect();
            tracks.sort_unstable();
            tracks.dedup();
            if tracks.is_empty() {
                return None;
            }
            LayoutCommand::DeleteObjects {
                objects: Objects {
                    tracks,
                    ..Objects::default()
                },
            }
        }
        Edit::Width { pick: k, width } => LayoutCommand::SetTrack {
            id: track(*k)?.id,
            width: Some(*width),
            layer: None,
        },
        Edit::Cap { pick: k, capped } => LayoutCommand::SetVias {
            ids: vec![via(*k)?.id],
            fill: match capped {
                true => ViaFill::Capped,
                false => ViaFill::None,
            },
        },
        Edit::Tent { pick: k, tented } => LayoutCommand::SetViaTented {
            id: via(*k)?.id,
            tented: *tented,
        },
        Edit::LockVia { pick: k, locked } => LayoutCommand::SetViaLock {
            id: via(*k)?.id,
            locked: *locked,
        },
        Edit::LockTrack { pick: k, locked } => LayoutCommand::SetTrackLock {
            id: track(*k)?.id,
            locked: *locked,
        },
        Edit::Wrong(k) => wrong(project, *k),
        _ => return None,
    })
}

/// A command no board takes.
fn wrong(project: &Project, k: usize) -> LayoutCommand {
    let missing = project.next_id + 1000;
    match k {
        0 => LayoutCommand::DeleteTrack { id: missing },
        1 => LayoutCommand::MoveVia {
            id: missing,
            position: [5.0, 5.0],
        },
        2 => LayoutCommand::Route {
            request: RouteRequest {
                net: Id::MAX,
                start: [5.0, 5.0],
                end: [9.0, 5.0],
                width: WIDTH,
                layer: CopperLayer::Top,
            },
        },
        3 => LayoutCommand::AddVia {
            via: via(
                project.nets.first().map_or(Id::MAX, |n| n.id),
                [BOARD[0] + 50.0, 5.0],
                0.3,
                true,
                false,
            ),
        },
        // Some of an edit set that is right, then one that is not.
        4 => LayoutCommand::ApplyRoute {
            edits: RouteEditSet {
                remove: project
                    .tracks
                    .first()
                    .map(|t| t.id)
                    .into_iter()
                    .chain([missing])
                    .collect(),
                ..RouteEditSet::default()
            },
        },
        _ => LayoutCommand::SetBoardSize { size: [1.0, 1.0] },
    }
}

/// Whether an edit takes copper away because that is what it is for.
pub(super) fn deletes(edit: &Edit) -> bool {
    matches!(
        edit,
        Edit::DeleteVia { .. }
            | Edit::DeleteTrack { .. }
            | Edit::DeleteSeveral { .. }
            | Edit::Undo { .. }
            | Edit::Redo { .. }
    )
}
