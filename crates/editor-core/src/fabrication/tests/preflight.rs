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
        }),
    )?;
    Ok(())
}

fn overlapping_parts() -> Fallible<Engine> {
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
        .position = at;
    Ok(Engine::from_file(file)?)
}

#[test]
fn each_clearance_pair_blocks_until_its_own_waiver() -> Fallible<()> {
    let mut engine = overlapping_parts()?;
    // The two two-pad parts coincide: pad 1 crosses pad 1, pad 2 crosses pad 2.
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
    let mut engine = overlapping_parts()?;
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

#[test]
fn each_unfilled_fill_has_its_own_waiver_identity() -> Fallible<()> {
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
    let other = fills[1].key.clone();
    waive(&mut engine, fills[0].key.clone())?;
    let after = report(&engine)?;
    assert_eq!(after.blocking + 1, before.blocking);
    assert!(
        after
            .items
            .iter()
            .any(|item| item.key == other && item.blocks())
    );
    Ok(())
}
