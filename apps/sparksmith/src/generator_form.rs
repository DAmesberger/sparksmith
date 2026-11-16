//! The one generator form (slice D1): every generator's parameters, built
//! from its description (`library.generators`) alone. Each parameter's
//! type gives its control and range (lengths in mm, angles in degrees,
//! counts, choices, switches, text, quantities and lists), its group a
//! section, its project setting a hint; the rules are listed in the
//! module's words, and a value the module would refuse is marked on its own
//! field (the core's `library::generated::complete` check, which runs no
//! module). A preset fills every value; an edited value keeps the rest.
//!
//! The form also says where the part stands in the project, as
//! `library.generate` would decide it (G15): not yet made, held (nothing
//! changes), made by an older version (making it again makes its next
//! revision; placed instances keep theirs until the update is applied), or
//! made by a newer version (refused: never downgraded). No family has code
//! of its own here.
use crate::generators::{MakeRequest, Offer};
use crate::kit;
use egui::{Response, Ui};
use sparksmith_automation::session::generators::v1;
use sparksmith_core::library::generated::{GeneratorParams, ParamProblem, ParamValue, Standing};
use std::collections::BTreeMap;

/// nm in one mm.
const NM_PER_MM: f64 = 1e6;
/// Millidegrees in one degree.
const MDEG_PER_DEG: f64 = 1e3;
/// Items a list parameter's text takes at most.
const MAX_LIST_TEXT: usize = 2048;

/// The values being edited for one generator.
#[derive(Clone, Debug, PartialEq)]
pub struct Values {
    /// The generator's id.
    pub generator: String,
    /// The preset the values started from, while none was edited away
    /// from it.
    pub preset: Option<String>,
    /// Every value the person set or the preset gave (the rest are the
    /// module's defaults and the project's settings).
    pub params: GeneratorParams,
    /// List parameters as typed, until they read.
    texts: BTreeMap<String, String>,
}

impl Values {
    /// The values of `preset` of `offer` (its defaults without one).
    pub fn of(offer: &Offer, preset: Option<&str>) -> Self {
        let params = preset
            .and_then(|key| offer.preset(key))
            .map(|p| {
                p.params
                    .iter()
                    .map(|(k, v)| (k.clone(), ParamValue::from_wire(v)))
                    .collect()
            })
            .unwrap_or_default();
        Self {
            generator: offer.description.id.clone(),
            preset: preset
                .filter(|k| offer.preset(k).is_some())
                .map(str::to_owned),
            params,
            texts: BTreeMap::new(),
        }
    }

    /// The values `request` asks for.
    pub fn of_request(offer: &Offer, request: &MakeRequest) -> Self {
        let mut values = Self::of(offer, request.preset.as_deref());
        values.params = request.given(offer);
        if !request.params.is_empty()
            && let Some(preset) = values.preset.as_deref().and_then(|k| offer.preset(k))
            && !same_as(&values.params, preset)
        {
            values.preset = None;
        }
        values
    }

    /// The request that makes these values, named `name`.
    pub fn request(&self, name: Option<String>) -> MakeRequest {
        MakeRequest {
            generator: self.generator.clone(),
            preset: None,
            params: self.params.clone(),
            name,
            values: Vec::new(),
        }
    }
}

fn same_as(params: &GeneratorParams, preset: &v1::Preset) -> bool {
    params.len() == preset.params.len()
        && params
            .iter()
            .all(|(k, v)| preset.params.get(k) == Some(&v.wire()))
}

/// What a parameter holds now: the value set, else its default.
fn current(values: &Values, param: &v1::Param) -> ParamValue {
    values
        .params
        .get(&param.key)
        .cloned()
        .unwrap_or_else(|| ParamValue::from_wire(&param.default))
}

/// A whole-number value as the control shows it, and back.
fn shown(value: i64, scale: f64) -> f64 {
    value as f64 / scale
}

fn stored(value: f64, scale: f64) -> Option<i64> {
    let v = (value * scale).round();
    (v.is_finite() && v.abs() < 9.0e15).then_some(v as i64)
}

/// A number control of `param` in `unit` at `scale` per unit over
/// `min..=max` (whole stored numbers).
fn number(
    ui: &mut Ui,
    param: &v1::Param,
    value: i64,
    unit: &str,
    scale: f64,
    (min, max): (i64, i64),
) -> (Option<i64>, Response, Option<String>) {
    let mut shown_value = shown(value, scale);
    let range = shown(min, scale)..=shown(max, scale);
    let input = kit::row(ui, &param.label, |ui| {
        kit::number_input(ui, &param.key, &mut shown_value, unit, range)
    });
    let changed = input
        .changed
        .then(|| stored(shown_value, scale))
        .flatten()
        .filter(|v| *v != value);
    (changed, input.response, input.invalid)
}

/// A list value of `of` as text: items separated by commas, lengths in mm
/// and angles in degrees with their unit, as the single-value fields show
/// them (and as [`read_list`] reads them back).
fn list_text(of: &v1::ParamType, value: &ParamValue) -> String {
    match value {
        ParamValue::List(items) => items
            .iter()
            .map(|item| match (of, item) {
                (v1::ParamType::Length { .. }, ParamValue::Int(v)) => {
                    format!("{} mm", kit::number_text(shown(*v, NM_PER_MM)))
                }
                (v1::ParamType::Angle { .. }, ParamValue::Int(v)) => {
                    format!("{}°", kit::number_text(shown(*v, MDEG_PER_DEG)))
                }
                (_, ParamValue::Int(v)) => v.to_string(),
                (_, ParamValue::Bool(v)) => v.to_string(),
                (_, ParamValue::Text(v)) => v.clone(),
                (_, ParamValue::List(_)) => String::new(),
            })
            .collect::<Vec<_>>()
            .join(", "),
        _ => String::new(),
    }
}

/// `text` as a list of `of`, when every item reads.
fn read_list(of: &v1::ParamType, text: &str) -> Option<ParamValue> {
    let items = text
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|item| match of {
            v1::ParamType::Length { .. } => kit::parse_number(item, "mm")
                .and_then(|mm| stored(mm, NM_PER_MM))
                .map(ParamValue::Int),
            v1::ParamType::Angle { .. } => kit::parse_number(item, "°")
                .and_then(|deg| stored(deg, MDEG_PER_DEG))
                .map(ParamValue::Int),
            v1::ParamType::Count { .. } => item.parse().ok().map(ParamValue::Int),
            v1::ParamType::Bool => item.parse().ok().map(ParamValue::Bool),
            _ => Some(ParamValue::Text(item.to_owned())),
        })
        .collect::<Option<Vec<_>>>()?;
    Some(ParamValue::List(items))
}

/// One parameter's row; the new value when it was edited, and the field's
/// response with why its text is invalid.
fn param_row(
    ui: &mut Ui,
    values: &mut Values,
    param: &v1::Param,
) -> (Option<ParamValue>, Option<Response>, Option<String>) {
    let value = current(values, param);
    let int = match value {
        ParamValue::Int(v) => v,
        _ => 0,
    };
    match &param.ty {
        v1::ParamType::Length { min_nm, max_nm, .. } => {
            let (new, response, invalid) =
                number(ui, param, int, "mm", NM_PER_MM, (*min_nm, *max_nm));
            (new.map(ParamValue::Int), Some(response), invalid)
        }
        v1::ParamType::Angle { min_mdeg, max_mdeg } => {
            let (new, response, invalid) =
                number(ui, param, int, "°", MDEG_PER_DEG, (*min_mdeg, *max_mdeg));
            (new.map(ParamValue::Int), Some(response), invalid)
        }
        v1::ParamType::Count { min, max } => {
            let (new, response, invalid) = number(ui, param, int, "", 1.0, (*min, *max));
            (new.map(ParamValue::Int), Some(response), invalid)
        }
        v1::ParamType::Enum { choices } => {
            let mut chosen = match &value {
                ParamValue::Text(key) => key.clone(),
                _ => String::new(),
            };
            let options: Vec<(String, String)> = choices
                .iter()
                .map(|c| (c.key.clone(), c.label.clone()))
                .collect();
            let changed = kit::choice_row(ui, &param.label, &mut chosen, &options);
            (changed.then_some(ParamValue::Text(chosen)), None, None)
        }
        v1::ParamType::Bool => {
            let mut on = matches!(value, ParamValue::Bool(true));
            let changed = kit::check_row(ui, &param.label, &mut on);
            (changed.then_some(ParamValue::Bool(on)), None, None)
        }
        v1::ParamType::Text { max_chars } => {
            let mut text = match &value {
                ParamValue::Text(t) => t.clone(),
                _ => String::new(),
            };
            let before = text.clone();
            let too_long = text.chars().count() > *max_chars as usize;
            let response = kit::checked_text_row(ui, &param.label, &mut text, "", too_long);
            let changed = text != before && text.chars().count() <= *max_chars as usize;
            (
                changed.then_some(ParamValue::Text(text)),
                Some(response),
                too_long.then(|| format!("At most {max_chars} characters")),
            )
        }
        v1::ParamType::Quantity { unit, .. } => {
            let mut text = match &value {
                ParamValue::Text(t) => t.clone(),
                _ => String::new(),
            };
            let before = text.clone();
            let response = kit::checked_text_row(ui, &param.label, &mut text, unit.as_str(), false);
            (
                (text != before).then(|| ParamValue::Text(text.trim().to_owned())),
                Some(response),
                None,
            )
        }
        v1::ParamType::List { of, max_len } => {
            let mut text = values
                .texts
                .get(&param.key)
                .cloned()
                .unwrap_or_else(|| list_text(of, &value));
            let before = text.clone();
            let read = read_list(of, &text);
            let too_many =
                matches!(&read, Some(ParamValue::List(items)) if items.len() > *max_len as usize);
            let invalid = read.is_none() || too_many || text.len() > MAX_LIST_TEXT;
            let response = kit::checked_text_row(
                ui,
                &param.label,
                &mut text,
                "items, separated by commas",
                invalid,
            );
            let mut new = None;
            if text != before {
                if let Some(list) = read_list(of, &text)
                    .filter(|l| !matches!(l, ParamValue::List(i) if i.len() > *max_len as usize))
                    .filter(|_| text.len() <= MAX_LIST_TEXT)
                {
                    new = Some(list);
                }
                values.texts.insert(param.key.clone(), text);
            }
            let why = invalid.then(|| {
                if too_many {
                    format!("At most {max_len} items")
                } else {
                    "Separate the items with commas".to_owned()
                }
            });
            (new, Some(response), why)
        }
    }
}

/// What the form did this frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Edited {
    /// A value or the preset changed.
    pub changed: bool,
}

/// The preset row, then every parameter grouped as the module groups
/// them, each problem of `problems` under its field, and the rules.
pub fn rows(ui: &mut Ui, offer: &Offer, values: &mut Values, problems: &[ParamProblem]) -> Edited {
    let mut edited = Edited::default();
    let mut preset = values.preset.clone();
    let mut options: Vec<(Option<String>, String)> = vec![(None, "Custom values".to_owned())];
    options.extend(
        offer
            .summary
            .presets
            .iter()
            .map(|p| (Some(p.key.clone()), p.name.clone())),
    );
    if kit::choice_row(ui, "Preset", &mut preset, &options) {
        if preset.is_some() {
            *values = Values::of(offer, preset.as_deref());
        } else {
            values.preset = None;
        }
        edited.changed = true;
    }
    if let Some(p) = values.preset.as_deref().and_then(|k| offer.preset(k)) {
        kit::hint(ui, &basis_line(p), &p.description);
    }
    // Groups in the order the module first names them.
    let mut groups: Vec<Option<&str>> = Vec::new();
    for param in &offer.summary.params {
        let group = param.group.as_deref();
        if !groups.contains(&group) {
            groups.push(group);
        }
    }
    for group in groups {
        let title = group.map_or_else(|| "Values".to_owned(), capitalised);
        kit::section(
            ui,
            ("generator-group", &offer.description.id, group),
            &title,
            true,
            None,
            |ui| {
                for param in offer
                    .summary
                    .params
                    .iter()
                    .filter(|p| p.group.as_deref() == group)
                {
                    let (new, response, invalid) = param_row(ui, values, param);
                    if let Some(new) = new {
                        values.params.insert(param.key.clone(), new);
                        values.preset = values.preset.take().filter(|k| {
                            offer.preset(k).is_some_and(|p| same_as(&values.params, p))
                        });
                        edited.changed = true;
                    }
                    let problem = invalid.or_else(|| {
                        problems
                            .iter()
                            .find(|p| p.field.as_deref() == Some(param.key.as_str()))
                            .map(|p| p.message.clone())
                    });
                    match (&response, &problem) {
                        // The module's sentence names values: read whole.
                        (Some(response), Some(message)) => {
                            kit::field_error_wrapped(ui, response, message, true);
                        }
                        (None, Some(message)) => {
                            kit::refusal_wrapped(ui, message);
                        }
                        _ => {}
                    }
                    if let Some(setting) = &param.context
                        && !values.params.contains_key(&param.key)
                    {
                        kit::hint(
                            ui,
                            "From the project's settings",
                            &format!("Filled from {setting} when the part is made"),
                        );
                    } else if let Some(help) = &param.help {
                        kit::hint(ui, help, help);
                    }
                }
            },
        );
    }
    if !offer.summary.rules.is_empty() {
        kit::section(
            ui,
            ("generator-rules", &offer.description.id),
            "Rules",
            false,
            Some(offer.summary.rules.len()),
            |ui| {
                for rule in &offer.summary.rules {
                    kit::hint(ui, rule, rule);
                }
            },
        );
    }
    edited
}

/// How a preset's values are sourced, in one line.
pub fn basis_line(preset: &v1::Preset) -> String {
    let source = match &preset.citation {
        Some(c) => {
            let mut text = c.document.clone();
            if let Some(revision) = &c.revision {
                text.push_str(&format!(" rev. {revision}"));
            }
            if let Some(page) = c.page {
                text.push_str(&format!(" p. {page}"));
            }
            text
        }
        None => String::new(),
    };
    match (preset.basis, source.is_empty()) {
        (v1::Basis::Cited, false) => format!("Values cited from {source}"),
        (v1::Basis::CitedUnverified, false) => {
            format!("Values cited from {source} (unverified)")
        }
        (v1::Basis::Estimated, false) => format!("Estimated values; see {source}"),
        _ => "Estimated values".to_owned(),
    }
}

fn capitalised(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// Where the part stands, in one line (the G15 rule as it applies), and
/// whether the primary can act.
pub fn standing_line(standing: &Standing, name: &str) -> (String, bool) {
    match standing {
        Standing::New => ("Not yet in this project".to_owned(), true),
        Standing::Held { root, version } => (
            format!(
                "{name} is in this project ({}, version {version}): nothing changes",
                root.revision
            ),
            true,
        ),
        Standing::Update {
            root,
            made_by,
            version,
        } => (
            format!(
                "Version {made_by} made it ({}); version {version} makes its next revision. Placed parts keep theirs until you update them.",
                root.revision
            ),
            true,
        ),
        Standing::Newer { .. } | Standing::Invalid { .. } => (
            sparksmith_core::catalog::generic::generators::refusal_of(standing).unwrap_or_default(),
            false,
        ),
    }
}
