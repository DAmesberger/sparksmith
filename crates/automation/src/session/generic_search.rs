//! `library.search` for an agent also reads its text as on-demand generic
//! parts, as the Place palette does (`catalog::generic::generators::search`,
//! the one matcher): "10k 0603 resistor" lists the shipped generator's
//! chip resistor with its value. E2E run 12: the chat searched for
//! "capacitor 0603", found nothing it could place and invented components
//! of its own.
//!
//! Each candidate says where it stands. One the project holds carries its
//! `component`, the `physical` choice the Place palette gives it and the
//! `properties` that set its values on a placed instance, ready for
//! `component.place` (a generic family's part selects its family's fields
//! when it is made); one not yet made carries the `make` operation
//! (`library.generate_part`) that makes it, after which the same search
//! lists it as held.
use super::Session;
use super::generators::v1::Description;
use serde_json::{Value, json};
use sparksmith_core::catalog::generic::{self, generators};
use sparksmith_core::instances;

impl Session {
    /// Add the generic parts `text` reads as to a `library.search` reply,
    /// under `generic`. Nothing is added when it reads as none or no
    /// generator is shipped.
    pub(super) fn add_generic(&self, text: &str, reply: &mut Value) {
        let descriptions = self.shipped_descriptions();
        if descriptions.is_empty() {
            return;
        }
        let _context = self.engine.enter_context();
        let Ok(found) = generators::search(self.engine.document(), text, descriptions) else {
            return;
        };
        if found.is_empty() {
            return;
        }
        let project = self.engine.project();
        let library = &project.library;
        let hints: Vec<Value> = found
            .iter()
            .map(|candidate| {
                let mut hint = json!({
                    "name": candidate.name,
                    "standing": generators::standing_words(&candidate.standing),
                });
                if let Some(why) = candidate.refusal() {
                    hint["refused"] = json!(why);
                } else if let Some(component) = candidate.component() {
                    hint["component"] = json!(component);
                    // The footprint the Place palette would give it.
                    if let Some(physical) =
                        instances::first_choices(project, component).and_then(|c| c.physical)
                    {
                        hint["physical"] = json!(physical);
                    }
                    let properties = generic::properties(library, component, &candidate.values);
                    if !properties.is_empty() {
                        hint["properties"] = json!(properties);
                    }
                } else {
                    hint["make"] = json!({"op": "library.generate_part", "params": {
                        "generator": candidate.generator,
                        "scope": "component",
                        "preset": candidate.preset,
                        "params": candidate.params,
                        "name": candidate.name,
                    }});
                    if !candidate.values.is_empty() {
                        hint["values"] = json!(candidate.values);
                    }
                }
                hint
            })
            .collect();
        reply["generic"] = Value::Array(hints);
    }

    /// The shipped generators' descriptions, read once per session.
    fn shipped_descriptions(&self) -> &[Description] {
        self.descriptions.get_or_init(|| {
            let Ok(shipped) = self.shipped() else {
                return Vec::new();
            };
            let names = shipped.generators().unwrap_or_default();
            names
                .iter()
                .filter_map(|name| shipped.generator(name).ok())
                .filter_map(|bundle| self.describe(&bundle.module).ok())
                .collect()
        })
    }
}
