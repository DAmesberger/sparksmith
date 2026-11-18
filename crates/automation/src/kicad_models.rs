//! 3D bodies for the shared desktop/CLI importer. Sparksmith shows its own
//! parametric recreations only: a footprint whose lands identify the package
//! of a shipped generator preset gets that preset's generated model. A vendor
//! STEP the project holds is never the body: it is recorded in the import
//! report as a reference source for a later recreation. No guessed geometry
//! or electrical mapping is written into the imported board.
mod presets;
use sparksmith_core::library::bundle::{AssetBytes, PackedAsset};
use sparksmith_core::library::model3d::*;
use sparksmith_core::library::{DraftEdit, ModelPlacement, PackageModel, Pad};
use sparksmith_core::units::Nm;
use sparksmith_core::{Engine, LibraryCommand, ModelRef, Operation, Outcome, Revision};
use sparksmith_io::project_models::{ProjectModels, library_reference};
use sparksmith_kicad::models::{Choice, Provider, Reference, Selection};
use sparksmith_kicad::report::{Case, Placeholder};
use std::collections::BTreeMap;
use std::path::Path;

pub(crate) struct Models {
    deadline: std::time::Instant,
    files: ProjectModels,
    presets: presets::Presets,
    /// Each project reference's resolved source ("path (sha256 …)"), or why
    /// it did not resolve.
    sources: BTreeMap<String, Result<String, String>>,
    models: BTreeMap<sparksmith_core::ids::ContentHash, ModelRef>,
}
impl Models {
    pub(crate) fn new(root: &Path) -> Result<Self, String> {
        Ok(Self {
            deadline: std::time::Instant::now() + std::time::Duration::from_secs(120),
            files: ProjectModels::new(root).map_err(|e| e.to_string())?,
            presets: presets::Presets::default(),
            sources: BTreeMap::new(),
            models: BTreeMap::new(),
        })
    }
    fn check_time(&self) -> Result<(), String> {
        check_time(self.deadline)
    }
    /// The project file a reference names, for the report only; its bytes
    /// are hashed and dropped, never inspected or packed.
    fn source(&mut self, path: &str) -> Result<Result<String, String>, String> {
        if let Some(value) = self.sources.get(path) {
            return Ok(value.clone());
        }
        self.check_time()?;
        let value = match self.files.resolve(path) {
            Ok(Some(found)) => Ok(format!(
                "{} (sha256 {})",
                found.source,
                AssetBytes::new(found.bytes).hash()
            )),
            Ok(None) => Err("not found in the project folder".to_owned()),
            Err(error) => Err(error.to_string()),
        };
        self.check_time()?;
        self.sources.insert(path.into(), value.clone());
        Ok(value)
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
        if !models.is_empty() && refs.is_empty() {
            return Ok(Selection {
                choice: None,
                case: Case::NoModel,
                reason: "All source model references are hidden; no generated replacement. The import retains its labelled placeholder because invisible package models are not supported.".into(),
                placeholder: Some(Placeholder::Hidden),
            });
        }
        let library = refs.iter().any(|r| library_reference(&r.path));
        let mut found = Vec::new();
        let mut missing = Vec::new();
        for reference in refs.iter().filter(|r| !library_reference(&r.path)) {
            match self.source(&reference.path)? {
                Ok(source) => found.push(source),
                Err(why) => missing.push(format!("{}: {why}", reference.path)),
            }
        }
        let mut notes = Vec::new();
        if refs.is_empty() {
            notes.push("No model reference".to_owned());
        }
        if library {
            notes.push("KiCad library reference is never resolved".to_owned());
        }
        if !found.is_empty() {
            notes.push(format!(
                "Vendor reference source for a recreation (never used as the body): {}",
                found.join("; ")
            ));
        }
        if !missing.is_empty() {
            notes.push(format!(
                "Vendor reference unresolved: {}",
                missing.join("; ")
            ));
        }
        let note = notes.join("; ");
        self.check_time()?;
        match self
            .presets
            .match_model(engine, pads, prefix, &mut self.models, self.deadline)
            .map_err(|error| format!("{note}; preset model unavailable: {error}"))?
        {
            Ok(choice) => Ok(Selection {
                reason: format!("{note}; {}", choice.provenance),
                choice: Some(choice),
                case: if library {
                    Case::LibraryModelGenerated
                } else {
                    Case::GeneratedModel
                },
                placeholder: None,
            }),
            Err(miss) => Ok(Selection {
                choice: None,
                case: Case::NoModel,
                placeholder: Some(if matches!(miss, presets::Miss::PrefixMismatch { .. }) {
                    Placeholder::PrefixMismatch
                } else if !found.is_empty() {
                    Placeholder::VendorReferenceFound
                } else if !missing.is_empty() {
                    Placeholder::VendorReferenceMissing
                } else if matches!(miss, presets::Miss::NoLands) {
                    Placeholder::NoLands
                } else if library {
                    Placeholder::KicadLibraryReferenceNoPreset
                } else {
                    Placeholder::NoReferenceNoPreset
                }),
                reason: format!("{note}; {miss}"),
            }),
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
    type Fallible = Result<(), Box<dyn std::error::Error>>;
    const VENDOR: &[u8] = b"Original synthetic vendor bytes; never parsed.";

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
    fn all_hidden_references_do_not_read_files_or_generate_replacement_bodies() -> Fallible {
        let dir = tempfile::tempdir()?;
        let mut models = Models::new(dir.path())?;
        let mut reference = reference("${KICAD9_3DMODEL_DIR}/never-read.step");
        reference.hidden = true;
        let mut engine = Engine::demo()?;
        let selected = models.prepare(&mut engine, &[], "R", &[reference])?;
        assert!(selected.choice.is_none());
        assert_eq!(selected.case, Case::NoModel);
        assert_eq!(selected.placeholder, Some(Placeholder::Hidden));
        assert!(selected.reason.contains("hidden"));
        assert!(models.sources.is_empty() && models.models.is_empty());
        Ok(())
    }

    #[test]
    fn expired_import_refuses_without_publishing_any_model() -> Fallible {
        let dir = tempfile::tempdir()?;
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
    fn a_kicad_library_reference_is_not_resolved_even_with_a_same_name_local_file() -> Fallible {
        let dir = tempfile::tempdir()?;
        std::fs::write(dir.path().join("own.step"), VENDOR)?;
        let mut models = Models::new(dir.path())?;
        let mut engine = Engine::demo()?;
        let selected = models.prepare(
            &mut engine,
            &[],
            "U",
            &[reference("${KICAD9_3DMODEL_DIR}/own.step")],
        )?;
        assert!(selected.choice.is_none());
        assert!(models.sources.is_empty(), "never looked up");
        assert!(!selected.reason.contains("sha256"), "{}", selected.reason);
        assert!(engine.project().library.models.is_empty());
        Ok(())
    }

    /// Vendor files are reference sources only: never a body, never packed;
    /// each placeholder instance is counted under its reason.
    #[test]
    fn vendor_files_are_recorded_as_sources_never_used_and_placeholders_are_categorised() -> Fallible
    {
        let context = sparksmith_core::library::standards::Context::fixture()?;
        let _context = context.enter();
        let dir = tempfile::tempdir()?;
        std::fs::write(dir.path().join("own.step"), VENDOR)?;
        let mut board = String::from(
            r#"(kicad_pcb (version 20240108) (generator "original placeholder reason fixture")
            (general (thickness 1.6)) (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
            (gr_rect (start 0 0) (end 60 30) (stroke (width 0.1) (type default)) (layer "Edge.Cuts"))"#,
        );
        for (name, x, model) in [
            ("U1", 10, r#"(model "${KIPRJMOD}/own.step")"#),
            ("U2", 20, r#"(model "own.step")"#),
            ("U3", 30, r#"(model "missing.step")"#),
            (
                "U4",
                40,
                r#"(model "${KICAD9_3DMODEL_DIR}/Synthetic.3dshapes/x.wrl")"#,
            ),
            ("U5", 50, ""),
        ] {
            board.push_str(&format!(
                r#"(footprint "Original:Odd" (layer "F.Cu") (at {x} 10)
                (property "Reference" "{name}")
                (pad "1" smd rect (at 0 0) (size 1.3 0.7) (layers "F.Cu" "F.Mask" "F.Paste"))
                (pad "2" smd rect (at 3.1 0.9) (size 0.4 2.2) (layers "F.Cu" "F.Mask" "F.Paste"))
                (pad "3" smd rect (at -2.2 1.7) (size 0.9 0.9) (layers "F.Cu" "F.Mask" "F.Paste"))
                {model})"#
            ));
        }
        board.push(')');
        let path = dir.path().join("reasons.kicad_pcb");
        std::fs::write(&path, board)?;
        let imported = crate::kicad_import::import(&path, None)?;
        let report = &imported.report;
        assert_eq!(report.count(Case::NoModel), 5);
        assert!(imported.engine.project().library.models.is_empty());
        assert!(imported.engine.project().library.assets.is_empty());
        let counts = report.placeholders();
        assert_eq!(counts.get(&Placeholder::VendorReferenceFound), Some(&2));
        assert_eq!(counts.get(&Placeholder::VendorReferenceMissing), Some(&1));
        assert_eq!(
            counts.get(&Placeholder::KicadLibraryReferenceNoPreset),
            Some(&1)
        );
        assert_eq!(counts.get(&Placeholder::NoReferenceNoPreset), Some(&1));
        let u1 = report
            .details()
            .iter()
            .find(|d| d.footprint == "U1" && d.case == Case::NoModel)
            .ok_or("U1 detail")?;
        assert_eq!(u1.placeholder, Some(Placeholder::VendorReferenceFound));
        assert!(u1.path.contains("own.step (sha256 "), "{}", u1.path);
        assert!(u1.path.contains("never used as the body"), "{}", u1.path);
        let summary = report.summary();
        assert!(summary.contains(Placeholder::VendorReferenceFound.describe()));
        Ok(())
    }
}
