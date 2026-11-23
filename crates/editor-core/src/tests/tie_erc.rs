//! ERC follows closed tie groups while their physical nets stay distinct.
use super::support::*;
use crate::ids::{BridgeId, PadId, TerminalId, TieId};
use crate::instances::variants::VariantCommand;
use crate::layout::{CopperGraph, LayoutCommand};
use crate::library::tie::{Bridge, JumperBridge, JumperState, TieForm, TieSpec};
use crate::library::{Definition, FootprintKeepout, PackageModel, PadSide};
use crate::model::{Layer, Population};
use crate::{Endpoint, Engine, Error, Id, ObjectRef, Operation, Result};

fn two_groups(engine: &mut Engine, form: TieForm) -> Result<Fixture> {
    let spec = Spec {
        name: "Independent pairs",
        terminals: vec![
            (1, "A", passive()),
            (2, "B", passive()),
            (3, "C", passive()),
            (4, "D", passive()),
        ],
        units: vec![(
            1,
            "A",
            vec![
                (1, [-4., -4.]),
                (2, [4., -4.]),
                (3, [-4., 4.]),
                (4, [4., 4.]),
            ],
        )],
        package: vec![(1, "1"), (2, "2"), (3, "3"), (4, "4")],
        mapping: vec![(1, 1), (2, 2), (3, 3), (4, 4)],
        physical: true,
    };
    let f = draft(engine, &spec)?;
    rewrite(engine, f.component, |d| {
        if let Definition::Component(c) = d {
            c.assembly = form.assembly();
            c.ties = [(1, [1, 2]), (2, [3, 4])]
                .map(|(id, ends)| TieSpec {
                    id: TieId(id),
                    ends: ends.map(TerminalId).to_vec(),
                    form,
                })
                .to_vec();
        }
    })?;
    rewrite(engine, f.package, |d| {
        if let Definition::Package(p) = d {
            p.model = PackageModel::NoBody;
        }
    })?;
    if form.needs_bridge() {
        rewrite(engine, f.footprint, |d| {
            if let Definition::Footprint(fp) = d {
                for (pair, x) in [([1, 2], -1.5), ([3, 4], 1.5)] {
                    fp.bridges.push(Bridge {
                        id: BridgeId(fp.next_local),
                        pads: pair.map(PadId).to_vec(),
                        side: PadSide::Top,
                        outline: FootprintKeepout::rect("", [x, -0.1], [x + 1.5, 0.1]).outline,
                    });
                    fp.next_local += 1;
                }
            }
        })?;
    }
    publish(engine, f.all())?;
    Ok(f)
}

fn undriven(engine: &Engine) -> Vec<Endpoint> {
    crate::erc::checks(engine.project())
        .into_iter()
        .filter(|f| f.code == "erc.undriven_power_input")
        .flat_map(|f| f.objects)
        .filter_map(|o| match o {
            ObjectRef::Terminal { endpoint } => Some(endpoint),
            _ => None,
        })
        .collect()
}

#[test]
fn closed_jumper_drives_only_its_group_and_opening_it_is_undoable() -> Result<()> {
    let mut engine = Engine::default();
    let tie = two_groups(
        &mut engine,
        TieForm::SolderJumper {
            default: JumperState::Closed,
            bridge: JumperBridge::Solder,
        },
    )?;
    let (jp, _) = place(&mut engine, &tie, "JP1", 1, [40., 24.])?;
    let supply = component(&mut engine, &probe("Supply", power_out()))?;
    let load = component(&mut engine, &probe("Load", power_in()))?;
    let tap = component(&mut engine, &probe("Contact", passive()))?;
    let (src, _) = place(&mut engine, &supply, "U1", 1, [24., 20.])?;
    let (fed, _) = place(&mut engine, &load, "U2", 1, [56., 20.])?;
    let (contact, _) = place(&mut engine, &tap, "TP1", 1, [24., 28.])?;
    let (isolated, _) = place(&mut engine, &load, "U3", 1, [56., 28.])?;
    for (n, node) in [(1, src), (2, fed), (3, contact), (4, isolated)] {
        connect(&mut engine, ep(jp, n), ep(node, 1))?;
    }
    let nets = engine.project().nets.clone();
    assert_eq!(nets.len(), 4);
    assert_eq!(undriven(&engine), [ep(isolated, 1)]);
    apply(
        &mut engine,
        Operation::Variant(VariantCommand::SetPopulation {
            instances: vec![jp],
            variant: None,
            population: Some(Population::NotPopulated),
        }),
    )?;
    assert_eq!(undriven(&engine), [ep(fed, 1), ep(isolated, 1)]);
    assert_eq!(engine.project().nets, nets);
    let bytes = crate::format::encode(&engine.to_file())?;
    let reopened = Engine::from_file(crate::format::decode(&bytes)?)?;
    let crate::QueryResult::Ties(reports) = reopened.query(crate::Query::Ties {})? else {
        return Err(Error::Invalid("expected the typed tie query".into()));
    };
    assert_eq!(reports.len(), 2);
    for (row, group) in reports.iter().zip([1, 2]) {
        assert_eq!(row.tie.instance, jp);
        assert_eq!(row.tie.designator, "JP1");
        assert_eq!(row.tie.group, TieId(group));
        assert_eq!(row.tie.state, JumperState::Open);
    }
    assert_eq!(reopened.project().nets, nets);
    engine.undo()?;
    assert_eq!(undriven(&engine), [ep(isolated, 1)]);
    assert_eq!(engine.project().nets, nets);
    Ok(())
}

fn centre(engine: &Engine, instance: Id, pad: u32) -> Result<[f64; 2]> {
    crate::layout::placed_pads(engine.project())
        .into_iter()
        .find(|p| p.instance == instance && p.pad.id == PadId(pad))
        .map(|p| p.center())
        .ok_or_else(|| Error::NotFound(format!("missing pad {pad} of instance {instance}")))
}

#[test]
fn unconnected_jumper_groups_have_separate_board_locations() -> Result<()> {
    let mut engine = Engine::profiled()?;
    let fixture = two_groups(
        &mut engine,
        TieForm::SolderJumper {
            default: JumperState::Open,
            bridge: JumperBridge::Solder,
        },
    )?;
    let (id, _) = place(&mut engine, &fixture, "JP1", 1, [40., 24.])?;
    sync_onto_board(&mut engine)?;
    let reports = crate::layout::connectivity::report(engine.project(), &engine.copper_graph());
    assert_eq!(reports.len(), 2);
    for (row, ends) in reports.iter().zip([[1, 2], [3, 4]]) {
        let a = centre(&engine, id, ends[0])?;
        let b = centre(&engine, id, ends[1])?;
        let expected = [(a[0] + b[0]) / 2., (a[1] + b[1]) / 2.];
        assert_eq!(row.joins.len(), 1);
        assert_eq!(row.joins[0].at, expected);
        assert!(row.joins[0].islands.is_empty());
    }
    assert_ne!(reports[0].joins[0].at, reports[1].joins[0].at);
    Ok(())
}

#[test]
fn foreign_pad_on_first_bridge_reports_the_contact_and_undo_restores_both_groups() -> Result<()> {
    let mut engine = Engine::profiled()?;
    let tie = two_groups(&mut engine, TieForm::Copper)?;
    let (nt, _) = place(&mut engine, &tie, "NT1", 1, [40., 24.])?;
    let contact = component(&mut engine, &probe("Contact", passive()))?;
    let mut probes = Vec::new();
    for (n, name, at) in [
        (1, "TP1", [24., 20.]),
        (2, "TP2", [56., 20.]),
        (3, "TP3", [24., 28.]),
        (4, "TP4", [56., 28.]),
    ] {
        let (id, _) = place(&mut engine, &contact, name, 1, at)?;
        connect(&mut engine, ep(id, 1), ep(nt, n))?;
        probes.push(id);
    }
    sync_onto_board(&mut engine)?;
    let contacts = |engine: &Engine| {
        let graph = CopperGraph::of(engine.project());
        graph
            .ties()
            .iter()
            .filter(|t| t.bridge.instance == nt)
            .map(|t| (t.bridge.group, t.foreign.clone()))
            .collect::<std::collections::BTreeMap<_, _>>()
    };
    let clean = [(TieId(1), vec![]), (TieId(2), vec![])].into();
    assert_eq!(contacts(&engine), clean);
    // Move the third net's contact onto copper midway between the first
    // group's two lands. It touches the bridge alone, away from both pads.
    let foreign = probes[2];
    let [a, b] = [centre(&engine, nt, 1)?, centre(&engine, nt, 2)?];
    let original = centre(&engine, foreign, 1)?;
    let placement = engine
        .project()
        .placement_of(foreign)
        .ok_or_else(|| Error::NotFound("foreign contact".into()))?;
    let position =
        std::array::from_fn(|i| placement.position[i] + (a[i] + b[i]) / 2. - original[i]);
    apply(
        &mut engine,
        Operation::Layout(LayoutCommand::Move {
            instance: foreign,
            position,
            rotation_deg: 0.,
            layer: Layer::Top,
        }),
    )?;
    let offending = ObjectRef::Pad {
        instance: foreign,
        pad: PadId(1),
    };
    assert_eq!(
        contacts(&engine),
        [(TieId(1), vec![offending]), (TieId(2), vec![])].into()
    );
    let findings = CopperGraph::of(engine.project()).findings(engine.project());
    let hits: Vec<_> = findings
        .iter()
        .filter(|f| f.code == "tie.bridge_contact")
        .collect();
    assert_eq!(hits.len(), 1);
    assert!(hits[0].objects.contains(&offending));
    assert!(hits[0].objects.contains(&ObjectRef::Instance { id: nt }));
    engine.undo()?;
    assert_eq!(contacts(&engine), clean);
    assert!(
        CopperGraph::of(engine.project())
            .findings(engine.project())
            .iter()
            .all(|f| f.code != "tie.bridge_contact")
    );
    Ok(())
}
