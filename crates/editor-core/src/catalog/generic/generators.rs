//! The catalog over generators (slice D1 of the part-generation plan): the
//! words of a component search read as a generator's part, and the
//! generators' presets a search finds. The host hands in the descriptions
//! of the generators it ships (`library.generators`); the core never runs a
//! module. Each result carries the generator, the parameters to make it
//! with (`library.generate_part`) and where that part stands in the
//! project ([`Standing`]): held, made by an older or a newer version, or
//! not yet made.
//!
//! A generic family names its generator in the grammar
//! (`catalog/generic-families.json`); a size code or a listed package picks
//! one of the generator's presets, counts and lengths fill parameters, and
//! the family's own parameters go on top. Nothing here knows a family.
use super::{
    Bound, GenericBadge, GenericFamily, GenericValue, MAX_WORDS, badge, parse_pattern, walk,
};
use crate::catalog::MAX_QUERY;
use crate::diagnostics::{Error, Result};
use crate::ids::{ComponentRef, DefinitionKind};
use crate::library::generated::{self, GeneratorParams, GeneratorScope, ParamValue, Standing};
use crate::library::standards::{self, FamilyPatterns, GenericGrammar, Standards};
use crate::model::Document;
use crate::quantity::Decimal;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sparksmith_generator_abi::v1;

/// Presets one preset search lists at most.
pub const MAX_PRESET_HITS: usize = 200;

/// One part a text reads as, made by a generator.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct GeneratorCandidate {
    pub family: GenericFamily,
    /// The declared pattern that read the text.
    pub pattern: String,
    /// The generator's id (`library.generate_part`'s `generator`).
    pub generator: String,
    /// Its version, as described.
    pub version: u32,
    /// The preset the parameters start from, if one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
    /// The parameters to make it with: the preset's, the text's and the
    /// family's.
    pub params: GeneratorParams,
    /// The part's name.
    pub name: String,
    /// The dim line: where it stands and how its values are sourced, or why
    /// it cannot be made.
    pub detail: String,
    pub badges: Vec<GenericBadge>,
    /// Values for each placed instance, not part of the identity.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub values: Vec<GenericValue>,
    pub standing: Standing,
    /// The preset this part is exactly, not to list beside it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub covers_preset: Option<String>,
}

impl GeneratorCandidate {
    /// The project's component this part is, when it holds one.
    pub fn component(&self) -> Option<ComponentRef> {
        component_of(&self.standing)
    }

    /// Why it cannot be made, when it cannot.
    pub fn refusal(&self) -> Option<String> {
        refusal_of(&self.standing)
    }
}

/// The project's component a standing names, if it names one.
pub fn component_of(standing: &Standing) -> Option<ComponentRef> {
    standing
        .root()
        .filter(|r| r.kind == DefinitionKind::Component)
        .map(|r| ComponentRef::new(r.id, r.revision))
}

/// Why a standing cannot be made, in the module's or the rule's words.
pub fn refusal_of(standing: &Standing) -> Option<String> {
    match standing {
        Standing::Invalid { problems } => Some(problems.first().map_or_else(
            || "These values cannot be made".into(),
            |p| p.message.clone(),
        )),
        Standing::Newer {
            made_by, version, ..
        } => Some(format!(
            "Made by version {made_by} of its generator; this program has version {version}, which never downgrades it"
        )),
        _ => None,
    }
}

/// Where `standing` leaves a part, in a few words for a list's dim line.
pub fn standing_words(standing: &Standing) -> String {
    match standing {
        Standing::New => "Not yet in this project".into(),
        Standing::Held { root, version } => {
            format!("In this project, {} · version {version}", root.revision)
        }
        Standing::Update {
            root,
            made_by,
            version,
        } => format!(
            "In this project, {} by version {made_by}; version {version} makes its next revision",
            root.revision
        ),
        Standing::Newer { .. } | Standing::Invalid { .. } => {
            refusal_of(standing).unwrap_or_default()
        }
    }
}

/// The standing of `params` for `description`'s module in `scope` on
/// `document`.
pub fn standing(
    document: &Document,
    description: &v1::Description,
    scope: GeneratorScope,
    params: &GeneratorParams,
) -> Standing {
    generated::standing(
        &document.project.library,
        &document.project.rules,
        &document.drafts,
        description,
        scope,
        params,
    )
}

fn wire_eq(given: &ParamValue, preset: Option<&v1::ParamValue>) -> bool {
    preset == Some(&given.wire())
}

/// The family's presets: those whose parameters include the family's.
fn family_presets<'a>(
    family: &FamilyPatterns,
    description: &'a v1::Description,
) -> Vec<&'a v1::Preset> {
    description
        .presets()
        .iter()
        .filter(|p| {
            family
                .presets
                .iter()
                .all(|(k, v)| wire_eq(v, p.params.get(k)))
        })
        .collect()
}

fn same_word(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

/// A length in mm as integer nanometres, when it is a whole number of them.
fn nm_of(mm: Decimal) -> Option<i64> {
    let (mantissa, exponent) = mm.shift(6).ok()?.parts();
    let exponent = u32::try_from(exponent).ok()?;
    mantissa.checked_mul(10_i64.checked_pow(exponent)?)
}

/// What the bound slots ask of the generator.
struct Asked<'a> {
    preset: Option<&'a v1::Preset>,
    params: GeneratorParams,
    /// Each slot's words for the label.
    words: Vec<(String, String)>,
    badges: Vec<GenericBadge>,
    /// Why the text cannot be made, when a size names no preset.
    refused: Option<String>,
}

fn asked<'a>(
    grammar: &GenericGrammar,
    family: &FamilyPatterns,
    presets: &[&'a v1::Preset],
    bounds: &[(String, Bound)],
) -> Option<Asked<'a>> {
    let mut out = Asked {
        preset: None,
        params: GeneratorParams::new(),
        words: Vec::new(),
        badges: Vec::new(),
        refused: None,
    };
    let targets = |slot: &str| {
        family
            .slots
            .get(slot)
            .cloned()
            .unwrap_or_else(|| vec![slot.to_owned()])
    };
    let mut lengths: Vec<String> = Vec::new();
    let mut length = |name: &str, mm: Decimal, out: &mut Asked<'a>| -> Option<()> {
        out.words.push((name.to_owned(), mm.to_string()));
        let nm = nm_of(mm)?;
        for key in targets(name) {
            lengths.push(key.clone());
            out.params.insert(key, ParamValue::Int(nm));
        }
        Some(())
    };
    for (name, bound) in bounds {
        match bound {
            Bound::Count(count) => {
                out.words.push((name.clone(), count.to_string()));
                for key in targets(name) {
                    out.params.insert(key, ParamValue::Int(i64::from(*count)));
                }
            }
            Bound::Length(mm) => {
                length(name, *mm, &mut out)?;
                out.badges
                    .push(badge(format!("{mm} mm"), "The length as typed"));
            }
            Bound::Code { code, metric } => {
                let (alias, shown, system, tip) = if *metric {
                    (
                        grammar.metric_alias.replace("{code}", code),
                        format!("{code}M"),
                        "metric",
                        format!("{code}M is a metric size code"),
                    )
                } else {
                    (
                        code.clone(),
                        code.clone(),
                        "imperial",
                        format!("Size codes read as imperial; {code}M names the metric {code}"),
                    )
                };
                out.badges.push(badge(system, tip));
                out.words.push((name.clone(), shown.clone()));
                match presets
                    .iter()
                    .find(|p| p.aliases.iter().any(|a| same_word(a, &alias)))
                {
                    Some(preset) => out.preset = Some(preset),
                    None => {
                        let sizes: Vec<&str> = presets
                            .iter()
                            .filter_map(|p| p.aliases.first().map(String::as_str))
                            .collect();
                        out.refused = Some(format!(
                            "No size {shown} among this generator's presets: {}.",
                            sizes.join(", ")
                        ));
                    }
                }
            }
            Bound::Package(word) => {
                let preset = presets.iter().find(|p| {
                    p.name
                        .split_whitespace()
                        .next()
                        .is_some_and(|w| same_word(w, word))
                        || p.aliases.iter().any(|a| same_word(a, word))
                })?;
                // The word as the preset spells it: its name's first word,
                // else the alias the text named.
                let first = preset
                    .name
                    .split_whitespace()
                    .next()
                    .filter(|w| same_word(w, word))
                    .or_else(|| {
                        preset
                            .aliases
                            .iter()
                            .find(|a| same_word(a, word))
                            .map(String::as_str)
                    })
                    .unwrap_or(word);
                out.words.push((name.clone(), first.to_owned()));
                out.preset = Some(preset);
            }
            Bound::Value(_) => {}
        }
    }
    // A pitch the text left out is the grammar's default, and said so.
    let typed = |slot: &str| out.words.iter().any(|(n, _)| n == slot);
    if family.patterns.iter().any(|p| p.contains("{pitch:length}")) && !typed("pitch") {
        let mm: Decimal = grammar.header_pitch_mm.parse().ok()?;
        length("pitch", mm, &mut out)?;
        out.badges.push(badge(
            format!("{mm} mm (default)"),
            "No pitch typed: add one, such as 2 mm",
        ));
    }
    // Counts and lengths start from a preset that has them all, else one
    // with the same lengths (proportions of that pitch), else the module's
    // defaults.
    if out.preset.is_none() && !out.params.is_empty() {
        let agrees = |p: &v1::Preset, keys: &[&String]| {
            keys.iter().all(|k| {
                out.params
                    .get(*k)
                    .is_some_and(|v| wire_eq(v, p.params.get(*k)))
            })
        };
        let all: Vec<&String> = out.params.keys().collect();
        let lengths: Vec<&String> = lengths.iter().collect();
        out.preset = presets
            .iter()
            .find(|p| agrees(p, &all))
            .or_else(|| {
                (!lengths.is_empty())
                    .then(|| presets.iter().find(|p| agrees(p, &lengths)))
                    .flatten()
            })
            .copied();
    }
    if let (Some((_, rows)), Some((_, columns))) = (
        out.words.iter().find(|(n, _)| n == "rows"),
        out.words.iter().find(|(n, _)| n == "columns"),
    ) {
        let tip = format!("{rows} rows of {columns} pins; {columns}x{rows} is a different part");
        out.badges.insert(0, badge("rows × columns", tip));
    }
    Some(out)
}

/// The family's label with the text's words.
fn label(family: &FamilyPatterns, words: &[(String, String)]) -> String {
    let mut text = family.label.clone();
    for (name, word) in words {
        text = text.replace(&format!("{{{name}}}"), word);
    }
    text
}

/// `length × width` of the body, from the parameters the family names.
fn body_badge(family: &FamilyPatterns, params: &v1::Params) -> Option<GenericBadge> {
    let [length, width] = family.body.as_slice() else {
        return None;
    };
    let mm = |key: &String| match params.get(key) {
        Some(v1::ParamValue::Int(nm)) => Decimal::from_f64(*nm as f64 / 1e6)
            .ok()
            .map(|d| d.to_string()),
        _ => None,
    };
    Some(badge(
        format!("{} × {} mm", mm(length)?, mm(width)?),
        "Body length × width of this package",
    ))
}

fn basis_words(basis: v1::Basis) -> &'static str {
    match basis {
        v1::Basis::Cited => "cited values",
        v1::Basis::CitedUnverified => "cited values (unverified)",
        v1::Basis::Estimated => "estimated values",
    }
}

/// The values a text gave each placed instance, as badges and values.
fn value_parts(bounds: &[(String, Bound)]) -> (Vec<GenericBadge>, Vec<GenericValue>) {
    let values: Vec<GenericValue> = bounds
        .iter()
        .filter_map(|(_, b)| match b {
            Bound::Value(v) => Some(v.clone()),
            _ => None,
        })
        .collect();
    let badges = values
        .iter()
        .map(|v| badge(&v.text, format!("{} of each instance you place", v.field)))
        .collect();
    (badges, values)
}

fn candidate(
    standards: &Standards,
    document: &Document,
    family: &FamilyPatterns,
    description: &v1::Description,
    pattern: &str,
    bounds: &[(String, Bound)],
) -> Option<GeneratorCandidate> {
    let presets = family_presets(family, description);
    let asked = asked(&standards.generic, family, &presets, bounds)?;
    let (value_badges, values) = value_parts(bounds);
    let name = label(family, &asked.words);
    let mut params: GeneratorParams = asked
        .preset
        .map(|p| {
            p.params
                .iter()
                .map(|(k, v)| (k.clone(), ParamValue::from_wire(v)))
                .collect()
        })
        .unwrap_or_default();
    params.extend(asked.params.clone());
    params.extend(family.params.clone());
    let covers_preset = asked
        .preset
        .filter(|p| {
            params.len() == p.params.len()
                && params.iter().all(|(k, v)| wire_eq(v, p.params.get(k)))
        })
        .map(|p| p.key.clone());
    let literal = pattern.split_whitespace().any(|t| !t.contains('{'));
    let standing = match &asked.refused {
        Some(reason) => Standing::Invalid {
            problems: vec![generated::ParamProblem {
                field: None,
                message: reason.clone(),
            }],
        },
        None => standing(document, description, GeneratorScope::Component, &params),
    };
    // A word of the family's own makes this a request for this family; a
    // pattern of slots alone may simply be another search.
    if matches!(standing, Standing::Invalid { .. }) && !literal {
        return None;
    }
    let mut badges = asked.badges;
    let wire = generated::wire_params(&params);
    badges.extend(body_badge(family, &wire));
    badges.extend(value_badges);
    let basis = match (asked.preset, &covers_preset) {
        (Some(p), Some(_)) => basis_words(p.basis),
        _ => "estimated values",
    };
    let detail = match &standing {
        Standing::Invalid { .. } | Standing::Newer { .. } => standing_words(&standing),
        _ => format!(
            "{} · {} generator v{} · {basis}",
            standing_words(&standing),
            description.title,
            description.version
        ),
    };
    let name = match component_of(&standing) {
        Some(component) => document
            .project
            .library
            .component(component)
            .map_or(name, |c| c.meta.name.clone()),
        None => name,
    };
    Some(GeneratorCandidate {
        family: family.family,
        pattern: pattern.to_owned(),
        generator: description.id.clone(),
        version: description.version,
        preset: asked.preset.map(|p| p.key.clone()),
        params,
        name,
        detail,
        badges,
        values,
        standing,
        covers_preset,
    })
}

/// Every part `query` reads as under the declared patterns, each made by
/// the generator its family names among `descriptions`, in family order;
/// none when it reads as none. A family whose generator is not among them
/// is skipped. The grammar is the installed standards'.
pub fn search(
    document: &Document,
    query: &str,
    descriptions: &[v1::Description],
) -> Result<Vec<GeneratorCandidate>> {
    let words: Vec<&str> = query.split_whitespace().collect();
    if query.len() > MAX_QUERY || words.is_empty() || words.len() > MAX_WORDS {
        return Ok(Vec::new());
    }
    let standards = standards::get()?;
    let mut candidates: Vec<GeneratorCandidate> = Vec::new();
    for family in &standards.generic.families {
        let Some(description) = descriptions.iter().find(|d| d.id == family.generator) else {
            continue;
        };
        for pattern in &family.patterns {
            let steps = parse_pattern(&standards.generic, pattern)
                .map_err(|why| Error::Invalid(format!("the generic-part grammar: {why}")))?;
            let mut bounds = Vec::new();
            if !walk(&standards, &steps, &words, &mut bounds) {
                continue;
            }
            let Some(found) =
                candidate(&standards, document, family, description, pattern, &bounds)
            else {
                continue;
            };
            if !candidates.iter().any(|c| {
                c.generator == found.generator
                    && c.params == found.params
                    && c.values == found.values
            }) {
                candidates.push(found);
            }
        }
    }
    Ok(candidates)
}

/// One preset a search found.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PresetHit {
    pub generator: String,
    pub version: u32,
    /// The generator's display name.
    pub title: String,
    pub preset: String,
    pub name: String,
    /// Its values' source and where the part stands.
    pub detail: String,
    pub basis: v1::Basis,
    pub standing: Standing,
}

/// The metric preset alias of each size code among `query`'s `words`
/// (lower case), with the code's place: a bare size code reads as imperial
/// (the grammar's rule), so it does not name a preset by its metric alias
/// ("0603 metric", the 0201) unless the text says metric itself.
pub fn metric_aliases(query: &str, words: &[String]) -> Vec<(usize, String)> {
    let lowered = query.to_lowercase();
    standards::get()
        .map(|s| s.generic.metric_alias.to_lowercase())
        .map(|pattern| {
            words
                .iter()
                .enumerate()
                .filter(|(_, w)| {
                    (3..=5).contains(&w.len()) && w.bytes().all(|b| b.is_ascii_digit())
                })
                .map(|(k, w)| (k, pattern.replace("{code}", w)))
                .filter(|(_, alias)| !lowered.contains(alias.as_str()))
                .collect()
        })
        .unwrap_or_default()
}

/// The presets of `descriptions` every word of `query` names (in the
/// preset's name, aliases or description, or the generator's title or
/// families), whole-word matches of an alias first; at most `limit`, and
/// how many more. An empty text finds nothing.
pub fn find_presets(
    document: &Document,
    query: &str,
    descriptions: &[v1::Description],
    limit: usize,
) -> (Vec<PresetHit>, usize) {
    let words: Vec<String> = query
        .split_whitespace()
        .map(|w| w.to_lowercase().replace('×', "x"))
        .collect();
    if query.len() > MAX_QUERY || words.is_empty() || words.len() > MAX_WORDS {
        return (Vec::new(), 0);
    }
    let metric = metric_aliases(query, &words);
    let mut found: Vec<(bool, &v1::Description, &v1::Preset)> = Vec::new();
    for description in descriptions {
        let families = description
            .offer()
            .map(|o| o.families.join(" "))
            .unwrap_or_default();
        for preset in description.presets() {
            let text = format!(
                "{} {} {} {} {}",
                preset.name,
                preset.aliases.join(" "),
                preset.description,
                description.title,
                families
            )
            .to_lowercase()
            .replace('×', "x");
            let names = |k: usize, w: &str| match metric.iter().find(|(m, _)| *m == k) {
                Some((_, alias)) => text.replace(alias.as_str(), "").contains(w),
                None => text.contains(w),
            };
            if words.iter().enumerate().all(|(k, w)| names(k, w)) {
                let exact = preset
                    .aliases
                    .iter()
                    .chain(std::iter::once(&preset.name))
                    .any(|a| same_word(a, query));
                found.push((exact, description, preset));
            }
        }
    }
    found.sort_by_key(|(exact, _, _)| !exact);
    let limit = limit.min(MAX_PRESET_HITS);
    let more = found.len().saturating_sub(limit);
    let hits = found
        .into_iter()
        .take(limit)
        .map(|(_, description, preset)| preset_hit(document, description, preset))
        .collect();
    (hits, more)
}

/// A preset as a hit, with where its part stands.
pub fn preset_hit(
    document: &Document,
    description: &v1::Description,
    preset: &v1::Preset,
) -> PresetHit {
    let params: GeneratorParams = preset
        .params
        .iter()
        .map(|(k, v)| (k.clone(), ParamValue::from_wire(v)))
        .collect();
    let standing = standing(document, description, GeneratorScope::Component, &params);
    let detail = match &standing {
        Standing::New => format!(
            "{} generator v{} · {}",
            description.title,
            description.version,
            basis_words(preset.basis)
        ),
        _ => standing_words(&standing),
    };
    PresetHit {
        generator: description.id.clone(),
        version: description.version,
        title: description.title.clone(),
        preset: preset.key.clone(),
        name: preset.name.clone(),
        detail,
        basis: preset.basis,
        standing,
    }
}
