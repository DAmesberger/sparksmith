//! SVG artwork templates: artwork whose text and groups follow the placed
//! part.
//!
//! Packed marking artwork may hold the text templates' placeholders in its
//! text (`<text>{mpn}</text>`, `<tspan>{code}</tspan>`) and conditions on
//! any element (`data-if="mpn^=ESP32-S3"`, `data-if="!code"`): an element
//! whose condition does not hold is not drawn, with its children, so whole
//! groups are shown, hidden or swapped by the part. Conditions are the
//! templates' ([`super::template`]): a field's number (`field:D8<=1%`), a
//! text the part gives compared as part numbers are (`mpn=`, `^=` starts,
//! `$=` ends, `*=` contains), a name alone (it has a value), `!` negating.
//!
//! The core binds them: [`bindings`] reads the placeholders and conditions
//! an artwork names and gives each its value for one placed part. The
//! layout carries the values with the artwork
//! ([`super::LaidElement::Artwork`]), so the drawing side substitutes them
//! without knowing the template language, and its decal cache is keyed by
//! what is drawn. A placeholder with no value draws nothing (never another
//! part's text), and a condition that cannot be read does not hold.
//!
//! Further artwork is referenced by id (`<use data-art="ID" x y width
//! height/>`): the drawing side fits the installed library's artwork `ID`
//! into the box, or draws nothing there when that artwork is not
//! installed. The core keeps no artwork of its own.
use super::MarkingInputs;
use super::template::{self, Missing};
use std::collections::BTreeMap;

/// Placeholders and conditions one artwork may bind.
pub const MAX_BINDINGS: usize = 64;
/// Longest placeholder or condition read, bytes.
const MAX_EXPRESSION: usize = 256;
/// The attribute a condition is written in.
pub const CONDITION_ATTRIBUTE: &str = "data-if";
/// The attribute that names further artwork by id on a `use` element.
pub const ART_ATTRIBUTE: &str = "data-art";

/// The key a condition's value is bound under: `?` and the condition as
/// written, trimmed. A placeholder's key is its body, trimmed.
pub fn condition_key(condition: &str) -> String {
    format!("?{}", condition.trim())
}

/// Whether `svg` may be a template (names a placeholder or a condition).
pub fn is_template(svg: &str) -> bool {
    svg.contains('{') || svg.contains(CONDITION_ATTRIBUTE)
}

/// The value of each placeholder and condition `svg` names, for `inputs`:
/// a placeholder's text (empty when the part has no value for it), and
/// `"1"` for a condition that holds, `""` for one that does not. Braces
/// that are not a placeholder (a style sheet's) and `{{` are skipped; at
/// most [`MAX_BINDINGS`] are read.
pub fn bindings(svg: &str, inputs: &MarkingInputs) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    if !is_template(svg) {
        return out;
    }
    for body in placeholders(svg) {
        if out.len() >= MAX_BINDINGS {
            break;
        }
        let Ok(pieces) = template::parse(&format!("{{{body}}}")) else {
            continue;
        };
        let text = match template::render(&pieces, inputs) {
            Ok(text) => text,
            Err(Missing::Value(_) | Missing::Code(_)) => String::new(),
        };
        out.insert(body.to_owned(), text);
    }
    for expression in conditions(svg) {
        if out.len() >= MAX_BINDINGS {
            break;
        }
        let held = template::condition(expression).is_ok_and(|c| template::holds(&c, inputs));
        out.insert(
            condition_key(expression),
            if held { "1".into() } else { String::new() },
        );
    }
    out
}

/// The bodies of `{…}` in `svg`, trimmed, `{{` skipped.
fn placeholders(svg: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = svg;
    while let Some(at) = rest.find('{') {
        let after = &rest[at + 1..];
        if after.starts_with('{') {
            rest = &after[1..];
            continue;
        }
        let Some(end) = after.find(['}', '{', '<', '>']) else {
            break;
        };
        if after[end..].starts_with('}') && end <= MAX_EXPRESSION {
            let body = after[..end].trim();
            if !body.is_empty() && !out.contains(&body) {
                out.push(body);
            }
        }
        rest = &after[end..];
    }
    out
}

/// The values of every `data-if` attribute in `svg`, trimmed.
fn conditions(svg: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = svg;
    while let Some(at) = rest.find(CONDITION_ATTRIBUTE) {
        let after = rest[at + CONDITION_ATTRIBUTE.len()..].trim_start();
        rest = after;
        let Some(after) = after.strip_prefix('=') else {
            continue;
        };
        let after = after.trim_start();
        let Some(quote) = after.chars().next().filter(|c| *c == '"' || *c == '\'') else {
            continue;
        };
        let value = &after[1..];
        let Some(end) = value.find(quote) else {
            break;
        };
        let expression = value[..end].trim();
        if !expression.is_empty() && end <= MAX_EXPRESSION && !out.contains(&expression) {
            out.push(expression);
        }
        rest = &value[end..];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::DefinitionId;

    /// An original synthetic shield label: the name and variant follow the
    /// part number, one group shows only for one family, one only without
    /// a code.
    const LABEL: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 10">
<style>.a{fill:#000}</style>
<use data-art="maker-logo" x="0" y="0" width="6" height="3"/>
<text x="10" y="5">{mpn}</text>
<g data-if="code"><text x="10" y="7">{code}</text></g>
<g data-if='!code'><text x="10" y="7">{{std}}</text></g>
<g data-if="mpn^=ACME-S3"><text x="10" y="9">S3 family</text></g>
<g data-if="field:D7<=1%"><rect width="1" height="1"/></g>
<text>{nonsense:x}</text>
</svg>"#;

    fn inputs(mpn: Option<&str>) -> MarkingInputs {
        let mut codes = BTreeMap::new();
        codes.insert(super::super::code_key("ACME-S3-MOD-N16R8"), "N16R8".into());
        let mut fields = BTreeMap::new();
        fields.insert(DefinitionId(7), ("1 %".into(), Some(0.01)));
        MarkingInputs {
            mpn: mpn.map(str::to_owned),
            codes,
            fields,
            ..MarkingInputs::default()
        }
    }

    #[test]
    fn placeholders_and_conditions_bind_to_the_placed_part() {
        let bound = bindings(LABEL, &inputs(Some("ACME-S3-MOD-N16R8")));
        let get = |k: &str| bound.get(k).map(String::as_str);
        assert_eq!(get("mpn"), Some("ACME-S3-MOD-N16R8"));
        assert_eq!(get("code"), Some("N16R8"));
        assert_eq!(get("?code"), Some("1"));
        assert_eq!(get("?!code"), Some(""));
        assert_eq!(get("?mpn^=ACME-S3"), Some("1"));
        assert_eq!(get("?field:D7<=1%"), Some("1"));
        // A style sheet's braces, an escaped brace and an unknown
        // placeholder bind nothing.
        assert_eq!(get("fill:#000"), None);
        assert_eq!(get("std"), None);
        assert_eq!(get("nonsense:x"), None);
        assert_eq!(bound.len(), 6, "{bound:?}");
    }

    #[test]
    fn another_variant_swaps_the_groups_and_never_shows_a_wrong_code() {
        let bound = bindings(LABEL, &inputs(Some("ACME-C6-MOD")));
        let get = |k: &str| bound.get(k).map(String::as_str);
        assert_eq!(get("mpn"), Some("ACME-C6-MOD"));
        assert_eq!(get("code"), Some(""));
        assert_eq!(get("?code"), Some(""));
        assert_eq!(get("?!code"), Some("1"));
        assert_eq!(get("?mpn^=ACME-S3"), Some(""));
        // No part number: no text, and no condition on it holds.
        let bound = bindings(LABEL, &inputs(None));
        assert_eq!(bound.get("mpn").map(String::as_str), Some(""));
        assert_eq!(bound.get("?mpn^=ACME-S3").map(String::as_str), Some(""));
    }

    #[test]
    fn plain_artwork_binds_nothing_and_bindings_are_bounded() {
        assert!(bindings("<svg><rect/></svg>", &inputs(Some("X"))).is_empty());
        let many: String = (0..200)
            .map(|i| format!("<g data-if=\"mpn=P{i}\"/>"))
            .collect();
        assert_eq!(
            bindings(&format!("<svg>{many}</svg>"), &inputs(Some("P1"))).len(),
            MAX_BINDINGS
        );
    }
}
