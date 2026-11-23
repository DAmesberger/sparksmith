//! Successful wire results. Core reports and outcomes use the schemas of
//! the types actually serialized. Host commands add the fields they emit.
use schemars::{JsonSchema, SchemaGenerator};
use serde_json::{Value, json};
use sparksmith_core::{Finding, Outcome, Project, Stage};
use std::collections::BTreeMap;

mod assistant;
mod commands;
mod generators;
mod operations;
pub(super) use operations::operation;
mod queries;

pub(super) struct Builder {
    generator: SchemaGenerator,
}

impl Builder {
    fn typed<T: JsonSchema>(&mut self) -> Value {
        json!(T::json_schema(&mut self.generator))
    }

    /// A nested domain type can have hundreds of fields. Keep its reference
    /// in the reply's first page and page its definition on demand.
    fn reference<T: JsonSchema>(&mut self) -> Value {
        json!(self.generator.subschema_for::<T>())
    }

    fn edit(&mut self) -> Value {
        let mut result = self.document();
        required(&mut result, "outcome", self.reference::<Outcome>());
        result
    }

    fn document(&mut self) -> Value {
        let mut result = object(
            &[
                ("revision", self.typed::<u64>()),
                ("can_undo", self.typed::<bool>()),
                ("can_redo", self.typed::<bool>()),
            ],
            &[],
        );
        let report = object(
            &[
                ("report", json!({"const":"definition"})),
                (
                    "result",
                    self.reference::<sparksmith_core::completeness::DefinitionReport>(),
                ),
            ],
            &[],
        );
        let error = object(
            &[(
                "error",
                json!({"anyOf":[self.reference::<sparksmith_core::Diagnostic>(),self.typed::<String>()]}),
            )],
            &[],
        );
        let snapshot = object(
            &[
                ("draft", self.reference::<sparksmith_core::library::Draft>()),
                ("check", json!({"oneOf":[report,error]})),
            ],
            &[],
        );
        let compact_draft = self.draft();
        let listed = self.typed::<BTreeMap<String, Vec<Value>>>();
        let changes = object(
            &[],
            &[
                ("added", listed.clone()),
                ("removed", listed.clone()),
                ("changed", listed),
                ("fields", self.typed::<Vec<String>>()),
                ("omitted", self.typed::<usize>()),
            ],
        );
        let counts = self.typed::<BTreeMap<String, usize>>();
        let finding = object(
            &[
                ("code", self.typed::<String>()),
                ("message", self.typed::<String>()),
                ("objects", self.typed::<Vec<sparksmith_core::ObjectRef>>()),
            ],
            &[("blocks", self.typed::<Stage>())],
        );
        let findings = object(
            &[
                ("new", array(finding)),
                ("resolved", counts),
                ("open", self.typed::<usize>()),
            ],
            &[
                ("omitted", self.typed::<usize>()),
                ("resolved_total", self.typed::<usize>()),
                (
                    "issues",
                    self.typed::<sparksmith_core::issues::IssueCounts>(),
                ),
            ],
        );
        let mut rerouted = self.typed::<sparksmith_core::layout::router::reroute::Rerouted>();
        required(&mut rerouted, "summary", self.typed::<String>());
        for (name, schema) in [
            ("project", self.reference::<Project>()),
            (
                "netlist",
                self.reference::<sparksmith_core::instances::ties::Netlist<'static>>(),
            ),
            ("drafts", array(json!({"anyOf":[compact_draft,snapshot]}))),
            ("erc", self.typed::<Vec<Finding>>()),
            ("board_checks", self.typed::<Vec<Finding>>()),
            ("changes", changes),
            ("findings", findings),
            ("rerouted", rerouted),
            ("dry_run", json!({"const":true})),
            (
                "rebased",
                object(
                    &[("from", self.typed::<u64>()), ("onto", self.typed::<u64>())],
                    &[],
                ),
            ),
        ] {
            result["properties"][name] = schema;
        }
        result
    }

    /// Inspect, new and open return a compact summary or the verbose
    /// snapshot. Both carry history flags, and each mode has a required body.
    fn document_or_summary(&mut self) -> Value {
        let mut result = self.document();
        result["properties"]["summary"] = self.summary();
        result["oneOf"] = json!([
            {"required":["summary"]},
            {"required":["project","drafts","erc","board_checks"]}
        ]);
        result
    }

    fn draft(&mut self) -> Value {
        let finding = object(
            &[
                ("code", self.typed::<String>()),
                ("message", self.typed::<String>()),
                ("blocks", self.typed::<Option<Stage>>()),
            ],
            &[],
        );
        object(
            &[
                ("id", self.typed::<sparksmith_core::ids::DefinitionId>()),
                ("kind", self.typed::<String>()),
                ("name", self.typed::<String>()),
                ("publishable", self.typed::<bool>()),
                ("finding_count", self.typed::<usize>()),
            ],
            &[("findings", array(finding))],
        )
    }

    fn summary(&mut self) -> Value {
        let map = self.typed::<BTreeMap<String, usize>>();
        let findings = object(
            &[
                ("open", self.typed::<usize>()),
                ("by_code", map.clone()),
                (
                    "issues",
                    self.typed::<sparksmith_core::issues::IssueCounts>(),
                ),
            ],
            &[],
        );
        object(
            &[
                ("name", self.typed::<String>()),
                ("path", self.typed::<Option<String>>()),
                ("counts", map),
                ("drafts", array(self.draft())),
                ("findings", findings),
            ],
            &[],
        )
    }
}

fn array(items: Value) -> Value {
    json!({"type":"array","items":items})
}

fn object(fields: &[(&str, Value)], optional: &[(&str, Value)]) -> Value {
    let properties: serde_json::Map<_, _> = fields
        .iter()
        .chain(optional)
        .map(|(k, v)| ((*k).to_owned(), v.clone()))
        .collect();
    let mut schema = json!({"type":"object","properties":properties});
    if !fields.is_empty() {
        schema["required"] = json!(fields.iter().map(|(k, _)| k).collect::<Vec<_>>());
    }
    schema
}

fn required(object: &mut Value, field: &str, schema: Value) {
    object["properties"][field] = schema;
    if object.get("required").is_none() {
        object["required"] = json!([]);
    }
    if let Some(fields) = object["required"].as_array_mut() {
        fields.push(json!(field));
    }
}

pub(super) fn command(key: super::RequestId, outcome_tags: &[&str]) -> Value {
    build(|cx| {
        let mut result = commands::command(cx, key);
        if !outcome_tags.is_empty() {
            result["properties"]["outcome"] = operations::payload(cx, outcome_tags);
        }
        result
    })
}

pub(super) fn query(decl: &sparksmith_core::permission::EntryDecl) -> Value {
    build(|cx| queries::query(cx, decl))
}

fn build(construct: impl FnOnce(&mut Builder) -> Value) -> Value {
    let mut cx = Builder {
        generator: schemars::generate::SchemaSettings::default()
            .for_serialize()
            .into_generator(),
    };
    let mut root = construct(&mut cx);
    // A catalog entry without an output declaration fails the coverage
    // test. An impossible schema cannot be mistaken for an open contract.
    if root.is_object() && !cx.generator.definitions().is_empty() {
        let defs = super::discovery::reachable_defs(&root, cx.generator.definitions());
        if !defs.is_empty() {
            root["$defs"] = Value::Object(defs);
        }
    }
    open(&mut root);
    root
}

/// A reply's objects are open (docs/extension-surface.md, "Replies"): a
/// consumer ignores fields it does not know, so a new optional reply field
/// is additive. The stored types refuse unknown fields when a file is read
/// (`deny_unknown_fields`); that is the file format's rule, not the reply
/// contract's, so it is dropped from every reply schema here. Parameter
/// schemas stay strict.
pub(super) fn open(schema: &mut Value) {
    match schema {
        Value::Object(object) => {
            if object.get("additionalProperties") == Some(&Value::Bool(false)) {
                object.remove("additionalProperties");
            }
            for value in object.values_mut() {
                open(value);
            }
        }
        Value::Array(items) => items.iter_mut().for_each(open),
        _ => {}
    }
}
