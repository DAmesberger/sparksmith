//! Step 4 to 5: the in-code package templates are gone (part-generation
//! slice D2, docs/design/part-generation-plan.md).
//!
//! - **A package's typed form leaves the file.** What it gave beyond the
//!   envelope (already stored) becomes plain data: the lead contacts, one
//!   per terminal, as the family's shipped generator places them (equal to
//!   the form's own lead points for every starter row and preset, measured
//!   when the forms were retired).
//! - **A generated model becomes its generator's part file.** The family's
//!   shipped generator runs on the form's values (package scope); its part
//!   file, at the values it gives, becomes a Model of the project (bytes
//!   packed) and the package's `part_file` model, with what it evaluates to
//!   and its lead bodies bound to the package's terminals as the board
//!   built them. A chip, whose model was still built in code, takes the
//!   chip generator's part file: the same bodies, roles and materials
//!   within 1 µm, but other face names.
//! - **A package row names its generator** and gives the generator's
//!   parameters instead of a typed form.
//!
//! Evaluating a part file needs Solidcraft, which `sparksmith-io` does not
//! depend on: the host that converts (`sparksmith convert`) passes a
//! [`Host`] in [`Options::host`](super::Options). A file that needs one and
//! gets none is refused.
//!
//! Nothing is dropped silently: a file holding anything the conversion
//! would lose is refused, naming each package and why: a material choice
//! on a generated model, a body the part file draws otherwise (another
//! material, or a lead where there was a body; a role label alone, with
//! the same drawing and material, is kept as the part file names it), a
//! marking whose surface the converted package does not offer alike, a
//! sketch footprint that reads a form's `pkg_*` value, a dimension
//! to a face of a chip's model (its face names change), and a package row
//! with land overrides or a symbol or skeleton suggestion.
use crate::convert::{Options, Result};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};

/// What converting one typed-form package needs from the host.
#[derive(Clone, Debug)]
pub struct PackageRequest<'a> {
    /// The package's name, for messages.
    pub name: &'a str,
    /// The version 4 form: tagged by `family`, lengths in `_nm`.
    pub form: &'a Value,
    /// The version 4 model (`source` tagged).
    pub model: &'a Value,
    /// The package's envelope (`[l, w, h]` nm).
    pub envelope: &'a Value,
    /// The package terminals' designations, in order.
    pub terminals: Vec<String>,
    /// The marking specs of every part on the package (a component's
    /// marking, a Part's top mark, an instance's replacing spec): the host
    /// refuses the conversion when one would lose the surface it is laid
    /// out on.
    pub markings: Vec<Value>,
}

/// A generated model as its generator's part file.
#[derive(Clone, Debug)]
pub struct ConvertedModel {
    /// The Model the part file becomes (`ModelDefinition` as JSON).
    pub definition: Value,
    /// That Model's name.
    pub name: String,
    /// The part file's bytes.
    pub bytes: Vec<u8>,
    /// The package's values for it (`{"name": {"nm": …}}`).
    pub values: Value,
    /// What it evaluates to (`PartDerived` as JSON), lead bodies bound.
    pub derived: Value,
}

/// What the host gives one typed-form package.
#[derive(Clone, Debug, Default)]
pub struct PackageReply {
    /// Each terminal's lead contact `[x, y]` in nanometres, in terminal
    /// order; empty when the form gives none for this package.
    pub leads: Vec<[i64; 2]>,
    /// The model, when the stored model was a generated one.
    pub model: Option<ConvertedModel>,
    /// What the conversion would lose; the file is refused when any.
    pub refusals: Vec<String>,
    /// Accepted differences, said to the person converting.
    pub notes: Vec<String>,
}

/// The parts of the conversion that run the shipped generators and
/// evaluate part files (`sparksmith convert` gives one).
pub trait Host: Send + Sync {
    /// The lead contacts and, for a generated model, the part-file model
    /// of one typed-form package.
    fn package(&self, request: &PackageRequest<'_>) -> std::result::Result<PackageReply, String>;
    /// A package row's typed values as its generator and parameters.
    fn row(&self, values: &Value) -> std::result::Result<(String, Value), String>;
}

/// A refusal, collected for the whole file.
struct Refusals(Vec<String>);

impl Refusals {
    fn add(&mut self, what: &str, why: impl Into<String>) {
        self.0.push(format!("{what}: {}", why.into()));
    }
}

/// One package definition the step converts: where it lives and its body.
struct Site {
    /// The lineage id.
    id: u64,
    name: String,
    /// `project.library.packages[i].definition` or `drafts.items[i].definition.body`.
    path: Vec<PathPart>,
}

#[derive(Clone)]
enum PathPart {
    Key(&'static str),
    Index(usize),
}

fn at<'a>(root: &'a mut Value, path: &[PathPart]) -> Option<&'a mut Value> {
    let mut v = root;
    for p in path {
        v = match p {
            PathPart::Key(k) => v.get_mut(*k)?,
            PathPart::Index(i) => v.get_mut(*i)?,
        };
    }
    Some(v)
}

fn at_ref<'a>(root: &'a Value, path: &[PathPart]) -> Option<&'a Value> {
    let mut v = root;
    for p in path {
        v = match p {
            PathPart::Key(k) => v.get(*k)?,
            PathPart::Index(i) => v.get(*i)?,
        };
    }
    Some(v)
}

/// Every package (published revision or draft) and row of `file`.
fn sites(file: &Value, kind: &str) -> Vec<Site> {
    let mut out = Vec::new();
    let collection = match kind {
        "package" => "packages",
        _ => "package_rows",
    };
    if let Some(items) = file
        .pointer(&format!("/project/library/{collection}"))
        .and_then(Value::as_array)
    {
        for (i, item) in items.iter().enumerate() {
            out.push(Site {
                id: item["id"].as_u64().unwrap_or(0),
                name: format!(
                    "{} ‘{}’ r{}",
                    if kind == "package" {
                        "package"
                    } else {
                        "package row"
                    },
                    item["meta"]["name"].as_str().unwrap_or(""),
                    item["revision"].as_u64().unwrap_or(0)
                ),
                path: vec![
                    PathPart::Key("project"),
                    PathPart::Key("library"),
                    PathPart::Key(if kind == "package" {
                        "packages"
                    } else {
                        "package_rows"
                    }),
                    PathPart::Index(i),
                    PathPart::Key("definition"),
                ],
            });
        }
    }
    if let Some(items) = file.pointer("/drafts/items").and_then(Value::as_array) {
        for (i, item) in items.iter().enumerate() {
            if item["definition"]["kind"] != kind {
                continue;
            }
            out.push(Site {
                id: item["id"].as_u64().unwrap_or(0),
                name: format!(
                    "{} draft ‘{}’",
                    if kind == "package" {
                        "package"
                    } else {
                        "package row"
                    },
                    item["meta"]["name"].as_str().unwrap_or("")
                ),
                path: vec![
                    PathPart::Key("drafts"),
                    PathPart::Key("items"),
                    PathPart::Index(i),
                    PathPart::Key("definition"),
                    PathPart::Key("body"),
                ],
            });
        }
    }
    out
}

/// Every definition body of `kind` (published and drafts) as
/// `(lineage, body)`.
fn bodies<'a>(file: &'a Value, kind: &str) -> Vec<(u64, &'a Value)> {
    let collection = match kind {
        "component" => "components",
        "binding" => "bindings",
        "footprint" => "footprints",
        "part" => "parts",
        _ => return Vec::new(),
    };
    let mut out: Vec<(u64, &Value)> = file
        .pointer(&format!("/project/library/{collection}"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|item| (item["id"].as_u64().unwrap_or(0), &item["definition"]))
        .collect();
    out.extend(
        file.pointer("/drafts/items")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|item| item["definition"]["kind"] == kind)
            .map(|item| {
                (
                    item["id"].as_u64().unwrap_or(0),
                    &item["definition"]["body"],
                )
            }),
    );
    out
}

/// The `pkg_*` values a package still offers sketch footprints without its
/// form: the envelope and the terminal count.
const KEPT_PACKAGE_VALUES: &[&str] = &["pkg_L", "pkg_W", "pkg_H", "pkg_n"];

/// The `pkg_*` names `text` uses beyond [`KEPT_PACKAGE_VALUES`].
fn form_values_used(text: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while let Some(found) = text[i..].find("pkg_") {
        let start = i + found;
        let preceded =
            start > 0 && (bytes[start - 1].is_ascii_alphanumeric() || bytes[start - 1] == b'_');
        let mut end = start + 4;
        while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
            end += 1;
        }
        let name = &text[start..end];
        if !preceded && end > start + 4 && !KEPT_PACKAGE_VALUES.contains(&name) {
            out.insert(name.to_owned());
        }
        i = end;
    }
    out
}

/// The solder joints the realistic view drew from a form: a chip's
/// terminations, gull-wing and tabbed feet, a header's pins.
fn joints(form: &Value) -> Option<Value> {
    Some(match form["family"].as_str()? {
        "chip" => json!({"kind": "chip", "resistor": form["kind"] == "resistor"}),
        "gull_wing" | "tabbed" => json!({
            "kind": "feet",
            "length_nm": form["foot_length_nm"],
            "width_nm": form["lead_width_nm"],
            "thickness_nm": form["lead_thickness_nm"],
        }),
        "header" => json!({"kind": "pins", "side_nm": form["pin_side_nm"]}),
        _ => return None,
    })
}

/// Step 4 to 5 of a document; the number of stored values changed.
pub fn step(file: &mut Value, options: &Options) -> Result<usize> {
    let mut n = 0;
    let mut refusals = Refusals(Vec::new());
    let packages = sites(file, "package");
    let typed: Vec<&Site> = packages
        .iter()
        .filter(|s| {
            at_ref(file, &s.path)
                .and_then(|b| b.get("form"))
                .is_some_and(|f| f["family"] != "generic")
        })
        .collect();
    let host = options.host.as_ref().map(|h| h.0.as_ref());
    if !typed.is_empty() && host.is_none() {
        return Err(format!(
            "{} typed package form(s) need the shipped generators and Solidcraft to convert; run `{} convert`",
            typed.len(),
            sparksmith_core::product::SHORT_NAME
        )
        .into());
    }

    // What the checks read: which components each package lineage reaches
    // (through bindings), and the marked components and instances.
    let mut components_of: BTreeMap<u64, BTreeSet<u64>> = BTreeMap::new();
    for (_, binding) in bodies(file, "binding") {
        if let (Some(package), Some(component)) = (
            binding["package"]["id"].as_u64(),
            binding["component"].as_u64(),
        ) {
            components_of.entry(package).or_default().insert(component);
        }
    }
    // Each component's marking specs: its own, its Parts' top marks and
    // its instances' replacing specs.
    let mut markings_of: BTreeMap<u64, Vec<Value>> = BTreeMap::new();
    for (id, c) in bodies(file, "component") {
        if let Some(m) = c.get("marking").filter(|m| !m.is_null()) {
            markings_of.entry(id).or_default().push(m.clone());
        }
    }
    for (_, p) in bodies(file, "part") {
        if let (Some(m), Some(c)) = (
            p.get("top_mark").filter(|m| !m.is_null()),
            p["component"]["id"].as_u64(),
        ) {
            markings_of.entry(c).or_default().push(m.clone());
        }
    }
    for i in file
        .pointer("/project/instances")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let (Some(m), Some(c)) = (
            i.pointer("/marking/replace").filter(|m| !m.is_null()),
            i["component"]["id"].as_u64(),
        ) {
            markings_of.entry(c).or_default().push(m.clone());
        }
    }
    let footprints: Vec<(u64, String)> = bodies(file, "footprint")
        .into_iter()
        .filter(|(_, f)| f.get("source").is_some_and(|s| !s.is_null()))
        .filter_map(|(_, f)| {
            Some((
                f["package"]["id"].as_u64()?,
                serde_json::to_string(&f["source"]).ok()?,
            ))
        })
        .collect();
    let traced: BTreeSet<u64> = file
        .pointer("/project/model_traces")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|t| t["package"]["id"].as_u64())
        .collect();

    let mut new_models: Vec<ConvertedModel> = Vec::new();
    let mut model_of: Vec<(usize, Option<usize>)> = Vec::new();
    for (k, site) in packages.iter().enumerate() {
        let Some(body) = at_ref(file, &site.path) else {
            continue;
        };
        let Some(form) = body.get("form") else {
            continue;
        };
        let family = form["family"].as_str().unwrap_or("").to_owned();
        if family == "generic" {
            model_of.push((k, None));
            continue;
        }
        if let Some(joints) = joints(form)
            && let Some(body) = at(file, &site.path).and_then(Value::as_object_mut)
        {
            body.insert("joints".into(), joints);
            n += 1;
        }
        let Some(body) = at_ref(file, &site.path) else {
            continue;
        };
        let Some(form) = body.get("form") else {
            continue;
        };
        let model = &body["model"];
        let source = model["source"].as_str().unwrap_or("");
        let generated = source == "generated";
        // The marking specs of every part on the package: the host
        // compares the surfaces they use with what the converted package
        // offers.
        let markings: Vec<Value> = components_of
            .get(&site.id)
            .into_iter()
            .flatten()
            .filter_map(|c| markings_of.get(c))
            .flatten()
            .cloned()
            .collect();
        for (_, text) in footprints.iter().filter(|(p, _)| *p == site.id) {
            let used = form_values_used(text);
            if !used.is_empty() {
                refusals.add(
                    &site.name,
                    format!(
                        "a sketch footprint reads {} of its form, which no longer exist",
                        used.into_iter().collect::<Vec<_>>().join(", ")
                    ),
                );
            }
        }
        if generated {
            if model["materials"]["roles"]
                .as_array()
                .is_some_and(|r| !r.is_empty())
            {
                refusals.add(
                    &site.name,
                    "its generated model chooses body materials, which a part file model cannot keep",
                );
            }
            if family == "chip" && traced.contains(&site.id) {
                refusals.add(
                    &site.name,
                    "a dimension refers to a face of its chip model, whose face names change with the chip's part file",
                );
            }
        }
        let terminals: Vec<String> = body["terminals"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|t| t["designation"].as_str().unwrap_or("").to_owned())
            .collect();
        let Some(host) = host else { continue };
        let reply = host
            .package(&PackageRequest {
                name: &site.name,
                form,
                model,
                envelope: &body["envelope_nm"],
                terminals,
                markings,
            })
            .map_err(|e| format!("{}: {e}", site.name))?;
        for refusal in reply.refusals {
            refusals.add(&site.name, refusal);
        }
        if let Some(notes) = &options.notes
            && let Ok(mut notes) = notes.lock()
        {
            notes.extend(reply.notes.iter().map(|m| format!("{}: {m}", site.name)));
        }
        let index = reply.model.map(|m| {
            new_models.push(m);
            new_models.len() - 1
        });
        if generated && index.is_none() {
            return Err(format!(
                "{}: the host made no model for its generated model",
                site.name
            )
            .into());
        }
        model_of.push((k, index));
        // The leads, written now (the body is rewritten below).
        let leads: Vec<Value> = body["terminals"]
            .as_array()
            .into_iter()
            .flatten()
            .zip(&reply.leads)
            .map(|(t, [x, y])| json!({"terminal": t["id"], "contact": {"x_nm": x, "y_nm": y}}))
            .collect();
        if let Some(body) = at(file, &site.path).and_then(Value::as_object_mut)
            && !leads.is_empty()
        {
            body.insert("leads".into(), Value::Array(leads));
            n += 1;
        }
    }

    // Package rows.
    let rows = sites(file, "package_row");
    let mut row_updates = Vec::new();
    for site in &rows {
        let Some(body) = at_ref(file, &site.path) else {
            continue;
        };
        if body.get("generator").is_some() {
            continue;
        }
        if body.get("lands").is_some_and(|l| !l.is_null()) {
            refusals.add(
                &site.name,
                "it overrides the lands, which a generator's parameters do not hold",
            );
        }
        if body.get("symbol").is_some_and(|l| !l.is_null()) {
            refusals.add(
                &site.name,
                "it suggests a symbol; a generator draws its own",
            );
        }
        if body.get("skeleton").is_some_and(|s| {
            !s.is_null() && s.get("skeleton").and_then(Value::as_str) != Some("blank")
        }) {
            refusals.add(
                &site.name,
                "it suggests a symbol skeleton; a generator draws its own",
            );
        }
        let converted = if body["family"].is_null() {
            (String::new(), json!({}))
        } else {
            let Some(host) = host else {
                return Err(format!(
                    "{}: a package row needs the shipped generators to convert; run `{} convert`",
                    site.name,
                    sparksmith_core::product::SHORT_NAME
                )
                .into());
            };
            match host.row(&body["values"]) {
                Ok(c) => c,
                Err(why) => {
                    refusals.add(&site.name, why);
                    continue;
                }
            }
        };
        row_updates.push((site.path.clone(), converted));
    }

    if !refusals.0.is_empty() {
        return Err(format!(
            "this file holds what converting to version 5 would lose, so it was not converted:\n  {}",
            refusals.0.join("\n  ")
        )
        .into());
    }

    for (path, (generator, params)) in row_updates {
        let Some(body) = at(file, &path).and_then(Value::as_object_mut) else {
            continue;
        };
        let mut row = Map::new();
        row.insert("generator".into(), Value::String(generator));
        row.insert("params".into(), params);
        for key in ["aliases", "size_code", "provenance", "citation"] {
            if let Some(v) = body.get(key) {
                row.insert(key.into(), v.clone());
            }
        }
        *body = row;
        n += 1;
    }

    // The part files as Models of the project (one per distinct file),
    // their bytes packed.
    let mut model_refs: Vec<(u64, u64)> = Vec::new();
    for model in &new_models {
        model_refs.push(add_model(file, model)?);
    }
    for (k, index) in model_of {
        let Some(site) = packages.get(k) else {
            continue;
        };
        let Some(body) = at(file, &site.path).and_then(Value::as_object_mut) else {
            continue;
        };
        n += usize::from(body.remove("form").is_some());
        if let Some(i) = index
            && let (Some(model), Some((id, revision))) = (new_models.get(i), model_refs.get(i))
        {
            body.insert(
                "model".into(),
                json!({
                    "source": "part_file",
                    "model": {"id": id, "revision": revision},
                    "values": model.values,
                    "derived": model.derived,
                }),
            );
            n += 1;
        }
    }
    // A generic form leaves too.
    for site in &packages {
        if let Some(body) = at(file, &site.path).and_then(Value::as_object_mut) {
            n += usize::from(body.remove("form").is_some());
        }
    }
    Ok(n)
}

/// The Model of `model`'s part file: the project's own when it already has
/// a `solidcraft` Model of those bytes, else a new lineage; the bytes are
/// packed when the project does not pack them yet (inline, which the
/// container conversion moves to an asset entry).
fn add_model(file: &mut Value, model: &ConvertedModel) -> Result<(u64, u64)> {
    let hash = model.definition["asset"]["hash"]
        .as_str()
        .ok_or("a converted model names no asset")?
        .to_owned();
    let media_type = model.definition["asset"]["media_type"]
        .as_str()
        .unwrap_or("application/x-solidcraft")
        .to_owned();
    let project = file
        .get_mut("project")
        .and_then(Value::as_object_mut)
        .ok_or("the document has no project")?;
    let next_id = project
        .get("next_id")
        .and_then(Value::as_u64)
        .ok_or("the project has no next_id")?;
    let library = project
        .entry("library")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or("the library is not an object")?;
    let assets = library
        .entry("assets")
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .ok_or("the library's assets are not a list")?;
    if !assets.iter().any(|a| a["hash"] == hash.as_str()) {
        let data = serde_json::to_value(sparksmith_core::library::bundle::AssetBytes::new(
            model.bytes.clone(),
        ))?;
        assets.push(json!({"hash": hash, "media_type": media_type, "data": data}));
    }
    let models = library
        .entry("models")
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .ok_or("the library's models are not a list")?;
    if let Some(held) = models.iter().find(|m| {
        m["definition"]["format"] == "solidcraft"
            && m["definition"]["asset"]["hash"] == hash.as_str()
    }) {
        return Ok((
            held["id"].as_u64().unwrap_or(0),
            held["revision"].as_u64().unwrap_or(1),
        ));
    }
    models.push(json!({
        "id": next_id,
        "revision": 1,
        "meta": {
            "name": model.name,
            "description": "",
            "provenance": "A generator's part file, made the model of packages whose typed form a project of format version 4 stored (sparksmith convert)."
        },
        "definition": model.definition,
    }));
    project.insert("next_id".into(), Value::from(next_id + 1));
    Ok((next_id, 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn form_values_beyond_the_envelope_are_found() {
        let used = form_values_used(r#"{"expr":"pkg_e * 2 + pkg_L","b":"xpkg_q pkg_n pkg_D1"}"#);
        assert_eq!(
            used.into_iter().collect::<Vec<_>>(),
            ["pkg_D1".to_owned(), "pkg_e".to_owned()]
        );
    }

    #[test]
    fn a_typed_form_without_a_host_is_refused_and_a_generic_one_converts() -> Result<()> {
        let mut file = json!({
            "version": 4,
            "project": {"next_id": 9, "library": {"packages": [
                {"id": 3, "revision": 1, "meta": {"name": "P"}, "definition": {
                    "terminals": [], "form": {"family": "generic"}, "envelope_nm": [1, 1, 1],
                    "model": {"source": "envelope", "material": "epoxy_mould"}
                }}
            ]}},
            "drafts": {"items": []}
        });
        assert_eq!(step(&mut file, &Options::default())?, 1);
        assert!(
            file["project"]["library"]["packages"][0]["definition"]
                .get("form")
                .is_none()
        );
        let mut typed = json!({
            "version": 4,
            "project": {"next_id": 9, "library": {"packages": [
                {"id": 3, "revision": 1, "meta": {"name": "R"}, "definition": {
                    "terminals": [], "form": {"family": "chip"}, "envelope_nm": [1, 1, 1],
                    "model": {"source": "generated", "family_rev": 1, "materials": {"roles": []}}
                }}
            ]}},
            "drafts": {"items": []}
        });
        let refused = step(&mut typed, &Options::default());
        assert!(refused.is_err_and(|e| e.to_string().contains("convert")));
        Ok(())
    }
}
