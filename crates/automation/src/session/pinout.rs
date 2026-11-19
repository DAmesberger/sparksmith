//! Slice C3: `library.pinout_schema`, the JSON schema of the
//! typed pinout document agents fill from a datasheet. Preview and apply
//! are core operations (`project.query` `library.pinout_preview`,
//! `project.apply` `library.pinout_apply`).
use super::Session;
use crate::Result;
use serde_json::Value;

impl Session {
    /// The pinout document schema, with its format number and the pin
    /// rule script contract (helpers, verdicts, examples).
    pub(super) fn pinout_schema(&self) -> Result<Value> {
        Ok(serde_json::json!({
            "format": sparksmith_core::library::pinout::PINOUT_FORMAT,
            "schema": sparksmith_core::library::pinout::schema(),
            "rule_api": sparksmith_core::functions::rules::RULE_API,
        }))
    }

    /// What `library.review_check` holds a report against: the drafts of
    /// one part (the component lineage `component`, else the component
    /// draft named as the report's `part`) or, with no draft of it open,
    /// its latest published revision with the pins it was published
    /// unreviewed. Without either, every open draft.
    pub(super) fn review_snapshot(
        &self,
        component: Option<sparksmith_core::ids::DefinitionId>,
        part: &str,
    ) -> Result<Value> {
        use sparksmith_core::completeness::CheckTarget;
        use sparksmith_core::ids::{DefinitionKind, DefinitionRef};
        use sparksmith_core::library::Definition;
        use sparksmith_core::{Query, QueryResult};
        let mut snapshot = self.snapshot();
        let drafts = self.engine.drafts();
        let library = &self.engine.project().library;
        let named = || {
            let mut matching = drafts.items.iter().filter(|d| {
                matches!(d.definition, Definition::Component(_))
                    && d.meta.name.trim() == part.trim()
            });
            match (matching.next(), matching.next()) {
                (Some(only), None) => Some(only.id),
                _ => None,
            }
        };
        let Some(lineage) = component.or_else(named) else {
            return Ok(snapshot);
        };
        if let Some(draft) = drafts.get(lineage) {
            let Definition::Component(body) = &draft.definition else {
                return Err(crate::Error::Document(sparksmith_core::Error::Invalid(
                    format!("library.review_check: {lineage} is not a component"),
                )));
            };
            let mut ids = vec![lineage];
            ids.extend(body.representations.iter().map(|s| s.id));
            for binding in &body.implementations {
                ids.push(binding.id);
                let definition = drafts
                    .get(binding.id)
                    .and_then(|d| match &d.definition {
                        Definition::Binding(b) => Some(b.clone()),
                        _ => None,
                    })
                    .or_else(|| library.binding(*binding).map(|b| b.definition.clone()));
                if let Some(b) = definition {
                    ids.push(b.package.id);
                    ids.extend(b.footprints.iter().map(|f| f.id));
                }
            }
            let wanted: Vec<Value> = ids.iter().map(|id| serde_json::json!(id)).collect();
            if let Some(items) = snapshot["drafts"].as_array_mut() {
                items.retain(|d| wanted.contains(&d["draft"]["id"]));
            }
            return Ok(snapshot);
        }
        let published = library
            .latest_components()
            .into_iter()
            .find(|c| c.id == lineage)
            .ok_or_else(|| {
                crate::Error::Document(sparksmith_core::Error::NotFound(format!(
                    "library.review_check: no component {lineage} is drafted or published"
                )))
            })?;
        let at = DefinitionRef {
            kind: DefinitionKind::Component,
            id: published.id,
            revision: published.revision,
        };
        let mut definition = Definition::Component(published.definition.clone());
        sparksmith_core::library::pinout::restore_review(library, at, &mut definition);
        let check = match self.engine.query(Query::CheckDefinition {
            target: CheckTarget::Revision(at),
        })? {
            report @ QueryResult::Definition(_) => serde_json::to_value(report)?,
            _ => Value::Null,
        };
        snapshot["drafts"] = serde_json::json!([{
            "draft": {"id": lineage, "meta": published.meta, "definition": definition},
            "check": check,
        }]);
        snapshot["published"] = serde_json::json!(true);
        Ok(snapshot)
    }
}
