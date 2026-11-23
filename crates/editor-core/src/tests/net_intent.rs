//! Net intent is read-only. Its corrections go through one revision-bound
//! batch and the saved document's own net table supplies the independent check.
use super::support::{Snapshot, apply, by_designator, connect, ep, net_of};
use crate::schematic::{
    SchematicCommand,
    intent::{Correction, IntentNets, Report},
};
use crate::{Endpoint, Engine, Error, Evaluators, Operation, Query, QueryResult, Result};
use std::collections::BTreeSet;

fn check(engine: &Engine, nets: &IntentNets) -> Result<Report> {
    let before = Snapshot::of(engine);
    let reply = engine.query(Query::CheckIntent { nets: nets.clone() });
    before.assert_kept(engine, "intent check");
    match reply? {
        QueryResult::NetIntent(report) => Ok(*report),
        other => Err(Error::Invalid(format!("unexpected {other:?}"))),
    }
}

fn commit(engine: &mut Engine, report: &Report) -> Result<()> {
    assert!(report.blocked.is_none(), "{report:?}");
    let revision = engine.revision();
    let operations: Vec<_> = report
        .operations
        .iter()
        .map(|s| Operation::Schematic(s.command()))
        .collect();
    assert!(matches!(
        engine.apply_all_hosted(
            revision.saturating_sub(1),
            operations.clone(),
            Evaluators::default()
        ),
        Err(Error::Stale { .. })
    ));
    engine.apply_all_hosted(revision, operations, Evaluators::default())?;
    assert_eq!(engine.revision(), revision + 1);
    Ok(())
}

fn assert_members(engine: &Engine, name: &str, expected: &[Endpoint]) -> Result<()> {
    let net = engine
        .project()
        .nets
        .iter()
        .find(|n| n.name == name)
        .ok_or_else(|| Error::NotFound(name.into()))?;
    assert_eq!(
        net.terminals.iter().copied().collect::<BTreeSet<_>>(),
        expected.iter().copied().collect::<BTreeSet<_>>()
    );
    Ok(())
}

#[test]
fn missing_members_are_fixed_without_reconnecting_joined_pins() -> Result<()> {
    let mut engine = Engine::demo()?;
    let a = by_designator(&engine, "R1")?;
    let b = by_designator(&engine, "R2")?;
    let kept = [ep(a, 2), ep(b, 1)];
    let missing = [ep(a, 1), ep(b, 2)];
    let goals = IntentNets::from([
        ("SENSE".into(), kept.to_vec()),
        ("RETURN".into(), missing.to_vec()),
    ]);
    let report = check(&engine, &goals)?;
    assert_eq!(report.missing.len(), 4);
    assert!(report.extra.is_empty() && report.wrong.is_empty());
    assert_eq!(
        report
            .operations
            .iter()
            .filter(|op| matches!(op, Correction::Connect { .. }))
            .count(),
        1
    );
    assert_eq!(report.operations.len(), 3);
    commit(&mut engine, &report)?;
    assert_members(&engine, "SENSE", &kept)?;
    assert_members(&engine, "RETURN", &missing)?;
    assert_eq!(check(&engine, &goals)?, Report::default());
    Ok(())
}

#[test]
fn wrong_leaf_is_reported_on_both_nets_and_moved_once() -> Result<()> {
    let mut engine = Engine::demo()?;
    let a = by_designator(&engine, "R1")?;
    let b = by_designator(&engine, "R2")?;
    let net = net_of(&engine, ep(a, 2))?;
    apply(
        &mut engine,
        Operation::Schematic(SchematicCommand::RenameNet {
            net,
            name: "SENSE".into(),
        }),
    )?;
    connect(&mut engine, ep(a, 2), ep(b, 2))?;
    let kept = [ep(a, 2), ep(b, 1)];
    let moved = [ep(b, 2), ep(a, 1)];
    let goals = IntentNets::from([
        ("SENSE".into(), kept.to_vec()),
        ("RETURN".into(), moved.to_vec()),
    ]);
    let r = check(&engine, &goals)?;
    assert_eq!(r.wrong.len(), 1);
    assert_eq!(r.wrong[0].terminal, ep(b, 2));
    assert_eq!(r.wrong[0].net, "RETURN");
    assert_eq!(r.wrong[0].actual_net, "SENSE");
    assert_eq!(r.extra.len(), 1);
    assert_eq!(r.extra[0].terminal, ep(b, 2));
    assert_eq!(r.extra[0].net, "SENSE");
    assert_eq!(
        r.operations
            .iter()
            .filter(|op| matches!(op, Correction::Disconnect { .. }))
            .count(),
        1
    );
    commit(&mut engine, &r)?;
    assert_members(&engine, "SENSE", &kept)?;
    assert_members(&engine, "RETURN", &moved)?;
    Ok(())
}

#[test]
fn contradictions_name_the_pin_and_a_stranded_named_net_has_no_partial_plan() -> Result<()> {
    let mut engine = Engine::demo()?;
    let a = by_designator(&engine, "R1")?;
    let end = ep(a, 1);
    let wrong = IntentNets::from([("VIN".into(), vec![end]), ("GND".into(), vec![end])]);
    let why = check(&engine, &wrong)
        .err()
        .ok_or_else(|| Error::Invalid("contradictory intent passed".into()))?
        .to_string();
    assert!(
        why.contains("R1") && why.contains("VIN") && why.contains("GND"),
        "{why}"
    );
    let bad = IntentNets::from([("VIN".into(), vec![ep(a, 999)])]);
    let why = check(&engine, &bad)
        .err()
        .ok_or_else(|| Error::Invalid("unknown terminal passed".into()))?
        .to_string();
    assert!(why.contains("R1") && why.contains("999"), "{why}");
    let net = net_of(&engine, end)?;
    apply(
        &mut engine,
        Operation::Schematic(SchematicCommand::RenameNet {
            net,
            name: "PROTECTED".into(),
        }),
    )?;
    let result = check(&engine, &IntentNets::from([("VIN".into(), vec![end])]))?;
    assert!(result.operations.is_empty());
    assert_eq!(result.wrong.len(), 1);
    let blocked = result.blocked.unwrap_or_default();
    assert!(
        blocked.contains("R1") && blocked.contains("PROTECTED"),
        "{blocked}"
    );
    Ok(())
}
