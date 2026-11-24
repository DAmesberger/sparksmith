//! Production gates and waiver identity, independent of Issues grouping.
use super::Fallible;
use crate::diagnostics::ObjectRef;
use crate::fabrication::{FabricationCommand, PreflightReport};
use crate::issues::{self, IssueScope};
use crate::model::production::StoredKey;
use crate::{Engine, Operation, Query, QueryResult, format};

fn report(engine: &Engine) -> Fallible<PreflightReport> {
    match engine.query(Query::Preflight { profile: None })? {
        QueryResult::Preflight(report) => Ok(*report),
        other => Err(format!("unexpected preflight result: {other:?}").into()),
    }
}

fn waive(engine: &mut Engine, key: String) -> Fallible<()> {
    engine.apply(
        engine.revision(),
        Operation::Fabrication(FabricationCommand::Waive {
            keys: vec![key],
            reason: "Reviewed this finding only".into(),
            profile: None,
        }),
    )?;
    Ok(())
}

/// The second part 1.5 mm above the first: their 1.0 by 1.4 mm lands are
/// 0.1 mm apart, pad 1 from pad 1 and pad 2 from pad 2, under the board's
/// own 0.3 mm (a limit of the fab profile's is never waived), and touch
/// nowhere.
pub(super) fn close_parts() -> Fallible<Engine> {
    let mut file = Engine::demo()?.to_file();
    let at = file
        .project
        .placements
        .first()
        .ok_or("first placement")?
        .position;
    file.project
        .placements
        .get_mut(1)
        .ok_or("second placement")?
        .position = [at[0], at[1] + 1.5];
    crate::rules::apply(
        &mut file.project.rules,
        crate::rules::RulesCommand::SetOverride {
            key: crate::rules::RuleKey::Clearance {
                a: crate::rules::CopperObject::Pad,
                b: crate::rules::CopperObject::Pad,
            },
            value: crate::rules::RuleValue::Length(0.3),
        },
    )?;
    Ok(Engine::from_file(file)?)
}

#[test]
fn each_clearance_pair_blocks_until_its_own_waiver() -> Fallible<()> {
    let mut engine = close_parts()?;
    let before = report(&engine)?;
    let pairs: Vec<_> = before
        .items
        .iter()
        .filter(|i| i.code == "board.clearance")
        .collect();
    assert_eq!(pairs.len(), 2, "two independent pad pairs must gate export");
    assert!(pairs.iter().all(|i| i.blocks()));
    assert_ne!(pairs[0].key, pairs[1].key);
    let second_key = pairs[1].key.clone();
    waive(&mut engine, pairs[0].key.clone())?;
    let after = report(&engine)?;
    assert_eq!(after.blocking + 1, before.blocking);
    assert!(
        after
            .items
            .iter()
            .any(|i| i.key == second_key && i.blocks())
    );
    waive(&mut engine, second_key)?;
    assert!(
        report(&engine)?
            .items
            .iter()
            .filter(|i| i.code == "board.clearance")
            .all(|i| !i.blocks())
    );
    engine.undo()?;
    assert_eq!(report(&engine)?.blocking, after.blocking);
    engine.redo()?;
    assert_eq!(report(&engine)?.blocking + 2, before.blocking);
    Ok(())
}

#[test]
fn waiving_a_primary_clearance_keeps_the_other_pair_in_issues() -> Fallible<()> {
    let mut engine = close_parts()?;
    let before = issues::report(
        engine.project(),
        engine.drafts(),
        &IssueScope::All,
        engine.check_settings(),
    )?;
    let row = before
        .rows
        .iter()
        .find(|r| r.finding.code == "board.clearance")
        .ok_or("clearance row")?;
    assert_eq!(row.findings(), 2, "Issues still groups the two pad pairs");
    let remaining = row.also.first().ok_or("second pad pair")?.clone();
    waive(
        &mut engine,
        crate::fabrication::preflight::key(&row.finding.code, &row.finding.objects),
    )?;
    let after = issues::report(
        engine.project(),
        engine.drafts(),
        &IssueScope::All,
        engine.check_settings(),
    )?;
    let rows: Vec<_> = after
        .rows
        .iter()
        .filter(|r| r.finding.code == "board.clearance")
        .collect();
    assert_eq!(rows.len(), 1, "the unwaived pair must stay visible");
    assert_eq!(rows[0].findings(), 1);
    assert_eq!(rows[0].finding, remaining);
    assert!(rows[0].objects().all(|o| remaining.objects.contains(o)));
    Ok(())
}

#[test]
fn legend_keys_roundtrip_and_waivers_survive_reopening() -> Fallible<()> {
    let mut file = Engine::demo()?.to_file();
    for placement in &mut file.project.placements {
        placement.position = [-100.0, -100.0];
    }
    // Two texts on the shared footprint: neither another graphic on the
    // same instance nor the same graphic on another instance is waived.
    for published in &mut file.project.library.footprints {
        let footprint = &mut published.definition;
        let mut graphic = footprint
            .graphics
            .iter()
            .find(|g| matches!(g.item, crate::library::graphics::Item::Text(_)))
            .ok_or("legend text")?
            .clone();
        graphic.id = crate::ids::GraphicId(footprint.next_local);
        footprint.next_local += 1;
        if let crate::library::graphics::Item::Text(text) = &mut graphic.item {
            text.content = crate::library::graphics::TextContent::Literal("Extra".into());
        }
        footprint.graphics.push(graphic);
    }
    let mut engine = Engine::from_file(file)?;
    let before = report(&engine)?;
    let legends: Vec<_> = before
        .items
        .iter()
        .filter(|i| i.code == "fabrication.legend_outside")
        .collect();
    assert_eq!(
        legends.len(),
        4,
        "two texts on each of the two parts are outside"
    );
    let keys: std::collections::BTreeSet<_> = legends.iter().map(|i| &i.key).collect();
    assert_eq!(keys.len(), 4, "every instance/graphic pair has its own key");
    for item in &before.items {
        let key = StoredKey::parse(&item.key)
            .ok_or_else(|| format!("unparseable production key: {}", item.key))?;
        assert_eq!(key.text(), item.key);
        assert_eq!(key.objects, item.objects);
    }
    let key = legends[0].key.clone();
    waive(&mut engine, key.clone())?;
    let reopened = Engine::from_file(format::decode(&format::encode(&engine.to_file())?)?)?;
    let after = report(&reopened)?;
    assert!(
        after
            .items
            .iter()
            .any(|i| i.key == key && i.waiver.is_some())
    );
    assert_eq!(
        after
            .items
            .iter()
            .filter(|i| i.code == "fabrication.legend_outside" && i.waiver.is_none())
            .count(),
        3,
        "other graphics and instances stay unwaived"
    );
    Ok(())
}

/// Each pour that was never filled is its own finding, and none can be
/// waived: the files would lack its copper.
#[test]
fn an_unfilled_fill_cannot_be_waived() -> Fallible<()> {
    let mut file = Engine::showcase()?.to_file();
    assert!(
        file.project.fills.len() >= 2,
        "fixture needs distinct fills"
    );
    file.project.fills.truncate(2);
    for fill in &mut file.project.fills {
        fill.computed = None;
        fill.name = "Same name".into();
    }
    let mut engine = Engine::from_file(file)?;
    let before = report(&engine)?;
    let fills: Vec<_> = before
        .items
        .iter()
        .filter(|i| i.code == "fabrication.fill_unfilled")
        .collect();
    assert_eq!(fills.len(), 2);
    assert_ne!(fills[0].key, fills[1].key);
    for fill in &engine.project().fills {
        assert!(
            fills
                .iter()
                .any(|item| item.objects == [ObjectRef::Fill { id: fill.id }])
        );
    }
    let keys: Vec<String> = before
        .items
        .iter()
        .filter(|i| i.code == "fabrication.fill_unfilled" || i.code == "fill.stale")
        .map(|i| i.key.clone())
        .collect();
    assert_eq!(keys.len(), 4);
    cannot_be_waived(&mut engine, &keys, "refill the fill")
}

/// The waive command refuses `keys`, saying `instead`, and changes nothing;
/// saved in a file they are ignored: each still blocks the export.
fn cannot_be_waived(engine: &mut Engine, keys: &[String], instead: &str) -> Fallible<()> {
    let before = report(engine)?;
    for key in keys {
        let item = before
            .items
            .iter()
            .find(|i| i.key == *key)
            .ok_or_else(|| format!("no {key}"))?;
        assert!(item.blocks() && !item.offers_waiver(), "{key}");
        let revision = engine.revision();
        let refused = waive(engine, key.clone())
            .err()
            .ok_or_else(|| format!("{key} was waived"))?
            .to_string();
        assert!(refused.contains(instead), "{refused}");
        assert_eq!(engine.revision(), revision);
    }
    let mut file = engine.to_file();
    let mut sorted = keys.to_vec();
    sorted.sort();
    file.project.production.waivers = sorted
        .into_iter()
        .map(|key| crate::model::production::Waiver {
            key,
            reason: "Reviewed".into(),
            accepted: Vec::new(),
            footprints: Vec::new(),
            given_for: None,
        })
        .collect();
    let held = Engine::from_file(file)?;
    let after = report(&held)?;
    assert_eq!((after.blocking, after.waived), (before.blocking, 0));
    for key in keys {
        assert!(after.items.iter().any(|i| i.key == *key && i.blocks()));
    }
    Ok(())
}

/// Copper of two nets that touches is a short: two crossing tracks cannot be
/// waived.
#[test]
fn copper_of_two_nets_that_touches_cannot_be_waived() -> Fallible<()> {
    let mut file = Engine::showcase()?.to_file();
    let project = &mut file.project;
    let [a, b, ..] = project.nets.iter().map(|n| n.id).collect::<Vec<_>>()[..] else {
        return Err("two nets".into());
    };
    // Two crossing tracks, clear of the board's other copper or not: what
    // they touch besides is found as well.
    for (net, start, end) in [
        (a, [10.0, 10.0], [12.0, 12.0]),
        (b, [10.0, 12.0], [12.0, 10.0]),
    ] {
        let id = project.allocate()?;
        project.tracks.push(crate::model::Track {
            id,
            net,
            start,
            end,
            width: 0.3,
            layer: crate::model::CopperLayer::Top,
            locked: false,
        });
    }
    let ids: Vec<_> = project.tracks.iter().rev().take(2).map(|t| t.id).collect();
    let mut engine = Engine::from_file(file)?;
    let keys: Vec<String> = report(&engine)?
        .items
        .iter()
        .filter(|i| {
            i.code == "board.overlap"
                && ids
                    .iter()
                    .all(|id| i.objects.contains(&ObjectRef::Track { id: *id }))
        })
        .map(|i| i.key.clone())
        .collect();
    assert_eq!(keys.len(), 1, "the crossing tracks");
    cannot_be_waived(&mut engine, &keys, "move the copper apart")
}

/// Every finding whose files would not be the design, or whose copper
/// would short two nets, refuses a waiver; a design rule keeps its own.
#[test]
fn fidelity_findings_and_shorts_are_the_ones_that_cannot_be_waived() {
    for code in [
        "fill.stale",
        "fill.overlap",
        "board.overlap",
        "fabrication.fill_unfilled",
        "fabrication.no_footprint",
        "fabrication.slot_unroutable.3",
        "board.shape_unsolved",
        "board.shape_open",
        "board.shape_invalid",
        "instance.check_failed",
    ] {
        assert!(!issues::can_waive(code), "{code}");
    }
    for code in [
        "fill.clearance",
        "fill.board_edge",
        "board.clearance",
        "board.track_width",
        "fabrication.legend_outside",
    ] {
        assert!(issues::can_waive(code), "{code}");
    }
}

/// An unrouted connection cannot be waived (e2e run 13, finding 37): the
/// waiver is refused, changing nothing, and a waiver saved in a file is
/// ignored, so the connection still blocks export and stays in Issues;
/// the board never reads as finished while airwires remain.
#[test]
fn an_unrouted_connection_cannot_be_waived() -> Fallible<()> {
    let mut engine = Engine::demo()?;
    let before = report(&engine)?;
    let unrouted: Vec<String> = before
        .items
        .iter()
        .filter(|i| i.code == "board.unrouted")
        .map(|i| i.key.clone())
        .collect();
    assert!(!unrouted.is_empty(), "the demo board has unrouted nets");
    let revision = engine.revision();
    let refused = engine.apply(
        revision,
        Operation::Fabrication(FabricationCommand::Waive {
            keys: unrouted.clone(),
            reason: "prototype".into(),
            profile: None,
        }),
    );
    let message = refused.err().ok_or("the waiver was accepted")?.to_string();
    assert!(message.contains("route the connection"), "{message}");
    assert_eq!(engine.revision(), revision);
    assert!(engine.project().production.waivers.is_empty());
    // A waiver saved before the rule (or written by hand) is ignored.
    let mut file = engine.to_file();
    file.project.production.waivers = unrouted
        .iter()
        .map(|key| crate::model::production::Waiver {
            key: key.clone(),
            reason: "prototype".into(),
            accepted: Vec::new(),
            footprints: Vec::new(),
            given_for: None,
        })
        .collect();
    let engine = Engine::from_file(file)?;
    let after = report(&engine)?;
    assert_eq!(after.blocking, before.blocking);
    assert_eq!(after.waived, 0);
    assert!(
        after
            .items
            .iter()
            .filter(|i| i.code == "board.unrouted")
            .all(|i| i.blocks() && i.waiver.is_none())
    );
    let shown = issues::report(
        engine.project(),
        engine.drafts(),
        &IssueScope::All,
        engine.check_settings(),
    )?;
    let open: usize = shown
        .rows
        .iter()
        .filter(|r| r.finding.code == "board.unrouted")
        .map(|r| r.findings())
        .sum();
    assert_eq!(open, unrouted.len(), "every unrouted connection stays open");
    Ok(())
}

/// A part not on the board cannot be waived either: the waiver is refused,
/// changing nothing, and a saved one is ignored, so the part still blocks
/// export; the fix points to placing it, or to Not fitted in a variant
/// for a part left unassembled.
#[test]
fn an_unplaced_part_cannot_be_waived() -> Fallible<()> {
    let mut engine = Engine::demo()?;
    let template = engine
        .project()
        .instances
        .first()
        .cloned()
        .ok_or("no instance")?;
    engine.apply(
        engine.revision(),
        Operation::Instance(crate::commands::InstanceCommand::Place(
            crate::instances::NewInstance {
                sheet: None,
                component: template.component,
                designator: Some("R9".into()),
                properties: vec![],
                representation: Some(template.representation),
                unit: None,
                position: None,
                physical: template.physical,
            },
        )),
    )?;
    let before = report(&engine)?;
    let unplaced = before
        .items
        .iter()
        .find(|i| i.code == "board.unplaced")
        .ok_or("R9 is not reported unplaced")?;
    assert!(unplaced.blocks());
    let fix = unplaced.fix.as_deref().ok_or("no fix")?;
    assert!(fix.contains("Not fitted in a variant"), "{fix}");
    let key = unplaced.key.clone();
    let revision = engine.revision();
    let refused = engine.apply(
        revision,
        Operation::Fabrication(FabricationCommand::Waive {
            keys: vec![key.clone()],
            reason: "prototype".into(),
            profile: None,
        }),
    );
    let message = refused.err().ok_or("the waiver was accepted")?.to_string();
    assert!(message.contains("place the part"), "{message}");
    assert_eq!(engine.revision(), revision);
    let mut file = engine.to_file();
    file.project.production.waivers = vec![crate::model::production::Waiver {
        key: key.clone(),
        reason: "prototype".into(),
        accepted: Vec::new(),
        footprints: Vec::new(),
        given_for: None,
    }];
    let engine = Engine::from_file(file)?;
    let after = report(&engine)?;
    assert_eq!(after.blocking, before.blocking);
    assert!(
        after
            .items
            .iter()
            .any(|i| i.key == key && i.blocks() && i.waiver.is_none())
    );
    Ok(())
}

/// Each assembled side needs its own fiducials, so each side's shortfall is
/// its own finding with its own waiver: waiving the top side's leaves the
/// bottom side's in production and in Issues.
#[test]
fn each_sides_fiducial_shortfall_has_its_own_waiver() -> Fallible<()> {
    let mut engine = Engine::showcase()?;
    engine.apply(
        engine.revision(),
        Operation::Rules(crate::rules::RulesCommand::SetOverride {
            key: crate::rules::RuleKey::FiducialCountMin,
            value: crate::rules::RuleValue::Count(64),
        }),
    )?;
    let short = |engine: &Engine| -> Fallible<Vec<_>> {
        Ok(report(engine)?
            .items
            .into_iter()
            .filter(|i| i.code == crate::layout::fiducial::TOO_FEW)
            .collect())
    };
    let sides = short(&engine)?;
    assert_eq!(
        sides.len(),
        2,
        "the showcase assembles both sides: {sides:?}"
    );
    assert_ne!(sides[0].key, sides[1].key, "each side is its own finding");
    let top = sides
        .iter()
        .find(|i| i.title.contains("top"))
        .ok_or("the top side's finding")?;
    let bottom_key = sides
        .iter()
        .find(|i| i.title.contains("bottom"))
        .ok_or("the bottom side's finding")?
        .key
        .clone();
    waive(&mut engine, top.key.clone())?;
    let after = short(&engine)?;
    assert!(
        after
            .iter()
            .any(|i| i.key == bottom_key && i.waiver.is_none()),
        "the bottom side is still unwaived: {after:?}"
    );
    let issues = issues::report(
        engine.project(),
        engine.drafts(),
        &IssueScope::All,
        engine.check_settings(),
    )?;
    let remaining: Vec<_> = issues
        .rows
        .iter()
        .filter(|r| r.finding.code == crate::layout::fiducial::TOO_FEW)
        .collect();
    assert_eq!(remaining.len(), 1, "Issues keeps the bottom side's");
    assert!(remaining[0].finding.message.contains("bottom"));
    Ok(())
}
