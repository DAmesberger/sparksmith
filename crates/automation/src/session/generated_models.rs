//! A generated part's 3D model (slice GP2, docs/generator-plugins.md): the
//! part file the module's generation names, found in the generator's
//! bundle or in the project, evaluated through Solidcraft at the
//! generation's values exactly as any part file is, and checked against
//! the generation before `library.generate` stores it: no blocking finding,
//! the bounds the generation's envelope within 10 µm, and every lead
//! contact under a body of a lead material.
use crate::Result;
use sparksmith_core::Engine;
use sparksmith_core::commands::GeneratedModel;
use sparksmith_core::ids::ContentHash;
use sparksmith_core::library::bundle::AssetBytes;
use sparksmith_core::library::model3d::{
    AssetRef, ModelDefinition, ModelFormat, ModelProvenance, Redistribution,
};
use sparksmith_core::library::{Basis, Provenance};
use sparksmith_core::units::Nm;
use sparksmith_generators::abi::v1;
use sparksmith_io::generator_bundle::GeneratorBundle;
use sparksmith_solidcraft::models::part_file::{self, PartValues};

/// How far the part file's bounds may lie from the generation's envelope,
/// and a lead contact outside its lead body, mm.
const TOLERANCE_MM: f64 = 0.01;

fn refused(message: String) -> crate::Error {
    crate::Error::Document(sparksmith_core::Error::Invalid(format!(
        "generated.model: {message}. Nothing was changed."
    )))
}

/// The part file `hash`: from `bundle`, else as a Model the project packs.
/// Its bytes, the Model it becomes (a new one, or the project's), its name
/// and whether the project already packs the bytes.
fn find(
    engine: &Engine,
    hash: ContentHash,
    bundle: Option<&GeneratorBundle>,
) -> Result<(Vec<u8>, ModelDefinition, String, bool)> {
    let library = &engine.project().library;
    let packed = library.assets.iter().find(|a| a.hash == hash);
    let held = library.models.iter().find(|m| {
        m.definition.format == ModelFormat::Solidcraft && m.definition.asset.hash == hash
    });
    if let Some(part) = bundle.and_then(|b| b.part(hash)) {
        let definition = match held {
            Some(m) => m.definition.clone(),
            None => ModelDefinition {
                asset: AssetRef {
                    hash,
                    bytes: u64::try_from(part.bytes.len()).unwrap_or(u64::MAX),
                    media_type: ModelFormat::Solidcraft.media_type().into(),
                    file_name: part
                        .entry
                        .file
                        .rsplit('/')
                        .next()
                        .unwrap_or("part.solidcraft")
                        .to_owned(),
                },
                format: ModelFormat::Solidcraft,
                provenance: ModelProvenance {
                    redistribution: Redistribution::Permitted,
                    record: Provenance {
                        origin: None,
                        basis: Basis::Authored,
                        licence: Some(part.entry.licence.clone()),
                    },
                    ..ModelProvenance::default()
                },
                ..ModelDefinition::default()
            },
        };
        return Ok((
            part.bytes.clone(),
            definition,
            part.entry.title.clone(),
            packed.is_some(),
        ));
    }
    match (held, packed) {
        (Some(model), Some(asset)) => Ok((
            asset.data.as_slice().to_vec(),
            model.definition.clone(),
            model.definition.asset.file_name.clone(),
            true,
        )),
        _ => Err(refused(format!(
            "the generator's part file sha256 {hash} is neither in its bundle nor in this project"
        ))),
    }
}

/// What `library.generate` takes as `model` for `generation`: nothing for
/// a part its generator makes without a body (`Model::NoBody`), else its
/// part file evaluated ([`generated_model`]).
pub(crate) fn model_for(
    engine: &Engine,
    generation: &v1::Generation,
    bundle: Option<&GeneratorBundle>,
) -> Result<Option<GeneratedModel>> {
    if matches!(generation.model, Some(v1::Model::NoBody)) {
        return Ok(None);
    }
    generated_model(engine, generation, bundle).map(Some)
}

/// The model of `generation` for `library.generate`: its part file
/// evaluated at its values and checked against the generation.
pub(crate) fn generated_model(
    engine: &Engine,
    generation: &v1::Generation,
    bundle: Option<&GeneratorBundle>,
) -> Result<GeneratedModel> {
    let Some(v1::Model::PartFile { asset, values }) = &generation.model else {
        return Err(refused(
            "the generator made no 3D model; a part needs one".into(),
        ));
    };
    let hash = ContentHash::parse(asset)
        .ok_or_else(|| refused(format!("\"{asset}\" is not a SHA-256")))?;
    let (bytes, definition, name, packed) = find(engine, hash, bundle)?;
    let as_is = part_file::evaluate(&bytes, &PartValues::new())?;
    let values = part_file::generator_values(&as_is, values).map_err(refused)?;
    let part = part_file::evaluate(&bytes, &values)?;
    if let Some(refusal) = part.refusal() {
        return Err(refused(format!(
            "its part file at the generator's values: {refusal}"
        )));
    }
    // The bounds against the envelope, centred in x and y on the seating
    // plane. Leads may reach below the seating plane (through-hole posts
    // and wires, which the envelope leaves out): the bottom is the lowest
    // body that is not a lead.
    let geometry = &generation.package.geometry;
    let mm = |v: i64| Nm::new(v).map(Nm::to_mm_f64).unwrap_or(f64::NAN);
    let [l, w, h] = geometry.envelope_nm.map(mm);
    let [mut lo, hi] = part.bounds_mm;
    if let Some(bottom) = part_file::seated_bottom(&part.bodies) {
        lo[2] = lo[2].max(bottom);
    }
    // The envelope is centred, so in x and y it is the smallest centred box
    // that holds the bounds: a part whose sides differ (a DPAK's leads on
    // one side, its flange on the other) reaches the envelope on one side
    // only.
    let reach = |axis: usize| lo[axis].abs().max(hi[axis].abs());
    let off = [
        (reach(0) - l / 2.0).abs(),
        (reach(1) - w / 2.0).abs(),
        lo[2].abs(),
        (hi[2] - h).abs(),
    ]
    .into_iter()
    .fold(0.0, f64::max);
    if off.is_nan() || off > TOLERANCE_MM {
        return Err(refused(format!(
            "its part file at the generator's values spans {lo:?} to {hi:?} mm, not the generation's {l} × {w} × {h} mm envelope"
        )));
    }
    // Every lead contact under a lead body, each lead body bound to its
    // terminal (slice B2); package terminals are designated by their keys.
    let contacts: Vec<(String, [f64; 2])> = geometry
        .leads
        .iter()
        .map(|lead| (lead.key.clone(), [mm(lead.contact[0]), mm(lead.contact[1])]))
        .collect();
    let mut leads = part_file::lead_bodies(&part, &contacts, true).map_err(refused)?;
    // Lead bodies under no lead contact take the terminal of the land they
    // stand on (a module's ground array: one contact, every pad bound); a
    // body in a hole that lands no terminal (an alignment pin) binds none.
    let lands: Vec<(String, [f64; 2])> = generation
        .footprint
        .pads
        .iter()
        .filter(|p| !p.terminal.is_empty())
        .map(|p| (p.terminal.clone(), [mm(p.at[0]), mm(p.at[1])]))
        .collect();
    for lead in part_file::lead_bodies(&part, &lands, false).map_err(refused)? {
        if !leads.iter().any(|l| l.body == lead.body) {
            leads.push(lead);
        }
    }
    leads.sort_by(|a, b| a.body.cmp(&b.body));
    let mut definition = definition;
    if definition.bounds_nm == ModelDefinition::default().bounds_nm {
        definition.bounds_nm = sparksmith_core::library::model3d::bounds_nm(as_is.bounds_mm)?;
    }
    Ok(GeneratedModel {
        definition,
        name,
        data: (!packed).then(|| AssetBytes::new(bytes)),
        derived: sparksmith_core::library::package_model::PartDerived {
            leads,
            ..part_file::derived(&part)?
        },
        values,
    })
}
