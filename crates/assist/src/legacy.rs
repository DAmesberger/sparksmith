//! Recordings made before the built-in agent moved onto the surface (ABI
//! S6) name the seventeen tools a run offered then (`library_pinout_edit`,
//! `submit_review`, …). They stay as recorded: the model's own output is
//! the evidence, and no model is asked again. A replay of such a recording
//! ([`crate::model::ModelBackend::legacy_tools`]) reads each old call as
//! the verb call that does what the tool did ([`translate`]); the call then
//! runs through the verbs like any other. Nothing else reads these names: a
//! run offers the six verbs only, and a live model's call of an old name is
//! refused as an unknown tool.
//!
//! The mapping is the old tools' own: the module and tabbed values in
//! millimetres become the shipped generator's parameters, a report without
//! its wrapper or with its lists as text is read as meant, and a draft is
//! inspected by its position in the snapshot.
use serde_json::{Map, Value, json};
use sparksmith_automation::Session;
use sparksmith_automation::session::generators::GeneratePartParams;
use sparksmith_automation::session::metadata::{self, Kind};

/// The page-view tool of old recordings.
pub const VIEW_PAGE: &str = "datasheet_view_page";
/// The tool that ended an old run.
pub const SUBMIT_REVIEW: &str = "submit_review";

/// The verb and its arguments that do what the old tool `name` did with
/// `input`, on `session` as it is now; `None` when `name` was no old tool,
/// and `Err` with the reason the old tool would have refused.
pub fn translate(
    session: &mut Session,
    name: &str,
    input: &Value,
) -> Option<Result<(&'static str, Value), String>> {
    let revision = session.engine().revision();
    // A read is a query, or an action when it can name a file.
    let query = |entry: &str, params: Value| {
        let verb = match metadata::find(entry).and_then(|e| e.kind) {
            Some(Kind::Action) => "act",
            _ => "query",
        };
        Ok((verb, json!({"name": entry, "params": params})))
    };
    // A change goes through the verb of its entry's kind: an operation
    // through apply, an action (a change that can name a file, say)
    // through act.
    let apply = |entry: &str, params: Value| {
        let action = metadata::find(entry).and_then(|e| e.kind) == Some(Kind::Action);
        Ok(if action {
            (
                "act",
                json!({"name": entry, "params": params, "expected_revision": revision}),
            )
        } else {
            (
                "apply",
                json!({"expected_revision": revision, "operations": [{"op": entry, "params": params}]}),
            )
        })
    };
    let field = |key: &str| input.get(key).cloned().unwrap_or(Value::Null);
    let some = |keys: &[&str]| -> Value {
        Value::Object(
            keys.iter()
                .filter_map(|k| input.get(*k).map(|v| ((*k).to_owned(), v.clone())))
                .collect::<Map<String, Value>>(),
        )
    };
    Some(match name {
        "library_pinout_schema" => query("library.pinout_schema", json!({})),
        "library_pinout_edit" => query(
            "assist.pinout_edit",
            some(&[
                "clear",
                "format",
                "device",
                "variants",
                "peripherals",
                "pins",
                "remove_pins",
                "rules",
                "remove_rules",
            ]),
        ),
        "library_pinout_preview" => query("assist.pinout_preview", some(&["target", "document"])),
        "library_pinout_apply" if input["document"].is_object() => {
            let accept = if input["accept"].is_object() {
                field("accept")
            } else {
                json!({"accept": "all"})
            };
            apply(
                "library.pinout_apply",
                json!({"target": field("target"), "document": field("document"),
                    "base": field("base"), "accept": accept}),
            )
        }
        "library_pinout_apply" => {
            let mut params = some(&["base", "target"]);
            if input["accept"].is_object() {
                params["accept"] = field("accept");
            }
            apply("assist.pinout_apply", params)
        }
        "library_package_schema" => Ok(("describe", json!({"name": "module"}))),
        "library_templates" => query("library.generators", json!({"detail": true})),
        "library_package_plan" | "library_package_from_template" => {
            match package_request(session, revision, &input["request"]) {
                Ok(params) => {
                    let mut call = json!({"expected_revision": revision,
                        "operations": [{"op": "library.package_from_generator", "params": params}]});
                    if name == "library_package_plan" {
                        call["dry_run"] = json!(true);
                    }
                    Ok(("apply", call))
                }
                Err(why) => Err(why),
            }
        }
        "library_set_marking" => apply("library.set_marking", some(&["draft", "marking"])),
        "library_check_part_file" if input["builtin"].is_null() => {
            query("library.part_files", json!({}))
        }
        "library_check_part_file" => query(
            "library.check_part_file",
            json!({"builtin": field("builtin"), "values": values(input)}),
        ),
        "library_use_part_file" => apply(
            "library.use_part_file",
            json!({"package": field("package"), "builtin": field("builtin"), "values": values(input)}),
        ),
        "library_set_part_values" => apply(
            "library.set_part_values",
            json!({"package": field("package"), "values": values(input)}),
        ),
        "library_inspect_draft" => {
            let position = session
                .engine()
                .drafts()
                .items
                .iter()
                .position(|d| json!(d.id) == input["id"]);
            match position {
                Some(n) => query("project.inspect", json!({"at": format!("/drafts/{n}")})),
                None => Err(format!("no draft {}", input["id"])),
            }
        }
        "library_edit" => apply("library.edit", some(&["edits"])),
        "component_check_drafts" => query("component.check_drafts", json!({})),
        SUBMIT_REVIEW => query(
            "library.review_check",
            json!({"report": report_input(input)}),
        ),
        VIEW_PAGE => query("assist.document_page", some(&["document", "page"])),
        _ => return None,
    })
}

/// The report a `submit_review` call carried. Local models often sent its
/// fields without the `report` wrapper, and its lists as JSON text: both
/// were read as meant.
fn report_input(input: &Value) -> Value {
    let mut report = if input["report"].is_object() {
        input["report"].clone()
    } else if let Some(text) = input["report"].as_str() {
        serde_json::from_str(text).unwrap_or(Value::Null)
    } else if input.get("part").is_some() {
        input.clone()
    } else {
        Value::Null
    };
    if let Some(fields) = report.as_object_mut() {
        for key in ["sources", "values", "open_questions"] {
            let parsed = fields
                .get(key)
                .and_then(Value::as_str)
                .and_then(|text| serde_json::from_str::<Value>(text).ok())
                .filter(Value::is_array);
            if let Some(list) = parsed {
                fields.insert(key.to_owned(), list);
            }
        }
    }
    report
}

/// A part-file tool's `values`, an empty object when absent.
fn values(input: &Value) -> Value {
    if input["values"].is_object() {
        input["values"].clone()
    } else {
        json!({})
    }
}

/// Millimetres from a value as integer nanometres.
fn nm(v: &Value) -> i64 {
    (v.as_f64().unwrap_or(f64::NAN) * 1e6).round() as i64
}

fn nm_pair(v: &Value) -> [i64; 2] {
    [nm(&v[0]), nm(&v[1])]
}

/// The module values (the custom module entry the tools took) as the
/// module generator's parameters.
pub(crate) fn module_params(v: &Value) -> Value {
    let rows = v["rows"].as_array().cloned().unwrap_or_default();
    let singles = v["singles"].as_array().cloned().unwrap_or_default();
    let list = |items: &[Value], f: &dyn Fn(&Value) -> Value| -> Value {
        Value::Array(items.iter().map(f).collect())
    };
    let [can_width, can_length] = nm_pair(&v["can_size"]);
    let [can_x, can_y] = nm_pair(&v["can_centre"]);
    let [pad_length, pad_width] = nm_pair(&v["pad_size"]);
    let [land_outer, land_inner] = nm_pair(&v["land_extend"]);
    let mut p = json!({
        "width": nm(&v["width"]), "length": nm(&v["length"]), "height": nm(&v["height"]),
        "board_thickness": nm(&v["pcb_thickness"]), "antenna_length": nm(&v["antenna_length"]),
        "can_width": can_width, "can_length": can_length, "can_x": can_x, "can_y": can_y,
        "pitch": nm(&v["pitch"]), "pad_length": pad_length, "pad_width": pad_width,
        "castellation": nm(&v["castellation"]), "land_width": nm(&v["land_width"]),
        "land_outer": land_outer, "land_inner": land_inner,
        "row_sides": list(&rows, &|r| r["side"].clone()),
        "row_counts": list(&rows, &|r| r["count"].clone()),
        "row_x": list(&rows, &|r| json!(nm(&r["first"][0]))),
        "row_y": list(&rows, &|r| json!(nm(&r["first"][1]))),
        "ground": v["ground"].is_object(),
        "single_numbers": list(&singles, &|s| s["number"].clone()),
        "single_x": list(&singles, &|s| json!(nm(&s["centre"][0]))),
        "single_y": list(&singles, &|s| json!(nm(&s["centre"][1]))),
        "single_length": list(&singles, &|s| json!(nm(&s["size"][0]))),
        "single_width": list(&singles, &|s| json!(nm(&s["size"][1]))),
        "keepout": v["keepout"].is_object(),
    });
    let ground = &v["ground"];
    if ground.is_object() {
        let [pitch_x, pitch_y] = nm_pair(&ground["pitch"]);
        let [size_x, size_y] = nm_pair(&ground["size"]);
        let [x, y] = nm_pair(&ground["centre"]);
        let turn = (ground["rotation_deg"].as_f64().unwrap_or(0.0) * 1000.0).round() as i64;
        for (key, value) in [
            ("ground_rows", ground["rows"].clone()),
            ("ground_columns", ground["cols"].clone()),
            ("ground_pitch_x", json!(pitch_x)),
            ("ground_pitch_y", json!(pitch_y)),
            ("ground_size_x", json!(size_x)),
            ("ground_size_y", json!(size_y)),
            ("ground_x", json!(x)),
            ("ground_y", json!(y)),
            ("ground_rotation", json!(turn)),
            ("ground_number", ground["number"].clone()),
        ] {
            p[key] = value;
        }
    }
    if v["keepout"].is_object() {
        let beyond = &v["keepout"]["beyond"];
        p["keepout_left"] = json!(nm(&beyond[0]));
        p["keepout_right"] = json!(nm(&beyond[1]));
        p["keepout_top"] = json!(nm(&beyond[2]));
    }
    p
}

/// The tabbed values as the tabbed generator's parameters; a datasheet's
/// land pattern as `lands: explicit`.
fn tabbed_params(v: &Value) -> Value {
    let (leads, step) = (&v["leads_per_side"], &v["lead_step"]);
    let mut p = json!({
        "leads_s": leads[0], "leads_n": leads[1],
        "step_s": step[0].as_u64().unwrap_or(1), "step_n": step[1].as_u64().unwrap_or(1),
        "pitch": nm(&v["pitch"]), "body_length": nm(&v["body_length"]),
        "body_width": nm(&v["body_width"]), "body_height": nm(&v["body_height"]),
        "standoff": nm(&v["standoff"]), "span": nm(&v["span"]),
        "lead_width": nm(&v["lead_width"]), "lead_thickness": nm(&v["lead_thickness"]),
        "foot_length": nm(&v["foot_length"]), "tab": "none",
    });
    if let Some(lands) = v.get("lands").filter(|l| l.is_object()) {
        let [width, length] = nm_pair(&lands["size"]);
        p["lands"] = json!("explicit");
        p["land_width"] = json!(width);
        p["land_length"] = json!(length);
        p["land_span"] = json!(nm(&lands["span"]));
    }
    p
}

/// A package request of the old tools (draft, row, values in millimetres)
/// as `library.package_from_generator`'s parameters: the package made by
/// the family's shipped generator with its model evaluated. Only the
/// module and the tabbed families were taken.
fn package_request(session: &mut Session, revision: u64, request: &Value) -> Result<Value, String> {
    let values = &request["values"];
    let (generator, params) = match values["family"].as_str() {
        Some("module") => ("module", module_params(values)),
        Some("tabbed") => ("tabbed", tabbed_params(values)),
        other => {
            return Err(format!(
                "no generator takes {} values here; the module and tabbed families are made by their generators",
                other.unwrap_or("these")
            ));
        }
    };
    let params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    session.ensure_generators();
    let generate = session
        .generate_request(GeneratePartParams {
            generator: Some(generator.into()),
            module: None,
            scope: sparksmith_core::library::generated::GeneratorScope::Package,
            preset: None,
            params,
            name: None,
            replaces: None,
            expected_revision: revision,
        })
        .map_err(|e| e.to_string())?;
    let mut out = json!({"draft": request["draft"], "generate": generate});
    if request["replace"] == true {
        out["replace"] = json!(true);
    }
    if request["renumber"].is_array() {
        out["renumber"] = request["renumber"].clone();
    }
    if !request["terminals"].is_null() {
        out["terminals"] = request["terminals"].clone();
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_report_without_its_wrapper_or_with_lists_as_text_is_read() {
        let report =
            json!({"part": "X", "summary": "s", "sources": [], "values": [], "open_questions": []});
        let flat = json!({"part": "X", "summary": "s", "sources": "[]", "values": "[]", "open_questions": "[]"});
        assert_eq!(report_input(&json!({"report": report.clone()})), report);
        assert_eq!(report_input(&flat), report);
        assert_eq!(report_input(&json!({"report": report.to_string()})), report);
    }

    #[test]
    fn every_old_tool_has_its_verb_and_other_names_none() {
        let mut session = Session::default();
        for (old, verb) in [
            ("library_pinout_schema", "query"),
            ("library_pinout_edit", "query"),
            ("library_pinout_preview", "query"),
            ("library_pinout_apply", "apply"),
            ("library_package_schema", "describe"),
            ("library_templates", "query"),
            ("library_set_marking", "apply"),
            ("library_check_part_file", "query"),
            ("library_use_part_file", "act"),
            ("library_set_part_values", "apply"),
            ("library_edit", "apply"),
            ("component_check_drafts", "query"),
            (SUBMIT_REVIEW, "query"),
            (VIEW_PAGE, "query"),
        ] {
            let translated = translate(&mut session, old, &json!({}));
            assert!(
                matches!(translated, Some(Ok((v, _))) if v == verb),
                "{old}: {translated:?}"
            );
        }
        assert!(translate(&mut session, "library_publish", &json!({})).is_none());
        assert!(translate(&mut session, "query", &json!({})).is_none());
    }
}
