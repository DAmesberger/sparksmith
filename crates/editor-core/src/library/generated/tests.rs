//! Generated parts through the engine with a stand-in runtime: the core's
//! side of GP1 (expansion, record, embedding, dedupe, replacement and the
//! checks a host runs on load). The real module runs in the session tests.
use super::{GeneratorParams, GeneratorRuntime, GeneratorScope, verify};
use crate::commands::{Generate, GenerateReport, LibraryCommand, ModuleSource, Outcome};
use crate::diagnostics::Error;
use crate::ids::Revision;
use crate::library::bundle::{AssetBytes, PackedAsset, asset_hash};
use crate::library::generated;
use crate::{Engine, Operation};
use sparksmith_generator_abi::v1::*;
use sparksmith_generator_abi::{Failure, Generated};
use std::sync::Arc;

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

/// A stand-in module: a two-terminal part whose pads sit `length / 2 −
/// 0.35 mm` from the origin. Its "bytes" choose a variant: `b"v2"` is
/// version 2 and moves the pads, as a changed module would; `REBUILT` is
/// version 1 built to other bytes; `OTHER` is another generator.
struct StandIn;

const MODULE: &[u8] = b"stand-in module";
const CHANGED: &[u8] = b"v2";
const REBUILT: &[u8] = b"stand-in module, rebuilt";
const OTHER: &[u8] = b"another generator";
/// Version 2 of the stand-in that inserts a pad, `3` (landing terminal
/// 1), between pads 1 and 2.
const INSERTED: &[u8] = b"v2 with a pad inserted";
/// A module whose model names another part file than the host evaluated.
const OTHER_PART: &[u8] = b"names another part file";
/// A module whose terminal 1 is a GPIO (an input that can be an output)
/// named `IO0` and terminal 2 a ground pin.
const PIN_TABLE: &[u8] = b"with a pin table";
/// A module whose complete part is two orderable part numbers.
const ORDERABLE: &[u8] = b"with part numbers";
/// A module whose terminal 1 is an open-drain flag named `PG`.
const OPEN_DRAIN: &[u8] = b"with an open-drain flag";
/// The stand-in's part file (the core never evaluates it).
const PART: &[u8] = b"stand-in part file";

/// The stand-in's model: its part file at the body length.
fn model(length: i64, part: &[u8]) -> Model {
    Model::PartFile {
        asset: asset_hash(part).to_string(),
        values: [("length".to_string(), ParamValue::Int(length))]
            .into_iter()
            .collect(),
    }
}

/// What the host gives `library.generate` for the stand-in's part file
/// at `length`: the Model it becomes, its bytes, the values in mm and
/// what it evaluated to.
fn evaluated(length: i64) -> crate::commands::GeneratedModel {
    use crate::library::model3d::{AssetRef, ModelDefinition, ModelFormat, ModelProvenance};
    use crate::library::package_model::{PartDerived, PartValue};
    use crate::units::Nm;
    let nm = |v: i64| Nm::new(v).unwrap_or_default();
    let bounds = [
        [nm(-length / 2), nm(-400_000), nm(0)],
        [nm(length / 2), nm(400_000), nm(450_000)],
    ];
    crate::commands::GeneratedModel {
        definition: ModelDefinition {
            asset: AssetRef {
                hash: asset_hash(PART),
                bytes: PART.len() as u64,
                media_type: ModelFormat::Solidcraft.media_type().into(),
                file_name: "stand-in.solidcraft".into(),
            },
            format: ModelFormat::Solidcraft,
            bounds_nm: bounds,
            provenance: ModelProvenance {
                record: crate::library::part::Provenance {
                    licence: Some("MIT OR Apache-2.0".into()),
                    ..Default::default()
                },
                ..Default::default()
            },
            ..ModelDefinition::default()
        },
        name: "Stand-in part".into(),
        data: Some(AssetBytes::new(PART.to_vec())),
        values: [("length".to_string(), PartValue::Nm(length))]
            .into_iter()
            .collect(),
        derived: PartDerived {
            bounds_nm: bounds,
            bodies: 1,
            ..PartDerived::default()
        },
    }
}

fn length_param() -> Param {
    Param {
        key: "length".into(),
        label: "Body length".into(),
        help: None,
        ty: ParamType::Length {
            min_nm: 500_000,
            max_nm: 10_000_000,
            step_nm: None,
        },
        default: ParamValue::Int(1_600_000),
        group: None,
        context: None,
    }
}

fn graphic(key: &str, shape: Primitive) -> Graphic {
    Graphic {
        key: key.into(),
        shape,
        stroke_nm: 150_000,
        fill: Fill::None,
    }
}

fn generation_with(
    length: i64,
    shift: i64,
    scope: Scope,
    part: &[u8],
    inserted: bool,
) -> Generation {
    let x = length / 2 - 350_000 + shift;
    let pad = |key: &str, at: Point| Pad {
        key: key.into(),
        terminal: key.into(),
        shape: PadShape::Rect {
            size: [900_000, 1_000_000],
        },
        at,
        rot_mdeg: 0,
        side: Side::Top,
        hole: None,
        mask: Opening::Default,
        paste: Opening::Default,
        number: None,
        kind: None,
    };
    let edge = x + 700_000;
    let component = scope == Scope::Component;
    Generation {
        orderable: Vec::new(),
        marking: None,
        component: component.then(|| Component {
            terminals: ["1", "2"]
                .iter()
                .map(|k| Terminal {
                    key: (*k).into(),
                    name: (*k).into(),
                    direction: Direction::Passive,
                    alternates: Vec::new(),
                    drive: None,
                })
                .collect(),
            prefix: "R".into(),
            name: "Stand-in resistor".into(),
            fields: vec!["value".into()],
        }),
        symbol: component.then(|| Symbol {
            units: vec![Unit {
                key: "A".into(),
                pins: vec![
                    SymbolPin {
                        terminal: "1".into(),
                        at: [-5_080_000, 0],
                        length_nm: 2_540_000,
                        rot_mdeg: 0,
                        show_name: false,
                        show_number: false,
                    },
                    SymbolPin {
                        terminal: "2".into(),
                        at: [5_080_000, 0],
                        length_nm: 2_540_000,
                        rot_mdeg: 180_000,
                        show_name: false,
                        show_number: false,
                    },
                ],
                graphics: vec![
                    Graphic {
                        key: "body".into(),
                        shape: Primitive::Rect {
                            min: [-2_540_000, -1_000_000],
                            max: [2_540_000, 1_000_000],
                        },
                        stroke_nm: 250_000,
                        fill: Fill::Body,
                    },
                    graphic(
                        "designator",
                        Primitive::Text {
                            at: [0, 1_500_000],
                            height_nm: 1_270_000,
                            content: TextContent::Designator,
                            h_align: HAlign::Center,
                            v_align: VAlign::Bottom,
                        },
                    ),
                    graphic(
                        "arc",
                        Primitive::Arc {
                            center: [0, 0],
                            start: [-500_000, 0],
                            end: [500_000, 0],
                            clockwise: true,
                        },
                    ),
                ],
            }],
        }),
        package: Package {
            terminals: vec!["1".into(), "2".into()],
            geometry: PackageGeometry {
                envelope_nm: [length, 800_000, 450_000],
                seating_nm: 0,
                leads: Vec::new(),
                pin1: "1".into(),
            },
        },
        footprint: Footprint {
            pads: if inserted {
                let mut third = pad("1", [0, 1_200_000]);
                third.key = "3".into();
                third.shape = PadShape::Rect {
                    size: [200_000, 200_000],
                };
                vec![pad("1", [-x, 0]), third, pad("2", [x, 0])]
            } else {
                vec![pad("1", [-x, 0]), pad("2", [x, 0])]
            },
            courtyard: vec![
                [-edge, -1_500_000],
                [edge, -1_500_000],
                [edge, 1_500_000],
                [-edge, 1_500_000],
            ],
            silk: Vec::new(),
            assembly: vec![graphic(
                "assembly",
                Primitive::Rect {
                    min: [-length / 2, -400_000],
                    max: [length / 2, 400_000],
                },
            )],
            keepouts: Vec::new(),
        },
        binding: if component {
            ["1", "2"]
                .iter()
                .map(|k| BindingEntry {
                    package_terminal: (*k).into(),
                    terminal: (*k).into(),
                })
                .collect()
        } else {
            Vec::new()
        },
        model: Some(model(length, part)),
        fields: Vec::new(),
        notes: vec!["A stand-in.".into()],
    }
}

impl GeneratorRuntime for StandIn {
    fn describe(&self, module: &[u8]) -> std::result::Result<Description, Failure> {
        Ok(Description {
            abi: ABI,
            id: if module == OTHER { "other" } else { "stand-in" }.into(),
            version: if module == CHANGED || module == INSERTED {
                2
            } else {
                1
            },
            title: "Stand-in".into(),
            licence: "MIT OR Apache-2.0".into(),
            deterministic: true,
            capabilities: vec![Capability {
                name: GENERATE_PART.into(),
                effect: Effect::None,
                title: None,
                params: vec![length_param()],
                rules: Vec::new(),
                presets: Vec::new(),
                part: Some(PartOffer {
                    families: vec!["chip".into()],
                    scopes: vec![Scope::Component, Scope::Package],
                    grammar: Vec::new(),
                    instance_fields: Vec::new(),
                }),
            }],
            needs: Vec::new(),
        })
    }

    fn generate(
        &self,
        module: &[u8],
        request: &Request,
    ) -> std::result::Result<Generated, Failure> {
        let length = match request.params.get("length") {
            Some(ParamValue::Int(v)) => *v,
            _ => return Err(Failure::one("param.missing", "length".into())),
        };
        let shift = if module == CHANGED { 10_000 } else { 0 };
        let part = if module == OTHER_PART { CHANGED } else { PART };
        let mut generation =
            generation_with(length, shift, request.scope, part, module == INSERTED);
        if module == OPEN_DRAIN
            && let Some(first) = generation
                .component
                .as_mut()
                .and_then(|c| c.terminals.first_mut())
        {
            first.name = "PG".into();
            first.direction = Direction::Output;
            first.drive = Some(Drive::OpenDrain);
        }
        if module == ORDERABLE && request.scope == Scope::Component {
            generation.orderable = ["SI-1000", "SI-1000-T"]
                .iter()
                .map(|mpn| Orderable {
                    manufacturer: "Stand-in Parts".into(),
                    mpn: (*mpn).into(),
                })
                .collect();
        }
        if module == PIN_TABLE
            && let Some(component) = generation.component.as_mut()
        {
            for (t, (name, direction, alternates)) in component.terminals.iter_mut().zip([
                ("IO0", Direction::Input, vec![Direction::Output]),
                ("GND", Direction::PowerInput, Vec::new()),
            ]) {
                t.name = name.into();
                t.direction = direction;
                t.alternates = alternates;
            }
        }
        let canonical = sparksmith_generator_abi::encode(&generation)
            .map_err(|e| Failure::one("generator.invalid_output", e.to_string()))?;
        Ok(Generated {
            result_hash: sparksmith_generator_abi::result_hash(&canonical),
            canonical,
            generation,
            fuel_used: None,
        })
    }
}

fn engine() -> Engine {
    let mut engine = Engine::default();
    engine.set_generators(Arc::new(StandIn));
    engine
}

fn request(module: &[u8], scope: GeneratorScope, length: i64) -> Generate {
    let mut params = GeneratorParams::new();
    params.insert("length".into(), generated::ParamValue::Int(length));
    Generate {
        module: ModuleSource {
            hash: asset_hash(module),
            data: Some(AssetBytes::new(module.to_vec())),
        },
        scope,
        params,
        name: None,
        replaces: None,
        model: Some(evaluated(length)),
        bundle: None,
        preset: None,
    }
}

fn generate(engine: &mut Engine, generate: Generate) -> crate::Result<GenerateReport> {
    let outcome = engine.apply(
        engine.revision(),
        Operation::Library(LibraryCommand::Generate(Box::new(generate))),
    )?;
    match outcome {
        Outcome::Generated { report } => Ok(report),
        other => Err(Error::Invalid(format!("unexpected outcome {other:?}"))),
    }
}

fn run(module: &PackedAsset, request: &Request) -> std::result::Result<Generated, Failure> {
    StandIn.generate(&module.data, request)
}

#[test]
fn a_part_is_published_embedded_and_recorded_in_one_step() -> TestResult {
    let mut engine = engine();
    let report = generate(
        &mut engine,
        request(MODULE, GeneratorScope::Component, 1_600_000),
    )?;
    assert!(report.created);
    assert_eq!(report.revisions.len(), 5);
    assert_eq!(report.notes, vec!["A stand-in.".to_string()]);
    assert_eq!(report.key.generator, "stand-in");
    let library = &engine.project().library;
    assert_eq!(library.generated.len(), 1);
    assert_eq!(library.components.len(), 1);
    assert_eq!(library.footprints.len(), 1);
    assert!(
        engine.drafts().items.is_empty(),
        "the drafts were published and closed"
    );
    let module = generated::module(library, asset_hash(MODULE)).ok_or("module not embedded")?;
    assert_eq!(module.data.as_slice(), MODULE);
    let part = &library.generated[0];
    // Complete parameters, the default filled in, and the result hash.
    assert_eq!(part.from.params.len(), 1);
    assert_eq!(part.from.result, report.result);
    assert_eq!(part.root, report.root);
    // The file round-trips and the host's check passes.
    let file = engine.to_file();
    file.validate()?;
    let bytes = crate::format::encode(&file)?;
    let reread = crate::format::decode(&bytes)?;
    assert_eq!(reread, file);
    verify(&reread.project.library, &mut run)?;
    // The expansion is exact: symbol arcs at quarter turns, pins along the
    // axes, pads where the generation put them.
    // The package's model is the module's part file, published as a Model
    // of the project, at the module's values.
    let package = library.packages.first().ok_or("no package")?;
    let crate::library::PackageModel::PartFile(part) = &package.definition.model else {
        return Err("the package has no part file".into());
    };
    let model = library
        .model(part.model)
        .ok_or("the part file is not a Model")?;
    assert_eq!(model.definition.asset.hash, asset_hash(PART));
    assert_eq!(
        part.values.get("length"),
        Some(&crate::library::package_model::PartValue::Nm(1_600_000))
    );
    // The stored package's values edited behind the record: refused on
    // load.
    let mut edited = engine.to_file();
    let packages = &mut edited.project.library.packages;
    if let Some(crate::library::PackageModel::PartFile(part)) =
        packages.first_mut().map(|p| &mut p.definition.model)
    {
        part.values.insert(
            "length".into(),
            crate::library::package_model::PartValue::Nm(1_700_000),
        );
    }
    let error = verify(&edited.project.library, &mut run)
        .err()
        .ok_or("accepted")?;
    assert_eq!(error.rule(), Some("generated.expansion_changed"), "{error}");
    let footprint = library.footprints.first().ok_or("no footprint")?;
    assert_eq!(footprint.definition.pads[0].position.x_nm.get(), -450_000);
    assert_eq!(footprint.definition.pads[1].number, "2");
    Ok(())
}

#[test]
fn the_same_key_finds_the_part_and_changes_nothing() -> TestResult {
    let mut engine = engine();
    let first = generate(
        &mut engine,
        request(MODULE, GeneratorScope::Component, 1_600_000),
    )?;
    let revision = engine.revision();
    // The module is embedded now: no bytes needed.
    let mut again = request(MODULE, GeneratorScope::Component, 1_600_000);
    again.module.data = None;
    let second = generate(&mut engine, again)?;
    assert!(!second.created);
    assert_eq!(second.key, first.key);
    assert_eq!(second.root, first.root);
    assert_eq!(engine.revision(), revision);
    // Another length is another part, sharing the embedded module.
    let other = generate(
        &mut engine,
        request(MODULE, GeneratorScope::Package, 2_000_000),
    )?;
    assert!(other.created);
    assert_eq!(other.revisions.len(), 2);
    assert_ne!(other.key, first.key);
    let library = &engine.project().library;
    // The module and the part file, each packed once; one Model of it.
    assert_eq!(library.assets.len(), 2);
    assert_eq!(library.models.len(), 1);
    assert_eq!(library.generated.len(), 2);
    Ok(())
}

#[test]
fn refusals_leave_the_document_unchanged() -> TestResult {
    let mut engine = Engine::default();
    let before = engine.to_file();
    // No runtime.
    let none = generate(
        &mut engine,
        request(MODULE, GeneratorScope::Component, 1_600_000),
    );
    assert!(matches!(none, Err(Error::Unsupported(m)) if m.starts_with("generator.runtime")));
    engine.set_generators(Arc::new(StandIn));
    // Bytes that are not the named module.
    let mut forged = request(MODULE, GeneratorScope::Component, 1_600_000);
    forged.module.hash = asset_hash(b"something else");
    let refused = generate(&mut engine, forged);
    assert!(
        matches!(&refused, Err(Error::Invalid(m)) if m.starts_with("generated.module_changed")),
        "{refused:?}"
    );
    // A module that is not embedded, given without bytes.
    let mut missing = request(MODULE, GeneratorScope::Component, 1_600_000);
    missing.module.data = None;
    assert!(matches!(
        generate(&mut engine, missing),
        Err(Error::NotFound(m)) if m.starts_with("generated.module_missing")
    ));
    // A model the host did not evaluate: refused, a part never exists
    // without its model.
    let mut unevaluated = request(MODULE, GeneratorScope::Component, 1_600_000);
    unevaluated.model = None;
    let refused = generate(&mut engine, unevaluated);
    assert!(
        matches!(&refused, Err(Error::Invalid(m)) if m.starts_with("generated.model_unevaluated")),
        "{refused:?}"
    );
    // The host evaluated another part file than the module names.
    let other = generate(
        &mut engine,
        request(OTHER_PART, GeneratorScope::Component, 1_600_000),
    );
    assert!(
        matches!(&other, Err(Error::Invalid(m)) if m.starts_with("generated.model:")),
        "{other:?}"
    );
    // Other values than the module's.
    let mut values = request(MODULE, GeneratorScope::Component, 1_600_000);
    if let Some(model) = &mut values.model {
        model.values.insert(
            "length".into(),
            crate::library::package_model::PartValue::Nm(1_700_000),
        );
    }
    let refused = generate(&mut engine, values);
    assert!(
        matches!(&refused, Err(Error::Invalid(m)) if m.contains("value \"length\" is not the generator's")),
        "{refused:?}"
    );
    // A parameter out of the module's range.
    let small = generate(&mut engine, request(MODULE, GeneratorScope::Component, 1));
    assert!(
        matches!(&small, Err(Error::Invalid(m)) if m.starts_with("param.out_of_range")),
        "{small:?}"
    );
    assert_eq!(engine.to_file(), before);
    assert_eq!(engine.revision(), 0);
    Ok(())
}

#[test]
fn a_changed_stored_part_or_result_is_refused_naming_both_hashes() -> TestResult {
    let mut engine = engine();
    generate(
        &mut engine,
        request(MODULE, GeneratorScope::Component, 1_600_000),
    )?;
    let file = engine.to_file();
    // The stored footprint edited behind the record's back.
    let mut edited = file.clone();
    let footprints = &mut edited.project.library.footprints;
    let footprint = footprints.first_mut().ok_or("no footprint")?;
    footprint.definition.pads[0].position.x_nm = crate::units::Nm::new(-460_000)?;
    let error = verify(&edited.project.library, &mut run)
        .err()
        .ok_or("accepted")?;
    let message = error.to_string();
    assert_eq!(
        error.rule(),
        Some("generated.expansion_changed"),
        "{message}"
    );
    assert_eq!(message.matches("sha256 ").count(), 2, "{message}");
    // The module now making something else from the same parameters.
    let mut changed = |_: &PackedAsset, request: &Request| StandIn.generate(CHANGED, request);
    let error = verify(&file.project.library, &mut changed)
        .err()
        .ok_or("accepted")?;
    let message = error.to_string();
    assert_eq!(error.rule(), Some("generated.result_changed"), "{message}");
    assert!(message.contains(&file.project.library.generated[0].from.result.to_string()));
    // Embedded bytes that no longer hash to the record's module.
    let mut swapped = file.clone();
    let asset = swapped
        .project
        .library
        .assets
        .first_mut()
        .ok_or("no asset")?;
    asset.data = AssetBytes::new(CHANGED.to_vec());
    let error = verify(&swapped.project.library, &mut run)
        .err()
        .ok_or("accepted")?;
    let message = error.to_string();
    assert_eq!(error.rule(), Some("generated.module_changed"), "{message}");
    assert!(
        message.contains(&asset_hash(MODULE).to_string()),
        "{message}"
    );
    assert!(
        message.contains(&asset_hash(CHANGED).to_string()),
        "{message}"
    );
    Ok(())
}

#[test]
fn records_need_their_module_and_revisions() -> TestResult {
    let mut engine = engine();
    generate(
        &mut engine,
        request(MODULE, GeneratorScope::Component, 1_600_000),
    )?;
    let file = engine.to_file();
    let mut no_module = file.clone();
    no_module
        .project
        .library
        .assets
        .retain(|a| a.media_type != generated::MODULE_MEDIA_TYPE);
    let error = no_module.validate().err().ok_or("accepted")?;
    assert_eq!(error.rule(), Some("generated.module_missing"), "{error}");
    let mut twice = file.clone();
    let part = twice.project.library.generated[0].clone();
    twice.project.library.generated.push(part);
    assert!(twice.validate().is_err());
    let mut wrong_kinds = file;
    wrong_kinds.project.library.generated[0]
        .expansion
        .swap(1, 2);
    assert!(wrong_kinds.validate().is_err());
    Ok(())
}

#[test]
fn a_changed_module_makes_the_next_revisions_of_the_same_lineages() -> TestResult {
    let mut engine = engine();
    let first = generate(
        &mut engine,
        request(MODULE, GeneratorScope::Component, 1_600_000),
    )?;
    let mut update = request(CHANGED, GeneratorScope::Component, 1_600_000);
    update.replaces = Some(first.root);
    let second = generate(&mut engine, update)?;
    assert!(second.created);
    assert_eq!(second.root.id, first.root.id, "the same lineage");
    assert_eq!(
        second.root.revision,
        Revision::FIRST.next().ok_or("revision")?
    );
    for (old, new) in first.revisions.iter().zip(&second.revisions) {
        assert_eq!((old.kind, old.id), (new.kind, new.id));
        assert_eq!(new.revision.0, 2);
    }
    let library = &engine.project().library;
    assert_eq!(
        library.generated.len(),
        2,
        "both revisions keep their record"
    );
    let modules = library
        .assets
        .iter()
        .filter(|a| a.media_type == generated::MODULE_MEDIA_TYPE)
        .count();
    assert_eq!(modules, 2, "both modules are embedded");
    verify(library, &mut |module: &PackedAsset, request: &Request| {
        StandIn.generate(&module.data, request)
    })?;
    // The new component revision pins the new symbol and binding.
    let component = library
        .component(crate::ids::ComponentRef::new(
            second.root.id,
            second.root.revision,
        ))
        .ok_or("no component r2")?;
    assert_eq!(component.definition.representations[0].revision.0, 2);
    assert_eq!(component.definition.implementations[0].revision.0, 2);
    Ok(())
}

#[test]
fn a_newer_version_makes_the_next_revisions_and_an_older_one_is_refused() -> TestResult {
    let mut engine = engine();
    let first = generate(
        &mut engine,
        request(MODULE, GeneratorScope::Component, 1_600_000),
    )?;
    assert_eq!(first.replaced, None);
    // Version 2 with the same parameters: the same lineages, revision 2.
    let second = generate(
        &mut engine,
        request(CHANGED, GeneratorScope::Component, 1_600_000),
    )?;
    assert!(second.created);
    assert_eq!(second.replaced, Some(first.root));
    for (old, new) in first.revisions.iter().zip(&second.revisions) {
        assert_eq!((old.kind, old.id), (new.kind, new.id));
        assert_eq!(new.revision.0, 2);
    }
    // Instances still pin revision 1: the update preview moves them.
    assert_eq!(engine.project().library.components.len(), 2);
    // Version 2 again: nothing happens.
    let revision = engine.revision();
    let mut again = request(CHANGED, GeneratorScope::Component, 1_600_000);
    again.module.data = None;
    let third = generate(&mut engine, again)?;
    assert!(!third.created);
    assert_eq!(third.root, second.root);
    assert_eq!(engine.revision(), revision);
    // Version 1 again, embedded or rebuilt to other bytes: refused, never
    // a downgrade, and the document is unchanged.
    let before = engine.to_file();
    for module in [MODULE, REBUILT] {
        let mut older = request(module, GeneratorScope::Component, 1_600_000);
        if module == MODULE {
            older.module.data = None;
        }
        let refused = generate(&mut engine, older);
        assert!(
            matches!(&refused, Err(Error::Conflict(m))
                if m.starts_with("generated.older_version")
                    && m.contains("version 2")
                    && m.contains("Nothing was changed")),
            "{refused:?}"
        );
    }
    // Naming the part to replace does not get round it.
    let mut named = request(MODULE, GeneratorScope::Component, 1_600_000);
    named.module.data = None;
    named.replaces = Some(second.root);
    assert!(matches!(
        generate(&mut engine, named),
        Err(Error::Conflict(m)) if m.starts_with("generated.older_version")
    ));
    assert_eq!(engine.to_file(), before);
    assert_eq!(engine.revision(), revision);
    verify(&engine.project().library, &mut run)?;
    Ok(())
}

#[test]
fn the_same_version_built_to_other_bytes_changes_nothing() -> TestResult {
    let mut engine = engine();
    let first = generate(
        &mut engine,
        request(MODULE, GeneratorScope::Component, 1_600_000),
    )?;
    let before = engine.to_file();
    let rebuilt = generate(
        &mut engine,
        request(REBUILT, GeneratorScope::Component, 1_600_000),
    )?;
    assert!(!rebuilt.created);
    assert_eq!(rebuilt.root, first.root);
    assert_eq!(rebuilt.key, first.key);
    assert_eq!(
        engine.to_file(),
        before,
        "the rebuilt module is not embedded"
    );
    Ok(())
}

#[test]
fn other_parameters_scopes_or_generators_make_new_lineages() -> TestResult {
    let mut engine = engine();
    let first = generate(
        &mut engine,
        request(MODULE, GeneratorScope::Component, 1_600_000),
    )?;
    for (module, scope, length) in [
        (CHANGED, GeneratorScope::Component, 2_000_000),
        (CHANGED, GeneratorScope::Package, 1_600_000),
        (OTHER, GeneratorScope::Component, 1_600_000),
    ] {
        let made = generate(&mut engine, request(module, scope, length))?;
        assert!(made.created);
        assert_eq!(made.replaced, None);
        assert_ne!(made.root.id, first.root.id);
        assert_eq!(made.root.revision, Revision::FIRST);
    }
    Ok(())
}

/// Slice B1: ids come from generation keys. A newer module that inserts a
/// pad between pads 1 and 2 leaves both their ids (and every other local
/// id) as they were, the new pad takes a new id, and a track routed to pad
/// 2 stays attached through the update: nothing conflicts.
#[test]
fn regenerating_with_an_inserted_pad_keeps_every_other_id_and_the_tracks() -> TestResult {
    use crate::ids::ComponentRef;
    use crate::instances::NewInstance;
    use crate::layout::{LayoutCommand, RouteRequest};
    use crate::model::CopperLayer;
    use crate::model::PhysicalChoice;
    use crate::tests::support::{apply, connect, ep, net_of, sync_onto_board};
    use crate::{InstanceCommand, Outcome as Placed, ids::UnitId, updates::Change};
    let mut engine = engine();
    let first = generate(
        &mut engine,
        request(MODULE, GeneratorScope::Component, 1_600_000),
    )?;
    let [component, symbol, _, footprint, binding] = first.revisions[..] else {
        return Err("five revisions".into());
    };
    let place = |engine: &mut Engine, designator: &str, x: f64| -> TestResult<crate::ids::Id> {
        match apply(
            engine,
            Operation::Instance(InstanceCommand::Place(NewInstance {
                sheet: None,
                component: ComponentRef::new(component.id, component.revision),
                designator: Some(designator.into()),
                properties: vec![],
                representation: Some(symbol.id),
                unit: Some(UnitId(1)),
                position: Some(crate::tests::support::pt([x, 0.0])),
                physical: Some(PhysicalChoice {
                    binding: binding.id,
                    footprint: footprint.id,
                }),
            })),
        )? {
            Placed::InstancePlaced { instance, .. } => Ok(instance),
            other => Err(format!("unexpected outcome {other:?}").into()),
        }
    };
    let r1 = place(&mut engine, "R1", 0.0)?;
    let r2 = place(&mut engine, "R2", 20.0)?;
    connect(&mut engine, ep(r1, 2), ep(r2, 1))?;
    sync_onto_board(&mut engine)?;
    let net = net_of(&engine, ep(r1, 2))?;
    let at = |instance| {
        engine
            .project()
            .placements
            .iter()
            .find(|p| p.instance == instance)
            .map(|p| p.position)
            .ok_or("not placed")
    };
    let (a, b) = (at(r1)?, at(r2)?);
    apply(
        &mut engine,
        Operation::Layout(LayoutCommand::Route {
            request: RouteRequest {
                net,
                start: [a[0] + 0.45, a[1]],
                end: [b[0] - 0.45, b[1]],
                width: 0.25,
                layer: CopperLayer::Top,
            },
        }),
    )?;
    assert_eq!(engine.project().tracks.len(), 1);
    let pads = |engine: &Engine, revision: Revision| -> TestResult<Vec<(String, u32)>> {
        let footprint = engine
            .project()
            .library
            .footprints
            .iter()
            .find(|f| f.id == footprint.id && f.revision == revision)
            .ok_or("no footprint")?;
        Ok(footprint
            .definition
            .pads
            .iter()
            .map(|p| (p.number.clone(), p.id.0))
            .collect())
    };
    assert_eq!(
        pads(&engine, Revision::FIRST)?,
        [("1".to_string(), 1), ("2".to_string(), 2)]
    );
    let old_ids = engine.project().library.generated[0].from.ids.clone();
    // Version 2 inserts pad 3 between them.
    let second = generate(
        &mut engine,
        request(INSERTED, GeneratorScope::Component, 1_600_000),
    )?;
    assert_eq!(second.replaced, Some(first.root));
    let two = Revision::FIRST.next().ok_or("revision")?;
    assert_eq!(
        pads(&engine, two)?,
        [
            ("1".to_string(), 1),
            ("3".to_string(), 5),
            ("2".to_string(), 2)
        ],
        "pads 1 and 2 keep their ids; pad 3 takes the next one after the drawings"
    );
    let library = &engine.project().library;
    let record = library
        .generated
        .iter()
        .find(|g| g.root == second.root)
        .ok_or("no record")?;
    for (key, id) in &old_ids.footprint {
        assert_eq!(record.from.ids.footprint.get(key), Some(id), "{key}");
    }
    assert_eq!(record.from.ids.component, old_ids.component);
    assert_eq!(record.from.ids.symbol, old_ids.symbol);
    verify(library, &mut run)?;
    // The update moves both instances to revision 2; the track on pad 2
    // conflicts with nothing and stays.
    let change = Change::ComponentRevision {
        instances: vec![r1, r2],
        target: ComponentRef::new(second.root.id, second.root.revision),
        resolutions: Default::default(),
    };
    let plan = engine.preview_update(change.clone());
    assert!(plan.blockers.is_empty(), "{:?}", plan.blockers);
    assert!(
        plan.conflicting_tracks.is_empty(),
        "{:?}",
        plan.conflicting_tracks
    );
    engine.apply_update(engine.revision(), change, false)?;
    assert_eq!(engine.project().tracks.len(), 1);
    assert_eq!(net_of(&engine, ep(r1, 2))?, net);
    Ok(())
}

/// A given land's details (an arc outline, an offset slot, paste windows,
/// a mechanical pad and an unnumbered hole that land no terminal) expand to
/// the footprint's pads exactly; mask windows are refused.
#[test]
fn given_land_details_expand_exactly() -> TestResult {
    use crate::library::{HoleShape as CoreHole, PadKind as CoreKind, PasteRule, Segment};
    let mut g = generation_with(1_600_000, 0, Scope::Component, b"part", false);
    let first = g.footprint.pads.first().cloned().ok_or("a pad")?;
    g.footprint.pads[0] = Pad {
        shape: PadShape::Outline {
            start: [-400_000, -500_000],
            segments: vec![
                OutlineSegment {
                    to: [400_000, -500_000],
                    bulge_ppb: None,
                },
                OutlineSegment {
                    to: [-400_000, -500_000],
                    bulge_ppb: Some(1_000_000_000),
                },
            ],
        },
        hole: Some(Hole {
            shape: HoleShape::Slot {
                size: [600_000, 300_000],
                rot_mdeg: Some(45_000),
            },
            plated: true,
            offset: Some([50_000, -20_000]),
        }),
        paste: Opening::Windows(vec![Window::Circle {
            center: [0, 0],
            radius_nm: 100_000,
        }]),
        ..first.clone()
    };
    g.footprint.pads.push(Pad {
        key: "tab".into(),
        terminal: String::new(),
        number: Some("MP".into()),
        kind: Some(PadKind::Mechanical),
        ..first.clone()
    });
    g.footprint.pads.push(Pad {
        key: "hole".into(),
        terminal: String::new(),
        number: Some(String::new()),
        hole: Some(Hole {
            shape: HoleShape::Round {
                diameter_nm: 500_000,
            },
            plated: false,
            offset: None,
        }),
        ..first
    });
    let pads = super::expand::land_pattern(&g)?;
    let outline = &pads[0];
    let crate::library::PadShape::Custom { outline: shape } = &outline.shape else {
        return Err("not an outline".into());
    };
    assert!(matches!(shape.segments[1], Segment::Arc { bulge, .. } if (bulge - 1.0).abs() < 1e-12));
    let hole = outline.hole.ok_or("a hole")?;
    assert_eq!(hole.offset.to_mm_f64(), [0.05, -0.02]);
    assert!(
        matches!(hole.shape, CoreHole::Slot { angle_mdeg, .. } if angle_mdeg.millidegrees() == 45_000)
    );
    assert_eq!(outline.kind, CoreKind::PlatedThrough);
    assert!(matches!(&outline.paste, PasteRule::Windows(w) if w.len() == 1));
    assert_eq!(pads[2].kind, CoreKind::Mechanical);
    assert_eq!((pads[2].terminal, pads[2].number.as_str()), (None, "MP"));
    assert_eq!(pads[3].kind, CoreKind::NonPlatedHole);
    assert_eq!((pads[3].terminal, pads[3].number.as_str()), (None, ""));
    g.footprint.pads[0].mask = Opening::Windows(Vec::new());
    assert!(super::expand::land_pattern(&g).is_err());
    Ok(())
}

/// The edit that gives an instance of `component` the value `text` in the
/// first field the generated component selects (its part value).
fn value(
    engine: &Engine,
    component: crate::ids::ComponentRef,
    text: &str,
) -> TestResult<Vec<crate::fields::PropertyEdit>> {
    let field = engine
        .project()
        .library
        .component(component)
        .and_then(|c| c.definition.fields.first())
        .ok_or("the generated component selects no field")?
        .field;
    Ok(vec![crate::fields::PropertyEdit {
        field: field.id,
        input: crate::fields::PropertyInput::Text(text.into()),
    }])
}

/// A placed part with an exact manufacturer Part, made again by a newer
/// module: the regeneration republishes the Part for the new component
/// revision (its binding lineage's new revision, by lineage, never by
/// name), and the update that moves the instance takes it, so the
/// instance keeps no Part of the old revision behind.
#[test]
fn regenerating_carries_a_placed_parts_exact_part_to_the_new_revision() -> TestResult {
    use crate::ids::{BindingRef, ComponentRef, PartRef};
    use crate::instances::NewInstance;
    use crate::library::{Definition, DraftEdit, PartDefinition};
    use crate::model::PhysicalChoice;
    use crate::tests::support::{apply, edit, meta, new_draft};
    use crate::{InstanceCommand, Outcome as Placed, updates::Change};
    let mut engine = engine();
    let first = generate(
        &mut engine,
        request(MODULE, GeneratorScope::Component, 1_600_000),
    )?;
    let [component, symbol, _, footprint, binding] = first.revisions[..] else {
        return Err("five revisions".into());
    };
    let instance = match apply(
        &mut engine,
        Operation::Instance(InstanceCommand::Place(NewInstance {
            sheet: None,
            component: ComponentRef::new(component.id, component.revision),
            designator: Some("R1".into()),
            properties: vec![],
            representation: Some(symbol.id),
            unit: Some(crate::ids::UnitId(1)),
            position: Some(crate::tests::support::pt([0.0, 0.0])),
            physical: Some(PhysicalChoice {
                binding: binding.id,
                footprint: footprint.id,
            }),
        })),
    )? {
        Placed::InstancePlaced { instance, .. } => instance,
        other => return Err(format!("unexpected outcome {other:?}").into()),
    };
    let body = PartDefinition {
        next_local: 1,
        manufacturer: "ExampleCo".into(),
        mpn: "EX-STANDIN-1".into(),
        component: ComponentRef::new(component.id, component.revision),
        binding: BindingRef::new(binding.id, binding.revision),
        specs: Vec::new(),
        references: Vec::new(),
        offers: Vec::new(),
        provenance: Default::default(),
        top_mark: None,
    };
    let id = new_draft(&mut engine, crate::ids::DefinitionKind::Part)?;
    edit(
        &mut engine,
        vec![DraftEdit {
            id,
            meta: meta("EX-STANDIN-1"),
            definition: Definition::Part(Box::new(body)),
        }],
    )?;
    let part = match apply(
        &mut engine,
        Operation::Library(LibraryCommand::Publish {
            drafts: vec![id],
            close: true,
        }),
    )? {
        Placed::Published { revisions } => revisions.first().copied().ok_or("nothing published")?,
        other => return Err(format!("unexpected outcome {other:?}").into()),
    };
    apply(
        &mut engine,
        Operation::Instance(InstanceCommand::AssignPart {
            instances: vec![instance],
            part: Some(PartRef::new(part.id, part.revision)),
        }),
    )?;
    let properties = value(
        &engine,
        ComponentRef::new(component.id, component.revision),
        "EX",
    )?;
    // A value the Part does not rate: a finding the regeneration neither
    // causes nor resolves.
    crate::tests::support::set_properties(&mut engine, instance, properties)?;
    let second = generate(
        &mut engine,
        request(CHANGED, GeneratorScope::Component, 1_600_000),
    )?;
    assert_eq!(second.replaced, Some(first.root));
    let change = Change::ComponentRevision {
        instances: vec![instance],
        target: ComponentRef::new(second.root.id, second.root.revision),
        resolutions: Default::default(),
    };
    let plan = engine.preview_update(change.clone());
    assert!(plan.blockers.is_empty(), "{:?}", plan.blockers);
    engine.apply_update(engine.revision(), change, false)?;
    let project = engine.project();
    let placed = project.instance(instance).ok_or("no instance")?;
    let findings: Vec<String> = crate::instances::part_findings(&project.library, placed)
        .into_iter()
        .map(|f| f.code)
        .collect();
    assert_eq!(findings, ["part.spec_unrated"]);
    let pinned = placed.part.ok_or("the Part was dropped")?;
    assert_eq!(pinned.id, part.id);
    let republished = project.library.part(pinned).ok_or("no Part")?;
    assert_eq!(
        republished.definition.component,
        ComponentRef::new(second.root.id, second.root.revision)
    );
    let [.., new_binding] = second.revisions[..] else {
        return Err("five revisions".into());
    };
    assert_eq!(
        republished.definition.binding,
        BindingRef::new(new_binding.id, new_binding.revision)
    );
    Ok(())
}

/// A Part an earlier regeneration left behind (the instance already on the
/// new revision, its Part still on the old one): making the part again
/// with the same version changes nothing of the part but carries the Part,
/// and an update to the revision the instance is on takes it.
#[test]
fn regenerating_at_the_same_version_carries_a_part_left_on_an_older_revision() -> TestResult {
    use crate::ids::{BindingRef, ComponentRef, PartRef};
    use crate::instances::NewInstance;
    use crate::library::{Definition, DraftEdit, PartDefinition};
    use crate::model::PhysicalChoice;
    use crate::tests::support::{apply, edit, meta, new_draft};
    use crate::updates::{Change, PartAction, PartResolution, Resolutions};
    use crate::{InstanceCommand, Outcome as Placed};
    let mut engine = engine();
    let first = generate(
        &mut engine,
        request(MODULE, GeneratorScope::Component, 1_600_000),
    )?;
    let [component, symbol, _, footprint, binding] = first.revisions[..] else {
        return Err("five revisions".into());
    };
    let instance = match apply(
        &mut engine,
        Operation::Instance(InstanceCommand::Place(NewInstance {
            sheet: None,
            component: ComponentRef::new(component.id, component.revision),
            designator: Some("R1".into()),
            properties: vec![],
            representation: Some(symbol.id),
            unit: Some(crate::ids::UnitId(1)),
            position: Some(crate::tests::support::pt([0.0, 0.0])),
            physical: Some(PhysicalChoice {
                binding: binding.id,
                footprint: footprint.id,
            }),
        })),
    )? {
        Placed::InstancePlaced { instance, .. } => instance,
        other => return Err(format!("unexpected outcome {other:?}").into()),
    };
    // Version 2 is made before the Part exists, so nothing carries it.
    let second = generate(
        &mut engine,
        request(CHANGED, GeneratorScope::Component, 1_600_000),
    )?;
    let body = PartDefinition {
        next_local: 1,
        manufacturer: "ExampleCo".into(),
        mpn: "EX-STANDIN-2".into(),
        component: ComponentRef::new(component.id, component.revision),
        binding: BindingRef::new(binding.id, binding.revision),
        specs: Vec::new(),
        references: Vec::new(),
        offers: Vec::new(),
        provenance: Default::default(),
        top_mark: None,
    };
    let id = new_draft(&mut engine, crate::ids::DefinitionKind::Part)?;
    edit(
        &mut engine,
        vec![DraftEdit {
            id,
            meta: meta("EX-STANDIN-2"),
            definition: Definition::Part(Box::new(body)),
        }],
    )?;
    let part = match apply(
        &mut engine,
        Operation::Library(LibraryCommand::Publish {
            drafts: vec![id],
            close: true,
        }),
    )? {
        Placed::Published { revisions } => revisions.first().copied().ok_or("nothing published")?,
        other => return Err(format!("unexpected outcome {other:?}").into()),
    };
    let part = PartRef::new(part.id, part.revision);
    apply(
        &mut engine,
        Operation::Instance(InstanceCommand::AssignPart {
            instances: vec![instance],
            part: Some(part),
        }),
    )?;
    let properties = value(
        &engine,
        ComponentRef::new(component.id, component.revision),
        "EX",
    )?;
    // A value the Part does not rate: a finding the regeneration neither
    // causes nor resolves.
    crate::tests::support::set_properties(&mut engine, instance, properties)?;
    let target = ComponentRef::new(second.root.id, second.root.revision);
    // The instance moves to revision 2 keeping its Part: left behind.
    engine.apply_update(
        engine.revision(),
        Change::ComponentRevision {
            instances: vec![instance],
            target,
            resolutions: Resolutions {
                parts: vec![PartResolution {
                    instance,
                    action: PartAction::Keep,
                }],
                ..Default::default()
            },
        },
        false,
    )?;
    let stale = |engine: &Engine| -> TestResult<Vec<String>> {
        let placed = engine.project().instance(instance).ok_or("no instance")?;
        Ok(
            crate::instances::part_findings(&engine.project().library, placed)
                .into_iter()
                .map(|f| f.code)
                .collect(),
        )
    };
    assert_eq!(
        stale(&engine)?,
        [
            "part.stale_component",
            "part.binding_mismatch",
            "part.spec_unrated"
        ]
    );
    // The same version again: the part stands, the Part is carried.
    let mut again = request(CHANGED, GeneratorScope::Component, 1_600_000);
    again.module.data = None;
    let third = generate(&mut engine, again)?;
    assert!(!third.created);
    assert_eq!(third.root, second.root);
    let carried = engine
        .project()
        .library
        .latest_parts()
        .into_iter()
        .find(|p| p.id == part.id)
        .ok_or("no Part")?;
    assert_eq!(carried.definition.component, target);
    // The update to the revision the instance is on takes it.
    let change = Change::ComponentRevision {
        instances: vec![instance],
        target,
        resolutions: Default::default(),
    };
    let plan = engine.preview_update(change.clone());
    assert!(plan.blockers.is_empty(), "{:?}", plan.blockers);
    engine.apply_update(engine.revision(), change, false)?;
    assert_eq!(stale(&engine)?, ["part.spec_unrated"]);
    Ok(())
}

/// A generated terminal's further functions expand after its default, the
/// default being the design default; a fixed terminal keeps one function.
#[test]
fn a_terminal_with_alternates_expands_to_selectable_functions() -> TestResult {
    use crate::functions::Direction as Core;
    use crate::library::TerminalClass;
    let mut engine = engine();
    generate(
        &mut engine,
        request(PIN_TABLE, GeneratorScope::Component, 1_600_000),
    )?;
    let library = &engine.project().library;
    let component = library.components.first().ok_or("no component")?;
    let terminals = &component.definition.terminals;
    let functions = |n: usize| match terminals.get(n).map(|t| &t.class) {
        Some(TerminalClass::Electrical(f)) => Some(f),
        _ => None,
    };
    assert_eq!(terminals.first().map(|t| t.name.as_str()), Some("IO0"));
    let io = functions(0).ok_or("IO0 has no functions")?;
    let directions: Vec<Core> = io.functions.iter().map(|f| f.direction).collect();
    assert_eq!(directions, [Core::Input, Core::Output]);
    assert_eq!(io.design_default, io.functions.first().map(|f| f.id));
    let gnd = functions(1).ok_or("GND has no functions")?;
    assert_eq!(gnd.functions.len(), 1);
    assert_eq!(
        gnd.functions.first().map(|f| f.direction),
        Some(Core::PowerInput)
    );
    assert_eq!(gnd.design_default, None);
    // The record checks on load like any generated part.
    verify(&engine.to_file().project.library, &mut run)?;
    Ok(())
}

/// E2E run 13: a generated IC had no MPN in the BOM. Its orderable part
/// numbers are published as the component's manufacturer Parts in the
/// same step, once each: the same request again adds none.
#[test]
fn orderable_parts_are_published_with_the_part() -> TestResult {
    let mut engine = engine();
    let report = generate(
        &mut engine,
        request(ORDERABLE, GeneratorScope::Component, 1_600_000),
    )?;
    let parts = |engine: &Engine| -> Vec<(String, String, crate::ids::ComponentRef)> {
        engine
            .project()
            .library
            .latest_parts()
            .into_iter()
            .map(|p| {
                (
                    p.definition.manufacturer.clone(),
                    p.definition.mpn.clone(),
                    p.definition.component,
                )
            })
            .collect()
    };
    let root = crate::ids::ComponentRef::new(report.root.id, report.root.revision);
    assert_eq!(
        parts(&engine),
        [
            ("Stand-in Parts".to_owned(), "SI-1000".to_owned(), root),
            ("Stand-in Parts".to_owned(), "SI-1000-T".to_owned(), root),
        ]
    );
    assert!(engine.drafts().items.is_empty());
    // One undo step takes the part and its Parts away.
    engine.undo()?;
    assert!(parts(&engine).is_empty());
    assert!(engine.project().library.components.is_empty());
    engine.redo()?;
    let again = generate(
        &mut engine,
        request(ORDERABLE, GeneratorScope::Component, 1_600_000),
    )?;
    assert!(!again.created);
    assert_eq!(parts(&engine).len(), 2);
    // A package has no orderable parts.
    let mut package = self::engine();
    generate(
        &mut package,
        request(ORDERABLE, GeneratorScope::Package, 1_600_000),
    )?;
    assert!(parts(&package).is_empty());
    Ok(())
}

/// The pads of [`a_turned_pad_turns_its_drill_slot_and_paste_windows`]:
/// a drilled 2 mm circle turned 90° at the origin with a paste window, a
/// slotted one at (3, 0) mm and a drilled 2 × 1 mm rect turned 270° at
/// (0, 3) mm, each drill (+0.2, 0) mm off centre in its pad frame.
fn turned_drilled_pads() -> TestResult<Vec<crate::library::Pad>> {
    let mut g = generation_with(1_600_000, 0, Scope::Component, b"part", false);
    let first = g.footprint.pads.first().cloned().ok_or("a pad")?;
    let drilled = |shape: PadShape, rot_mdeg: i64, hole: HoleShape| Pad {
        shape,
        at: [0, 0],
        rot_mdeg,
        hole: Some(Hole {
            shape: hole,
            plated: true,
            offset: Some([200_000, 0]),
        }),
        ..first.clone()
    };
    let round = HoleShape::Round {
        diameter_nm: 800_000,
    };
    let circle = PadShape::Circle {
        diameter_nm: 2_000_000,
    };
    g.footprint.pads[0] = Pad {
        paste: Opening::Windows(vec![Window::Rect {
            min: [100_000, -100_000],
            max: [500_000, 100_000],
            radius_nm: 0,
        }]),
        ..drilled(circle.clone(), 90_000, round.clone())
    };
    g.footprint.pads[1] = Pad {
        key: "2".into(),
        terminal: "2".into(),
        at: [3_000_000, 0],
        ..drilled(
            circle,
            90_000,
            HoleShape::Slot {
                size: [600_000, 300_000],
                rot_mdeg: None,
            },
        )
    };
    g.footprint.pads.push(Pad {
        key: "3".into(),
        terminal: "1".into(),
        at: [0, 3_000_000],
        ..drilled(
            PadShape::Rect {
                size: [2_000_000, 1_000_000],
            },
            270_000,
            round,
        )
    });
    Ok(super::expand::land_pattern(&g)?)
}

/// A pad turned by more than its outline's symmetry keeps its turn for
/// everything inside it: a 2 mm circular land turned 90° counter-clockwise
/// with its drill (+0.2, 0) mm off centre in the pad frame drills at
/// (0, +0.2) mm from the land's centre, its x-long slot runs along y and
/// its paste window at x 0.1…0.5 mm, y ±0.1 mm sits at x ±0.1 mm,
/// y 0.1…0.5 mm. A rect turned 270° (its outline folds to 90°) moves the
/// same drill to (0, −0.2) mm.
#[test]
fn a_turned_pad_turns_its_drill_slot_and_paste_windows() -> TestResult {
    use crate::geometry::Bounds;
    let pads = turned_drilled_pads()?;
    let close = |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9;
    let drill = |pad: &crate::library::Pad| -> TestResult<[f64; 2]> {
        Ok(pad.to_footprint(pad.hole.ok_or("a hole")?.offset()))
    };
    assert!(
        close(drill(&pads[0])?, [0.0, 0.2]),
        "{:?}",
        drill(&pads[0])?
    );
    let crate::library::PasteRule::Windows(windows) = &pads[0].paste else {
        return Err("paste windows".into());
    };
    let window = Bounds::of(
        windows[0]
            .shape
            .to_mm()
            .transformed(&pads[0].frame())
            .flatten(0.001),
    )
    .ok_or("a window")?;
    assert!(close(window.min, [-0.1, 0.1]), "{window:?}");
    assert!(close(window.max, [0.1, 0.5]), "{window:?}");
    assert!(
        close(drill(&pads[1])?, [3.0, 0.2]),
        "{:?}",
        drill(&pads[1])?
    );
    let Some(crate::library::Hole {
        shape: crate::library::HoleShape::Slot { angle_mdeg, .. },
        ..
    }) = pads[1].hole
    else {
        return Err("a slot".into());
    };
    let along = pads[1].frame().apply_angle(angle_mdeg.to_degrees_f64());
    assert!((along.rem_euclid(180.0) - 90.0).abs() < 1e-9, "{along}");
    assert!(
        close(drill(&pads[2])?, [0.0, 2.8]),
        "{:?}",
        drill(&pads[2])?
    );
    Ok(())
}

/// The turned pads on the board's bottom side: the demo's R1 footprint
/// takes the drilled circle and the slotted one (keeping its pad ids and
/// terminals), placed at (10, 20) mm turned 90° on the bottom. The bottom
/// flip negates y, then the 90° turn maps (x, y) to (−y, x): the drill at
/// (0, 0.2) mm in the footprint lands at (10.2, 20); the paste window
/// (x ±0.1, y 0.1…0.5) at x 10.1…10.5, y 19.9…20.1; the slot centre at
/// (3, 0.2) at (10.2, 23), its length along x.
#[test]
fn a_turned_pad_placed_on_the_bottom_drills_and_pastes_where_the_flip_puts_them() -> TestResult {
    use crate::geometry::{Bounds, Shape};
    use crate::model::Layer;
    let pads = turned_drilled_pads()?;
    let engine = Engine::demo()?;
    let r1 = crate::tests::support::by_designator(&engine, "R1")?;
    let mut project = engine.project().clone();
    let footprint = &mut project.library.footprints[0].definition;
    footprint.datum.offset = crate::units::PointNm::ORIGIN;
    for (stored, turned) in footprint.pads.iter_mut().zip(&pads) {
        *stored = crate::library::Pad {
            id: stored.id,
            terminal: stored.terminal,
            number: stored.number.clone(),
            ..turned.clone()
        };
    }
    let placement = project
        .placements
        .iter_mut()
        .find(|p| p.instance == r1)
        .ok_or("R1 placement")?;
    placement.position = [10.0, 20.0];
    placement.rotation_deg = 90.0;
    placement.layer = Layer::Bottom;
    let placed: Vec<_> = crate::layout::placed_pads(&project)
        .into_iter()
        .filter(|p| p.instance == r1)
        .collect();
    let [drilled, slotted] = &placed[..] else {
        return Err("two placed pads".into());
    };
    let close = |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9;
    let Shape::Circle { center, .. } = drilled.holes()[0].shape else {
        return Err("a round drill".into());
    };
    assert!(close(center, [10.2, 20.0]), "{center:?}");
    let crate::library::PasteRule::Windows(windows) = &drilled.pad.paste else {
        return Err("paste windows".into());
    };
    let window = Bounds::of(
        windows[0]
            .shape
            .to_mm()
            .transformed(&drilled.pad_transform())
            .flatten(0.001),
    )
    .ok_or("a window")?;
    assert!(close(window.min, [10.1, 19.9]), "{window:?}");
    assert!(close(window.max, [10.5, 20.1]), "{window:?}");
    let slot = Bounds::of(slotted.holes()[0].shape.flatten(0.0001)).ok_or("a slot")?;
    let middle = [
        (slot.min[0] + slot.max[0]) / 2.0,
        (slot.min[1] + slot.max[1]) / 2.0,
    ];
    assert!(close(middle, [10.2, 23.0]), "{slot:?}");
    assert!(slot.size()[0] > slot.size()[1], "{slot:?}");
    Ok(())
}

/// An open-drain terminal expands to an output whose drive is an open
/// drain, so ERC lets it share a net with a pull-up or another flag.
#[test]
fn an_open_drain_terminal_keeps_its_drive() -> TestResult {
    use crate::library::TerminalClass;
    let mut engine = engine();
    generate(
        &mut engine,
        request(OPEN_DRAIN, GeneratorScope::Component, 1_600_000),
    )?;
    let library = &engine.project().library;
    let component = library.components.first().ok_or("no component")?;
    let Some(TerminalClass::Electrical(f)) =
        component.definition.terminals.first().map(|t| &t.class)
    else {
        return Err("PG has no functions".into());
    };
    let first = f.functions.first().ok_or("no function")?;
    assert_eq!(first.direction, crate::functions::Direction::Output);
    assert_eq!(first.drive, crate::functions::Drive::OpenDrain);
    Ok(())
}
