//! Multi-sheet capture through public commands: labels and power ports
//! joining nets by name, the naming rule and rename propagation, sheets and
//! the hierarchy, annotation across sheets, board sync, ERC findings, the
//! scene's title block and persistence (docs/schematic-capture.md).
use super::support::*;
use crate::model::{
    LabelKind, Paper, PaperOrientation, PortShape, PowerStyle, ProjectProperties, SheetItem,
    SheetSide,
};
use crate::schematic::{
    AnnotateMode, AnnotateOrder, Numbering, SchematicCommand as C, WirePoint, WireTarget, scene,
};
use crate::updates::Change;
use crate::{
    Endpoint, Engine, Error, Id, ObjectRef, Operation, Outcome, Point, Query, QueryResult, Result,
    SheetId, WireEnd, erc, format, layout,
};

/// Two passive terminals, pins at x = ±5 mm of the unit origin, with a
/// footprint so the board can place it; prefix U.
fn two_pin(engine: &mut Engine) -> Result<Fixture> {
    component(engine, &two_pin_spec())
}

fn two_pin_spec() -> Spec<'static> {
    Spec {
        name: "Two terminal",
        terminals: vec![(1, "A", passive()), (2, "B", passive())],
        units: vec![(1, "A", vec![(1, [-5.0, 0.0]), (2, [5.0, 0.0])])],
        package: vec![(1, "1"), (2, "2")],
        mapping: vec![(1, 1), (2, 2)],
        physical: true,
    }
}

fn op(command: C) -> Operation {
    Operation::Schematic(command)
}

fn run(engine: &mut Engine, command: C) -> Result<Outcome> {
    apply(engine, op(command))
}

fn add_sheet(engine: &mut Engine, name: &str) -> Result<SheetId> {
    match run(
        engine,
        C::AddSheet {
            name: Some(name.into()),
            paper: None,
            orientation: None,
        },
    )? {
        Outcome::SheetAdded { sheet } => Ok(sheet),
        other => Err(Error::Invalid(format!("unexpected {other:?}"))),
    }
}

fn place_on(
    engine: &mut Engine,
    f: &Fixture,
    designator: &str,
    sheet: SheetId,
    at: Point,
) -> Result<Id> {
    match apply(
        engine,
        Operation::Instance(crate::InstanceCommand::Place(
            crate::instances::NewInstance {
                sheet: Some(sheet),
                component: f.r1(),
                designator: Some(designator.into()),
                properties: vec![],
                representation: Some(f.symbol),
                unit: Some(crate::UnitId(1)),
                position: Some(pt(at)),
                physical: Some(f.physical()),
            },
        )),
    )? {
        Outcome::InstancePlaced { instance, .. } => Ok(instance),
        other => Err(Error::Invalid(format!("unexpected {other:?}"))),
    }
}

fn label(
    engine: &mut Engine,
    sheet: SheetId,
    name: &str,
    kind: LabelKind,
    at: Point,
    attach: Option<WireTarget>,
) -> Result<Id> {
    match run(
        engine,
        C::PlaceLabel {
            sheet: Some(sheet),
            name: name.into(),
            kind,
            position: pt(at),
            rotation: Default::default(),
            attach,
        },
    )? {
        Outcome::LabelPlaced { label } => Ok(label),
        other => Err(Error::Invalid(format!("unexpected {other:?}"))),
    }
}

fn global() -> LabelKind {
    LabelKind::Global {
        shape: PortShape::Bidirectional,
    }
}

fn hier() -> LabelKind {
    LabelKind::Hierarchical {
        shape: PortShape::Input,
    }
}

fn power(style: PowerStyle) -> LabelKind {
    LabelKind::Power { style }
}

fn at(e: Endpoint) -> Option<WireTarget> {
    Some(WireTarget::Terminal(e))
}

fn net_name(engine: &Engine, e: Endpoint) -> Result<String> {
    let net = net_of(engine, e)?;
    Ok(engine.project().net_name(net).to_owned())
}

/// U1 on sheet 1 and U2 on a second sheet "Power", unconnected.
struct Two {
    engine: Engine,
    f: Fixture,
    s1: SheetId,
    s2: SheetId,
    u1: Id,
    u2: Id,
}

fn two_sheets() -> Result<Two> {
    let mut engine = Engine::default();
    let f = two_pin(&mut engine)?;
    let s1 = engine.project().sheets.first();
    let s2 = add_sheet(&mut engine, "Power")?;
    let u1 = place_on(&mut engine, &f, "U1", s1, [20.0, -20.0])?;
    let u2 = place_on(&mut engine, &f, "U2", s2, [20.0, -20.0])?;
    Ok(Two {
        engine,
        f,
        s1,
        s2,
        u1,
        u2,
    })
}

#[test]
fn global_labels_join_nets_across_sheets_one_undo_step_each() -> Result<()> {
    let Two {
        mut engine,
        s1,
        s2,
        u1,
        u2,
        ..
    } = two_sheets()?;
    assert_ne!(net_of(&engine, ep(u1, 2))?, net_of(&engine, ep(u2, 1))?);
    label(
        &mut engine,
        s1,
        "SDA",
        global(),
        [30.0, -20.0],
        at(ep(u1, 2)),
    )?;
    let before = engine.revision();
    label(
        &mut engine,
        s2,
        "SDA",
        global(),
        [10.0, -20.0],
        at(ep(u2, 1)),
    )?;
    assert_eq!(engine.revision(), before + 1, "one step");
    let joined = net_of(&engine, ep(u1, 2))?;
    assert_eq!(joined, net_of(&engine, ep(u2, 1))?);
    assert_eq!(net_name(&engine, ep(u1, 2))?, "SDA");
    // Each label is wired to its terminal by one wire on its own sheet.
    for wire in &engine.project().wires {
        let sheets = (
            crate::schematic::end_sheet(engine.project(), wire.from),
            crate::schematic::end_sheet(engine.project(), wire.to),
        );
        assert_eq!(sheets.0, sheets.1);
    }
    engine.undo()?;
    assert_ne!(net_of(&engine, ep(u1, 2))?, net_of(&engine, ep(u2, 1))?);
    engine.redo()?;
    assert_eq!(net_of(&engine, ep(u1, 2))?, net_of(&engine, ep(u2, 1))?);
    // The net keeps one ID however it was joined.
    assert_eq!(net_of(&engine, ep(u1, 2))?, joined);
    Ok(())
}

#[test]
fn a_wire_cannot_cross_sheets_and_a_refusal_changes_nothing() -> Result<()> {
    let Two {
        mut engine, u1, u2, ..
    } = two_sheets()?;
    let kept = Snapshot::of(&engine);
    let refused = connect(&mut engine, ep(u1, 2), ep(u2, 1));
    assert!(matches!(refused, Err(Error::Invalid(_))), "{refused:?}");
    kept.assert_kept(&engine, "cross-sheet wire");
    Ok(())
}

#[test]
fn local_labels_join_on_their_sheet_only_and_shared_names_are_qualified() -> Result<()> {
    let Two {
        mut engine,
        f,
        s1,
        s2,
        u1,
        u2,
    } = two_sheets()?;
    let u3 = place_on(&mut engine, &f, "U3", s1, [60.0, -20.0])?;
    label(
        &mut engine,
        s1,
        "EN",
        LabelKind::Local,
        [0.0, 0.0],
        at(ep(u1, 2)),
    )?;
    label(
        &mut engine,
        s1,
        "EN",
        LabelKind::Local,
        [0.0, 0.0],
        at(ep(u3, 1)),
    )?;
    assert_eq!(net_of(&engine, ep(u1, 2))?, net_of(&engine, ep(u3, 1))?);
    assert_eq!(net_name(&engine, ep(u1, 2))?, "EN");
    label(
        &mut engine,
        s2,
        "EN",
        LabelKind::Local,
        [0.0, 0.0],
        at(ep(u2, 1)),
    )?;
    assert_ne!(net_of(&engine, ep(u1, 2))?, net_of(&engine, ep(u2, 1))?);
    // Two nets carry EN: each is qualified by its sheet's name.
    assert_eq!(net_name(&engine, ep(u1, 2))?, "Sheet_1/EN");
    assert_eq!(net_name(&engine, ep(u2, 1))?, "Power/EN");
    Ok(())
}

#[test]
fn power_ports_win_the_name_and_conflicting_globals_warn() -> Result<()> {
    let Two {
        mut engine,
        s1,
        s2,
        u1,
        u2,
        ..
    } = two_sheets()?;
    label(
        &mut engine,
        s1,
        "AGND",
        LabelKind::Local,
        [0.0, 0.0],
        at(ep(u1, 1)),
    )?;
    label(
        &mut engine,
        s1,
        "GND",
        power(PowerStyle::Ground),
        [0.0, 0.0],
        at(ep(u1, 1)),
    )?;
    label(
        &mut engine,
        s2,
        "gnd",
        power(PowerStyle::Ground),
        [0.0, 0.0],
        at(ep(u2, 1)),
    )?;
    // Power names compare case-insensitively: one net, named by the port
    // on the earlier page.
    assert_eq!(net_of(&engine, ep(u1, 1))?, net_of(&engine, ep(u2, 1))?);
    assert_eq!(net_name(&engine, ep(u1, 1))?, "GND");
    let conflicts = |e: &Engine| {
        erc::checks(e.project())
            .into_iter()
            .filter(|f| f.code == "erc.label_conflict")
            .count()
    };
    assert_eq!(conflicts(&engine), 0, "a local name under a power name");
    label(&mut engine, s2, "VSS", global(), [0.0, 0.0], at(ep(u2, 1)))?;
    assert_eq!(conflicts(&engine), 1);
    assert_eq!(net_name(&engine, ep(u1, 1))?, "GND", "power beats global");
    Ok(())
}

#[test]
fn a_label_on_a_wire_splits_it_and_removing_it_keeps_the_wires_joined() -> Result<()> {
    let mut engine = Engine::default();
    let f = two_pin(&mut engine)?;
    let s1 = engine.project().sheets.first();
    let u1 = place_on(&mut engine, &f, "U1", s1, [0.0, 0.0])?;
    let u2 = place_on(&mut engine, &f, "U2", s1, [30.0, 0.0])?;
    connect(&mut engine, ep(u1, 2), ep(u2, 1))?;
    let wire = wire_between(&engine, ep(u1, 2), ep(u2, 1))?;
    let id = label(
        &mut engine,
        s1,
        "MID",
        LabelKind::Local,
        [99.0, 99.0],
        Some(WireTarget::OnWire(WirePoint {
            wire,
            at: pt([15.0, 0.0]),
        })),
    )?;
    let project = engine.project();
    assert_eq!(project.label(id).map(|l| l.position), Some(pt([15.0, 0.0])));
    assert_eq!(project.wires.len(), 2, "split at the label");
    assert_eq!(project.wires_at(WireEnd::label(id)).count(), 2);
    assert_eq!(net_name(&engine, ep(u1, 2))?, "MID");
    run(&mut engine, C::RemoveLabel { label: id })?;
    let project = engine.project();
    assert!(project.labels.is_empty());
    // The two halves merge back into one wire. A junction with two wires
    // used to stay as a bend point; now a bend is a corner, and this one
    // is in line, so it goes too.
    assert!(project.junctions.is_empty());
    assert_eq!(project.wires.len(), 1);
    assert!(project.wires[0].corners.is_empty());
    assert_eq!(net_of(&engine, ep(u1, 2))?, net_of(&engine, ep(u2, 1))?);
    // The net keeps the name the label gave it.
    assert_eq!(net_name(&engine, ep(u1, 2))?, "MID");
    Ok(())
}

#[test]
fn renaming_a_labelled_net_renames_its_labels_and_keeps_its_id() -> Result<()> {
    let Two {
        mut engine,
        f,
        s1,
        s2,
        u1,
        u2,
    } = two_sheets()?;
    let a = label(&mut engine, s1, "SDA", global(), [0.0, 0.0], at(ep(u1, 2)))?;
    let b = label(&mut engine, s2, "SDA", global(), [0.0, 0.0], at(ep(u2, 1)))?;
    let net = net_of(&engine, ep(u1, 2))?;
    run(
        &mut engine,
        C::RenameNet {
            net,
            name: "I2C_SDA".into(),
        },
    )?;
    let project = engine.project();
    assert_eq!(project.net_name(net), "I2C_SDA");
    for id in [a, b] {
        assert_eq!(project.label(id).map(|l| l.name.as_str()), Some("I2C_SDA"));
    }
    // Renaming onto another net's name is refused, never a merge.
    let u3 = place_on(&mut engine, &f, "U3", s1, [60.0, -20.0])?;
    let other = net_of(&engine, ep(u3, 1))?;
    run(
        &mut engine,
        C::RenameNet {
            net: other,
            name: "SCL".into(),
        },
    )?;
    let kept = Snapshot::of(&engine);
    let refused = run(
        &mut engine,
        C::RenameNet {
            net,
            name: "scl".into(),
        },
    );
    assert!(matches!(refused, Err(Error::Conflict(_))), "{refused:?}");
    kept.assert_kept(&engine, "rename onto another net");
    // Renaming one label onto another net's name joins them, but only
    // deliberately: directly it is refused, and a reviewed update that asks
    // for the merge joins them.
    label(
        &mut engine,
        s1,
        "SCL",
        global(),
        [10.0, -10.0],
        at(ep(u3, 1)),
    )?;
    let rename = C::EditLabel {
        label: a,
        name: Some("SCL".into()),
        kind: None,
        rotation: None,
    };
    let kept = Snapshot::of(&engine);
    let refused = run(&mut engine, rename.clone());
    assert!(
        matches!(&refused, Err(Error::Conflict(m)) if m.starts_with("net.named_merge:")),
        "{refused:?}"
    );
    kept.assert_kept(&engine, "a rename that merges named nets");
    let revision = engine.revision();
    apply(
        &mut engine,
        Operation::Update(Box::new(crate::updates::ApplyUpdate {
            expected_revision: revision,
            change: Change::Schematic { command: rename },
            remove_conflicting_tracks: false,
            merge_named_nets: true,
        })),
    )?;
    assert_eq!(net_of(&engine, ep(u1, 2))?, net_of(&engine, ep(u3, 1))?);
    engine.undo()?;
    assert_ne!(net_of(&engine, ep(u1, 2))?, net_of(&engine, ep(u3, 1))?);
    Ok(())
}

/// U1 on sheet 1 wired to pin IN of a sheet symbol whose child sheet holds
/// U2 and a hierarchical label IN on U2.1.
struct Hier {
    engine: Engine,
    child: SheetId,
    symbol: Id,
    pin: Id,
    port: Id,
    u1: Id,
    u2: Id,
}

fn hierarchy() -> Result<Hier> {
    let mut engine = Engine::default();
    let f = two_pin(&mut engine)?;
    let s1 = engine.project().sheets.first();
    let u1 = place_on(&mut engine, &f, "U1", s1, [20.0, -40.0])?;
    let (symbol, child) = match run(
        &mut engine,
        C::PlaceSheetSymbol {
            sheet: None,
            child: None,
            name: Some("Sensor".into()),
            position: pt([60.0, -30.0]),
            size: pt([30.0, 20.0]),
        },
    )? {
        Outcome::SheetSymbolPlaced { symbol, child } => (symbol, child),
        other => return Err(Error::Invalid(format!("unexpected {other:?}"))),
    };
    let u2 = place_on(&mut engine, &f, "U2", child, [40.0, -40.0])?;
    let port = label(
        &mut engine,
        child,
        "IN",
        hier(),
        [20.0, -40.0],
        at(ep(u2, 1)),
    )?;
    let pins = match run(&mut engine, C::SyncSheetPins { symbol })? {
        Outcome::SheetPinsAdded { pins } => pins,
        other => return Err(Error::Invalid(format!("unexpected {other:?}"))),
    };
    let [pin] = pins[..] else {
        return Err(Error::Invalid(format!("pins {pins:?}")));
    };
    run(
        &mut engine,
        C::Connect {
            from: ep(u1, 2).into(),
            to: WireTarget::SheetPin(crate::model::SheetPinRef { sheet_pin: pin }),
        },
    )?;
    Ok(Hier {
        engine,
        child,
        symbol,
        pin,
        port,
        u1,
        u2,
    })
}

#[test]
fn sheet_pins_join_hierarchical_labels_on_the_child_sheet() -> Result<()> {
    let Hier {
        mut engine,
        child,
        port,
        u1,
        u2,
        ..
    } = hierarchy()?;
    assert_eq!(net_of(&engine, ep(u1, 2))?, net_of(&engine, ep(u2, 1))?);
    assert_eq!(net_name(&engine, ep(u1, 2))?, "IN");
    let mismatches = |e: &Engine| {
        codes(&erc::checks(e.project()))
            .into_iter()
            .filter(|(c, _)| c == "erc.hierarchy_mismatch")
            .collect::<Vec<_>>()
    };
    assert!(mismatches(&engine).is_empty());
    // Rename the net: the pin and the label behind it rename together.
    let net = net_of(&engine, ep(u1, 2))?;
    run(
        &mut engine,
        C::RenameNet {
            net,
            name: "SENSE".into(),
        },
    )?;
    assert_eq!(net_of(&engine, ep(u2, 1))?, net);
    assert_eq!(
        engine.project().label(port).map(|l| l.name.as_str()),
        Some("SENSE")
    );
    // Without its label the pin joins nothing on the child sheet.
    run(&mut engine, C::RemoveLabel { label: port })?;
    assert_ne!(net_of(&engine, ep(u1, 2))?, net_of(&engine, ep(u2, 1))?);
    let found = mismatches(&engine);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(matches!(found[0].1[..], [ObjectRef::SheetPin { .. }]));
    // A hierarchical label with no pin is the other mismatch.
    label(&mut engine, child, "OUT", hier(), [0.0, 0.0], None)?;
    assert_eq!(mismatches(&engine).len(), 2);
    Ok(())
}

#[test]
fn the_hierarchy_refuses_cycles() -> Result<()> {
    let Hier {
        mut engine, child, ..
    } = hierarchy()?;
    let top = engine.project().sheets.first();
    let kept = Snapshot::of(&engine);
    let cycle = run(
        &mut engine,
        C::PlaceSheetSymbol {
            sheet: Some(child),
            child: Some(top),
            name: None,
            position: pt([0.0, 0.0]),
            size: pt([20.0, 20.0]),
        },
    );
    assert!(matches!(cycle, Err(Error::Conflict(_))), "{cycle:?}");
    kept.assert_kept(&engine, "refused sheet symbols");
    Ok(())
}

#[test]
fn removing_a_sheet_symbol_or_deleting_a_sheet_keeps_a_valid_document() -> Result<()> {
    let Hier {
        mut engine,
        child,
        symbol,
        pin,
        u1,
        u2,
        ..
    } = hierarchy()?;
    let before = engine.revision();
    run(&mut engine, C::RemoveSheetSymbol { symbol })?;
    assert_eq!(engine.revision(), before + 1);
    assert!(engine.project().sheet_pin(pin).is_none());
    assert!(
        engine.project().sheet(child).is_some(),
        "definition retained"
    );
    assert!(engine.project().instance(u2).is_none());
    assert!(engine.project().net_of(ep(u2, 1)).is_none());
    assert!(engine.project().net_of(ep(u1, 2)).is_some());
    engine.undo()?;
    run(&mut engine, C::DeleteSheet { sheet: child })?;
    let project = engine.project();
    assert!(project.sheet(child).is_none());
    assert!(project.sheet_symbol(symbol).is_none());
    assert_eq!(project.occurrences_of(u2).count(), 0, "unplaced");
    assert!(project.instance(u2).is_some(), "the instance stays");
    assert!(project.labels.is_empty());
    let last = project.sheets.first();
    let kept = Snapshot::of(&engine);
    let refused = run(&mut engine, C::DeleteSheet { sheet: last });
    assert!(matches!(refused, Err(Error::Conflict(_))), "{refused:?}");
    kept.assert_kept(&engine, "deleting the last sheet");
    Ok(())
}

#[test]
fn sheets_rename_reorder_and_page_settings() -> Result<()> {
    let Two {
        mut engine, s1, s2, ..
    } = two_sheets()?;
    run(
        &mut engine,
        C::ReorderSheet {
            sheet: s2,
            index: 0,
        },
    )?;
    assert_eq!(engine.project().sheets.number(s2), Some(1));
    let kept = Snapshot::of(&engine);
    let duplicate = run(
        &mut engine,
        C::RenameSheet {
            sheet: s1,
            name: "power".into(),
        },
    );
    assert!(
        matches!(duplicate, Err(Error::Conflict(_))),
        "{duplicate:?}"
    );
    kept.assert_kept(&engine, "duplicate sheet name");
    run(
        &mut engine,
        C::SetPage {
            sheet: s1,
            paper: Paper::A3,
            orientation: PaperOrientation::Portrait,
        },
    )?;
    let sheet = engine
        .project()
        .sheet(s1)
        .ok_or(Error::NotFound("s1".into()))?;
    assert_eq!(sheet.size_mm(), [297.0, 420.0]);
    match engine.query(Query::Sheets {})? {
        QueryResult::Sheets(rows) => {
            assert_eq!(rows.len(), 2);
            assert_eq!(rows[0].id, s2);
            assert_eq!(rows[1].symbols, 1);
        }
        other => return Err(Error::Invalid(format!("unexpected {other:?}"))),
    }
    Ok(())
}

/// A sheet added without a name is numbered after the sheets there are:
/// the third is "Sheet 3", even when the first two have other names
/// (night round 4 Q6), and a taken number moves on to the next.
#[test]
fn an_unnamed_sheet_is_numbered_after_the_existing_ones() -> Result<()> {
    let mut engine = Engine::default();
    let first = engine.project().sheets.first();
    run(
        &mut engine,
        C::RenameSheet {
            sheet: first,
            name: "Main".into(),
        },
    )?;
    add_sheet(&mut engine, "Power")?;
    let unnamed = |engine: &mut Engine| -> Result<String> {
        let Outcome::SheetAdded { sheet } = run(
            engine,
            C::AddSheet {
                name: None,
                paper: None,
                orientation: None,
            },
        )?
        else {
            return Err(Error::Invalid("no sheet added".into()));
        };
        Ok(engine
            .project()
            .sheet(sheet)
            .map(|s| s.name.clone())
            .unwrap_or_default())
    };
    assert_eq!(unnamed(&mut engine)?, "Sheet 3");
    add_sheet(&mut engine, "Sheet 5")?;
    assert_eq!(unnamed(&mut engine)?, "Sheet 6");
    Ok(())
}

#[test]
fn annotation_numbers_across_sheets_and_keeps_existing_designators() -> Result<()> {
    let mut engine = Engine::default();
    let f = two_pin(&mut engine)?;
    let s1 = engine.project().sheets.first();
    let s2 = add_sheet(&mut engine, "Two")?;
    // Page 2 first in creation order; page 1 holds a kept U2 and two new
    // parts, the right one above the left one.
    let late = place_on(&mut engine, &f, "U?a", s2, [10.0, -10.0])?;
    let kept = place_on(&mut engine, &f, "U2", s1, [50.0, -50.0])?;
    let right = place_on(&mut engine, &f, "U?b", s1, [80.0, -10.0])?;
    let left = place_on(&mut engine, &f, "U?c", s1, [10.0, -30.0])?;
    let annotate = |mode, numbering| C::Annotate {
        order: AnnotateOrder::Rows,
        mode,
        numbering,
        sheets: vec![],
    };
    let changes = match run(
        &mut engine,
        annotate(AnnotateMode::Unannotated, Numbering::Sequential),
    )? {
        Outcome::Annotated { changes } => changes,
        other => return Err(Error::Invalid(format!("unexpected {other:?}"))),
    };
    assert_eq!(changes.len(), 3);
    let designator = |e: &Engine, id: Id| {
        e.project()
            .instance(id)
            .map(|i| i.designator.clone())
            .unwrap_or_default()
    };
    // Rows: the top part first; U2 is kept and skipped.
    assert_eq!(designator(&engine, right), "U1");
    assert_eq!(designator(&engine, left), "U3");
    assert_eq!(designator(&engine, late), "U4");
    assert_eq!(designator(&engine, kept), "U2");
    // Annotating again changes nothing and takes no undo step's worth of
    // changes.
    match run(
        &mut engine,
        annotate(AnnotateMode::Unannotated, Numbering::Sequential),
    )? {
        Outcome::Annotated { changes } => assert!(changes.is_empty()),
        other => return Err(Error::Invalid(format!("unexpected {other:?}"))),
    }
    run(
        &mut engine,
        annotate(AnnotateMode::All, Numbering::SheetHundreds),
    )?;
    assert_eq!(designator(&engine, right), "U101");
    assert_eq!(designator(&engine, left), "U102");
    assert_eq!(designator(&engine, kept), "U103");
    assert_eq!(designator(&engine, late), "U201");
    engine.undo()?;
    assert_eq!(designator(&engine, kept), "U2");
    Ok(())
}

#[test]
fn board_sync_places_parts_from_every_sheet_on_one_net() -> Result<()> {
    let Two {
        mut engine,
        s1,
        s2,
        u1,
        u2,
        ..
    } = two_sheets()?;
    label(
        &mut engine,
        s1,
        "VBUS",
        power(PowerStyle::Arrow),
        [0.0, 0.0],
        at(ep(u1, 2)),
    )?;
    label(
        &mut engine,
        s2,
        "VBUS",
        power(PowerStyle::Arrow),
        [0.0, 0.0],
        at(ep(u2, 1)),
    )?;
    let net = net_of(&engine, ep(u1, 2))?;
    engine.apply_update(engine.revision(), Change::SyncPcb {}, false)?;
    let project = engine.project();
    assert!(project.placement_of(u1).is_some() && project.placement_of(u2).is_some());
    let airwires = layout::airwires(project);
    assert!(
        airwires.iter().any(|(a, b)| a.net == net
            && b.net == net
            && [a.instance, b.instance].contains(&u1)
            && [a.instance, b.instance].contains(&u2)),
        "{airwires:?}"
    );
    // Net classes see the label's name.
    assert_eq!(project.net_name(net), "VBUS");
    Ok(())
}

#[test]
fn erc_reports_unwired_single_and_unsupplied_power_labels() -> Result<()> {
    let mut engine = Engine::default();
    let load = component(&mut engine, &probe("Load", power_in()))?;
    let s1 = engine.project().sheets.first();
    let (u1, _) = place(&mut engine, &load, "U1", 1, [0.0, 0.0])?;
    let lonely = label(&mut engine, s1, "LONELY", global(), [50.0, 0.0], None)?;
    label(
        &mut engine,
        s1,
        "+3V3",
        power(PowerStyle::Bar),
        [0.0, 10.0],
        at(ep(u1, 1)),
    )?;
    let found = codes(&erc::checks(engine.project()));
    let has = |code: &str| found.iter().filter(|(c, _)| c == code).count();
    assert_eq!(has("erc.label_unconnected"), 1, "{found:?}");
    assert_eq!(has("erc.global_label_single"), 1, "{found:?}");
    assert!(found.iter().any(|(c, objects)| c == "erc.label_unconnected"
        && objects == &vec![ObjectRef::Label { id: lonely }]));
    // One power-port finding replaces the per-input one.
    assert_eq!(has("erc.power_port_undriven"), 1, "{found:?}");
    assert_eq!(has("erc.undriven_power_input"), 0, "{found:?}");
    // Every new code is classified for Issues.
    let QueryResult::Issues(report) = engine.query(Query::Issues {
        scope: crate::issues::IssueScope::All,
    })?
    else {
        return Err(Error::Invalid("issues".into()));
    };
    assert!(
        report
            .rows
            .iter()
            .any(|r| r.title.contains("LONELY") || r.title.contains("Global label")),
        "{:?}",
        report.rows.iter().map(|r| &r.title).collect::<Vec<_>>()
    );
    Ok(())
}

#[test]
fn net_places_list_a_nets_labels_and_pins_across_sheets() -> Result<()> {
    let Two {
        mut engine,
        s1,
        s2,
        u1,
        u2,
        ..
    } = two_sheets()?;
    label(
        &mut engine,
        s1,
        "SDA",
        global(),
        [40.0, -20.0],
        at(ep(u1, 2)),
    )?;
    label(
        &mut engine,
        s2,
        "SDA",
        global(),
        [0.0, -20.0],
        at(ep(u2, 1)),
    )?;
    let net = net_of(&engine, ep(u1, 2))?;
    let QueryResult::NetPlaces(places) = engine.query(Query::NetPlaces { net })? else {
        return Err(Error::Invalid("places".into()));
    };
    let sheets: Vec<SheetId> = places.places.iter().map(|p| p.sheet).collect();
    // Two labels and two terminals, page 1 first.
    assert_eq!(sheets, vec![s1, s1, s2, s2]);
    Ok(())
}

/// `project.rename`: the name reaches the title block, is one undo step,
/// says nothing new when it is the same name, and a refused name changes
/// nothing.
#[test]
fn renaming_the_project_is_one_undo_step() -> Result<()> {
    let Two { mut engine, s2, .. } = two_sheets()?;
    let was = engine.project().name.clone();
    let start = engine.revision();
    run(
        &mut engine,
        C::RenameProject {
            name: "  ESP32 board  ".into(),
        },
    )?;
    assert_eq!(engine.project().name, "ESP32 board", "trimmed");
    assert_eq!(engine.revision(), start + 1);
    let frame = scene::frame(engine.project(), s2);
    assert!(frame.texts.iter().any(|t| t.text == "ESP32 board"));

    // The same name: no new revision and no undo step.
    run(
        &mut engine,
        C::RenameProject {
            name: "ESP32 board ".into(),
        },
    )?;
    assert_eq!(engine.revision(), start + 1);

    let kept = engine.to_file();
    let revision = engine.revision();
    for bad in [
        String::new(),
        "   ".into(),
        "two\nlines".into(),
        "x".repeat(257),
    ] {
        let refused = run(&mut engine, C::RenameProject { name: bad.clone() });
        assert!(matches!(refused, Err(Error::Invalid(_))), "{bad:?}");
        assert_eq!(engine.revision(), revision);
        assert_eq!(engine.to_file(), kept);
    }
    let longest = "x".repeat(256);
    run(
        &mut engine,
        C::RenameProject {
            name: longest.clone(),
        },
    )?;
    assert_eq!(engine.project().name, longest);
    assert!(
        engine
            .apply(start, op(C::RenameProject { name: "Old".into() }))
            .is_err()
    );
    assert_eq!(engine.project().name, longest, "an outdated revision");

    engine.undo()?;
    assert_eq!(engine.project().name, "ESP32 board");
    engine.undo()?;
    assert_eq!(engine.project().name, was);
    assert_eq!(
        engine.redo().map(|()| engine.project().name.clone())?,
        "ESP32 board"
    );
    Ok(())
}

/// A first save names an Untitled project after its file; a chosen name,
/// a file called Untitled and a name the rename refuses all keep it.
#[test]
fn a_first_save_names_an_untitled_project_after_its_file() {
    use std::path::Path;
    let mut document = crate::Document::default();
    let file = Path::new("/boards/usb-led-e2e.sparksmith");
    let given = crate::first_save(&mut document, file, "");
    assert!(!given.is_empty());
    assert_eq!(document.project.name, "usb-led-e2e");
    // Now a name: a later save under another file keeps it.
    let named = document.clone();
    let again = crate::first_save(&mut document, Path::new("other.sparksmith"), "");
    assert!(again.is_empty());
    assert_eq!(document, named);
    for kept in [
        "Untitled.sparksmith",
        " .sparksmith",
        &format!("{}.sparksmith", "x".repeat(257)),
    ] {
        let mut document = crate::Document::default();
        crate::first_save(&mut document, Path::new(kept), "");
        assert_eq!(document.project.name, crate::UNTITLED, "{kept}");
        assert!(document.project.production.project_id.is_some(), "{kept}");
    }
}

/// The engine takes what a first save gave its copy: the name is one undo
/// step, the identity none. A name the project took while the file was
/// written stays.
#[test]
fn the_engine_takes_what_a_first_save_gave() -> Result<()> {
    use std::path::Path;
    let mut engine = Engine::demo()?;
    assert_eq!(engine.project().name, crate::UNTITLED);
    let mut written = engine.document().clone();
    let given = crate::first_save(&mut written, Path::new("one.sparksmith"), "");
    let (revision, before) = (engine.revision(), engine.document().clone());
    engine.adopt_first_save(&given)?;
    assert_eq!(engine.document(), &written);
    assert_eq!(engine.revision(), revision + 1);
    engine.undo()?;
    assert_eq!(engine.project().name, crate::UNTITLED);
    assert_eq!(
        engine.project().production.project_id,
        written.project.production.project_id
    );

    let mut engine = Engine::demo()?;
    assert_eq!(engine.document(), &before);
    engine.apply(
        engine.revision(),
        Operation::Schematic(C::RenameProject {
            name: "Chosen meanwhile".into(),
        }),
    )?;
    let revision = engine.revision();
    engine.adopt_first_save(&given)?;
    assert_eq!(engine.project().name, "Chosen meanwhile");
    assert_eq!(engine.revision(), revision);
    assert!(engine.project().production.project_id.is_some());
    Ok(())
}

#[test]
fn the_title_block_shows_properties_and_the_page() -> Result<()> {
    let Two { mut engine, s2, .. } = two_sheets()?;
    run(
        &mut engine,
        C::SetProperties {
            properties: ProjectProperties {
                title: String::new(),
                revision: "B".into(),
                date: "2026-11-02".into(),
                author: "A. Designer".into(),
                company: "Example".into(),
            },
        },
    )?;
    let frame = scene::frame(engine.project(), s2);
    let texts: Vec<&str> = frame.texts.iter().map(|t| t.text.as_str()).collect();
    for wanted in [
        "Untitled",
        "B",
        "2026-11-02",
        "A. Designer",
        "2 of 2",
        "A4",
        "Power",
    ] {
        assert!(texts.contains(&wanted), "{wanted} in {texts:?}");
    }
    let page = engine.project().sheet(s2).map(|s| s.size_mm());
    assert_eq!(page, Some([297.0, 210.0]), "A4 landscape");
    let bounds = frame
        .bounds(crate::library::TextMetric::default())
        .ok_or(Error::Invalid("empty frame".into()))?;
    // The page's top-left corner is at (-30, 30) on the sheet.
    assert!(
        bounds.min[0] >= -30.0 && bounds.max[0] <= 267.0,
        "{bounds:?}"
    );
    assert!(
        bounds.min[1] >= -180.0 && bounds.max[1] <= 30.0,
        "{bounds:?}"
    );
    Ok(())
}

#[test]
fn sheets_labels_symbols_and_furniture_survive_save_and_reopen() -> Result<()> {
    let Hier {
        mut engine, child, ..
    } = hierarchy()?;
    run(
        &mut engine,
        C::AddDrawing {
            sheet: Some(child),
            item: SheetItem::Text {
                at: pt([20.0, -100.0]),
                text: "LDO: keep input cap close".into(),
                height_nm: nm(2.5),
                rotation: Default::default(),
            },
        },
    )?;
    run(
        &mut engine,
        C::AddDrawing {
            sheet: None,
            item: SheetItem::Rect {
                min: pt([10.0, -150.0]),
                max: pt([80.0, -120.0]),
            },
        },
    )?;
    let pin = engine
        .project()
        .sheet_symbols
        .first()
        .and_then(|s| s.pins.first())
        .map(|p| p.id)
        .ok_or(Error::NotFound("pin".into()))?;
    run(
        &mut engine,
        C::EditSheetPin {
            pin,
            name: None,
            shape: Some(PortShape::Output),
            side: Some(SheetSide::Right),
            offset: Some(nm(10.0)),
        },
    )?;
    let bytes = format::encode(&engine.to_file())?;
    let reopened = format::decode(&bytes)?;
    assert_eq!(reopened.project, *engine.project());
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("\"sheet_symbols\"") && text.contains("\"sheet_pin\""));
    // A design that uses none of it writes none of it.
    let plain = format::encode(&Engine::default().to_file())?;
    let plain = String::from_utf8_lossy(&plain);
    for absent in ["\"sheets\"", "\"labels\"", "\"drawings\"", "\"sheet\""] {
        assert!(!plain.contains(absent), "{absent}");
    }
    Ok(())
}

#[test]
fn malformed_commands_are_refused_without_changes() -> Result<()> {
    let Two {
        mut engine, s1, u1, ..
    } = two_sheets()?;
    label(
        &mut engine,
        s1,
        "OK",
        LabelKind::Local,
        [0.0, 0.0],
        at(ep(u1, 2)),
    )?;
    let kept = Snapshot::of(&engine);
    for (what, command) in [
        (
            "spaced name",
            C::PlaceLabel {
                sheet: None,
                name: "A B".into(),
                kind: LabelKind::Local,
                position: pt([0.0, 0.0]),
                rotation: Default::default(),
                attach: None,
            },
        ),
        (
            "unknown sheet",
            C::PlaceLabel {
                sheet: Some(SheetId(99)),
                name: "X".into(),
                kind: LabelKind::Local,
                position: pt([0.0, 0.0]),
                rotation: Default::default(),
                attach: None,
            },
        ),
        (
            "unknown label",
            C::MoveLabel {
                label: 9999,
                position: pt([0.0, 0.0]),
            },
        ),
        (
            "tiny symbol",
            C::PlaceSheetSymbol {
                sheet: None,
                child: None,
                name: None,
                position: pt([0.0, 0.0]),
                size: pt([1.0, 1.0]),
            },
        ),
        (
            "empty note",
            C::AddDrawing {
                sheet: None,
                item: SheetItem::Text {
                    at: pt([0.0, 0.0]),
                    text: " ".into(),
                    height_nm: nm(2.0),
                    rotation: Default::default(),
                },
            },
        ),
        (
            "unknown annotate sheet",
            C::Annotate {
                order: AnnotateOrder::Columns,
                mode: AnnotateMode::All,
                numbering: Numbering::Sequential,
                sheets: vec![SheetId(42)],
            },
        ),
    ] {
        let refused = run(&mut engine, command);
        assert!(refused.is_err(), "{what}: {refused:?}");
        kept.assert_kept(&engine, what);
    }
    // JSON with an unknown field is refused at the transport.
    let bad = serde_json::from_value::<Operation>(serde_json::json!({
        "editor": "Schematic",
        "operation": {"command": "label.place", "params": {
            "name": "X", "kind": {"type": "local"}, "position": [0, 0], "colour": "red"}}
    }));
    assert!(bad.is_err());
    // A position not finite, or off the sheet's range, is refused there too.
    for position in [serde_json::json!(["NaN", 0]), serde_json::json!([0, 2001])] {
        let refused = serde_json::from_value::<Operation>(serde_json::json!({
            "editor": "Schematic",
            "operation": {"command": "label.move", "params": {"label": 9999, "position": position}}
        }));
        assert!(refused.is_err(), "{position}");
    }
    let good = serde_json::from_value::<Operation>(serde_json::json!({
        "editor": "Schematic",
        "operation": {"command": "label.place", "params": {
            "name": "GND", "kind": {"type": "power", "style": "ground"}, "position": [0, 0]}}
    }));
    assert!(good.is_ok(), "{good:?}");
    Ok(())
}

/// Beyond the next real design's scale: 240 parts on four sheets, a global
/// label or power port on every terminal, 60 shared names. Every command
/// stays one reconciled, validated step and the names follow the rule.
/// Timing is printed, not asserted (a target, not a gate).
#[test]
fn hundreds_of_labels_across_sheets_stay_consistent() -> Result<()> {
    let mut engine = Engine::default();
    let f = two_pin(&mut engine)?;
    let mut sheets = vec![engine.project().sheets.first()];
    for n in 2..=4 {
        sheets.push(add_sheet(&mut engine, &format!("Page {n}"))?);
    }
    let started = std::time::Instant::now();
    let mut parts = Vec::new();
    for k in 0..240usize {
        let sheet = sheets[k % 4];
        let at_ = [20.0 * (k / 4 % 12) as f64, -20.0 * (k / 48) as f64];
        let id = place_on(&mut engine, &f, &format!("U{}", k + 1), sheet, at_)?;
        label(
            &mut engine,
            sheet,
            &format!("S{}", k % 60),
            global(),
            // Beside each pin: labels at one point would connect.
            [at_[0] - 7.5, at_[1]],
            at(ep(id, 1)),
        )?;
        label(
            &mut engine,
            sheet,
            "GND",
            power(PowerStyle::Ground),
            [at_[0] + 7.5, at_[1]],
            at(ep(id, 2)),
        )?;
        parts.push(id);
    }
    eprintln!("240 parts, 480 labels: {:?}", started.elapsed());
    let project = engine.project();
    // 60 signal nets of 4 terminals and one GND net of 240.
    assert_eq!(project.nets.len(), 61);
    let gnd = net_of(&engine, ep(parts[0], 2))?;
    assert_eq!(project.net_name(gnd), "GND");
    assert_eq!(project.net(gnd).map(|n| n.terminals.len()), Some(240));
    assert_eq!(net_name(&engine, ep(parts[61], 1))?, "S1");
    let started = std::time::Instant::now();
    let findings = erc::checks(project);
    eprintln!("ERC: {:?}", started.elapsed());
    assert!(
        !findings.iter().any(|f| f.code.starts_with("erc.label")),
        "{:?}",
        codes(&findings)
    );
    Ok(())
}

/// A feedback resistor from OUT to FB, and a return resistor from FB to
/// global GND. Only the top sheet names OUT_A and OUT_B.
#[test]
fn repeated_front_ends_have_independent_parts_and_copper() -> Result<()> {
    let mut engine = Engine::default();
    use crate::fields::{FieldDefinition, QuantityKind, ValueType};
    use crate::instances::variants::VariantCommand as V;
    use crate::quantity::{UnitOfMeasure, UnitSet};
    let ohms = publish_kind(
        &mut engine,
        "Resistance",
        QuantityKind {
            units: UnitSet {
                dimension: [2, 1, -3, -2, 0, 0, 0],
                units: vec![UnitOfMeasure {
                    symbol: "Ω".into(),
                    aliases: vec![],
                    scale: 1.,
                    si_prefixes: true,
                }],
            },
            default_unit: "Ω".into(),
        },
    )?;
    let field = publish_field(
        &mut engine,
        "Feedback resistance",
        FieldDefinition {
            next_local: 1,
            value_type: ValueType::Quantity {
                kind: ohms,
                min: None,
                max: None,
            },
            default: None,
            required_at: None,
        },
    )?;
    let resistor = component_with_fields(&mut engine, &two_pin_spec(), vec![select(field)])?;
    let parent = engine.project().sheets.first();
    let front = add_sheet(&mut engine, "Front end")?;
    let r1 = place_on(&mut engine, &resistor, "R1", front, [30., -20.])?;
    let ret = place_on(&mut engine, &resistor, "R2", front, [30., -50.])?;
    set_properties(&mut engine, r1, vec![text(field, "10 kΩ")])?;
    set_properties(&mut engine, ret, vec![text(field, "1 kΩ")])?;
    label(
        &mut engine,
        front,
        "OUT",
        hier(),
        [10., -20.],
        at(ep(r1, 1)),
    )?;
    label(
        &mut engine,
        front,
        "FB",
        LabelKind::Local,
        [45., -20.],
        at(ep(r1, 2)),
    )?;
    label(
        &mut engine,
        front,
        "FB",
        LabelKind::Local,
        [10., -50.],
        at(ep(ret, 1)),
    )?;
    label(
        &mut engine,
        front,
        "GND",
        power(PowerStyle::Ground),
        [45., -50.],
        at(ep(ret, 2)),
    )?;
    let feedback = net_of(&engine, ep(r1, 2))?;
    let mut channels = Vec::new();
    for (name, y) in [("A", -20.), ("B", -80.)] {
        let Outcome::SheetSymbolPlaced { symbol, .. } = run(
            &mut engine,
            C::PlaceSheetSymbol {
                sheet: Some(parent),
                child: Some(front),
                name: Some(name.into()),
                position: pt([40., y]),
                size: pt([30., 20.]),
            },
        )?
        else {
            return Err(Error::Invalid("sheet symbol".into()));
        };
        let Outcome::SheetPinsAdded { pins } = run(&mut engine, C::SyncSheetPins { symbol })?
        else {
            return Err(Error::Invalid("output pins".into()));
        };
        let [pin] = pins[..] else {
            return Err(Error::Invalid("one OUT pin".into()));
        };
        label(
            &mut engine,
            parent,
            &format!("OUT_{name}"),
            LabelKind::Local,
            [20., y - 5.],
            Some(WireTarget::SheetPin(crate::model::SheetPinRef {
                sheet_pin: pin,
            })),
        )?;
        channels.push(symbol);
    }
    assert_eq!(engine.project().instances.len(), 4);
    let b = engine
        .project()
        .sheets
        .uses
        .iter()
        .find(|u| u.path.symbols == [channels[1]])
        .ok_or_else(|| Error::Invalid("missing second channel".into()))?;
    let r101 = b.parts[&r1];
    let ret_b = b.parts[&ret];
    apply(
        &mut engine,
        Operation::Instance(crate::InstanceCommand::SetDesignator {
            instance: r101,
            designator: "R101".into(),
        }),
    )?;
    assert_eq!(net_name(&engine, ep(r1, 1))?, "OUT_A");
    assert_eq!(net_name(&engine, ep(r101, 1))?, "OUT_B");
    assert_eq!(net_of(&engine, ep(r1, 2))?, feedback);
    assert_ne!(feedback, net_of(&engine, ep(r101, 2))?);
    assert_eq!(net_name(&engine, ep(r1, 2))?, "A/FB");
    assert_eq!(net_name(&engine, ep(r101, 2))?, "B/FB");
    assert_eq!(net_of(&engine, ep(ret, 2))?, net_of(&engine, ep(ret_b, 2))?);
    sync_onto_board(&mut engine)?;
    assert_ne!(
        engine.project().placement_of(r1).map(|p| p.position),
        engine.project().placement_of(r101).map(|p| p.position)
    );
    let pad = |e: &Engine, id| {
        layout::pads(e.project())
            .into_iter()
            .find(|p| p.terminal == ep(id, 2))
            .ok_or_else(|| Error::Invalid("feedback pad".into()))
    };
    choose_profile(&mut engine)?;
    let mut tracks = Vec::new();
    for id in [r1, r101] {
        let p = pad(&engine, id)?;
        tracks.push(engine.project().next_id);
        apply(
            &mut engine,
            Operation::Layout(layout::LayoutCommand::Route {
                request: layout::RouteRequest {
                    net: p.net,
                    start: p.position,
                    end: [p.position[0], p.position[1] + 2.],
                    width: 0.3,
                    layer: crate::Layer::Top.into(),
                },
            }),
        )?;
    }
    let bom = |e: &Engine| -> Result<_> {
        let QueryResult::Bom(b) = e.query(Query::Bom {})? else {
            return Err(Error::Invalid("BOM".into()));
        };
        Ok(b)
    };
    assert!(
        bom(&engine)?
            .lines
            .iter()
            .any(|l| l.fitted && l.quantity == 2 && l.designators == ["R1", "R101"])
    );
    let edit = Change::Instance {
        edit: crate::updates::InstanceChange::SetProperties {
            instance: r101,
            edits: vec![text(field, "22 kΩ")],
        },
    };
    let preview = engine.preview_update(edit.clone());
    for name in ["R1:", "R101:"] {
        assert!(
            preview.changes.iter().any(|line| line.contains(name)),
            "{:?}",
            preview.changes
        );
    }
    engine.apply_update(engine.revision(), edit, false)?;
    for id in [r1, r101] {
        let report = inspect(&engine, id)?;
        assert!(report.properties.iter().any(|f| matches!(f.value,Some(crate::fields::FieldValue::Quantity {si,..}) if si == dec(22000.))));
    }
    run(
        &mut engine,
        C::RenameSheetSymbol {
            symbol: channels[0],
            name: "Left".into(),
        },
    )?;
    run(
        &mut engine,
        C::ReorderSheet {
            sheet: front,
            index: 0,
        },
    )?;
    let saved = format::encode(&engine.to_file())?;
    let reopen = Engine::from_file(format::decode(&saved)?)?;
    assert_eq!(reopen.project(), engine.project());
    engine = reopen;
    assert_eq!(net_name(&engine, ep(r1, 2))?, "Left/FB");
    assert_eq!(net_of(&engine, ep(r1, 2))?, feedback);
    let Outcome::VariantAdded { variant } = apply(
        &mut engine,
        Operation::Variant(V::Add {
            name: "One channel".into(),
            copy_of: None,
        }),
    )?
    else {
        return Err(Error::Invalid("variant".into()));
    };
    apply(
        &mut engine,
        Operation::Variant(V::SetPopulation {
            instances: vec![r101],
            variant: Some(variant),
            population: Some(crate::model::Population::NotPopulated),
        }),
    )?;
    apply(
        &mut engine,
        Operation::Variant(V::SetActive {
            variant: Some(variant),
        }),
    )?;
    assert!(
        bom(&engine)?
            .lines
            .iter()
            .any(|l| l.fitted && l.quantity == 1 && l.designators == ["R1"])
    );
    assert!(engine.project().placement_of(r101).is_some());
    let before = engine.project().clone();
    let remove = Change::Schematic {
        command: C::RemoveSheetSymbol {
            symbol: channels[1],
        },
    };
    let plan = engine.preview_update(remove.clone());
    assert_eq!(plan.conflicting_tracks, vec![tracks[1]]);
    assert_eq!(
        plan.removed_placements
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>(),
        [r101, ret_b].into_iter().collect()
    );
    engine.apply_update(engine.revision(), remove, true)?;
    assert!(engine.project().instance(r101).is_none());
    assert!(engine.project().placement_of(r1).is_some());
    engine.undo()?;
    assert_eq!(engine.project(), &before);
    Ok(())
}

fn put_sheet(engine: &mut Engine, sheet: SheetId, child: SheetId, name: &str) -> Result<Id> {
    let outcome = run(
        engine,
        C::PlaceSheetSymbol {
            sheet: Some(sheet),
            child: Some(child),
            name: Some(name.into()),
            position: pt([0., 0.]),
            size: pt([20., 20.]),
        },
    )?;
    match outcome {
        Outcome::SheetSymbolPlaced { symbol, .. } => Ok(symbol),
        other => Err(Error::Invalid(format!("{other:?}"))),
    }
}

#[test]
fn nested_feedback_uses_the_whole_path_and_unused_parts_leave_the_board() -> Result<()> {
    let mut e = Engine::default();
    let top = e.project().sheets.first();
    let front = add_sheet(&mut e, "Front")?;
    let filter = add_sheet(&mut e, "Filter")?;
    let f = two_pin(&mut e)?;
    let local = place_on(&mut e, &f, "R1", filter, [15., -20.])?;
    label(
        &mut e,
        filter,
        "FB",
        LabelKind::Local,
        [5., -20.],
        at(ep(local, 1)),
    )?;
    let sub = put_sheet(&mut e, front, filter, "Pole")?;
    let a = put_sheet(&mut e, top, front, "A")?;
    let b = put_sheet(&mut e, top, front, "B")?;
    let lookup = |e: &Engine, path: &[Id]| -> Result<Id> {
        e.project()
            .sheets
            .uses
            .iter()
            .find(|u| u.path.symbols == path)
            .and_then(|u| u.parts.get(&local))
            .copied()
            .ok_or_else(|| Error::Invalid("nested binding".into()))
    };
    assert_eq!(lookup(&e, &[a, sub])?, local);
    let second = lookup(&e, &[b, sub])?;
    assert_ne!(second, local);
    assert_eq!(net_name(&e, ep(local, 1))?, "A/Pole/FB");
    assert_eq!(net_name(&e, ep(second, 1))?, "B/Pole/FB");
    assert_ne!(net_of(&e, ep(local, 1))?, net_of(&e, ep(second, 1))?);
    sync_onto_board(&mut e)?;
    let remove = |symbol| Change::Schematic {
        command: C::RemoveSheetSymbol { symbol },
    };
    e.apply_update(e.revision(), remove(a), true)?;
    assert_eq!(lookup(&e, &[b, sub])?, second);
    assert!(e.project().instance(local).is_none());
    assert!(e.project().placement_of(second).is_some());
    e.apply_update(e.revision(), remove(b), true)?;
    assert!(
        e.project().instances.is_empty()
            && e.project().placements.is_empty()
            && e.project().nets.is_empty()
    );
    assert!(e.project().sheet(filter).is_some() && e.project().sheet_symbol(sub).is_some());
    let bytes = format::encode(&e.to_file())?;
    e = Engine::from_file(format::decode(&bytes)?)?;
    let c = put_sheet(&mut e, top, front, "C")?;
    let third = lookup(&e, &[c, sub])?;
    assert!(third != local && third != second);
    assert_eq!(net_name(&e, ep(third, 1))?, "FB");
    Ok(())
}

#[test]
fn repeating_a_boundary_spanning_component_names_the_units_and_rolls_back() -> Result<()> {
    let mut e = Engine::default();
    let top = e.project().sheets.first();
    let child = add_sheet(&mut e, "Amplifier")?;
    let mut spec = two_pin_spec();
    spec.units = vec![
        (1, "A", vec![(1, [-5., 0.])]),
        (2, "B", vec![(2, [5., 0.])]),
    ];
    let part = component(&mut e, &spec)?;
    let id = place_on(&mut e, &part, "U7", child, [15., -20.])?;
    run(
        &mut e,
        C::PlaceUnit {
            instance: id,
            unit: crate::UnitId(2),
            sheet: Some(top),
            position: pt([30., -20.]),
        },
    )?;
    put_sheet(&mut e, top, child, "First")?;
    let kept = Snapshot::of(&e);
    let Err(rejected) = put_sheet(&mut e, top, child, "Second") else {
        return Err(Error::Invalid("accepted a boundary-spanning repeat".into()));
    };
    let line = rejected.to_string();
    assert!(
        line.contains("U7") && line.contains("Amplifier") && line.contains("Sheet 1"),
        "{line}"
    );
    kept.assert_kept(&e, "split units");
    Ok(())
}

#[test]
fn a_cancelled_or_stale_sheet_worker_cannot_place_any_parts() -> Result<()> {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    let mut e = Engine::default();
    let top = e.project().sheets.first();
    let child = add_sheet(&mut e, "Filter")?;
    let command = C::PlaceSheetSymbol {
        sheet: Some(top),
        child: Some(child),
        name: Some("Channel".into()),
        position: pt([10., 0.]),
        size: pt([20., 20.]),
    };
    let token = Arc::new(AtomicBool::new(true));
    let kept = Snapshot::of(&e);
    let ev = crate::Evaluators {
        footprint: None,
        board: None,
    };
    assert!(
        e.fork()
            .prepare_sheet(command.clone(), token.clone(), ev)
            .is_err()
    );
    kept.assert_kept(&e, "cancelled before expansion");
    token.store(false, Ordering::Relaxed);
    let prepared = e.fork().prepare_sheet(command.clone(), token.clone(), ev)?;
    token.store(true, Ordering::Relaxed);
    assert!(e.commit_sheet(prepared).is_err());
    kept.assert_kept(&e, "cancelled before commit");
    token.store(false, Ordering::Relaxed);
    let prepared = e.fork().prepare_sheet(command, token, ev)?;
    run(
        &mut e,
        C::RenameSheet {
            sheet: top,
            name: "Parent".into(),
        },
    )?;
    let renamed = Snapshot::of(&e);
    assert!(e.commit_sheet(prepared).is_err());
    renamed.assert_kept(&e, "stale sheet expansion");
    Ok(())
}

#[test]
fn single_use_files_gain_bindings_without_reallocating_physical_parts_or_nets() -> Result<()> {
    let Hier { mut engine, .. } = hierarchy()?;
    sync_onto_board(&mut engine)?;
    let mut old: serde_json::Value = serde_json::from_slice(&format::encode(&engine.to_file())?)?;
    let sheets = old["project"]["sheets"]
        .as_object_mut()
        .ok_or_else(|| Error::Invalid("saved sheets".into()))?;
    sheets.remove("uses");
    sheets.remove("definitions");
    let reopened = Engine::from_file(format::decode(&serde_json::to_vec(&old)?)?)?;
    assert_eq!(reopened.project(), engine.project());
    Ok(())
}

#[test]
fn repeated_files_require_a_physical_part_record_for_each_binding() -> Result<()> {
    let Hier {
        mut engine, child, ..
    } = hierarchy()?;
    let root = engine.project().sheets.first();
    put_sheet(&mut engine, root, child, "Second")?;
    let mut stored: serde_json::Value =
        serde_json::from_slice(&format::encode(&engine.to_file())?)?;
    let parts = stored["occurrence_parts"]
        .as_array_mut()
        .ok_or_else(|| Error::Invalid("saved parts".into()))?;
    assert_eq!(parts.len(), 1);
    assert!(parts[0].get("properties").is_none());
    parts.clear();
    assert!(format::decode(&serde_json::to_vec(&stored)?).is_err());
    Ok(())
}

#[test]
fn compact_exponential_hierarchy_is_refused_before_enumeration() -> Result<()> {
    let mut project = Engine::default().project().clone();
    for n in 1..40 {
        let child = SheetId(n + 1);
        project.sheets.list.push(crate::model::sheet::Sheet {
            id: child,
            name: format!("Page {n}"),
            ..crate::model::sheet::Sheet::first()
        });
        project.sheets.definitions.insert(child);
        for _ in 0..2 {
            let id = project.allocate()?;
            project.sheet_symbols.push(crate::model::SheetSymbol {
                id,
                sheet: SheetId(n),
                child,
                name: None,
                position: pt([0., 0.]),
                size: pt([20., 20.]),
                pins: vec![],
            });
        }
    }
    assert!(matches!(
        crate::schematic::hierarchy::paths(&project, || false),
        Err(Error::Limit(_))
    ));
    Ok(())
}

#[test]
fn replacing_from_channel_b_maps_the_drawing_and_both_physical_parts() -> Result<()> {
    use crate::updates::{Alignment, Replacement, TerminalPair};
    let Hier {
        mut engine,
        child,
        u2: local,
        symbol: a,
        ..
    } = hierarchy()?;
    let top = engine.project().sheets.first();
    let b = put_sheet(&mut engine, top, child, "B")?;
    let path = crate::model::sheet::SheetPath::root(top).child(b);
    let second = engine
        .project()
        .sheet_use(&path)
        .and_then(|u| u.parts.get(&local))
        .copied()
        .ok_or_else(|| Error::Invalid("missing B".into()))?;
    let target = two_pin(&mut engine)?;
    let change = Change::ReplaceComponent {
        instances: vec![second],
        replacement: Replacement {
            target: target.r1(),
            representation: Some(target.symbol),
            physical: Some(target.physical()),
            terminals: vec![
                TerminalPair {
                    from: crate::TerminalId(1),
                    to: Some(crate::TerminalId(2)),
                },
                TerminalPair {
                    from: crate::TerminalId(2),
                    to: Some(crate::TerminalId(1)),
                },
            ],
            units: vec![],
            alignment: Alignment::FitPads,
        },
    };
    for remove_first in [false, true] {
        let mut e = engine.fork();
        if remove_first {
            e.apply_update(
                e.revision(),
                Change::Schematic {
                    command: C::RemoveSheetSymbol { symbol: a },
                },
                true,
            )?;
        }
        let physical: Vec<_> = [local, second]
            .into_iter()
            .filter(|id| e.project().instance(*id).is_some())
            .collect();
        let ids: Vec<_> = physical
            .iter()
            .map(|id| net_of(&e, ep(*id, 1)))
            .collect::<Result<_>>()?;
        e.apply_update(e.revision(), change.clone(), true)?;
        for (id, net) in physical.into_iter().zip(ids) {
            assert_eq!(
                e.project().instance(id).map(|p| p.component),
                Some(target.r1())
            );
            assert_eq!(net_of(&e, ep(id, 2))?, net);
        }
        assert!(e.project().wires_at(ep(local, 2)).next().is_some());
    }
    Ok(())
}
