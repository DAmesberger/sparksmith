//! Which islands stay when fills of one net join.
//!
//! The board is the demo's: R1 at (10, 10) and R2 at (25, 10), net 2 joining
//! R1.2 (land 10.3–11.3 mm in x) and R2.1 (land 23.7–24.7 mm); R1.1, at
//! 9.2 mm, is on net 1.
use super::super::{FillCache, RefillTarget, compute, compute_stale, inputs_keys, refill};
use crate::diagnostics::{Error, Finding, Result};
use crate::geometry::{Point, polygon_contains};
use crate::ids::Id;
use crate::layout;
use crate::layout::tests::{SHARED, fixture};
use crate::library::{Outline, Segment};
use crate::model::fill::{FillSettings, FillSpec, FilledCopper, IslandRemoval};
use crate::model::{Fill, Layer, Project};
use std::collections::BTreeMap;

const OTHER: Id = 1;
const UNCONNECTED: IslandRemoval = IslandRemoval::Unconnected;
const NEVER: IslandRemoval = IslandRemoval::Never;

type Pour = (Id, u32, [f64; 4], IslandRemoval);

/// Reaches R1.2, not R2.1.
const PLANE: [f64; 4] = [10.4, 8.0, 18.0, 12.0];
/// On from the plane, reaching no pad.
const PATCH: [f64; 4] = [17.0, 9.0, 20.0, 11.0];
/// On from the patch, reaching no pad either.
const NEXT: [f64; 4] = [19.5, 9.0, 23.0, 11.0];

/// The demo with pours on its top, unfilled, and their ids.
fn board(pours: &[Pour]) -> Result<(Project, Vec<Id>)> {
    let mut project = fixture()?;
    let mut ids = Vec::new();
    for (net, priority, [x0, y0, x1, y1], islands) in pours {
        let id = project.allocate()?;
        ids.push(id);
        project.fills.push(Fill::from_spec(
            id,
            FillSpec {
                region: crate::model::fill::FillRegion::Drawn,
                name: String::new(),
                net: Some(*net),
                layer: Layer::Top.into(),
                outline: Outline {
                    start: [*x0, *y0],
                    segments: [[*x1, *y0], [*x1, *y1], [*x0, *y1], [*x0, *y0]]
                        .into_iter()
                        .map(|to| Segment::Line { to })
                        .collect(),
                },
                priority: *priority,
                settings: FillSettings {
                    islands: *islands,
                    ..FillSettings::default()
                },
            },
        ));
    }
    Ok((project, ids))
}

fn store(project: &mut Project, copper: Vec<(Id, FilledCopper)>) {
    for (id, copper) in copper {
        if let Some(fill) = project.fills.iter_mut().find(|f| f.id == id) {
            fill.computed = Some(copper);
        }
    }
}

/// The board filled, the same however it is: all at once, what is stale,
/// and each fill alone, in either order, on the unfilled board and one after
/// the other.
fn filled(pours: &[Pour]) -> Result<(Project, Vec<Id>)> {
    let (empty, ids) = board(pours)?;
    let all: BTreeMap<Id, FilledCopper> = refill(&empty, RefillTarget::All)?.into_iter().collect();
    let same = |what: &str, got: Vec<(Id, FilledCopper)>| {
        assert_eq!(got.len(), ids.len(), "{what}");
        for (id, copper) in got {
            assert_eq!(Some(&copper), all.get(&id), "{what}");
        }
    };
    same("stale", compute_stale(&empty, &mut FillCache::default())?);
    for backwards in [false, true] {
        let order: Vec<Id> = match backwards {
            true => ids.iter().rev().copied().collect(),
            false => ids.clone(),
        };
        let mut alone = Vec::new();
        let mut stepwise = empty.clone();
        for id in order {
            alone.push((id, compute(&empty, id)?));
            let one = refill(&stepwise, RefillTarget::One(id))?;
            store(&mut stepwise, one);
        }
        same("alone", alone);
        let stored = stepwise.fills.iter();
        same(
            "one after the other",
            stored
                .filter_map(|f| Some((f.id, f.computed.clone()?)))
                .collect(),
        );
        // Filled one after the other, none is stale: no refill follows.
        assert!(compute_stale(&stepwise, &mut FillCache::default())?.is_empty());
    }
    let mut project = empty;
    store(&mut project, all.into_iter().collect());
    Ok((project, ids))
}

/// The (kept, removed) islands of each fill.
fn islands(project: &Project, ids: &[Id]) -> Vec<(usize, u32)> {
    ids.iter()
        .filter_map(|id| project.fill(*id)?.computed.as_ref())
        .map(|c| (c.regions.len(), c.removed_islands))
        .collect()
}

fn codes(project: &Project, code: &str) -> Vec<Finding> {
    layout::checks(project, &Default::default())
        .into_iter()
        .filter(|f| f.code == code)
        .collect()
}

fn covers(project: &Project, id: Id, p: Point) -> Result<bool> {
    let copper = project
        .fill(id)
        .and_then(|f| f.computed.as_ref())
        .ok_or_else(|| Error::NotFound("copper".into()))?;
    Ok(copper
        .regions
        .iter()
        .any(|r| polygon_contains(&r.outer, p) && !r.holes.iter().any(|h| polygon_contains(h, p))))
}

#[test]
fn an_island_joined_through_another_pour_of_its_net_stays() -> Result<()> {
    // Whichever outranks the other, and whichever is older.
    for (plane, patch, next) in [(0, 0, 0), (0, 1, 2), (2, 1, 0), (1, 0, 1), (0, 1, 0)] {
        for turned in [false, true] {
            let mut pours = vec![
                (SHARED, plane, PLANE, UNCONNECTED),
                (SHARED, patch, PATCH, UNCONNECTED),
                (SHARED, next, NEXT, UNCONNECTED),
            ];
            if turned {
                pours.reverse();
            }
            let (project, ids) = filled(&pours)?;
            assert_eq!(islands(&project, &ids), [(1, 0); 3], "{pours:?}");
            assert_eq!(codes(&project, "fill.island_unconnected"), Vec::new());
            assert_eq!(codes(&project, "fill.empty"), Vec::new());
        }
    }
    // Through a pour that keeps every island.
    let (project, ids) = filled(&[(SHARED, 0, PLANE, NEVER), (SHARED, 0, PATCH, UNCONNECTED)])?;
    assert_eq!(islands(&project, &ids), [(1, 0); 2]);
    Ok(())
}

#[test]
fn pours_that_join_only_each_other_go() -> Result<()> {
    let (project, ids) = filled(&[
        (SHARED, 0, PLANE, UNCONNECTED),
        (SHARED, 0, [30.0, 20.0, 34.0, 24.0], UNCONNECTED),
        (SHARED, 1, [33.0, 20.0, 37.0, 24.0], UNCONNECTED),
    ])?;
    assert_eq!(islands(&project, &ids), [(1, 0), (0, 1), (0, 1)]);
    assert_eq!(codes(&project, "fill.empty").len(), 2);
    // Nor does one that floats and is kept hold another.
    let (project, ids) = filled(&[
        (SHARED, 0, [30.0, 20.0, 34.0, 24.0], NEVER),
        (SHARED, 0, [33.0, 20.0, 37.0, 24.0], UNCONNECTED),
    ])?;
    assert_eq!(islands(&project, &ids), [(1, 0), (0, 1)]);
    assert_eq!(codes(&project, "fill.island_unconnected").len(), 1);
    // Nor another net's.
    let (project, ids) = filled(&[
        (SHARED, 1, PLANE, UNCONNECTED),
        (OTHER, 0, PATCH, UNCONNECTED),
    ])?;
    assert_eq!(islands(&project, &ids), [(1, 0), (0, 1)]);
    // Nor one on another layer.
    let (mut project, ids) = board(&[
        (SHARED, 0, PLANE, UNCONNECTED),
        (SHARED, 0, PATCH, UNCONNECTED),
    ])?;
    if let Some(f) = project.fills.last_mut() {
        f.layer = Layer::Bottom.into();
    }
    let all = refill(&project, RefillTarget::All)?;
    store(&mut project, all);
    assert_eq!(islands(&project, &ids), [(1, 0), (0, 1)]);
    Ok(())
}

/// A pour of the net, one of another net it clears, and one of the net that
/// clears that: each reads the next, and the first the last.
#[test]
fn fills_that_read_each_other_are_settled_together() -> Result<()> {
    let first = (SHARED, 2, [14.0, 14.0, 18.0, 18.0], UNCONNECTED);
    let last = (SHARED, 0, [10.4, 8.0, 15.0, 16.0], UNCONNECTED);
    // The other net's comes between the two: the first joins nothing and
    // goes, and the other net's is poured there.
    let between = (OTHER, 1, [5.0, 9.5, 19.0, 30.0], UNCONNECTED);
    let (project, ids) = filled(&[first, between, last])?;
    assert_eq!(islands(&project, &ids).first(), Some(&(0, 1)));
    assert!(covers(&project, ids[1], [16.0, 16.0])?);
    // Beside them: the first is joined through the last and keeps its place.
    let beside = (OTHER, 1, [15.5, 13.0, 19.0, 30.0], NEVER);
    let (project, ids) = filled(&[first, beside, last])?;
    assert_eq!(islands(&project, &ids).first(), Some(&(1, 0)));
    assert!(covers(&project, ids[0], [16.0, 16.0])?);
    assert!(!covers(&project, ids[1], [16.0, 16.0])?);
    assert_eq!(codes(&project, "fill.island_unconnected").len(), 1);
    // Between them, but joining nothing: it stands in no one's way.
    let floating = (OTHER, 1, [9.0, 12.5, 19.0, 30.0], UNCONNECTED);
    let (project, ids) = filled(&[first, floating, last])?;
    assert_eq!(islands(&project, &ids), [(1, 0), (0, 1), (1, 0)]);
    Ok(())
}

#[test]
fn a_pour_goes_stale_with_what_joins_it_and_is_filled_once() -> Result<()> {
    let (mut project, ids) = filled(&[
        (SHARED, 0, PLANE, UNCONNECTED),
        (SHARED, 0, PATCH, UNCONNECTED),
        (SHARED, 0, NEXT, UNCONNECTED),
        (SHARED, 0, [30.0, 20.0, 34.0, 24.0], NEVER),
        (SHARED, 0, [33.0, 20.0, 37.0, 24.0], NEVER),
    ])?;
    let before = inputs_keys(&project)?;
    // R1 away: the plane reaches no pad, and what it held goes with it.
    let r1 = project.placements.first_mut();
    r1.ok_or_else(|| Error::NotFound("R1".into()))?.position = [10.0, 30.0];
    let after = inputs_keys(&project)?;
    let stale: Vec<bool> = ids
        .iter()
        .map(|id| before.get(id) != after.get(id))
        .collect();
    // Pours that keep every island read no other of their net.
    assert_eq!(stale, [true, true, true, false, false]);
    let mut cache = FillCache::default();
    let again = compute_stale(&project, &mut cache)?;
    assert_eq!(again.len(), 3);
    store(&mut project, again);
    assert!(compute_stale(&project, &mut cache)?.is_empty());
    assert_eq!(
        islands(&project, &ids),
        [(0, 1), (0, 1), (0, 1), (1, 0), (1, 0)]
    );
    // Back, and filled from what the host kept: as it was.
    if let Some(r1) = project.placements.first_mut() {
        r1.position = [10.0, 10.0];
    }
    assert_eq!(inputs_keys(&project)?, before);
    let back = compute_stale(&project, &mut cache)?;
    store(&mut project, back);
    assert_eq!(islands(&project, &ids), [(1, 0); 5]);
    Ok(())
}

/// What a move is told to have opened is counted as the refill joins it.
#[test]
fn pads_in_two_pours_that_come_together_are_one_group() -> Result<()> {
    let groups = |pours: &[Pour]| -> Result<usize> {
        let (project, _) = board(pours)?;
        let mut bare = project.clone();
        bare.fills.clear();
        Ok(layout::CopperGraph::of_nets(&bare, &[SHARED]).pad_groups(&project, SHARED))
    };
    let on = |x0: f64| (SHARED, 0, [x0, 8.0, 24.6, 12.0], UNCONNECTED);
    let plane = (SHARED, 1, PLANE, UNCONNECTED);
    assert_eq!(groups(&[plane])?, 2);
    assert_eq!(groups(&[plane, on(18.5)])?, 2);
    assert_eq!(groups(&[plane, on(17.0)])?, 1);
    Ok(())
}
