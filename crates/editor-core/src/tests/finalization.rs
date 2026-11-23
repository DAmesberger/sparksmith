//! One finalization for every entry path: the same schematic edit applied
//! directly and as a reviewed update (`project.apply_update`) gives the same
//! design, the same findings and the same undo. The reviewed path once
//! reconciled its candidate twice and lost the second pass's net changes, so
//! half of a split net dropped its class; and it checked a removed part's
//! variant overrides before they followed the part.
use super::support::*;
use crate::instances::variants::VariantCommand;
use crate::model::Population;
use crate::rules::{NetClassSpec, RulesCommand};
use crate::schematic::SchematicCommand;
use crate::updates::{ApplyUpdate, Change, InstanceChange};
use crate::{Engine, Error, InstanceCommand, Operation, Outcome, Query, QueryResult, Result};

fn reviewed(engine: &mut Engine, change: Change) -> Result<Outcome> {
    let plan = engine.preview_update(change.clone());
    assert!(plan.blockers.is_empty(), "{:?}", plan.blockers);
    let revision = engine.revision();
    apply(
        engine,
        Operation::Update(Box::new(ApplyUpdate {
            expected_revision: revision,
            change,
            remove_conflicting_tracks: false,
            merge_named_nets: false,
        })),
    )
}

fn issues(engine: &Engine) -> Result<Vec<(String, Vec<crate::ObjectRef>)>> {
    match engine.query(Query::Issues {
        scope: crate::issues::IssueScope::All,
    })? {
        QueryResult::Issues(report) => Ok(report
            .rows
            .into_iter()
            .map(|r| (r.finding.code.to_string(), r.finding.objects))
            .collect()),
        other => Err(Error::Invalid(format!("{other:?}"))),
    }
}

/// `direct` and `reviewed` reached the same design, findings and history.
fn assert_same(direct: &mut Engine, reviewed: &mut Engine, what: &str) -> Result<()> {
    assert_eq!(direct.project(), reviewed.project(), "{what}: design");
    assert_eq!(issues(direct)?, issues(reviewed)?, "{what}: findings");
    direct.undo()?;
    reviewed.undo()?;
    assert_eq!(direct.project(), reviewed.project(), "{what}: undo");
    direct.redo()?;
    reviewed.redo()?;
    assert_eq!(direct.project(), reviewed.project(), "{what}: redo");
    Ok(())
}

#[test]
fn a_reviewed_disconnect_keeps_the_class_on_both_halves_like_a_direct_one() -> Result<()> {
    let mut engine = Engine::demo()?;
    let r1 = by_designator(&engine, "R1")?;
    let class = match apply(
        &mut engine,
        Operation::Rules(RulesCommand::AddNetClass {
            class: NetClassSpec {
                name: "Isolated".into(),
                clearance: Some(2.2),
                ..Default::default()
            },
        }),
    )? {
        Outcome::NetClassAdded { class } => class,
        other => return Err(Error::Invalid(format!("{other:?}"))),
    };
    connect(&mut engine, ep(r1, 1), ep(r1, 2))?;
    let joined = net_of(&engine, ep(r1, 1))?;
    apply(
        &mut engine,
        Operation::Rules(RulesCommand::AssignNets {
            nets: vec![joined],
            class: Some(class),
        }),
    )?;
    let wire = wire_between(&engine, ep(r1, 1), ep(r1, 2))?;
    let command = SchematicCommand::Disconnect { wire };

    let mut direct = engine.fork();
    apply(&mut direct, Operation::Schematic(command.clone()))?;
    let mut review = engine.fork();
    reviewed(&mut review, Change::Schematic { command })?;

    for e in [&direct, &review] {
        let (a, b) = (net_of(e, ep(r1, 1))?, net_of(e, ep(r1, 2))?);
        assert_ne!(a, b, "the disconnect splits the net");
        let project = e.project();
        for net in [a, b] {
            assert_eq!(
                project.rules.class_of(net, project.net_name(net)),
                Some(class),
                "both halves keep the class"
            );
        }
    }
    assert_same(&mut direct, &mut review, "disconnect")
}

#[test]
fn a_reviewed_removal_takes_the_parts_variant_overrides_like_a_direct_one() -> Result<()> {
    let mut engine = Engine::demo()?;
    // An unwired part, which a direct command may remove.
    let demo = demo_fixture(&engine)?;
    let (r1, _) = place(&mut engine, &demo, "R3", 1, [0.0, 20.0])?;
    let lite = match apply(
        &mut engine,
        Operation::Variant(VariantCommand::Add {
            name: "Lite".into(),
            copy_of: None,
        }),
    )? {
        Outcome::VariantAdded { variant } => variant,
        other => return Err(Error::Invalid(format!("{other:?}"))),
    };
    apply(
        &mut engine,
        Operation::Variant(VariantCommand::SetPopulation {
            instances: vec![r1],
            variant: Some(lite),
            population: Some(Population::NotPopulated),
        }),
    )?;

    let mut direct = engine.fork();
    apply(
        &mut direct,
        Operation::Instance(InstanceCommand::Remove { instance: r1 }),
    )?;
    let mut review = engine.fork();
    reviewed(
        &mut review,
        Change::Instance {
            edit: InstanceChange::Remove { instance: r1 },
        },
    )?;
    for e in [&direct, &review] {
        assert!(e.project().instance(r1).is_none());
        assert_eq!(
            e.project().variants.get(lite).map(|v| v.overrides.len()),
            Some(0),
            "R3's override went with it"
        );
    }
    assert_same(&mut direct, &mut review, "removal")
}
