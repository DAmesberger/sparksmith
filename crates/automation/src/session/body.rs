//! A 3D body for an existing part (docs/generator-plugins.md, "A body for
//! an existing part"). `library.body_fit` describes a placed part's
//! package and pads, and with a generator the body it would make, fitted to
//! them, changing nothing; `library.make_body` evaluates the generator's
//! part file and applies the one typed command `library.replace_body`. The
//! footprint is kept: only the package's model, placement, envelope and
//! lead contacts change, for every placed part that uses it.
use super::Session;
use super::generators::GeneratePartParams;
use crate::Result;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sparksmith_core::commands::body::{self, LeadPin, ReplaceBody};
use sparksmith_core::commands::{Generate, LibraryCommand};
use sparksmith_core::library::generated::{GeneratorParams, GeneratorScope};
use sparksmith_core::library::package_fit::to_datum;
use sparksmith_core::units::Length;
use sparksmith_core::{Operation, ids::PackageRef};

/// Bytes of a designator, a reason and the sources.
const MAX_DESIGNATOR: usize = 64;
const MAX_TEXT: usize = 4096;

/// `library.body_fit`. Required: `designator`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BodyFitParams {
    /// Required: the placed part by designator, e.g. `J16`.
    pub designator: String,
    /// A shipped generator by name (`library.generators`). Without it the
    /// reply is the part's package and pads only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generator: Option<String>,
    /// A preset of that generator, whose values `params` override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
    /// Parameters by key; lengths in integer nanometres, angles in integer
    /// millidegrees.
    #[serde(default)]
    pub params: GeneratorParams,
    /// Which pad each lead lands on, `[{lead, pad}]`; a lead not listed
    /// lands the pad of its own number.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pins: Vec<LeadPin>,
    /// How far, in mm, a lead may miss its land beyond the rule; 0–0.5.
    #[serde(default)]
    pub tolerance: Length,
    /// Instead of a generator: why no shipped generator makes this body
    /// (the family or the parameter that is missing). Refused with this
    /// text, so a run reports it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub missing: Option<String>,
}

/// `library.make_body`. Required: `designator`, `generator`, `sources` and
/// `expected_revision`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MakeBodyParams {
    /// The placed part by designator; every part that uses its package
    /// changes with it.
    pub designator: String,
    /// A shipped generator by name.
    pub generator: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
    #[serde(default)]
    pub params: GeneratorParams,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pins: Vec<LeadPin>,
    #[serde(default)]
    pub tolerance: Length,
    /// Where the values come from: each datasheet with the pages read
    /// ("evqp2.pdf: drawing 2, land pattern 3"), and any photos compared.
    pub sources: String,
    pub expected_revision: u64,
}

fn invalid(message: String) -> crate::Error {
    crate::Error::Document(sparksmith_core::Error::Invalid(message))
}

fn check_text(what: &str, text: &str, max: usize) -> Result<()> {
    if text.trim().is_empty() || text.len() > max {
        return Err(invalid(format!("{what}: give 1–{max} bytes")));
    }
    Ok(())
}

/// The body a request asks for: the generator and its values, and how
/// its leads land.
struct BodySpec {
    generator: String,
    preset: Option<String>,
    params: GeneratorParams,
    pins: Vec<LeadPin>,
    tolerance: Length,
}

impl Session {
    /// The command `library.replace_body` the parameters ask for, its part
    /// file evaluated.
    fn body_request(
        &mut self,
        designator: &str,
        spec: BodySpec,
        provenance: String,
    ) -> Result<ReplaceBody> {
        let package = body::package_of(self.engine.document(), designator)?;
        self.ensure_generators();
        let generate: Generate = self.generate_request(GeneratePartParams {
            generator: Some(spec.generator),
            module: None,
            scope: GeneratorScope::Package,
            preset: spec.preset,
            params: spec.params,
            name: None,
            replaces: None,
            expected_revision: self.engine.revision(),
        })?;
        Ok(ReplaceBody {
            package,
            generate,
            pins: spec.pins,
            tolerance: spec.tolerance,
            provenance,
        })
    }

    /// `library.body_fit`: the part's package and pads; with a generator,
    /// the plan of the body it makes. A `missing` reason is refused as it
    /// stands.
    pub(super) fn body_fit(&mut self, params: BodyFitParams) -> Result<Value> {
        check_text("body.designator", &params.designator, MAX_DESIGNATOR)?;
        let target = self.body_target(&params.designator)?;
        let Some(generator) = params.generator else {
            if let Some(missing) = params.missing {
                check_text("body.missing", &missing, MAX_TEXT)?;
                return Err(invalid(format!(
                    "body.no_generator: no shipped generator makes the body of {}: {missing}. The part keeps its body.",
                    params.designator
                )));
            }
            return Ok(json!({"target": target}));
        };
        let spec = BodySpec {
            generator,
            preset: params.preset,
            params: params.params,
            pins: params.pins,
            tolerance: params.tolerance,
        };
        let request = self.body_request(&params.designator, spec, "library.body_fit".into())?;
        let plan = self.engine.body_plan(&request)?;
        let mut reply = serde_json::to_value(&plan)?;
        reply["target"] = target;
        Ok(reply)
    }

    /// `library.make_body`: the body evaluated, fitted and applied as
    /// `library.replace_body`, with its provenance.
    pub(super) fn make_body(&mut self, params: MakeBodyParams) -> Result<Value> {
        check_text("body.designator", &params.designator, MAX_DESIGNATOR)?;
        check_text("body.sources", &params.sources, MAX_TEXT / 2)?;
        let preset = params
            .preset
            .as_ref()
            .map(|p| format!(", preset {p}"))
            .unwrap_or_default();
        let provenance = format!(
            "{} {} with the generator ‘{}’{preset}, fitted to the footprint's pads (the footprint kept). Reviewed: no.",
            body::RECREATED,
            params.sources.trim(),
            params.generator
        );
        let spec = BodySpec {
            generator: params.generator,
            preset: params.preset,
            params: params.params,
            pins: params.pins,
            tolerance: params.tolerance,
        };
        let request = self.body_request(&params.designator, spec, provenance)?;
        let operation = Operation::Library(LibraryCommand::ReplaceBody(Box::new(request)));
        self.apply(params.expected_revision, operation)
    }

    /// The body of the placed part `designator`, for the review check.
    pub(super) fn body_state(&self, designator: &str) -> Result<crate::review::BodyState> {
        let document = self.engine.document();
        let package = body::package_of(document, designator)?;
        let published = document
            .project
            .library
            .package(package)
            .ok_or_else(|| invalid(format!("body.package: {package} is missing")))?;
        Ok(crate::review::BodyState {
            part: designator.to_owned(),
            package: format!("‘{}’ {}", published.meta.name, package.revision),
            model: published.definition.model.describe(),
            parts: body::parts_using(document, package),
            recreated: published.meta.provenance.contains(body::RECREATED),
        })
    }

    /// The part's package, the parts that share it and its footprint's
    /// pads in the package's datum frame (mm, degrees).
    fn body_target(&self, designator: &str) -> Result<Value> {
        let document = self.engine.document();
        let package: PackageRef = body::package_of(document, designator)?;
        let library = &document.project.library;
        let published = library
            .package(package)
            .ok_or_else(|| invalid(format!("body.package: {package} is missing")))?;
        let instance = document
            .project
            .instances
            .iter()
            .find(|i| i.designator == designator);
        let footprint = instance
            .and_then(|i| {
                let component = library.component(i.component)?;
                sparksmith_core::instances::resolve_physical(
                    library,
                    &component.definition,
                    i.physical?,
                )
                .ok()
            })
            .map(|view| view.footprint);
        let pads: Vec<Value> = footprint
            .map(|f| {
                f.definition
                    .pads
                    .iter()
                    .map(|p| {
                        let at = to_datum(&f.definition.datum, p.position());
                        json!({
                            "id": p.id,
                            "number": p.number,
                            "at": [round(at[0]), round(at[1])],
                            "size": p.shape.extent().map(round),
                            "rotation_deg": round(p.rotation_deg()),
                            "kind": p.kind,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let p = &published.definition;
        Ok(json!({
            "part": designator,
            "parts": body::parts_using(document, package),
            "package": {
                "id": package.id, "revision": package.revision,
                "name": published.meta.name,
                "terminals": p.terminals.iter().map(|t| t.designation.clone()).collect::<Vec<_>>(),
                "envelope_mm": p.envelope().map(round),
                "model": p.model.describe(),
            },
            "footprint": {
                "name": footprint.map(|f| f.meta.name.clone()),
                "pads": pads,
            },
        }))
    }
}

/// A millimetre or degree value to the micrometre, for a reply.
fn round(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

/// An imported board for tests of bodies: R1 and R2 share a two-pad chip
/// footprint (pads 1.6 mm apart, 0.95 × 1.0 mm), U1 has a three-pad one,
/// J1 one pad and an unnumbered 1.2 mm non-plated hole 1.6 mm from it;
/// every package keeps the import's placeholder body. Original synthetic
/// data.
#[cfg(any(test, feature = "test-fixtures"))]
pub fn fixture_engine() -> std::result::Result<sparksmith_core::Engine, String> {
    let mut board = String::from(
        r#"(kicad_pcb (version 20240108) (generator "original body fixture")
        (general (thickness 1.6)) (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
        (gr_rect (start 0 0) (end 40 20) (stroke (width 0.1) (type default)) (layer "Edge.Cuts"))"#,
    );
    for (name, x) in [("R1", 10), ("R2", 20)] {
        board.push_str(&format!(
            r#"(footprint "Original:Chip2" (layer "F.Cu") (at {x} 10)
            (property "Reference" "{name}")
            (pad "1" smd rect (at -0.8 0) (size 0.95 1.0) (layers "F.Cu" "F.Mask" "F.Paste"))
            (pad "2" smd rect (at 0.8 0) (size 0.95 1.0) (layers "F.Cu" "F.Mask" "F.Paste")))"#
        ));
    }
    board.push_str(
        r#"(footprint "Original:Odd" (layer "F.Cu") (at 30 10)
            (property "Reference" "U1")
            (pad "1" smd rect (at 0 0) (size 1.3 0.7) (layers "F.Cu" "F.Mask" "F.Paste"))
            (pad "2" smd rect (at 3.1 0.9) (size 0.4 2.2) (layers "F.Cu" "F.Mask" "F.Paste"))
            (pad "3" smd rect (at -2.2 1.7) (size 0.9 0.9) (layers "F.Cu" "F.Mask" "F.Paste")))
        (footprint "Original:Peg" (layer "F.Cu") (at 5 10)
            (property "Reference" "J1")
            (pad "1" smd rect (at -0.8 0) (size 0.95 1.0) (layers "F.Cu" "F.Mask" "F.Paste"))
            (pad "" np_thru_hole circle (at 0.8 0) (size 1.2 1.2) (drill 1.2) (layers "*.Cu" "*.Mask"))))"#,
    );
    let defaults = sparksmith_io::new_project::shipped().map_err(|e| e.to_string())?;
    let imported = sparksmith_kicad::import(
        board.as_bytes(),
        &sparksmith_kicad::Setup {
            board: &defaults.board,
            presets: &defaults.presets,
            profile: None,
            evaluators: super::hosted(),
            project_file: None,
            schematic: None,
        },
    )
    .map_err(|e| e.to_string())?;
    Ok(imported.engine)
}
