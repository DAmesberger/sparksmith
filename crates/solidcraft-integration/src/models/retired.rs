//! What the retired typed package forms gave, for the format 5 converter
//! (`sparksmith_io::convert::v4`, part-generation slice D2). Nothing else
//! reads a version 4 form.
//!
//! A stored form (version 4 JSON: tagged by `family`, lengths in `_nm`)
//! becomes its family's shipped generator's parameters, as the 3D build
//! of a stored generated model ran it until D2 (the former
//! `models::family`); the generation's lead contacts are the package's,
//! and its part file at the generation's values the model. The bodies the
//! board built from the form (the family's former roles, the package's
//! materials, bound to terminals in lead order) are compared with what the
//! part-file model builds: any body whose role or material differs is a
//! refusal, so a converted file draws exactly as before. A chip's model
//! was built in code; it takes the chip generator's part file (the same
//! bodies, roles and materials within 1 µm, other face names), which is
//! said, not refused.
use super::part_file::{self, PartValues};
use crate::{Error, Result};
use serde_json::Value;
use sparksmith_core::ids::ContentHash;
use sparksmith_core::library::model3d::{
    AssetRef, ModelDefinition, ModelFormat, ModelProvenance, Redistribution,
};
use sparksmith_core::library::package_model::{LeadBody, PartDerived, PartValue};
use sparksmith_core::library::standards::Context;
use sparksmith_core::library::{Basis, BodyRole, Provenance};
use sparksmith_core::materials::MaterialId;
use sparksmith_generators::abi::v1::{self, Model, ParamValue, Scope};
use sparksmith_generators::abi::{Failure, GeneratorRuntime};
use sparksmith_generators::params::complete;
use sparksmith_generators::{Limits, WasmiRuntime};

/// A generated model as its generator's part file.
#[derive(Clone, Debug)]
pub struct RetiredModel {
    pub definition: ModelDefinition,
    pub name: String,
    pub bytes: Vec<u8>,
    pub values: PartValues,
    pub derived: PartDerived,
}

/// A bundled or shipped part file's file name, title and licence (from
/// its bundle manifest or sidecar), for the Model it becomes.
#[derive(Clone, Debug)]
pub struct PartInfo {
    pub file: String,
    pub title: String,
    pub licence: String,
}

/// What one stored form gives a package.
#[derive(Clone, Debug, Default)]
pub struct Retired {
    /// Each terminal's lead contact `[x, y]` nm, in terminal order; empty
    /// when the form gives none for this package.
    pub leads: Vec<[i64; 2]>,
    pub model: Option<RetiredModel>,
    pub refusals: Vec<String>,
    pub notes: Vec<String>,
}

fn int(v: &Value) -> Option<i64> {
    v.as_i64()
}

/// A whole number of `form[key]`.
fn get(form: &Value, key: &str) -> std::result::Result<i64, String> {
    int(&form[key]).ok_or_else(|| format!("the form has no whole {key}"))
}

fn text(v: &str) -> ParamValue {
    ParamValue::Text(v.into())
}

/// The shipped generator that makes `form`'s family and the form's values
/// as its parameters; `None` for a family no generator makes (generic,
/// USB-C).
pub fn generator(form: &Value) -> std::result::Result<Option<(&'static str, v1::Params)>, String> {
    let mut p = v1::Params::new();
    let mut put = |key: &str, v: ParamValue| {
        p.insert(key.into(), v);
    };
    let family = form["family"].as_str().unwrap_or("");
    let g = |key: &str| get(form, key).map(ParamValue::Int);
    let pair = |key: &str, i: usize| {
        int(&form[key][i])
            .map(ParamValue::Int)
            .ok_or_else(|| format!("the form has no {key}[{i}]"))
    };
    let name = match family {
        "chip" => {
            put("part", text(form["kind"].as_str().unwrap_or("resistor")));
            put("length", g("length_nm")?);
            put("width", g("width_nm")?);
            put("height", g("height_nm")?);
            put("termination", g("termination_nm")?);
            "chip"
        }
        "header" => {
            put("rows", g("rows")?);
            put("columns", g("pins_per_row")?);
            put("pitch", g("pitch_nm")?);
            put("row_pitch", g("row_pitch_nm")?);
            put("body_height", g("body_height_nm")?);
            put("pin_side", g("pin_side_nm")?);
            put("pin_above", g("pin_above_nm")?);
            put("pin_below", g("pin_below_nm")?);
            "header"
        }
        "gull_wing" => {
            put("leads", pair("leads_per_side", 0)?);
            put("leads_short", pair("leads_per_side", 1)?);
            put("pitch", g("pitch_nm")?);
            put("body_length", g("body_length_nm")?);
            put("body_width", g("body_width_nm")?);
            put("body_height", g("body_height_nm")?);
            put("standoff", g("standoff_nm")?);
            put("span", pair("span_nm", 0)?);
            put("span_x", pair("span_nm", 1)?);
            put("lead_width", g("lead_width_nm")?);
            put("lead_thickness", g("lead_thickness_nm")?);
            put("foot_length", g("foot_length_nm")?);
            put(
                "sides",
                text(if form["sides"] == "four" {
                    "four"
                } else {
                    "two"
                }),
            );
            "gull-wing"
        }
        "tht" => {
            let kind = &form["kind"];
            let construction = kind["construction"].as_str().unwrap_or("");
            put("construction", text(construction));
            if form.get("standoff_nm").is_some_and(|v| !v.is_null()) {
                put("standoff_basis", text("given"));
                put("standoff", g("standoff_nm")?);
            }
            put("lead_pitch", g("lead_pitch_nm")?);
            put("lead_diameter", g("lead_diameter_nm")?);
            put("lead_below", g("lead_below_nm")?);
            if form.get("bend_radius_nm").is_some_and(|v| !v.is_null()) {
                put("bend_radius", g("bend_radius_nm")?);
            }
            if form.get("body_pitch_nm").is_some_and(|v| !v.is_null()) {
                put("body_pitch", g("body_pitch_nm")?);
            }
            let k = |key: &str| get(kind, key).map(ParamValue::Int);
            match construction {
                "axial" => {
                    put("pins", ParamValue::Int(2));
                    put("body_diameter", k("body_diameter_nm")?);
                    put("body_length", k("body_length_nm")?);
                }
                "radial" => {
                    put("pins", ParamValue::Int(2));
                    put("body_diameter", k("body_diameter_nm")?);
                    put("body_height", k("body_height_nm")?);
                }
                "dip" => {
                    put("pins", k("pins")?);
                    put("row_pitch", k("row_pitch_nm")?);
                    put("body_length", k("body_length_nm")?);
                    put("body_width", k("body_width_nm")?);
                    put("body_height", k("body_height_nm")?);
                }
                "to" => {
                    put("pins", k("pins")?);
                    put("body_length", k("body_length_nm")?);
                    put("body_width", k("body_width_nm")?);
                    put("body_height", k("body_height_nm")?);
                }
                other => return Err(format!("a through-hole form of construction {other:?}")),
            }
            "tht"
        }
        "tabbed" => {
            put("leads_s", pair("leads_per_side", 0)?);
            put("leads_n", pair("leads_per_side", 1)?);
            let step = |i: usize| ParamValue::Int(int(&form["lead_step"][i]).unwrap_or(1));
            put("step_s", step(0));
            put("step_n", step(1));
            put("pitch", g("pitch_nm")?);
            put("body_length", g("body_length_nm")?);
            put("body_width", g("body_width_nm")?);
            put("body_height", g("body_height_nm")?);
            put("standoff", g("standoff_nm")?);
            put("span", g("span_nm")?);
            put("lead_width", g("lead_width_nm")?);
            put("lead_thickness", g("lead_thickness_nm")?);
            put("foot_length", g("foot_length_nm")?);
            let tab = &form["tab"];
            let kind = if tab.is_null() {
                "none"
            } else {
                put("tab_width", ParamValue::Int(get(tab, "width_nm")?));
                put("tab_length", ParamValue::Int(get(tab, "length_nm")?));
                match int(&tab["flange_nm"]) {
                    None => "lead",
                    Some(flange) => {
                        put("flange", ParamValue::Int(flange));
                        "flange"
                    }
                }
            };
            put("tab", text(kind));
            let lands = &form["lands"];
            if lands.is_object() && lands.as_object().is_some_and(|l| !l.is_empty()) {
                put("lands", text("explicit"));
                if let Some(size) = lands.get("size") {
                    put("land_width", ParamValue::Int(get(size, "x_nm")?));
                    put("land_length", ParamValue::Int(get(size, "y_nm")?));
                }
                if let Some(span) = int(&lands["span_nm"]) {
                    put("land_span", ParamValue::Int(span));
                }
                if let Some(size) = lands.get("tab_size") {
                    put("tab_land_width", ParamValue::Int(get(size, "x_nm")?));
                    put("tab_land_length", ParamValue::Int(get(size, "y_nm")?));
                }
            }
            "tabbed"
        }
        "molded" => {
            put("body_length", g("body_length_nm")?);
            put("body_width", g("body_width_nm")?);
            put("body_height", g("body_height_nm")?);
            put("span", g("span_nm")?);
            put("terminal_length", g("terminal_length_nm")?);
            put("terminal_width", g("terminal_width_nm")?);
            put("lead", text(form["lead"].as_str().unwrap_or("flat")));
            put("polarised", ParamValue::Bool(form["polarised"] == true));
            // The package alone: what the part is leaves the model as it is.
            put("part", text("diode"));
            "molded"
        }
        "no_lead" => {
            put(
                "sides",
                text(if form["sides"] == "four" {
                    "four"
                } else {
                    "two"
                }),
            );
            put("terminals", pair("terminals_per_side", 0)?);
            put("terminals_short", pair("terminals_per_side", 1)?);
            put("pitch", g("pitch_nm")?);
            put("body_length", g("body_length_nm")?);
            put("body_width", g("body_width_nm")?);
            put("body_height", g("body_height_nm")?);
            put("terminal_length", g("terminal_length_nm")?);
            put("terminal_width", g("terminal_width_nm")?);
            let pad = &form["exposed_pad"];
            if !pad.is_null() {
                put("pad_length", ParamValue::Int(get(pad, "x_nm")?));
                put("pad_width", ParamValue::Int(get(pad, "y_nm")?));
            }
            put("exposed_pad", ParamValue::Bool(!pad.is_null()));
            "no-lead"
        }
        "grid" => {
            put(
                "contact",
                text(if form["contact"] == "land" {
                    "land"
                } else {
                    "ball"
                }),
            );
            put("rows", g("rows")?);
            put("columns", g("cols")?);
            put("pitch", ParamValue::Int(get(&form["pitch"], "x_nm")?));
            put("row_pitch", ParamValue::Int(get(&form["pitch"], "y_nm")?));
            put("body_length", g("body_length_nm")?);
            put("body_width", g("body_width_nm")?);
            put("body_height", g("body_height_nm")?);
            put("contact_diameter", g("contact_diameter_nm")?);
            put("contact_height", g("contact_height_nm")?);
            let letters: Vec<char> = form["row_letters"].as_str().unwrap_or("").chars().collect();
            let row_name = |row: u64| -> Option<String> {
                let n = letters.len();
                if n == 0 {
                    return None;
                }
                let mut i = usize::try_from(row).ok()?;
                let mut out = Vec::new();
                loop {
                    out.push(*letters.get(i % n)?);
                    if i < n {
                        break;
                    }
                    i = i / n - 1;
                }
                Some(out.iter().rev().collect())
            };
            let empty = form["depopulated"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|rc| {
                    let (r, c) = (rc[0].as_u64()?, rc[1].as_u64()?);
                    Some(ParamValue::Text(format!("{}{}", row_name(r)?, c + 1)))
                })
                .collect();
            put("depopulated", ParamValue::List(empty));
            put(
                "row_letters",
                text(form["row_letters"].as_str().unwrap_or("")),
            );
            "grid"
        }
        "module" => {
            let point =
                |v: &Value, k: &str| int(&v[k]).ok_or_else(|| format!("the form has no {k}"));
            put("width", g("width_nm")?);
            put("length", g("length_nm")?);
            put("height", g("height_nm")?);
            put("board_thickness", g("pcb_thickness_nm")?);
            put("antenna_length", g("antenna_length_nm")?);
            put(
                "can_width",
                ParamValue::Int(point(&form["can_size"], "x_nm")?),
            );
            put(
                "can_length",
                ParamValue::Int(point(&form["can_size"], "y_nm")?),
            );
            put(
                "can_x",
                ParamValue::Int(point(&form["can_centre"], "x_nm")?),
            );
            put(
                "can_y",
                ParamValue::Int(point(&form["can_centre"], "y_nm")?),
            );
            put("pitch", g("pitch_nm")?);
            put("pad_length", pair("pad_size_nm", 0)?);
            put("pad_width", pair("pad_size_nm", 1)?);
            put(
                "castellation",
                ParamValue::Int(int(&form["castellation_nm"]).unwrap_or(0)),
            );
            put("land_width", g("land_width_nm")?);
            put(
                "land_outer",
                ParamValue::Int(int(&form["land_extend_nm"][0]).unwrap_or(0)),
            );
            put(
                "land_inner",
                ParamValue::Int(int(&form["land_extend_nm"][1]).unwrap_or(0)),
            );
            let rows: Vec<&Value> = form["rows"].as_array().into_iter().flatten().collect();
            let list = |f: &dyn Fn(&Value) -> Option<ParamValue>,
                        items: &[&Value]|
             -> std::result::Result<ParamValue, String> {
                items
                    .iter()
                    .map(|v| f(v).ok_or_else(|| "a module row or pad is incomplete".to_owned()))
                    .collect::<std::result::Result<Vec<_>, _>>()
                    .map(ParamValue::List)
            };
            put("row_sides", list(&|r| r["side"].as_str().map(text), &rows)?);
            put(
                "row_counts",
                list(&|r| int(&r["count"]).map(ParamValue::Int), &rows)?,
            );
            put(
                "row_x",
                list(&|r| int(&r["first"]["x_nm"]).map(ParamValue::Int), &rows)?,
            );
            put(
                "row_y",
                list(&|r| int(&r["first"]["y_nm"]).map(ParamValue::Int), &rows)?,
            );
            let ground = &form["ground"];
            put("ground", ParamValue::Bool(ground.is_object()));
            if ground.is_object() {
                put("ground_rows", ParamValue::Int(get(ground, "rows")?));
                put("ground_columns", ParamValue::Int(get(ground, "cols")?));
                put(
                    "ground_pitch_x",
                    ParamValue::Int(point(&ground["pitch"], "x_nm")?),
                );
                put(
                    "ground_pitch_y",
                    ParamValue::Int(point(&ground["pitch"], "y_nm")?),
                );
                put(
                    "ground_size_x",
                    ParamValue::Int(point(&ground["size"], "x_nm")?),
                );
                put(
                    "ground_size_y",
                    ParamValue::Int(point(&ground["size"], "y_nm")?),
                );
                put(
                    "ground_x",
                    ParamValue::Int(point(&ground["centre"], "x_nm")?),
                );
                put(
                    "ground_y",
                    ParamValue::Int(point(&ground["centre"], "y_nm")?),
                );
                put(
                    "ground_rotation",
                    ParamValue::Int(int(&ground["rotation_mdeg"]).unwrap_or(0)),
                );
                put(
                    "ground_number",
                    text(ground["number"].as_str().unwrap_or("").trim()),
                );
            }
            let singles: Vec<&Value> = form["singles"].as_array().into_iter().flatten().collect();
            put(
                "single_numbers",
                list(&|s| s["number"].as_str().map(|n| text(n.trim())), &singles)?,
            );
            put(
                "single_x",
                list(
                    &|s| int(&s["centre"]["x_nm"]).map(ParamValue::Int),
                    &singles,
                )?,
            );
            put(
                "single_y",
                list(
                    &|s| int(&s["centre"]["y_nm"]).map(ParamValue::Int),
                    &singles,
                )?,
            );
            put(
                "single_length",
                list(&|s| int(&s["size"]["x_nm"]).map(ParamValue::Int), &singles)?,
            );
            put(
                "single_width",
                list(&|s| int(&s["size"]["y_nm"]).map(ParamValue::Int), &singles)?,
            );
            let keepout = &form["keepout"];
            put("keepout", ParamValue::Bool(keepout.is_object()));
            if keepout.is_object() {
                let beyond = &keepout["beyond_nm"];
                put(
                    "keepout_left",
                    ParamValue::Int(int(&beyond[0]).unwrap_or(0)),
                );
                put(
                    "keepout_right",
                    ParamValue::Int(int(&beyond[1]).unwrap_or(0)),
                );
                put("keepout_top", ParamValue::Int(int(&beyond[2]).unwrap_or(0)));
            }
            "module"
        }
        _ => return Ok(None),
    };
    Ok(Some((name, p)))
}

/// A stored USB-C form's lead contacts in nanometres, in terminal order:
/// each contact land's centre, then the rear −x stake for the shell (the
/// form's own lead points).
fn usb_c_leads(form: &Value) -> std::result::Result<Vec<[i64; 2]>, String> {
    let pitch = get(form, "pitch_nm")?;
    let land_y = get(form, "land_y_nm")?;
    let vbus = get(form, "vbus_x_nm")?;
    let gnd = get(form, "gnd_x_nm")?;
    let tab_x = get(form, "tab_x_nm")?;
    let rear_y = get(&form["tabs"][1], "y_nm")?;
    let mut xs = vec![-gnd, -vbus];
    // (k − 3.5) × pitch, exact in nanometres for an even pitch.
    xs.extend((0..8).map(|k: i64| ((2 * k - 7) * pitch) / 2));
    xs.extend([vbus, gnd]);
    let mut out: Vec<[i64; 2]> = xs.into_iter().map(|x| [x, land_y]).collect();
    out.push([-tab_x, rear_y]);
    Ok(out)
}

/// A chip's lead contacts: each termination's centre.
fn chip_leads(form: &Value) -> std::result::Result<Vec<[i64; 2]>, String> {
    let length = get(form, "length_nm")?;
    let termination = get(form, "termination_nm")?;
    let x = length / 2 - termination / 2;
    Ok(vec![[-x, 0], [x, 0]])
}

/// The package's material for a body role, when its generated model chose
/// none: the family's default (the former `default_material`).
fn default_material(family: &str, role: BodyRole) -> MaterialId {
    match (family, role) {
        ("module", BodyRole::Body) => MaterialId::SolderMaskGreen,
        ("module", BodyRole::Other) => MaterialId::NickelSilver,
        ("module", BodyRole::Lead | BodyRole::Termination | BodyRole::ExposedPad) => {
            MaterialId::Gold
        }
        ("usb_c", BodyRole::Body) => MaterialId::Lcp,
        ("usb_c", BodyRole::Other) => MaterialId::Nickel,
        ("usb_c", BodyRole::Lead | BodyRole::Termination | BodyRole::ExposedPad) => {
            MaterialId::Gold
        }
        ("header", BodyRole::Body) => MaterialId::Pbt,
        ("header", BodyRole::Lead) => MaterialId::Gold,
        ("gull_wing", BodyRole::Lead) => MaterialId::TinBright,
        (_, BodyRole::Lead | BodyRole::Termination | BodyRole::ExposedPad) => MaterialId::TinMatte,
        (_, BodyRole::Marking) => MaterialId::EpoxyLaserMarked,
        (_, BodyRole::Lens) => MaterialId::SiliconeClear,
        (_, BodyRole::Sleeve) => MaterialId::PvcSleeve,
        (_, BodyRole::Body | BodyRole::Other) => MaterialId::EpoxyMould,
    }
}

/// The role the retired builder gave a part file's body (the former
/// `builder_role`): it chose the package's material.
fn builder_role(form: &Value, name: &str, role: BodyRole) -> BodyRole {
    let family = form["family"].as_str().unwrap_or("");
    match (family, role) {
        ("header", BodyRole::Termination) => BodyRole::Lead,
        ("grid", BodyRole::Termination) if form["contact"] != "land" => BodyRole::Lead,
        ("molded", BodyRole::Termination) if form["lead"] != "melf" => BodyRole::Lead,
        ("no_lead", BodyRole::Termination) if name == "exposed pad" => BodyRole::ExposedPad,
        ("module", BodyRole::Termination) if name.starts_with("ground pad") => BodyRole::ExposedPad,
        ("module", _) if name == "shield can" || name == "antenna" => BodyRole::Other,
        _ => role,
    }
}

/// The roles the retired USB-C builder gave the shipped part file's bodies
/// (drawn as, material chosen by).
fn usb_c_roles(name: &str) -> (BodyRole, BodyRole) {
    if name == "shell" {
        (BodyRole::Other, BodyRole::Other)
    } else if name.starts_with("stake ") {
        (BodyRole::Termination, BodyRole::Other)
    } else if name.starts_with("tail ") {
        (BodyRole::Termination, BodyRole::Lead)
    } else {
        (BodyRole::Body, BodyRole::Body)
    }
}

/// Whether a body of `role` draws as a lead (`BodyKind::ModelLead`).
fn lead_like(role: BodyRole) -> bool {
    matches!(
        role,
        BodyRole::Lead | BodyRole::Termination | BodyRole::ExposedPad
    )
}

fn mm(nm: i64) -> f64 {
    nm as f64 / 1e6
}

/// The Model a bundled part file becomes (as `library.generate` makes it).
fn definition(
    hash: ContentHash,
    bytes: &[u8],
    file: &str,
    licence: &str,
    as_is: &part_file::EvaluatedPart,
) -> Result<ModelDefinition> {
    Ok(ModelDefinition {
        asset: AssetRef {
            hash,
            bytes: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
            media_type: ModelFormat::Solidcraft.media_type().into(),
            file_name: file.to_owned(),
        },
        format: ModelFormat::Solidcraft,
        provenance: ModelProvenance {
            redistribution: Redistribution::Permitted,
            record: Provenance {
                origin: None,
                basis: Basis::Authored,
                licence: Some(licence.to_owned()),
            },
            ..ModelProvenance::default()
        },
        bounds_nm: sparksmith_core::library::model3d::bounds_nm(as_is.bounds_mm)
            .map_err(|e| Error::Invalid(e.to_string()))?,
        ..ModelDefinition::default()
    })
}

/// What one stored form gives its package: the lead contacts and, for a
/// generated model (`model`'s `source`), the part-file model, its bodies
/// compared with what the board built from the form.
pub fn package(
    context: &Context,
    info: &dyn Fn(&str, ContentHash) -> Option<PartInfo>,
    form: &Value,
    model: &Value,
    envelope: &Value,
    terminals: &[String],
    markings: &[Value],
) -> std::result::Result<Retired, String> {
    let mut out = converted(context, info, form, model, terminals)?;
    let source = model["source"].as_str().unwrap_or("");
    let surfaces = out.model.as_ref().map(|m| m.derived.surfaces.as_slice());
    out.refusals
        .extend(marking_refusals(form, envelope, source, surfaces, markings));
    Ok(out)
}

/// The marking area a version 4 package's form gave (`marking::package_area`
/// then): a chip between its terminations, a gull-wing or no-lead body's
/// moulding, a module's can, else the envelope's length and width.
fn form_area(form: &Value, envelope: &Value) -> Option<[f64; 2]> {
    let g = |v: &Value| v.as_i64().map(mm);
    let (l, w) = match form["family"].as_str()? {
        "chip" => (
            mm(get(form, "length_nm").ok()? - 2 * get(form, "termination_nm").ok()?),
            g(&form["width_nm"])?,
        ),
        "gull_wing" | "no_lead" => (g(&form["body_length_nm"])?, g(&form["body_width_nm"])?),
        "module" => (g(&form["can_size"]["x_nm"])?, g(&form["can_size"]["y_nm"])?),
        _ => (g(&envelope[0])?, g(&envelope[1])?),
    };
    (l > 0.0 && w > 0.0).then_some([l, w])
}

/// What converting loses of the markings on a package: a spec laid out on
/// the model's area that the converted package sizes otherwise, or on a
/// named anchor of the retired form (the part file's surfaces are other
/// geometry, planes only). Box faces follow the envelope, which stays.
fn marking_refusals(
    form: &Value,
    envelope: &Value,
    source: &str,
    surfaces: Option<&[sparksmith_core::library::marking::surface::AnchorInfo]>,
    markings: &[Value],
) -> Vec<String> {
    use sparksmith_core::library::marking::surface::SurfaceGeometry;
    // A part file's or a library model's surfaces were their own already.
    if matches!(source, "part_file" | "library") {
        return Vec::new();
    }
    let before = form_area(form, envelope);
    let after = match surfaces {
        Some(surfaces) => surfaces.iter().find_map(|s| match &s.geometry {
            SurfaceGeometry::Plane(a) => Some(a.size()),
            SurfaceGeometry::Cylinder(_) => None,
        }),
        None => envelope[0]
            .as_i64()
            .zip(envelope[1].as_i64())
            .map(|(l, w)| [mm(l), mm(w)]),
    };
    let mut specs: Vec<&Value> = markings.iter().collect();
    let mut out = Vec::new();
    while let Some(spec) = specs.pop() {
        specs.extend(spec["decals"].as_array().into_iter().flatten());
        let on = spec["placement"]["on"]["surface"]
            .as_str()
            .unwrap_or("model");
        let area = spec["area"]["area"].as_str().unwrap_or("from_model");
        match on {
            "model" if area == "from_model" => {
                let same = match (before, after) {
                    (Some(a), Some(b)) => (0..2).all(|k| (a[k] - b[k]).abs() <= 1e-6),
                    (None, None) => true,
                    _ => false,
                };
                if !same {
                    out.push(format!(
                        "a marking is laid out on the model's area, {before:?} mm from its form, which the converted package gives as {after:?} mm"
                    ));
                }
            }
            "anchor" => out.push(format!(
                "a marking is laid out on the form's surface {}, which the part file does not offer alike",
                spec["placement"]["on"]["name"].as_str().unwrap_or("")
            )),
            _ => {}
        }
    }
    out
}

fn converted(
    context: &Context,
    info: &dyn Fn(&str, ContentHash) -> Option<PartInfo>,
    form: &Value,
    model: &Value,
    terminals: &[String],
) -> std::result::Result<Retired, String> {
    let family = form["family"].as_str().unwrap_or("").to_owned();
    let generated = model["source"] == "generated";
    let mut out = Retired::default();
    if family == "usb_c" {
        let leads = usb_c_leads(form)?;
        if leads.len() == terminals.len() {
            out.leads = leads;
        }
        if generated {
            usb_c_model(context, info, form, terminals, &mut out)?;
        }
        return Ok(out);
    }
    if family == "module" && int(&form["castellation_nm"]).is_some_and(|c| c > 0) {
        out.refusals.push(
            "its castellated module form placed each lead contact at the pad's centre; its generator places it half a castellation in from the edge"
                .into(),
        );
        return Ok(out);
    }
    let Some((name, params)) = generator(form)? else {
        return Err(format!("no shipped generator makes {family} packages"));
    };
    let shipped = context.generator(name).map_err(|e| e.to_string())?;
    let fail = |f: Failure| format!("the {name} generator: {f}");
    let runtime = WasmiRuntime::new(Limits::DEFAULT);
    let description = runtime.describe(&shipped.module).map_err(fail)?;
    let request = complete(&description, Scope::Package, &params).map_err(fail)?;
    let generation = runtime
        .generate(&shipped.module, &request)
        .map_err(fail)?
        .generation;
    let leads = &generation.package.geometry.leads;
    let exact = leads.len() == terminals.len();
    if family == "chip" {
        out.leads = chip_leads(form)?;
    } else if exact {
        out.leads = leads.iter().map(|l| [l.contact[0], l.contact[1]]).collect();
    }
    if !generated {
        return Ok(out);
    }
    let Some(Model::PartFile { asset, values }) = &generation.model else {
        return Err(format!("the {name} generator made no 3D model"));
    };
    let hash = ContentHash::parse(asset).ok_or_else(|| format!("\"{asset}\" is not a SHA-256"))?;
    let bytes = shipped
        .part(hash)
        .ok_or_else(|| format!("the {name} bundle has no part file sha256 {asset}"))?;
    let as_is = part_file::evaluate(bytes, &PartValues::new()).map_err(|e| e.to_string())?;
    let values = part_file::generator_values(&as_is, values)?;
    let part = part_file::evaluate(bytes, &values).map_err(|e| e.to_string())?;
    if let Some(refusal) = part.refusal() {
        return Err(format!(
            "its part file at the generator's values: {refusal}"
        ));
    }
    // Lead k is the package's k-th terminal when it lists one per lead.
    let contacts: Vec<(String, [f64; 2])> = leads
        .iter()
        .zip(terminals)
        .filter(|_| exact)
        .map(|(lead, t)| (t.clone(), [mm(lead.contact[0]), mm(lead.contact[1])]))
        .collect();
    let mut bound = part_file::lead_bodies(&part, &contacts, true)?;
    let terminal_of = |key: &str| {
        generation
            .package
            .terminals
            .iter()
            .position(|k| k == key)
            .and_then(|i| terminals.get(i))
            .filter(|_| exact)
            .cloned()
    };
    let lands: Vec<(String, [f64; 2])> = generation
        .footprint
        .pads
        .iter()
        .filter_map(|p| Some((terminal_of(&p.terminal)?, [mm(p.at[0]), mm(p.at[1])])))
        .collect();
    for lead in part_file::lead_bodies(&part, &lands, false)? {
        if !bound.iter().any(|l| l.body == lead.body) {
            bound.push(lead);
        }
    }
    bound.sort_by(|a, b| a.body.cmp(&b.body));
    if family == "chip" {
        out.notes.push(format!(
            "its chip model, built in code until now, is the chip generator's part file: the same bodies, roles and materials within 1 µm (its faces have other names); {} bodies",
            part.bodies.len()
        ));
    } else {
        let module = family == "module";
        for b in &part.bodies {
            let role = part_file::role_of(b.material);
            let builder = builder_role(form, &b.name, role);
            let own = module && b.name == "antenna";
            let before_material = b
                .material
                .unwrap_or_else(|| default_material(&family, if own { role } else { builder }));
            let before_role = if builder == BodyRole::ExposedPad || module {
                builder
            } else {
                role
            };
            let after_material = b.material.unwrap_or(MaterialId::EpoxyMould);
            // A role label alone is no loss: the body draws as the same kind
            // (lead or body) in the same material.
            if lead_like(before_role) != lead_like(role) || before_material != after_material {
                out.refusals.push(format!(
                    "its body {} was drawn as {before_role:?} in {before_material:?}; its part file makes it {role:?} in {after_material:?}",
                    b.name
                ));
            }
        }
    }
    let info = info(name, hash)
        .ok_or_else(|| format!("the {name} bundle names no part file sha256 {asset}"))?;
    out.model = Some(RetiredModel {
        definition: definition(hash, bytes, &info.file, &info.licence, &as_is)
            .map_err(|e| e.to_string())?,
        name: info.title,
        bytes: bytes.to_vec(),
        values,
        derived: PartDerived {
            leads: bound,
            ..part_file::derived(&part).map_err(|e| e.to_string())?
        },
    });
    Ok(out)
}

/// A stored USB-C form's generated model: the shipped library part file at
/// the form's values, compared as the other families are.
fn usb_c_model(
    context: &Context,
    info: &dyn Fn(&str, ContentHash) -> Option<PartInfo>,
    form: &Value,
    terminals: &[String],
    out: &mut Retired,
) -> std::result::Result<(), String> {
    const PART: &str = "gct-usb4105";
    let shipped = context.part(PART).map_err(|e| e.to_string())?;
    let mut values = PartValues::new();
    let tab = |i: usize, k: &str| int(&form["tabs"][i][k]);
    for (name, nm) in [
        ("width", int(&form["width_nm"])),
        ("depth", int(&form["depth_nm"])),
        ("height", int(&form["height_nm"])),
        ("opening_w", int(&form["opening_nm"][0])),
        ("opening_h", int(&form["opening_nm"][1])),
        ("opening_z", int(&form["opening_z_nm"])),
        ("shell_t", int(&form["shell_thickness_nm"])),
        ("pitch", int(&form["pitch_nm"])),
        ("land_y", int(&form["land_y_nm"])),
        ("land_length", int(&form["land_length_nm"])),
        ("signal_land_w", int(&form["signal_land_width_nm"])),
        ("power_land_w", int(&form["power_land_width_nm"])),
        ("vbus_x", int(&form["vbus_x_nm"])),
        ("gnd_x", int(&form["gnd_x_nm"])),
        ("front_y", tab(0, "y_nm")),
        ("front_slot", int(&form["tabs"][0]["slot"]["y_nm"])),
        ("rear_y", tab(1, "y_nm")),
        ("rear_slot", int(&form["tabs"][1]["slot"]["y_nm"])),
        ("stake_length", int(&form["stake_length_nm"])),
        ("peg_x", int(&form["peg_x_nm"])),
        ("peg_y", int(&form["peg_y_nm"])),
        ("peg_d", int(&form["peg_diameter_nm"])),
    ] {
        let nm = nm.ok_or_else(|| format!("the USB-C form has no value for {name}"))?;
        values.insert(name.into(), PartValue::Nm(nm));
    }
    let part = part_file::evaluate(&shipped.bytes, &values).map_err(|e| e.to_string())?;
    if let Some(refusal) = part.refusal() {
        return Err(format!(
            "the shipped USB-C part file at the form's values: {refusal}"
        ));
    }
    for b in &part.bodies {
        let (role, material_role) = usb_c_roles(&b.name);
        let before = b
            .material
            .unwrap_or_else(|| default_material("usb_c", material_role));
        let after_role = part_file::role_of(b.material);
        let after = b.material.unwrap_or(MaterialId::EpoxyMould);
        if lead_like(role) != lead_like(after_role) || before != after {
            out.refusals.push(format!(
                "its body {} was drawn as {role:?} in {before:?}; the part file makes it {after_role:?} in {after:?}",
                b.name
            ));
        }
    }
    // The lead bodies as the builder bound them: each contact land's tail
    // to its terminal, the four stakes to the shell.
    let land_y = mm(get(form, "land_y_nm")?);
    let mut contacts: Vec<(String, [f64; 2])> = Vec::new();
    let leads = usb_c_leads(form)?;
    if terminals.len() == leads.len() {
        for (lead, terminal) in leads.iter().take(12).zip(terminals) {
            contacts.push((terminal.clone(), [mm(lead[0]), land_y]));
        }
        if let Some(shell) = terminals.last() {
            let tab_x = mm(get(form, "tab_x_nm")?);
            for i in 0..2 {
                let y = mm(get(&form["tabs"][i], "y_nm")?);
                contacts.push((shell.clone(), [-tab_x, y]));
                contacts.push((shell.clone(), [tab_x, y]));
            }
        }
    }
    let mut bound: Vec<LeadBody> = part_file::lead_bodies(&part, &contacts, false)?;
    bound.sort_by(|a, b| a.body.cmp(&b.body));
    let hash = sparksmith_core::library::bundle::asset_hash(&shipped.bytes);
    let info = info(PART, hash).unwrap_or(PartInfo {
        file: format!("{PART}.solidcraft"),
        title: PART.into(),
        licence: "MIT OR Apache-2.0".into(),
    });
    out.model = Some(RetiredModel {
        definition: definition(hash, &shipped.bytes, &info.file, &info.licence, &part)
            .map_err(|e| e.to_string())?,
        name: info.title,
        bytes: shipped.bytes.to_vec(),
        values,
        derived: PartDerived {
            leads: bound,
            ..part_file::derived(&part).map_err(|e| e.to_string())?
        },
    });
    Ok(())
}

/// A package row's typed values (version 4) as its generator and
/// parameters, as JSON.
pub fn row(values: &Value) -> std::result::Result<(String, Value), String> {
    let Some((name, params)) = generator(values)? else {
        return Err(format!(
            "no shipped generator makes {} rows",
            values["family"].as_str().unwrap_or("these")
        ));
    };
    let params = serde_json::to_value(params).map_err(|e| e.to_string())?;
    Ok((name.to_owned(), params))
}
