//! That the rules are alive: one board, made by hand, and what was seen of
//! it made wrong the way a fault would. Each fault names the rules that must
//! tell it. The product is not changed; `docs/development.md` names the
//! faults to seed in it by hand.
use super::Fallible;
use super::build;
use super::edits;
use super::invariants::{self, Broken, Kept, Rule, Seen, State};
use super::plan::{Atom, Case, Edit, Form, Opening, Pour};
use crate::layout::fill;
use crate::layout::router::reroute::Rerouted;
use crate::model::fill::FilledRegion;
use crate::model::{Layer, Project};
use crate::rules::{CopperObject, RuleKey, RuleOverride, RuleValue};
use crate::{Engine, format};
use std::collections::BTreeSet;

const PIXEL: f64 = 0.02;

fn pour(layer: usize) -> Atom {
    Atom::Copper(Edit::Pour(Pour {
        net: 2,
        free: false,
        layer,
        plane: true,
        from: [0.0; 2],
        to: [0.0; 2],
        priority: 0,
        clearance: None,
        islands: true,
        solid: false,
        hatched: false,
    }))
}

/// A pour of RAIL2 on the second layer, drawn.
fn drawn(from: [f64; 2], to: [f64; 2], priority: u32) -> Atom {
    Atom::Copper(Edit::Pour(Pour {
        net: 2,
        free: false,
        layer: 1,
        plane: false,
        from,
        to,
        priority,
        clearance: None,
        islands: false,
        solid: false,
        hatched: false,
    }))
}

/// Four layers; a part on each side; RAIL0 from one to the other through a
/// via, RAIL1 a stub, RAIL2 the pours of both outer layers and, on the
/// second, one at a pad and a patch on from it.
fn case() -> Case {
    let pad = |part: usize, spot: usize, form: Form, net: usize| {
        [
            Atom::Pad {
                part,
                spot,
                form,
                turn: 0.0,
                mask: Opening::Board,
                paste: Opening::Board,
            },
            Atom::Join { part, spot, net },
        ]
    };
    let part = |part: usize, side: Layer| Atom::Part {
        part,
        turn: 0.0,
        side,
        datum: false,
    };
    let line = |net: usize, layer: usize, from: [f64; 2], to: [f64; 2]| {
        Atom::Copper(Edit::Straight {
            net,
            layer,
            from,
            to,
            width: 0.25,
        })
    };
    let mut atoms = vec![part(0, Layer::Top), part(5, Layer::Bottom)];
    for (k, spot, form, net) in [
        (0, 8, Form::Round, 0),
        (0, 0, Form::Rect, 1),
        (0, 2, Form::Through, 2),
        (5, 0, Form::Rect, 0),
        (5, 1, Form::RoundRect, 1),
        (5, 2, Form::Through, 2),
    ] {
        atoms.extend(pad(k, spot, form, net));
    }
    atoms.extend([
        Atom::Copper(Edit::Via {
            net: 0,
            at: [19.0, 13.0],
            drill: 0.3,
            tented: false,
            capped: false,
        }),
        line(0, 0, [9.9, 9.9], [19.0, 13.0]),
        // The part below is seen from above: its first spot is up left.
        line(0, 3, [19.0, 13.0], [28.1, 20.9]),
        line(1, 0, [5.1, 5.1], [5.1, 16.0]),
        pour(0),
        pour(3),
        drawn([8.0, 3.0], [16.0, 7.5], 0),
        drawn([15.0, 3.5], [20.0, 7.0], 1),
    ]);
    Case {
        seed: 0,
        layers: 4,
        atoms: atoms.into_iter().enumerate().collect(),
    }
}

/// The board, all its copper made: here nothing may be refused.
fn board() -> Fallible<Engine> {
    let case = case();
    let mut engine = build::built(&case)?;
    for (_, atom) in &case.atoms {
        if let Atom::Copper(edit) = atom {
            let operation = edits::operation(&engine, edit)?
                .ok_or_else(|| format!("nothing to make {edit:?} of"))?;
            engine
                .apply(engine.revision(), operation)
                .map_err(|e| format!("{edit:?}: {e}"))?;
        }
    }
    engine.refill_stale()?;
    Ok(engine)
}

/// Lines of a file of `seen` changed.
fn filed(seen: &Seen, name: &str, change: impl Fn(Vec<String>) -> Vec<String>) -> Fallible<Seen> {
    let mut seen = seen.clone();
    let name = format!("random-{name}");
    let file = seen
        .files
        .iter_mut()
        .find(|f| f.0 == name)
        .ok_or_else(|| format!("no {name}"))?;
    let text = std::str::from_utf8(&file.1)?;
    let lines = change(text.lines().map(str::to_owned).collect());
    let after = (lines.join("\n") + "\n").into_bytes();
    if after == file.1 {
        return Err(format!("nothing seeded in {name}").into());
    }
    file.1 = after;
    Ok(seen)
}

/// The first line that is a place.
fn first(lines: &[String]) -> Option<usize> {
    lines.iter().position(|l| l.starts_with('X'))
}

fn after(project: &Project, change: impl Fn(&mut Project)) -> Project {
    let mut project = project.clone();
    change(&mut project);
    project
}

/// The fills of `from`, as if they were computed for `to`.
fn passed_off(from: &Project, to: &Project) -> Fallible<Project> {
    let mut out = to.clone();
    for fill in out.fills.iter_mut() {
        let Some(copper) = from.fill(fill.id).and_then(|f| f.computed.clone()) else {
            continue;
        };
        fill.computed = Some(crate::model::fill::FilledCopper {
            key: fill::inputs_key(to, fill.id)?,
            ..copper
        });
    }
    Ok(out)
}

type Fault = fn(&Engine, &Seen) -> Fallible<Vec<Broken>>;

/// Fills computed under a smaller clearance than the board's.
fn loose_fill(_: &Engine, seen: &Seen) -> Fallible<Vec<Broken>> {
    let loose = after(&seen.project, |p| {
        let clearance = |b: CopperObject| RuleKey::Clearance {
            a: CopperObject::Fill,
            b,
        };
        let keys = [
            RuleKey::FillClearance,
            clearance(CopperObject::Track).normalized(),
            clearance(CopperObject::Pad).normalized(),
            clearance(CopperObject::Via).normalized(),
        ];
        for key in keys {
            p.rules.overrides.push(RuleOverride {
                key,
                value: RuleValue::Length(0.12),
            });
        }
    });
    let loose = invariants::refilled(&loose)?;
    let project = passed_off(&loose, &seen.project)?;
    let mut seen = Seen::board(&project)?;
    // The host's own fills have the fault too.
    seen.made = project;
    invariants::all(&seen, PIXEL)
}

/// The patch's copper changed, in the board and in what a host fills.
fn patched(seen: &Seen, change: impl Fn(&mut Vec<FilledRegion>)) -> Fallible<Vec<Broken>> {
    let mut seen = seen.clone();
    for project in [&mut seen.project, &mut seen.made] {
        let patch = project.fills.last_mut().and_then(|f| f.computed.as_mut());
        change(&mut patch.ok_or("the patch")?.regions);
    }
    invariants::all(&seen, PIXEL)
}

/// The patch left out, as if it joined nothing.
fn patch_lost(_: &Engine, seen: &Seen) -> Fallible<Vec<Broken>> {
    patched(seen, Vec::clear)
}

/// An island that joins nothing, kept.
fn floating_kept(_: &Engine, seen: &Seen) -> Fallible<Vec<Broken>> {
    patched(seen, |regions| {
        regions.push(FilledRegion {
            outer: vec![[25.0, 4.0], [27.0, 4.0], [27.0, 6.0], [25.0, 6.0]],
            holes: Vec::new(),
        });
    })
}

fn moved_hole(_: &Engine, seen: &Seen) -> Fallible<Vec<Broken>> {
    let seen = filed(seen, "plated.drl", |mut l| {
        if let Some(line) = first(&l).and_then(|k| l.get_mut(k)) {
            *line = super::super::proof::shifted(line, 'X', 0.1, true);
        }
        l
    })?;
    invariants::all(&seen, PIXEL)
}

fn mirrored_mask(_: &Engine, seen: &Seen) -> Fallible<Vec<Broken>> {
    let middle = super::plan::BOARD[0] / 2.0;
    let seen = filed(seen, "mask-bottom.gbr", |l| {
        l.iter()
            .map(|s| match s.starts_with('X') {
                true => super::super::proof::mirrored(s, 'X', middle, false),
                false => s.clone(),
            })
            .collect()
    })?;
    invariants::all(&seen, PIXEL)
}

fn via_not_drilled(_: &Engine, seen: &Seen) -> Fallible<Vec<Broken>> {
    let seen = filed(seen, "plated.drl", |mut l| {
        if let Some(k) = first(&l) {
            l.remove(k);
        }
        l
    })?;
    invariants::all(&seen, PIXEL)
}

fn via_not_drawn(_: &Engine, seen: &Seen) -> Fallible<Vec<Broken>> {
    let via = seen.made.vias.first().ok_or("a via")?.position;
    let start = format!(
        "X{}Y{}D03",
        (via[0] * 1e6).round() as i64,
        (via[1] * 1e6).round() as i64
    );
    let seen = filed(seen, "copper-l2-inner1.gbr", |mut l| {
        l.retain(|s| !s.starts_with(&start));
        l
    })?;
    invariants::all(&seen, PIXEL)
}

/// The board without its via, seen through the graph and findings of before.
fn via_lost_unseen(_: &Engine, seen: &Seen) -> Fallible<Vec<Broken>> {
    let project = after(&seen.project, |p| p.vias.clear());
    let lost = Seen {
        islands: seen.islands.clone(),
        airwires: seen.airwires.clone(),
        findings: seen.findings.clone(),
        ..Seen::board(&project)?
    };
    invariants::all(&lost, PIXEL)
}

fn via_lost_unsaid(_: &Engine, seen: &Seen) -> Fallible<Vec<Broken>> {
    let lost = after(&seen.project, |p| p.vias.clear());
    Ok(invariants::lost(
        &seen.project,
        &lost,
        &Rerouted::default(),
        &[],
    ))
}

/// A track moved and the pours' copper of before passed off as current.
fn old_fill(_: &Engine, seen: &Seen) -> Fallible<Vec<Broken>> {
    let moved = after(&seen.project, |p| {
        if let Some(t) = p.tracks.last_mut() {
            t.end[0] += 3.0;
        }
    });
    let old = passed_off(&seen.project, &moved)?;
    invariants::all(&Seen::board(&old)?, PIXEL)
}

/// A track deleted and the graph of before held.
fn old_graph(_: &Engine, seen: &Seen) -> Fallible<Vec<Broken>> {
    let project = after(&seen.project, |p| {
        p.tracks.remove(1);
    });
    let old = Seen {
        islands: seen.islands.clone(),
        ..Seen::board(&project)?
    };
    invariants::all(&old, PIXEL)
}

/// The engine asked about a board its graph is not of.
fn old_engine(engine: &Engine, seen: &Seen) -> Fallible<Vec<Broken>> {
    let project = after(&seen.project, |p| {
        p.tracks.remove(1);
    });
    invariants::cached(&Kept::of(engine)?, engine, &Seen::board(&project)?)
}

/// A track deleted, and the issues of before kept beside the graph of now.
fn old_issues(engine: &Engine, _: &Seen) -> Fallible<Vec<Broken>> {
    let (before, after) = edited(engine)?;
    let kept = Kept {
        issues: Kept::of(&before)?.issues,
        ..Kept::of(&after)?
    };
    invariants::cached(&kept, &after, &Seen::board(after.project())?)
}

/// RAIL1's stub laid over RAIL0's via.
fn shorted(seen: &Seen) -> Project {
    after(&seen.project, |p| {
        if let Some(t) = p.tracks.last_mut() {
            t.end = [19.0, 13.0];
        }
    })
}

fn short_unsaid(_: &Engine, seen: &Seen) -> Fallible<Vec<Broken>> {
    let mut short = Seen::board(&shorted(seen))?;
    if invariants::all(&short, PIXEL)?
        .iter()
        .any(|b| b.rule == Rule::Short)
    {
        return Err("the product's own finding of a short is not taken".into());
    }
    short
        .findings
        .retain(|f| f.code != crate::diagnostics::FindingCode::BoardOverlap);
    invariants::all(&short, PIXEL)
}

/// A file that lost a track.
fn lossy_file(engine: &Engine, seen: &Seen) -> Fallible<Vec<Broken>> {
    let mut file = engine.to_file();
    file.project.tracks.remove(0);
    invariants::reopened(seen, engine, &format::encode(&file)?)
}

fn edited(engine: &Engine) -> Fallible<(Engine, Engine)> {
    let before = engine.fork();
    let mut after = engine.fork();
    let edit = Edit::DeleteTrack { pick: 0 };
    let operation = edits::operation(&after, &edit)?.ok_or("a track")?;
    after.apply(after.revision(), operation)?;
    Ok((before, after))
}

fn half_refused(engine: &Engine, _: &Seen) -> Fallible<Vec<Broken>> {
    let (before, after) = edited(engine)?;
    Ok(invariants::refused(&State::of(&before), &State::of(&after))
        .into_iter()
        .collect())
}

fn undo_undone(engine: &Engine, _: &Seen) -> Fallible<Vec<Broken>> {
    let (before, after) = edited(engine)?;
    let (a, b) = (before.document(), after.document());
    Ok(invariants::history(a, b, b, b).into_iter().collect())
}

fn redo_lost(engine: &Engine, _: &Seen) -> Fallible<Vec<Broken>> {
    let (before, after) = edited(engine)?;
    let (a, b) = (before.document(), after.document());
    Ok(invariants::history(a, b, a, a).into_iter().collect())
}

const FAULTS: [(&str, &[Rule], Fault); 20] = [
    (
        "the fill clearance too small",
        &[Rule::Clearance],
        loose_fill,
    ),
    (
        "a hole drilled 0.1 mm aside",
        &[Rule::Inventory, Rule::Image, Rule::ReadBack],
        moved_hole,
    ),
    (
        "the bottom mask mirrored",
        &[Rule::Image, Rule::ReadBack],
        mirrored_mask,
    ),
    (
        "a hole missing from the drill file",
        &[Rule::Inventory, Rule::Image, Rule::ReadBack],
        via_not_drilled,
    ),
    (
        "a via missing from an inner layer",
        &[Rule::Inventory, Rule::Image, Rule::ReadBack],
        via_not_drawn,
    ),
    (
        "a via lost and the graph and findings of before kept",
        &[Rule::Connectivity],
        via_lost_unseen,
    ),
    (
        "a via lost and nothing said",
        &[Rule::Silent],
        via_lost_unsaid,
    ),
    (
        "old fill copper passed off as current",
        &[Rule::Stale, Rule::Clearance],
        old_fill,
    ),
    (
        "a pour joined through another left out",
        &[Rule::Island],
        patch_lost,
    ),
    (
        "an island that joins nothing kept",
        &[Rule::Island],
        floating_kept,
    ),
    ("an old copper graph", &[Rule::Connectivity], old_graph),
    ("an engine that kept its graph", &[Rule::Cache], old_engine),
    ("an engine that kept its issues", &[Rule::Cache], old_issues),
    (
        "a short that no finding tells",
        &[Rule::Short],
        short_unsaid,
    ),
    ("a file that lost a track", &[Rule::Reopen], lossy_file),
    (
        "a refused edit that was half made",
        &[Rule::Refusal],
        half_refused,
    ),
    ("an undo that undoes nothing", &[Rule::History], undo_undone),
    ("a redo that redoes nothing", &[Rule::History], redo_lost),
    (
        "the two masks swapped",
        &[Rule::Image, Rule::ReadBack],
        swapped_masks,
    ),
    (
        "a pour missing from its file",
        &[Rule::Image, Rule::ReadBack],
        pour_not_drawn,
    ),
    (
        "a locked block's part moved",
        &[Rule::Block],
        locked_part_moved,
    ),
    (
        "a block moved and a part of it left",
        &[Rule::Block],
        block_torn,
    ),
];

fn swapped_masks(_: &Engine, seen: &Seen) -> Fallible<Vec<Broken>> {
    let mut seen = seen.clone();
    let at = |name: &str| seen.files.iter().position(|f| f.0 == name);
    let (Some(i), Some(j)) = (at("random-mask-top.gbr"), at("random-mask-bottom.gbr")) else {
        return Err("two masks".into());
    };
    let top = seen.files[i].1.clone();
    seen.files[i].1 = std::mem::replace(&mut seen.files[j].1, top);
    invariants::all(&seen, PIXEL)
}

fn pour_not_drawn(_: &Engine, seen: &Seen) -> Fallible<Vec<Broken>> {
    let seen = filed(
        seen,
        "copper-l1-top.gbr",
        |l| match super::super::proof::regions(&l).first().cloned() {
            Some(gone) => super::super::proof::without_region(l, gone),
            None => l,
        },
    )?;
    invariants::all(&seen, PIXEL)
}

/// The board with a block of both its parts, and that block's id.
fn blocked(engine: &Engine, locked: bool) -> Fallible<(Engine, crate::ids::Id)> {
    let mut engine = engine.fork();
    let add = edits::operation(&engine, &Edit::Block { pick: 0, count: 2 })?.ok_or("no parts")?;
    engine.apply(engine.revision(), add)?;
    let id = edits::block(engine.project(), 0).ok_or("no block")?;
    if locked {
        let lock = edits::operation(
            &engine,
            &Edit::LockBlock {
                pick: 0,
                locked: true,
            },
        )?
        .ok_or("no block")?;
        engine.apply(engine.revision(), lock)?;
    }
    Ok((engine, id))
}

fn locked_part_moved(engine: &Engine, _: &Seen) -> Fallible<Vec<Broken>> {
    let (engine, _) = blocked(engine, true)?;
    let torn = after(engine.project(), |p| {
        if let Some(p) = p.placements.first_mut() {
            p.position[0] += 1.0;
        }
    });
    Ok(invariants::blocks(engine.project(), &torn, None))
}

fn block_torn(engine: &Engine, _: &Seen) -> Fallible<Vec<Broken>> {
    let (engine, id) = blocked(engine, false)?;
    let torn = after(engine.project(), |p| {
        if let Some(p) = p.placements.first_mut() {
            p.position[0] += 1.0;
        }
    });
    Ok(invariants::blocks(engine.project(), &torn, Some(id)))
}

#[test]
fn every_seeded_fault_is_told_by_the_rules_that_must_tell_it() -> Fallible {
    let engine = board()?;
    let _context = engine.enter_context();
    let seen = Seen::files(engine.project())?;
    // The board is right, and has what the faults are about.
    let mut clean = invariants::all(&seen, PIXEL)?;
    clean.extend(invariants::cached(&Kept::of(&engine)?, &engine, &seen)?);
    clean.extend(invariants::reopened(
        &seen,
        &engine,
        &format::encode(&engine.to_file())?,
    )?);
    assert!(clean.is_empty(), "{clean:#?}");
    let joined = seen.islands.as_ref().is_some_and(|i| {
        build::net(&seen.project, 0)
            .and_then(|n| i.get(&n))
            .is_some_and(|pads| {
                pads.len() == 2 && pads.values().collect::<BTreeSet<_>>().len() == 1
            })
    });
    assert!(joined, "RAIL0 goes through the via: {:?}", seen.islands);
    let filled = |f: &crate::model::Fill| f.computed.iter().any(|c| !c.regions.is_empty());
    assert!(seen.project.fills.iter().all(filled));
    let mut missed = Vec::new();
    for (what, by, fault) in FAULTS {
        let told: BTreeSet<Rule> = fault(&engine, &seen)
            .map_err(|e| format!("{what}: {e}"))?
            .iter()
            .map(|b| b.rule)
            .collect();
        eprintln!("{what}: told by {told:?}");
        // The rule of what is kept tells what is kept, and nothing else.
        if told.contains(&Rule::Cache) && !by.contains(&Rule::Cache) {
            missed.push(format!("{what}: told by {:?} too", Rule::Cache));
        }
        for rule in by {
            if !told.contains(rule) {
                missed.push(format!("{what}: not told by {rule:?}, only by {told:?}"));
            }
        }
    }
    assert!(missed.is_empty(), "{missed:#?}");
    Ok(())
}
