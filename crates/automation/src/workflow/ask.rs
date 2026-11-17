//! `assist.ask`: the chat's question card (`docs/assistant-panel.md`
//! §3.4). The model asks the person what it needs to know before it
//! designs: one card with one or more questions, each a single choice
//! (options with a short plain explanation, one recommended), a multiple
//! choice, a number with its unit and range, or a short text. The card is
//! drawn with the same compact controls as a workflow's inputs; the
//! person answers on it, or in their own words in the composer, and the
//! answers come back to the model as the call's reply.
//!
//! The front door checks the card here and the window shows it; a window
//! without a chat (live MCP) answers `host.unavailable`, so an external
//! agent asks in its own conversation.
use serde::{Deserialize, Serialize};
use serde_json::{Map, Number, Value};

/// The longest id, title, option value or recommended text, in bytes.
pub const MAX_LABEL: usize = 120;
/// The longest question text, explanation or `why`, in bytes.
pub const MAX_TEXT: usize = 600;
/// The most bytes the whole card may take as JSON: a bound for safe
/// handling, not a count of questions.
pub const MAX_CARD_BYTES: usize = 64 * 1024;
/// The longest short-text answer, in bytes.
pub const MAX_ANSWER: usize = 400;

/// `assist.ask`: put a question card in the chat and wait for the
/// person's answers.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AskParams {
    /// The card's heading, a few plain words ("A few things first").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// One sentence under the heading on why these matter, in everyday
    /// words; may be left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
    /// The questions, in the order they show.
    pub questions: Vec<Question>,
}

/// One question on the card. (Its kind is flattened in, so unknown fields
/// cannot be refused here; the kind's tag is.)
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Question {
    /// The key of its answer ("power").
    pub id: String,
    /// Its short label on the card ("Power").
    pub title: String,
    /// The question in plain words, shown under the label when given
    /// ("How will it get its power?").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(flatten)]
    pub kind: Kind,
}

/// What kind of answer a question takes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Kind {
    /// One of the options.
    Single {
        options: Vec<AskOption>,
        /// The option's value chosen until the person changes it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        recommended: Option<String>,
    },
    /// Any of the options.
    Multiple {
        options: Vec<AskOption>,
        /// The options' values chosen until the person changes them.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        recommended: Vec<String>,
    },
    /// A number, in `unit` ("minutes", "°C", "hours"), within `min` and
    /// `max` when given.
    Number {
        #[serde(default, skip_serializing_if = "String::is_empty")]
        unit: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min: Option<Number>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<Number>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        recommended: Option<Number>,
    },
    /// A short line of text.
    Text {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        recommended: Option<String>,
    },
}

/// One option of a choice.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AskOption {
    /// What the answer carries ("usb").
    pub value: String,
    /// What the card shows ("USB").
    pub title: String,
    /// A short plain explanation ("plugs into a phone charger").
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub about: String,
}

/// The reply of `assist.ask`: the card's id and how the person answered:
/// on the card (`answers`, by question id), in their own words in the chat
/// (`said`), or not at all (`skipped`: choose sensibly and say what).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AskReply {
    pub card: String,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub answers: Map<String, Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub said: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub skipped: bool,
}

impl AskParams {
    /// Whether the card can be shown: at least one question, ids unique
    /// and plain, every label and text bounded, choices with two options
    /// or more and unique values, recommended values among the options and
    /// numbers within their range. The reason names the question.
    pub fn check(&self) -> Result<(), String> {
        let bytes = serde_json::to_vec(self).map_or(usize::MAX, |b| b.len());
        if bytes > MAX_CARD_BYTES {
            return Err(format!(
                "the card is {bytes} bytes; at most {MAX_CARD_BYTES}: shorten the texts"
            ));
        }
        if self.questions.is_empty() {
            return Err("give at least one question".into());
        }
        bounded(
            "title",
            self.title.as_deref().unwrap_or_default(),
            MAX_LABEL,
        )?;
        bounded("why", self.why.as_deref().unwrap_or_default(), MAX_TEXT)?;
        let mut ids: Vec<&str> = Vec::new();
        for question in &self.questions {
            let id = question.id.as_str();
            let plain = !id.is_empty()
                && id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
            if !plain || id.len() > MAX_LABEL {
                return Err(format!(
                    "question id \"{}\": use letters, digits, _ or -, at most {MAX_LABEL}",
                    cut(id)
                ));
            }
            if ids.contains(&id) {
                return Err(format!("question id \"{id}\" is used twice"));
            }
            ids.push(id);
            question.check().map_err(|why| format!("{id}: {why}"))?;
        }
        Ok(())
    }
}

impl Question {
    fn check(&self) -> Result<(), String> {
        if self.title.trim().is_empty() {
            return Err("give it a title".into());
        }
        bounded("title", &self.title, MAX_LABEL)?;
        bounded("text", self.text.as_deref().unwrap_or_default(), MAX_TEXT)?;
        match &self.kind {
            Kind::Single {
                options,
                recommended,
            } => {
                check_options(options)?;
                if let Some(value) = recommended {
                    known(options, value)?;
                }
            }
            Kind::Multiple {
                options,
                recommended,
            } => {
                check_options(options)?;
                for value in recommended {
                    known(options, value)?;
                }
            }
            Kind::Number {
                unit,
                min,
                max,
                recommended,
            } => {
                bounded("unit", unit, MAX_LABEL)?;
                let least = min.as_ref().and_then(Number::as_f64);
                let most = max.as_ref().and_then(Number::as_f64);
                if let (Some(least), Some(most)) = (least, most)
                    && least > most
                {
                    return Err(format!("min {least} is above max {most}"));
                }
                if let Some(n) = recommended.as_ref().and_then(Number::as_f64)
                    && (least.is_some_and(|l| n < l) || most.is_some_and(|m| n > m))
                {
                    return Err(format!("recommended {n} is outside min and max"));
                }
            }
            Kind::Text { recommended } => {
                bounded(
                    "recommended",
                    recommended.as_deref().unwrap_or_default(),
                    MAX_ANSWER,
                )?;
            }
        }
        Ok(())
    }
}

fn check_options(options: &[AskOption]) -> Result<(), String> {
    if options.len() < 2 {
        return Err("a choice needs two options or more".into());
    }
    let mut values: Vec<&str> = Vec::new();
    for option in options {
        if option.value.trim().is_empty() || option.title.trim().is_empty() {
            return Err("every option needs a value and a title".into());
        }
        bounded("option value", &option.value, MAX_LABEL)?;
        bounded("option title", &option.title, MAX_LABEL)?;
        bounded("option about", &option.about, MAX_TEXT)?;
        if values.contains(&option.value.as_str()) {
            return Err(format!("option \"{}\" is listed twice", option.value));
        }
        values.push(&option.value);
    }
    Ok(())
}

fn known(options: &[AskOption], value: &str) -> Result<(), String> {
    if options.iter().any(|o| o.value == value) {
        Ok(())
    } else {
        Err(format!(
            "recommended \"{}\" is not one of its options",
            cut(value)
        ))
    }
}

fn bounded(what: &str, text: &str, most: usize) -> Result<(), String> {
    if text.len() > most {
        Err(format!("{what} is {} bytes; at most {most}", text.len()))
    } else {
        Ok(())
    }
}

/// `text` cut for an error message.
fn cut(text: &str) -> String {
    text.chars().take(40).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn params(value: Value) -> Result<AskParams, String> {
        serde_json::from_value(value).map_err(|e| e.to_string())
    }

    fn three() -> Value {
        json!({"title": "A few things first", "questions": [
            {"id": "power", "title": "Power", "type": "single",
             "options": [{"value": "usb", "title": "USB", "about": "plugs into a phone charger"},
                         {"value": "battery", "title": "Battery"}],
             "recommended": "usb"},
            {"id": "place", "title": "Where", "type": "multiple",
             "options": [{"value": "indoor", "title": "Indoors"}, {"value": "outdoor", "title": "Outdoors"}]},
            {"id": "every", "title": "How often", "type": "number", "unit": "minutes",
             "min": 1, "max": 1440, "recommended": 10}
        ]})
    }

    #[test]
    fn a_card_of_three_kinds_passes() -> Result<(), String> {
        params(three())?.check()
    }

    #[test]
    fn any_number_of_questions_passes_within_the_byte_bound() -> Result<(), String> {
        let questions: Vec<Value> = (0..40)
            .map(|i| json!({"id": format!("q{i}"), "title": "Name", "type": "text"}))
            .collect();
        params(json!({"questions": questions}))?.check()
    }

    #[test]
    fn a_bad_card_is_refused_naming_what() -> Result<(), String> {
        let cases = [
            (json!({"questions": []}), "at least one question"),
            (
                json!({"questions": [{"id": "a b", "title": "X", "type": "text"}]}),
                "letters, digits",
            ),
            (
                json!({"questions": [{"id": "a", "title": "X", "type": "text"},
                                     {"id": "a", "title": "Y", "type": "text"}]}),
                "used twice",
            ),
            (
                json!({"questions": [{"id": "a", "title": "X", "type": "single",
                    "options": [{"value": "x", "title": "X"}]}]}),
                "two options",
            ),
            (
                json!({"questions": [{"id": "a", "title": "X", "type": "single",
                    "options": [{"value": "x", "title": "X"}, {"value": "y", "title": "Y"}],
                    "recommended": "z"}]}),
                "not one of its options",
            ),
            (
                json!({"questions": [{"id": "a", "title": "X", "type": "number",
                    "min": 5, "max": 1}]}),
                "above max",
            ),
            (
                json!({"questions": [{"id": "a", "title": "X", "type": "number",
                    "min": 1, "max": 5, "recommended": 9}]}),
                "outside min and max",
            ),
        ];
        for (value, expected) in cases {
            let why = params(value.clone())?
                .check()
                .err()
                .ok_or(format!("{value} passed"))?;
            assert!(why.contains(expected), "{value}: {why}");
        }
        Ok(())
    }

    #[test]
    fn an_unknown_type_is_refused_by_its_schema() {
        assert!(
            params(json!({"questions": [{"id": "a", "title": "X", "type": "slider"}]})).is_err()
        );
    }
}
