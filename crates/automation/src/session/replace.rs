//! `project.preview_replace` and `project.replace_component`: a placed
//! part's component replaced by a component of another lineage (an
//! imported or generated part by a reviewed library part), named as a
//! person names them: parts by designator, terminals by id or name. The
//! core change is `updates::Change::ReplaceComponent`, previewed and
//! applied as `project.preview_update` and `project.apply_update` do: the
//! preview is bound to the revision it was made at, the apply is one undo
//! step at `expected_revision`.
use super::{Session, hosted};
use crate::Result;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sparksmith_core::ids::{ComponentRef, DefinitionId, Id, TerminalId};
use sparksmith_core::library::ComponentDefinition;
use sparksmith_core::model::PhysicalChoice;
use sparksmith_core::updates::{self, Alignment, Change, Replacement, TerminalPair, UnitPair};
use sparksmith_core::{Operation, library::MAX_PACKAGE_TERMINALS};

/// Bytes of a designator or terminal name.
const MAX_NAME: usize = 256;

/// A terminal by its id (a number) or its exact name (a string).
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(untagged)]
pub enum TerminalName {
    Id(TerminalId),
    Name(String),
}

/// One old terminal and the new terminal it becomes; `to` null or absent
/// drops it (its wires are removed and it leaves its net).
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TerminalChoice {
    pub from: TerminalName,
    #[serde(default)]
    pub to: Option<TerminalName>,
}

/// `project.preview_replace`. Required: `designators`, `target`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PreviewReplaceParams {
    /// The placed parts, e.g. `["U3", "U4"]`; they must use the same
    /// component revision.
    pub designators: Vec<String>,
    /// The library component revision that replaces theirs, of another
    /// lineage. A part from a library folder is imported into the project
    /// first (`library.import_from_folder`).
    pub target: ComponentRef,
    /// The target's symbol lineage; required when it offers several.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub representation: Option<DefinitionId>,
    /// The target's binding and footprint; required when it offers
    /// several.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub physical: Option<PhysicalChoice>,
    /// Every terminal of the current component mapped to a terminal of the
    /// target, or to null to drop it. Left out, the preview only offers a
    /// suggestion by equal names, which is never applied by itself.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub terminals: Vec<TerminalChoice>,
    /// Where each placed or unused unit goes; may be left out when both
    /// symbols have one unit.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub units: Vec<UnitPair>,
    /// `fit_pads` (the default): the new footprint is moved, and turned by
    /// quarter turns, so its mapped pads best fit the old ones, each
    /// terminal's residual in the plan's `fits`. `keep_origin`: it keeps
    /// the old placement's origin and rotation.
    #[serde(default)]
    pub alignment: Alignment,
}

/// `project.replace_component`. Required: `replace`, `expected_revision`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReplaceComponentParams {
    /// The replacement, as previewed (`terminals` complete).
    pub replace: PreviewReplaceParams,
    /// Remove the conflicting tracks and vias the preview listed.
    #[serde(default)]
    pub remove_conflicting_tracks: bool,
    /// Join the named nets the preview listed in `merges_named`.
    #[serde(default)]
    pub merge_named_nets: bool,
    pub expected_revision: u64,
}

fn invalid(message: String) -> crate::Error {
    crate::Error::Document(sparksmith_core::Error::Invalid(message))
}

/// The one terminal of `component` that `name` names.
fn resolve(component: &ComponentDefinition, what: &str, name: &TerminalName) -> Result<TerminalId> {
    match name {
        TerminalName::Id(id) => component
            .terminal(*id)
            .map(|t| t.id)
            .ok_or_else(|| invalid(format!("replace.terminal: {what} has no terminal {id}"))),
        TerminalName::Name(text) => {
            let found: Vec<TerminalId> = component
                .terminals
                .iter()
                .filter(|t| t.name == *text)
                .map(|t| t.id)
                .collect();
            match found.as_slice() {
                [one] => Ok(*one),
                [] => Err(invalid(format!(
                    "replace.terminal: {what} has no terminal named ‘{text}’"
                ))),
                many => Err(invalid(format!(
                    "replace.terminal: {what} has {} terminals named ‘{text}’ ({}); name it by id",
                    many.len(),
                    many.iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                ))),
            }
        }
    }
}

fn terminals_json(component: &ComponentDefinition) -> Value {
    component
        .terminals
        .iter()
        .map(|t| json!({"id": t.id, "name": t.name}))
        .collect()
}

impl Session {
    /// The core change the parameters name, with the current and the
    /// target component.
    fn replace_change(
        &self,
        params: &PreviewReplaceParams,
    ) -> Result<(Change, ComponentDefinition, ComponentDefinition)> {
        let project = self.engine.project();
        if params.designators.is_empty()
            || params.designators.len() > project.instances.len().max(1)
            || params.designators.iter().any(|d| d.len() > MAX_NAME)
            || params.terminals.len() > MAX_PACKAGE_TERMINALS
        {
            return Err(invalid(
                "replace.params: name 1 to as many parts as the project has, and at most one mapping per terminal"
                    .into(),
            ));
        }
        let mut instances: Vec<Id> = Vec::with_capacity(params.designators.len());
        for designator in &params.designators {
            let instance = project
                .instances
                .iter()
                .find(|i| i.designator == *designator)
                .ok_or_else(|| {
                    invalid(format!("replace.no_part: there is no part {designator}"))
                })?;
            instances.push(instance.id);
        }
        let first = project
            .instance(
                instances
                    .first()
                    .copied()
                    .ok_or_else(|| invalid("replace.no_part".into()))?,
            )
            .ok_or_else(|| invalid("replace.no_part".into()))?;
        let library = &project.library;
        let old = library
            .component(first.component)
            .map(|c| c.definition.clone())
            .ok_or_else(|| invalid("replace.component: the parts' component is missing".into()))?;
        let target = library
            .component(params.target)
            .map(|c| c.definition.clone())
            .ok_or_else(|| {
                invalid(format!(
                    "replace.target: the project holds no component {}; import it first (library.import_from_folder)",
                    params.target
                ))
            })?;
        let mut terminals = Vec::with_capacity(params.terminals.len());
        for choice in &params.terminals {
            terminals.push(TerminalPair {
                from: resolve(&old, "the current component", &choice.from)?,
                to: match &choice.to {
                    Some(to) => Some(resolve(&target, "the target component", to)?),
                    None => None,
                },
            });
        }
        let change = Change::ReplaceComponent {
            instances,
            replacement: Replacement {
                target: params.target,
                representation: params.representation,
                physical: params.physical,
                terminals,
                units: params.units.clone(),
                alignment: params.alignment,
            },
        };
        Ok((change, old, target))
    }

    /// `project.preview_replace`: the update plan at the current revision,
    /// with both components' terminals and a mapping by equal names offered
    /// as a suggestion only.
    pub(super) fn preview_replace(&mut self, params: PreviewReplaceParams) -> Result<Value> {
        let (change, old, target) = self.replace_change(&params)?;
        let plan = self.engine.preview_update_hosted(change, hosted());
        let suggestion: Vec<Value> = updates::suggest_by_name(&old, &target)
            .into_iter()
            .map(|(from, name, to)| {
                let to = match to.as_slice() {
                    [one] => json!(one),
                    _ => Value::Null,
                };
                json!({"from": from, "name": name, "to": to})
            })
            .collect();
        Ok(json!({
            "plan": plan,
            "terminals": {"current": terminals_json(&old), "target": terminals_json(&target)},
            "suggested_terminals": suggestion,
            "suggestion_note": "A mapping by equal names, offered only: matching names do not make the same pin. Check it against both datasheets and pass it as terminals; nothing is applied without it.",
        }))
    }

    /// `project.replace_component`: the previewed replacement applied as
    /// `project.apply_update` does, one undo step; the reply carries the
    /// plan's copper report, warnings and detached copper.
    pub(super) fn replace_component(&mut self, params: ReplaceComponentParams) -> Result<Value> {
        let (change, _, _) = self.replace_change(&params.replace)?;
        let plan = self.engine.preview_update_hosted(change.clone(), hosted());
        let operation = Operation::Update(Box::new(updates::ApplyUpdate {
            expected_revision: params.expected_revision,
            change,
            remove_conflicting_tracks: params.remove_conflicting_tracks,
            merge_named_nets: params.merge_named_nets,
        }));
        let mut reply = self.apply(params.expected_revision, operation)?;
        reply["copper"] = serde_json::to_value(&plan.copper)?;
        reply["warnings"] = serde_json::to_value(&plan.warning_lines)?;
        reply["summary"] = json!(plan.summary);
        Ok(reply)
    }
}
