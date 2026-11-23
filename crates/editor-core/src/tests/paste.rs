//! `schematic.copy` and `schematic.paste`: what a clip holds, what a paste
//! creates, nets staying separate, one undo step, and refusals.
use super::support::*;
use crate::ids::TerminalId;
use crate::library::Rotation;
use crate::model::{LabelKind, SheetItem};
use crate::schematic::{ClipObject, Pasted, SchematicCommand as C, SheetClip, WireTarget};
use crate::{Endpoint, Engine, Error, Id, Operation, Outcome, Point, Query, QueryResult, Result};

fn two_pin(engine: &mut Engine) -> Result<Fixture> {
    component(
        engine,
        &Spec {
            name: "Two terminal",
            terminals: vec![(1, "A", passive()), (2, "B", passive())],
            units: vec![(1, "A", vec![(1, [-5.0, 0.0]), (2, [5.0, 0.0])])],
            package: vec![(1, "1"), (2, "2")],
            mapping: vec![(1, 1), (2, 2)],
            physical: true,
        },
    )
}

/// U1 at the origin and U2 at x = 20, with one wire U1.B to U2.A.
struct Sheet {
    engine: Engine,
    u1: Id,
    u2: Id,
    wire: Id,
}

fn pin(instance: Id, terminal: u32) -> Endpoint {
    Endpoint {
        instance,
        terminal: TerminalId(terminal),
    }
}

fn wired() -> Result<Sheet> {
    let mut engine = Engine::default();
    let f = two_pin(&mut engine)?;
    let (u1, _) = place(&mut engine, &f, "U1", 1, [0.0, 0.0])?;
    let (u2, _) = place(&mut engine, &f, "U2", 1, [20.0, 0.0])?;
    connect(&mut engine, pin(u1, 2), pin(u2, 1))?;
    let wire = wire_between(&engine, pin(u1, 2), pin(u2, 1))?;
    Ok(Sheet {
        engine,
        u1,
        u2,
        wire,
    })
}

fn copied(engine: &Engine, objects: Vec<ClipObject>) -> Result<SheetClip> {
    match engine.query(Query::SheetCopy {
        sheet: None,
        objects,
        portable: false,
    })? {
        QueryResult::SheetClip(clip) => Ok(*clip),
        other => Err(Error::Invalid(format!("unexpected {other:?}"))),
    }
}

fn paste_op(clip: SheetClip, offset: Point) -> Operation {
    Operation::Schematic(C::Paste {
        sheet: None,
        clip: Box::new(clip),
        offset: pt(offset),
    })
}

fn pasted(engine: &mut Engine, clip: SheetClip, offset: Point) -> Result<Pasted> {
    match apply(engine, paste_op(clip, offset))? {
        Outcome::Pasted { report } => Ok(report),
        other => Err(Error::Invalid(format!("unexpected {other:?}"))),
    }
}

/// A symbol: a new instance of the same component with the next designator
/// and its orientation, at the offset, unconnected; one undo takes it back.
#[test]
fn a_symbol_pastes_as_a_new_instance_with_a_new_designator() -> Result<()> {
    let Sheet {
        mut engine,
        u1,
        wire,
        ..
    } = wired()?;
    let occurrence = engine.project().occurrences_of(u1).next().map(|o| o.id);
    apply(
        &mut engine,
        Operation::Schematic(C::OrientUnit {
            occurrence: occurrence.ok_or(Error::Invalid("no unit".into()))?,
            rotation: Rotation::R90,
            mirror: true,
        }),
    )?;
    let before = engine.project().clone();
    let clip = copied(&engine, vec![ClipObject::Instance { id: u1 }])?;
    assert!(clip.wires.is_empty(), "the wire to U2 stays behind");
    let report = pasted(&mut engine, clip, [0.0, -30.0])?;
    let [symbol] = report.symbols.as_slice() else {
        return Err(Error::Invalid("one symbol".into()));
    };
    assert_eq!(symbol.designator, "U3");
    let project = engine.project();
    let (old, new) = (project.instance(u1), project.instance(symbol.instance));
    assert_eq!(new.map(|i| i.component), old.map(|i| i.component));
    let placed = project
        .occurrence(symbol.occurrence)
        .ok_or(Error::Invalid("no occurrence".into()))?;
    assert_eq!(placed.position, pt([0.0, -30.0]));
    assert_eq!((placed.rotation, placed.mirror), (Rotation::R90, true));
    assert_ne!(
        net_of(&engine, pin(symbol.instance, 2))?,
        net_of(&engine, pin(u1, 2))?
    );
    assert!(engine.project().wire(wire).is_some());
    engine.undo()?;
    assert_eq!(engine.project(), &before);
    Ok(())
}

/// A label keeps its name and so joins that net by name, as any label does.
#[test]
fn a_label_keeps_its_name_and_joins_its_net() -> Result<()> {
    let Sheet {
        mut engine, u1, u2, ..
    } = wired()?;
    let sig = signal_label(&mut engine, u1)?;
    let clip = copied(&engine, vec![ClipObject::Label { id: sig }])?;
    let report = pasted(&mut engine, clip, [0.0, -30.0])?;
    let [new] = report.labels.as_slice() else {
        return Err(Error::Invalid("one label".into()));
    };
    let label = engine
        .project()
        .label(*new)
        .ok_or(Error::Invalid("no label".into()))?;
    assert_eq!(label.name, "SIG");
    assert_eq!(
        label.position,
        pt([-5.0, -30.0]),
        "where the source is drawn"
    );
    assert!(label.manual, "alone it stays where it was pasted");
    apply(
        &mut engine,
        Operation::Schematic(C::Connect {
            from: WireTarget::Label(crate::model::LabelRef { label: *new }),
            to: WireTarget::Terminal(pin(u2, 2)),
        }),
    )?;
    assert_eq!(net_of(&engine, pin(u2, 2))?, net_of(&engine, pin(u1, 1))?);
    Ok(())
}

/// A local label "SIG" on U1.A, drawn on that pin.
fn signal_label(engine: &mut Engine, u1: Id) -> Result<Id> {
    match apply(
        engine,
        Operation::Schematic(C::PlaceLabel {
            sheet: None,
            name: "SIG".into(),
            kind: LabelKind::Local,
            position: pt([100.0, 100.0]),
            rotation: Rotation::R0,
            attach: Some(WireTarget::Terminal(pin(u1, 1))),
        }),
    )? {
        Outcome::LabelPlaced { label } => Ok(label),
        other => Err(Error::Invalid(format!("unexpected {other:?}"))),
    }
}

/// Both ends selected: the wire comes along without being named and joins
/// the two new pins, on a net of their own.
#[test]
fn a_wire_with_both_ends_selected_joins_the_pasted_pins() -> Result<()> {
    let Sheet {
        mut engine, u1, u2, ..
    } = wired()?;
    let clip = copied(
        &engine,
        vec![
            ClipObject::Instance { id: u1 },
            ClipObject::Instance { id: u2 },
        ],
    )?;
    assert_eq!(clip.wires.len(), 1);
    let report = pasted(&mut engine, clip, [0.0, -30.0])?;
    let [a, b] = report.symbols.as_slice() else {
        return Err(Error::Invalid("two symbols".into()));
    };
    let (a, b) = (a.instance, b.instance);
    wire_between(&engine, pin(a, 2), pin(b, 1))?;
    assert_eq!(net_of(&engine, pin(a, 2))?, net_of(&engine, pin(b, 1))?);
    assert_ne!(net_of(&engine, pin(a, 2))?, net_of(&engine, pin(u1, 2))?);
    assert_eq!(net_of(&engine, pin(u1, 2))?, net_of(&engine, pin(u2, 1))?);
    Ok(())
}

/// One end selected: the named wire follows the pasted symbol there and has
/// a free end, moved by the offset, where it lost the other.
#[test]
fn a_wire_with_one_end_selected_gets_a_free_end() -> Result<()> {
    let Sheet {
        mut engine,
        u1,
        wire,
        ..
    } = wired()?;
    let clip = copied(
        &engine,
        vec![
            ClipObject::Instance { id: u1 },
            ClipObject::Wire { id: wire },
        ],
    )?;
    let report = pasted(&mut engine, clip, [0.0, -30.0])?;
    let new = report.symbols.first().map(|s| s.instance);
    let new = new.ok_or(Error::Invalid("no symbol".into()))?;
    let at = crate::WireEnd::Terminal(pin(new, 2));
    let wire = engine.project().wires_at(at).next().cloned();
    let wire = wire.ok_or(Error::Invalid("no wire on the new pin".into()))?;
    let other = if wire.from == at { wire.to } else { wire.from };
    assert_eq!(other.free_point(), Some([15.0, -30.0]));
    Ok(())
}

/// A wire alone: both its ends are free, its shape kept.
#[test]
fn a_wire_alone_pastes_with_two_free_ends() -> Result<()> {
    let Sheet {
        mut engine, wire, ..
    } = wired()?;
    let clip = copied(&engine, vec![ClipObject::Wire { id: wire }])?;
    let report = pasted(&mut engine, clip, [0.0, -30.0])?;
    let new = report.wires.first().and_then(|w| engine.project().wire(*w));
    let new = new.ok_or(Error::Invalid("no wire".into()))?;
    assert_eq!(new.from.free_point(), Some([5.0, -30.0]));
    assert_eq!(new.to.free_point(), Some([15.0, -30.0]));
    Ok(())
}

/// A clip stands alone: what it copied may be gone, as after a cut, and a
/// drawing moves by the offset.
#[test]
fn a_clip_outlives_its_objects_and_drawings_shift() -> Result<()> {
    let mut engine = Engine::default();
    let drawing = match apply(
        &mut engine,
        Operation::Schematic(C::AddDrawing {
            sheet: None,
            item: SheetItem::Line {
                from: pt([0.0, 0.0]),
                to: pt([5.0, 0.0]),
            },
        }),
    )? {
        Outcome::DrawingAdded { drawing } => drawing,
        other => return Err(Error::Invalid(format!("unexpected {other:?}"))),
    };
    let clip = copied(&engine, vec![ClipObject::Drawing { id: drawing }])?;
    apply(
        &mut engine,
        Operation::Schematic(C::RemoveDrawing { drawing }),
    )?;
    let report = pasted(&mut engine, clip, [1.0, 2.0])?;
    let new = report
        .drawings
        .first()
        .and_then(|d| engine.project().drawing(*d));
    assert_eq!(
        new.map(|d| d.item.clone()),
        Some(SheetItem::Line {
            from: pt([1.0, 2.0]),
            to: pt([6.0, 2.0]),
        })
    );
    Ok(())
}

/// The copy query and every refused paste leave the document, its revision
/// and its history as they were; so does a stale revision.
#[test]
fn refusals_change_nothing() -> Result<()> {
    let Sheet {
        mut engine,
        u1,
        wire,
        ..
    } = wired()?;
    let symbol = copied(&engine, vec![ClipObject::Instance { id: u1 }])?;
    let lost = copied(&engine, vec![ClipObject::Wire { id: wire }])?;
    let before = engine.project().clone();
    let revision = engine.revision();
    let mut unknown_component = symbol.clone();
    for s in &mut unknown_component.symbols {
        s.instance.component.id = crate::DefinitionId(u64::MAX - 1);
    }
    let mut no_units = symbol.clone();
    for s in &mut no_units.symbols {
        s.occurrences.clear();
    }
    let mut foreign_end = lost.clone();
    for w in &mut foreign_end.wires {
        w.from = crate::WireEnd::Terminal(pin(u1, 1));
    }
    for (name, clip, offset) in [
        ("nothing", SheetClip::default(), [0.0, 0.0]),
        ("off range", lost.clone(), [1_999.0, 0.0]),
        ("unknown component", unknown_component, [0.0, 10.0]),
        ("no units", no_units, [0.0, 10.0]),
        ("foreign wire end", foreign_end, [0.0, 10.0]),
    ] {
        assert!(
            engine.apply(revision, paste_op(clip, offset)).is_err(),
            "{name}"
        );
        assert_eq!(engine.project(), &before, "{name}");
        assert_eq!(engine.revision(), revision, "{name}");
    }
    // An offset beyond the range, or not finite, is refused where the
    // command is read.
    for offset in [
        serde_json::json!([1e9, 0.0]),
        serde_json::json!(["NaN", 0.0]),
    ] {
        let command = serde_json::json!({"command": "schematic.paste", "params": {
            "clip": symbol, "offset": offset
        }});
        assert!(serde_json::from_value::<C>(command).is_err(), "{offset}");
    }
    let unknown_sheet = Operation::Schematic(C::Paste {
        sheet: Some(crate::SheetId(9)),
        clip: Box::new(symbol.clone()),
        offset: pt([0.0, 10.0]),
    });
    assert!(engine.apply(revision, unknown_sheet).is_err());
    assert!(matches!(
        engine.apply(revision + 1, paste_op(symbol, [0.0, 10.0])),
        Err(Error::Stale { .. })
    ));
    assert_eq!(engine.project(), &before);
    // The query names only what exists on the sheet.
    assert!(copied(&engine, vec![]).is_err());
    assert!(copied(&engine, vec![ClipObject::Label { id: u1 }]).is_err());
    assert!(copied(&engine, vec![ClipObject::Wire { id: u1 }]).is_err());
    Ok(())
}

/// The command and query as a transport writes them: JSON in, JSON out, with
/// unknown parameters refused.
#[test]
fn the_wire_format_round_trips() -> Result<()> {
    let Sheet { mut engine, u1, .. } = wired()?;
    let query: Query = serde_json::from_value(serde_json::json!({
        "query": "schematic.copy",
        "params": { "objects": [{ "kind": "instance", "id": u1 }] },
    }))
    .map_err(|e| Error::Invalid(e.to_string()))?;
    let QueryResult::SheetClip(clip) = engine.query(query)? else {
        return Err(Error::Invalid("not a clip".into()));
    };
    let operation: Operation = serde_json::from_value(serde_json::json!({
        "editor": "Schematic",
        "operation": {
            "command": "schematic.paste",
            "params": { "clip": clip, "offset": [0.0, -30.0] },
        },
    }))
    .map_err(|e| Error::Invalid(e.to_string()))?;
    assert!(matches!(
        apply(&mut engine, operation)?,
        Outcome::Pasted { .. }
    ));
    assert!(
        serde_json::from_value::<Operation>(serde_json::json!({
            "editor": "Schematic",
            "operation": {
                "command": "schematic.paste",
                "params": { "clip": {}, "offset": [0.0, 0.0], "extra": 1 },
            },
        }))
        .is_err()
    );
    Ok(())
}

/// Two projects from one template hold different bindings under the same
/// ids: in the source R1's pad 1 is terminal 1, in the destination's copy
/// of the "same" revision pad 1 is terminal 2. A paste reads the clip's
/// library revisions by their content, never by id alone: a plain clip is
/// refused, and a portable one imports the source's revisions with fresh
/// ids, so the pasted part joins pad 1 to terminal 1 as its source does.
#[test]
fn a_paste_into_another_project_never_changes_a_pin_mapping() -> Result<()> {
    use crate::schematic::clipboard::terminal_pads;
    let source = Engine::demo()?;
    let r1 = by_designator(&source, "R1")?;
    let instance = source
        .project()
        .instance(r1)
        .cloned()
        .ok_or_else(|| Error::NotFound("R1".into()))?;
    let expected = terminal_pads(&source.project().library, &instance)?;
    assert_eq!(expected.len(), 2);

    // The destination: the same project with the binding's two terminals
    // swapped in place, under the same ids and revision.
    let mut file = source.to_file();
    let binding = crate::instances::resolve(&file.project.library, &instance)?
        .physical
        .map(|p| (p.binding.id, p.binding.revision))
        .ok_or_else(|| Error::NotFound("R1's binding".into()))?;
    for b in &mut file.project.library.bindings {
        if (b.id, b.revision) == binding {
            let terminals: Vec<TerminalId> =
                b.definition.mappings.iter().map(|m| m.terminal).collect();
            for (m, t) in b.definition.mappings.iter_mut().zip(terminals.iter().rev()) {
                m.terminal = *t;
            }
        }
    }
    let mut destination = Engine::from_file(file)?;
    let theirs = destination
        .project()
        .instance(r1)
        .cloned()
        .ok_or_else(|| Error::NotFound("R1".into()))?;
    assert_ne!(
        terminal_pads(&destination.project().library, &theirs)?,
        expected,
        "the fixture maps the pads the other way"
    );

    let copy = |portable| match source.query(Query::SheetCopy {
        sheet: None,
        objects: vec![ClipObject::Instance { id: r1 }],
        portable,
    })? {
        QueryResult::SheetClip(clip) => Ok(*clip),
        other => Err(Error::Invalid(format!("unexpected {other:?}"))),
    };

    // A plain clip cannot establish the parts' identity there: refused.
    let kept = Snapshot::of(&destination);
    let original = destination.project().clone();
    let refused = destination.apply(destination.revision(), paste_op(copy(false)?, [0.0, 30.0]));
    assert!(
        matches!(&refused, Err(Error::Conflict(m)) if m.contains("pin mapping")),
        "{refused:?}"
    );
    kept.assert_kept(&destination, "plain cross-project paste");

    // A portable clip imports the source's revisions and keeps the mapping.
    let revisions = destination.project().library.revisions().len();
    let report = pasted(&mut destination, copy(true)?, [0.0, 30.0])?;
    let pasted_id = report.symbols[0].instance;
    let copy_of = destination
        .project()
        .instance(pasted_id)
        .cloned()
        .ok_or_else(|| Error::NotFound("the pasted part".into()))?;
    assert_eq!(
        terminal_pads(&destination.project().library, &copy_of)?,
        expected
    );
    assert_ne!(
        copy_of.component.id, theirs.component.id,
        "an imported copy"
    );
    let imported = destination.project().library.revisions().len();
    assert!(imported > revisions);
    // Pasting again reuses the imported copy.
    pasted(&mut destination, copy(true)?, [0.0, 60.0])?;
    assert_eq!(destination.project().library.revisions().len(), imported);
    // It saves and reopens, and undo takes the paste and its import back.
    let bytes = crate::format::encode(&crate::format::EditorFile::from_document(
        destination.document(),
    ))?;
    crate::format::decode(&bytes)?;
    destination.undo()?;
    destination.undo()?;
    assert_eq!(destination.project(), &original, "undone");

    // Within one project nothing is imported.
    let mut same = Engine::demo()?;
    let before = same.project().library.revisions().len();
    pasted(&mut same, copy(true)?, [0.0, 30.0])?;
    assert_eq!(same.project().library.revisions().len(), before);
    Ok(())
}

/// A pin review still pending travels with the part, whichever way it is
/// reused (review §11): a portable paste into a project that holds the
/// same revision bodies under the same ids without the obligation, a
/// paste again over the copy, a bundle import into a project that already
/// holds that copy with another obligation (the two are joined), and undo
/// and redo of each.
#[test]
fn a_pending_pin_review_survives_paste_and_import_reuse() -> Result<()> {
    use crate::library::pinout::{ImportReview, PublishedReview};
    let pending = |t: u32| ImportReview {
        unreviewed: [TerminalId(t)].into(),
        ..Default::default()
    };
    let plain = Engine::demo()?;
    let r1 = by_designator(&plain, "R1")?;
    let component = plain
        .project()
        .instance(r1)
        .map(|i| i.component)
        .ok_or_else(|| Error::NotFound("R1".into()))?;
    // The source: the same project with R1's terminal 1 still to review.
    let mut file = plain.to_file();
    file.project.library.reviews.push(PublishedReview {
        component,
        review: pending(1),
    });
    let source = Engine::from_file(file)?;
    let clip = |engine: &Engine| match engine.query(Query::SheetCopy {
        sheet: None,
        objects: vec![ClipObject::Instance { id: r1 }],
        portable: true,
    })? {
        QueryResult::SheetClip(clip) => Ok(clip),
        other => Err(Error::Invalid(format!("unexpected {other:?}"))),
    };
    let review_of = |engine: &Engine, instance: Id| {
        engine
            .project()
            .instance(instance)
            .and_then(|i| engine.project().library.review_of(i.component))
            .cloned()
    };

    // Paste into a project with equal bodies under the same ids and no
    // obligation.
    let mut destination = Engine::demo()?;
    let report = pasted(&mut destination, *clip(&source)?, [0.0, 30.0])?;
    let copy = report.symbols[0].instance;
    assert_eq!(review_of(&destination, copy), Some(pending(1)), "pasted");
    destination.undo()?;
    assert!(destination.project().instance(copy).is_none());
    destination.redo()?;
    assert_eq!(review_of(&destination, copy), Some(pending(1)), "redone");
    // Pasting again reuses the copy and its obligation.
    let again = pasted(&mut destination, *clip(&source)?, [0.0, 60.0])?;
    assert_eq!(
        review_of(&destination, again.symbols[0].instance),
        Some(pending(1))
    );
    // The source's own R1 is unaffected in a project without the review.
    assert_eq!(review_of(&destination, r1), None);

    // A bundle import of the same part into a project that holds the
    // same revisions under the same ids without the obligation.
    let bundle = clip(&source)?
        .library
        .bundles
        .first()
        .cloned()
        .ok_or_else(|| Error::NotFound("a portable bundle".into()))?;
    let import = |bundle: &crate::library::bundle::Bundle| {
        Operation::Library(crate::LibraryCommand::Import {
            bundle: Box::new(bundle.clone()),
        })
    };
    let mut holder = Engine::demo()?;
    let before = holder.project().clone();
    let Outcome::Imported { report } = apply(&mut holder, import(&bundle))? else {
        return Err(Error::Invalid("not imported".into()));
    };
    let root = report.root;
    let held = crate::ids::ComponentRef::new(root.id, root.revision);
    assert_ne!(held, component, "a copy, not the project's own revision");
    assert_eq!(holder.project().library.review_of(held), Some(&pending(1)));
    holder.undo()?;
    assert_eq!(holder.project(), &before, "undo takes the import back");
    holder.redo()?;
    assert_eq!(holder.project().library.review_of(held), Some(&pending(1)));
    // A paste of the same part then reuses that copy and its obligation.
    let reused = pasted(&mut holder, *clip(&source)?, [0.0, 30.0])?;
    let copy = reused.symbols[0].instance;
    assert_eq!(
        holder.project().instance(copy).map(|i| i.component),
        Some(held)
    );
    assert_eq!(review_of(&holder, copy), Some(pending(1)));

    // The same source revision with another pending review is another
    // record of it: refused, never merged silently over the held copy.
    let mut other_file = plain.to_file();
    other_file.project.library.reviews.push(PublishedReview {
        component,
        review: pending(2),
    });
    let other = clip(&Engine::from_file(other_file)?)?
        .library
        .bundles
        .first()
        .cloned()
        .ok_or_else(|| Error::NotFound("a portable bundle".into()))?;
    let kept = Snapshot::of(&holder);
    assert!(matches!(
        holder.apply(holder.revision(), import(&other)),
        Err(Error::Conflict(_))
    ));
    kept.assert_kept(&holder, "a differently reviewed copy");
    Ok(())
}

fn kelvin_draft(engine: &mut Engine, extra_pin: bool) -> Result<Fixture> {
    use crate::library::tie::{TieForm, TieSpec};
    use crate::library::{Definition, PackageModel};
    let mut spec = Spec {
        name: "Sense contact",
        terminals: vec![(1, "Sense", passive()), (2, "Force", passive())],
        units: vec![(1, "A", vec![(1, [-4., 0.]), (2, [4., 0.])])],
        physical: true,
        package: vec![],
        mapping: vec![],
    };
    if extra_pin {
        spec.terminals.push((3, "Auxiliary input", input()));
        spec.units[0].2.push((3, [4., 4.]));
    }
    let f = draft(engine, &spec)?;
    rewrite(engine, f.package, |d| {
        if let Definition::Package(p) = d {
            p.model = PackageModel::NoBody;
        }
    })?;
    rewrite(engine, f.component, |d| {
        if let Definition::Component(c) = d {
            c.assembly = TieForm::Copper.assembly();
            c.ties = vec![TieSpec {
                id: crate::ids::TieId(1),
                ends: vec![TerminalId(1), TerminalId(2)],
                form: TieForm::Copper,
            }];
        }
    })?;
    Ok(f)
}

#[test]
fn landless_binding_waives_mappings_only_for_its_declared_tie_ends() -> Result<()> {
    let mut engine = Engine::default();
    let draft = kelvin_draft(&mut engine, true)?;
    let Err(error) = publish(&mut engine, draft.all()) else {
        return Err(Error::Invalid(
            "published an unmapped auxiliary terminal".into(),
        ));
    };
    let description = format!("{error:?}");
    assert!(description.contains("Auxiliary input"), "{description}");
    Ok(())
}

#[test]
fn cross_project_paste_remaps_only_kelvin_anchors_with_copied_targets() -> Result<()> {
    use crate::ids::{PadId, TieId};
    use crate::library::tie::Anchor;
    let mut source = Engine::profiled()?;
    let f = kelvin_draft(&mut source, false)?;
    publish(&mut source, f.all())?;
    let (owner, _) = place(&mut source, &f, "NT1", 1, [12., 12.])?;
    let probe = component(&mut source, &probe("Probe", passive()))?;
    let (target, _) = place(&mut source, &probe, "TP1", 1, [26., 12.])?;
    sync_onto_board(&mut source)?;
    apply(
        &mut source,
        Operation::Instance(crate::InstanceCommand::SetAnchor {
            group: None,
            instance: owner,
            end: TerminalId(1),
            anchor: Some(Anchor {
                instance: target,
                pad: PadId(1),
            }),
        }),
    )?;
    // The destination is an independent document seeded with the same
    // ids, including an unrelated instance that collides with the old target.
    let mut dest = Engine::from_file(source.to_file())?;
    let anchor_of = |engine: &Engine, id| {
        engine
            .project()
            .instance(id)?
            .ties
            .get(&TieId(1))?
            .anchors
            .get(&TerminalId(1))
            .copied()
    };
    let clip = copied(
        &source,
        vec![
            ClipObject::Instance { id: owner },
            ClipObject::Instance { id: target },
        ],
    )?;
    let report = pasted(&mut dest, clip, [0., 24.])?;
    let [tie, probe] = report.symbols.as_slice() else {
        return Err(Error::Invalid("two pasted symbols".into()));
    };
    assert_ne!(probe.instance, target);
    assert_eq!(
        anchor_of(&dest, tie.instance),
        Some(Anchor {
            instance: probe.instance,
            pad: PadId(1)
        })
    );
    assert_eq!(
        anchor_of(&dest, owner),
        Some(Anchor {
            instance: target,
            pad: PadId(1)
        })
    );
    let clip = copied(&source, vec![ClipObject::Instance { id: owner }])?;
    let report = pasted(&mut dest, clip, [24., 24.])?;
    let [only] = report.symbols.as_slice() else {
        return Err(Error::Invalid("one pasted symbol".into()));
    };
    assert_eq!(anchor_of(&dest, only.instance), None);
    Engine::from_file(dest.to_file())?;
    Ok(())
}
