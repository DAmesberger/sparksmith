//! Differential tests: a report made from the last state's ([`Checked::next`])
//! equals the full report ([`super::super::report`]) after every one of many
//! random edits to the routed showcase board. The edits move, resize,
//! re-net, add and delete tracks and vias (the incremental path), and mix
//! in part moves, rule changes, settings changes, unchanged copies and
//! jumps back to earlier states (the paths that check again in full).
use super::*;
use crate::layout::CopperGraph;
use crate::model::CopperLayer;

type Fallible<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

/// A small deterministic generator (xorshift64*), so a failure replays.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }

    /// A value in `[lo, hi)`.
    fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * ((self.next() >> 11) as f64 / (1u64 << 53) as f64)
    }
}

/// One random edit of `project`, and whether it touched only tracks and
/// vias.
fn edit(rng: &mut Rng, project: &mut Project) -> bool {
    let [w, h] = project.board_size;
    let nets: Vec<crate::ids::Id> = project.nets.iter().map(|n| n.id).collect();
    let layers = project.stackup.layers();
    let net = |rng: &mut Rng| nets.get(rng.below(nets.len())).copied();
    let nudge = |rng: &mut Rng, p: [f64; 2]| {
        [
            (p[0] + rng.range(-1.5, 1.5)).clamp(1.0, w - 1.0),
            (p[1] + rng.range(-1.5, 1.5)).clamp(1.0, h - 1.0),
        ]
    };
    let vias = project.vias.len();
    let tracks = project.tracks.len();
    match rng.below(16) {
        0 if vias > 0 => {
            let k = rng.below(vias);
            if let Some(v) = project.vias.get_mut(k) {
                v.position = nudge(rng, v.position);
            }
            true
        }
        1 if vias > 0 => {
            let k = rng.below(vias);
            if let Some(v) = project.vias.get_mut(k) {
                v.tented = !v.tented;
            }
            true
        }
        2 if vias > 0 => {
            let k = rng.below(vias);
            if let Some(v) = project.vias.get_mut(k) {
                v.drill = rng.range(0.1, 0.5);
                v.diameter = v.drill + rng.range(0.05, 0.6);
            }
            true
        }
        3 if vias > 0 => {
            let k = rng.below(vias);
            let n = net(rng);
            if let (Some(v), Some(n)) = (project.vias.get_mut(k), n) {
                v.net = n;
            }
            true
        }
        4 if tracks > 0 => {
            let k = rng.below(tracks);
            if let Some(t) = project.tracks.get_mut(k) {
                if rng.below(2) == 0 {
                    t.end = nudge(rng, t.end);
                } else {
                    let d = [rng.range(-1.0, 1.0), rng.range(-1.0, 1.0)];
                    t.start = nudge(rng, [t.start[0] + d[0], t.start[1] + d[1]]);
                    t.end = nudge(rng, [t.end[0] + d[0], t.end[1] + d[1]]);
                }
            }
            true
        }
        5 if tracks > 0 => {
            let k = rng.below(tracks);
            if let Some(t) = project.tracks.get_mut(k) {
                t.width = rng.range(0.05, 1.2);
            }
            true
        }
        6 if tracks > 0 => {
            let k = rng.below(tracks);
            let layer = layers.get(rng.below(layers.len())).copied();
            if let (Some(t), Some(layer)) = (project.tracks.get_mut(k), layer) {
                t.layer = layer;
            }
            true
        }
        7 if tracks > 0 => {
            let k = rng.below(tracks);
            let n = net(rng);
            if let (Some(t), Some(n)) = (project.tracks.get_mut(k), n) {
                t.net = n;
            }
            true
        }
        8 if tracks > 0 => {
            let k = rng.below(tracks);
            project.tracks.remove(k);
            true
        }
        9 if vias > 0 => {
            let k = rng.below(vias);
            project.vias.remove(k);
            true
        }
        10 if tracks > 0 => {
            let k = rng.below(tracks);
            if let Some(mut t) = project.tracks.get(k).cloned() {
                t.id = project.next_id;
                project.next_id += 1;
                t.start = nudge(rng, t.start);
                t.end = nudge(rng, t.end);
                project.tracks.push(t);
            }
            true
        }
        11 if vias > 0 => {
            let k = rng.below(vias);
            if let Some(mut v) = project.vias.get(k).cloned() {
                v.id = project.next_id;
                project.next_id += 1;
                v.position = nudge(rng, v.position);
                project.vias.push(v);
            }
            true
        }
        12 if !project.placements.is_empty() => {
            let k = rng.below(project.placements.len());
            if let Some(p) = project.placements.get_mut(k) {
                p.position = nudge(rng, p.position);
            }
            false
        }
        13 if !project.nets.is_empty() => {
            // A net renamed: findings name tracks and vias by their net.
            let k = rng.below(project.nets.len());
            let n = rng.below(1000);
            if let Some(net) = project.nets.get_mut(k) {
                net.name = format!("NET{n}");
            }
            false
        }
        14 if tracks > 0 => {
            // Several copper edits at once, as a route does.
            for _ in 0..4 {
                let k = rng.below(project.tracks.len());
                if let Some(t) = project.tracks.get_mut(k) {
                    t.end = nudge(rng, t.end);
                    t.layer = CopperLayer::Bottom;
                }
            }
            true
        }
        _ => {
            // An unchanged copy.
            false
        }
    }
}

/// The full report of `project`.
fn full(project: &Project, drafts: &Drafts, settings: &CheckSettings) -> Fallible<IssueReport> {
    Ok(super::super::report(
        project,
        drafts,
        &IssueScope::All,
        settings,
    )?)
}

fn graph_of(project: &Arc<Project>) -> impl Fn() -> Arc<CopperGraph> + '_ {
    move || Arc::new(CopperGraph::of(project))
}

#[test]
fn reports_made_from_the_last_state_equal_full_reports_after_every_edit() -> Fallible {
    let engine = crate::Engine::showcase()?;
    let _context = engine.enter_context();
    let drafts = engine.document().drafts.clone();
    let mut project = engine.project_arc().clone();
    let mut settings = CheckSettings::default();
    let mut checked = Checked::full(&project, &drafts, &settings, &graph_of(&project))?;
    assert_eq!(**checked.report(), full(&project, &drafts, &settings)?);
    let mut history = vec![project.clone()];
    let mut rng = Rng(0x5eed_1234_abcd_ef01);
    let (mut copper, mut codes) = (0usize, std::collections::BTreeSet::new());
    // States made from the last one: kept findings, then clearance, vias,
    // pad holes and connectivity.
    let mut paths = [0usize; 5];
    for step in 0..240 {
        let roll = rng.below(20);
        let next = if roll == 0 && history.len() > 1 {
            // Back to an earlier state, as undo does.
            history[rng.below(history.len())].clone()
        } else if roll == 1 {
            settings.impedance_tolerance = match settings.impedance_tolerance {
                Some(_) => None,
                None => Some(10.0),
            };
            project.clone()
        } else {
            let mut next = (*project).clone();
            copper += usize::from(edit(&mut rng, &mut next));
            Arc::new(next)
        };
        checked = checked.next(&next, &drafts, &settings, &graph_of(&next))?;
        let p = checked.paths();
        for (count, taken) in
            paths
                .iter_mut()
                .zip([p.kept, p.clearance, p.vias, p.pad_holes, p.connectivity])
        {
            *count += usize::from(taken);
        }
        let expected = full(&next, &drafts, &settings)?;
        assert_eq!(**checked.report(), expected, "step {step}");
        codes.extend(expected.rows.iter().map(|r| r.finding.code.to_string()));
        project = next;
        history.push(project.clone());
    }
    // The run exercised the incremental path and board findings of many
    // kinds.
    assert!(copper > 120, "{copper} copper edits");
    let [kept, clearance, vias, holes, connectivity] = paths;
    assert!(kept > 100 && clearance == kept && vias == kept, "{paths:?}");
    assert!(holes > 100 && connectivity > 100, "{paths:?}");
    for code in [
        "board.clearance",
        "board.unrouted",
        "board.track_dangling",
        "board.via_drill_clearance",
        "fill.stale",
    ] {
        assert!(codes.contains(code), "{code} never appeared: {codes:?}");
    }
    Ok(())
}

/// `project.issues` through the engine: made once per state from the last
/// report (shared with forks), equal to the full report after commands,
/// undo, redo, a settings change and on a fork.
#[test]
fn the_engine_report_follows_commands_undo_redo_and_forks() -> Fallible {
    use crate::layout::LayoutCommand;
    use crate::{Operation, Query, QueryResult};
    let mut engine = crate::Engine::showcase()?;
    let _context = engine.enter_context();
    let check = |engine: &crate::Engine| -> Fallible {
        let expected = full(engine.project(), engine.drafts(), engine.check_settings())?;
        assert_eq!(*engine.issue_report()?, expected);
        let QueryResult::Issues(answer) = engine.query(Query::Issues {
            scope: IssueScope::All,
        })?
        else {
            return Err("project.issues gave another answer".into());
        };
        assert_eq!(answer, expected);
        Ok(())
    };
    check(&engine)?;
    let vias: Vec<crate::ids::Id> = engine.project().vias.iter().map(|v| v.id).collect();
    for (k, id) in vias.iter().take(6).enumerate() {
        let via = engine.project().via(*id).cloned().ok_or("via")?;
        let operation = if k % 2 == 0 {
            LayoutCommand::SetViaTented {
                id: *id,
                tented: !via.tented,
            }
        } else {
            LayoutCommand::MoveVia {
                id: *id,
                position: [via.position[0] + 0.4, via.position[1] - 0.3],
            }
        };
        if engine
            .apply(engine.revision(), Operation::Layout(operation))
            .is_ok()
        {
            check(&engine)?;
        }
    }
    engine.undo()?;
    check(&engine)?;
    engine.undo()?;
    check(&engine)?;
    engine.redo()?;
    check(&engine)?;
    engine.set_check_settings(CheckSettings {
        impedance_tolerance: Some(5.0),
    })?;
    check(&engine)?;
    let mut fork = engine.fork();
    let first = vias.first().copied().ok_or("no via")?;
    fork.apply(
        fork.revision(),
        Operation::Layout(LayoutCommand::DeleteVia { id: first }),
    )?;
    check(&fork)?;
    // The fork's report went into the shared slot; the engine's state is
    // still its own.
    check(&engine)?;
    Ok(())
}

/// A pad hole spares copper its pad's island holds, so cutting that
/// island anywhere changes the hole's findings: a track that comes within
/// the hole's clearance without touching the pad is spared while another
/// track joins it to the pad, and reported once a far track that made the
/// joint is gone. The report made from the last state checks that hole
/// again though nothing near it changed.
#[test]
fn a_pad_hole_is_checked_again_when_its_island_is_cut_far_away() -> Fallible {
    use crate::diagnostics::ObjectRef;
    use crate::rules::{CopperObject, RuleKey, RuleOverride, RuleValue};
    let engine = crate::Engine::showcase()?;
    let _context = engine.enter_context();
    let drafts = engine.document().drafts.clone();
    let settings = CheckSettings::default();
    let mut project = engine.project().clone();
    let pad = crate::layout::placed_pads(&project)
        .into_iter()
        .find(|p| !p.holes().is_empty() && p.net.is_some())
        .ok_or("no pad hole")?;
    let net = pad.net.ok_or("no net")?;
    let (centre, hole) = match pad.holes().first().map(|h| h.shape.clone()) {
        Some(crate::geometry::Shape::Circle { center, radius }) => (center, radius),
        _ => return Err("not a round hole".into()),
    };
    let reach = pad.outline().bounds(0.0);
    let pad_radius = (reach.max[0] - reach.min[0]).max(reach.max[1] - reach.min[1]) / 2.0;
    // Hole to track clearance well past the pad's copper.
    for (a, b) in [
        (CopperObject::Track, CopperObject::PthHole),
        (CopperObject::PthHole, CopperObject::Track),
    ] {
        project.rules.overrides.push(RuleOverride {
            key: RuleKey::Clearance { a, b },
            value: RuleValue::Length(pad_radius - hole + 1.0),
        });
    }
    let at = |dx: f64, dy: f64| [centre[0] + dx, centre[1] + dy];
    let mut track = |start, end| {
        let id = project.next_id;
        project.next_id += 1;
        project.tracks.push(crate::model::Track {
            id,
            net,
            start,
            end,
            width: 0.25,
            layer: CopperLayer::Top,
            locked: false,
        });
        id
    };
    track(centre, at(6.0, 0.0));
    let bridge = track(at(6.0, 0.0), at(6.0, 4.0));
    let near = track(at(6.0, 4.0), at(0.0, pad_radius + 0.4));
    let joined = Arc::new(project.clone());
    let mut cut = project;
    cut.tracks.retain(|t| t.id != bridge);
    let cut = Arc::new(cut);
    let reports = |hole_near: &IssueReport| {
        hole_near
            .rows
            .iter()
            .flat_map(|r| std::iter::once(&r.finding).chain(&r.also))
            .any(|f| {
                f.code == "board.pad_hole_clearance"
                    && f.objects.contains(&ObjectRef::Track { id: near })
                    && f.objects.contains(&ObjectRef::Pad {
                        instance: pad.instance,
                        pad: pad.pad.id,
                    })
            })
    };
    let checked = Checked::full(&joined, &drafts, &settings, &graph_of(&joined))?;
    assert!(!reports(checked.report()), "the joined track is spared");
    let next = checked.next(&cut, &drafts, &settings, &graph_of(&cut))?;
    let expected = full(&cut, &drafts, &settings)?;
    assert!(reports(&expected), "the cut track is reported");
    assert!(next.paths().pad_holes, "made from the last state");
    assert_eq!(**next.report(), expected);
    Ok(())
}
