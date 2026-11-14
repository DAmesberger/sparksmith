//! Generated models of families whose in-code builder has given way to a
//! shipped generator (docs/design/part-generation-plan.md, phase C).
//!
//! A package that stores `PackageModel::Generated` with such a family's
//! typed form (an earlier project, a template build, the 3D library's
//! "Generated" entry) keeps its model: its form's values become the
//! generator's parameters, the shipped module runs (package scope) and its
//! part file is evaluated at the values the module gives, exactly as for a
//! part the generator made. The lead bodies bind to the package's
//! terminals in lead order, as the builders bound them, and the package's
//! own material choices (`GeneratedMaterials`) give the looks. The module
//! and its part files come from the context the host made from the
//! shipped library; a host without them refuses the build, naming why.
//!
//! One arm of [`generator`] per retired family; nothing else here knows a
//! family.
use super::GeneratedBody;
use super::part_file::{self, PartValues};
use crate::{Error, Result};
use sparksmith_core::library::standards::Context;
use sparksmith_core::library::{BodyRole, PackageDefinition, PackageForm, PackageModel};
use sparksmith_core::units::Nm;
use sparksmith_generators::abi::v1::{self, Model, ParamValue, Scope};
use sparksmith_generators::abi::{Failure, GeneratorRuntime};
use sparksmith_generators::params::complete;
use sparksmith_generators::{Limits, WasmiRuntime};
use std::sync::Arc;

/// The shipped generator that makes the model of `form`'s family, with the
/// form's values as its parameters; `None` for a family that still has its
/// in-code builder.
pub fn generator(form: &PackageForm) -> Option<(&'static str, v1::Params)> {
    let mut params = v1::Params::new();
    let mut int = |key: &str, v: i64| {
        params.insert(key.into(), ParamValue::Int(v));
    };
    match form {
        PackageForm::Header(h) => {
            int("rows", i64::from(h.rows));
            int("columns", i64::from(h.pins_per_row));
            int("pitch", h.pitch_nm.get());
            int("row_pitch", h.row_pitch_nm.get());
            int("body_height", h.body_height_nm.get());
            int("pin_side", h.pin_side_nm.get());
            int("pin_above", h.pin_above_nm.get());
            int("pin_below", h.pin_below_nm.get());
            Some(("header", params))
        }
        PackageForm::Tabbed(t) => {
            int("leads_s", i64::from(t.leads_per_side[0]));
            int("leads_n", i64::from(t.leads_per_side[1]));
            int("step_s", i64::from(t.lead_step[0]));
            int("step_n", i64::from(t.lead_step[1]));
            int("pitch", t.pitch_nm.get());
            int("body_length", t.body_length_nm.get());
            int("body_width", t.body_width_nm.get());
            int("body_height", t.body_height_nm.get());
            int("standoff", t.standoff_nm.get());
            int("span", t.span_nm.get());
            int("lead_width", t.lead_width_nm.get());
            int("lead_thickness", t.lead_thickness_nm.get());
            int("foot_length", t.foot_length_nm.get());
            let kind = match t.tab {
                None => "none",
                Some(tab) => {
                    int("tab_width", tab.width_nm.get());
                    int("tab_length", tab.length_nm.get());
                    match tab.flange_nm {
                        None => "lead",
                        Some(p) => {
                            int("flange", p.get());
                            "flange"
                        }
                    }
                }
            };
            params.insert("tab".into(), ParamValue::Text(kind.into()));
            Some(("tabbed", params))
        }
        _ => None,
    }
}

/// The builder revision a retired family's stored models name: the one
/// its last in-code builder had, so they keep building.
pub const FAMILY_REV: u16 = 1;

fn refused(name: &str, why: impl std::fmt::Display) -> Error {
    Error::Invalid(format!(
        "model.generator: the {name} generator could not make this package's model: {why}"
    ))
}

/// The evaluated part file of `package`'s generated model, with its bodies
/// as the board inserts them (role, material, terminal).
pub fn evaluate(
    package: &PackageDefinition,
) -> Result<(Arc<part_file::EvaluatedPart>, Vec<GeneratedBody>)> {
    let PackageModel::Generated { materials, .. } = &package.model else {
        return Err(Error::Invalid(
            "model.unsupported: only generated package models are built here".into(),
        ));
    };
    let (name, params) = generator(&package.form).ok_or_else(|| {
        Error::Invalid(format!(
            "model.unsupported: no shipped generator makes {} models",
            package.form.label()
        ))
    })?;
    let context = Context::current().map_err(|e| refused(name, e))?;
    let shipped = context.generator(name).map_err(|e| refused(name, e))?;
    let fail = |f: Failure| refused(name, f);
    let runtime = WasmiRuntime::new(Limits::DEFAULT);
    let description = runtime.describe(&shipped.module).map_err(fail)?;
    let request = complete(&description, Scope::Package, &params).map_err(fail)?;
    let made = runtime.generate(&shipped.module, &request).map_err(fail)?;
    let generation = made.generation;
    let Some(Model::PartFile { asset, values }) = &generation.model else {
        return Err(refused(name, "it made no 3D model"));
    };
    let bytes = sparksmith_core::ids::ContentHash::parse(asset)
        .and_then(|hash| shipped.part(hash))
        .ok_or_else(|| refused(name, format!("its bundle has no part file sha256 {asset}")))?;
    let as_is = part_file::evaluate(bytes, &PartValues::new())?;
    let values = part_file::generator_values(&as_is, values).map_err(|e| refused(name, e))?;
    let part = part_file::evaluate(bytes, &values)?;
    if let Some(refusal) = part.refusal() {
        return Err(refused(name, refusal));
    }
    // Lead k is the package's k-th terminal when it lists exactly one per
    // lead (the builders' rule); otherwise the leads stay unbound.
    let leads = &generation.package.geometry.leads;
    let exact = leads.len() == package.terminals.len();
    let mm = |v: i64| Nm::new(v).map(Nm::to_mm_f64).unwrap_or(f64::NAN);
    let contacts: Vec<(String, [f64; 2])> = leads
        .iter()
        .zip(&package.terminals)
        .filter(|_| exact)
        .map(|(lead, terminal)| {
            (
                terminal.id.0.to_string(),
                [mm(lead.contact[0]), mm(lead.contact[1])],
            )
        })
        .collect();
    let bound = part_file::lead_bodies(&part, &contacts, true).map_err(|e| refused(name, e))?;
    let bodies = part
        .bodies
        .iter()
        .map(|b| {
            let role = part_file::role_of(b.material);
            // The package's material for the builder's role of the body.
            let builder_role = match role {
                BodyRole::Termination => BodyRole::Lead,
                other => other,
            };
            GeneratedBody {
                name: b.name.clone(),
                role,
                material: materials.material(&package.form, builder_role),
                terminal: bound
                    .iter()
                    .find(|l| l.body == b.name)
                    .and_then(|l| l.terminal.parse().ok()),
            }
        })
        .collect();
    Ok((part, bodies))
}
