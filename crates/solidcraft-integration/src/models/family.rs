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
use sparksmith_core::library::{BodyRole, LeadSides, PackageDefinition, PackageForm, PackageModel};
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
        PackageForm::Molded(m) => {
            use sparksmith_core::library::package_form::molded::MoldedLead;
            int("body_length", m.body_length_nm.get());
            int("body_width", m.body_width_nm.get());
            int("body_height", m.body_height_nm.get());
            int("span", m.span_nm.get());
            int("terminal_length", m.terminal_length_nm.get());
            int("terminal_width", m.terminal_width_nm.get());
            let lead = match m.lead {
                MoldedLead::Flat => "flat",
                MoldedLead::Bent => "bent",
                MoldedLead::JLead => "j_lead",
                MoldedLead::Melf => "melf",
            };
            params.insert("lead".into(), ParamValue::Text(lead.into()));
            params.insert("polarised".into(), ParamValue::Bool(m.polarised));
            // The package alone: what the part is (its symbol) leaves the
            // model as it is.
            params.insert("part".into(), ParamValue::Text("diode".into()));
            Some(("molded", params))
        }
        PackageForm::NoLead(n) => {
            let four = n.sides == LeadSides::Four;
            params.insert(
                "sides".into(),
                ParamValue::Text(if four { "four" } else { "two" }.into()),
            );
            let mut int = |key: &str, v: i64| {
                params.insert(key.into(), ParamValue::Int(v));
            };
            int("terminals", i64::from(n.terminals_per_side[0]));
            int("terminals_short", i64::from(n.terminals_per_side[1]));
            int("pitch", n.pitch_nm.get());
            int("body_length", n.body_length_nm.get());
            int("body_width", n.body_width_nm.get());
            int("body_height", n.body_height_nm.get());
            int("terminal_length", n.terminal_length_nm.get());
            int("terminal_width", n.terminal_width_nm.get());
            if let Some(pad) = n.exposed_pad {
                int("pad_length", pad.x_nm.get());
                int("pad_width", pad.y_nm.get());
            }
            params.insert(
                "exposed_pad".into(),
                ParamValue::Bool(n.exposed_pad.is_some()),
            );
            Some(("no-lead", params))
        }
        PackageForm::Grid(g) => {
            use sparksmith_core::library::package_form::grid::GridContact;
            let balls = g.contact == GridContact::Ball;
            params.insert(
                "contact".into(),
                ParamValue::Text(if balls { "ball" } else { "land" }.into()),
            );
            let mut int = |key: &str, v: i64| {
                params.insert(key.into(), ParamValue::Int(v));
            };
            int("rows", i64::from(g.rows));
            int("columns", i64::from(g.cols));
            int("pitch", g.pitch.x_nm.get());
            int("row_pitch", g.pitch.y_nm.get());
            int("body_length", g.body_length_nm.get());
            int("body_width", g.body_width_nm.get());
            int("body_height", g.body_height_nm.get());
            int("contact_diameter", g.contact_diameter_nm.get());
            int("contact_height", g.contact_height_nm.get());
            // The empty positions by name, in the form's own alphabet.
            let empty = g
                .depopulated
                .iter()
                .filter_map(|[r, c]| g.ball_name(*r, *c))
                .map(ParamValue::Text)
                .collect();
            params.insert("depopulated".into(), ParamValue::List(empty));
            params.insert(
                "row_letters".into(),
                ParamValue::Text(g.row_letters.clone()),
            );
            Some(("grid", params))
        }
        _ => None,
    }
}

/// The body of a no-lead part file that is the exposed pad.
const EXPOSED_PAD: &str = "exposed pad";

/// The role the retired builder gave a part file's body of `role` (by its
/// material) and `name`: the package's materials are chosen by it. A
/// header's posts and a grid's balls were leads (a land grid's lands
/// terminations); a no-lead package's exposed pad its own role.
fn builder_role(form: &PackageForm, name: &str, role: BodyRole) -> BodyRole {
    match (form, role) {
        (PackageForm::Header(_), BodyRole::Termination) => BodyRole::Lead,
        (PackageForm::Grid(g), BodyRole::Termination)
            if g.contact == sparksmith_core::library::package_form::grid::GridContact::Ball =>
        {
            BodyRole::Lead
        }
        (PackageForm::Molded(m), BodyRole::Termination)
            if m.lead != sparksmith_core::library::package_form::molded::MoldedLead::Melf =>
        {
            BodyRole::Lead
        }
        (PackageForm::NoLead(_), BodyRole::Termination) if name == EXPOSED_PAD => {
            BodyRole::ExposedPad
        }
        _ => role,
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
            // The package's material for the builder's role of the body,
            // else the part file's own (a MELF's glass, a band's ink), else
            // the family's.
            let builder_role = builder_role(&package.form, &b.name, role);
            let material = materials
                .roles
                .iter()
                .find(|c| c.role == builder_role)
                .map(|c| c.material)
                .or(b.material)
                .unwrap_or_else(|| materials.material(&package.form, builder_role));
            GeneratedBody {
                name: b.name.clone(),
                // An exposed pad keeps its own role; a post or lead is the
                // part file's termination where the builders said lead.
                role: if builder_role == BodyRole::ExposedPad {
                    builder_role
                } else {
                    role
                },
                material,
                terminal: bound
                    .iter()
                    .find(|l| l.body == b.name)
                    .and_then(|l| l.terminal.parse().ok()),
            }
        })
        .collect();
    Ok((part, bodies))
}
