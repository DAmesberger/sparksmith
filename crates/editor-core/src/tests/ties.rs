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
            c.ties = vec![TieSpec {
                id: crate::ids::TieId(1),
                form: TieForm::Copper,
                ends: vec![TerminalId(1), TerminalId(2)],
            }];
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
    let mut engine = Engine::profiled()?;
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
    let mut engine = Engine::profiled()?;
    let fixture = copper_draft(&mut engine)?;
    add_bridge(&mut engine, fixture, 0.0)?;
    publish(&mut engine, fixture.all())?;
    let (id, _) = place(&mut engine, &fixture, "NT1", 1, [45.0, 20.0])?;
    let terminal = component(&mut engine, &probe("Reference contact", passive()))?;
    let (a, _) = place(&mut engine, &terminal, "TP1", 1, [36., 20.])?;
    let (b, _) = place(&mut engine, &terminal, "TP2", 1, [55., 20.])?;
    connect(&mut engine, ep(a, 1), ep(id, 1))?;
    connect(&mut engine, ep(b, 1), ep(id, 2))?;
    sync_onto_board(&mut engine)?;
    let graph = crate::layout::CopperGraph::of(engine.project());
    let join = graph
        .ties()
        .iter()
        .find(|b| b.bridge.instance == id)
        .ok_or_else(|| crate::Error::NotFound("Expected the bridge of NT1".into()))?;
    assert!(
        join.foreign.is_empty(),
        "An isolated bridge only touches its own lands: {:?}",
        join.foreign
    );
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
        group: crate::ids::TieId(1),
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
    assert_eq!(
        network.loops,
        [1, 2, 3].map(|id| (id, crate::ids::TieId(1))).into()
    );
    assert_eq!(network.group(13), network.group(11));
    let network = Network::of(&[first, second, row(3, [13, 11], Open)]);
    assert!(network.loops.is_empty());
}

#[test]
fn an_ordinary_copper_tie_rejects_an_anchor_without_changing_the_document() -> Result<()> {
    let mut engine = Engine::profiled()?;
    let fixture = copper_draft(&mut engine)?;
    add_bridge(&mut engine, fixture, 0.0)?;
    publish(&mut engine, fixture.all())?;
    let (id, _) = place(&mut engine, &fixture, "NT1", 1, [45.0, 20.0])?;
    let before = Snapshot::of(&engine);
    assert!(
        apply(
            &mut engine,
            Operation::Instance(crate::InstanceCommand::SetAnchor {
                group: None,
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

#[test]
fn current_uses_the_supplied_profile_and_the_placed_neck() -> Result<()> {
    use crate::rules::{
        RulesCommand,
        current::{CurrentPoint, CurrentRule, CurrentTable},
    };
    let mut engine = Engine::profiled()?;
    apply(
        &mut engine,
        Operation::Library(crate::LibraryCommand::ImportStandardFields {}),
    )?;
    let field = engine
        .context()
        .and_then(|c| {
            c.standards()
                .fields
                .local(&engine.project().library, "Rated current")
        })
        .ok_or_else(|| crate::Error::NotFound("Rated current".into()))?;
    let fixture = copper_draft(&mut engine)?;
    rewrite(&mut engine, fixture.component, |d| {
        if let Definition::Component(c) = d {
            c.fields.push(crate::fields::FieldSelection {
                field,
                default: None,
                show_on_symbol: false,
            });
        }
    })?;
    add_bridge(&mut engine, fixture, 0.0)?;
    publish(&mut engine, fixture.all())?;
    let (id, _) = place(&mut engine, &fixture, "NT1", 1, [25., 20.])?;
    sync_onto_board(&mut engine)?;
    set_properties(&mut engine, id, vec![text(field, "2 A")])?;
    let _context = engine.enter_context();
    let code = |engine: &Engine, name: &str| {
        crate::layout::CopperGraph::of(engine.project())
            .findings(engine.project())
            .iter()
            .any(|f| f.code == name)
    };
    assert!(code(&engine, "tie.current_unchecked"));
    let rule = CurrentRule {
        rise_c: Some(7.),
        table: None,
    };
    apply(
        &mut engine,
        Operation::Rules(RulesCommand::SetTieCurrent { rule: Some(rule) }),
    )?;
    let mut project = engine.project().clone();
    let profile = project
        .rules
        .profile
        .as_mut()
        .ok_or_else(|| crate::Error::NotFound("test profile".into()))?;
    profile.current_table = Some(CurrentTable {
        source: "Synthetic test fixture".into(),
        edition: "fixture".into(),
        figure: "A".into(),
        rows: vec![CurrentPoint {
            rise_c: 7.,
            area_mm2: 0.0001,
            amperes: 1.25,
        }],
    });
    let graph = crate::layout::CopperGraph::of(&project);
    let report = crate::layout::connectivity::report(&project, &graph);
    let row = report
        .first()
        .ok_or_else(|| crate::Error::NotFound("missing current report".into()))?;
    let area = row
        .current
        .area_mm2
        .ok_or_else(|| crate::Error::NotFound("missing area".into()))?;
    let expected = 0.2 * project.stackup.copper[0].thickness_um / 1000.;
    assert!(
        (area - expected).abs() < 0.00001,
        "area {area} expected {expected}"
    );
    assert_eq!(row.current.margin_a, Some(-0.75));
    assert!(
        graph
            .findings(&project)
            .iter()
            .any(|f| f.code == "tie.current_exceeded")
    );
    Ok(())
}

#[test]
fn kelvin_contact_allows_the_land_and_reports_a_track_at_the_same_land() -> Result<()> {
    use crate::layout::CopperGraph;
    use crate::model::Track;
    let mut engine = Engine::profiled()?;
    let fixture = copper_draft(&mut engine)?;
    rewrite(&mut engine, fixture.package, |d| {
        if let Definition::Package(p) = d {
            p.terminals.clear()
        }
    })?;
    rewrite(&mut engine, fixture.binding, |d| {
        if let Definition::Binding(b) = d {
            b.mappings.clear()
        }
    })?;
    rewrite(&mut engine, fixture.footprint, |d| {
        if let Definition::Footprint(f) = d {
            f.pads.clear()
        }
    })?;
    publish(&mut engine, fixture.all())?;
    let (id, _) = place(&mut engine, &fixture, "NT1", 1, [25., 20.])?;
    let contact = component(&mut engine, &probe("Contact", passive()))?;
    let (sense, _) = place(&mut engine, &contact, "TP1", 1, [15., 20.])?;
    let (power, _) = place(&mut engine, &contact, "TP2", 1, [35., 20.])?;
    connect(&mut engine, ep(id, 1), ep(sense, 1))?;
    connect(&mut engine, ep(id, 2), ep(power, 1))?;
    sync_onto_board(&mut engine)?;
    apply(
        &mut engine,
        Operation::Instance(crate::InstanceCommand::SetAnchor {
            instance: id,
            group: None,
            end: TerminalId(1),
            anchor: Some(crate::library::tie::Anchor {
                instance: power,
                pad: PadId(1),
            }),
        }),
    )?;
    let mut project = engine.project().clone();
    let sense_net = project
        .net_of(ep(id, 1))
        .ok_or_else(|| crate::Error::NotFound("sense net".into()))?
        .id;
    let land = crate::layout::pads(&project)
        .into_iter()
        .find(|p| p.instance == power)
        .ok_or_else(|| crate::Error::NotFound("power land".into()))?;
    let track = Track {
        id: project.allocate()?,
        net: sense_net,
        layer: land.layer.into(),
        start: [land.position[0], land.position[1] - 2.],
        end: land.position,
        width: 0.1,
        locked: false,
    };
    project.tracks.push(track);
    let graph = CopperGraph::of(&project);
    assert_eq!(
        graph.anchor_points(id, TerminalId(1)),
        &[(land.position, land.layer.into())]
    );
    assert!(
        graph
            .findings(&project)
            .iter()
            .all(|f| f.code != "tie.kelvin_off_anchor")
    );
    let power_track = Track {
        id: project.allocate()?,
        net: land.net,
        layer: land.layer.into(),
        start: land.position,
        end: [land.position[0], land.position[1] + 2.],
        width: 0.1,
        locked: false,
    };
    let offending = power_track.id;
    project.tracks.push(power_track);
    let graph = CopperGraph::of(&project);
    assert!(graph.findings(&project).iter().any(|f| {
        f.code == "tie.kelvin_off_anchor"
            && f.objects
                .contains(&crate::ObjectRef::Track { id: offending })
    }));
    Ok(())
}
