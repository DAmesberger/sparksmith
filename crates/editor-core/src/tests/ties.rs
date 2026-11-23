//! A copper bridge joins two separately mapped passive lands.
use super::support::*;
use crate::ids::{BridgeId, PadId, TerminalId};
use crate::instances::variants::VariantCommand;
use crate::library::tie::{Bridge, TieForm, TieSpec};
use crate::library::{Definition, FootprintKeepout, PackageModel, PadSide};
use crate::model::Population;
use crate::{Engine, Operation, Result};

fn copper_draft(engine: &mut Engine) -> Result<Fixture> {
    let spec = Spec {
        name: "Split reference",
        terminals: vec![
            (1, "Signal return", passive()),
            (2, "Board return", passive()),
        ],
        units: vec![(1, "A", vec![(1, [-4.0, 0.0]), (2, [4.0, 0.0])])],
        package: vec![(1, "A"), (2, "B")],
        mapping: vec![(1, 1), (2, 2)],
        physical: true,
    };
    let fixture = draft(engine, &spec)?;
    rewrite(engine, fixture.component, |d| {
        if let Definition::Component(c) = d {
            c.assembly = TieForm::Copper.assembly();
            c.tie = Some(TieSpec {
                form: TieForm::Copper,
                ends: vec![TerminalId(1), TerminalId(2)],
            });
        }
    })?;
    rewrite(engine, fixture.package, |d| {
        if let Definition::Package(p) = d {
            p.model = PackageModel::NoBody;
        }
    })?;
    Ok(fixture)
}

fn add_bridge(engine: &mut Engine, fixture: Fixture, far_x: f64) -> Result<()> {
    rewrite(engine, fixture.footprint, |d| {
        if let Definition::Footprint(fp) = d {
            fp.bridges = vec![Bridge {
                id: BridgeId(3),
                side: PadSide::Top,
                pads: vec![PadId(1), PadId(2)],
                outline: FootprintKeepout::rect("", [-1.5, -0.1], [far_x, 0.1]).outline,
            }];
            fp.next_local = 4;
        }
    })
}

#[test]
fn publishing_checks_physical_contact_and_assembly_is_typed() -> Result<()> {
    let mut engine = Engine::demo()?;
    let fixture = copper_draft(&mut engine)?;
    assert!(publish(&mut engine, fixture.all()).is_err());
    add_bridge(&mut engine, fixture, -0.75)?;
    let before = Snapshot::of(&engine);
    assert!(publish(&mut engine, fixture.all()).is_err());
    before.assert_kept(&engine, "bridge misses the second land");
    add_bridge(&mut engine, fixture, 0.0)?;
    publish(&mut engine, fixture.all())?;
    let (id, _) = place(&mut engine, &fixture, "NT1", 1, [45.0, 20.0])?;
    let project = engine.project();
    assert!(!crate::library::mechanical::in_bom(
        &project.library,
        fixture.r1()
    ));
    assert!(!crate::library::mechanical::in_pick_and_place(
        &project.library,
        fixture.r1()
    ));
    let before = Snapshot::of(&engine);
    let result = apply(
        &mut engine,
        Operation::Variant(VariantCommand::SetPopulation {
            instances: vec![id],
            variant: None,
            population: Some(Population::NotPopulated),
        }),
    );
    assert!(result.is_err());
    before.assert_kept(&engine, "opening a copper tie");
    Ok(())
}

#[test]
fn a_bridge_records_both_islands_without_merging_them() -> Result<()> {
    let mut engine = Engine::demo()?;
    let fixture = copper_draft(&mut engine)?;
    add_bridge(&mut engine, fixture, 0.0)?;
    publish(&mut engine, fixture.all())?;
    let (id, _) = place(&mut engine, &fixture, "NT1", 1, [45.0, 20.0])?;
    sync_onto_board(&mut engine)?;
    let graph = crate::layout::CopperGraph::of(engine.project());
    let join = graph
        .ties()
        .iter()
        .find(|b| b.bridge.instance == id)
        .ok_or_else(|| crate::Error::NotFound("Expected the bridge of NT1".into()))?;
    assert_eq!(join.bridge.nets.len(), 2);
    assert_eq!(join.islands.len(), 2);
    assert_ne!(join.islands[0].island, join.islands[1].island);
    Ok(())
}

#[test]
fn the_erc_network_sees_parallel_closed_ties_as_a_loop() {
    use crate::instances::ties::{Network, TieRow};
    use crate::library::tie::TiePurpose;
    let row = |instance, nets: [u64; 2], state| TieRow {
        instance,
        designator: format!("NT{instance}"),
        form: TieForm::Copper,
        purpose: TiePurpose::StarGround,
        state,
        ends: [
            (TerminalId(1), Some(nets[0])),
            (TerminalId(2), Some(nets[1])),
        ]
        .into(),
    };
    use crate::library::tie::JumperState::{Closed, Open};
    let first = row(1, [11, 12], Closed);
    let second = row(2, [12, 13], Closed);
    let network = Network::of(&[first.clone(), second.clone(), row(3, [13, 11], Closed)]);
    assert_eq!(network.loops, [1, 2, 3].into());
    assert_eq!(network.group(13), network.group(11));
    let network = Network::of(&[first, second, row(3, [13, 11], Open)]);
    assert!(network.loops.is_empty());
}

#[test]
fn an_ordinary_copper_tie_rejects_an_anchor_without_changing_the_document() -> Result<()> {
    let mut engine = Engine::demo()?;
    let fixture = copper_draft(&mut engine)?;
    add_bridge(&mut engine, fixture, 0.0)?;
    publish(&mut engine, fixture.all())?;
    let (id, _) = place(&mut engine, &fixture, "NT1", 1, [45.0, 20.0])?;
    let before = Snapshot::of(&engine);
    assert!(
        apply(
            &mut engine,
            Operation::Instance(crate::InstanceCommand::SetAnchor {
                instance: id,
                end: TerminalId(1),
                anchor: Some(crate::library::tie::Anchor {
                    instance: id,
                    pad: PadId(1)
                })
            })
        )
        .is_err()
    );
    before.assert_kept(&engine, "self anchor");
    Ok(())
}
