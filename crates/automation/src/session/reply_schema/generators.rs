//! Generator discovery, generation and fitting a body to the placed part.
use super::super::generators::{GeneratorSummary, GeneratorsReport, ShippedGenerator, v1};
use super::{Builder, array, object, required};
use serde_json::{Value, json};
use sparksmith_core::{
    commands::body::BodyPlan,
    ids::{DefinitionId, PadId, Revision},
    library::PadKind,
    updates::UpdatePlan,
};

pub(super) fn generators(cx: &mut Builder) -> Value {
    listing(cx)
}

pub(super) fn body_fit(cx: &mut Builder) -> Value {
    let target = body_target(cx);
    let mut only = object(&[("target", target.clone())], &[]);
    only["additionalProperties"] = false.into();
    let mut fitted = cx.typed::<BodyPlan>();
    required(&mut fitted, "target", target);
    json!({"anyOf":[only, fitted]})
}

pub(super) fn make_body(cx: &mut Builder) -> Value {
    cx.edit()
}

pub(super) fn generate_part(cx: &mut Builder) -> Value {
    let mut edit = cx.edit();
    edit["properties"]["update"] = cx.typed::<UpdatePlan>();
    edit
}

/// The detailed report contains full presets. Named requests can abbreviate a
/// preset to its key, name and basis; the catalog list summarizes whole modules.
fn listing(cx: &mut Builder) -> Value {
    let text = cx.typed::<String>();
    let mut compact_preset = object(
        &[("key", text.clone()), ("name", text.clone())],
        &[("basis", cx.typed::<v1::Basis>())],
    );
    compact_preset["additionalProperties"] = false.into();
    let mut description = cx.typed::<GeneratorSummary>();
    description["properties"]["presets"] =
        array(json!({"anyOf":[cx.typed::<v1::Preset>(), compact_preset]}));
    let mut shipped = cx.typed::<ShippedGenerator>();
    shipped["properties"]["description"] = description;
    let mut short_preset = object(&[("key", text.clone())], &[("name", text.clone())]);
    short_preset["additionalProperties"] = false.into();
    let mut summary = object(
        &[("name", text.clone())],
        &[
            ("title", text.clone()),
            ("families", cx.typed::<Vec<String>>()),
            ("scopes", cx.typed::<Vec<v1::Scope>>()),
            ("version", cx.typed::<u32>()),
            ("presets", array(short_preset)),
            ("refused", text.clone()),
        ],
    );
    summary["additionalProperties"] = false.into();
    let mut report = cx.typed::<GeneratorsReport>();
    report["properties"]["shipped"] = array(json!({"anyOf":[shipped, summary]}));
    report["properties"]["detail"] = text;
    report
}

fn body_target(cx: &mut Builder) -> Value {
    let text = cx.typed::<String>();
    let point = cx.typed::<[f64; 2]>();
    let pad = object(
        &[
            ("id", cx.typed::<PadId>()),
            ("number", text.clone()),
            ("at", point.clone()),
            ("size", point),
            ("rotation_deg", cx.typed::<f64>()),
            ("kind", cx.typed::<PadKind>()),
        ],
        &[],
    );
    let footprint = object(
        &[("name", cx.typed::<Option<String>>()), ("pads", array(pad))],
        &[],
    );
    let package = object(
        &[
            ("id", cx.typed::<DefinitionId>()),
            ("revision", cx.typed::<Revision>()),
            ("name", text.clone()),
            ("terminals", cx.typed::<Vec<String>>()),
            ("envelope_mm", cx.typed::<[f64; 3]>()),
            ("model", text.clone()),
        ],
        &[],
    );
    object(
        &[
            ("part", text),
            ("parts", cx.typed::<Vec<String>>()),
            ("package", package),
            ("footprint", footprint),
        ],
        &[],
    )
}
