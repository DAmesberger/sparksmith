//! Assembly variants through the public commands and queries
//! (docs/assembly-variants.md): a board with two variants, one leaving a
//! part off and one fitting another Part on the same footprint. Parts,
//! manufacturers and MPNs are invented (`ExampleCo`, `EX-…`).
use super::support::*;
use crate::completeness::BomReport;
use crate::fabrication::assembly::{self, PlacementConvention};
use crate::instances::variants::{self, Substitute, VariantCommand, VariantsReport};
use crate::library::fixtures::{Kind, two_terminal, two_terminal_header};
use crate::library::{
    Basis, Definition, DistributorOffer, DraftEdit, PartDefinition, Provenance, Reference,
    ReferenceKind,
};
use crate::model::Population;
use crate::{
    BindingRef, ComponentRef, DefinitionKind, Engine, Error, Id, LibraryCommand, Operation,
    Outcome, PartRef, Query, QueryResult, Result, Revision, format,
};

struct Board {
    engine: Engine,
    r1: Id,
    r2: Id,
    /// A 0 Ω Part of the demo resistor, on its own binding.
    zero: PartRef,
    /// A capacitor Part: another component and package on the same lands.
    cap: PartRef,
    cap_package: crate::DefinitionId,
    /// A Part on a header's lands, which fit no resistor footprint.
    header: PartRef,
}

fn part(mpn: &str, component: ComponentRef, binding: BindingRef) -> PartDefinition {
    PartDefinition {
        next_local: 3,
        manufacturer: "ExampleCo".into(),
        mpn: mpn.into(),
        component,
        binding,
        specs: Vec::new(),
        references: vec![Reference {
            id: 1,
            kind: ReferenceKind::Datasheet,
            title: format!("{mpn} datasheet"),
            url: Some("https://example.com/exampleco/parts.pdf".into()),
            document_revision: Some("A".into()),
            asset: None,
            licence: None,
            provenance: Provenance::default(),
        }],
        offers: vec![DistributorOffer {
            id: 2,
            distributor: "Example Distribution".into(),
            sku: format!("EXD-{mpn}"),
            url: None,
            packaging: None,
        }],
        provenance: Provenance {
            basis: Basis::FromReference(1),
            ..Provenance::default()
        },
        top_mark: None,
    }
}

fn publish_part(engine: &mut Engine, body: PartDefinition) -> Result<PartRef> {
    let id = new_draft(engine, DefinitionKind::Part)?;
    edit(
        engine,
        vec![DraftEdit {
            id,
            meta: meta(&body.mpn),
            definition: Definition::Part(Box::new(body)),
        }],
    )?;
    let r = publish(engine, vec![id])?
        .into_iter()
        .next()
        .ok_or_else(|| Error::Invalid("nothing published".into()))?;
    Ok(PartRef::new(r.id, r.revision))
}

/// A sample component set made by `make`, published, and a Part of it.
fn publish_set(
    engine: &mut Engine,
    name: &str,
    make: fn(&str, Kind<'_>, crate::library::fixtures::SampleRefs) -> Result<Vec<DraftEdit>>,
    kind: Kind<'_>,
    mpn: &str,
) -> Result<(PartRef, crate::DefinitionId)> {
    let refs = engine.reserve_lineages()?;
    let edits = make(name, kind, refs)?;
    apply(engine, Operation::Library(LibraryCommand::Edit { edits }))?;
    publish(
        engine,
        vec![
            refs.component,
            refs.symbol.id,
            refs.package.id,
            refs.footprint.id,
            refs.binding.id,
        ],
    )?;
    let part = publish_part(
        engine,
        part(
            mpn,
            ComponentRef::new(refs.component, Revision::FIRST),
            BindingRef::new(refs.binding.id, Revision::FIRST),
        ),
    )?;
    Ok((part, refs.package.id))
}

fn board() -> Result<Board> {
    let mut engine = Engine::demo()?;
    let demo = demo_fixture(&engine)?;
    let r1 = by_designator(&engine, "R1")?;
    let r2 = by_designator(&engine, "R2")?;
    let zero = publish_part(
        &mut engine,
        part(
            "EX-R0603-0R",
            ComponentRef::new(demo.component, Revision::FIRST),
            BindingRef::new(demo.binding, Revision::FIRST),
        ),
    )?;
    let (cap, cap_package) = publish_set(
        &mut engine,
        "Capacitor",
        two_terminal,
        Kind::CAPACITOR,
        "EX-C0603-100N",
    )?;
    let (header, _) = publish_set(
        &mut engine,
        "Header",
        two_terminal_header,
        Kind::CONNECTOR,
        "EX-HDR-2",
    )?;
    Ok(Board {
        engine,
        r1,
        r2,
        zero,
        cap,
        cap_package,
        header,
    })
}

fn variant(engine: &mut Engine, command: VariantCommand) -> Result<Outcome> {
    apply(engine, Operation::Variant(command))
}

fn add(engine: &mut Engine, name: &str) -> Result<Id> {
    match variant(
        engine,
        VariantCommand::Add {
            name: name.into(),
            copy_of: None,
        },
    )? {
        Outcome::VariantAdded { variant } => Ok(variant),
        other => Err(Error::Invalid(format!("unexpected outcome {other:?}"))),
    }
}

fn activate(engine: &mut Engine, id: Option<Id>) -> Result<()> {
    variant(engine, VariantCommand::SetActive { variant: id }).map(|_| ())
}

fn bom(engine: &Engine) -> Result<BomReport> {
    match engine.query(Query::Bom {})? {
        QueryResult::Bom(report) => Ok(report),
        other => Err(Error::Invalid(format!("unexpected report {other:?}"))),
    }
}

fn listed(engine: &Engine) -> Result<VariantsReport> {
    match engine.query(Query::Variants {})? {
        QueryResult::Variants(report) => Ok(report),
        other => Err(Error::Invalid(format!("unexpected report {other:?}"))),
    }
}

fn substitutes(engine: &Engine, instance: Id) -> Result<Vec<Substitute>> {
    match engine.query(Query::Substitutes { instance })? {
        QueryResult::Substitutes(list) => Ok(list),
        other => Err(Error::Invalid(format!("unexpected report {other:?}"))),
    }
}

/// Designators of the fitted BOM lines and of the lines not fitted.
fn bom_split(engine: &Engine) -> Result<(Vec<String>, Vec<String>)> {
    let report = bom(engine)?;
    let mut fitted = Vec::new();
    let mut not = Vec::new();
    for line in report.lines {
        if line.fitted {
            fitted.extend(line.designators);
        } else {
            not.extend(line.designators);
        }
    }
    Ok((fitted, not))
}

fn placed_designators(engine: &Engine) -> Result<Vec<String>> {
    Ok(
        assembly::rows(engine.project(), &PlacementConvention::default())?
            .into_iter()
            .map(|r| r.designator)
            .collect(),
    )
}

/// The package each placed body is made from, by designator, as the body
/// checks and the 3D view read it.
fn fitted_package(engine: &Engine, instance: Id) -> Result<Option<crate::DefinitionId>> {
    let project = engine.project();
    let found = project
        .instance(instance)
        .ok_or_else(|| Error::NotFound("instance".into()))?;
    let fitted = variants::fitted(project, found);
    Ok(fitted
        .populated()
        .then_some(fitted.physical.map(|p| p.package.id))
        .flatten())
}

fn checked_bodies(engine: &Engine) -> Vec<String> {
    let report = crate::layout::bodies::bodies(engine.project());
    report
        .parts
        .iter()
        .map(|p| p.designator.clone())
        .chain(report.without_leads)
        .collect()
}

#[test]
fn population_is_typed_and_variants_change_bodies_bom_and_placement() -> Result<()> {
    let mut b = board()?;
    let engine = &mut b.engine;
    let resistor_package = fitted_package(engine, b.r1)?;
    assert!(resistor_package.is_some());

    // Two variants: "Lite" leaves R2 off, "Alt" fits the capacitor on R1.
    let lite = add(engine, "Lite")?;
    let alt = add(engine, "Alt")?;
    variant(
        engine,
        VariantCommand::SetPopulation {
            instances: vec![b.r2],
            variant: Some(lite),
            population: Some(Population::NotPopulated),
        },
    )?;
    variant(
        engine,
        VariantCommand::SetPart {
            instance: b.r1,
            variant: alt,
            part: Some(b.cap),
        },
    )?;

    // The default is unchanged by either variant.
    assert_eq!(bom_split(engine)?.1, Vec::<String>::new());
    assert_eq!(placed_designators(engine)?, ["R1", "R2"]);
    assert_eq!(checked_bodies(engine).len(), 2);

    // Lite: R2 is listed as not fitted, has no placement row and no body.
    activate(engine, Some(lite))?;
    let (fitted, not) = bom_split(engine)?;
    assert_eq!(
        (fitted, not),
        (vec!["R1".to_owned()], vec!["R2".to_owned()])
    );
    assert_eq!(bom(engine)?.variant.as_deref(), Some("Lite"));
    assert_eq!(placed_designators(engine)?, ["R1"]);
    assert_eq!(fitted_package(engine, b.r2)?, None);
    assert_eq!(checked_bodies(engine), ["R1"]);
    let parts_csv = assembly::bom_parts_csv(engine.project())?;
    assert!(
        parts_csv.contains("\nR2,") && parts_csv.trim_end().ends_with(",no"),
        "{parts_csv}"
    );
    let grouped = assembly::bom_csv(engine.project())?;
    assert!(!grouped.contains("R2"), "{grouped}");

    // Alt: R1 carries the capacitor's body and is bought as that Part.
    activate(engine, Some(alt))?;
    assert_eq!(fitted_package(engine, b.r1)?, Some(b.cap_package));
    assert_ne!(Some(b.cap_package), resistor_package);
    let report = bom(engine)?;
    let r1_line = report
        .lines
        .iter()
        .find(|l| l.designators == ["R1"])
        .ok_or_else(|| Error::NotFound("R1's line".into()))?;
    assert_eq!(r1_line.part, Some(b.cap));
    assert_eq!(r1_line.mpn.as_deref(), Some("EX-C0603-100N"));
    assert!(r1_line.fitted && !r1_line.unresolved);
    assert_eq!(placed_designators(engine)?, ["R1", "R2"]);
    let rows = assembly::rows(engine.project(), &PlacementConvention::default())?;
    let r1_row = rows
        .iter()
        .find(|r| r.designator == "R1")
        .ok_or_else(|| Error::NotFound("R1 row".into()))?;
    assert_eq!(
        r1_row.value, "EX-C0603-100N",
        "a Part without specs reads as its MPN"
    );

    // The copper never changes between variants.
    let copper = |e: &Engine| {
        (
            e.project().placements.clone(),
            e.project().tracks.clone(),
            e.project().nets.clone(),
        )
    };
    let before = copper(engine);
    activate(engine, Some(lite))?;
    assert_eq!(copper(engine), before);

    // The manager's table.
    let table = listed(engine)?;
    assert_eq!(table.active, Some(lite));
    let names: Vec<_> = table.variants.iter().map(|v| v.name.as_str()).collect();
    assert_eq!(names, ["Lite", "Alt"]);
    assert_eq!(
        (
            table.variants[0].not_populated,
            table.variants[1].substituted
        ),
        (1, 1)
    );
    let r2_row = table
        .parts
        .iter()
        .find(|r| r.instance == b.r2)
        .ok_or_else(|| Error::NotFound("R2 row".into()))?;
    assert_eq!(r2_row.population, Population::Populated);
    assert!(r2_row.variants[0].population_set);
    assert_eq!(r2_row.variants[0].population, Population::NotPopulated);
    Ok(())
}

#[test]
fn the_default_population_is_typed_and_variants_follow_it() -> Result<()> {
    let mut b = board()?;
    let engine = &mut b.engine;
    let lite = add(engine, "Lite")?;
    variant(
        engine,
        VariantCommand::SetPopulation {
            instances: vec![b.r1],
            variant: None,
            population: Some(Population::NotPopulated),
        },
    )?;
    let instance = engine
        .project()
        .instance(b.r1)
        .ok_or_else(|| Error::NotFound("R1".into()))?;
    assert_eq!(instance.population, Population::NotPopulated);
    assert_eq!(bom_split(engine)?.1, ["R1"]);
    // A variant that does not say otherwise follows the default...
    activate(engine, Some(lite))?;
    assert_eq!(bom_split(engine)?.1, ["R1"]);
    // ...and one that does wins, until it follows the default again.
    let fit = |population| VariantCommand::SetPopulation {
        instances: vec![b.r1],
        variant: Some(lite),
        population,
    };
    variant(engine, fit(Some(Population::Populated)))?;
    assert!(bom_split(engine)?.1.is_empty());
    variant(engine, fit(None))?;
    assert_eq!(bom_split(engine)?.1, ["R1"]);
    assert!(
        engine
            .project()
            .variants
            .get(lite)
            .is_some_and(|v| v.overrides.is_empty()),
        "following the default again leaves no override"
    );
    // The default needs a population.
    let refused = variant(
        engine,
        VariantCommand::SetPopulation {
            instances: vec![b.r1],
            variant: None,
            population: None,
        },
    );
    assert_eq!(
        refused.err().as_ref().map(Error::code),
        Some("invalid_input")
    );
    Ok(())
}

#[test]
fn a_substitute_must_sit_on_the_same_lands_with_the_same_pad_grouping() -> Result<()> {
    let mut b = board()?;
    let engine = &mut b.engine;
    let alt = add(engine, "Alt")?;
    let before = engine.project().clone();
    let revision = engine.revision();
    // A header's lands are not the resistor's.
    let refused = variant(
        engine,
        VariantCommand::SetPart {
            instance: b.r1,
            variant: alt,
            part: Some(b.header),
        },
    )
    .err();
    assert_eq!(refused.as_ref().map(Error::code), Some("conflict"));
    let message = refused.map(|e| e.to_string()).unwrap_or_default();
    assert!(
        message.contains("EX-HDR-2") && message.contains("land pattern"),
        "{message}"
    );
    assert_eq!(
        *engine.project(),
        before,
        "a refused command changes nothing"
    );
    assert_eq!(engine.revision(), revision);

    // The picker offers what fits: the 0 Ω resistor and the capacitor.
    let offered: Vec<String> = substitutes(engine, b.r1)?
        .into_iter()
        .map(|s| s.mpn)
        .collect();
    assert_eq!(offered, ["EX-C0603-100N", "EX-R0603-0R"]);

    // A binding that joins both pads into one terminal would short the
    // board's nets: refused, whatever the lands.
    let mut joined = engine.project().clone();
    let cap_binding = joined
        .library
        .part(b.cap)
        .map(|p| p.definition.binding)
        .ok_or_else(|| Error::NotFound("cap".into()))?;
    for binding in &mut joined.library.bindings {
        if binding.id == cap_binding.id
            && let Some(first) = binding.definition.mappings.first().map(|m| m.terminal)
        {
            for m in &mut binding.definition.mappings {
                m.terminal = first;
            }
        }
    }
    let r1 = joined
        .instance(b.r1)
        .ok_or_else(|| Error::NotFound("R1".into()))?;
    let error = variants::substitute_fit(&joined.library, r1, b.cap).err();
    assert_eq!(error.as_ref().map(Error::code), Some("conflict"));
    assert!(
        error
            .map(|e| e.to_string())
            .unwrap_or_default()
            .contains("nets would change")
    );
    // Same component, same binding: fits.
    variant(
        engine,
        VariantCommand::SetPart {
            instance: b.r1,
            variant: alt,
            part: Some(b.zero),
        },
    )?;
    Ok(())
}

#[test]
fn variant_edits_undo_round_trip_and_follow_removed_parts() -> Result<()> {
    let mut b = board()?;
    let engine = &mut b.engine;
    let lite = add(engine, "Lite")?;
    // Names are unique, case-insensitively, and bounded.
    for name in ["lite", " ", &"x".repeat(65)] {
        let refused = variant(
            engine,
            VariantCommand::Add {
                name: name.into(),
                copy_of: None,
            },
        );
        assert!(refused.is_err(), "{name:?}");
    }
    variant(
        engine,
        VariantCommand::SetPopulation {
            instances: vec![b.r2],
            variant: Some(lite),
            population: Some(Population::NotPopulated),
        },
    )?;
    // Duplicate copies the overrides; rename and delete.
    let copy = match variant(
        engine,
        VariantCommand::Add {
            name: "Lite B".into(),
            copy_of: Some(lite),
        },
    )? {
        Outcome::VariantAdded { variant } => variant,
        other => return Err(Error::Invalid(format!("{other:?}"))),
    };
    assert_eq!(
        engine
            .project()
            .variants
            .get(copy)
            .map(|v| v.overrides.len()),
        Some(1)
    );
    variant(
        engine,
        VariantCommand::Rename {
            variant: copy,
            name: "Lite rev B".into(),
        },
    )?;
    activate(engine, Some(copy))?;
    variant(engine, VariantCommand::Remove { variant: copy })?;
    assert_eq!(
        engine.project().variants.active,
        None,
        "the default is active"
    );
    engine.undo()?;
    assert_eq!(engine.project().variants.active, Some(copy));
    assert_eq!(
        engine.undo_step().map(|s| s.title),
        Some("Choose the active assembly variant")
    );
    engine.redo()?;
    assert!(engine.project().variants.get(copy).is_none());

    // Restating the state takes no undo step.
    let revision = engine.revision();
    activate(engine, None)?;
    assert_eq!(engine.revision(), revision);

    // The file keeps the typed state and the variants.
    activate(engine, Some(lite))?;
    variant(
        engine,
        VariantCommand::SetPopulation {
            instances: vec![b.r1],
            variant: None,
            population: Some(Population::NotPopulated),
        },
    )?;
    let file = format::EditorFile::from_document(engine.document());
    let bytes = format::encode(&file)?;
    let text = String::from_utf8_lossy(&bytes);
    assert!(
        text.contains("\"not_populated\"") && text.contains("\"population\""),
        "typed, not a value"
    );
    assert!(!text.contains("DNP"));
    let reopened = format::decode(&bytes)?;
    assert_eq!(reopened.project.variants, engine.project().variants);
    assert_eq!(
        reopened.project.instance(b.r1).map(|i| i.population),
        Some(Population::NotPopulated)
    );

    // Removing a part takes its overrides with it.
    let demo = demo_fixture(engine)?;
    let (r3, _) = place(engine, &demo, "R3", 1, [0.0, 20.0])?;
    variant(
        engine,
        VariantCommand::SetPopulation {
            instances: vec![r3],
            variant: Some(lite),
            population: Some(Population::NotPopulated),
        },
    )?;
    let overrides = |e: &Engine| e.project().variants.get(lite).map(|v| v.overrides.len());
    assert_eq!(overrides(engine), Some(2));
    apply(
        engine,
        Operation::Instance(crate::InstanceCommand::Remove { instance: r3 }),
    )?;
    assert_eq!(overrides(engine), Some(1), "R3's override went with it");
    Ok(())
}

#[test]
fn a_substitute_left_behind_by_the_library_is_an_issue_not_a_silent_swap() -> Result<()> {
    let mut b = board()?;
    let engine = &mut b.engine;
    let alt = add(engine, "Alt")?;
    variant(
        engine,
        VariantCommand::SetPart {
            instance: b.r1,
            variant: alt,
            part: Some(b.cap),
        },
    )?;
    let mut project = engine.project().clone();
    project.variants.active = Some(alt);
    // The capacitor's footprint lands moved (as a library update could).
    let cap_footprints: Vec<_> = project
        .library
        .part(b.cap)
        .and_then(|p| project.library.binding(p.definition.binding))
        .map(|b| b.definition.footprints.clone())
        .unwrap_or_default();
    for f in &mut project.library.footprints {
        if cap_footprints.iter().any(|r| r.id == f.id)
            && let Some(pad) = f.definition.pads.first_mut()
        {
            pad.position = pt([-1.0, 0.0]);
        }
    }
    let findings = variants::findings(&project);
    assert_eq!(
        findings.iter().map(|f| f.code.as_str()).collect::<Vec<_>>(),
        [variants::SUBSTITUTE_UNFIT]
    );
    let r1 = project
        .instance(b.r1)
        .ok_or_else(|| Error::NotFound("R1".into()))?;
    let fitted = variants::fitted(&project, r1);
    assert!(!fitted.substituted, "the default part is fitted");
    Ok(())
}

/// A copy is the same part: pasting a part left off by default gives a
/// copy left off too, in the BOM and on the board, not a fitted one.
#[test]
fn a_copied_part_keeps_its_default_population() -> Result<()> {
    let mut b = board()?;
    let engine = &mut b.engine;
    variant(
        engine,
        VariantCommand::SetPopulation {
            instances: vec![b.r2],
            variant: None,
            population: Some(Population::NotPopulated),
        },
    )?;
    let QueryResult::SheetClip(clip) = engine.query(Query::SheetCopy {
        sheet: None,
        objects: vec![crate::schematic::ClipObject::Instance { id: b.r2 }],
    })?
    else {
        return Err(Error::Invalid("not a clip".into()));
    };
    let Outcome::Pasted { report } = apply(
        engine,
        Operation::Schematic(crate::schematic::SchematicCommand::Paste {
            sheet: None,
            clip,
            offset: pt([0.0, 30.0]),
        }),
    )?
    else {
        return Err(Error::Invalid("not pasted".into()));
    };
    let [copy] = report.symbols.as_slice() else {
        return Err(Error::Invalid(format!("{report:?}")));
    };
    assert_eq!(
        engine.project().instance(copy.instance).map(|i| i.population),
        Some(Population::NotPopulated)
    );
    sync_onto_board(engine)?;
    let (fitted, not) = bom_split(engine)?;
    assert!(not.contains(&copy.designator), "{not:?}");
    assert!(!fitted.contains(&copy.designator), "{fitted:?}");
    assert!(!placed_designators(engine)?.contains(&copy.designator));
    Ok(())
}
