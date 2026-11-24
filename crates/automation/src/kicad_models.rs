//! The parts of the shared desktop/CLI importer. A source footprint whose
//! lands identify the package of a shipped generator preset becomes that
//! generator's part, made through `library.generate` with the source's lands
//! as its land pattern ([`parts`]), so it is generated exactly as a part from
//! the palette or the AI is, and the board's copper stays the source's. Any
//! other footprint becomes a project part of its lands, whose body comes
//! later from a recreation from datasheets. A vendor STEP the project holds is
//! never the body: it is recorded in the import report as a reference source
//! for that recreation.
mod lands;
mod parts;
pub(crate) mod presets;
use sparksmith_core::Engine;
use sparksmith_core::library::bundle::AssetBytes;
use sparksmith_core::library::{ModelPlacement, Pad};
use sparksmith_io::project_models::{ProjectModels, library_reference};
use sparksmith_kicad::models::{Made, Outcome, PartRequest, Provider};
use sparksmith_kicad::report::{Case, Placeholder};
use std::collections::BTreeMap;
use std::path::Path;

pub(crate) struct Models<'a> {
    files: ProjectModels,
    presets: &'a mut presets::Presets,
    /// Each project reference's resolved source ("path (sha256 …)"), or why
    /// it did not resolve.
    sources: BTreeMap<String, Result<String, String>>,
}
impl<'a> Models<'a> {
    /// The parts of the project at `root`, made by the generator workers
    /// `runtime` gives: a busy pool delays the import, until whoever waits
    /// for it cancels it.
    pub(crate) fn new(
        root: &Path,
        runtime: crate::kicad_import::Runtime,
        presets: &'a mut presets::Presets,
    ) -> Result<Self, String> {
        presets.start_import(runtime());
        Ok(Self {
            files: ProjectModels::new(root).map_err(|e| e.to_string())?,
            presets,
            sources: BTreeMap::new(),
        })
    }
    /// The project file a reference names, for the report only; its bytes
    /// are hashed and dropped, never inspected or packed.
    fn source(&mut self, path: &str) -> Result<Result<String, String>, String> {
        if let Some(value) = self.sources.get(path) {
            return Ok(value.clone());
        }
        check_cancelled()?;
        let value = match self.files.resolve(path) {
            Ok(Some(found)) => Ok(format!(
                "{} (sha256 {})",
                found.source,
                AssetBytes::new(found.bytes).hash()
            )),
            Ok(None) => Err("not found in the project folder".to_owned()),
            Err(error) => Err(error.to_string()),
        };
        check_cancelled()?;
        self.sources.insert(path.into(), value.clone());
        Ok(value)
    }
}
impl Provider for Models<'_> {
    fn part(&mut self, engine: &mut Engine, request: &PartRequest<'_>) -> Result<Outcome, String> {
        check_cancelled()?;
        let runtime = self.presets.runtime(engine);
        if engine.generator_runtime().is_none() {
            engine.set_generators(runtime.clone());
        }
        let refs: Vec<_> = request.models.iter().filter(|r| !r.hidden).collect();
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
            notes.push(if request.models.is_empty() {
                "No model reference".to_owned()
            } else {
                "All model references hidden".to_owned()
            });
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
        check_cancelled()?;
        let identified = self
            .presets
            .identify(
                engine,
                request.pads,
                request.prefix,
                request.facts,
                request.mpn,
            )
            .map_err(|error| format!("{note}; presets unavailable: {error}"))?;
        let contradiction = request
            .mpn
            .and_then(|mpn| self.presets.contradiction(mpn, identified.as_ref().ok()));
        let source = |reason: String, placeholder: Placeholder| Made::Source {
            case: Case::SourcePart,
            reason,
            placeholder: Some(placeholder),
        };
        let fallback = if !found.is_empty() {
            Placeholder::VendorReferenceFound
        } else if !missing.is_empty() {
            Placeholder::VendorReferenceMissing
        } else if refs.is_empty() && !request.models.is_empty() {
            Placeholder::Hidden
        } else if library {
            Placeholder::KicadLibraryReferenceNoPreset
        } else {
            Placeholder::NoReferenceNoPreset
        };
        let made = match identified {
            Err(miss) => {
                let placeholder = match miss {
                    presets::Miss::PrefixMismatch { .. } => Placeholder::PrefixMismatch,
                    presets::Miss::NoLands => Placeholder::NoLands,
                    _ => fallback,
                };
                source(format!("{note}; {miss}"), placeholder)
            }
            Ok(identified) => {
                let candidate = self
                    .presets
                    .candidate(identified.index)
                    .ok_or("the identified preset is missing")?;
                check_cancelled()?;
                match parts::make(engine, candidate, &identified, request, runtime.as_ref())? {
                    Ok(mut generated) => {
                        generated.reason = format!("{note}; {}", generated.reason);
                        Made::Generated(Box::new(generated))
                    }
                    Err(why) => source(
                        format!("{note}; package {}, but {why}", identified.name),
                        Placeholder::GeneratorRefused,
                    ),
                }
            }
        };
        check_cancelled()?;
        Ok(Outcome {
            made,
            contradiction,
        })
    }
}

/// Refused once whoever waits for the import cancelled it.
fn check_cancelled() -> Result<(), String> {
    if sparksmith_generators::cancel::requested() {
        Err("the import was cancelled; nothing was imported".into())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
