//! A source part's value text as its part's typed fields: "10k" on a
//! resistor becomes its Resistance, "100n 16V" on a capacitor its
//! Capacitance and Rated voltage, each parsed by the core against the
//! field's type, so value templates and markings read them as they read a
//! value typed into any placed part.
use crate::import::{ImportError, Session};
use crate::report::{Case, Report};
use sparksmith_core::fields::{PropertyEdit, PropertyInput};
use sparksmith_core::{ComponentRef, Id, InstanceCommand, Operation};

/// The words of a value: split at spaces, `/`, `,` and parentheses, empty
/// ones left out (`2k(1%)` is `2k` and `1%`).
fn words(value: &str) -> Vec<&str> {
    value
        .split(|c: char| c.is_whitespace() || matches!(c, '/' | ',' | '(' | ')'))
        .filter(|w| !w.is_empty())
        .collect()
}

/// Set `value` on `instance` as the fields its component selects, in the
/// component's order: the whole text first, else each word to the first
/// field that takes it. A refused word changes nothing.
pub(crate) fn set(
    session: &mut Session<'_>,
    instance: Id,
    component: ComponentRef,
    value: &str,
    report: &mut Report,
) -> Result<(), ImportError> {
    let fields: Vec<_> = session
        .engine
        .project()
        .library
        .component(component)
        .map(|c| c.definition.fields.iter().map(|f| f.field.id).collect())
        .unwrap_or_default();
    if fields.is_empty() {
        report.add(Case::ValueWithoutField);
        return Ok(());
    }
    let mut taken = vec![false; fields.len()];
    let try_set = |session: &mut Session<'_>, field: sparksmith_core::DefinitionId, text: &str| {
        session
            .apply(
                "instance.set_properties",
                Operation::Instance(InstanceCommand::SetProperties {
                    instance,
                    edits: vec![PropertyEdit {
                        field,
                        input: PropertyInput::Text(text.to_owned()),
                    }],
                }),
            )
            .is_ok()
    };
    // The whole text, to the first field that takes it.
    for (k, field) in fields.iter().enumerate() {
        if try_set(session, *field, value.trim()) {
            taken[k] = true;
            report.add(Case::ValueField);
            return Ok(());
        }
    }
    let mut left = 0;
    for word in words(value) {
        let slot = fields
            .iter()
            .enumerate()
            .filter(|(k, _)| !taken[*k])
            .find(|(_, field)| try_set(session, **field, word))
            .map(|(k, _)| k);
        match slot {
            Some(k) => taken[k] = true,
            None => left += 1,
        }
    }
    if taken.iter().any(|t| *t) {
        report.add(Case::ValueField);
    }
    if left > 0 {
        report.add(Case::ValueWordUnread);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn values_split_into_words() {
        assert_eq!(super::words("100n 16V"), vec!["100n", "16V"]);
        assert_eq!(super::words("10k/1%"), vec!["10k", "1%"]);
        assert_eq!(super::words("0.1uF/50V(10%)"), vec!["0.1uF", "50V", "10%"]);
        assert_eq!(super::words("  "), Vec::<&str>::new());
    }
}
