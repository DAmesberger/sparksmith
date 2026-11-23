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
    // Fitting the lands is not a reviewed purchase.
    assert!(r1_line.fitted && r1_line.unresolved);
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
        portable: false,
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
        engine
            .project()
            .instance(copy.instance)
            .map(|i| i.population),
        Some(Population::NotPopulated)
    );
    sync_onto_board(engine)?;
    let (fitted, not) = bom_split(engine)?;
    assert!(not.contains(&copy.designator), "{not:?}");
    assert!(!fitted.contains(&copy.designator), "{fitted:?}");
    assert!(!placed_designators(engine)?.contains(&copy.designator));
    Ok(())
}

fn unresolved_r1(engine: &Engine) -> Result<Vec<String>> {
    let report = bom(engine)?;
    let line = report
        .lines
        .iter()
        .find(|l| l.designators == ["R1"])
        .ok_or_else(|| Error::NotFound("R1's line".into()))?;
    let mut codes: Vec<String> = variants::findings(engine.project())
        .into_iter()
        .filter(|f| {
            f.objects.contains(&crate::ObjectRef::Instance {
                id: line.instances[0],
            })
        })
        .map(|f| f.code.to_string())
        .collect();
    codes.sort();
    assert_eq!(line.unresolved, !codes.is_empty(), "{codes:?}");
    Ok(codes)
}

/// Physical fit is not electrical equivalence: a substitute on the same
/// lands with the same pad grouping (which a swapped VCC/GND pinout or a
/// lower-rated part also has) is resolved only after a person reviewed
/// its pinout, and only when it meets the requirements of the part it
/// replaces or each miss is a recorded exception with a reason.
#[test]
fn a_substitute_is_resolved_only_after_review_and_within_its_requirements() -> Result<()> {
    let mut b = board()?;
    let engine = &mut b.engine;
    let resistance = crate::instances::requirements(
        &engine.project().library,
        engine
            .project()
            .instance(b.r1)
            .ok_or_else(|| Error::NotFound("R1".into()))?,
    )?
    .into_iter()
    .find(|r| r.label == "Resistance")
    .ok_or_else(|| Error::NotFound("R1 requires a resistance".into()))?;
    let demo = demo_fixture(engine)?;
    let rated = |engine: &mut Engine, mpn: &str, ohms: i64| {
        let mut body = part(
            mpn,
            ComponentRef::new(demo.component, Revision::FIRST),
            BindingRef::new(demo.binding, Revision::FIRST),
        );
        body.specs = vec![crate::fields::RatedSpec {
            field: resistance.field,
            value: crate::fields::FieldValue::Quantity {
                si: dec(ohms as f64),
                display_unit: Some("Ω".into()),
            },
            meaning: crate::fields::SpecMeaning::ExactNominal,
        }];
        publish_part(engine, body)
    };
    let same = rated(engine, "EX-R0603-10K", 10_000)?;
    let lower = rated(engine, "EX-R0603-1K", 1_000)?;
    let alt = add(engine, "Alt")?;
    activate(engine, Some(alt))?;
    let fit = |engine: &mut Engine, part: PartRef| {
        variant(
            engine,
            VariantCommand::SetPart {
                instance: b.r1,
                variant: alt,
                part: Some(part),
            },
        )
    };
    let review = |engine: &mut Engine, equivalent: bool, exceptions| {
        variant(
            engine,
            VariantCommand::ReviewSubstitute {
                instance: b.r1,
                variant: alt,
                equivalent,
                exceptions,
            },
        )
    };
    let (unreviewed, unmet) = (variants::SUBSTITUTE_UNREVIEWED, variants::SUBSTITUTE_UNMET);

    // Reviewing a substitute the variant does not fit is refused.
    assert!(review(engine, true, vec![]).is_err());

    // An equal part on the same lands: resolved once its pinout is
    // reviewed.
    fit(engine, same)?;
    assert_eq!(unresolved_r1(engine)?, [unreviewed]);
    review(engine, true, vec![])?;
    assert!(unresolved_r1(engine)?.is_empty());

    // A part rated otherwise: fitting it voids the review, and reviewing
    // its pinout leaves the missed requirement.
    fit(engine, lower)?;
    assert_eq!(unresolved_r1(engine)?, [unmet, unreviewed]);
    review(engine, true, vec![])?;
    assert_eq!(unresolved_r1(engine)?, [unmet]);
    // An exception needs a reason.
    let exception = |reason: &str| crate::model::variants::RequirementException {
        field: resistance.field.id,
        reason: reason.into(),
    };
    let kept = Snapshot::of(engine);
    assert!(review(engine, true, vec![exception(" ")]).is_err());
    kept.assert_kept(engine, "an exception without a reason");
    review(
        engine,
        true,
        vec![exception("1 kΩ is the bias the Alt build needs")],
    )?;
    assert!(unresolved_r1(engine)?.is_empty());

    // The review is saved with the variant, and one undo takes it back.
    let bytes = format::encode(&format::EditorFile::from_document(engine.document()))?;
    let reopened = format::decode(&bytes)?;
    assert_eq!(reopened.project.variants, engine.project().variants);
    engine.undo()?;
    assert_eq!(unresolved_r1(engine)?, [unmet]);
    Ok(())
}

/// A board whose Alt variant fits a 1 kΩ substitute on R1 (which asks for
/// 10 kΩ), reviewed as equivalent with the resistance excepted: resolved.
fn reviewed_substitute() -> Result<(Board, Id, PartRef)> {
    let mut b = board()?;
    let engine = &mut b.engine;
    let r1 = engine
        .project()
        .instance(b.r1)
        .ok_or_else(|| Error::NotFound("R1".into()))?
        .clone();
    let resistance = crate::instances::requirements(&engine.project().library, &r1)?
        .into_iter()
        .find(|r| r.label == "Resistance")
        .ok_or_else(|| Error::NotFound("R1 requires a resistance".into()))?;
    let demo = demo_fixture(engine)?;
    let mut body = part(
        "EX-R0603-1K",
        ComponentRef::new(demo.component, Revision::FIRST),
        BindingRef::new(demo.binding, Revision::FIRST),
    );
    body.specs = vec![crate::fields::RatedSpec {
        field: resistance.field,
        value: crate::fields::FieldValue::Quantity {
            si: dec(1_000.0),
            display_unit: Some("Ω".into()),
        },
        meaning: crate::fields::SpecMeaning::ExactNominal,
    }];
    let lower = publish_part(engine, body)?;
    let alt = add(engine, "Alt")?;
    activate(engine, Some(alt))?;
    variant(
        engine,
        VariantCommand::SetPart {
            instance: b.r1,
            variant: alt,
            part: Some(lower),
        },
    )?;
    variant(
        engine,
        VariantCommand::ReviewSubstitute {
            instance: b.r1,
            variant: alt,
            equivalent: true,
            exceptions: vec![crate::model::variants::RequirementException {
                field: resistance.field.id,
                reason: "1 kΩ is the bias the Alt build needs".into(),
            }],
        },
    )?;
    assert!(unresolved_r1(engine)?.is_empty());
    Ok((b, alt, lower))
}

/// The messages of R1's variant findings in `project`.
fn r1_messages(project: &crate::model::Project, r1: Id) -> Vec<String> {
    variants::findings(project)
        .into_iter()
        .filter(|f| f.objects.contains(&crate::ObjectRef::Instance { id: r1 }))
        .map(|f| f.message)
        .collect()
}

/// Move R1 to the demo component's next revision, published after `change`
/// edits the drafts.
fn revise_r1(b: &mut Board, change: impl FnOnce(&mut Engine) -> Result<()>) -> Result<()> {
    let f = demo_fixture(&b.engine)?;
    let target = cascade(&mut b.engine, &f, change)?;
    let revision = b.engine.revision();
    b.engine.apply_update(
        revision,
        crate::updates::Change::ComponentRevision {
            instances: vec![b.r1],
            target,
            resolutions: Default::default(),
        },
        true,
    )
}

/// A review is bound to exactly what the person decided on: the component
/// revisions, each pad's terminal, the requirements and the rated values.
/// Changing any one of them voids it (Issues names what changed and the
/// purchase is unresolved again); the stored decision stays, and undo
/// brings back the state it applied to.
#[test]
fn a_substitute_review_stands_only_while_what_it_decided_on_is_unchanged() -> Result<()> {
    let unreviewed = variants::SUBSTITUTE_UNREVIEWED;
    let unmet = variants::SUBSTITUTE_UNMET;

    // The requirement the exception accepted moves from 10 kΩ to 220 kΩ.
    let (mut b, alt, _) = reviewed_substitute()?;
    let resistance = crate::instances::requirements(
        &b.engine.project().library,
        b.engine
            .project()
            .instance(b.r1)
            .ok_or_else(|| Error::NotFound("R1".into()))?,
    )?
    .into_iter()
    .find(|r| r.label == "Resistance")
    .ok_or_else(|| Error::NotFound("resistance".into()))?;
    set_properties(&mut b.engine, b.r1, vec![text(resistance.field, "220 kΩ")])?;
    assert_eq!(unresolved_r1(&b.engine)?, [unmet, unreviewed]);
    let messages = r1_messages(b.engine.project(), b.r1);
    assert!(
        messages.iter().any(|m| m.contains("no longer applies")
            && m.contains("the requirements")
            && !m.contains("pad-to-terminal")),
        "{messages:?}"
    );
    assert!(
        b.engine
            .project()
            .variants
            .get(alt)
            .is_some_and(|v| v.reviews.contains_key(&b.r1)),
        "the earlier decision is kept, not erased"
    );
    b.engine.undo()?;
    assert!(
        unresolved_r1(&b.engine)?.is_empty(),
        "undo restores what was reviewed"
    );

    // A new component revision that changes nothing else.
    let (mut b, _, _) = reviewed_substitute()?;
    revise_r1(&mut b, |_| Ok(()))?;
    let messages = r1_messages(b.engine.project(), b.r1);
    // The voided review takes its exception with it.
    assert_eq!(
        unresolved_r1(&b.engine)?,
        [unmet, unreviewed],
        "{messages:?}"
    );
    assert!(
        messages
            .iter()
            .any(|m| m.contains("the component revision changed")),
        "{messages:?}"
    );

    // The base part's pad 1 now carries terminal 2 (and pad 2 terminal 1);
    // the substitute still lands pad 1 on terminal 1.
    let (mut b, _, lower) = reviewed_substitute()?;
    let f = demo_fixture(&b.engine)?;
    revise_r1(&mut b, |engine| {
        with(engine, f.binding, binding_body, |binding| {
            let first = binding.mappings[0].terminal;
            binding.mappings[0].terminal = binding.mappings[1].terminal;
            binding.mappings[1].terminal = first;
        })
    })?;
    {
        let project = b.engine.project();
        let r1 = project
            .instance(b.r1)
            .ok_or_else(|| Error::NotFound("R1".into()))?;
        assert!(
            variants::substitute_fit(&project.library, r1, lower).is_ok(),
            "the substitute still fits physically"
        );
    }
    let messages = r1_messages(b.engine.project(), b.r1);
    assert!(
        messages
            .iter()
            .any(|m| m.contains("no longer applies") && m.contains("the pad-to-terminal map")),
        "{messages:?}"
    );
    assert!(unresolved_r1(&b.engine)?.contains(&unreviewed.to_owned()));

    // The substitute's rated value differs from the one reviewed.
    let (b, _, lower) = reviewed_substitute()?;
    let mut project = b.engine.project().clone();
    for p in &mut project.library.parts {
        if p.id == lower.id
            && let Some(crate::fields::FieldValue::Quantity { si, .. }) =
                p.definition.specs.first_mut().map(|s| &mut s.value)
        {
            *si = dec(2_000.0);
        }
    }
    let messages = r1_messages(&project, b.r1);
    assert!(
        messages
            .iter()
            .any(|m| m.contains("the rated values") && !m.contains("the requirements")),
        "{messages:?}"
    );
    Ok(())
}

#[test]
fn substitute_preserves_typed_tie_groups_on_the_matched_lands() -> Result<()> {
    use crate::ids::{TerminalId, TieId};
    use crate::library::tie::{TieForm, TieSpec};
    let mut engine = Engine::default();
    let mut make = |name: &str, ends: [u32; 2], link: bool| -> Result<(Fixture, PartRef)> {
        let [a, b] = ends;
        let f = draft(
            &mut engine,
            &Spec {
                name,
                terminals: vec![(a, "A", passive()), (b, "B", passive())],
                units: vec![(1, "A", vec![(a, [-4., 0.]), (b, [4., 0.])])],
                package: vec![(1, "1"), (2, "2")],
                mapping: vec![(1, a), (2, b)],
                physical: true,
            },
        )?;
        if link {
            rewrite(&mut engine, f.component, |d| {
                if let Definition::Component(c) = d {
                    c.ties = vec![TieSpec {
                        id: TieId(a),
                        ends: ends.map(TerminalId).to_vec(),
                        form: TieForm::ZeroOhm,
                    }];
                }
            })?;
        }
        publish(&mut engine, f.all())?;
        let p = publish_part(
            &mut engine,
            part(name, f.r1(), BindingRef::new(f.binding, Revision::FIRST)),
        )?;
        Ok((f, p))
    };
    let (passive, ordinary_part) = make("Resistor control", [1, 2], false)?;
    let (link, zero_part) = make("Jumper resistor", [1, 2], true)?;
    let (_, compatible) = make("Compatible jumper", [27, 11], true)?;
    let (regular, _) = place(&mut engine, &passive, "R1", 1, [12., 12.])?;
    let (jumper, _) = place(&mut engine, &link, "R2", 1, [26., 12.])?;
    let alt = add(&mut engine, "Alternate")?;
    for (instance, part) in [(regular, zero_part), (jumper, ordinary_part)] {
        let before = Snapshot::of(&engine);
        let result = variant(
            &mut engine,
            VariantCommand::SetPart {
                instance,
                variant: alt,
                part: Some(part),
            },
        );
        assert!(matches!(result, Err(Error::Conflict(_))));
        before.assert_kept(&engine, "changing the typed join on the same lands");
    }
    variant(
        &mut engine,
        VariantCommand::SetPart {
            instance: jumper,
            variant: alt,
            part: Some(compatible),
        },
    )?;
    assert_eq!(
        engine
            .project()
            .variants
            .get(alt)
            .and_then(|v| v.overrides.get(&jumper))
            .and_then(|o| o.part),
        Some(compatible)
    );
    Ok(())
}

/// Review §12: the firmware device is the part the active variant fits:
/// its Part and MPN, as the BOM buys it. A substitute of another component
/// is another device, and its firmware is refused naming the variant.
#[test]
fn firmware_names_the_device_the_active_variant_fits() -> Result<()> {
    let (mut b, alt, lower) = reviewed_substitute()?;
    let device = crate::firmware::pin_config(b.engine.project(), b.r1)?.device;
    assert_eq!(device.part, Some(lower));
    assert_eq!(device.mpn.as_deref(), Some("EX-R0603-1K"));
    activate(&mut b.engine, None)?;
    let device = crate::firmware::pin_config(b.engine.project(), b.r1)?.device;
    assert_eq!(
        device.part,
        b.engine.project().instance(b.r1).and_then(|i| i.part)
    );
    // Another component on the same lands: not this report's device.
    activate(&mut b.engine, Some(alt))?;
    variant(
        &mut b.engine,
        VariantCommand::SetPart {
            instance: b.r1,
            variant: alt,
            part: Some(b.cap),
        },
    )?;
    let refused = crate::firmware::pin_config(b.engine.project(), b.r1);
    assert!(
        matches!(&refused, Err(Error::Conflict(m)) if m.contains("“Alt”") && m.contains("EX-C0603-100N")),
        "{refused:?}"
    );
    Ok(())
}

/// The one assembly decision keeps purchasing, population and placement
/// apart: a part kept off the BOM is still fitted and placed, a part kept
/// out of the position files is still bought, and a part not fitted is
/// listed but neither fitted nor placed.
#[test]
fn purchasing_population_and_placement_are_separate_answers() -> Result<()> {
    let b = board()?;
    let mut file = b.engine.to_file();
    for i in &mut file.project.instances {
        if i.id == b.r1 {
            i.bom_excluded = true;
        }
        if i.id == b.r2 {
            i.placement_excluded = true;
        }
    }
    let mut engine = Engine::from_file(file)?;
    let decide = |engine: &Engine, id: Id| -> Result<(bool, bool, bool)> {
        let project = engine.project();
        let i = project
            .instance(id)
            .ok_or_else(|| Error::NotFound("instance".into()))?;
        let d = crate::completeness::assembly(project, i);
        Ok((d.purchasing, d.populated(), d.placement))
    };
    assert_eq!(decide(&engine, b.r1)?, (false, true, true));
    assert_eq!(decide(&engine, b.r2)?, (true, true, false));
    let (fitted, not) = bom_split(&engine)?;
    assert!(!fitted.contains(&"R1".to_owned()) && !not.contains(&"R1".to_owned()));
    assert!(fitted.contains(&"R2".to_owned()), "{fitted:?}");
    let placed = placed_designators(&engine)?;
    assert!(placed.contains(&"R1".to_owned()), "{placed:?}");
    assert!(!placed.contains(&"R2".to_owned()), "{placed:?}");
    let parts = assembly::bom_parts_csv(engine.project())?;
    assert!(!parts.contains("\nR1,"), "{parts}");
    assert!(parts.contains("\nR2,"), "{parts}");
    // Not fitted: still listed, not fitted, not placed.
    variant(
        &mut engine,
        VariantCommand::SetPopulation {
            instances: vec![b.r2],
            variant: None,
            population: Some(Population::NotPopulated),
        },
    )?;
    assert_eq!(decide(&engine, b.r2)?, (true, false, false));
    // The flags survive saving and a copy keeps them.
    let bytes = format::encode(&format::EditorFile::from_document(engine.document()))?;
    let reopened = format::decode(&bytes)?;
    assert!(
        reopened
            .project
            .instances
            .iter()
            .any(|i| i.id == b.r1 && i.bom_excluded)
    );
    Ok(())
}

#[test]
fn paired_zero_ohm_package_accepts_reordered_equivalents_and_refuses_crossed_links() -> Result<()> {
    use crate::ids::{TerminalId, TieId};
    use crate::library::tie::{TieForm, TieSpec};
    let mut engine = Engine::default();
    let mut make =
        |name: &str, mapped: [u32; 4], groups: [[u32; 2]; 2]| -> Result<(Fixture, PartRef)> {
            let [a, b, c, d] = mapped;
            let f = draft(
                &mut engine,
                &Spec {
                    name,
                    terminals: vec![
                        (a, "A", passive()),
                        (b, "B", passive()),
                        (c, "C", passive()),
                        (d, "D", passive()),
                    ],
                    units: vec![(
                        1,
                        "A",
                        vec![
                            (a, [-4., -2.]),
                            (b, [4., -2.]),
                            (c, [-4., 2.]),
                            (d, [4., 2.]),
                        ],
                    )],
                    package: vec![(1, "1"), (2, "2"), (3, "3"), (4, "4")],
                    mapping: vec![(1, a), (2, b), (3, c), (4, d)],
                    physical: true,
                },
            )?;
            rewrite(&mut engine, f.component, |def| {
                if let Definition::Component(component) = def {
                    component.ties = groups
                        .map(|pair| TieSpec {
                            id: TieId(pair[0]),
                            form: TieForm::ZeroOhm,
                            ends: pair.map(TerminalId).to_vec(),
                        })
                        .to_vec();
                }
            })?;
            publish(&mut engine, f.all())?;
            let part = publish_part(
                &mut engine,
                part(name, f.r1(), BindingRef::new(f.binding, Revision::FIRST)),
            )?;
            Ok((f, part))
        };
    let (original, _) = make("Paired links", [1, 2, 3, 4], [[1, 2], [3, 4]])?;
    let (_, reordered) = make("Renumbered links", [9, 8, 7, 6], [[6, 7], [8, 9]])?;
    let (_, crossed) = make("Crossed links", [9, 8, 7, 6], [[6, 8], [7, 9]])?;
    let (owner, _) = place(&mut engine, &original, "RN1", 1, [20., 20.])?;
    let alt = add(&mut engine, "Alternate package")?;
    variant(
        &mut engine,
        VariantCommand::SetPart {
            instance: owner,
            variant: alt,
            part: Some(reordered),
        },
    )?;
    let before = Snapshot::of(&engine);
    let result = variant(
        &mut engine,
        VariantCommand::SetPart {
            instance: owner,
            variant: alt,
            part: Some(crossed),
        },
    );
    assert!(
        matches!(result,Err(Error::Conflict(ref msg)) if msg.contains("lands joined by a tie"))
    );
    before.assert_kept(&engine, "crossed links in a pin-compatible package");
    assert_eq!(
        engine
            .project()
            .variants
            .get(alt)
            .and_then(|v| v.overrides.get(&owner))
            .and_then(|o| o.part),
        Some(reordered)
    );
    Ok(())
}
