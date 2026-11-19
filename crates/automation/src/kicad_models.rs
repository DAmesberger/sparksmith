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
mod presets;
use sparksmith_core::Engine;
use sparksmith_core::library::bundle::AssetBytes;
use sparksmith_core::library::{ModelPlacement, Pad};
use sparksmith_io::project_models::{ProjectModels, library_reference};
use sparksmith_kicad::models::{Made, Outcome, PartRequest, Provider};
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
}
impl Models {
    /// The parts of the project at `root`, made by generator workers that
    /// `runtime` gives for the import's deadline: batch admission, so a busy
    /// pool delays the import instead of costing it its parts.
    pub(crate) fn new(root: &Path, runtime: crate::kicad_import::Runtime) -> Result<Self, String> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(300);
        Ok(Self {
            deadline,
            files: ProjectModels::new(root).map_err(|e| e.to_string())?,
            presets: presets::Presets::with_runtime(runtime(deadline)),
            sources: BTreeMap::new(),
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
    fn part(&mut self, engine: &mut Engine, request: &PartRequest<'_>) -> Result<Outcome, String> {
        self.check_time()?;
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
        self.check_time()?;
        let identified = self
            .presets
            .identify(
                engine,
                request.pads,
                request.prefix,
                request.facts,
                self.deadline,
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
                self.check_time()?;
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
        self.check_time()?;
        Ok(Outcome {
            made,
            contradiction,
        })
    }
}

fn check_time(deadline: std::time::Instant) -> Result<(), String> {
    if std::time::Instant::now() >= deadline {
        Err("the import's parts reached their 300 second cooperative time limit".into())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
