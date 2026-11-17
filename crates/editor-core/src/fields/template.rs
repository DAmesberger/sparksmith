//! The value a placed part shows: a component's value template
//! (`docs/design/part-search-and-package-picker.md` §3.5). Alternatives of
//! text with field placeholders; the first whose fields all have a value
//! wins. Placeholders are stored as field lineages, so a renamed field
//! never breaks a template, and each value is formatted by its typed field.
use crate::diagnostics::{Error, Result};
use crate::ids::DefinitionId;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Alternatives one template holds.
pub const MAX_ALTERNATIVES: usize = 4;
/// Bytes of one alternative as it is written (`{Capacitance} {Rated voltage}`).
pub const MAX_ALTERNATIVE_BYTES: usize = 128;

/// One piece of an alternative.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum TemplatePiece {
    Text(String),
    /// A field lineage the component selects.
    Field(DefinitionId),
}

/// A component's value template.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ValueTemplate {
    /// 1–[`MAX_ALTERNATIVES`] alternatives, each with at least one field.
    pub alternatives: Vec<Vec<TemplatePiece>>,
}

impl ValueTemplate {
    /// Read written alternatives (`{Resistance}`), each placeholder named
    /// as `field` resolves it; a name it does not resolve is refused,
    /// naming it.
    pub fn parse(written: &[String], field: impl Fn(&str) -> Option<DefinitionId>) -> Result<Self> {
        if written.is_empty() || written.len() > MAX_ALTERNATIVES {
            return Err(Error::Invalid(format!(
                "a value template has 1–{MAX_ALTERNATIVES} alternatives"
            )));
        }
        let alternatives = written
            .iter()
            .map(|text| parse_alternative(text, &field))
            .collect::<Result<Vec<_>>>()?;
        Ok(Self { alternatives })
    }

    /// Bounds a stored template must respect.
    pub fn within_bounds(&self) -> bool {
        (1..=MAX_ALTERNATIVES).contains(&self.alternatives.len())
            && self.alternatives.iter().all(|a| {
                a.iter()
                    .map(|p| match p {
                        TemplatePiece::Text(t) => t.len(),
                        TemplatePiece::Field(_) => 0,
                    })
                    .sum::<usize>()
                    <= MAX_ALTERNATIVE_BYTES
            })
    }

    /// Every field lineage the template names.
    pub fn fields(&self) -> impl Iterator<Item = DefinitionId> + '_ {
        self.alternatives.iter().flatten().filter_map(|p| match p {
            TemplatePiece::Field(id) => Some(*id),
            TemplatePiece::Text(_) => None,
        })
    }

    /// The first alternative whose fields all have a non-empty text from
    /// `text`, filled in; `None` when none is.
    pub fn render(&self, text: impl Fn(DefinitionId) -> Option<String>) -> Option<String> {
        self.alternatives.iter().find_map(|alternative| {
            let mut out = String::new();
            for piece in alternative {
                match piece {
                    TemplatePiece::Text(t) => out.push_str(t),
                    TemplatePiece::Field(id) => {
                        out.push_str(text(*id).filter(|t| !t.trim().is_empty())?.trim());
                    }
                }
            }
            let out = out.trim().to_owned();
            (!out.is_empty()).then_some(out)
        })
    }
}

fn parse_alternative(
    text: &str,
    field: &impl Fn(&str) -> Option<DefinitionId>,
) -> Result<Vec<TemplatePiece>> {
    if text.len() > MAX_ALTERNATIVE_BYTES || text.chars().any(char::is_control) {
        return Err(Error::Invalid(format!(
            "a value template alternative is at most {MAX_ALTERNATIVE_BYTES} bytes without control characters"
        )));
    }
    let mut pieces = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        if let Some(open) = rest.strip_prefix('{') {
            let (name, after) = open.split_once('}').ok_or_else(|| {
                Error::Invalid(format!(
                    "the value template ‘{text}’ has an open placeholder"
                ))
            })?;
            let id = field(name.trim()).ok_or_else(|| {
                Error::Invalid(format!(
                    "the value template ‘{text}’ names {{{name}}}, which is not a field of the part"
                ))
            })?;
            pieces.push(TemplatePiece::Field(id));
            rest = after;
        } else {
            let end = rest.find('{').unwrap_or(rest.len());
            let (literal, after) = rest.split_at(end);
            if literal.contains('}') {
                return Err(Error::Invalid(format!(
                    "the value template ‘{text}’ closes a placeholder it never opened"
                )));
            }
            pieces.push(TemplatePiece::Text(literal.to_owned()));
            rest = after;
        }
    }
    if !pieces.iter().any(|p| matches!(p, TemplatePiece::Field(_))) {
        return Err(Error::Invalid(format!(
            "the value template ‘{text}’ names no field"
        )));
    }
    Ok(pieces)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(name: &str) -> Option<DefinitionId> {
        match name {
            "Capacitance" => Some(DefinitionId(1)),
            "Rated voltage" => Some(DefinitionId(2)),
            _ => None,
        }
    }

    #[test]
    fn the_first_filled_alternative_wins() -> Result<()> {
        let template = ValueTemplate::parse(
            &[
                "{Capacitance} {Rated voltage}".into(),
                "{Capacitance}".into(),
            ],
            named,
        )?;
        let both = |id: DefinitionId| match id.0 {
            1 => Some("1 µF".to_owned()),
            2 => Some("16 V".to_owned()),
            _ => None,
        };
        assert_eq!(template.render(both).as_deref(), Some("1 µF 16 V"));
        let one = |id: DefinitionId| (id.0 == 1).then(|| "100 nF".to_owned());
        assert_eq!(template.render(one).as_deref(), Some("100 nF"));
        assert_eq!(template.render(|_| None), None);
        Ok(())
    }

    #[test]
    fn unknown_or_malformed_placeholders_are_refused() {
        for bad in ["{Resistance}", "{Capacitance", "no field", "Capacitance}"] {
            assert!(ValueTemplate::parse(&[bad.into()], named).is_err(), "{bad}");
        }
        assert!(ValueTemplate::parse(&[], named).is_err());
        let five = vec!["{Capacitance}".to_owned(); 5];
        assert!(ValueTemplate::parse(&five, named).is_err());
    }
}
