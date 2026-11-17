//! Project-only 3D recovery for the shared desktop/CLI importer. Vendor STEP
//! comes first; uniquely matching shipped land patterns are the second layer.
//! No guessed geometry or electrical mapping is written into the imported board.
#[cfg(test)]
mod pose_tests;
mod presets;
use sparksmith_core::library::bundle::{AssetBytes, PackedAsset, asset_hash};
use sparksmith_core::library::model3d::*;
use sparksmith_core::library::{DraftEdit, ModelPlacement, PackageModel, Pad};
use sparksmith_core::units::Nm;
use sparksmith_core::{Engine, LibraryCommand, ModelRef, Operation, Outcome, Revision};
use sparksmith_io::project_models::{ProjectModels, library_reference};
use sparksmith_kicad::models::{Choice, Provider, Reference, Selection};
use sparksmith_kicad::report::Case;
use sparksmith_solidcraft::models::{inspect, step_assembly};
use std::collections::BTreeMap;
use std::path::Path;

pub(crate) struct Models {
    deadline: std::time::Instant,
    files: ProjectModels,
    presets: presets::Presets,
    read: BTreeMap<String, Option<(AssetBytes, String)>>,
    models: BTreeMap<sparksmith_core::ids::ContentHash, ModelRef>,
    vendor_choices: BTreeMap<sparksmith_core::ids::ContentHash, Choice>,
}
impl Models {
    pub(crate) fn new(root: &Path) -> Result<Self, String> {
        Ok(Self {
            deadline: std::time::Instant::now() + std::time::Duration::from_secs(120),
            files: ProjectModels::new(root).map_err(|e| e.to_string())?,
            presets: presets::Presets::default(),
            read: BTreeMap::new(),
            models: BTreeMap::new(),
            vendor_choices: BTreeMap::new(),
        })
    }
    fn check_time(&self) -> Result<(), String> {
        check_time(self.deadline)
    }
    fn source(&mut self, path: &str) -> Result<Option<(AssetBytes, String)>, String> {
        if let Some(value) = self.read.get(path) {
            return Ok(value.clone());
        }
        self.check_time()?;
        let value = self
            .files
            .resolve(path)
            .map_err(|e| e.to_string())?
            .map(|r| (AssetBytes::new(r.bytes), r.source));
        self.check_time()?;
        self.read.insert(path.into(), value.clone());
        Ok(value)
    }
    fn vendor(
        &mut self,
        engine: &mut Engine,
        refs: &[&Reference],
    ) -> Result<Option<Choice>, String> {
        if refs.is_empty() || refs.iter().any(|r| library_reference(&r.path)) {
            return Ok(None);
        }
        let mut files = Vec::new();
        for reference in refs {
            let Some(source) = self.source(&reference.path)? else {
                return Ok(None);
            };
            files.push(source);
        }
        // A single uniformly scaled model keeps its original bytes and its
        // placement outside the source model. Other assemblies are flattened by
        // Solidcraft's public STEP writer, retaining each source's provenance.
        let single = refs.len() == 1 && refs[0].scale.iter().all(|v| *v == refs[0].scale[0]);
        let (bytes, frame, placement) = if single {
            (
                files[0].0.clone(),
                SourceFrame {
                    unit_mm: refs[0].scale[0],
                    ..SourceFrame::default()
                },
                ModelPlacement::from_mm(
                    refs[0].offset_nm.map(Nm::to_mm_f64),
                    refs[0].rotation_deg.map(|v| -v),
                )
                .map_err(|e| e.to_string())?,
            )
        } else {
            let instances: Vec<_> = refs
                .iter()
                .zip(&files)
                .map(|(r, (bytes, _))| step_assembly::Instance {
                    bytes: bytes.as_slice(),
                    scale: r.scale,
                    rotation_deg: r.rotation_deg.map(|v| -v),
                    offset_mm: r.offset_nm.map(Nm::to_mm_f64),
                })
                .collect();
            (
                AssetBytes::new(
                    step_assembly::compose_until(&instances, Some(self.deadline))
                        .map_err(|e| e.to_string())?,
                ),
                SourceFrame::default(),
                ModelPlacement::default(),
            )
        };
        let source = files
            .iter()
            .map(|(b, path)| format!("{path} (sha256 {})", b.hash()))
            .collect::<Vec<_>>()
            .join("; ");
        let provenance = format!(
            "Project-only vendor STEP: {source}. Never publish to a shared library. {}",
            if single {
                "Original STEP geometry and colours; source scale and placement retained."
            } else {
                "Composed by Solidcraft from the recorded source transforms."
            }
        );
        let key =
            asset_hash(&serde_json::to_vec(&(bytes.hash(), frame)).map_err(|e| e.to_string())?);
        if let Some(cached) = self.vendor_choices.get(&key) {
            let mut choice = cached.clone();
            choice.placement = placement;
            choice.provenance = provenance;
            return Ok(Some(choice));
        }
        self.check_time()?;
        let seen = inspect::inspect_in(bytes.as_slice(), ModelFormat::Step, &frame)
            .map_err(|e| e.to_string())?;
        self.check_time()?;
        if !seen.warnings.is_empty() {
            return Err(format!(
                "STEP reader reported incomplete geometry: {}",
                seen.warnings.join("; ")
            ));
        }
        let mut definition = ModelDefinition {
            asset: AssetRef {
                hash: bytes.hash(),
                bytes: bytes.len() as u64,
                media_type: ModelFormat::Step.media_type().into(),
                file_name: if single {
                    Path::new(&files[0].1)
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("model.step")
                        .into()
                } else {
                    "composed-project-model.step".into()
                },
            },
            format: ModelFormat::Step,
            frame,
            bounds_nm: bounds_nm(seen.bounds_mm).map_err(|e| e.to_string())?,
            provenance: ModelProvenance {
                redistribution: Redistribution::Forbidden,
                record: sparksmith_core::library::Provenance {
                    basis: sparksmith_core::library::Basis::Imported,
                    licence: Some(
                        "Project-only; vendor redistribution permission not granted".into(),
                    ),
                    ..Default::default()
                },
                ..Default::default()
            },
            ..ModelDefinition::default()
        };
        for body in seen.bodies {
            let material = if let Some(colour) = body.colour {
                let index = definition
                    .custom_materials
                    .iter()
                    .position(|m| m.base_colour == colour)
                    .unwrap_or(definition.custom_materials.len());
                if index == definition.custom_materials.len() {
                    if index >= MAX_CUSTOM_MATERIALS {
                        return Err("STEP model exceeds 64 distinct colours".into());
                    }
                    definition.custom_materials.push(CustomMaterial {
                        name: format!("STEP colour {index}"),
                        base_colour: colour,
                        metallic: 0.0,
                        roughness: 0.5,
                        clearcoat: None,
                        transmission: None,
                        ior: None,
                        emissive: None,
                        textures_ignored: false,
                    });
                }
                ModelMaterial::Custom(index as u16)
            } else {
                ModelMaterial::Builtin(sparksmith_core::materials::MaterialId::EpoxyMould)
            };
            definition.bodies.push(ModelBody {
                key: body.key,
                role: BodyRole::Body,
                material,
            });
        }
        // Unit scale belongs to the revision's source frame, so its key includes
        // that frame; equal source bytes at different scales are distinct models.
        let envelope_nm = envelope(seen.bounds_mm)?;
        let reference = if let Some(reference) = self.models.get(&key) {
            *reference
        } else {
            let reference = publish_model(
                engine,
                "Imported project STEP",
                definition,
                bytes,
                &provenance,
            )?;
            self.models.insert(key, reference);
            reference
        };
        let choice = Choice {
            model: PackageModel::Library(LibraryModel {
                model: reference,
                lead_map: Vec::new(),
            }),
            placement,
            envelope_nm,
            provenance,
        };
        self.vendor_choices.insert(key, choice.clone());
        Ok(Some(choice))
    }
}
impl Provider for Models {
    fn prepare(
        &mut self,
        engine: &mut Engine,
        pads: &[Pad],
        prefix: &str,
        models: &[Reference],
    ) -> Result<Selection, String> {
        self.check_time()?;
        let refs: Vec<_> = models.iter().filter(|r| !r.hidden).collect();
        let library = refs.iter().any(|r| library_reference(&r.path));
        let reason = match self.vendor(engine, &refs) {
            Ok(Some(choice)) => {
                self.check_time()?;
                return Ok(Selection {
                    reason: choice.provenance.clone(),
                    choice: Some(choice),
                    case: Case::VendorModel,
                });
            }
            Ok(None) => {
                if library {
                    "KiCad library reference is never resolved".into()
                } else {
                    "No project-local STEP resolved".into()
                }
            }
            Err(reason) => reason,
        };
        self.check_time()?;
        match self
            .presets
            .match_model(engine, pads, prefix, &mut self.models, self.deadline)
        {
            Ok(Some(choice)) => Ok(Selection {
                reason: format!("{reason}; {}", choice.provenance),
                choice: Some(choice),
                case: if library {
                    Case::LibraryModelGenerated
                } else {
                    Case::GeneratedModel
                },
            }),
            Ok(None) => Ok(Selection {
                choice: None,
                case: Case::NoModel,
                reason: format!(
                    "{reason}; no unique shipped preset matches every pad within 50 µm"
                ),
            }),
            Err(error) => Err(format!("{reason}; preset model unavailable: {error}")),
        }
    }
}

fn check_time(deadline: std::time::Instant) -> Result<(), String> {
    if std::time::Instant::now() >= deadline {
        Err("3D model import reached its 120 second cooperative time limit".into())
    } else {
        Ok(())
    }
}

pub(super) fn envelope([lo, hi]: [[f64; 3]; 2]) -> Result<[Nm; 3], String> {
    triple_nm(
        [
            2.0 * lo[0].abs().max(hi[0].abs()),
            2.0 * lo[1].abs().max(hi[1].abs()),
            hi[2].max(hi[2] - lo[2]),
        ]
        .map(|v| v.max(0.001)),
    )
    .map_err(|e| e.to_string())
}
pub(super) fn publish_model(
    engine: &mut Engine,
    name: &str,
    definition: ModelDefinition,
    bytes: AssetBytes,
    provenance: &str,
) -> Result<ModelRef, String> {
    let mut staged = engine.fork();
    let candidate = &mut staged;
    let assets = vec![PackedAsset {
        hash: bytes.hash(),
        media_type: definition.format.media_type().into(),
        data: bytes,
    }];
    let outcome = candidate
        .apply(
            candidate.revision(),
            Operation::Library(LibraryCommand::ImportModel(Box::new(ImportModel {
                name: name.into(),
                assets,
                definition: Box::new(definition),
                assign: None,
            }))),
        )
        .map_err(|e| e.to_string())?;
    let Outcome::DraftCreated { id } = outcome else {
        return Err("Model import produced no draft".into());
    };
    let draft = candidate
        .document()
        .drafts
        .get(id)
        .ok_or("Model draft is missing")?;
    let mut meta = draft.meta.clone();
    meta.provenance = provenance.into();
    let definition = draft.definition.clone();
    candidate
        .apply(
            candidate.revision(),
            Operation::Library(LibraryCommand::Edit {
                edits: vec![DraftEdit {
                    id,
                    meta,
                    definition,
                }],
            }),
        )
        .map_err(|e| e.to_string())?;
    candidate
        .apply(
            candidate.revision(),
            Operation::Library(LibraryCommand::Publish {
                drafts: vec![id],
                close: true,
            }),
        )
        .map_err(|e| e.to_string())?;
    *engine = staged;
    Ok(ModelRef::new(id, Revision::FIRST))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sparksmith_solidcraft::native::{BoxBody, Expr, Look, Native};
    type Fallible = Result<(), Box<dyn std::error::Error>>;
    fn fixture() -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        let mut native = Native::new("synthetic vendor part")?;
        native.solid_box(BoxBody {
            name: "body",
            corner: [0, 0, 0].map(Expr::from),
            size: [1, 2, 3].map(Expr::from),
        })?;
        native.look(
            &["body"],
            &Look {
                name: "original red",
                srgb8: [200, 50, 20],
                metallic: 0.0,
                roughness: 0.5,
                ior: 1.5,
            },
        )?;
        Ok(native.step_bytes(&["body".into()], "synthetic vendor part")?)
    }
    fn reference(path: &str) -> Reference {
        Reference {
            path: path.into(),
            offset_nm: [Nm::ZERO; 3],
            rotation_deg: [0.0; 3],
            scale: [1.0; 3],
            hidden: false,
        }
    }
    #[test]
    fn expired_import_refuses_without_publishing_any_model() -> Fallible {
        let dir = tempfile::tempdir()?;
        std::fs::write(dir.path().join("own.step"), fixture()?)?;
        let mut models = Models::new(dir.path())?;
        models.deadline = std::time::Instant::now();
        let mut engine = Engine::demo()?;
        let before = engine.revision();
        let error = models
            .prepare(&mut engine, &[], "U", &[reference("own.step")])
            .err()
            .ok_or("expired import succeeded")?;
        assert!(error.contains("time limit"), "{error}");
        assert_eq!(engine.revision(), before);
        assert!(engine.project().library.models.is_empty());
        Ok(())
    }

    #[test]
    fn vendor_models_share_assets_keep_colours_transforms_and_refuse_sharing() -> Fallible {
        let dir = tempfile::tempdir()?;
        std::fs::write(dir.path().join("own.step"), fixture()?)?;
        let mut models = Models::new(dir.path())?;
        let mut engine = Engine::demo()?;
        let mut a = reference("${KIPRJMOD}/own.step");
        a.offset_nm = triple_nm([10.0, 20.0, 30.0])?;
        a.rotation_deg = [0.0, 0.0, 90.0];
        let first = models.vendor(&mut engine, &[&a])?.ok_or("vendor missing")?;
        a.path = "own.step".into();
        a.offset_nm = triple_nm([20.0, 30.0, 40.0])?;
        let second = models.vendor(&mut engine, &[&a])?.ok_or("vendor missing")?;
        assert_eq!(first.model, second.model);
        assert_ne!(first.placement, second.placement);
        let point = first.placement.transform().apply([1.0, 2.0, 3.0]);
        assert_eq!(point, [12.0, 19.0, 33.0]);
        assert_eq!(engine.project().library.models.len(), 1);
        let model = &engine.project().library.models[0].definition;
        assert_eq!(model.provenance.redistribution, Redistribution::Forbidden);
        let PackageModel::Library(used) = &first.model else {
            return Err("Expected library model".into());
        };
        assert!(
            sparksmith_core::library::import::project_bundle(
                &engine.project().library,
                used.model.erased(),
                "forbidden"
            )
            .is_err()
        );
        assert_eq!(model.custom_materials.len(), 1);
        assert!(first.provenance.contains("sha256"));
        assert!(first.provenance.contains("own.step"));
        let path = dir.path().join("offline.sparksmith");
        sparksmith_io::write_project(&path, &engine.to_file())?;
        std::fs::remove_file(dir.path().join("own.step"))?;
        let reopened = sparksmith_io::read_project(&path)?;
        assert_eq!(reopened.project.library.models[0].definition, *model);
        Ok(())
    }
    #[test]
    fn a_kicad_library_reference_is_not_resolved_even_with_a_same_name_local_file() -> Fallible {
        let dir = tempfile::tempdir()?;
        std::fs::write(dir.path().join("own.step"), fixture()?)?;
        let mut models = Models::new(dir.path())?;
        let mut engine = Engine::demo()?;
        assert!(
            models
                .vendor(&mut engine, &[&reference("${KICAD9_3DMODEL_DIR}/own.step")])?
                .is_none()
        );
        assert!(engine.project().library.models.is_empty());
        Ok(())
    }
    #[test]
    fn report_counts_instances_of_complete_assemblies_and_names_missing_models() -> Fallible {
        let context = sparksmith_core::library::standards::Context::fixture()?;
        let _context = context.enter();
        let dir = tempfile::tempdir()?;
        std::fs::write(dir.path().join("own.step"), fixture()?)?;
        let mut board = String::from(
            r#"(kicad_pcb (version 20240108) (generator "original assembly report fixture")
            (general (thickness 1.6)) (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
            (gr_rect (start 0 0) (end 40 30) (stroke (width 0.1) (type default)) (layer "Edge.Cuts"))"#,
        );
        for (name, x) in [("U1", 10), ("U2", 20)] {
            board.push_str(&format!(
                r#"(footprint "Original:Assembly" (layer "F.Cu") (at {x} 10)
                (property "Reference" "{name}")
                (model "own.step" (offset (xyz 0 0 0)))
                (model "own.step" (offset (xyz 5 0 0))))"#
            ));
        }
        board.push_str(
            r#"(footprint "Original:Missing" (layer "F.Cu") (at 30 10)
            (property "Reference" "U3") (model "missing.step")))"#,
        );
        let path = dir.path().join("assembly.kicad_pcb");
        std::fs::write(&path, board)?;
        let imported = crate::kicad_import::import(&path, None)?;
        assert_eq!(
            imported.report.count(Case::VendorModel),
            2,
            "two footprint instances, not four references or one unique model"
        );
        assert_eq!(imported.report.count(Case::NoModel), 1);
        assert_eq!(imported.engine.project().library.models.len(), 1);
        assert_eq!(
            imported.engine.project().library.models[0]
                .definition
                .bodies
                .len(),
            2
        );
        Ok(())
    }

    #[test]
    fn host_import_shares_one_vendor_model_across_top_and_bottom_instances() -> Fallible {
        let context = sparksmith_core::library::standards::Context::fixture()?;
        let _context = context.enter();
        let dir = tempfile::tempdir()?;
        std::fs::write(dir.path().join("own.step"), fixture()?)?;
        let mut text = String::from(
            r#"(kicad_pcb (version 20240108) (generator "original synthetic fixture") (general (thickness 1.6)) (layers (0 "F.Cu" signal) (31 "B.Cu" signal)) (gr_rect (start 0 0) (end 40 30) (stroke (width 0.1) (type default)) (layer "Edge.Cuts"))"#,
        );
        for (reference, side, x) in [("U1", "F", 10), ("U2", "B", 20)] {
            text.push_str(&format!(r#"(footprint "Original:Box" (layer "{side}.Cu") (at {x} 10) (property "Reference" "{reference}") (pad "1" smd rect (at 0 0) (size 1 1) (layers "{side}.Cu" "{side}.Mask" "{side}.Paste")) (model "${{KIPRJMOD}}/own.step" (offset (xyz 0 0 0)) (scale (xyz 1 1 1)) (rotate (xyz 0 0 0))))"#));
        }
        text.push(')');
        let path = dir.path().join("original.kicad_pcb");
        std::fs::write(&path, text)?;
        let imported = crate::kicad_import::import(&path, None)?;
        assert_eq!(imported.report.count(Case::VendorModel), 2);
        assert_eq!(imported.report.count(Case::NoModel), 0);
        assert_eq!(imported.engine.project().library.models.len(), 1);
        let model = sparksmith_solidcraft::project::build_flat(
            imported.engine.project(),
            &sparksmith_solidcraft::models::imported::SourceCache::default(),
            None,
            sparksmith_solidcraft::project::Flat::Meshed,
        )?;
        assert_eq!(model.part_designs(), 1);
        let parts: Vec<_> = model
            .geometry
            .iter()
            .filter(|g| matches!(g.kind, sparksmith_solidcraft::material::BodyKind::ModelBody))
            .collect();
        assert_eq!(parts.len(), 2);
        assert!(std::sync::Arc::ptr_eq(&parts[0].mesh, &parts[1].mesh));
        let z: Vec<_> = parts
            .iter()
            .map(|g| {
                let mesh = g.world_mesh();
                mesh.positions
                    .iter()
                    .map(|v| v.z)
                    .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), z| {
                        (lo.min(z), hi.max(z))
                    })
            })
            .collect();
        assert!(
            z.iter().any(|(lo, hi)| *lo >= 0.0 && *hi > 3.0),
            "top model: {z:?}"
        );
        assert!(
            z.iter().any(|(lo, hi)| *lo < -2.9 && *hi <= 0.001),
            "bottom model: {z:?}"
        );
        Ok(())
    }
}
