//! The search grammar of on-demand generic parts. Each family declares its
//! patterns as data with typed slots (`header {rows}x{columns}
//! {pitch:length}?`); one matcher reads them all, and the family's
//! generator makes the part ([`generators`]).
//!
//! The families, their patterns and examples, the value slot types and the
//! defaults are data (the shipped library's `catalog/generic-families.json`,
//! [`GenericGrammar`]); this module is the matcher that reads them.
//!
//! Patterns match the whole text, word by word, with no reordering: "5x2" is
//! 5 rows of 2 columns and "2x5" another part. A size code reads as imperial
//! whatever the PCB's display units, unless spelled `1608M` or `metric 1608`.
//! Matching is bounded: at most [`MAX_WORDS`] words over patterns of at most
//! [`MAX_STEPS`] steps, each trying at most two spans.
use super::rkm;
use crate::diagnostics::{Error, Result};
use crate::library::standards::{GenericGrammar, Standards};
use crate::quantity::{self, Decimal};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Words of a text the matcher reads.
pub const MAX_WORDS: usize = 8;
/// Steps of one declared pattern.
pub const MAX_STEPS: usize = 6;

pub use crate::library::standards::{FamilyPatterns, GenericFamily};

/// A typed slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Slot {
    Count,
    Length,
    Code,
    /// A value slot type, by its index in the grammar's value slots.
    Value(usize),
    Colour,
    Package,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Piece {
    Word(String),
    Slot(String, Slot),
}

/// One whitespace-separated token of a pattern.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    pieces: Vec<Piece>,
    optional: bool,
}

/// Every declared pattern of `grammar` parses: what loading the grammar
/// checks beyond [`GenericGrammar::check`].
pub fn check(grammar: &GenericGrammar) -> Result<()> {
    grammar.check()?;
    for family in &grammar.families {
        for pattern in &family.patterns {
            parse_pattern(grammar, pattern)
                .map_err(|why| Error::Invalid(format!("the generic-part grammar: {why}")))?;
        }
    }
    Ok(())
}

/// Parse a declared pattern, or say why it does not parse.
pub fn parse_pattern(
    grammar: &GenericGrammar,
    text: &str,
) -> std::result::Result<Vec<Step>, String> {
    let steps = text
        .split_whitespace()
        .map(|token| parse_step(grammar, token))
        .collect::<std::result::Result<Vec<Step>, String>>()?;
    if steps.is_empty() || steps.len() > MAX_STEPS {
        return Err(format!("‘{text}’ needs 1–{MAX_STEPS} steps"));
    }
    let mut names: Vec<&str> = steps
        .iter()
        .flat_map(|s| &s.pieces)
        .filter_map(|p| match p {
            Piece::Slot(name, _) => Some(name.as_str()),
            Piece::Word(_) => None,
        })
        .collect();
    let slots = names.len();
    names.sort_unstable();
    names.dedup();
    if names.len() != slots {
        return Err(format!("‘{text}’ names a slot twice"));
    }
    Ok(steps)
}

fn parse_step(grammar: &GenericGrammar, token: &str) -> std::result::Result<Step, String> {
    let (body, optional) = match token.strip_suffix('?') {
        Some(body) => (body, true),
        None => (token, false),
    };
    let mut pieces = Vec::new();
    let mut rest = body;
    while !rest.is_empty() {
        if let Some(open) = rest.strip_prefix('{') {
            let (slot, after) = open
                .split_once('}')
                .ok_or_else(|| format!("‘{token}’ has an open slot"))?;
            let (name, kind) = slot.split_once(':').unwrap_or((slot, ""));
            pieces.push(Piece::Slot(
                name.to_owned(),
                slot_of(grammar, name, kind, token)?,
            ));
            rest = after;
        } else {
            let (word, after) = rest.split_at(rest.find('{').unwrap_or(rest.len()));
            pieces.push(Piece::Word(word.to_owned()));
            rest = after;
        }
    }
    let glued = pieces.len() > 1;
    if pieces.is_empty()
        || (glued
            && pieces
                .iter()
                .any(|p| matches!(p, Piece::Slot(_, s) if *s != Slot::Count)))
    {
        return Err(format!("‘{token}’: only a count slot joins text"));
    }
    Ok(Step { pieces, optional })
}

fn slot_of(
    grammar: &GenericGrammar,
    name: &str,
    kind: &str,
    token: &str,
) -> std::result::Result<Slot, String> {
    Ok(match (name, kind) {
        (_, "") => Slot::Count,
        (_, "length") => Slot::Length,
        (_, "code") => Slot::Code,
        ("colour", "list") => Slot::Colour,
        ("package", "list") => Slot::Package,
        _ => match grammar.value_slots.iter().position(|v| v.slot == kind) {
            Some(index) => Slot::Value(index),
            None => return Err(format!("‘{token}’ has an unknown slot type ‘{kind}’")),
        },
    })
}

/// What a slot read.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Bound {
    Count(u32),
    Length(Decimal),
    Code { code: String, metric: bool },
    Value(GenericValue),
    Package(String),
}

/// `×` and `x` are one letter; literals compare without case.
fn folded(word: &str) -> String {
    word.to_lowercase().replace('×', "x")
}

/// A length in the grammar's length units; a bare number is in the first.
fn read_length(grammar: &GenericGrammar, text: &str) -> Option<Bound> {
    let units = grammar.length_unit_set();
    let bare = units.units.first()?.symbol.clone();
    let parsed = quantity::parse(&folded(text), &units, &bare).ok()?;
    if parsed.si.is_zero() || parsed.si.is_negative() {
        return None;
    }
    parsed.si.shift(3).ok().map(Bound::Length)
}

fn read_code(text: &str) -> Option<Bound> {
    let text = folded(text);
    let digits = |code: &str, metric: bool| {
        ((3..=5).contains(&code.len()) && code.bytes().all(|b| b.is_ascii_digit())).then(|| {
            Bound::Code {
                code: code.to_owned(),
                metric,
            }
        })
    };
    if let Some(code) = text.strip_prefix("metric ") {
        return digits(code, true);
    }
    if let Some(code) = text.strip_prefix("imperial ") {
        return digits(code, false);
    }
    match text.strip_suffix('m') {
        Some(code) => digits(code, true),
        None => digits(&text, false),
    }
}

/// A quantity through the one parser, so it stays an exact decimal: `10k`,
/// `4k7`, `10 kΩ`. It is not read as this kind when its suffix is neither a
/// unit of the kind nor one the kind reads bare, or when it is not positive.
fn read_value(standards: &Standards, text: &str, kind: usize) -> Option<Bound> {
    let kind = standards.generic.value_slots.get(kind)?;
    let units = standards.fields.units(&kind.field)?;
    let symbol = units.units.first()?.symbol.clone();
    let spelled = text
        .replace('K', "k")
        .replace("Ohm", "ohm")
        .replace("OHM", "ohm");
    let spelled = rkm(&spelled, units).unwrap_or(spelled);
    if !spelled.starts_with(|c: char| c.is_ascii_digit() || c == '.') {
        return None;
    }
    let suffix = spelled
        .trim_start_matches(|c: char| c.is_ascii_digit() || c == '.')
        .trim();
    let bare = units.resolve(suffix).is_none();
    // "0603" alone is a size code, not 603 Ω.
    let code_like = suffix.is_empty() && spelled.starts_with('0') && !spelled.contains('.');
    if code_like || (bare && !kind.bare.iter().any(|b| b == suffix)) {
        return None;
    }
    let parsed = quantity::parse(&spelled, units, &symbol).ok()?;
    if parsed.si.is_zero() || parsed.si.is_negative() {
        return None;
    }
    Some(Bound::Value(GenericValue {
        field: kind.field.clone(),
        text: quantity::format_decimal(parsed.si, units, &symbol, None),
    }))
}

fn read_slot(standards: &Standards, slot: Slot, words: &[&str]) -> Option<Bound> {
    let text = words.join(" ");
    match slot {
        Slot::Count => None,
        Slot::Length => read_length(&standards.generic, &text),
        Slot::Code => read_code(&text),
        Slot::Value(kind) => read_value(standards, &text, kind),
        Slot::Colour => {
            let field = &standards.generic.colour_field;
            standards
                .fields
                .choices(field)?
                .iter()
                .find(|c| c.eq_ignore_ascii_case(&text))
                .map(|colour| {
                    Bound::Value(GenericValue {
                        field: field.clone(),
                        text: colour.clone(),
                    })
                })
        }
        // A listed package is one word; the family's rows or presets
        // decide whether it names one.
        Slot::Package => (words.len() == 1).then(|| Bound::Package(text.clone())),
    }
}

/// One word against a step of joined pieces (`{rows}x{columns}`).
fn read_glued(pieces: &[Piece], word: &str) -> Option<Vec<(String, Bound)>> {
    let word = folded(word);
    let mut rest = word.as_str();
    let mut out = Vec::new();
    for piece in pieces {
        match piece {
            Piece::Word(text) => rest = rest.strip_prefix(text.as_str())?,
            Piece::Slot(name, _) => {
                let end = rest
                    .find(|c: char| !c.is_ascii_digit())
                    .unwrap_or(rest.len());
                let (digits, after) = rest.split_at(end);
                if digits.is_empty() {
                    return None;
                }
                // More digits than a count holds still reach the form's own
                // refusal, as a count beyond any limit.
                let count = digits.parse().unwrap_or(u32::MAX);
                out.push((name.clone(), Bound::Count(count)));
                rest = after;
            }
        }
    }
    rest.is_empty().then_some(out)
}

fn walk(
    standards: &Standards,
    steps: &[Step],
    words: &[&str],
    found: &mut Vec<(String, Bound)>,
) -> bool {
    let Some((step, rest)) = steps.split_first() else {
        return words.is_empty();
    };
    // A value or a length may be two words ("10 kΩ"), a code `metric 1608`.
    for span in 1..=2 {
        let (Some(taken), Some(left)) = (words.get(..span), words.get(span..)) else {
            break;
        };
        let bound = match (step.pieces.as_slice(), taken) {
            ([Piece::Slot(name, slot)], _) if *slot != Slot::Count => {
                read_slot(standards, *slot, taken).map(|b| vec![(name.clone(), b)])
            }
            (pieces, [word]) => read_glued(pieces, word),
            _ => None,
        };
        if let Some(bound) = bound {
            let kept = found.len();
            found.extend(bound);
            if walk(standards, rest, left, found) {
                return true;
            }
            found.truncate(kept);
        }
    }
    step.optional && walk(standards, rest, words, found)
}

/// One interpretation badge of a result row.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GenericBadge {
    pub label: String,
    /// What it means, in the row's hover.
    pub tip: String,
}

/// One value a query gave a placed instance, as field text core parses.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GenericValue {
    /// The field's name, as the standard fields spell it.
    pub field: String,
    pub text: String,
}

/// The instance property edits of `values`, for the fields `component`
/// selects by these names; a value with no such field is left out.
pub fn properties(
    library: &crate::library::Library,
    component: crate::ids::ComponentRef,
    values: &[GenericValue],
) -> Vec<crate::fields::PropertyEdit> {
    let Some(published) = library.component(component) else {
        return Vec::new();
    };
    values
        .iter()
        .filter_map(|value| {
            let selection = published.definition.fields.iter().find(|s| {
                library
                    .field(s.field)
                    .is_some_and(|f| f.meta.name == value.field)
            })?;
            Some(crate::fields::PropertyEdit {
                field: selection.field.id,
                input: crate::fields::PropertyInput::Text(value.text.clone()),
            })
        })
        .collect()
}

fn badge(label: impl Into<String>, tip: impl Into<String>) -> GenericBadge {
    GenericBadge {
        label: label.into(),
        tip: tip.into(),
    }
}

pub mod generators;

#[cfg(test)]
mod tests;
